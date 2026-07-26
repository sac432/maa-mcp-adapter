use std::fs;
use std::time::Duration;

use maa_mcp_adapter::domain::{
    DailyPlan, DailyTask, InfrastPlan, ProcessStartIdentity, RunId, RunRecord, RunStatus,
    StopMethod,
};
use maa_mcp_adapter::planning::hash::hash_plan;
use maa_mcp_adapter::platform::{process_identity_matches, terminate_job};
use maa_mcp_adapter::storage::{FileRunRepository, RunRepository, StorageError};

mod support;

use maa_mcp_adapter::maa_cli::MaaCliAdapter;
use maa_mcp_adapter::maa_cli::process::spawn_background;
use maa_mcp_adapter::planning::service::PlanService;
use maa_mcp_adapter::runs::{
    LogStream, RecoveryOutcome, RunControlError, RunControlService, recover_active_run,
};
use support::fake_maa::FakeMaa;

#[test]
fn run_record_updates_are_revision_checked() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let repository = FileRunRepository::new(state.clone()).expect("run repository");
    let mut record = sample_record(&state, RunStatus::Running, 1_000);
    fs::create_dir_all(
        record
            .task_path
            .parent()
            .expect("task path has run directory"),
    )
    .expect("run directory");
    repository.create(&record).expect("create record");

    record.revision = 1;
    record.status = RunStatus::Succeeded;
    record.exit_code = Some(0);
    record.finished_at_unix_ms = Some(2_000);
    repository.replace(0, &record).expect("replace record");

    let stale = repository.replace(0, &record).expect_err("stale revision");
    assert!(matches!(
        stale,
        StorageError::RunRevisionConflict {
            expected: 0,
            actual: 1
        }
    ));
    assert_eq!(
        repository.load(&record.run_id).expect("updated record"),
        record
    );
}

#[test]
fn retention_removes_only_old_terminal_runs() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let repository = FileRunRepository::new(state.clone()).expect("run repository");
    let old_terminal = sample_record(&state, RunStatus::Succeeded, 1_000);
    let recent_terminal = sample_record(&state, RunStatus::Stopped, 9_500);
    let old_unknown = sample_record(&state, RunStatus::Unknown, 1_000);
    let old_with_extra_file = sample_record(&state, RunStatus::Failed, 1_000);
    for record in [
        &old_terminal,
        &recent_terminal,
        &old_unknown,
        &old_with_extra_file,
    ] {
        fs::create_dir_all(
            record
                .task_path
                .parent()
                .expect("task path has run directory"),
        )
        .expect("run directory");
        repository.create(record).expect("create record");
    }
    fs::write(
        old_with_extra_file
            .task_path
            .parent()
            .expect("run directory")
            .join("user-kept.txt"),
        "unknown content",
    )
    .expect("unknown file");

    let removed = repository
        .cleanup_terminal(10_000, 2_000)
        .expect("retention cleanup");

    assert_eq!(removed, 1);
    assert!(matches!(
        repository.load(&old_terminal.run_id),
        Err(StorageError::RunNotFound)
    ));
    assert!(repository.load(&recent_terminal.run_id).is_ok());
    assert!(repository.load(&old_unknown.run_id).is_ok());
    assert!(repository.load(&old_with_extra_file.run_id).is_ok());
}

#[tokio::test]
async fn named_job_can_be_reopened_and_terminate_the_recorded_process() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let fake = configured_background_fake(temp.path());
    let plans = PlanService::new(plan_repository(&state));
    let created = plans
        .create("default", vec![DailyTask::Infrast(InfrastPlan {})])
        .expect("create plan");
    let ready = plans.preview(&created.plan.plan_id).expect("preview plan");
    let hash = hash_plan(&ready.plan).expect("plan hash");
    let execution = maa_mcp_adapter::execution::ExecutionService::new(state.clone());
    let prepared = execution
        .prepare(&ready.plan.plan_id, hash.as_str())
        .expect("prepare execution");
    let launched = execution
        .launch(
            &prepared,
            &MaaCliAdapter::new(fake.executable().to_path_buf()),
        )
        .await
        .expect("launch execution");

    terminate_job(&launched.record.run_id, 1).expect("terminate named job");

    assert!(
        !process_identity_matches(launched.record.pid, launched.record.process_start_identity)
            .unwrap_or(false)
    );
}

