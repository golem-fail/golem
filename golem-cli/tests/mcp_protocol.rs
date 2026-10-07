//! The MCP server end to end in one process: an rmcp client talks to
//! `GolemMcp` over a pipe, the server talks to an in-process daemon, and
//! the session runs on the device-free stub driver.

#![cfg(unix)]

mod common;

use std::time::Duration;

use golem_cli::mcp::{GolemMcp, McpOptions};
use rmcp::model::CallToolRequestParams;
use rmcp::ServiceExt;

/// The tool's content as JSON, and whether it is an error result.
async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tool: &'static str,
    args: serde_json::Value,
) -> (serde_json::Value, bool) {
    let mut params = CallToolRequestParams::new(tool);
    if let serde_json::Value::Object(map) = args {
        params = params.with_arguments(map);
    }
    let result = client
        .call_tool(params)
        .await
        .unwrap_or_else(|e| panic!("{tool}: {e:?}"));
    let v = serde_json::to_value(&result).expect("json");
    let is_error = v["isError"].as_bool().unwrap_or(false);
    (v["content"].clone(), is_error)
}

fn text_of(content: &serde_json::Value) -> String {
    content[0]["text"].as_str().unwrap_or_default().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_tool_works_against_a_stub_session() {
    let dir = tempfile::Builder::new()
        .prefix("gmcp")
        .tempdir_in("/tmp")
        .expect("tempdir");
    std::fs::write(dir.path().join("golem.toml"), common::golem_toml()).expect("golem.toml");
    let socket = dir.path().join("d.sock");
    let _daemon = golem_orchestrator::ipc::start_server(
        &socket,
        &golem_orchestrator::ipc::Identity::current(),
    )
    .await
    .expect("daemon");

    let server = GolemMcp::new(McpOptions {
        socket: socket.clone(),
        project_root: dir.path().to_path_buf(),
        soft_timeout: Duration::from_secs(10),
        stub: true,
    });
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        let service = server.serve(server_io).await.expect("serve");
        let _ = service.waiting().await;
    });
    let client = ().serve(client_io).await.expect("client");

    let tools: Vec<String> = client
        .list_all_tools()
        .await
        .expect("list")
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for name in [
        "devices",
        "session_open",
        "session_close",
        "act",
        "probe",
        "tree",
        "screenshot",
        "wait",
        "status",
        "cancel",
        "actions_help",
        "draft_show",
        "export_flow",
    ] {
        assert!(
            tools.iter().any(|t| t == name),
            "missing tool {name}: {tools:?}"
        );
    }

    let (c, err) = call(
        &client,
        "act",
        serde_json::json!({ "step": r#"{ action = "tap", on_text = "Submit" }"# }),
    )
    .await;
    assert!(err, "a session tool SHALL fail before session_open");
    assert!(text_of(&c).contains("session_open"), "{c}");

    let (c, err) = call(&client, "session_open", serde_json::json!({})).await;
    assert!(!err, "{c}");
    assert!(
        text_of(&c).contains("session open · android/Stub Device"),
        "{c}"
    );

    let (c, err) = call(&client, "tree", serde_json::json!({})).await;
    assert!(!err, "{c}");
    assert!(text_of(&c).starts_with("tree visible"), "{c}");

    let (c, err) = call(
        &client,
        "probe",
        serde_json::json!({ "selector": r#"{ on_text = "Submit" }"# }),
    )
    .await;
    assert!(!err, "{c}");
    assert!(text_of(&c).contains("1 visible match"), "{c}");

    let (c, err) = call(
        &client,
        "act",
        serde_json::json!({ "step": r#"{ action = "tap", on_text = "Submit" }"#, "tree": true }),
    )
    .await;
    assert!(!err, "{c}");
    let t = text_of(&c);
    assert!(t.starts_with("+tap:on_text=\"Submit\""), "{t}");
    assert!(t.contains("tree visible"), "{t}");

    let (c, err) = call(&client, "act", serde_json::json!({ "step": r#"{ action = "assert_visible", on_text = "Submit" }"#, "format": "json" })).await;
    assert!(!err, "{c}");
    let v: serde_json::Value = serde_json::from_str(&text_of(&c)).expect("json result");
    assert_eq!(v["result"]["outcome"], "success", "{v}");

    let (c, err) = call(
        &client,
        "act",
        serde_json::json!({ "step": r#"{ action = "tapp", on_text = "Submit" }"# }),
    )
    .await;
    assert!(err, "a malformed step SHALL be a tool error");
    assert!(text_of(&c).contains("did you mean"), "{c}");

    let (c, err) = call(&client, "screenshot", serde_json::json!({})).await;
    assert!(!err, "{c}");
    assert_eq!(c[0]["type"], "image", "{c}");
    assert_eq!(c[0]["mimeType"], "image/png", "{c}");

    let (c, _) = call(&client, "status", serde_json::json!({})).await;
    assert!(text_of(&c).starts_with("idle"), "{c}");
    let (c, _) = call(&client, "wait", serde_json::json!({})).await;
    assert_eq!(
        c[0]["type"], "image",
        "wait SHALL return the last result: {c}"
    );
    let (c, _) = call(&client, "cancel", serde_json::json!({})).await;
    assert_eq!(text_of(&c), "nothing was running");

    let (c, err) = call(&client, "actions_help", serde_json::json!({})).await;
    assert!(!err);
    assert!(text_of(&c).contains("- tap: Tap an element"), "{c}");
    let (c, err) = call(
        &client,
        "actions_help",
        serde_json::json!({ "action": "type" }),
    )
    .await;
    assert!(!err);
    assert!(text_of(&c).contains(r#"{ action = "type""#), "{c}");

    let (c, err) = call(&client, "draft_show", serde_json::json!({})).await;
    assert!(!err, "{c}");
    assert!(
        text_of(&c).contains(r#"{ action = "tap", on_text = "Submit" },"#),
        "the passing tap SHALL be in the draft: {c}"
    );
    assert!(!text_of(&c).contains("tapp"), "{c}");

    let out = dir.path().join("flows/new.test.toml");
    let (c, err) = call(
        &client,
        "export_flow",
        serde_json::json!({ "path": out.display().to_string() }),
    )
    .await;
    assert!(!err, "{c}");
    assert!(text_of(&c).starts_with("exported "), "{c}");
    let flow = golem_parser::parse_flow(&std::fs::read_to_string(&out).expect("exported file"))
        .expect("the export SHALL parse");
    assert_eq!(flow.block[0].steps[0].action, "tap");
    let (c, err) = call(
        &client,
        "export_flow",
        serde_json::json!({ "path": dir.path().join("golem.toml").display().to_string() }),
    )
    .await;
    assert!(err, "an export over another file SHALL need overwrite: {c}");

    let (c, err) = call(&client, "session_close", serde_json::json!({})).await;
    assert!(!err, "{c}");
    let (c, err) = call(&client, "tree", serde_json::json!({})).await;
    assert!(err, "a closed session SHALL refuse work: {c}");

    client.cancel().await.expect("close client");
}
