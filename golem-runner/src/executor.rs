use anyhow::{bail, Result};
use golem_driver::PlatformDriver;
use golem_parser::{Block, FlowFile};
use golem_vars::VariableStore;

use crate::branch::evaluate_branch;
use crate::policy::{execute_step_with_policy, StepOutcome};

/// The result of executing a complete flow.
#[derive(Debug)]
pub struct FlowResult {
    /// Whether the flow completed without step failures.
    pub success: bool,
    /// Warnings collected from steps with `on_fail = "warn"`.
    pub warnings: Vec<String>,
    /// The index of the step that failed (within its block), if any.
    pub failed_step: Option<usize>,
    /// The name of the block containing the failed step, if any.
    pub failed_block: Option<String>,
}

/// Execute a parsed FlowFile by traversing blocks in order.
///
/// Block traversal:
/// 1. Start at the first block (or the block named by `start_block`).
/// 2. Execute all steps in the current block via [`execute_step_with_policy`].
/// 3. After all steps complete:
///    a. If the block has `branch` conditions, evaluate via [`evaluate_branch`] and goto target.
///    b. If the block has `next`, jump to that named block.
///    c. Otherwise, fall through to the next block in document order.
/// 4. When no more blocks remain, the flow ends successfully.
/// 5. If a goto/next targets a non-existent block, return an error.
pub async fn execute_flow(
    flow: &FlowFile,
    driver: &dyn PlatformDriver,
    vars: &mut VariableStore,
    start_block: Option<&str>,
    default_timeout_ms: u64,
) -> Result<FlowResult> {
    let blocks = &flow.block;

    // Find starting block index
    let mut current_idx = match start_block {
        Some(name) => find_block_index(blocks, name)?,
        None => 0,
    };

    let mut warnings = Vec::new();

    loop {
        if current_idx >= blocks.len() {
            break; // End of flow
        }

        let block = &blocks[current_idx];

        // Execute steps in current block
        for (step_idx, step) in block.steps.iter().enumerate() {
            match execute_step_with_policy(step, driver, vars, default_timeout_ms).await {
                Ok(StepOutcome::Success) => {}
                Ok(StepOutcome::Warning(msg)) => warnings.push(msg),
                Ok(StepOutcome::Ignored) => {}
                Err(_) => {
                    return Ok(FlowResult {
                        success: false,
                        warnings,
                        failed_step: Some(step_idx),
                        failed_block: block.name.clone(),
                    });
                }
            }
        }

        // Determine next block
        if !block.branch.is_empty() {
            match evaluate_branch(&block.branch, driver, vars).await? {
                Some(target) => {
                    current_idx = find_block_index(blocks, &target)?;
                    continue;
                }
                None => {
                    current_idx += 1; // No branch matched, fall through
                }
            }
        } else if let Some(ref next) = block.next {
            current_idx = find_block_index(blocks, next)?;
        } else {
            current_idx += 1; // Fall through
        }
    }

    Ok(FlowResult {
        success: true,
        warnings,
        failed_step: None,
        failed_block: None,
    })
}

