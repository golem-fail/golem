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

/// Did this flow fail before running any step (precondition failure)? Such a
/// flow has no step-level testcase, so JUnit synthesizes one.
fn has_synthetic_failure(flow: &FlowReport) -> bool {
    flow.step_results.is_empty() && flow.is_failed()
}

/// `(failures, errors)` a flow contributes to JUnit counts. Failed steps split
/// by domain — Flow/Parsing faults are `<failure>`, Host/Device/App faults are
/// `<error>` — plus a synthetic entry when the flow never ran. Shared by the
/// per-flow `<testsuite>` and the suite-level `<testsuites>` so they agree.
fn flow_failure_counts(flow: &FlowReport) -> (usize, usize) {
    let mut failures = flow
        .step_results
        .iter()
        .filter(|s| matches!(&s.outcome, StepOutcome::Failed { code, .. } if !code.domain().is_infrastructure()))
        .count();
    let mut errors = flow
        .step_results
        .iter()
        .filter(|s| matches!(&s.outcome, StepOutcome::Failed { code, .. } if code.domain().is_infrastructure()))
        .count();
    if has_synthetic_failure(flow) {
        let infra = flow
            .first_failure_code
            .map(|c| c.domain().is_infrastructure())
            .unwrap_or(false);
        if infra { errors += 1; } else { failures += 1; }
    }
    (failures, errors)
}

