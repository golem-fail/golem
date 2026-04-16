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

    match &step.outcome {
        StepOutcome::Success => {
            format!(" +{label} {}", step.duration_ms)
        }
        StepOutcome::Warning(msg) => {
            format!(" ~{label} {} {msg}", step.duration_ms)
        }
        StepOutcome::Failed(msg) => {
            format!(" !{label} {} {msg}", step.duration_ms)
        }
        StepOutcome::Skipped => {
            format!(" -{label}")
        }
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
            let _ = write!(out, " m:{v:.1}");
        }
        if let Some(v) = snap.cpu_percent {
            let _ = write!(out, " c:{v:.1}");
        }
        if let Some(v) = snap.threads {
            let _ = write!(out, " t:{v}");
        }
        if let Some(v) = snap.file_descriptors {
            let _ = write!(out, " f:{v}");
        }
        if let Some(v) = snap.disk_mb {
            let _ = write!(out, " d:{v:.1}");
        }
        if let Some(v) = snap.net_rx_kb {
            let _ = write!(out, " nr:{v:.0}");
        }
        if let Some(v) = snap.net_tx_kb {
            let _ = write!(out, " nt:{v:.0}");
        }
        if let Some(v) = snap.launch_ms {
            let _ = write!(out, " l:{v}");
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
                },
            ],
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
        };

        let out = format_flow_toon(&report);
        let perf_line = out.lines().find(|l| l.starts_with("P ")).expect("SHALL contain a P line");
        assert!(perf_line.contains("login:iPhone_16:0"), "SHALL contain snapshot label");
        assert!(perf_line.contains("m:142.5"), "SHALL contain memory value");
        assert!(perf_line.contains("c:23.1"), "SHALL contain cpu value");
        assert!(perf_line.contains("l:1240"), "SHALL contain launch value");
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
        };

        let out = format_flow_toon(&report);
        assert!(
            !out.lines().any(|l| l.starts_with("P ")),
            "SHALL NOT contain P lines when no perf snapshots"
        );
    }
}
