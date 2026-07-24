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
    let previous_state = plan.state;

    plan.profile = profile;
    plan.tasks = tasks;
    plan.revision = revision;
    plan.confirmed_hash = None;
    let complete = validate_plan(plan).is_valid();
    plan.state = match previous_state {
        PlanState::Draft => PlanState::Draft,
        PlanState::Ready | PlanState::Confirmed if complete => PlanState::Ready,
        PlanState::Ready | PlanState::Confirmed => PlanState::Draft,
        _ => unreachable!("non-editable states returned before mutation"),
    };

    Ok(())
}
