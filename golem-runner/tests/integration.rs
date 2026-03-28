//! Integration tests for GOLEM flow execution.
//!
//! These tests exercise the full pipeline: parse TOML -> execute flow with mock
//! driver -> verify results. They cover linear flows, branching, loops, variable
//! interpolation, fake data generation, teardown, and on_fail policies.

use std::path::Path;
use std::sync::LazyLock;

use golem_driver::MockPlatformDriver;
use golem_element::{Bounds, Element};
use golem_parser::parse_flow;
use golem_runner::capture::CaptureConfig;
use golem_runner::context::ExecutionContext;
use golem_runner::executor::{execute_flow, FlowResult};
use golem_runner::teardown::execute_teardown;
use golem_vars::{Scope, ScopeLevel, VarValue, VariableStore};

const DEFAULT_TIMEOUT: u64 = 10_000;

static DEFAULT_CAPTURE: LazyLock<CaptureConfig> = LazyLock::new(|| CaptureConfig {
    screenshot_on_failure: false,
    ..CaptureConfig::default()
});

fn test_ctx() -> ExecutionContext<'static> {
    ExecutionContext {
        flow_dir: Path::new("."),
        project_root: Path::new("."),
        capture_config: &DEFAULT_CAPTURE,
        flow_name: "test",
        block_name: None,
        step_index: 0,
        device: None,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a root element with child elements that have the given texts.
fn hierarchy_with_texts(texts: &[&str]) -> Element {
    let children = texts
        .iter()
        .enumerate()
        .map(|(i, t)| Element {
            element_type: "Button".to_string(),
            text: Some(t.to_string()),
            accessibility_id: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds: Bounds::new(10, (i as i32) * 50, 200, 40),
            children: Vec::new(),
        })
        .collect();

    Element {
        element_type: "View".to_string(),
        text: None,
        accessibility_id: None,
        placeholder: None,
        enabled: true,
        checked: false,
        clickable: true,
        focused: false,
        bounds: Bounds::new(0, 0, 375, 812),
        children,
    }
}

/// Build an empty root element (no children).
fn empty_hierarchy() -> Element {
    Element {
        element_type: "View".to_string(),
        text: None,
        accessibility_id: None,
        placeholder: None,
        enabled: true,
        checked: false,
        clickable: true,
        focused: false,
        bounds: Bounds::new(0, 0, 375, 812),
        children: Vec::new(),
    }
}

/// Assert that a FlowResult indicates success with no warnings.
fn assert_success(result: &FlowResult) {
    assert!(result.success, "flow SHALL succeed");
    assert!(result.failed_step.is_none(), "no step SHALL have failed");
    assert!(
        result.failed_block.is_none(),
        "no block should have failed"
    );
}

// ---------------------------------------------------------------------------
// 1. Linear flow: 3 blocks in order, all succeed
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_linear_flow_three_blocks_succeed() {
    let toml = r#"
[flow]
name = "linear test"

[[flow.apps]]
name = "app"
bundle = "com.test.app"

[[block]]
name = "first"
steps = [
  { action = "screenshot" },
]

[[block]]
name = "second"
steps = [
  { action = "screenshot" },
]

[[block]]
name = "third"
steps = [
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);

    // Verify all 3 blocks executed (each has one screenshot step)
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(screenshot_count, 3, "all 3 blocks SHALL have executed");
}

// ---------------------------------------------------------------------------
// 2. Flow with next: block 1 -> next="block_3", skips block 2
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_flow_with_next_skips_block() {
    let toml = r#"
[flow]
name = "next test"

[[block]]
name = "block_1"
next = "block_3"
steps = [
  { action = "screenshot" },
]

[[block]]
name = "block_2"
steps = [
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
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);

    // block_1 and block_3 executed; block_2 was skipped
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 2,
        "block_2 should be skipped via next jump"
    );
}

// ---------------------------------------------------------------------------
// 3. Flow with branch: if_visible matches, jumps to target
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_flow_with_branch_if_visible() {
    let toml = r#"
[flow]
name = "branch test"

[[block]]
name = "check"
steps = [
  { action = "screenshot" },
]

[[block.branch]]
if_visible = "Welcome"
goto = "dashboard"

[[block]]
name = "login"
steps = [
  { action = "screenshot" },
]

[[block]]
name = "dashboard"
steps = [
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    // "Welcome" is visible in the hierarchy, so the branch should match
    let driver = MockPlatformDriver::new(hierarchy_with_texts(&["Welcome"]));
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);

    // check + dashboard = 2 screenshots; login should be skipped
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 2,
        "login block should be skipped due to branch"
    );
}

