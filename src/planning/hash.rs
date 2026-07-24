use std::fmt::Write;

use sha2::{Digest, Sha256};

use crate::domain::{DailyPlan, PlanHash};

use super::DomainError;
use super::normalize::normalize_plan;

pub fn hash_plan(plan: &DailyPlan) -> Result<PlanHash, DomainError> {
    let normalized = normalize_plan(plan)?;
    let bytes = serde_json::to_vec(&normalized)
        .expect("canonical daily plan contains only serializable values");
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(PlanHash::from_hex(hex))
}
