use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

pub(crate) mod atomic;
mod repository;
mod runs;

pub use repository::{FilePlanRepository, PlanRepository};
pub use runs::{
    ActiveRunLock, FileRunRepository, RunRepository, active_run_id, release_active_run_lock,
};

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("state directory must be an absolute path without traversal components: {0}")]
    InvalidStateDirectory(PathBuf),
    #[error("LOCALAPPDATA is unavailable and no state directory was configured")]
    MissingLocalAppData,
    #[error("plan already exists")]
    AlreadyExists,
    #[error("plan was not found")]
    NotFound,
    #[error("plan revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("replacement revision must be {expected}, got {actual}")]
    InvalidNextRevision { expected: u64, actual: u64 },
    #[error("replacement plan id does not match the stored plan")]
    MismatchedPlanId,
    #[error("plan repository lock is poisoned")]
    LockPoisoned,
    #[error("run directory does not exist")]
    RunDirectoryMissing,
    #[error("run record was not found")]
    RunNotFound,
    #[error("run record id does not match its path")]
    MismatchedRunId,
    #[error("run revision conflict: expected {expected}, actual {actual}")]
    RunRevisionConflict { expected: u64, actual: u64 },
    #[error("replacement run revision must be {expected}, got {actual}")]
    InvalidNextRunRevision { expected: u64, actual: u64 },
    #[error("run task hash must be 64 lowercase hexadecimal characters")]
    InvalidTaskHash,
    #[error("an active run lock already exists")]
    ActiveRunExists,
    #[error("active run lock owner does not match")]
    ActiveRunOwnerMismatch,
    #[error("active run lock is invalid")]
    InvalidActiveRunLock,
    #[error("plan record is invalid: {path}")]
    InvalidRecord {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("run record is invalid: {path}")]
    InvalidRunRecord {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("storage operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub fn resolve_state_directory(
    configured: Option<&Path>,
    local_app_data: Option<&OsStr>,
) -> Result<PathBuf, StorageError> {
    let directory = match configured {
        Some(directory) => directory.to_path_buf(),
        None => PathBuf::from(local_app_data.ok_or(StorageError::MissingLocalAppData)?)
            .join("maa-mcp-adapter"),
    };
    let has_unsafe_component = directory
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir));
    if !directory.is_absolute() || has_unsafe_component {
        return Err(StorageError::InvalidStateDirectory(directory));
    }
    Ok(directory)
}
