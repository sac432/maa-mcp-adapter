use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::domain::{PlanState, RunId, RunRecord, RunStatus, StopMethod};
use crate::maa_cli::process::BackgroundProcess;
use crate::platform::{
    PlatformError, process_identity_matches, request_graceful_stop, terminate_job, wait_for_exit,
};
use crate::storage::atomic::{WriteMode, write_atomically};
use crate::storage::{
    FilePlanRepository, FileRunRepository, PlanRepository, RunRepository, StorageError,
    active_run_id, release_active_run_lock,
};

const MAX_LOG_BYTES: u64 = 64 * 1024;
const MAX_LOG_LINES: usize = 200;
static MONITORED_RUNS: LazyLock<Mutex<HashSet<RunId>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogTail {
    pub content: String,
    pub lines: Vec<String>,
    pub bytes: usize,
    pub truncated: bool,
}

#[derive(Debug, Error)]
pub enum RunControlError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("recorded log path is outside the selected run directory")]
    UnsafeLogPath,
    #[error("failed to read run log {path}: {source}")]
    LogIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("recorded PID no longer has the recorded process identity")]
    ProcessIdentityMismatch,
    #[error(transparent)]
    Platform(#[from] PlatformError),
    #[error("run is not currently running")]
    NotRunning,
}

pub struct RunControlService {
    state_directory: PathBuf,
    redaction_patterns: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryOutcome {
    None,
    Monitoring(RunId),
    ReleasedTerminal(RunId),
    Unknown(RunId),
}

impl RunControlService {
    pub fn new(state_directory: PathBuf, redaction_patterns: Vec<String>) -> Self {
        Self {
            state_directory,
            redaction_patterns,
        }
    }

    pub fn get_status(&self, run_id: &RunId) -> Result<RunRecord, RunControlError> {
        let repository = FileRunRepository::new(self.state_directory.clone())?;
        let record = repository.load(run_id)?;
        if record.status != RunStatus::Running {
            return Ok(record);
        }
        match process_identity_matches(record.pid, record.process_start_identity) {
            Ok(true) => Ok(record),
            Ok(false) => {
                if is_monitored(run_id) {
                    return Ok(record);
                }
                mark_unknown(&self.state_directory, run_id)?;
                Ok(repository.load(run_id)?)
            }
            Err(_) => {
                if is_monitored(run_id) {
                    return Ok(record);
                }
                mark_unknown(&self.state_directory, run_id)?;
                Ok(repository.load(run_id)?)
            }
        }
    }

    pub fn get_log(&self, run_id: &RunId, stream: LogStream) -> Result<LogTail, RunControlError> {
        let record = FileRunRepository::new(self.state_directory.clone())?.load(run_id)?;
        let run_directory = self.state_directory.join("runs").join(run_id.to_string());
        let (recorded, expected_name) = match stream {
            LogStream::Stdout => (&record.stdout_path, "stdout.log"),
            LogStream::Stderr => (&record.stderr_path, "stderr.log"),
        };
        let expected = run_directory.join(expected_name);
        let canonical_directory =
            fs::canonicalize(&run_directory).map_err(|source| RunControlError::LogIo {
                path: run_directory.clone(),
                source,
            })?;
        let canonical_expected =
            fs::canonicalize(&expected).map_err(|source| RunControlError::LogIo {
                path: expected,
                source,
            })?;
        let canonical_log =
            fs::canonicalize(recorded).map_err(|source| RunControlError::LogIo {
                path: recorded.clone(),
                source,
            })?;
        if canonical_log != canonical_expected || !canonical_log.starts_with(&canonical_directory) {
            return Err(RunControlError::UnsafeLogPath);
        }
        read_log_tail(&canonical_log, &self.redaction_patterns)
    }

