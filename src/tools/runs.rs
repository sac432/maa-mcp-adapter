use rmcp::service::ElicitationError;
use rmcp::{Peer, RoleServer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::ConfigError;
use crate::execution::{ExecutionError, ExecutionService};
use crate::maa_cli::MaaCliAdapter;

use super::{SharedRuntime, ToolError, ToolResult, parse_plan_id, path_search_value};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteDailyPlanInput {
    pub plan_id: String,
    pub expected_plan_hash: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteDailyPlanOutput {
    pub outcome: String,
    pub run_id: Option<String>,
    pub pid: Option<u32>,
    pub status: Option<String>,
    pub summary: String,
    pub stdout_path: Option<String>,
    pub stderr_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExecutionConfirmation {
    #[schemars(description = "确认执行上述准确计划时选择 true，否则选择 false")]
    confirmed: bool,
}

rmcp::elicit_safe!(ExecutionConfirmation);

pub async fn execute_daily_plan(
    runtime: &SharedRuntime,
    input: ExecuteDailyPlanInput,
    peer: Peer<RoleServer>,
) -> ToolResult<ExecuteDailyPlanOutput> {
    let plan_id = parse_plan_id(&input.plan_id)?;
    let state_directory = runtime
        .state_directory()
        .map_err(ToolError::from_runtime)
        .map_err(rmcp::Json)?;
    let execution = ExecutionService::new(state_directory.to_path_buf());
    let prepared = execution
        .prepare(&plan_id, &input.expected_plan_hash)
        .map_err(ToolError::from_execution)
        .map_err(rmcp::Json)?;
    let message = format!(
        "{}\n计划哈希：{}\n\n是否确认执行以上准确计划？",
        prepared.summary(),
        prepared.plan_hash().as_str()
    );

    let confirmation = match peer.elicit::<ExecutionConfirmation>(message).await {
        Ok(Some(confirmation)) => confirmation,
        Ok(None) | Err(ElicitationError::UserCancelled) => {
            return Ok(rmcp::Json(not_started("cancelled", prepared.summary())));
        }
        Err(ElicitationError::UserDeclined) => {
            return Ok(rmcp::Json(not_started("declined", prepared.summary())));
        }
        Err(ElicitationError::CapabilityNotSupported) => {
            return Err(rmcp::Json(ToolError::new(
                "CLIENT_ELICITATION_UNSUPPORTED",
                "当前 MCP 客户端未声明表单 elicitation 能力。",
                Vec::new(),
                vec!["使用支持 MCP elicitation 的客户端重新连接后再执行。".to_owned()],
            )));
        }
        Err(_) => {
            return Err(rmcp::Json(ToolError::new(
                "ELICITATION_FAILED",
                "无法取得可靠的执行确认。",
                Vec::new(),
                vec!["重新查看计划预览，并显式发起新的执行请求。".to_owned()],
            )));
        }
    };
    if !confirmation.confirmed {
        return Ok(rmcp::Json(not_started("declined", prepared.summary())));
    }

    let config = runtime
        .config()
        .map_err(ToolError::from_runtime)
        .map_err(rmcp::Json)?;
    let resolved = config
        .resolve_maa_executable(path_search_value().as_deref())
        .map_err(maa_config_error)
        .map_err(rmcp::Json)?;
    let adapter = MaaCliAdapter::new(resolved.path);
    let launched = execution
        .launch(&prepared, &adapter)
        .await
        .map_err(ToolError::from_execution)
        .map_err(rmcp::Json)?;
    Ok(rmcp::Json(ExecuteDailyPlanOutput {
        outcome: "started".to_owned(),
        run_id: Some(launched.record.run_id.to_string()),
        pid: Some(launched.record.pid),
        status: Some("running".to_owned()),
        summary: launched.summary,
        stdout_path: Some(launched.record.stdout_path.display().to_string()),
        stderr_path: Some(launched.record.stderr_path.display().to_string()),
    }))
}

fn not_started(outcome: &str, summary: &str) -> ExecuteDailyPlanOutput {
    ExecuteDailyPlanOutput {
        outcome: outcome.to_owned(),
        run_id: None,
        pid: None,
        status: None,
        summary: summary.to_owned(),
        stdout_path: None,
        stderr_path: None,
    }
}

fn maa_config_error(error: ConfigError) -> ToolError {
    let details = match &error {
        ConfigError::RelativeExecutable(path)
        | ConfigError::ExecutableNotFound(path)
        | ConfigError::ExecutableNotFile(path) => {
            vec![format!("maa_executable：{}", path.display())]
        }
        ConfigError::InvalidToml(_) | ConfigError::MaaCliNotFound => Vec::new(),
    };
    ToolError::new(
        "MAA_CLI_UNAVAILABLE",
        "无法找到可安全执行的 maa-cli。",
        details,
        vec!["修正 config.toml 中的绝对路径，或将 maa.exe / maa-cli.exe 加入 PATH。".to_owned()],
    )
}

impl ToolError {
    pub fn from_execution(error: ExecutionError) -> Self {
        let code = error.code();
        let (message, recovery) = match code {
            "PLAN_NOT_FOUND" => (
                "没有找到指定计划。",
                "使用 create_daily_plan 返回的 plan_id。",
            ),
            "PLAN_NOT_READY" => (
                "计划尚未完成规范预览。",
                "先调用 preview_daily_plan，确认计划进入 Ready。",
            ),
            "PLAN_HASH_INVALID" | "PLAN_HASH_MISMATCH" => (
                "请求中的计划哈希与当前 Ready 计划不一致。",
                "重新调用 preview_daily_plan，并使用其准确 plan_hash。",
            ),
            "PLAN_CHANGED" => (
                "计划在确认前后发生了变化。",
                "重新预览完整计划并发起新的执行请求。",
            ),
            "ACTIVE_RUN_EXISTS" => (
                "已有活动中的 MAA 任务。",
                "等待当前任务结束，或查询后显式停止当前任务。",
            ),
            "TASK_FILE_CHANGED" => (
                "任务文件在验证后发生变化。",
                "检查状态目录安全性后重新创建并预览计划。",
            ),
            "PROFILE_UNAVAILABLE" => (
                "计划选择的配置档当前不可用。",
                "调用 check_environment，并改用返回的配置档。",
            ),
            "MAA_CAPABILITIES_UNAVAILABLE" | "ENVIRONMENT_CHECK_FAILED" => (
                "maa-cli 环境不满足安全执行要求。",
                "调用 check_environment 并修复所有执行能力诊断。",
            ),
            "PROCESS_ROLLBACK_FAILED" => (
                "启动失败后无法验证并终止目标进程。",
                "不要重试；手动检查活动锁、运行记录和对应 PID。",
            ),
            _ => (
                "无法安全启动日常任务。",
                "保留状态目录并根据 check_environment 诊断排查后重试。",
            ),
        };
        Self::new(code, message, Vec::new(), vec![recovery.to_owned()])
    }
}
