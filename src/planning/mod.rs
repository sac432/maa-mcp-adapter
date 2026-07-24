use thiserror::Error;

use crate::domain::PlanState;

pub mod hash;
pub mod lifecycle;
pub mod normalize;
pub mod policy;
pub mod summary;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("invalid plan state transition: {from:?} -> {to:?}")]
    InvalidTransition { from: PlanState, to: PlanState },
    #[error("plan is incomplete or unsafe")]
    InvalidPlan(ValidationReport),
    #[error("plan in state {state:?} cannot be modified")]
    PlanNotEditable { state: PlanState },
    #[error("plan revision overflow")]
    RevisionOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskKind {
    StartUp,
    Fight,
    Recruit,
    Infrast,
    Mall,
    Award,
    CloseDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingField {
    Profile,
    StartUpStartGame { task_index: usize },
    FightCategory { task_index: usize },
    FightStage { task_index: usize },
    FightTimes { task_index: usize },
    FightMedicine { task_index: usize },
    RecruitMaxRecruits { task_index: usize },
    RecruitConfirmRecruitment { task_index: usize },
    RecruitUseExpeditedPlans { task_index: usize },
    MallVisitFriends { task_index: usize },
    MallShopping { task_index: usize },
    AwardDaily { task_index: usize },
    AwardWeekly { task_index: usize },
    AwardMail { task_index: usize },
    AwardEvent { task_index: usize },
}

impl MissingField {
    pub fn valid_options(self) -> &'static [&'static str] {
        match self {
            Self::Profile => &["<available-profile>"],
            Self::StartUpStartGame { .. }
            | Self::RecruitConfirmRecruitment { .. }
            | Self::RecruitUseExpeditedPlans { .. }
            | Self::MallVisitFriends { .. }
            | Self::MallShopping { .. }
            | Self::AwardDaily { .. }
            | Self::AwardWeekly { .. }
            | Self::AwardMail { .. }
            | Self::AwardEvent { .. } => &["false", "true"],
            Self::FightCategory { .. } => &["main", "resource", "event", "annihilation"],
            Self::FightStage { .. } => &["<stage-code>"],
            Self::FightTimes { .. } => &["1..=100"],
            Self::FightMedicine { .. } => &["0..=10"],
            Self::RecruitMaxRecruits { .. } => &["0..=4"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    EmptyPlan,
    InvalidProfile,
    DuplicateTask { task: TaskKind },
    DuplicateFight,
    InvalidStage { task_index: usize },
    FightTimesOutOfRange { task_index: usize, value: u32 },
    MedicineOutOfRange { task_index: usize, value: u8 },
    RecruitCountOutOfRange { task_index: usize, value: u8 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    pub missing_fields: Vec<MissingField>,
    pub errors: Vec<ValidationError>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.missing_fields.is_empty() && self.errors.is_empty()
    }
}