    pub fn stop(
        &self,
        run_id: &RunId,
        graceful_timeout: Duration,
    ) -> Result<RunRecord, RunControlError> {
        let repository = FileRunRepository::new(self.state_directory.clone())?;
        let record = repository.load(run_id)?;
        if record.status != RunStatus::Running {
            return Err(RunControlError::NotRunning);
        }
        match process_identity_matches(record.pid, record.process_start_identity) {
            Ok(true) => {}
            Ok(false) | Err(PlatformError::IdentityMismatch) => {
                return Err(RunControlError::ProcessIdentityMismatch);
            }
            Err(error) => return Err(error.into()),
        }

        let stop_path = self
            .state_directory
            .join("runs")
            .join(run_id.to_string())
            .join("stop.requested");
        write_stop_marker(&stop_path, StopMethod::Graceful)?;
        let _ = request_graceful_stop(record.pid);
        if let Some(exit_code) =
            wait_for_exit(record.pid, record.process_start_identity, graceful_timeout)?
        {
            return finalize_run(&self.state_directory, run_id, exit_code);
        }

        write_stop_marker(&stop_path, StopMethod::Forced)?;
        terminate_job(run_id, 1)?;
        let exit_code = wait_for_exit(
            record.pid,
            record.process_start_identity,
            Duration::from_secs(5),
        )?
        .ok_or(PlatformError::TerminationTimeout)?;
        finalize_run(&self.state_directory, run_id, exit_code)
    }
}

pub fn monitor_child(state_directory: PathBuf, process: BackgroundProcess, run_id: RunId) {
    if let Ok(mut runs) = MONITORED_RUNS.lock() {
        runs.insert(run_id);
    }
    std::thread::spawn(move || {
        let mut child = process.into_child();
        let outcome = child.wait();
        match outcome {
            Ok(status) => {
                let code = status.code().unwrap_or(-1);
                let _ = finalize_run(&state_directory, &run_id, code);
            }
            Err(_) => {
                let _ = mark_unknown(&state_directory, &run_id);
            }
        }
        if let Ok(mut runs) = MONITORED_RUNS.lock() {
            runs.remove(&run_id);
        }
    });
}

fn is_monitored(run_id: &RunId) -> bool {
    MONITORED_RUNS
        .lock()
        .is_ok_and(|runs| runs.contains(run_id))
}

pub fn recover_active_run(state_directory: &Path) -> Result<RecoveryOutcome, RunControlError> {
    let Some(run_id) = active_run_id(state_directory)? else {
        return Ok(RecoveryOutcome::None);
    };
    let repository = FileRunRepository::new(state_directory.to_path_buf())?;
    let record = match repository.load(&run_id) {
        Ok(record) => record,
        Err(_) => return Ok(RecoveryOutcome::Unknown(run_id)),
    };
    match record.status {
        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Stopped => {
            release_active_run_lock(state_directory, &run_id)?;
            Ok(RecoveryOutcome::ReleasedTerminal(run_id))
        }
        RunStatus::Unknown => Ok(RecoveryOutcome::Unknown(run_id)),
        RunStatus::Running => {
            match process_identity_matches(record.pid, record.process_start_identity) {
                Ok(true) => {
                    monitor_existing_process(
                        state_directory.to_path_buf(),
                        run_id,
                        record.pid,
                        record.process_start_identity,
                    );
                    Ok(RecoveryOutcome::Monitoring(run_id))
                }
                Ok(false) | Err(_) => {
                    mark_unknown(state_directory, &run_id)?;
                    Ok(RecoveryOutcome::Unknown(run_id))
                }
            }
        }
    }
}

fn monitor_existing_process(
    state_directory: PathBuf,
    run_id: RunId,
    pid: u32,
    identity: crate::domain::ProcessStartIdentity,
) {
    if let Ok(mut runs) = MONITORED_RUNS.lock() {
        runs.insert(run_id);
    }
    std::thread::spawn(move || {
        match wait_for_exit(pid, identity, Duration::from_millis(u64::from(u32::MAX))) {
            Ok(Some(exit_code)) => {
                let _ = finalize_run(&state_directory, &run_id, exit_code);
            }
            Ok(None) | Err(_) => {
                let _ = mark_unknown(&state_directory, &run_id);
            }
        }
        if let Ok(mut runs) = MONITORED_RUNS.lock() {
            runs.remove(&run_id);
        }
    });
}

fn finalize_run(
    state_directory: &Path,
    run_id: &RunId,
    exit_code: i32,
) -> Result<RunRecord, RunControlError> {
    let repository = FileRunRepository::new(state_directory.to_path_buf())?;
    let mut record = repository.load(run_id)?;
    if record.status != RunStatus::Running {
        return Ok(record);
    }
    let stop_method = read_stop_method(
        &state_directory
            .join("runs")
            .join(run_id.to_string())
            .join("stop.requested"),
    );
    let expected_revision = record.revision;
    record.revision = record.revision.saturating_add(1);
    record.status = match stop_method {
        Some(_) => RunStatus::Stopped,
        None if exit_code == 0 => RunStatus::Succeeded,
        None => RunStatus::Failed,
    };
    record.exit_code = Some(exit_code);
    record.stop_method = stop_method;
    record.finished_at_unix_ms = Some(unix_time_ms());
    if let Err(error) = repository.replace(expected_revision, &record) {
        if matches!(error, StorageError::RunRevisionConflict { .. }) {
            let current = repository.load(run_id)?;
            if current.status != RunStatus::Running {
                return Ok(current);
            }
        }
        return Err(error.into());
    }
    update_plan_terminal(state_directory, &record);
    crate::emulator::stop_marked_emulator(&state_directory.join("runs").join(run_id.to_string()));
    let _ = release_active_run_lock(state_directory, run_id);
    Ok(record)
}

fn mark_unknown(state_directory: &Path, run_id: &RunId) -> Result<(), RunControlError> {
    let repository = FileRunRepository::new(state_directory.to_path_buf())?;
    let mut record = repository.load(run_id)?;
    if record.status != RunStatus::Running {
        return Ok(());
    }
    let expected_revision = record.revision;
    record.revision = record.revision.saturating_add(1);
    record.status = RunStatus::Unknown;
    repository.replace(expected_revision, &record)?;
    Ok(())
}

fn update_plan_terminal(state_directory: &Path, record: &RunRecord) {
    let Ok(repository) = FilePlanRepository::new(state_directory.to_path_buf()) else {
        return;
    };
    let Ok(mut plan) = repository.load(&record.plan_id) else {
        return;
    };
    if plan.state != PlanState::Running {
        return;
    }
    let expected_revision = plan.revision;
    let Some(revision) = plan.revision.checked_add(1) else {
        return;
    };
    plan.revision = revision;
    plan.state = match record.status {
        RunStatus::Succeeded => PlanState::Succeeded,
        RunStatus::Failed => PlanState::Failed,
        RunStatus::Stopped => PlanState::Stopped,
        RunStatus::Running | RunStatus::Unknown => return,
    };
    let _ = repository.replace(expected_revision, &plan);
}

fn read_stop_method(path: &Path) -> Option<StopMethod> {
    match fs::read_to_string(path).ok()?.trim() {
        "graceful" => Some(StopMethod::Graceful),
        "forced" => Some(StopMethod::Forced),
        _ => None,
    }
}

fn write_stop_marker(path: &Path, method: StopMethod) -> Result<(), RunControlError> {
    let bytes: &[u8] = match method {
        StopMethod::Graceful => b"graceful",
        StopMethod::Forced => b"forced",
    };
    let mode = if path.exists() {
        WriteMode::Replace
    } else {
        WriteMode::Create
    };
    write_atomically(path, bytes, mode)?;
    Ok(())
}

fn read_log_tail(path: &Path, patterns: &[String]) -> Result<LogTail, RunControlError> {
    let mut file = File::open(path).map_err(|source| RunControlError::LogIo {
        path: path.to_path_buf(),
        source,
    })?;
    let length = file
        .metadata()
        .map_err(|source| RunControlError::LogIo {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let start = length.saturating_sub(MAX_LOG_BYTES);
    file.seek(SeekFrom::Start(start))
        .map_err(|source| RunControlError::LogIo {
            path: path.to_path_buf(),
            source,
        })?;
    let mut bytes = Vec::with_capacity((length - start) as usize);
    file.read_to_end(&mut bytes)
        .map_err(|source| RunControlError::LogIo {
            path: path.to_path_buf(),
            source,
        })?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        if let Some(newline) = text.find('\n') {
            text.drain(..=newline);
        } else {
            text.clear();
        }
    }
    let all_lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    let line_truncated = all_lines.len() > MAX_LOG_LINES;
    let mut lines = all_lines
        .into_iter()
        .rev()
        .take(MAX_LOG_LINES)
        .collect::<Vec<_>>();
    lines.reverse();
    for line in &mut lines {
        for pattern in patterns.iter().filter(|pattern| !pattern.is_empty()) {
            *line = line.replace(pattern, "[REDACTED]");
        }
    }
    let mut content = lines.join("\n");
    let byte_truncated = content.len() > MAX_LOG_BYTES as usize;
    if byte_truncated {
        let mut start = content.len() - MAX_LOG_BYTES as usize;
        while !content.is_char_boundary(start) {
            start += 1;
        }
        content.drain(..start);
    }
    let bytes = content.len();
    Ok(LogTail {
        content,
        lines,
        bytes,
        truncated: start > 0 || line_truncated || byte_truncated,
    })
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::{MAX_LOG_BYTES, read_log_tail};

    #[test]
    fn redaction_growth_past_the_byte_cap_still_bounds_and_reports_truncation() {
        let temp = tempfile::tempdir().expect("temporary log directory");
        let path = temp.path().join("stdout.log");
        let line = "sec".repeat(200);
        let raw = vec![line.as_str(); 100].join("\n");
        assert!(
            raw.len() as u64 <= MAX_LOG_BYTES,
            "raw fixture must fit the byte cap"
        );
        std::fs::write(&path, &raw).expect("write log fixture");

        let tail = read_log_tail(&path, &["sec".to_owned()]).expect("read redacted log tail");

        assert!(tail.bytes <= MAX_LOG_BYTES as usize);
        assert_eq!(tail.content.len(), tail.bytes);
        assert!(!tail.content.contains("sec"));
        assert!(
            tail.truncated,
            "byte-capped content must be reported as truncated"
        );
    }
}
