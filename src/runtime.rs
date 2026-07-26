use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::config::{AdapterConfig, ConfigError};
use crate::storage::{StorageError, resolve_state_directory};

/// 运行时按 `config.toml` 的修改时间热加载配置：`setup_environment` 或
/// 用户编辑配置后，下一个工具调用即生效，无需重启 MCP Server。
#[derive(Debug)]
pub struct AdapterRuntime {
    cached: Mutex<CachedRuntime>,
}

#[derive(Debug)]
struct CachedRuntime {
    config: Option<Arc<AdapterConfig>>,
    state_directory: Option<PathBuf>,
    issue: Option<RuntimeIssue>,
    config_modified: Option<SystemTime>,
}

impl AdapterRuntime {
    pub fn from_environment() -> Self {
        Self {
            cached: Mutex::new(Self::snapshot()),
        }
    }

    fn snapshot() -> CachedRuntime {
        let config_modified = config_modified_time();
        match load_runtime() {
            Ok((config, state_directory)) => CachedRuntime {
                config: Some(Arc::new(config)),
                state_directory: Some(state_directory),
                issue: None,
                config_modified,
            },
            Err(issue) => CachedRuntime {
                config: None,
                state_directory: None,
                issue: Some(issue),
                config_modified,
            },
        }
    }

    fn refreshed(&self) -> std::sync::MutexGuard<'_, CachedRuntime> {
        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = config_modified_time();
        if current != cached.config_modified {
            let fresh = Self::snapshot();
            // 状态目录热切换会让活动锁与运行记录跨目录脱节（可能出现双活动
            // 运行），因此整个新配置钉住到重启为止；其余变更整体原子生效。
            let state_directory_moved = matches!(
                (&cached.state_directory, &fresh.state_directory),
                (Some(old), Some(new)) if old != new
            );
            if state_directory_moved {
                cached.config_modified = fresh.config_modified;
            } else {
                *cached = fresh;
            }
        }
        cached
    }

    pub fn config(&self) -> Result<Arc<AdapterConfig>, RuntimeIssue> {
        let cached = self.refreshed();
        cached.config.clone().ok_or_else(|| issue_of(&cached))
    }

    pub fn state_directory(&self) -> Result<PathBuf, RuntimeIssue> {
        let cached = self.refreshed();
        cached
            .state_directory
            .clone()
            .ok_or_else(|| issue_of(&cached))
    }

    pub fn can_plan(&self) -> bool {
        self.refreshed().issue.is_none()
    }

    pub fn startup_issue_ref(&self) -> Option<RuntimeIssue> {
        self.refreshed().issue.clone()
    }
}

fn issue_of(cached: &CachedRuntime) -> RuntimeIssue {
    cached.issue.clone().unwrap_or_else(|| {
        RuntimeIssue::new(
            "RUNTIME_UNAVAILABLE",
            "适配器运行环境不可用。",
            Vec::new(),
            vec!["检查适配器配置后重试。".to_owned()],
        )
    })
}

fn config_modified_time() -> Option<SystemTime> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    let default_state = resolve_state_directory(None, Some(&local_app_data)).ok()?;
    fs::metadata(default_state.join("config.toml"))
        .and_then(|metadata| metadata.modified())
        .ok()
}

#[derive(Debug, Clone)]
pub struct RuntimeIssue {
    pub code: String,
    pub message: String,
    pub details: Vec<String>,
    pub recovery: Vec<String>,
}

impl RuntimeIssue {
    fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        details: Vec<String>,
        recovery: Vec<String>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details,
            recovery,
        }
    }
}

fn load_runtime() -> Result<(AdapterConfig, PathBuf), RuntimeIssue> {
    let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        RuntimeIssue::new(
            "LOCAL_APP_DATA_UNAVAILABLE",
            "无法确定 Windows 应用数据目录。",
            Vec::new(),
            vec!["设置 LOCALAPPDATA，或在受支持的 Windows 用户会话中启动适配器。".to_owned()],
        )
    })?;
    let default_state =
        resolve_state_directory(None, Some(&local_app_data)).map_err(runtime_storage_issue)?;
    let config_path = default_state.join("config.toml");
    let config = match fs::read_to_string(&config_path) {
        Ok(contents) => AdapterConfig::from_toml(&contents).map_err(|error| {
            RuntimeIssue::new(
                "CONFIG_INVALID",
                "适配器配置文件无效。",
                vec![
                    format!("配置文件：{}", config_path.display()),
                    safe_config_error(&error),
                ],
                vec!["修正配置文件中的未知字段或路径后重试。".to_owned()],
            )
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => AdapterConfig::default(),
        Err(_) => {
            return Err(RuntimeIssue::new(
                "CONFIG_UNREADABLE",
                "无法读取适配器配置文件。",
                vec![format!("配置文件：{}", config_path.display())],
                vec!["检查该文件的读取权限后重试。".to_owned()],
            ));
        }
    };
    let state_directory =
        resolve_state_directory(config.state_directory.as_deref(), Some(&local_app_data))
            .map_err(runtime_storage_issue)?;
    Ok((config, state_directory))
}

fn runtime_storage_issue(error: StorageError) -> RuntimeIssue {
    match error {
        StorageError::MissingLocalAppData | StorageError::InvalidStateDirectory(_) => {
            RuntimeIssue::new(
                "STATE_DIRECTORY_INVALID",
                "适配器状态目录无效。",
                Vec::new(),
                vec!["在 config.toml 中设置不含 . 或 .. 的绝对 state_directory。".to_owned()],
            )
        }
        _ => RuntimeIssue::new(
            "STORAGE_UNAVAILABLE",
            "无法初始化适配器状态目录。",
            Vec::new(),
            vec!["检查状态目录权限和可用空间后重试。".to_owned()],
        ),
    }
}

fn safe_config_error(error: &ConfigError) -> String {
    match error {
        ConfigError::InvalidToml(_) => "TOML 语法错误或包含未知字段。".to_owned(),
        ConfigError::RelativeExecutable(path) => {
            format!("maa_executable 必须是绝对路径：{}", path.display())
        }
        ConfigError::ExecutableNotFound(path) => {
            format!("maa_executable 不存在：{}", path.display())
        }
        ConfigError::ExecutableNotFile(path) => {
            format!("maa_executable 不是文件：{}", path.display())
        }
        ConfigError::MaaCliNotFound => "未找到 maa.exe 或 maa-cli.exe。".to_owned(),
    }
}
