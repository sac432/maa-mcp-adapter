use rmcp::{
    Json, Peer, RoleServer, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, tool::schema_for_type},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, transport,
};

use crate::error::AdapterResult;
use crate::runtime::AdapterRuntime;
use crate::tools::diagnostics::{CollectDiagnosticsInput, CollectDiagnosticsOutput};
use crate::tools::environment::{
    CheckEnvironmentInput, CheckEnvironmentOutput, DailyOptionsOutput, GetDailyOptionsInput,
};
use crate::tools::plans::{
    CancelDailyPlanInput, CreateDailyPlanInput, PreviewDailyPlanInput, UpdateDailyPlanInput,
};
use crate::tools::runs::{
    ExecuteDailyPlanInput, ExecuteDailyPlanOutput, GetRunLogInput, GetRunLogOutput,
    GetRunStatusInput, RunStatusOutput, StopRunInput,
};
use crate::tools::setup::{SetupEnvironmentInput, SetupEnvironmentOutput};
use crate::tools::{PlanViewOutput, SharedRuntime, ToolError, ToolResult, parse_arguments};

const SERVER_INSTRUCTIONS: &str = "\
先调用 check_environment；can_execute 为 false 且缺少配置档或连接信息时，\
经用户同意调用 setup_environment 一键初始化（配置写入后即时生效，无需重启）。\
引导用户补全日常计划，展示完整中文摘要并取得确认后，\
才能调用执行工具。不得尝试绕过服务器端安全策略。\
工具报错且按 recovery 无法恢复时，调用 collect_diagnostics \
生成脱敏报告，请用户核对后到 GitHub 反馈。";

#[derive(Debug, Clone)]
struct AdapterServer {
    runtime: SharedRuntime,
    tool_router: ToolRouter<Self>,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AdapterServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(SERVER_INSTRUCTIONS)
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
    }
}

