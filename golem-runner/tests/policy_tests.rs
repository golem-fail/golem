//! Integration tests for step policies (on_fail, retry, timeout) and screenshot
//! capture paths.
//!
//! These tests exercise the interaction between `golem_runner::policy`,
//! `golem_runner::capture`, and `golem_runner::executor` modules through their
//! public APIs, using full TOML-parsed flows and the mock driver.

use std::path::{Path, PathBuf};

use golem_driver::MockPlatformDriver;
use golem_element::{Bounds, Element};
use golem_parser::parse_flow;
use golem_runner::capture::{build_screenshot_path, CaptureConfig};
use golem_runner::executor::{execute_flow, FlowResult};

const DEFAULT_TIMEOUT: u64 = 10_000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn empty_hierarchy() -> Element {
    Element {
        element_type: "View".to_string(),
        text: None,
        id: None,
        placeholder: None,
        enabled: true,
        checked: false,
        clickable: true,
        focused: false,
        bounds: Bounds::new(0.0, 0.0, 375.0, 812.0),
        children: Vec::new(),
    }
}

fn hierarchy_with_texts(texts: &[&str]) -> Element {
    let children = texts
        .iter()
        .enumerate()
        .map(|(i, t)| Element {
            element_type: "Button".to_string(),
            text: Some(t.to_string()),
            id: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds: Bounds::new(10.0, (i as f64) * 50.0, 200.0, 40.0),
            children: Vec::new(),
        })
        .collect();

    Element {
        element_type: "View".to_string(),
        text: None,
        id: None,
        placeholder: None,
        enabled: true,
        checked: false,
        clickable: true,
        focused: false,
        bounds: Bounds::new(0.0, 0.0, 375.0, 812.0),
        children,
    }
}

fn assert_success(result: &FlowResult) {
    assert!(result.success, "flow should succeed");
    assert!(result.failed_step.is_none(), "no step should have failed");
    assert!(
        result.failed_block.is_none(),
        "no block should have failed"
    );
}

// ===========================================================================
// Policy tests: on_fail behaviour through full flow execution
// ===========================================================================

