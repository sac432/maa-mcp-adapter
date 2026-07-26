use schemars::JsonSchema;
use serde::Deserialize;

use crate::domain::DailyTask;
use crate::planning::service::PlanPatch;

use super::{PlanViewOutput, SharedRuntime, ToolError, ToolResult, parse_plan_id, plan_service};

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateDailyPlanInput {
    #[serde(default)]
    pub intent: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub tasks: Vec<DailyTask>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateDailyPlanInput {
    pub plan_id: String,
    pub expected_revision: u64,
    pub patch: PlanPatch,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewDailyPlanInput {
    pub plan_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelDailyPlanInput {
    pub plan_id: String,
    pub expected_revision: u64,
}

pub fn create_daily_plan(
    runtime: &SharedRuntime,
    input: CreateDailyPlanInput,
) -> ToolResult<PlanViewOutput> {
    let _intent_was_supplied = input
        .intent
        .as_ref()
        .is_some_and(|intent| !intent.trim().is_empty());
    let tasks = if input.tasks.is_empty() {
        match runtime.config() {
            Ok(config) => config
                .maa_gui_config
                .as_deref()
                .and_then(crate::planning::maa_defaults::derive_default_tasks)
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    } else {
        input.tasks
    };
    let service = plan_service(runtime).map_err(rmcp::Json)?;
    let view = service
        .create(input.profile.unwrap_or_default(), tasks)
        .map_err(|error| rmcp::Json(ToolError::from_service(error)))?;
    Ok(rmcp::Json(PlanViewOutput::from_view(view)))
}

pub fn update_daily_plan(
    runtime: &SharedRuntime,
    input: UpdateDailyPlanInput,
) -> ToolResult<PlanViewOutput> {
    let plan_id = parse_plan_id(&input.plan_id)?;
    let service = plan_service(runtime).map_err(rmcp::Json)?;
    let view = service
        .update(&plan_id, input.expected_revision, input.patch)
        .map_err(|error| rmcp::Json(ToolError::from_service(error)))?;
    Ok(rmcp::Json(PlanViewOutput::from_view(view)))
}

pub fn preview_daily_plan(
    runtime: &SharedRuntime,
    input: PreviewDailyPlanInput,
) -> ToolResult<PlanViewOutput> {
    let plan_id = parse_plan_id(&input.plan_id)?;
    let service = plan_service(runtime).map_err(rmcp::Json)?;
    let view = service
        .preview(&plan_id)
        .map_err(|error| rmcp::Json(ToolError::from_service(error)))?;
    Ok(rmcp::Json(PlanViewOutput::from_view(view)))
}

pub fn cancel_daily_plan(
    runtime: &SharedRuntime,
    input: CancelDailyPlanInput,
) -> ToolResult<PlanViewOutput> {
    let plan_id = parse_plan_id(&input.plan_id)?;
    let service = plan_service(runtime).map_err(rmcp::Json)?;
    let view = service
        .cancel(&plan_id, input.expected_revision)
        .map_err(|error| rmcp::Json(ToolError::from_service(error)))?;
    Ok(rmcp::Json(PlanViewOutput::from_view(view)))
}
