//! Integration: the process exit code reflects flow outcomes end-to-end.

mod common;

use common::{read_results_json, run_stub};

#[test]
fn passing_flow_exits_zero_and_writes_results() {
    let r = run_stub("", &[]);
    assert_eq!(r.code, 0, "a passing flow SHALL exit 0; stderr={}", r.stderr);
    // The composition writes a top-level results file even on the flat
    // (single-run) layout — a gap that shipped once for daemon mode.
    let v = read_results_json(&r, "");
    assert_eq!(v["suite"]["passed"], 1, "json={v}");
    assert_eq!(v["suite"]["failed"], 0, "json={v}");
}

#[test]
fn failing_flow_exits_one() {
    // Run 1 fails → the only run fails.
    let r = run_stub("fail_on_runs = [1]", &[]);
    assert_eq!(r.code, 1, "a failing flow SHALL exit 1; stderr={}", r.stderr);
    let v = read_results_json(&r, "");
    assert_eq!(v["suite"]["failed"], 1, "json={v}");
}
