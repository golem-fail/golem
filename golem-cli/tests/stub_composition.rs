//! Integration: composition surfaces that only appear when the whole
//! CLI → orchestrator → runner → renderer → file pipeline runs.
//!
//! These are the cases a unit test structurally can't reach: the on-disk
//! shape `--trace` produces, whether the daemon and in-process paths agree,
//! and how coverage strategy fans a multi-axis flow out into FlowRuns.
//! Scripted-outcome fidelity only — anything needing real device behaviour
//! stays on the real-device sweep.
//!
//! nextest-SLOW by nature: each case drives a full run (some of them twice).

mod common;

use common::{read_results_json, run_stub, run_stub_opts, RunResult, StubOpts};

/// Every file written under the run's results directory, relative to it.
fn result_files(res: &RunResult) -> Vec<String> {
    let base = res.dir.join(".golem").join("results");
    let mut out = Vec::new();
    collect(&base, &base, &mut out);
    out.sort();
    out
}

fn collect(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.display().to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// --trace boundary capture
// ---------------------------------------------------------------------------

#[test]
fn trace_writes_a_screenshot_and_tree_at_every_boundary() {
    let r = run_stub("", &["--trace"]);
    assert_eq!(r.code, 0, "a traced run SHALL pass; stderr={}", r.stderr);

    let files = result_files(&r);
    let trace: Vec<&String> = files.iter().filter(|f| f.contains("/trace/")).collect();
    assert!(
        !trace.is_empty(),
        "--trace SHALL write boundary captures; files={files:?}"
    );

    // The first boundary is the pre-run state; every later one is named for
    // the step it follows, so a reader can line a frame up with a step.
    assert!(
        trace.iter().any(|f| f.ends_with("trace/000_start.png")),
        "the opening boundary SHALL be captured as 000_start.png; trace={trace:?}"
    );
    assert!(
        trace.iter().any(|f| f.ends_with("trace/000_start.json")),
        "each boundary SHALL pair the screenshot with a tree; trace={trace:?}"
    );
    assert!(
        trace
            .iter()
            .any(|f| f.contains("/trace/001_after_") && f.ends_with(".json")),
        "the boundary after step 1 SHALL be named for that step; trace={trace:?}"
    );

    // Every captured tree is real JSON, not a truncated write.
    for rel in trace.iter().filter(|f| f.ends_with(".json")) {
        let text = std::fs::read_to_string(r.dir.join(".golem/results").join(rel))
            .unwrap_or_else(|e| panic!("read {rel}: {e}"));
        serde_json::from_str::<serde_json::Value>(&text)
            .unwrap_or_else(|e| panic!("{rel} SHALL be valid JSON: {e}"));
    }
}

#[test]
fn trace_writes_a_step_sidecar_next_to_the_recording() {
    let r = run_stub("", &["--trace"]);
    let files = result_files(&r);

    // The sidecar is what maps a step to its offset inside the block's
    // recording — the thing a future `trace-extract` (#190) seeks with.
    let sidecar = files
        .iter()
        .find(|f| f.contains("/recordings/") && f.ends_with("_steps.json"))
        .unwrap_or_else(|| panic!("--trace SHALL write a step sidecar; files={files:?}"));

    let text = std::fs::read_to_string(r.dir.join(".golem/results").join(sidecar))
        .expect("sidecar SHALL be readable");
    let v: serde_json::Value = serde_json::from_str(&text).expect("sidecar SHALL be valid JSON");
    assert!(
        v.get("boundaries").is_some() || v.is_array(),
        "sidecar SHALL carry boundary offsets; got {v}"
    );
}

// ---------------------------------------------------------------------------
// daemon vs in-process parity
// ---------------------------------------------------------------------------

/// The fields a run's identity is made of. Timings and seeds differ run to
/// run by design, so parity is asserted on outcomes and structure.
fn outcome_shape(v: &serde_json::Value) -> serde_json::Value {
    let flows: Vec<serde_json::Value> = v["flows"]
        .as_array()
        .expect("flows SHALL be an array")
        .iter()
        .map(|f| {
            serde_json::json!({
                "name": f["name"],
                "success": f["success"],
                "device": f["device"],
                "steps": f["steps"].as_array().map(|s| s.iter().map(|st| {
                    serde_json::json!({ "action": st["action"], "outcome": st["outcome"] })
                }).collect::<Vec<_>>()),
            })
        })
        .collect();
    serde_json::json!({
        "total": v["suite"]["total"],
        "passed": v["suite"]["passed"],
        "failed": v["suite"]["failed"],
        "skipped": v["suite"]["skipped"],
        "install_blocked": v["suite"]["install_blocked"],
        "flows": flows,
    })
}

#[test]
fn daemon_and_in_process_runs_report_the_same_outcome() {
    let in_process = run_stub("", &[]);
    let via_daemon = run_stub_opts(
        "",
        &[],
        StubOpts {
            daemon: true,
            ..Default::default()
        },
    );

    assert_eq!(
        in_process.code, via_daemon.code,
        "both paths SHALL exit the same; in-process stderr={} daemon stderr={}",
        in_process.stderr, via_daemon.stderr
    );
    assert_eq!(
        outcome_shape(&read_results_json(&in_process, "")),
        outcome_shape(&read_results_json(&via_daemon, "")),
        "a daemon run SHALL report what the in-process run reports"
    );
}

#[test]
fn daemon_and_in_process_agree_on_a_failing_run() {
    // Parity has to hold for the failure path too — that's where the two
    // paths could diverge on exit code or on which flow is blamed.
    let in_process = run_stub("fail_on_runs = [1]", &[]);
    let via_daemon = run_stub_opts(
        "fail_on_runs = [1]",
        &[],
        StubOpts {
            daemon: true,
            ..Default::default()
        },
    );

    assert_eq!(in_process.code, 1, "the scripted failure SHALL exit 1");
    assert_eq!(via_daemon.code, 1, "…via the daemon too");
    assert_eq!(
        outcome_shape(&read_results_json(&in_process, "")),
        outcome_shape(&read_results_json(&via_daemon, "")),
        "both paths SHALL blame the same flow"
    );
}

// ---------------------------------------------------------------------------
// coverage-strategy fan-out
// ---------------------------------------------------------------------------

/// Run the two-axis coverage fixture under one strategy. No `--platform`
/// override: the point is what the flow's own axes expand into.
fn run_coverage(strategy: &str) -> (RunResult, serde_json::Value) {
    let r = run_stub_opts(
        "",
        &["--flow", "coverage.test.toml", "--coverage", strategy],
        StubOpts {
            no_platform_override: true,
            ..Default::default()
        },
    );
    assert_eq!(
        r.code, 0,
        "coverage={strategy} SHALL pass; stderr={}",
        r.stderr
    );
    let v = read_results_json(&r, "");
    (r, v)
}

#[test]
fn coverage_full_runs_every_axis() {
    let (_r, v) = run_coverage("full");
    assert_eq!(v["suite"]["total"], 2, "two os axes SHALL plan two runs");
    assert_eq!(v["suite"]["passed"], 2, "full SHALL run both; json={v}");
    assert_eq!(
        v["suite"]["skipped"], 0,
        "full SHALL skip nothing; json={v}"
    );
}

#[test]
fn coverage_one_stops_after_the_first_success() {
    let (_r, v) = run_coverage("one");
    assert_eq!(
        v["suite"]["total"], 2,
        "the plan SHALL still fan out to two runs; json={v}"
    );
    assert_eq!(
        (
            v["suite"]["passed"].as_u64(),
            v["suite"]["skipped"].as_u64()
        ),
        (Some(1), Some(1)),
        "one SHALL execute a single axis and spare the peer; json={v}"
    );

    // The spared run is reported, not silently dropped, and says why. Which
    // axis wins is scheduling-dependent, so the assertion is on the pair.
    let skipped: Vec<&serde_json::Value> = v["flows"]
        .as_array()
        .expect("flows array")
        .iter()
        .filter(|f| !f["skipped_reason"].is_null())
        .collect();
    assert_eq!(
        skipped.len(),
        1,
        "exactly one flow SHALL be spared; json={v}"
    );
    assert!(
        skipped[0]["skipped_reason"]
            .as_str()
            .is_some_and(|r| r.contains("coverage group")),
        "the spared flow SHALL name coverage as the cause; got {}",
        skipped[0]["skipped_reason"]
    );
    assert!(
        skipped[0]["success"].as_bool() == Some(true),
        "a coverage skip SHALL NOT read as a failure; json={v}"
    );
}

#[test]
fn coverage_smart_still_ticks_both_platforms() {
    // `smart` stops early only once every coverage box is ticked, and android
    // and ios are different boxes — so unlike `one` it runs both. Pinning this
    // keeps the two strategies from quietly collapsing into each other.
    let (_r, v) = run_coverage("smart");
    assert_eq!(
        v["suite"]["passed"], 2,
        "smart SHALL cover both platforms; json={v}"
    );
    assert_eq!(
        v["suite"]["skipped"], 0,
        "smart SHALL spare nothing here; json={v}"
    );
}
