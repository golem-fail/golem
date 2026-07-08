//! Plan phase: parse flows, merge project defaults, expand device coverage,
//! emit `FlowRun`s with `DeviceSlot` requirements.
//!
//! **Invariant**: a `DeviceSlot` describes WHAT a device must satisfy —
//! UDIDs are never pinned at plan time. The runtime scheduler picks any
//! free matching device per slot. This lets a suite with 20 FlowRuns all
//! needing `ios:26` run on any 5 available sims (queue grabs whichever
//! frees up first).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(test)]
use golem_devices::DeviceState;
use golem_devices::{DeviceInfo, DeviceType, OsVersionSpec, Platform};
use golem_parser::mixin::expand_mixins;
#[cfg(test)]
use golem_parser::AppConfig;
use golem_parser::{parse_flow, CoverageStrategy, FlowFile, ProjectAppConfig};

use crate::install_matrix::{build_install_matrix, InstallEntry};

/// A parsed flow after project-level `[[apps]]` merge and mixin expansion.
#[derive(Debug, Clone)]
pub struct ParsedFlow {
    pub path: PathBuf,
    pub flow: FlowFile,
}

/// One execution of the flow. `slots.len() == 1` for single-device flows;
/// `> 1` for coordinated multi-device flows (chat-test pattern — deferred to
/// roadmap, the struct supports it).
///
/// `coverage_group` + `covers_boxes` wire this run into an execute-time
/// adaptive coverage pool. When set, the scheduler consults the group's
/// progress tracker before launching: once the group has met its stop
/// condition (max_runs reached or all boxes ticked), remaining members skip.
/// `covers_boxes` holds indices into the group's flat tick-box pool that
/// this run's slots are guaranteed to tick on success; the tracker unions
/// them in (plus any bonus boxes the picked device coincidentally satisfies).
#[derive(Debug, Clone)]
pub struct FlowRun {
    pub flow_idx: usize,
    pub slots: Vec<DeviceSlot>,
    pub coverage_group: Option<usize>,
    pub covers_boxes: Vec<usize>,
    /// Repeat-index for `--repeat N` invocations. 0..N-1. Always 0
    /// when N=1. Drives the per-FlowRun output dir override
    /// (`{output_dir}/run_{repeat_index+1}/`) and flows through into
    /// `FlowStarted` / `FlowFinished` events so renderers and the
    /// flake-summary tally can group by run.
    pub repeat_index: u32,
}

/// Coverage group: shared goal across a set of FlowRuns. Used by `One`
/// (stop after `max_runs` successful runs — typically 1) and `Smart`
/// (stop once every box in the pool has been ticked by at least one
/// picked device). `Min` and `Full` do not use groups; their `FlowRun`s
/// carry `coverage_group: None`.
///
/// `max_runs` is `Option<u32>` to leave room for JIT-N variants
/// (e.g. `coverage = "two"`) without another schema change; `None` means
/// "no run-count cap, stop on full tick-box coverage".
#[derive(Debug, Clone)]
pub struct CoverageGroup {
    pub flow_idx: usize,
    pub strategy: CoverageStrategy,
    /// Flat tick-box pool. Indices into this Vec are used by
    /// `FlowRun.covers_boxes` and by the progress tracker.
    pub boxes: Vec<DeviceSlot>,
    pub max_runs: Option<u32>,
}

/// Device-matching requirements, a.k.a. a "tick box". `None` means "any"
/// on that dimension — a device satisfies the slot when every `Some(_)`
/// field matches the device's actual attributes. Each slot represents
/// one coverage point: the flow is considered to cover this slot once a
/// matching device runs it.
///
/// `platform: None` means the flow is platform-agnostic (responsive-design
/// style) — any platform's device can tick the box so long as the apps are
/// installable there. The scheduler performs the install-matrix
/// intersection when picking.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceSlot {
    pub platform: Option<Platform>,
    pub os_version: Option<OsVersionSpec>,
    pub device_type: Option<DeviceType>,
    pub physical: Option<bool>,
    pub name: Option<String>,
    pub playstore: Option<bool>,
    pub accessibility_label: Option<String>,
    /// Optional boot-state requirement. Only set for the default
    /// empty-devices-block case, where the plan emits one partial box per
    /// currently-booted platform so the suite runs on whatever's up.
    pub booted: Option<bool>,
    /// Apps to install on this slot's device. Multi-app flows may pack 2+
    /// when their constraints permit (default sharing).
    pub apps: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedSuite {
    pub flows: Vec<ParsedFlow>,
    pub flow_runs: Vec<FlowRun>,
    /// Coverage groups referenced by `FlowRun.coverage_group` indices.
    /// Empty for suites whose flows all use Min or Full.
    pub coverage_groups: Vec<CoverageGroup>,
    pub install_matrix: Vec<InstallEntry>,
    /// Pre-formatted availability summary — one line per unique slot shape
    /// (e.g. `ios/v26/phone`) with counts of matching devices from the
    /// plan-time snapshot (total, booted, shutdown). Diagnostic; surfaces
    /// via the `SuitePlanned` event for `--verbose` display.
    pub device_availability: Vec<String>,
    /// Flows whose file could not be read, parsed, or mixin-expanded.
    /// Kept out of `flows` / `flow_runs` so a single bad file doesn't abort
    /// the suite — the scheduler converts each entry into a failed
    /// FlowReport.
    pub parse_failures: Vec<ParseFailure>,
    /// Pre-formatted lint warnings collected during parse. Empty when
    /// every flow is lint-clean. Surfaced via the `SuiteLint` event.
    pub lint_warnings: Vec<String>,
}

/// A flow file that failed to load before the Plan could expand it.
#[derive(Debug, Clone)]
pub struct ParseFailure {
    pub path: PathBuf,
    pub error: String,
}

/// Parse flows, merge project apps, discover a device snapshot (for
/// `:latest:N` expansion + feasibility), and emit the full suite plan.
///
/// `coverage_override` forces every flow's coverage strategy when set —
/// mirrors the CLI `--coverage` flag. Flow-level `[flow.options].coverage`
/// is ignored while the override is in effect.
pub async fn plan(
    flow_paths: &[PathBuf],
    project_apps: &[ProjectAppConfig],
    project_root: &Path,
    platform_override: Option<Platform>,
    coverage_override: Option<CoverageStrategy>,
    repeat: u32,
) -> Result<ParsedSuite> {
    let mut flows: Vec<ParsedFlow> = Vec::with_capacity(flow_paths.len());
    let mut parse_failures: Vec<ParseFailure> = Vec::new();
    let mut lint_warnings: Vec<String> = Vec::new();
    for path in flow_paths {
        match parse_one(path, project_apps, project_root) {
            Ok(flow) => {
                lint_warnings.extend(lint_warnings_for(path, &flow));
                flows.push(ParsedFlow {
                    path: path.clone(),
                    flow,
                });
            }
            Err(e) => parse_failures.push(ParseFailure {
                path: path.clone(),
                error: format!("{e:#}"),
            }),
        }
    }

    let snapshot = device_snapshot().await;

    let mut flow_runs: Vec<FlowRun> = Vec::new();
    let mut coverage_groups: Vec<CoverageGroup> = Vec::new();
    for (idx, pf) in flows.iter().enumerate() {
        flow_runs.extend(expand_flow(
            idx,
            &pf.flow,
            &snapshot,
            platform_override,
            coverage_override,
            &mut coverage_groups,
        )?);
    }

    // --repeat: replicate the entire (flow_runs + coverage_groups)
    // batch N times. Each replica is tagged with its `repeat_index`
    // and gets its own coverage groups (cloned with adjusted indices)
    // so smart/one strategies operate per-run for valid flake
    // comparison.
    let repeat = repeat.max(1);
    if repeat > 1 {
        let base_groups = coverage_groups.clone();
        let base_runs = flow_runs.clone();
        coverage_groups.clear();
        flow_runs.clear();
        for run_idx in 0..repeat {
            let group_base = coverage_groups.len();
            coverage_groups.extend(base_groups.iter().cloned());
            for run in &base_runs {
                let mut r = run.clone();
                r.repeat_index = run_idx;
                if let Some(g) = r.coverage_group {
                    r.coverage_group = Some(group_base + g);
                }
                flow_runs.push(r);
            }
        }
    }

    let install_matrix = build_install_matrix(&flows, &flow_runs, project_root);
    let device_availability = compute_device_availability(&flow_runs, &snapshot);

    Ok(ParsedSuite {
        flows,
        flow_runs,
        coverage_groups,
        install_matrix,
        device_availability,
        parse_failures,
        lint_warnings,
    })
}

