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
use golem_devices::{DeviceInfo, DeviceState, DeviceType, OsVersionSpec, Platform};
use golem_devices::version::{parse_os_version, resolve_latest};
use golem_parser::mixin::expand_mixins;
use golem_parser::{parse_flow, AppConfig, CoverageStrategy, FlowFile, ProjectAppConfig};

use crate::coverage::set_cover_greedy;
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

    let install_matrix = build_install_matrix(&flows, &flow_runs, project_root);
    let device_availability = compute_device_availability(&flow_runs, &snapshot);

    Ok(ParsedSuite {
        flows,
        flow_runs,
        coverage_groups,
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
        parts.push(if phys { "physical".into() } else { "sim".into() });
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
    platform: Option<Platform>,
    os_version: Option<OsVersionSpec>,
    device_type: Option<DeviceType>,
    physical: Option<bool>,
    name: Option<String>,
    playstore: Option<bool>,
    accessibility_label: Option<String>,
    booted: Option<bool>,
}

/// Expand a flow into one or more `FlowRun`s. Each FlowRun holds slots
/// that run simultaneously. Coverage fan-out produces multiple FlowRuns;
/// multi-device coordination produces multiple slots within one FlowRun.
fn expand_flow(
    flow_idx: usize,
    flow: &FlowFile,
    snapshot: &[DeviceInfo],
    platform_override: Option<Platform>,
    coverage_override: Option<CoverageStrategy>,
    coverage_groups: &mut Vec<CoverageGroup>,
) -> Result<Vec<FlowRun>> {
    // Precedence: CLI --coverage > [flow.options].coverage > default (Smart).
    let strategy = coverage_override
        .or_else(|| flow.flow.options.as_ref().and_then(|o| o.coverage))
        .unwrap_or(CoverageStrategy::Smart);

    // Step 1: per-app, expand [[flow.apps.devices]] into concrete
    // AppRequirements. Strategy is passed so each app can emit partial
    // (axis-independent) boxes for Min/Smart/One or Cartesian for Full.
    let app_reqs: Vec<(String, Vec<AppRequirement>)> = flow
        .flow
        .apps
        .iter()
        .map(|app| {
            let reqs =
                expand_app_requirements(app, snapshot, platform_override, strategy)?;
            Ok::<_, anyhow::Error>((app.name.clone(), reqs))
        })
        .collect::<Result<Vec<_>>>()?;

    if app_reqs.is_empty() || app_reqs.iter().all(|(_, r)| r.is_empty()) {
        return Ok(Vec::new());
    }

    match strategy {
        CoverageStrategy::Full => expand_full(flow_idx, &app_reqs),
        CoverageStrategy::Min => expand_min(flow_idx, &app_reqs, snapshot),
        CoverageStrategy::Smart => {
            expand_jit(flow_idx, &app_reqs, snapshot, coverage_groups, CoverageStrategy::Smart, None)
        }
        CoverageStrategy::One => {
            expand_jit(flow_idx, &app_reqs, snapshot, coverage_groups, CoverageStrategy::One, Some(1))
        }
    }
}

/// Full (Cartesian): one FlowRun per coverage combo, cycled across apps
/// by index. Preserves pre-tick-box behaviour for users who opt in.
fn expand_full(
    flow_idx: usize,
    app_reqs: &[(String, Vec<AppRequirement>)],
) -> Result<Vec<FlowRun>> {

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

        for (app_name, reqs) in app_reqs {
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
                    booted: req.booted,
                    apps: vec![app_name.clone()],
                });
            }
        }

        if !slots.is_empty() {
            runs.push(FlowRun {
                flow_idx,
                slots,
                coverage_group: None,
                covers_boxes: Vec::new(),
            });
        }
    }

    Ok(runs)
}

/// Min: per-app greedy set-cover reduces each app's tick-box pool to
/// the minimum device set ticking every box, then the per-app picked-
/// slot lists are cycled to form FlowRuns (same cycling + packing as
/// Full, just over the reduced set). No coverage group registered —
/// every FlowRun runs unconditionally.
///
/// Two apps on different platforms → separate slots in one FlowRun
/// (chat-test coordination preserved). Coverage fan-out within one app
/// → multiple FlowRuns (run per picked device).
///
/// Empty snapshot: skip set-cover entirely and emit boxes as abstract
/// FlowRuns — the scheduler gets a chance at execute time (auto-boot,
/// create-if-missing, waiting). Matches pre-tick-box fallback for
/// abstract `Latest` specs.
fn expand_min(
    flow_idx: usize,
    app_reqs: &[(String, Vec<AppRequirement>)],
    snapshot: &[DeviceInfo],
) -> Result<Vec<FlowRun>> {
    let reduced: Vec<(String, Vec<AppRequirement>)> = app_reqs
        .iter()
        .map(|(name, reqs)| {
            let picked = reduce_app_reqs_via_cover(reqs, snapshot)
                .with_context(|| format!("app \"{name}\""))?;
            Ok::<_, anyhow::Error>((name.clone(), picked))
        })
        .collect::<Result<_>>()?;
    // Delegate run-cycling + slot-packing to the Full emitter; it already
    // handles coordination (multi-slot FlowRuns when apps' platforms
    // differ) and multi-app packing.
    expand_full(flow_idx, &reduced)
}

