//! Human-readable terminal output formatter.
//!
//! Formats [`FlowReport`], [`StepReport`], and [`SuiteReport`] as coloured,
//! Unicode-decorated text suitable for terminal display.

use crate::{FlowReport, PerfSnapshot, StepOutcome, StepReport, SuiteReport};
use std::fmt::Write;

// ── Unicode symbols ──────────────────────────────────────────────────

const SYM_SUCCESS: &str = "\u{2713}"; // ✓
const SYM_FAILED: &str = "\u{2717}"; // ✗
const SYM_WARNING: &str = "\u{26A0}"; // ⚠
const SYM_SKIPPED: &str = "\u{2212}"; // −
const SYM_FLOW: &str = "\u{25B6}"; // ▶
const SEPARATOR: &str = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"; // 38 ─

// ── Helpers ──────────────────────────────────────────────────────────

/// Format a duration given in milliseconds as a fixed-width seconds field.
/// Matches the streaming reporter — `[   0.045s]`, `[  10.012s]` — so all
/// durations align in a single visual column.
fn format_duration(ms: u64) -> String {
    let secs = ms as f64 / 1_000.0;
    format!("{secs:>8.3}s")
}

/// Format a single perf snapshot as a compact one-line summary.
fn format_perf_snapshot(snap: &PerfSnapshot) -> String {
    let mut parts = Vec::new();

    // Pad label to 40 chars for alignment
    let label = format!("{:<40}", snap.label);

    if let Some(v) = snap.memory_mb {
        parts.push(format!("mem: {v:.1} MB"));
    }
    if let Some(v) = snap.cpu_percent {
        parts.push(format!("cpu: {v:.1}%"));
    }
    if let Some(v) = snap.threads {
        parts.push(format!("thr: {v}"));
    }
    if let Some(v) = snap.file_descriptors {
        parts.push(format!("fd: {v}"));
    }
    if let Some(v) = snap.disk_mb {
        parts.push(format!("disk: {v:.1} MB"));
    }
    if let (Some(rx), Some(tx)) = (snap.net_rx_kb, snap.net_tx_kb) {
        parts.push(format!("net: {rx:.0}/{tx:.0} KB"));
    }
    if let Some(v) = snap.launch_ms {
        parts.push(format!("launch: {v}ms"));
    }

    format!("{label}{}", parts.join("  "))
}

// ── Public API ───────────────────────────────────────────────────────

/// Format a single step as one line of human-readable text.
///
/// Example outputs:
/// ```text
///   ✓ tap "Sign Up"                    [   0.045s]
///   ✗ assert_visible "Welcome"         [  10.012s] (timed out)
/// ```
pub fn format_step(step: &StepReport) -> String {
    let (symbol, suffix) = match &step.outcome {
        StepOutcome::Success => (SYM_SUCCESS, String::new()),
        StepOutcome::Warning { message, code } => (
            SYM_WARNING,
            format!(
                "  {} ({message})",
                code.render(golem_events::Severity::Warning)
            ),
        ),
        StepOutcome::Failed { message, code } => (
            SYM_FAILED,
            format!(
                "  {} ({message})",
                code.render(golem_events::Severity::Error)
            ),
        ),
        StepOutcome::Skipped => (SYM_SKIPPED, "  (skipped)".to_string()),
    };

    let label = if step.target.is_empty() {
        step.action.clone()
    } else {
        format!("{} \"{}\"", step.action, step.target)
    };

    let timing = format!("[{}]", format_duration(step.duration_ms));

    // Pad the label so the timing column lines up at column 40.
    let pad_width = 36_usize.saturating_sub(label.len());
    let padding = " ".repeat(pad_width);

    format!("  {symbol} {label}{padding}{timing}{suffix}")
}

