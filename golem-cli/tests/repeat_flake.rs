//! Integration: `--repeat N` fans a flow out to N runs, each executes
//! independently, and the aggregate report + exit code reflect every run's
//! outcome. Guards the plan→execute fan-out, per-run outcome capture in the
//! client-accumulated report, the aggregate result file, and the exit code.
//!
//! Each parallel run presents as a distinct stub device (as real `--repeat`
//! runs do when they spread across the device pool), so the flake-summary's
//! per-(flow, device) grouping is not exercised here — that rendering is
//! covered by unit tests in `lib.rs`. This test targets the fan-out and
//! outcome aggregation.

mod common;

use common::{read_results_json, run_stub};

#[test]
fn repeat_fans_out_and_reports_each_outcome() {
    // Run 2 of 3 fails (stub serves the target-less tree on run 2).
    let r = run_stub("fail_on_runs = [2]", &["--repeat", "3"]);

    // A failing run makes the whole suite exit non-zero.
    assert_eq!(
        r.code, 1,
        "a failing repeat SHALL make the suite exit 1; stderr={}",
        r.stderr
    );

    // The aggregate results file carries all three repeat runs as flows:
    // two passed, one failed.
    let v = read_results_json(&r, "");
    assert_eq!(v["suite"]["total"], 3, "3 repeats SHALL produce 3 flows; json={v}");
    assert_eq!(v["suite"]["passed"], 2, "2 runs SHALL pass; json={v}");
    assert_eq!(v["suite"]["failed"], 1, "1 run SHALL fail; json={v}");
}

#[test]
fn repeat_all_pass_exits_zero() {
    let r = run_stub("", &["--repeat", "3"]);
    assert_eq!(r.code, 0, "all-passing repeats SHALL exit 0; stderr={}", r.stderr);
    let v = read_results_json(&r, "");
    assert_eq!(v["suite"]["total"], 3, "json={v}");
    assert_eq!(v["suite"]["passed"], 3, "all 3 runs SHALL pass; json={v}");
    assert_eq!(v["suite"]["failed"], 0, "no run SHALL fail; json={v}");
}
