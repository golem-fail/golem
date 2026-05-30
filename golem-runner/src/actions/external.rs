use std::collections::HashMap;

use anyhow::{bail, Result};
use golem_driver::PlatformDriver;
use golem_element::glob::glob_match;
use golem_email::ImapPoller;
use golem_parser::Step;
use golem_vars::{ScopeLevel, VarValue, VariableStore};
use regex::Regex;

use crate::context::ExecutionContext;

/// Open a deep link or URL on the device.
pub(crate) async fn handle_open_link(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let url = step
        .params
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("open_link action requires 'url' param"))?;
    driver.open_url(url).await
}

/// Send a push notification to the device.
pub(crate) async fn handle_push_notification(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let title = step
        .params
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let body = step
        .params
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let payload = step
        .params
        .get("payload")
        .and_then(|v| v.as_str());
    driver.push_notification(title, body, payload).await
}

/// Execute a shell command on the host via `sh -c`, optionally saving the output.
///
/// The command is read from the `run` param.
pub(crate) async fn handle_bash(step: &Step, vars: &mut VariableStore) -> Result<()> {
    let command = step
        .params
        .get("run")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("bash action requires 'run' param"))?;

    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Command failed with exit code {}: {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if let Some(ref var_name) = step.save_to {
        vars.set_in_scope(ScopeLevel::Flow, var_name, VarValue::string(&stdout));
    }

    Ok(())
}

/// Execute a project-scoped script file directly (not through `sh -c`).
///
/// - `script` param is required.
/// - Path traversal (`..`) is rejected.
/// - Leading `/` means relative to `ctx.project_root`, otherwise relative to `ctx.flow_dir`.
/// - Optional `args` array of arguments to pass.
/// - If `save_to` is set, stdout and exit_code are stored as an object.
pub(crate) async fn handle_run(
    step: &Step,
    vars: &mut VariableStore,
    ctx: &ExecutionContext<'_>,
) -> Result<()> {
    let script = step
        .params
        .get("script")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("run action requires 'script' param"))?;

    // Reject path traversal
    if script.contains("..") {
        bail!("run action: path traversal ('..') is not allowed in script path");
    }

    // Resolve the script path
    let script_path = if script.starts_with('/') {
        ctx.project_root.join(script.trim_start_matches('/'))
    } else {
        ctx.flow_dir.join(script)
    };

    // Check file exists
    if !script_path.exists() {
        bail!(
            "run action: script not found: {}",
            script_path.display()
        );
    }

    // Parse optional args array
    let args: Vec<&str> = step
        .params
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut cmd = tokio::process::Command::new(&script_path);
    for arg in &args {
        cmd.arg(arg);
    }

    let output = cmd.output().await?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if let Some(ref var_name) = step.save_to {
        let mut obj = HashMap::new();
        obj.insert("stdout".to_string(), VarValue::string(&stdout));
        obj.insert(
            "exit_code".to_string(),
            VarValue::string(exit_code.to_string()),
        );
        vars.set_in_scope(ScopeLevel::Flow, var_name, VarValue::Object(obj));
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Script failed with exit code {}: {}",
            exit_code,
            stderr.trim()
        );
    }

    Ok(())
}

