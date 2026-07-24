use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::{DailyPlan, DailyTask, PlanId, PlanState};
use crate::storage::{PlanRepository, StorageError};

use super::lifecycle::revise_plan;
use super::normalize::{CanonicalPlan, normalize_plan};
use super::policy::validate_plan;
use super::summary::summarize_plan;
use super::{DomainError, MissingField, ValidationError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanPatch {
    SetProfile { profile: String },
    AddTask { task: DailyTask },
    ReplaceTask { task_index: usize, task: DailyTask },
    RemoveTask { task_index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanView {
    pub plan: DailyPlan,
    pub normalized: Option<CanonicalPlan>,
    pub summary: Option<String>,
    pub missing_fields: Vec<MissingField>,
    pub validation_errors: Vec<ValidationError>,
    pub next_question: Option<String>,
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("task index {task_index} is out of range")]
    InvalidTaskIndex { task_index: usize },
    #[error("plan in state {state:?} cannot be previewed")]
    CannotPreview { state: PlanState },
    #[error("plan in state {state:?} cannot be cancelled")]
    CannotCancel { state: PlanState },
}

pub struct PlanService<R: PlanRepository> {
    repository: R,
}

impl<R: PlanRepository> PlanService<R> {
    pub fn new(repository: R) -> Self {
        Self { repository }
    }

    pub fn create(
        &self,
        profile: impl Into<String>,
        tasks: Vec<DailyTask>,
    ) -> Result<PlanView, ServiceError> {
        let plan = DailyPlan::new(profile, tasks);
        self.repository.create(&plan)?;
        Self::view(plan)
    }

    pub fn update(
        &self,
        plan_id: &PlanId,
        expected_revision: u64,
        patch: PlanPatch,
    ) -> Result<PlanView, ServiceError> {
        let mut plan = self.repository.load(plan_id)?;
        Self::require_revision(expected_revision, plan.revision)?;
        let mut profile = plan.profile.clone();
        let mut tasks = plan.tasks.clone();

        match patch {
            PlanPatch::SetProfile { profile: value } => profile = value,
            PlanPatch::AddTask { task } => tasks.push(task),
            PlanPatch::ReplaceTask { task_index, task } => {
                let target = tasks
                    .get_mut(task_index)
                    .ok_or(ServiceError::InvalidTaskIndex { task_index })?;
                *target = task;
            }
            PlanPatch::RemoveTask { task_index } => {
                if task_index >= tasks.len() {
                    return Err(ServiceError::InvalidTaskIndex { task_index });
                }
                tasks.remove(task_index);
            }
        }

        revise_plan(&mut plan, profile, tasks)?;
        self.repository.replace(expected_revision, &plan)?;
        Self::view(plan)
    }

    pub fn preview(&self, plan_id: &PlanId) -> Result<PlanView, ServiceError> {
        let mut plan = self.repository.load(plan_id)?;
        if matches!(
            plan.state,
            PlanState::Running
                | PlanState::Succeeded
                | PlanState::Failed
                | PlanState::Stopped
                | PlanState::Cancelled
        ) {
            return Err(ServiceError::CannotPreview { state: plan.state });
        }
        if !validate_plan(&plan).is_valid() {
            return Self::view(plan);
        }
        if plan.state == PlanState::Draft {
            let expected_revision = plan.revision;
            plan.revision = plan
                .revision
                .checked_add(1)
                .ok_or(DomainError::RevisionOverflow)?;
            plan.state = PlanState::Ready;
            plan.confirmed_hash = None;
            self.repository.replace(expected_revision, &plan)?;
        }
        Self::view(plan)
    }

    pub fn cancel(
        &self,
        plan_id: &PlanId,
        expected_revision: u64,
    ) -> Result<PlanView, ServiceError> {
        let mut plan = self.repository.load(plan_id)?;
        Self::require_revision(expected_revision, plan.revision)?;
        if !matches!(
            plan.state,
            PlanState::Draft | PlanState::Ready | PlanState::Confirmed
        ) {
            return Err(ServiceError::CannotCancel { state: plan.state });
        }
        plan.revision = plan
            .revision
            .checked_add(1)
            .ok_or(DomainError::RevisionOverflow)?;
        plan.state = PlanState::Cancelled;
        plan.confirmed_hash = None;
        self.repository.replace(expected_revision, &plan)?;
        Self::view(plan)
    }

    fn require_revision(expected: u64, actual: u64) -> Result<(), ServiceError> {
        if expected == actual {
            Ok(())
        } else {
            Err(StorageError::RevisionConflict { expected, actual }.into())
        }
    }

    fn view(plan: DailyPlan) -> Result<PlanView, ServiceError> {
        let report = validate_plan(&plan);
        let next_question = report.missing_fields.first().map(question_for);
        let (normalized, summary) = if report.is_valid() {
            (Some(normalize_plan(&plan)?), Some(summarize_plan(&plan)?))
        } else {
            (None, None)
        };

        Ok(PlanView {
            plan,
            normalized,
            summary,
            missing_fields: report.missing_fields,
            validation_errors: report.errors,
            next_question,
        })
    }
}

fn question_for(field: &MissingField) -> String {
    match field {
        MissingField::Profile => "请选择要使用的配置档。".to_owned(),
        MissingField::StartUpStartGame { .. } => "是否启动游戏？".to_owned(),
        MissingField::FightCategory { .. } => "要刷主线、资源关、活动还是剿灭？".to_owned(),
        MissingField::FightStage { .. } => "要刷哪个关卡？".to_owned(),
        MissingField::FightTimes { .. } => "要刷几次（1 到 100）？".to_owned(),
        MissingField::FightMedicine { .. } => "最多使用几瓶理智药（0 到 10）？".to_owned(),
        MissingField::RecruitMaxRecruits { .. } => "最多执行几次公开招募（0 到 4）？".to_owned(),
        MissingField::RecruitConfirmRecruitment { .. } => "是否确认招募并消耗招募许可？".to_owned(),
        MissingField::RecruitUseExpeditedPlans { .. } => "是否使用加急许可？".to_owned(),
        MissingField::MallVisitFriends { .. } => "是否访问好友？".to_owned(),
        MissingField::MallShopping { .. } => "是否购买信用商店物品？".to_owned(),
        MissingField::AwardDaily { .. } => "是否领取日常奖励？".to_owned(),
        MissingField::AwardWeekly { .. } => "是否领取周常奖励？".to_owned(),
        MissingField::AwardMail { .. } => "是否领取邮件？".to_owned(),
        MissingField::AwardEvent { .. } => "是否领取活动奖励？".to_owned(),
    }
}