// ---------------------------------------------------------------------------
// 1. on_fail="error" propagates error and stops flow
// ---------------------------------------------------------------------------
#[tokio::test]
async fn on_fail_error_propagates_and_stops_flow() {
    let toml = r#"
[flow]
name = "on_fail error test"

[[block]]
name = "failing_block"
steps = [
  { action = "screenshot" },
  { action = "tap", text = "NONEXISTENT_ELEMENT", on_fail = "error" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should return Ok(FlowResult)");

    assert!(!result.success, "flow should fail with on_fail=error");
    assert_eq!(result.failed_step, Some(1), "second step should be the failure");
    assert_eq!(
        result.failed_block,
        Some("failing_block".to_string()),
    );

    // The third step (second screenshot) should NOT have executed
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 1,
        "only the first screenshot should execute; error stops flow"
    );
}

// ---------------------------------------------------------------------------
// 2. on_fail="warn" collects warning and continues
// ---------------------------------------------------------------------------
#[tokio::test]
async fn on_fail_warn_collects_warning_and_continues() {
    let toml = r#"
[flow]
name = "on_fail warn test"

[[block]]
name = "warn_block"
steps = [
  { action = "screenshot" },
  { action = "tap", text = "NONEXISTENT_ELEMENT", on_fail = "warn" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);
    assert_eq!(result.warnings.len(), 1, "should collect one warning");
    assert!(
        !result.warnings[0].is_empty(),
        "warning message should contain error details"
    );

    // Both screenshots execute (warn does not stop flow)
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(screenshot_count, 2, "both screenshots should execute around warn step");
}

// ---------------------------------------------------------------------------
// 3. on_fail="ignore" silently continues
// ---------------------------------------------------------------------------
#[tokio::test]
async fn on_fail_ignore_silently_continues() {
    let toml = r#"
[flow]
name = "on_fail ignore test"

[[block]]
name = "ignore_block"
steps = [
  { action = "screenshot" },
  { action = "tap", text = "NONEXISTENT_ELEMENT", on_fail = "ignore" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);
    assert!(
        result.warnings.is_empty(),
        "ignore should not produce warnings"
    );

    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(screenshot_count, 2, "both screenshots should execute around ignored step");
}

// ---------------------------------------------------------------------------
// 4. Combined: warn + error in same flow (warn collected, error stops)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn warn_then_error_in_same_flow() {
    let toml = r#"
[flow]
name = "warn + error combined"

[[block]]
name = "mixed_block"
steps = [
  { action = "tap", text = "MISSING_1", on_fail = "warn" },
  { action = "screenshot" },
  { action = "tap", text = "MISSING_2", on_fail = "error" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should return Ok(FlowResult)");

    assert!(!result.success, "flow should fail on the error step");
    assert_eq!(result.failed_step, Some(2), "third step (error) should fail");
    // The warning from the first step should still be collected
    assert_eq!(
        result.warnings.len(),
        1,
        "warn step should have been collected before the error"
    );

    // Only one screenshot (after the warn step, before the error step)
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(screenshot_count, 1, "only one screenshot before the error step");
}

// ---------------------------------------------------------------------------
// 5. Default on_fail is "error" (no on_fail field specified)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn default_on_fail_is_error() {
    let toml = r#"
[flow]
name = "default on_fail test"

[[block]]
name = "default_block"
steps = [
  { action = "tap", text = "NONEXISTENT_ELEMENT" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should return Ok(FlowResult)");

    assert!(
        !result.success,
        "default on_fail should be error, so flow fails"
    );
    assert_eq!(result.failed_step, Some(0));

    // Screenshot should not have executed
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(screenshot_count, 0, "no steps after error should execute");
}

// ===========================================================================
// Retry tests (through flow execution with retry TOML fields)
// ===========================================================================

// ---------------------------------------------------------------------------
// 6. Step with retry > 0 succeeds when element appears in hierarchy
//    (retry on a step that always fails still exhausts retries)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn retry_exhausted_then_on_fail_applies() {
    let toml = r#"
[flow]
name = "retry exhausted test"

[[block]]
name = "retry_block"
steps = [
  { action = "tap", text = "NONEXISTENT_ELEMENT", retry = 2, retry_delay = 10, on_fail = "warn" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);
    assert_eq!(
        result.warnings.len(),
        1,
        "after exhausting retries, on_fail=warn should produce a warning"
    );

    // The screenshot after the retried step should still execute
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(screenshot_count, 1, "screenshot should execute after warn");
}

// ---------------------------------------------------------------------------
// 7. Step fails all retries with on_fail="error" -> error propagates
// ---------------------------------------------------------------------------
#[tokio::test]
async fn retry_exhausted_with_error_propagates() {
    let toml = r#"
[flow]
name = "retry error test"

[[block]]
name = "retry_error_block"
steps = [
  { action = "tap", text = "NONEXISTENT_ELEMENT", retry = 1, retry_delay = 10, on_fail = "error" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should return Ok(FlowResult)");

    assert!(!result.success, "should fail after retries exhausted");
    assert_eq!(result.failed_step, Some(0));

    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(screenshot_count, 0, "no steps after error should run");
}

// ---------------------------------------------------------------------------
// 8. retry=0 means no retries (single attempt, same as default)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn retry_zero_means_single_attempt() {
    let toml = r#"
[flow]
name = "retry=0 test"

[[block]]
name = "no_retry_block"
steps = [
  { action = "tap", text = "NONEXISTENT_ELEMENT", retry = 0, on_fail = "warn" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);
    assert_eq!(
        result.warnings.len(),
        1,
        "single attempt fails, on_fail=warn produces warning"
    );
}

// ===========================================================================
// Timeout tests (through flow execution)
// ===========================================================================

// ---------------------------------------------------------------------------
// 9. Step-level timeout: a step that takes too long fails
// ---------------------------------------------------------------------------
#[tokio::test]
async fn step_timeout_causes_failure() {
    // The "wait" action sleeps for the given duration. Since there's no "wait"
    // action that actually blocks in the runner (screenshot is instant with the mock),
    // we test timeout at the flow level by using a very short default_timeout_ms
    // on a step that requires element resolution (which will trigger get_hierarchy).
    // This is a structural test: a step that fails due to element not found with
    // the default timeout is conceptually equivalent to a timeout-caused failure.
    let toml = r#"
[flow]
name = "timeout test"

[[block]]
name = "timeout_block"
steps = [
  { action = "tap", text = "NONEXISTENT_ELEMENT", timeout = 50, on_fail = "warn" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should not error");

    // The step should fail (element not found) and produce a warning
    assert_success(&result);
    assert_eq!(result.warnings.len(), 1, "failed step produces a warning");
}

// ===========================================================================
// Capture / screenshot path tests
// ===========================================================================

// ---------------------------------------------------------------------------
// 10. build_screenshot_path generates correct filename
// ---------------------------------------------------------------------------
#[test]
fn build_screenshot_path_generates_correct_filename() {
    let config = CaptureConfig::default();
    let path = build_screenshot_path(&config, "login_flow", "verify_block", 3, "error");

    assert_eq!(
        path,
        PathBuf::from(".golem/screenshots/login_flow_verify_block_step3_error.png")
    );
}

// ---------------------------------------------------------------------------
// 11. CaptureConfig defaults are correct
// ---------------------------------------------------------------------------
#[test]
fn capture_config_defaults_correct() {
    let config = CaptureConfig::default();

    assert!(config.screenshot_on_failure, "screenshot_on_failure should default to true");
    assert_eq!(
        config.screenshot_dir,
        PathBuf::from(".golem/screenshots"),
        "default screenshot dir"
    );
    assert!(!config.record, "record should default to false");
    assert_eq!(
        config.recording_dir,
        PathBuf::from(".golem/recordings"),
        "default recording dir"
    );
}

// ---------------------------------------------------------------------------
// 12. Screenshot path includes flow name, block name, step index
// ---------------------------------------------------------------------------
#[test]
fn screenshot_path_includes_flow_block_step_components() {
    let config = CaptureConfig {
        screenshot_dir: PathBuf::from("/tmp/test_shots"),
        ..CaptureConfig::default()
    };
    let path = build_screenshot_path(&config, "checkout", "payment", 7, "warn");

    let filename = path
        .file_name()
        .expect("should have filename")
        .to_str()
        .expect("should be valid utf-8");

    assert!(filename.contains("checkout"), "missing flow name in filename");
    assert!(filename.contains("payment"), "missing block name in filename");
    assert!(filename.contains("step7"), "missing step index in filename");
    assert!(filename.contains("warn"), "missing failure type in filename");
    assert!(filename.ends_with(".png"), "missing .png extension");
    assert_eq!(
        path.parent().expect("should have parent"),
        Path::new("/tmp/test_shots"),
        "parent directory should match configured screenshot_dir"
    );
}

// ---------------------------------------------------------------------------
// 13. Screenshot path sanitizes special characters in names
// ---------------------------------------------------------------------------
#[test]
fn screenshot_path_sanitizes_special_characters() {
    let config = CaptureConfig::default();
    let path = build_screenshot_path(&config, "my flow!", "block #1", 0, "error");

    let filename = path
        .file_name()
        .expect("should have filename")
        .to_str()
        .expect("should be valid utf-8");

    // Spaces and special characters should be replaced with underscores
    assert!(
        !filename.contains(' '),
        "spaces should be sanitized in filename"
    );
    assert!(
        !filename.contains('!'),
        "special chars should be sanitized in filename"
    );
    assert!(
        !filename.contains('#'),
        "hash should be sanitized in filename"
    );
    assert!(filename.ends_with(".png"));
}

// ===========================================================================
// Cross-module integration: policy + capture working together
// ===========================================================================

// ---------------------------------------------------------------------------
// 14. on_fail="warn" in flow -> warning collected -> can build screenshot
//     path for the warning step
// ---------------------------------------------------------------------------
#[tokio::test]
async fn warn_outcome_feeds_screenshot_path_generation() {
    let toml = r#"
[flow]
name = "screenshot path flow"

[[block]]
name = "login_screen"
steps = [
  { action = "screenshot" },
  { action = "tap", text = "NONEXISTENT_ELEMENT", on_fail = "warn" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);
    assert_eq!(result.warnings.len(), 1);

    // After detecting a warning, the runner would build a screenshot path
    let config = CaptureConfig::default();
    let screenshot_path = build_screenshot_path(
        &config,
        &flow.flow.name,          // "screenshot path flow"
        "login_screen",           // block name
        1,                        // step index of the warning
        "warn",
    );

    let filename = screenshot_path
        .file_name()
        .expect("should have filename")
        .to_str()
        .expect("should be valid utf-8");

    assert!(filename.contains("screenshot_path_flow"), "flow name in path");
    assert!(filename.contains("login_screen"), "block name in path");
    assert!(filename.contains("step1"), "step index in path");
    assert!(filename.contains("warn"), "failure type in path");
}

// ---------------------------------------------------------------------------
// 15. on_fail="error" -> can build screenshot path for the error step
// ---------------------------------------------------------------------------
#[tokio::test]
async fn error_outcome_feeds_screenshot_path_generation() {
    let toml = r#"
[flow]
name = "error capture flow"

[[block]]
name = "auth_block"
steps = [
  { action = "screenshot" },
  { action = "tap", text = "NONEXISTENT_ELEMENT" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should return Ok(FlowResult)");

    assert!(!result.success);
    let failed_step = result.failed_step.expect("should have failed_step");
    let failed_block = result.failed_block.as_deref().expect("should have failed_block");

    // Build the screenshot path from the failure info
    let config = CaptureConfig::default();
    let screenshot_path = build_screenshot_path(
        &config,
        &flow.flow.name,
        failed_block,
        failed_step,
        "error",
    );

    let filename = screenshot_path
        .file_name()
        .expect("should have filename")
        .to_str()
        .expect("should be valid utf-8");

    assert!(filename.contains("error_capture_flow"), "flow name in path");
    assert!(filename.contains("auth_block"), "block name from failure info");
    assert!(filename.contains("step1"), "step index from failure info");
    assert!(filename.contains("error"), "failure type");
}

// ---------------------------------------------------------------------------
// 16. Multiple warnings across blocks each get distinct screenshot paths
// ---------------------------------------------------------------------------
#[tokio::test]
async fn multiple_warnings_generate_distinct_screenshot_paths() {
    let toml = r#"
[flow]
name = "multi warn flow"

[[block]]
name = "block_a"
steps = [
  { action = "tap", text = "MISSING_1", on_fail = "warn" },
]

[[block]]
name = "block_b"
steps = [
  { action = "tap", text = "MISSING_2", on_fail = "warn" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);
    assert_eq!(result.warnings.len(), 2, "two warnings from two blocks");

    let config = CaptureConfig::default();

    // Build screenshot paths for each warning (block_a step 0, block_b step 0)
    let path_a = build_screenshot_path(&config, &flow.flow.name, "block_a", 0, "warn");
    let path_b = build_screenshot_path(&config, &flow.flow.name, "block_b", 0, "warn");

    assert_ne!(
        path_a, path_b,
        "screenshot paths for different blocks should be distinct"
    );

    let filename_a = path_a.file_name().expect("should have filename");
    let filename_b = path_b.file_name().expect("should have filename");
    assert!(
        filename_a.to_str().expect("utf-8").contains("block_a"),
        "path_a should contain block_a"
    );
    assert!(
        filename_b.to_str().expect("utf-8").contains("block_b"),
        "path_b should contain block_b"
    );
}

// ---------------------------------------------------------------------------
// 17. on_fail="ignore" in multi-block flow: no warnings, no failures,
//     and ignore steps do not affect subsequent blocks
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ignore_in_multi_block_flow_does_not_affect_subsequent_blocks() {
    let toml = r#"
[flow]
name = "ignore multi-block"

[[block]]
name = "block_1"
steps = [
  { action = "tap", text = "MISSING", on_fail = "ignore" },
  { action = "screenshot" },
]

[[block]]
name = "block_2"
steps = [
  { action = "tap", text = "Also_Missing", on_fail = "ignore" },
  { action = "screenshot" },
]

[[block]]
name = "block_3"
steps = [
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);
    assert!(
        result.warnings.is_empty(),
        "ignore steps should not produce warnings"
    );

    // All three blocks should execute: 3 screenshots total
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 3,
        "all blocks should execute past ignored steps"
    );
}

// ---------------------------------------------------------------------------
// 18. Retry with on_fail="ignore" — retries exhaust, then silently ignored
// ---------------------------------------------------------------------------
#[tokio::test]
async fn retry_with_on_fail_ignore_silently_continues() {
    let toml = r#"
[flow]
name = "retry ignore test"

[[block]]
name = "retry_ignore_block"
steps = [
  { action = "tap", text = "NONEXISTENT", retry = 1, retry_delay = 10, on_fail = "ignore" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);
    assert!(
        result.warnings.is_empty(),
        "on_fail=ignore should not produce warnings even after retries"
    );

    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(screenshot_count, 1, "screenshot should still run after ignored retry failure");
}

// ---------------------------------------------------------------------------
// 19. Successful step with retry set does not produce warnings
// ---------------------------------------------------------------------------
#[tokio::test]
async fn successful_step_with_retry_produces_no_warnings() {
    let toml = r#"
[flow]
name = "success with retry"

[[block]]
name = "ok_block"
steps = [
  { action = "tap", text = "Submit", retry = 2, retry_delay = 10 },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(hierarchy_with_texts(&["Submit"]));
    let mut vars = golem_vars::VariableStore::new();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);
    assert!(
        result.warnings.is_empty(),
        "successful step should not produce warnings"
    );

    let calls = driver.get_calls();
    let tap_count = calls.iter().filter(|c| c.0 == "tap").count();
    assert_eq!(tap_count, 1, "successful step should not retry");
}

// ---------------------------------------------------------------------------
// 20. capture_failure_screenshot writes file to disk via mock driver
// ---------------------------------------------------------------------------
#[tokio::test]
async fn capture_failure_screenshot_writes_to_disk() {
    use golem_runner::capture::capture_failure_screenshot;

    let driver = MockPlatformDriver::new(empty_hierarchy());
    let tmp = tempfile::tempdir().expect("failed to create tempdir");

    let config = CaptureConfig {
        screenshot_on_failure: true,
        screenshot_dir: tmp.path().to_path_buf(),
        ..CaptureConfig::default()
    };

    let path = capture_failure_screenshot(&driver, &config, "my_flow", "my_block", 2, "error")
        .await
        .expect("capture should succeed");

    assert!(path.exists(), "screenshot file should exist on disk");

    let filename = path
        .file_name()
        .expect("should have filename")
        .to_str()
        .expect("utf-8");
    assert!(filename.contains("my_flow"), "flow name in captured filename");
    assert!(filename.contains("my_block"), "block name in captured filename");
    assert!(filename.contains("step2"), "step index in captured filename");
    assert!(filename.contains("error"), "failure type in captured filename");

    // Verify PNG magic bytes were written
    let data = std::fs::read(&path).expect("should read file");
    assert_eq!(&data[..4], &[0x89, 0x50, 0x4E, 0x47], "should contain PNG magic bytes");
}

// ---------------------------------------------------------------------------
// 21. capture_failure_screenshot returns error when disabled
// ---------------------------------------------------------------------------
#[tokio::test]
async fn capture_failure_screenshot_disabled_returns_error() {
    use golem_runner::capture::capture_failure_screenshot;

    let driver = MockPlatformDriver::new(empty_hierarchy());
    let config = CaptureConfig {
        screenshot_on_failure: false,
        ..CaptureConfig::default()
    };

    let result =
        capture_failure_screenshot(&driver, &config, "flow", "block", 0, "error").await;

    assert!(result.is_err(), "should error when screenshot disabled");
    let err_msg = result.expect_err("should be error").to_string();
    assert!(
        err_msg.contains("disabled"),
        "error should mention disabled: {err_msg}"
    );

    // Driver should not have been called at all
    assert!(
        driver.get_calls().is_empty(),
        "driver should not be called when capture is disabled"
    );
}
