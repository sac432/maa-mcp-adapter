use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::EmulatorConfig;
use crate::domain::ProcessStartIdentity;
use crate::platform::{PlatformError, process_identity_for_child, terminate_if_matches};

const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
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

/// 确保模拟器在线。已在线（地址可连通）时不做任何事并返回 `None`；
/// 否则启动配置的可执行文件并等待地址可连通（未配置地址时只做定长等待）。
pub async fn ensure_running(
    config: &EmulatorConfig,
) -> Result<Option<StartedEmulator>, EmulatorError> {
    if let Some(address) = config.address.as_deref()
        && address_is_open(address).await
    {
        return Ok(None);
    }

    let mut command = std::process::Command::new(&config.path);
    command
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

    let wait = Duration::from_secs(u64::from(config.startup_wait_seconds));
    match config.address.as_deref() {
        Some(address) => {
            let deadline = tokio::time::Instant::now() + wait;
            loop {
                if address_is_open(address).await {
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

    Ok(Some(started))
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

pub fn write_marker(run_directory: &Path, started: &StartedEmulator) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(started).expect("started emulator is serializable");
    std::fs::write(marker_path(run_directory), bytes)
}

/// 终态清理：若运行目录中记录了由本适配器启动的模拟器，则在身份完全
/// 匹配时关闭它。任何失败都只放弃关闭，绝不触碰其他进程。
pub fn stop_marked_emulator(run_directory: &Path) {
    let path = marker_path(run_directory);
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    if let Ok(started) = serde_json::from_slice::<StartedEmulator>(&bytes) {
        let _ = terminate_if_matches(started.pid, started.identity());
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
            address: Some(address),
            startup_wait_seconds: 1,
            stop_after_run: true,
        };

        let started = ensure_running(&config).await.expect("no spawn needed");

        assert!(started.is_none());
    }

    #[tokio::test]
    async fn a_missing_executable_fails_before_touching_anything() {
        let config = EmulatorConfig {
            path: std::path::PathBuf::from("Z:/definitely/not/a/real/emulator.exe"),
            address: Some("127.0.0.1:9".to_owned()),
            startup_wait_seconds: 1,
            stop_after_run: false,
        };

        let error = ensure_running(&config)
            .await
            .expect_err("missing emulator executable must fail");

        assert!(matches!(error, EmulatorError::Spawn { .. }));
    }

    #[tokio::test]
    async fn an_unreachable_address_reports_not_ready_within_the_deadline() {
        let helper = std::env::var_os("ComSpec").expect("cmd.exe available");
        let config = EmulatorConfig {
            path: std::path::PathBuf::from(helper),
            address: Some("127.0.0.1:9".to_owned()),
            startup_wait_seconds: 1,
            stop_after_run: false,
        };

        let started = tokio::time::timeout(Duration::from_secs(10), ensure_running(&config))
            .await
            .expect("deadline is honoured");

        assert!(matches!(started, Err(EmulatorError::NotReady { .. })));
    }
}
