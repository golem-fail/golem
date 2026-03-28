use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use golem_driver::PlatformDriver;
use golem_parser::{Block, FlowFile};
use golem_vars::VariableStore;

use crate::branch::evaluate_branch;
use crate::context::ExecutionContext;
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
pub async fn execute_flow<'a>(
    flow: &'a FlowFile,
    driver: &dyn PlatformDriver,
    vars: &mut VariableStore,
    start_block: Option<&str>,
    default_timeout_ms: u64,
    ctx: &mut ExecutionContext<'a>,
) -> Result<FlowResult> {
    let blocks = &flow.block;

    // Find starting block index
    let mut current_idx = match start_block {
        Some(name) => find_block_index(blocks, name)?,
        None => 0,
    };

    let max_steps = flow
        .flow
        .options
        .as_ref()
        .and_then(|o| o.max_steps)
        .unwrap_or(10_000);
    let max_runtime = flow
        .flow
        .options
        .as_ref()
        .and_then(|o| o.max_runtime.as_deref())
        .and_then(parse_duration)
        .unwrap_or(Duration::from_secs(3600));
    let start_time = Instant::now();
    let mut step_count: u64 = 0;

    let mut warnings = Vec::new();

    loop {
        if current_idx >= blocks.len() {
            break; // End of flow
        }

        let block = &blocks[current_idx];

        // Skip blocks whose `where` filter doesn't match the current device.
        if let Some(ref device_filter) = block.r#where {
            if let Some(device) = ctx.device {
                let filter = crate::for_each::WhereFilter::from_device_filter(device_filter);
                if !filter.matches(device) {
                    current_idx += 1;
                    continue;
                }
            }
        }

        // Sub-flow execution: if the block has run_flow, execute the child flow
        // instead of the block's own steps.
        if let Some(ref run_flow_path) = block.run_flow {
            let config = crate::subflow::extract_subflow_config(block);
            let child_path = ctx.flow_dir.join(run_flow_path);
            let child_content = std::fs::read_to_string(&child_path)
                .with_context(|| format!("failed to read sub-flow: {}", child_path.display()))?;
            let child_flow = golem_parser::parse_flow(&child_content)?;

            let block_vars = config.as_ref().map_or(&block.vars, |c| &c.vars);
            let mut child_vars = crate::subflow::prepare_child_vars(vars, block_vars);

            // Apply the child flow's own flow-level variables.
            for (key, value) in &child_flow.flow.vars {
                child_vars.set_in_scope(
                    golem_vars::ScopeLevel::Flow,
                    key,
                    golem_vars::VarValue::String(value.clone()),
                );
            }

            // Build a child execution context scoped to the child flow's lifetime.
            let child_flow_dir = child_path
                .parent()
                .unwrap_or(ctx.flow_dir);
            let mut child_ctx = ExecutionContext {
                flow_dir: child_flow_dir,
                project_root: ctx.project_root,
                capture_config: ctx.capture_config,
                flow_name: &child_flow.flow.name,
                block_name: None,
                step_index: 0,
                device: ctx.device,
            };

            let child_result = Box::pin(execute_flow(
                &child_flow,
                driver,
                &mut child_vars,
                None,
                default_timeout_ms,
                &mut child_ctx,
            ))
            .await?;

            let save_to = config.as_ref().map_or(&block.save_to, |c| &c.save_to);
            crate::subflow::propagate_results(&child_vars, vars, save_to)?;

            if !child_result.success {
                return Ok(FlowResult {
                    success: false,
                    warnings,
                    failed_step: child_result.failed_step,
                    failed_block: block.name.clone(),
                });
            }

            // Determine next block (same logic as normal blocks)
            if !block.branch.is_empty() {
                match evaluate_branch(&block.branch, driver, vars).await? {
                    Some(target) => {
                        current_idx = find_block_index(blocks, &target)?;
                        continue;
                    }
                    None => {
                        current_idx += 1;
                    }
                }
            } else if let Some(ref next) = block.next {
                current_idx = find_block_index(blocks, next)?;
            } else {
                current_idx += 1;
            }
            continue;
        }

        // Execute steps in current block
        ctx.block_name = block.name.as_deref();
        for (step_idx, step) in block.steps.iter().enumerate() {
            ctx.step_index = step_idx;

            step_count += 1;
            if step_count > max_steps {
                bail!("max_steps ({max_steps}) exceeded at step {step_count}");
            }
            if start_time.elapsed() > max_runtime {
                bail!("max_runtime exceeded after {:?}", start_time.elapsed());
            }
            match execute_step_with_policy(step, driver, vars, default_timeout_ms, ctx).await {
                Ok(StepOutcome::Success) => {}
                Ok(StepOutcome::Warning(msg)) => warnings.push(msg),
                Ok(StepOutcome::Ignored) => {}
                Err(_e) => {
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

/// Parse a human-readable duration string into a [`Duration`].
///
/// Supported suffixes: `ms` (milliseconds), `s` (seconds), `m` (minutes), `h` (hours).
/// Returns `None` if the format is not recognised.
fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix("ms") {
        return n.trim().parse::<u64>().ok().map(Duration::from_millis);
    }
    if let Some(n) = s.strip_suffix('s') {
        return n.trim().parse::<u64>().ok().map(Duration::from_secs);
    }
    if let Some(n) = s.strip_suffix('m') {
        return n.trim().parse::<u64>().ok().map(|v| Duration::from_secs(v * 60));
    }
    if let Some(n) = s.strip_suffix('h') {
        return n.trim().parse::<u64>().ok().map(|v| Duration::from_secs(v * 3600));
    }
    None
}

/// Execute a flow once per data-driven row (or once if there are no data rows).
///
/// Returns the first failing [`FlowResult`] if any row fails, otherwise returns the
/// result of the last run (which is successful).
pub async fn execute_flow_with_data<'a>(
    flow: &'a FlowFile,
    driver: &dyn PlatformDriver,
    vars: &mut VariableStore,
    start_block: Option<&str>,
    default_timeout_ms: u64,
    ctx: &mut ExecutionContext<'a>,
) -> Result<FlowResult> {
    let runs = crate::data_driven::get_runs(&flow.data);
    let mut last_result = None;
    for run in &runs {
        if !run.vars.is_empty() {
            crate::data_driven::apply_data_vars(vars, &run.vars);
        }
        let result = execute_flow(flow, driver, vars, start_block, default_timeout_ms, ctx).await?;
        if !result.success {
            return Ok(result);
        }
        last_result = Some(result);
    }
    Ok(last_result.unwrap_or(FlowResult {
        success: true,
        warnings: Vec::new(),
        failed_step: None,
        failed_block: None,
    }))
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
    use crate::context::test_ctx;
    use golem_driver::MockPlatformDriver;
    use golem_element::{Bounds, Element};
    use golem_parser::{BranchCondition, FlowMeta, FlowOptions, Step};
    use std::collections::HashMap;
    use std::path::Path;

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

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

    fn hierarchy_with_text(texts: &[&str]) -> Element {
        let children = texts
            .iter()
            .enumerate()
            .map(|(i, t)| Element {
                element_type: "Label".to_string(),
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
            accessibility_id: None,
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
            auto_scroll: None,
            params: HashMap::new(),
        }
    }

    /// Build a step that will fail: "tap" with text that won't be found.
    fn make_failing_step() -> Step {
        Step {
            action: "tap".to_string(),
            text: Some("NONEXISTENT_ELEMENT_xyz_12345".to_string()),
            accessibility_id: None,
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
            auto_scroll: None,
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
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![make_block(
            Some("only"),
            vec![make_success_step(), make_success_step(), make_success_step()],
        )]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
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
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![
            make_block(Some("first"), vec![make_success_step()]),
            make_block(Some("second"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(screenshot_calls.len(), 2, "both blocks SHALL execute");
    }

    // ---------------------------------------------------------------
    // 3. Block with `next` jumps to named block
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn block_with_next_jumps_to_named_block() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));

        // Block order: first -> skipped -> target
        // "first" has next="target", so "skipped" should not execute
        let flow = make_flow(vec![
            make_block_with_next(Some("first"), vec![make_success_step()], "target"),
            make_block(Some("skipped"), vec![make_success_step()]),
            make_block(Some("target"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
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
        let mut ctx = test_ctx(Path::new("."));

        let flow = make_flow(vec![
            make_block_with_branch(
                Some("check"),
                vec![make_success_step()],
                vec![cond_if_visible("Welcome", "dashboard")],
            ),
            make_block(Some("login"), vec![make_success_step()]),
            make_block(Some("dashboard"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        // check + dashboard = 2 screenshots; "login" should be skipped
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(screenshot_calls.len(), 2, "login block SHALL be skipped");
    }

    // ---------------------------------------------------------------
    // 5. Start at specific block (--start)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn start_at_specific_block() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![
            make_block(Some("first"), vec![make_success_step()]),
            make_block(Some("second"), vec![make_success_step()]),
            make_block(Some("third"), vec![make_success_step()]),
        ]);

        let result =
            execute_flow(&flow, &driver, &mut vars, Some("second"), DEFAULT_TIMEOUT, &mut ctx)
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
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![make_block(Some("only"), vec![make_success_step()])]);

        let result =
            execute_flow(&flow, &driver, &mut vars, Some("nonexistent"), DEFAULT_TIMEOUT, &mut ctx).await;

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
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![make_block_with_next(
            Some("first"),
            vec![make_success_step()],
            "does_not_exist",
        )]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx).await;

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
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![
            make_block(
                Some("failing_block"),
                vec![make_success_step(), make_failing_step(), make_success_step()],
            ),
            make_block(Some("second"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
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
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![make_block(
            Some("block"),
            vec![make_success_step(), make_warn_step(), make_success_step()],
        )]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
            .await
            .expect("execute_flow should not error");

        assert!(result.success, "flow SHALL succeed despite warning");
        assert_eq!(result.warnings.len(), 1, "SHALL have collected one warning");
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
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![make_block(
            Some("block"),
            vec![make_success_step(), make_ignore_step(), make_success_step()],
        )]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
            .await
            .expect("execute_flow should not error");

        assert!(result.success, "flow SHALL succeed");
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
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
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
        let mut ctx = test_ctx(Path::new("."));

        let flow = make_flow(vec![
            make_block_with_branch(
                Some("check"),
                vec![make_success_step()],
                vec![cond_if_visible("Login", "login_block")],
            ),
            make_block(Some("fallthrough"), vec![make_success_step()]),
            make_block(Some("login_block"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
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
        let mut ctx = test_ctx(Path::new("."));

        // Document order: a(0), b(1), c(2), d(3)
        // Chain: a -> c -> d (b is skipped)
        let flow = make_flow(vec![
            make_block_with_next(Some("a"), vec![make_success_step()], "c"),
            make_block(Some("b"), vec![make_success_step()]),
            make_block_with_next(Some("c"), vec![make_success_step()], "d"),
            make_block(Some("d"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
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
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![
            make_block(
                Some("block1"),
                vec![make_warn_step(), make_success_step(), make_warn_step()],
            ),
            make_block(Some("block2"), vec![make_warn_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
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
        let mut ctx = test_ctx(Path::new("."));

        let flow = make_flow(vec![
            make_block_with_branch(
                Some("start"),
                vec![make_success_step()],
                vec![cond_default("end")],
            ),
            make_block(Some("skipped"), vec![make_success_step()]),
            make_block(Some("end"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
            .await
            .expect("execute_flow should not error");

        assert!(result.success);
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        // start + end = 2 (skipped is bypassed), then end falls through past index 2
        assert_eq!(screenshot_calls.len(), 2, "default branch SHALL jump to end");
    }

    // ---------------------------------------------------------------
    // 18. Failure in second block reports correct block name
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn failure_reports_correct_block_name() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![
            make_block(Some("block_a"), vec![make_success_step()]),
            make_block(
                Some("block_b"),
                vec![make_success_step(), make_success_step(), make_failing_step()],
            ),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
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
        let mut ctx = test_ctx(Path::new("."));
        let flow = make_flow(vec![make_block(None, vec![make_failing_step()])]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
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
        let mut ctx = test_ctx(Path::new("."));

        let flow = make_flow(vec![make_block_with_branch(
            Some("check"),
            vec![make_success_step()],
            vec![cond_default("nonexistent_target")],
        )]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx).await;

        assert!(result.is_err());
        let err_msg = result.expect_err("should be error").to_string();
        assert!(
            err_msg.contains("nonexistent_target"),
            "error should mention missing target: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // Agent B: parse_duration tests
    // ---------------------------------------------------------------

    // ---------------------------------------------------------------
    // 21. parse_duration recognises all supported formats
    // ---------------------------------------------------------------
    #[test]
    fn parse_duration_formats() {
        assert_eq!(parse_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("100ms"), Some(Duration::from_millis(100)));
        assert_eq!(parse_duration("invalid"), None);
    }

    // ---------------------------------------------------------------
    // 22. parse_duration trims whitespace
    // ---------------------------------------------------------------
    #[test]
    fn parse_duration_trims_whitespace() {
        assert_eq!(parse_duration("  30s  "), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration(" 5 m"), Some(Duration::from_secs(300)));
    }

    // ---------------------------------------------------------------
    // 23. parse_duration rejects empty string
    // ---------------------------------------------------------------
    #[test]
    fn parse_duration_rejects_empty() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("   "), None);
    }

    // ---------------------------------------------------------------
    // 24. parse_duration rejects negative / non-numeric values
    // ---------------------------------------------------------------
    #[test]
    fn parse_duration_rejects_non_numeric() {
        assert_eq!(parse_duration("abcs"), None);
        assert_eq!(parse_duration("-5s"), None);
        assert_eq!(parse_duration("3.5s"), None);
    }

    // ---------------------------------------------------------------
    // Agent B: max_steps enforcement
    // ---------------------------------------------------------------

    fn make_flow_meta_with_options(options: FlowOptions) -> FlowMeta {
        FlowMeta {
            name: "test flow".to_string(),
            start: None,
            seed: None,
            tags: Vec::new(),
            vars: HashMap::new(),
            apps: Vec::new(),
            options: Some(options),
        }
    }

    fn make_flow_with_options(blocks: Vec<Block>, options: FlowOptions) -> FlowFile {
        FlowFile {
            flow: make_flow_meta_with_options(options),
            block: blocks,
            data: Vec::new(),
            teardown: Vec::new(),
        }
    }

    // ---------------------------------------------------------------
    // 25. max_steps exceeded produces error with descriptive message
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn max_steps_exceeded() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));

        let options = FlowOptions {
            max_steps: Some(3),
            ..FlowOptions::default()
        };
        let flow = make_flow_with_options(
            vec![make_block(
                Some("big_block"),
                vec![
                    make_success_step(),
                    make_success_step(),
                    make_success_step(),
                    make_success_step(),
                    make_success_step(),
                ],
            )],
            options,
        );

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx).await;
        assert!(
            result.is_err(),
            "SHALL fail when step count exceeds max_steps"
        );
        let err_msg = result.expect_err("should be error").to_string();
        assert!(
            err_msg.contains("max_steps"),
            "error SHALL mention max_steps: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 26. max_steps exactly at limit succeeds
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn max_steps_at_limit_succeeds() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));

        let options = FlowOptions {
            max_steps: Some(3),
            ..FlowOptions::default()
        };
        let flow = make_flow_with_options(
            vec![make_block(
                Some("exact"),
                vec![make_success_step(), make_success_step(), make_success_step()],
            )],
            options,
        );

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
            .await
            .expect("execute_flow SHALL succeed when step count equals max_steps");
        assert!(result.success, "flow SHALL succeed at exact max_steps limit");
    }

    // ---------------------------------------------------------------
    // 27. Default max_steps (10_000) allows normal flows
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn default_max_steps_allows_normal_flows() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));

        let flow = make_flow(vec![make_block(
            Some("small"),
            vec![make_success_step(), make_success_step()],
        )]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
            .await
            .expect("execute_flow SHALL succeed with default limits");
        assert!(result.success);
    }

    // ---------------------------------------------------------------
    // Agent N: sub-flow execution
    // ---------------------------------------------------------------

    // ---------------------------------------------------------------
    // 28. Sub-flow block executes child flow from file
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn subflow_block_executes_child_flow() {
        let tmp_dir = tempfile::tempdir().expect("SHALL create temp dir");

        let child_toml = r#"
[flow]
name = "child flow"

[[block]]
name = "child_block"

[[block.steps]]
action = "screenshot"
"#;
        std::fs::write(tmp_dir.path().join("child.test.toml"), child_toml)
            .expect("SHALL write child flow");

        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(tmp_dir.path());

        let mut parent_block = make_block(Some("run_child"), vec![]);
        parent_block.run_flow = Some("child.test.toml".to_string());

        let flow = make_flow(vec![parent_block]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
            .await
            .expect("execute_flow SHALL succeed with sub-flow");
        assert!(result.success, "flow SHALL succeed when child flow succeeds");

        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            1,
            "child flow's screenshot step SHALL have been executed"
        );
    }

    // ---------------------------------------------------------------
    // 29. Parent continues after successful sub-flow
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn parent_continues_after_successful_subflow() {
        let tmp_dir = tempfile::tempdir().expect("SHALL create temp dir");

        let child_toml = r#"
[flow]
name = "child flow"

[[block]]
name = "child_block"

[[block.steps]]
action = "screenshot"
"#;
        std::fs::write(tmp_dir.path().join("child.test.toml"), child_toml)
            .expect("SHALL write child flow");

        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(tmp_dir.path());

        let mut subflow_block = make_block(Some("run_child"), vec![]);
        subflow_block.run_flow = Some("child.test.toml".to_string());

        let flow = make_flow(vec![
            subflow_block,
            make_block(Some("after"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
            .await
            .expect("execute_flow SHALL succeed");
        assert!(
            result.success,
            "flow SHALL succeed when both parent and child complete"
        );

        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            2,
            "parent SHALL continue executing after successful sub-flow"
        );
    }

    // ---------------------------------------------------------------
    // 30. Sub-flow failure stops parent flow
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn subflow_failure_stops_parent_flow() {
        let tmp_dir = tempfile::tempdir().expect("SHALL create temp dir");

        let child_toml = r#"
[flow]
name = "failing child"

[[block]]
name = "child_fail"

[[block.steps]]
action = "tap"
text = "NONEXISTENT_ELEMENT_xyz_12345"
"#;
        std::fs::write(tmp_dir.path().join("fail_child.test.toml"), child_toml)
            .expect("SHALL write child flow");

        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(tmp_dir.path());

        let mut subflow_block = make_block(Some("run_fail_child"), vec![]);
        subflow_block.run_flow = Some("fail_child.test.toml".to_string());

        let flow = make_flow(vec![
            subflow_block,
            make_block(Some("never_reached"), vec![make_success_step()]),
        ]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
            .await
            .expect("execute_flow SHALL return FlowResult, not Err");
        assert!(
            !result.success,
            "flow SHALL fail when sub-flow fails"
        );
        assert_eq!(
            result.failed_block,
            Some("run_fail_child".to_string()),
            "failed_block SHALL be the parent block that ran the sub-flow"
        );
    }

    // ---------------------------------------------------------------
    // 31. Sub-flow with missing file returns error
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn subflow_missing_file_returns_error() {
        let tmp_dir = tempfile::tempdir().expect("SHALL create temp dir");
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(tmp_dir.path());

        let mut subflow_block = make_block(Some("missing"), vec![]);
        subflow_block.run_flow = Some("does_not_exist.test.toml".to_string());

        let flow = make_flow(vec![subflow_block]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx).await;
        assert!(
            result.is_err(),
            "SHALL return Err when sub-flow file does not exist"
        );
        let err_msg = result.expect_err("should be error").to_string();
        assert!(
            err_msg.contains("sub-flow"),
            "error SHALL mention sub-flow: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 32. Sub-flow propagates variables back via save_to
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn subflow_propagates_variables_via_save_to() {
        use golem_vars::{Scope, ScopeLevel, VarValue};

        let tmp_dir = tempfile::tempdir().expect("SHALL create temp dir");

        let child_toml = r#"
[flow]
name = "child with var"

[flow.vars]
token = "jwt-abc-123"

[[block]]
name = "child_block"

[[block.steps]]
action = "screenshot"
"#;
        std::fs::write(tmp_dir.path().join("child_var.test.toml"), child_toml)
            .expect("SHALL write child flow");

        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Flow);
        scope.set("existing", VarValue::string("keep me"));
        vars.push_scope(scope);

        let mut ctx = test_ctx(tmp_dir.path());

        let mut subflow_block = make_block(Some("run_child"), vec![]);
        subflow_block.run_flow = Some("child_var.test.toml".to_string());
        subflow_block
            .save_to
            .insert("token".to_string(), "session_token".to_string());

        let flow = make_flow(vec![subflow_block]);

        let result = execute_flow(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
            .await
            .expect("execute_flow SHALL succeed");
        assert!(result.success);

        assert_eq!(
            vars.get("session_token"),
            Some(&VarValue::string("jwt-abc-123")),
            "save_to SHALL propagate child variable back to parent"
        );
        assert_eq!(
            vars.get("existing"),
            Some(&VarValue::string("keep me")),
            "parent variables SHALL be preserved after sub-flow"
        );
    }

    // ---------------------------------------------------------------
    // Agent N: data-driven row execution
    // ---------------------------------------------------------------

    // ---------------------------------------------------------------
    // 33. execute_flow_with_data runs once when no data rows
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn execute_flow_with_data_no_rows_runs_once() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));

        let flow = make_flow(vec![make_block(
            Some("only"),
            vec![make_success_step()],
        )]);

        let result =
            execute_flow_with_data(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
                .await
                .expect("execute_flow_with_data SHALL succeed");
        assert!(result.success);

        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            1,
            "SHALL execute flow exactly once when there are no data rows"
        );
    }

    // ---------------------------------------------------------------
    // 34. execute_flow_with_data runs once per data row
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn execute_flow_with_data_runs_per_row() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));

        let mut flow = make_flow(vec![make_block(
            Some("step_block"),
            vec![make_success_step()],
        )]);
        flow.data = vec![
            HashMap::from([("user".to_string(), "alice".to_string())]),
            HashMap::from([("user".to_string(), "bob".to_string())]),
            HashMap::from([("user".to_string(), "charlie".to_string())]),
        ];

        let result =
            execute_flow_with_data(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
                .await
                .expect("execute_flow_with_data SHALL succeed");
        assert!(result.success);

        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(
            screenshot_calls.len(),
            3,
            "SHALL execute flow once per data row (3 rows = 3 executions)"
        );
    }

    // ---------------------------------------------------------------
    // 35. execute_flow_with_data stops on first failing row
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn execute_flow_with_data_stops_on_failure() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));

        let mut flow = make_flow(vec![make_block(
            Some("fail_block"),
            vec![make_failing_step()],
        )]);
        flow.data = vec![
            HashMap::from([("user".to_string(), "alice".to_string())]),
            HashMap::from([("user".to_string(), "bob".to_string())]),
        ];

        let result =
            execute_flow_with_data(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
                .await
                .expect("execute_flow_with_data SHALL return FlowResult");
        assert!(
            !result.success,
            "SHALL fail when any data row fails"
        );
    }

    // ---------------------------------------------------------------
    // 36. execute_flow_with_data applies row variables
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn execute_flow_with_data_applies_row_variables() {
        use golem_vars::VarValue;

        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));

        let mut flow = make_flow(vec![make_block(
            Some("step_block"),
            vec![make_success_step()],
        )]);
        flow.data = vec![HashMap::from([
            ("payment".to_string(), "credit_card".to_string()),
        ])];

        let result =
            execute_flow_with_data(&flow, &driver, &mut vars, None, DEFAULT_TIMEOUT, &mut ctx)
                .await
                .expect("execute_flow_with_data SHALL succeed");
        assert!(result.success);

        assert_eq!(
            vars.resolve("payment").ok(),
            Some(&VarValue::String("credit_card".to_string())),
            "row variables SHALL be applied to the variable store"
        );
    }

    // ---------------------------------------------------------------
    // Where clause: block skipped when device doesn't match
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn where_clause_skips_non_matching_block() {
        let driver = MockPlatformDriver::new(empty_hierarchy());

        let mut android_only = make_block(Some("android_only"), vec![make_success_step()]);
        android_only.r#where = Some(golem_parser::DeviceFilter {
            device_type: None,
            os: Some("android".to_string()),
            physical: None,
        });

        let flow = make_flow(vec![android_only]);
        let mut vars = VariableStore::new();
        let tmp = std::env::temp_dir();

        // Use an iOS device — the android-only block should be skipped.
        let ios_device = golem_devices::DeviceInfo {
            name: "iPhone 15".to_string(),
            udid: "test-udid".to_string(),
            platform: golem_devices::Platform::Ios,
            device_type: golem_devices::DeviceType::Phone,
            os_major: 17,
            os_version: "17.2".to_string(),
            state: golem_devices::DeviceState::Booted,
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        };

        let capture = crate::capture::CaptureConfig::default();
        let mut ctx = ExecutionContext {
            flow_dir: &tmp,
            project_root: &tmp,
            capture_config: &capture,
            flow_name: "test",
            block_name: None,
            step_index: 0,
            device: Some(&ios_device),
        };

        let result = execute_flow(&flow, &driver, &mut vars, None, 10_000, &mut ctx)
            .await
            .unwrap();

        assert!(result.success, "flow SHALL succeed when block is skipped");
        assert!(
            driver.get_calls().is_empty(),
            "no driver calls SHALL be made when the only block is skipped"
        );
    }

    // ---------------------------------------------------------------
    // Where clause: block executes when device matches
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn where_clause_executes_matching_block() {
        let driver = MockPlatformDriver::new(empty_hierarchy());

        let mut android_only = make_block(Some("android_only"), vec![make_success_step()]);
        android_only.r#where = Some(golem_parser::DeviceFilter {
            device_type: None,
            os: Some("android".to_string()),
            physical: None,
        });

        let flow = make_flow(vec![android_only]);
        let mut vars = VariableStore::new();
        let tmp = std::env::temp_dir();

        let android_device = golem_devices::DeviceInfo {
            name: "Pixel 8".to_string(),
            udid: "emulator-5554".to_string(),
            platform: golem_devices::Platform::Android,
            device_type: golem_devices::DeviceType::Phone,
            os_major: 14,
            os_version: "14".to_string(),
            state: golem_devices::DeviceState::Booted,
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        };

        let capture = crate::capture::CaptureConfig::default();
        let mut ctx = ExecutionContext {
            flow_dir: &tmp,
            project_root: &tmp,
            capture_config: &capture,
            flow_name: "test",
            block_name: None,
            step_index: 0,
            device: Some(&android_device),
        };

        let result = execute_flow(&flow, &driver, &mut vars, None, 10_000, &mut ctx)
            .await
            .unwrap();

        assert!(result.success, "flow SHALL succeed when block matches");
        assert!(
            !driver.get_calls().is_empty(),
            "driver SHALL be called when the block's where matches the device"
        );
    }

    // ---------------------------------------------------------------
    // Where clause: mixed blocks — only matching ones execute
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn where_clause_mixed_blocks_only_matching_execute() {
        let driver = MockPlatformDriver::new(empty_hierarchy());

        let mut ios_block = make_block(Some("ios_only"), vec![make_success_step()]);
        ios_block.r#where = Some(golem_parser::DeviceFilter {
            device_type: None,
            os: Some("ios".to_string()),
            physical: None,
        });

        let mut android_block = make_block(Some("android_only"), vec![make_success_step()]);
        android_block.r#where = Some(golem_parser::DeviceFilter {
            device_type: None,
            os: Some("android".to_string()),
            physical: None,
        });

        let shared_block = make_block(Some("shared"), vec![make_success_step()]);

        let flow = make_flow(vec![ios_block, android_block, shared_block]);
        let mut vars = VariableStore::new();
        let tmp = std::env::temp_dir();

        let ios_device = golem_devices::DeviceInfo {
            name: "iPhone 15".to_string(),
            udid: "test-udid".to_string(),
            platform: golem_devices::Platform::Ios,
            device_type: golem_devices::DeviceType::Phone,
            os_major: 17,
            os_version: "17.2".to_string(),
            state: golem_devices::DeviceState::Booted,
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        };

        let capture = crate::capture::CaptureConfig::default();
        let mut ctx = ExecutionContext {
            flow_dir: &tmp,
            project_root: &tmp,
            capture_config: &capture,
            flow_name: "test",
            block_name: None,
            step_index: 0,
            device: Some(&ios_device),
        };

        let result = execute_flow(&flow, &driver, &mut vars, None, 10_000, &mut ctx)
            .await
            .unwrap();

        assert!(result.success);
        // iOS block + shared block = 2 screenshot calls; android block skipped
        let calls = driver.get_calls();
        assert_eq!(
            calls.len(),
            2,
            "only the ios and shared blocks SHALL execute (got {calls:?})"
        );
    }
}
