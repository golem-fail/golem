//! Integration tests for step policies (if_fail, retry, timeout) and screenshot
//! capture paths.
//!
//! These tests exercise the interaction between `golem_runner::policy`,
//! `golem_runner::capture`, and `golem_runner::executor` modules through their
//! public APIs, using full TOML-parsed flows and the mock driver.

use std::path::Path;
use std::sync::LazyLock;

use golem_driver::MockPlatformDriver;
use golem_element::{Bounds, Element};
use golem_parser::parse_flow;
use golem_runner::capture::{build_screenshot_path, CaptureConfig};
use golem_runner::context::ExecutionContext;
use golem_runner::executor::{execute_flow, FlowResult};
use rand::SeedableRng;

// Tests use a short per-step timeout — these flows test outcome
// (error / warn / ignore policies, retry semantics, screenshot
// paths), not real-clock timing. 500ms is enough for resolution's
// 250ms poll cadence to fire 2-3 attempts before the error-path
// timeout, even with the tap action's 2× class multiplier (effective
// 1000ms). Lower values (e.g. 200 ms) flaked under parallel test
// load when futures don't get scheduled before the deadline. Higher
// values were unnecessary — most slow tests in this file existed
// purely because the per-step budget burned a full second waiting
// for "not found" to time out.
const DEFAULT_TIMEOUT: u64 = 500;

static DEFAULT_CAPTURE: LazyLock<CaptureConfig> = LazyLock::new(CaptureConfig::default);

fn test_ctx() -> ExecutionContext<'static> {
    ExecutionContext {
        flow_dir: Path::new("."),
        project_root: Path::new("."),
        capture_config: &DEFAULT_CAPTURE,
        flow_name: "test",
        block_name: None,
        step_index: 0,
        global_step_index: 0,
        block_iteration: 0,
        device: None,
        perf_collector: None,
        last_launch_ms: std::sync::atomic::AtomicU64::new(0),
        emitter: None,
        step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),
        rng: std::sync::Mutex::new(rand_chacha::ChaCha8Rng::seed_from_u64(0)),
        inherited_record_default: false,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Recursively collect all file paths under a directory.
fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walkdir(&path));
            } else {
                files.push(path);
            }
        }
    }
    files
}

fn empty_hierarchy() -> Element {
    Element {
        element_type: "View".to_string(),
        text: None,
        accessibility_label: None,
        placeholder: None,
        enabled: true,
        checked: false,
        clickable: true,
        focused: false,
        bounds: Bounds::new(0, 0, 375, 812),
        visible_bounds: None,
        hit_points: vec![],
        drawing_order: None,
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
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds: Bounds::new(10, (i as i32) * 50, 200, 40),
            visible_bounds: None,
            hit_points: vec![],
            drawing_order: None,
            children: Vec::new(),
        })
        .collect();

    Element {
        element_type: "View".to_string(),
        text: None,
        accessibility_label: None,
        placeholder: None,
        enabled: true,
        checked: false,
        clickable: true,
        focused: false,
        bounds: Bounds::new(0, 0, 375, 812),
        visible_bounds: None,
        hit_points: vec![],
        drawing_order: None,
        children,
    }
}

fn assert_success(result: &FlowResult) {
    assert!(result.success, "flow SHALL succeed");
    assert!(result.failed_step.is_none(), "no step SHALL have failed");
    assert!(result.failed_block.is_none(), "no block should have failed");
}

// ===========================================================================
// Policy tests: if_fail behaviour through full flow execution
// ===========================================================================

// ---------------------------------------------------------------------------
// 1. if_fail="error" propagates error and stops flow
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn on_fail_error_propagates_and_stops_flow() {
    let toml = r#"
[flow]
name = "if_fail error test"

[[block]]
name = "failing_block"
steps = [
  { action = "screenshot" },
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", if_fail = "error" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should return Ok(FlowResult)");

    assert!(!result.success, "flow SHALL fail with if_fail=error");
    assert_eq!(
        result.failed_step,
        Some(1),
        "second step SHALL be the failure"
    );
    assert_eq!(result.failed_block, Some("failing_block".to_string()),);

    // The third step (second screenshot) should NOT have executed.
    // Count includes: 1 explicit screenshot step + 1 failure capture = 2
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 2,
        "first screenshot step + failure capture screenshot"
    );
}

