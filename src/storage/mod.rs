use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

mod atomic;
mod repository;

pub use repository::{FilePlanRepository, PlanRepository};

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
    #[error("plan record is invalid: {path}")]
    InvalidRecord {
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
