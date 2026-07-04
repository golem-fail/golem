//! JSON output formatter.
//!
//! Produces structured JSON from [`FlowReport`] and [`SuiteReport`].
//! Uses intermediate serialization structs so the JSON shape can evolve
//! independently of the internal report types.

use crate::{FlowReport, InstallReport, PerfSnapshot, StepOutcome, StepReport, SuiteReport};
use serde::Serialize;

// ── JSON-specific intermediate types ────────────────────────────────

#[derive(Serialize)]
struct JsonStep {
    index: u64,
    #[serde(skip_serializing_if = "String::is_empty")]
    block: String,
    action: String,
    target: String,
    outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at: Option<String>,
    retries: u32,
    tree_fetches: u32,
    tree_nodes: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    substeps: Vec<crate::SubstepDetail>,
}

#[derive(Serialize)]
struct JsonFlow {
    name: String,
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped_reason: Option<String>,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    os_major: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screenshot: Option<String>,
    steps: Vec<JsonStep>,
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    perf_snapshots: Vec<JsonPerfSnapshot>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    a11y_audits: Vec<JsonA11yAudit>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    covered_axes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    recordings: Vec<JsonRecording>,
}

#[derive(Serialize)]
struct JsonA11yAudit {
    label: String,
    errors: usize,
    warnings: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    screenshot_path: Option<String>,
    issues: Vec<JsonA11yIssue>,
}

#[derive(Serialize)]
struct JsonA11yIssue {
    /// 1-based index matching the rectangle marker on the annotated screenshot.
    marker: usize,
    check: String,
    severity: golem_events::Severity,
    message: String,
    element_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    element_label: Option<String>,
    confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Serialize)]
struct JsonRecording {
    block: String,
    iteration: u32,
    path: String,
}

#[derive(Serialize)]
struct JsonInstall {
    app_name: String,
    bundle_id: String,
    device: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    os_major: Option<u32>,
    success: bool,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    skipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    skip_reason: Option<String>,
}

#[derive(Serialize)]
struct JsonPerfSnapshot {
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_mb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    threads: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_descriptors: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk_mb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    net_rx_kb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    net_tx_kb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_ms: Option<u64>,
    timestamp: String,
}

#[derive(Serialize)]
struct JsonSuiteSummary {
    total: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at: Option<String>,
}

#[derive(Serialize)]
struct JsonSuite {
    suite: JsonSuiteSummary,
    flows: Vec<JsonFlow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    installs: Vec<JsonInstall>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    flake_summary: Vec<JsonFlakeEntry>,
}

#[derive(Serialize)]
struct JsonFlakeEntry {
    flow: String,
    passed: u32,
    failed: u32,
    skipped: u32,
    total: u32,
}

// ── Conversion helpers ──────────────────────────────────────────────

fn step_to_json(step: &StepReport) -> JsonStep {
    let (outcome, code, error, warning) = match &step.outcome {
        StepOutcome::Success => ("success".to_string(), None, None, None),
        StepOutcome::Warning { message, code } => (
            "warning".to_string(),
            Some(code.render(golem_events::Severity::Warning)),
            None,
            Some(message.clone()),
        ),
        StepOutcome::Failed { message, code } => (
            "failed".to_string(),
            Some(code.render(golem_events::Severity::Error)),
            Some(message.clone()),
            None,
        ),
        StepOutcome::Skipped => ("skipped".to_string(), None, None, None),
    };

    JsonStep {
        index: step.global_step_index,
        block: step.block_name.clone(),
        action: step.action.clone(),
        target: step.target.clone(),
        outcome,
        code,
        error,
        warning,
        duration_ms: step.duration_ms,
        started_at: step.started_at.clone(),
        finished_at: step.finished_at.clone(),
        retries: step.retry_count,
        tree_fetches: step.tree_stats.fetches,
        tree_nodes: step.tree_stats.max_nodes,
        substeps: step.substeps.clone(),
    }
}