/// Read + parse + merge + mixin-expand one flow file. Returns an error for
/// any of the three steps so `plan()` can bucket the failure.
fn parse_one(
    path: &Path,
    project_apps: &[ProjectAppConfig],
    project_root: &Path,
) -> Result<FlowFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading flow file {}", path.display()))?;
    let mut flow =
        parse_flow(&text).with_context(|| format!("parsing flow file {}", path.display()))?;
    merge_project_apps(&mut flow, project_apps);

    let flow_dir = path.parent().unwrap_or(Path::new("."));
    for block in &mut flow.block {
        block.steps = expand_mixins(&block.steps, flow_dir, project_root)
            .with_context(|| format!("expanding mixins in {}", path.display()))?;
    }

    Ok(flow)
}

/// Soft lints — warnings but not failures. A future `--validate` mode
/// would promote these to hard errors. Today they are surfaced through
/// the `SuiteLint` event so remote clients see them too.
fn lint_warnings_for(path: &Path, flow: &FlowFile) -> Vec<String> {
    let mut warnings: Vec<String> = golem_parser::validation::lint_within_no_op(flow)
        .into_iter()
        .map(|issue| {
            let block = issue.block_name.as_deref().unwrap_or("<unnamed>");
            format!(
                "{}:{}::{} `within` is set on action `{}` which doesn't \
                 consume it — only `scroll` and steps with `auto_scroll = true` \
                 use `within`. See README §swipe.",
                path.display(),
                block,
                issue.step_index,
                issue.action,
            )
        })
        .collect();
    warnings.extend(
        golem_parser::validation::lint_push_notification_phys(flow)
            .into_iter()
            .map(|issue| {
                let block = issue.block_name.as_deref().unwrap_or("<unnamed>");
                format!(
                    "{}:{}::{} `push_notification` (app=`{}`) targets an app \
                     that may run on physical hardware. The action is sim/emu-\
                     only on both platforms and will error at runtime there. \
                     Branch on `_hardware` and use `*_http` to your APNS/FCM \
                     backend for phys delivery. See README §push_notification.",
                    path.display(),
                    block,
                    issue.step_index,
                    issue.app_name,
                )
            }),
    );
    warnings
}

/// For each unique slot shape across all FlowRuns, count how many devices
/// in the plan-time snapshot match that shape. Emit one pre-formatted line
/// per shape: `<shape> — <n> device(s) (<booted> booted[, <shutdown>
/// shutdown][, <physical> physical])`.
///
/// **Semantics note:** the total count is *eligible* devices, not
/// *concurrently-usable* devices. A shutdown sim counts once but only
/// becomes usable once booted; physical devices are each single-user. The
/// scheduler's parallel capacity is therefore bounded by booted + any
/// boot-on-demand budget — not by this total. Read the line as "plan saw N
/// matching devices" not "N flows can run in parallel".
fn compute_device_availability(flow_runs: &[FlowRun], snapshot: &[DeviceInfo]) -> Vec<String> {
    use std::collections::HashSet;

    // Collect unique slot shapes (one per DeviceSlot, deduped by signature).
    let mut seen: HashSet<String> = HashSet::new();
    let mut shapes: Vec<&DeviceSlot> = Vec::new();
    for run in flow_runs {
        for slot in &run.slots {
            let sig = slot_signature(slot);
            if seen.insert(sig) {
                shapes.push(slot);
            }
        }
    }

    shapes
        .iter()
        .map(|slot| {
            let matches: Vec<&DeviceInfo> = snapshot
                .iter()
                .filter(|d| device_matches_slot(d, slot))
                .collect();
            let booted = matches
                .iter()
                .filter(|d| d.state == golem_devices::DeviceState::Booted)
                .count();
            let shutdown = matches
                .iter()
                .filter(|d| d.state == golem_devices::DeviceState::Shutdown)
                .count();
            let physical = matches.iter().filter(|d| d.physical).count();
            let label = shape_label(slot);
            let mut parts = vec![format!("{} booted", booted)];
            if shutdown > 0 {
                parts.push(format!("{shutdown} shutdown"));
            }
            if physical > 0 {
                parts.push(format!("{physical} physical"));
            }
            format!(
                "{label} — {n} device(s) ({details})",
                n = matches.len(),
                details = parts.join(", "),
            )
        })
        .collect()
}

fn slot_signature(slot: &DeviceSlot) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        slot.platform,
        slot.os_version,
        slot.device_type,
        slot.physical,
        slot.name,
        slot.playstore,
        slot.accessibility_label,
        slot.booted,
    )
}

/// Render a slot's shape-only label: `platform/version/type/physical/name`.
/// Excludes `apps`. Used for availability counts where apps are irrelevant.
pub fn shape_label(slot: &DeviceSlot) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(match slot.platform {
        Some(p) => p.to_string(),
        None => "any-platform".to_string(),
    });
    if let Some(spec) = &slot.os_version {
        match spec {
            OsVersionSpec::Exact { major, .. } => parts.push(format!("v{major}")),
            OsVersionSpec::Minimum { major, .. } => parts.push(format!("v{major}+")),
            OsVersionSpec::Latest { count, .. } => parts.push(if *count > 1 {
                format!("latest:{count}")
            } else {
                "latest".to_string()
            }),
        }
    }
    if let Some(t) = &slot.device_type {
        parts.push(t.to_string());
    }
    if let Some(phys) = slot.physical {
        parts.push(if phys {
            "physical".into()
        } else {
            "sim".into()
        });
    }
    if let Some(n) = &slot.name {
        parts.push(format!("name={n}"));
    }
    if slot.booted == Some(true) {
        parts.push("booted".into());
    }
    parts.join("/")
}

/// Render a slot's full label: shape + `apps=[...]` suffix.
/// Used for flow-run summary lines where apps matter.
pub fn describe_slot(slot: &DeviceSlot) -> String {
    let shape = shape_label(slot);
    let apps = if slot.apps.is_empty() {
        "no apps".to_string()
    } else {
        format!("apps=[{}]", slot.apps.join(","))
    };
    format!("{shape} {apps}")
}

/// Check whether a snapshot device satisfies a slot's requirements.
///
/// Used at plan time for availability counts and at execute time by the
/// scheduler's device picker (`find_available_device` in `golem-cli`) so
/// both paths agree on what "a device matching this slot" means.
///
/// Matched fields: `platform`, `os_version` (Exact = equal major; Minimum
/// = ≥ major; Latest = any — already expanded upstream), `device_type`,
/// `physical`, `name`. `playstore` and `accessibility_label` are not on
/// `DeviceInfo`, so we accept any device when those are set — companion
/// or runtime checks handle them.
pub fn device_matches_slot(device: &DeviceInfo, slot: &DeviceSlot) -> bool {
    if let Some(p) = slot.platform {
        if device.platform != p {
            return false;
        }
    }
    match &slot.os_version {
        Some(OsVersionSpec::Exact { major, .. }) => {
            if device.os_major != *major {
                return false;
            }
        }
        Some(OsVersionSpec::Minimum { major, .. }) => {
            if device.os_major < *major {
                return false;
            }
        }
        Some(OsVersionSpec::Latest { .. }) | None => {}
    }
    if let Some(dt) = &slot.device_type {
        if &device.device_type != dt {
            return false;
        }
    }
    if let Some(phys) = slot.physical {
        if device.physical != phys {
            return false;
        }
    }
    if let Some(n) = &slot.name {
        if &device.name != n {
            return false;
        }
    }
    if let Some(want_booted) = slot.booted {
        let is_booted = device.state == golem_devices::DeviceState::Booted;
        if is_booted != want_booted {
            return false;
        }
    }
    true
}

/// Discover devices from both platforms. Best-effort — errors are soft,
/// treated as "platform has no devices". Used for `:latest:N` version
/// expansion and for knowing available major versions; we do NOT pin UDIDs.
async fn device_snapshot() -> Vec<DeviceInfo> {
    let mut all = golem_devices::ios::discover_ios_devices()
        .await
        .unwrap_or_default();
    if let Ok(android) = golem_devices::android::discover_android_devices().await {
        all.extend(android);
    }
    all
}

/// Fill in missing flow-level app fields from the matching project-level
/// `[[apps]]` entry by name. Flow values always win; project fills gaps only.
pub fn merge_project_apps(flow: &mut FlowFile, project_apps: &[ProjectAppConfig]) {
    for flow_app in &mut flow.flow.apps {
        if let Some(proj) = project_apps.iter().find(|p| p.name == flow_app.name) {
            if flow_app.bundle.is_none() {
                flow_app.bundle = proj.bundle.clone();
            }
            if flow_app.install_script.is_none() {
                flow_app.install_script = proj.install_script.clone();
            }
            if flow_app.install_timeout_ms.is_none() {
                flow_app.install_timeout_ms = proj.install_timeout_ms;
            }
            if flow_app.devices.is_empty() {
                flow_app.devices = proj.devices.clone();
            }
        }
    }
}

mod expand;