/// Apply greedy set-cover to one app's requirements. Returns either:
/// - Snapshot empty: the reqs unchanged (scheduler resolves at execute).
/// - Snapshot non-empty: one requirement per picked device, with axes
///   unioned across the boxes that device ticks.
///
/// Errors if snapshot has devices but some boxes go unticked — this is
/// the `Min` / `Smart` underspec check. `One` uses
/// [`reduce_app_reqs_via_cover_lenient`] which ignores uncovered boxes.
fn reduce_app_reqs_via_cover(
    reqs: &[AppRequirement],
    snapshot: &[DeviceInfo],
) -> Result<Vec<AppRequirement>> {
    let (reduced, uncovered) = cover_and_union(reqs, snapshot);
    if !uncovered.is_empty() {
        anyhow::bail!(
            "coverage = \"min\" cannot be satisfied — no devices match: {}",
            uncovered.join(", ")
        );
    }
    Ok(reduced)
}

/// Union a group of `AppRequirement`s — each field takes its first
/// `Some(_)` in iteration order. Conflicts are impossible by
/// construction (the picked device satisfies every input box, so their
/// `Some(_)` axes must be mutually compatible).
fn union_requirements(reqs: &[&AppRequirement]) -> AppRequirement {
    AppRequirement {
        platform: reqs.iter().find_map(|r| r.platform),
        os_version: reqs.iter().find_map(|r| r.os_version.clone()),
        device_type: reqs.iter().find_map(|r| r.device_type),
        physical: reqs.iter().find_map(|r| r.physical),
        name: reqs.iter().find_map(|r| r.name.clone()),
        playstore: reqs.iter().find_map(|r| r.playstore),
        accessibility_label: reqs.iter().find_map(|r| r.accessibility_label.clone()),
        booted: reqs.iter().find_map(|r| r.booted),
    }
}

/// JIT generator shared by `One` and `Smart`. Emits FlowRuns derived
/// from the same greedy set-cover that `Min` uses — so slots come out
/// fully-pinned and the scheduler doesn't face partial-axis (platform-
/// None) slot requirements it can't act on. What makes this JIT rather
/// than plan-only: each FlowRun carries a shared
/// [`CoverageGroup`] index + `covers_boxes`, and the scheduler gates
/// every spawn on live group progress.
///
/// The group stops dispatching members once either
/// - `max_runs` successful runs have completed (`One` → 1), OR
/// - every box in the pool has been ticked (`Smart` → `max_runs = None`).
///
/// The pool is the post-cover reduced per-app requirement list —
/// identical to what `expand_min` emits — concatenated across apps. A
/// picked device matching multiple pool entries credits bonus ticks to
/// the tracker, so e.g. `os = ["ios:latest", "android:latest"]` +
/// `type = ["phone", "tablet"]` on a single iPad-26 + Pixel-tab setup
/// can terminate `Smart` after 2 runs (one per picked device).
fn expand_jit(
    flow_idx: usize,
    app_reqs: &[(String, Vec<AppRequirement>)],
    snapshot: &[DeviceInfo],
    coverage_groups: &mut Vec<CoverageGroup>,
    strategy: CoverageStrategy,
    max_runs: Option<u32>,
) -> Result<Vec<FlowRun>> {
    // Reduce each app's tick-box list the same way Min does: greedy
    // set-cover against the snapshot, then union the covered boxes per
    // picked device into concrete `AppRequirement`s. Slots end up
    // fully-pinned — no platform-None leaks into the scheduler.
    //
    // For `One` we tolerate uncovered boxes (underspec), since any
    // single success satisfies the strategy. For `Smart` we let
    // `reduce_app_reqs_via_cover` error on uncovered boxes, matching
    // `Min`'s semantics.
    let reduced: Vec<(String, Vec<AppRequirement>)> = app_reqs
        .iter()
        .map(|(name, reqs)| {
            let picked = match strategy {
                CoverageStrategy::One => reduce_app_reqs_via_cover_lenient(reqs, snapshot),
                _ => reduce_app_reqs_via_cover(reqs, snapshot)
                    .with_context(|| format!("app \"{name}\""))?,
            };
            Ok::<_, anyhow::Error>((name.clone(), picked))
        })
        .collect::<Result<_>>()?;

    if reduced.iter().all(|(_, r)| r.is_empty()) {
        anyhow::bail!(
            "coverage = \"{}\" found no device matching any tick box for this flow",
            match strategy {
                CoverageStrategy::One => "one",
                CoverageStrategy::Smart => "smart",
                _ => "jit",
            }
        );
    }

    // Pool = flat concat of each app's reduced AppRequirements as tick
    // boxes. Each FlowRun's covers_boxes then follows the cycling
    // `expand_full` uses: app_a's req at index `i % reqs.len()`, etc.
    let mut pool: Vec<DeviceSlot> = Vec::new();
    let mut per_app_pool_base: Vec<usize> = Vec::with_capacity(reduced.len());
    for (_, reqs) in &reduced {
        per_app_pool_base.push(pool.len());
        pool.extend(reqs.iter().map(req_to_tick_box));
    }

    let group_idx = coverage_groups.len();
    coverage_groups.push(CoverageGroup {
        flow_idx,
        strategy,
        boxes: pool,
        max_runs,
    });

    let mut runs = expand_full(flow_idx, &reduced)?;
    for (i, run) in runs.iter_mut().enumerate() {
        run.coverage_group = Some(group_idx);
        let mut covers = Vec::with_capacity(reduced.len());
        for (app_idx, (_, reqs)) in reduced.iter().enumerate() {
            if reqs.is_empty() {
                continue;
            }
            let pool_idx = per_app_pool_base[app_idx] + (i % reqs.len());
            covers.push(pool_idx);
        }
        run.covers_boxes = covers;
    }
    Ok(runs)
}