fn perf_to_json(snap: &PerfSnapshot) -> JsonPerfSnapshot {
    JsonPerfSnapshot {
        label: snap.label.clone(),
        memory_mb: snap.memory_mb,
        cpu_percent: snap.cpu_percent,
        threads: snap.threads,
        file_descriptors: snap.file_descriptors,
        disk_mb: snap.disk_mb,
        net_rx_kb: snap.net_rx_kb,
        net_tx_kb: snap.net_tx_kb,
        launch_ms: snap.launch_ms,
        timestamp: snap.timestamp.clone(),
    }
}

fn a11y_to_json(audit: &crate::A11yAudit) -> JsonA11yAudit {
    JsonA11yAudit {
        label: audit.label.clone(),
        errors: audit.error_count(),
        warnings: audit.warning_count(),
        screenshot_path: audit.screenshot_path.clone(),
        issues: audit
            .issues
            .iter()
            .enumerate()
            .map(|(n, i)| JsonA11yIssue {
                marker: n + 1,
                check: i.check_id.clone(),
                severity: i.severity,
                message: i.message.clone(),
                element_type: i.element_type.clone(),
                element_label: i.element_label.clone(),
                confidence: i.confidence,
                detail: i.detail.clone(),
            })
            .collect(),
    }
}

fn flow_to_json(report: &FlowReport) -> JsonFlow {
    JsonFlow {
        name: report.flow_name.clone(),
        success: report.success,
        code: report
            .first_failure_code
            .filter(|_| !report.success)
            .map(|c| c.render(golem_events::Severity::Error)),
        skipped_reason: report.skipped_reason.clone(),
        duration_ms: report.duration_ms,
        started_at: report.started_at.clone(),
        finished_at: report.finished_at.clone(),
        seed: report.seed,
        device: report.device_name.clone(),
        os_major: report.os_major,
        screenshot: report.screenshot_path.clone(),
        steps: report.step_results.iter().map(step_to_json).collect(),
        warnings: report.warnings.clone(),
        perf_snapshots: report.perf_snapshots.iter().map(perf_to_json).collect(),
        a11y_audits: report.a11y_audits.iter().map(a11y_to_json).collect(),
        covered_axes: report.covered_axes.clone(),
        recordings: report
            .recordings
            .iter()
            .map(|r| JsonRecording {
                block: r.block.clone(),
                iteration: r.iteration,
                path: r.path.clone(),
            })
            .collect(),
    }
}

fn install_to_json(inst: &InstallReport) -> JsonInstall {
    JsonInstall {
        app_name: inst.app_name.clone(),
        bundle_id: inst.bundle_id.clone(),
        device: inst.device_name.clone(),
        os_major: inst.os_major,
        success: inst.success,
        duration_ms: inst.duration_ms,
        started_at: inst.started_at.clone(),
        finished_at: inst.finished_at.clone(),
        exit_code: inst.exit_code,
        error: inst.error.clone(),
        skipped: inst.skipped,
        skip_reason: inst.skip_reason.clone(),
    }
}

// ── Public API ──────────────────────────────────────────────────────

/// Serialize a single flow report to a pretty-printed JSON string.
pub fn format_flow_json(report: &FlowReport) -> Result<String, serde_json::Error> {
    let json_flow = flow_to_json(report);
    serde_json::to_string_pretty(&json_flow)
}

