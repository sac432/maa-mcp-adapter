use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tempfile::TempDir;

use maa_mcp_adapter::domain::{
    DailyPlan, DailyTask, InfrastPlan, ProcessStartIdentity, RunId, RunRecord, RunStatus,
};
use maa_mcp_adapter::planning::hash::hash_plan;
use maa_mcp_adapter::storage::{FileRunRepository, RunRepository};

mod support;

use support::fake_maa::FakeMaa;

#[test]
fn stdio_server_initializes_and_lists_tools() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_maa-mcp-adapter"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn maa-mcp-adapter");

    let requests = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {
                    "name": "stage-zero-test",
                    "version": "0.1.0"
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    ];

    {
        let stdin = child.stdin.as_mut().expect("child stdin");
        for request in requests {
            writeln!(stdin, "{request}").expect("write MCP request");
        }
    }
    drop(child.stdin.take());

    let output = wait_for_output(child, Duration::from_secs(5));
    assert!(
        output.status.success(),
        "server failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .expect("stdout is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout line is JSON"))
        .collect();

    let initialize = response_with_id(&responses, 1);
    assert_eq!(
        initialize["result"]["serverInfo"]["name"],
        "maa-mcp-adapter"
    );
    assert_eq!(
        initialize["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        initialize["result"]["capabilities"]["tools"].is_object(),
        "server must declare tools capability"
    );
    assert!(
        initialize["result"]["instructions"]
            .as_str()
            .is_some_and(|instructions| instructions.contains("确认")),
        "server must provide Chinese confirmation instructions"
    );

    let tools = response_with_id(&responses, 2);
    let listed_tools = tools["result"]["tools"]
        .as_array()
        .expect("tools/list result must contain an array");
    let names = listed_tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "cancel_daily_plan",
            "check_environment",
            "collect_diagnostics",
            "create_daily_plan",
            "execute_daily_plan",
            "get_daily_options",
            "get_run_log",
            "get_run_status",
            "preview_daily_plan",
            "setup_environment",
            "stop_run",
            "update_daily_plan",
        ]
    );

    for tool in listed_tools {
        assert!(
            tool["description"]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "every tool needs a description: {tool}"
        );
        assert_eq!(
            tool["inputSchema"]["additionalProperties"], false,
            "top-level input must reject unknown fields: {tool}"
        );
        if tool["name"] != "execute_daily_plan" && tool["name"] != "stop_run" {
            assert_eq!(
                tool["annotations"]["openWorldHint"], false,
                "planning tools must not access the open world: {tool}"
            );
        }
    }

    for name in ["check_environment", "get_daily_options"] {
        let tool = listed_tools
            .iter()
            .find(|tool| tool["name"] == name)
            .expect("read-only tool");
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert_eq!(tool["annotations"]["idempotentHint"], true);
    }

    for name in ["create_daily_plan", "preview_daily_plan"] {
        let tool = listed_tools
            .iter()
            .find(|tool| tool["name"] == name)
            .expect("non-destructive state-changing tool");
        assert_eq!(tool["annotations"]["readOnlyHint"], false);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
    }

    for name in ["update_daily_plan", "cancel_daily_plan"] {
        let tool = listed_tools
            .iter()
            .find(|tool| tool["name"] == name)
            .expect("destructive local state-changing tool");
        assert_eq!(tool["annotations"]["readOnlyHint"], false);
        assert_eq!(tool["annotations"]["destructiveHint"], true);
    }

    let execute = listed_tools
        .iter()
        .find(|tool| tool["name"] == "execute_daily_plan")
        .expect("execution tool");
    assert_eq!(execute["annotations"]["readOnlyHint"], false);
    assert_eq!(execute["annotations"]["destructiveHint"], true);
    assert_eq!(execute["annotations"]["idempotentHint"], false);
    assert_eq!(execute["annotations"]["openWorldHint"], true);

    for name in ["get_run_status", "get_run_log"] {
        let tool = listed_tools
            .iter()
            .find(|tool| tool["name"] == name)
            .expect("run query tool");
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert_eq!(tool["annotations"]["idempotentHint"], true);
        assert_eq!(tool["annotations"]["openWorldHint"], false);
    }

    let stop = listed_tools
        .iter()
        .find(|tool| tool["name"] == "stop_run")
        .expect("stop tool");
    assert_eq!(stop["annotations"]["readOnlyHint"], false);
    assert_eq!(stop["annotations"]["destructiveHint"], true);
    assert_eq!(stop["annotations"]["idempotentHint"], false);
    assert_eq!(stop["annotations"]["openWorldHint"], true);

    for name in [
        "update_daily_plan",
        "preview_daily_plan",
        "cancel_daily_plan",
    ] {
        let tool = listed_tools
            .iter()
            .find(|tool| tool["name"] == name)
            .expect("idempotent state-changing tool");
        assert_eq!(tool["annotations"]["idempotentHint"], true);
    }

    let create = listed_tools
        .iter()
        .find(|tool| tool["name"] == "create_daily_plan")
        .expect("create tool");
    assert_eq!(create["annotations"]["idempotentHint"], false);
    let create_schema = serde_json::to_string(&create["inputSchema"]).expect("serialize schema");
    assert!(
        create_schema
            .matches("\"additionalProperties\":false")
            .count()
            >= 8,
        "top-level input and every task parameter object must be strict: {create_schema}"
    );
    assert!(!create_schema.contains("\"stone\""));
}

