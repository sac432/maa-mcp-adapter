use std::{
    io::{Read, Write},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

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
    assert_eq!(tools["result"]["tools"], json!([]));
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