/// Find the index of a block by name. Returns an error if not found.
fn find_block_index(blocks: &[Block], name: &str) -> Result<usize> {
    for (i, block) in blocks.iter().enumerate() {
        if block.name.as_deref() == Some(name) {
            return Ok(i);
        }
    }
    bail!("Block not found: {name}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::{Bounds, Element};
    use golem_parser::{BranchCondition, FlowMeta, Step};
    use std::collections::HashMap;

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

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

    fn hierarchy_with_text(texts: &[&str]) -> Element {
        let children = texts
            .iter()
            .enumerate()
            .map(|(i, t)| Element {
                element_type: "Label".to_string(),
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

    fn make_flow_meta() -> FlowMeta {
        FlowMeta {
            name: "test flow".to_string(),
            start: None,
            seed: None,
            tags: Vec::new(),
            vars: HashMap::new(),
            apps: Vec::new(),
            options: None,
        }
    }

    fn make_flow(blocks: Vec<Block>) -> FlowFile {
        FlowFile {
            flow: make_flow_meta(),
            block: blocks,
            data: Vec::new(),
            teardown: Vec::new(),
        }
    }

    fn make_block(name: Option<&str>, steps: Vec<Step>) -> Block {
        Block {
            name: name.map(|s| s.to_string()),
            app: None,
            steps,
            next: None,
            branch: Vec::new(),
            for_each: None,
            r#where: None,
            run_flow: None,
            max_iterations: None,
            vars: HashMap::new(),
            save_to: HashMap::new(),
        }
    }

    fn make_block_with_next(name: Option<&str>, steps: Vec<Step>, next: &str) -> Block {
        let mut block = make_block(name, steps);
        block.next = Some(next.to_string());
        block
    }

    fn make_block_with_branch(
        name: Option<&str>,
        steps: Vec<Step>,
        branch: Vec<BranchCondition>,
    ) -> Block {
        let mut block = make_block(name, steps);
        block.branch = branch;
        block
    }

    /// Build a step that will succeed: "screenshot" requires no element resolution
    /// and works with the MockPlatformDriver.
    fn make_success_step() -> Step {
        Step {
            action: "screenshot".to_string(),
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

    /// Build a step that will fail: "tap" with text that won't be found.
    fn make_failing_step() -> Step {
        Step {
            action: "tap".to_string(),
            text: Some("NONEXISTENT_ELEMENT_xyz_12345".to_string()),
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

    fn make_warn_step() -> Step {
        let mut step = make_failing_step();
        step.on_fail = Some("warn".to_string());
        step
    }

    fn make_ignore_step() -> Step {
        let mut step = make_failing_step();
        step.on_fail = Some("ignore".to_string());
        step
    }

    fn cond_default(goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_var: None,
            equals: None,
            matches: None,
            gte: None,
            goto: goto.to_string(),
        }
    }

    fn cond_if_visible(text: &str, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: Some(text.to_string()),
            if_not_visible: None,
            if_var: None,
            equals: None,
            matches: None,
            gte: None,
            goto: goto.to_string(),
        }
    }

    const DEFAULT_TIMEOUT: u64 = 10_000;

    // ---------------------------------------------------------------
    // 1. Single block with steps executes all steps
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn single_block_executes_all_steps() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![make_block(
            Some("only"),
            vec![make_success_step(), make_success_step(), make_success_step()],
        )]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        assert!(result.warnings.is_empty());
        assert!(result.failed_step.is_none());
        assert!(result.failed_block.is_none());

        // Verify all 3 screenshot calls were made
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(screenshot_calls.len(), 3);
    }

    // ---------------------------------------------------------------
    // 2. Two blocks execute in document order (fall-through)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn two_blocks_fall_through_in_order() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![
            make_block(Some("first"), vec![make_success_step()]),
            make_block(Some("second"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(screenshot_calls.len(), 2, "both blocks should execute");
    }

    // ---------------------------------------------------------------
    // 3. Block with `next` jumps to named block
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn block_with_next_jumps_to_named_block() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();

        // Block order: first -> skipped -> target
        // "first" has next="target", so "skipped" should not execute
        let flow = make_flow(vec![
            make_block_with_next(Some("first"), vec![make_success_step()], "target"),
            make_block(Some("skipped"), vec![make_success_step()]),
            make_block(Some("target"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        // first + target = 2 screenshots; "skipped" should not run
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            2,
            "skipped block should not execute"
        );
    }

    // ---------------------------------------------------------------
    // 4. Block with `branch` evaluates and jumps
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn block_with_branch_evaluates_and_jumps() {
        // Set up a hierarchy that has "Welcome" visible
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Welcome"]));
        let mut vars = VariableStore::new();

        let flow = make_flow(vec![
            make_block_with_branch(
                Some("check"),
                vec![make_success_step()],
                vec![cond_if_visible("Welcome", "dashboard")],
            ),
            make_block(Some("login"), vec![make_success_step()]),
            make_block(Some("dashboard"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        // check + dashboard = 2 screenshots; "login" should be skipped
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(screenshot_calls.len(), 2, "login block should be skipped");
    }

    // ---------------------------------------------------------------
    // 5. Start at specific block (--start)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn start_at_specific_block() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![
            make_block(Some("first"), vec![make_success_step()]),
            make_block(Some("second"), vec![make_success_step()]),
            make_block(Some("third"), vec![make_success_step()]),
        ]);

        let result =
            execute_flow(&flow, &driver, &mut vars, Some("second"), DEFAULT_TIMEOUT)
                .await
                .expect("execute_flow should not error");

        assert!(result.success);
        // Starting at "second" means second + third = 2 screenshots
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            2,
            "should start at second block and fall through to third"
        );
    }

    // ---------------------------------------------------------------
    // 6. Invalid start block returns error
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn invalid_start_block_returns_error() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![make_block(Some("only"), vec![make_success_step()])]);

        let result =
            execute_flow(&flow, &driver, &mut vars, Some("nonexistent"), DEFAULT_TIMEOUT).await;

        assert!(result.is_err());
        let err_msg = result.expect_err("should be error").to_string();
        assert!(
            err_msg.contains("nonexistent"),
            "error should mention missing block name: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 7. Invalid next target returns error
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn invalid_next_target_returns_error() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![make_block_with_next(
            Some("first"),
            vec![make_success_step()],
            "does_not_exist",
        )]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT).await;

        assert!(result.is_err());
        let err_msg = result.expect_err("should be error").to_string();
        assert!(
            err_msg.contains("does_not_exist"),
            "error should mention missing target: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 8. Step failure stops flow, returns failed block/step info
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn step_failure_stops_flow() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![
            make_block(
                Some("failing_block"),
                vec![make_success_step(), make_failing_step(), make_success_step()],
            ),
            make_block(Some("second"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should return Ok(FlowResult), not Err");

        assert!(!result.success);
        assert_eq!(result.failed_step, Some(1), "second step (index 1) failed");
        assert_eq!(
            result.failed_block,
            Some("failing_block".to_string()),
            "should report the block name"
        );
    }

    // ---------------------------------------------------------------
    // 9. Step with on_fail="warn" collects warning and continues
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn step_with_on_fail_warn_collects_warning() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![make_block(
            Some("block"),
            vec![make_success_step(), make_warn_step(), make_success_step()],
        )]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should not error");

        assert!(result.success, "flow should succeed despite warning");
        assert_eq!(result.warnings.len(), 1, "should have collected one warning");
        assert!(
            !result.warnings[0].is_empty(),
            "warning message should not be empty"
        );
    }

    // ---------------------------------------------------------------
    // 10. Step with on_fail="ignore" continues silently
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn step_with_on_fail_ignore_continues_silently() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![make_block(
            Some("block"),
            vec![make_success_step(), make_ignore_step(), make_success_step()],
        )]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should not error");

        assert!(result.success, "flow should succeed");
        assert!(
            result.warnings.is_empty(),
            "ignored steps should not produce warnings"
        );
    }

    // ---------------------------------------------------------------
    // 11. Empty flow (no blocks) succeeds
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn empty_flow_succeeds() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        assert!(result.warnings.is_empty());
        assert!(result.failed_step.is_none());
        assert!(result.failed_block.is_none());
    }

    // ---------------------------------------------------------------
    // 12. Branch with no match falls through to next block
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn branch_no_match_falls_through() {
        // "Login" is NOT visible, so the branch condition won't match
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();

        let flow = make_flow(vec![
            make_block_with_branch(
                Some("check"),
                vec![make_success_step()],
                vec![cond_if_visible("Login", "login_block")],
            ),
            make_block(Some("fallthrough"), vec![make_success_step()]),
            make_block(Some("login_block"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        // check + fallthrough + login_block = 3 screenshots (fallthrough falls into login_block)
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            3,
            "no branch match should fall through to next block"
        );
    }

    // ---------------------------------------------------------------
    // 13. Multiple blocks with next chain (no loops)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn next_chain_no_loop() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();

        // Document order: a(0), b(1), c(2), d(3)
        // Chain: a -> c -> d (b is skipped)
        let flow = make_flow(vec![
            make_block_with_next(Some("a"), vec![make_success_step()], "c"),
            make_block(Some("b"), vec![make_success_step()]),
            make_block_with_next(Some("c"), vec![make_success_step()], "d"),
            make_block(Some("d"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        // a -> c -> d -> end (d falls through past index 3)
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            3,
            "should execute a, c, d (skipping b)"
        );
    }

    // ---------------------------------------------------------------
    // 14. Flow result includes all collected warnings
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn flow_result_includes_all_warnings() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![
            make_block(
                Some("block1"),
                vec![make_warn_step(), make_success_step(), make_warn_step()],
            ),
            make_block(Some("block2"), vec![make_warn_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        assert_eq!(
            result.warnings.len(),
            3,
            "should have 3 warnings (2 from block1, 1 from block2)"
        );
    }

    // ---------------------------------------------------------------
    // 15. find_block_index returns correct index
    // ---------------------------------------------------------------
    #[test]
    fn find_block_index_returns_correct_index() {
        let blocks = vec![
            make_block(Some("alpha"), vec![]),
            make_block(Some("beta"), vec![]),
            make_block(Some("gamma"), vec![]),
        ];

        assert_eq!(
            find_block_index(&blocks, "alpha").expect("should find alpha"),
            0
        );
        assert_eq!(
            find_block_index(&blocks, "beta").expect("should find beta"),
            1
        );
        assert_eq!(
            find_block_index(&blocks, "gamma").expect("should find gamma"),
            2
        );
    }

    // ---------------------------------------------------------------
    // 16. find_block_index errors on missing block
    // ---------------------------------------------------------------
    #[test]
    fn find_block_index_errors_on_missing() {
        let blocks = vec![make_block(Some("alpha"), vec![])];

        let result = find_block_index(&blocks, "missing");
        assert!(result.is_err());
        let err_msg = result.expect_err("should be error").to_string();
        assert!(err_msg.contains("missing"));
    }

    // ---------------------------------------------------------------
    // 17. Block with branch default (unconditional goto)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn branch_with_default_goto() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();

        let flow = make_flow(vec![
            make_block_with_branch(
                Some("start"),
                vec![make_success_step()],
                vec![cond_default("end")],
            ),
            make_block(Some("skipped"), vec![make_success_step()]),
            make_block(Some("end"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        // start + end = 2 (skipped is bypassed), then end falls through past index 2
        assert_eq!(screenshot_calls.len(), 2, "default branch should jump to end");
    }

    // ---------------------------------------------------------------
    // 18. Failure in second block reports correct block name
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn failure_reports_correct_block_name() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![
            make_block(Some("block_a"), vec![make_success_step()]),
            make_block(
                Some("block_b"),
                vec![make_success_step(), make_success_step(), make_failing_step()],
            ),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should return FlowResult");

        assert!(!result.success);
        assert_eq!(result.failed_block, Some("block_b".to_string()));
        assert_eq!(result.failed_step, Some(2), "third step (index 2) failed");
    }

    // ---------------------------------------------------------------
    // 19. Block without name reports None for failed_block
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn unnamed_block_reports_none_for_failed_block() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let flow = make_flow(vec![make_block(None, vec![make_failing_step()])]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT)
            .await
            .expect("execute_flow should return FlowResult");

        assert!(!result.success);
        assert_eq!(result.failed_block, None, "unnamed block has no name to report");
        assert_eq!(result.failed_step, Some(0));
    }

    // ---------------------------------------------------------------
    // 20. Invalid branch target returns error
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn invalid_branch_target_returns_error() {
        // Set up a branch that matches and targets a nonexistent block
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();

        let flow = make_flow(vec![make_block_with_branch(
            Some("check"),
            vec![make_success_step()],
            vec![cond_default("nonexistent_target")],
        )]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT).await;

        assert!(result.is_err());
        let err_msg = result.expect_err("should be error").to_string();
        assert!(
            err_msg.contains("nonexistent_target"),
            "error should mention missing target: {err_msg}"
        );
    }
}