/// Format a complete flow report as human-readable text.
///
/// Includes the flow header, all steps, a summary line, and optional
/// metadata (seed, screenshot path).
/// The failure code to show on a flow's summary line, or `None` when the
/// flow didn't fail. Prefers the report's `first_failure_code`; falls back
/// to the first failed step's code. Pure helper so the FAIL-line code
/// rendering can be unit-tested and stays in parity with the live stream
/// renderer (which also surfaces the code on the flow-finished line).
fn flow_fail_code(report: &FlowReport) -> Option<golem_events::FailureCode> {
    if report.success || report.is_skipped() {
        return None;
    }
    report.first_failure_code.or_else(|| {
        report.step_results.iter().find_map(|s| match &s.outcome {
            StepOutcome::Failed { code, .. } => Some(*code),
            _ => None,
        })
    })
}

pub fn format_flow(report: &FlowReport) -> String {
    let mut out = String::new();

    // Header
    let _ = writeln!(out, "{SYM_FLOW} {}", report.flow_name);

    // Steps
    for step in &report.step_results {
        let _ = writeln!(out, "{}", format_step(step));
    }

    // Performance snapshots
    if !report.perf_snapshots.is_empty() {
        let _ = writeln!(out, "  Performance:");
        for snap in &report.perf_snapshots {
            let _ = writeln!(out, "    {}", format_perf_snapshot(snap));
        }
    }

    // Accessibility audits
    if !report.a11y_audits.is_empty() {
        let _ = writeln!(out, "  Accessibility:");
        for audit in &report.a11y_audits {
            let errors = audit.error_count();
            let warnings = audit.warning_count();
            if audit.issues.is_empty() {
                let _ = writeln!(out, "    {:<32} clean", audit.label);
                continue;
            }
            let summary = format!("{errors} error(s), {warnings} warning(s)");
            match &audit.screenshot_path {
                Some(p) => {
                    let _ = writeln!(out, "    {:<32} {summary}  → {p}", audit.label);
                }
                None => {
                    let _ = writeln!(out, "    {:<32} {summary}", audit.label);
                }
            }
            for (i, issue) in audit.issues.iter().enumerate() {
                let tag = match issue.severity {
                    golem_events::Severity::Error => "ERR",
                    golem_events::Severity::Warning => "WRN",
                };
                // Surface confidence for heuristic findings (< 1.0); deterministic
                // checks are certain and need no annotation.
                let conf = if issue.is_heuristic() {
                    format!("  (confidence {:.2})", issue.confidence)
                } else {
                    String::new()
                };
                let _ = writeln!(
                    out,
                    "      [{:>2}] [{tag}] {:<24} {}{conf}",
                    i + 1,
                    issue.check_id,
                    issue.message
                );
            }
        }
    }

    // Blank line before summary
    let _ = writeln!(out);

    // Counts
    let mut passed: usize = 0;
    let mut warned: usize = 0;
    let mut failed: usize = 0;
    let mut skipped: usize = 0;
    for step in &report.step_results {
        match &step.outcome {
            StepOutcome::Success => passed += 1,
            StepOutcome::Warning { .. } => warned += 1,
            StepOutcome::Failed { .. } => failed += 1,
            StepOutcome::Skipped => skipped += 1,
        }
    }

    let (status_symbol, status_word) = if report.is_skipped() {
        (SYM_SKIPPED, "SKIPPED")
    } else if report.success {
        (SYM_SUCCESS, "PASSED")
    } else {
        (SYM_FAILED, "FAILED")
    };

    let mut counts = Vec::new();
    if passed > 0 {
        counts.push(format!("{passed} passed"));
    }
    if warned > 0 {
        counts.push(format!("{warned} warning"));
    }
    if failed > 0 {
        counts.push(format!("{failed} failed"));
    }
    if skipped > 0 {
        counts.push(format!("{skipped} skipped"));
    }

    let counts_str = if counts.is_empty() {
        String::new()
    } else {
        format!("  ({})", counts.join(", "))
    };

    let timing = format_duration(report.duration_ms);
    // Surface the flow-level failure code on the summary line (parity
    // with the live stream renderer), e.g. `✗ FAILED  login  [10.1s]  EF408`.
    let code_suffix = match flow_fail_code(report) {
        Some(code) => format!("  {}", code.render(golem_events::Severity::Error)),
        None => String::new(),
    };
    let _ = writeln!(
        out,
        "{status_symbol} {status_word}  {}{counts_str}  [{timing}]{code_suffix}",
        report.flow_name
    );

    // Metadata
    if let Some(ref reason) = report.skipped_reason {
        let _ = writeln!(out, "  Skipped: {reason}");
    }
    if let Some(seed) = report.seed {
        let _ = writeln!(out, "  Seed: {seed}");
    }
    if let Some(ref path) = report.screenshot_path {
        let _ = writeln!(out, "  Screenshot: {path}");
    }
    if !report.covered_axes.is_empty() {
        let _ = writeln!(out, "  Covered: {}", report.covered_axes.join(", "));
    }
    for rec in &report.recordings {
        let _ = writeln!(
            out,
            "  Recording: {} (block {}, iter {})",
            rec.path, rec.block, rec.iteration
        );
    }

    out
}

