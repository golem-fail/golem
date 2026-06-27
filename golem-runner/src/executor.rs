use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use golem_driver::PlatformDriver;
use golem_parser::{Block, FlowFile, FlowOptions};
use golem_report::PerfSnapshot;
use golem_vars::{ScopeLevel, VarValue, VariableStore};

use crate::barrier::FailureBarrier;
use crate::branch::evaluate_branch;
use crate::context::ExecutionContext;
use crate::perf::RawPerfData;
use crate::policy::{execute_step_with_policy, StepOutcome};

/// Build a target label for a step (excludes action name).
/// E.g. `on_text="Submit"` or `app="app"`.
fn step_target(step: &golem_parser::Step) -> String {
    let mut parts = Vec::new();
    if let Some(ref t) = step.on_text {
        parts.push(format!("on_text=\"{t}\""));
    }
    if let Some(ref a) = step.on_accessibility_label {
        parts.push(format!("on_accessibility_label=\"{a}\""));
    }
    if let Some(ref g) = step.on {
        if let Some(ref t) = g.text {
            parts.push(format!("text=\"{t}\""));
        }
        if let Some(ref a) = g.accessibility_label {
            parts.push(format!("accessibility_label=\"{a}\""));
        }
    }
    if let Some(ref b) = step.on_below {
        parts.push(format!("on_below=\"{b}\""));
    }
    if let Some(ref r) = step.on_right_of {
        parts.push(format!("on_right_of=\"{r}\""));
    }
    if let Some(ref a) = step.app {
        parts.push(format!("app=\"{a}\""));
    }
    if let Some(ref i) = step.input {
        // Truncate by CHARS, not bytes — `&i[..20]` panics when byte 20 lands
        // inside a multibyte char (e.g. typing Japanese / emoji).
        let shown: String = i.chars().take(20).collect();
        parts.push(format!("input=\"{shown}\""));
    }
    if step.auto_scroll == Some(true) {
        parts.push("auto_scroll".to_string());
    }
    if let Some(t) = step.timeout {
        parts.push(format!("timeout={t}"));
    }
    parts.join(" ")
}

