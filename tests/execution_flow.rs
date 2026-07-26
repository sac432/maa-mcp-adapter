use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::str::FromStr;
use std::thread;
use std::time::{Duration, Instant};

use maa_mcp_adapter::domain::{
    DailyPlan, DailyTask, InfrastPlan, PlanState, ProcessStartIdentity, RunId, RunRecord, RunStatus,
};
use maa_mcp_adapter::maa_cli::MaaCliAdapter;
use maa_mcp_adapter::maa_cli::process::{ProcessError, spawn_background};
use maa_mcp_adapter::planning::hash::hash_plan;
use maa_mcp_adapter::planning::service::{PlanPatch, PlanService};
use maa_mcp_adapter::storage::{
    ActiveRunLock, FilePlanRepository, FileRunRepository, PlanRepository, RunRepository,
    StorageError, release_active_run_lock,
};

mod support;

use serde_json::{Value, json};
use support::fake_maa::FakeMaa;

#[test]
fn run_record_round_trips_with_strict_identity_and_paths() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let run_id = RunId::new();
    let run_directory = state.join("runs").join(run_id.to_string());
    fs::create_dir_all(&run_directory).expect("run directory");
    let plan = DailyPlan::new("default", vec![DailyTask::Infrast(InfrastPlan {})]);
    let record = RunRecord {
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
        status: RunStatus::Running,
        exit_code: None,
        stop_method: None,
        started_at_unix_ms: 1_700_000_000_000,
        finished_at_unix_ms: None,
    };
    let repository = FileRunRepository::new(state).expect("run repository");

    repository.create(&record).expect("create run record");
    assert_eq!(repository.load(&run_id).expect("load run record"), record);
}

#[test]
fn active_run_lock_is_atomic_and_only_owner_can_release_it() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let first_id = RunId::new();
    let second_id = RunId::new();

    let first = ActiveRunLock::acquire(&state, first_id).expect("first lock");
    assert!(matches!(
        ActiveRunLock::acquire(&state, second_id),
        Err(StorageError::ActiveRunExists)
    ));
    drop(first);

    let first = ActiveRunLock::acquire(&state, first_id).expect("reacquire lock");
    first.commit().expect("commit active lock");
    assert!(matches!(
        ActiveRunLock::acquire(&state, second_id),
        Err(StorageError::ActiveRunExists)
    ));
    assert!(matches!(
        release_active_run_lock(&state, &second_id),
        Err(StorageError::ActiveRunOwnerMismatch)
    ));
    release_active_run_lock(&state, &first_id).expect("owner releases lock");
    ActiveRunLock::acquire(&state, second_id).expect("second lock after release");
}

#[test]
fn prepared_execution_requires_ready_state_and_exact_hash() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let repository = FilePlanRepository::new(state.clone()).expect("plan repository");
    let service = PlanService::new(repository);
    let draft = service
        .create("default", vec![DailyTask::Infrast(InfrastPlan {})])
        .expect("create plan");
    let plan_id = draft.plan.plan_id;
    let expected_hash = draft
        .normalized
        .as_ref()
        .and_then(|_| hash_plan(&draft.plan).ok())
        .expect("complete plan hash");

    let execution = maa_mcp_adapter::execution::ExecutionService::new(state.clone());
    let draft_error = execution
        .prepare(&plan_id, expected_hash.as_str())
        .expect_err("draft cannot execute");
    assert_eq!(draft_error.code(), "PLAN_NOT_READY");

    let ready = service.preview(&plan_id).expect("preview plan");
    assert_eq!(ready.plan.state, PlanState::Ready);
    let wrong_hash = "0".repeat(64);
    let hash_error = execution
        .prepare(&plan_id, &wrong_hash)
        .expect_err("wrong hash cannot execute");
    assert_eq!(hash_error.code(), "PLAN_HASH_MISMATCH");

    let prepared = execution
        .prepare(&plan_id, expected_hash.as_str())
        .expect("prepare exact ready plan");
    assert_eq!(prepared.plan_id(), &plan_id);
    assert_eq!(prepared.plan_hash().as_str(), expected_hash.as_str());
    assert!(prepared.summary().contains("配置档：default"));

    let persisted = FilePlanRepository::new(state)
        .expect("reopen plan repository")
        .load(&plan_id)
        .expect("load ready plan");
    assert_eq!(persisted.state, PlanState::Ready);
}