/// Number of JUnit testcases a flow emits: one per step, plus a synthetic one
/// when the flow failed before running any step.
fn flow_test_count(flow: &FlowReport) -> usize {
    flow.step_results.len() + usize::from(has_synthetic_failure(flow))
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
            SubstepDetail::DriverWarning { message } =>
                lines.push(format!("[warning] {}", message)),
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

    // A failed step maps to a JUnit <failure> when the fault is in the test or
    // its spec (Flow/Parsing domains), and to an <error> when the environment
    // broke (Host/Device/App domains). A flow that failed without running any
    // step gets a synthetic entry so the failure isn't invisible in JUnit.
    let (failures, errors) = flow_failure_counts(report);
    let synthetic_fail = has_synthetic_failure(report);
    let synthetic_infra = report
        .first_failure_code
        .map(|c| c.domain().is_infrastructure())
        .unwrap_or(false);
    let total_tests = flow_test_count(report);
    let time = ms_to_secs(report.duration_ms);
    let flow_name = xml_escape(&report.flow_name);
    // `timestamp` on <testsuite> is standard (surefire/ant-junit schema);
    // on <testcase> it's a widely-accepted extension (Jenkins/GitLab).
    let suite_ts = report
        .started_at
        .as_deref()
        .map(|s| format!(" timestamp=\"{}\"", xml_escape(s)))
        .unwrap_or_default();

    let _ = writeln!(
        out,
        "  <testsuite name=\"{flow_name}\" tests=\"{total_tests}\" \
         failures=\"{failures}\" errors=\"{errors}\" time=\"{time}\"{suite_ts}>"
    );

    for step in &report.step_results {
        let name = xml_escape(&step_name(&step.action, &step.target));
        let step_time = ms_to_secs(step.duration_ms);
        let substep_text = format_substeps_text(&step.substeps);
        let step_ts = step
            .started_at
            .as_deref()
            .map(|s| format!(" timestamp=\"{}\"", xml_escape(s)))
            .unwrap_or_default();

        match &step.outcome {
            StepOutcome::Success => {
                if substep_text.is_empty() {
                    let _ = writeln!(
                        out,
                        "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\"{step_ts}/>"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\"{step_ts}>"
                    );
                    let _ = writeln!(out, "      <system-out>{}</system-out>", xml_escape(&substep_text));
                    let _ = writeln!(out, "    </testcase>");
                }
            }
            StepOutcome::Warning { message, code } => {
                let rendered = code.render(golem_events::Severity::Warning);
                let escaped_msg = xml_escape(&format!("[{rendered}] {message}"));
                let _ = writeln!(
                    out,
                    "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\"{step_ts}>"
                );
                let combined = if substep_text.is_empty() {
                    escaped_msg.clone()
                } else {
                    format!("{}\n{}", xml_escape(&substep_text), escaped_msg)
                };
                let _ = writeln!(out, "      <system-out>{combined}</system-out>");
                let _ = writeln!(out, "    </testcase>");
            }
            StepOutcome::Failed { message, code } => {
                let rendered = code.render(golem_events::Severity::Error);
                let escaped_msg = xml_escape(message);
                // Infrastructure-domain faults (Host/Device/App) are JUnit
                // <error>s; test/spec faults (Flow/Parsing) are <failure>s.
                let elem = if code.domain().is_infrastructure() { "error" } else { "failure" };
                let _ = writeln!(
                    out,
                    "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\"{step_ts}>"
                );
                let failure_detail = if substep_text.is_empty() {
                    format!("Step failed: {name} - {escaped_msg}")
                } else {
                    format!("{}\nStep failed: {name} - {escaped_msg}", xml_escape(&substep_text))
                };
                let _ = writeln!(
                    out,
                    "      <{elem} message=\"{escaped_msg}\" type=\"{rendered}\">"
                );
                let _ = writeln!(out, "{failure_detail}");
                let _ = writeln!(out, "      </{elem}>");
                let _ = writeln!(out, "    </testcase>");
            }
            StepOutcome::Skipped => {
                let _ = writeln!(
                    out,
                    "    <testcase name=\"{name}\" classname=\"{flow_name}\" time=\"{step_time}\"{step_ts}>"
                );
                let _ = writeln!(out, "      <skipped/>");
                let _ = writeln!(out, "    </testcase>");
            }
        }
    }

    // Synthetic testcase for a flow that failed before running any step.
    if synthetic_fail {
        let elem = if synthetic_infra { "error" } else { "failure" };
        let msg = xml_escape(
            report
                .skipped_reason
                .as_deref()
                .unwrap_or("flow could not run"),
        );
        let type_attr = report
            .first_failure_code
            .map(|c| format!(" type=\"{}\"", c.render(golem_events::Severity::Error)))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "    <testcase name=\"flow could not run\" classname=\"{flow_name}\" time=\"0\">"
        );
        let _ = writeln!(out, "      <{elem} message=\"{msg}\"{type_attr}/>");
        let _ = writeln!(out, "    </testcase>");
    }

    // Properties: os_major (when known) + perf snapshots + covered axes.
    // Emit a single <properties> block so CI renderers see everything
    // together.
    let has_props = report.os_major.is_some()
        || !report.perf_snapshots.is_empty()
        || !report.covered_axes.is_empty()
        || !report.recordings.is_empty();
    if has_props {
        let _ = writeln!(out, "    <properties>");
        if let Some(os) = report.os_major {
            let _ = writeln!(out, "      <property name=\"os_major\" value=\"{os}\"/>");
        }
        if !report.covered_axes.is_empty() {
            let _ = writeln!(
                out,
                "      <property name=\"covered_axes\" value=\"{}\"/>",
                xml_escape(&report.covered_axes.join(","))
            );
        }
        for rec in &report.recordings {
            let _ = writeln!(
                out,
                "      <property name=\"recording.{}.{}\" value=\"{}\"/>",
                xml_escape(&rec.block),
                rec.iteration,
                xml_escape(&rec.path),
            );
        }
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

    let flow_tests: usize = report.flows.iter().map(flow_test_count).sum();
    let install_tests = report.installs.len();
    let total_tests = flow_tests + install_tests;

    let flow_failures: usize = report.flows.iter().map(|f| flow_failure_counts(f).0).sum();
    let flow_errors: usize = report.flows.iter().map(|f| flow_failure_counts(f).1).sum();
    // A failed install is an environment/app problem, not a test-logic
    // failure — count it as a JUnit error.
    let install_failures: usize = report.installs.iter().filter(|i| !i.success).count();
    let total_failures = flow_failures;
    let total_errors = flow_errors + install_failures;
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
        // Use the earliest install's started_at as the install suite's
        // timestamp — closest thing to a meaningful suite start.
        let install_suite_ts = report
            .installs
            .iter()
            .filter_map(|i| i.started_at.as_deref())
            .min()
            .map(|s| format!(" timestamp=\"{}\"", xml_escape(s)))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  <testsuite name=\"install\" tests=\"{}\" failures=\"0\" errors=\"{}\" time=\"{}\"{install_suite_ts}>",
            install_tests, install_failures, ms_to_secs(install_time)
        );
        for inst in &report.installs {
            let classname = xml_escape(&inst.device_name);
            let name = xml_escape(&format!("{} ({})", inst.app_name, inst.bundle_id));
            let time = ms_to_secs(inst.duration_ms);
            let inst_ts = inst
                .started_at
                .as_deref()
                .map(|s| format!(" timestamp=\"{}\"", xml_escape(s)))
                .unwrap_or_default();
            let inst_os = inst
                .os_major
                .map(|os| format!(" os_major=\"{os}\""))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time}\"{inst_ts}{inst_os}>"
            );
            if !inst.success {
                let msg = inst.error.as_deref().unwrap_or("install failed");
                let type_attr = inst
                    .code
                    .map(|c| format!(" type=\"{}\"", c.render(golem_events::Severity::Error)))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "      <error message=\"install script failed\"{type_attr}>{}</error>",
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

    // Flake summary as a synthetic <testsuite>. Empty for single-run.
    // Each (flow, device) entry becomes one <testcase> with a
    // <failure> if any of the repeat runs failed; passes are clean.
    let flake = crate::flake::build_summary(&report.flows);
    if !flake.is_empty() {
        let total_entries = flake.len();
        let failed_entries = flake.iter().filter(|e| e.failed > 0).count();
        let _ = writeln!(
            out,
            "  <testsuite name=\"flake-summary\" tests=\"{total_entries}\" failures=\"{failed_entries}\" errors=\"0\" time=\"0\">"
        );
        for e in &flake {
            let name = xml_escape(&e.flow);
            let _ = writeln!(
                out,
                "    <testcase classname=\"flake\" name=\"{name}\" time=\"0\" passed=\"{}\" failed=\"{}\" skipped=\"{}\" total=\"{}\">",
                e.passed, e.failed, e.skipped, e.total
            );
            if e.failed > 0 {
                let kind = if e.passed > 0 { "flake" } else { "stable-fail" };
                let _ = writeln!(
                    out,
                    "      <failure message=\"{kind}: {}/{} runs failed\"/>",
                    e.failed, e.total
                );
            }
            let _ = writeln!(out, "    </testcase>");
        }
        let _ = writeln!(out, "  </testsuite>");
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

    fn sample_flow() -> FlowReport {
        FlowReport {
            first_failure_code: None,
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

    fn sample_suite() -> SuiteReport {
        SuiteReport {
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
            xml.contains("type=\"EX000\""),
            "failure should have type attribute carrying the failure code"
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

    #[test]
    fn failed_step_failure_type_carries_code() {
        let mut step = failed_step("assert_visible", "Welcome", 10012, "timed out");
        step.outcome = StepOutcome::Failed {
            message: "timed out".to_string(),
            code: golem_events::FailureCode::FlowStepTimeout,
        };
        let flow = FlowReport {
            first_failure_code: Some(golem_events::FailureCode::FlowStepTimeout),
            flow_name: "timeout_flow".to_string(),
            success: false,
            step_results: vec![step],
            warnings: vec![],
            duration_ms: 10012,
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
        let xml = format_flow_junit(&flow);
        assert!(
            xml.contains("type=\"EF408\""),
            "failure type SHALL carry the failure code"
        );
    }

    // 4b. Infrastructure-domain failure maps to <error>, flow-domain to
    //     <failure>, and the testsuite counts split accordingly.
    #[test]
    fn infrastructure_failure_maps_to_error_element() {
        let mut flow_fault = failed_step("assert_visible", "Welcome", 10012, "timed out");
        flow_fault.outcome = StepOutcome::Failed {
            message: "timed out".to_string(),
            code: golem_events::FailureCode::FlowStepTimeout, // F → <failure>
        };
        let mut infra_fault = failed_step("tap", "Submit", 5000, "companion wedged");
        infra_fault.outcome = StepOutcome::Failed {
            message: "companion wedged".to_string(),
            code: golem_events::FailureCode::DeviceCompanionWedged, // D → <error>
        };
        let flow = FlowReport {
            flow_name: "mixed".to_string(),
            success: false,
            step_results: vec![flow_fault, infra_fault],
            first_failure_code: Some(golem_events::FailureCode::FlowStepTimeout),
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        assert!(xml.contains("<failure message=\"timed out\" type=\"EF408\""),
            "flow-domain fault SHALL be a <failure>");
        assert!(xml.contains("<error message=\"companion wedged\" type=\"ED503\""),
            "infrastructure-domain fault SHALL be an <error>");
        assert!(xml.contains("failures=\"1\" errors=\"1\""),
            "testsuite SHALL count one failure and one error, got: {xml}");
    }

    // 4c. A flow that failed without running any step gets a synthetic
    //     testcase so the failure is visible in JUnit (not an empty suite).
    #[test]
    fn no_step_failure_synthesizes_testcase() {
        let flow = FlowReport {
            flow_name: "needs_app".to_string(),
            success: false,
            step_results: vec![],
            skipped_reason: Some("install_script failed".to_string()),
            first_failure_code: Some(golem_events::FailureCode::AppInstallFailed),
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        // AppInstallFailed is App-domain (infrastructure) → <error>, EA500.
        assert!(xml.contains("tests=\"1\" failures=\"0\" errors=\"1\""),
            "no-step failure SHALL surface as one error, got: {xml}");
        assert!(xml.contains("<error message=\"install_script failed\" type=\"EA500\"/>"),
            "synthetic testcase SHALL carry the reason and code, got: {xml}");
    }

    // 5. Warning step has system-out with message ---------------------

    #[test]
    fn warning_step_has_system_out() {
        let flow = sample_flow();
        let xml = format_flow_junit(&flow);
        assert!(
            xml.contains("<system-out>[WX000] element not found</system-out>"),
            "warning step should have <system-out> with message"
        );
    }

    // 6. Skipped step has skipped element -----------------------------

    #[test]
    fn skipped_step_has_skipped_element() {
        let flow = FlowReport {
            first_failure_code: None,
            flow_name: "skip_flow".to_string(),
            success: true,
            step_results: vec![skipped_step("tap", "Cancel")],
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
            first_failure_code: None,
            flow_name: "time_test".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 120)],
            warnings: vec![],
            duration_ms: 120,
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
            first_failure_code: None,
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
            os_major: None,
            perf_snapshots: vec![],
            skipped_reason: None,
            covered_axes: Vec::new(),
            recordings: Vec::new(),
            repeat: None,
            started_at: None,
            finished_at: None,
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

        let xml = format_flow_junit(&flow);
        assert!(xml.contains("<properties>"), "SHALL contain <properties> element");
        assert!(
            xml.contains("perf.login:iPhone_16:0.memory_mb"),
            "SHALL contain perf property name with label"
        );
        assert!(xml.contains("142.5"), "SHALL contain memory_mb value");
    }

    #[test]
    fn junit_includes_covered_axes_property() {
        let flow = FlowReport {
            first_failure_code: None,
            flow_name: "cov_flow".to_string(),
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
            covered_axes: vec!["ios".into(), "v26".into(), "tablet".into()],
            recordings: Vec::new(),
            repeat: None,
            started_at: None,
            finished_at: None,
        };

        let xml = format_flow_junit(&flow);
        assert!(xml.contains("<properties>"), "SHALL emit <properties> when covered_axes set");
        assert!(
            xml.contains(r#"<property name="covered_axes" value="ios,v26,tablet"/>"#),
            "SHALL contain covered_axes property with comma-joined value; got:\n{xml}"
        );
    }

    #[test]
    fn junit_omits_properties_when_no_perf() {
        let flow = FlowReport {
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
            started_at: None,
            finished_at: None,
        };
        let xml = format_suite_junit(&suite);
        assert!(xml.contains("<?xml"));
        assert!(xml.contains("tests=\"0\""));
        assert!(xml.contains("failures=\"0\""));
        assert!(xml.contains("<testsuites"));
        assert!(xml.contains("</testsuites>"));
    }

    // 13. ms_to_secs rounds to three decimal places --------------------

    #[test]
    fn ms_to_secs_formats_three_decimals() {
        assert_eq!(ms_to_secs(0), "0.000", "zero ms SHALL render 0.000");
        assert_eq!(ms_to_secs(1), "0.001", "1ms SHALL render 0.001");
        assert_eq!(ms_to_secs(1000), "1.000", "1000ms SHALL render 1.000");
        assert_eq!(ms_to_secs(1500), "1.500", "1500ms SHALL render 1.500");
        // Sub-millisecond precision is truncated to 3 decimals.
        assert_eq!(ms_to_secs(1234), "1.234", "1234ms SHALL render 1.234");
    }

    // 14. step_name joins action and target, or action alone -----------

    #[test]
    fn step_name_joins_or_uses_action_only() {
        assert_eq!(step_name("tap", "Submit"), "tap: Submit",
            "non-empty target SHALL be appended after a colon");
        assert_eq!(step_name("launch", ""), "launch",
            "empty target SHALL yield action alone");
    }

    // 15. Successful step with substeps emits a system-out block -------

    #[test]
    fn success_step_with_substeps_has_system_out() {
        let mut step = success_step("tap", "Submit", 40);
        step.substeps = vec![SubstepDetail::Tap {
            point: golem_events::Point { x: 10, y: 20 },
            element_bounds: None,
        }];
        let flow = FlowReport {
            flow_name: "sub_flow".to_string(),
            success: true,
            step_results: vec![step],
            first_failure_code: None,
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        // Success with substeps SHALL open the testcase and emit system-out.
        assert!(xml.contains("<system-out>tap (10,20)</system-out>"),
            "success step with substeps SHALL emit <system-out>, got: {xml}");
        assert!(xml.contains("</testcase>"),
            "success step with substeps SHALL close the testcase explicitly");
    }

    // 16. Successful step without substeps is a self-closing testcase --

    #[test]
    fn success_step_without_substeps_is_self_closing() {
        let flow = FlowReport {
            flow_name: "sc_flow".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 100)],
            first_failure_code: None,
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        assert!(xml.contains("time=\"0.100\"/>"),
            "success step without substeps SHALL be a self-closing testcase, got: {xml}");
        assert!(!xml.contains("<system-out>"),
            "success step without substeps SHALL NOT emit <system-out>");
    }

    // 17. Warning step with substeps prepends substep text -------------

    #[test]
    fn warning_step_with_substeps_combines_text() {
        let mut step = warning_step("assert_visible", "Promo", 15, "not found");
        step.substeps = vec![SubstepDetail::Tap {
            point: golem_events::Point { x: 5, y: 6 },
            element_bounds: None,
        }];
        let flow = FlowReport {
            flow_name: "warn_flow".to_string(),
            success: true,
            step_results: vec![step],
            first_failure_code: None,
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        // Substep text is prepended, then the rendered warning line.
        assert!(xml.contains("<system-out>tap (5,6)\n[WX000] not found</system-out>"),
            "warning step with substeps SHALL combine substep text then message, got: {xml}");
    }

    // 18. Failed step with substeps prepends substep text to detail ----

    #[test]
    fn failed_step_with_substeps_prepends_detail() {
        let mut step = failed_step("assert_visible", "Welcome", 100, "timed out");
        step.substeps = vec![SubstepDetail::ElementNotFound {
            selector: "text=Welcome".into(),
            timeout_ms: 5000,
        }];
        let flow = FlowReport {
            flow_name: "fail_sub".to_string(),
            success: false,
            step_results: vec![step],
            first_failure_code: None,
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        // The whole detail (substeps + "Step failed:") is XML-escaped, so the
        // selector's quotes become &quot;.
        assert!(xml.contains("element_not_found &quot;text=Welcome&quot; after 5000ms\nStep failed:"),
            "failed step with substeps SHALL prepend (escaped) substep text to the failure detail, got: {xml}");
    }

    // 19. Synthetic failure without reason/code uses defaults ----------

    #[test]
    fn synthetic_failure_defaults_when_no_reason_or_code() {
        let flow = FlowReport {
            flow_name: "bare_fail".to_string(),
            success: false,
            step_results: vec![],
            skipped_reason: None,
            first_failure_code: None,
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        // No first_failure_code → not infrastructure → <failure>, no type attr.
        assert!(xml.contains("tests=\"1\" failures=\"1\" errors=\"0\""),
            "no-code synthetic failure SHALL count as one <failure>, got: {xml}");
        assert!(xml.contains("name=\"flow could not run\""),
            "synthetic testcase SHALL be named 'flow could not run'");
        assert!(xml.contains("<failure message=\"flow could not run\"/>"),
            "synthetic failure SHALL use the default reason and omit type, got: {xml}");
    }

    // 20. timestamp attributes appear when started_at is set -----------

    #[test]
    fn timestamps_emitted_when_started_at_present() {
        let mut step = success_step("launch", "", 100);
        step.started_at = Some("2026-06-15T10:00:01Z".to_string());
        let flow = FlowReport {
            flow_name: "ts_flow".to_string(),
            success: true,
            step_results: vec![step],
            first_failure_code: None,
            started_at: Some("2026-06-15T10:00:00Z".to_string()),
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        assert!(xml.contains("timestamp=\"2026-06-15T10:00:00Z\""),
            "testsuite SHALL carry the flow timestamp, got: {xml}");
        assert!(xml.contains("timestamp=\"2026-06-15T10:00:01Z\""),
            "testcase SHALL carry the step timestamp, got: {xml}");
    }

    // 21. recordings render as recording.<block>.<iter> properties -----

    #[test]
    fn recordings_render_as_properties() {
        let flow = FlowReport {
            flow_name: "rec_flow".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 100)],
            first_failure_code: None,
            recordings: vec![crate::RecordingEntry {
                block: "checkout".to_string(),
                iteration: 2,
                path: "/tmp/rec.mp4".to_string(),
            }],
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        assert!(xml.contains("<properties>"),
            "recordings SHALL trigger a <properties> block");
        assert!(xml.contains(r#"<property name="recording.checkout.2" value="/tmp/rec.mp4"/>"#),
            "recording SHALL render block.iteration name and path value, got: {xml}");
    }

    // 22. perf snapshot threads / fds / launch_ms properties -----------

    #[test]
    fn perf_snapshot_renders_all_integer_metrics() {
        let flow = FlowReport {
            flow_name: "perf2".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 100)],
            first_failure_code: None,
            perf_snapshots: vec![sample_perf_snapshot()],
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        assert!(xml.contains(r#"<property name="perf.login:iPhone_16:0.cpu_percent" value="23.1"/>"#),
            "cpu_percent SHALL render with one decimal, got: {xml}");
        assert!(xml.contains(r#"<property name="perf.login:iPhone_16:0.threads" value="42"/>"#),
            "threads SHALL render as integer");
        assert!(xml.contains(r#"<property name="perf.login:iPhone_16:0.file_descriptors" value="87"/>"#),
            "file_descriptors SHALL render as integer");
        assert!(xml.contains(r#"<property name="perf.login:iPhone_16:0.launch_ms" value="1240"/>"#),
            "launch_ms SHALL render as integer");
    }

    // 23. perf snapshot with all-None metrics emits no perf properties -

    #[test]
    fn perf_snapshot_with_no_metrics_emits_no_perf_properties() {
        let snap = crate::PerfSnapshot {
            label: "empty".into(),
            memory_mb: None,
            cpu_percent: None,
            threads: None,
            file_descriptors: None,
            disk_mb: None,
            net_rx_kb: None,
            net_tx_kb: None,
            launch_ms: None,
            timestamp: "0".into(),
        };
        let flow = FlowReport {
            flow_name: "empty_perf".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 100)],
            first_failure_code: None,
            perf_snapshots: vec![snap],
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        // has_props is true (perf_snapshots non-empty) so <properties> opens,
        // but no perf.* lines are emitted since every metric is None.
        assert!(xml.contains("<properties>"),
            "non-empty perf_snapshots SHALL open <properties>");
        assert!(!xml.contains("perf.empty"),
            "all-None snapshot SHALL emit no perf.* properties, got: {xml}");
    }

    // 24. os_major renders as a property -------------------------------

    #[test]
    fn os_major_renders_as_property() {
        let flow = FlowReport {
            flow_name: "os_flow".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 100)],
            first_failure_code: None,
            os_major: Some(26),
            ..sample_flow()
        };
        let xml = format_flow_junit(&flow);
        assert!(xml.contains(r#"<property name="os_major" value="26"/>"#),
            "os_major SHALL render as a property, got: {xml}");
    }

    // ── Install suite tests ─────────────────────────────────────────

    fn install(success: bool) -> crate::InstallReport {
        crate::InstallReport {
            app_name: "MyApp".to_string(),
            bundle_id: "com.example.app".to_string(),
            device_name: "Pixel_7a".to_string(),
            os_major: Some(34),
            success,
            duration_ms: 3000,
            exit_code: if success { Some(0) } else { Some(1) },
            error: if success { None } else { Some("gradle exploded".to_string()) },
            code: if success {
                None
            } else {
                Some(golem_events::FailureCode::AppInstallFailed)
            },
            started_at: Some("2026-06-15T09:00:00Z".to_string()),
            finished_at: None,
        }
    }

    // 25. Successful install renders a clean testcase under its suite --

    #[test]
    fn install_success_renders_clean_testcase() {
        let suite = SuiteReport {
            installs: vec![install(true)],
            ..sample_suite()
        };
        let xml = format_suite_junit(&suite);
        assert!(xml.contains("<testsuite name=\"install\""),
            "installs SHALL form a dedicated 'install' testsuite, got: {xml}");
        assert!(xml.contains("name=\"MyApp (com.example.app)\""),
            "install testcase name SHALL combine app and bundle");
        assert!(xml.contains("classname=\"Pixel_7a\""),
            "install classname SHALL be the device name");
        assert!(xml.contains("os_major=\"34\""),
            "install testcase SHALL carry os_major attribute");
        assert!(xml.contains("timestamp=\"2026-06-15T09:00:00Z\""),
            "install suite SHALL use earliest install started_at as timestamp");
        // Successful install SHALL have no <error>.
        assert!(!xml.contains("<error message=\"install script failed\""),
            "successful install SHALL NOT emit an error");
    }

    // 26. Failed install becomes a JUnit error counted suite-wide ------

    #[test]
    fn install_failure_renders_error_and_counts() {
        let suite = SuiteReport {
            installs: vec![install(false)],
            ..sample_suite()
        };
        let xml = format_suite_junit(&suite);
        assert!(xml.contains("<error message=\"install script failed\" type=\"EA500\">gradle exploded</error>"),
            "failed install SHALL emit an <error> carrying the code and message, got: {xml}");
        // Suite errors = flow_errors (0 here) + install_failures (1).
        // sample_suite has 4 flow tests + 1 install test = 5, 1 flow failure.
        assert!(xml.contains("tests=\"5\" failures=\"1\" errors=\"1\""),
            "install failure SHALL add to total tests and errors, got: {xml}");
    }

    // 27. Failed install without error message uses default text -------

    #[test]
    fn install_failure_defaults_error_message() {
        let mut inst = install(false);
        inst.error = None;
        inst.code = None;
        let suite = SuiteReport {
            installs: vec![inst],
            flows: vec![],
            ..sample_suite()
        };
        let xml = format_suite_junit(&suite);
        // No code → no type attribute; missing error → default body text.
        assert!(xml.contains("<error message=\"install script failed\">install failed</error>"),
            "missing install error SHALL default to 'install failed' with no type, got: {xml}");
    }

    // ── Flake summary tests ─────────────────────────────────────────

    fn repeated_flow(name: &str, success: bool) -> FlowReport {
        FlowReport {
            flow_name: name.to_string(),
            success,
            step_results: vec![],
            first_failure_code: None,
            repeat: Some(golem_events::RepeatContext { index: 0, total: 2 }),
            ..sample_flow()
        }
    }

    // 28. A flow that flakes (some pass, some fail) renders a flake row -

    #[test]
    fn flake_summary_renders_flake_failure() {
        let suite = SuiteReport {
            flows: vec![
                repeated_flow("login", true),
                repeated_flow("login", false),
            ],
            installs: vec![],
            ..sample_suite()
        };
        let xml = format_suite_junit(&suite);
        assert!(xml.contains("<testsuite name=\"flake-summary\""),
            "repeat suites SHALL emit a flake-summary testsuite, got: {xml}");
        assert!(xml.contains(r#"passed="1" failed="1" skipped="0" total="2""#),
            "flake testcase SHALL carry the run tallies");
        assert!(xml.contains(r#"<failure message="flake: 1/2 runs failed"/>"#),
            "a flow with both passes and fails SHALL be marked a flake, got: {xml}");
    }

    // 29. A flow that fails every run is a stable-fail, not a flake -----

    #[test]
    fn flake_summary_stable_fail_when_no_pass() {
        let suite = SuiteReport {
            flows: vec![
                repeated_flow("broken", false),
                repeated_flow("broken", false),
            ],
            installs: vec![],
            ..sample_suite()
        };
        let xml = format_suite_junit(&suite);
        assert!(xml.contains(r#"<failure message="stable-fail: 2/2 runs failed"/>"#),
            "an all-fail repeated flow SHALL be a stable-fail, got: {xml}");
    }

    // ── Remaining substep formatters ────────────────────────────────

    // 30. Each substep variant renders its expected line.
    #[test]
    fn substeps_text_covers_remaining_variants() {
        let p = golem_events::Point { x: 1, y: 2 };
        let q = golem_events::Point { x: 3, y: 4 };
        let cases = vec![
            (
                SubstepDetail::DoubleTap { point: p, element_bounds: None },
                "double_tap (1,2)",
            ),
            (
                SubstepDetail::Swipe { from: p, to: q },
                "swipe (1,2)\u{2192}(3,4)",
            ),
            (
                SubstepDetail::ScrollStarted { selector: "text=X".into(), direction: "down".into() },
                "scroll_started \"text=X\" direction=down",
            ),
            (
                SubstepDetail::ScrollFound { selector: "text=X".into(), position: p, total_attempts: 3 },
                "scroll_found \"text=X\" at (1,2) after 3 attempts",
            ),
            (
                SubstepDetail::ScrollDirectionReversed { to_direction: "up".into(), reason: "edge".into() },
                "scroll_reversed \u{2192}up edge",
            ),
            (
                SubstepDetail::ScrollStrategySwitch { to_index: 1, reason: "stuck".into() },
                "scroll_strategy_switch \u{2192}2 stuck",
            ),
            (
                SubstepDetail::AppStop { bundle: "com.x".into() },
                "app_stop bundle=com.x",
            ),
            (
                SubstepDetail::DriverWarning { message: "slow".into() },
                "[warning] slow",
            ),
            (
                SubstepDetail::RetryAttempt { attempt: 1, max: 3, delay_ms: 500, error: "boom".into() },
                "retry 1/3 delay=500ms: boom",
            ),
            (
                SubstepDetail::Screenshot { path: "/tmp/s.png".into() },
                "screenshot /tmp/s.png",
            ),
        ];
        for (substep, expected) in cases {
            let out = format_substeps_text(&[substep]);
            assert_eq!(out, expected, "substep SHALL render as '{expected}'");
        }
    }

    // 31. HttpRequest / BashCommand render '?' when status/exit is None.
    #[test]
    fn substeps_text_http_and_bash_handle_missing_status() {
        let http_some = format_substeps_text(&[SubstepDetail::HttpRequest {
            method: "GET".into(),
            url: "http://x".into(),
            status: Some(200),
            duration_ms: 12,
        }]);
        assert_eq!(http_some, "http GET http://x \u{2192} 200 [12ms]",
            "HttpRequest with status SHALL render the code");
        let http_none = format_substeps_text(&[SubstepDetail::HttpRequest {
            method: "GET".into(),
            url: "http://x".into(),
            status: None,
            duration_ms: 12,
        }]);
        assert_eq!(http_none, "http GET http://x \u{2192} ? [12ms]",
            "HttpRequest without status SHALL render '?'");
        let bash_none = format_substeps_text(&[SubstepDetail::BashCommand {
            command: "ls".into(),
            exit_code: None,
            duration_ms: 5,
        }]);
        assert_eq!(bash_none, "bash \"ls\" exit=? [5ms]",
            "BashCommand without exit code SHALL render '?'");
    }
}
