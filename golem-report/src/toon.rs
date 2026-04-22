//! TOON (Token-Optimized Output Notation) formatter.
//!
//! Produces a compact, line-based format designed for LLM consumption.
//! Uses fewer tokens than the human-readable format while remaining parseable.
//!
//! # Format overview
//!
//! ```text
//! S:flow_name d:duration_ms [seed:N]
//!  +action:target duration
//!  ~action:target duration message
//!  !action:target duration error
//!  -action:target
//! R:PASS|FAIL passed/warned/failed
//!
//! T:passed/failed/skipped d:duration
//! ```

use crate::{FlowReport, StepOutcome, StepReport, SuiteReport};
use std::fmt::Write;

/// Format a single step as one compact TOON line.
///
/// Examples:
/// - ` +tap:Sign Up 45`
/// - ` !assert_visible:Welcome 10012 timed out`
/// - ` ~assert_visible:Promo 15 element not found`
/// - ` -tap:Cancel`
pub fn format_step_toon(step: &StepReport) -> String {
    let label = if step.target.is_empty() {
        step.action.clone()
    } else {
        format!("{}:{}", step.action, step.target)
    };

    let substep_suffix = format_substeps_toon(&step.substeps);
    let tree_suffix = if step.tree_stats.fetches > 0 {
        format!(" t:{}/{}", step.tree_stats.fetches, step.tree_stats.max_nodes)
    } else {
        String::new()
    };

    match &step.outcome {
        StepOutcome::Success => {
            format!(" +{label} {}{substep_suffix}{tree_suffix}", step.duration_ms)
        }
        StepOutcome::Warning(msg) => {
            format!(" ~{label} {} {msg}{substep_suffix}", step.duration_ms)
        }
        StepOutcome::Failed(msg) => {
            format!(" !{label} {} {msg}{substep_suffix}", step.duration_ms)
        }
        StepOutcome::Skipped => {
            format!(" -{label}{substep_suffix}")
        }
    }
}