/// Lenient variant of [`reduce_app_reqs_via_cover`] for `coverage = "one"`.
/// Uncovered boxes are silently dropped: a smoke run doesn't need every
/// axis represented, just one successful run. If no box is ever
/// reachable (empty picked set + non-empty snapshot), returns an empty
/// Vec so the caller can bail with a friendly error.
fn reduce_app_reqs_via_cover_lenient(
    reqs: &[AppRequirement],
    snapshot: &[DeviceInfo],
) -> Vec<AppRequirement> {
    cover_and_union(reqs, snapshot).0
}

/// Shared set-cover + union core. Returns `(reduced, uncovered_labels)`:
///
/// - `reduced` — one `AppRequirement` per picked device, with every
///   `Some(_)` axis from the boxes that device ticks unioned in.
///   Example: partial `{ios, 26, ·}` + `{·, ·, tablet}` both ticked by
///   one iPad-v26 → union `{ios, 26, tablet}`.
/// - `uncovered_labels` — shape strings for boxes no picked device
///   satisfies. Empty means full coverage; non-empty is the underspec
///   error signal strict callers (`Min` / `Smart`) raise.
///
/// Empty `reqs` or empty `snapshot` short-circuits with full inputs
/// preserved and no "uncovered" claim (the scheduler gets a chance at
/// execute time to auto-boot, create-if-missing, or wait).
fn cover_and_union(
    reqs: &[AppRequirement],
    snapshot: &[DeviceInfo],
) -> (Vec<AppRequirement>, Vec<String>) {
    if reqs.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if snapshot.is_empty() {
        return (reqs.to_vec(), Vec::new());
    }
    let boxes: Vec<DeviceSlot> = reqs.iter().map(req_to_tick_box).collect();
    let picked = set_cover_greedy(&boxes, snapshot);

    let mut covered: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let reduced: Vec<AppRequirement> = picked
        .into_iter()
        .map(|d_idx| {
            let mut ticks: Vec<&AppRequirement> = Vec::new();
            for (b_idx, b) in boxes.iter().enumerate() {
                if device_matches_slot(&snapshot[d_idx], b) {
                    covered.insert(b_idx);
                    ticks.push(&reqs[b_idx]);
                }
            }
            union_requirements(&ticks)
        })
        .collect();

    let uncovered: Vec<String> = (0..boxes.len())
        .filter(|b| !covered.contains(b))
        .map(|b| shape_label(&boxes[b]))
        .collect();
    (reduced, uncovered)
}

/// Convert an `AppRequirement` into a `DeviceSlot` tick box without
/// apps. Used as the input to set-cover.
fn req_to_tick_box(req: &AppRequirement) -> DeviceSlot {
    DeviceSlot {
        platform: req.platform,
        os_version: req.os_version.clone(),
        device_type: req.device_type,
        physical: req.physical,
        name: req.name.clone(),
        playstore: req.playstore,
        accessibility_label: req.accessibility_label.clone(),
        booted: req.booted,
        apps: Vec::new(),
    }
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
        && slot.booted == req.booted
}