#[tokio::test]
async fn accepted_execution_launches_detached_process_and_persists_identity() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let (plan_id, expected_hash, _summary) = ready_plan(&state);
    let fake = configured_background_fake(temp.path());
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());
    let execution = maa_mcp_adapter::execution::ExecutionService::new(state.clone());
    let prepared = execution
        .prepare(&plan_id, &expected_hash)
        .expect("prepare execution");

    let launched = execution
        .launch(&prepared, &adapter)
        .await
        .expect("launch execution");

    assert_eq!(launched.record.status, RunStatus::Running);
    assert_eq!(launched.record.plan_id, plan_id);
    assert_eq!(launched.record.plan_hash.as_str(), expected_hash);
    assert!(launched.record.pid > 0);
    assert!(launched.record.task_path.is_absolute());
    assert!(launched.record.stdout_path.is_absolute());
    assert!(launched.record.stderr_path.is_absolute());
    assert_eq!(
        FileRunRepository::new(state.clone())
            .expect("run repository")
            .load(&launched.record.run_id)
            .expect("persisted run"),
        launched.record
    );
    assert!(
        maa_mcp_adapter::platform::process_identity_matches(
            launched.record.pid,
            launched.record.process_start_identity,
        )
        .expect("check process identity")
    );
    assert_eq!(
        FilePlanRepository::new(state.clone())
            .expect("plan repository")
            .load(&plan_id)
            .expect("running plan")
            .state,
        PlanState::Running
    );

    maa_mcp_adapter::platform::terminate_if_matches(
        launched.record.pid,
        launched.record.process_start_identity,
    )
    .expect("terminate fake maa");
    release_active_run_lock(&state, &launched.record.run_id).expect("release active lock");
}

#[tokio::test]
async fn two_concurrent_launches_allow_at_most_one_active_run() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let (plan_id, expected_hash, _summary) = ready_plan(&state);
    let fake = configured_background_fake(temp.path());
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());
    let first_service = maa_mcp_adapter::execution::ExecutionService::new(state.clone());
    let second_service = maa_mcp_adapter::execution::ExecutionService::new(state.clone());
    let first = first_service
        .prepare(&plan_id, &expected_hash)
        .expect("first prepare");
    let second = second_service
        .prepare(&plan_id, &expected_hash)
        .expect("second prepare");

    let (first_result, second_result) = tokio::join!(
        first_service.launch(&first, &adapter),
        second_service.launch(&second, &adapter)
    );
    let successes = usize::from(first_result.is_ok()) + usize::from(second_result.is_ok());
    assert_eq!(successes, 1);
    let failure = if first_result.is_err() {
        first_result.as_ref().expect_err("first failure")
    } else {
        second_result.as_ref().expect_err("second failure")
    };
    assert_eq!(failure.code(), "ACTIVE_RUN_EXISTS");
    let launched = first_result
        .or(second_result)
        .expect("one successful launch");

    maa_mcp_adapter::platform::terminate_if_matches(
        launched.record.pid,
        launched.record.process_start_identity,
    )
    .expect("terminate fake maa");
    release_active_run_lock(&state, &launched.record.run_id).expect("release active lock");
}

