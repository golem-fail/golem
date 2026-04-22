//! JUnit XML output formatter.
//!
//! Produces JUnit-compatible XML from [`FlowReport`] and [`SuiteReport`]
//! for CI integration (Jenkins, GitHub Actions, etc.).

use crate::{FlowReport, StepOutcome, SubstepDetail, SuiteReport};
use std::fmt::Write;

// ── XML escaping ────────────────────────────────────────────────────

/// Escape characters that are special in XML attribute values and text content.
fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Convert a duration in milliseconds to seconds as a decimal string.
fn ms_to_secs(ms: u64) -> String {
    let secs = ms as f64 / 1_000.0;
    format!("{secs:.3}")
}

/// Build the testcase name from a step's action and target.
fn step_name(action: &str, target: &str) -> String {
    if target.is_empty() {
        action.to_string()
    } else {
        format!("{action}: {target}")
    }
}

// ── Public API ──────────────────────────────────────────────────────

/// Format substeps as plain text for JUnit system-out.
fn format_substeps_text(substeps: &[SubstepDetail]) -> String {
    if substeps.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    for sub in substeps {
        match sub {
            SubstepDetail::ElementResolved { selector, bounds, tap_point } =>
                lines.push(format!("element_resolved \"{}\" bounds=({},{},{},{}) tap=({},{})",
                    selector, bounds.x, bounds.y, bounds.width, bounds.height, tap_point.x, tap_point.y)),
            SubstepDetail::ElementNotFound { selector, timeout_ms } =>
                lines.push(format!("element_not_found \"{}\" after {}ms", selector, timeout_ms)),
            SubstepDetail::Tap { point, .. } =>
                lines.push(format!("tap ({},{})", point.x, point.y)),
            SubstepDetail::DoubleTap { point, .. } =>
                lines.push(format!("double_tap ({},{})", point.x, point.y)),
            SubstepDetail::TextInput { text, .. } =>
                lines.push(format!("text_input \"{}\"", text)),
            SubstepDetail::Swipe { from, to } =>
                lines.push(format!("swipe ({},{})→({},{})", from.x, from.y, to.x, to.y)),
            SubstepDetail::ScrollStarted { selector, direction } =>
                lines.push(format!("scroll_started \"{}\" direction={}", selector, direction)),
            SubstepDetail::ScrollAttempt { attempt, direction, strategy_index, from, to, result, tree_stats } =>
                lines.push(format!("scroll_attempt #{} strategy={} {} ({},{})→({},{}) {} {{{} trees, {} nodes}}",
                    attempt, strategy_index + 1, direction, from.x, from.y, to.x, to.y, result,
                    tree_stats.fetches, tree_stats.max_nodes)),
            SubstepDetail::ScrollFound { selector, position, total_attempts } =>
                lines.push(format!("scroll_found \"{}\" at ({},{}) after {} attempts",
                    selector, position.x, position.y, total_attempts)),
            SubstepDetail::ScrollDirectionReversed { to_direction, reason } =>
                lines.push(format!("scroll_reversed →{} {}", to_direction, reason)),
            SubstepDetail::ScrollStrategySwitch { to_index, reason } =>
                lines.push(format!("scroll_strategy_switch →{} {}", to_index + 1, reason)),
            SubstepDetail::AppLaunch { bundle, duration_ms } =>
                lines.push(format!("app_launch bundle={} {}ms", bundle, duration_ms)),
            SubstepDetail::AppStop { bundle } =>
                lines.push(format!("app_stop bundle={}", bundle)),
            SubstepDetail::RetryAttempt { attempt, max, delay_ms, error } =>
                lines.push(format!("retry {}/{} delay={}ms: {}", attempt, max, delay_ms, error)),
            SubstepDetail::HttpRequest { method, url, status, duration_ms } => {
                let s = status.map(|s| s.to_string()).unwrap_or_else(|| "?".to_string());
                lines.push(format!("http {} {} → {} [{}ms]", method, url, s, duration_ms));
            }
            SubstepDetail::BashCommand { command, exit_code, duration_ms } => {
                let c = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
                lines.push(format!("bash \"{}\" exit={} [{}ms]", command, c, duration_ms));
            }
            SubstepDetail::Screenshot { path } =>
                lines.push(format!("screenshot {}", path)),
            _ => {}
        }
    }
    lines.join("\n")
}