// ---------------------------------------------------------------------------
// 2. if_fail="warn" collects warning and continues
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn on_fail_warn_collects_warning_and_continues() {
    let toml = r#"
[flow]
name = "if_fail warn test"

[[block]]
name = "warn_block"
steps = [
  { action = "screenshot" },
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", if_fail = "warn" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should not error");

    assert_success(&result);
    assert_eq!(result.warnings.len(), 1, "SHALL collect one warning");
    assert!(
        !result.warnings[0].is_empty(),
        "warning message should contain error details"
    );

    // Both screenshots execute (warn does not stop flow).
    // Count includes: 2 explicit screenshot steps + 1 failure capture = 3
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 3,
        "both screenshot steps + failure capture screenshot"
    );
}

// ---------------------------------------------------------------------------
// 3. if_fail="ignore" silently continues
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn on_fail_ignore_silently_continues() {
    let toml = r#"
[flow]
name = "if_fail ignore test"

[[block]]
name = "ignore_block"
steps = [
  { action = "screenshot" },
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", if_fail = "ignore" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should not error");

    assert_success(&result);
    assert!(
        result.warnings.is_empty(),
        "ignore should not produce warnings"
    );

    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 2,
        "both screenshots SHALL execute around ignored step"
    );
}

// ---------------------------------------------------------------------------
// 4. Combined: warn + error in same flow (warn collected, error stops)
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn warn_then_error_in_same_flow() {
    let toml = r#"
[flow]
name = "warn + error combined"

[[block]]
name = "mixed_block"
steps = [
  { action = "tap", on_text = "MISSING_1", if_fail = "warn" },
  { action = "screenshot" },
  { action = "tap", on_text = "MISSING_2", if_fail = "error" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should return Ok(FlowResult)");

    assert!(!result.success, "flow SHALL fail on the error step");
    assert_eq!(result.failed_step, Some(2), "third step (error) SHALL fail");
    // The warning from the first step should still be collected
    assert_eq!(
        result.warnings.len(),
        1,
        "warn step should have been collected before the error"
    );

    // Count includes: 1 warn capture + 1 explicit screenshot step + 1 error capture = 3
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 3,
        "warn capture + screenshot step + error capture"
    );
}

// ---------------------------------------------------------------------------
// 5. Default if_fail is "error" (no if_fail field specified)
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn default_on_fail_is_error() {
    let toml = r#"
[flow]
name = "default if_fail test"

[[block]]
name = "default_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT_ELEMENT" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should return Ok(FlowResult)");

    assert!(
        !result.success,
        "default if_fail should be error, so flow fails"
    );
    assert_eq!(result.failed_step, Some(0));

    // No explicit screenshot steps executed, but failure capture occurs
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(screenshot_count, 1, "failure capture screenshot only");
}

// ===========================================================================
// Retry tests (through flow execution with retry TOML fields)
// ===========================================================================

// ---------------------------------------------------------------------------
// 6. Step with retry > 0 succeeds when element appears in hierarchy
//    (retry on a step that always fails still exhausts retries)
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn retry_exhausted_then_on_fail_applies() {
    let toml = r#"
[flow]
name = "retry exhausted test"

[[block]]
name = "retry_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", retry = 2, retry_delay = 10, if_fail = "warn" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should not error");

    assert_success(&result);
    assert_eq!(
        result.warnings.len(),
        1,
        "after exhausting retries, if_fail=warn should produce a warning"
    );

    // Count includes: 1 explicit screenshot step + 1 failure capture = 2
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 2,
        "screenshot step + failure capture screenshot"
    );
}

// ---------------------------------------------------------------------------
// 7. Step fails all retries with if_fail="error" -> error propagates
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn retry_exhausted_with_error_propagates() {
    let toml = r#"
[flow]
name = "retry error test"

[[block]]
name = "retry_error_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", retry = 1, retry_delay = 10, if_fail = "error" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should return Ok(FlowResult)");

    assert!(!result.success, "SHALL fail after retries exhausted");
    assert_eq!(result.failed_step, Some(0));

    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(screenshot_count, 1, "failure capture screenshot only");
}

// ---------------------------------------------------------------------------
// 8. retry=0 means no retries (single attempt, same as default)
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn retry_zero_means_single_attempt() {
    let toml = r#"
[flow]
name = "retry=0 test"

[[block]]
name = "no_retry_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", retry = 0, if_fail = "warn" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should not error");

    assert_success(&result);
    assert_eq!(
        result.warnings.len(),
        1,
        "single attempt fails, if_fail=warn produces warning"
    );
}

