//! TOON (Token-Optimized Output Notation) formatter.
//!
//! Produces a compact, line-based format designed for LLM consumption.
//! Uses fewer tokens than the human-readable format while remaining parseable.
//!
//! # Format overview
//!
//! ```text
//! F:flow_name dev:platform/name os:major d:duration_ms [seed:N] [cov:axis1,axis2] [t0+:offset]
//!  B:block_name [i:N]
//!  +action:target d:duration
//!  ~action:target d:duration message
//!  !action:target d:duration error
//!  -action:target
//! R:PASS|FAIL|SKIP passed/warned/failed [skip_reason]
//!
//! total:N×pass,N×fail,N×skip d:duration
//! ```

use crate::{FlowReport, StepOutcome, StepReport, SuiteReport};
use std::fmt::Write;

/// Parse an ISO-8601 UTC timestamp (as stored on `*Report.started_at` /
/// `finished_at`) into unix-epoch milliseconds. Returns `None` on any
/// parse failure — TOON treats absent/unparseable timestamps identically
/// (no `T0:` / `t0+:` emitted rather than a bogus value).
fn iso_to_unix_ms(iso: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Format a per-line anchor offset: ` t0+:<delta-ms>` when both the line's
/// `started_at` and the suite anchor are present and valid, empty string
/// otherwise. Keeps the call sites clean.
fn format_t0_offset(line_started_at: Option<&str>, suite_t0_ms: Option<i64>) -> String {
    match (line_started_at.and_then(iso_to_unix_ms), suite_t0_ms) {
        (Some(line_ms), Some(t0)) => format!(" t0+:{}", line_ms - t0),
        _ => String::new(),
    }
}

/// Format a single step as one compact TOON line.
///
/// Examples:
/// - ` +tap:Sign Up d:45`
/// - ` !assert_visible:Welcome d:10012 timed out`
/// - ` ~assert_visible:Promo d:15 element not found`
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
            format!(" +{label} d:{}{substep_suffix}{tree_suffix}", step.duration_ms)
        }
        StepOutcome::Warning { message, code } => {
            let c = code.render(golem_events::Severity::Warning);
            format!(" ~{label} d:{} {c} {message}{substep_suffix}", step.duration_ms)
        }
        StepOutcome::Failed { message, code } => {
            let c = code.render(golem_events::Severity::Error);
            format!(" !{label} d:{} {c} {message}{substep_suffix}", step.duration_ms)
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
            SubstepDetail::DriverWarning { message } =>
                parts.push(format!("warn:\"{message}\"")),
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
    format_flow_toon_anchored(report, None)
}

