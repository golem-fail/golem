use std::collections::HashMap;
use std::time::Duration;

use anyhow::{bail, Result};
use golem_driver::{Direction, PlatformDriver};
use golem_element::glob::glob_match;
use golem_element::selector::find_elements;
use golem_element::Element;
use golem_email::ImapPoller;
use golem_parser::Step;
use golem_vars::{ScopeLevel, VarValue, VariableStore};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use regex::Regex;
use tokio::time::Instant;

use crate::context::ExecutionContext;
use crate::resolution::{build_selector, resolve_element};
use crate::scroll::{scroll_to_element, DEFAULT_MAX_SCROLLS};

/// Resolve an element using all step selectors except `text`.
///
/// For actions like `type` and `backspace`, the step's `text` field holds the
/// value to type rather than a selector. This helper builds a selector that
/// ignores `text`, finds the element, and returns it with tap coordinates.
async fn resolve_element_ignore_text(
    step: &Step,
    driver: &dyn PlatformDriver,
) -> Result<(Element, (f64, f64))> {
    let mut selector = build_selector(step);
    selector.text = None;

    let root = driver.get_hierarchy().await?;
    let results = find_elements(&root, &selector);

    if results.is_empty() {
        bail!(
            "No element found matching selector: text={:?}, id={:?}, type={:?}",
            selector.text,
            selector.id,
            selector.element_type,
        );
    }

    let first = &results[0];
    Ok((first.element.clone(), (first.tap_x, first.tap_y)))
}

/// Dispatch a step to the appropriate action handler.
pub async fn execute_action(
    step: &Step,
    driver: &dyn PlatformDriver,
    vars: &mut VariableStore,
    ctx: &ExecutionContext<'_>,
) -> Result<()> {
    let action = step.action.as_str();
    match action {
        "tap" => handle_tap(step, driver).await,
        "type" => handle_type(step, driver).await,
        "backspace" => handle_backspace(step, driver).await,
        "long_press" => handle_long_press(step, driver).await,
        "swipe" => handle_swipe(step, driver).await,
        "scroll" => handle_scroll(step, driver).await,
        "read" => handle_read(step, driver, vars).await,
        "hide_keyboard" => handle_hide_keyboard(driver).await,
        "assert_visible" => handle_assert_visible(step, driver).await,
        "assert_not_visible" => handle_assert_not_visible(step, driver).await,
        "assert_text" => handle_assert_text(step, driver).await,
        "assert_enabled" => handle_assert_enabled(step, driver).await,
        "assert_checked" => handle_assert_checked(step, driver).await,
        "assert_alert" => handle_assert_alert(step, driver).await,
        "dismiss_alert" => handle_dismiss_alert(step, driver).await,
        "wait" => handle_wait(step, driver).await,
        "wait_not" => handle_wait_not(step, driver).await,
        "fail" => handle_fail(step),
        "launch" => handle_launch(step, driver).await,
        "stop" => handle_stop(step, driver).await,
        "clear_data" => handle_clear_data(step, driver).await,
        "rotate" => handle_rotate(step, driver).await,
        "dark_mode" => handle_dark_mode(step, driver).await,
        "set_location" => handle_set_location(step, driver).await,
        "press" => handle_press(step, driver).await,
        "grant_permission" => handle_grant_permission(step, driver).await,
        "revoke_permission" => handle_revoke_permission(step, driver).await,
        "screenshot" => handle_screenshot(step, driver).await,
        "start_recording" => handle_start_recording(step, driver).await,
        "stop_recording" => handle_stop_recording(step, driver).await,
        "add_media" => handle_add_media(step, driver).await,
        "open_link" => handle_open_link(step, driver).await,
        "push_notification" => handle_push_notification(step, driver).await,
        "bash" => handle_bash(step, vars).await,
        "run" => handle_run(step, vars, ctx).await,
        "await_email" => handle_await_email(step, vars).await,
        "load_fixture" => handle_load_fixture(step, vars, ctx).await,
        "http_get" => handle_http(step, vars, "GET").await,
        "http_post" => handle_http(step, vars, "POST").await,
        "http_put" => handle_http(step, vars, "PUT").await,
        "http_patch" => handle_http(step, vars, "PATCH").await,
        "http_delete" => handle_http(step, vars, "DELETE").await,
        _ => bail!("Unknown action: {}", action),
    }
}

/// Find the target element and tap at its center coordinates.
async fn handle_tap(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (_elem, (x, y)) = resolve_element(step, driver).await?;
    driver.tap(x, y).await
}

/// Find the target element (input field), tap it to focus, then type text.
///
/// The step's `text` field is the string to type, not an element selector,
/// so we resolve the element using other selectors (id, type, etc.).
async fn handle_type(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (_elem, (x, y)) = resolve_element_ignore_text(step, driver).await?;
    driver.tap(x, y).await?;

    let text = step
        .text
        .as_deref()
        .unwrap_or("");
    driver.type_text(text).await
}

/// Find the target element, tap it to focus, then send backspace key presses.
/// `count` defaults to 1 if not specified in `step.params`.
async fn handle_backspace(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (_elem, (x, y)) = resolve_element(step, driver).await?;
    driver.tap(x, y).await?;

    let count = step
        .params
        .get("count")
        .and_then(|v| v.as_integer())
        .map(|n| n as u32)
        .unwrap_or(1);

    driver.backspace(count).await
}

