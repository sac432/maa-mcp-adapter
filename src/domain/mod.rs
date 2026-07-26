use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod plan;
mod run;
mod tasks;

pub use plan::*;
pub use run::*;
pub use tasks::*;

/// 全项目统一的 SHA-256 十六进制形态校验（64 位小写）。
pub(crate) fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

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