/// Format a single flow as a JUnit `<testsuite>` XML element.
pub fn format_flow_junit(report: &FlowReport) -> String {
    let mut out = String::new();

    let total_tests = report.step_results.len();
    let failures = report
        .step_results
        .iter()
        .filter(|s| matches!(s.outcome, StepOutcome::Failed(_)))
        .count();
    let errors = 0;
    let time = ms_to_secs(report.duration_ms);
    let flow_name = xml_escape(&report.flow_name);

    let _ = writeln!(
        out,
        "  <testsuite name=\"{flow_name}\" tests=\"{total_tests}\" \
         failures=\"{failures}\" errors=\"{errors}\" time=\"{time}\">"
    );

    for step in &report.step_results {
        let name = xml_escape(&step_name(&step.action, &step.target));
        let step_time = ms_to_secs(step.duration_ms);
        let substep_text = format_substeps_text(&step.substeps);

        match &step.outcome {
            StepOutcome::Success => {
                if substep_text.is_empty() {
                    let _ = writeln!(
                        out,
                        "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\"/>"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\">"
                    );
                    let _ = writeln!(out, "      <system-out>{}</system-out>", xml_escape(&substep_text));
                    let _ = writeln!(out, "    </testcase>");
                }
            }
            StepOutcome::Warning(msg) => {
                let escaped_msg = xml_escape(msg);
                let _ = writeln!(
                    out,
                    "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\">"
                );
                let combined = if substep_text.is_empty() {
                    escaped_msg.clone()
                } else {
                    format!("{}\n{}", xml_escape(&substep_text), escaped_msg)
                };
                let _ = writeln!(out, "      <system-out>{combined}</system-out>");
                let _ = writeln!(out, "    </testcase>");
            }
            StepOutcome::Failed(msg) => {
                let escaped_msg = xml_escape(msg);
                let _ = writeln!(
                    out,
                    "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\">"
                );
                let failure_detail = if substep_text.is_empty() {
                    format!("Step failed: {name} - {escaped_msg}")
                } else {
                    format!("{}\nStep failed: {name} - {escaped_msg}", xml_escape(&substep_text))
                };
                let _ = writeln!(
                    out,
                    "      <failure message=\"{escaped_msg}\" type=\"AssertionError\">"
                );
                let _ = writeln!(out, "{failure_detail}");
                let _ = writeln!(out, "      </failure>");
                let _ = writeln!(out, "    </testcase>");
            }
            StepOutcome::Skipped => {
                let _ = writeln!(
                    out,
                    "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\">"
                );
                let _ = writeln!(out, "      <skipped/>");
                let _ = writeln!(out, "    </testcase>");
            }
        }
    }

    // Perf properties
    if !report.perf_snapshots.is_empty() {
        let _ = writeln!(out, "    <properties>");
        for snap in &report.perf_snapshots {
            if let Some(v) = snap.memory_mb {
                let _ = writeln!(
                    out,
                    "      <property name=\"perf.{}.memory_mb\" value=\"{v:.1}\"/>",
                    xml_escape(&snap.label)
                );
            }
            if let Some(v) = snap.cpu_percent {
                let _ = writeln!(
                    out,
                    "      <property name=\"perf.{}.cpu_percent\" value=\"{v:.1}\"/>",
                    xml_escape(&snap.label)
                );
            }
            if let Some(v) = snap.threads {
                let _ = writeln!(
                    out,
                    "      <property name=\"perf.{}.threads\" value=\"{v}\"/>",
                    xml_escape(&snap.label)
                );
            }
            if let Some(v) = snap.file_descriptors {
                let _ = writeln!(
                    out,
                    "      <property name=\"perf.{}.file_descriptors\" value=\"{v}\"/>",
                    xml_escape(&snap.label)
                );
            }
            if let Some(v) = snap.launch_ms {
                let _ = writeln!(
                    out,
                    "      <property name=\"perf.{}.launch_ms\" value=\"{v}\"/>",
                    xml_escape(&snap.label)
                );
            }
        }
        let _ = writeln!(out, "    </properties>");
    }

    let _ = writeln!(out, "  </testsuite>");
    out
}

