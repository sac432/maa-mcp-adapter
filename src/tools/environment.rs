use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{ConfigError, DiscoverySource};
use crate::maa_cli::{MaaCliAdapter, MaaCliCapabilities};

use super::{Diagnostic, SharedRuntime, ToolError, ToolResult, path_search_value};

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckEnvironmentInput {}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckEnvironmentOutput {
    pub executable_found: bool,
    pub discovery_source: Option<String>,
    pub cli_version: Option<String>,
    pub core_version: Option<String>,
    pub capabilities: CapabilityOutput,
    pub profiles: Vec<String>,
    pub can_plan: bool,
    pub can_execute: bool,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityOutput {
    pub batch: bool,
    pub profile: bool,
    pub dry_run: bool,
    pub absolute_task_path: bool,
    pub log_file: bool,
}

impl From<MaaCliCapabilities> for CapabilityOutput {
    fn from(value: MaaCliCapabilities) -> Self {
        Self {
            batch: value.batch,
            profile: value.profile,
            dry_run: value.dry_run,
            absolute_task_path: value.absolute_task_path,
            log_file: value.log_file,
        }
    }
}

pub async fn check_environment(runtime: &SharedRuntime) -> ToolResult<CheckEnvironmentOutput> {
    let can_plan = runtime.can_plan();
    let mut diagnostics = runtime
        .startup_issue_ref()
        .map(|issue| Diagnostic {
            code: issue.code.clone(),
            message: issue.message.clone(),
            recovery: issue.recovery.clone(),
        })
        .into_iter()
        .collect::<Vec<_>>();
    let Ok(config) = runtime.config() else {
        return Ok(rmcp::Json(CheckEnvironmentOutput {
            executable_found: false,
            discovery_source: None,
            cli_version: None,
            core_version: None,
            capabilities: CapabilityOutput::default(),
            profiles: Vec::new(),
            can_plan,
            can_execute: false,
            diagnostics,
        }));
    };
    let resolved = match config.resolve_maa_executable(path_search_value().as_deref()) {
        Ok(resolved) => resolved,
        Err(error) => {
            diagnostics.push(config_diagnostic(&error));
            return Ok(rmcp::Json(CheckEnvironmentOutput {
                executable_found: false,
                discovery_source: None,
                cli_version: None,
                core_version: None,
                capabilities: CapabilityOutput::default(),
                profiles: Vec::new(),
                can_plan,
                can_execute: false,
                diagnostics,
            }));
        }
    };
    let source = match resolved.source {
        DiscoverySource::Explicit => "config",
        DiscoverySource::Path => "path",
    }
    .to_owned();
    let state_directory = runtime
        .state_directory()
        .map_err(ToolError::from_runtime)
        .map_err(rmcp::Json)?;
    let adapter = MaaCliAdapter::new(resolved.path);
    match adapter.inspect_environment(state_directory).await {
        Ok(environment) => {
            let can_execute = environment.can_execute();
            diagnostics.extend(
                environment
                    .diagnostics
                    .into_iter()
                    .map(|message| Diagnostic {
                        code: "CAPABILITY_PROBE_WARNING".to_owned(),
                        message,
                        recovery: vec!["检查已安装 maa-cli 版本与命令帮助。".to_owned()],
                    }),
            );
            Ok(rmcp::Json(CheckEnvironmentOutput {
                executable_found: true,
                discovery_source: Some(source),
                cli_version: Some(environment.cli_version),
                core_version: environment.core_version,
                capabilities: environment.capabilities.into(),
                profiles: environment.profiles,
                can_plan,
                can_execute,
                diagnostics,
            }))
        }
        Err(_) => {
            diagnostics.push(Diagnostic {
                code: "MAA_CLI_PROBE_FAILED".to_owned(),
                message: "找到了 maa-cli，但无法安全完成版本或能力探测。".to_owned(),
                recovery: vec![
                    "在终端手动检查 maa version、maa dir config 和 maa run --help。".to_owned(),
                ],
            });
            Ok(rmcp::Json(CheckEnvironmentOutput {
                executable_found: true,
                discovery_source: Some(source),
                cli_version: None,
                core_version: None,
                capabilities: CapabilityOutput::default(),
                profiles: Vec::new(),
                can_plan,
                can_execute: false,
                diagnostics,
            }))
        }
    }
}

fn config_diagnostic(error: &ConfigError) -> Diagnostic {
    let (code, message, recovery) = match error {
        ConfigError::MaaCliNotFound => (
            "MAA_CLI_NOT_FOUND",
            "没有在配置或 PATH 中找到 maa.exe / maa-cli.exe。",
            "安装 maa-cli，或在 config.toml 中设置 maa_executable 的绝对路径。",
        ),
        ConfigError::RelativeExecutable(_) => (
            "MAA_CLI_PATH_RELATIVE",
            "配置的 maa_executable 不是绝对路径。",
            "将 maa_executable 改为现有文件的绝对路径。",
        ),
        ConfigError::ExecutableNotFound(_) => (
            "MAA_CLI_PATH_NOT_FOUND",
            "配置的 maa_executable 不存在。",
            "修正 maa_executable，或重新安装 maa-cli。",
        ),
        ConfigError::ExecutableNotFile(_) => (
            "MAA_CLI_PATH_NOT_FILE",
            "配置的 maa_executable 不是文件。",
            "将 maa_executable 指向 maa.exe 或 maa-cli.exe。",
        ),
        ConfigError::InvalidToml(_) => (
            "CONFIG_INVALID",
            "适配器配置无效。",
            "修正 config.toml 后重试。",
        ),
    };
    Diagnostic {
        code: code.to_owned(),
        message: message.to_owned(),
        recovery: vec![recovery.to_owned()],
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetDailyOptionsInput {
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub intent: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DailyOptionsOutput {
    pub profile: Option<String>,
    pub intent_received: bool,
    pub task_types: Vec<TaskOption>,
    pub fight_categories: Vec<ChoiceOption>,
    pub safety_limits: SafetyLimits,
    pub activity_stage_source: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskOption {
    pub task_type: String,
    pub description: String,
    pub required_fields: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SafetyLimits {
    pub fight_times: String,
    pub medicine: String,
    pub stone: u8,
    pub recruit_times: String,
    pub forced_credit_shopping: bool,
    pub preserves_credit_blacklist: bool,
    pub account_switching: bool,
    pub paid_purchases: bool,
}

pub fn get_daily_options(input: GetDailyOptionsInput) -> DailyOptionsOutput {
    DailyOptionsOutput {
        profile: input.profile,
        intent_received: input
            .intent
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty()),
        task_types: vec![
            task("start_up", "连接设备，并可选择启动游戏。", &["start_game"]),
            task(
                "fight",
                "执行主线、资源关、活动或剿灭战斗。",
                &["category", "stage", "times", "medicine"],
            ),
            task(
                "recruit",
                "执行公开招募。",
                &["max_recruits", "confirm_recruitment", "use_expedited_plans"],
            ),
            task("infrast", "使用现有配置档执行基建换班。", &[]),
            task(
                "mall",
                "访问好友并按安全限制操作信用商店。",
                &["visit_friends", "shopping"],
            ),
            task(
                "award",
                "领取日常、周常、邮件和受支持的活动奖励。",
                &["daily", "weekly", "mail", "event"],
            ),
            task("close_down", "仅在计划明确包含时关闭游戏。", &[]),
        ],
        fight_categories: vec![
            choice("main", "主线"),
            choice("resource", "资源关"),
            choice("event", "活动"),
            choice("annihilation", "剿灭"),
        ],
        safety_limits: SafetyLimits {
            fight_times: "1..=100".to_owned(),
            medicine: "0..=10".to_owned(),
            stone: 0,
            recruit_times: "0..=4".to_owned(),
            forced_credit_shopping: false,
            preserves_credit_blacklist: true,
            account_switching: false,
            paid_purchases: false,
        },
        activity_stage_source: "不访问网络；无法从本地资源确定活动关卡时，必须询问用户关卡标识。"
            .to_owned(),
    }
}

fn task(task_type: &str, description: &str, fields: &[&str]) -> TaskOption {
    TaskOption {
        task_type: task_type.to_owned(),
        description: description.to_owned(),
        required_fields: fields.iter().map(|field| (*field).to_owned()).collect(),
    }
}

fn choice(value: &str, label: &str) -> ChoiceOption {
    ChoiceOption {
        value: value.to_owned(),
        label: label.to_owned(),
    }
}
