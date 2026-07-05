//! Integration: `--output <fmt>` selects the right renderer.
//!
//! Guards the shipped-and-fixed bug where `--output toon` silently produced
//! human output because the client always spawned the human stream renderer
//! regardless of the requested format. Pure composition — every unit was
//! fine; only the wiring was wrong.

mod common;

use common::run_stub;

#[test]
fn toon_output_is_toon_on_stdout() {
    let r = run_stub("", &["--output", "toon"]);
    assert_eq!(
        r.code, 0,
        "a passing stub run SHALL exit 0; stderr={}",
        r.stderr
    );
    // The TOON schema header only appears in the TOON render. If human had
    // leaked to stdout (the historical bug) this would not hold.
    assert!(
        r.stdout.trim_start().starts_with("# F=flow-run"),
        "--output toon SHALL emit the TOON schema header on stdout; stdout={:?}",
        r.stdout
    );
    // And the human step stream SHALL NOT run at all — its per-step lines
    // (e.g. `assert_visible …`) must be absent from stderr. This is the
    // other half of the toon-printed-human regression guard: not just
    // "toon on stdout" but "no human anywhere".
    assert!(
        !r.stderr.contains("assert_visible"),
        "--output toon SHALL NOT stream human step lines to stderr; stderr={}",
        r.stderr
    );
}

#[test]
fn json_output_parses_as_json_on_stdout() {
    let r = run_stub("", &["--output", "json"]);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let v: serde_json::Value = serde_json::from_str(r.stdout.trim()).unwrap_or_else(|e| {
        panic!("--output json SHALL emit valid JSON on stdout: {e}; stdout={:?}", r.stdout)
    });
    assert_eq!(v["suite"]["passed"], 1, "one flow SHALL pass; json={v}");
    assert_eq!(v["suite"]["failed"], 0, "no flow SHALL fail; json={v}");
}

#[test]
fn human_output_streams_to_stderr_not_stdout() {
    let r = run_stub("", &["--output", "human"]);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.trim().is_empty(),
        "human output SHALL NOT go to stdout (it streams live to stderr); stdout={:?}",
        r.stdout
    );
    // stderr is never empty (the orchestrator prints setup banners there
    // regardless of format), so assert a human-STREAM marker specifically:
    // the per-step `assert_visible …` line is only emitted by the human
    // renderer, which runs only for human output.
    assert!(
        r.stderr.contains("assert_visible"),
        "human output SHALL stream per-step lines to stderr; stderr={}",
        r.stderr
    );
}