/// Poll an IMAP inbox waiting for an email.
///
/// Reads inbox credentials from the variable store using the `inbox` param as a
/// namespace (e.g. inbox_name.imap_host, inbox_name.imap_port, etc.).
/// Optionally filters by `to` address. Applies `extract` regexes to capture
/// fields from the email body. Stores results under `save_to`.
pub(crate) async fn handle_await_email(step: &Step, vars: &mut VariableStore) -> Result<()> {
    let inbox_name = step
        .params
        .get("inbox")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("await_email action requires 'inbox' param"))?;

    // Look up inbox credentials from variable store
    let inbox_val = vars
        .resolve(inbox_name)
        .map_err(|_| {
            anyhow::anyhow!(
                "await_email: inbox '{}' not found in variables",
                inbox_name
            )
        })?
        .clone();

    let imap_host = inbox_val
        .get_path("imap_host")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("await_email: {inbox_name}.imap_host not found"))?
        .to_string();
    let imap_port = inbox_val
        .get_path("imap_port")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| {
            anyhow::anyhow!("await_email: {inbox_name}.imap_port not found or invalid")
        })?;
    let user = inbox_val
        .get_path("user")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("await_email: {inbox_name}.user not found"))?
        .to_string();
    let pass = inbox_val
        .get_path("pass")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("await_email: {inbox_name}.pass not found"))?
        .to_string();

    let to_filter = step.params.get("to").and_then(|v| v.as_str());
    let timeout = step.timeout.unwrap_or(30000);

    let subject_pattern = step
        .params
        .get("subject")
        .and_then(|v| v.as_str())
        .unwrap_or("*");

    let poller = ImapPoller::new(imap_host, imap_port, user, pass);
    let email = poller.await_email(subject_pattern, timeout, 2000).await?;

    // Filter by `to` if specified
    if let Some(to) = to_filter {
        if !glob_match(to, &email.to) {
            bail!(
                "await_email: email 'to' field {:?} does not match filter {:?}",
                email.to,
                to,
            );
        }
    }

    // Apply extract regexes
    let mut extracted = HashMap::new();
    if let Some(extract_table) = step.params.get("extract").and_then(|v| v.as_table()) {
        for (key, pattern_val) in extract_table {
            if let Some(pattern_str) = pattern_val.as_str() {
                let re = Regex::new(pattern_str).map_err(|e| {
                    anyhow::anyhow!("await_email: invalid regex for '{key}': {e}")
                })?;
                if let Some(caps) = re.captures(&email.body) {
                    let captured = caps
                        .get(1)
                        .map(|m| m.as_str())
                        .unwrap_or_else(|| caps.get(0).map_or("", |m| m.as_str()));
                    extracted.insert(key.clone(), VarValue::string(captured));
                }
            }
        }
    }

    // Store results
    if let Some(ref var_name) = step.save_to {
        let mut obj = HashMap::new();
        obj.insert("body".to_string(), VarValue::string(&email.body));
        obj.insert("subject".to_string(), VarValue::string(&email.subject));
        obj.insert("from".to_string(), VarValue::string(&email.from));
        obj.insert("to".to_string(), VarValue::string(&email.to));
        obj.insert("date".to_string(), VarValue::string(&email.date));
        for (k, v) in extracted {
            obj.insert(k, v);
        }
        vars.set_in_scope(ScopeLevel::Flow, var_name, VarValue::Object(obj));
    }

    Ok(())
}

/// Load a fixture file and store its variables under a namespace.
///
/// - `fixture` param: name of the fixture to load
/// - `as` param: namespace to store the fixture variables under
pub(crate) async fn handle_load_fixture(
    step: &Step,
    vars: &mut VariableStore,
    ctx: &ExecutionContext<'_>,
) -> Result<()> {
    let fixture_name = step
        .params
        .get("fixture")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("load_fixture action requires 'fixture' param"))?;

    let namespace = step
        .params
        .get("as")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("load_fixture action requires 'as' param"))?;

    let mut rng = ctx.rng.lock().map_err(|e| anyhow::anyhow!("rng lock: {e}"))?;

    crate::fixture_loader::load_fixture_into_store(
        fixture_name,
        namespace,
        ctx.flow_dir,
        ctx.project_root,
        vars,
        &mut *rng,
    )
}