#[tokio::test]
async fn monitor_persists_success_and_failure_exit_codes_and_releases_lock() {
    for (mode, expected_status, expected_code) in [
        ("complete", RunStatus::Succeeded, 0),
        ("actual-fail", RunStatus::Failed, 29),
    ] {
        let temp = tempfile::tempdir().expect("temporary state");
        let state = temp.path().join("state");
        fs::create_dir(&state).expect("state directory");
        let fake = configured_background_fake(temp.path());
        fake.set_mode(mode);
        let launched = launch_ready_plan(&state, &fake).await;
        let controls = RunControlService::new(state.clone(), Vec::new());

        let final_record = wait_for_terminal(&controls, launched.record.run_id).await;

        assert_eq!(final_record.status, expected_status);
        assert_eq!(final_record.exit_code, Some(expected_code));
        assert!(final_record.finished_at_unix_ms.is_some());
        let mut released = false;
        for _ in 0..100 {
            if maa_mcp_adapter::storage::ActiveRunLock::acquire(&state, RunId::new()).is_ok() {
                released = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(released, "terminal monitor releases active lock");
    }
}

#[tokio::test]
async fn stop_run_forces_the_named_job_and_records_the_method() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let fake = configured_background_fake(temp.path());
    let launched = launch_ready_plan(&state, &fake).await;
    let controls = RunControlService::new(state.clone(), Vec::new());

    let stopped = controls
        .stop(&launched.record.run_id, Duration::from_millis(50))
        .expect("stop recorded run");

    assert_eq!(stopped.status, RunStatus::Stopped);
    assert_eq!(stopped.stop_method, Some(StopMethod::Forced));
    assert!(
        !process_identity_matches(stopped.pid, stopped.process_start_identity).unwrap_or(false)
    );
}

#[tokio::test]
async fn terminal_run_stops_only_the_recorded_adapter_started_emulator() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let fake = configured_background_fake(temp.path());
    let launched = launch_ready_plan(&state, &fake).await;

    let comspec = std::env::var_os("ComSpec").expect("cmd.exe available");
    let mut emulator = std::process::Command::new(&comspec)
        .args(["/C", "ping", "-n", "30", "127.0.0.1"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn stand-in emulator");
    let identity = maa_mcp_adapter::platform::process_identity_for_child(&emulator)
        .expect("stand-in emulator identity");
    let run_directory = state.join("runs").join(launched.record.run_id.to_string());
    maa_mcp_adapter::emulator::write_marker(
        &run_directory,
        &maa_mcp_adapter::emulator::StartedEmulator {
            pid: emulator.id(),
            creation_time_100ns: identity.creation_time_100ns,
        },
    )
    .expect("write emulator marker");

    let controls = RunControlService::new(state.clone(), Vec::new());
    let stopped = controls
        .stop(&launched.record.run_id, Duration::from_millis(50))
        .expect("stop recorded run");

    assert_eq!(stopped.status, RunStatus::Stopped);
    assert!(
        !run_directory.join("emulator.json").exists(),
        "the emulator marker is consumed exactly once"
    );
    assert!(
        !process_identity_matches(emulator.id(), identity).unwrap_or(false),
        "the adapter-started emulator is terminated with the run"
    );
    let _ = emulator.wait();
}

#[test]
fn stop_run_rejects_a_reused_pid_identity_without_terminating_it() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let repository = FileRunRepository::new(state.clone()).expect("run repository");
    let mut record = sample_record(&state, RunStatus::Running, 0);
    record.pid = std::process::id();
    record.process_start_identity.creation_time_100ns = 1;
    fs::create_dir_all(record.task_path.parent().expect("run directory")).expect("run directory");
    repository.create(&record).expect("record");
    let controls = RunControlService::new(state, Vec::new());

    let error = controls
        .stop(&record.run_id, Duration::from_millis(1))
        .expect_err("identity mismatch");

    assert!(matches!(error, RunControlError::ProcessIdentityMismatch));
}

#[test]
fn status_query_marks_a_reused_pid_unknown() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let repository = FileRunRepository::new(state.clone()).expect("run repository");
    let mut record = sample_record(&state, RunStatus::Running, 0);
    record.pid = std::process::id();
    record.process_start_identity.creation_time_100ns = 1;
    fs::create_dir_all(record.task_path.parent().expect("run directory")).expect("run directory");
    repository.create(&record).expect("record");

    let observed = RunControlService::new(state, Vec::new())
        .get_status(&record.run_id)
        .expect("status");

    assert_eq!(observed.status, RunStatus::Unknown);
}

