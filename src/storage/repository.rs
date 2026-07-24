use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use crate::domain::{DailyPlan, PlanId, PlanState};

use super::atomic::{WriteMode, write_atomically};
use super::{StorageError, resolve_state_directory};

pub trait PlanRepository {
    fn create(&self, plan: &DailyPlan) -> Result<(), StorageError>;
    fn load(&self, id: &PlanId) -> Result<DailyPlan, StorageError>;
    fn replace(&self, expected_revision: u64, plan: &DailyPlan) -> Result<(), StorageError>;
}

pub struct FilePlanRepository {
    plans_directory: PathBuf,
}

static REPOSITORY_GATE: Mutex<()> = Mutex::new(());
const CANCELLED_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

impl FilePlanRepository {
    pub fn new(state_directory: PathBuf) -> Result<Self, StorageError> {
        let state_directory = resolve_state_directory(Some(&state_directory), None)?;
        let plans_directory = state_directory.join("plans");
        fs::create_dir_all(&plans_directory).map_err(|source| StorageError::Io {
            path: plans_directory.clone(),
            source,
        })?;
        Ok(Self { plans_directory })
    }

    fn plan_path(&self, id: &PlanId) -> PathBuf {
        self.plans_directory.join(format!("{id}.json"))
    }

    fn load_unlocked(&self, id: &PlanId) -> Result<DailyPlan, StorageError> {
        let path = self.plan_path(id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(StorageError::NotFound);
            }
            Err(source) => return Err(StorageError::Io { path, source }),
        };
        let plan: DailyPlan =
            serde_json::from_slice(&bytes).map_err(|source| StorageError::InvalidRecord {
                path: path.clone(),
                source,
            })?;
        if &plan.plan_id != id {
            return Err(StorageError::MismatchedPlanId);
        }
        Ok(plan)
    }

    fn serialized(plan: &DailyPlan) -> Vec<u8> {
        serde_json::to_vec_pretty(plan)
            .expect("DailyPlan contains only values supported by JSON serialization")
    }

    pub fn cleanup_cancelled(&self, now: SystemTime) -> Result<usize, StorageError> {
        let cutoff = now.checked_sub(CANCELLED_RETENTION).unwrap_or(now);
        let _guard = REPOSITORY_GATE
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let entries = fs::read_dir(&self.plans_directory).map_err(|source| StorageError::Io {
            path: self.plans_directory.clone(),
            source,
        })?;
        let mut removed = 0;

        for entry in entries {
            let entry = entry.map_err(|source| StorageError::Io {
                path: self.plans_directory.clone(),
                source,
            })?;
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if !file_type.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Some(plan_id) = path
                .file_stem()
                .and_then(|value| value.to_str())
                .and_then(|value| PlanId::from_str(value).ok())
            else {
                continue;
            };
            let Ok(plan) = self.load_unlocked(&plan_id) else {
                continue;
            };
            if plan.state != PlanState::Cancelled {
                continue;
            }
            let modified = match entry.metadata().and_then(|metadata| metadata.modified()) {
                Ok(modified) => modified,
                Err(_) => continue,
            };
            if modified >= cutoff {
                continue;
            }
            fs::remove_file(&path).map_err(|source| StorageError::Io { path, source })?;
            removed += 1;
        }

        Ok(removed)
    }
}

impl PlanRepository for FilePlanRepository {
    fn create(&self, plan: &DailyPlan) -> Result<(), StorageError> {
        let _guard = REPOSITORY_GATE
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let path = self.plan_path(&plan.plan_id);
        if path.exists() {
            return Err(StorageError::AlreadyExists);
        }
        write_atomically(&path, &Self::serialized(plan), WriteMode::Create)
    }

    fn load(&self, id: &PlanId) -> Result<DailyPlan, StorageError> {
        let _guard = REPOSITORY_GATE
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        self.load_unlocked(id)
    }

    fn replace(&self, expected_revision: u64, plan: &DailyPlan) -> Result<(), StorageError> {
        let _guard = REPOSITORY_GATE
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let current = self.load_unlocked(&plan.plan_id)?;
        if current.revision != expected_revision {
            return Err(StorageError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        let next_revision =
            expected_revision
                .checked_add(1)
                .ok_or(StorageError::InvalidNextRevision {
                    expected: expected_revision,
                    actual: plan.revision,
                })?;
        if plan.revision != next_revision {
            return Err(StorageError::InvalidNextRevision {
                expected: next_revision,
                actual: plan.revision,
            });
        }
        if current.plan_id != plan.plan_id {
            return Err(StorageError::MismatchedPlanId);
        }

        let path = self.plan_path(&plan.plan_id);
        write_atomically(&path, &Self::serialized(plan), WriteMode::Replace)
    }
}
