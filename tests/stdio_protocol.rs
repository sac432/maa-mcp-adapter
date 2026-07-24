use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tempfile::TempDir;

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
            "create_daily_plan",
            "execute_daily_plan",
            "get_daily_options",
            "preview_daily_plan",
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
        if tool["name"] != "execute_daily_plan" {
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
