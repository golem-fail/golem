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
use golem_devices::{DeviceInfo, DeviceType, OsVersionSpec, Platform};
use golem_devices::version::{parse_os_version, resolve_latest};
use golem_parser::mixin::expand_mixins;
use golem_parser::{parse_flow, AppConfig, FlowFile, ProjectAppConfig};

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
#[derive(Debug, Clone)]
pub struct FlowRun {
    pub flow_idx: usize,
    pub slots: Vec<DeviceSlot>,
}

/// Device-matching requirements. `None` means "any" on that dimension.
/// Match semantics at runtime: scheduler finds any free device whose actual
/// attributes satisfy every `Some(_)` field.
#[derive(Debug, Clone)]
pub struct DeviceSlot {
    pub platform: Platform,
    pub os_version: Option<OsVersionSpec>,
    pub device_type: Option<DeviceType>,
    pub physical: Option<bool>,
    pub name: Option<String>,
    pub playstore: Option<bool>,
    pub accessibility_label: Option<String>,
    /// Apps to install on this slot's device. Multi-app flows may pack 2+
    /// when their constraints permit (default sharing).
    pub apps: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedSuite {
    pub flows: Vec<ParsedFlow>,
    pub flow_runs: Vec<FlowRun>,
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
}

/// A flow file that failed to load before the Plan could expand it.
#[derive(Debug, Clone)]
pub struct ParseFailure {
    pub path: PathBuf,
    pub error: String,
}

/// Parse flows, merge project apps, discover a device snapshot (for
/// `:latest:N` expansion + feasibility), and emit the full suite plan.
pub async fn plan(
    flow_paths: &[PathBuf],
    project_apps: &[ProjectAppConfig],
    project_root: &Path,
    platform_override: Option<Platform>,
) -> Result<ParsedSuite> {
    let mut flows: Vec<ParsedFlow> = Vec::with_capacity(flow_paths.len());
    let mut parse_failures: Vec<ParseFailure> = Vec::new();
    for path in flow_paths {
        match parse_one(path, project_apps, project_root) {
            Ok(flow) => flows.push(ParsedFlow { path: path.clone(), flow }),
            Err(e) => parse_failures.push(ParseFailure {
                path: path.clone(),
                error: format!("{e:#}"),
            }),
        }
    }

    let snapshot = device_snapshot().await;

    let mut flow_runs: Vec<FlowRun> = Vec::new();
    for (idx, pf) in flows.iter().enumerate() {
        flow_runs.extend(expand_flow(idx, &pf.flow, &snapshot, platform_override)?);
    }

    let install_matrix = build_install_matrix(&flows, &flow_runs, project_root);
    let device_availability = compute_device_availability(&flow_runs, &snapshot);

    Ok(ParsedSuite {
        flows,
        flow_runs,
        install_matrix,
        device_availability,
        parse_failures,
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
    let mut flow = parse_flow(&text)
        .with_context(|| format!("parsing flow file {}", path.display()))?;
    merge_project_apps(&mut flow, project_apps);

    let flow_dir = path.parent().unwrap_or(Path::new("."));
    for block in &mut flow.block {
        block.steps = expand_mixins(&block.steps, flow_dir, project_root)
            .with_context(|| format!("expanding mixins in {}", path.display()))?;
    }

    Ok(flow)
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
fn compute_device_availability(
    flow_runs: &[FlowRun],
    snapshot: &[DeviceInfo],
) -> Vec<String> {
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
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        slot.platform,
        slot.os_version,
        slot.device_type,
        slot.physical,
        slot.name,
        slot.playstore,
        slot.accessibility_label,
    )
}

/// Render a slot's shape-only label: `platform/version/type/physical/name`.
/// Excludes `apps`. Used for availability counts where apps are irrelevant.
pub fn shape_label(slot: &DeviceSlot) -> String {
    let mut parts: Vec<String> = vec![slot.platform.to_string()];
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
        parts.push(if phys { "physical".into() } else { "sim".into() });
    }
    if let Some(n) = &slot.name {
        parts.push(format!("name={n}"));
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
    if device.platform != slot.platform {
        return false;
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
    true
}

/// Discover devices from both platforms. Best-effort — errors are soft,
/// treated as "platform has no devices". Used for `:latest:N` version
/// expansion and for knowing available major versions; we do NOT pin UDIDs.
async fn device_snapshot() -> Vec<DeviceInfo> {
    let mut all = golem_devices::ios::discover_ios_devices().await.unwrap_or_default();
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

/// A single flattened requirement tuple for one app — one concrete
/// coverage point that scheduler can match a device against.
#[derive(Debug, Clone)]
struct AppRequirement {
    platform: Platform,
    os_version: Option<OsVersionSpec>,
    device_type: Option<DeviceType>,
    physical: Option<bool>,
    name: Option<String>,
    playstore: Option<bool>,
    accessibility_label: Option<String>,
}

/// Expand a flow into one or more `FlowRun`s. Each FlowRun holds slots
/// that run simultaneously. Coverage fan-out produces multiple FlowRuns;
/// multi-device coordination produces multiple slots within one FlowRun.
fn expand_flow(
    flow_idx: usize,
    flow: &FlowFile,
    snapshot: &[DeviceInfo],
    platform_override: Option<Platform>,
) -> Result<Vec<FlowRun>> {
    // Step 1: per-app, expand [[flow.apps.devices]] into concrete AppRequirements.
    let app_reqs: Vec<(String, Vec<AppRequirement>)> = flow
        .flow
        .apps
        .iter()
        .map(|app| {
            let reqs = expand_app_requirements(app, snapshot, platform_override)?;
            Ok::<_, anyhow::Error>((app.name.clone(), reqs))
        })
        .collect::<Result<Vec<_>>>()?;

    if app_reqs.is_empty() || app_reqs.iter().all(|(_, r)| r.is_empty()) {
        return Ok(Vec::new());
    }

    // Step 2: determine fan-out count = max coverage length across apps.
    // An app with 1 req cycles through (same single req used for every run);
    // an app with N reqs contributes one req per run.
    let run_count = app_reqs
        .iter()
        .map(|(_, r)| r.len())
        .max()
        .unwrap_or(1)
        .max(1);

    let mut runs = Vec::with_capacity(run_count);
    for i in 0..run_count {
        let mut slots: Vec<DeviceSlot> = Vec::new();

        for (app_name, reqs) in &app_reqs {
            if reqs.is_empty() {
                continue;
            }
            let req = &reqs[i % reqs.len()];
            if let Some(slot) = slots.iter_mut().find(|s| slot_compatible_with(s, req)) {
                slot.apps.push(app_name.clone());
            } else {
                slots.push(DeviceSlot {
                    platform: req.platform,
                    os_version: req.os_version.clone(),
                    device_type: req.device_type,
                    physical: req.physical,
                    name: req.name.clone(),
                    playstore: req.playstore,
                    accessibility_label: req.accessibility_label.clone(),
                    apps: vec![app_name.clone()],
                });
            }
        }

        if !slots.is_empty() {
            runs.push(FlowRun { flow_idx, slots });
        }
    }

    Ok(runs)
}

/// An app's requirement can pack into an existing slot if every `Some(_)`
/// field matches (or the slot is more general — has `None` where the req
/// has `Some(_)`). For simplicity here: require exact match on each field.
/// A looser "slot is a superset" check could pack more aggressively but
/// we keep strict match to avoid surprising merges.
fn slot_compatible_with(slot: &DeviceSlot, req: &AppRequirement) -> bool {
    slot.platform == req.platform
        && slot.os_version == req.os_version
        && slot.device_type == req.device_type
        && slot.physical == req.physical
        && slot.name == req.name
        && slot.playstore == req.playstore
        && slot.accessibility_label == req.accessibility_label
}

/// Expand an `AppConfig`'s `[[flow.apps.devices]]` constraints into flat
/// `AppRequirement`s. Multi-valued `os` / `type` fans out; `ios:latest:N`
/// expands to N concrete `Exact` specs using the device snapshot.
fn expand_app_requirements(
    app: &AppConfig,
    snapshot: &[DeviceInfo],
    platform_override: Option<Platform>,
) -> Result<Vec<AppRequirement>> {
    // No device constraints at all → default to any iOS (preserves today's
    // behaviour; `os = "any"` support is a separate roadmap item).
    if app.devices.is_empty() {
        let platform = platform_override.unwrap_or(Platform::Ios);
        return Ok(vec![AppRequirement {
            platform,
            os_version: None,
            device_type: None,
            physical: None,
            name: None,
            playstore: None,
            accessibility_label: None,
        }]);
    }

    let mut out: Vec<AppRequirement> = Vec::new();
    for dc in &app.devices {
        // Expand `os` into a list of (platform, Option<OsVersionSpec>) pairs.
        // "ios" alone (no colon) is not accepted by parse_os_version; treat
        // it as "any iOS version" via fallback.
        let os_pairs: Vec<(Platform, Option<OsVersionSpec>)> = if let Some(os_sv) = &dc.os {
            let mut pairs = Vec::new();
            for s in os_sv.to_vec() {
                match parse_os_version(&s) {
                    Ok(OsVersionSpec::Latest { platform, count }) => {
                        // Expand `:latest` and `:latest:N` to concrete
                        // versions using the plan-time device snapshot —
                        // checklist output is meaningful ("v26" not "latest")
                        // and per-run requirements pin to specific majors.
                        // If the snapshot has no matching devices, fall back
                        // to the abstract Latest spec so resolver can still
                        // try at execute time.
                        let majors: Vec<u32> = snapshot
                            .iter()
                            .filter(|d| d.platform == platform)
                            .map(|d| d.os_major)
                            .collect();
                        let tops = resolve_latest(platform, count, &majors);
                        if tops.is_empty() {
                            pairs.push((
                                platform,
                                Some(OsVersionSpec::Latest { platform, count }),
                            ));
                        } else {
                            for m in tops {
                                pairs.push((
                                    platform,
                                    Some(OsVersionSpec::Exact { platform, major: m }),
                                ));
                            }
                        }
                    }
                    Ok(spec) => {
                        let platform = match &spec {
                            OsVersionSpec::Exact { platform, .. }
                            | OsVersionSpec::Minimum { platform, .. }
                            | OsVersionSpec::Latest { platform, .. } => *platform,
                        };
                        pairs.push((platform, Some(spec)));
                    }
                    Err(_) => {
                        // "ios" or "android" alone — any version of that platform.
                        let platform = if s.starts_with("android") {
                            Platform::Android
                        } else if s.starts_with("ios") {
                            Platform::Ios
                        } else {
                            anyhow::bail!("unrecognised os constraint: {s}");
                        };
                        pairs.push((platform, None));
                    }
                }
            }
            pairs
        } else {
            // No os field → default platform, any version.
            vec![(platform_override.unwrap_or(Platform::Ios), None)]
        };

        // Expand `type` into a list of Option<DeviceType>.
        let type_entries: Vec<Option<DeviceType>> = if let Some(type_sv) = &dc.device_type {
            type_sv
                .to_vec()
                .iter()
                .map(|s| match s.as_str() {
                    "phone" => Some(DeviceType::Phone),
                    "tablet" => Some(DeviceType::Tablet),
                    _ => None,
                })
                .collect()
        } else {
            vec![None]
        };

        // Cross-product: every (os, type) pair is a distinct coverage point.
        for (platform, os_version) in &os_pairs {
            // Honour `--platform` override by DROPPING constraints whose
            // platform doesn't match (rather than silently forcing them —
            // that produced incoherent AppRequirements with e.g. platform=ios
            // but os_version=Latest{android}).
            if let Some(forced) = platform_override {
                if *platform != forced {
                    continue;
                }
            }
            for type_e in &type_entries {
                out.push(AppRequirement {
                    platform: *platform,
                    os_version: os_version.clone(),
                    device_type: *type_e,
                    physical: dc.physical,
                    name: dc.name.clone(),
                    playstore: dc.playstore,
                    accessibility_label: dc.accessibility_label.clone(),
                });
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_flow(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
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

    #[tokio::test]
    async fn plan_single_app_single_ios_run() {
        let tmp = TempDir::new().unwrap();
        let flow = write_flow(tmp.path(), "f.test.toml", r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios"
        "#);
        let apps = vec![project_app("app", "com.app", Some("scripts/i.sh"))];
        let suite = plan(&[flow], &apps, tmp.path(), None).await.unwrap();
        assert_eq!(suite.flow_runs.len(), 1);
        assert_eq!(suite.flow_runs[0].slots.len(), 1);
        assert_eq!(suite.flow_runs[0].slots[0].platform, Platform::Ios);
        assert!(suite.flow_runs[0].slots[0].os_version.is_none(),
            "'os = \"ios\"' alone SHALL have no specific version requirement");
        assert_eq!(suite.flow_runs[0].slots[0].apps, vec!["app".to_string()]);
    }

    #[tokio::test]
    async fn plan_ios_18_exact_version_captured() {
        let tmp = TempDir::new().unwrap();
        let flow = write_flow(tmp.path(), "f.test.toml", r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios:18"
        "#);
        let apps = vec![project_app("app", "com.app", None)];
        let suite = plan(&[flow], &apps, tmp.path(), None).await.unwrap();
        assert_eq!(suite.flow_runs.len(), 1);
        let spec = suite.flow_runs[0].slots[0].os_version.as_ref().unwrap();
        assert!(matches!(spec, OsVersionSpec::Exact { major: 18, .. }),
            "os = \"ios:18\" SHALL populate os_version with Exact(18)");
    }

    #[tokio::test]
    async fn plan_os_list_fans_out_per_version() {
        let tmp = TempDir::new().unwrap();
        let flow = write_flow(tmp.path(), "f.test.toml", r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = ["ios:18", "ios:26"]
        "#);
        let apps = vec![project_app("app", "com.app", None)];
        let suite = plan(&[flow], &apps, tmp.path(), None).await.unwrap();
        assert_eq!(suite.flow_runs.len(), 2,
            "os list of 2 entries SHALL produce 2 FlowRuns");
    }

    #[tokio::test]
    async fn plan_type_list_fans_out() {
        let tmp = TempDir::new().unwrap();
        let flow = write_flow(tmp.path(), "f.test.toml", r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios"
            type = ["phone", "tablet"]
        "#);
        let apps = vec![project_app("app", "com.app", None)];
        let suite = plan(&[flow], &apps, tmp.path(), None).await.unwrap();
        assert_eq!(suite.flow_runs.len(), 2);
        let mut types: Vec<_> = suite
            .flow_runs
            .iter()
            .map(|r| r.slots[0].device_type.unwrap())
            .collect();
        types.sort_by_key(|t| t.to_string());
        assert_eq!(types, vec![DeviceType::Phone, DeviceType::Tablet]);
    }

    #[tokio::test]
    async fn plan_two_apps_same_constraint_share_slot() {
        let tmp = TempDir::new().unwrap();
        let flow = write_flow(tmp.path(), "f.test.toml", r#"
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
        "#);
        let apps = vec![
            project_app("a", "com.a", None),
            project_app("b", "com.b", None),
        ];
        let suite = plan(&[flow], &apps, tmp.path(), None).await.unwrap();
        assert_eq!(suite.flow_runs.len(), 1);
        assert_eq!(suite.flow_runs[0].slots.len(), 1,
            "apps with identical constraints SHALL share a slot");
        let mut slot_apps = suite.flow_runs[0].slots[0].apps.clone();
        slot_apps.sort();
        assert_eq!(slot_apps, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn plan_two_apps_different_platforms_produce_two_slots() {
        let tmp = TempDir::new().unwrap();
        let flow = write_flow(tmp.path(), "f.test.toml", r#"
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
        "#);
        let apps = vec![
            project_app("client", "com.c", None),
            project_app("supplier", "com.s", None),
        ];
        let suite = plan(&[flow], &apps, tmp.path(), None).await.unwrap();
        assert_eq!(suite.flow_runs.len(), 1,
            "chat-test pattern SHALL emit one FlowRun (not two)");
        assert_eq!(suite.flow_runs[0].slots.len(), 2,
            "incompatible per-app constraints SHALL produce separate slots");
    }

    #[tokio::test]
    async fn plan_client_multi_version_supplier_single_platform() {
        let tmp = TempDir::new().unwrap();
        let flow = write_flow(tmp.path(), "f.test.toml", r#"
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
        "#);
        let apps = vec![
            project_app("client", "com.c", None),
            project_app("supplier", "com.s", None),
        ];
        let suite = plan(&[flow], &apps, tmp.path(), None).await.unwrap();
        assert_eq!(suite.flow_runs.len(), 2,
            "client coverage fan-out SHALL produce 2 FlowRuns");
        for run in &suite.flow_runs {
            assert_eq!(run.slots.len(), 2,
                "each FlowRun SHALL coordinate client + supplier slots");
        }
    }

    #[tokio::test]
    async fn plan_platform_override_forces_single_platform() {
        let tmp = TempDir::new().unwrap();
        let flow = write_flow(tmp.path(), "f.test.toml", r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios"
            [[flow.apps.devices]]
            os = "android"
        "#);
        let apps = vec![project_app("app", "com.app", None)];
        let suite = plan(&[flow], &apps, tmp.path(), Some(Platform::Android)).await.unwrap();
        for run in &suite.flow_runs {
            for slot in &run.slots {
                assert_eq!(slot.platform, Platform::Android,
                    "platform override SHALL force every slot to the chosen platform");
            }
        }
    }

    #[tokio::test]
    async fn plan_merges_project_bundle_when_flow_omits() {
        let tmp = TempDir::new().unwrap();
        let flow = write_flow(tmp.path(), "f.test.toml", r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "a"
            [[flow.apps.devices]]
            os = "ios"
        "#);
        let apps = vec![project_app("a", "com.project.a", Some("scripts/a.sh"))];
        let suite = plan(&[flow], &apps, tmp.path(), None).await.unwrap();
        let app = &suite.flows[0].flow.flow.apps[0];
        assert_eq!(app.bundle.as_deref(), Some("com.project.a"));
    }

    #[tokio::test]
    async fn plan_drops_unreferenced_project_apps() {
        let tmp = TempDir::new().unwrap();
        let flow = write_flow(tmp.path(), "f.test.toml", r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "b"
            [[flow.apps.devices]]
            os = "ios"
        "#);
        let apps = vec![
            project_app("a", "com.a", Some("scripts/a.sh")),
            project_app("b", "com.b", Some("scripts/b.sh")),
        ];
        let suite = plan(&[flow], &apps, tmp.path(), None).await.unwrap();
        let bundles: Vec<_> = suite
            .install_matrix
            .iter()
            .map(|e| e.bundle_id.clone())
            .collect();
        assert_eq!(bundles, vec!["com.b".to_string()]);
    }

    #[tokio::test]
    async fn plan_bad_flow_moves_to_parse_failures_not_error() {
        let tmp = TempDir::new().unwrap();
        let good = write_flow(tmp.path(), "good.test.toml", r#"
            [flow]
            name = "good"
            [[flow.apps]]
            name = "a"
            [[flow.apps.devices]]
            os = "ios"
        "#);
        let missing = tmp.path().join("does-not-exist.test.toml");
        let bad_syntax = write_flow(tmp.path(), "bad.test.toml", "this is not [[[valid toml");
        let apps = vec![project_app("a", "com.a", None)];
        let suite = plan(&[good, missing.clone(), bad_syntax.clone()], &apps, tmp.path(), None)
            .await
            .expect("plan SHALL succeed even when some files fail to parse");
        assert_eq!(suite.flows.len(), 1, "only the parseable flow SHALL remain");
        assert_eq!(
            suite.parse_failures.len(),
            2,
            "both the missing file and the bad-syntax file SHALL be in parse_failures",
        );
        let paths: Vec<_> = suite.parse_failures.iter().map(|f| f.path.clone()).collect();
        assert!(paths.contains(&missing));
        assert!(paths.contains(&bad_syntax));
    }

    #[tokio::test]
    async fn plan_flow_only_install_script_still_in_matrix() {
        let tmp = TempDir::new().unwrap();
        let flow = write_flow(tmp.path(), "f.test.toml", r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "a"
            bundle = "com.flow.a"
            install_script = "scripts/flow-only.sh"
            [[flow.apps.devices]]
            os = "ios"
        "#);
        let suite = plan(&[flow], &[], tmp.path(), None).await.unwrap();
        assert_eq!(suite.install_matrix.len(), 1);
        assert!(suite.install_matrix[0].script_path.ends_with("scripts/flow-only.sh"));
    }
}
