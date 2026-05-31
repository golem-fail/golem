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
        b_flake.cmp(&a_flake)
            .then(b.failed.cmp(&a.failed))
            .then(a.flow.cmp(&b.flow))
    });
    out
}
