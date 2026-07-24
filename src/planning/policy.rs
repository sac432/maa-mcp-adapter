use std::collections::BTreeSet;

use crate::domain::DailyPlan;
use crate::domain::DailyTask;
use crate::domain::PlanState;

use super::{DomainError, MissingField, TaskKind, ValidationError, ValidationReport};

pub fn validate_plan(plan: &DailyPlan) -> ValidationReport {
    let mut report = ValidationReport::default();
    if plan.profile.trim().is_empty() {
        report.missing_fields.push(MissingField::Profile);
    } else if !is_valid_profile(&plan.profile) {
        report.errors.push(ValidationError::InvalidProfile);
    }
    if plan.tasks.is_empty() {
        report.errors.push(ValidationError::EmptyPlan);
    }

    let mut seen_singletons = BTreeSet::new();
    let mut duplicate_singletons = BTreeSet::new();
    let mut seen_fights = Vec::new();
    let mut duplicate_fight = false;

    for (task_index, task) in plan.tasks.iter().enumerate() {
        let task_kind = task_kind(task);
        if task_kind == TaskKind::Fight {
            if let DailyTask::Fight(fight) = task {
                let fight_target = (fight.category, fight.stage.as_deref());
                if seen_fights.contains(&fight_target) {
                    duplicate_fight = true;
                } else {
                    seen_fights.push(fight_target);
                }
            }
        } else if !seen_singletons.insert(task_kind) {
            duplicate_singletons.insert(task_kind);
        }

        match task {
            DailyTask::StartUp(start_up) => {
                if start_up.start_game.is_none() {
                    report
                        .missing_fields
                        .push(MissingField::StartUpStartGame { task_index });
                }
            }
            DailyTask::Fight(fight) => {
                if fight.category.is_none() {
                    report
                        .missing_fields
                        .push(MissingField::FightCategory { task_index });
                }
                match fight.stage.as_deref() {
                    None => report
                        .missing_fields
                        .push(MissingField::FightStage { task_index }),
                    Some(stage) if !is_valid_stage(stage) => report
                        .errors
                        .push(ValidationError::InvalidStage { task_index }),
                    Some(_) => {}
                }
                match fight.times {
                    None => report
                        .missing_fields
                        .push(MissingField::FightTimes { task_index }),
                    Some(value) if !(1..=100).contains(&value) => {
                        report
                            .errors
                            .push(ValidationError::FightTimesOutOfRange { task_index, value });
                    }
                    Some(_) => {}
                }
                match fight.medicine {
                    None => report
                        .missing_fields
                        .push(MissingField::FightMedicine { task_index }),
                    Some(value) if value > 10 => {
                        report
                            .errors
                            .push(ValidationError::MedicineOutOfRange { task_index, value });
                    }
                    Some(_) => {}
                }
            }
            DailyTask::Recruit(recruit) => {
                match recruit.max_recruits {
                    None => report
                        .missing_fields
                        .push(MissingField::RecruitMaxRecruits { task_index }),
                    Some(value) if value > 4 => {
                        report
                            .errors
                            .push(ValidationError::RecruitCountOutOfRange { task_index, value });
                    }
                    Some(_) => {}
                }
                if recruit.confirm_recruitment.is_none() {
                    report
                        .missing_fields
                        .push(MissingField::RecruitConfirmRecruitment { task_index });
                }
                if recruit.use_expedited_plans.is_none() {
                    report
                        .missing_fields
                        .push(MissingField::RecruitUseExpeditedPlans { task_index });
                }
            }
            DailyTask::Mall(mall) => {
                if mall.visit_friends.is_none() {
                    report
                        .missing_fields
                        .push(MissingField::MallVisitFriends { task_index });
                }
                if mall.shopping.is_none() {
                    report
                        .missing_fields
                        .push(MissingField::MallShopping { task_index });
                }
            }
            DailyTask::Award(award) => {
                if award.daily.is_none() {
                    report
                        .missing_fields
                        .push(MissingField::AwardDaily { task_index });
                }
                if award.weekly.is_none() {
                    report
                        .missing_fields
                        .push(MissingField::AwardWeekly { task_index });
                }
                if award.mail.is_none() {
                    report
                        .missing_fields
                        .push(MissingField::AwardMail { task_index });
                }
                if award.event.is_none() {
                    report
                        .missing_fields
                        .push(MissingField::AwardEvent { task_index });
                }
            }
            DailyTask::Infrast(_) | DailyTask::CloseDown(_) => {}
        }
    }

    report.errors.extend(
        duplicate_singletons
            .into_iter()
            .map(|task| ValidationError::DuplicateTask { task }),
    );
    if duplicate_fight {
        report.errors.push(ValidationError::DuplicateFight);
    }

    report
}

pub fn validate_transition(from: PlanState, to: PlanState) -> Result<(), DomainError> {
    let allowed = matches!(
        (from, to),
        (PlanState::Draft, PlanState::Ready)
            | (PlanState::Ready, PlanState::Confirmed)
            | (PlanState::Confirmed, PlanState::Ready)
            | (PlanState::Confirmed, PlanState::Running)
            | (PlanState::Running, PlanState::Succeeded)
            | (PlanState::Running, PlanState::Failed)
            | (PlanState::Running, PlanState::Stopped)
            | (PlanState::Draft, PlanState::Cancelled)
            | (PlanState::Ready, PlanState::Cancelled)
            | (PlanState::Confirmed, PlanState::Cancelled)
    );

    if allowed {
        Ok(())
    } else {
        Err(DomainError::InvalidTransition { from, to })
    }
}

fn task_kind(task: &DailyTask) -> TaskKind {
    match task {
        DailyTask::StartUp(_) => TaskKind::StartUp,
        DailyTask::Fight(_) => TaskKind::Fight,
        DailyTask::Recruit(_) => TaskKind::Recruit,
        DailyTask::Infrast(_) => TaskKind::Infrast,
        DailyTask::Mall(_) => TaskKind::Mall,
        DailyTask::Award(_) => TaskKind::Award,
        DailyTask::CloseDown(_) => TaskKind::CloseDown,
    }
}

fn is_valid_stage(stage: &str) -> bool {
    (1..=32).contains(&stage.len())
        && stage
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn is_valid_profile(profile: &str) -> bool {
    profile.trim() == profile
        && profile != "."
        && profile != ".."
        && !profile.chars().any(char::is_control)
}
