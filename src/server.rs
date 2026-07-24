use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, tool::schema_for_type},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, transport,
};

use crate::error::AdapterResult;
use crate::runtime::AdapterRuntime;
use crate::tools::environment::{
    CheckEnvironmentInput, CheckEnvironmentOutput, DailyOptionsOutput, GetDailyOptionsInput,
};
use crate::tools::plans::{
    CancelDailyPlanInput, CreateDailyPlanInput, PreviewDailyPlanInput, UpdateDailyPlanInput,
};
use crate::tools::{PlanViewOutput, SharedRuntime, ToolError, ToolResult, parse_arguments};

const SERVER_INSTRUCTIONS: &str = "\
引导用户补全日常计划，展示完整中文摘要并取得确认后，\
才能调用执行工具。不得尝试绕过服务器端安全策略。";

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
        Self {
            runtime: std::sync::Arc::new(AdapterRuntime::from_environment()),
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
}

pub async fn serve_stdio() -> AdapterResult<()> {
    let service = AdapterServer::new().serve(transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