/// Expand an `AppConfig`'s `[[flow.apps.devices]]` constraints into flat
/// `AppRequirement`s (tick boxes for this app).
///
/// - Empty `devices` block: emit one partial box per currently-booted
///   platform in the snapshot (with `booted=true`). Error if nothing is
///   booted — no iOS bias.
/// - `Full` strategy: Cartesian cross-product across axes — every combo
///   is a distinct fully-pinned box.
/// - `Min` / `Smart` / `One` strategy: partial-axis emission — when
///   multiple axes are multi-valued in a block, emit one box per axis
///   value (other multi-axes left `None`; single-valued axes pin
///   alongside). When only one axis is multi, this collapses to the same
///   as Full for that block.
/// - `ios:latest:N` is resolved against the snapshot. Too few matching
///   majors is a plan-time error (except under `One`).
fn expand_app_requirements(
    app: &AppConfig,
    snapshot: &[DeviceInfo],
    platform_override: Option<Platform>,
    strategy: CoverageStrategy,
) -> Result<Vec<AppRequirement>> {
    if app.devices.is_empty() {
        return default_any_booted_requirements(snapshot, platform_override);
    }

    let mut out: Vec<AppRequirement> = Vec::new();
    for dc in &app.devices {
        let os_pairs =
            expand_os_pairs(dc, snapshot, platform_override, strategy)?;
        let type_entries = expand_type_entries(dc)?;
        let hardware_entries = expand_hardware_entries(dc)?;

        // Narrow to the forced platform override, if any.
        let filtered_os: Vec<(Platform, Option<OsVersionSpec>)> = os_pairs
            .into_iter()
            .filter(|(p, _)| platform_override.map(|f| f == *p).unwrap_or(true))
            .collect();
        if filtered_os.is_empty() {
            continue;
        }

        let os_multi = filtered_os.len() > 1;
        let type_multi = type_entries.len() > 1;
        let hw_multi = hardware_entries.len() > 1;
        let multi_count =
            [os_multi, type_multi, hw_multi].iter().filter(|&&x| x).count();
        let partial_strategy = matches!(
            strategy,
            CoverageStrategy::Min | CoverageStrategy::Smart | CoverageStrategy::One
        );

        if partial_strategy && multi_count >= 2 {
            // Partial-axis emission — one box per value on each multi axis,
            // other axes left as `None` so the picked device can satisfy
            // multiple tick boxes at once. Single-valued axes pin on every
            // box they show up in; only pass through if multi on its axis.
            if os_multi {
                for (platform, os_version) in &filtered_os {
                    out.push(AppRequirement {
                        platform: Some(*platform),
                        os_version: os_version.clone(),
                        device_type: None,
                        physical: None,
                        name: dc.name.clone(),
                        playstore: dc.playstore,
                        accessibility_label: dc.accessibility_label.clone(),
                        booted: dc.booted,
                    });
                }
            }
            if type_multi {
                for type_e in &type_entries {
                    out.push(AppRequirement {
                        platform: None,
                        os_version: None,
                        device_type: *type_e,
                        physical: None,
                        name: dc.name.clone(),
                        playstore: dc.playstore,
                        accessibility_label: dc.accessibility_label.clone(),
                        booted: dc.booted,
                    });
                }
            }
            if hw_multi {
                for phys in &hardware_entries {
                    out.push(AppRequirement {
                        platform: None,
                        os_version: None,
                        device_type: None,
                        physical: *phys,
                        name: dc.name.clone(),
                        playstore: dc.playstore,
                        accessibility_label: dc.accessibility_label.clone(),
                        booted: dc.booted,
                    });
                }
            }
        } else {
            // Full, OR partial strategies with ≤1 multi-axis: Cartesian.
            for (platform, os_version) in &filtered_os {
                for type_e in &type_entries {
                    for phys in &hardware_entries {
                        out.push(AppRequirement {
                            platform: Some(*platform),
                            os_version: os_version.clone(),
                            device_type: *type_e,
                            physical: *phys,
                            name: dc.name.clone(),
                            playstore: dc.playstore,
                            accessibility_label: dc.accessibility_label.clone(),
                            booted: dc.booted,
                        });
                    }
                }
            }
        }
    }

    // Dedup — two blocks may emit the same box (e.g. redundant subset
    // constraints). Order-preserving dedup via seen-set on signature.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    out.retain(|r| seen.insert(req_signature(r)));
    Ok(out)
}

/// Emit a default box per currently-booted platform. If no platform has
/// booted devices, error — we do not silently pick a platform.
fn default_any_booted_requirements(
    snapshot: &[DeviceInfo],
    platform_override: Option<Platform>,
) -> Result<Vec<AppRequirement>> {
    let mut platforms: Vec<Platform> = [Platform::Ios, Platform::Android]
        .into_iter()
        .filter(|p| {
            platform_override.map(|f| &f == p).unwrap_or(true)
                && snapshot
                    .iter()
                    .any(|d| d.platform == *p && d.state == DeviceState::Booted)
        })
        .collect();
    // Platforms don't implement Ord; iOS-first by construction. Dedup
    // defensively — the filter iterates the static [Ios, Android] list.
    platforms.dedup_by(|a, b| a == b);
    if platforms.is_empty() {
        anyhow::bail!(
            "No `[[flow.apps.devices]]` block and no booted device found. \
             Boot a simulator/emulator or add a device constraint."
        );
    }
    Ok(platforms
        .into_iter()
        .map(|p| AppRequirement {
            platform: Some(p),
            os_version: None,
            device_type: None,
            physical: Some(false),
            name: None,
            playstore: None,
            accessibility_label: None,
            booted: Some(true),
        })
        .collect())
}

