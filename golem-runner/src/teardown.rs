use golem_driver::PlatformDriver;
use golem_parser::{Step, TeardownBlock};
use golem_vars::VariableStore;

use crate::context::ExecutionContext;
use crate::policy::{execute_step_with_policy, StepOutcome};

/// The result of executing teardown blocks.
///
/// Teardown results are **isolated** from the test result: even if every
/// teardown step fails, the flow's pass/fail status is unchanged.
#[derive(Debug)]
pub struct TeardownResult {
    /// Warnings from steps with explicit `if_fail = "warn"`.
    pub warnings: Vec<String>,
    /// Errors collected from steps that failed — recorded but never propagated.
    pub errors: Vec<String>,
}

/// Execute all teardown blocks for a flow.
///
/// Key behaviours:
/// - **Always runs** (caller decides whether to invoke based on `--no-teardown`).
/// - Steps default to `if_fail = "ignore"` (opposite of regular blocks).
/// - Failures are collected but **never** change the test result.
/// - All variables captured during the flow are accessible.
/// - Multiple teardown blocks execute in document order.
pub async fn execute_teardown(
    teardown_blocks: &[TeardownBlock],
    driver: &dyn PlatformDriver,
    vars: &mut VariableStore,
    default_timeout_ms: u64,
    ctx: &ExecutionContext<'_>,
) -> TeardownResult {
    let mut result = TeardownResult {
        warnings: Vec::new(),
        errors: Vec::new(),
    };

    for block in teardown_blocks {
        for step in &block.steps {
            let effective_step = apply_teardown_defaults(step);

            match execute_step_with_policy(&effective_step, driver, vars, default_timeout_ms, ctx, &[]).await
            {
                Ok(StepOutcome::Success) => {}
                Ok(StepOutcome::Warning(msg)) => result.warnings.push(msg),
                Ok(StepOutcome::Ignored) => {}
                Err(e) => {
                    // Collect error but DON'T propagate — teardown never fails the test
                    result.errors.push(e.to_string());
                }
            }
        }
    }

    result
}

