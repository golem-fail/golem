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
        soft_timeout: Some(Duration::from_secs(10)),
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
        "help",
        "draft_show",
        "draft_steps",
        "export_flow",
        "flow_set",
        "apps_set",
        "block_begin",
        "block_link",
        "teardown_add",
        "data_add",
        "comment_add",
        "record_only",
        "step_edit",
        "step_delete",
        "step_move",
        "block_rename",
        "block_delete",
        "options_set",
        "block_set",
        "teardown_delete",
        "mixins_list",
        "app_logs",
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

    let (c, err) = call(
        &client,
        "session_open",
        serde_json::json!({ "platform": "android" }),
    )
    .await;
    assert!(err, "an unknown argument SHALL be refused: {c}");
    assert!(
        text_of(&c).contains("unknown field `platform`, expected one of `os`"),
        "{c}"
    );

    let (c, err) = call(&client, "session_open", serde_json::json!({})).await;
    assert!(!err, "{c}");
    assert!(
        text_of(&c).contains("session open · android/Stub Device"),
        "{c}"
    );
    assert!(
        c.to_string().contains("help() lists the docs"),
        "an opened session SHALL point to help: {c}"
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

    let (c, err) = call(&client, "help", serde_json::json!({})).await;
    assert!(!err);
    assert!(text_of(&c).contains("- act: "), "{c}");
    let (c, err) = call(&client, "help", serde_json::json!({ "topic": "act" })).await;
    assert!(!err);
    assert!(text_of(&c).contains("- interaction: "), "{c}");
    let (c, err) = call(
        &client,
        "help",
        serde_json::json!({ "topic": "act", "item": "type" }),
    )
    .await;
    assert!(!err);
    assert!(text_of(&c).contains(r#"{ action = "type""#), "{c}");
    let (c, err) = call(
        &client,
        "help",
        serde_json::json!({ "topic": "act", "item": "explode" }),
    )
    .await;
    assert!(err, "an unknown item SHALL be a tool error: {c}");

    let (c, err) = call(&client, "draft_show", serde_json::json!({})).await;
    assert!(!err, "{c}");
    assert!(
        text_of(&c).contains(r#"{ action = "tap", on_text = "Submit" },"#),
        "the passing tap SHALL be in the draft: {c}"
    );
    assert!(!text_of(&c).contains("tapp"), "{c}");

    let (c, err) = call(&client, "draft_steps", serde_json::json!({})).await;
    assert!(!err, "{c}");
    assert!(
        text_of(&c).contains(r#"main:1 ✓ { action = "tap", on_text = "Submit" }"#),
        "the passing tap SHALL be listed as passed: {c}"
    );
    let (c, err) = call(
        &client,
        "draft_steps",
        serde_json::json!({ "around": "main:7" }),
    )
    .await;
    assert!(err, "a step the draft does not have SHALL be an error: {c}");

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

    for (tool, args) in [
        (
            "flow_set",
            serde_json::json!({ "name": "Drafted", "tags": ["smoke"] }),
        ),
        (
            "apps_set",
            serde_json::json!({ "app": { "name": "app", "bundle": golem_driver::stub::STUB_BUNDLE_ID, "devices": [{ "os": "android:latest" }] } }),
        ),
        ("block_begin", serde_json::json!({ "name": "second" })),
        ("comment_add", serde_json::json!({ "text": "Check it" })),
        (
            "record_only",
            serde_json::json!({ "step": r#"{ action = "tap", on_text = "Maybe" }"#, "comment": "error path" }),
        ),
        (
            "block_link",
            serde_json::json!({ "block": "main", "next": "second" }),
        ),
        (
            "teardown_add",
            serde_json::json!({ "step": r#"{ action = "screenshot" }"# }),
        ),
        ("data_add", serde_json::json!({ "row": { "who": "Ada" } })),
    ] {
        let (c, err) = call(&client, tool, args).await;
        assert!(!err, "{tool}: {c}");
        assert!(text_of(&c).starts_with("draft updated"), "{tool}: {c}");
    }
    let (c, err) = call(
        &client,
        "record_only",
        serde_json::json!({ "step": r#"{ action = "tapp" }"# }),
    )
    .await;
    assert!(
        err,
        "a malformed step SHALL be refused even when it does not run: {c}"
    );
    let (c, _) = call(&client, "mixins_list", serde_json::json!({})).await;
    assert!(text_of(&c).contains("no mixins"), "{c}");
    let all = dir.path().join("flows/all.test.toml");
    let (c, err) = call(
        &client,
        "export_flow",
        serde_json::json!({ "path": all.display().to_string() }),
    )
    .await;
    assert!(!err, "{c}");
    assert!(
        text_of(&c).contains("unverified (never run in this form):\n  second:1 "),
        "{c}"
    );
    let flow =
        golem_parser::parse_flow(&std::fs::read_to_string(&all).expect("read")).expect("parse");
    assert_eq!(flow.flow.name, "Drafted");
    assert_eq!(flow.block.len(), 2);
    assert_eq!(flow.block[0].next.as_deref(), Some("second"));
    assert_eq!(flow.teardown[0].steps.len(), 1);
    assert_eq!(flow.data.len(), 1);

    for (tool, args, shows) in [
        (
            "step_edit",
            serde_json::json!({ "at": "main:1", "comment": "Submit the form" }),
            "main:1 ✓",
        ),
        (
            "step_move",
            serde_json::json!({ "from": "second:1", "to": "main:2" }),
            "main:2 ?",
        ),
        (
            "step_delete",
            serde_json::json!({ "at": "main:2" }),
            "main:1 ✓",
        ),
        (
            "block_rename",
            serde_json::json!({ "name": "second", "to": "third" }),
            "[third]",
        ),
    ] {
        let (c, err) = call(&client, tool, args).await;
        assert!(!err, "{tool}: {c}");
        let t = text_of(&c);
        assert!(t.starts_with("draft updated\n"), "{tool}: {t}");
        assert!(t.contains(shows), "{tool} SHALL show {shows}: {t}");
    }
    let (c, err) = call(
        &client,
        "block_delete",
        serde_json::json!({ "name": "third" }),
    )
    .await;
    assert!(err, "a block that next names SHALL stay: {c}");
    assert!(text_of(&c).contains("block \"main\" next"), "{c}");

    for (tool, args) in [
        (
            "options_set",
            serde_json::json!({ "options": { "step_timeout": 8000 } }),
        ),
        (
            "block_set",
            serde_json::json!({ "name": "third", "fields": { "record": true } }),
        ),
        (
            "teardown_add",
            serde_json::json!({ "step": r#"{ action = "stop", app = "app" }"# }),
        ),
        ("teardown_delete", serde_json::json!({ "n": 2 })),
    ] {
        let (c, err) = call(&client, tool, args).await;
        assert!(!err, "{tool}: {c}");
    }
    let (c, _) = call(&client, "draft_show", serde_json::json!({})).await;
    let draft = text_of(&c);
    assert!(
        draft.contains("[flow.options]\nstep_timeout = 8000"),
        "{draft}"
    );
    assert!(draft.contains("record = true"), "{draft}");
    assert!(!draft.contains(r#"action = "stop""#), "{draft}");
    assert!(draft.contains(r#"{ action = "screenshot" }"#), "{draft}");
    let (c, err) = call(
        &client,
        "options_set",
        serde_json::json!({ "options": { "step_timout": 1 } }),
    )
    .await;
    assert!(err, "an unknown option SHALL be refused: {c}");

    let (c, err) = call(
        &client,
        "app_logs",
        serde_json::json!({ "filter": "marker", "since": 60 }),
    )
    .await;
    assert!(!err, "{c}");
    let t = text_of(&c);
    assert!(
        !t.contains("crash["),
        "the filter SHALL apply to crash lines too: {t}"
    );
    assert!(t.contains("golem-marker stub"), "{t}");
    let (c, _) = call(&client, "app_logs", serde_json::json!({})).await;
    assert!(text_of(&c).contains("crash[1]:"), "{c}");

    let (c, err) = call(&client, "session_close", serde_json::json!({})).await;
    assert!(!err, "{c}");
    let (c, err) = call(&client, "tree", serde_json::json!({})).await;
    assert!(err, "a closed session SHALL refuse work: {c}");

    client.cancel().await.expect("close client");
}

#[tokio::test]
async fn a_client_with_a_short_limit_gets_a_shorter_soft_timeout() {
    let dir = tempfile::Builder::new()
        .prefix("gmcp")
        .tempdir_in("/tmp")
        .expect("tempdir");
    let server = GolemMcp::new(McpOptions {
        socket: dir.path().join("d.sock"),
        project_root: dir.path().to_path_buf(),
        soft_timeout: None,
        stub: true,
    });
    let watched = server.clone();
    let (server_io, client_io) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        let service = server.serve(server_io).await.expect("serve");
        let _ = service.waiting().await;
    });
    // GitHub Copilot CLI waits 30 s for one call.
    let copilot = rmcp::model::InitializeRequestParams::new(
        rmcp::model::ClientCapabilities::default(),
        rmcp::model::Implementation::new("github-copilot-developer", "1.0.62"),
    );
    let client = copilot.serve(client_io).await.expect("client");
    assert_eq!(watched.soft_timeout(), Duration::from_secs(20));
    client.cancel().await.expect("close client");
}