/// Expand the `os` field of one `[[flow.apps.devices]]` block into a
/// list of (platform, version-spec) pairs. Handles `ios:latest:N` with
/// strict underspec checking (except under `One`).
fn expand_os_pairs(
    dc: &golem_parser::DeviceConstraint,
    snapshot: &[DeviceInfo],
    platform_override: Option<Platform>,
    strategy: CoverageStrategy,
) -> Result<Vec<(Platform, Option<OsVersionSpec>)>> {
    let Some(os_sv) = &dc.os else {
        // No `os` → default to override platform when forced, else
        // emit a box per platform (any version) so partial-axis covers.
        return Ok(match platform_override {
            Some(p) => vec![(p, None)],
            None => vec![(Platform::Ios, None), (Platform::Android, None)],
        });
    };

    let mut pairs: Vec<(Platform, Option<OsVersionSpec>)> = Vec::new();
    for s in os_sv.to_vec() {
        // `os = "any"` → platform-agnostic, any version. Emitted as a
        // box with platform=None by letting the caller see no platform
        // pair here; we synthesise a platform-less marker via both
        // platforms-any-version AND a None-platform sentinel. Simpler:
        // treat `any` as "every platform, any version" — set-cover will
        // pick whatever has devices.
        if s == "any" {
            pairs.push((Platform::Ios, None));
            pairs.push((Platform::Android, None));
            continue;
        }

        match parse_os_version(&s) {
            Ok(OsVersionSpec::Latest { platform, count }) => {
                let majors: Vec<u32> = snapshot
                    .iter()
                    .filter(|d| d.platform == platform)
                    .map(|d| d.os_major)
                    .collect();
                let tops = resolve_latest(platform, count, &majors);
                if tops.is_empty() {
                    // No matching platform devices in snapshot → fall back
                    // to abstract Latest (scheduler re-tries at execute).
                    pairs.push((
                        platform,
                        Some(OsVersionSpec::Latest { platform, count }),
                    ));
                } else if (tops.len() as u32) < count && strategy != CoverageStrategy::One {
                    anyhow::bail!(
                        "`os = \"{s}\"` requested {count} versions but only {} \
                         available in snapshot ({}). Boot another runtime or \
                         use coverage = \"one\" for local smoke testing.",
                        tops.len(),
                        tops.iter()
                            .map(|m| format!("v{m}"))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
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
    Ok(pairs)
}

/// Expand the `type` field. Absent → one entry `None` (any type).
/// Unrecognised values are a hard error — catches `"Tablet"`, `"phone "`,
/// typos, etc. before they silently map to "any type".
fn expand_type_entries(
    dc: &golem_parser::DeviceConstraint,
) -> Result<Vec<Option<DeviceType>>> {
    let Some(type_sv) = &dc.device_type else {
        return Ok(vec![None]);
    };
    type_sv
        .to_vec()
        .iter()
        .map(|s| match s.as_str() {
            "phone" => Ok(Some(DeviceType::Phone)),
            "tablet" => Ok(Some(DeviceType::Tablet)),
            other => anyhow::bail!(
                "unrecognised `type` value: {other:?}. Expected \"phone\" or \"tablet\"."
            ),
        })
        .collect()
}

/// Expand the `hardware` field. Absent → `[Some(false)]` (virtual-only
/// default — physical devices require explicit opt-in). Single string
/// → one entry. Array → N entries (partial-axis expansion candidate).
/// Unrecognised values error out with the allowed list.
fn expand_hardware_entries(
    dc: &golem_parser::DeviceConstraint,
) -> Result<Vec<Option<bool>>> {
    let Some(hw_sv) = &dc.hardware else {
        return Ok(vec![Some(false)]);
    };
    let values = hw_sv.to_vec();
    if values.is_empty() {
        anyhow::bail!(
            "`hardware = []` matches no device — omit the field for the \
             virtual-only default, or list at least one value."
        );
    }
    values
        .iter()
        .map(|s| match s.as_str() {
            "virtual" => Ok(Some(false)),
            "real" => Ok(Some(true)),
            other => anyhow::bail!(
                "unrecognised `hardware` value: {other:?}. \
                 Expected \"virtual\" (sim/emulator) or \"real\" (physical device)."
            ),
        })
        .collect()
}

fn req_signature(r: &AppRequirement) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        r.platform,
        r.os_version,
        r.device_type,
        r.physical,
        r.name,
        r.playstore,
        r.accessibility_label,
        r.booted,
    )
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
        let suite = plan(&[flow], &apps, tmp.path(), None, None).await.unwrap();
        assert_eq!(suite.flow_runs.len(), 1);
        assert_eq!(suite.flow_runs[0].slots.len(), 1);
        assert_eq!(suite.flow_runs[0].slots[0].platform, Some(Platform::Ios));
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
        let suite = plan(&[flow], &apps, tmp.path(), None, None).await.unwrap();
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
        let suite = plan(&[flow], &apps, tmp.path(), None, None).await.unwrap();
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
        let suite = plan(&[flow], &apps, tmp.path(), None, None).await.unwrap();
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
        let suite = plan(&[flow], &apps, tmp.path(), None, None).await.unwrap();
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
        let suite = plan(&[flow], &apps, tmp.path(), None, None).await.unwrap();
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
        let suite = plan(&[flow], &apps, tmp.path(), None, None).await.unwrap();
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
        let suite = plan(&[flow], &apps, tmp.path(), Some(Platform::Android), None).await.unwrap();
        for run in &suite.flow_runs {
            for slot in &run.slots {
                assert_eq!(slot.platform, Some(Platform::Android),
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
        let suite = plan(&[flow], &apps, tmp.path(), None, None).await.unwrap();
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
        let suite = plan(&[flow], &apps, tmp.path(), None, None).await.unwrap();
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
        let suite = plan(&[good, missing.clone(), bad_syntax.clone()], &apps, tmp.path(), None, None)
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
        let suite = plan(&[flow], &[], tmp.path(), None, None).await.unwrap();
        assert_eq!(suite.install_matrix.len(), 1);
        assert!(suite.install_matrix[0].script_path.ends_with("scripts/flow-only.sh"));
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
            state: if booted { DeviceState::Booted } else { DeviceState::Shutdown },
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
        let app = mk_app_with_devices("a", vec![dc(
            Some(vec!["ios:18", "ios:26"]),
            Some(vec!["phone", "tablet"]),
        )]);
        let snap: Vec<DeviceInfo> = Vec::new();
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Full).unwrap();
        assert_eq!(reqs.len(), 4,
            "Full SHALL emit full Cartesian (2 os × 2 types = 4 boxes)");
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
        let app = mk_app_with_devices("a", vec![dc(
            Some(vec!["ios:18", "ios:26"]),
            Some(vec!["phone", "tablet"]),
        )]);
        let snap: Vec<DeviceInfo> = Vec::new();
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min).unwrap();
        assert_eq!(reqs.len(), 4);
        // 2 os-only boxes (device_type=None) + 2 type-only boxes (os_version=None).
        let os_only = reqs.iter().filter(|r| r.os_version.is_some() && r.device_type.is_none()).count();
        let type_only = reqs.iter().filter(|r| r.os_version.is_none() && r.device_type.is_some()).count();
        assert_eq!(os_only, 2, "SHALL emit 2 partial os boxes");
        assert_eq!(type_only, 2, "SHALL emit 2 partial type boxes");
    }

    // Min with only one multi-valued axis → collapses to Cartesian-like (same as Full).
    #[test]
    fn expand_app_requirements_min_single_multi_axis_fully_pinned() {
        let app = mk_app_with_devices("a", vec![dc(
            Some(vec!["ios:latest"]),          // single os
            Some(vec!["phone", "tablet"]),     // 2 types
        )]);
        let snap = vec![mk_device("iPad", "u1", Platform::Ios, 26, DeviceType::Tablet, true)];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min).unwrap();
        assert_eq!(reqs.len(), 2, "single-multi-axis SHALL collapse to 2 fully-pinned boxes");
        for r in &reqs {
            assert!(r.os_version.is_some(), "os SHALL remain pinned");
            assert!(r.device_type.is_some(), "type SHALL remain pinned");
        }
    }

    // Responsive-design: 2 platforms × 2 types under Min → 4 partial boxes,
    // 2 devices cover all (iOS-phone + Android-tablet).
    #[test]
    fn expand_min_responsive_design_two_devices_cover_four_boxes() {
        let app = mk_app_with_devices("a", vec![dc(
            Some(vec!["ios:latest", "android:latest"]),
            Some(vec!["phone", "tablet"]),
        )]);
        let snap = vec![
            mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true),
            mk_device("Pixel-tab", "u2", Platform::Android, 34, DeviceType::Tablet, true),
        ];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min).unwrap();
        let reduced = reduce_app_reqs_via_cover(&reqs, &snap).unwrap();
        assert_eq!(reduced.len(), 2,
            "min-cover SHALL use exactly 2 devices to cover the 4 responsive axes");
    }

    // Underspec: ios:latest:2 with snapshot containing only 1 iOS major → error.
    #[test]
    fn expand_app_requirements_underspec_latest_errors() {
        let app = mk_app_with_devices("a", vec![dc(Some(vec!["ios:latest:2"]), None)]);
        let snap = vec![mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true)];
        let result = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min);
        assert!(result.is_err(), "ios:latest:2 with 1 version SHALL error under Min");
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("requested 2"), "error SHALL mention requested count: {msg}");
    }

    // Underspec under "one" strategy — no error, takes what's available.
    #[test]
    fn expand_app_requirements_underspec_one_strategy_tolerates() {
        let app = mk_app_with_devices("a", vec![dc(Some(vec!["ios:latest:2"]), None)]);
        let snap = vec![mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true)];
        let result = expand_app_requirements(&app, &snap, None, CoverageStrategy::One);
        assert!(result.is_ok(), "One SHALL tolerate underspec: {:?}", result.err());
    }

    // Empty devices block with booted iOS → one booted-platform box.
    #[test]
    fn expand_app_requirements_empty_devices_emits_booted_platform_boxes() {
        let app = mk_app_with_devices("a", vec![]);
        let snap = vec![
            mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true),
        ];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min).unwrap();
        assert_eq!(reqs.len(), 1, "one booted platform → one default box");
        assert_eq!(reqs[0].platform, Some(Platform::Ios));
        assert_eq!(reqs[0].booted, Some(true),
            "default empty-devices box SHALL require a booted device");
    }

    // Empty devices block with both platforms booted → one box per.
    #[test]
    fn expand_app_requirements_empty_devices_both_platforms_booted() {
        let app = mk_app_with_devices("a", vec![]);
        let snap = vec![
            mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true),
            mk_device("Pixel", "u2", Platform::Android, 34, DeviceType::Phone, true),
        ];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min).unwrap();
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
            "iPhone-offline", "u1", Platform::Ios, 26, DeviceType::Phone, /*booted=*/ false,
        )];
        let result = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min);
        assert!(result.is_err(), "nothing booted SHALL error (no iOS bias fallback)");
    }

    // Dedup: two redundant blocks emit the same box → deduped.
    #[test]
    fn expand_app_requirements_dedups_overlapping_blocks() {
        let app = mk_app_with_devices("a", vec![
            dc(Some(vec!["ios:26"]), Some(vec!["phone"])),
            dc(Some(vec!["ios:26"]), Some(vec!["phone"])), // identical — dedup
        ]);
        let snap: Vec<DeviceInfo> = Vec::new();
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min).unwrap();
        assert_eq!(reqs.len(), 1, "identical blocks SHALL dedup to 1 requirement");
    }

    // reduce_app_reqs_via_cover: one device covers both axes → 1 pinned requirement.
    #[test]
    fn reduce_app_reqs_via_cover_one_device_covers_all() {
        let reqs = vec![
            // Partial boxes: os + type
            AppRequirement {
                platform: Some(Platform::Ios),
                os_version: Some(OsVersionSpec::Exact { platform: Platform::Ios, major: 26 }),
                device_type: None, physical: None, name: None, playstore: None,
                accessibility_label: None, booted: None,
            },
            AppRequirement {
                platform: None, os_version: None,
                device_type: Some(DeviceType::Tablet),
                physical: None, name: None, playstore: None,
                accessibility_label: None, booted: None,
            },
        ];
        let snap = vec![mk_device("iPad-26", "u1", Platform::Ios, 26, DeviceType::Tablet, true)];
        let reduced = reduce_app_reqs_via_cover(&reqs, &snap).unwrap();
        assert_eq!(reduced.len(), 1, "one device SHALL satisfy both partial boxes");
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
        assert!(result.is_err(), "Unknown type SHALL error, not silently map to any-type");
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("Tablet"), "error SHALL include the offending value: {msg}");
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
        let result = expand_hardware_entries(&dc).unwrap();
        assert_eq!(result, vec![Some(false)],
            "SHALL default to virtual-only when `hardware` is omitted");
    }

    #[test]
    fn expand_hardware_entries_single_virtual_pins_false() {
        let dc = dc_with_hardware(Some(golem_parser::StringOrVec::Single("virtual".into())));
        let result = expand_hardware_entries(&dc).unwrap();
        assert_eq!(result, vec![Some(false)]);
    }

    #[test]
    fn expand_hardware_entries_single_real_pins_true() {
        let dc = dc_with_hardware(Some(golem_parser::StringOrVec::Single("real".into())));
        let result = expand_hardware_entries(&dc).unwrap();
        assert_eq!(result, vec![Some(true)]);
    }

    #[test]
    fn expand_hardware_entries_array_form_emits_two() {
        let dc = dc_with_hardware(Some(golem_parser::StringOrVec::Multiple(
            vec!["virtual".into(), "real".into()],
        )));
        let result = expand_hardware_entries(&dc).unwrap();
        assert_eq!(result, vec![Some(false), Some(true)],
            "SHALL emit one entry per axis value, preserving order");
    }

    #[test]
    fn expand_hardware_entries_rejects_empty_array() {
        let dc = dc_with_hardware(Some(golem_parser::StringOrVec::Multiple(vec![])));
        let result = expand_hardware_entries(&dc);
        assert!(result.is_err(),
            "SHALL reject `hardware = []` instead of silently emitting zero boxes");
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("omit"), "error SHALL suggest omitting the field: {msg}");
    }

    #[test]
    fn expand_hardware_entries_rejects_unknown_value() {
        let dc = dc_with_hardware(Some(golem_parser::StringOrVec::Single("sim".into())));
        let result = expand_hardware_entries(&dc);
        assert!(result.is_err(),
            "SHALL reject unknown values (e.g. \"sim\" instead of \"virtual\")");
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("sim"), "error SHALL include offending value: {msg}");
        assert!(msg.contains("virtual"), "error SHALL name allowed \"virtual\": {msg}");
        assert!(msg.contains("real"), "error SHALL name allowed \"real\": {msg}");
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
            mk_device("Pixel", "u2", Platform::Android, 34, DeviceType::Phone, true),
        ];
        let a_reqs = expand_app_requirements(&app_a, &snap, None, CoverageStrategy::One).unwrap();
        let b_reqs = expand_app_requirements(&app_b, &snap, None, CoverageStrategy::One).unwrap();
        let mut groups: Vec<CoverageGroup> = Vec::new();
        let runs = expand_jit(
            0,
            &[("a".to_string(), a_reqs), ("b".to_string(), b_reqs)],
            &snap,
            &mut groups,
            CoverageStrategy::One,
            Some(1),
        )
        .unwrap();
        assert_eq!(runs.len(), 1, "one SHALL emit a single FlowRun");
        let slots = &runs[0].slots;
        assert_eq!(slots.len(), 2,
            "multi-app flow SHALL produce one slot per incompatible platform");
        let apps: std::collections::HashSet<_> = slots
            .iter()
            .flat_map(|s| s.apps.iter().cloned())
            .collect();
        assert!(apps.contains("a"), "app a SHALL be present");
        assert!(apps.contains("b"), "app b SHALL be present — not silently dropped");
        assert_eq!(groups.len(), 1, "JIT-one SHALL register one coverage group");
        assert_eq!(groups[0].max_runs, Some(1));
        assert_eq!(groups[0].strategy, CoverageStrategy::One);
        assert_eq!(runs[0].coverage_group, Some(0));
        assert_eq!(runs[0].covers_boxes.len(), 2,
            "one FlowRun SHALL cover one pool entry per app");
    }

    // JIT-one with coverage fan-out: multi-version os SHALL produce
    // N FlowRuns (one per reachable version), all sharing one group.
    #[test]
    fn expand_one_os_fanout_produces_n_flowruns_sharing_group() {
        let app = mk_app_with_devices("a", vec![dc(Some(vec!["ios:18", "ios:26"]), None)]);
        let snap = vec![
            mk_device("iPhone-18", "u1", Platform::Ios, 18, DeviceType::Phone, true),
            mk_device("iPhone-26", "u2", Platform::Ios, 26, DeviceType::Phone, true),
        ];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::One).unwrap();
        let mut groups: Vec<CoverageGroup> = Vec::new();
        let runs = expand_jit(
            0,
            &[("a".to_string(), reqs)],
            &snap,
            &mut groups,
            CoverageStrategy::One,
            Some(1),
        )
        .unwrap();
        assert_eq!(runs.len(), 2, "SHALL emit one FlowRun per os fan-out");
        assert!(runs.iter().all(|r| r.coverage_group == Some(0)),
            "SHALL share the same group");
        assert_eq!(groups[0].boxes.len(), 2, "pool SHALL hold both reachable boxes");
    }

    // Smart strategy: same fan-out but max_runs=None → stop-on-all-ticked.
    #[test]
    fn expand_smart_strategy_uses_none_max_runs() {
        let app = mk_app_with_devices("a", vec![dc(Some(vec!["ios:18", "ios:26"]), None)]);
        let snap = vec![
            mk_device("iPhone-18", "u1", Platform::Ios, 18, DeviceType::Phone, true),
            mk_device("iPhone-26", "u2", Platform::Ios, 26, DeviceType::Phone, true),
        ];
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Smart).unwrap();
        let mut groups: Vec<CoverageGroup> = Vec::new();
        let runs = expand_jit(
            0,
            &[("a".to_string(), reqs)],
            &snap,
            &mut groups,
            CoverageStrategy::Smart,
            None,
        )
        .unwrap();
        assert_eq!(groups[0].max_runs, None,
            "Smart SHALL have no run-count cap — stop on pool fully ticked");
        assert_eq!(groups[0].strategy, CoverageStrategy::Smart);
        assert_eq!(runs.len(), 2);
    }

    // JIT with all boxes unreachable in snapshot → error.
    #[test]
    fn expand_jit_errors_when_no_box_reachable() {
        let app = mk_app_with_devices("a", vec![dc(Some(vec!["android:34"]), None)]);
        let reqs = vec![AppRequirement {
            platform: Some(Platform::Android),
            os_version: Some(OsVersionSpec::Exact { platform: Platform::Android, major: 34 }),
            device_type: None, physical: None, name: None, playstore: None,
            accessibility_label: None, booted: None,
        }];
        let snap = vec![mk_device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone, true)];
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
}
