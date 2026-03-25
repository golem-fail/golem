//! JUnit XML output formatter.
//!
//! Produces JUnit-compatible XML from [`FlowReport`] and [`SuiteReport`]
//! for CI integration (Jenkins, GitHub Actions, etc.).

use crate::{FlowReport, StepOutcome, SuiteReport};
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

        match &step.outcome {
            StepOutcome::Success => {
                let _ = writeln!(
                    out,
                    "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\"/>"
                );
            }
            StepOutcome::Warning(msg) => {
                let escaped_msg = xml_escape(msg);
                let _ = writeln!(
                    out,
                    "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\">"
                );
                let _ = writeln!(out, "      <system-out>{escaped_msg}</system-out>");
                let _ = writeln!(out, "    </testcase>");
            }
            StepOutcome::Failed(msg) => {
                let escaped_msg = xml_escape(msg);
                let _ = writeln!(
                    out,
                    "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\">"
                );
                let _ = writeln!(
                    out,
                    "      <failure message=\"{escaped_msg}\" type=\"AssertionError\">"
                );
                let _ = writeln!(
                    out,
                    "Step failed: {name} - {escaped_msg}"
                );
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

    let _ = writeln!(out, "  </testsuite>");
    out
}

/// Format an entire suite report as JUnit XML.
///
/// Produces a complete XML document with `<?xml ...?>` declaration and a
/// `<testsuites>` root element containing one `<testsuite>` per flow.
pub fn format_suite_junit(report: &SuiteReport) -> String {
    let mut out = String::new();

    let total_tests: usize = report.flows.iter().map(|f| f.step_results.len()).sum();
    let total_failures: usize = report
        .flows
        .iter()
        .map(|f| {
            f.step_results
                .iter()
                .filter(|s| matches!(s.outcome, StepOutcome::Failed(_)))
                .count()
        })
        .sum();
    let total_errors = 0;
    let time = ms_to_secs(report.total_duration_ms);

    let _ = writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(
        out,
        "<testsuites tests=\"{total_tests}\" failures=\"{total_failures}\" \
         errors=\"{total_errors}\" time=\"{time}\">"
    );

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
                },
            ],
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

    // 12. Empty suite produces valid XML ------------------------------

    #[test]
    fn empty_suite_produces_valid_xml() {
        let suite = SuiteReport {
            flows: vec![],
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