/// The result of executing a complete flow.
#[derive(Debug)]
pub struct FlowResult {
    /// Whether the flow completed without step failures.
    pub success: bool,
    /// Warnings collected from steps with `if_fail = "warn"`.
    pub warnings: Vec<String>,
    /// The index of the step that failed (within its block), if any.
    pub failed_step: Option<usize>,
    /// The name of the block containing the failed step, if any.
    pub failed_block: Option<String>,
    /// The action that failed (e.g. "tap", "launch", "assert_visible").
    pub failed_action: Option<String>,
    /// The error message from the failed step.
    pub failed_reason: Option<String>,
    /// The failure code of the failed step.
    pub failed_code: Option<golem_events::FailureCode>,
    /// True if this flow was aborted because another device failed (barrier).
    pub barrier_aborted: bool,
    /// Performance snapshots captured at block boundaries.
    pub perf_snapshots: Vec<golem_report::PerfSnapshot>,
    /// Screen recordings produced during this flow execution.
    pub recordings: Vec<golem_report::RecordingEntry>,
    /// Accessibility audits captured at block boundaries.
    pub a11y_audits: Vec<golem_report::A11yAudit>,
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
    barrier: Option<&FailureBarrier>,
) -> Result<FlowResult> {
    let blocks = &flow.block;

    // Seed [flow.vars], evaluating `fake:` generators with the flow RNG so
    // generated data works wherever vars are declared (not just fixtures).
    // Runs for the top-level flow and — via the recursive call in the
    // run_flow branch — for sub-flows. CLI `--var` overrides sit in a
    // higher-priority Cli scope, so they still win.
    seed_vars_with_generators(&flow.flow.vars, vars, ScopeLevel::Flow, &ctx.rng)?;

    // Refine the inherited record-default for this flow level. The
    // current flow's `[flow.options].record` wins over what the caller
    // (parent flow, or top-level resolver) handed in. Project-level
    // `[options].record` is folded in at the top-level entry so we
    // don't re-consult it here. CLI overrides (`--record` /
    // `--no-record`) are applied per-block on top of this.
    if let Some(v) = flow.flow.options.as_ref().and_then(|o| o.record) {
        ctx.inherited_record_default = v;
    }

    // App lifecycle management. Defaults to Reset for all flows.
    // When --start is used, skip app lifecycle — caller assumes app is
    // already running and in the correct state for the target block.
    let lifecycle = if start_block.is_some() {
        golem_parser::AppLifecycle::Manual
    } else {
        flow.flow
            .options
            .as_ref()
            .and_then(|o| o.app_lifecycle)
            .unwrap_or(golem_parser::AppLifecycle::Reset)
    };

    let apps = &flow.flow.apps;
    // Lifecycle stop/launch budget. The companion's handleLaunch runs
    // three sequential 5s `waitForExistence` probes (foregrounded,
    // window, staticText) — worst case 15s on a cold iOS 26 launch.
    // Matching that exactly with `× 3` left no safety margin: the
    // runner's deadline fired the same instant the companion was
    // still finishing its third probe. `× 5` gives headroom for
    // request-routing latency, the off-main watchdog dispatch, and
    // small first-launch slowdowns without uncapping forever.
    let lifecycle_timeout = std::time::Duration::from_millis(default_timeout_ms * 5);
    match lifecycle {
        golem_parser::AppLifecycle::Reset => {
            for app in apps {
                if let Some(bundle) = app.bundle.as_deref() {
                    let _ = tokio::time::timeout(lifecycle_timeout, driver.stop_app(bundle)).await;
                }
            }
            if let Some(app) = apps.first() {
                let bundle = app.bundle.as_deref()
                    .ok_or_else(|| anyhow::anyhow!(
                        "app '{}' has no bundle id — add one to [[flow.apps]] or to [[apps]] in golem.toml",
                        app.name))?;
                tokio::time::timeout(lifecycle_timeout, driver.launch_app(bundle))
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!(
                            "app_lifecycle reset: launch of {} timed out after {}ms \
                         (companion unresponsive?)",
                            bundle,
                            lifecycle_timeout.as_millis()
                        )
                    })?
                    .with_context(|| format!("app_lifecycle reset: failed to launch {}", bundle))?;
            }
        }
        golem_parser::AppLifecycle::Launch => {
            if let Some(app) = apps.first() {
                let bundle = app.bundle.as_deref()
                    .ok_or_else(|| anyhow::anyhow!(
                        "app '{}' has no bundle id — add one to [[flow.apps]] or to [[apps]] in golem.toml",
                        app.name))?;
                tokio::time::timeout(lifecycle_timeout, driver.launch_app(bundle))
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!(
                            "app_lifecycle launch: launch of {} timed out after {}ms \
                         (companion unresponsive?)",
                            bundle,
                            lifecycle_timeout.as_millis()
                        )
                    })?
                    .with_context(|| {
                        format!("app_lifecycle launch: failed to launch {}", bundle)
                    })?;
            }
        }
        golem_parser::AppLifecycle::Manual => {}
    }

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
    let mut perf_snapshots: Vec<PerfSnapshot> = Vec::new();
    let mut recordings: Vec<golem_report::RecordingEntry> = Vec::new();
    let mut a11y_audits: Vec<golem_report::A11yAudit> = Vec::new();
    let mut block_iterations: HashMap<usize, u32> = HashMap::new();

    // `--trace` boundary state, lazily populated when a recording block
    // starts. One tracker active at a time since blocks don't nest.
    // Reset to None when the block recording stops (sidecar flushed).
    let mut block_trace: Option<BlockTrace> = None;

    // Pre-first-step capture (boundary 0). Only fire at the topmost
    // entry into a flow — subflows pick up an already-incremented
    // global_step_index from the parent, so checking == 0 distinguishes
    // them. No tracker yet (no block recording has started); the trace
    // sits in the trace/ dir but won't appear in any sidecar. That's
    // fine — pre-flow context is rarely consulted via sidecar.
    if ctx.capture_config.trace && ctx.global_step_index == 0 {
        if let Err(e) = crate::capture::capture_trace_boundary(
            driver,
            ctx.capture_config,
            0,
            "start",
            crate::capture::TraceMeta {
                after_step: None,
                action: None,
                wall_clock: &iso8601_now(),
            },
        )
        .await
        {
            warnings.push(format!("trace boundary 0 capture failed: {e}"));
        }
    }
    let perf_enabled = flow
        .flow
        .options
        .as_ref()
        .and_then(|o| o.perf)
        .unwrap_or(true);

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

            // Inherit the parent store, then seed the block-level overrides
            // passed to the sub-flow — evaluating `fake:` generators with the
            // (parent) flow RNG. The child flow's own `[flow.vars]` are seeded
            // by the recursive `execute_flow` below, so they're not applied
            // here.
            let block_vars = config.as_ref().map_or(&block.vars, |c| &c.vars);
            let mut child_vars = crate::subflow::prepare_child_vars(vars, &HashMap::new());
            seed_vars_with_generators(
                block_vars,
                &mut child_vars,
                golem_vars::ScopeLevel::Flow,
                &ctx.rng,
            )?;

            // Build a child execution context scoped to the child flow's lifetime.
            let child_flow_dir = child_path.parent().unwrap_or(ctx.flow_dir);
            // Derive child RNG from parent for deterministic sub-flow
            // generation. `child()` carries the parent's date anchor — the run
            // has one "now" shared by every (sub-)flow.
            let child_rng = ctx
                .rng
                .lock()
                .expect("parent rng mutex poisoned")
                .child();
            let mut child_ctx = ExecutionContext {
                flow_dir: child_flow_dir,
                project_root: ctx.project_root,
                capture_config: ctx.capture_config,
                flow_name: &child_flow.flow.name,
                block_name: None,
                step_index: 0,
                global_step_index: ctx.global_step_index,
                block_iteration: 0,
                device: ctx.device,
                perf_collector: ctx.perf_collector,
                last_launch_ms: std::sync::atomic::AtomicU64::new(0),
                emitter: ctx.emitter,
                a11y_level: crate::accessibility::A11yLevel::Off,
                step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),            last_settled_tree: std::sync::Mutex::new(None),
                rng: std::sync::Mutex::new(child_rng),
                // Carry parent's effective default in as the child's
                // starting point — `execute_flow` will refine it from
                // the child's own `[flow.options].record` if set.
                inherited_record_default: ctx.inherited_record_default,
            };

            let child_result = Box::pin(execute_flow(
                &child_flow,
                driver,
                &mut child_vars,
                None,
                default_timeout_ms,
                &mut child_ctx,
                barrier,
            ))
            .await?;

            let save_to = config.as_ref().map_or(&block.save_to, |c| &c.save_to);
            crate::subflow::propagate_results(&child_vars, vars, save_to)?;

            // Roll subflow recordings up into the parent. The parent's
            // FlowReport surfaces every recording produced under this
            // device, regardless of which flow level produced it.
            recordings.extend(child_result.recordings.iter().cloned());

            if !child_result.success {
                return Ok(FlowResult {
                    success: false,
                    warnings,
                    failed_step: child_result.failed_step,
                    failed_block: block.name.clone(),
                    failed_action: child_result.failed_action,
                    failed_reason: child_result.failed_reason,
                    failed_code: child_result.failed_code,
                    barrier_aborted: child_result.barrier_aborted,
                    perf_snapshots: vec![],
                    recordings: recordings.clone(),
                    a11y_audits: vec![],
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
        let iteration = block_iterations.entry(current_idx).or_insert(0);
        let block_label = block
            .name
            .clone()
            .unwrap_or_else(|| format!("block_{current_idx}"));
        let block_iter_for_recording = *iteration;
        // Expose the 0-based per-block iteration as the reserved `_loop`
        // variable: 0 on first entry, 1 on the second, etc. This is what
        // bounds a branch loop — `[[block.branch]] if_var = "_loop", gte = N`
        // breaks out once the block has re-entered enough times. Re-set on
        // every block entry (each block reflects its own count, no stale
        // value leaks across blocks). Generator scope mirrors `_perf` below.
        vars.set_in_scope(
            ScopeLevel::Generator,
            "_loop",
            VarValue::String(iteration.to_string()),
        );
        ctx.emit(golem_events::EventKind::BlockStarted {
            block_name: block_label.clone(),
            block_index: current_idx,
            iteration: *iteration,
        });
        *iteration += 1;

        // Effective per-block recording. CLI flags are overrides on
        // the final effective value, not just defaults:
        //   --no-record beats everything below (force off).
        //   --record beats explicit `[[block]] record = false` (force on).
        //   else `[[block]] record` wins if set.
        //   else fall through to `ctx.inherited_record_default`
        //   (resolved at flow entry from this flow's options, the
        //   parent flow's default, and project [options].record).
        let record_block = match ctx.capture_config.cli_force_record {
            Some(force) => force,
            None => block.record.unwrap_or(ctx.inherited_record_default),
        };
        if record_block {
            if let Err(e) = crate::capture::start_recording(
                driver,
                ctx.capture_config,
                &block_label,
                block_iter_for_recording,
            )
            .await
            {
                warnings.push(format!(
                    "start_recording failed for block '{}' iter {}: {}",
                    block_label, block_iter_for_recording, e
                ));
            } else if ctx.capture_config.trace {
                // Open a trace tracker for this block iteration. The
                // first boundary (offset 0) is "just after recording
                // started, before step 1 of this block." When the
                // parent flow has run previous steps, this also lets
                // viewers locate post-step-N inside the right video.
                block_trace = Some(BlockTrace {
                    block_label: block_label.clone(),
                    iteration: block_iter_for_recording,
                    recording_started_at_ms: now_unix_ms(),
                    recording_started_at: std::time::Instant::now(),
                    boundaries: Vec::new(),
                });
            }
        }
        for (step_idx, step) in block.steps.iter().enumerate() {
            ctx.step_index = step_idx;
            ctx.block_iteration = iteration.saturating_sub(1);

            step_count += 1;
            ctx.global_step_index = step_count;
            if step_count > max_steps {
                return Err(golem_events::coded(
                    golem_events::FailureCode::FlowMaxSteps,
                    anyhow::anyhow!("max_steps ({max_steps}) exceeded at step {step_count}"),
                ));
            }
            if start_time.elapsed() > max_runtime {
                return Err(golem_events::coded(
                    golem_events::FailureCode::FlowMaxRuntime,
                    anyhow::anyhow!("max_runtime exceeded after {:?}", start_time.elapsed()),
                ));
            }
            // Check failure barrier before executing the step
            if let Some(b) = barrier {
                if b.should_stop(step_count) {
                    let recording_path = if record_block {
                        match crate::capture::stop_recording(
                            driver,
                            ctx.capture_config,
                            &block_label,
                            block_iter_for_recording,
                        )
                        .await
                        {
                            Ok(p) => {
                                let path_str = p.display().to_string();
                                recordings.push(golem_report::RecordingEntry {
                                    block: block_label.clone(),
                                    iteration: block_iter_for_recording,
                                    path: path_str.clone(),
                                });
                                Some(path_str)
                            }
                            Err(_) => None,
                        }
                    } else {
                        None
                    };
                    flush_trace_sidecar(&mut block_trace, ctx.capture_config, &mut warnings);
                    ctx.emit(golem_events::EventKind::BlockFinished {
                        block_name: block_label.clone(),
                        block_index: current_idx,
                        iteration: block_iter_for_recording,
                        recording_path,
                    });
                    return Ok(FlowResult {
                        success: false,
                        warnings,
                        failed_step: Some(step_idx),
                        failed_block: block.name.clone(),
                        failed_action: Some(step.action.clone()),
                        failed_reason: Some("aborted: another device failed".to_string()),
                        failed_code: None,
                        barrier_aborted: true,
                        perf_snapshots: vec![],
                        recordings: recordings.clone(),
                        a11y_audits: vec![],
                    });
                }
            }

            // Resolve `${…}` variable references and inline `${fake:…}`
            // generators in this step's fields (selectors, input, params)
            // before it runs. Errors — undefined var, object-in-string,
            // malformed generator — fail the flow with a ParseVariable code.
            let step_owned = {
                let rng = &ctx.rng;
                let generator = |def: &str| {
                    golem_vars::evaluate::generate_fake(
                        def,
                        &mut rng.lock().expect("flow rng mutex poisoned"),
                    )
                };
                let mut ictx = golem_vars::interpolation::InterpolationContext::new(&*vars);
                ictx.generator = Some(&generator);
                crate::interp::interpolate_step(step, &ictx)?
            };
            let step = &step_owned;

            let block_name_str = block.name.clone().unwrap_or_default();
            ctx.emit(golem_events::EventKind::StepStarted {
                global_step_index: step_count,
                block_name: block_name_str.clone(),
                step_index_in_block: step_idx,
                action: step.action.clone(),
                selector_label: step_target(step),
            });
            let step_start = Instant::now();
            crate::reset_step_tree_stats();
            let step_action_result = execute_step_with_policy(
                step,
                driver,
                vars,
                default_timeout_ms,
                ctx,
                &flow.flow.apps,
            )
            .await;

            // `--trace` post-step capture, OOB of the step timeout
            // budget. Boundary index N == "after step N". Errors are
            // collected as warnings; the flow continues so trace
            // failures never abort a real run.
            if ctx.capture_config.trace {
                let suffix = format!(
                    "after_{}_{}_{}",
                    crate::capture::sanitize_filename(&block_label),
                    block_iter_for_recording,
                    step_idx + 1,
                );
                match crate::capture::capture_trace_boundary(
                    driver,
                    ctx.capture_config,
                    step_count,
                    &suffix,
                    crate::capture::TraceMeta {
                        after_step: Some(step_count),
                        action: Some(&step.action),
                        wall_clock: &iso8601_now(),
                    },
                )
                .await
                {
                    Ok(_) => {
                        if let Some(ref mut bt) = block_trace {
                            bt.boundaries.push(crate::capture::TraceBoundary {
                                boundary: step_count,
                                after_step: Some(step_count),
                                offset_ms: bt.recording_started_at.elapsed().as_millis() as u64,
                            });
                        }
                    }
                    Err(e) => {
                        warnings.push(format!("trace boundary {step_count} capture failed: {e}"))
                    }
                }
            }

            match step_action_result {
                Ok(StepOutcome::Success) => {
                    ctx.emit(golem_events::EventKind::StepFinished {
                        global_step_index: step_count,
                        outcome: golem_events::StepOutcome::Success,
                        duration_ms: step_start.elapsed().as_millis() as u64,
                        retry_count: 0,
                        screenshot_path: None,
                        tree_stats: crate::take_step_tree_stats(),
                    });
                }
                Ok(StepOutcome::Warning { message, code }) => {
                    ctx.emit(golem_events::EventKind::StepFinished {
                        global_step_index: step_count,
                        outcome: golem_events::StepOutcome::Warning {
                            message: message.clone(),
                            code,
                        },
                        duration_ms: step_start.elapsed().as_millis() as u64,
                        retry_count: 0,
                        screenshot_path: None,
                        tree_stats: crate::take_step_tree_stats(),
                    });
                    warnings.push(message);
                }
                Ok(StepOutcome::Ignored) => {
                    ctx.emit(golem_events::EventKind::StepFinished {
                        global_step_index: step_count,
                        outcome: golem_events::StepOutcome::Ignored,
                        duration_ms: step_start.elapsed().as_millis() as u64,
                        retry_count: 0,
                        screenshot_path: None,
                        tree_stats: crate::take_step_tree_stats(),
                    });
                }
                Err(e) => {
                    let code = golem_events::extract_code(&e)
                        .unwrap_or(golem_events::FailureCode::Uncoded);
                    let message = golem_events::clean_msg(&e);
                    ctx.emit(golem_events::EventKind::StepFinished {
                        global_step_index: step_count,
                        outcome: golem_events::StepOutcome::Failed {
                            message: message.clone(),
                            code,
                        },
                        duration_ms: step_start.elapsed().as_millis() as u64,
                        retry_count: 0,
                        screenshot_path: None,
                        tree_stats: crate::take_step_tree_stats(),
                    });
                    // Report failure to barrier so other devices stop at this point
                    if let Some(b) = barrier {
                        b.report_failure(step_count);
                    }
                    let recording_path = if record_block {
                        match crate::capture::stop_recording(
                            driver,
                            ctx.capture_config,
                            &block_label,
                            block_iter_for_recording,
                        )
                        .await
                        {
                            Ok(p) => {
                                let path_str = p.display().to_string();
                                recordings.push(golem_report::RecordingEntry {
                                    block: block_label.clone(),
                                    iteration: block_iter_for_recording,
                                    path: path_str.clone(),
                                });
                                Some(path_str)
                            }
                            Err(_) => None,
                        }
                    } else {
                        None
                    };
                    flush_trace_sidecar(&mut block_trace, ctx.capture_config, &mut warnings);
                    // Emit BlockFinished even on failure so the
                    // recording-path event reaches stream/accumulator
                    // before the FlowFinished sweep.
                    ctx.emit(golem_events::EventKind::BlockFinished {
                        block_name: block_label.clone(),
                        block_index: current_idx,
                        iteration: block_iter_for_recording,
                        recording_path,
                    });
                    return Ok(FlowResult {
                        success: false,
                        warnings,
                        failed_step: Some(step_idx),
                        failed_block: block.name.clone(),
                        failed_action: Some(step.action.clone()),
                        failed_reason: Some(message),
                        failed_code: Some(code),
                        barrier_aborted: false,
                        perf_snapshots: vec![],
                        recordings: recordings.clone(),
                        a11y_audits: vec![],
                    });
                }
            }
        }

        let recording_path = if record_block {
            match crate::capture::stop_recording(
                driver,
                ctx.capture_config,
                &block_label,
                block_iter_for_recording,
            )
            .await
            {
                Ok(p) => {
                    let path_str = p.display().to_string();
                    recordings.push(golem_report::RecordingEntry {
                        block: block_label.clone(),
                        iteration: block_iter_for_recording,
                        path: path_str.clone(),
                    });
                    Some(path_str)
                }
                Err(e) => {
                    warnings.push(format!(
                        "stop_recording failed for block '{}' iter {}: {}",
                        block_label, block_iter_for_recording, e
                    ));
                    None
                }
            }
        } else {
            None
        };

        flush_trace_sidecar(&mut block_trace, ctx.capture_config, &mut warnings);
        ctx.emit(golem_events::EventKind::BlockFinished {
            block_name: block_label.clone(),
            block_index: current_idx,
            iteration: block_iter_for_recording,
            recording_path,
        });

        // Capture perf snapshot after block completes
        if perf_enabled {
            if let Some(collector) = ctx.perf_collector {
                let raw = collector.capture().await;
                let launch_ms = ctx.take_launch_ms();
                let device_name = ctx.device.map_or("unknown", |d| d.name.as_str());
                let active_app = collector.active_bundle_id();

                let label =
                    build_snapshot_label(block, current_idx, active_app.as_deref(), device_name, 0);
                let timestamp = chrono_now();

                let snapshot = build_snapshot(&raw, label, launch_ms, timestamp);
                write_perf_var(vars, &snapshot);
                let threshold_result = evaluate_thresholds(&snapshot, flow.flow.options.as_ref());
                perf_snapshots.push(snapshot);

                match threshold_result {
                    ThresholdResult::Ok => {}
                    ThresholdResult::Warn(msg) => warnings.push(msg),
                    ThresholdResult::Error(msg) => {
                        let reason = msg.clone();
                        warnings.push(msg);
                        return Ok(FlowResult {
                            success: false,
                            warnings,
                            failed_step: None,
                            failed_block: block.name.clone(),
                            failed_action: None,
                            failed_reason: Some(reason),
                            failed_code: None,
                            barrier_aborted: false,
                            perf_snapshots,
                            recordings,
                            a11y_audits,
                        });
                    }
                }
            }
        }

        // Accessibility audit after block completes. Reuses the tree the
        // post-step settle cached for THIS block's last step (no extra
        // hierarchy fetch); falls back to a fresh fetch when the last step
        // didn't settle (e.g. a read/http step). Judges only the visible
        // tree (`audit_hierarchy` gates on the viewport predicate).
        if ctx.a11y_level.is_enabled() {
            let density = ctx.device.and_then(a11y_density).unwrap_or(1.0);
            let config = crate::accessibility::A11yConfig::new(ctx.a11y_level, density);
            // When we need a screenshot (strict), capture it and a FRESH tree
            // back-to-back so element bounds align with the pixels — the
            // cached settled tree predates the shot and would drift, throwing
            // off every crop. Non-screenshot levels reuse the settled tree.
            let (tree, shot) = if ctx.a11y_level.forces_screenshot() {
                let shot = crate::resolution::screenshot_bounded(driver).await.ok();
                let tree = driver.get_hierarchy().await.ok().map(|(t, _meta)| t);
                (tree, shot)
            } else {
                let tree = match ctx.take_settled_tree_at(ctx.global_step_index) {
                    Some(t) => Some(t),
                    None => driver.get_hierarchy().await.ok().map(|(t, _meta)| t),
                };
                (tree, None)
            };
            if let Some(tree) = tree {
                let viewport = golem_element::Viewport::from_root(&tree);
                let mut issues = crate::accessibility::audit_hierarchy(&tree, &viewport, &config);

                // Screenshot-based check (contrast), against the coherent
                // tree+shot pair captured above.
                let mut screenshot_path: Option<String> = None;
                if let Some(shot) = shot {
                    issues.extend(crate::accessibility::check_contrast(
                        &shot.data, &tree, &viewport, &config,
                    ));
                    // Drop low-confidence findings before annotating so the
                    // drawn rectangles match the reported issues.
                    if let Some(min) = flow.flow.options.as_ref().and_then(|o| o.a11y_min_confidence)
                    {
                        issues.retain(|i| i.confidence >= min);
                    }
                    if !issues.is_empty() {
                        if let Ok(png) = crate::accessibility::annotate_screenshot(
                            &shot.data,
                            &issues,
                            viewport.width,
                        ) {
                            let path = crate::capture::build_a11y_screenshot_path(
                                ctx.capture_config,
                                block,
                                current_idx,
                                ctx.global_step_index,
                                block_iter_for_recording,
                            );
                            if let Some(parent) = path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            if std::fs::write(&path, &png).is_ok() {
                                screenshot_path = Some(path.to_string_lossy().into_owned());
                            }
                        }
                    }
                }

                let device_name = ctx.device.map_or("unknown", |d| d.name.as_str());
                let label = build_snapshot_label(
                    block,
                    current_idx,
                    None,
                    device_name,
                    block_iter_for_recording,
                );
                let audit = golem_report::A11yAudit {
                    label,
                    issues,
                    screenshot_path,
                };
                if !audit.issues.is_empty() {
                    warnings.push(format!(
                        "a11y[{}]: {} error(s), {} warning(s)",
                        audit.label,
                        audit.error_count(),
                        audit.warning_count()
                    ));
                }
                // Surface findings on the live event stream (the renderer
                // decides verbose detail vs a one-line summary).
                ctx.emit(golem_events::EventKind::A11yAudit {
                    audit: audit.clone(),
                });
                a11y_audits.push(audit);

                // Threshold gate: fail the flow when cumulative errors/warnings
                // exceed the configured maxima.
                if let Some(reason) = a11y_threshold_breach(&a11y_audits, flow.flow.options.as_ref())
                {
                    warnings.push(reason.clone());
                    return Ok(FlowResult {
                        success: false,
                        warnings,
                        failed_step: None,
                        failed_block: block.name.clone(),
                        failed_action: None,
                        failed_reason: Some(reason),
                        failed_code: None,
                        barrier_aborted: false,
                        perf_snapshots,
                        recordings,
                        a11y_audits,
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
        failed_action: None,
        failed_reason: None,
        failed_code: None,
        barrier_aborted: false,
        perf_snapshots,
        recordings,
        a11y_audits,
    })
}

/// Parse a human-readable duration string into a [`Duration`].
///
/// Supported suffixes: `ms` (milliseconds), `s` (seconds), `m` (minutes), `h` (hours).
/// Returns `None` if the format is not recognised.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix("ms") {
        return n.trim().parse::<u64>().ok().map(Duration::from_millis);
    }
    if let Some(n) = s.strip_suffix('s') {
        return n.trim().parse::<u64>().ok().map(Duration::from_secs);
    }
    if let Some(n) = s.strip_suffix('m') {
        return n
            .trim()
            .parse::<u64>()
            .ok()
            .map(|v| Duration::from_secs(v * 60));
    }
    if let Some(n) = s.strip_suffix('h') {
        return n
            .trim()
            .parse::<u64>()
            .ok()
            .map(|v| Duration::from_secs(v * 3600));
    }
    None
}

// ── A11y helpers ─────────────────────────────────────────────────────

/// Px-per-dp factor for normalising `Element.bounds` to dp. Android bounds
/// are device px (factor = `screen_scale`); iOS bounds are already points
/// (factor 1.0).
fn a11y_density(device: &golem_devices::DeviceInfo) -> Option<f64> {
    match device.platform {
        golem_devices::Platform::Android => device.screen_scale,
        _ => Some(1.0),
    }
}

/// A failure reason when cumulative a11y errors/warnings across all audits
/// so far exceed the flow's configured maxima; `None` when within limits or
/// no maxima are set.
fn a11y_threshold_breach(
    audits: &[golem_report::A11yAudit],
    options: Option<&golem_parser::FlowOptions>,
) -> Option<String> {
    let opts = options?;
    let errors: usize = audits.iter().map(golem_report::A11yAudit::error_count).sum();
    let warnings: usize = audits
        .iter()
        .map(golem_report::A11yAudit::warning_count)
        .sum();
    if let Some(max) = opts.a11y_max_errors {
        if errors > max {
            return Some(format!("a11y errors {errors} exceed max {max}"));
        }
    }
    if let Some(max) = opts.a11y_max_warnings {
        if warnings > max {
            return Some(format!("a11y warnings {warnings} exceed max {max}"));
        }
    }
    None
}

// ── Perf helpers ─────────────────────────────────────────────────────

fn build_snapshot_label(
    block: &Block,
    block_idx: usize,
    active_app: Option<&str>,
    device_name: &str,
    iteration: u32,
) -> String {
    let block_part = match &block.name {
        Some(name) => name.clone(),
        None => {
            let hint = block
                .steps
                .first()
                .map(|s| {
                    let target = s
                        .on_text
                        .as_deref()
                        .or(s.on.as_ref().and_then(|g| g.text.as_deref()))
                        .unwrap_or("_");
                    format!("{}:{}", s.action, target)
                })
                .unwrap_or_else(|| "empty".to_string());
            format!("block_{block_idx}({hint})")
        }
    };
    match active_app {
        Some(app) => format!("{block_part}:{app}:{device_name}:{iteration}"),
        None => format!("{block_part}:{device_name}:{iteration}"),
    }
}

fn build_snapshot(
    raw: &RawPerfData,
    label: String,
    launch_ms: Option<u64>,
    timestamp: String,
) -> PerfSnapshot {
    PerfSnapshot {
        label,
        memory_mb: raw.memory_mb,
        cpu_percent: raw.cpu_percent,
        threads: raw.threads,
        file_descriptors: raw.file_descriptors,
        disk_mb: raw.disk_mb,
        net_rx_kb: raw.net_rx_kb,
        net_tx_kb: raw.net_tx_kb,
        launch_ms,
        timestamp,
    }
}

fn write_perf_var(vars: &mut VariableStore, snapshot: &PerfSnapshot) {
    let mut map = HashMap::new();

    fn opt_f64(val: Option<f64>) -> VarValue {
        VarValue::String(val.map_or(String::new(), |v| format!("{v:.1}")))
    }
    fn opt_u32(val: Option<u32>) -> VarValue {
        VarValue::String(val.map_or(String::new(), |v| v.to_string()))
    }
    fn opt_u64(val: Option<u64>) -> VarValue {
        VarValue::String(val.map_or(String::new(), |v| v.to_string()))
    }

    map.insert("memory_mb".into(), opt_f64(snapshot.memory_mb));
    map.insert("cpu_percent".into(), opt_f64(snapshot.cpu_percent));
    map.insert("threads".into(), opt_u32(snapshot.threads));
    map.insert(
        "file_descriptors".into(),
        opt_u32(snapshot.file_descriptors),
    );
    map.insert("disk_mb".into(), opt_f64(snapshot.disk_mb));
    map.insert("net_rx_kb".into(), opt_f64(snapshot.net_rx_kb));
    map.insert("net_tx_kb".into(), opt_f64(snapshot.net_tx_kb));
    map.insert("launch_ms".into(), opt_u64(snapshot.launch_ms));
    map.insert("label".into(), VarValue::String(snapshot.label.clone()));
    map.insert(
        "timestamp".into(),
        VarValue::String(snapshot.timestamp.clone()),
    );

    vars.set_in_scope(ScopeLevel::Generator, "_perf", VarValue::Object(map));
}

enum ThresholdResult {
    Ok,
    Warn(String),
    Error(String),
}

struct BlockTrace {
    block_label: String,
    iteration: u32,
    recording_started_at_ms: u64,
    recording_started_at: std::time::Instant,
    boundaries: Vec<crate::capture::TraceBoundary>,
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn iso8601_now() -> String {
    let dt: chrono::DateTime<chrono::Utc> = std::time::SystemTime::now().into();
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Take the active `BlockTrace`, write its sidecar JSON next to the
/// block recording, and reset the slot to `None`. No-op when no trace
/// is active. Failures collected as warnings — sidecar write must not
/// fail a flow.
fn flush_trace_sidecar(
    slot: &mut Option<BlockTrace>,
    capture_config: &crate::capture::CaptureConfig,
    warnings: &mut Vec<String>,
) {
    let Some(bt) = slot.take() else { return };
    let sidecar = crate::capture::TraceSidecar {
        flow: capture_config.flow_name.clone(),
        device: capture_config.device_name.clone(),
        block: bt.block_label.clone(),
        iteration: bt.iteration,
        golem_version: env!("CARGO_PKG_VERSION").to_string(),
        recording_started_at_ms: bt.recording_started_at_ms,
        boundaries: bt.boundaries,
    };
    if let Err(e) =
        crate::capture::write_trace_sidecar(capture_config, &bt.block_label, bt.iteration, &sidecar)
    {
        warnings.push(format!(
            "trace sidecar write failed for block '{}' iter {}: {}",
            bt.block_label, bt.iteration, e
        ));
    }
}

fn evaluate_thresholds(snapshot: &PerfSnapshot, options: Option<&FlowOptions>) -> ThresholdResult {
    let Some(opts) = options else {
        return ThresholdResult::Ok;
    };

    // Check error thresholds first (more severe)
    if let (Some(val), Some(limit)) = (snapshot.memory_mb, opts.perf_memory_error_mb) {
        if val > limit {
            return ThresholdResult::Error(format!(
                "perf: memory {val:.1} MB exceeds error threshold {limit:.1} MB"
            ));
        }
    }
    if let (Some(val), Some(limit)) = (snapshot.cpu_percent, opts.perf_cpu_error_percent) {
        if val > limit {
            return ThresholdResult::Error(format!(
                "perf: CPU {val:.1}% exceeds error threshold {limit:.1}%"
            ));
        }
    }
    if let (Some(val), Some(limit)) = (snapshot.threads, opts.perf_threads_error) {
        if val > limit {
            return ThresholdResult::Error(format!(
                "perf: {val} threads exceeds error threshold {limit}"
            ));
        }
    }
    if let (Some(val), Some(limit)) = (snapshot.file_descriptors, opts.perf_fd_error) {
        if val > limit {
            return ThresholdResult::Error(format!(
                "perf: {val} FDs exceeds error threshold {limit}"
            ));
        }
    }

    // Check warning thresholds
    if let (Some(val), Some(limit)) = (snapshot.memory_mb, opts.perf_memory_warn_mb) {
        if val > limit {
            return ThresholdResult::Warn(format!(
                "perf: memory {val:.1} MB exceeds warning threshold {limit:.1} MB"
            ));
        }
    }
    if let (Some(val), Some(limit)) = (snapshot.cpu_percent, opts.perf_cpu_warn_percent) {
        if val > limit {
            return ThresholdResult::Warn(format!(
                "perf: CPU {val:.1}% exceeds warning threshold {limit:.1}%"
            ));
        }
    }
    if let (Some(val), Some(limit)) = (snapshot.threads, opts.perf_threads_warn) {
        if val > limit {
            return ThresholdResult::Warn(format!(
                "perf: {val} threads exceeds warning threshold {limit}"
            ));
        }
    }
    if let (Some(val), Some(limit)) = (snapshot.file_descriptors, opts.perf_fd_warn) {
        if val > limit {
            return ThresholdResult::Warn(format!(
                "perf: {val} FDs exceeds warning threshold {limit}"
            ));
        }
    }

    ThresholdResult::Ok
}

fn chrono_now() -> String {
    // ISO 8601 with timezone
    let now = std::time::SystemTime::now();
    let since_epoch = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since_epoch.as_secs();
    // Simple UTC timestamp without external dependency
    format!("{secs}")
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
    barrier: Option<&FailureBarrier>,
) -> Result<FlowResult> {
    let runs = crate::data_driven::get_runs(&flow.data);
    let mut last_result = None;
    for run in &runs {
        if !run.vars.is_empty() {
            crate::data_driven::apply_data_vars(vars, &run.vars);
        }
        let result = execute_flow(
            flow,
            driver,
            vars,
            start_block,
            default_timeout_ms,
            ctx,
            barrier,
        )
        .await?;
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
        failed_action: None,
        failed_reason: None,
        failed_code: None,
        barrier_aborted: false,
        perf_snapshots: vec![],
        recordings: Vec::new(),
    a11y_audits: Vec::new(),
    }))
}

/// Seed a `[*.vars]` map into `target` at `scope`, evaluating `fake:`
/// generators with the flow RNG (`${var}` cross-references resolve against
/// already-evaluated vars). Plain values pass through as strings.
///
/// `flow.vars` / `block.vars` are `HashMap`s (unordered), so the pairs are
/// **sorted by key** before evaluation — without a stable order, two
/// `fake:` vars could draw each other's RNG values and `--seed` replay
/// would not reproduce. (Cross-references therefore only resolve when the
/// referenced var sorts earlier; same best-effort contract as fixtures.)
fn seed_vars_with_generators(
    raw: &HashMap<String, String>,
    target: &mut VariableStore,
    scope: ScopeLevel,
    rng: &std::sync::Mutex<golem_vars::seed::FakeRng>,
) -> Result<()> {
    if raw.is_empty() {
        return Ok(());
    }
    let mut pairs: Vec<(String, String)> =
        raw.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let evaluated = {
        let mut guard = rng.lock().expect("flow rng mutex poisoned");
        golem_vars::evaluate::evaluate_generators(&pairs, &mut guard)?
    };
    for (key, value) in evaluated {
        target.set_in_scope(scope, key, value);
    }
    Ok(())
}

/// Find the index of a block by name. Returns an error if not found.
fn find_block_index(blocks: &[Block], name: &str) -> Result<usize> {
    for (i, block) in blocks.iter().enumerate() {
        if block.name.as_deref() == Some(name) {
            return Ok(i);
        }
    }
    Err(golem_events::coded(
        golem_events::FailureCode::ParseMissingReference,
        anyhow::anyhow!("Block not found: {name}"),
    ))
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

    // ── a11y executor helpers ────────────────────────────────────────

    fn audit_with(errors: usize, warnings: usize) -> golem_report::A11yAudit {
        let mut issues = Vec::new();
        for _ in 0..errors {
            issues.push(golem_report::A11yIssue {
                check_id: "missing_label".into(),
                severity: golem_events::Severity::Error,
                message: "m".into(),
                element_type: "Button".into(),
                element_label: None,
                element_bounds: None,
                confidence: 1.0,
            });
        }
        for _ in 0..warnings {
            issues.push(golem_report::A11yIssue {
                check_id: "nested_clickable".into(),
                severity: golem_events::Severity::Warning,
                message: "m".into(),
                element_type: "Button".into(),
                element_label: None,
                element_bounds: None,
                confidence: 1.0,
            });
        }
        golem_report::A11yAudit {
            label: "b:d:0".into(),
            issues,
            screenshot_path: None,
        }
    }

    #[test]
    fn a11y_threshold_none_when_no_maxima() {
        let audits = vec![audit_with(3, 3)];
        assert!(a11y_threshold_breach(&audits, None).is_none());
        let opts = FlowOptions::default();
        assert!(
            a11y_threshold_breach(&audits, Some(&opts)).is_none(),
            "no maxima set → never breaches"
        );
    }

    #[test]
    fn a11y_threshold_errors_breach_is_cumulative() {
        let audits = vec![audit_with(1, 0), audit_with(1, 0)];
        let opts = FlowOptions {
            a11y_max_errors: Some(1),
            ..FlowOptions::default()
        };
        // 2 cumulative errors > max 1 → breach.
        assert!(a11y_threshold_breach(&audits, Some(&opts)).is_some());
    }

    #[test]
    fn a11y_threshold_within_limit_ok() {
        let audits = vec![audit_with(0, 5)];
        let opts = FlowOptions {
            a11y_max_warnings: Some(10),
            ..FlowOptions::default()
        };
        assert!(a11y_threshold_breach(&audits, Some(&opts)).is_none());
    }

    fn device_info(platform: golem_devices::Platform, scale: Option<f64>) -> golem_devices::DeviceInfo {
        golem_devices::DeviceInfo {
            name: "dev".into(),
            udid: "x".into(),
            platform,
            device_type: golem_devices::DeviceType::Phone,
            os_major: 17,
            os_version: "17".into(),
            state: golem_devices::DeviceState::Booted,
            physical: false,
            playstore: false,
            screen_width: Some(1080),
            screen_height: Some(1920),
            screen_scale: scale,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        }
    }

    #[test]
    fn a11y_density_android_uses_screen_scale_ios_is_one() {
        let android = device_info(golem_devices::Platform::Android, Some(3.0));
        assert_eq!(a11y_density(&android), Some(3.0));
        // iOS bounds are already points → factor 1.0 regardless of backing scale.
        let ios = device_info(golem_devices::Platform::Ios, Some(3.0));
        assert_eq!(a11y_density(&ios), Some(1.0));
    }

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
            hit_points: vec![],
            drawing_order: None,
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
            record: None,
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
            ..Default::default()
        }
    }

    /// Build a step that will fail: "tap" with text that won't be found.
    /// Uses a tight per-step timeout so tests exercising the failure
    /// path don't wait the full 10s default poll.
    fn make_failing_step() -> Step {
        Step {
            action: "tap".to_string(),
            on_text: Some("NONEXISTENT_ELEMENT_xyz_12345".to_string()),
            timeout: Some(50),
            ..Default::default()
        }
    }

    fn make_warn_step() -> Step {
        let mut step = make_failing_step();
        step.if_fail = Some("warn".to_string());
        step
    }

    fn make_ignore_step() -> Step {
        let mut step = make_failing_step();
        step.if_fail = Some("ignore".to_string());
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

    fn cond_if_var_gte(var: &str, threshold: i64, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_var: Some(var.to_string()),
            equals: None,
            matches: None,
            gte: Some(threshold),
            goto: goto.to_string(),
        }
    }

    fn cond_if_var_equals(var: &str, value: &str, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_var: Some(var.to_string()),
            equals: Some(value.to_string()),
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
            vec![
                make_success_step(),
                make_success_step(),
                make_success_step(),
            ],
        )]);

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

        let result = execute_flow(
            &flow,
            &driver,
            &mut vars,
            Some("second"),
            DEFAULT_TIMEOUT,
            &mut ctx,
            None,
        )
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

        let result = execute_flow(
            &flow,
            &driver,
            &mut vars,
            Some("nonexistent"),
            DEFAULT_TIMEOUT,
            &mut ctx,
            None,
        )
        .await;

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

        let result = execute_flow(
            &flow,
            &driver,
            &mut vars,
            None,
            DEFAULT_TIMEOUT,
            &mut ctx,
            None,
        )
        .await;

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
                vec![
                    make_success_step(),
                    make_failing_step(),
                    make_success_step(),
                ],
            ),
            make_block(Some("second"), vec![make_success_step()]),
        ]);

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
    // 9. Step with if_fail="warn" collects warning and continues
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

        assert!(result.success, "flow SHALL succeed despite warning");
        assert_eq!(result.warnings.len(), 1, "SHALL have collected one warning");
        assert!(
            !result.warnings[0].is_empty(),
            "warning message should not be empty"
        );
    }

    // ---------------------------------------------------------------
    // 10. Step with if_fail="ignore" continues silently
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
    // 12b. Bounded branch loop terminates on the `_loop` counter
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn branch_loop_terminates_on_loop_counter() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));

        // loop_body re-enters itself until `_loop` (0-based per-block
        // iteration) reaches 2 — i.e. it runs at _loop = 0, 1, 2 (three
        // times) then jumps to `done`. Without the `_loop` injection the
        // gte never matches and the loop runs until max_steps.
        let flow = make_flow(vec![
            make_block_with_branch(
                Some("loop_body"),
                vec![make_success_step()],
                vec![
                    cond_if_var_gte("_loop", 2, "done"),
                    cond_default("loop_body"),
                ],
            ),
            make_block(Some("done"), vec![make_success_step()]),
        ]);

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

        assert!(
            result.success,
            "bounded branch loop SHALL terminate cleanly"
        );
        let calls = driver.get_calls();
        let screenshots = calls.iter().filter(|c| c.0 == "screenshot").count();
        assert_eq!(
            screenshots, 4,
            "loop body SHALL run 3x (_loop 0,1,2) then `done` once"
        );
    }

    // ---------------------------------------------------------------
    // 12c. `_loop` is 0-based on first entry (injected before branch)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn loop_counter_is_zero_on_first_entry() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let mut ctx = test_ctx(Path::new("."));

        // First entry sees `_loop == "0"`, so it jumps to `done`
        // immediately — `body` runs once, `done` once, `never` not at all.
        let flow = make_flow(vec![
            make_block_with_branch(
                Some("body"),
                vec![make_success_step()],
                vec![cond_if_var_equals("_loop", "0", "done")],
            ),
            make_block(Some("never"), vec![make_success_step()]),
            make_block(Some("done"), vec![make_success_step()]),
        ]);

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

        assert!(result.success);
        let calls = driver.get_calls();
        let screenshots = calls.iter().filter(|c| c.0 == "screenshot").count();
        assert_eq!(
            screenshots, 2,
            "`_loop`==0 on first entry SHALL jump straight to done (body + done = 2)"
        );
        // And the var is observable in the store after the run.
        assert_eq!(
            vars.get("_loop").and_then(|v| v.as_str()),
            Some("0"),
            "`_loop` SHALL be exposed in the variable store"
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
    // 16b. seed_vars_with_generators — fake: eval + seed determinism
    // ---------------------------------------------------------------
    fn seeded_rng(seed: u64) -> std::sync::Mutex<golem_vars::seed::FakeRng> {
        std::sync::Mutex::new(golem_vars::seed::FakeRng::from_seed(seed))
    }

    #[test]
    fn seed_vars_evaluates_fake_and_passes_plain_through() {
        let mut raw = HashMap::new();
        raw.insert("email".to_string(), "${fake:email}".to_string());
        raw.insert("literal".to_string(), "hello".to_string());

        let mut store = VariableStore::new();
        seed_vars_with_generators(&raw, &mut store, ScopeLevel::Flow, &seeded_rng(42))
            .expect("seeding SHALL succeed");

        let email = store
            .get("email")
            .and_then(|v| v.as_str())
            .expect("email set");
        assert!(
            email.contains('@'),
            "fake:email SHALL generate an address, got {email:?}"
        );
        assert_ne!(
            email, "${fake:email}",
            "fake: SHALL NOT be stored as the literal"
        );
        assert_eq!(
            store.get("literal").and_then(|v| v.as_str()),
            Some("hello"),
            "plain values SHALL pass through unchanged"
        );
    }

    #[test]
    fn seed_vars_is_seed_deterministic_regardless_of_map_order() {
        // Two fake vars inserted in OPPOSITE orders. Sorting by key makes
        // evaluation order — and thus the RNG draw each var gets — identical,
        // so the same seed reproduces the same values (replay determinism).
        let mut raw1 = HashMap::new();
        raw1.insert("a_addr".to_string(), "${fake:email}".to_string());
        raw1.insert("z_addr".to_string(), "${fake:email}".to_string());
        let mut raw2 = HashMap::new();
        raw2.insert("z_addr".to_string(), "${fake:email}".to_string());
        raw2.insert("a_addr".to_string(), "${fake:email}".to_string());

        let mut s1 = VariableStore::new();
        seed_vars_with_generators(&raw1, &mut s1, ScopeLevel::Flow, &seeded_rng(7)).expect("s1");
        let mut s2 = VariableStore::new();
        seed_vars_with_generators(&raw2, &mut s2, ScopeLevel::Flow, &seeded_rng(7)).expect("s2");

        assert_eq!(
            s1.get("a_addr").and_then(|v| v.as_str()),
            s2.get("a_addr").and_then(|v| v.as_str()),
            "same seed SHALL reproduce a_addr regardless of map insertion order"
        );
        assert_eq!(
            s1.get("z_addr").and_then(|v| v.as_str()),
            s2.get("z_addr").and_then(|v| v.as_str()),
            "same seed SHALL reproduce z_addr regardless of map insertion order"
        );
        assert_ne!(
            s1.get("a_addr").and_then(|v| v.as_str()),
            s1.get("z_addr").and_then(|v| v.as_str()),
            "two distinct fake vars SHALL get distinct draws"
        );
    }

    #[test]
    fn seed_vars_empty_map_is_noop() {
        let mut store = VariableStore::new();
        seed_vars_with_generators(
            &HashMap::new(),
            &mut store,
            ScopeLevel::Flow,
            &seeded_rng(1),
        )
        .expect("empty SHALL succeed");
        assert!(store.get("anything").is_none());
    }

    #[test]
    fn seed_vars_bad_generator_errors() {
        let mut raw = HashMap::new();
        raw.insert("x".to_string(), "${fake:}".to_string()); // empty generator name
        let mut store = VariableStore::new();
        let result = seed_vars_with_generators(&raw, &mut store, ScopeLevel::Flow, &seeded_rng(1));
        assert!(
            result.is_err(),
            "a malformed fake: def SHALL surface an error"
        );
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

        assert!(result.success);
        let calls = driver.get_calls();
        let screenshot_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        // start + end = 2 (skipped is bypassed), then end falls through past index 2
        assert_eq!(
            screenshot_calls.len(),
            2,
            "default branch SHALL jump to end"
        );
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
                vec![
                    make_success_step(),
                    make_success_step(),
                    make_failing_step(),
                ],
            ),
        ]);

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
        .expect("execute_flow should return FlowResult");

        assert!(!result.success);
        assert_eq!(
            result.failed_block, None,
            "unnamed block has no name to report"
        );
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

        let result = execute_flow(
            &flow,
            &driver,
            &mut vars,
            None,
            DEFAULT_TIMEOUT,
            &mut ctx,
            None,
        )
        .await;

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

        let result = execute_flow(
            &flow,
            &driver,
            &mut vars,
            None,
            DEFAULT_TIMEOUT,
            &mut ctx,
            None,
        )
        .await;
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
                vec![
                    make_success_step(),
                    make_success_step(),
                    make_success_step(),
                ],
            )],
            options,
        );

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
        .expect("execute_flow SHALL succeed when step count equals max_steps");
        assert!(
            result.success,
            "flow SHALL succeed at exact max_steps limit"
        );
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
        .expect("execute_flow SHALL succeed with sub-flow");
        assert!(
            result.success,
            "flow SHALL succeed when child flow succeeds"
        );

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
on_text = "NONEXISTENT_ELEMENT_xyz_12345"
# Tight test-only timeout — without it, the tap polls for the full 10s
# default before declaring the element missing.
timeout = 50
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
        .expect("execute_flow SHALL return FlowResult, not Err");
        assert!(!result.success, "flow SHALL fail when sub-flow fails");
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

        let result = execute_flow(
            &flow,
            &driver,
            &mut vars,
            None,
            DEFAULT_TIMEOUT,
            &mut ctx,
            None,
        )
        .await;
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

        let flow = make_flow(vec![make_block(Some("only"), vec![make_success_step()])]);

        let result = execute_flow_with_data(
            &flow,
            &driver,
            &mut vars,
            None,
            DEFAULT_TIMEOUT,
            &mut ctx,
            None,
        )
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

        let result = execute_flow_with_data(
            &flow,
            &driver,
            &mut vars,
            None,
            DEFAULT_TIMEOUT,
            &mut ctx,
            None,
        )
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

        let result = execute_flow_with_data(
            &flow,
            &driver,
            &mut vars,
            None,
            DEFAULT_TIMEOUT,
            &mut ctx,
            None,
        )
        .await
        .expect("execute_flow_with_data SHALL return FlowResult");
        assert!(!result.success, "SHALL fail when any data row fails");
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
        flow.data = vec![HashMap::from([(
            "payment".to_string(),
            "credit_card".to_string(),
        )])];

        let result = execute_flow_with_data(
            &flow,
            &driver,
            &mut vars,
            None,
            DEFAULT_TIMEOUT,
            &mut ctx,
            None,
        )
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
            global_step_index: 0,
            block_iteration: 0,
            device: Some(&ios_device),
            perf_collector: None,
            last_launch_ms: std::sync::atomic::AtomicU64::new(0),
            emitter: None,
            a11y_level: crate::accessibility::A11yLevel::Off,
            step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),            last_settled_tree: std::sync::Mutex::new(None),
            rng: std::sync::Mutex::new(golem_vars::seed::FakeRng::from_optional_seed(None)),
            inherited_record_default: false,
        };

        let result = execute_flow(&flow, &driver, &mut vars, None, 10_000, &mut ctx, None)
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
            global_step_index: 0,
            block_iteration: 0,
            device: Some(&android_device),
            perf_collector: None,
            last_launch_ms: std::sync::atomic::AtomicU64::new(0),
            emitter: None,
            a11y_level: crate::accessibility::A11yLevel::Off,
            step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),            last_settled_tree: std::sync::Mutex::new(None),
            rng: std::sync::Mutex::new(golem_vars::seed::FakeRng::from_optional_seed(None)),
            inherited_record_default: false,
        };

        let result = execute_flow(&flow, &driver, &mut vars, None, 10_000, &mut ctx, None)
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
            global_step_index: 0,
            block_iteration: 0,
            device: Some(&ios_device),
            perf_collector: None,
            last_launch_ms: std::sync::atomic::AtomicU64::new(0),
            emitter: None,
            a11y_level: crate::accessibility::A11yLevel::Off,
            step_tree_stats: std::sync::Mutex::new(golem_events::TreeStats::default()),            last_settled_tree: std::sync::Mutex::new(None),
            rng: std::sync::Mutex::new(golem_vars::seed::FakeRng::from_optional_seed(None)),
            inherited_record_default: false,
        };

        let result = execute_flow(&flow, &driver, &mut vars, None, 10_000, &mut ctx, None)
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

    // ── perf: snapshot labeling ──────────────────────────────────────

    fn empty_block(name: Option<&str>) -> Block {
        Block {
            name: name.map(String::from),
            app: None,
            steps: vec![],
            next: None,
            branch: vec![],
            for_each: None,
            r#where: None,
            run_flow: None,
            max_iterations: None,
            vars: HashMap::new(),
            save_to: HashMap::new(),
            record: None,
        }
    }

    fn empty_step(action: &str) -> golem_parser::Step {
        golem_parser::Step {
            action: action.into(),
            ..Default::default()
        }
    }

    #[test]
    fn snapshot_label_named_block_no_app() {
        let block = empty_block(Some("login"));
        let label = build_snapshot_label(&block, 0, None, "iPhone_16", 0);
        assert_eq!(label, "login:iPhone_16:0");
    }

    #[test]
    fn snapshot_label_named_block_with_app() {
        let block = empty_block(Some("login"));
        let label = build_snapshot_label(&block, 0, Some("com.example.app"), "iPhone_16", 0);
        assert_eq!(label, "login:com.example.app:iPhone_16:0");
    }

    #[test]
    fn snapshot_label_unnamed_block() {
        let mut block = empty_block(None);
        let mut step = empty_step("tap");
        step.on_text = Some("Submit".into());
        block.steps.push(step);
        let label = build_snapshot_label(&block, 2, None, "Pixel_8", 1);
        assert_eq!(label, "block_2(tap:Submit):Pixel_8:1");
    }

    #[test]
    fn snapshot_label_unnamed_no_steps() {
        let block = empty_block(None);
        let label = build_snapshot_label(&block, 0, None, "device", 0);
        assert_eq!(label, "block_0(empty):device:0");
    }

    // ── perf: threshold evaluation ───────────────────────────────────

    #[test]
    fn threshold_warn_on_memory() {
        let snapshot = PerfSnapshot {
            label: "test:dev:0".into(),
            memory_mb: Some(250.0),
            cpu_percent: None,
            threads: None,
            file_descriptors: None,
            disk_mb: None,
            net_rx_kb: None,
            net_tx_kb: None,
            launch_ms: None,
            timestamp: "0".into(),
        };
        let opts = FlowOptions {
            perf_memory_warn_mb: Some(200.0),
            ..Default::default()
        };
        match evaluate_thresholds(&snapshot, Some(&opts)) {
            ThresholdResult::Warn(msg) => assert!(msg.contains("250.0"), "SHALL mention value"),
            other => panic!(
                "expected Warn, got {}",
                match other {
                    ThresholdResult::Ok => "Ok",
                    ThresholdResult::Error(_) => "Error",
                    _ => "?",
                }
            ),
        }
    }

    #[test]
    fn threshold_error_on_memory() {
        let snapshot = PerfSnapshot {
            label: "test:dev:0".into(),
            memory_mb: Some(600.0),
            cpu_percent: None,
            threads: None,
            file_descriptors: None,
            disk_mb: None,
            net_rx_kb: None,
            net_tx_kb: None,
            launch_ms: None,
            timestamp: "0".into(),
        };
        let opts = FlowOptions {
            perf_memory_warn_mb: Some(200.0),
            perf_memory_error_mb: Some(500.0),
            ..Default::default()
        };
        match evaluate_thresholds(&snapshot, Some(&opts)) {
            ThresholdResult::Error(msg) => assert!(msg.contains("error threshold")),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn threshold_ok_when_below() {
        let snapshot = PerfSnapshot {
            label: "test:dev:0".into(),
            memory_mb: Some(100.0),
            cpu_percent: Some(50.0),
            threads: Some(30),
            file_descriptors: Some(50),
            disk_mb: None,
            net_rx_kb: None,
            net_tx_kb: None,
            launch_ms: None,
            timestamp: "0".into(),
        };
        let opts = FlowOptions {
            perf_memory_warn_mb: Some(200.0),
            perf_cpu_warn_percent: Some(80.0),
            perf_threads_warn: Some(100),
            perf_fd_warn: Some(200),
            ..Default::default()
        };
        assert!(matches!(
            evaluate_thresholds(&snapshot, Some(&opts)),
            ThresholdResult::Ok
        ));
    }

    #[test]
    fn threshold_ok_with_no_options() {
        let snapshot = PerfSnapshot {
            label: "test:dev:0".into(),
            memory_mb: Some(9999.0),
            cpu_percent: None,
            threads: None,
            file_descriptors: None,
            disk_mb: None,
            net_rx_kb: None,
            net_tx_kb: None,
            launch_ms: None,
            timestamp: "0".into(),
        };
        assert!(matches!(
            evaluate_thresholds(&snapshot, None),
            ThresholdResult::Ok
        ));
    }

    // ── perf: _perf variable writing ─────────────────────────────────

    #[test]
    fn write_perf_var_creates_object_with_dot_paths() {
        let snapshot = PerfSnapshot {
            label: "login:dev:0".into(),
            memory_mb: Some(142.5),
            cpu_percent: Some(23.1),
            threads: Some(42),
            file_descriptors: Some(87),
            disk_mb: Some(24.1),
            net_rx_kb: Some(156.0),
            net_tx_kb: Some(32.0),
            launch_ms: Some(1240),
            timestamp: "12345".into(),
        };
        let mut vars = VariableStore::new();
        write_perf_var(&mut vars, &snapshot);

        let val = vars.get("_perf").expect("_perf SHALL exist");
        assert_eq!(
            val.get_path("memory_mb").and_then(|v| v.as_str()),
            Some("142.5")
        );
        assert_eq!(
            val.get_path("cpu_percent").and_then(|v| v.as_str()),
            Some("23.1")
        );
        assert_eq!(val.get_path("threads").and_then(|v| v.as_str()), Some("42"));
        assert_eq!(
            val.get_path("launch_ms").and_then(|v| v.as_str()),
            Some("1240")
        );
        assert_eq!(
            val.get_path("label").and_then(|v| v.as_str()),
            Some("login:dev:0")
        );
    }

    // ── step_target: label construction ──────────────────────────────

    // 1. A bare step (no selectors/input/options) yields an empty label.
    #[test]
    fn step_target_empty_for_bare_step() {
        let step = empty_step("screenshot");
        assert_eq!(
            step_target(&step),
            "",
            "bare step SHALL produce empty label"
        );
    }

    // 2. Each direct selector field is rendered with its own key=value.
    #[test]
    fn step_target_renders_direct_selectors_in_order() {
        let mut step = empty_step("tap");
        step.on_text = Some("Submit".into());
        step.on_accessibility_label = Some("submit-btn".into());
        step.on_below = Some("Header".into());
        step.on_right_of = Some("Icon".into());
        step.app = Some("myapp".into());
        assert_eq!(
            step_target(&step),
            "on_text=\"Submit\" on_accessibility_label=\"submit-btn\" on_below=\"Header\" on_right_of=\"Icon\" app=\"myapp\"",
            "fields SHALL render in definition order with quoted values"
        );
    }

    // 3. The `on` SelectorGroup contributes text and accessibility_label parts.
    #[test]
    fn step_target_renders_on_group_fields() {
        let mut step = empty_step("tap");
        step.on = Some(golem_parser::SelectorGroup {
            text: Some("Next".into()),
            accessibility_label: Some("next-aria".into()),
            ..Default::default()
        });
        assert_eq!(
            step_target(&step),
            "text=\"Next\" accessibility_label=\"next-aria\"",
            "on group SHALL contribute text and accessibility_label parts"
        );
    }

    // 4. Input is truncated to 20 chars, counting characters not bytes
    //    (a multibyte char at the boundary SHALL not panic or split).
    #[test]
    fn step_target_truncates_input_by_chars() {
        let mut step = empty_step("type");
        // 25 multibyte characters; only the first 20 SHALL be shown.
        step.input = Some("あ".repeat(25));
        let label = step_target(&step);
        assert_eq!(
            label,
            format!("input=\"{}\"", "あ".repeat(20)),
            "input SHALL be truncated to 20 chars without splitting a multibyte char"
        );
    }

    // 5. auto_scroll=Some(false) SHALL NOT emit a token; only Some(true) does.
    #[test]
    fn step_target_auto_scroll_only_when_true() {
        let mut step = empty_step("tap");
        step.on_text = Some("X".into());
        step.auto_scroll = Some(false);
        assert_eq!(
            step_target(&step),
            "on_text=\"X\"",
            "auto_scroll=false SHALL NOT add a token"
        );
        step.auto_scroll = Some(true);
        assert_eq!(
            step_target(&step),
            "on_text=\"X\" auto_scroll",
            "auto_scroll=true SHALL add the bare token"
        );
    }

    // 6. timeout renders as timeout=<n>.
    #[test]
    fn step_target_renders_timeout() {
        let mut step = empty_step("tap");
        step.on_text = Some("X".into());
        step.timeout = Some(1500);
        assert_eq!(step_target(&step), "on_text=\"X\" timeout=1500");
    }

    // ── build_snapshot: field copying ────────────────────────────────

    // 7. build_snapshot copies every raw metric and attaches the
    //    supplied label / launch_ms / timestamp verbatim.
    #[test]
    fn build_snapshot_copies_raw_and_metadata() {
        let raw = RawPerfData {
            memory_mb: Some(101.5),
            cpu_percent: Some(12.0),
            threads: Some(11),
            file_descriptors: Some(22),
            disk_mb: Some(33.0),
            net_rx_kb: Some(44.0),
            net_tx_kb: Some(55.0),
        };
        let snap = build_snapshot(&raw, "lbl:dev:0".into(), Some(900), "ts-1".into());
        assert_eq!(snap.label, "lbl:dev:0");
        assert_eq!(snap.memory_mb, Some(101.5));
        assert_eq!(snap.cpu_percent, Some(12.0));
        assert_eq!(snap.threads, Some(11));
        assert_eq!(snap.file_descriptors, Some(22));
        assert_eq!(snap.disk_mb, Some(33.0));
        assert_eq!(snap.net_rx_kb, Some(44.0));
        assert_eq!(snap.net_tx_kb, Some(55.0));
        assert_eq!(snap.launch_ms, Some(900), "launch_ms SHALL pass through");
        assert_eq!(snap.timestamp, "ts-1", "timestamp SHALL pass through");
    }

    // ── evaluate_thresholds: per-metric branches ─────────────────────

    fn snapshot_with(
        memory: Option<f64>,
        cpu: Option<f64>,
        threads: Option<u32>,
        fds: Option<u32>,
    ) -> PerfSnapshot {
        PerfSnapshot {
            label: "t:dev:0".into(),
            memory_mb: memory,
            cpu_percent: cpu,
            threads,
            file_descriptors: fds,
            disk_mb: None,
            net_rx_kb: None,
            net_tx_kb: None,
            launch_ms: None,
            timestamp: "0".into(),
        }
    }

    // 8. CPU over the error threshold yields an Error.
    #[test]
    fn threshold_error_on_cpu() {
        let snap = snapshot_with(None, Some(95.0), None, None);
        let opts = FlowOptions {
            perf_cpu_error_percent: Some(90.0),
            ..Default::default()
        };
        match evaluate_thresholds(&snap, Some(&opts)) {
            ThresholdResult::Error(msg) => assert!(msg.contains("CPU"), "SHALL mention CPU: {msg}"),
            _ => panic!("expected Error for CPU over error threshold"),
        }
    }

    // 9. Threads over the error threshold yields an Error.
    #[test]
    fn threshold_error_on_threads() {
        let snap = snapshot_with(None, None, Some(500), None);
        let opts = FlowOptions {
            perf_threads_error: Some(256),
            ..Default::default()
        };
        match evaluate_thresholds(&snap, Some(&opts)) {
            ThresholdResult::Error(msg) => {
                assert!(msg.contains("threads"), "SHALL mention threads: {msg}")
            }
            _ => panic!("expected Error for threads over error threshold"),
        }
    }

    // 10. File descriptors over the error threshold yields an Error.
    #[test]
    fn threshold_error_on_fds() {
        let snap = snapshot_with(None, None, None, Some(1024));
        let opts = FlowOptions {
            perf_fd_error: Some(512),
            ..Default::default()
        };
        match evaluate_thresholds(&snap, Some(&opts)) {
            ThresholdResult::Error(msg) => assert!(msg.contains("FDs"), "SHALL mention FDs: {msg}"),
            _ => panic!("expected Error for FDs over error threshold"),
        }
    }

    // 11. CPU over the warn threshold (and under/absent error) yields a Warn.
    #[test]
    fn threshold_warn_on_cpu() {
        let snap = snapshot_with(None, Some(85.0), None, None);
        let opts = FlowOptions {
            perf_cpu_warn_percent: Some(80.0),
            ..Default::default()
        };
        match evaluate_thresholds(&snap, Some(&opts)) {
            ThresholdResult::Warn(msg) => {
                assert!(msg.contains("warning"), "SHALL mention warning: {msg}")
            }
            _ => panic!("expected Warn for CPU over warn threshold"),
        }
    }

    // 12. Threads over the warn threshold yields a Warn.
    #[test]
    fn threshold_warn_on_threads() {
        let snap = snapshot_with(None, None, Some(150), None);
        let opts = FlowOptions {
            perf_threads_warn: Some(100),
            ..Default::default()
        };
        match evaluate_thresholds(&snap, Some(&opts)) {
            ThresholdResult::Warn(msg) => {
                assert!(msg.contains("threads"), "SHALL mention threads: {msg}")
            }
            _ => panic!("expected Warn for threads over warn threshold"),
        }
    }

    // 13. File descriptors over the warn threshold yields a Warn.
    #[test]
    fn threshold_warn_on_fds() {
        let snap = snapshot_with(None, None, None, Some(300));
        let opts = FlowOptions {
            perf_fd_warn: Some(200),
            ..Default::default()
        };
        match evaluate_thresholds(&snap, Some(&opts)) {
            ThresholdResult::Warn(msg) => assert!(msg.contains("FDs"), "SHALL mention FDs: {msg}"),
            _ => panic!("expected Warn for FDs over warn threshold"),
        }
    }

    // 14. Error thresholds are checked before warn thresholds: when a
    //     metric trips both, the result SHALL be the Error variant.
    #[test]
    fn threshold_error_takes_precedence_over_warn() {
        let snap = snapshot_with(None, Some(95.0), None, None);
        let opts = FlowOptions {
            perf_cpu_warn_percent: Some(80.0),
            perf_cpu_error_percent: Some(90.0),
            ..Default::default()
        };
        match evaluate_thresholds(&snap, Some(&opts)) {
            ThresholdResult::Error(msg) => {
                assert!(msg.contains("error threshold"), "SHALL be error: {msg}")
            }
            _ => panic!("error SHALL win over warn when both trip"),
        }
    }

    // 15. A metric exactly equal to its threshold does NOT trip (strict >).
    #[test]
    fn threshold_equal_is_not_over() {
        let snap = snapshot_with(Some(200.0), None, None, None);
        let opts = FlowOptions {
            perf_memory_warn_mb: Some(200.0),
            ..Default::default()
        };
        assert!(
            matches!(evaluate_thresholds(&snap, Some(&opts)), ThresholdResult::Ok),
            "value equal to threshold SHALL NOT trip (comparison is strict >)"
        );
    }

    // 16. A metric value present but the matching limit unset is ignored.
    #[test]
    fn threshold_ok_when_limit_unset() {
        let snap = snapshot_with(Some(9999.0), Some(99.0), Some(9999), Some(9999));
        let opts = FlowOptions::default();
        assert!(
            matches!(evaluate_thresholds(&snap, Some(&opts)), ThresholdResult::Ok),
            "no configured limits SHALL never trip a threshold"
        );
    }

    // ── write_perf_var: None metrics render as empty strings ─────────

    // 17. A snapshot whose numeric metrics are all None writes empty
    //     strings for those keys (never absent, never "0").
    #[test]
    fn write_perf_var_none_metrics_become_empty_strings() {
        let snapshot = PerfSnapshot {
            label: "lbl".into(),
            memory_mb: None,
            cpu_percent: None,
            threads: None,
            file_descriptors: None,
            disk_mb: None,
            net_rx_kb: None,
            net_tx_kb: None,
            launch_ms: None,
            timestamp: "ts".into(),
        };
        let mut vars = VariableStore::new();
        write_perf_var(&mut vars, &snapshot);
        let val = vars.get("_perf").expect("_perf SHALL exist");
        for key in [
            "memory_mb",
            "cpu_percent",
            "threads",
            "file_descriptors",
            "disk_mb",
            "net_rx_kb",
            "net_tx_kb",
            "launch_ms",
        ] {
            assert_eq!(
                val.get_path(key).and_then(|v| v.as_str()),
                Some(""),
                "None metric '{key}' SHALL render as an empty string"
            );
        }
        assert_eq!(
            val.get_path("label").and_then(|v| v.as_str()),
            Some("lbl"),
            "label SHALL still be written"
        );
    }

    // ── parse_duration: numeric boundaries ───────────────────────────

    // 18. Zero values for the second/millisecond units parse to a zero
    //     Duration rather than being rejected as falsy/empty. (Minute and
    //     hour multiplication are already covered by parse_duration_formats
    //     via 5m/2h, so they are not re-asserted here.)
    #[test]
    fn parse_duration_zero_and_multiplied_units() {
        assert_eq!(parse_duration("0s"), Some(Duration::from_secs(0)));
        assert_eq!(parse_duration("0ms"), Some(Duration::from_millis(0)));
    }

    // ── Perf-collection path (item #24) ──────────────────────────────
    //
    // The block-boundary perf capture in `execute_flow` (the
    // `if perf_enabled { if let Some(collector) = ctx.perf_collector`
    // arm) is unreachable from `test_ctx`, which wires `perf_collector:
    // None`. `TestHarness` injects a `PerfCollectorSet::from_raw`
    // (caller-supplied `RawPerfData`, no device I/O) plus a capturing
    // emitter, letting these tests drive the real capture pipeline
    // deterministically.

    // 19. With a collector wired in and perf enabled (default), a block
    //     boundary SHALL capture one snapshot per executed block from the
    //     injected raw data — no real device touched.
    #[tokio::test]
    async fn perf_snapshot_captured_from_injected_raw() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let raw = RawPerfData {
            memory_mb: Some(123.5),
            cpu_percent: Some(8.0),
            threads: Some(11),
            ..RawPerfData::default()
        };
        let harness =
            crate::context::TestHarness::new(tmp.path(), &[("com.example.app".to_string(), raw)]);
        let mut ctx = harness.ctx();
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        // 19a. Two blocks SHALL each contribute a boundary snapshot.
        let flow = make_flow(vec![
            make_block(Some("first"), vec![make_success_step()]),
            make_block(Some("second"), vec![make_success_step()]),
        ]);

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
        .expect("execute_flow SHALL not error");

        assert!(result.success, "flow SHALL succeed");
        assert_eq!(
            result.perf_snapshots.len(),
            2,
            "one snapshot SHALL be captured per executed block"
        );
        // 19b. The injected raw data SHALL flow through into the snapshot.
        let snap = &result.perf_snapshots[0];
        assert_eq!(
            snap.memory_mb,
            Some(123.5),
            "injected memory SHALL appear in the captured snapshot"
        );
        assert_eq!(snap.cpu_percent, Some(8.0));
        assert_eq!(snap.threads, Some(11));
        // 19c. The snapshot label SHALL carry the collector's active bundle.
        assert!(
            snap.label.contains("com.example.app"),
            "snapshot label SHALL name the active bundle: {}",
            snap.label
        );
        // 19d. The capture SHALL publish the metrics into the `_perf` var.
        let perf_var = vars.get("_perf").expect("_perf var SHALL be written");
        assert_eq!(
            perf_var.get_path("memory_mb").and_then(|v| v.as_str()),
            Some("123.5"),
            "_perf.memory_mb SHALL reflect the injected reading"
        );
    }

    // 20. An injected reading above a configured error threshold SHALL
    //     fail the flow with a perf reason — exercising the
    //     `ThresholdResult::Error` arm of the capture path.
    #[tokio::test]
    async fn perf_error_threshold_fails_flow() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let raw = RawPerfData {
            memory_mb: Some(500.0),
            ..RawPerfData::default()
        };
        let harness =
            crate::context::TestHarness::new(tmp.path(), &[("com.example.app".to_string(), raw)]);
        let mut ctx = harness.ctx();
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let options = FlowOptions {
            perf_memory_error_mb: Some(256.0),
            ..FlowOptions::default()
        };
        let flow = make_flow_with_options(
            vec![make_block(Some("heavy"), vec![make_success_step()])],
            options,
        );

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
        .expect("execute_flow SHALL return Ok(FlowResult), not Err");

        assert!(
            !result.success,
            "breaching the error threshold SHALL fail the flow"
        );
        let reason = result
            .failed_reason
            .expect("a perf error SHALL set a failure reason");
        assert!(
            reason.contains("memory") && reason.contains("error threshold"),
            "failure reason SHALL describe the perf error: {reason}"
        );
    }

    // 21. With perf explicitly disabled, the collector SHALL NOT be
    //     consulted: no snapshots are produced even though one is wired in.
    #[tokio::test]
    async fn perf_disabled_skips_collection() {
        let tmp = tempfile::tempdir().expect("SHALL create temp dir");
        let raw = RawPerfData {
            memory_mb: Some(999.0),
            ..RawPerfData::default()
        };
        let harness =
            crate::context::TestHarness::new(tmp.path(), &[("com.example.app".to_string(), raw)]);
        let mut ctx = harness.ctx();
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut vars = VariableStore::new();
        let options = FlowOptions {
            perf: Some(false),
            ..FlowOptions::default()
        };
        let flow = make_flow_with_options(
            vec![make_block(Some("only"), vec![make_success_step()])],
            options,
        );

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
        .expect("execute_flow SHALL not error");

        assert!(result.success, "flow SHALL succeed");
        assert!(
            result.perf_snapshots.is_empty(),
            "perf=false SHALL skip capture entirely"
        );
        assert!(
            vars.get("_perf").is_none(),
            "disabled perf SHALL not write the _perf var"
        );
    }
}
