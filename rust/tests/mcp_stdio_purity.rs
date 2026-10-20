//! stdout carries JSON-RPC frames only: one stray line breaks the client's
//! parse of the whole stream. Boots the real binary (no credentials needed:
//! tools register regardless and initialize never touches the network).
//! Port of `StdioPurityTest.kt`, extended to a full handshake.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"stdio-purity-test","version":"1.0.0"}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
const TOOLS_LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
const TOOL_CALL: &str =
    r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_calendars","arguments":{}}}"#;

#[test]
fn every_stdout_line_is_a_json_rpc_message() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_icloud-calendar-mcp"))
        .env_remove("ICLOUD_USERNAME")
        .env_remove("ICLOUD_PASSWORD")
        .env("CALDAV_LOG_HTTP", "true")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("server starts");

    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{INITIALIZE}").unwrap();
    let first = rx.recv_timeout(Duration::from_secs(20)).expect("server wrote nothing to stdout");
    for message in [INITIALIZED, TOOLS_LIST, TOOL_CALL] {
        writeln!(stdin, "{message}").unwrap();
    }
    stdin.flush().unwrap();

    let mut lines = vec![first];
    while lines.len() < 3 {
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(line) => lines.push(line),
            Err(_) => break,
        }
    }
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    assert_eq!(lines.len(), 3, "expected three responses, got {lines:?}");
    let mut ids = Vec::new();
    for line in &lines {
        let message: Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("non-JSON stdout line {line:?}: {e}"));
        assert_eq!(message["jsonrpc"], "2.0", "stdout line: {line}");
        ids.push(message["id"].as_i64().unwrap());
    }
    ids.sort_unstable();
    assert_eq!(ids, [1, 2, 3]);

    let first: Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(first["id"], 1, "the first stdout line answers initialize");
    assert_eq!(first["result"]["serverInfo"]["name"], "icloud-calendar-mcp");
}