/// Clone a step with teardown-specific defaults applied.
///
/// In teardown context, `if_fail` defaults to `"ignore"` rather than `"error"`.
/// If the step already has an explicit `if_fail`, it is preserved.
fn apply_teardown_defaults(step: &Step) -> Step {
    let mut step = step.clone();
    if step.if_fail.is_none() {
        step.if_fail = Some("ignore".to_string());
    }
    step
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_ctx;
    use golem_driver::MockPlatformDriver;
    use golem_element::{Bounds, Element};
    use golem_vars::{Scope, ScopeLevel, VarValue};
    use std::collections::HashMap;
    use std::path::Path;

    const DEFAULT_TIMEOUT: u64 = 10_000;

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

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
            children: Vec::new(),
        }
    }

    /// A step that always succeeds (screenshot needs no element resolution).
    fn make_success_step() -> Step {
        Step {
            action: "screenshot".to_string(),
            on_text: None,
            on_accessibility_label: None,
            on_index: None,
            on_enabled: None,
            on_clickable: None,
            on_below: None,
            on_above: None,
            on_right_of: None,
            on_left_of: None,
            on: None,
            input: None,
            if_fail: None,
            save_to: None,
            timeout: None,
            retry: None,
            retry_delay: None,
            app: None,
            auto_scroll: None,
            max_scrolls: None,
            scroll_timeout: None,
            within: None, start: None, end: None, points: vec![], duration: None,
            params: HashMap::new(),
        }
    }

    /// A step that always fails (taps a nonexistent element).
    fn make_failing_step() -> Step {
        Step {
            action: "tap".to_string(),
            on_text: Some("NONEXISTENT_ELEMENT_xyz_12345".to_string()),
            on_accessibility_label: None,
            on_index: None,
            on_enabled: None,
            on_clickable: None,
            on_below: None,
            on_above: None,
            on_right_of: None,
            on_left_of: None,
            on: None,
            input: None,
            if_fail: None,
            save_to: None,
            timeout: None,
            retry: None,
            retry_delay: None,
            app: None,
            auto_scroll: None,
            max_scrolls: None,
            scroll_timeout: None,
            within: None, start: None, end: None, points: vec![], duration: None,
            params: HashMap::new(),
        }
    }

    fn make_teardown_block(steps: Vec<Step>) -> TeardownBlock {
        TeardownBlock { steps }
    }

    // ---------------------------------------------------------------
    // 1. Teardown executes all steps
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn teardown_executes_all_steps() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let ctx = test_ctx(Path::new("."));
        let blocks = vec![make_teardown_block(vec![
            make_success_step(),
            make_success_step(),
            make_success_step(),
        ])];

        let result = execute_teardown(&blocks, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

        assert!(result.warnings.is_empty());
        assert!(result.errors.is_empty());

        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(screenshot_calls.len(), 3, "all 3 teardown steps SHALL run");
    }

    // ---------------------------------------------------------------
    // 2. Teardown continues after step failure (default if_fail="ignore")
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn teardown_continues_after_step_failure() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let ctx = test_ctx(Path::new("."));
        // failing step in the middle — should not stop subsequent steps
        let blocks = vec![make_teardown_block(vec![
            make_success_step(),
            make_failing_step(), // if_fail defaults to "ignore" in teardown
            make_success_step(),
        ])];

        let result = execute_teardown(&blocks, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

        // The failing step is silently ignored (default if_fail = "ignore")
        assert!(result.warnings.is_empty());
        assert!(result.errors.is_empty());

        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            2,
            "both success steps should execute despite middle failure"
        );
    }

    // ---------------------------------------------------------------
    // 3. Teardown with explicit if_fail="warn" collects warning
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn teardown_explicit_on_fail_warn_collects_warning() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let ctx = test_ctx(Path::new("."));

        let mut warn_step = make_failing_step();
        warn_step.if_fail = Some("warn".to_string());

        let blocks = vec![make_teardown_block(vec![
            make_success_step(),
            warn_step,
            make_success_step(),
        ])];

        let result = execute_teardown(&blocks, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

        assert_eq!(result.warnings.len(), 1, "SHALL collect one warning");
        assert!(
            !result.warnings[0].is_empty(),
            "warning message should not be empty"
        );
        assert!(result.errors.is_empty());
    }

    // ---------------------------------------------------------------
    // 4. Teardown with explicit if_fail="error" collects error but doesn't propagate
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn teardown_explicit_on_fail_error_collects_error_no_propagation() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let ctx = test_ctx(Path::new("."));

        let mut error_step = make_failing_step();
        error_step.if_fail = Some("error".to_string());

        let blocks = vec![make_teardown_block(vec![
            make_success_step(),
            error_step,
            make_success_step(),
        ])];

        let result = execute_teardown(&blocks, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

        assert_eq!(
            result.errors.len(),
            1,
            "should collect the error without propagating"
        );
        assert!(
            !result.errors[0].is_empty(),
            "error message should not be empty"
        );

        // The step after the error step should still execute.
        // test_ctx has screenshot_on_failure=false, so only the 2 success
        // steps (screenshot action) produce driver screenshot calls.
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            2,
            "both success steps around the error step should still execute"
        );
    }

    // ---------------------------------------------------------------
    // 5. Empty teardown blocks produce empty result
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn empty_teardown_blocks_produce_empty_result() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let ctx = test_ctx(Path::new("."));

        // No blocks at all
        let result = execute_teardown(&[], &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;
        assert!(result.warnings.is_empty());
        assert!(result.errors.is_empty());

        // One block with no steps
        let blocks = vec![make_teardown_block(vec![])];
        let result = execute_teardown(&blocks, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;
        assert!(result.warnings.is_empty());
        assert!(result.errors.is_empty());
    }

    // ---------------------------------------------------------------
    // 6. Variables from flow are accessible in teardown
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn variables_from_flow_are_accessible_in_teardown() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let ctx = test_ctx(Path::new("."));

        // Simulate flow having set a variable
        let mut flow_scope = Scope::new(ScopeLevel::Flow);
        flow_scope.set("user_id", VarValue::string("42"));
        vars.push_scope(flow_scope);

        let blocks = vec![make_teardown_block(vec![make_success_step()])];

        let result = execute_teardown(&blocks, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

        assert!(result.warnings.is_empty());
        assert!(result.errors.is_empty());

        // Verify the variable is still accessible (teardown didn't wipe it)
        assert_eq!(
            vars.get("user_id"),
            Some(&VarValue::string("42")),
            "flow variables should be accessible during and after teardown"
        );
    }

    // ---------------------------------------------------------------
    // 7. Multiple teardown blocks execute in order
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn multiple_teardown_blocks_execute_in_order() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let ctx = test_ctx(Path::new("."));

        let blocks = vec![
            make_teardown_block(vec![make_success_step(), make_success_step()]),
            make_teardown_block(vec![make_success_step()]),
            make_teardown_block(vec![make_success_step(), make_success_step()]),
        ];

        let result = execute_teardown(&blocks, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

        assert!(result.warnings.is_empty());
        assert!(result.errors.is_empty());

        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            5,
            "all steps from all 3 blocks should execute (2 + 1 + 2)"
        );
    }

    // ---------------------------------------------------------------
    // 8. Teardown result has correct warnings list
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn teardown_result_has_correct_warnings_list() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let ctx = test_ctx(Path::new("."));

        let mut warn_step_1 = make_failing_step();
        warn_step_1.if_fail = Some("warn".to_string());
        let mut warn_step_2 = make_failing_step();
        warn_step_2.if_fail = Some("warn".to_string());

        let blocks = vec![
            make_teardown_block(vec![warn_step_1, make_success_step()]),
            make_teardown_block(vec![warn_step_2]),
        ];

        let result = execute_teardown(&blocks, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

        assert_eq!(
            result.warnings.len(),
            2,
            "should collect warnings from both blocks"
        );
        assert!(result.errors.is_empty());
        for w in &result.warnings {
            assert!(!w.is_empty(), "each warning SHALL have a message");
        }
    }

    // ---------------------------------------------------------------
    // 9. Teardown result has correct errors list
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn teardown_result_has_correct_errors_list() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let ctx = test_ctx(Path::new("."));

        let mut error_step_1 = make_failing_step();
        error_step_1.if_fail = Some("error".to_string());
        let mut error_step_2 = make_failing_step();
        error_step_2.if_fail = Some("error".to_string());
        let mut error_step_3 = make_failing_step();
        error_step_3.if_fail = Some("error".to_string());

        let blocks = vec![make_teardown_block(vec![
            error_step_1,
            error_step_2,
            make_success_step(),
            error_step_3,
        ])];

        let result = execute_teardown(&blocks, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

        assert_eq!(
            result.errors.len(),
            3,
            "should collect all 3 errors from the block"
        );
        assert!(result.warnings.is_empty());
        for e in &result.errors {
            assert!(!e.is_empty(), "each error SHALL have a message");
        }
    }

    // ---------------------------------------------------------------
    // 10. Teardown step failure never returns Err (always Ok via TeardownResult)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn teardown_step_failure_never_returns_err() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let ctx = test_ctx(Path::new("."));

        // Mix of failure modes — none should cause execute_teardown to panic or return Err
        let mut error_step = make_failing_step();
        error_step.if_fail = Some("error".to_string());
        let mut warn_step = make_failing_step();
        warn_step.if_fail = Some("warn".to_string());
        let ignore_step = make_failing_step(); // if_fail defaults to "ignore" in teardown

        let blocks = vec![make_teardown_block(vec![
            error_step,
            warn_step,
            ignore_step,
            make_success_step(),
        ])];

        // execute_teardown always returns a TeardownResult, never an Err
        let result = execute_teardown(&blocks, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

        assert_eq!(result.errors.len(), 1, "one error from if_fail=error step");
        assert_eq!(
            result.warnings.len(),
            1,
            "one warning from if_fail=warn step"
        );

        // The last success step should have still executed.
        // test_ctx has screenshot_on_failure=false, so only the success
        // step (screenshot action) produces a driver screenshot call.
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(screenshot_calls.len(), 1);
    }

    // ---------------------------------------------------------------
    // 11. Teardown with explicit if_fail="ignore" also works
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn teardown_explicit_on_fail_ignore_works() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let ctx = test_ctx(Path::new("."));

        let mut ignore_step = make_failing_step();
        ignore_step.if_fail = Some("ignore".to_string());

        let blocks = vec![make_teardown_block(vec![
            ignore_step,
            make_success_step(),
        ])];

        let result = execute_teardown(&blocks, &driver, &mut vars, DEFAULT_TIMEOUT, &ctx).await;

        // Explicit ignore should behave the same as default ignore
        assert!(result.warnings.is_empty());
        assert!(result.errors.is_empty());

        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            1,
            "success step should still execute after ignored failure"
        );
    }

    // ---------------------------------------------------------------
    // 12. apply_teardown_defaults sets if_fail to "ignore" when absent
    // ---------------------------------------------------------------
    #[test]
    fn apply_teardown_defaults_sets_on_fail_to_ignore() {
        let step = make_success_step();
        assert!(step.if_fail.is_none(), "precondition: if_fail is None");

        let effective = apply_teardown_defaults(&step);
        assert_eq!(
            effective.if_fail.as_deref(),
            Some("ignore"),
            "should default to ignore in teardown context"
        );
    }

    // ---------------------------------------------------------------
    // 13. apply_teardown_defaults preserves explicit if_fail
    // ---------------------------------------------------------------
    #[test]
    fn apply_teardown_defaults_preserves_explicit_on_fail() {
        let mut step = make_success_step();
        step.if_fail = Some("warn".to_string());

        let effective = apply_teardown_defaults(&step);
        assert_eq!(
            effective.if_fail.as_deref(),
            Some("warn"),
            "explicit if_fail should be preserved"
        );

        let mut step2 = make_success_step();
        step2.if_fail = Some("error".to_string());

        let effective2 = apply_teardown_defaults(&step2);
        assert_eq!(
            effective2.if_fail.as_deref(),
            Some("error"),
            "explicit if_fail=error should be preserved"
        );
    }
}