/// Find the target element and long press at its center coordinates.
/// `duration` in ms, defaults to 1000 if not specified in `step.params`.
async fn handle_long_press(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (_elem, (x, y)) = resolve_element(step, driver).await?;

    let duration = step
        .params
        .get("duration")
        .and_then(|v| v.as_integer())
        .map(|n| n as u64)
        .unwrap_or(1000);

    driver.long_press(x, y, duration).await
}

/// Swipe in a direction. May optionally target a specific element (ignored for
/// the swipe call itself, but element resolution validates the element exists).
async fn handle_swipe(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let direction_str = step
        .params
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let direction = match direction_str {
        "up" => Direction::Up,
        "down" => Direction::Down,
        "left" => Direction::Left,
        "right" => Direction::Right,
        other => bail!("Invalid swipe direction: \"{}\"", other),
    };

    driver.swipe(direction).await
}

/// Scroll in a direction until an element matching the step's selectors is found.
///
/// Params:
/// - `direction`: up/down/left/right (default "down")
/// - `max_scrolls`: optional, defaults to `DEFAULT_MAX_SCROLLS`
async fn handle_scroll(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let direction_str = step
        .params
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("down");

    let direction = match direction_str {
        "up" => Direction::Up,
        "down" => Direction::Down,
        "left" => Direction::Left,
        "right" => Direction::Right,
        other => bail!("Invalid scroll direction: \"{}\"", other),
    };

    let max_scrolls = step
        .params
        .get("max_scrolls")
        .and_then(|v| v.as_integer())
        .map(|n| n as u32)
        .unwrap_or(DEFAULT_MAX_SCROLLS);

    let selector = build_selector(step);
    scroll_to_element(&selector, driver, direction, max_scrolls).await?;
    Ok(())
}

/// Find the target element, read its text content, and optionally save it
/// to a variable using `save_to`.
async fn handle_read(
    step: &Step,
    driver: &dyn PlatformDriver,
    vars: &mut VariableStore,
) -> Result<()> {
    let (elem, _coords) = resolve_element(step, driver).await?;

    let text = elem.text.unwrap_or_default();

    if let Some(ref var_name) = step.save_to {
        vars.set_in_scope(ScopeLevel::Flow, var_name, VarValue::string(&text));
    }

    Ok(())
}

/// Dismiss the on-screen keyboard. No element resolution needed.
async fn handle_hide_keyboard(driver: &dyn PlatformDriver) -> Result<()> {
    driver.hide_keyboard().await
}

/// Assert that an element matching the step's selectors exists in the hierarchy.
async fn handle_assert_visible(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    resolve_element(step, driver).await?;
    Ok(())
}

/// Assert that NO element matches the step's selectors.
async fn handle_assert_not_visible(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let selector = build_selector(step);
    let root = driver.get_hierarchy().await?;
    let results = find_elements(&root, &selector);

    if results.is_empty() {
        Ok(())
    } else {
        bail!(
            "Expected no element matching selector but found {}: text={:?}, id={:?}, type={:?}",
            results.len(),
            selector.text,
            selector.id,
            selector.element_type,
        )
    }
}

/// Assert that an element's text exactly matches the expected value.
///
/// The element is located by `id` (or other non-text selectors).
/// The step's `text` field is used as the expected text to compare against.
async fn handle_assert_text(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let expected = step
        .text
        .as_deref()
        .unwrap_or("");

    // Find element using selectors other than text
    let (elem, _coords) = resolve_element_ignore_text(step, driver).await?;
    let actual = elem.text.as_deref().unwrap_or("");

    if actual == expected {
        Ok(())
    } else {
        bail!(
            "assert_text failed: expected {:?}, got {:?}",
            expected,
            actual,
        )
    }
}

/// Assert that the matched element is enabled.
async fn handle_assert_enabled(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (elem, _coords) = resolve_element(step, driver).await?;
    if elem.enabled {
        Ok(())
    } else {
        bail!(
            "assert_enabled failed: element is disabled (id={:?}, text={:?})",
            elem.id,
            elem.text,
        )
    }
}

/// Assert that the matched element is checked.
async fn handle_assert_checked(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let (elem, _coords) = resolve_element(step, driver).await?;
    if elem.checked {
        Ok(())
    } else {
        bail!(
            "assert_checked failed: element is not checked (id={:?}, text={:?})",
            elem.id,
            elem.text,
        )
    }
}

/// Assert that an alert/dialog is currently displayed.
///
/// If the step has a `text` field, the alert element's text is glob-matched
/// against it. If no `text` is provided, any alert satisfies the assertion.
async fn handle_assert_alert(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let alert = driver.get_alert().await?;
    let alert_elem = alert.ok_or_else(|| anyhow::anyhow!("assert_alert failed: no alert is displayed"))?;

    if let Some(ref expected_pattern) = step.text {
        let alert_text = alert_elem.text.as_deref().unwrap_or("");
        if !glob_match(expected_pattern, alert_text) {
            bail!(
                "assert_alert failed: alert text {:?} does not match pattern {:?}",
                alert_text,
                expected_pattern,
            );
        }
    }

    Ok(())
}

/// Dismiss the current alert/dialog.
///
/// If the step has a `text` param or `button` param, it is passed as the button
/// label to dismiss with. Otherwise the alert is dismissed with the default action.
async fn handle_dismiss_alert(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let button = step
        .text
        .as_deref()
        .or_else(|| step.params.get("button").and_then(|v| v.as_str()));

    driver.dismiss_alert(button).await
}

