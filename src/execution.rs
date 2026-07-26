use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::config::EmulatorConfig;
use crate::domain::{DailyPlan, PlanHash, PlanId, PlanState, RunId, RunRecord, RunStatus};
use crate::emulator::EmulatorError;
use crate::maa_cli::task_file::{TaskFileError, TaskFileStore};
use crate::maa_cli::{MaaCliAdapter, MaaCliError};
use crate::planning::hash::hash_plan;
use crate::planning::service::PlanService;
use crate::planning::summary::summarize_plan;
use crate::platform::{PlatformError, process_identity_matches, terminate_job, wait_for_exit};
use crate::storage::{
    ActiveRunLock, FilePlanRepository, FileRunRepository, PlanRepository, RunRepository,
    StorageError,
};

#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("plan storage failed")]
    Storage(#[from] StorageError),
    #[error("plan must be Ready before execution")]
    PlanNotReady,
    #[error("expected plan hash is invalid")]
    InvalidExpectedHash,
    #[error("expected plan hash does not match the ready plan")]
    PlanHashMismatch,
    #[error("expected plan summary does not match the ready plan")]
    SummaryMismatch,
    #[error("ready plan summary could not be generated")]
    InvalidPlan,
    #[error("plan changed after the confirmation snapshot")]
    PlanChanged,
    #[error("maa-cli environment inspection failed")]
    Environment(#[from] MaaCliError),
    #[error("maa-cli does not provide all required execution capabilities")]
    CapabilitiesUnavailable,
    #[error("selected profile is not available")]
    ProfileUnavailable,
    #[error("generated task file failed validation or launch")]
    TaskFile(#[from] TaskFileError),
    #[error("system time is before Unix epoch")]
    InvalidSystemTime,
    #[error("configured emulator could not be made ready")]
    Emulator(#[from] EmulatorError),
    #[error("spawned process could not be safely terminated after persistence failure")]
    RollbackFailed(#[from] PlatformError),
}

impl ExecutionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Storage(StorageError::NotFound) => "PLAN_NOT_FOUND",
            Self::Storage(StorageError::ActiveRunExists) => "ACTIVE_RUN_EXISTS",
            Self::Storage(_) => "STORAGE_ERROR",
            Self::PlanNotReady => "PLAN_NOT_READY",
            Self::InvalidExpectedHash => "PLAN_HASH_INVALID",
            Self::PlanHashMismatch => "PLAN_HASH_MISMATCH",
            Self::SummaryMismatch => "PLAN_SUMMARY_MISMATCH",
            Self::InvalidPlan => "PLAN_INVALID",
            Self::PlanChanged => "PLAN_CHANGED",
            Self::Environment(_) => "ENVIRONMENT_CHECK_FAILED",
            Self::CapabilitiesUnavailable => "MAA_CAPABILITIES_UNAVAILABLE",
            Self::ProfileUnavailable => "PROFILE_UNAVAILABLE",
            Self::TaskFile(TaskFileError::IntegrityMismatch) => "TASK_FILE_CHANGED",
            Self::TaskFile(TaskFileError::Process(_)) => "MAA_PROCESS_FAILED",
            Self::TaskFile(_) => "TASK_FILE_ERROR",
            Self::InvalidSystemTime => "SYSTEM_TIME_INVALID",
            Self::Emulator(_) => "EMULATOR_UNAVAILABLE",
            Self::RollbackFailed(_) => "PROCESS_ROLLBACK_FAILED",
        }
    }
}

#[derive(Debug)]
pub struct PreparedExecution {
    plan_id: PlanId,
    revision: u64,
    plan_hash: PlanHash,
    summary: String,
}

impl PreparedExecution {
    pub fn plan_id(&self) -> &PlanId {
        &self.plan_id
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn plan_hash(&self) -> &PlanHash {
        &self.plan_hash
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn require_summary(&self, expected_summary: &str) -> Result<(), ExecutionError> {
        if self.summary == expected_summary {
            Ok(())
        } else {
            Err(ExecutionError::SummaryMismatch)
        }
    }
}

pub struct ExecutionService {
    state_directory: PathBuf,
    emulator: Option<EmulatorConfig>,
}

#[derive(Debug)]
pub struct LaunchedExecution {
    pub record: RunRecord,
    pub summary: String,
}

impl ExecutionService {
    pub fn new(state_directory: PathBuf) -> Self {
        Self {
            state_directory,
            emulator: None,
        }
    }

    pub fn with_emulator(mut self, emulator: Option<EmulatorConfig>) -> Self {
        self.emulator = emulator;
        self
    }

    pub fn prepare(
        &self,
        plan_id: &PlanId,
        expected_plan_hash: &str,
    ) -> Result<PreparedExecution, ExecutionError> {
        if !is_plan_hash(expected_plan_hash) {
            return Err(ExecutionError::InvalidExpectedHash);
        }
        let repository = FilePlanRepository::new(self.state_directory.clone())?;
        let plan = repository.load(plan_id)?;
        if plan.state != PlanState::Ready {
            return Err(ExecutionError::PlanNotReady);
        }
        let plan_hash = hash_plan(&plan).map_err(|_| ExecutionError::InvalidPlan)?;
        if plan_hash.as_str() != expected_plan_hash {
            return Err(ExecutionError::PlanHashMismatch);
        }
        let view = PlanService::new(repository)
            .preview(plan_id)
            .map_err(|_| ExecutionError::InvalidPlan)?;
        let summary = view.summary.ok_or(ExecutionError::InvalidPlan)?;
        Ok(PreparedExecution {
            plan_id: *plan_id,
            revision: plan.revision,
            plan_hash,
            summary,
        })
    }

    pub async fn launch(
        &self,
        prepared: &PreparedExecution,
        adapter: &MaaCliAdapter,
    ) -> Result<LaunchedExecution, ExecutionError> {
        let run_id = RunId::new();
        let active_lock = ActiveRunLock::acquire(&self.state_directory, run_id)?;
        let confirmed_revision = self.confirm_prepared(prepared)?;

        let setup = self
            .validate_environment_and_task(prepared, adapter, run_id)
            .await;
        let artifact = match setup {
            Ok(artifact) => artifact,
            Err(error) => {
                self.rollback_confirmed(prepared, confirmed_revision);
                return Err(error);
            }
        };
        let emulator = match &self.emulator {
            Some(config) => match crate::emulator::ensure_running(config).await {
                Ok(started) => started,
                Err(error) => {
                    self.rollback_confirmed(prepared, confirmed_revision);
                    return Err(error.into());
                }
            },
            None => None,
        };

        let run_directory = artifact
            .path()
            .parent()
            .expect("generated task has a run directory");
        let stdout_path = run_directory.join("stdout.log");
        let stderr_path = run_directory.join("stderr.log");
        let started_at_unix_ms = match unix_time_ms() {
            Ok(value) => value,
            Err(error) => {
                self.rollback_confirmed(prepared, confirmed_revision);
                return Err(error);
            }
        };
        let run_repository = match FileRunRepository::new(self.state_directory.clone()) {
            Ok(repository) => repository,
            Err(error) => {
                self.rollback_confirmed(prepared, confirmed_revision);
                return Err(error.into());
            }
        };
        if let Err(error) = self.mark_running(prepared, confirmed_revision) {
            self.rollback_confirmed(prepared, confirmed_revision);
            return Err(error);
        }
        let spawned =
            match adapter.launch_generated_task(&run_id, &artifact, &stdout_path, &stderr_path) {
                Ok(spawned) => spawned,
                Err(error) => {
                    self.mark_running_failed(prepared);
                    return Err(error.into());
                }
            };
        let record = RunRecord {
            run_id,
            plan_id: *prepared.plan_id(),
            plan_hash: prepared.plan_hash().clone(),
            revision: 0,
            pid: spawned.pid(),
            process_start_identity: spawned.identity(),
            task_path: artifact.path().to_path_buf(),
            task_sha256: artifact.sha256_hex(),
            stdout_path,
            stderr_path,
            status: RunStatus::Running,
            exit_code: None,
            stop_method: None,
            started_at_unix_ms,
            finished_at_unix_ms: None,
        };
        if let Err(error) = run_repository.create(&record) {
            self.mark_running_failed(prepared);
            return self.rollback_spawned(active_lock, spawned, error.into());
        }
        if let (Some(started), Some(config)) = (emulator.as_ref(), self.emulator.as_ref())
            && config.stop_after_run
        {
            let _ = crate::emulator::write_marker(run_directory, started);
        }
        if let Err(error) = active_lock.commit() {
            self.mark_running_failed(prepared);
            self.terminate_spawned_job(run_id, spawned.pid(), spawned.identity())?;
            return Err(error.into());
        }
        crate::runs::monitor_child(self.state_directory.clone(), spawned, record.run_id);
        Ok(LaunchedExecution {
            record,
            summary: prepared.summary.clone(),
        })
    }

    async fn validate_environment_and_task(
        &self,
        prepared: &PreparedExecution,
        adapter: &MaaCliAdapter,
        run_id: RunId,
    ) -> Result<crate::maa_cli::task_file::TaskFileArtifact, ExecutionError> {
        let repository = FilePlanRepository::new(self.state_directory.clone())?;
        let plan = repository.load(prepared.plan_id())?;
        self.require_confirmed_snapshot(&plan, prepared)?;
        let environment = adapter.inspect_environment(&self.state_directory).await?;
        if !environment.can_execute() {
            return Err(ExecutionError::CapabilitiesUnavailable);
        }
        if !environment
            .profiles
            .iter()
            .any(|profile| profile == &plan.profile)
        {
            return Err(ExecutionError::ProfileUnavailable);
        }
        let store = TaskFileStore::new(self.state_directory.clone())?;
        let artifact = store.generate(run_id, &plan)?;
        adapter
            .dry_run_generated_task(&artifact, &plan.profile, None)
            .await?;
        Ok(artifact)
    }

    fn confirm_prepared(&self, prepared: &PreparedExecution) -> Result<u64, ExecutionError> {
        let repository = FilePlanRepository::new(self.state_directory.clone())?;
        let mut plan = repository.load(prepared.plan_id())?;
        self.require_ready_snapshot(&plan, prepared)?;
        let expected_revision = plan.revision;
        plan.revision = plan
            .revision
            .checked_add(1)
            .ok_or(ExecutionError::InvalidPlan)?;
        plan.state = PlanState::Confirmed;
        plan.confirmed_hash = Some(prepared.plan_hash.clone());
        repository.replace(expected_revision, &plan)?;
        Ok(plan.revision)
    }

    fn mark_running(
        &self,
        prepared: &PreparedExecution,
        confirmed_revision: u64,
    ) -> Result<(), ExecutionError> {
        let repository = FilePlanRepository::new(self.state_directory.clone())?;
        let mut plan = repository.load(prepared.plan_id())?;
        if plan.revision != confirmed_revision {
            return Err(ExecutionError::PlanChanged);
        }
        self.require_confirmed_snapshot(&plan, prepared)?;
        let expected_revision = plan.revision;
        plan.revision = plan
            .revision
            .checked_add(1)
            .ok_or(ExecutionError::InvalidPlan)?;
        plan.state = PlanState::Running;
        repository.replace(expected_revision, &plan)?;
        Ok(())
    }

    fn require_ready_snapshot(
        &self,
        plan: &DailyPlan,
        prepared: &PreparedExecution,
    ) -> Result<(), ExecutionError> {
        if plan.state != PlanState::Ready
            || plan.revision != prepared.revision
            || hash_plan(plan)
                .map_err(|_| ExecutionError::InvalidPlan)?
                .as_str()
                != prepared.plan_hash.as_str()
            || summarize_plan(plan).map_err(|_| ExecutionError::InvalidPlan)? != prepared.summary
        {
            return Err(ExecutionError::PlanChanged);
        }
        Ok(())
    }

    fn require_confirmed_snapshot(
        &self,
        plan: &DailyPlan,
        prepared: &PreparedExecution,
    ) -> Result<(), ExecutionError> {
        if plan.state != PlanState::Confirmed
            || plan.confirmed_hash.as_ref() != Some(&prepared.plan_hash)
            || hash_plan(plan)
                .map_err(|_| ExecutionError::InvalidPlan)?
                .as_str()
                != prepared.plan_hash.as_str()
        {
            return Err(ExecutionError::PlanChanged);
        }
        Ok(())
    }

    fn rollback_confirmed(&self, prepared: &PreparedExecution, confirmed_revision: u64) {
        let Ok(repository) = FilePlanRepository::new(self.state_directory.clone()) else {
            return;
        };
        let Ok(mut plan) = repository.load(prepared.plan_id()) else {
            return;
        };
        if plan.state != PlanState::Confirmed
            || plan.revision != confirmed_revision
            || plan.confirmed_hash.as_ref() != Some(prepared.plan_hash())
        {
            return;
        }
        let expected_revision = plan.revision;
        let Some(revision) = plan.revision.checked_add(1) else {
            return;
        };
        plan.revision = revision;
        plan.state = PlanState::Ready;
        plan.confirmed_hash = None;
        let _ = repository.replace(expected_revision, &plan);
    }

    fn mark_running_failed(&self, prepared: &PreparedExecution) {
        let Ok(repository) = FilePlanRepository::new(self.state_directory.clone()) else {
            return;
        };
        let Ok(mut plan) = repository.load(prepared.plan_id()) else {
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
        plan.state = PlanState::Failed;
        let _ = repository.replace(expected_revision, &plan);
    }

    fn rollback_spawned(
        &self,
        active_lock: ActiveRunLock,
        spawned: crate::maa_cli::process::BackgroundProcess,
        original: ExecutionError,
    ) -> Result<LaunchedExecution, ExecutionError> {
        if let Err(error) = self.terminate_spawned_job(
            spawned_run_id(&active_lock),
            spawned.pid(),
            spawned.identity(),
        ) {
            let _ = active_lock.commit();
            return Err(ExecutionError::RollbackFailed(error));
        }
        Err(original)
    }

    fn terminate_spawned_job(
        &self,
        run_id: RunId,
        pid: u32,
        identity: crate::domain::ProcessStartIdentity,
    ) -> Result<(), PlatformError> {
        if !process_identity_matches(pid, identity)? {
            return Err(PlatformError::IdentityMismatch);
        }
        terminate_job(&run_id, 1)?;
        wait_for_exit(pid, identity, std::time::Duration::from_secs(5))?
            .ok_or(PlatformError::TerminationTimeout)?;
        Ok(())
    }
}

fn spawned_run_id(lock: &ActiveRunLock) -> RunId {
    lock.run_id()
}

fn is_plan_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn unix_time_ms() -> Result<u64, ExecutionError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ExecutionError::InvalidSystemTime)?
        .as_millis();
    u64::try_from(millis).map_err(|_| ExecutionError::InvalidSystemTime)
}

#[cfg(test)]
mod tests {
    use crate::domain::{DailyTask, InfrastPlan};
    use crate::planning::hash::hash_plan;
    use crate::planning::service::{PlanService, PlanView};
    use crate::storage::FilePlanRepository;

    use super::{ExecutionError, ExecutionService};

    #[test]
    fn final_snapshot_rejects_a_changed_confirmation_summary() {
        let temp = tempfile::tempdir().expect("temporary state");
        let state = temp.path().join("state");
        std::fs::create_dir(&state).expect("state directory");
        let plans =
            PlanService::new(FilePlanRepository::new(state.clone()).expect("plan repository"));
        let PlanView { plan, .. } = plans
            .create("default", vec![DailyTask::Infrast(InfrastPlan {})])
            .expect("create plan");
        let ready = plans.preview(&plan.plan_id).expect("preview plan");
        let hash = hash_plan(&ready.plan).expect("plan hash");
        let service = ExecutionService::new(state);
        let mut prepared = service
            .prepare(&ready.plan.plan_id, hash.as_str())
            .expect("prepare execution");
        prepared.summary.push_str("\n已被改变");

        let error = service
            .require_ready_snapshot(&ready.plan, &prepared)
            .expect_err("changed summary must invalidate confirmation");

        assert!(matches!(error, ExecutionError::PlanChanged));
    }

    #[test]
    fn prepared_execution_rejects_a_summary_that_differs_from_the_server_text() {
        let temp = tempfile::tempdir().expect("temporary state");
        let state = temp.path().join("state");
        std::fs::create_dir(&state).expect("state directory");
        let plans =
            PlanService::new(FilePlanRepository::new(state.clone()).expect("plan repository"));
        let PlanView { plan, .. } = plans
            .create("default", vec![DailyTask::Infrast(InfrastPlan {})])
            .expect("create plan");
        let ready = plans.preview(&plan.plan_id).expect("preview plan");
        let hash = hash_plan(&ready.plan).expect("plan hash");
        let service = ExecutionService::new(state);
        let prepared = service
            .prepare(&ready.plan.plan_id, hash.as_str())
            .expect("prepare execution");

        assert!(prepared.require_summary(prepared.summary()).is_ok());
        let error = prepared
            .require_summary("被改写的摘要")
            .expect_err("a mismatched summary must refuse execution");

        assert!(matches!(error, ExecutionError::SummaryMismatch));
    }
}
