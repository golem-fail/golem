//! `golem mcp` as a real process: a whole session writes nothing to stdout
//! but JSON-RPC. One stray line would break the client's protocol stream.

#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn a_session_writes_only_json_rpc_to_stdout() {
    let dir = tempfile::Builder::new()
        .prefix("gmo")
        .tempdir_in("/tmp")
        .expect("tempdir");
    std::fs::write(dir.path().join("golem.toml"), common::golem_toml()).expect("golem.toml");
    let mut child = Command::new(env!("CARGO_BIN_EXE_golem"))
        .args(["mcp", "--stub-session", "--project"])
        .arg(dir.path())
        .env("HOME", dir.path())
        .env("GOLEM_SOCKET", dir.path().join("d.sock"))
        .env("GOLEM_DAEMON_IDLE_SECS", "1")
        .env_remove("GOLEM_DAEMON_IN_PROCESS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("golem mcp");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));

    let mut send = |msg: serde_json::Value| {
        writeln!(stdin, "{msg}").expect("write");
        stdin.flush().expect("flush");
    };
    let mut lines = Vec::new();
    let mut read_reply = |lines: &mut Vec<String>| {
        let mut line = String::new();
        stdout.read_line(&mut line).expect("read");
        lines.push(line.trim_end().to_string());
    };

    send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "stdout-guard", "version": "0" },
        },
    }));
    read_reply(&mut lines);
    send(serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    let calls = [
        ("session_open", serde_json::json!({})),
        ("tree", serde_json::json!({})),
        (
            "act",
            serde_json::json!({ "step": r#"{ action = "tap", on_text = "Submit" }"# }),
        ),
        ("screenshot", serde_json::json!({})),
        ("session_close", serde_json::json!({})),
    ];
    for (i, (name, args)) in calls.iter().enumerate() {
        send(serde_json::json!({
            "jsonrpc": "2.0", "id": i + 2, "method": "tools/call",
            "params": { "name": name, "arguments": args },
        }));
        read_reply(&mut lines);
    }
    drop(stdin);
    let mut rest = String::new();
    while stdout.read_line(&mut rest).expect("read") > 0 {
        lines.push(rest.trim_end().to_string());
        rest.clear();
    }
    let status = child.wait().expect("wait");
    assert!(
        status.success(),
        "golem mcp SHALL exit cleanly on stdin EOF"
    );

    assert_eq!(
        lines.iter().filter(|l| !l.is_empty()).count(),
        6,
        "{lines:#?}"
    );
    for line in lines.iter().filter(|l| !l.is_empty()) {
        let v: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"));
        assert_eq!(v["jsonrpc"], "2.0", "{line}");
        assert!(v.get("error").is_none(), "{line}");
    }
    assert!(lines[1].contains("session open"), "{}", lines[1]);
    assert!(lines[3].contains("+tap"), "{}", lines[3]);
}
