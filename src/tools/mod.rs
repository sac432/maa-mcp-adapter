use std::ffi::OsString;
use std::str::FromStr;
use std::sync::Arc;

use rmcp::Json;
use rmcp::model::JsonObject;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::domain::{DailyTask, PlanId, PlanState};
use crate::planning::hash::hash_plan;
use crate::planning::service::{PlanService, PlanView, ServiceError};
use crate::planning::{MissingField, TaskKind, ValidationError};
use crate::runtime::{AdapterRuntime, RuntimeIssue};
use crate::storage::{FilePlanRepository, StorageError};

pub mod environment;
pub mod plans;

pub fn plan_service(runtime: &SharedRuntime) -> Result<PlanService<FilePlanRepository>, ToolError> {
    let state_directory = runtime.state_directory().map_err(ToolError::from_runtime)?;
    let repository = FilePlanRepository::new(state_directory.to_path_buf())
        .map_err(ToolError::from_service_storage)?;
    Ok(PlanService::new(repository))
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolError {
    pub error: ErrorBody,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub details: Vec<String>,
    pub recovery: Vec<String>,
}

impl ToolError {
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        details: Vec<String>,
        recovery: Vec<String>,
    ) -> Self {
        Self {
            error: ErrorBody {
                code: code.into(),
                message: message.into(),
                details,
                recovery,
            },
        }
    }

    pub fn invalid_arguments(error: serde_json::Error) -> Self {
        Self::new(
            "INVALID_ARGUMENTS",
            "工具参数无效。",
            vec![error.to_string()],
            vec!["仅使用 tools/list Schema 中声明的字段和值后重试。".to_owned()],
        )
    }

    pub fn invalid_plan_id() -> Self {
        Self::new(
            "INVALID_PLAN_ID",
            "计划 ID 无效。",
            Vec::new(),
            vec!["使用 create_daily_plan 返回的 plan_id。".to_owned()],
        )
    }

    pub fn from_runtime(issue: RuntimeIssue) -> Self {
        Self::new(issue.code, issue.message, issue.details, issue.recovery)
    }

    pub fn from_service(error: ServiceError) -> Self {
        match error {
            ServiceError::Storage(error) => Self::from_service_storage(error),
            ServiceError::InvalidTaskIndex { task_index } => Self::new(
                "INVALID_TASK_INDEX",
                "任务序号超出计划范围。",
                vec![format!("task_index={task_index}")],
                vec!["先查看当前计划，再使用有效的任务序号。".to_owned()],
            ),
            ServiceError::CannotPreview { state } => Self::invalid_state("预览", state),
            ServiceError::CannotCancel { state } => Self::invalid_state("取消", state),
            ServiceError::Domain(_) => Self::new(
                "PLAN_INVALID",
                "计划无法完成该操作。",
                Vec::new(),
                vec!["根据缺失字段和验证错误修正计划后重试。".to_owned()],
            ),
        }
    }

    fn invalid_state(operation: &str, state: PlanState) -> Self {
        Self::new(
            "INVALID_PLAN_STATE",
            format!("当前状态的计划不能{operation}。"),
            vec![format!("state={}", state_name(state))],
            vec!["重新创建计划，或对仍可编辑的计划执行该操作。".to_owned()],
        )
    }

    fn from_service_storage(error: StorageError) -> Self {
        match error {
            StorageError::NotFound => Self::new(
                "PLAN_NOT_FOUND",
                "没有找到指定计划。",
                Vec::new(),
                vec!["检查 plan_id 是否来自当前适配器状态目录。".to_owned()],
            ),
            StorageError::RevisionConflict { expected, actual } => Self::new(
                "REVISION_CONFLICT",
                "计划已被其他操作更新。",
                vec![
                    format!("expected_revision={expected}"),
                    format!("actual_revision={actual}"),
                ],
                vec!["使用最新 revision 重新提交修改。".to_owned()],
            ),
            StorageError::MissingLocalAppData | StorageError::InvalidStateDirectory(_) => {
                Self::new(
                    "STATE_DIRECTORY_INVALID",
                    "适配器状态目录无效。",
                    Vec::new(),
                    vec!["在 config.toml 中设置不含 . 或 .. 的绝对 state_directory。".to_owned()],
                )
            }
            StorageError::AlreadyExists => Self::new(
                "PLAN_ID_COLLISION",
                "无法分配新的计划 ID。",
                Vec::new(),
                vec!["重新调用 create_daily_plan。".to_owned()],
            ),
            StorageError::LockPoisoned => Self::new(
                "STORAGE_UNAVAILABLE",
                "计划存储暂时不可用。",
                Vec::new(),
                vec!["重启适配器后重试。".to_owned()],
            ),
            StorageError::InvalidRecord { .. }
            | StorageError::MismatchedPlanId
            | StorageError::InvalidNextRevision { .. } => Self::new(
                "PLAN_RECORD_INVALID",
                "计划记录损坏或不一致。",
                Vec::new(),
                vec!["保留状态目录并检查对应计划记录。".to_owned()],
            ),
            StorageError::Io { .. } => Self::new(
                "STORAGE_IO_ERROR",
                "无法安全读写计划存储。",
                Vec::new(),
                vec!["检查状态目录权限和可用空间后重试。".to_owned()],
            ),
        }
    }
}

