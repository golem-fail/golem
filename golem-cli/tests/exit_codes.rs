//! Integration: the process exit code reflects flow outcomes end-to-end.

mod common;

use common::{read_results_json, run_stub};

#[test]
fn passing_flow_exits_zero_and_writes_results() {
    let r = run_stub("", &[]);
    assert_eq!(
        r.code, 0,
        "a passing flow SHALL exit 0; stderr={}",
        r.stderr
    );
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
    assert_eq!(
        r.code, 1,
        "a failing flow SHALL exit 1; stderr={}",
        r.stderr
    );
    let v = read_results_json(&r, "");
    assert_eq!(v["suite"]["failed"], 1, "json={v}");
}

// A browser step reaches the browser through the real CLI: args → orchestrator
// → runner dispatch → golem-browser. Unit tests can't see this path break,
// because each one compiles the browser feature in itself; only a run of the
// assembled binary notices it shipping without the feature forwarded.
//
// That is not hypothetical — it is how this test came to exist. `golem-cli`'s
// `browser` feature forwarded to the orchestrator but not the runner, so every
// suite planned happily and then reported EH501 on its first browser step, and
// nothing in the default test lane disagreed.
//
// Drives a real Chrome (nextest `live_` group); skipped where none exists.
// Gated on the feature because `--no-default-features` is the build that
// legitimately has no browser — there, H501 is the correct answer, not a bug.
#[cfg(feature = "browser")]
#[test]
fn live_browser_step_runs_through_the_cli() {
    if golem_browser::locate().is_err() {
        return;
    }
    let run = common::run_stub("", &["--flow", "browser.test.toml"]);
    assert!(
        !run.stdout.contains("EH501"),
        "the shipped binary SHALL carry browser support, got:\n{}",
        run.stdout
    );
    assert_eq!(
        run.code, 0,
        "the browser flow SHALL pass:\n{}\n{}",
        run.stdout, run.stderr
    );
}