/// Wait for an element to appear, polling the hierarchy until found or timeout.
///
/// Default timeout is 10000ms. Poll interval is 500ms.
async fn handle_wait(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let timeout_ms = step.timeout.unwrap_or(10000);
    let poll_interval = Duration::from_millis(500);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        match resolve_element(step, driver).await {
            Ok(_) => return Ok(()),
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(poll_interval).await;
            }
            Err(e) => return Err(anyhow::anyhow!("Timed out waiting for element: {}", e)),
        }
    }
}

/// Wait for an element to disappear, polling the hierarchy until not found or timeout.
///
/// Default timeout is 10000ms. Poll interval is 500ms.
async fn handle_wait_not(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let timeout_ms = step.timeout.unwrap_or(10000);
    let poll_interval = Duration::from_millis(500);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let selector = build_selector(step);

    loop {
        let root = driver.get_hierarchy().await?;
        let results = find_elements(&root, &selector);

        if results.is_empty() {
            return Ok(());
        }

        if Instant::now() >= deadline {
            bail!(
                "Timed out waiting for element to disappear: text={:?}, id={:?}, type={:?}",
                selector.text,
                selector.id,
                selector.element_type,
            );
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// Immediately fail the flow with a message from the step's `text` field.
fn handle_fail(step: &Step) -> Result<()> {
    let message = step
        .text
        .as_deref()
        .unwrap_or("Flow failed (no message provided)");
    bail!("{}", message)
}

// ── Environment action helpers ──────────────────────────────────────

/// Helper to get the app bundle_id from step.app, falling back to an error.
fn get_app_bundle(step: &Step) -> Result<&str> {
    step.app
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No app specified for {} action", step.action))
}

/// Launch the app with the given bundle_id.
async fn handle_launch(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let bundle_id = get_app_bundle(step)?;
    driver.launch_app(bundle_id).await
}

/// Stop/terminate the app.
async fn handle_stop(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let bundle_id = get_app_bundle(step)?;
    driver.stop_app(bundle_id).await
}

/// Clear app data/cache.
async fn handle_clear_data(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let bundle_id = get_app_bundle(step)?;
    driver.clear_app_data(bundle_id).await
}

/// Set device orientation (portrait or landscape).
async fn handle_rotate(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let orientation = step
        .params
        .get("orientation")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("rotate action requires 'orientation' param"))?;
    driver.set_orientation(orientation).await
}

/// Toggle dark mode on or off.
async fn handle_dark_mode(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let enabled = step
        .params
        .get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| anyhow::anyhow!("dark_mode action requires 'enabled' param"))?;
    driver.set_dark_mode(enabled).await
}

/// Set GPS coordinates on the device.
async fn handle_set_location(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let latitude = step
        .params
        .get("latitude")
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .ok_or_else(|| anyhow::anyhow!("set_location action requires 'latitude' param"))?;
    let longitude = step
        .params
        .get("longitude")
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .ok_or_else(|| anyhow::anyhow!("set_location action requires 'longitude' param"))?;
    driver.set_location(latitude, longitude).await
}

/// Press a hardware button (home, back, volume_up, etc.).
async fn handle_press(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let button = step
        .params
        .get("button")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("press action requires 'button' param"))?;
    driver.press_button(button).await
}

/// Grant an app permission.
async fn handle_grant_permission(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let bundle_id = get_app_bundle(step)?;
    let permission = step
        .params
        .get("permission")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("grant_permission action requires 'permission' param"))?;
    driver.grant_permission(bundle_id, permission).await
}

/// Revoke an app permission.
async fn handle_revoke_permission(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let bundle_id = get_app_bundle(step)?;
    let permission = step
        .params
        .get("permission")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("revoke_permission action requires 'permission' param"))?;
    driver.revoke_permission(bundle_id, permission).await
}

// ── Media, link, and external action helpers ────────────────────────

/// Take a screenshot, optionally saving to a specific path.
async fn handle_screenshot(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let result = driver.screenshot().await?;

    if let Some(path) = step.params.get("path").and_then(|v| v.as_str()) {
        tokio::fs::write(path, &result.data).await?;
    }

    Ok(())
}

/// Start screen recording. Uses a name from the `path` param or defaults to "recording".
async fn handle_start_recording(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let name = step
        .params
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("recording");
    driver.start_recording(name).await
}

/// Stop screen recording, optionally copying the result to a given path.
async fn handle_stop_recording(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let recording_path = driver.stop_recording().await?;

    if let Some(dest) = step.params.get("path").and_then(|v| v.as_str()) {
        tokio::fs::copy(&recording_path, dest).await?;
    }

    Ok(())
}

/// Push a media file to the device.
async fn handle_add_media(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let path = step
        .params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("add_media action requires 'path' param"))?;
    driver.add_media(path).await
}

/// Open a deep link or URL on the device.
async fn handle_open_link(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let url = step
        .params
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("open_link action requires 'url' param"))?;
    driver.open_url(url).await
}

