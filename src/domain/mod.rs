use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod plan;
mod run;
mod tasks;

pub use plan::*;
pub use run::*;
pub use tasks::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanState {
    Draft,
    Ready,
    Confirmed,
    Running,
    Succeeded,
    Failed,
    Stopped,
    Cancelled,
}