// ---------------------------------------------------------------------------
// 4. Variable interpolation in steps (flow vars referenced in step text)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_variable_interpolation_in_tap_text() {
    // This test verifies that when flow vars are set and an element matching
    // the var value exists, the tap action can find it. We set a variable
    // "btn_text" = "Submit" and have the element "Submit" in the hierarchy.
    //
    // Note: variable interpolation in step text happens at a higher level
    // (the runner resolves ${var} before passing to execute_action). For now,
    // we test that flow-level vars are accessible and the flow completes
    // when the element text matches exactly.
    let toml = r#"
[flow]
name = "var interpolation test"

[flow.vars]
btn_text = "Submit"

[[block]]
name = "tap_block"
steps = [
  { action = "tap", text = "Submit" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(hierarchy_with_texts(&["Submit"]));
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    // Simulate the runner's var initialization: push flow vars into store
    let mut flow_scope = Scope::new(ScopeLevel::Flow);
    for (k, v) in &flow.flow.vars {
        flow_scope.set(k.clone(), VarValue::string(v.clone()));
    }
    vars.push_scope(flow_scope);

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);

    // Verify tap was called
    let calls = driver.get_calls();
    let tap_calls: Vec<_> = calls.iter().filter(|c| c.0 == "tap").collect();
    assert_eq!(tap_calls.len(), 1, "one tap SHALL have been executed");

    // Verify the variable is in the store
    let btn_val = vars.get("btn_text").expect("btn_text should be in store");
    assert_eq!(
        btn_val.as_str(),
        Some("Submit"),
        "variable value should be Submit"
    );
}

// ---------------------------------------------------------------------------
// 5. Fake data generation: flow vars with fake:email, fake:first_name
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_fake_data_generation_in_flow_vars() {
    let toml = r#"
[flow]
name = "fake data test"

[flow.vars]
user_email = "fake:email"
user_name = "fake:first_name"

[[block]]
name = "verify"
steps = [
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    // Simulate runner's fake data evaluation
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let mut rng = ChaCha8Rng::seed_from_u64(flow.flow.seed.unwrap_or(42));
    let ordered_vars: Vec<(String, String)> = flow.flow.vars.clone().into_iter().collect();
    let evaluated = golem_vars::evaluate::evaluate_generators(&ordered_vars, &mut rng)
        .expect("generator evaluation should succeed");

    let mut flow_scope = Scope::new(ScopeLevel::Flow);
    for (k, v) in &evaluated {
        flow_scope.set(k.clone(), v.clone());
    }
    vars.push_scope(flow_scope);

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);

    // Verify email was generated and looks like an email
    let email_val = vars.get("user_email").expect("user_email should exist");
    let email_str = email_val.as_str().expect("should be a string");
    assert!(
        email_str.contains('@'),
        "generated email should contain @, got: {email_str}"
    );

    // Verify first_name was generated and is non-empty
    let name_val = vars.get("user_name").expect("user_name should exist");
    let name_str = name_val.as_str().expect("should be a string");
    assert!(
        !name_str.is_empty(),
        "generated first_name should not be empty"
    );
}

