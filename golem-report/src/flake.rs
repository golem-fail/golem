use crate::FlowReport;
use std::collections::BTreeMap;

/// One (flow, device) tally across N repeat runs.
pub struct FlakeEntry {
    pub flow: String,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub total: u32,
}

/// Tally pass/fail per (flow_name, device) across all repeat runs.
/// Returns empty when no entry has `repeat` set (single-run suites).
/// Sorted flakiest-first: flakes (passed > 0 && failed > 0) → most fails
/// → alphabetical.
pub fn build_summary(flows: &[FlowReport]) -> Vec<FlakeEntry> {
    if !flows.iter().any(|f| f.repeat.is_some()) {
        return Vec::new();
    }
    let mut acc: BTreeMap<String, FlakeEntry> = BTreeMap::new();
    for f in flows {
        let key = match &f.device_name {
            Some(d) => format!("{} ({})", f.flow_name, d),
            None => f.flow_name.clone(),
        };
        let entry = acc.entry(key.clone()).or_insert_with(|| FlakeEntry {
            flow: key,
            passed: 0,
            failed: 0,
            skipped: 0,
            total: 0,
        });
        entry.total += 1;
        if f.is_skipped() {
            entry.skipped += 1;
        } else if f.success {
            entry.passed += 1;
        } else {
            entry.failed += 1;
        }
    }
    let mut out: Vec<FlakeEntry> = acc.into_values().collect();
    out.sort_by(|a, b| {
        let a_flake = a.passed > 0 && a.failed > 0;
        let b_flake = b.passed > 0 && b.failed > 0;
        b_flake
            .cmp(&a_flake)
            .then(b.failed.cmp(&a.failed))
            .then(a.flow.cmp(&b.flow))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_events::RepeatContext;

    /// Build a minimal FlowReport with the knobs the flake tally reads.
    fn flow(
        name: &str,
        device: Option<&str>,
        success: bool,
        skipped_reason: Option<&str>,
        repeat: Option<RepeatContext>,
    ) -> FlowReport {
        FlowReport {
            flow_name: name.to_string(),
            success,
            skipped_reason: skipped_reason.map(|s| s.to_string()),
            step_results: Vec::new(),
            warnings: Vec::new(),
            duration_ms: 0,
            seed: None,
            screenshot_path: None,
            device_name: device.map(|d| d.to_string()),
            os_major: None,
            perf_snapshots: Vec::new(),
            started_at: None,
            finished_at: None,
            covered_axes: Vec::new(),
            recordings: Vec::new(),
            repeat,
            first_failure_code: None,
        }
    }

    const RC: RepeatContext = RepeatContext { index: 0, total: 3 };

    // 1. No entry carries a repeat context → single-run suite → empty tally.
    #[test]
    fn no_repeat_yields_empty() {
        let flows = vec![
            flow("a", None, true, None, None),
            flow("b", None, false, None, None),
        ];
        assert!(
            build_summary(&flows).is_empty(),
            "single-run suite SHALL produce no flake summary"
        );
    }

    // 2. Empty input → empty tally (no flow has a repeat context).
    #[test]
    fn empty_flows_yields_empty() {
        assert!(
            build_summary(&[]).is_empty(),
            "no flows SHALL produce no flake summary"
        );
    }

    // 3. A single flow repeated across runs accumulates passed/failed/total.
    #[test]
    fn tallies_pass_fail_across_runs() {
        let flows = vec![
            flow("login", None, true, None, Some(RC)),
            flow("login", None, false, None, Some(RC)),
            flow("login", None, true, None, Some(RC)),
        ];
        let out = build_summary(&flows);
        assert_eq!(
            out.len(),
            1,
            "one (flow,device) key SHALL collapse to one entry"
        );
        let e = &out[0];
        assert_eq!(
            e.flow, "login",
            "key SHALL be the bare flow name when no device"
        );
        assert_eq!(e.total, 3, "total SHALL count every run");
        assert_eq!(e.passed, 2, "passed SHALL count successes");
        assert_eq!(e.failed, 1, "failed SHALL count failures");
        assert_eq!(e.skipped, 0, "no run was skipped");
    }

    // 4. device_name present → key is "flow (device)" and partitions per device.
    #[test]
    fn device_name_partitions_key() {
        let flows = vec![
            flow("checkout", Some("Pixel 7a"), true, None, Some(RC)),
            flow("checkout", Some("iPhone 15"), false, None, Some(RC)),
        ];
        let out = build_summary(&flows);
        assert_eq!(out.len(), 2, "distinct devices SHALL form distinct keys");
        let names: Vec<&str> = out.iter().map(|e| e.flow.as_str()).collect();
        assert!(
            names.contains(&"checkout (Pixel 7a)"),
            "key SHALL embed device name: {names:?}"
        );
        assert!(
            names.contains(&"checkout (iPhone 15)"),
            "key SHALL embed device name: {names:?}"
        );
    }

    // 5. A coverage-group skip (success=true + reason) counts as skipped, not passed.
    #[test]
    fn skipped_run_classified_as_skipped() {
        let flows = vec![
            flow("smoke", None, true, Some("covered by peer"), Some(RC)),
            flow("smoke", None, true, None, Some(RC)),
        ];
        let out = build_summary(&flows);
        let e = &out[0];
        assert_eq!(e.skipped, 1, "success+reason run SHALL count as skipped");
        assert_eq!(e.passed, 1, "the bare success SHALL count as passed");
        assert_eq!(e.failed, 0, "no run failed");
        assert_eq!(e.total, 2, "total SHALL still count every run");
    }

    // 6. Install-precondition skip (success=false + reason) counts as failed.
    #[test]
    fn install_skip_classified_as_failed() {
        let flows = vec![flow("boot", None, false, Some("install failed"), Some(RC))];
        let out = build_summary(&flows);
        let e = &out[0];
        assert_eq!(e.failed, 1, "success=false + reason SHALL count as failed");
        assert_eq!(
            e.skipped, 0,
            "an install failure SHALL NOT count as skipped"
        );
        assert_eq!(e.passed, 0, "an install failure SHALL NOT count as passed");
    }

    // 7. Sort: a true flake (passed>0 && failed>0) outranks a pure-fail entry.
    #[test]
    fn flake_sorts_before_pure_failure() {
        let flows = vec![
            // pure failure: 3 fails, 0 passes — high fail count but not a flake.
            flow("steady_fail", None, false, None, Some(RC)),
            flow("steady_fail", None, false, None, Some(RC)),
            flow("steady_fail", None, false, None, Some(RC)),
            // flake: 1 pass, 1 fail.
            flow("flaky", None, true, None, Some(RC)),
            flow("flaky", None, false, None, Some(RC)),
        ];
        let out = build_summary(&flows);
        assert_eq!(
            out[0].flow, "flaky",
            "flakes SHALL sort before pure failures regardless of fail count"
        );
        assert_eq!(
            out[1].flow, "steady_fail",
            "pure failure SHALL follow the flake"
        );
    }

    // 8. Sort within same flake-ness: more failures first.
    #[test]
    fn more_failures_sort_first() {
        let flows = vec![
            // flake A: 1 fail
            flow("alpha", None, true, None, Some(RC)),
            flow("alpha", None, false, None, Some(RC)),
            // flake B: 2 fails
            flow("beta", None, true, None, Some(RC)),
            flow("beta", None, false, None, Some(RC)),
            flow("beta", None, false, None, Some(RC)),
        ];
        let out = build_summary(&flows);
        assert_eq!(out[0].flow, "beta", "more failures SHALL sort first");
        assert_eq!(out[1].flow, "alpha", "fewer failures SHALL sort after");
    }

    // 9. Sort tie-break: equal flake-ness and equal fails → alphabetical by key.
    #[test]
    fn equal_rank_sorts_alphabetically() {
        let flows = vec![
            flow("zeta", None, true, None, Some(RC)),
            flow("alpha", None, true, None, Some(RC)),
            flow("mid", None, true, None, Some(RC)),
        ];
        let out = build_summary(&flows);
        let order: Vec<&str> = out.iter().map(|e| e.flow.as_str()).collect();
        assert_eq!(
            order,
            vec!["alpha", "mid", "zeta"],
            "all-pass entries SHALL fall back to alphabetical order"
        );
    }

    // 10. A single flow with a repeat context activates the tally even when
    //     other flows have none (the any-repeat gate is suite-wide).
    #[test]
    fn one_repeat_flow_activates_tally() {
        let flows = vec![
            flow("with_repeat", None, true, None, Some(RC)),
            flow("no_repeat", None, true, None, None),
        ];
        let out = build_summary(&flows);
        assert_eq!(
            out.len(),
            2,
            "any flow carrying repeat SHALL include all flows in the tally"
        );
    }
}
