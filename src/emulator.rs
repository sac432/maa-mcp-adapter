use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::EmulatorConfig;
use crate::domain::ProcessStartIdentity;
use crate::platform::{PlatformError, process_identity_for_child, terminate_if_matches};

const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const ADB_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Error)]
pub enum EmulatorError {
    #[error("failed to start emulator {path}: {source}")]
    Spawn {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to record spawned emulator identity: {0}")]
    Identity(#[from] PlatformError),
    #[error("emulator address {address} did not become reachable within {waited:?}")]
    NotReady { address: String, waited: Duration },
}

/// 由适配器本次启动的模拟器进程；仅这类进程允许在运行结束后被关闭。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartedEmulator {
    pub pid: u32,
    pub creation_time_100ns: u64,
}

impl StartedEmulator {
    pub fn identity(&self) -> ProcessStartIdentity {
        ProcessStartIdentity {
            creation_time_100ns: self.creation_time_100ns,
        }
    }
}

/// 终态时关闭模拟器的方式，由启动方在标记文件中记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum EmulatorStop {
    /// 终止启动时记录的进程；PID 与创建时间双重匹配才生效。
    Terminate { pid: u32, creation_time_100ns: u64 },
    /// 执行一次用户配置的关机命令（启动器型 CLI，如 MuMuManager shutdown）。
    Command { path: PathBuf, args: Vec<String> },
}