#[tokio::test]
async fn restart_recovery_monitors_a_matching_detached_process() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let fake = configured_background_fake(temp.path());
    let run_id = RunId::new();
    let run_directory = state.join("runs").join(run_id.to_string());
    fs::create_dir_all(&run_directory).expect("run directory");
    let lock =
        maa_mcp_adapter::storage::ActiveRunLock::acquire(&state, run_id).expect("active lock");
    let process = spawn_background(
        &run_id,
        fake.executable(),
        &[
            "--batch".into(),
            "run".into(),
            run_directory.join("task.toml").into_os_string(),
            "--profile".into(),
            "default".into(),
            "-v".into(),
        ],
        &run_directory.join("stdout.log"),
        &run_directory.join("stderr.log"),
    )
    .expect("background process");
    let mut record = sample_record(&state, RunStatus::Running, 0);
    record.run_id = run_id;
    record.pid = process.pid();
    record.process_start_identity = process.identity();
    record.task_path = run_directory.join("task.toml");
    record.stdout_path = run_directory.join("stdout.log");
    record.stderr_path = run_directory.join("stderr.log");
    FileRunRepository::new(state.clone())
        .expect("run repository")
        .create(&record)
        .expect("record");
    lock.commit().expect("persist active lock");
    process.detach();

    assert_eq!(
        recover_active_run(&state).expect("recover active run"),
        RecoveryOutcome::Monitoring(run_id)
    );
    terminate_job(&run_id, 7).expect("terminate recovered job");
    let controls = RunControlService::new(state.clone(), Vec::new());
    let terminal = wait_for_terminal(&controls, run_id).await;
    assert_eq!(terminal.status, RunStatus::Failed);
    assert_eq!(terminal.exit_code, Some(7));
}

#[test]
fn restart_recovery_marks_identity_conflict_unknown_and_keeps_lock() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let repository = FileRunRepository::new(state.clone()).expect("run repository");
    let mut record = sample_record(&state, RunStatus::Running, 0);
    record.pid = std::process::id();
    record.process_start_identity.creation_time_100ns = 1;
    fs::create_dir_all(record.task_path.parent().expect("run directory")).expect("run directory");
    repository.create(&record).expect("record");
    maa_mcp_adapter::storage::ActiveRunLock::acquire(&state, record.run_id)
        .expect("lock")
        .commit()
        .expect("persist lock");

    assert_eq!(
        recover_active_run(&state).expect("recover conflict"),
        RecoveryOutcome::Unknown(record.run_id)
    );
    assert_eq!(
        repository
            .load(&record.run_id)
            .expect("unknown record")
            .status,
        RunStatus::Unknown
    );
    assert!(matches!(
        maa_mcp_adapter::storage::ActiveRunLock::acquire(&state, RunId::new()),
        Err(StorageError::ActiveRunExists)
    ));
}

