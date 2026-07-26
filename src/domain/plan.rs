use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use super::{DailyTask, PlanState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlanId(Uuid);

impl PlanId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Display for PlanId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, formatter)
    }
}

impl FromStr for PlanId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

impl Default for PlanId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PlanHash(String);

impl PlanHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn from_hex(value: String) -> Self {
        Self(value)
    }
}

impl<'de> Deserialize<'de> for PlanHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if super::is_sha256_hex(&value) {
            Ok(Self(value))
        } else {
            Err(D::Error::custom(
                "plan hash must be 64 lowercase hexadecimal characters",
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyPlan {
    pub plan_id: PlanId,
    pub profile: String,
    pub tasks: Vec<DailyTask>,
    pub state: PlanState,
    pub revision: u64,
    pub confirmed_hash: Option<PlanHash>,
}

impl DailyPlan {
    pub fn new(profile: impl Into<String>, tasks: Vec<DailyTask>) -> Self {
        Self {
            plan_id: PlanId::new(),
            profile: profile.into(),
            tasks,
            state: PlanState::Draft,
            revision: 0,
            confirmed_hash: None,
        }
    }
}