/// Send a push notification to the device.
async fn handle_push_notification(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
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
async fn handle_bash(step: &Step, vars: &mut VariableStore) -> Result<()> {
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
async fn handle_run(
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
async fn handle_await_email(step: &Step, vars: &mut VariableStore) -> Result<()> {
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
async fn handle_load_fixture(
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

    let mut rng = ChaCha8Rng::from_entropy();

    crate::fixture_loader::load_fixture_into_store(
        fixture_name,
        namespace,
        ctx.flow_dir,
        ctx.project_root,
        vars,
        &mut rng,
    )
}

/// Make an HTTP request and optionally save the response body.
async fn handle_http(step: &Step, vars: &mut VariableStore, method: &str) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_ctx;
    use golem_driver::MockPlatformDriver;
    use golem_element::{Bounds, Element};
    use golem_vars::Scope;
    use std::collections::HashMap;
    use std::path::Path;

    // ── Test helpers ──────────────────────────────────────────────────

    fn make_step(action: &str) -> Step {
        Step {
            action: action.to_string(),
            text: None,
            id: None,
            element_type: None,
            index: None,
            enabled: None,
            checked: None,
            clickable: None,
            below: None,
            above: None,
            right_of: None,
            left_of: None,
            child_of: None,
            placeholder: None,
            on_fail: None,
            save_to: None,
            timeout: None,
            retry: None,
            retry_delay: None,
            app: None,
            params: HashMap::new(),
        }
    }

    fn make_element(element_type: &str, bounds: Bounds) -> Element {
        Element {
            element_type: element_type.to_string(),
            text: None,
            id: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds,
            children: Vec::new(),
        }
    }

    fn make_element_with_text(element_type: &str, text: &str, bounds: Bounds) -> Element {
        let mut e = make_element(element_type, bounds);
        e.text = Some(text.to_string());
        e
    }

    fn make_element_with_id(element_type: &str, id: &str, bounds: Bounds) -> Element {
        let mut e = make_element(element_type, bounds);
        e.id = Some(id.to_string());
        e
    }

    fn make_element_with_id_and_text(
        element_type: &str,
        id: &str,
        text: &str,
        bounds: Bounds,
    ) -> Element {
        let mut e = make_element(element_type, bounds);
        e.id = Some(id.to_string());
        e.text = Some(text.to_string());
        e
    }

    fn make_vars() -> VariableStore {
        let mut store = VariableStore::new();
        store.push_scope(Scope::new(ScopeLevel::Flow));
        store
    }

    fn root_with_button(text: &str) -> Element {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        root.children.push(make_element_with_text(
            "Button",
            text,
            Bounds::new(100.0, 200.0, 100.0, 44.0),
        ));
        root
    }

    fn root_with_input(id: &str) -> Element {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        root.children.push(make_element_with_id(
            "TextField",
            id,
            Bounds::new(20.0, 100.0, 300.0, 44.0),
        ));
        root
    }

    // ── 1. tap action finds element and taps at correct coordinates ──

    #[tokio::test]
    async fn tap_action_finds_element_and_taps_at_correct_coordinates() {
        let root = root_with_button("Submit");
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("tap");
        step.text = Some("Submit".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("tap should succeed");

        let calls = driver.get_calls();
        // get_hierarchy + tap
        let tap_calls: Vec<_> = calls.iter().filter(|c| c.0 == "tap").collect();
        assert_eq!(tap_calls.len(), 1);
        // Button bounds: x=100, y=200, w=100, h=44 => center = (150, 222)
        assert_eq!(tap_calls[0].1, vec!["150", "222"]);
    }

    // ── 2. read action captures text into variable ───────────────────

    #[tokio::test]
    async fn read_action_captures_text_into_variable() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "otp-code",
            "123456",
            Bounds::new(50.0, 300.0, 200.0, 30.0),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("read");
        step.id = Some("otp-code".to_string());
        step.save_to = Some("otp".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("read should succeed");

        let saved = vars.get("otp").expect("otp variable should exist");
        assert_eq!(saved, &VarValue::string("123456"));
    }

    // ── 3. type action types text to element ─────────────────────────

    #[tokio::test]
    async fn type_action_types_text_to_element() {
        let root = root_with_input("email");
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("type");
        step.id = Some("email".to_string());
        step.text = Some("user@example.com".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("type should succeed");

        let calls = driver.get_calls();
        // Should have: get_hierarchy, tap (to focus), type_text
        let tap_calls: Vec<_> = calls.iter().filter(|c| c.0 == "tap").collect();
        assert_eq!(tap_calls.len(), 1);
        // TextField bounds: x=20, y=100, w=300, h=44 => center = (170, 122)
        assert_eq!(tap_calls[0].1, vec!["170", "122"]);

        let type_calls: Vec<_> = calls.iter().filter(|c| c.0 == "type_text").collect();
        assert_eq!(type_calls.len(), 1);
        assert_eq!(type_calls[0].1, vec!["user@example.com"]);
    }

    // ── 4. backspace action with count ───────────────────────────────

    #[tokio::test]
    async fn backspace_action_with_count() {
        let root = root_with_input("search");
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("backspace");
        step.id = Some("search".to_string());
        step.params
            .insert("count".to_string(), toml::Value::Integer(5));

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("backspace should succeed");

        let calls = driver.get_calls();
        let bs_calls: Vec<_> = calls.iter().filter(|c| c.0 == "backspace").collect();
        assert_eq!(bs_calls.len(), 1);
        assert_eq!(bs_calls[0].1, vec!["5"]);
    }

    // ── 5. long_press action at element coordinates ──────────────────

    #[tokio::test]
    async fn long_press_action_at_element_coordinates() {
        let root = root_with_button("Item to select");
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("long_press");
        step.text = Some("Item to select".to_string());
        step.params
            .insert("duration".to_string(), toml::Value::Integer(2000));

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("long_press should succeed");

        let calls = driver.get_calls();
        let lp_calls: Vec<_> = calls.iter().filter(|c| c.0 == "long_press").collect();
        assert_eq!(lp_calls.len(), 1);
        // Button center = (150, 222), duration = 2000
        assert_eq!(lp_calls[0].1, vec!["150", "222", "2000"]);
    }

    // ── 6. swipe action with direction ───────────────────────────────

    #[tokio::test]
    async fn swipe_action_with_direction() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("swipe");
        step.params
            .insert("direction".to_string(), toml::Value::String("up".to_string()));

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("swipe should succeed");

        let calls = driver.get_calls();
        let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe").collect();
        assert_eq!(swipe_calls.len(), 1);
        assert_eq!(swipe_calls[0].1, vec!["Up"]);
    }

    // ── 7. hide_keyboard action ──────────────────────────────────────

    #[tokio::test]
    async fn hide_keyboard_action() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("hide_keyboard");

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("hide_keyboard should succeed");

        let calls = driver.get_calls();
        let hk_calls: Vec<_> = calls.iter().filter(|c| c.0 == "hide_keyboard").collect();
        assert_eq!(hk_calls.len(), 1);
    }

    // ── 8. multiple actions in sequence ──────────────────────────────

    #[tokio::test]
    async fn multiple_actions_in_sequence() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        root.children.push(make_element_with_id(
            "TextField",
            "username",
            Bounds::new(20.0, 100.0, 300.0, 44.0),
        ));
        root.children.push(make_element_with_text(
            "Button",
            "Login",
            Bounds::new(100.0, 200.0, 100.0, 44.0),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        // Type into username field
        let mut type_step = make_step("type");
        type_step.id = Some("username".to_string());
        type_step.text = Some("admin".to_string());
        execute_action(&type_step, &driver, &mut vars, &ctx)
            .await
            .expect("type should succeed");

        // Hide keyboard
        let hk_step = make_step("hide_keyboard");
        execute_action(&hk_step, &driver, &mut vars, &ctx)
            .await
            .expect("hide_keyboard should succeed");

        // Tap login button
        let mut tap_step = make_step("tap");
        tap_step.text = Some("Login".to_string());
        execute_action(&tap_step, &driver, &mut vars, &ctx)
            .await
            .expect("tap should succeed");

        let calls = driver.get_calls();
        let method_names: Vec<&str> = calls.iter().map(|c| c.0.as_str()).collect();
        // type: get_hierarchy, tap, type_text
        // hide_keyboard: hide_keyboard
        // tap: get_hierarchy, tap
        assert_eq!(
            method_names,
            vec![
                "get_hierarchy",
                "tap",
                "type_text",
                "hide_keyboard",
                "get_hierarchy",
                "tap",
            ]
        );
    }

    // ── 9. unknown action returns error ──────────────────────────────

    #[tokio::test]
    async fn unknown_action_returns_error() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("fly_to_moon");

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Unknown action"),
            "error should mention unknown action, got: {err_msg}"
        );
        assert!(
            err_msg.contains("fly_to_moon"),
            "error should mention the action name, got: {err_msg}"
        );
    }

    // ── 10. tap on non-existent element returns error ────────────────

    #[tokio::test]
    async fn tap_on_nonexistent_element_returns_error() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("tap");
        step.text = Some("Does Not Exist".to_string());

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("No element found"),
            "error should mention no element found, got: {err_msg}"
        );
    }

    // ── Extra: backspace defaults count to 1 ─────────────────────────

    #[tokio::test]
    async fn backspace_defaults_count_to_one() {
        let root = root_with_input("field");
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("backspace");
        step.id = Some("field".to_string());
        // No count param set

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("backspace should succeed");

        let calls = driver.get_calls();
        let bs_calls: Vec<_> = calls.iter().filter(|c| c.0 == "backspace").collect();
        assert_eq!(bs_calls.len(), 1);
        assert_eq!(bs_calls[0].1, vec!["1"]);
    }

    // ── Extra: long_press defaults duration to 1000 ──────────────────

    #[tokio::test]
    async fn long_press_defaults_duration_to_1000() {
        let root = root_with_button("Hold me");
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("long_press");
        step.text = Some("Hold me".to_string());
        // No duration param set

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("long_press should succeed");

        let calls = driver.get_calls();
        let lp_calls: Vec<_> = calls.iter().filter(|c| c.0 == "long_press").collect();
        assert_eq!(lp_calls.len(), 1);
        assert_eq!(lp_calls[0].1[2], "1000");
    }

    // ── Extra: read without save_to does not error ───────────────────

    #[tokio::test]
    async fn read_without_save_to_does_not_error() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "info",
            "Some text",
            Bounds::new(10.0, 10.0, 100.0, 30.0),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("read");
        step.id = Some("info".to_string());
        // No save_to set

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("read without save_to should succeed");
    }

    // ── Extra: swipe with all four directions ────────────────────────

    #[tokio::test]
    async fn swipe_all_directions() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        for (dir_str, expected) in [
            ("up", "Up"),
            ("down", "Down"),
            ("left", "Left"),
            ("right", "Right"),
        ] {
            driver.clear_calls();
            let mut step = make_step("swipe");
            step.params.insert(
                "direction".to_string(),
                toml::Value::String(dir_str.to_string()),
            );

            execute_action(&step, &driver, &mut vars, &ctx)
                .await
                .unwrap_or_else(|_| panic!("swipe {dir_str} should succeed"));

            let calls = driver.get_calls();
            let swipe_calls: Vec<_> = calls.iter().filter(|c| c.0 == "swipe").collect();
            assert_eq!(swipe_calls.len(), 1);
            assert_eq!(swipe_calls[0].1, vec![expected]);
        }
    }

    // ── Extra: swipe with invalid direction returns error ────────────

    #[tokio::test]
    async fn swipe_invalid_direction_returns_error() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("swipe");
        step.params.insert(
            "direction".to_string(),
            toml::Value::String("diagonal".to_string()),
        );

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Invalid swipe direction"),
            "error should mention invalid direction, got: {err_msg}"
        );
    }

    // ── Assertion action tests ──────────────────────────────────────

    // ── assert_visible succeeds when element exists ─────────────────

    #[tokio::test]
    async fn assert_visible_succeeds_when_element_exists() {
        let root = root_with_button("Welcome");
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("assert_visible");
        step.text = Some("Welcome".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("assert_visible should succeed when element exists");
    }

    // ── assert_visible fails when element not found ─────────────────

    #[tokio::test]
    async fn assert_visible_fails_when_element_not_found() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("assert_visible");
        step.text = Some("Nonexistent".to_string());

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("No element found"),
            "error should mention no element found, got: {err_msg}"
        );
    }

    // ── assert_not_visible succeeds when element not found ──────────

    #[tokio::test]
    async fn assert_not_visible_succeeds_when_element_not_found() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("assert_not_visible");
        step.text = Some("Error*".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("assert_not_visible should succeed when element absent");
    }

    // ── assert_not_visible fails when element exists ────────────────

    #[tokio::test]
    async fn assert_not_visible_fails_when_element_exists() {
        let root = root_with_button("Error occurred");
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("assert_not_visible");
        step.text = Some("Error*".to_string());

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Expected no element"),
            "error should mention unexpected element, got: {err_msg}"
        );
    }

    // ── assert_text succeeds when text matches ──────────────────────

    #[tokio::test]
    async fn assert_text_succeeds_when_text_matches() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "total",
            "$42.00",
            Bounds::new(50.0, 100.0, 200.0, 30.0),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("assert_text");
        step.id = Some("total".to_string());
        step.text = Some("$42.00".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("assert_text should succeed when text matches");
    }

    // ── assert_text fails when text doesn't match ───────────────────

    #[tokio::test]
    async fn assert_text_fails_when_text_does_not_match() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "total",
            "$99.99",
            Bounds::new(50.0, 100.0, 200.0, 30.0),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("assert_text");
        step.id = Some("total".to_string());
        step.text = Some("$42.00".to_string());

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("assert_text failed"),
            "error should mention assert_text failed, got: {err_msg}"
        );
        assert!(
            err_msg.contains("$42.00"),
            "error should mention expected value, got: {err_msg}"
        );
        assert!(
            err_msg.contains("$99.99"),
            "error should mention actual value, got: {err_msg}"
        );
    }

    // ── assert_enabled succeeds when element is enabled ─────────────

    #[tokio::test]
    async fn assert_enabled_succeeds_when_element_is_enabled() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let mut btn = make_element_with_id("Button", "submit-button", Bounds::new(50.0, 200.0, 100.0, 44.0));
        btn.enabled = true;
        root.children.push(btn);
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("assert_enabled");
        step.id = Some("submit-button".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("assert_enabled should succeed when element is enabled");
    }

    // ── assert_enabled fails when element is disabled ───────────────

    #[tokio::test]
    async fn assert_enabled_fails_when_element_is_disabled() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let mut btn = make_element_with_id("Button", "submit-button", Bounds::new(50.0, 200.0, 100.0, 44.0));
        btn.enabled = false;
        root.children.push(btn);
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("assert_enabled");
        step.id = Some("submit-button".to_string());

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("assert_enabled failed"),
            "error should mention assert_enabled failed, got: {err_msg}"
        );
    }

    // ── assert_checked succeeds when element is checked ─────────────

    #[tokio::test]
    async fn assert_checked_succeeds_when_element_is_checked() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let mut cb = make_element_with_id("Checkbox", "agree-checkbox", Bounds::new(50.0, 300.0, 30.0, 30.0));
        cb.checked = true;
        root.children.push(cb);
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("assert_checked");
        step.id = Some("agree-checkbox".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("assert_checked should succeed when element is checked");
    }

    // ── assert_checked fails when element is unchecked ──────────────

    #[tokio::test]
    async fn assert_checked_fails_when_element_is_unchecked() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let mut cb = make_element_with_id("Checkbox", "agree-checkbox", Bounds::new(50.0, 300.0, 30.0, 30.0));
        cb.checked = false;
        root.children.push(cb);
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("assert_checked");
        step.id = Some("agree-checkbox".to_string());

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("assert_checked failed"),
            "error should mention assert_checked failed, got: {err_msg}"
        );
    }

    // ── fail action always returns error with message ────────────────

    #[tokio::test]
    async fn fail_action_always_returns_error_with_message() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("fail");
        step.text = Some("Should not reach here".to_string());

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Should not reach here"),
            "error should contain the fail message, got: {err_msg}"
        );
    }

    // ── wait succeeds immediately when element present ──────────────

    #[tokio::test]
    async fn wait_succeeds_immediately_when_element_present() {
        let root = root_with_button("Welcome");
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("wait");
        step.text = Some("Welcome".to_string());
        step.timeout = Some(1000);

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("wait should succeed immediately when element is present");
    }

    // ── wait_not succeeds immediately when element absent ───────────

    #[tokio::test]
    async fn wait_not_succeeds_immediately_when_element_absent() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("wait_not");
        step.text = Some("Loading...".to_string());
        step.timeout = Some(1000);

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("wait_not should succeed immediately when element is absent");
    }

    // ── Environment action tests ─────────────────────────────────────

    // ── launch action calls driver.launch_app ─────────────────────────

    #[tokio::test]
    async fn launch_action_calls_driver_launch_app() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("launch");
        step.app = Some("com.example.app".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("launch should succeed");

        let calls = driver.get_calls();
        let launch_calls: Vec<_> = calls.iter().filter(|c| c.0 == "launch_app").collect();
        assert_eq!(launch_calls.len(), 1);
        assert_eq!(launch_calls[0].1, vec!["com.example.app"]);
    }

    // ── stop action calls driver.stop_app ─────────────────────────────

    #[tokio::test]
    async fn stop_action_calls_driver_stop_app() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("stop");
        step.app = Some("com.example.app".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("stop should succeed");

        let calls = driver.get_calls();
        let stop_calls: Vec<_> = calls.iter().filter(|c| c.0 == "stop_app").collect();
        assert_eq!(stop_calls.len(), 1);
        assert_eq!(stop_calls[0].1, vec!["com.example.app"]);
    }

    // ── clear_data action calls driver.clear_app_data ─────────────────

    #[tokio::test]
    async fn clear_data_action_calls_driver_clear_app_data() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("clear_data");
        step.app = Some("com.example.app".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("clear_data should succeed");

        let calls = driver.get_calls();
        let clear_calls: Vec<_> = calls.iter().filter(|c| c.0 == "clear_app_data").collect();
        assert_eq!(clear_calls.len(), 1);
        assert_eq!(clear_calls[0].1, vec!["com.example.app"]);
    }

    // ── rotate landscape calls driver.set_orientation ──────────────────

    #[tokio::test]
    async fn rotate_landscape_calls_driver_set_orientation() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("rotate");
        step.params.insert(
            "orientation".to_string(),
            toml::Value::String("landscape".to_string()),
        );

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("rotate should succeed");

        let calls = driver.get_calls();
        let orient_calls: Vec<_> = calls.iter().filter(|c| c.0 == "set_orientation").collect();
        assert_eq!(orient_calls.len(), 1);
        assert_eq!(orient_calls[0].1, vec!["landscape"]);
    }

    // ── dark_mode enabled calls driver.set_dark_mode(true) ────────────

    #[tokio::test]
    async fn dark_mode_enabled_calls_driver_set_dark_mode_true() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("dark_mode");
        step.params
            .insert("enabled".to_string(), toml::Value::Boolean(true));

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("dark_mode should succeed");

        let calls = driver.get_calls();
        let dm_calls: Vec<_> = calls.iter().filter(|c| c.0 == "set_dark_mode").collect();
        assert_eq!(dm_calls.len(), 1);
        assert_eq!(dm_calls[0].1, vec!["true"]);
    }

    // ── dark_mode disabled calls driver.set_dark_mode(false) ──────────

    #[tokio::test]
    async fn dark_mode_disabled_calls_driver_set_dark_mode_false() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("dark_mode");
        step.params
            .insert("enabled".to_string(), toml::Value::Boolean(false));

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("dark_mode should succeed");

        let calls = driver.get_calls();
        let dm_calls: Vec<_> = calls.iter().filter(|c| c.0 == "set_dark_mode").collect();
        assert_eq!(dm_calls.len(), 1);
        assert_eq!(dm_calls[0].1, vec!["false"]);
    }

    // ── set_location calls driver.set_location with correct coords ────

    #[tokio::test]
    async fn set_location_calls_driver_set_location_with_correct_coords() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("set_location");
        step.params
            .insert("latitude".to_string(), toml::Value::Float(35.6762));
        step.params
            .insert("longitude".to_string(), toml::Value::Float(139.6503));

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("set_location should succeed");

        let calls = driver.get_calls();
        let loc_calls: Vec<_> = calls.iter().filter(|c| c.0 == "set_location").collect();
        assert_eq!(loc_calls.len(), 1);
        assert_eq!(loc_calls[0].1, vec!["35.6762", "139.6503"]);
    }

    // ── press home calls driver.press_button("home") ──────────────────

    #[tokio::test]
    async fn press_home_calls_driver_press_button() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("press");
        step.params.insert(
            "button".to_string(),
            toml::Value::String("home".to_string()),
        );

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("press should succeed");

        let calls = driver.get_calls();
        let press_calls: Vec<_> = calls.iter().filter(|c| c.0 == "press_button").collect();
        assert_eq!(press_calls.len(), 1);
        assert_eq!(press_calls[0].1, vec!["home"]);
    }

    // ── grant_permission calls driver.grant_permission ────────────────

    #[tokio::test]
    async fn grant_permission_calls_driver_grant_permission() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("grant_permission");
        step.app = Some("com.example.app".to_string());
        step.params.insert(
            "permission".to_string(),
            toml::Value::String("camera".to_string()),
        );

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("grant_permission should succeed");

        let calls = driver.get_calls();
        let gp_calls: Vec<_> = calls.iter().filter(|c| c.0 == "grant_permission").collect();
        assert_eq!(gp_calls.len(), 1);
        assert_eq!(gp_calls[0].1, vec!["com.example.app", "camera"]);
    }

    // ── revoke_permission calls driver.revoke_permission ──────────────

    #[tokio::test]
    async fn revoke_permission_calls_driver_revoke_permission() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("revoke_permission");
        step.app = Some("com.example.app".to_string());
        step.params.insert(
            "permission".to_string(),
            toml::Value::String("location".to_string()),
        );

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("revoke_permission should succeed");

        let calls = driver.get_calls();
        let rp_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.0 == "revoke_permission")
            .collect();
        assert_eq!(rp_calls.len(), 1);
        assert_eq!(rp_calls[0].1, vec!["com.example.app", "location"]);
    }

    // ── launch without app param returns error ────────────────────────

    #[tokio::test]
    async fn launch_without_app_param_returns_error() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("launch");
        // No app set

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("No app specified"),
            "error should mention no app specified, got: {err_msg}"
        );
    }

    // ── rotate without orientation param returns error ─────────────────

    #[tokio::test]
    async fn rotate_without_orientation_returns_error() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("rotate");
        // No orientation param

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("orientation"),
            "error should mention orientation, got: {err_msg}"
        );
    }

    // ── Media, link, and external action tests ────────────────────────

    // ── screenshot calls driver.screenshot ─────────────────────────────

    #[tokio::test]
    async fn screenshot_calls_driver_screenshot() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("screenshot");

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("screenshot should succeed");

        let calls = driver.get_calls();
        let sc_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(sc_calls.len(), 1);
    }

    // ── start_recording calls driver.start_recording ──────────────────

    #[tokio::test]
    async fn start_recording_calls_driver_start_recording() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("start_recording");

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("start_recording should succeed");

        let calls = driver.get_calls();
        let sr_calls: Vec<_> = calls.iter().filter(|c| c.0 == "start_recording").collect();
        assert_eq!(sr_calls.len(), 1);
        // Default name is "recording"
        assert_eq!(sr_calls[0].1, vec!["recording"]);
    }

    // ── stop_recording calls driver.stop_recording ────────────────────

    #[tokio::test]
    async fn stop_recording_calls_driver_stop_recording() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("stop_recording");

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("stop_recording should succeed");

        let calls = driver.get_calls();
        let sr_calls: Vec<_> = calls.iter().filter(|c| c.0 == "stop_recording").collect();
        assert_eq!(sr_calls.len(), 1);
    }

    // ── add_media calls driver.add_media ──────────────────────────────

    #[tokio::test]
    async fn add_media_calls_driver_add_media() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("add_media");
        step.params.insert(
            "path".to_string(),
            toml::Value::String("test_data/photo.jpg".to_string()),
        );

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("add_media should succeed");

        let calls = driver.get_calls();
        let am_calls: Vec<_> = calls.iter().filter(|c| c.0 == "add_media").collect();
        assert_eq!(am_calls.len(), 1);
        assert_eq!(am_calls[0].1, vec!["test_data/photo.jpg"]);
    }

    // ── open_link calls driver.open_url ───────────────────────────────

    #[tokio::test]
    async fn open_link_calls_driver_open_url() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("open_link");
        step.params.insert(
            "url".to_string(),
            toml::Value::String("myapp://profile/123".to_string()),
        );

        execute_action(&step, &driver, &mut vars, &ctx)
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
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

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

        execute_action(&step, &driver, &mut vars, &ctx)
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
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("bash");
        step.params.insert(
            "run".to_string(),
            toml::Value::String("echo hello".to_string()),
        );

        execute_action(&step, &driver, &mut vars, &ctx)
            .await
            .expect("bash should succeed");

        // No save_to, so no variable should be set
        assert!(!vars.has("output"));
    }

    // ── bash with save_to stores result in vars ──────────────────────

    #[tokio::test]
    async fn bash_with_save_to_stores_result_in_vars() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let mut step = make_step("bash");
        step.params.insert(
            "run".to_string(),
            toml::Value::String("echo hello".to_string()),
        );
        step.save_to = Some("output".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
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

        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(tmp.path());

        let mut step = make_step("run");
        step.params.insert(
            "script".to_string(),
            toml::Value::String("hello.sh".to_string()),
        );
        step.save_to = Some("result".to_string());

        execute_action(&step, &driver, &mut vars, &ctx)
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
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("http_get");
        // No url param

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
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
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("http_post");
        // No url param

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("requires 'url' param"),
            "error should mention url param required, got: {err_msg}"
        );
    }

    // ── unknown action still returns error ───────────────────────────

    #[tokio::test]
    async fn unknown_action_still_returns_error_after_new_actions() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("teleport");

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Unknown action"),
            "error should mention unknown action, got: {err_msg}"
        );
        assert!(
            err_msg.contains("teleport"),
            "error should mention the action name, got: {err_msg}"
        );
    }

    // ── add_media without path param returns error ────────────────────

    #[tokio::test]
    async fn add_media_without_path_returns_error() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("add_media");
        // No path param

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("path"),
            "error should mention path param, got: {err_msg}"
        );
    }

    // ── open_link without url param returns error ─────────────────────

    #[tokio::test]
    async fn open_link_without_url_returns_error() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("open_link");
        // No url param

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("url"),
            "error should mention url param, got: {err_msg}"
        );
    }

    // ── bash without command param returns error ──────────────────────

    #[tokio::test]
    async fn bash_without_command_returns_error() {
        let root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("bash");
        // No command param

        let result = execute_action(&step, &driver, &mut vars, &ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("run"),
            "error should mention 'run' param, got: {err_msg}"
        );
    }
}