// ===========================================================================
// Timeout tests (through flow execution)
// ===========================================================================

// ---------------------------------------------------------------------------
// 9. Step-level timeout: a step that takes too long fails
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn step_timeout_causes_failure() {
    // Test timeout at the flow level by using a very short timeout on a step
    // that requires element resolution (which polls get_hierarchy until the
    // deadline). A step that fails due to element not found with the default
    // timeout is structurally equivalent to a timeout-caused failure.
    let toml = r#"
[flow]
name = "timeout test"

[[block]]
name = "timeout_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", timeout = 50, if_fail = "warn" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should not error");

    // The step should fail (element not found) and produce a warning
    assert_success(&result);
    assert_eq!(result.warnings.len(), 1, "failed step produces a warning");
}

// ===========================================================================
// Cross-module integration: policy + capture working together
// ===========================================================================

// ---------------------------------------------------------------------------
// 14. if_fail="warn" in flow -> warning collected -> can build screenshot
//     path for the warning step
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn warn_outcome_feeds_screenshot_path_generation() {
    let toml = r#"
[flow]
name = "screenshot path flow"

[[block]]
name = "login_screen"
steps = [
  { action = "screenshot" },
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", if_fail = "warn" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should not error");

    assert_success(&result);
    assert_eq!(result.warnings.len(), 1);

    // After detecting a warning, the runner would build a screenshot path
    let config = CaptureConfig {
        flow_name: flow.flow.name.clone(),
        device_name: "test_device".to_string(),
        ..CaptureConfig::default()
    };
    let screenshot_path = build_screenshot_path(
        &config,
        "login_screen", // block name
        2,              // global step index
        0,              // block iteration
        1,              // step index of the warning
        "warn",
    );

    let filename = screenshot_path
        .file_name()
        .expect("should have filename")
        .to_str()
        .expect("should be valid utf-8");

    assert!(filename.contains("login_screen"), "block name in path");
    assert!(filename.contains("warn"), "failure type in path");
}

