//! Integration: `--max-concurrency N` caps how many FlowRuns execute at
//! once. The cap lives in the suite, not the shared `ResourceManager` (an
//! orchestrator daemon's manager is shared by every client), so the only
//! honest place to observe it is a real submit → execute cycle.
//!
//! `--repeat 2` gives two independent FlowRuns that the scheduler would
//! otherwise start together. The flow sleeps half a second so the two
//! outcomes are distinguishable at millisecond resolution: uncapped, the
//! windows overlap; under `--max-concurrency 1` they must not. That sleep
//! is why this test is nextest-SLOW — the fast fixture flow finishes inside
//! a millisecond, where both cases look identical.

mod common;

use common::{read_results_json, run_stub};

/// Parse an RFC-3339 stamp from the results file into epoch millis. Avoids
/// pulling a date crate into the test for two comparisons.
fn epoch_ms(stamp: &str) -> i64 {
    let (date, rest) = stamp.split_once('T').expect("stamp has a date and a time");
    let time = rest.trim_end_matches('Z');
    let d: Vec<i64> = date
        .split('-')
        .map(|p| p.parse().expect("date part"))
        .collect();
    let (hms, millis) = time.split_once('.').unwrap_or((time, "0"));
    let t: Vec<i64> = hms
        .split(':')
        .map(|p| p.parse().expect("time part"))
        .collect();
    // Days-from-civil (Howard Hinnant's algorithm) — the runs are same-day,
    // but a midnight boundary shouldn't invert the comparison.
    let (y, m, day) = (d[0], d[1], d[2]);
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ((days * 24 + t[0]) * 60 + t[1]) * 60_000 + t[2] * 1000 + millis.parse::<i64>().unwrap_or(0)
}

/// Both flow windows from a 2-repeat stub run, as epoch-ms pairs.
fn run_windows(extra: &[&str]) -> ((i64, i64), (i64, i64)) {
    let r = run_stub("", extra);
    assert_eq!(r.code, 0, "the stub suite SHALL pass; stderr={}", r.stderr);

    let v = read_results_json(&r, "");
    assert_eq!(
        v["suite"]["total"], 2,
        "2 repeats SHALL produce 2 flows; json={v}"
    );
    let win = |f: &serde_json::Value| {
        (
            epoch_ms(f["started_at"].as_str().expect("started_at")),
            epoch_ms(f["finished_at"].as_str().expect("finished_at")),
        )
    };
    (win(&v["flows"][0]), win(&v["flows"][1]))
}

#[test]
fn max_concurrency_one_serialises_flow_runs() {
    let ((a_start, a_end), (b_start, b_end)) = run_windows(&[
        "--flow",
        "slow.test.toml",
        "--repeat",
        "2",
        "--max-concurrency",
        "1",
    ]);

    let serialised = a_end <= b_start || b_end <= a_start;
    assert!(
        serialised,
        "--max-concurrency 1 SHALL NOT overlap two FlowRuns: \
         [{a_start}..{a_end}] vs [{b_start}..{b_end}]"
    );
}

/// The control the cap is measured against: without it the same two runs
/// overlap, so the assertion above is testing the flag and not the
/// scheduler happening to serialise anyway.
#[test]
fn uncapped_flow_runs_overlap() {
    let ((a_start, a_end), (b_start, b_end)) =
        run_windows(&["--flow", "slow.test.toml", "--repeat", "2"]);

    let overlaps = a_start < b_end && b_start < a_end;
    assert!(
        overlaps,
        "an uncapped suite SHALL run both FlowRuns concurrently: \
         [{a_start}..{a_end}] vs [{b_start}..{b_end}]"
    );
}