/// Compact substep notation for TOON: @x,y for tap/found position, s:N for scroll attempts, b:x,y,w,h for bounds.
fn format_substeps_toon(substeps: &[crate::SubstepDetail]) -> String {
    use crate::SubstepDetail;
    if substeps.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    for sub in substeps {
        match sub {
            SubstepDetail::Tap { point, element_bounds: Some(b), .. } =>
                parts.push(format!("@{},{} b{},{},{},{}", point.x, point.y, b.x, b.y, b.width, b.height)),
            SubstepDetail::Tap { point, .. } =>
                parts.push(format!("@{},{}", point.x, point.y)),
            SubstepDetail::ElementResolved { bounds, tap_point, .. } =>
                parts.push(format!("@{},{} b{},{},{},{}", tap_point.x, tap_point.y, bounds.x, bounds.y, bounds.width, bounds.height)),
            SubstepDetail::ScrollFound { position, total_attempts, .. } =>
                parts.push(format!("s:{total_attempts} @{},{}", position.x, position.y)),
            SubstepDetail::AppLaunch { bundle, duration_ms } =>
                parts.push(format!("launch:{bundle} {duration_ms}ms")),
            SubstepDetail::AppStop { bundle } =>
                parts.push(format!("stop:{bundle}")),
            SubstepDetail::TextInput { text, .. } =>
                parts.push(format!("t:\"{text}\"")),
            SubstepDetail::Swipe { from, to } =>
                parts.push(format!("({},{})→({},{})", from.x, from.y, to.x, to.y)),
            SubstepDetail::ElementNotFound { timeout_ms, .. } =>
                parts.push(format!("!found {timeout_ms}ms")),
            SubstepDetail::ScrollStarted { direction, .. } =>
                parts.push(format!("dir:{direction}")),
            SubstepDetail::ScrollAttempt { attempt, direction, from, to, result, .. } =>
                parts.push(format!("#{attempt} {direction} ({},{})→({},{}) {result}",
                    from.x, from.y, to.x, to.y)),
            SubstepDetail::ScrollDirectionReversed { to_direction, reason } =>
                parts.push(format!("rev→{to_direction} {reason}")),
            SubstepDetail::ScrollStrategySwitch { to_index, reason } =>
                parts.push(format!("strat→{} {reason}", to_index + 1)),
            SubstepDetail::RetryAttempt { attempt, max, error, .. } =>
                parts.push(format!("retry {attempt}/{max}: {error}")),
            SubstepDetail::HttpRequest { method, status, duration_ms, .. } => {
                let s = status.map(|s| s.to_string()).unwrap_or_else(|| "?".to_string());
                parts.push(format!("{method}→{s} {duration_ms}ms"));
            }
            SubstepDetail::BashCommand { command, exit_code, duration_ms } => {
                let c = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
                parts.push(format!("bash:\"{command}\" exit={c} {duration_ms}ms"));
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" {}", parts.join(" "))
    }
}

/// Format a complete flow report in TOON notation.
///
/// Includes a header line, step lines, and a result line.
pub fn format_flow_toon(report: &FlowReport) -> String {
    let mut out = String::new();

    // Header: S:flow_name d:duration [seed:N]
    let _ = write!(out, "S:{} d:{}", report.flow_name, report.duration_ms);
    if let Some(seed) = report.seed {
        let _ = write!(out, " seed:{seed}");
    }
    out.push('\n');

    // Steps
    for step in &report.step_results {
        let _ = writeln!(out, "{}", format_step_toon(step));
    }

    // Counts
    let mut passed: usize = 0;
    let mut warned: usize = 0;
    let mut failed: usize = 0;
    for step in &report.step_results {
        match &step.outcome {
            StepOutcome::Success => passed += 1,
            StepOutcome::Warning(_) => warned += 1,
            StepOutcome::Failed(_) => failed += 1,
            StepOutcome::Skipped => {}
        }
    }

    // Perf lines: P label m:val c:val t:val f:val d:val nr:val nt:val l:val
    for snap in &report.perf_snapshots {
        let _ = write!(out, "P {}", snap.label);
        if let Some(v) = snap.memory_mb {
            let _ = write!(out, " mem:{v:.1}");
        }
        if let Some(v) = snap.cpu_percent {
            let _ = write!(out, " cpu:{v:.1}");
        }
        if let Some(v) = snap.threads {
            let _ = write!(out, " thr:{v}");
        }
        if let Some(v) = snap.file_descriptors {
            let _ = write!(out, " fd:{v}");
        }
        if let Some(v) = snap.disk_mb {
            let _ = write!(out, " disk:{v:.1}");
        }
        if let Some(v) = snap.net_rx_kb {
            let _ = write!(out, " net_rx:{v:.0}");
        }
        if let Some(v) = snap.net_tx_kb {
            let _ = write!(out, " net_tx:{v:.0}");
        }
        if let Some(v) = snap.launch_ms {
            let _ = write!(out, " launch:{v}");
        }
        out.push('\n');
    }

    // Result line: R:PASS|FAIL passed/warned/failed
    let status = if report.success { "PASS" } else { "FAIL" };
    let _ = writeln!(out, "R:{status} {passed}/{warned}/{failed}");

    out
}

/// Format an entire suite report in TOON notation.
///
/// Includes all flows followed by a total summary line.
pub fn format_suite_toon(report: &SuiteReport) -> String {
    let mut out = String::new();

    // Schema header for LLM comprehension
    out.push_str("# S=suite R=result(passed/warned/failed) T=total(passed/failed/skipped) d:N=duration_ms\n");
    out.push_str("# step: +=pass !=fail ~=warn -=skip @x,y=position b=bounds(x,y,w,h) s:N=scroll_attempts t:N/M=trees/nodes\n");
    out.push_str("# perf: P block:app:device:iteration mem=MB cpu=% thr=threads fd=file_descriptors disk=MB net_rx/tx=KB launch=ms\n");
    out.push_str("# install: I app:bundle:device R=ok/fail d:ms (device = `{platform}/{name}`)\n");

    // Install results (one line per (device, bundle) attempted)
    for inst in &report.installs {
        let r = if inst.success { "ok" } else { "fail" };
        let _ = writeln!(
            out,
            "I {}:{}:{} R:{} d:{}",
            inst.app_name, inst.bundle_id, inst.device_name, r, inst.duration_ms
        );
        if let Some(ref err) = inst.error {
            // Indent error lines under the install entry.
            for line in err.lines() {
                let _ = writeln!(out, "  {line}");
            }
        }
    }

    for flow in &report.flows {
        let _ = write!(out, "{}", format_flow_toon(flow));
        out.push('\n');
    }

    // Aggregate counts at the flow level
    let total_passed = report.flows.iter().filter(|f| f.success).count();
    let total_failed = report.flows.iter().filter(|f| !f.success).count();
    let total_skipped = report
        .flows
        .iter()
        .filter(|f| {
            f.step_results
                .iter()
                .all(|s| matches!(s.outcome, StepOutcome::Skipped))
        })
        .count();

    let _ = writeln!(
        out,
        "T:{total_passed}/{total_failed}/{total_skipped} d:{}",
        report.total_duration_ms
    );

    out
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::human;

    // Helpers --------------------------------------------------------

    fn success_step(action: &str, target: &str, ms: u64) -> StepReport {
        StepReport {
            global_step_index: 0,
            block_name: String::new(),
            step_index_in_block: 0,
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Success,
            duration_ms: ms,
            retry_count: 0,
            screenshot_path: None,
            substeps: vec![],
            tree_stats: golem_events::TreeStats::default(),
        }
    }

    fn failed_step(action: &str, target: &str, ms: u64, msg: &str) -> StepReport {
        StepReport {
            global_step_index: 0,
            block_name: String::new(),
            step_index_in_block: 0,
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Failed(msg.to_string()),
            duration_ms: ms,
            retry_count: 0,
            screenshot_path: None,
            substeps: vec![],
            tree_stats: golem_events::TreeStats::default(),
        }
    }

    fn warning_step(action: &str, target: &str, ms: u64, msg: &str) -> StepReport {
        StepReport {
            global_step_index: 0,
            block_name: String::new(),
            step_index_in_block: 0,
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Warning(msg.to_string()),
            duration_ms: ms,
            retry_count: 0,
            screenshot_path: None,
            substeps: vec![],
            tree_stats: golem_events::TreeStats::default(),
        }
    }

    fn skipped_step(action: &str, target: &str) -> StepReport {
        StepReport {
            global_step_index: 0,
            block_name: String::new(),
            step_index_in_block: 0,
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Skipped,
            duration_ms: 0,
            retry_count: 0,
            screenshot_path: None,
            substeps: vec![],
            tree_stats: golem_events::TreeStats::default(),
        }
    }

    fn sample_flow(success: bool, seed: Option<u64>) -> FlowReport {
        FlowReport {
            flow_name: "login_flow".to_string(),
            success,
            step_results: vec![
                success_step("launch", "", 120),
                success_step("tap", "Sign Up", 45),
                success_step("type", "email", 32),
                warning_step("assert_visible", "Promo", 15, "element not found"),
                success_step("tap", "Submit", 38),
                failed_step("assert_visible", "Welcome", 10012, "timed out"),
            ],
            warnings: vec![],
            duration_ms: 10200,
            seed,
            screenshot_path: None,
            device_name: None,
            perf_snapshots: vec![],
            skipped_reason: None,
        }
    }

    // 1. Step success format: ` +action:target duration` ---------------

    #[test]
    fn step_success_format() {
        let step = success_step("tap", "Sign Up", 45);
        let out = format_step_toon(&step);
        assert_eq!(out, " +tap:Sign Up 45");
    }

    // 2. Step failure format: ` !action:target duration error` ---------

    #[test]
    fn step_failure_format() {
        let step = failed_step("assert_visible", "Welcome", 10012, "timed out");
        let out = format_step_toon(&step);
        assert_eq!(out, " !assert_visible:Welcome 10012 timed out");
    }

    // 3. Step warning format: ` ~action:target duration message` -------

    #[test]
    fn step_warning_format() {
        let step = warning_step("assert_visible", "Promo", 15, "element not found");
        let out = format_step_toon(&step);
        assert_eq!(out, " ~assert_visible:Promo 15 element not found");
    }

    // 4. Step skipped format: ` -action:target` ------------------------

    #[test]
    fn step_skipped_format() {
        let step = skipped_step("tap", "Cancel");
        let out = format_step_toon(&step);
        assert_eq!(out, " -tap:Cancel");
    }

    // 5. Flow header includes name and duration ------------------------

    #[test]
    fn flow_header_includes_name_and_duration() {
        let report = sample_flow(false, None);
        let out = format_flow_toon(&report);
        let first_line = out.lines().next().expect("should have at least one line");
        assert_eq!(first_line, "S:login_flow d:10200");
    }

    // 6. Flow header includes seed when present ------------------------

    #[test]
    fn flow_header_includes_seed() {
        let report = sample_flow(false, Some(847_291_036));
        let out = format_flow_toon(&report);
        let first_line = out.lines().next().expect("should have at least one line");
        assert_eq!(first_line, "S:login_flow d:10200 seed:847291036");
    }

    // 7. Flow result line shows PASS/FAIL with counts ------------------

    #[test]
    fn flow_result_line_pass_with_counts() {
        let report = FlowReport {
            flow_name: "simple".to_string(),
            success: true,
            step_results: vec![
                success_step("launch", "", 100),
                success_step("tap", "OK", 50),
            ],
            warnings: vec![],
            duration_ms: 150,
            seed: None,
            screenshot_path: None,
            device_name: None,
            perf_snapshots: vec![],
            skipped_reason: None,
        };
        let out = format_flow_toon(&report);
        let last_line = out.lines().last().expect("should have lines");
        assert_eq!(last_line, "R:PASS 2/0/0");
    }

    #[test]
    fn flow_result_line_fail_with_counts() {
        let report = sample_flow(false, None);
        let out = format_flow_toon(&report);
        let last_line = out.lines().last().expect("should have lines");
        // 4 passed, 1 warned, 1 failed
        assert_eq!(last_line, "R:FAIL 4/1/1");
    }

    // 8. Suite total line format correct --------------------------------

    #[test]
    fn suite_total_line_format() {
        let suite = SuiteReport {
            flows: vec![
                FlowReport {
                    flow_name: "login_flow".to_string(),
                    success: true,
                    step_results: vec![
                        success_step("launch", "", 100),
                        success_step("tap", "OK", 50),
                    ],
                    warnings: vec![],
                    duration_ms: 150,
                    seed: None,
                    screenshot_path: None,
                    device_name: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                },
                FlowReport {
                    flow_name: "signup_flow".to_string(),
                    success: false,
                    step_results: vec![
                        success_step("launch", "", 80),
                        failed_step("assert_visible", "Welcome", 5000, "not found"),
                    ],
                    warnings: vec![],
                    duration_ms: 5080,
                    seed: None,
                    screenshot_path: None,
                    device_name: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                },
            ],
            installs: Vec::new(),
            total_duration_ms: 45300,
        };

        let out = format_suite_toon(&suite);
        let last_line = out.lines().last().expect("should have lines");
        assert_eq!(last_line, "T:1/1/0 d:45300");
    }

    // 9. Multiple flows in suite ----------------------------------------

    #[test]
    fn suite_multiple_flows() {
        let suite = SuiteReport {
            flows: vec![
                FlowReport {
                    flow_name: "flow_a".to_string(),
                    success: true,
                    step_results: vec![success_step("launch", "", 100)],
                    warnings: vec![],
                    duration_ms: 100,
                    seed: None,
                    screenshot_path: None,
                    device_name: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                },
                FlowReport {
                    flow_name: "flow_b".to_string(),
                    success: true,
                    step_results: vec![success_step("launch", "", 200)],
                    warnings: vec![],
                    duration_ms: 200,
                    seed: Some(42),
                    screenshot_path: None,
                    device_name: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                },
                FlowReport {
                    flow_name: "flow_c".to_string(),
                    success: false,
                    step_results: vec![
                        failed_step("tap", "Nope", 300, "gone"),
                    ],
                    warnings: vec![],
                    duration_ms: 300,
                    seed: None,
                    screenshot_path: None,
                    device_name: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                },
            ],
            installs: Vec::new(),
            total_duration_ms: 600,
        };

        let out = format_suite_toon(&suite);

        // All three flow headers should appear in order
        let flow_a_pos = out.find("S:flow_a").expect("should contain flow_a");
        let flow_b_pos = out.find("S:flow_b").expect("should contain flow_b");
        let flow_c_pos = out.find("S:flow_c").expect("should contain flow_c");
        assert!(flow_a_pos < flow_b_pos, "flow_a before flow_b");
        assert!(flow_b_pos < flow_c_pos, "flow_b before flow_c");

        // flow_b has seed
        assert!(out.contains("S:flow_b d:200 seed:42"));

        // Result lines
        assert!(out.contains("R:PASS 1/0/0"));
        assert!(out.contains("R:FAIL 0/0/1"));

        // Total line: 2 passed, 1 failed, 0 skipped
        let last_line = out.lines().last().expect("should have lines");
        assert_eq!(last_line, "T:2/1/0 d:600");
    }

    // 10. TOON uses fewer characters than human format (token efficiency)

    #[test]
    fn toon_is_more_compact_than_human() {
        let report = sample_flow(false, Some(847_291_036));
        let human_out = human::format_flow(&report);
        let toon_out = format_flow_toon(&report);

        assert!(
            toon_out.len() < human_out.len(),
            "TOON ({} chars) should be shorter than human ({} chars)",
            toon_out.len(),
            human_out.len()
        );
    }

    // 11. Step with empty target omits colon separator -------------------

    #[test]
    fn step_success_no_target_omits_colon() {
        let step = success_step("launch", "", 120);
        let out = format_step_toon(&step);
        assert_eq!(out, " +launch 120");
    }

    // ── format_substeps_toon tests ─────────────────────────────────

    #[test]
    fn substeps_toon_tap_with_bounds_produces_at_xy_bxywh() {
        let substeps = vec![crate::SubstepDetail::Tap {
            point: golem_events::Point { x: 150, y: 300 },
            element_bounds: Some(golem_events::Rect { x: 100, y: 280, width: 100, height: 44 }),
        }];
        let out = format_substeps_toon(&substeps);
        assert_eq!(out, " @150,300 b100,280,100,44",
            "SHALL produce @x,y bx,y,w,h notation for Tap with bounds");
    }

    #[test]
    fn substeps_toon_scroll_found_produces_s_n_at_xy() {
        let substeps = vec![crate::SubstepDetail::ScrollFound {
            selector: "text=Price".into(),
            position: golem_events::Point { x: 200, y: 800 },
            total_attempts: 3,
        }];
        let out = format_substeps_toon(&substeps);
        assert_eq!(out, " s:3 @200,800",
            "SHALL produce s:N @x,y notation for ScrollFound");
    }

    #[test]
    fn substeps_toon_empty_produces_empty_string() {
        let out = format_substeps_toon(&[]);
        assert_eq!(out, "", "SHALL produce empty string for empty substeps");
    }

    #[test]
    fn substeps_toon_app_launch_produces_launch_bundle_nms() {
        let substeps = vec![crate::SubstepDetail::AppLaunch {
            bundle: "com.example.app".into(),
            duration_ms: 1500,
        }];
        let out = format_substeps_toon(&substeps);
        assert_eq!(out, " launch:com.example.app 1500ms",
            "SHALL produce launch:bundle Nms notation for AppLaunch");
    }

    #[test]
    fn substeps_toon_tap_without_bounds_produces_at_xy_only() {
        let substeps = vec![crate::SubstepDetail::Tap {
            point: golem_events::Point { x: 50, y: 60 },
            element_bounds: None,
        }];
        let out = format_substeps_toon(&substeps);
        assert_eq!(out, " @50,60",
            "SHALL produce @x,y without bounds for Tap without element_bounds");
    }

    #[test]
    fn substeps_toon_element_resolved_produces_at_and_bounds() {
        let substeps = vec![crate::SubstepDetail::ElementResolved {
            selector: "text=OK".into(),
            bounds: golem_events::Rect { x: 10, y: 20, width: 80, height: 40 },
            tap_point: golem_events::Point { x: 50, y: 40 },
        }];
        let out = format_substeps_toon(&substeps);
        assert_eq!(out, " @50,40 b10,20,80,40",
            "SHALL produce @tap_x,tap_y bx,y,w,h for ElementResolved");
    }

    #[test]
    fn substeps_toon_multiple_substeps_joined_with_space() {
        let substeps = vec![
            crate::SubstepDetail::ElementResolved {
                selector: "text=OK".into(),
                bounds: golem_events::Rect { x: 10, y: 20, width: 80, height: 40 },
                tap_point: golem_events::Point { x: 50, y: 40 },
            },
            crate::SubstepDetail::Tap {
                point: golem_events::Point { x: 50, y: 40 },
                element_bounds: Some(golem_events::Rect { x: 10, y: 20, width: 80, height: 40 }),
            },
        ];
        let out = format_substeps_toon(&substeps);
        assert_eq!(out, " @50,40 b10,20,80,40 @50,40 b10,20,80,40",
            "SHALL join multiple substep notations with spaces");
    }

    // ── Perf rendering tests ────────────────────────────────────────

    fn sample_perf_snapshot() -> crate::PerfSnapshot {
        crate::PerfSnapshot {
            label: "login:iPhone_16:0".into(),
            memory_mb: Some(142.5),
            cpu_percent: Some(23.1),
            threads: Some(42),
            file_descriptors: Some(87),
            disk_mb: Some(24.1),
            net_rx_kb: Some(156.0),
            net_tx_kb: Some(32.0),
            launch_ms: Some(1240),
            timestamp: "12345".into(),
        }
    }

    #[test]
    fn toon_includes_perf_lines() {
        let report = FlowReport {
            flow_name: "perf_flow".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 100)],
            warnings: vec![],
            duration_ms: 100,
            seed: None,
            screenshot_path: None,
            device_name: None,
            perf_snapshots: vec![sample_perf_snapshot()],
            skipped_reason: None,
        };

        let out = format_flow_toon(&report);
        let perf_line = out.lines().find(|l| l.starts_with("P ")).expect("SHALL contain a P line");
        assert!(perf_line.contains("login:iPhone_16:0"), "SHALL contain snapshot label");
        assert!(perf_line.contains("mem:142.5"), "SHALL contain memory value");
        assert!(perf_line.contains("cpu:23.1"), "SHALL contain cpu value");
        assert!(perf_line.contains("launch:1240"), "SHALL contain launch value");
    }

    #[test]
    fn toon_omits_perf_when_empty() {
        let report = FlowReport {
            flow_name: "no_perf_flow".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 100)],
            warnings: vec![],
            duration_ms: 100,
            seed: None,
            screenshot_path: None,
            device_name: None,
            perf_snapshots: vec![],
            skipped_reason: None,
        };

        let out = format_flow_toon(&report);
        assert!(
            !out.lines().any(|l| l.starts_with("P ")),
            "SHALL NOT contain P lines when no perf snapshots"
        );
    }
}