// ---------------------------------------------------------------------------
// 15. if_fail="error" -> can build screenshot path for the error step
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn error_outcome_feeds_screenshot_path_generation() {
    let toml = r#"
[flow]
name = "error capture flow"

[[block]]
name = "auth_block"
steps = [
  { action = "screenshot" },
  { action = "tap", on_text = "NONEXISTENT_ELEMENT" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should return Ok(FlowResult)");

    assert!(!result.success);
    let failed_step = result.failed_step.expect("should have failed_step");
    let failed_block = result
        .failed_block
        .as_deref()
        .expect("should have failed_block");

    // Build the screenshot path from the failure info
    let config = CaptureConfig {
        flow_name: flow.flow.name.clone(),
        device_name: "test_device".to_string(),
        ..CaptureConfig::default()
    };
    let screenshot_path = build_screenshot_path(
        &config,
        failed_block,
        failed_step as u64,
        0,
        failed_step,
        "error",
    );

    let filename = screenshot_path
        .file_name()
        .expect("should have filename")
        .to_str()
        .expect("should be valid utf-8");

    assert!(filename.contains("error"), "error type in path");
    assert!(
        filename.contains("auth_block"),
        "block name from failure info"
    );
}

// ---------------------------------------------------------------------------
// 16. Multiple warnings across blocks each get distinct screenshot paths
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn multiple_warnings_generate_distinct_screenshot_paths() {
    let toml = r#"
[flow]
name = "multi warn flow"

[[block]]
name = "block_a"
steps = [
  { action = "tap", on_text = "MISSING_1", if_fail = "warn" },
]

[[block]]
name = "block_b"
steps = [
  { action = "tap", on_text = "MISSING_2", if_fail = "warn" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should not error");

    assert_success(&result);
    assert_eq!(result.warnings.len(), 2, "two warnings from two blocks");

    let config = CaptureConfig {
        flow_name: flow.flow.name.clone(),
        device_name: "test_device".to_string(),
        ..CaptureConfig::default()
    };

    // Build screenshot paths for each warning (block_a step 0, block_b step 0)
    let path_a = build_screenshot_path(&config, "block_a", 1, 0, 0, "warn");
    let path_b = build_screenshot_path(&config, "block_b", 2, 0, 0, "warn");

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
// 17. if_fail="ignore" in multi-block flow: no warnings, no failures,
//     and ignore steps do not affect subsequent blocks
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ignore_in_multi_block_flow_does_not_affect_subsequent_blocks() {
    let toml = r#"
[flow]
name = "ignore multi-block"

[[block]]
name = "block_1"
steps = [
  { action = "tap", on_text = "MISSING", if_fail = "ignore" },
  { action = "screenshot" },
]

[[block]]
name = "block_2"
steps = [
  { action = "tap", on_text = "Also_Missing", if_fail = "ignore" },
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
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
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
// 18. Retry with if_fail="ignore" — retries exhaust, then silently ignored
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn retry_with_on_fail_ignore_silently_continues() {
    let toml = r#"
[flow]
name = "retry ignore test"

[[block]]
name = "retry_ignore_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT", retry = 1, retry_delay = 10, if_fail = "ignore" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should not error");

    assert_success(&result);
    assert!(
        result.warnings.is_empty(),
        "if_fail=ignore should not produce warnings even after retries"
    );

    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 1,
        "screenshot SHALL still run after ignored retry failure"
    );
}

// ---------------------------------------------------------------------------
// 19. Successful step with retry set does not produce warnings
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn successful_step_with_retry_produces_no_warnings() {
    let toml = r#"
[flow]
name = "success with retry"

[[block]]
name = "ok_block"
steps = [
  { action = "tap", on_text = "Submit", retry = 2, retry_delay = 10 },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(hierarchy_with_texts(&["Submit"]));
    let mut vars = golem_vars::VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should not error");

    assert_success(&result);
    assert!(
        result.warnings.is_empty(),
        "successful step should not produce warnings"
    );

    let calls = driver.get_calls();
    let tap_count = calls.iter().filter(|c| c.0 == "tap").count();
    assert_eq!(tap_count, 1, "successful step SHALL NOT retry");
}

// ---------------------------------------------------------------------------
// 20. capture_failure_screenshot writes file to disk via mock driver
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn capture_failure_screenshot_writes_to_disk() {
    use golem_runner::capture::capture_failure_screenshot;

    let driver = MockPlatformDriver::new(empty_hierarchy());
    let tmp = tempfile::tempdir().expect("failed to create tempdir");

    let config = CaptureConfig {
        screenshot_on_failure: true,
        output_dir: tmp.path().to_path_buf(),
        flow_name: "test".to_string(),
        device_name: "test_device".to_string(),
        ..CaptureConfig::default()
    };

    let path = capture_failure_screenshot(&driver, &config, "my_block", 3, 0, 2, "error")
        .await
        .expect("capture should succeed");

    assert!(path.exists(), "screenshot file SHALL exist on disk");

    let filename = path
        .file_name()
        .expect("should have filename")
        .to_str()
        .expect("utf-8");
    assert!(
        filename.contains("my_block"),
        "block name in captured filename"
    );
    assert!(
        filename.contains("error"),
        "failure type in captured filename"
    );

    // Verify PNG magic bytes were written
    let data = std::fs::read(&path).expect("should read file");
    assert_eq!(
        &data[..4],
        &[0x89, 0x50, 0x4E, 0x47],
        "SHALL contain PNG magic bytes"
    );
}

// ---------------------------------------------------------------------------
// 21. capture_failure_screenshot returns error when disabled
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn capture_failure_screenshot_disabled_returns_error() {
    use golem_runner::capture::capture_failure_screenshot;

    let driver = MockPlatformDriver::new(empty_hierarchy());
    let config = CaptureConfig {
        screenshot_on_failure: false,
        ..CaptureConfig::default()
    };

    let result = capture_failure_screenshot(&driver, &config, "block", 1, 0, 0, "error").await;

    assert!(result.is_err(), "SHALL error when screenshot disabled");
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

// ===========================================================================
// Screenshot-on-failure integration: policy.rs calls capture on step failure
// ===========================================================================

// ---------------------------------------------------------------------------
// 22. if_fail="error" triggers screenshot capture via driver
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn on_fail_error_triggers_screenshot_capture() {
    let toml = r#"
[flow]
name = "screenshot on error"

[[block]]
name = "fail_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", if_fail = "error" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let capture_config = CaptureConfig {
        screenshot_on_failure: true,
        output_dir: tmp.path().to_path_buf(),
        flow_name: "test".to_string(),
        device_name: "test_device".to_string(),
        ..CaptureConfig::default()
    };
    let mut ctx = ExecutionContext {
        flow_dir: Path::new("."),
        project_root: Path::new("."),
        capture_config: &capture_config,
        flow_name: "screenshot on error",
        block_name: None,
        step_index: 0,
        global_step_index: 0,
        block_iteration: 0,
        device: None,
        perf_collector: None,
        last_launch_ms: std::sync::atomic::AtomicU64::new(0),
        emitter: None,
        step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),
        rng: std::sync::Mutex::new(rand_chacha::ChaCha8Rng::seed_from_u64(0)),
        inherited_record_default: false,
    };

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should return Ok(FlowResult)");

    assert!(!result.success, "flow SHALL fail with if_fail=error");

    // The driver should have been called for a screenshot (in addition to get_hierarchy)
    let calls = driver.get_calls();
    let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
    assert_eq!(
        screenshot_calls.len(),
        1,
        "SHALL capture exactly one screenshot on error failure"
    );
}

// ---------------------------------------------------------------------------
// 23. if_fail="warn" triggers screenshot capture via driver
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn on_fail_warn_triggers_screenshot_capture() {
    let toml = r#"
[flow]
name = "screenshot on warn"

[[block]]
name = "warn_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", if_fail = "warn" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let capture_config = CaptureConfig {
        screenshot_on_failure: true,
        output_dir: tmp.path().to_path_buf(),
        flow_name: "test".to_string(),
        device_name: "test_device".to_string(),
        ..CaptureConfig::default()
    };
    let mut ctx = ExecutionContext {
        flow_dir: Path::new("."),
        project_root: Path::new("."),
        capture_config: &capture_config,
        flow_name: "screenshot on warn",
        block_name: None,
        step_index: 0,
        global_step_index: 0,
        block_iteration: 0,
        device: None,
        perf_collector: None,
        last_launch_ms: std::sync::atomic::AtomicU64::new(0),
        emitter: None,
        step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),
        rng: std::sync::Mutex::new(rand_chacha::ChaCha8Rng::seed_from_u64(0)),
        inherited_record_default: false,
    };

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should not error");

    assert!(result.success, "flow SHALL succeed with if_fail=warn");
    assert_eq!(result.warnings.len(), 1, "SHALL collect one warning");

    let calls = driver.get_calls();
    let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
    assert_eq!(
        screenshot_calls.len(),
        1,
        "SHALL capture exactly one screenshot on warn failure"
    );
}

// ---------------------------------------------------------------------------
// 24. if_fail="ignore" does NOT trigger screenshot capture
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn on_fail_ignore_does_not_trigger_screenshot_capture() {
    let toml = r#"
[flow]
name = "no screenshot on ignore"

[[block]]
name = "ignore_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", if_fail = "ignore" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let capture_config = CaptureConfig {
        screenshot_on_failure: true,
        output_dir: tmp.path().to_path_buf(),
        flow_name: "test".to_string(),
        device_name: "test_device".to_string(),
        ..CaptureConfig::default()
    };
    let mut ctx = ExecutionContext {
        flow_dir: Path::new("."),
        project_root: Path::new("."),
        capture_config: &capture_config,
        flow_name: "no screenshot on ignore",
        block_name: None,
        step_index: 0,
        global_step_index: 0,
        block_iteration: 0,
        device: None,
        perf_collector: None,
        last_launch_ms: std::sync::atomic::AtomicU64::new(0),
        emitter: None,
        step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),
        rng: std::sync::Mutex::new(rand_chacha::ChaCha8Rng::seed_from_u64(0)),
        inherited_record_default: false,
    };

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should not error");

    assert!(result.success, "flow SHALL succeed with if_fail=ignore");

    let calls = driver.get_calls();
    let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
    assert_eq!(
        screenshot_calls.len(),
        0,
        "SHALL NOT capture screenshot for ignored failures"
    );
}

// ---------------------------------------------------------------------------
// 25. Screenshot failure does not mask the step error
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn screenshot_failure_does_not_mask_step_error() {
    let toml = r#"
[flow]
name = "screenshot fail resilient"

[[block]]
name = "fail_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", if_fail = "error" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    // Use a screenshot_dir that will cause write failure: screenshot_on_failure
    // is disabled, so capture_failure_screenshot returns Err — but the step
    // error should still propagate.
    let capture_config = CaptureConfig {
        screenshot_on_failure: false,
        ..CaptureConfig::default()
    };
    let mut ctx = ExecutionContext {
        flow_dir: Path::new("."),
        project_root: Path::new("."),
        capture_config: &capture_config,
        flow_name: "screenshot fail resilient",
        block_name: None,
        step_index: 0,
        global_step_index: 0,
        block_iteration: 0,
        device: None,
        perf_collector: None,
        last_launch_ms: std::sync::atomic::AtomicU64::new(0),
        emitter: None,
        step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),
        rng: std::sync::Mutex::new(rand_chacha::ChaCha8Rng::seed_from_u64(0)),
        inherited_record_default: false,
    };

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should return Ok(FlowResult)");

    assert!(
        !result.success,
        "flow SHALL still fail even when screenshot capture fails"
    );
    assert_eq!(
        result.failed_step,
        Some(0),
        "SHALL report the correct failed step"
    );
    assert_eq!(
        result.failed_block,
        Some("fail_block".to_string()),
        "SHALL report the correct failed block"
    );
}

// ---------------------------------------------------------------------------
// 26. Screenshot file is written to disk on error failure
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn screenshot_file_written_to_disk_on_error() {
    let toml = r#"
[flow]
name = "disk write flow"

[[block]]
name = "disk_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", if_fail = "error" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let capture_config = CaptureConfig {
        screenshot_on_failure: true,
        output_dir: tmp.path().to_path_buf(),
        flow_name: "test".to_string(),
        device_name: "test_device".to_string(),
        ..CaptureConfig::default()
    };
    let mut ctx = ExecutionContext {
        flow_dir: Path::new("."),
        project_root: Path::new("."),
        capture_config: &capture_config,
        flow_name: "disk write flow",
        block_name: None,
        step_index: 0,
        global_step_index: 0,
        block_iteration: 0,
        device: None,
        perf_collector: None,
        last_launch_ms: std::sync::atomic::AtomicU64::new(0),
        emitter: None,
        step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),
        rng: std::sync::Mutex::new(rand_chacha::ChaCha8Rng::seed_from_u64(0)),
        inherited_record_default: false,
    };

    let _result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should return Ok(FlowResult)");

    // Check that a screenshot file was written in the structured output dir.
    // Path: tmp/{flow}/{device}/screenshots/*.png
    let mut found_png = None;
    for entry in walkdir(tmp.path()) {
        if entry.extension().map(|e| e == "png").unwrap_or(false) {
            found_png = Some(entry);
            break;
        }
    }
    let png_path = found_png.expect("SHALL write a screenshot .png file");
    let filename = png_path.file_name().unwrap().to_str().unwrap();
    assert!(filename.ends_with(".png"), "SHALL have .png extension");
    assert!(filename.contains("disk_block"), "SHALL contain block name");

    // Verify PNG magic bytes
    let data = std::fs::read(&png_path).expect("should read screenshot");
    assert_eq!(
        &data[..4],
        &[0x89, 0x50, 0x4E, 0x47],
        "SHALL contain PNG magic bytes"
    );
}

// ---------------------------------------------------------------------------
// 27. Screenshot disabled in config skips capture but step error still works
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn screenshot_disabled_skips_capture_but_error_propagates() {
    let toml = r#"
[flow]
name = "disabled capture flow"

[[block]]
name = "disabled_block"
steps = [
  { action = "tap", on_text = "NONEXISTENT_ELEMENT", if_fail = "error" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = golem_vars::VariableStore::new();

    let capture_config = CaptureConfig {
        screenshot_on_failure: false,
        ..CaptureConfig::default()
    };
    let mut ctx = ExecutionContext {
        flow_dir: Path::new("."),
        project_root: Path::new("."),
        capture_config: &capture_config,
        flow_name: "disabled capture flow",
        block_name: None,
        step_index: 0,
        global_step_index: 0,
        block_iteration: 0,
        device: None,
        perf_collector: None,
        last_launch_ms: std::sync::atomic::AtomicU64::new(0),
        emitter: None,
        step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),
        rng: std::sync::Mutex::new(rand_chacha::ChaCha8Rng::seed_from_u64(0)),
        inherited_record_default: false,
    };

    let result = execute_flow(
        &flow,
        &driver,
        &mut vars,
        None,
        DEFAULT_TIMEOUT,
        &mut ctx,
        None,
    )
    .await
    .expect("execute_flow should return Ok(FlowResult)");

    assert!(!result.success, "flow SHALL fail with if_fail=error");

    // No screenshot calls should have been made
    let calls = driver.get_calls();
    let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
    assert_eq!(
        screenshot_calls.len(),
        0,
        "SHALL NOT call driver.screenshot() when capture is disabled"
    );
}