/// Format an entire suite report as human-readable text.
///
/// Includes each flow followed by a separator and an aggregate summary.
pub fn format_suite(report: &SuiteReport) -> String {
    let mut out = String::new();

    for flow in &report.flows {
        let _ = write!(out, "{}", format_flow(flow));
        let _ = writeln!(out);
    }

    // Separator
    let _ = writeln!(out, "{SEPARATOR}");

    // Aggregate counts at the flow level
    let total_flows_passed = report.flows.iter().filter(|f| f.is_passed()).count();
    let total_flows_failed = report.flows.iter().filter(|f| f.is_failed()).count();
    let total_flows_skipped = report
        .flows
        .iter()
        .filter(|f| {
            // 1. Genuine coverage-group skip (success=true + skip reason), OR
            // 2. Every step skipped — but only when the flow did NOT pass, so a
            //    passing all-skipped flow stays counted once (as passed), never double-counted.
            f.is_skipped()
                || (!f.is_passed()
                    && !f.step_results.is_empty()
                    && f.step_results
                        .iter()
                        .all(|s| matches!(s.outcome, StepOutcome::Skipped)))
        })
        .count();
    let timing = format_duration(report.total_duration_ms);

    let _ = writeln!(
        out,
        "Suite: {total_flows_passed} passed, {total_flows_failed} failed, {total_flows_skipped} skipped  [{timing}]"
    );

    out
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
            outcome: StepOutcome::Failed {
                message: msg.to_string(),
                code: golem_events::FailureCode::Uncoded,
            },
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
            outcome: StepOutcome::Warning {
                message: msg.to_string(),
                code: golem_events::FailureCode::Uncoded,
            },
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

    // 1. format_step success shows ✓ with timing ---------------------

    #[test]
    fn step_success_shows_check_and_timing() {
        let step = success_step("tap", "Sign Up", 45);
        let out = format_step(&step);
        assert!(out.contains(SYM_SUCCESS), "SHALL contain ✓");
        assert!(out.contains("0.045s]"), "SHALL contain timing");
        assert!(out.contains("tap"), "SHALL contain action");
        assert!(out.contains("\"Sign Up\""), "SHALL contain target");
    }

    // 2. format_step failed shows ✗ with error message ---------------

    #[test]
    fn step_failed_shows_cross_and_message() {
        let step = failed_step("assert_visible", "Welcome", 10012, "timed out");
        let out = format_step(&step);
        assert!(out.contains(SYM_FAILED), "SHALL contain ✗");
        assert!(out.contains("10.012s]"), "SHALL format as seconds");
        assert!(out.contains("(timed out)"), "SHALL contain error message");
    }

    #[test]
    fn step_failed_includes_failure_code_token() {
        let mut step = failed_step("assert_visible", "Welcome", 10012, "timed out");
        step.outcome = StepOutcome::Failed {
            message: "timed out".to_string(),
            code: golem_events::FailureCode::FlowStepTimeout,
        };
        let out = format_step(&step);
        assert!(
            out.contains("EF408"),
            "SHALL surface the failure code token"
        );
        assert!(out.contains("(timed out)"), "SHALL keep the message");
    }

    // 3. format_step warning shows ⚠ with message --------------------

    #[test]
    fn step_warning_shows_symbol_and_message() {
        let step = warning_step("assert_visible", "Promo", 15, "warning: element not found");
        let out = format_step(&step);
        assert!(out.contains(SYM_WARNING), "SHALL contain ⚠");
        assert!(out.contains("0.015s]"), "SHALL contain timing");
        assert!(
            out.contains("(warning: element not found)"),
            "should contain warning"
        );
    }

    // 4. format_flow shows all steps in order -------------------------

    #[test]
    fn flow_shows_steps_in_order() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "login_flow".to_string(),
            success: true,
            step_results: vec![
                success_step("launch", "", 120),
                success_step("tap", "Sign Up", 45),
                success_step("type", "email", 32),
            ],
            warnings: vec![],
            duration_ms: 197,
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

        let out = format_flow(&report);
        let lines: Vec<&str> = out.lines().collect();

        // Header line
        assert!(
            lines[0].contains("login_flow"),
            "first line SHALL name the flow"
        );
        assert!(lines[0].contains(SYM_FLOW), "first line SHALL have ▶");

        // Steps appear in order: launch before tap before type
        let launch_pos = out.find("launch").expect("should contain launch");
        let tap_pos = out.find("tap").expect("should contain tap");
        let type_pos = out.find("type").expect("should contain type");
        assert!(launch_pos < tap_pos, "launch before tap");
        assert!(tap_pos < type_pos, "tap before type");
    }

    // 5. format_flow failed shows seed and screenshot -----------------

    #[test]
    fn flow_failed_shows_seed_and_screenshot() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "login_flow".to_string(),
            success: false,
            step_results: vec![
                success_step("launch", "", 120),
                failed_step("assert_visible", "Welcome", 10012, "timed out"),
            ],
            warnings: vec![],
            duration_ms: 10132,
            seed: Some(847_291_036),
            screenshot_path: Some(".golem/screenshots/login_flow_main_step5_error.png".to_string()),
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

        let out = format_flow(&report);
        assert!(out.contains("FAILED"), "SHALL say FAILED");
        assert!(out.contains("Seed: 847291036"), "SHALL show seed");
        assert!(
            out.contains("Screenshot: .golem/screenshots/login_flow_main_step5_error.png"),
            "should show screenshot path"
        );
    }

    // 5b. format_flow surfaces the flow-level failure code -------------

    fn make_failed_flow(
        first_failure_code: Option<golem_events::FailureCode>,
        step_results: Vec<StepReport>,
    ) -> FlowReport {
        FlowReport {
            first_failure_code,
            a11y_audits: vec![],
            flow_name: "checkout".to_string(),
            success: false,
            step_results,
            warnings: vec![],
            duration_ms: 4200,
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
        }
    }

    fn failed_step_coded(action: &str, msg: &str, code: golem_events::FailureCode) -> StepReport {
        let mut s = failed_step(action, "", 100, msg);
        s.outcome = StepOutcome::Failed {
            message: msg.to_string(),
            code,
        };
        s
    }

    #[test]
    fn fail_line_uses_first_failure_code() {
        let report = make_failed_flow(
            Some(golem_events::FailureCode::FlowStepTimeout),
            vec![failed_step("assert_visible", "Welcome", 3000, "timed out")],
        );
        assert_eq!(
            flow_fail_code(&report),
            Some(golem_events::FailureCode::FlowStepTimeout),
            "SHALL prefer the report's first_failure_code"
        );
        let out = format_flow(&report);
        let summary = out
            .lines()
            .find(|l| l.contains("FAILED"))
            .expect("FAILED line");
        assert!(
            summary.contains("EF408"),
            "FAILED summary line SHALL carry the code, got: {summary}"
        );
    }

    #[test]
    fn fail_line_falls_back_to_failed_step_code() {
        // No flow-level code recorded — derive from the first failed step.
        let report = make_failed_flow(
            None,
            vec![
                success_step("launch", "", 50),
                failed_step_coded(
                    "tap",
                    "not found",
                    golem_events::FailureCode::FlowElementNotFound,
                ),
            ],
        );
        assert_eq!(
            flow_fail_code(&report),
            Some(golem_events::FailureCode::FlowElementNotFound)
        );
        assert!(
            format_flow(&report).contains("EF404"),
            "SHALL fall back to the first failed step's code"
        );
    }

    #[test]
    fn fail_line_has_no_code_on_success() {
        let report = FlowReport {
            success: true,
            ..make_failed_flow(
                Some(golem_events::FailureCode::FlowStepTimeout),
                vec![success_step("tap", "OK", 30)],
            )
        };
        assert_eq!(
            flow_fail_code(&report),
            None,
            "passing flow SHALL have no code"
        );
        let out = format_flow(&report);
        let summary = out
            .lines()
            .find(|l| l.contains("PASSED"))
            .expect("PASSED line");
        assert!(
            !summary.contains("EF408") && !summary.contains("EX000"),
            "PASSED summary line SHALL NOT carry a failure code, got: {summary}"
        );
    }

    // 6. format_flow shows summary counts -----------------------------

    #[test]
    fn flow_summary_counts() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "login_flow".to_string(),
            success: false,
            step_results: vec![
                success_step("launch", "", 120),
                success_step("tap", "Sign Up", 45),
                success_step("type", "email", 32),
                warning_step("assert_visible", "Promo", 15, "element not found"),
                success_step("tap", "Submit", 38),
                failed_step("assert_visible", "Welcome", 10012, "timed out"),
            ],
            warnings: vec![],
            duration_ms: 10262,
            seed: Some(847_291_036),
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

        let out = format_flow(&report);
        // Should include counts
        assert!(out.contains("4 passed"), "SHALL count 4 passed");
        assert!(out.contains("1 warning"), "SHALL count 1 warning");
        assert!(out.contains("1 failed"), "SHALL count 1 failed");
    }

    // 7. format_suite shows aggregate counts --------------------------

    #[test]
    fn suite_aggregate_counts() {
        let suite = SuiteReport {
            flows: vec![
                FlowReport {
                    first_failure_code: None,
                    a11y_audits: vec![],
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
                    a11y_audits: vec![],
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

        let out = format_suite(&suite);
        assert!(out.contains("1 passed"), "1 flow passed");
        assert!(out.contains("1 failed"), "1 flow failed");
        assert!(out.contains("0 skipped"), "0 flows skipped");
    }

    // 8. format_suite shows total duration ----------------------------

    #[test]
    fn suite_total_duration() {
        let suite = SuiteReport {
            flows: vec![FlowReport {
                first_failure_code: None,
                a11y_audits: vec![],
                flow_name: "quick_flow".to_string(),
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
            }],
            installs: Vec::new(),
            total_duration_ms: 45300,
            started_at: None,
            finished_at: None,
        };

        let out = format_suite(&suite);
        assert!(out.contains("45.300s]"), "SHALL show total duration");
    }

    // 9. format_step skipped shows appropriate indicator ---------------

    #[test]
    fn step_skipped_shows_indicator() {
        let step = skipped_step("tap", "Cancel");
        let out = format_step(&step);
        assert!(out.contains(SYM_SKIPPED), "SHALL contain − symbol");
        assert!(out.contains("(skipped)"), "SHALL say skipped");
        assert!(out.contains("0.000s]"), "skipped steps show 0ms");
    }

    // 10. Empty flow report formats correctly -------------------------

    #[test]
    fn empty_flow_formats_correctly() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "empty_flow".to_string(),
            success: true,
            step_results: vec![],
            warnings: vec![],
            duration_ms: 0,
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

        let out = format_flow(&report);
        assert!(out.contains("empty_flow"), "SHALL contain flow name");
        assert!(out.contains(SYM_FLOW), "SHALL have flow symbol");
        assert!(out.contains("PASSED"), "SHALL say PASSED");
        assert!(out.contains("0.000s]"), "SHALL show 0ms duration");
        // No seed or screenshot lines
        assert!(!out.contains("Seed:"), "no seed line");
        assert!(!out.contains("Screenshot:"), "no screenshot line");
    }

    // ── Perf rendering tests ────────────────────────────────────────

    fn sample_perf_snapshot() -> PerfSnapshot {
        PerfSnapshot {
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
    fn perf_section_renders_when_snapshots_present() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
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

        let out = format_flow(&report);
        assert!(
            out.contains("Performance:"),
            "SHALL contain Performance: header"
        );
        assert!(
            out.contains("login:iPhone_16:0"),
            "SHALL contain snapshot label"
        );
        assert!(out.contains("mem: 142.5 MB"), "SHALL contain memory value");
        assert!(out.contains("cpu: 23.1%"), "SHALL contain cpu value");
        assert!(out.contains("launch: 1240ms"), "SHALL contain launch value");
    }

    // 11. format_step with an empty target uses the bare action (no quotes)

    #[test]
    fn step_empty_target_uses_bare_action() {
        let step = success_step("launch", "", 120);
        let out = format_step(&step);
        assert!(out.contains("launch"), "SHALL contain action");
        assert!(!out.contains('"'), "empty target SHALL NOT add quote chars");
    }

    // 12. format_step with a long label clamps padding to zero (no panic,
    //     timing still present right after the label)

    #[test]
    fn step_long_label_clamps_padding() {
        let long_target = "X".repeat(80);
        let step = success_step("tap", &long_target, 45);
        let out = format_step(&step);
        // saturating_sub means no panic and zero padding; timing still appears.
        assert!(out.contains(&long_target), "SHALL contain the long target");
        assert!(out.contains("0.045s]"), "SHALL still render the timing");
        // The bracket should sit immediately after the label (no spaces between).
        let label = format!("tap \"{long_target}\"");
        assert!(
            out.contains(&format!("{label}[")),
            "long label SHALL have no padding before timing"
        );
    }

    // 13. format_flow skipped flow shows SKIPPED status word and reason line

    #[test]
    fn flow_skipped_shows_status_and_reason() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "covered_flow".to_string(),
            success: true,
            step_results: vec![],
            warnings: vec![],
            duration_ms: 0,
            seed: None,
            screenshot_path: None,
            device_name: None,
            os_major: None,
            perf_snapshots: vec![],
            skipped_reason: Some("peer run met coverage goal".to_string()),
            covered_axes: Vec::new(),
            recordings: Vec::new(),
            repeat: None,
            started_at: None,
            finished_at: None,
        };

        let out = format_flow(&report);
        assert!(out.contains("SKIPPED"), "SHALL say SKIPPED");
        assert!(out.contains(SYM_SKIPPED), "SHALL use the skipped symbol");
        assert!(
            out.contains("Skipped: peer run met coverage goal"),
            "SHALL show the skipped reason"
        );
        assert!(
            !out.contains("PASSED"),
            "SHALL NOT say PASSED for a skipped flow"
        );
        // No step counts → no trailing parenthesised counts on the summary line.
        assert!(!out.contains("passed"), "no step counts SHALL be rendered");
    }

    // 14. format_flow renders covered axes and recording entries

    #[test]
    fn flow_shows_covered_axes_and_recordings() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "matrix_flow".to_string(),
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
            covered_axes: vec!["os:android".to_string(), "locale:en".to_string()],
            recordings: vec![crate::RecordingEntry {
                block: "main".to_string(),
                iteration: 2,
                path: ".golem/rec/main_2.mp4".to_string(),
            }],
            repeat: None,
            started_at: None,
            finished_at: None,
        };

        let out = format_flow(&report);
        assert!(
            out.contains("Covered: os:android, locale:en"),
            "SHALL join covered axes"
        );
        assert!(
            out.contains("Recording: .golem/rec/main_2.mp4 (block main, iter 2)"),
            "SHALL render the recording entry with block and iteration"
        );
    }

    // 15. format_flow counts skipped steps and renders the skipped count

    #[test]
    fn flow_counts_skipped_steps() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "partial_flow".to_string(),
            success: true,
            step_results: vec![
                success_step("launch", "", 100),
                skipped_step("tap", "Cancel"),
                skipped_step("tap", "Dismiss"),
            ],
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

        let out = format_flow(&report);
        assert!(out.contains("1 passed"), "SHALL count 1 passed");
        assert!(out.contains("2 skipped"), "SHALL count 2 skipped steps");
    }

    // 16. format_perf_snapshot omits fields whose values are None

    #[test]
    fn perf_snapshot_omits_none_fields() {
        let snap = PerfSnapshot {
            label: "sparse".into(),
            memory_mb: Some(10.0),
            cpu_percent: None,
            threads: None,
            file_descriptors: None,
            disk_mb: None,
            net_rx_kb: None,
            net_tx_kb: None,
            launch_ms: None,
            timestamp: "0".into(),
        };
        let out = format_perf_snapshot(&snap);
        assert!(
            out.contains("mem: 10.0 MB"),
            "SHALL render present memory field"
        );
        assert!(!out.contains("cpu:"), "SHALL omit absent cpu field");
        assert!(!out.contains("thr:"), "SHALL omit absent threads field");
        assert!(!out.contains("net:"), "SHALL omit absent net field");
        assert!(!out.contains("launch:"), "SHALL omit absent launch field");
    }

    // 17. format_perf_snapshot only renders net when BOTH rx and tx are Some

    #[test]
    fn perf_snapshot_net_requires_both_directions() {
        let snap = PerfSnapshot {
            label: "half_net".into(),
            memory_mb: None,
            cpu_percent: None,
            threads: None,
            file_descriptors: None,
            disk_mb: None,
            net_rx_kb: Some(100.0),
            net_tx_kb: None,
            launch_ms: None,
            timestamp: "0".into(),
        };
        let out = format_perf_snapshot(&snap);
        assert!(!out.contains("net:"), "net SHALL require both rx and tx");
    }

    // 18. format_perf_snapshot renders threads, fd, disk, and net when present

    #[test]
    fn perf_snapshot_renders_secondary_fields() {
        let snap = PerfSnapshot {
            label: "full".into(),
            memory_mb: None,
            cpu_percent: None,
            threads: Some(7),
            file_descriptors: Some(13),
            disk_mb: Some(5.5),
            net_rx_kb: Some(200.0),
            net_tx_kb: Some(50.0),
            launch_ms: None,
            timestamp: "0".into(),
        };
        let out = format_perf_snapshot(&snap);
        assert!(out.contains("thr: 7"), "SHALL render threads");
        assert!(out.contains("fd: 13"), "SHALL render file descriptors");
        assert!(out.contains("disk: 5.5 MB"), "SHALL render disk");
        assert!(
            out.contains("net: 200/50 KB"),
            "SHALL render net with both directions"
        );
    }

    // 19. format_suite separator line is present between flows and summary

    #[test]
    fn suite_renders_separator() {
        let suite = SuiteReport {
            flows: vec![],
            installs: Vec::new(),
            total_duration_ms: 0,
            started_at: None,
            finished_at: None,
        };
        let out = format_suite(&suite);
        assert!(out.contains(SEPARATOR), "SHALL render the separator line");
        assert!(
            out.contains("Suite: 0 passed, 0 failed, 0 skipped"),
            "empty suite SHALL report all zero counts"
        );
    }

    // 20. format_suite classifies a coverage-group skip as skipped, not passed

    #[test]
    fn suite_counts_coverage_skip_as_skipped() {
        let suite = SuiteReport {
            flows: vec![FlowReport {
                first_failure_code: None,
                a11y_audits: vec![],
                flow_name: "skipped_flow".to_string(),
                success: true,
                step_results: vec![],
                warnings: vec![],
                duration_ms: 0,
                seed: None,
                screenshot_path: None,
                device_name: None,
                os_major: None,
                perf_snapshots: vec![],
                skipped_reason: Some("peer met goal".to_string()),
                covered_axes: Vec::new(),
                recordings: Vec::new(),
                repeat: None,
                started_at: None,
                finished_at: None,
            }],
            installs: Vec::new(),
            total_duration_ms: 0,
            started_at: None,
            finished_at: None,
        };
        let out = format_suite(&suite);
        // is_skipped() flow is skipped, not passed and not failed.
        assert!(
            out.contains("0 passed, 0 failed, 1 skipped"),
            "SHALL classify as skipped"
        );
    }

    // 21. format_suite counts a passing all-skipped-steps flow as passed only, never as skipped

    #[test]
    fn suite_counts_all_skipped_steps_flow_as_passed_only() {
        let suite = SuiteReport {
            flows: vec![FlowReport {
                first_failure_code: None,
                a11y_audits: vec![],
                flow_name: "all_skip_steps".to_string(),
                success: true,
                step_results: vec![skipped_step("tap", "A"), skipped_step("tap", "B")],
                warnings: vec![],
                duration_ms: 0,
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
            }],
            installs: Vec::new(),
            total_duration_ms: 0,
            started_at: None,
            finished_at: None,
        };
        let out = format_suite(&suite);
        // success=true + no skipped_reason → is_passed() is true; the all-steps-skipped
        // branch is now suppressed for passing flows, so it tallies once (passed), not twice.
        assert!(
            out.contains("1 passed, 0 failed, 0 skipped"),
            "passing all-skipped-steps flow SHALL count as passed only, never as skipped"
        );
    }

    // 21b. counts never over-tally a passing all-skipped-steps flow (regression for double-count)

    #[test]
    fn suite_passing_all_skipped_flow_counts_within_total() {
        let suite = SuiteReport {
            flows: vec![FlowReport {
                first_failure_code: None,
                a11y_audits: vec![],
                flow_name: "all_skip_steps".to_string(),
                success: true,
                step_results: vec![skipped_step("tap", "A"), skipped_step("tap", "B")],
                warnings: vec![],
                duration_ms: 0,
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
            }],
            installs: Vec::new(),
            total_duration_ms: 0,
            started_at: None,
            finished_at: None,
        };

        // 1. Recompute the three suite tallies exactly as format_suite does.
        let passed = suite.flows.iter().filter(|f| f.is_passed()).count();
        let failed = suite.flows.iter().filter(|f| f.is_failed()).count();
        let skipped = suite
            .flows
            .iter()
            .filter(|f| {
                f.is_skipped()
                    || (!f.is_passed()
                        && !f.step_results.is_empty()
                        && f.step_results
                            .iter()
                            .all(|s| matches!(s.outcome, StepOutcome::Skipped)))
            })
            .count();

        // 2. The three categories SHALL be mutually exclusive: their sum never exceeds the flow count.
        assert!(
            passed + failed + skipped <= suite.flows.len(),
            "passed+failed+skipped SHALL NOT exceed total flow count (no double-counting)"
        );
        assert_eq!(passed, 1, "the flow SHALL count once as passed");
        assert_eq!(skipped, 0, "the flow SHALL NOT also count as skipped");
    }

    // 22. format_duration pads narrow values and renders three decimals

    #[test]
    fn duration_is_fixed_width_three_decimals() {
        assert_eq!(
            format_duration(45),
            "   0.045s",
            "SHALL right-pad to width 8 with 3 decimals"
        );
        assert_eq!(
            format_duration(0),
            "   0.000s",
            "zero SHALL render as 0.000s"
        );
        assert_eq!(
            format_duration(10012),
            "  10.012s",
            "SHALL render seconds with padding"
        );
    }

    #[test]
    fn perf_section_omitted_when_empty() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
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

        let out = format_flow(&report);
        assert!(
            !out.contains("Performance:"),
            "SHALL NOT contain Performance: when no snapshots"
        );
    }
}