/// Internal variant that appends a ` t0+:<delta-ms>` offset to the flow
/// header when called from a suite context that carries an anchor.
pub(crate) fn format_flow_toon_anchored(
    report: &FlowReport,
    suite_t0_ms: Option<i64>,
) -> String {
    let mut out = String::new();

    // Header: F:flow_name [dev:...] [os:...] d:duration [seed:N] [t0+:offset]
    let _ = write!(out, "F:{}", report.flow_name);
    if let Some(dev) = report.device_name.as_deref() {
        let _ = write!(out, " dev:{dev}");
    }
    if let Some(os) = report.os_major {
        let _ = write!(out, " os:{os}");
    }
    let _ = write!(out, " d:{}", report.duration_ms);
    if let Some(seed) = report.seed {
        let _ = write!(out, " seed:{seed}");
    }
    if !report.covered_axes.is_empty() {
        let _ = write!(out, " cov:{}", report.covered_axes.join(","));
    }
    let _ = write!(
        out,
        "{}",
        format_t0_offset(report.started_at.as_deref(), suite_t0_ms)
    );
    out.push('\n');

    // Steps, grouped under `B:<block>` headers. A new header is emitted
    // whenever the (block_name, block_iteration) pair changes — so
    // iterating blocks show one header per iteration.
    let mut current_block: Option<(String, u32)> = None;
    for step in &report.step_results {
        let key = (step.block_name.clone(), step.block_iteration);
        if current_block.as_ref() != Some(&key) {
            if !step.block_name.is_empty() {
                if step.block_iteration > 0 {
                    let _ = writeln!(out, " B:{} i:{}", step.block_name, step.block_iteration);
                } else {
                    let _ = writeln!(out, " B:{}", step.block_name);
                }
            }
            current_block = Some(key);
        }
        let _ = writeln!(out, "{}", format_step_toon(step));
    }

    // Counts
    let mut passed: usize = 0;
    let mut warned: usize = 0;
    let mut failed: usize = 0;
    for step in &report.step_results {
        match &step.outcome {
            StepOutcome::Success => passed += 1,
            StepOutcome::Warning { .. } => warned += 1,
            StepOutcome::Failed { .. } => failed += 1,
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

    // Recordings: one ` rec block:iter path` line per recorded block iteration.
    for rec in &report.recordings {
        let _ = writeln!(out, " rec {}:{} {}", rec.block, rec.iteration, rec.path);
    }

    // Result line: R:PASS|FAIL|SKIP passed/warned/failed [skip_reason]
    let status = if report.is_skipped() {
        "SKIP"
    } else if report.success {
        "PASS"
    } else {
        "FAIL"
    };
    let _ = write!(out, "R:{status} {passed}/{warned}/{failed}");
    // Flow-level failure code, mirroring the other formats' flow-summary code.
    if !report.success {
        if let Some(code) = report.first_failure_code {
            let _ = write!(out, " {}", code.render(golem_events::Severity::Error));
        }
    }
    if let Some(ref reason) = report.skipped_reason {
        let _ = write!(out, " {reason}");
    }
    out.push('\n');

    out
}

/// Format an entire suite report in TOON notation.
///
/// Includes all flows followed by a total summary line.
pub fn format_suite_toon(report: &SuiteReport) -> String {
    let mut out = String::new();

    // Schema header for LLM comprehension.
    // `total:` line is self-describing (appears once at end); `F=`, `B=`,
    // `R=` repeat per flow so their compact keys pay off the schema cost.
    out.push_str("# F=flow-run B=block R=result(passed/warned/failed) d:N=duration_ms os:N=os_major cov:a,b,c=covered_axes\n");
    out.push_str("# step: +=pass !=fail ~=warn -=skip @x,y=position b=bounds(x,y,w,h) s:N=scroll_attempts t:N/M=trees/nodes\n");
    out.push_str("# perf: P block:app:device:iteration mem=MB cpu=% thr=threads fd=file_descriptors disk=MB net_rx/tx=KB launch=ms\n");
    out.push_str("# rec: rec block:iteration path-to-recording.mp4\n");
    out.push_str("# install: I app:bundle:device R=ok/fail d:ms os:N (device = `{platform}/{name}`)\n");
    out.push_str("# time: T0:<unix-ms> suite-anchor (once); t0+:<delta-ms> per-line start offset from T0\n");

    // Suite-anchor timestamp. Only emitted when parseable. Offsets below
    // are computed relative to this; if the anchor is absent, offsets are
    // dropped too rather than rendering bogus values.
    let suite_t0_ms = report.started_at.as_deref().and_then(iso_to_unix_ms);
    if let Some(t0) = suite_t0_ms {
        let _ = writeln!(out, "T0:{t0}");
    }

    // Install results (one line per (device, bundle) attempted)
    for inst in &report.installs {
        let r = if inst.success { "ok" } else { "fail" };
        let t0_offset = format_t0_offset(inst.started_at.as_deref(), suite_t0_ms);
        let os_suffix = match inst.os_major {
            Some(os) => format!(" os:{os}"),
            None => String::new(),
        };
        let _ = writeln!(
            out,
            "I {}:{}:{} R:{} d:{}{os_suffix}{t0_offset}",
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
        let _ = write!(out, "{}", format_flow_toon_anchored(flow, suite_t0_ms));
        out.push('\n');
    }

    // Aggregate counts at the flow level
    let total_passed = report.flows.iter().filter(|f| f.is_passed()).count();
    let total_failed = report.flows.iter().filter(|f| f.is_failed()).count();
    let total_skipped = report
        .flows
        .iter()
        .filter(|f| {
            f.is_skipped()
                || (!f.step_results.is_empty()
                    && f.step_results
                        .iter()
                        .all(|s| matches!(s.outcome, StepOutcome::Skipped)))
        })
        .count();

    // Flake summary: per (flow, device) tally when --repeat > 1.
    // Empty for single-run suites (no `flake:` lines emitted).
    let flake = crate::flake::build_summary(&report.flows);
    if !flake.is_empty() {
        out.push_str("# flake: flake:passed/total flow (sorted flakiest-first)\n");
        for e in &flake {
            let _ = writeln!(out, "flake:{}/{} {}", e.passed, e.total, e.flow);
        }
    }

    let _ = writeln!(
        out,
        "total:{total_passed}×pass,{total_failed}×fail,{total_skipped}×skip d:{}",
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
            block_iteration: 0,
            step_index_in_block: 0,
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Success,
            duration_ms: ms,
            retry_count: 0,
            screenshot_path: None,
            substeps: vec![],
            tree_stats: golem_events::TreeStats::default(),
            started_at: None,
            finished_at: None,
        }
    }

    fn failed_step(action: &str, target: &str, ms: u64, msg: &str) -> StepReport {
        StepReport {
            global_step_index: 0,
            block_name: String::new(),
            block_iteration: 0,
            step_index_in_block: 0,
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Failed { message: msg.to_string(), code: golem_events::FailureCode::Uncoded },
            duration_ms: ms,
            retry_count: 0,
            screenshot_path: None,
            substeps: vec![],
            tree_stats: golem_events::TreeStats::default(),
            started_at: None,
            finished_at: None,
        }
    }

    fn warning_step(action: &str, target: &str, ms: u64, msg: &str) -> StepReport {
        StepReport {
            global_step_index: 0,
            block_name: String::new(),
            block_iteration: 0,
            step_index_in_block: 0,
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Warning { message: msg.to_string(), code: golem_events::FailureCode::Uncoded },
            duration_ms: ms,
            retry_count: 0,
            screenshot_path: None,
            substeps: vec![],
            tree_stats: golem_events::TreeStats::default(),
            started_at: None,
            finished_at: None,
        }
    }

    fn skipped_step(action: &str, target: &str) -> StepReport {
        StepReport {
            global_step_index: 0,
            block_name: String::new(),
            block_iteration: 0,
            step_index_in_block: 0,
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Skipped,
            duration_ms: 0,
            retry_count: 0,
            screenshot_path: None,
            substeps: vec![],
            tree_stats: golem_events::TreeStats::default(),
            started_at: None,
            finished_at: None,
        }
    }

    fn sample_flow(success: bool, seed: Option<u64>) -> FlowReport {
        FlowReport {
            first_failure_code: None,
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
            os_major: None,
            perf_snapshots: vec![],
            skipped_reason: None,
            covered_axes: Vec::new(),
            recordings: Vec::new(),
            repeat: None,
            started_at: None,
            finished_at: None,
        }
    }

    // 1. Step success format: ` +action:target duration` ---------------

    #[test]
    fn step_success_format() {
        let step = success_step("tap", "Sign Up", 45);
        let out = format_step_toon(&step);
        assert_eq!(out, " +tap:Sign Up d:45");
    }

    // 2. Step failure format: ` !action:target d:duration error` -------

    #[test]
    fn step_failure_format() {
        let step = failed_step("assert_visible", "Welcome", 10012, "timed out");
        let out = format_step_toon(&step);
        assert_eq!(out, " !assert_visible:Welcome d:10012 EX000 timed out");
    }

    #[test]
    fn step_failure_line_carries_code_token() {
        let mut step = failed_step("assert_visible", "Welcome", 10012, "timed out");
        step.outcome = StepOutcome::Failed {
            message: "timed out".to_string(),
            code: golem_events::FailureCode::FlowStepTimeout,
        };
        let out = format_step_toon(&step);
        assert_eq!(out, " !assert_visible:Welcome d:10012 EF408 timed out");
    }

    // 3. Step warning format: ` ~action:target d:duration message` -----

    #[test]
    fn step_warning_format() {
        let step = warning_step("assert_visible", "Promo", 15, "element not found");
        let out = format_step_toon(&step);
        assert_eq!(out, " ~assert_visible:Promo d:15 WX000 element not found");
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
        assert_eq!(first_line, "F:login_flow d:10200");
    }

    // 6. Flow header includes seed when present ------------------------

    #[test]
    fn flow_header_includes_seed() {
        let report = sample_flow(false, Some(847_291_036));
        let out = format_flow_toon(&report);
        let first_line = out.lines().next().expect("should have at least one line");
        assert_eq!(first_line, "F:login_flow d:10200 seed:847291036");
    }

    // Flow header includes covered_axes when populated ----------------

    #[test]
    fn flow_header_includes_covered_axes() {
        let mut report = sample_flow(true, None);
        report.covered_axes = vec!["ios".into(), "v26".into(), "tablet".into()];
        let out = format_flow_toon(&report);
        let first_line = out.lines().next().expect("header line");
        assert_eq!(first_line, "F:login_flow d:10200 cov:ios,v26,tablet");
    }

    #[test]
    fn flow_header_omits_covered_axes_when_empty() {
        let report = sample_flow(true, None);
        let out = format_flow_toon(&report);
        let first_line = out.lines().next().expect("header line");
        assert!(
            !first_line.contains(" cov:"),
            "SHALL NOT emit cov: when covered_axes empty; got: {first_line}"
        );
    }

    // 7. Flow result line shows PASS/FAIL with counts ------------------

    #[test]
    fn flow_result_line_pass_with_counts() {
        let report = FlowReport {
            first_failure_code: None,
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
            os_major: None,
            perf_snapshots: vec![],
            skipped_reason: None,
            covered_axes: Vec::new(),
            recordings: Vec::new(),
            repeat: None,
            started_at: None,
            finished_at: None,
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
                    first_failure_code: None,
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
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                    covered_axes: Vec::new(),
                    recordings: Vec::new(),
                    repeat: None,
                    started_at: None,
                    finished_at: None,
                },
                FlowReport {
                    first_failure_code: None,
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
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                    covered_axes: Vec::new(),
                    recordings: Vec::new(),
                    repeat: None,
                    started_at: None,
                    finished_at: None,
                },
            ],
            installs: Vec::new(),
            total_duration_ms: 45300,
            started_at: None,
            finished_at: None,
        };

        let out = format_suite_toon(&suite);
        let last_line = out.lines().last().expect("should have lines");
        assert_eq!(last_line, "total:1×pass,1×fail,0×skip d:45300");
    }

    // 9. Multiple flows in suite ----------------------------------------

    #[test]
    fn suite_multiple_flows() {
        let suite = SuiteReport {
            flows: vec![
                FlowReport {
                    first_failure_code: None,
                    flow_name: "flow_a".to_string(),
                    success: true,
                    step_results: vec![success_step("launch", "", 100)],
                    warnings: vec![],
                    duration_ms: 100,
                    seed: None,
                    screenshot_path: None,
                    device_name: None,
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                    covered_axes: Vec::new(),
                    recordings: Vec::new(),
                    repeat: None,
                    started_at: None,
                    finished_at: None,
                },
                FlowReport {
                    first_failure_code: None,
                    flow_name: "flow_b".to_string(),
                    success: true,
                    step_results: vec![success_step("launch", "", 200)],
                    warnings: vec![],
                    duration_ms: 200,
                    seed: Some(42),
                    screenshot_path: None,
                    device_name: None,
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                    covered_axes: Vec::new(),
                    recordings: Vec::new(),
                    repeat: None,
                    started_at: None,
                    finished_at: None,
                },
                FlowReport {
                    first_failure_code: None,
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
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                    covered_axes: Vec::new(),
                    recordings: Vec::new(),
                    repeat: None,
                    started_at: None,
                    finished_at: None,
                },
            ],
            installs: Vec::new(),
            total_duration_ms: 600,
            started_at: None,
            finished_at: None,
        };

        let out = format_suite_toon(&suite);

        // All three flow headers should appear in order
        let flow_a_pos = out.find("F:flow_a").expect("should contain flow_a");
        let flow_b_pos = out.find("F:flow_b").expect("should contain flow_b");
        let flow_c_pos = out.find("F:flow_c").expect("should contain flow_c");
        assert!(flow_a_pos < flow_b_pos, "flow_a before flow_b");
        assert!(flow_b_pos < flow_c_pos, "flow_b before flow_c");

        // flow_b has seed
        assert!(out.contains("F:flow_b d:200 seed:42"));

        // Result lines
        assert!(out.contains("R:PASS 1/0/0"));
        assert!(out.contains("R:FAIL 0/0/1"));

        // Total line: 2 passed, 1 failed, 0 skipped
        let last_line = out.lines().last().expect("should have lines");
        assert_eq!(last_line, "total:2×pass,1×fail,0×skip d:600");
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
        assert_eq!(out, " +launch d:120");
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
            first_failure_code: None,
            flow_name: "perf_flow".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 100)],
            warnings: vec![],
            duration_ms: 100,
            seed: None,
            screenshot_path: None,
            device_name: None,
            os_major: None,
            perf_snapshots: vec![sample_perf_snapshot()],
            skipped_reason: None,
            covered_axes: Vec::new(),
            recordings: Vec::new(),
            repeat: None,
            started_at: None,
            finished_at: None,
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
            first_failure_code: None,
            flow_name: "no_perf_flow".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 100)],
            warnings: vec![],
            duration_ms: 100,
            seed: None,
            screenshot_path: None,
            device_name: None,
            os_major: None,
            perf_snapshots: vec![],
            skipped_reason: None,
            covered_axes: Vec::new(),
            recordings: Vec::new(),
            repeat: None,
            started_at: None,
            finished_at: None,
        };

        let out = format_flow_toon(&report);
        assert!(
            !out.lines().any(|l| l.starts_with("P ")),
            "SHALL NOT contain P lines when no perf snapshots"
        );
    }

    // ── iso_to_unix_ms / format_t0_offset tests ─────────────────────

    // 1. Valid RFC-3339 timestamp parses to its epoch-millis value.
    #[test]
    fn iso_to_unix_ms_parses_valid_rfc3339() {
        // 1970-01-01T00:00:01Z is exactly 1000ms past the epoch.
        assert_eq!(
            iso_to_unix_ms("1970-01-01T00:00:01Z"),
            Some(1000),
            "valid RFC-3339 SHALL parse to epoch millis"
        );
    }

    // 2. Unparseable timestamp yields None (no bogus value).
    #[test]
    fn iso_to_unix_ms_rejects_garbage() {
        assert_eq!(
            iso_to_unix_ms("not-a-timestamp"),
            None,
            "garbage timestamp SHALL parse to None"
        );
    }

    // 3. Both line ts and anchor present → ` t0+:<delta>` with the difference.
    #[test]
    fn format_t0_offset_emits_delta_when_both_present() {
        // line at 5000ms, anchor at 1000ms → delta 4000.
        let out = format_t0_offset(Some("1970-01-01T00:00:05Z"), Some(1000));
        assert_eq!(out, " t0+:4000", "SHALL emit delta when line and anchor present");
    }

    // 4. Anchor missing → empty string (offset dropped, not bogus).
    #[test]
    fn format_t0_offset_empty_when_anchor_missing() {
        let out = format_t0_offset(Some("1970-01-01T00:00:05Z"), None);
        assert_eq!(out, "", "SHALL emit empty when anchor absent");
    }

    // 5. Line ts unparseable → empty string even if anchor present.
    #[test]
    fn format_t0_offset_empty_when_line_unparseable() {
        let out = format_t0_offset(Some("garbage"), Some(1000));
        assert_eq!(out, "", "SHALL emit empty when line ts unparseable");
    }

    // ── format_step_toon tree-stats / device / os tests ─────────────

    // 6. Success step with tree fetches appends ` t:fetches/max_nodes`.
    #[test]
    fn step_success_with_tree_stats_appends_t_suffix() {
        let mut step = success_step("tap", "OK", 45);
        step.tree_stats = golem_events::TreeStats { fetches: 3, max_nodes: 128, ..Default::default() };
        let out = format_step_toon(&step);
        assert_eq!(out, " +tap:OK d:45 t:3/128", "SHALL append t:fetches/max_nodes when fetches > 0");
    }

    // 7. Zero tree fetches omits the t: suffix entirely.
    #[test]
    fn step_success_zero_tree_fetches_omits_t_suffix() {
        let mut step = success_step("tap", "OK", 45);
        step.tree_stats = golem_events::TreeStats { fetches: 0, max_nodes: 200, ..Default::default() };
        let out = format_step_toon(&step);
        assert_eq!(out, " +tap:OK d:45", "SHALL omit t: suffix when fetches == 0");
    }

    // 8. Success step with substeps appends substep notation before tree suffix.
    #[test]
    fn step_success_substeps_precede_tree_suffix() {
        let mut step = success_step("tap", "OK", 45);
        step.substeps = vec![crate::SubstepDetail::Tap {
            point: golem_events::Point { x: 10, y: 20 },
            element_bounds: None,
        }];
        step.tree_stats = golem_events::TreeStats { fetches: 1, max_nodes: 50, ..Default::default() };
        let out = format_step_toon(&step);
        assert_eq!(out, " +tap:OK d:45 @10,20 t:1/50", "substeps SHALL precede tree suffix");
    }

    // 9. Warning step carries its substep suffix (but no tree suffix).
    #[test]
    fn step_warning_carries_substeps() {
        let mut step = warning_step("scroll", "Promo", 15, "element not found");
        step.substeps = vec![crate::SubstepDetail::DriverWarning {
            message: "slow".into(),
        }];
        let out = format_step_toon(&step);
        assert_eq!(
            out,
            " ~scroll:Promo d:15 WX000 element not found warn:\"slow\"",
            "warning step SHALL append substep notation"
        );
    }

    // ── format_flow_toon device / os / block-header tests ───────────

    // 10. Header emits dev: and os: when present, in order.
    #[test]
    fn flow_header_includes_device_and_os() {
        let mut report = sample_flow(true, None);
        report.device_name = Some("android/Pixel_7a".into());
        report.os_major = Some(34);
        let out = format_flow_toon(&report);
        let first = out.lines().next().expect("header line");
        assert_eq!(first, "F:login_flow dev:android/Pixel_7a os:34 d:10200");
    }

    // 11. Steps under a named block emit a ` B:<block>` header once.
    #[test]
    fn flow_block_header_emitted_for_named_block() {
        let mut report = sample_flow(true, None);
        let mut s = success_step("tap", "OK", 10);
        s.block_name = "checkout".into();
        report.step_results = vec![s];
        let out = format_flow_toon(&report);
        assert!(out.contains(" B:checkout\n"), "SHALL emit B:checkout header; got: {out}");
    }

    // 12. Block iteration > 0 emits ` B:<block> i:<n>` header.
    #[test]
    fn flow_block_header_with_iteration() {
        let mut report = sample_flow(true, None);
        let mut s = success_step("tap", "OK", 10);
        s.block_name = "loop".into();
        s.block_iteration = 2;
        report.step_results = vec![s];
        let out = format_flow_toon(&report);
        assert!(out.contains(" B:loop i:2\n"), "SHALL emit B:loop i:2 header; got: {out}");
    }

    // 13. A new header is emitted each time the (block, iteration) pair changes.
    #[test]
    fn flow_block_header_reemitted_on_iteration_change() {
        let mut report = sample_flow(true, None);
        let mut s0 = success_step("tap", "A", 10);
        s0.block_name = "loop".into();
        s0.block_iteration = 1;
        let mut s1 = success_step("tap", "B", 10);
        s1.block_name = "loop".into();
        s1.block_iteration = 2;
        report.step_results = vec![s0, s1];
        let out = format_flow_toon(&report);
        let headers: Vec<&str> = out.lines().filter(|l| l.starts_with(" B:")).collect();
        assert_eq!(headers, vec![" B:loop i:1", " B:loop i:2"], "SHALL re-emit header per iteration");
    }

    // 14. Empty block_name suppresses the B: header line.
    #[test]
    fn flow_empty_block_name_omits_header() {
        let report = sample_flow(true, None);
        let out = format_flow_toon(&report);
        assert!(
            !out.lines().any(|l| l.starts_with(" B:")),
            "SHALL NOT emit B: header when block_name empty"
        );
    }

    // ── flow result-line code / skip tests ──────────────────────────

    // 15. FAIL result line appends first_failure_code token when set.
    #[test]
    fn flow_result_fail_appends_failure_code() {
        let mut report = sample_flow(false, None);
        report.first_failure_code = Some(golem_events::FailureCode::FlowStepTimeout);
        let out = format_flow_toon(&report);
        let last = out.lines().last().expect("result line");
        assert_eq!(last, "R:FAIL 4/1/1 EF408", "FAIL line SHALL carry the failure code token");
    }

    // 16. PASS flow never appends a failure code even if one is somehow set.
    #[test]
    fn flow_result_pass_omits_failure_code() {
        let mut report = sample_flow(true, None);
        report.first_failure_code = Some(golem_events::FailureCode::FlowStepTimeout);
        let out = format_flow_toon(&report);
        let last = out.lines().last().expect("result line");
        assert!(!last.contains("EF408"), "PASS line SHALL NOT carry a failure code; got: {last}");
    }

    // 17. Skipped flow (success + reason) renders SKIP and appends the reason.
    #[test]
    fn flow_result_skip_appends_reason() {
        let mut report = sample_flow(true, None);
        report.skipped_reason = Some("covered by peer run".into());
        report.step_results = vec![];
        let out = format_flow_toon(&report);
        let last = out.lines().last().expect("result line");
        assert_eq!(last, "R:SKIP 0/0/0 covered by peer run", "SKIP line SHALL append the reason");
    }

    // ── recording-line tests ────────────────────────────────────────

    // 18. Recordings emit one ` rec block:iter path` line each.
    #[test]
    fn flow_emits_recording_lines() {
        let mut report = sample_flow(true, None);
        report.step_results = vec![success_step("launch", "", 100)];
        report.recordings = vec![
            crate::RecordingEntry { block: "checkout".into(), iteration: 0, path: "/tmp/a.mp4".into() },
            crate::RecordingEntry { block: "checkout".into(), iteration: 1, path: "/tmp/b.mp4".into() },
        ];
        let out = format_flow_toon(&report);
        assert!(out.contains(" rec checkout:0 /tmp/a.mp4\n"), "SHALL emit first recording line; got: {out}");
        assert!(out.contains(" rec checkout:1 /tmp/b.mp4\n"), "SHALL emit second recording line; got: {out}");
    }

    // ── suite T0 / install / flake tests ────────────────────────────

    fn empty_suite() -> SuiteReport {
        SuiteReport {
            flows: Vec::new(),
            installs: Vec::new(),
            total_duration_ms: 0,
            started_at: None,
            finished_at: None,
        }
    }

    // 19. Suite anchor timestamp emits a single T0: line when parseable.
    #[test]
    fn suite_emits_t0_when_started_at_parseable() {
        let mut suite = empty_suite();
        suite.started_at = Some("1970-01-01T00:00:01Z".into());
        let out = format_suite_toon(&suite);
        assert!(out.contains("\nT0:1000\n"), "SHALL emit T0:<ms> when anchor parseable; got: {out}");
    }

    // 20. Unparseable suite anchor omits the T0: line entirely.
    #[test]
    fn suite_omits_t0_when_started_at_unparseable() {
        let mut suite = empty_suite();
        suite.started_at = Some("nonsense".into());
        let out = format_suite_toon(&suite);
        assert!(
            !out.lines().any(|l| l.starts_with("T0:")),
            "SHALL NOT emit T0: when anchor unparseable"
        );
    }

    // 21. Install lines render device, bundle, ok/fail, duration, os, and error indent.
    #[test]
    fn suite_renders_install_lines_with_error_indent() {
        let mut suite = empty_suite();
        suite.installs = vec![crate::InstallReport {
            app_name: "myapp".into(),
            bundle_id: "com.example".into(),
            device_name: "android/emu".into(),
            os_major: Some(34),
            success: false,
            duration_ms: 4200,
            exit_code: Some(1),
            error: Some("line one\nline two".into()),
            code: None,
            started_at: None,
            finished_at: None,
        }];
        let out = format_suite_toon(&suite);
        assert!(
            out.contains("I myapp:com.example:android/emu R:fail d:4200 os:34\n"),
            "install line SHALL carry name/bundle/device/result/duration/os; got: {out}"
        );
        assert!(out.contains("  line one\n"), "SHALL indent error line one; got: {out}");
        assert!(out.contains("  line two\n"), "SHALL indent error line two; got: {out}");
    }

    // 22. Successful install renders R:ok and omits os: when os_major absent.
    #[test]
    fn suite_install_ok_omits_os_when_absent() {
        let mut suite = empty_suite();
        suite.installs = vec![crate::InstallReport {
            app_name: "app".into(),
            bundle_id: "b".into(),
            device_name: "ios/sim".into(),
            os_major: None,
            success: true,
            duration_ms: 900,
            exit_code: Some(0),
            error: None,
            code: None,
            started_at: None,
            finished_at: None,
        }];
        let out = format_suite_toon(&suite);
        let line = out.lines().find(|l| l.starts_with("I ")).expect("install line");
        assert_eq!(line, "I app:b:ios/sim R:ok d:900", "ok install SHALL omit os: when absent");
    }

    // 23. Suite schema header comment lines are always present.
    #[test]
    fn suite_emits_schema_header_comments() {
        let out = format_suite_toon(&empty_suite());
        assert!(out.starts_with("# F=flow-run"), "SHALL begin with the F= schema header; got: {out}");
        assert!(out.contains("# step:"), "SHALL include the step schema header");
        assert!(out.contains("# perf:"), "SHALL include the perf schema header");
        assert!(out.contains("# install:"), "SHALL include the install schema header");
        assert!(out.contains("# time:"), "SHALL include the time schema header");
    }

    // 24. A flow whose every step is Skipped counts toward the suite skip total.
    #[test]
    fn suite_total_counts_all_skipped_steps_flow_as_skip() {
        let mut suite = empty_suite();
        let mut flow = sample_flow(false, None);
        flow.success = false;
        flow.step_results = vec![skipped_step("tap", "A"), skipped_step("tap", "B")];
        suite.flows = vec![flow];
        suite.total_duration_ms = 10;
        let out = format_suite_toon(&suite);
        let last = out.lines().last().expect("total line");
        // is_failed (success=false) AND all-skipped → counted in both fail and skip tallies.
        assert_eq!(last, "total:0×pass,1×fail,1×skip d:10", "all-skipped-steps flow SHALL count as skip");
    }

    // 25. Flake summary emitted when any flow carries repeat context.
    #[test]
    fn suite_emits_flake_summary_when_repeated() {
        let mut suite = empty_suite();
        let mut pass = sample_flow(true, None);
        pass.flow_name = "f".into();
        pass.step_results = vec![success_step("launch", "", 1)];
        pass.repeat = Some(golem_events::RepeatContext { index: 0, total: 2 });
        let mut fail = sample_flow(false, None);
        fail.flow_name = "f".into();
        fail.step_results = vec![failed_step("tap", "x", 1, "boom")];
        fail.repeat = Some(golem_events::RepeatContext { index: 1, total: 2 });
        suite.flows = vec![pass, fail];
        suite.total_duration_ms = 2;
        let out = format_suite_toon(&suite);
        assert!(out.contains("# flake:"), "SHALL emit flake schema header; got: {out}");
        assert!(out.contains("flake:1/2 f"), "SHALL tally 1/2 for flow f; got: {out}");
    }

    // 26. No flake lines for a single-run suite (no repeat set).
    #[test]
    fn suite_omits_flake_summary_when_not_repeated() {
        let mut suite = empty_suite();
        let mut flow = sample_flow(true, None);
        flow.step_results = vec![success_step("launch", "", 1)];
        suite.flows = vec![flow];
        let out = format_suite_toon(&suite);
        assert!(
            !out.lines().any(|l| l.starts_with("flake:") || l == "# flake: flake:passed/total flow (sorted flakiest-first)"),
            "single-run suite SHALL NOT emit flake lines; got: {out}"
        );
    }

    // ── remaining substep-variant tests ─────────────────────────────

    // 27. AppStop renders stop:bundle.
    #[test]
    fn substeps_toon_app_stop() {
        let out = format_substeps_toon(&[crate::SubstepDetail::AppStop { bundle: "com.x".into() }]);
        assert_eq!(out, " stop:com.x", "AppStop SHALL render stop:bundle");
    }

    // 28. TextInput renders t:"text".
    #[test]
    fn substeps_toon_text_input() {
        let out = format_substeps_toon(&[crate::SubstepDetail::TextInput {
            text: "hello".into(),
            field_bounds: None,
        }]);
        assert_eq!(out, " t:\"hello\"", "TextInput SHALL render t:\"text\"");
    }

    // 29. Swipe renders (from)→(to).
    #[test]
    fn substeps_toon_swipe() {
        let out = format_substeps_toon(&[crate::SubstepDetail::Swipe {
            from: golem_events::Point { x: 1, y: 2 },
            to: golem_events::Point { x: 3, y: 4 },
        }]);
        assert_eq!(out, " (1,2)→(3,4)", "Swipe SHALL render (from)→(to)");
    }

    // 30. ElementNotFound renders !found Nms.
    #[test]
    fn substeps_toon_element_not_found() {
        let out = format_substeps_toon(&[crate::SubstepDetail::ElementNotFound {
            selector: "text=X".into(),
            timeout_ms: 5000,
        }]);
        assert_eq!(out, " !found 5000ms", "ElementNotFound SHALL render !found Nms");
    }

    // 31. ScrollStarted renders dir:<direction>.
    #[test]
    fn substeps_toon_scroll_started() {
        let out = format_substeps_toon(&[crate::SubstepDetail::ScrollStarted {
            selector: "text=X".into(),
            direction: "down".into(),
        }]);
        assert_eq!(out, " dir:down", "ScrollStarted SHALL render dir:<direction>");
    }

    // 32. ScrollAttempt renders #attempt dir (from)→(to) result.
    #[test]
    fn substeps_toon_scroll_attempt() {
        let out = format_substeps_toon(&[crate::SubstepDetail::ScrollAttempt {
            attempt: 2,
            direction: "down".into(),
            strategy_index: 0,
            container: false,
            from: golem_events::Point { x: 1, y: 2 },
            to: golem_events::Point { x: 3, y: 4 },
            result: "moved".into(),
            tree_stats: golem_events::TreeStats::default(),
        }]);
        assert_eq!(out, " #2 down (1,2)→(3,4) moved", "ScrollAttempt SHALL render #n dir (from)→(to) result");
    }

    // 33. ScrollDirectionReversed renders rev→<dir> <reason>.
    #[test]
    fn substeps_toon_scroll_direction_reversed() {
        let out = format_substeps_toon(&[crate::SubstepDetail::ScrollDirectionReversed {
            to_direction: "up".into(),
            reason: "edge".into(),
        }]);
        assert_eq!(out, " rev→up edge", "ScrollDirectionReversed SHALL render rev→<dir> <reason>");
    }

    // 34. ScrollStrategySwitch renders strat→<index+1> <reason> (1-based).
    #[test]
    fn substeps_toon_scroll_strategy_switch_is_one_based() {
        let out = format_substeps_toon(&[crate::SubstepDetail::ScrollStrategySwitch {
            to_index: 0,
            reason: "fallback".into(),
        }]);
        assert_eq!(out, " strat→1 fallback", "ScrollStrategySwitch SHALL render 1-based index");
    }

    // 35. RetryAttempt renders retry attempt/max: error.
    #[test]
    fn substeps_toon_retry_attempt() {
        let out = format_substeps_toon(&[crate::SubstepDetail::RetryAttempt {
            attempt: 1,
            max: 3,
            delay_ms: 100,
            error: "stale".into(),
        }]);
        assert_eq!(out, " retry 1/3: stale", "RetryAttempt SHALL render retry n/m: error");
    }

    // 36. HttpRequest with a status renders method→status Nms.
    #[test]
    fn substeps_toon_http_request_with_status() {
        let out = format_substeps_toon(&[crate::SubstepDetail::HttpRequest {
            method: "GET".into(),
            url: "https://x".into(),
            status: Some(200),
            duration_ms: 42,
        }]);
        assert_eq!(out, " GET→200 42ms", "HttpRequest with status SHALL render method→status Nms");
    }

    // 37. HttpRequest without a status renders ? in place of the code.
    #[test]
    fn substeps_toon_http_request_without_status() {
        let out = format_substeps_toon(&[crate::SubstepDetail::HttpRequest {
            method: "POST".into(),
            url: "https://x".into(),
            status: None,
            duration_ms: 7,
        }]);
        assert_eq!(out, " POST→? 7ms", "HttpRequest without status SHALL render ?");
    }

    // 38. BashCommand with an exit code renders bash:"cmd" exit=N Nms.
    #[test]
    fn substeps_toon_bash_command_with_exit() {
        let out = format_substeps_toon(&[crate::SubstepDetail::BashCommand {
            command: "ls".into(),
            exit_code: Some(0),
            duration_ms: 12,
        }]);
        assert_eq!(out, " bash:\"ls\" exit=0 12ms", "BashCommand SHALL render bash:\"cmd\" exit=N Nms");
    }

    // 39. BashCommand without an exit code renders exit=?.
    #[test]
    fn substeps_toon_bash_command_without_exit() {
        let out = format_substeps_toon(&[crate::SubstepDetail::BashCommand {
            command: "sleep".into(),
            exit_code: None,
            duration_ms: 99,
        }]);
        assert_eq!(out, " bash:\"sleep\" exit=? 99ms", "BashCommand without exit SHALL render exit=?");
    }

    // 40. An unhandled substep variant contributes nothing (empty result).
    #[test]
    fn substeps_toon_unhandled_variant_produces_empty() {
        let out = format_substeps_toon(&[crate::SubstepDetail::Backspace { count: 3 }]);
        assert_eq!(out, "", "unhandled substep variant SHALL contribute no notation");
    }
}
