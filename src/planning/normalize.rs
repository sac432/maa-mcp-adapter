use std::cmp::Ordering;

use serde::Serialize;

use crate::domain::{DailyPlan, DailyTask, FightCategory};

use super::DomainError;
use super::policy::validate_plan;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CanonicalPlan {
    pub schema_version: u8,
    pub profile: String,
    pub tasks: Vec<CanonicalTask>,
    pub safety: CanonicalSafety,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CanonicalSafety {
    pub account_switching: bool,
    pub paid_purchases: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "params", rename_all = "snake_case")]
pub enum CanonicalTask {
    StartUp {
        start_game: bool,
    },
    Fight {
        category: FightCategory,
        stage: String,
        times: u32,
        medicine: u8,
        stone: u8,
    },
    Recruit {
        max_recruits: u8,
        confirm_recruitment: bool,
        use_expedited_plans: bool,
    },
    Infrast {},
    Mall {
        visit_friends: bool,
        shopping: bool,
        force_shopping_if_credit_full: bool,
        preserve_blacklist: bool,
    },
    Award {
        daily: bool,
        weekly: bool,
        mail: bool,
        event: bool,
    },
    CloseDown {},
}

pub fn normalize_plan(plan: &DailyPlan) -> Result<CanonicalPlan, DomainError> {
    let report = validate_plan(plan);
    if !report.is_valid() {
        return Err(DomainError::InvalidPlan(report));
    }

    let mut tasks = plan.tasks.iter().collect::<Vec<_>>();
    tasks.sort_by(|left, right| compare_tasks(left, right));
    let tasks = tasks.into_iter().map(canonical_task).collect();

    Ok(CanonicalPlan {
        schema_version: 1,
        profile: plan.profile.clone(),
        tasks,
        safety: CanonicalSafety {
            account_switching: false,
            paid_purchases: false,
        },
    })
}

fn canonical_task(task: &DailyTask) -> CanonicalTask {
    match task {
        DailyTask::StartUp(plan) => CanonicalTask::StartUp {
            start_game: plan.start_game.expect("validated StartUp"),
        },
        DailyTask::Fight(plan) => CanonicalTask::Fight {
            category: plan.category.expect("validated Fight category"),
            stage: plan.stage.clone().expect("validated Fight stage"),
            times: plan.times.expect("validated Fight times"),
            medicine: plan.medicine.expect("validated Fight medicine"),
            stone: 0,
        },
        DailyTask::Recruit(plan) => CanonicalTask::Recruit {
            max_recruits: plan.max_recruits.expect("validated Recruit count"),
            confirm_recruitment: plan
                .confirm_recruitment
                .expect("validated Recruit confirmation"),
            use_expedited_plans: plan
                .use_expedited_plans
                .expect("validated Recruit expedited plans"),
        },
        DailyTask::Infrast(_) => CanonicalTask::Infrast {},
        DailyTask::Mall(plan) => CanonicalTask::Mall {
            visit_friends: plan.visit_friends.expect("validated Mall friend visit"),
            shopping: plan.shopping.expect("validated Mall shopping"),
            force_shopping_if_credit_full: false,
            preserve_blacklist: true,
        },
        DailyTask::Award(plan) => CanonicalTask::Award {
            daily: plan.daily.expect("validated Award daily"),
            weekly: plan.weekly.expect("validated Award weekly"),
            mail: plan.mail.expect("validated Award mail"),
            event: plan.event.expect("validated Award event"),
        },
        DailyTask::CloseDown(_) => CanonicalTask::CloseDown {},
    }
}

fn compare_tasks(left: &DailyTask, right: &DailyTask) -> Ordering {
    let rank_order = task_rank(left).cmp(&task_rank(right));
    if rank_order != Ordering::Equal {
        return rank_order;
    }

    match (left, right) {
        (DailyTask::Fight(left), DailyTask::Fight(right)) => left
            .category
            .cmp(&right.category)
            .then_with(|| left.stage.cmp(&right.stage))
            .then_with(|| left.times.cmp(&right.times))
            .then_with(|| left.medicine.cmp(&right.medicine)),
        _ => Ordering::Equal,
    }
}

fn task_rank(task: &DailyTask) -> u8 {
    match task {
        DailyTask::StartUp(_) => 0,
        DailyTask::Fight(_) => 1,
        DailyTask::Recruit(_) => 2,
        DailyTask::Infrast(_) => 3,
        DailyTask::Mall(_) => 4,
        DailyTask::Award(_) => 5,
        DailyTask::CloseDown(_) => 6,
    }
}