use expand::expand_flow;
#[cfg(test)]
use expand::{
    cover_and_union, default_any_booted_requirements, expand_app_requirements,
    expand_hardware_entries, expand_jit, expand_os_pairs, expand_type_entries,
    reduce_app_reqs_via_cover, slot_compatible_with, union_requirements, AppRequirement,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_flow(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).expect("create() SHALL succeed");
        f.write_all(contents.as_bytes())
            .expect("value SHALL be present");
        path
    }

    fn project_app(name: &str, bundle: &str, script: Option<&str>) -> ProjectAppConfig {
        ProjectAppConfig {
            name: name.into(),
            bundle: Some(bundle.into()),
            devices: Vec::new(),
            install_script: script.map(|s| golem_parser::InstallScriptValue::Single(s.into())),
            install_timeout_ms: None,
        }
    }

    // --repeat fan-out: every FlowRun replicated N times, each tagged
    // with its repeat_index. Coverage groups are cloned per replica so
    // smart/one progress stays per-run (each repeat is its own
    // independent execution for flake comparison).
    #[tokio::test]
    async fn plan_repeat_fans_out_flow_runs() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios"
        "#,
        );
        let apps = vec![project_app("app", "com.app", Some("scripts/i.sh"))];

        let single = plan(
            std::slice::from_ref(&flow),
            &apps,
            tmp.path(),
            None,
            None,
            1,
        )
        .await
        .expect("async operation SHALL succeed");
        let base_runs = single.flow_runs.len();
        assert!(
            base_runs > 0,
            "preflight: single-run plan SHALL emit at least one FlowRun"
        );

        let repeated = plan(&[flow], &apps, tmp.path(), None, None, 3)
            .await
            .expect("async operation SHALL succeed");
        assert_eq!(
            repeated.flow_runs.len(),
            base_runs * 3,
            "repeat=3 SHALL emit 3× as many FlowRuns",
        );

        // repeat_index spread: 0..3, each appearing `base_runs` times.
        let mut counts = [0usize; 3];
        for r in &repeated.flow_runs {
            assert!((r.repeat_index as usize) < 3, "repeat_index in range");
            counts[r.repeat_index as usize] += 1;
        }
        assert_eq!(counts, [base_runs; 3]);
    }

    #[tokio::test]
    async fn plan_repeat_clones_coverage_groups() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        // `coverage = "smart"` ensures a coverage group is emitted.
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [flow.options]
            coverage = "smart"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios:latest"
        "#,
        );
        let apps = vec![project_app("app", "com.app", Some("scripts/i.sh"))];

        let single = plan(
            std::slice::from_ref(&flow),
            &apps,
            tmp.path(),
            None,
            None,
            1,
        )
        .await
        .expect("async operation SHALL succeed");
        let base_groups = single.coverage_groups.len();
        if base_groups == 0 {
            // Device snapshot may not have an ios device at plan-time —
            // bail rather than make a misleading assertion.
            return;
        }

        let repeated = plan(&[flow], &apps, tmp.path(), None, None, 3)
            .await
            .expect("async operation SHALL succeed");
        assert_eq!(
            repeated.coverage_groups.len(),
            base_groups * 3,
            "each repeat batch SHALL get its own coverage groups so smart/one progress is per-run",
        );

        // Each FlowRun's `coverage_group` index should land inside its
        // own batch's group range — i.e. groups grow by `base_groups`
        // per repeat index.
        for r in &repeated.flow_runs {
            if let Some(g) = r.coverage_group {
                let batch_start = r.repeat_index as usize * base_groups;
                let batch_end = batch_start + base_groups;
                assert!(
                    g >= batch_start && g < batch_end,
                    "FlowRun repeat={} coverage_group={} SHALL land in [{batch_start},{batch_end})",
                    r.repeat_index,
                    g,
                );
            }
        }
    }

    #[tokio::test]
    async fn plan_single_app_single_ios_run() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios"
        "#,
        );
        let apps = vec![project_app("app", "com.app", Some("scripts/i.sh"))];
        let suite = plan(&[flow], &apps, tmp.path(), None, None, 1)
            .await
            .expect("async operation SHALL succeed");
        assert_eq!(suite.flow_runs.len(), 1);
        assert_eq!(suite.flow_runs[0].slots.len(), 1);
        assert_eq!(suite.flow_runs[0].slots[0].platform, Some(Platform::Ios));
        assert!(
            suite.flow_runs[0].slots[0].os_version.is_none(),
            "'os = \"ios\"' alone SHALL have no specific version requirement"
        );
        assert_eq!(suite.flow_runs[0].slots[0].apps, vec!["app".to_string()]);
    }

    #[tokio::test]
    async fn plan_ios_18_exact_version_captured() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios:18"
        "#,
        );
        let apps = vec![project_app("app", "com.app", None)];
        let suite = plan(&[flow], &apps, tmp.path(), None, None, 1)
            .await
            .expect("async operation SHALL succeed");
        assert_eq!(suite.flow_runs.len(), 1);
        let spec = suite.flow_runs[0].slots[0]
            .os_version
            .as_ref()
            .expect("as_ref() SHALL succeed");
        assert!(
            matches!(spec, OsVersionSpec::Exact { major: 18, .. }),
            "os = \"ios:18\" SHALL populate os_version with Exact(18)"
        );
    }

    #[tokio::test]
    async fn plan_os_list_fans_out_per_version() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = ["ios:18", "ios:26"]
        "#,
        );
        let apps = vec![project_app("app", "com.app", None)];
        let suite = plan(&[flow], &apps, tmp.path(), None, None, 1)
            .await
            .expect("async operation SHALL succeed");
        assert_eq!(
            suite.flow_runs.len(),
            2,
            "os list of 2 entries SHALL produce 2 FlowRuns"
        );
    }

    #[tokio::test]
    async fn plan_type_list_fans_out() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios"
            type = ["phone", "tablet"]
        "#,
        );
        let apps = vec![project_app("app", "com.app", None)];
        let suite = plan(&[flow], &apps, tmp.path(), None, None, 1)
            .await
            .expect("async operation SHALL succeed");
        assert_eq!(suite.flow_runs.len(), 2);
        let mut types: Vec<_> = suite
            .flow_runs
            .iter()
            .map(|r| r.slots[0].device_type.expect("value SHALL be present"))
            .collect();
        types.sort_by_key(|t| t.to_string());
        assert_eq!(types, vec![DeviceType::Phone, DeviceType::Tablet]);
    }

    #[tokio::test]
    async fn plan_two_apps_same_constraint_share_slot() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "a"
            [[flow.apps.devices]]
            os = "ios"
            [[flow.apps]]
            name = "b"
            [[flow.apps.devices]]
            os = "ios"
        "#,
        );
        let apps = vec![
            project_app("a", "com.a", None),
            project_app("b", "com.b", None),
        ];
        let suite = plan(&[flow], &apps, tmp.path(), None, None, 1)
            .await
            .expect("async operation SHALL succeed");
        assert_eq!(suite.flow_runs.len(), 1);
        assert_eq!(
            suite.flow_runs[0].slots.len(),
            1,
            "apps with identical constraints SHALL share a slot"
        );
        let mut slot_apps = suite.flow_runs[0].slots[0].apps.clone();
        slot_apps.sort();
        assert_eq!(slot_apps, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn plan_two_apps_different_platforms_produce_two_slots() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "client"
            [[flow.apps.devices]]
            os = "ios"
            [[flow.apps]]
            name = "supplier"
            [[flow.apps.devices]]
            os = "android"
        "#,
        );
        let apps = vec![
            project_app("client", "com.c", None),
            project_app("supplier", "com.s", None),
        ];
        let suite = plan(&[flow], &apps, tmp.path(), None, None, 1)
            .await
            .expect("async operation SHALL succeed");
        assert_eq!(
            suite.flow_runs.len(),
            1,
            "chat-test pattern SHALL emit one FlowRun (not two)"
        );
        assert_eq!(
            suite.flow_runs[0].slots.len(),
            2,
            "incompatible per-app constraints SHALL produce separate slots"
        );
    }

    #[tokio::test]
    async fn plan_client_multi_version_supplier_single_platform() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "client"
            [[flow.apps.devices]]
            os = ["ios:18", "ios:26"]
            [[flow.apps]]
            name = "supplier"
            [[flow.apps.devices]]
            os = "android"
        "#,
        );
        let apps = vec![
            project_app("client", "com.c", None),
            project_app("supplier", "com.s", None),
        ];
        let suite = plan(&[flow], &apps, tmp.path(), None, None, 1)
            .await
            .expect("async operation SHALL succeed");
        assert_eq!(
            suite.flow_runs.len(),
            2,
            "client coverage fan-out SHALL produce 2 FlowRuns"
        );
        for run in &suite.flow_runs {
            assert_eq!(
                run.slots.len(),
                2,
                "each FlowRun SHALL coordinate client + supplier slots"
            );
        }
    }

    #[tokio::test]
    async fn plan_platform_override_forces_single_platform() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios"
            [[flow.apps.devices]]
            os = "android"
        "#,
        );
        let apps = vec![project_app("app", "com.app", None)];
        let suite = plan(&[flow], &apps, tmp.path(), Some(Platform::Android), None, 1)
            .await
            .expect("async operation SHALL succeed");
        for run in &suite.flow_runs {
            for slot in &run.slots {
                assert_eq!(
                    slot.platform,
                    Some(Platform::Android),
                    "platform override SHALL force every slot to the chosen platform"
                );
            }
        }
    }

    #[tokio::test]
    async fn plan_merges_project_bundle_when_flow_omits() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "a"
            [[flow.apps.devices]]
            os = "ios"
        "#,
        );
        let apps = vec![project_app("a", "com.project.a", Some("scripts/a.sh"))];
        let suite = plan(&[flow], &apps, tmp.path(), None, None, 1)
            .await
            .expect("async operation SHALL succeed");
        let app = &suite.flows[0].flow.flow.apps[0];
        assert_eq!(app.bundle.as_deref(), Some("com.project.a"));
    }

    #[tokio::test]
    async fn plan_drops_unreferenced_project_apps() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "b"
            [[flow.apps.devices]]
            os = "ios"
        "#,
        );
        let apps = vec![
            project_app("a", "com.a", Some("scripts/a.sh")),
            project_app("b", "com.b", Some("scripts/b.sh")),
        ];
        let suite = plan(&[flow], &apps, tmp.path(), None, None, 1)
            .await
            .expect("async operation SHALL succeed");
        let bundles: Vec<_> = suite
            .install_matrix
            .iter()
            .map(|e| e.bundle_id.clone())
            .collect();
        assert_eq!(bundles, vec!["com.b".to_string()]);
    }

    #[tokio::test]
    async fn plan_bad_flow_moves_to_parse_failures_not_error() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let good = write_flow(
            tmp.path(),
            "good.test.toml",
            r#"
            [flow]
            name = "good"
            [[flow.apps]]
            name = "a"
            [[flow.apps.devices]]
            os = "ios"
        "#,
        );
        let missing = tmp.path().join("does-not-exist.test.toml");
        let bad_syntax = write_flow(tmp.path(), "bad.test.toml", "this is not [[[valid toml");
        let apps = vec![project_app("a", "com.a", None)];
        let suite = plan(
            &[good, missing.clone(), bad_syntax.clone()],
            &apps,
            tmp.path(),
            None,
            None,
            1,
        )
        .await
        .expect("plan SHALL succeed even when some files fail to parse");
        assert_eq!(suite.flows.len(), 1, "only the parseable flow SHALL remain");
        assert_eq!(
            suite.parse_failures.len(),
            2,
            "both the missing file and the bad-syntax file SHALL be in parse_failures",
        );
        let paths: Vec<_> = suite
            .parse_failures
            .iter()
            .map(|f| f.path.clone())
            .collect();
        assert!(paths.contains(&missing));
        assert!(paths.contains(&bad_syntax));
    }

    #[tokio::test]
    async fn plan_flow_only_install_script_still_in_matrix() {
        let tmp = TempDir::new().expect("new() SHALL succeed");
        let flow = write_flow(
            tmp.path(),
            "f.test.toml",
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "a"
            bundle = "com.flow.a"
            install_script = "scripts/flow-only.sh"
            [[flow.apps.devices]]
            os = "ios"
        "#,
        );
        let suite = plan(&[flow], &[], tmp.path(), None, None, 1)
            .await
            .expect("async operation SHALL succeed");
        assert_eq!(suite.install_matrix.len(), 1);
        assert!(suite.install_matrix[0]
            .script_path
            .ends_with("scripts/flow-only.sh"));
    }

    // ---------------------------------------------------------------
    // Tick-box model — direct tests with injected snapshots.
    //
    // These exercise `expand_app_requirements` and `expand_min` without
    // going through the public `plan()` entry point, so we can control
    // the device snapshot and keep tests deterministic regardless of
    // the host's actual simulators.
    // ---------------------------------------------------------------

    fn mk_device(
        name: &str,
        udid: &str,
        platform: Platform,
        major: u32,
        dt: DeviceType,
        booted: bool,
    ) -> DeviceInfo {
        DeviceInfo {
            name: name.into(),
            udid: udid.into(),
            platform,
            device_type: dt,
            os_major: major,
            os_version: format!("{major}.0"),
            state: if booted {
                DeviceState::Booted
            } else {
                DeviceState::Shutdown
            },
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        }
    }

    fn mk_app_with_devices(name: &str, devices: Vec<golem_parser::DeviceConstraint>) -> AppConfig {
        AppConfig {
            name: name.into(),
            bundle: Some(format!("com.{name}")),
            install_script: None,
            install_timeout_ms: None,
            devices,
        }
    }

    fn dc(os: Option<Vec<&str>>, types: Option<Vec<&str>>) -> golem_parser::DeviceConstraint {
        use golem_parser::{DeviceConstraint, StringOrVec};
        DeviceConstraint {
            os: os.map(|v| {
                if v.len() == 1 {
                    StringOrVec::Single(v[0].into())
                } else {
                    StringOrVec::Multiple(v.iter().map(|s| s.to_string()).collect())
                }
            }),
            device_type: types.map(|v| {
                if v.len() == 1 {
                    StringOrVec::Single(v[0].into())
                } else {
                    StringOrVec::Multiple(v.iter().map(|s| s.to_string()).collect())
                }
            }),
            name: None,
            accessibility_label: None,
            hardware: None,
            playstore: None,
            booted: None,
            expand: None,
        }
    }

    // Full strategy — Cartesian product. 2 os × 2 types = 4 boxes.
    #[test]
    fn expand_app_requirements_full_cartesian() {
        let app = mk_app_with_devices(
            "a",
            vec![dc(
                Some(vec!["ios:18", "ios:26"]),
                Some(vec!["phone", "tablet"]),
            )],
        );
        let snap: Vec<DeviceInfo> = Vec::new();
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Full)
            .expect("expand_app_requirements() SHALL succeed");
        assert_eq!(
            reqs.len(),
            4,
            "Full SHALL emit full Cartesian (2 os × 2 types = 4 boxes)"
        );
        // Every box should be fully pinned on both axes.
        for r in &reqs {
            assert!(r.os_version.is_some());
            assert!(r.device_type.is_some());
        }
    }

    // Min strategy — both axes multi-valued → partial-axis emission.
    // 2 os + 2 type = 4 partial boxes (NOT 4 fully-pinned Cartesian combos).
    #[test]
    fn expand_app_requirements_min_partial_axes() {
        let app = mk_app_with_devices(
            "a",
            vec![dc(
                Some(vec!["ios:18", "ios:26"]),
                Some(vec!["phone", "tablet"]),
            )],
        );
        let snap: Vec<DeviceInfo> = Vec::new();
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min)
            .expect("expand_app_requirements() SHALL succeed");
        assert_eq!(reqs.len(), 4);
        // 2 os-only boxes (device_type=None) + 2 type-only boxes (os_version=None).
        let os_only = reqs
            .iter()
            .filter(|r| r.os_version.is_some() && r.device_type.is_none())
            .count();
        let type_only = reqs
            .iter()
            .filter(|r| r.os_version.is_none() && r.device_type.is_some())
            .count();
        assert_eq!(os_only, 2, "SHALL emit 2 partial os boxes");
        assert_eq!(type_only, 2, "SHALL emit 2 partial type boxes");
    }

    // Min with only one multi-valued axis → collapses to Cartesian-like (same as Full).
    #[test]
    fn expand_app_requirements_min_single_multi_axis_fully_pinned() {
        let app = mk_app_with_devices(
            "a",
            vec![dc(
                Some(vec!["ios:latest"]),      // single os
                Some(vec!["phone", "tablet"]), // 2 types
            )],
        );
        let snap = vec![mk_device(
            "iPad",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Tablet,
            true,
        )];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min)
            .expect("expand_app_requirements() SHALL succeed");
        assert_eq!(
            reqs.len(),
            2,
            "single-multi-axis SHALL collapse to 2 fully-pinned boxes"
        );
        for r in &reqs {
            assert!(r.os_version.is_some(), "os SHALL remain pinned");
            assert!(r.device_type.is_some(), "type SHALL remain pinned");
        }
    }

    // Responsive-design: 2 platforms × 2 types under Min → 4 partial boxes,
    // 2 devices cover all (iOS-phone + Android-tablet).
    #[test]
    fn expand_min_responsive_design_two_devices_cover_four_boxes() {
        let app = mk_app_with_devices(
            "a",
            vec![dc(
                Some(vec!["ios:latest", "android:latest"]),
                Some(vec!["phone", "tablet"]),
            )],
        );
        let snap = vec![
            mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true),
            mk_device(
                "Pixel-tab",
                "u2",
                Platform::Android,
                34,
                DeviceType::Tablet,
                true,
            ),
        ];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min)
            .expect("expand_app_requirements() SHALL succeed");
        let reduced = reduce_app_reqs_via_cover(&reqs, &snap)
            .expect("reduce_app_reqs_via_cover() SHALL succeed");
        assert_eq!(
            reduced.len(),
            2,
            "min-cover SHALL use exactly 2 devices to cover the 4 responsive axes"
        );
    }

    // Underspec: ios:latest:2 with snapshot containing only 1 iOS major → error.
    #[test]
    fn expand_app_requirements_underspec_latest_errors() {
        let app = mk_app_with_devices("a", vec![dc(Some(vec!["ios:latest:2"]), None)]);
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let result = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min);
        assert!(
            result.is_err(),
            "ios:latest:2 with 1 version SHALL error under Min"
        );
        let msg = format!("{}", result.expect_err("operation SHALL fail"));
        assert!(
            msg.contains("requested 2"),
            "error SHALL mention requested count: {msg}"
        );
    }

    // Underspec under "one" strategy — no error, takes what's available.
    #[test]
    fn expand_app_requirements_underspec_one_strategy_tolerates() {
        let app = mk_app_with_devices("a", vec![dc(Some(vec!["ios:latest:2"]), None)]);
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let result = expand_app_requirements(&app, &snap, None, CoverageStrategy::One);
        assert!(
            result.is_ok(),
            "One SHALL tolerate underspec: {:?}",
            result.err()
        );
    }

    // Empty devices block with booted iOS → one booted-platform box.
    #[test]
    fn expand_app_requirements_empty_devices_emits_booted_platform_boxes() {
        let app = mk_app_with_devices("a", vec![]);
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min)
            .expect("expand_app_requirements() SHALL succeed");
        assert_eq!(reqs.len(), 1, "one booted platform → one default box");
        assert_eq!(reqs[0].platform, Some(Platform::Ios));
        assert_eq!(
            reqs[0].booted,
            Some(true),
            "default empty-devices box SHALL require a booted device"
        );
    }

    // Empty devices block with both platforms booted → one box per.
    #[test]
    fn expand_app_requirements_empty_devices_both_platforms_booted() {
        let app = mk_app_with_devices("a", vec![]);
        let snap = vec![
            mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true),
            mk_device(
                "Pixel",
                "u2",
                Platform::Android,
                34,
                DeviceType::Phone,
                true,
            ),
        ];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min)
            .expect("expand_app_requirements() SHALL succeed");
        assert_eq!(reqs.len(), 2, "both platforms booted → 2 default boxes");
        let platforms: std::collections::HashSet<_> =
            reqs.iter().filter_map(|r| r.platform).collect();
        assert!(platforms.contains(&Platform::Ios));
        assert!(platforms.contains(&Platform::Android));
    }

    // Empty devices block with nothing booted → error, no iOS default.
    #[test]
    fn expand_app_requirements_empty_devices_no_booted_errors() {
        let app = mk_app_with_devices("a", vec![]);
        let snap = vec![mk_device(
            "iPhone-offline",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            /*booted=*/ false,
        )];
        let result = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min);
        assert!(
            result.is_err(),
            "nothing booted SHALL error (no iOS bias fallback)"
        );
    }

    // Dedup: two redundant blocks emit the same box → deduped.
    #[test]
    fn expand_app_requirements_dedups_overlapping_blocks() {
        let app = mk_app_with_devices(
            "a",
            vec![
                dc(Some(vec!["ios:26"]), Some(vec!["phone"])),
                dc(Some(vec!["ios:26"]), Some(vec!["phone"])), // identical — dedup
            ],
        );
        let snap: Vec<DeviceInfo> = Vec::new();
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min)
            .expect("expand_app_requirements() SHALL succeed");
        assert_eq!(
            reqs.len(),
            1,
            "identical blocks SHALL dedup to 1 requirement"
        );
    }

    // reduce_app_reqs_via_cover: one device covers both axes → 1 pinned requirement.
    #[test]
    fn reduce_app_reqs_via_cover_one_device_covers_all() {
        let reqs = vec![
            // Partial boxes: os + type
            AppRequirement {
                platform: Some(Platform::Ios),
                os_version: Some(OsVersionSpec::Exact {
                    platform: Platform::Ios,
                    major: 26,
                }),
                device_type: None,
                physical: None,
                name: None,
                playstore: None,
                accessibility_label: None,
                booted: None,
            },
            AppRequirement {
                platform: None,
                os_version: None,
                device_type: Some(DeviceType::Tablet),
                physical: None,
                name: None,
                playstore: None,
                accessibility_label: None,
                booted: None,
            },
        ];
        let snap = vec![mk_device(
            "iPad-26",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Tablet,
            true,
        )];
        let reduced = reduce_app_reqs_via_cover(&reqs, &snap)
            .expect("reduce_app_reqs_via_cover() SHALL succeed");
        assert_eq!(
            reduced.len(),
            1,
            "one device SHALL satisfy both partial boxes"
        );
        // Union: platform + os + type all present.
        assert_eq!(reduced[0].platform, Some(Platform::Ios));
        assert!(reduced[0].os_version.is_some());
        assert_eq!(reduced[0].device_type, Some(DeviceType::Tablet));
    }

    // Unrecognised `type` values error out (typo guard).
    #[test]
    fn expand_type_entries_rejects_unknown_values() {
        use golem_parser::{DeviceConstraint, StringOrVec};
        let dc = DeviceConstraint {
            os: None,
            device_type: Some(StringOrVec::Single("Tablet".into())), // wrong case
            name: None,
            accessibility_label: None,
            hardware: None,
            playstore: None,
            booted: None,
            expand: None,
        };
        let result = expand_type_entries(&dc);
        assert!(
            result.is_err(),
            "Unknown type SHALL error, not silently map to any-type"
        );
        let msg = format!("{}", result.expect_err("operation SHALL fail"));
        assert!(
            msg.contains("Tablet"),
            "error SHALL include the offending value: {msg}"
        );
    }

    // ── hardware axis expansion ─────────────────────────────────────

    fn dc_with_hardware(hw: Option<golem_parser::StringOrVec>) -> golem_parser::DeviceConstraint {
        golem_parser::DeviceConstraint {
            os: None,
            device_type: None,
            name: None,
            accessibility_label: None,
            hardware: hw,
            playstore: None,
            booted: None,
            expand: None,
        }
    }

    #[test]
    fn expand_hardware_entries_absent_defaults_to_virtual_only() {
        let dc = dc_with_hardware(None);
        let result = expand_hardware_entries(&dc).expect("expand_hardware_entries() SHALL succeed");
        assert_eq!(
            result,
            vec![Some(false)],
            "SHALL default to virtual-only when `hardware` is omitted"
        );
    }

    #[test]
    fn expand_hardware_entries_single_virtual_pins_false() {
        let dc = dc_with_hardware(Some(golem_parser::StringOrVec::Single("virtual".into())));
        let result = expand_hardware_entries(&dc).expect("expand_hardware_entries() SHALL succeed");
        assert_eq!(result, vec![Some(false)]);
    }

    #[test]
    fn expand_hardware_entries_single_real_pins_true() {
        let dc = dc_with_hardware(Some(golem_parser::StringOrVec::Single("real".into())));
        let result = expand_hardware_entries(&dc).expect("expand_hardware_entries() SHALL succeed");
        assert_eq!(result, vec![Some(true)]);
    }

    #[test]
    fn expand_hardware_entries_array_form_emits_two() {
        let dc = dc_with_hardware(Some(golem_parser::StringOrVec::Multiple(vec![
            "virtual".into(),
            "real".into(),
        ])));
        let result = expand_hardware_entries(&dc).expect("expand_hardware_entries() SHALL succeed");
        assert_eq!(
            result,
            vec![Some(false), Some(true)],
            "SHALL emit one entry per axis value, preserving order"
        );
    }

    #[test]
    fn expand_hardware_entries_rejects_empty_array() {
        let dc = dc_with_hardware(Some(golem_parser::StringOrVec::Multiple(vec![])));
        let result = expand_hardware_entries(&dc);
        assert!(
            result.is_err(),
            "SHALL reject `hardware = []` instead of silently emitting zero boxes"
        );
        let msg = format!("{}", result.expect_err("operation SHALL fail"));
        assert!(
            msg.contains("omit"),
            "error SHALL suggest omitting the field: {msg}"
        );
    }

    #[test]
    fn expand_hardware_entries_rejects_unknown_value() {
        let dc = dc_with_hardware(Some(golem_parser::StringOrVec::Single("sim".into())));
        let result = expand_hardware_entries(&dc);
        assert!(
            result.is_err(),
            "SHALL reject unknown values (e.g. \"sim\" instead of \"virtual\")"
        );
        let msg = format!("{}", result.expect_err("operation SHALL fail"));
        assert!(
            msg.contains("sim"),
            "error SHALL include offending value: {msg}"
        );
        assert!(
            msg.contains("virtual"),
            "error SHALL name allowed \"virtual\": {msg}"
        );
        assert!(
            msg.contains("real"),
            "error SHALL name allowed \"real\": {msg}"
        );
    }

    // JIT-one with multiple apps: SHALL produce one FlowRun carrying
    // one slot per app (or packed where compatible), not silently drop
    // apps after the first matching one.
    #[test]
    fn expand_one_multi_app_keeps_all_apps() {
        let app_a = mk_app_with_devices("a", vec![dc(Some(vec!["ios:latest"]), None)]);
        let app_b = mk_app_with_devices("b", vec![dc(Some(vec!["android:latest"]), None)]);
        let snap = vec![
            mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true),
            mk_device(
                "Pixel",
                "u2",
                Platform::Android,
                34,
                DeviceType::Phone,
                true,
            ),
        ];
        let a_reqs = expand_app_requirements(&app_a, &snap, None, CoverageStrategy::One)
            .expect("expand_app_requirements() SHALL succeed");
        let b_reqs = expand_app_requirements(&app_b, &snap, None, CoverageStrategy::One)
            .expect("expand_app_requirements() SHALL succeed");
        let mut groups: Vec<CoverageGroup> = Vec::new();
        let runs = expand_jit(
            0,
            &[("a".to_string(), a_reqs), ("b".to_string(), b_reqs)],
            &snap,
            &mut groups,
            CoverageStrategy::One,
            Some(1),
        )
        .expect("value SHALL be present");
        assert_eq!(runs.len(), 1, "one SHALL emit a single FlowRun");
        let slots = &runs[0].slots;
        assert_eq!(
            slots.len(),
            2,
            "multi-app flow SHALL produce one slot per incompatible platform"
        );
        let apps: std::collections::HashSet<_> =
            slots.iter().flat_map(|s| s.apps.iter().cloned()).collect();
        assert!(apps.contains("a"), "app a SHALL be present");
        assert!(
            apps.contains("b"),
            "app b SHALL be present — not silently dropped"
        );
        assert_eq!(groups.len(), 1, "JIT-one SHALL register one coverage group");
        assert_eq!(groups[0].max_runs, Some(1));
        assert_eq!(groups[0].strategy, CoverageStrategy::One);
        assert_eq!(runs[0].coverage_group, Some(0));
        assert_eq!(
            runs[0].covers_boxes.len(),
            2,
            "one FlowRun SHALL cover one pool entry per app"
        );
    }

    // JIT-one with coverage fan-out: multi-version os SHALL produce
    // N FlowRuns (one per reachable version), all sharing one group.
    #[test]
    fn expand_one_os_fanout_produces_n_flowruns_sharing_group() {
        let app = mk_app_with_devices("a", vec![dc(Some(vec!["ios:18", "ios:26"]), None)]);
        let snap = vec![
            mk_device(
                "iPhone-18",
                "u1",
                Platform::Ios,
                18,
                DeviceType::Phone,
                true,
            ),
            mk_device(
                "iPhone-26",
                "u2",
                Platform::Ios,
                26,
                DeviceType::Phone,
                true,
            ),
        ];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::One)
            .expect("expand_app_requirements() SHALL succeed");
        let mut groups: Vec<CoverageGroup> = Vec::new();
        let runs = expand_jit(
            0,
            &[("a".to_string(), reqs)],
            &snap,
            &mut groups,
            CoverageStrategy::One,
            Some(1),
        )
        .expect("value SHALL be present");
        assert_eq!(runs.len(), 2, "SHALL emit one FlowRun per os fan-out");
        assert!(
            runs.iter().all(|r| r.coverage_group == Some(0)),
            "SHALL share the same group"
        );
        assert_eq!(
            groups[0].boxes.len(),
            2,
            "pool SHALL hold both reachable boxes"
        );
    }

    // Smart strategy: same fan-out but max_runs=None → stop-on-all-ticked.
    #[test]
    fn expand_smart_strategy_uses_none_max_runs() {
        let app = mk_app_with_devices("a", vec![dc(Some(vec!["ios:18", "ios:26"]), None)]);
        let snap = vec![
            mk_device(
                "iPhone-18",
                "u1",
                Platform::Ios,
                18,
                DeviceType::Phone,
                true,
            ),
            mk_device(
                "iPhone-26",
                "u2",
                Platform::Ios,
                26,
                DeviceType::Phone,
                true,
            ),
        ];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Smart)
            .expect("expand_app_requirements() SHALL succeed");
        let mut groups: Vec<CoverageGroup> = Vec::new();
        let runs = expand_jit(
            0,
            &[("a".to_string(), reqs)],
            &snap,
            &mut groups,
            CoverageStrategy::Smart,
            None,
        )
        .expect("value SHALL be present");
        assert_eq!(
            groups[0].max_runs, None,
            "Smart SHALL have no run-count cap — stop on pool fully ticked"
        );
        assert_eq!(groups[0].strategy, CoverageStrategy::Smart);
        assert_eq!(runs.len(), 2);
    }

    // JIT with all boxes unreachable in snapshot → error.
    #[test]
    fn expand_jit_errors_when_no_box_reachable() {
        let app = mk_app_with_devices("a", vec![dc(Some(vec!["android:34"]), None)]);
        let reqs = vec![AppRequirement {
            platform: Some(Platform::Android),
            os_version: Some(OsVersionSpec::Exact {
                platform: Platform::Android,
                major: 34,
            }),
            device_type: None,
            physical: None,
            name: None,
            playstore: None,
            accessibility_label: None,
            booted: None,
        }];
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let mut groups: Vec<CoverageGroup> = Vec::new();
        let result = expand_jit(
            0,
            &[("a".to_string(), reqs)],
            &snap,
            &mut groups,
            CoverageStrategy::One,
            Some(1),
        );
        let _ = app;
        assert!(result.is_err(), "no reachable box SHALL error");
    }

    // ── shape_label / describe_slot rendering ───────────────────────

    fn empty_slot() -> DeviceSlot {
        DeviceSlot {
            platform: None,
            os_version: None,
            device_type: None,
            physical: None,
            name: None,
            playstore: None,
            accessibility_label: None,
            booted: None,
            apps: Vec::new(),
        }
    }

    // 1. A fully-unset slot renders as the platform-agnostic marker only.
    #[test]
    fn shape_label_all_none_is_any_platform() {
        let slot = empty_slot();
        assert_eq!(
            shape_label(&slot),
            "any-platform",
            "all-None slot SHALL render as just the any-platform marker"
        );
    }

    // 2. Exact os version renders bare major; Minimum appends `+`.
    #[test]
    fn shape_label_exact_and_minimum_versions() {
        let mut exact = empty_slot();
        exact.platform = Some(Platform::Ios);
        exact.os_version = Some(OsVersionSpec::Exact {
            platform: Platform::Ios,
            major: 18,
        });
        assert_eq!(
            shape_label(&exact),
            "ios/v18",
            "Exact version SHALL render as v<major>"
        );

        let mut min = empty_slot();
        min.platform = Some(Platform::Android);
        min.os_version = Some(OsVersionSpec::Minimum {
            platform: Platform::Android,
            major: 34,
        });
        assert_eq!(
            shape_label(&min),
            "android/v34+",
            "Minimum version SHALL render as v<major>+"
        );
    }

    // 3. Latest renders `latest` for count<=1 and `latest:N` for count>1.
    #[test]
    fn shape_label_latest_count_variants() {
        let mut one = empty_slot();
        one.platform = Some(Platform::Ios);
        one.os_version = Some(OsVersionSpec::Latest {
            platform: Platform::Ios,
            count: 1,
        });
        assert_eq!(
            shape_label(&one),
            "ios/latest",
            "Latest count=1 SHALL render as bare `latest`"
        );

        let mut two = empty_slot();
        two.platform = Some(Platform::Ios);
        two.os_version = Some(OsVersionSpec::Latest {
            platform: Platform::Ios,
            count: 2,
        });
        assert_eq!(
            shape_label(&two),
            "ios/latest:2",
            "Latest count>1 SHALL render as `latest:<count>`"
        );
    }

    // 4. type, physical=false (sim), name, and booted=true segments all
    //    render in order; physical=true renders `physical`.
    #[test]
    fn shape_label_full_segment_order() {
        let mut slot = empty_slot();
        slot.platform = Some(Platform::Ios);
        slot.os_version = Some(OsVersionSpec::Exact {
            platform: Platform::Ios,
            major: 26,
        });
        slot.device_type = Some(DeviceType::Tablet);
        slot.physical = Some(false);
        slot.name = Some("iPad".into());
        slot.booted = Some(true);
        assert_eq!(
            shape_label(&slot),
            "ios/v26/tablet/sim/name=iPad/booted",
            "segments SHALL render platform/version/type/physical/name/booted in order"
        );

        let mut phys = empty_slot();
        phys.platform = Some(Platform::Android);
        phys.physical = Some(true);
        assert_eq!(
            shape_label(&phys),
            "android/physical",
            "physical=true SHALL render as `physical`"
        );
    }

    // 5. booted=Some(false) is NOT rendered (only Some(true) shows).
    #[test]
    fn shape_label_booted_false_omitted() {
        let mut slot = empty_slot();
        slot.platform = Some(Platform::Ios);
        slot.booted = Some(false);
        assert_eq!(
            shape_label(&slot),
            "ios",
            "booted=Some(false) SHALL NOT add a `booted` segment"
        );
    }

    // 6. describe_slot appends an apps suffix; empty apps reads `no apps`.
    #[test]
    fn describe_slot_apps_suffix() {
        let mut slot = empty_slot();
        slot.platform = Some(Platform::Ios);
        assert_eq!(
            describe_slot(&slot),
            "ios no apps",
            "empty apps SHALL render `no apps`"
        );

        slot.apps = vec!["a".into(), "b".into()];
        assert_eq!(
            describe_slot(&slot),
            "ios apps=[a,b]",
            "apps SHALL render comma-joined inside apps=[...]"
        );
    }

    // ── device_matches_slot per-axis ────────────────────────────────

    // 7. A slot with all-None fields matches any device.
    #[test]
    fn device_matches_slot_all_none_matches_anything() {
        let d = mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true);
        assert!(
            device_matches_slot(&d, &empty_slot()),
            "all-None slot SHALL match every device"
        );
    }

    // 8. Platform mismatch rejects.
    #[test]
    fn device_matches_slot_platform_mismatch() {
        let d = mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true);
        let mut slot = empty_slot();
        slot.platform = Some(Platform::Android);
        assert!(
            !device_matches_slot(&d, &slot),
            "platform mismatch SHALL reject"
        );
    }

    // 9. Exact version requires equality; Minimum requires >=.
    #[test]
    fn device_matches_slot_version_exact_and_minimum() {
        let d = mk_device("iPhone", "u1", Platform::Ios, 18, DeviceType::Phone, true);

        let mut exact_ok = empty_slot();
        exact_ok.os_version = Some(OsVersionSpec::Exact {
            platform: Platform::Ios,
            major: 18,
        });
        assert!(
            device_matches_slot(&d, &exact_ok),
            "Exact equal major SHALL match"
        );

        let mut exact_no = empty_slot();
        exact_no.os_version = Some(OsVersionSpec::Exact {
            platform: Platform::Ios,
            major: 26,
        });
        assert!(
            !device_matches_slot(&d, &exact_no),
            "Exact unequal major SHALL reject"
        );

        let mut min_ok = empty_slot();
        min_ok.os_version = Some(OsVersionSpec::Minimum {
            platform: Platform::Ios,
            major: 16,
        });
        assert!(
            device_matches_slot(&d, &min_ok),
            "Minimum below device major SHALL match"
        );

        let mut min_no = empty_slot();
        min_no.os_version = Some(OsVersionSpec::Minimum {
            platform: Platform::Ios,
            major: 26,
        });
        assert!(
            !device_matches_slot(&d, &min_no),
            "Minimum above device major SHALL reject"
        );
    }

    // 10. Latest version spec is permissive — matches any major.
    #[test]
    fn device_matches_slot_latest_is_permissive() {
        let d = mk_device("iPhone", "u1", Platform::Ios, 18, DeviceType::Phone, true);
        let mut slot = empty_slot();
        slot.os_version = Some(OsVersionSpec::Latest {
            platform: Platform::Ios,
            count: 1,
        });
        assert!(
            device_matches_slot(&d, &slot),
            "Latest SHALL match any major (resolution happens upstream)"
        );
    }

    // 11. device_type, name, and booted axes each reject on mismatch.
    #[test]
    fn device_matches_slot_type_name_booted_axes() {
        let d = mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true);

        let mut wrong_type = empty_slot();
        wrong_type.device_type = Some(DeviceType::Tablet);
        assert!(
            !device_matches_slot(&d, &wrong_type),
            "device_type mismatch SHALL reject"
        );

        let mut wrong_name = empty_slot();
        wrong_name.name = Some("iPad".into());
        assert!(
            !device_matches_slot(&d, &wrong_name),
            "name mismatch SHALL reject"
        );

        let mut want_shutdown = empty_slot();
        want_shutdown.booted = Some(false);
        assert!(
            !device_matches_slot(&d, &want_shutdown),
            "booted device against booted=Some(false) SHALL reject"
        );

        let mut want_booted = empty_slot();
        want_booted.booted = Some(true);
        assert!(
            device_matches_slot(&d, &want_booted),
            "booted device against booted=Some(true) SHALL match"
        );
    }

    // 12. playstore / accessibility_label on a slot are not on DeviceInfo,
    //     so they NEVER reject — any device passes.
    #[test]
    fn device_matches_slot_ignores_playstore_and_a11y() {
        let d = mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true);
        let mut slot = empty_slot();
        slot.playstore = Some(true);
        slot.accessibility_label = Some("label".into());
        assert!(
            device_matches_slot(&d, &slot),
            "playstore/accessibility_label SHALL not gate plan-time matching"
        );
    }

    // ── compute_device_availability ─────────────────────────────────

    // 13. Each unique slot shape yields one line with booted/shutdown/
    //     physical counts; identical shapes are deduped to one line.
    #[test]
    fn compute_device_availability_counts_and_dedups() {
        let mut slot = empty_slot();
        slot.platform = Some(Platform::Ios);
        let runs = vec![
            FlowRun {
                flow_idx: 0,
                slots: vec![slot.clone()],
                coverage_group: None,
                covers_boxes: Vec::new(),
                repeat_index: 0,
            },
            // Second run with identical slot shape — must dedup to one line.
            FlowRun {
                flow_idx: 0,
                slots: vec![slot.clone()],
                coverage_group: None,
                covers_boxes: Vec::new(),
                repeat_index: 0,
            },
        ];
        let mut shutdown_ios = mk_device(
            "iPhone-off",
            "u2",
            Platform::Ios,
            18,
            DeviceType::Phone,
            false,
        );
        shutdown_ios.state = DeviceState::Shutdown;
        let snap = vec![
            mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true),
            shutdown_ios,
            // Android device must NOT match an ios slot.
            mk_device(
                "Pixel",
                "u3",
                Platform::Android,
                34,
                DeviceType::Phone,
                true,
            ),
        ];
        let lines = compute_device_availability(&runs, &snap);
        assert_eq!(
            lines.len(),
            1,
            "identical slot shapes SHALL dedup to one line"
        );
        let line = &lines[0];
        assert!(
            line.contains("ios"),
            "line SHALL be labelled for the ios shape: {line}"
        );
        assert!(
            line.contains("2 device(s)"),
            "SHALL count 2 matching ios devices: {line}"
        );
        assert!(line.contains("1 booted"), "SHALL report 1 booted: {line}");
        assert!(
            line.contains("1 shutdown"),
            "SHALL report 1 shutdown: {line}"
        );
    }

    // 14. physical segment only appears when at least one physical device
    //     matches; absent otherwise.
    #[test]
    fn compute_device_availability_omits_physical_when_none() {
        let mut slot = empty_slot();
        slot.platform = Some(Platform::Ios);
        let runs = vec![FlowRun {
            flow_idx: 0,
            slots: vec![slot],
            coverage_group: None,
            covers_boxes: Vec::new(),
            repeat_index: 0,
        }];
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let lines = compute_device_availability(&runs, &snap);
        assert_eq!(lines.len(), 1);
        assert!(
            !lines[0].contains("physical"),
            "no physical match SHALL omit the physical segment: {}",
            lines[0]
        );
    }

    // ── merge_project_apps ──────────────────────────────────────────

    fn flow_with_app(app: AppConfig) -> FlowFile {
        let mut flow = parse_flow("[flow]\nname = \"f\"\n").expect("base flow SHALL parse");
        flow.flow.apps = vec![app];
        flow
    }

    // 15. Flow values always win — a flow that already set bundle keeps it
    //     even when the project entry differs.
    #[test]
    fn merge_project_apps_flow_value_wins() {
        let mut app = mk_app_with_devices("a", vec![]);
        app.bundle = Some("com.flow.win".into());
        let mut flow = flow_with_app(app);
        let projects = vec![project_app("a", "com.project.lose", Some("scripts/p.sh"))];
        merge_project_apps(&mut flow, &projects);
        assert_eq!(
            flow.flow.apps[0].bundle.as_deref(),
            Some("com.flow.win"),
            "flow-set bundle SHALL win over project bundle"
        );
        // install_script was None on the flow → filled from project.
        assert!(
            flow.flow.apps[0].install_script.is_some(),
            "absent flow install_script SHALL be filled from project"
        );
    }

    // 16. install_timeout_ms and devices gaps are filled from the project
    //     entry when the flow leaves them empty.
    #[test]
    fn merge_project_apps_fills_timeout_and_devices() {
        let mut app = mk_app_with_devices("a", vec![]);
        app.bundle = None;
        app.install_timeout_ms = None;
        let mut flow = flow_with_app(app);
        let mut proj = project_app("a", "com.a", None);
        proj.install_timeout_ms = Some(9000);
        proj.devices = vec![dc(Some(vec!["ios"]), None)];
        merge_project_apps(&mut flow, &[proj]);
        assert_eq!(
            flow.flow.apps[0].install_timeout_ms,
            Some(9000),
            "absent flow install_timeout_ms SHALL be filled from project"
        );
        assert_eq!(
            flow.flow.apps[0].devices.len(),
            1,
            "empty flow devices SHALL be filled from project devices"
        );
    }

    // 17. An app with no matching project entry is left untouched.
    #[test]
    fn merge_project_apps_no_matching_name_no_change() {
        let app = mk_app_with_devices("orphan", vec![]);
        let mut flow = flow_with_app(app);
        let before = flow.flow.apps[0].bundle.clone();
        merge_project_apps(&mut flow, &[project_app("other", "com.other", None)]);
        assert_eq!(
            flow.flow.apps[0].bundle, before,
            "an app with no project match SHALL be unchanged"
        );
    }

    // ── union_requirements / slot_compatible_with ───────────────────

    // 18. union takes the first Some(_) per axis across the inputs.
    #[test]
    fn union_requirements_takes_first_some_per_axis() {
        let a = AppRequirement {
            platform: None,
            os_version: Some(OsVersionSpec::Exact {
                platform: Platform::Ios,
                major: 26,
            }),
            device_type: None,
            physical: None,
            name: None,
            playstore: None,
            accessibility_label: None,
            booted: None,
        };
        let b = AppRequirement {
            platform: Some(Platform::Ios),
            os_version: Some(OsVersionSpec::Exact {
                platform: Platform::Ios,
                major: 18,
            }),
            device_type: Some(DeviceType::Tablet),
            physical: Some(true),
            name: None,
            playstore: None,
            accessibility_label: None,
            booted: None,
        };
        let u = union_requirements(&[&a, &b]);
        // platform: a is None → take b's Some.
        assert_eq!(
            u.platform,
            Some(Platform::Ios),
            "platform SHALL take first Some (b)"
        );
        // os_version: a's Some wins (first in iteration order).
        assert!(
            matches!(u.os_version, Some(OsVersionSpec::Exact { major: 26, .. })),
            "os_version SHALL keep the first Some (a's major 26)"
        );
        assert_eq!(
            u.device_type,
            Some(DeviceType::Tablet),
            "device_type SHALL take b's Some"
        );
        assert_eq!(u.physical, Some(true));
    }

    // 19. slot_compatible_with requires exact equality on every axis —
    //     a single differing field rejects packing.
    #[test]
    fn slot_compatible_with_requires_exact_match() {
        let mut slot = empty_slot();
        slot.platform = Some(Platform::Ios);
        slot.device_type = Some(DeviceType::Phone);
        let req_match = AppRequirement {
            platform: Some(Platform::Ios),
            os_version: None,
            device_type: Some(DeviceType::Phone),
            physical: None,
            name: None,
            playstore: None,
            accessibility_label: None,
            booted: None,
        };
        assert!(
            slot_compatible_with(&slot, &req_match),
            "identical axes SHALL be compatible"
        );
        let mut req_diff = req_match.clone();
        req_diff.device_type = Some(DeviceType::Tablet);
        assert!(
            !slot_compatible_with(&slot, &req_diff),
            "a single differing axis SHALL block packing"
        );
    }

    // ── cover_and_union short-circuits ──────────────────────────────

    // 20. Empty reqs → empty output, no uncovered claim.
    #[test]
    fn cover_and_union_empty_reqs_short_circuits() {
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let (reduced, uncovered) = cover_and_union(&[], &snap);
        assert!(reduced.is_empty(), "empty reqs SHALL yield no reduced reqs");
        assert!(
            uncovered.is_empty(),
            "empty reqs SHALL claim nothing uncovered"
        );
    }

    // 21. Empty snapshot → reqs passed through unchanged, nothing uncovered
    //     (scheduler resolves at execute time).
    #[test]
    fn cover_and_union_empty_snapshot_passes_reqs_through() {
        let reqs = vec![AppRequirement {
            platform: Some(Platform::Ios),
            os_version: Some(OsVersionSpec::Latest {
                platform: Platform::Ios,
                count: 1,
            }),
            device_type: None,
            physical: None,
            name: None,
            playstore: None,
            accessibility_label: None,
            booted: None,
        }];
        let (reduced, uncovered) = cover_and_union(&reqs, &[]);
        assert_eq!(
            reduced.len(),
            1,
            "empty snapshot SHALL pass reqs through unchanged"
        );
        assert!(
            uncovered.is_empty(),
            "empty snapshot SHALL NOT claim uncovered boxes"
        );
    }

    // ── expand_os_pairs ─────────────────────────────────────────────

    // 22. No `os` field, no override → one pair per platform (any version).
    #[test]
    fn expand_os_pairs_no_os_defaults_to_both_platforms() {
        let d = dc(None, None);
        let pairs =
            expand_os_pairs(&d, &[], None, CoverageStrategy::Min).expect("no-os SHALL succeed");
        assert_eq!(
            pairs.len(),
            2,
            "no-os no-override SHALL emit a pair per platform"
        );
        let plats: std::collections::HashSet<_> = pairs.iter().map(|(p, _)| *p).collect();
        assert!(plats.contains(&Platform::Ios) && plats.contains(&Platform::Android));
        assert!(
            pairs.iter().all(|(_, v)| v.is_none()),
            "default SHALL pin no version"
        );
    }

    // 23. No `os` field WITH platform override → single pair on that platform.
    #[test]
    fn expand_os_pairs_no_os_with_override() {
        let d = dc(None, None);
        let pairs = expand_os_pairs(&d, &[], Some(Platform::Android), CoverageStrategy::Min)
            .expect("no-os override SHALL succeed");
        assert_eq!(
            pairs,
            vec![(Platform::Android, None)],
            "override SHALL collapse no-os default to the forced platform"
        );
    }

    // 24. `os = "any"` expands to both platforms, any version.
    #[test]
    fn expand_os_pairs_any_expands_both_platforms() {
        let d = dc(Some(vec!["any"]), None);
        let pairs =
            expand_os_pairs(&d, &[], None, CoverageStrategy::Min).expect("`any` SHALL succeed");
        assert_eq!(pairs.len(), 2, "`any` SHALL emit both platforms");
        assert!(
            pairs.iter().all(|(_, v)| v.is_none()),
            "`any` SHALL pin no version"
        );
    }

    // 25. Bare `os = "android"` → one Android pair with no version.
    #[test]
    fn expand_os_pairs_bare_platform_name() {
        let d = dc(Some(vec!["android"]), None);
        let pairs = expand_os_pairs(&d, &[], None, CoverageStrategy::Min)
            .expect("bare platform SHALL succeed");
        assert_eq!(
            pairs,
            vec![(Platform::Android, None)],
            "bare `android` SHALL emit one Android pair, any version"
        );
    }

    // 26. Unrecognised os constraint → error.
    #[test]
    fn expand_os_pairs_unrecognised_errors() {
        let d = dc(Some(vec!["windows"]), None);
        let result = expand_os_pairs(&d, &[], None, CoverageStrategy::Min);
        assert!(result.is_err(), "unrecognised os SHALL error");
        let msg = format!("{}", result.expect_err("expected error"));
        assert!(
            msg.contains("windows"),
            "error SHALL include the offending value: {msg}"
        );
    }

    // 27. `ios:latest` with no matching iOS device in snapshot → falls back
    //     to an abstract Latest pair (scheduler retries at execute time).
    #[test]
    fn expand_os_pairs_latest_empty_snapshot_falls_back_to_abstract() {
        let d = dc(Some(vec!["ios:latest"]), None);
        let pairs = expand_os_pairs(&d, &[], None, CoverageStrategy::Min)
            .expect("latest with empty snapshot SHALL succeed (abstract fallback)");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, Platform::Ios);
        assert!(
            matches!(pairs[0].1, Some(OsVersionSpec::Latest { .. })),
            "no matching majors SHALL keep an abstract Latest spec for execute-time retry"
        );
    }

    // ── default_any_booted_requirements ─────────────────────────────

    // 28. Platform override filters the default booted boxes to the chosen
    //     platform even when both are booted.
    #[test]
    fn default_any_booted_requirements_respects_override() {
        let snap = vec![
            mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true),
            mk_device(
                "Pixel",
                "u2",
                Platform::Android,
                34,
                DeviceType::Phone,
                true,
            ),
        ];
        let reqs = default_any_booted_requirements(&snap, Some(Platform::Android))
            .expect("override with a booted match SHALL succeed");
        assert_eq!(
            reqs.len(),
            1,
            "override SHALL keep only the forced platform"
        );
        assert_eq!(reqs[0].platform, Some(Platform::Android));
        assert_eq!(
            reqs[0].physical,
            Some(false),
            "default booted box SHALL require a virtual device"
        );
    }

    // 29. Override to a platform with no booted device → error.
    #[test]
    fn default_any_booted_requirements_override_no_match_errors() {
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let result = default_any_booted_requirements(&snap, Some(Platform::Android));
        assert!(
            result.is_err(),
            "override to a platform with no booted device SHALL error"
        );
    }
}
