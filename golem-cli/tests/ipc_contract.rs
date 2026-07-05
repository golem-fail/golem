//! Integration: config values thread across the client↔server IPC boundary
//! and reach execution intact.
//!
//! `golem run` (in-process) serialises the resolved config into a JSON
//! `config` object, ships it over the unix socket, and the server rebuilds
//! `SuiteConfig` from it. A field dropped or mis-decoded anywhere along that
//! path is a pure composition bug. `--seed` is a clean probe: it's forced
//! onto every flow and echoed back in each flow's report, so a correct value
//! in `results.json` proves the whole round-trip (CLI → config_json → wire →
//! parse_submit_config → SuiteConfig → execution → report → done → file).

mod common;

use common::{read_results_json, run_stub};

#[test]
fn seed_threads_through_wire_to_flow_report() {
    let seed: u64 = 424242;
    let seed_arg = seed.to_string();
    let r = run_stub("", &["--seed", &seed_arg]);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);

    let v = read_results_json(&r, "");
    let got = v["flows"][0]["seed"].as_u64();
    assert_eq!(
        got,
        Some(seed),
        "--seed SHALL thread config_json → server → execution into the flow report; json={v}"
    );
}