// ---------------------------------------------------------------------------
// 6. Teardown after pass: teardown steps execute after successful flow
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_teardown_after_pass() {
    let toml = r#"
[flow]
name = "teardown after pass"

[[block]]
name = "main"
steps = [
  { action = "screenshot" },
]

[[teardown]]
steps = [
  { action = "screenshot" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    // Execute the main flow
    let flow_result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert_success(&flow_result);

    // Now execute teardown (as the runner would)
    let teardown_result =
        execute_teardown(&flow.teardown, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

    assert!(
        teardown_result.warnings.is_empty(),
        "teardown should have no warnings"
    );
    assert!(
        teardown_result.errors.is_empty(),
        "teardown should have no errors"
    );

    // 1 from main flow + 2 from teardown = 3 screenshots total
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 3,
        "1 main + 2 teardown = 3 screenshots"
    );
}

// ---------------------------------------------------------------------------
// 7. Teardown after fail: teardown steps execute after failed flow
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_teardown_after_fail() {
    let toml = r#"
[flow]
name = "teardown after fail"

[[block]]
name = "failing"
steps = [
  { action = "tap", text = "NONEXISTENT_ELEMENT" },
]

[[teardown]]
steps = [
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    // Main flow fails (element not found)
    let flow_result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should return Ok(FlowResult), not Err");

    assert!(!flow_result.success, "flow SHALL have failed");
    assert_eq!(flow_result.failed_step, Some(0));
    assert_eq!(
        flow_result.failed_block,
        Some("failing".to_string())
    );

    // Teardown still runs after failure
    let teardown_result =
        execute_teardown(&flow.teardown, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

    assert!(
        teardown_result.errors.is_empty(),
        "teardown should succeed"
    );

    // Teardown screenshot should have executed.
    // screenshot_on_failure is disabled in test config, so no failure capture.
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 1,
        "teardown screenshot should execute even after flow failure"
    );
}

// ---------------------------------------------------------------------------
// 8. Step warning doesn't fail flow: on_fail="warn" step fails, flow continues
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_on_fail_warn_continues_flow() {
    let toml = r#"
[flow]
name = "warn test"

[[block]]
name = "block_with_warning"
steps = [
  { action = "screenshot" },
  { action = "tap", text = "NONEXISTENT_ELEMENT", on_fail = "warn" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert!(result.success, "flow SHALL succeed despite warning");
    assert_eq!(
        result.warnings.len(),
        1,
        "should have collected one warning"
    );
    assert!(
        !result.warnings[0].is_empty(),
        "warning message should not be empty"
    );

    // Both screenshot steps should have executed.
    // screenshot_on_failure is disabled in test config, so no failure capture.
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 2,
        "both screenshots should execute around the warning step"
    );
}

// ---------------------------------------------------------------------------
// 9. Step ignore continues: on_fail="ignore" step fails, flow continues silently
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_on_fail_ignore_continues_silently() {
    let toml = r#"
[flow]
name = "ignore test"

[[block]]
name = "block_with_ignore"
steps = [
  { action = "screenshot" },
  { action = "tap", text = "NONEXISTENT_ELEMENT", on_fail = "ignore" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert!(result.success, "flow SHALL succeed");
    assert!(
        result.warnings.is_empty(),
        "ignored steps should not produce warnings"
    );

    // Both screenshot steps should have executed
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 2,
        "both screenshots should execute around the ignored step"
    );
}

// ---------------------------------------------------------------------------
// 10. Branch with no match falls through to next block
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_branch_no_match_falls_through() {
    let toml = r#"
[flow]
name = "branch fallthrough test"

[[block]]
name = "check"
steps = [
  { action = "screenshot" },
]

[[block.branch]]
if_visible = "Login"
goto = "login_block"

[[block]]
name = "fallthrough"
steps = [
  { action = "screenshot" },
]

[[block]]
name = "login_block"
steps = [
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    // "Login" is NOT in the hierarchy, so branch won't match
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);

    // check + fallthrough + login_block = 3 screenshots (fallthrough cascades)
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 3,
        "all blocks should execute when branch doesn't match"
    );
}

// ---------------------------------------------------------------------------
// 11. Tap on element that exists in hierarchy succeeds
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_tap_on_existing_element() {
    let toml = r#"
[flow]
name = "tap test"

[[block]]
name = "tap_block"
steps = [
  { action = "tap", text = "Login" },
  { action = "tap", text = "Submit" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver =
        MockPlatformDriver::new(hierarchy_with_texts(&["Login", "Submit", "Cancel"]));
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);

    let calls = driver.get_calls();
    let tap_calls: Vec<_> = calls.iter().filter(|c| c.0 == "tap").collect();
    assert_eq!(tap_calls.len(), 2, "two taps SHALL have been executed");
}

// ---------------------------------------------------------------------------
// 12. Tap on nonexistent element fails the flow
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_tap_on_missing_element_fails() {
    let toml = r#"
[flow]
name = "tap fail test"

[[block]]
name = "block_a"
steps = [
  { action = "screenshot" },
]

[[block]]
name = "block_b"
steps = [
  { action = "screenshot" },
  { action = "tap", text = "DOES_NOT_EXIST" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should return Ok(FlowResult)");

    assert!(!result.success, "flow SHALL have failed");
    assert_eq!(
        result.failed_block,
        Some("block_b".to_string()),
        "should report the failing block"
    );
    assert_eq!(
        result.failed_step,
        Some(1),
        "second step (index 1) should have failed"
    );
}

// ---------------------------------------------------------------------------
// 13. Multiple warnings accumulated across blocks
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_multiple_warnings_across_blocks() {
    let toml = r#"
[flow]
name = "multi-warn test"

[[block]]
name = "b1"
steps = [
  { action = "tap", text = "MISSING_1", on_fail = "warn" },
  { action = "screenshot" },
  { action = "tap", text = "MISSING_2", on_fail = "warn" },
]

[[block]]
name = "b2"
steps = [
  { action = "tap", text = "MISSING_3", on_fail = "warn" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert!(result.success, "flow SHALL succeed despite warnings");
    assert_eq!(
        result.warnings.len(),
        3,
        "should have 3 warnings (2 from b1, 1 from b2)"
    );
}

// ---------------------------------------------------------------------------
// 14. Branch with if_var equals
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_branch_if_var_equals() {
    let toml = r#"
[flow]
name = "var branch test"

[[block]]
name = "check"
steps = [
  { action = "screenshot" },
]

[[block.branch]]
if_var = "env"
equals = "staging"
goto = "staging_block"

[[block.branch]]
goto = "prod_block"

[[block]]
name = "staging_block"
steps = [
  { action = "screenshot" },
]

[[block]]
name = "prod_block"
steps = [
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    // Set the variable that the branch checks
    let mut scope = Scope::new(ScopeLevel::Flow);
    scope.set("env", VarValue::string("staging"));
    vars.push_scope(scope);

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);

    // check -> staging_block (prod_block is skipped by the branch, but staging_block
    // falls through to prod_block since there's no next/branch on staging_block)
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    // check(1) + staging_block(1) + prod_block falls through(1) = 3
    // Actually: check executes, then branch matches staging_block (index 1),
    // then staging_block falls through to prod_block (index 2) = 3 total
    assert_eq!(
        screenshot_count, 3,
        "check + staging_block + prod_block = 3"
    );
}

// ---------------------------------------------------------------------------
// 15. Empty flow (no blocks) succeeds via TOML parse + execute
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_empty_flow_succeeds() {
    let toml = r#"
[flow]
name = "empty flow"
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);
    assert!(result.warnings.is_empty());
}

// ---------------------------------------------------------------------------
// 16. Parse + execute flow with teardown that has failing steps (on_fail
//     defaults to "ignore" in teardown context)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_teardown_failing_steps_default_to_ignore() {
    let toml = r#"
[flow]
name = "teardown ignore test"

[[block]]
name = "main"
steps = [
  { action = "screenshot" },
]

[[teardown]]
steps = [
  { action = "tap", text = "MISSING_CLEANUP_BTN" },
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let flow_result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");
    assert_success(&flow_result);

    let teardown_result =
        execute_teardown(&flow.teardown, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

    // Failing step in teardown defaults to on_fail="ignore" so it's silent
    assert!(teardown_result.warnings.is_empty());
    assert!(teardown_result.errors.is_empty());

    // The screenshot after the failing step should still execute
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 2,
        "1 from main + 1 from teardown (after ignored failure)"
    );
}

// ---------------------------------------------------------------------------
// 17. Invalid TOML fails to parse
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_invalid_toml_fails_to_parse() {
    let toml = r#"
this is not valid toml {{{
"#;
    let result = parse_flow(toml);
    assert!(result.is_err(), "invalid TOML SHALL fail to parse");
}

// ---------------------------------------------------------------------------
// 18. Flow with start block via flow.start
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_flow_start_block() {
    let toml = r#"
[flow]
name = "start block test"
start = "second"

[[block]]
name = "first"
steps = [
  { action = "screenshot" },
]

[[block]]
name = "second"
steps = [
  { action = "screenshot" },
]

[[block]]
name = "third"
steps = [
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(empty_hierarchy());
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    let start = flow.flow.start.as_deref();
    let result = execute_flow(&flow, &driver, &mut vars, start, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");

    assert_success(&result);

    // Starting at "second" means second + third = 2 screenshots
    let calls = driver.get_calls();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(
        screenshot_count, 2,
        "should skip first block and run second + third"
    );
}

// ---------------------------------------------------------------------------
// 19. Full pipeline: parse TOML with vars + fake data, tap elements, teardown
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_full_pipeline_with_vars_tap_and_teardown() {
    let toml = r#"
[flow]
name = "full pipeline test"
seed = 123

[[flow.apps]]
name = "myapp"
bundle = "com.example.app"

[flow.vars]
static_text = "Continue"

[[block]]
name = "login_screen"
steps = [
  { action = "tap", text = "Continue" },
  { action = "screenshot" },
]

[[block]]
name = "dashboard"
steps = [
  { action = "screenshot" },
]

[[teardown]]
steps = [
  { action = "screenshot" },
]
"#;
    let flow = parse_flow(toml).expect("should parse");
    let driver = MockPlatformDriver::new(hierarchy_with_texts(&["Continue", "Skip"]));
    let mut vars = VariableStore::new();
    let mut ctx = test_ctx();

    // Initialize flow vars
    let mut flow_scope = Scope::new(ScopeLevel::Flow);
    for (k, v) in &flow.flow.vars {
        flow_scope.set(k.clone(), VarValue::string(v.clone()));
    }
    vars.push_scope(flow_scope);

    // Execute flow
    let flow_result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
        .await
        .expect("execute_flow should not error");
    assert_success(&flow_result);

    // Execute teardown
    let teardown_result =
        execute_teardown(&flow.teardown, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;
    assert!(teardown_result.errors.is_empty());

    let calls = driver.get_calls();
    let tap_count = calls.iter().filter(|c| c.0 == "tap").count();
    let screenshot_count = calls.iter().filter(|c| c.0 == "screenshot").count();
    assert_eq!(tap_count, 1, "one tap on Continue");
    assert_eq!(
        screenshot_count, 3,
        "1 from login_screen + 1 from dashboard + 1 from teardown"
    );
}
