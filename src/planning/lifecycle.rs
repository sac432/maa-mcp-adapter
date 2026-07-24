use crate::domain::{DailyPlan, DailyTask, PlanState};

use super::DomainError;
use super::policy::validate_plan;

pub fn revise_plan(
    plan: &mut DailyPlan,
    profile: String,
    tasks: Vec<DailyTask>,
) -> Result<(), DomainError> {
    if matches!(
        plan.state,
        PlanState::Running
            | PlanState::Succeeded
            | PlanState::Failed
            | PlanState::Stopped
            | PlanState::Cancelled
    ) {
        return Err(DomainError::PlanNotEditable { state: plan.state });
    }
    let revision = plan
        .revision
        .checked_add(1)
        .ok_or(DomainError::RevisionOverflow)?;

    plan.profile = profile;
    plan.tasks = tasks;
    plan.revision = revision;
    plan.confirmed_hash = None;
    plan.state = if validate_plan(plan).is_valid() {
        PlanState::Ready
    } else {
        PlanState::Draft
    };

    Ok(())
}