pub type ToolResult<T> = Result<Json<T>, Json<ToolError>>;

pub fn parse_arguments<T: DeserializeOwned>(arguments: JsonObject) -> Result<T, Json<ToolError>> {
    serde_json::from_value(Value::Object(arguments))
        .map_err(|error| Json(ToolError::invalid_arguments(error)))
}

pub fn parse_plan_id(value: &str) -> Result<PlanId, Json<ToolError>> {
    PlanId::from_str(value).map_err(|_| Json(ToolError::invalid_plan_id()))
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub recovery: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanViewOutput {
    pub plan_id: String,
    pub profile: String,
    pub tasks: Vec<DailyTask>,
    pub state: PlanState,
    pub revision: u64,
    pub missing_fields: Vec<MissingFieldOutput>,
    pub validation_errors: Vec<ValidationErrorOutput>,
    pub next_question: Option<String>,
    pub summary: Option<String>,
    pub normalized_plan: Option<Value>,
    pub plan_hash: Option<String>,
}

impl PlanViewOutput {
    pub fn from_view(view: PlanView) -> Self {
        let plan_hash = view
            .normalized
            .as_ref()
            .and_then(|_| hash_plan(&view.plan).ok())
            .map(|hash| hash.as_str().to_owned());
        let normalized_plan = view
            .normalized
            .as_ref()
            .map(|plan| serde_json::to_value(plan).expect("canonical plan is serializable"));
        Self {
            plan_id: view.plan.plan_id.to_string(),
            profile: view.plan.profile,
            tasks: view.plan.tasks,
            state: view.plan.state,
            revision: view.plan.revision,
            missing_fields: view
                .missing_fields
                .into_iter()
                .map(MissingFieldOutput::from)
                .collect(),
            validation_errors: view
                .validation_errors
                .into_iter()
                .map(ValidationErrorOutput::from)
                .collect(),
            next_question: view.next_question,
            summary: view.summary,
            normalized_plan,
            plan_hash,
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MissingFieldOutput {
    pub field: String,
    pub task_index: Option<usize>,
    pub valid_options: Vec<String>,
}

impl From<MissingField> for MissingFieldOutput {
    fn from(field: MissingField) -> Self {
        let (name, task_index) = match field {
            MissingField::Profile => ("profile", None),
            MissingField::StartUpStartGame { task_index } => {
                ("start_up.start_game", Some(task_index))
            }
            MissingField::FightCategory { task_index } => ("fight.category", Some(task_index)),
            MissingField::FightStage { task_index } => ("fight.stage", Some(task_index)),
            MissingField::FightTimes { task_index } => ("fight.times", Some(task_index)),
            MissingField::FightMedicine { task_index } => ("fight.medicine", Some(task_index)),
            MissingField::RecruitMaxRecruits { task_index } => {
                ("recruit.max_recruits", Some(task_index))
            }
            MissingField::RecruitConfirmRecruitment { task_index } => {
                ("recruit.confirm_recruitment", Some(task_index))
            }
            MissingField::RecruitUseExpeditedPlans { task_index } => {
                ("recruit.use_expedited_plans", Some(task_index))
            }
            MissingField::MallVisitFriends { task_index } => {
                ("mall.visit_friends", Some(task_index))
            }
            MissingField::MallShopping { task_index } => ("mall.shopping", Some(task_index)),
            MissingField::AwardDaily { task_index } => ("award.daily", Some(task_index)),
            MissingField::AwardWeekly { task_index } => ("award.weekly", Some(task_index)),
            MissingField::AwardMail { task_index } => ("award.mail", Some(task_index)),
            MissingField::AwardEvent { task_index } => ("award.event", Some(task_index)),
        };
        Self {
            field: name.to_owned(),
            task_index,
            valid_options: field
                .valid_options()
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidationErrorOutput {
    pub code: String,
    pub task_index: Option<usize>,
    pub detail: Option<String>,
}

impl From<ValidationError> for ValidationErrorOutput {
    fn from(error: ValidationError) -> Self {
        match error {
            ValidationError::EmptyPlan => Self::new("EMPTY_PLAN", None, None),
            ValidationError::InvalidProfile => Self::new("INVALID_PROFILE", None, None),
            ValidationError::DuplicateTask { task } => Self::new(
                "DUPLICATE_TASK",
                None,
                Some(task_kind_name(task).to_owned()),
            ),
            ValidationError::DuplicateFight => Self::new("DUPLICATE_FIGHT", None, None),
            ValidationError::InvalidStage { task_index } => {
                Self::new("INVALID_STAGE", Some(task_index), None)
            }
            ValidationError::FightTimesOutOfRange { task_index, value } => Self::new(
                "FIGHT_TIMES_OUT_OF_RANGE",
                Some(task_index),
                Some(value.to_string()),
            ),
            ValidationError::MedicineOutOfRange { task_index, value } => Self::new(
                "MEDICINE_OUT_OF_RANGE",
                Some(task_index),
                Some(value.to_string()),
            ),
            ValidationError::RecruitCountOutOfRange { task_index, value } => Self::new(
                "RECRUIT_COUNT_OUT_OF_RANGE",
                Some(task_index),
                Some(value.to_string()),
            ),
        }
    }
}

impl ValidationErrorOutput {
    fn new(code: &str, task_index: Option<usize>, detail: Option<String>) -> Self {
        Self {
            code: code.to_owned(),
            task_index,
            detail,
        }
    }
}

fn state_name(state: PlanState) -> &'static str {
    match state {
        PlanState::Draft => "draft",
        PlanState::Ready => "ready",
        PlanState::Confirmed => "confirmed",
        PlanState::Running => "running",
        PlanState::Succeeded => "succeeded",
        PlanState::Failed => "failed",
        PlanState::Stopped => "stopped",
        PlanState::Cancelled => "cancelled",
    }
}

fn task_kind_name(task: TaskKind) -> &'static str {
    match task {
        TaskKind::StartUp => "start_up",
        TaskKind::Fight => "fight",
        TaskKind::Recruit => "recruit",
        TaskKind::Infrast => "infrast",
        TaskKind::Mall => "mall",
        TaskKind::Award => "award",
        TaskKind::CloseDown => "close_down",
    }
}

pub(crate) fn path_search_value() -> Option<OsString> {
    std::env::var_os("PATH")
}

pub type SharedRuntime = Arc<AdapterRuntime>;