#[test]
fn log_tail_is_bounded_redacted_and_rejects_recorded_path_escape() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let repository = FileRunRepository::new(state.clone()).expect("run repository");
    let record = sample_record(&state, RunStatus::Succeeded, 2_000);
    let run_directory = record.task_path.parent().expect("run directory");
    fs::create_dir_all(run_directory).expect("run directory");
    let lines = (0..300)
        .map(|index| format!("line-{index:03} account-123 token=secret-value"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&record.stdout_path, lines).expect("stdout log");
    fs::write(&record.stderr_path, "stderr").expect("stderr log");
    repository.create(&record).expect("create record");
    let controls = RunControlService::new(
        state.clone(),
        vec!["account-123".to_owned(), "secret-value".to_owned()],
    );

    let output = controls
        .get_log(&record.run_id, LogStream::Stdout)
        .expect("bounded log");

    assert!(output.lines.len() <= 200);
    assert!(output.bytes <= 64 * 1024);
    assert!(output.truncated);
    assert!(!output.content.contains("account-123"));
    assert!(!output.content.contains("secret-value"));
    assert!(output.content.contains("[REDACTED]"));

    let mut escaped = repository.load(&record.run_id).expect("record");
    escaped.revision += 1;
    escaped.stdout_path = temp.path().join("outside.log");
    fs::write(&escaped.stdout_path, "must not read").expect("outside log");
    repository
        .replace(escaped.revision - 1, &escaped)
        .expect("tamper stored path");
    assert!(matches!(
        controls.get_log(&record.run_id, LogStream::Stdout),
        Err(RunControlError::UnsafeLogPath)
    ));
}

fn sample_record(state: &std::path::Path, status: RunStatus, finished_at: u64) -> RunRecord {
    let run_id = RunId::new();
    let run_directory = state.join("runs").join(run_id.to_string());
    let plan = DailyPlan::new("default", vec![DailyTask::Infrast(InfrastPlan {})]);
    RunRecord {
        run_id,
        plan_id: plan.plan_id,
        plan_hash: hash_plan(&plan).expect("plan hash"),
        revision: 0,
        pid: 42,
        process_start_identity: ProcessStartIdentity {
            creation_time_100ns: 123_456,
        },
        task_path: run_directory.join("task.toml"),
        task_sha256: "a".repeat(64),
        stdout_path: run_directory.join("stdout.log"),
        stderr_path: run_directory.join("stderr.log"),
        status,
        exit_code: (status == RunStatus::Succeeded).then_some(0),
        stop_method: (status == RunStatus::Stopped).then_some(StopMethod::Forced),
        started_at_unix_ms: 500,
        finished_at_unix_ms: Some(finished_at),
    }
}

fn configured_background_fake(root: &std::path::Path) -> FakeMaa {
    let fake = FakeMaa::build(root);
    let config_directory = root.join("maa-config");
    fs::create_dir_all(config_directory.join("profiles")).expect("profile directory");
    fs::write(
        config_directory.join("profiles").join("default.toml"),
        "configured = true\n",
    )
    .expect("profile");
    fake.set_config_directory(&config_directory);
    fake.set_mode("background");
    fake
}

async fn launch_ready_plan(
    state: &std::path::Path,
    fake: &FakeMaa,
) -> maa_mcp_adapter::execution::LaunchedExecution {
    let plans = PlanService::new(plan_repository(state));
    let created = plans
        .create("default", vec![DailyTask::Infrast(InfrastPlan {})])
        .expect("create plan");
    let ready = plans.preview(&created.plan.plan_id).expect("preview plan");
    let hash = hash_plan(&ready.plan).expect("plan hash");
    let execution = maa_mcp_adapter::execution::ExecutionService::new(state.to_path_buf());
    let prepared = execution
        .prepare(&ready.plan.plan_id, hash.as_str())
        .expect("prepare execution");
    execution
        .launch(
            &prepared,
            &MaaCliAdapter::new(fake.executable().to_path_buf()),
        )
        .await
        .expect("launch execution")
}

async fn wait_for_terminal(controls: &RunControlService, run_id: RunId) -> RunRecord {
    let mut last = None;
    for _ in 0..100 {
        let record = controls.get_status(&run_id).expect("run status");
        if matches!(
            record.status,
            RunStatus::Succeeded | RunStatus::Failed | RunStatus::Stopped
        ) {
            return record;
        }
        last = Some(record);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("run did not reach terminal state: {last:?}");
}

fn plan_repository(state: &std::path::Path) -> maa_mcp_adapter::storage::FilePlanRepository {
    maa_mcp_adapter::storage::FilePlanRepository::new(state.to_path_buf()).expect("plan repository")
}