/// Serialize an entire suite report to a pretty-printed JSON string.
pub fn format_suite_json(report: &SuiteReport) -> Result<String, serde_json::Error> {
    let total = report.flows.len();
    let passed = report.flows.iter().filter(|f| f.is_passed()).count();
    let failed = report.flows.iter().filter(|f| f.is_failed()).count();
    let skipped = report
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

    let json_suite = JsonSuite {
        suite: JsonSuiteSummary {
            total,
            passed,
            failed,
            skipped,
            duration_ms: report.total_duration_ms,
            started_at: report.started_at.clone(),
            finished_at: report.finished_at.clone(),
        },
        flows: report.flows.iter().map(flow_to_json).collect(),
        installs: report.installs.iter().map(install_to_json).collect(),
        flake_summary: crate::flake::build_summary(&report.flows)
            .into_iter()
            .map(|e| JsonFlakeEntry {
                flow: e.flow,
                passed: e.passed,
                failed: e.failed,
                skipped: e.skipped,
                total: e.total,
            })
            .collect(),
    };

    serde_json::to_string_pretty(&json_suite)
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

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
            substeps: Vec::new(),
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
            substeps: Vec::new(),
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
            substeps: Vec::new(),
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
            substeps: Vec::new(),
            tree_stats: golem_events::TreeStats::default(),
            started_at: None,
            finished_at: None,
        }
    }

    fn sample_flow() -> FlowReport {
        FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "login_flow".to_string(),
            success: false,
            step_results: vec![
                success_step("launch", "", 120),
                success_step("tap", "Sign Up", 45),
                failed_step(
                    "assert_visible",
                    "Welcome",
                    10012,
                    "timed out after 10000ms",
                ),
            ],
            warnings: vec!["element 'Promo' not found".to_string()],
            duration_ms: 10200,
            seed: Some(847_291_036),
            screenshot_path: Some(".golem/screenshots/login_flow_main_step5_error.png".to_string()),
            device_name: Some("iPhone 15 Pro".to_string()),
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
                    success: true,
                    step_results: vec![success_step("launch", "", 80)],
                    warnings: vec![],
                    duration_ms: 80,
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
                    flow_name: "checkout_flow".to_string(),
                    success: true,
                    step_results: vec![success_step("launch", "", 90)],
                    warnings: vec![],
                    duration_ms: 90,
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
                    flow_name: "broken_flow".to_string(),
                    success: false,
                    step_results: vec![
                        success_step("launch", "", 70),
                        failed_step("assert_visible", "Welcome", 5000, "not found"),
                    ],
                    warnings: vec![],
                    duration_ms: 5070,
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

    // 1. format_flow_json produces valid JSON -------------------------

    #[test]
    fn flow_json_is_valid() {
        let report = sample_flow();
        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let parsed: Value = serde_json::from_str(&json_str).expect("should be valid JSON");
        assert!(parsed.is_object(), "top-level value SHALL be an object");
    }

    // 2. JSON contains all flow fields --------------------------------

    #[test]
    fn flow_json_contains_all_fields() {
        let report = sample_flow();
        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert_eq!(v["name"], "login_flow");
        assert_eq!(v["success"], false);
        assert_eq!(v["duration_ms"], 10200);
        assert_eq!(v["seed"], 847_291_036);
        assert_eq!(v["device"], "iPhone 15 Pro");
        assert_eq!(
            v["screenshot"],
            ".golem/screenshots/login_flow_main_step5_error.png"
        );
    }

    // 3. JSON step has action, target, outcome, duration_ms -----------

    #[test]
    fn step_json_has_required_fields() {
        let report = sample_flow();
        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let first_step = &v["steps"][0];
        assert_eq!(first_step["action"], "launch");
        assert_eq!(first_step["target"], "");
        assert_eq!(first_step["outcome"], "success");
        assert_eq!(first_step["duration_ms"], 120);
    }

    // 4. Failed step includes error message ---------------------------

    #[test]
    fn failed_step_includes_error() {
        let report = sample_flow();
        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let failed = &v["steps"][2];
        assert_eq!(failed["outcome"], "failed");
        assert_eq!(failed["error"], "timed out after 10000ms");
        // No warning field on a failed step
        assert!(
            failed.get("warning").is_none(),
            "failed step has no warning field"
        );
    }

    #[test]
    fn failed_step_and_flow_serialize_failure_code() {
        let mut step = failed_step("assert_visible", "Welcome", 10012, "timed out");
        step.outcome = StepOutcome::Failed {
            message: "timed out".to_string(),
            code: golem_events::FailureCode::FlowStepTimeout,
        };
        let report = FlowReport {
            first_failure_code: Some(golem_events::FailureCode::FlowStepTimeout),
            a11y_audits: vec![],
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
        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert_eq!(
            v["steps"][0]["code"], "EF408",
            "step SHALL carry its failure code"
        );
        assert_eq!(v["code"], "EF408", "flow SHALL carry first failure code");
    }

    // 5. Warning step includes warning message ------------------------

    #[test]
    fn warning_step_includes_warning() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "warn_flow".to_string(),
            success: true,
            step_results: vec![warning_step(
                "assert_visible",
                "Promo",
                15,
                "element not found",
            )],
            warnings: vec![],
            duration_ms: 15,
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

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let step = &v["steps"][0];
        assert_eq!(step["outcome"], "warning");
        assert_eq!(step["warning"], "element not found");
        // No error field on a warning step
        assert!(
            step.get("error").is_none(),
            "warning step has no error field"
        );
    }

    // 6. Seed is included when present --------------------------------

    #[test]
    fn seed_included_when_present() {
        let report = sample_flow();
        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert_eq!(v["seed"], 847_291_036);
    }

    // 7. Screenshot path included when present ------------------------

    #[test]
    fn screenshot_included_when_present() {
        let report = sample_flow();
        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert_eq!(
            v["screenshot"],
            ".golem/screenshots/login_flow_main_step5_error.png"
        );
    }

    // 8. format_suite_json includes aggregate stats -------------------

    #[test]
    fn suite_json_includes_aggregate_stats() {
        let suite = sample_suite();
        let json_str = format_suite_json(&suite).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let summary = &v["suite"];
        assert_eq!(summary["total"], 4);
        assert_eq!(summary["passed"], 3);
        assert_eq!(summary["failed"], 1);
        assert_eq!(summary["skipped"], 0);
        assert_eq!(summary["duration_ms"], 45300);
    }

    // 9. Suite JSON includes all flows --------------------------------

    #[test]
    fn suite_json_includes_all_flows() {
        let suite = sample_suite();
        let json_str = format_suite_json(&suite).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let flows = v["flows"].as_array().expect("flows should be an array");
        assert_eq!(flows.len(), 4);
        assert_eq!(flows[0]["name"], "login_flow");
        assert_eq!(flows[1]["name"], "signup_flow");
        assert_eq!(flows[2]["name"], "checkout_flow");
        assert_eq!(flows[3]["name"], "broken_flow");
    }

    // 10. None fields are omitted from JSON ---------------------------

    #[test]
    fn none_fields_omitted() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "minimal_flow".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 50)],
            warnings: vec![],
            duration_ms: 50,
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

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        // Optional fields should not appear at all
        assert!(v.get("seed").is_none(), "seed SHALL be absent");
        assert!(v.get("screenshot").is_none(), "screenshot SHALL be absent");
        assert!(v.get("device").is_none(), "device SHALL be absent");
        assert!(
            v.get("covered_axes").is_none(),
            "covered_axes SHALL be absent when empty"
        );

        // Step-level optional fields
        let step = &v["steps"][0];
        assert!(step.get("error").is_none(), "error SHALL be absent");
        assert!(step.get("warning").is_none(), "warning SHALL be absent");
    }

    // Coverage axes render as JSON array ------------------------------

    #[test]
    fn covered_axes_rendered_as_array() {
        let mut report = sample_flow();
        report.covered_axes = vec!["ios".into(), "v26".into(), "tablet".into()];

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let axes = v["covered_axes"]
            .as_array()
            .expect("covered_axes SHALL be an array");
        assert_eq!(axes.len(), 3);
        assert_eq!(axes[0], "ios");
        assert_eq!(axes[1], "v26");
        assert_eq!(axes[2], "tablet");
    }

    // 11. Skipped step has correct outcome ----------------------------

    #[test]
    fn skipped_step_outcome() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
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

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let step = &v["steps"][0];
        assert_eq!(step["outcome"], "skipped");
        assert_eq!(step["duration_ms"], 0);
    }

    // 12. Flow-level warnings appear in JSON --------------------------

    #[test]
    fn flow_warnings_in_json() {
        let report = sample_flow();
        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let warnings = v["warnings"].as_array().expect("warnings should be array");
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0], "element 'Promo' not found");
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
    fn json_includes_perf_snapshots() {
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

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert_eq!(
            v["perf_snapshots"][0]["label"], "login:iPhone_16:0",
            "SHALL contain snapshot label"
        );
        assert_eq!(
            v["perf_snapshots"][0]["memory_mb"], 142.5,
            "SHALL contain memory_mb value"
        );
    }

    // 13. Warning step renders its failure code -----------------------

    #[test]
    fn warning_step_includes_code() {
        let mut step = warning_step("assert_visible", "Promo", 15, "element not found");
        step.outcome = StepOutcome::Warning {
            message: "element not found".to_string(),
            code: golem_events::FailureCode::FlowStepTimeout,
        };
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "warn_flow".to_string(),
            success: true,
            step_results: vec![step],
            warnings: vec![],
            duration_ms: 15,
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

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let step = &v["steps"][0];
        // Warning renders code with Warning severity (W-domain).
        assert_eq!(step["code"], "WF408", "warning step SHALL carry its code");
    }

    // 14. Success/skipped steps omit code/error/warning ---------------

    #[test]
    fn success_and_skipped_steps_omit_optional_fields() {
        let report = FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "mixed_flow".to_string(),
            success: true,
            step_results: vec![success_step("launch", "", 10), skipped_step("tap", "X")],
            warnings: vec![],
            duration_ms: 10,
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

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        for idx in 0..2 {
            let step = &v["steps"][idx];
            assert!(step.get("code").is_none(), "step {idx} SHALL omit code");
            assert!(step.get("error").is_none(), "step {idx} SHALL omit error");
            assert!(
                step.get("warning").is_none(),
                "step {idx} SHALL omit warning"
            );
        }
    }

    // 15. first_failure_code suppressed when flow succeeded -----------

    #[test]
    fn flow_code_suppressed_when_success() {
        let mut report = sample_flow();
        // A success flow that still carries a failure code (e.g. a
        // recovered warning) SHALL NOT surface the code at flow level.
        report.success = true;
        report.first_failure_code = Some(golem_events::FailureCode::FlowStepTimeout);

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert!(v.get("code").is_none(), "successful flow SHALL omit code");
    }

    // 16. Wall-clock timestamps render at flow + step level -----------

    #[test]
    fn timestamps_rendered_when_present() {
        let mut report = sample_flow();
        report.started_at = Some("2026-06-15T12:00:00Z".to_string());
        report.finished_at = Some("2026-06-15T12:00:10Z".to_string());
        report.step_results[0].started_at = Some("2026-06-15T12:00:00Z".to_string());
        report.step_results[0].finished_at = Some("2026-06-15T12:00:00.120Z".to_string());

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert_eq!(v["started_at"], "2026-06-15T12:00:00Z");
        assert_eq!(v["finished_at"], "2026-06-15T12:00:10Z");
        assert_eq!(v["steps"][0]["started_at"], "2026-06-15T12:00:00Z");
        assert_eq!(v["steps"][0]["finished_at"], "2026-06-15T12:00:00.120Z");
    }

    // 17. os_major + skipped_reason render when present ---------------

    #[test]
    fn os_major_and_skipped_reason_rendered() {
        let mut report = sample_flow();
        report.success = true;
        report.os_major = Some(26);
        report.skipped_reason = Some("coverage group already satisfied".to_string());

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert_eq!(v["os_major"], 26, "os_major SHALL render when present");
        assert_eq!(
            v["skipped_reason"], "coverage group already satisfied",
            "skipped_reason SHALL render when present"
        );
    }

    // 18. Step retry/tree stats propagate to JSON ---------------------

    #[test]
    fn step_retry_and_tree_stats_rendered() {
        let mut report = sample_flow();
        report.step_results[0].retry_count = 3;
        report.step_results[0].tree_stats = golem_events::TreeStats {
            fetches: 7,
            min_nodes: 10,
            max_nodes: 152,
        };

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let step = &v["steps"][0];
        assert_eq!(step["retries"], 3, "retries SHALL propagate");
        assert_eq!(step["tree_fetches"], 7, "tree_fetches SHALL propagate");
        assert_eq!(step["tree_nodes"], 152, "tree_nodes SHALL propagate");
    }

    // 19. block name rendered, omitted when empty ---------------------

    #[test]
    fn step_block_name_render_and_omit() {
        let mut report = sample_flow();
        report.step_results[0].block_name = "main".to_string();
        // step 1 keeps an empty block name.

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert_eq!(
            v["steps"][0]["block"], "main",
            "non-empty block SHALL render"
        );
        assert!(
            v["steps"][1].get("block").is_none(),
            "empty block SHALL be omitted"
        );
    }

    // 20. Substeps render as an array, omitted when empty -------------

    #[test]
    fn substeps_rendered_and_omitted() {
        let mut report = sample_flow();
        report.step_results[1].substeps = vec![
            crate::SubstepDetail::Tap {
                point: golem_events::Point { x: 10, y: 20 },
                element_bounds: None,
            },
            crate::SubstepDetail::Backspace { count: 2 },
        ];

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let subs = v["steps"][1]["substeps"]
            .as_array()
            .expect("substeps SHALL be an array");
        assert_eq!(subs.len(), 2, "both substeps SHALL render");
        // step 0 has no substeps and SHALL omit the key entirely.
        assert!(
            v["steps"][0].get("substeps").is_none(),
            "empty substeps SHALL be omitted"
        );
    }

    // 21. Recordings render as objects, omitted when empty ------------

    #[test]
    fn recordings_rendered_and_omitted() {
        let mut report = sample_flow();
        report.recordings = vec![crate::RecordingEntry {
            block: "checkout".to_string(),
            iteration: 2,
            path: ".golem/recordings/checkout_2.mp4".to_string(),
        }];

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let recs = v["recordings"]
            .as_array()
            .expect("recordings SHALL be array");
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["block"], "checkout");
        assert_eq!(recs[0]["iteration"], 2);
        assert_eq!(recs[0]["path"], ".golem/recordings/checkout_2.mp4");
    }

    #[test]
    fn recordings_omitted_when_empty() {
        let report = sample_flow();
        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert!(
            v.get("recordings").is_none(),
            "empty recordings SHALL be omitted"
        );
    }

    // 22. Perf snapshot omits its None metrics ------------------------

    #[test]
    fn perf_snapshot_omits_none_metrics() {
        let mut report = sample_flow();
        report.perf_snapshots = vec![PerfSnapshot {
            label: "launch:0".into(),
            memory_mb: Some(100.0),
            cpu_percent: None,
            threads: None,
            file_descriptors: None,
            disk_mb: None,
            net_rx_kb: None,
            net_tx_kb: None,
            launch_ms: None,
            timestamp: "999".into(),
        }];

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let snap = &v["perf_snapshots"][0];
        assert_eq!(snap["memory_mb"], 100.0, "set metric SHALL render");
        assert_eq!(snap["timestamp"], "999", "timestamp SHALL render");
        assert!(
            snap.get("cpu_percent").is_none(),
            "None cpu_percent SHALL be omitted"
        );
        assert!(
            snap.get("threads").is_none(),
            "None threads SHALL be omitted"
        );
        assert!(
            snap.get("launch_ms").is_none(),
            "None launch_ms SHALL be omitted"
        );
    }

    // 23. Suite renders installs; omits when empty --------------------

    #[test]
    fn suite_json_renders_installs() {
        let mut suite = sample_suite();
        suite.installs = vec![InstallReport {
            app_name: "MyApp".to_string(),
            bundle_id: "com.example.app".to_string(),
            device_name: "iPhone 15 Pro".to_string(),
            os_major: Some(18),
            success: false,
            duration_ms: 4200,
            exit_code: Some(1),
            error: Some("provisioning profile missing".to_string()),
            code: None,
            started_at: None,
            finished_at: None,
            skipped: false,
            skip_reason: None,
        }];

        let json_str = format_suite_json(&suite).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let installs = v["installs"].as_array().expect("installs SHALL be array");
        assert_eq!(installs.len(), 1);
        let inst = &installs[0];
        assert_eq!(inst["app_name"], "MyApp");
        assert_eq!(inst["bundle_id"], "com.example.app");
        assert_eq!(inst["device"], "iPhone 15 Pro");
        assert_eq!(inst["os_major"], 18);
        assert_eq!(inst["success"], false);
        assert_eq!(inst["duration_ms"], 4200);
        assert_eq!(inst["exit_code"], 1);
        assert_eq!(inst["error"], "provisioning profile missing");
    }

    #[test]
    fn suite_json_renders_skipped_install() {
        let mut suite = sample_suite();
        suite.installs = vec![InstallReport {
            app_name: "MyApp".to_string(),
            bundle_id: "com.example.app".to_string(),
            device_name: "Pixel".to_string(),
            os_major: None,
            success: true,
            duration_ms: 0,
            exit_code: None,
            error: None,
            code: None,
            started_at: None,
            finished_at: None,
            skipped: true,
            skip_reason: Some("cache hit (git:abc1234)".to_string()),
        }];

        let json_str = format_suite_json(&suite).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");
        let inst = &v["installs"][0];
        assert_eq!(inst["skipped"], true);
        assert_eq!(inst["skip_reason"], "cache hit (git:abc1234)");
        assert_eq!(inst["success"], true);
    }

    #[test]
    fn suite_json_omits_installs_when_empty() {
        let suite = sample_suite();
        let json_str = format_suite_json(&suite).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert!(
            v.get("installs").is_none(),
            "empty installs SHALL be omitted"
        );
    }

    // 24. Install omits None optional fields --------------------------

    #[test]
    fn install_json_omits_none_fields() {
        let mut suite = sample_suite();
        suite.installs = vec![InstallReport {
            app_name: "MyApp".to_string(),
            bundle_id: "com.example.app".to_string(),
            device_name: "Pixel".to_string(),
            os_major: None,
            success: true,
            duration_ms: 1000,
            exit_code: None,
            error: None,
            code: None,
            started_at: None,
            finished_at: None,
            skipped: false,
            skip_reason: None,
        }];

        let json_str = format_suite_json(&suite).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let inst = &v["installs"][0];
        assert!(
            inst.get("os_major").is_none(),
            "None os_major SHALL be omitted"
        );
        assert!(
            inst.get("skipped").is_none(),
            "skipped=false SHALL be omitted"
        );
        assert!(
            inst.get("skip_reason").is_none(),
            "None skip_reason SHALL be omitted"
        );
        assert!(
            inst.get("exit_code").is_none(),
            "None exit_code SHALL be omitted"
        );
        assert!(inst.get("error").is_none(), "None error SHALL be omitted");
        assert!(
            inst.get("started_at").is_none(),
            "None started_at SHALL be omitted"
        );
    }

    // 25. Suite skipped count: all-steps-skipped flow counts as skipped

    #[test]
    fn suite_counts_all_steps_skipped_flow() {
        let mut suite = sample_suite();
        // A flow whose every step is Skipped but which is NOT a
        // coverage-group skip (skipped_reason = None) SHALL still count
        // toward the suite's skipped tally via the all-steps branch.
        suite.flows.push(FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "all_skipped_flow".to_string(),
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
        });

        let json_str = format_suite_json(&suite).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert_eq!(v["suite"]["total"], 5);
        assert_eq!(
            v["suite"]["skipped"], 1,
            "all-steps-skipped flow SHALL count as skipped"
        );
    }

    // 26. Coverage-group skip counts as skipped, not passed -----------

    #[test]
    fn suite_counts_coverage_group_skip() {
        let mut suite = sample_suite();
        suite.flows.push(FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "spared_flow".to_string(),
            success: true,
            step_results: vec![],
            warnings: vec![],
            duration_ms: 0,
            seed: None,
            screenshot_path: None,
            device_name: None,
            os_major: None,
            perf_snapshots: vec![],
            skipped_reason: Some("peer run satisfied coverage".to_string()),
            covered_axes: Vec::new(),
            recordings: Vec::new(),
            repeat: None,
            started_at: None,
            finished_at: None,
        });

        let json_str = format_suite_json(&suite).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        // 3 real passes from sample_suite remain; spared flow is not a pass.
        assert_eq!(
            v["suite"]["passed"], 3,
            "coverage skip SHALL NOT count as passed"
        );
        assert_eq!(
            v["suite"]["skipped"], 1,
            "coverage skip SHALL count as skipped"
        );
    }

    // 27. flake_summary populated when repeat is set ------------------

    #[test]
    fn suite_json_includes_flake_summary() {
        let mut suite = sample_suite();
        // Two repeat runs of the same flow+device: one pass, one fail =>
        // a flake entry SHALL surface.
        let mk = |success: bool, idx: u32| FlowReport {
            first_failure_code: None,
            a11y_audits: vec![],
            flow_name: "flaky".to_string(),
            success,
            step_results: vec![success_step("launch", "", 10)],
            warnings: vec![],
            duration_ms: 10,
            seed: None,
            screenshot_path: None,
            device_name: Some("Pixel".to_string()),
            os_major: None,
            perf_snapshots: vec![],
            skipped_reason: None,
            covered_axes: Vec::new(),
            recordings: Vec::new(),
            repeat: Some(golem_events::RepeatContext {
                index: idx,
                total: 2,
            }),
            started_at: None,
            finished_at: None,
        };
        suite.flows = vec![mk(true, 0), mk(false, 1)];

        let json_str = format_suite_json(&suite).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let flakes = v["flake_summary"]
            .as_array()
            .expect("flake_summary SHALL be array");
        assert_eq!(flakes.len(), 1);
        let e = &flakes[0];
        assert_eq!(e["flow"], "flaky (Pixel)");
        assert_eq!(e["passed"], 1);
        assert_eq!(e["failed"], 1);
        assert_eq!(e["total"], 2);
    }

    #[test]
    fn suite_json_omits_flake_summary_without_repeat() {
        let suite = sample_suite();
        let json_str = format_suite_json(&suite).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert!(
            v.get("flake_summary").is_none(),
            "single-run suite SHALL omit flake_summary"
        );
    }

    // 28. Empty suite produces zeroed summary -------------------------

    #[test]
    fn empty_suite_zeroed_summary() {
        let suite = SuiteReport::default();
        let json_str = format_suite_json(&suite).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let s = &v["suite"];
        assert_eq!(s["total"], 0);
        assert_eq!(s["passed"], 0);
        assert_eq!(s["failed"], 0);
        assert_eq!(s["skipped"], 0);
        let flows = v["flows"].as_array().expect("flows SHALL be array");
        assert!(flows.is_empty(), "empty suite SHALL have no flows");
    }

    #[test]
    fn json_omits_perf_when_empty() {
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

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert!(
            v.get("perf_snapshots").is_none(),
            "SHALL NOT contain perf_snapshots key when empty"
        );
    }
}