/// Format an entire suite report as JUnit XML.
///
/// Produces a complete XML document with `<?xml ...?>` declaration and a
/// `<testsuites>` root element containing one `<testsuite>` per flow.
pub fn format_suite_junit(report: &SuiteReport) -> String {
    let mut out = String::new();

    let flow_tests: usize = report.flows.iter().map(|f| f.step_results.len()).sum();
    let install_tests = report.installs.len();
    let total_tests = flow_tests + install_tests;

    let flow_failures: usize = report
        .flows
        .iter()
        .map(|f| {
            f.step_results
                .iter()
                .filter(|s| matches!(s.outcome, StepOutcome::Failed(_)))
                .count()
        })
        .sum();
    let install_failures: usize = report.installs.iter().filter(|i| !i.success).count();
    let total_failures = flow_failures + install_failures;
    let total_errors = 0;
    let time = ms_to_secs(report.total_duration_ms);

    let _ = writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(
        out,
        "<testsuites tests=\"{total_tests}\" failures=\"{total_failures}\" \
         errors=\"{total_errors}\" time=\"{time}\">"
    );

    // Install results as a separate testsuite so CI tools can surface them.
    if !report.installs.is_empty() {
        let install_time: u64 = report.installs.iter().map(|i| i.duration_ms).sum();
        let _ = writeln!(
            out,
            "  <testsuite name=\"install\" tests=\"{}\" failures=\"{}\" errors=\"0\" time=\"{}\">",
            install_tests, install_failures, ms_to_secs(install_time)
        );
        for inst in &report.installs {
            let classname = xml_escape(&inst.device_name);
            let name = xml_escape(&format!("{} ({})", inst.app_name, inst.bundle_id));
            let time = ms_to_secs(inst.duration_ms);
            let _ = writeln!(
                out,
                "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time}\">"
            );
            if !inst.success {
                let msg = inst.error.as_deref().unwrap_or("install failed");
                let _ = writeln!(
                    out,
                    "      <failure message=\"install script failed\">{}</failure>",
                    xml_escape(msg)
                );
            }
            let _ = writeln!(out, "    </testcase>");
        }
        let _ = writeln!(out, "  </testsuite>");
    }

    for flow in &report.flows {
        let _ = write!(out, "{}", format_flow_junit(flow));
    }

    let _ = writeln!(out, "</testsuites>");
    out
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StepReport;

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

    fn sample_flow() -> FlowReport {
        FlowReport {
            flow_name: "login_flow".to_string(),
            success: false,
            step_results: vec![
                success_step("launch", "", 120),
                success_step("tap", "Sign Up", 45),
                success_step("type", "email", 32),
                warning_step("assert_visible", "Promo", 15, "element not found"),
                success_step("tap", "Submit", 38),
                failed_step("assert_visible", "Welcome", 10012, "timed out after 10000ms"),
            ],
            warnings: vec![],
            duration_ms: 10262,
            seed: None,
            screenshot_path: None,
            device_name: None,
            perf_snapshots: vec![],
            skipped_reason: None,
        }
    }

    fn sample_suite() -> SuiteReport {
        SuiteReport {
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
        }
    }

    // 1. Valid XML structure -------------------------------------------

    #[test]
    fn valid_xml_structure() {
        let suite = sample_suite();
        let xml = format_suite_junit(&suite);
        assert!(
            xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
            "should start with XML declaration"
        );
        assert!(xml.contains("<testsuites"), "SHALL contain <testsuites>");
        assert!(xml.contains("</testsuites>"), "SHALL contain closing </testsuites>");
    }

    // 2. Flow maps to testsuite with correct attributes ---------------

    #[test]
    fn flow_maps_to_testsuite_element() {
        let flow = sample_flow();
        let xml = format_flow_junit(&flow);
        assert!(xml.contains("<testsuite"), "SHALL contain <testsuite>");
        assert!(
            xml.contains("name=\"login_flow\""),
            "should have name attribute"
        );
        assert!(
            xml.contains("tests=\"6\""),
            "should have correct tests count"
        );
        assert!(
            xml.contains("failures=\"1\""),
            "should have correct failures count"
        );
        assert!(
            xml.contains("errors=\"0\""),
            "should have errors=0"
        );
    }

    // 3. Step maps to testcase with name and time ---------------------

    #[test]
    fn step_maps_to_testcase() {
        let flow = sample_flow();
        let xml = format_flow_junit(&flow);
        assert!(
            xml.contains("name=\"tap: Sign Up\""),
            "testcase should have 'action: target' name"
        );
        assert!(
            xml.contains("classname=\"login_flow\""),
            "testcase should have classname from flow"
        );
        // The launch step has no target, so just the action
        assert!(
            xml.contains("name=\"launch\""),
            "testcase with empty target uses action only"
        );
    }

    // 4. Failed step has failure element with message -----------------

    #[test]
    fn failed_step_has_failure_element() {
        let flow = sample_flow();
        let xml = format_flow_junit(&flow);
        assert!(
            xml.contains("<failure"),
            "should contain <failure> element"
        );
        assert!(
            xml.contains("message=\"timed out after 10000ms\""),
            "failure should have message attribute"
        );
        assert!(
            xml.contains("type=\"AssertionError\""),
            "failure should have type attribute"
        );
        assert!(
            xml.contains("Step failed:"),
            "failure body should contain description"
        );
        assert!(
            xml.contains("</failure>"),
            "should have closing </failure> tag"
        );
    }

    // 5. Warning step has system-out with message ---------------------

    #[test]
    fn warning_step_has_system_out() {
        let flow = sample_flow();
        let xml = format_flow_junit(&flow);
        assert!(
            xml.contains("<system-out>element not found</system-out>"),
            "warning step should have <system-out> with message"
        );
    }

    // 6. Skipped step has skipped element -----------------------------

    #[test]
    fn skipped_step_has_skipped_element() {
        let flow = FlowReport {
            flow_name: "skip_flow".to_string(),
            success: true,
            step_results: vec![skipped_step("tap", "Cancel")],
            warnings: vec![],
            duration_ms: 0,
            seed: None,
            screenshot_path: None,
            device_name: None,
            perf_snapshots: vec![],
            skipped_reason: None,
        };
        let xml = format_flow_junit(&flow);
        assert!(
            xml.contains("<skipped/>"),
            "skipped step should have <skipped/> element"
        );
    }

    // 7. Time is in seconds (decimal), not milliseconds ---------------

    #[test]
    fn time_is_in_seconds() {
        let flow = FlowReport {
            flow_name: "time_test".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 120)],
            warnings: vec![],
            duration_ms: 120,
            seed: None,
            screenshot_path: None,
            device_name: None,
            perf_snapshots: vec![],
            skipped_reason: None,
        };
        let xml = format_flow_junit(&flow);
        // Flow-level time: 120ms -> 0.120
        assert!(
            xml.contains("time=\"0.120\""),
            "flow time should be in seconds: got {}",
            xml
        );
        // Step-level time: 120ms -> 0.120
        let step_time_count = xml.matches("time=\"0.120\"").count();
        assert_eq!(
            step_time_count, 2,
            "both testsuite and testcase should have time in seconds"
        );
    }

    // 8. XML entities are escaped in names and messages ----------------

    #[test]
    fn xml_entities_are_escaped() {
        let flow = FlowReport {
            flow_name: "flow & <friends>".to_string(),
            success: false,
            step_results: vec![failed_step(
                "assert",
                "x > 0 & y < 10",
                100,
                "expected \"true\" but got 'false'",
            )],
            warnings: vec![],
            duration_ms: 100,
            seed: None,
            screenshot_path: None,
            device_name: None,
            perf_snapshots: vec![],
            skipped_reason: None,
        };
        let xml = format_flow_junit(&flow);

        // Flow name should be escaped
        assert!(
            xml.contains("flow &amp; &lt;friends&gt;"),
            "flow name should escape &, <, >"
        );
        // Step target should be escaped
        assert!(
            xml.contains("x &gt; 0 &amp; y &lt; 10"),
            "step target should escape special chars"
        );
        // Failure message should be escaped
        assert!(
            xml.contains("&quot;true&quot;"),
            "failure message should escape quotes"
        );
        assert!(
            xml.contains("&apos;false&apos;"),
            "failure message should escape apostrophes"
        );
    }

    // 9. Suite totals (tests, failures) are correct -------------------

    #[test]
    fn suite_totals_are_correct() {
        let suite = sample_suite();
        let xml = format_suite_junit(&suite);
        // Total steps: 2 (login) + 2 (signup) = 4
        assert!(
            xml.contains("tests=\"4\""),
            "testsuites should show total step count"
        );
        // Total failures: 0 (login) + 1 (signup) = 1
        assert!(
            xml.contains("failures=\"1\""),
            "testsuites should show total failures"
        );
        assert!(
            xml.contains("errors=\"0\""),
            "testsuites should show errors=0"
        );
        // Suite time: 45300ms -> 45.300
        assert!(
            xml.contains("time=\"45.300\""),
            "testsuites should show time in seconds"
        );
    }

    // 10. Multiple flows produce multiple testsuite elements ----------

    #[test]
    fn multiple_flows_produce_multiple_testsuites() {
        let suite = sample_suite();
        let xml = format_suite_junit(&suite);
        let testsuite_count = xml.matches("<testsuite ").count();
        assert_eq!(
            testsuite_count, 2,
            "should have one <testsuite> per flow"
        );
        assert!(
            xml.contains("name=\"login_flow\""),
            "should contain login_flow testsuite"
        );
        assert!(
            xml.contains("name=\"signup_flow\""),
            "should contain signup_flow testsuite"
        );
    }

    // 11. xml_escape handles all special characters -------------------

    #[test]
    fn xml_escape_handles_all_specials() {
        assert_eq!(xml_escape("&"), "&amp;");
        assert_eq!(xml_escape("<"), "&lt;");
        assert_eq!(xml_escape(">"), "&gt;");
        assert_eq!(xml_escape("\""), "&quot;");
        assert_eq!(xml_escape("'"), "&apos;");
        assert_eq!(xml_escape("hello"), "hello");
        assert_eq!(
            xml_escape("a & b < c > d \"e\" 'f'"),
            "a &amp; b &lt; c &gt; d &quot;e&quot; &apos;f&apos;"
        );
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
    fn junit_includes_perf_properties() {
        let flow = FlowReport {
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

        let xml = format_flow_junit(&flow);
        assert!(xml.contains("<properties>"), "SHALL contain <properties> element");
        assert!(
            xml.contains("perf.login:iPhone_16:0.memory_mb"),
            "SHALL contain perf property name with label"
        );
        assert!(xml.contains("142.5"), "SHALL contain memory_mb value");
    }

    #[test]
    fn junit_omits_properties_when_no_perf() {
        let flow = FlowReport {
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

        let xml = format_flow_junit(&flow);
        assert!(
            !xml.contains("<properties>"),
            "SHALL NOT contain <properties> when no perf snapshots"
        );
    }

    // ── format_substeps_text tests ─────────────────────────────────

    #[test]
    fn substeps_text_empty_returns_empty_string() {
        let out = format_substeps_text(&[]);
        assert_eq!(out, "", "SHALL return empty string for empty substeps");
    }

    #[test]
    fn substeps_text_element_resolved_formats_bounds_and_tap_point() {
        let substeps = vec![SubstepDetail::ElementResolved {
            selector: "text=Submit".into(),
            bounds: golem_events::Rect { x: 20, y: 400, width: 200, height: 50 },
            tap_point: golem_events::Point { x: 120, y: 425 },
        }];
        let out = format_substeps_text(&substeps);
        assert_eq!(
            out,
            "element_resolved \"text=Submit\" bounds=(20,400,200,50) tap=(120,425)",
            "SHALL format ElementResolved with bounds and tap_point"
        );
    }

    #[test]
    fn substeps_text_scroll_attempt_formats_strategy_and_coords() {
        let substeps = vec![SubstepDetail::ScrollAttempt {
            attempt: 2,
            direction: "down".into(),
            strategy_index: 0,
            from: golem_events::Point { x: 200, y: 800 },
            to: golem_events::Point { x: 200, y: 400 },
            result: "PageScrolled".into(),
            tree_stats: golem_events::TreeStats::default(),
        }];
        let out = format_substeps_text(&substeps);
        assert_eq!(
            out,
            "scroll_attempt #2 strategy=1 down (200,800)\u{2192}(200,400) PageScrolled {0 trees, 0 nodes}",
            "SHALL format ScrollAttempt with strategy (1-indexed), direction, coords, result, and tree stats"
        );
    }

    #[test]
    fn substeps_text_multiple_substeps_joined_with_newlines() {
        let substeps = vec![
            SubstepDetail::Tap {
                point: golem_events::Point { x: 100, y: 200 },
                element_bounds: None,
            },
            SubstepDetail::TextInput {
                text: "hello".into(),
                field_bounds: None,
            },
        ];
        let out = format_substeps_text(&substeps);
        assert_eq!(
            out,
            "tap (100,200)\ntext_input \"hello\"",
            "SHALL join multiple substep lines with newlines"
        );
    }

    #[test]
    fn substeps_text_app_launch_formats_bundle_and_duration() {
        let substeps = vec![SubstepDetail::AppLaunch {
            bundle: "com.example.app".into(),
            duration_ms: 2000,
        }];
        let out = format_substeps_text(&substeps);
        assert_eq!(out, "app_launch bundle=com.example.app 2000ms",
            "SHALL format AppLaunch with bundle and duration");
    }

    #[test]
    fn substeps_text_element_not_found_formats_selector_and_timeout() {
        let substeps = vec![SubstepDetail::ElementNotFound {
            selector: "text=Ghost".into(),
            timeout_ms: 10000,
        }];
        let out = format_substeps_text(&substeps);
        assert_eq!(out, "element_not_found \"text=Ghost\" after 10000ms",
            "SHALL format ElementNotFound with selector and timeout");
    }

    // 12. Empty suite produces valid XML ------------------------------

    #[test]
    fn empty_suite_produces_valid_xml() {
        let suite = SuiteReport {
            flows: vec![],
            installs: Vec::new(),
            total_duration_ms: 0,
        };
        let xml = format_suite_junit(&suite);
        assert!(xml.contains("<?xml"));
        assert!(xml.contains("tests=\"0\""));
        assert!(xml.contains("failures=\"0\""));
        assert!(xml.contains("<testsuites"));
        assert!(xml.contains("</testsuites>"));
    }
}
