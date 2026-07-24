use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::domain::{RunId, RunRecord};

use super::atomic::{WriteMode, write_atomically};
use super::{StorageError, resolve_state_directory};

pub trait RunRepository {
    fn create(&self, record: &RunRecord) -> Result<(), StorageError>;
    fn load(&self, run_id: &RunId) -> Result<RunRecord, StorageError>;
}

pub struct FileRunRepository {
    runs_directory: PathBuf,
}

impl FileRunRepository {
    pub fn new(state_directory: PathBuf) -> Result<Self, StorageError> {
        let state_directory = resolve_state_directory(Some(&state_directory), None)?;
        let runs_directory = state_directory.join("runs");
        fs::create_dir_all(&runs_directory).map_err(|source| StorageError::Io {
            path: runs_directory.clone(),
            source,
        })?;
        Ok(Self { runs_directory })
    }

    fn record_path(&self, run_id: &RunId) -> PathBuf {
        self.runs_directory
            .join(run_id.to_string())
            .join("record.json")
    }
}

impl RunRepository for FileRunRepository {
    fn create(&self, record: &RunRecord) -> Result<(), StorageError> {
        validate_task_hash(&record.task_sha256)?;
        let path = self.record_path(&record.run_id);
        let parent = path.parent().expect("record path has a run directory");
        if !parent.is_dir() {
            return Err(StorageError::RunDirectoryMissing);
        }
        let bytes = serde_json::to_vec_pretty(record).expect("run record is JSON serializable");
        write_atomically(&path, &bytes, WriteMode::Create)
    }

    fn load(&self, run_id: &RunId) -> Result<RunRecord, StorageError> {
        let path = self.record_path(run_id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(StorageError::RunNotFound);
            }
            Err(source) => return Err(StorageError::Io { path, source }),
        };
        let record: RunRecord =
            serde_json::from_slice(&bytes).map_err(|source| StorageError::InvalidRunRecord {
                path: path.clone(),
                source,
            })?;
        if &record.run_id != run_id {
            return Err(StorageError::MismatchedRunId);
        }
        validate_task_hash(&record.task_sha256)?;
        Ok(record)
    }
}

pub struct ActiveRunLock {
    path: PathBuf,
    run_id: RunId,
    committed: bool,
}

impl ActiveRunLock {
    pub fn acquire(state_directory: &Path, run_id: RunId) -> Result<Self, StorageError> {
        let state_directory = resolve_state_directory(Some(state_directory), None)?;
        let locks_directory = state_directory.join("locks");
        fs::create_dir_all(&locks_directory).map_err(|source| StorageError::Io {
            path: locks_directory.clone(),
            source,
        })?;
        let path = locks_directory.join("active-run.lock");
        let mut file = match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(StorageError::ActiveRunExists);
            }
            Err(source) => return Err(StorageError::Io { path, source }),
        };
        let result = (|| {
            file.write_all(run_id.to_string().as_bytes())
                .map_err(|source| StorageError::Io {
                    path: path.clone(),
                    source,
                })?;
            file.flush().map_err(|source| StorageError::Io {
                path: path.clone(),
                source,
            })?;
            file.sync_all().map_err(|source| StorageError::Io {
                path: path.clone(),
                source,
            })
        })();
        if let Err(error) = result {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(Self {
            path,
            run_id,
            committed: false,
        })
    }

    pub fn commit(mut self) -> Result<(), StorageError> {
        verify_active_run_owner(&self.path, &self.run_id)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for ActiveRunLock {
    fn drop(&mut self) {
        if !self.committed {
            let _ = release_lock_path(&self.path, &self.run_id);
        }
    }
}

pub fn release_active_run_lock(state_directory: &Path, run_id: &RunId) -> Result<(), StorageError> {
    let state_directory = resolve_state_directory(Some(state_directory), None)?;
    release_lock_path(
        &state_directory.join("locks").join("active-run.lock"),
        run_id,
    )
}

fn release_lock_path(path: &Path, run_id: &RunId) -> Result<(), StorageError> {
    verify_active_run_owner(path, run_id)?;
    fs::remove_file(path).map_err(|source| StorageError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn verify_active_run_owner(path: &Path, run_id: &RunId) -> Result<(), StorageError> {
    let contents = fs::read_to_string(path).map_err(|source| StorageError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let owner = RunId::from_str(contents.trim()).map_err(|_| StorageError::InvalidActiveRunLock)?;
    if &owner == run_id {
        Ok(())
    } else {
        Err(StorageError::ActiveRunOwnerMismatch)
    }
}

fn validate_task_hash(value: &str) -> Result<(), StorageError> {
    let valid = value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(StorageError::InvalidTaskHash)
    }
}