#[tokio::test]
async fn plan_change_after_prepare_is_rejected_before_any_probe_or_launch() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let (plan_id, expected_hash, _summary) = ready_plan(&state);
    let fake = configured_background_fake(temp.path());
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());
    let execution = maa_mcp_adapter::execution::ExecutionService::new(state.clone());
    let prepared = execution
        .prepare(&plan_id, &expected_hash)
        .expect("prepare execution");
    let plan_service =
        PlanService::new(FilePlanRepository::new(state.clone()).expect("plan repository"));
    plan_service
        .update(
            &plan_id,
            prepared.revision(),
            PlanPatch::AddTask {
                task: DailyTask::CloseDown(Default::default()),
            },
        )
        .expect("change ready plan");

    let error = execution
        .launch(&prepared, &adapter)
        .await
        .expect_err("changed plan must not launch");

    assert_eq!(error.code(), "PLAN_CHANGED");
    assert!(
        !fake.executable().with_extension("args").exists(),
        "maa-cli must not even be probed after snapshot mismatch"
    );
    ActiveRunLock::acquire(&state, RunId::new()).expect("lock released after mismatch");
}

#[tokio::test]
async fn task_tampering_after_dry_run_prevents_actual_execution() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let (plan_id, expected_hash, _summary) = ready_plan(&state);
    let fake = configured_background_fake(temp.path());
    fake.set_mode("tamper-task");
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());
    let execution = maa_mcp_adapter::execution::ExecutionService::new(state.clone());
    let prepared = execution
        .prepare(&plan_id, &expected_hash)
        .expect("prepare execution");

    let error = execution
        .launch(&prepared, &adapter)
        .await
        .expect_err("tampered task must not launch");

    assert_eq!(error.code(), "TASK_FILE_CHANGED");
    assert!(
        fake.recorded_invocations().iter().all(|args| {
            args.as_slice() == ["version"]
                || args.as_slice() == ["dir", "config"]
                || args.as_slice() == ["run", "--help"]
                || args.as_slice() == ["--help"]
                || args.iter().any(|arg| arg == "--dry-run")
        }),
        "no actual execution invocation is allowed"
    );
    assert_eq!(
        FilePlanRepository::new(state.clone())
            .expect("plan repository")
            .load(&plan_id)
            .expect("rolled back plan")
            .state,
        PlanState::Ready
    );
    ActiveRunLock::acquire(&state, RunId::new()).expect("lock released after tamper");
}

#[test]
fn background_spawn_failure_returns_no_process() {
    let temp = tempfile::tempdir().expect("temporary logs");
    let stdout = temp.path().join("stdout.log");
    let stderr = temp.path().join("stderr.log");
    let missing = temp.path().join("missing-maa.exe");

    let result = spawn_background(&RunId::new(), &missing, &[], &stdout, &stderr);

    assert!(matches!(result, Err(ProcessError::Spawn(_))));
}

#[tokio::test]
async fn run_record_failure_terminates_spawned_process_and_releases_lock() {
    let temp = tempfile::tempdir().expect("temporary state");
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    let (plan_id, expected_hash, _summary) = ready_plan(&state);
    let fake = configured_background_fake(temp.path());
    fake.set_mode("block-record");
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());
    let execution = maa_mcp_adapter::execution::ExecutionService::new(state.clone());
    let prepared = execution
        .prepare(&plan_id, &expected_hash)
        .expect("prepare execution");

    let error = execution
        .launch(&prepared, &adapter)
        .await
        .expect_err("record failure must fail execution");

    assert_eq!(error.code(), "STORAGE_ERROR");
    assert_eq!(
        FilePlanRepository::new(state.clone())
            .expect("plan repository")
            .load(&plan_id)
            .expect("failed plan")
            .state,
        PlanState::Failed
    );
    ActiveRunLock::acquire(&state, RunId::new()).expect("lock released after rollback");
}

