use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{AdapterConfig, ConfigError};
use crate::storage::{StorageError, resolve_state_directory};

#[derive(Debug)]
pub struct AdapterRuntime {
    config: Option<AdapterConfig>,
    state_directory: Option<PathBuf>,
    startup_issue: Option<RuntimeIssue>,
}

impl AdapterRuntime {
    pub fn from_environment() -> Self {
        match load_runtime() {
            Ok((config, state_directory)) => Self {
                config: Some(config),
                state_directory: Some(state_directory),
                startup_issue: None,
            },
            Err(issue) => Self {
                config: None,
                state_directory: None,
                startup_issue: Some(issue),
            },
        }
    }

    pub fn config(&self) -> Result<&AdapterConfig, RuntimeIssue> {
        self.config.as_ref().ok_or_else(|| self.startup_issue())
    }

    pub fn state_directory(&self) -> Result<&Path, RuntimeIssue> {
        self.state_directory
            .as_deref()
            .ok_or_else(|| self.startup_issue())
    }

    pub fn can_plan(&self) -> bool {
        self.startup_issue.is_none()
    }

    pub fn startup_issue_ref(&self) -> Option<&RuntimeIssue> {
        self.startup_issue.as_ref()
    }

    fn startup_issue(&self) -> RuntimeIssue {
        self.startup_issue.clone().unwrap_or_else(|| {
            RuntimeIssue::new(
                "RUNTIME_UNAVAILABLE",
                "适配器运行环境不可用。",
                Vec::new(),
                vec!["检查适配器配置后重试。".to_owned()],
            )
        })
    }
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