impl Default for AdapterServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router(router = tool_router)]
impl AdapterServer {
    fn new() -> Self {
        let runtime = std::sync::Arc::new(AdapterRuntime::from_environment());
        if let Ok(state_directory) = runtime.state_directory() {
            let _ = crate::runs::recover_active_run(&state_directory);
            if let Ok(repository) =
                crate::storage::FileRunRepository::new(state_directory.to_path_buf())
            {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_millis() as u64);
                let retention_days = runtime.config().map_or(7, |config| config.retention_days());
                let retention_ms = u64::from(retention_days) * 24 * 60 * 60 * 1_000;
                let _ = repository.cleanup_terminal(now, retention_ms);
            }
        }
        Self {
            runtime,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "check_environment",
        description = "检查本机 maa-cli 版本、能力和配置档；不会执行日常任务。",
        input_schema = schema_for_type::<CheckEnvironmentInput>(),
        annotations(
            title = "检查 MAA 环境",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn check_environment(
        &self,
        arguments: rmcp::model::JsonObject,
    ) -> ToolResult<CheckEnvironmentOutput> {
        let _: CheckEnvironmentInput = parse_arguments(arguments)?;
        crate::tools::environment::check_environment(&self.runtime).await
    }

    #[tool(
        name = "collect_diagnostics",
        description = "生成脱敏的本地诊断报告（版本、环境、配置形状、最近运行与日志尾部）并给出 GitHub 反馈地址；不修改任何状态。",
        input_schema = schema_for_type::<CollectDiagnosticsInput>(),
        annotations(
            title = "收集诊断报告",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn collect_diagnostics(
        &self,
        arguments: rmcp::model::JsonObject,
    ) -> ToolResult<CollectDiagnosticsOutput> {
        let input = parse_arguments(arguments)?;
        crate::tools::diagnostics::collect_diagnostics(&self.runtime, input).await
    }

    #[tool(
        name = "get_daily_options",
        description = "列出安全日常任务、战斗类别、必填字段和固定安全限制；不访问网络。",
        input_schema = schema_for_type::<GetDailyOptionsInput>(),
        annotations(
            title = "获取日常选项",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_daily_options(
        &self,
        arguments: rmcp::model::JsonObject,
    ) -> Result<Json<DailyOptionsOutput>, Json<ToolError>> {
        let input = parse_arguments(arguments)?;
        Ok(Json(crate::tools::environment::get_daily_options(input)))
    }

    #[tool(
        name = "create_daily_plan",
        description = "根据一句话意图和已知结构化选项创建本地 Draft；不会运行 MAA。",
        input_schema = schema_for_type::<CreateDailyPlanInput>(),
        annotations(
            title = "创建日常计划",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_daily_plan(
        &self,
        arguments: rmcp::model::JsonObject,
    ) -> ToolResult<PlanViewOutput> {
        let input = parse_arguments(arguments)?;
        crate::tools::plans::create_daily_plan(&self.runtime, input)
    }

    #[tool(
        name = "update_daily_plan",
        description = "以带 revision 的结构化 patch 更新本地计划；不会运行 MAA。",
        input_schema = schema_for_type::<UpdateDailyPlanInput>(),
        annotations(
            title = "更新日常计划",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_daily_plan(
        &self,
        arguments: rmcp::model::JsonObject,
    ) -> ToolResult<PlanViewOutput> {
        let input = parse_arguments(arguments)?;
        crate::tools::plans::update_daily_plan(&self.runtime, input)
    }

    #[tool(
        name = "preview_daily_plan",
        description = "验证并生成规范中文摘要；完整 Draft 会持久化为 Ready，但不会运行 MAA。",
        input_schema = schema_for_type::<PreviewDailyPlanInput>(),
        annotations(
            title = "预览日常计划",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn preview_daily_plan(
        &self,
        arguments: rmcp::model::JsonObject,
    ) -> ToolResult<PlanViewOutput> {
        let input = parse_arguments(arguments)?;
        crate::tools::plans::preview_daily_plan(&self.runtime, input)
    }

    #[tool(
        name = "cancel_daily_plan",
        description = "取消尚未执行的本地计划；不会影响运行中的任务。",
        input_schema = schema_for_type::<CancelDailyPlanInput>(),
        annotations(
            title = "取消日常计划",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn cancel_daily_plan(
        &self,
        arguments: rmcp::model::JsonObject,
    ) -> ToolResult<PlanViewOutput> {
        let input = parse_arguments(arguments)?;
        crate::tools::plans::cancel_daily_plan(&self.runtime, input)
    }

    #[tool(
        name = "execute_daily_plan",
        description = "逐字核对 expected_summary 与计划哈希后展示 MCP 二次确认，并在接受后启动唯一的后台 MAA 日常任务。",
        input_schema = schema_for_type::<ExecuteDailyPlanInput>(),
        annotations(
            title = "确认并执行日常计划",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn execute_daily_plan(
        &self,
        arguments: rmcp::model::JsonObject,
        peer: Peer<RoleServer>,
    ) -> ToolResult<ExecuteDailyPlanOutput> {
        let input = parse_arguments(arguments)?;
        crate::tools::runs::execute_daily_plan(&self.runtime, input, peer).await
    }

    #[tool(
        name = "setup_environment",
        description = "从 MAA 图形版配置发现启动信息，经表单确认后新建 maa-cli 配置档并补全适配器 [emulator] 配置；绝不覆盖已有文件。",
        input_schema = schema_for_type::<SetupEnvironmentInput>(),
        annotations(
            title = "一键初始化环境",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn setup_environment(
        &self,
        arguments: rmcp::model::JsonObject,
        peer: Peer<RoleServer>,
    ) -> ToolResult<SetupEnvironmentOutput> {
        let input = parse_arguments(arguments)?;
        crate::tools::setup::setup_environment(&self.runtime, input, peer).await
    }

    #[tool(
        name = "get_run_status",
        description = "核对持久化运行记录并返回当前 MAA 任务状态。",
        input_schema = schema_for_type::<GetRunStatusInput>(),
        annotations(
            title = "查询运行状态",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn get_run_status(&self, arguments: rmcp::model::JsonObject) -> ToolResult<RunStatusOutput> {
        let input = parse_arguments(arguments)?;
        crate::tools::runs::get_run_status(&self.runtime, input)
    }

    #[tool(
        name = "get_run_log",
        description = "读取记录内 stdout 或 stderr 的受限、脱敏尾部内容。",
        input_schema = schema_for_type::<GetRunLogInput>(),
        annotations(
            title = "读取受限运行日志",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn get_run_log(&self, arguments: rmcp::model::JsonObject) -> ToolResult<GetRunLogOutput> {
        let input = parse_arguments(arguments)?;
        crate::tools::runs::get_run_log(&self.runtime, input)
    }

    #[tool(
        name = "stop_run",
        description = "仅在 PID 与创建时间匹配时先请求优雅停止，再强制终止对应命名 Job。",
        input_schema = schema_for_type::<StopRunInput>(),
        annotations(
            title = "停止运行任务",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn stop_run(&self, arguments: rmcp::model::JsonObject) -> ToolResult<RunStatusOutput> {
        let input = parse_arguments(arguments)?;
        crate::tools::runs::stop_run(&self.runtime, input).await
    }
}

pub async fn serve_stdio() -> AdapterResult<()> {
    crate::platform::harden_stdio_inheritance()?;
    let service = AdapterServer::new().serve(transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
