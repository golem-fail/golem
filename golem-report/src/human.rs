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

/// Format a duration given in milliseconds into a human-friendly string.
///
/// * Under 1 000 ms → `"120ms"`
/// * 1 000 ms and above → `"10.2s"`
fn format_duration(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else {
        let secs = ms as f64 / 1_000.0;
        format!("{secs:.1}s")
    }
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
///   ✓ tap "Sign Up"                    [45ms]
///   ✗ assert_visible "Welcome"         [10012ms] (timed out)
/// ```
pub fn format_step(step: &StepReport) -> String {
    let (symbol, suffix) = match &step.outcome {
        StepOutcome::Success => (SYM_SUCCESS, String::new()),
        StepOutcome::Warning(msg) => (SYM_WARNING, format!("  ({msg})")),
        StepOutcome::Failed(msg) => (SYM_FAILED, format!("  ({msg})")),
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
            StepOutcome::Warning(_) => warned += 1,
            StepOutcome::Failed(_) => failed += 1,
            StepOutcome::Skipped => skipped += 1,
        }
    }

    let status_symbol = if report.success { SYM_SUCCESS } else { SYM_FAILED };
    let status_word = if report.success { "PASSED" } else { "FAILED" };

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
    let _ = writeln!(
        out,
        "{status_symbol} {status_word}  {}{counts_str}  [{timing}]",
        report.flow_name
    );

    // Metadata
    if let Some(seed) = report.seed {
        let _ = writeln!(out, "  Seed: {seed}");
    }
    if let Some(ref path) = report.screenshot_path {
        let _ = writeln!(out, "  Screenshot: {path}");
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
    let total_flows_passed = report.flows.iter().filter(|f| f.success).count();
    let total_flows_failed = report.flows.iter().filter(|f| !f.success).count();
    let total_flows_skipped = report
        .flows
        .iter()
        .filter(|f| f.step_results.iter().all(|s| matches!(s.outcome, StepOutcome::Skipped)))
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
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Success,
            duration_ms: ms,
        }
    }

    fn failed_step(action: &str, target: &str, ms: u64, msg: &str) -> StepReport {
        StepReport {
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Failed(msg.to_string()),
            duration_ms: ms,
        }
    }

    fn warning_step(action: &str, target: &str, ms: u64, msg: &str) -> StepReport {
        StepReport {
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Warning(msg.to_string()),
            duration_ms: ms,
        }
    }

    fn skipped_step(action: &str, target: &str) -> StepReport {
        StepReport {
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Skipped,
            duration_ms: 0,
        }
    }

    // 1. format_step success shows ✓ with timing ---------------------

    #[test]
    fn step_success_shows_check_and_timing() {
        let step = success_step("tap", "Sign Up", 45);
        let out = format_step(&step);
        assert!(out.contains(SYM_SUCCESS), "SHALL contain ✓");
        assert!(out.contains("[45ms]"), "SHALL contain timing");
        assert!(out.contains("tap"), "SHALL contain action");
        assert!(out.contains("\"Sign Up\""), "SHALL contain target");
    }

    // 2. format_step failed shows ✗ with error message ---------------

    #[test]
    fn step_failed_shows_cross_and_message() {
        let step = failed_step("assert_visible", "Welcome", 10012, "timed out");
        let out = format_step(&step);
        assert!(out.contains(SYM_FAILED), "SHALL contain ✗");
        assert!(out.contains("[10.0s]"), "SHALL format as seconds");
        assert!(out.contains("(timed out)"), "SHALL contain error message");
    }

    // 3. format_step warning shows ⚠ with message --------------------

    #[test]
    fn step_warning_shows_symbol_and_message() {
        let step = warning_step("assert_visible", "Promo", 15, "warning: element not found");
        let out = format_step(&step);
        assert!(out.contains(SYM_WARNING), "SHALL contain ⚠");
        assert!(out.contains("[15ms]"), "SHALL contain timing");
        assert!(
            out.contains("(warning: element not found)"),
            "should contain warning"
        );
    }

    // 4. format_flow shows all steps in order -------------------------

    #[test]
    fn flow_shows_steps_in_order() {
        let report = FlowReport {
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
            perf_snapshots: vec![],
        };

        let out = format_flow(&report);
        let lines: Vec<&str> = out.lines().collect();

        // Header line
        assert!(lines[0].contains("login_flow"), "first line SHALL name the flow");
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
            flow_name: "login_flow".to_string(),
            success: false,
            step_results: vec![
                success_step("launch", "", 120),
                failed_step("assert_visible", "Welcome", 10012, "timed out"),
            ],
            warnings: vec![],
            duration_ms: 10132,
            seed: Some(847_291_036),
            screenshot_path: Some(
                ".golem/screenshots/login_flow_main_step5_error.png".to_string(),
            ),
            device_name: None,
            perf_snapshots: vec![],
        };

        let out = format_flow(&report);
        assert!(out.contains("FAILED"), "SHALL say FAILED");
        assert!(out.contains("Seed: 847291036"), "SHALL show seed");
        assert!(
            out.contains("Screenshot: .golem/screenshots/login_flow_main_step5_error.png"),
            "should show screenshot path"
        );
    }

    // 6. format_flow shows summary counts -----------------------------

    #[test]
    fn flow_summary_counts() {
        let report = FlowReport {
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
            perf_snapshots: vec![],
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
                },
            ],
            total_duration_ms: 45300,
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
                flow_name: "quick_flow".to_string(),
                success: true,
                step_results: vec![success_step("launch", "", 100)],
                warnings: vec![],
                duration_ms: 100,
                seed: None,
                screenshot_path: None,
                device_name: None,
                perf_snapshots: vec![],
            }],
            total_duration_ms: 45300,
        };

        let out = format_suite(&suite);
        assert!(out.contains("[45.3s]"), "SHALL show total duration");
    }

    // 9. format_step skipped shows appropriate indicator ---------------

    #[test]
    fn step_skipped_shows_indicator() {
        let step = skipped_step("tap", "Cancel");
        let out = format_step(&step);
        assert!(out.contains(SYM_SKIPPED), "SHALL contain − symbol");
        assert!(out.contains("(skipped)"), "SHALL say skipped");
        assert!(out.contains("[0ms]"), "skipped steps show 0ms");
    }

    // 10. Empty flow report formats correctly -------------------------

    #[test]
    fn empty_flow_formats_correctly() {
        let report = FlowReport {
            flow_name: "empty_flow".to_string(),
            success: true,
            step_results: vec![],
            warnings: vec![],
            duration_ms: 0,
            seed: None,
            screenshot_path: None,
            device_name: None,
            perf_snapshots: vec![],
        };

        let out = format_flow(&report);
        assert!(out.contains("empty_flow"), "SHALL contain flow name");
        assert!(out.contains(SYM_FLOW), "SHALL have flow symbol");
        assert!(out.contains("PASSED"), "SHALL say PASSED");
        assert!(out.contains("[0ms]"), "SHALL show 0ms duration");
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
            flow_name: "perf_flow".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 100)],
            warnings: vec![],
            duration_ms: 100,
            seed: None,
            screenshot_path: None,
            device_name: None,
            perf_snapshots: vec![sample_perf_snapshot()],
        };

        let out = format_flow(&report);
        assert!(out.contains("Performance:"), "SHALL contain Performance: header");
        assert!(out.contains("login:iPhone_16:0"), "SHALL contain snapshot label");
        assert!(out.contains("mem: 142.5 MB"), "SHALL contain memory value");
        assert!(out.contains("cpu: 23.1%"), "SHALL contain cpu value");
        assert!(out.contains("launch: 1240ms"), "SHALL contain launch value");
    }

    #[test]
    fn perf_section_omitted_when_empty() {
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
        };

        let out = format_flow(&report);
        assert!(!out.contains("Performance:"), "SHALL NOT contain Performance: when no snapshots");
    }
}
