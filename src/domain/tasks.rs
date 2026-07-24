use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DailyTask {
    StartUp(StartUpPlan),
    Fight(FightPlan),
    Recruit(RecruitPlan),
    Infrast(InfrastPlan),
    Mall(MallPlan),
    Award(AwardPlan),
    CloseDown(CloseDownPlan),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FightCategory {
    Main,
    Resource,
    Event,
    Annihilation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartUpPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_game: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FightPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<FightCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub times: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medicine: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecruitPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_recruits: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_recruitment: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_expedited_plans: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InfrastPlan {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MallPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visit_friends: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shopping: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AwardPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseDownPlan {}