/// Make an HTTP request and optionally save the response body.
pub(crate) async fn handle_http(step: &Step, vars: &mut VariableStore, method: &str) -> Result<()> {
    let url = step
        .params
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("{} action requires 'url' param", step.action))?;

    let client = reqwest::Client::new();

    let mut request = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "PATCH" => client.patch(url),
        "DELETE" => client.delete(url),
        _ => bail!("Unsupported HTTP method: {}", method),
    };

    // Add body for methods that support it
    if let Some(body) = step.params.get("body").and_then(|v| v.as_str()) {
        request = request
            .header("Content-Type", "application/json")
            .body(body.to_string());
    }

    // Add custom headers from params
    if let Some(headers) = step.params.get("headers") {
        if let Some(table) = headers.as_table() {
            for (key, value) in table {
                if let Some(val_str) = value.as_str() {
                    request = request.header(key.as_str(), val_str);
                }
            }
        }
    }

    let response = request.send().await?;
    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        bail!(
            "HTTP {} {} returned status {}: {}",
            method,
            url,
            status.as_u16(),
            body
        );
    }

    if let Some(ref var_name) = step.save_to {
        vars.set_in_scope(ScopeLevel::Flow, var_name, VarValue::string(&body));
    }

    Ok(())
}

/// Immediately fail the flow with a message from the step's `text` field.
pub(crate) fn handle_fail(step: &Step) -> Result<()> {
    let message = step
        .on_text
        .as_deref()
        .unwrap_or("Flow failed (no message provided)");
    bail!("{}", message)
}