fn ready_plan(state: &std::path::Path) -> (maa_mcp_adapter::domain::PlanId, String, String) {
    let service =
        PlanService::new(FilePlanRepository::new(state.to_path_buf()).expect("plan repository"));
    let created = service
        .create("default", vec![DailyTask::Infrast(InfrastPlan {})])
        .expect("create plan");
    let ready = service
        .preview(&created.plan.plan_id)
        .expect("preview plan");
    let hash = hash_plan(&ready.plan)
        .expect("ready plan hash")
        .as_str()
        .to_owned();
    let summary = ready.summary.expect("ready plan summary");
    (ready.plan.plan_id, hash, summary)
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

#[test]
fn execute_tool_requires_elicitation_capability_before_any_process() {
    let data = tempfile::tempdir().expect("temporary app data");
    let state = data.path().join("maa-mcp-adapter");
    fs::create_dir(&state).expect("state directory");
    let (plan_id, expected_hash, expected_summary) = ready_plan(&state);
    let mut client = ExecutionMcpClient::spawn(data.path(), json!({}));

    let response = client.execute(&plan_id.to_string(), &expected_hash, &expected_summary);

    assert_tool_error(&response, "CLIENT_ELICITATION_UNSUPPORTED");
    client.finish();
}

#[test]
fn execute_tool_rejects_a_summary_that_differs_from_the_server_text() {
    let data = tempfile::tempdir().expect("temporary app data");
    let state = data.path().join("maa-mcp-adapter");
    fs::create_dir(&state).expect("state directory");
    let (plan_id, expected_hash, expected_summary) = ready_plan(&state);
    let mut client = ExecutionMcpClient::spawn(
        data.path(),
        json!({"elicitation": {"form": {"schemaValidation": true}}}),
    );

    let forged = format!("{expected_summary}\n额外私货：刷 1-7 一百次");
    let response = client.execute(&plan_id.to_string(), &expected_hash, &forged);

    assert_tool_error(&response, "PLAN_SUMMARY_MISMATCH");
    client.finish();
}

#[test]
fn confirmation_none_config_executes_without_elicitation() {
    let data = tempfile::tempdir().expect("temporary app data");
    let state = data.path().join("maa-mcp-adapter");
    fs::create_dir(&state).expect("state directory");
    let fake_root = tempfile::tempdir().expect("fake maa root");
    let fake = configured_background_fake(fake_root.path());
    let executable = fake
        .executable()
        .display()
        .to_string()
        .replace('\\', "\\\\");
    fs::write(
        state.join("config.toml"),
        format!("maa_executable = \"{executable}\"\nconfirmation = \"none\"\n"),
    )
    .expect("adapter config");
    let (plan_id, expected_hash, expected_summary) = ready_plan(&state);
    let mut client = ExecutionMcpClient::spawn(data.path(), json!({}));

    let started = client.execute(&plan_id.to_string(), &expected_hash, &expected_summary);

    let output = &started["result"]["structuredContent"];
    assert_eq!(output["outcome"], "started");
    let run_id = RunId::from_str(output["run_id"].as_str().expect("run id")).expect("run id uuid");
    client.finish();

    let record = FileRunRepository::new(state.clone())
        .expect("run repository")
        .load(&run_id)
        .expect("run record");
    maa_mcp_adapter::platform::terminate_if_matches(record.pid, record.process_start_identity)
        .expect("terminate background fake");
    release_active_run_lock(&state, &run_id).expect("release active lock");
}

#[test]
fn execute_tool_handles_decline_cancel_and_accept_with_exact_summary() {
    let data = tempfile::tempdir().expect("temporary app data");
    let state = data.path().join("maa-mcp-adapter");
    fs::create_dir(&state).expect("state directory");
    let fake_root = tempfile::tempdir().expect("fake maa root");
    let fake = configured_background_fake(fake_root.path());
    let executable = fake
        .executable()
        .display()
        .to_string()
        .replace('\\', "\\\\");
    fs::write(
        state.join("config.toml"),
        format!("maa_executable = \"{executable}\"\n"),
    )
    .expect("adapter config");
    let (plan_id, expected_hash, expected_summary) = ready_plan(&state);
    let capabilities = json!({
        "elicitation": {"form": {"schemaValidation": true}}
    });
    let mut client = ExecutionMcpClient::spawn(data.path(), capabilities);

    let decline_request =
        client.begin_execute(&plan_id.to_string(), &expected_hash, &expected_summary, 40);
    assert_elicitation_request(&decline_request, &expected_hash);
    client.respond_elicitation(&decline_request, "decline", None);
    let declined = client.read_response(40);
    assert_eq!(
        declined["result"]["structuredContent"]["outcome"],
        "declined"
    );
    assert!(!fake.executable().with_extension("args").exists());

    let cancel_request =
        client.begin_execute(&plan_id.to_string(), &expected_hash, &expected_summary, 41);
    assert_elicitation_request(&cancel_request, &expected_hash);
    client.respond_elicitation(&cancel_request, "cancel", None);
    let cancelled = client.read_response(41);
    assert_eq!(
        cancelled["result"]["structuredContent"]["outcome"],
        "cancelled"
    );
    assert!(!fake.executable().with_extension("args").exists());

    let accept_request =
        client.begin_execute(&plan_id.to_string(), &expected_hash, &expected_summary, 42);
    assert_elicitation_request(&accept_request, &expected_hash);
    client.respond_elicitation(&accept_request, "accept", Some(true));
    let started = client.read_response(42);
    let output = &started["result"]["structuredContent"];
    assert_eq!(output["outcome"], "started");
    assert_eq!(output["status"], "running");
    assert!(output["pid"].as_u64().is_some_and(|pid| pid > 0));
    assert!(
        output["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("配置档：default"))
    );
    assert!(
        output["stdout_path"]
            .as_str()
            .is_some_and(|path| path.ends_with("stdout.log"))
    );
    assert!(
        output["stderr_path"]
            .as_str()
            .is_some_and(|path| path.ends_with("stderr.log"))
    );
    let run_id = RunId::from_str(output["run_id"].as_str().expect("run id")).expect("valid run id");

    client.finish();

    let record = FileRunRepository::new(state.clone())
        .expect("run repository")
        .load(&run_id)
        .expect("run record");
    assert!(
        maa_mcp_adapter::platform::process_identity_matches(
            record.pid,
            record.process_start_identity,
        )
        .expect("process survives MCP server exit")
    );
    maa_mcp_adapter::platform::terminate_if_matches(record.pid, record.process_start_identity)
        .expect("terminate background fake");
    release_active_run_lock(&state, &run_id).expect("release active lock");
}

#[test]
fn plan_change_during_elicitation_invalidates_acceptance() {
    let data = tempfile::tempdir().expect("temporary app data");
    let state = data.path().join("maa-mcp-adapter");
    fs::create_dir(&state).expect("state directory");
    let fake_root = tempfile::tempdir().expect("fake maa root");
    let fake = configured_background_fake(fake_root.path());
    let executable = fake
        .executable()
        .display()
        .to_string()
        .replace('\\', "\\\\");
    fs::write(
        state.join("config.toml"),
        format!("maa_executable = \"{executable}\"\n"),
    )
    .expect("adapter config");
    let (plan_id, expected_hash, expected_summary) = ready_plan(&state);
    let mut client = ExecutionMcpClient::spawn(
        data.path(),
        json!({"elicitation": {"form": {"schemaValidation": true}}}),
    );

    let request = client.begin_execute(&plan_id.to_string(), &expected_hash, &expected_summary, 50);
    assert_elicitation_request(&request, &expected_hash);
    let service =
        PlanService::new(FilePlanRepository::new(state.clone()).expect("plan repository"));
    let current = FilePlanRepository::new(state.clone())
        .expect("plan repository")
        .load(&plan_id)
        .expect("ready plan");
    service
        .update(
            &plan_id,
            current.revision,
            PlanPatch::AddTask {
                task: DailyTask::CloseDown(Default::default()),
            },
        )
        .expect("change plan during elicitation");
    client.respond_elicitation(&request, "accept", Some(true));

    let response = client.read_response(50);
    assert_tool_error(&response, "PLAN_CHANGED");
    assert!(!fake.executable().with_extension("args").exists());
    client.finish();
}

#[test]
fn full_stdio_daily_flow_reaches_status_log_and_stop() {
    let data = tempfile::tempdir().expect("temporary app data");
    let state = data.path().join("maa-mcp-adapter");
    fs::create_dir(&state).expect("state directory");
    let fake_root = tempfile::tempdir().expect("fake maa root");
    let fake = configured_background_fake(fake_root.path());
    let executable = fake
        .executable()
        .display()
        .to_string()
        .replace('\\', "\\\\");
    fs::write(
        state.join("config.toml"),
        format!("maa_executable = \"{executable}\"\nredaction_patterns = [\"background\"]\n"),
    )
    .expect("adapter config");
    let mut client = ExecutionMcpClient::spawn(
        data.path(),
        json!({"elicitation": {"form": {"schemaValidation": true}}}),
    );

    let environment = client.call_tool(60, "check_environment", json!({}));
    assert_eq!(
        environment["result"]["structuredContent"]["can_execute"],
        true
    );
    let options = client.call_tool(61, "get_daily_options", json!({"intent": "刷日常"}));
    assert_eq!(
        options["result"]["structuredContent"]["task_types"]
            .as_array()
            .expect("task types")
            .len(),
        7
    );
    let created = client.call_tool(
        62,
        "create_daily_plan",
        json!({
            "profile": "default",
            "tasks": [{"type": "infrast", "params": {}}]
        }),
    );
    let plan_id = created["result"]["structuredContent"]["plan_id"]
        .as_str()
        .expect("plan id")
        .to_owned();
    let updated = client.call_tool(
        63,
        "update_daily_plan",
        json!({
            "plan_id": plan_id,
            "expected_revision": 0,
            "patch": {
                "operation": "add_task",
                "task": {"type": "award", "params": {
                    "daily": true,
                    "weekly": false,
                    "mail": false,
                    "event": false
                }}
            }
        }),
    );
    assert_eq!(updated["result"]["structuredContent"]["revision"], 1);
    let preview = client.call_tool(64, "preview_daily_plan", json!({"plan_id": plan_id}));
    let expected_hash = preview["result"]["structuredContent"]["plan_hash"]
        .as_str()
        .expect("plan hash")
        .to_owned();
    let expected_summary = preview["result"]["structuredContent"]["summary"]
        .as_str()
        .expect("plan summary")
        .to_owned();

    let elicitation = client.begin_execute(&plan_id, &expected_hash, &expected_summary, 65);
    assert_elicitation_request(&elicitation, &expected_hash);
    client.respond_elicitation(&elicitation, "accept", Some(true));
    let started = client.read_response(65);
    let run_id = started["result"]["structuredContent"]["run_id"]
        .as_str()
        .expect("run id")
        .to_owned();
    let status = client.call_tool(66, "get_run_status", json!({"run_id": run_id}));
    assert_eq!(status["result"]["structuredContent"]["status"], "running");
    thread::sleep(Duration::from_millis(50));
    let log = client.call_tool(
        67,
        "get_run_log",
        json!({"run_id": run_id, "stream": "stdout"}),
    );
    assert!(
        log["result"]["structuredContent"]["content"]
            .as_str()
            .is_some_and(|content| content.contains("[REDACTED] run started")),
        "{log}"
    );
    let stopped = client.call_tool(68, "stop_run", json!({"run_id": run_id}));
    assert_eq!(stopped["result"]["structuredContent"]["status"], "stopped");
    assert_eq!(
        stopped["result"]["structuredContent"]["stop_method"],
        "forced"
    );
    client.finish();
}

fn assert_elicitation_request(request: &Value, expected_hash: &str) {
    assert_eq!(request["method"], "elicitation/create");
    let params = &request["params"];
    assert!(params["message"].as_str().is_some_and(
        |message| message.contains("配置档：default") && message.contains(expected_hash)
    ));
    assert_eq!(
        params["requestedSchema"]["properties"]["confirmed"]["type"],
        "boolean"
    );
    assert!(
        params["requestedSchema"]["required"]
            .as_array()
            .expect("required fields")
            .iter()
            .any(|field| field == "confirmed")
    );
}

fn assert_tool_error(response: &Value, code: &str) {
    assert_eq!(response["result"]["isError"], true, "{response}");
    assert_eq!(
        response["result"]["structuredContent"]["error"]["code"],
        code
    );
}

struct ExecutionMcpClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_request_id: u64,
}

impl ExecutionMcpClient {
    fn spawn(local_app_data: &std::path::Path, capabilities: Value) -> Self {
        let empty_path = local_app_data.join("empty-path");
        fs::create_dir_all(&empty_path).expect("empty path");
        let mut child = Command::new(env!("CARGO_BIN_EXE_maa-mcp-adapter"))
            .env("LOCALAPPDATA", local_app_data)
            .env("PATH", empty_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn MCP server");
        let stdin = child.stdin.take().expect("MCP stdin");
        let stdout = BufReader::new(child.stdout.take().expect("MCP stdout"));
        let mut client = Self {
            child,
            stdin: Some(stdin),
            stdout,
            next_request_id: 40,
        };
        client.send(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": capabilities,
                "clientInfo": {"name": "execution-test", "version": "0.1.0"}
            }
        }));
        assert_eq!(
            client.read_response(1)["result"]["serverInfo"]["name"],
            "maa-mcp-adapter"
        );
        client.send(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }));
        client
    }

    fn execute(&mut self, plan_id: &str, expected_hash: &str, expected_summary: &str) -> Value {
        let id = self.next_request_id;
        self.next_request_id += 1;
        self.send_execute(plan_id, expected_hash, expected_summary, id);
        self.read_response(id)
    }

    fn call_tool(&mut self, id: u64, name: &str, arguments: Value) -> Value {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }));
        self.read_response(id)
    }

    fn begin_execute(
        &mut self,
        plan_id: &str,
        expected_hash: &str,
        expected_summary: &str,
        id: u64,
    ) -> Value {
        self.send_execute(plan_id, expected_hash, expected_summary, id);
        self.read_message()
    }

    fn send_execute(
        &mut self,
        plan_id: &str,
        expected_hash: &str,
        expected_summary: &str,
        id: u64,
    ) {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "execute_daily_plan",
                "arguments": {
                    "plan_id": plan_id,
                    "expected_plan_hash": expected_hash,
                    "expected_summary": expected_summary
                }
            }
        }));
    }

    fn respond_elicitation(&mut self, request: &Value, action: &str, confirmed: Option<bool>) {
        let mut result = json!({"action": action});
        if let Some(confirmed) = confirmed {
            result["content"] = json!({"confirmed": confirmed});
        }
        let response = json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": result
        });
        self.send(response);
    }

    fn read_response(&mut self, id: u64) -> Value {
        loop {
            let message = self.read_message();
            if message["id"] == id {
                return message;
            }
        }
    }

    fn read_message(&mut self) -> Value {
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).expect("read MCP message");
        assert_ne!(read, 0, "MCP server closed unexpectedly");
        serde_json::from_str(&line).expect("MCP message is JSON")
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().expect("MCP stdin open");
        writeln!(stdin, "{message}").expect("write MCP message");
        stdin.flush().expect("flush MCP message");
    }

    fn finish(mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().expect("poll MCP server") {
                let mut stderr = String::new();
                self.child
                    .stderr
                    .take()
                    .expect("MCP stderr")
                    .read_to_string(&mut stderr)
                    .expect("read MCP stderr");
                assert!(status.success(), "MCP server failed: {stderr}");
                assert!(stderr.is_empty(), "MCP server stderr: {stderr}");
                break;
            }
            if Instant::now() >= deadline {
                self.child.kill().expect("kill timed-out MCP server");
                panic!("MCP server did not exit");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