/// 确保模拟器在线并真正可用。快路径：地址可连通且（配置了 `adb_path`
/// 时）安卓已启动完成，直接返回 `None`。否则在端口不可达时启动配置的
/// 可执行文件，并在总时限内等待端口可达、安卓启动完成，最后按
/// `boot_settle_seconds` 静置。返回 `Some` 仅表示本次由适配器启动。
pub async fn ensure_running(
    config: &EmulatorConfig,
) -> Result<Option<StartedEmulator>, EmulatorError> {
    if let Some(address) = config.address.as_deref()
        && address_is_open(address).await
        && boot_completed(config, address).await
    {
        return Ok(None);
    }

    let started = match config.address.as_deref() {
        Some(address) if address_is_open(address).await => None,
        _ => Some(spawn_emulator(config)?),
    };

    let wait = Duration::from_secs(u64::from(config.startup_wait_seconds));
    match config.address.as_deref() {
        Some(address) => {
            let deadline = tokio::time::Instant::now() + wait;
            loop {
                if address_is_open(address).await && boot_completed(config, address).await {
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(EmulatorError::NotReady {
                        address: address.to_owned(),
                        waited: wait,
                    });
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
        None => tokio::time::sleep(wait).await,
    }
    tokio::time::sleep(Duration::from_secs(u64::from(config.boot_settle_seconds))).await;

    Ok(started)
}

fn spawn_emulator(config: &EmulatorConfig) -> Result<StartedEmulator, EmulatorError> {
    let mut command = std::process::Command::new(&config.path);
    command
        .args(&config.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }
    let child = command.spawn().map_err(|source| EmulatorError::Spawn {
        path: config.path.clone(),
        source,
    })?;
    let identity = process_identity_for_child(&child)?;
    let started = StartedEmulator {
        pid: child.id(),
        creation_time_100ns: identity.creation_time_100ns,
    };
    drop(child);
    Ok(started)
}

/// 未配置 `adb_path` 时退化为恒真（只依赖端口探测）。配置后通过
/// `adb shell getprop sys.boot_completed` 判定安卓真正启动完成。
async fn boot_completed(config: &EmulatorConfig, address: &str) -> bool {
    let Some(adb) = config.adb_path.as_deref() else {
        return true;
    };
    let connect_args = [OsString::from("connect"), OsString::from(address)];
    let _ = crate::maa_cli::process::run(adb, &connect_args, ADB_PROBE_TIMEOUT).await;
    let getprop_args = [
        OsString::from("-s"),
        OsString::from(address),
        OsString::from("shell"),
        OsString::from("getprop"),
        OsString::from("sys.boot_completed"),
    ];
    match crate::maa_cli::process::run(adb, &getprop_args, ADB_PROBE_TIMEOUT).await {
        Ok(output) => output.stdout.trim() == "1",
        Err(_) => false,
    }
}

async fn address_is_open(address: &str) -> bool {
    matches!(
        tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(address)).await,
        Ok(Ok(_))
    )
}

pub fn marker_path(run_directory: &Path) -> PathBuf {
    run_directory.join("emulator.json")
}

pub fn write_marker(run_directory: &Path, stop: &EmulatorStop) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(stop).expect("emulator stop marker is serializable");
    std::fs::write(marker_path(run_directory), bytes)
}

/// 终态清理：执行运行目录中记录的关停方式。`Terminate` 只在 PID 与创建
/// 时间完全匹配时生效；`Command` 执行一次用户配置的关机命令。任何失败都
/// 只放弃关闭，绝不触碰其他进程。
pub fn stop_marked_emulator(run_directory: &Path) {
    let path = marker_path(run_directory);
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    match serde_json::from_slice::<EmulatorStop>(&bytes) {
        Ok(EmulatorStop::Terminate {
            pid,
            creation_time_100ns,
        }) => {
            let _ = terminate_if_matches(
                pid,
                ProcessStartIdentity {
                    creation_time_100ns,
                },
            );
        }
        Ok(EmulatorStop::Command { path, args }) => {
            let _ = std::process::Command::new(path)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        Err(_) => {}
    }
    let _ = std::fs::remove_file(&path);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::config::EmulatorConfig;

    use super::{EmulatorError, ensure_running};

    #[tokio::test]
    async fn an_already_reachable_address_spawns_nothing() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
        let address = listener.local_addr().expect("listener address").to_string();
        let config = EmulatorConfig {
            path: std::path::PathBuf::from("Z:/definitely/not/a/real/emulator.exe"),
            args: Vec::new(),
            address: Some(address),
            adb_path: None,
            startup_wait_seconds: 1,
            boot_settle_seconds: 0,
            stop_after_run: true,
            stop_args: None,
        };

        let started = ensure_running(&config).await.expect("no spawn needed");

        assert!(started.is_none());
    }

    #[tokio::test]
    async fn a_missing_executable_fails_before_touching_anything() {
        let config = EmulatorConfig {
            path: std::path::PathBuf::from("Z:/definitely/not/a/real/emulator.exe"),
            args: Vec::new(),
            address: Some("127.0.0.1:9".to_owned()),
            adb_path: None,
            startup_wait_seconds: 1,
            boot_settle_seconds: 0,
            stop_after_run: false,
            stop_args: None,
        };

        let error = ensure_running(&config)
            .await
            .expect_err("missing emulator executable must fail");

        assert!(matches!(error, EmulatorError::Spawn { .. }));
    }

    #[test]
    fn a_command_marker_runs_the_configured_shutdown_once() {
        let temp = tempfile::tempdir().expect("run directory");
        let sentinel = temp.path().join("stopped-sentinel");
        let comspec = std::env::var_os("ComSpec").expect("cmd.exe available");
        super::write_marker(
            temp.path(),
            &super::EmulatorStop::Command {
                path: comspec.into(),
                args: vec![
                    "/C".to_owned(),
                    "mkdir".to_owned(),
                    sentinel.display().to_string(),
                ],
            },
        )
        .expect("write marker");

        super::stop_marked_emulator(temp.path());

        assert!(sentinel.is_dir(), "shutdown command executed");
        assert!(
            !super::marker_path(temp.path()).exists(),
            "marker consumed exactly once"
        );
    }

    #[tokio::test]
    async fn an_unreachable_address_reports_not_ready_within_the_deadline() {
        let helper = std::env::var_os("ComSpec").expect("cmd.exe available");
        let config = EmulatorConfig {
            path: std::path::PathBuf::from(helper),
            args: Vec::new(),
            address: Some("127.0.0.1:9".to_owned()),
            adb_path: None,
            startup_wait_seconds: 1,
            boot_settle_seconds: 0,
            stop_after_run: false,
            stop_args: None,
        };

        let started = tokio::time::timeout(Duration::from_secs(10), ensure_running(&config))
            .await
            .expect("deadline is honoured");

        assert!(matches!(started, Err(EmulatorError::NotReady { .. })));
    }
}