/// Dismiss the current alert/dialog.
///
/// If the step has a `text` param or `button` param, it is passed as the button
/// label to dismiss with. Otherwise the alert is dismissed with the default action.
/// Accept (positive): tap the last button in the alert (OK, Yes).
pub(crate) async fn handle_accept_alert(
    _step: &Step,
    driver: &dyn PlatformDriver,
    ctx: &crate::context::ExecutionContext<'_>,
) -> Result<()> {
    // Two-phase resolve:
    //
    // 1. In-app dialogs (JS confirms, WKWebView dialogs) show up in
    //    `get_hierarchy()` as alert/sheet elements — tap the positive
    //    button directly.
    // 2. OS-owned dialogs (deep-link "Open in <App>?", permission
    //    prompts) live in SpringBoard's process. We can't safely
    //    query that cross-app from XCTest (cross-app XCUI attach
    //    terminates the harness in iOS 26). Instead the companion
    //    pre-installs a UIInterruptionMonitor that taps the common
    //    positive labels (Open / Allow / OK / Yes); iOS invokes the
    //    handler on the next UI action against the test app. The
    //    `poke_for_system_alert` call below synthesises that action.
    //
    // Idempotent: if no alert surfaces, accept_alert fails — callers
    // who want optional behaviour (warm sims that have already
    // accepted the URL scheme) should set `if_fail = "ignore"`.
    //
    // No internal deadline: the step's timeout (via policy.rs's
    // `tokio::time::timeout`) governs how long we poll. The previous
    // hard-coded 5s gave up before alerts that appeared at ~5-6s
    // under sweep load, while the surrounding step still had budget
    // left to find them.
    let mut poked = false;
    loop {
        let (root, _meta) = driver.get_hierarchy().await?;
        if let Some(alert) = golem_driver::common::find_alert(&root) {
            let buttons = golem_driver::common::find_alert_buttons(&alert);
            if buttons.is_empty() {
                bail!("accept_alert failed: no buttons found in alert");
            }
            // Last button is the positive action (OK, Yes, Open).
            let btn = &buttons[buttons.len() - 1];
            let b = btn.effective_bounds();
            let (x, y) = (b.center_x(), b.center_y());
            ctx.substep(golem_events::SubstepEvent::Tap {
                point: golem_events::Point { x, y },
                element_bounds: Some(golem_events::Rect {
                    x: b.x, y: b.y, width: b.width, height: b.height,
                }),
            });
            driver.tap(x, y).await?;
            return wait_for_alert_gone(driver).await;
        }
        // First miss: poke the test app so the harness's interruption
        // monitor gets a chance to fire and dismiss any pending system
        // dialog. Only poke once — repeated taps would interfere with
        // a test that genuinely has no alert.
        if !poked {
            poked = true;
            let _ = driver.poke_for_system_alert().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

/// Block until the alert window is gone from the hierarchy. Called
/// after `accept_alert` / `dismiss_alert` taps a button — without
/// this, the next step can race a still-dismissing alert: `find_alert`
/// in a subsequent `dismiss_alert` re-matches the lingering window and
/// taps a phantom Cancel that no-ops, then the action returns "success"
/// in tens of ms and the test moves on while the underlying app is
/// actually mid-animation. Subsequent `Show X` tap then races the
/// closing overlay and the new alert never reaches the foreground.
async fn wait_for_alert_gone(driver: &dyn PlatformDriver) -> Result<()> {
    // Bounded poll. The step's surrounding timeout is the hard cap;
    // this loop short-circuits as soon as the alert leaves. A 2.5s
    // ceiling matches the typical native AlertDialog dismiss animation
    // on Android (mostly 200-500ms in practice).
    //
    // Hierarchy fetch errors are tolerated mid-loop: between an alert
    // tap and the next window gaining focus, Android can briefly have
    // no active window and return a 500 "no active window" — that
    // transient state is consistent with the alert being gone, so we
    // count it as success.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(2500);
    while tokio::time::Instant::now() < deadline {
        match driver.get_hierarchy().await {
            Ok((root, _meta)) => {
                if golem_driver::common::find_alert(&root).is_none() {
                    return Ok(());
                }
            }
            Err(_) => {
                // Transient "no active window" or similar — treat as
                // "alert is gone". Don't fail the action over it.
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Ok(())
}

/// Dismiss (negative): tap the first button in the alert (Cancel, No).
/// For single-button alerts, taps the only button.
pub(crate) async fn handle_dismiss_alert(
    _step: &Step,
    driver: &dyn PlatformDriver,
    ctx: &crate::context::ExecutionContext<'_>,
) -> Result<()> {
    // Mirror accept_alert's structure. dismiss_alert only resolves
    // in-app dialogs cleanly — system dialogs are auto-handled by the
    // companion's UIInterruptionMonitor with the *positive* button,
    // not the negative one. The monitor doesn't expose a per-call
    // choice. If a test author needs to assert a particular system
    // dialog appeared and was cancelled, that's a future enhancement
    // (e.g. a configurable monitor verb).
    //
    // No internal deadline — the step's timeout governs (see
    // accept_alert for rationale).
    loop {
        let (root, _meta) = driver.get_hierarchy().await?;
        if let Some(alert) = golem_driver::common::find_alert(&root) {
            let buttons = golem_driver::common::find_alert_buttons(&alert);
            if buttons.is_empty() {
                bail!("dismiss_alert failed: no buttons found in alert");
            }
            // First button is the negative action (Cancel, No).
            let btn = &buttons[0];
            let b = btn.effective_bounds();
            let (x, y) = (b.center_x(), b.center_y());
            ctx.substep(golem_events::SubstepEvent::Tap {
                point: golem_events::Point { x, y },
                element_bounds: Some(golem_events::Rect {
                    x: b.x, y: b.y, width: b.width, height: b.height,
                }),
            });
            driver.tap(x, y).await?;
            return wait_for_alert_gone(driver).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use crate::context::test_ctx;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;
    use std::path::Path;

    // ── open_link calls driver.open_url ───────────────────────────────

    #[tokio::test]
    async fn open_link_calls_driver_open_url() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("open_link");
        step.params.insert(
            "url".to_string(),
            toml::Value::String("myapp://profile/123".to_string()),
        );

        handle_open_link(&step, &driver)
            .await
            .expect("open_link should succeed");

        let calls = driver.get_calls();
        let ol_calls: Vec<_> = calls.iter().filter(|c| c.0 == "open_url").collect();
        assert_eq!(ol_calls.len(), 1);
        assert_eq!(ol_calls[0].1, vec!["myapp://profile/123"]);
    }

    // ── push_notification calls driver.push_notification ──────────────

    #[tokio::test]
    async fn push_notification_calls_driver_push_notification() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("push_notification");
        step.params.insert(
            "title".to_string(),
            toml::Value::String("Test Title".to_string()),
        );
        step.params.insert(
            "body".to_string(),
            toml::Value::String("Test Body".to_string()),
        );
        step.params.insert(
            "payload".to_string(),
            toml::Value::String("notification.json".to_string()),
        );

        handle_push_notification(&step, &driver)
            .await
            .expect("push_notification should succeed");

        let calls = driver.get_calls();
        let pn_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.0 == "push_notification")
            .collect();
        assert_eq!(pn_calls.len(), 1);
        assert_eq!(
            pn_calls[0].1,
            vec!["Test Title", "Test Body", "notification.json"]
        );
    }

    // ── bash/run executes command and captures output ─────────────────

    #[tokio::test]
    async fn bash_executes_command_and_captures_output() {
        let mut vars = make_vars();

        let mut step = make_step("bash");
        step.params.insert(
            "run".to_string(),
            toml::Value::String("echo hello".to_string()),
        );

        handle_bash(&step, &mut vars)
            .await
            .expect("bash should succeed");

        // No save_to, so no variable should be set
        assert!(!vars.has("output"));
    }

    // ── bash with save_to stores result in vars ──────────────────────

    #[tokio::test]
    async fn bash_with_save_to_stores_result_in_vars() {
        let mut vars = make_vars();

        let mut step = make_step("bash");
        step.params.insert(
            "run".to_string(),
            toml::Value::String("echo hello".to_string()),
        );
        step.save_to = Some("output".to_string());

        handle_bash(&step, &mut vars)
            .await
            .expect("bash should succeed");

        let saved = vars.get("output").expect("output variable should exist");
        assert_eq!(saved, &VarValue::string("hello"));
    }

    // ── run action executes a project-scoped script file ──────────────

    #[tokio::test]
    async fn run_action_executes_script_file() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let script_path = tmp.path().join("hello.sh");
        std::fs::write(&script_path, "#!/bin/sh\necho world\n").expect("write script");

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
                .expect("set permissions");
        }

        let mut vars = make_vars();
        let ctx = test_ctx(tmp.path());

        let mut step = make_step("run");
        step.params.insert(
            "script".to_string(),
            toml::Value::String("hello.sh".to_string()),
        );
        step.save_to = Some("result".to_string());

        handle_run(&step, &mut vars, &ctx)
            .await
            .expect("run should succeed");

        let saved = vars.get("result").expect("result variable should exist");
        let obj = saved.as_object().expect("result SHALL be an object");
        assert_eq!(
            obj.get("stdout"),
            Some(&VarValue::string("world")),
            "stdout SHALL contain the script output"
        );
    }

    // ── http_get dispatches correctly ────────────────────────────────

    #[tokio::test]
    async fn http_get_requires_url_param() {
        let mut vars = make_vars();

        let step = make_step("http_get");
        // No url param

        let result = handle_http(&step, &mut vars, "GET").await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("requires 'url' param"),
            "error should mention url param required, got: {err_msg}"
        );
    }

    // ── http_post dispatches correctly ───────────────────────────────

    #[tokio::test]
    async fn http_post_requires_url_param() {
        let mut vars = make_vars();

        let step = make_step("http_post");
        // No url param

        let result = handle_http(&step, &mut vars, "POST").await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("requires 'url' param"),
            "error should mention url param required, got: {err_msg}"
        );
    }

    // ── fail action always returns error with message ────────────────

    #[tokio::test]
    async fn fail_action_always_returns_error_with_message() {
        let mut step = make_step("fail");
        step.on_text = Some("Should not reach here".to_string());

        let result = handle_fail(&step);
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Should not reach here"),
            "error should contain the fail message, got: {err_msg}"
        );
    }

    // ── open_link without url param returns error ─────────────────────

    #[tokio::test]
    async fn open_link_without_url_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("open_link");
        // No url param

        let result = handle_open_link(&step, &driver).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("url"),
            "error should mention url param, got: {err_msg}"
        );
    }

    // ── bash without run param returns error ──────────────────────────

    #[tokio::test]
    async fn bash_without_run_param_returns_error() {
        let mut vars = make_vars();

        let step = make_step("bash");
        // No run param

        let result = handle_bash(&step, &mut vars).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("run"),
            "error SHALL mention 'run' param, got: {err_msg}"
        );
    }

    // ── dismiss_alert tests ───────────────────────────────────────────

    #[tokio::test]
    async fn dismiss_alert_taps_first_button() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let mut alert = make_element("Alert", Bounds::new(50, 200, 275, 150));
        let cancel_btn = make_element_with_text("Button", "Cancel", Bounds::new(60, 310, 80, 30));
        let ok_btn = make_element_with_text("Button", "OK", Bounds::new(200, 310, 80, 30));
        alert.children.push(cancel_btn);
        alert.children.push(ok_btn);
        root.children.push(alert);
        let driver = MockPlatformDriver::new(root);

        let step = make_step("dismiss_alert");
        let ctx = test_ctx(Path::new("."));
        handle_dismiss_alert(&step, &driver, &ctx)
            .await
            .expect("dismiss_alert SHALL succeed");

        let calls = driver.get_calls();
        let tap_calls: Vec<_> = calls.iter().filter(|c| c.0 == "tap").collect();
        assert_eq!(tap_calls.len(), 1, "SHALL tap exactly one button");
        // First button (Cancel) center: x=60+40=100, y=310+15=325
        assert_eq!(tap_calls[0].1, vec!["100", "325"], "SHALL tap first button (negative)");
    }

    // ── accept_alert tests ───────────────────────────────────────────

    #[tokio::test]
    async fn accept_alert_taps_last_button() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        let mut alert = make_element("Alert", Bounds::new(50, 200, 275, 150));
        let cancel_btn = make_element_with_text("Button", "Cancel", Bounds::new(60, 310, 80, 30));
        let ok_btn = make_element_with_text("Button", "OK", Bounds::new(200, 310, 80, 30));
        alert.children.push(cancel_btn);
        alert.children.push(ok_btn);
        root.children.push(alert);
        let driver = MockPlatformDriver::new(root);

        let step = make_step("accept_alert");
        let ctx = test_ctx(Path::new("."));
        handle_accept_alert(&step, &driver, &ctx)
            .await
            .expect("accept_alert SHALL succeed");

        let calls = driver.get_calls();
        let tap_calls: Vec<_> = calls.iter().filter(|c| c.0 == "tap").collect();
        assert_eq!(tap_calls.len(), 1, "SHALL tap exactly one button");
        // Last button (OK) center: x=200+40=240, y=310+15=325
        assert_eq!(tap_calls[0].1, vec!["240", "325"], "SHALL tap last button (positive)");
    }

    #[tokio::test]
    async fn dismiss_alert_fails_when_no_alert() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("dismiss_alert");
        let ctx = test_ctx(Path::new("."));
        // dismiss_alert polls until step timeout for an alert. With no
        // alert present, the action either errors out (preserved fast
        // path) or polls until the wall-clock cap below trips.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            handle_dismiss_alert(&step, &driver, &ctx),
        )
        .await;
        match result {
            Ok(Ok(_)) => panic!("dismiss_alert SHALL NOT succeed when no alert is displayed"),
            Ok(Err(_)) | Err(_) => {}
        }
    }

    // ── run action path validation tests ──────────────────────────────

    #[tokio::test]
    async fn run_action_rejects_path_traversal() {
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("run");
        step.params.insert(
            "script".to_string(),
            toml::Value::String("../etc/passwd".to_string()),
        );

        let result = handle_run(&step, &mut vars, &ctx).await;
        assert!(result.is_err(), "run SHALL reject path traversal");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("path traversal"),
            "error SHALL mention path traversal, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn run_action_rejects_nonexistent_script() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let mut vars = make_vars();
        let ctx = test_ctx(tmp.path());

        let mut step = make_step("run");
        step.params.insert(
            "script".to_string(),
            toml::Value::String("nonexistent.sh".to_string()),
        );

        let result = handle_run(&step, &mut vars, &ctx).await;
        assert!(result.is_err(), "run SHALL fail when script does not exist");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("not found"),
            "error SHALL mention script not found, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn run_action_without_script_param_returns_error() {
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("run");
        // No script param

        let result = handle_run(&step, &mut vars, &ctx).await;
        assert!(result.is_err(), "run SHALL require 'script' param");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("script"),
            "error SHALL mention 'script' param, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn run_action_resolves_leading_slash_to_project_root() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let script_path = tmp.path().join("scripts").join("tool.sh");
        std::fs::create_dir_all(script_path.parent().expect("has parent")).expect("create dir");
        std::fs::write(&script_path, "#!/bin/sh\necho rooted\n").expect("write script");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
                .expect("set permissions");
        }

        let mut vars = make_vars();
        let ctx = test_ctx(tmp.path());

        let mut step = make_step("run");
        step.params.insert(
            "script".to_string(),
            toml::Value::String("/scripts/tool.sh".to_string()),
        );
        step.save_to = Some("out".to_string());

        handle_run(&step, &mut vars, &ctx)
            .await
            .expect("run SHALL resolve leading / to project_root");

        let saved = vars.get("out").expect("out variable should exist");
        let obj = saved.as_object().expect("result SHALL be an object");
        assert_eq!(
            obj.get("stdout"),
            Some(&VarValue::string("rooted")),
            "stdout SHALL contain the script output"
        );
    }

    // ── await_email requires inbox param ──────────────────────────────

    #[tokio::test]
    async fn await_email_requires_inbox_param() {
        let mut vars = make_vars();

        let step = make_step("await_email");
        // No inbox param

        let result = handle_await_email(&step, &mut vars).await;
        assert!(result.is_err(), "await_email SHALL require 'inbox' param");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("inbox"),
            "error SHALL mention 'inbox' param, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn await_email_fails_when_inbox_not_in_vars() {
        let mut vars = make_vars();

        let mut step = make_step("await_email");
        step.params.insert(
            "inbox".to_string(),
            toml::Value::String("test_inbox".to_string()),
        );

        let result = handle_await_email(&step, &mut vars).await;
        assert!(result.is_err(), "await_email SHALL fail when inbox not in vars");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("not found"),
            "error SHALL mention inbox not found, got: {err_msg}"
        );
    }

    // ── load_fixture requires params ──────────────────────────────────

    #[tokio::test]
    async fn load_fixture_requires_fixture_param() {
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("load_fixture");
        // No fixture param

        let result = handle_load_fixture(&step, &mut vars, &ctx).await;
        assert!(result.is_err(), "load_fixture SHALL require 'fixture' param");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("fixture"),
            "error SHALL mention 'fixture' param, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn load_fixture_requires_as_param() {
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("load_fixture");
        step.params.insert(
            "fixture".to_string(),
            toml::Value::String("some_fixture".to_string()),
        );
        // No 'as' param

        let result = handle_load_fixture(&step, &mut vars, &ctx).await;
        assert!(result.is_err(), "load_fixture SHALL require 'as' param");
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("as"),
            "error SHALL mention 'as' param, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn load_fixture_loads_vars_into_store() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let fixtures_dir = tmp.path().join("__fixtures__");
        std::fs::create_dir_all(&fixtures_dir).expect("create fixtures dir");
        std::fs::write(
            fixtures_dir.join("user.toml"),
            "[vars]\nemail = \"test@example.com\"\nname = \"Test User\"\n",
        )
        .expect("write fixture");

        let mut vars = make_vars();
        let ctx = test_ctx(tmp.path());

        let mut step = make_step("load_fixture");
        step.params.insert(
            "fixture".to_string(),
            toml::Value::String("user".to_string()),
        );
        step.params.insert(
            "as".to_string(),
            toml::Value::String("account".to_string()),
        );

        handle_load_fixture(&step, &mut vars, &ctx)
            .await
            .expect("load_fixture SHALL succeed");

        let account = vars
            .resolve("account")
            .expect("account SHALL exist in store");
        let obj = account.as_object().expect("account SHALL be an object");
        assert_eq!(
            obj.get("email"),
            Some(&VarValue::string("test@example.com")),
            "email SHALL be loaded from fixture"
        );
        assert_eq!(
            obj.get("name"),
            Some(&VarValue::string("Test User")),
            "name SHALL be loaded from fixture"
        );
    }
}