#[test]
fn stage_five_tools_complete_a_strict_planning_flow() {
    let mut client = McpClient::spawn();
    client.initialize();

    let incomplete = client.call(
        10,
        "create_daily_plan",
        json!({"intent": "帮我启动游戏刷日常"}),
    );
    assert_eq!(incomplete["result"]["isError"], false);
    let incomplete = &incomplete["result"]["structuredContent"];
    assert_eq!(incomplete["state"], "draft");
    assert_eq!(incomplete["revision"], 0);
    assert!(
        incomplete["missing_fields"]
            .as_array()
            .expect("missing fields")
            .iter()
            .any(|field| field["field"] == "profile")
    );
    assert_eq!(incomplete["next_question"], "请选择要使用的配置档。");
    assert!(
        incomplete["validation_errors"]
            .as_array()
            .expect("validation errors")
            .iter()
            .any(|error| error["code"] == "EMPTY_PLAN")
    );

    let unknown = client.call(11, "get_daily_options", json!({"unexpected": true}));
    assert_tool_error(&unknown, "INVALID_ARGUMENTS");

    let forbidden = client.call(
        12,
        "create_daily_plan",
        json!({
            "profile": "default",
            "tasks": [{
                "type": "fight",
                "params": {
                    "category": "main",
                    "stage": "1-7",
                    "times": 1,
                    "medicine": 0,
                    "stone": 1
                }
            }]
        }),
    );
    assert_tool_error(&forbidden, "INVALID_ARGUMENTS");

    let created = client.call(
        13,
        "create_daily_plan",
        json!({
            "profile": "default",
            "tasks": [{"type": "infrast", "params": {}}]
        }),
    );
    let created = &created["result"]["structuredContent"];
    let plan_id = created["plan_id"]
        .as_str()
        .expect("created plan id")
        .to_owned();
    assert_eq!(created["state"], "draft");
    assert_eq!(created["revision"], 0);
    assert!(
        created["missing_fields"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );

    let updated = client.call(
        14,
        "update_daily_plan",
        json!({
            "plan_id": plan_id,
            "expected_revision": 0,
            "patch": {
                "operation": "add_task",
                "task": {"type": "close_down", "params": {}}
            }
        }),
    );
    let updated = &updated["result"]["structuredContent"];
    assert_eq!(updated["state"], "draft");
    assert_eq!(updated["revision"], 1);
    assert_eq!(updated["tasks"].as_array().expect("tasks").len(), 2);

    let previewed = client.call(15, "preview_daily_plan", json!({"plan_id": plan_id}));
    let previewed = &previewed["result"]["structuredContent"];
    assert_eq!(previewed["state"], "ready");
    assert_eq!(previewed["revision"], 2);
    assert_eq!(
        previewed["plan_hash"].as_str().expect("plan hash").len(),
        64
    );
    assert!(
        previewed["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("关闭游戏"))
    );

    let cancelled = client.call(
        16,
        "cancel_daily_plan",
        json!({"plan_id": plan_id, "expected_revision": 2}),
    );
    let cancelled = &cancelled["result"]["structuredContent"];
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(cancelled["revision"], 3);

    let stale_revision = client.call(
        17,
        "cancel_daily_plan",
        json!({"plan_id": plan_id, "expected_revision": 2}),
    );
    assert_tool_error(&stale_revision, "REVISION_CONFLICT");

    let invalid_id = client.call(
        18,
        "preview_daily_plan",
        json!({"plan_id": "not-a-plan-id"}),
    );
    assert_tool_error(&invalid_id, "INVALID_PLAN_ID");

    let missing_plan = client.call(
        19,
        "preview_daily_plan",
        json!({"plan_id": "00000000-0000-4000-8000-000000000000"}),
    );
    assert_tool_error(&missing_plan, "PLAN_NOT_FOUND");

    client.finish();
}

#[test]
fn environment_and_daily_options_are_safe_without_maa() {
    let mut client = McpClient::spawn();
    client.initialize();

    let environment = client.call(20, "check_environment", json!({}));
    let environment = &environment["result"]["structuredContent"];
    assert_eq!(environment["executable_found"], false);
    assert_eq!(environment["can_plan"], true);
    assert_eq!(environment["can_execute"], false);
    assert!(
        environment["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|item| item["code"] == "MAA_CLI_NOT_FOUND")
    );
    assert!(environment.get("config").is_none());
    assert!(environment.get("environment_variables").is_none());

    let options = client.call(
        21,
        "get_daily_options",
        json!({"intent": "刷日常", "profile": "default"}),
    );
    let options = &options["result"]["structuredContent"];
    let task_types = options["task_types"].as_array().expect("task types");
    assert_eq!(task_types.len(), 7);
    assert_eq!(
        options["fight_categories"]
            .as_array()
            .expect("fight categories")
            .len(),
        4
    );
    assert_eq!(options["safety_limits"]["stone"], 0);
    assert_eq!(options["safety_limits"]["forced_credit_shopping"], false);
    assert_eq!(options["safety_limits"]["preserves_credit_blacklist"], true);
    assert_eq!(options["safety_limits"]["account_switching"], false);
    assert_eq!(options["safety_limits"]["paid_purchases"], false);
    assert!(
        options["activity_stage_source"]
            .as_str()
            .is_some_and(|message| message.contains("不访问网络"))
    );

    client.finish();
}

#[test]
fn invalid_config_is_reported_without_starting_planning() {
    let mut client = McpClient::spawn_with_config(Some("unknown_field = true\n"));
    client.initialize();

    let environment = client.call(30, "check_environment", json!({}));
    let environment = &environment["result"]["structuredContent"];
    assert_eq!(environment["can_plan"], false);
    assert_eq!(environment["can_execute"], false);
    assert!(
        environment["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|item| item["code"] == "CONFIG_INVALID")
    );

    let create = client.call(31, "create_daily_plan", json!({"intent": "刷日常"}));
    assert_tool_error(&create, "CONFIG_INVALID");
    assert!(
        create["result"]["structuredContent"]["error"]["details"]
            .as_array()
            .expect("error details")
            .iter()
            .any(|detail| detail
                .as_str()
                .is_some_and(|value| value.contains("config.toml")))
    );

    client.finish();
}

#[test]
fn run_tools_return_strict_status_bounded_log_and_safe_stop_error() {
    let mut client =
        McpClient::spawn_with_config(Some("redaction_patterns = [\"secret-value\"]\n"));
    client.initialize();
    let state = client._data.path().join("maa-mcp-adapter");
    let run_id = RunId::new();
    let run_directory = state.join("runs").join(run_id.to_string());
    std::fs::create_dir_all(&run_directory).expect("run directory");
    std::fs::write(
        run_directory.join("stdout.log"),
        "visible token=secret-value\n",
    )
    .expect("stdout");
    std::fs::write(run_directory.join("stderr.log"), "").expect("stderr");
    let plan = DailyPlan::new("default", vec![DailyTask::Infrast(InfrastPlan {})]);
    let record = RunRecord {
        run_id,
        plan_id: plan.plan_id,
        plan_hash: hash_plan(&plan).expect("plan hash"),
        revision: 0,
        pid: 42,
        process_start_identity: ProcessStartIdentity {
            creation_time_100ns: 123,
        },
        task_path: run_directory.join("task.toml"),
        task_sha256: "a".repeat(64),
        stdout_path: run_directory.join("stdout.log"),
        stderr_path: run_directory.join("stderr.log"),
        status: RunStatus::Succeeded,
        exit_code: Some(0),
        stop_method: None,
        started_at_unix_ms: 1_000,
        finished_at_unix_ms: Some(2_000),
    };
    FileRunRepository::new(state)
        .expect("run repository")
        .create(&record)
        .expect("record");

    let status = client.call(40, "get_run_status", json!({"run_id": run_id.to_string()}));
    let status = &status["result"]["structuredContent"];
    assert_eq!(status["status"], "succeeded");
    assert_eq!(status["exit_code"], 0);

    let log = client.call(
        41,
        "get_run_log",
        json!({"run_id": run_id.to_string(), "stream": "stdout"}),
    );
    let log = &log["result"]["structuredContent"];
    assert_eq!(log["stream"], "stdout");
    assert_eq!(log["content"], "visible token=[REDACTED]");
    assert_eq!(log["lines"], 1);

    let stop = client.call(42, "stop_run", json!({"run_id": run_id.to_string()}));
    assert_tool_error(&stop, "RUN_NOT_RUNNING");
    let invalid = client.call(43, "get_run_status", json!({"run_id": "invalid"}));
    assert_tool_error(&invalid, "INVALID_RUN_ID");
    let unknown = client.call(
        44,
        "get_run_log",
        json!({"run_id": run_id.to_string(), "stream": "stdout", "extra": true}),
    );
    assert_tool_error(&unknown, "INVALID_ARGUMENTS");
    client.finish();
}

#[test]
fn setup_environment_writes_profile_and_emulator_config_once() {
    let data = tempfile::tempdir().expect("temporary app data");
    let fake_root = tempfile::tempdir().expect("fake maa root");
    let fake = FakeMaa::build(fake_root.path());
    let maa_config_dir = fake_root.path().join("maa-config");
    std::fs::create_dir_all(&maa_config_dir).expect("maa config dir");
    fake.set_config_directory(&maa_config_dir);

    let emulator_root = fake_root.path().join("MuMu Player 12");
    std::fs::create_dir_all(emulator_root.join("shell")).expect("shell dir");
    std::fs::create_dir_all(emulator_root.join("nx_main")).expect("nx_main dir");
    let adb = emulator_root.join("shell").join("adb.exe");
    std::fs::write(&adb, "fake").expect("fake adb");
    std::fs::write(
        emulator_root.join("nx_main").join("MuMuManager.exe"),
        "fake",
    )
    .expect("fake manager");

    let gui_dir = fake_root.path().join("maa-gui");
    std::fs::create_dir_all(&gui_dir).expect("gui dir");
    let gui_path = gui_dir.join("gui.json");
    std::fs::write(
        &gui_path,
        serde_json::to_string(&json!({
            "Configurations": {"Default": {
                "Connect.AdbPath": adb.display().to_string(),
                "Connect.Address": "127.0.0.1:16384"
            }},
            "Current": "Default"
        }))
        .expect("serialize gui fixture"),
    )
    .expect("write gui fixture");

    let executable = fake
        .executable()
        .display()
        .to_string()
        .replace('\\', "\\\\");
    let config = format!("maa_executable = \"{executable}\"\nconfirmation = \"none\"\n");
    let mut client = McpClient::spawn_into(data, Some(&config));
    client.initialize();

    let applied = client.call(
        10,
        "setup_environment",
        json!({"maa_gui_config": gui_path.display().to_string()}),
    );
    let output = &applied["result"]["structuredContent"];
    assert_eq!(output["outcome"], "applied", "setup applies: {applied}");
    assert_eq!(output["profile_written"], true);
    assert_eq!(output["adapter_config_updated"], true);
    assert_eq!(output["emulator_configured"], true);

    let profile_path = maa_config_dir.join("profiles").join("mcp-daily.toml");
    let profile = std::fs::read_to_string(&profile_path).expect("profile written");
    assert!(profile.contains("preset = \"ADB\""));
    assert!(profile.contains("address = \"127.0.0.1:16384\""));

    let adapter_config = std::fs::read_to_string(
        client
            ._data
            .path()
            .join("maa-mcp-adapter")
            .join("config.toml"),
    )
    .expect("adapter config");
    assert!(adapter_config.contains("[emulator]"));
    assert!(adapter_config.contains("MuMuManager.exe"));
    assert!(adapter_config.contains("maa_gui_config"));
    assert!(adapter_config.contains("stop_after_run = false"));

    let again = client.call(
        11,
        "setup_environment",
        json!({"maa_gui_config": gui_path.display().to_string()}),
    );
    let output = &again["result"]["structuredContent"];
    assert_eq!(output["outcome"], "applied");
    assert_eq!(
        output["profile_written"], false,
        "profile never overwritten"
    );
    assert_eq!(
        output["adapter_config_updated"], false,
        "second run adds nothing"
    );
    client.finish();
}

#[test]
fn create_daily_plan_derives_defaults_from_saved_maa_settings() {
    let data = tempfile::tempdir().expect("temporary app data");
    let gui_dir = data.path().join("maa-gui");
    std::fs::create_dir(&gui_dir).expect("gui directory");
    std::fs::write(
        gui_dir.join("gui.new.json"),
        r#"{
            "Configurations": {
                "Default": {
                    "TaskQueue": [
                        {"TaskType": "StartUp", "IsEnable": true},
                        {"TaskType": "Recruit", "IsEnable": true, "MaxTimes": 100,
                         "Level3Choose": true, "Level4Choose": true, "Level5Choose": true},
                        {"TaskType": "Infrast", "IsEnable": true},
                        {"TaskType": "Fight", "IsEnable": true, "TimesLimit": 5},
                        {"TaskType": "Mall", "IsEnable": true, "Shopping": true, "VisitFriends": true},
                        {"TaskType": "Award", "IsEnable": true, "Award": true, "Mail": false},
                        {"TaskType": "Roguelike", "IsEnable": true}
                    ]
                }
            },
            "Current": "Default"
        }"#,
    )
    .expect("write task queue fixture");
    let gui_path = gui_dir.join("gui.json");
    std::fs::write(&gui_path, "{}").expect("write gui placeholder");
    let config = format!("maa_gui_config = '{}'\n", gui_path.display());
    let mut client = McpClient::spawn_into(data, Some(&config));
    client.initialize();

    let created = client.call(10, "create_daily_plan", json!({"profile": "default"}));
    let output = &created["result"]["structuredContent"];
    let kinds: Vec<&str> = output["tasks"]
        .as_array()
        .expect("derived tasks")
        .iter()
        .map(|task| task["type"].as_str().expect("task type"))
        .collect();

    assert_eq!(
        kinds,
        ["start_up", "recruit", "infrast", "mall", "award"],
        "enabled whitelisted tasks map in order; fight and roguelike are excluded"
    );
    let recruit = &output["tasks"][1]["params"];
    assert_eq!(recruit["max_recruits"], 4, "MaxTimes 100 clamps to 4");
    assert_eq!(recruit["confirm_recruitment"], true);
    assert_eq!(
        recruit["use_expedited_plans"], false,
        "expedited plans never derive to true"
    );
    assert_eq!(output["state"], "draft");
    assert!(
        output["validation_errors"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    client.finish();
}

#[test]
fn collect_diagnostics_produces_a_redacted_pasteable_report() {
    let mut client =
        McpClient::spawn_with_config(Some("redaction_patterns = [\"secret-value\"]\n"));
    client.initialize();
    let state = client._data.path().join("maa-mcp-adapter");
    let run_id = RunId::new();
    let run_directory = state.join("runs").join(run_id.to_string());
    std::fs::create_dir_all(&run_directory).expect("run directory");
    std::fs::write(run_directory.join("stdout.log"), "").expect("stdout");
    let profile_path = std::env::var("USERPROFILE").expect("USERPROFILE available");
    std::fs::write(
        run_directory.join("stderr.log"),
        format!("token=secret-value at {profile_path}\\AppData\\some.log\nStartUp Start\n"),
    )
    .expect("stderr");
    std::fs::write(
        run_directory.join("task.toml"),
        "[[tasks]]\ntype = \"StartUp\"\n\n[tasks.params]\nstart_game_enabled = true\n",
    )
    .expect("task file");
    let plan = DailyPlan::new("default", vec![DailyTask::Infrast(InfrastPlan {})]);
    let record = RunRecord {
        run_id,
        plan_id: plan.plan_id,
        plan_hash: hash_plan(&plan).expect("plan hash"),
        revision: 0,
        pid: 42,
        process_start_identity: ProcessStartIdentity {
            creation_time_100ns: 123,
        },
        task_path: run_directory.join("task.toml"),
        task_sha256: "a".repeat(64),
        stdout_path: run_directory.join("stdout.log"),
        stderr_path: run_directory.join("stderr.log"),
        status: RunStatus::Failed,
        exit_code: Some(-1073741819),
        stop_method: None,
        started_at_unix_ms: 1_000,
        finished_at_unix_ms: Some(42_000),
    };
    FileRunRepository::new(state)
        .expect("run repository")
        .create(&record)
        .expect("record");

    let response = client.call(50, "collect_diagnostics", json!({}));
    let output = &response["result"]["structuredContent"];

    assert_eq!(
        output["issue_url"],
        "https://github.com/sac432/maa-mcp-adapter/issues/new"
    );
    let report = output["report_markdown"].as_str().expect("report text");
    assert!(report.contains("[REDACTED]"), "redaction applied: {report}");
    assert!(!report.contains("secret-value"), "secret must not leak");
    assert!(report.contains("0xC0000005"), "exit code shown as hex");
    assert!(report.contains("StartUp"), "task types included");
    assert!(report.contains("%USERPROFILE%"), "user paths masked");
    assert!(
        !report
            .to_ascii_lowercase()
            .contains(&profile_path.to_ascii_lowercase()),
        "raw user profile path must not leak"
    );
    assert!(
        report.contains("复现步骤"),
        "issue template sections present"
    );
    client.finish();
}

fn assert_tool_error(response: &Value, code: &str) {
    assert_eq!(response["result"]["isError"], true, "{response}");
    let error = &response["result"]["structuredContent"]["error"];
    assert_eq!(error["code"], code);
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(error["details"].is_array());
    assert!(
        error["recovery"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
}

struct McpClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    _data: TempDir,
}

impl McpClient {
    fn spawn() -> Self {
        Self::spawn_with_config(None)
    }

    fn spawn_with_config(config: Option<&str>) -> Self {
        let data = tempfile::tempdir().expect("temporary app data");
        Self::spawn_into(data, config)
    }

    fn spawn_into(data: TempDir, config: Option<&str>) -> Self {
        let empty_path = data.path().join("empty-path");
        std::fs::create_dir(&empty_path).expect("empty PATH directory");
        if let Some(config) = config {
            let adapter_directory = data.path().join("maa-mcp-adapter");
            std::fs::create_dir(&adapter_directory).expect("adapter config directory");
            std::fs::write(adapter_directory.join("config.toml"), config)
                .expect("write adapter config");
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_maa-mcp-adapter"))
            .env("LOCALAPPDATA", data.path())
            .env("PATH", &empty_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn maa-mcp-adapter");
        let stdin = child.stdin.take().expect("child stdin");
        let stdout = BufReader::new(child.stdout.take().expect("child stdout"));
        Self {
            child,
            stdin: Some(stdin),
            stdout,
            _data: data,
        }
    }

    fn initialize(&mut self) {
        let initialized = self.request(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "stage-five-test", "version": "0.1.0"}
            }),
        );
        assert_eq!(
            initialized["result"]["serverInfo"]["name"],
            "maa-mcp-adapter"
        );
        self.notify("notifications/initialized", json!({}));
    }

    fn call(&mut self, id: u64, name: &str, arguments: Value) -> Value {
        self.request(
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }));
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).expect("read MCP response");
            assert_ne!(read, 0, "server closed before responding to id {id}");
            let response: Value = serde_json::from_str(&line).expect("MCP response is JSON");
            if response["id"] == id {
                return response;
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }));
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().expect("MCP stdin is open");
        writeln!(stdin, "{message}").expect("write MCP request");
        stdin.flush().expect("flush MCP request");
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
                    .expect("child stderr")
                    .read_to_string(&mut stderr)
                    .expect("read child stderr");
                assert!(status.success(), "server failed: {stderr}");
                assert!(stderr.is_empty(), "server wrote to stderr: {stderr}");
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

fn response_with_id(responses: &[Value], id: u64) -> &Value {
    responses
        .iter()
        .find(|response| response["id"] == id)
        .unwrap_or_else(|| panic!("missing response id {id}: {responses:?}"))
}

fn wait_for_output(mut child: Child, timeout: Duration) -> Output {
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll MCP server") {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill timed-out MCP server");
            child.wait().expect("reap timed-out MCP server");
            panic!("MCP server did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(10));
    };

    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .expect("child stdout")
        .read_to_end(&mut stdout)
        .expect("read child stdout");

    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .expect("child stderr")
        .read_to_end(&mut stderr)
        .expect("read child stderr");

    Output {
        status,
        stdout,
        stderr,
    }
}
