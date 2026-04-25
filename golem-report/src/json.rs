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
}

// ── Conversion helpers ──────────────────────────────────────────────

fn step_to_json(step: &StepReport) -> JsonStep {
    let (outcome, error, warning) = match &step.outcome {
        StepOutcome::Success => ("success".to_string(), None, None),
        StepOutcome::Warning(msg) => ("warning".to_string(), None, Some(msg.clone())),
        StepOutcome::Failed(msg) => ("failed".to_string(), Some(msg.clone()), None),
        StepOutcome::Skipped => ("skipped".to_string(), None, None),
    };

    JsonStep {
        index: step.global_step_index,
        block: step.block_name.clone(),
        action: step.action.clone(),
        target: step.target.clone(),
        outcome,
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

fn flow_to_json(report: &FlowReport) -> JsonFlow {
    JsonFlow {
        name: report.flow_name.clone(),
        success: report.success,
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
    let passed = report.flows.iter().filter(|f| f.success).count();
    let failed = report.flows.iter().filter(|f| !f.success).count();
    let skipped = report
        .flows
        .iter()
        .filter(|f| {
            f.skipped_reason.is_some() ||
            (!f.step_results.is_empty() && f.step_results
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
            outcome: StepOutcome::Failed(msg.to_string()),
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
            outcome: StepOutcome::Warning(msg.to_string()),
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
            flow_name: "login_flow".to_string(),
            success: false,
            step_results: vec![
                success_step("launch", "", 120),
                success_step("tap", "Sign Up", 45),
                failed_step("assert_visible", "Welcome", 10012, "timed out after 10000ms"),
            ],
            warnings: vec!["element 'Promo' not found".to_string()],
            duration_ms: 10200,
            seed: Some(847_291_036),
            screenshot_path: Some(
                ".golem/screenshots/login_flow_main_step5_error.png".to_string(),
            ),
            device_name: Some("iPhone 15 Pro".to_string()),
            os_major: None,
            perf_snapshots: vec![],
            skipped_reason: None,
            covered_axes: Vec::new(),
            started_at: None,
            finished_at: None,
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
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                    covered_axes: Vec::new(),
                    started_at: None,
                    finished_at: None,
                },
                FlowReport {
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
                    started_at: None,
                    finished_at: None,
                },
                FlowReport {
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
                    started_at: None,
                    finished_at: None,
                },
                FlowReport {
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
        assert!(failed.get("warning").is_none(), "failed step has no warning field");
    }

    // 5. Warning step includes warning message ------------------------

    #[test]
    fn warning_step_includes_warning() {
        let report = FlowReport {
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
            started_at: None,
            finished_at: None,
        };

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        let step = &v["steps"][0];
        assert_eq!(step["outcome"], "warning");
        assert_eq!(step["warning"], "element not found");
        // No error field on a warning step
        assert!(step.get("error").is_none(), "warning step has no error field");
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
            started_at: None,
            finished_at: None,
        };

        let json_str = format_flow_json(&report).expect("serialization should succeed");
        let v: Value = serde_json::from_str(&json_str).expect("valid JSON");

        // Optional fields should not appear at all
        assert!(v.get("seed").is_none(), "seed SHALL be absent");
        assert!(v.get("screenshot").is_none(), "screenshot SHALL be absent");
        assert!(v.get("device").is_none(), "device SHALL be absent");

        // Step-level optional fields
        let step = &v["steps"][0];
        assert!(step.get("error").is_none(), "error SHALL be absent");
        assert!(step.get("warning").is_none(), "warning SHALL be absent");
    }

    // 11. Skipped step has correct outcome ----------------------------

    #[test]
    fn skipped_step_outcome() {
        let report = FlowReport {
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

    #[test]
    fn json_omits_perf_when_empty() {
        let report = FlowReport {
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
