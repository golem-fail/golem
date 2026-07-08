//! Coverage-expansion engine: turns a parsed flow's `[[flow.apps.devices]]`
//! constraints into concrete `FlowRun`s (Full/Min/Smart/One strategies).

use anyhow::{Context, Result};
use golem_devices::version::{parse_os_version, resolve_latest};
use golem_devices::{DeviceInfo, DeviceState, DeviceType, OsVersionSpec, Platform};
use golem_parser::{AppConfig, CoverageStrategy, FlowFile};

use crate::coverage::set_cover_greedy;

use super::{device_matches_slot, shape_label, CoverageGroup, DeviceSlot, FlowRun};

/// A single flattened requirement tuple for one app — one concrete
/// coverage point that scheduler can match a device against.
#[derive(Debug, Clone)]
pub(super) struct AppRequirement {
    pub(super) platform: Option<Platform>,
    pub(super) os_version: Option<OsVersionSpec>,
    pub(super) device_type: Option<DeviceType>,
    pub(super) physical: Option<bool>,
    pub(super) name: Option<String>,
    pub(super) playstore: Option<bool>,
    pub(super) accessibility_label: Option<String>,
    pub(super) booted: Option<bool>,
}

/// Expand a flow into one or more `FlowRun`s. Each FlowRun holds slots
/// that run simultaneously. Coverage fan-out produces multiple FlowRuns;
/// multi-device coordination produces multiple slots within one FlowRun.
pub(super) fn expand_flow(
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
            let reqs = expand_app_requirements(app, snapshot, platform_override, strategy)?;
            Ok::<_, anyhow::Error>((app.name.clone(), reqs))
        })
        .collect::<Result<Vec<_>>>()?;

    if app_reqs.is_empty() || app_reqs.iter().all(|(_, r)| r.is_empty()) {
        return Ok(Vec::new());
    }

    match strategy {
        CoverageStrategy::Full => expand_full(flow_idx, &app_reqs),
        CoverageStrategy::Min => expand_min(flow_idx, &app_reqs, snapshot),
        CoverageStrategy::Smart => expand_jit(
            flow_idx,
            &app_reqs,
            snapshot,
            coverage_groups,
            CoverageStrategy::Smart,
            None,
        ),
        CoverageStrategy::One => expand_jit(
            flow_idx,
            &app_reqs,
            snapshot,
            coverage_groups,
            CoverageStrategy::One,
            Some(1),
        ),
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
                // Default 0 — `plan()` rewrites this when --repeat > 1.
                repeat_index: 0,
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
pub(super) fn reduce_app_reqs_via_cover(
    reqs: &[AppRequirement],
    snapshot: &[DeviceInfo],
) -> Result<Vec<AppRequirement>> {
    let (reduced, uncovered) = cover_and_union(reqs, snapshot);
    if !uncovered.is_empty() {
        return Err(golem_events::coded(
            golem_events::FailureCode::ParseDeviceConstraint,
            anyhow::anyhow!(
                "coverage = \"min\" cannot be satisfied — no devices match: {}",
                uncovered.join(", ")
            ),
        ));
    }
    Ok(reduced)
}

/// Union a group of `AppRequirement`s — each field takes its first
/// `Some(_)` in iteration order. Conflicts are impossible by
/// construction (the picked device satisfies every input box, so their
/// `Some(_)` axes must be mutually compatible).
pub(super) fn union_requirements(reqs: &[&AppRequirement]) -> AppRequirement {
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
pub(super) fn expand_jit(
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
        return Err(golem_events::coded(
            golem_events::FailureCode::ParseDeviceConstraint,
            anyhow::anyhow!(
                "coverage = \"{}\" found no device matching any tick box for this flow",
                match strategy {
                    CoverageStrategy::One => "one",
                    CoverageStrategy::Smart => "smart",
                    _ => "jit",
                }
            ),
        ));
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
pub(super) fn cover_and_union(
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
pub(super) fn slot_compatible_with(slot: &DeviceSlot, req: &AppRequirement) -> bool {
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
pub(super) fn expand_app_requirements(
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
        let os_pairs = expand_os_pairs(dc, snapshot, platform_override, strategy)?;
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
        let multi_count = [os_multi, type_multi, hw_multi]
            .iter()
            .filter(|&&x| x)
            .count();
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
pub(super) fn default_any_booted_requirements(
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
        return Err(golem_events::coded(
            golem_events::FailureCode::ParseDeviceConstraint,
            anyhow::anyhow!(
                "No `[[flow.apps.devices]]` block and no booted device found. \
                 Boot a simulator/emulator or add a device constraint."
            ),
        ));
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
pub(super) fn expand_os_pairs(
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
                    pairs.push((platform, Some(OsVersionSpec::Latest { platform, count })));
                } else if (tops.len() as u32) < count && strategy != CoverageStrategy::One {
                    return Err(golem_events::coded(
                        golem_events::FailureCode::ParseDeviceConstraint,
                        anyhow::anyhow!(
                            "`os = \"{s}\"` requested {count} versions but only {} \
                             available in snapshot ({}). Boot another runtime or \
                             use coverage = \"one\" for local smoke testing.",
                            tops.len(),
                            tops.iter()
                                .map(|m| format!("v{m}"))
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                    ));
                } else {
                    for m in tops {
                        pairs.push((platform, Some(OsVersionSpec::Exact { platform, major: m })));
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
                    return Err(golem_events::coded(
                        golem_events::FailureCode::ParseDeviceConstraint,
                        anyhow::anyhow!("unrecognised os constraint: {s}"),
                    ));
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
pub(super) fn expand_type_entries(
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
            other => Err(golem_events::coded(
                golem_events::FailureCode::ParseDeviceConstraint,
                anyhow::anyhow!(
                    "unrecognised `type` value: {other:?}. Expected \"phone\" or \"tablet\"."
                ),
            )),
        })
        .collect()
}

/// Expand the `hardware` field. Absent → `[Some(false)]` (virtual-only
/// default — physical devices require explicit opt-in). Single string
/// → one entry. Array → N entries (partial-axis expansion candidate).
/// Unrecognised values error out with the allowed list.
pub(super) fn expand_hardware_entries(
    dc: &golem_parser::DeviceConstraint,
) -> Result<Vec<Option<bool>>> {
    let Some(hw_sv) = &dc.hardware else {
        return Ok(vec![Some(false)]);
    };
    let values = hw_sv.to_vec();
    if values.is_empty() {
        return Err(golem_events::coded(
            golem_events::FailureCode::ParseDeviceConstraint,
            anyhow::anyhow!(
                "`hardware = []` matches no device — omit the field for the \
                 virtual-only default, or list at least one value."
            ),
        ));
    }
    values
        .iter()
        .map(|s| match s.as_str() {
            "virtual" => Ok(Some(false)),
            "real" => Ok(Some(true)),
            other => Err(golem_events::coded(
                golem_events::FailureCode::ParseDeviceConstraint,
                anyhow::anyhow!(
                    "unrecognised `hardware` value: {other:?}. \
                     Expected \"virtual\" (sim/emulator) or \"real\" (physical device)."
                ),
            )),
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
    use golem_parser::{parse_flow, DeviceConstraint, StringOrVec};

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

    fn sv(v: Vec<&str>) -> StringOrVec {
        if v.len() == 1 {
            StringOrVec::Single(v[0].into())
        } else {
            StringOrVec::Multiple(v.iter().map(|s| s.to_string()).collect())
        }
    }

    fn dc(
        os: Option<Vec<&str>>,
        types: Option<Vec<&str>>,
        hardware: Option<Vec<&str>>,
    ) -> DeviceConstraint {
        DeviceConstraint {
            os: os.map(sv),
            device_type: types.map(sv),
            name: None,
            accessibility_label: None,
            hardware: hardware.map(sv),
            playstore: None,
            booted: None,
            expand: None,
        }
    }

    fn req(platform: Option<Platform>) -> AppRequirement {
        AppRequirement {
            platform,
            os_version: None,
            device_type: None,
            physical: None,
            name: None,
            playstore: None,
            accessibility_label: None,
            booted: None,
        }
    }

    fn flow_one_app_ios() -> FlowFile {
        parse_flow(
            r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "a"
            [[flow.apps.devices]]
            os = "ios"
            "#,
        )
        .expect("parse_flow() SHALL succeed")
    }

    // ── expand_flow strategy dispatch ────────────────────────────────

    // A platform_override that excludes the app's only `os` entry SHALL
    // collapse expand_app_requirements to an empty Vec for every app —
    // expand_flow SHALL then return zero FlowRuns rather than erroring or
    // producing a bogus platform-less run.
    #[test]
    fn expand_flow_returns_empty_when_all_apps_have_no_requirements() {
        let flow = flow_one_app_ios();
        let mut groups = Vec::new();
        let runs = expand_flow(0, &flow, &[], Some(Platform::Android), None, &mut groups)
            .expect("expand_flow() SHALL succeed");
        assert!(
            runs.is_empty(),
            "no app has any matching requirement SHALL yield zero FlowRuns"
        );
        assert!(groups.is_empty());
    }

    #[test]
    fn expand_flow_full_strategy_dispatches_without_coverage_group() {
        let flow = flow_one_app_ios();
        let mut groups = Vec::new();
        let runs = expand_flow(
            0,
            &flow,
            &[],
            None,
            Some(CoverageStrategy::Full),
            &mut groups,
        )
        .expect("expand_flow() SHALL succeed");
        assert_eq!(runs.len(), 1);
        assert!(
            groups.is_empty(),
            "Full SHALL NOT register a coverage group"
        );
        assert!(runs[0].coverage_group.is_none());
    }

    #[test]
    fn expand_flow_min_strategy_dispatches_without_coverage_group() {
        let flow = flow_one_app_ios();
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let mut groups = Vec::new();
        let runs = expand_flow(
            0,
            &flow,
            &snap,
            None,
            Some(CoverageStrategy::Min),
            &mut groups,
        )
        .expect("expand_flow() SHALL succeed");
        assert_eq!(runs.len(), 1);
        assert!(groups.is_empty(), "Min SHALL NOT register a coverage group");
    }

    #[test]
    fn expand_flow_one_strategy_registers_group_capped_at_one_run() {
        let flow = flow_one_app_ios();
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let mut groups = Vec::new();
        let runs = expand_flow(
            0,
            &flow,
            &snap,
            None,
            Some(CoverageStrategy::One),
            &mut groups,
        )
        .expect("expand_flow() SHALL succeed");
        assert_eq!(
            groups.len(),
            1,
            "One SHALL register exactly one coverage group"
        );
        assert_eq!(groups[0].max_runs, Some(1));
        assert!(runs.iter().all(|r| r.coverage_group == Some(0)));
    }

    // ── expand_full ──────────────────────────────────────────────────

    // An app whose reqs list is empty (e.g. filtered out entirely by a
    // platform override upstream) SHALL be silently excluded from every
    // slot — not create a phantom empty-apps slot.
    #[test]
    fn expand_full_skips_apps_with_no_requirements() {
        let app_reqs = vec![
            ("a".to_string(), vec![req(Some(Platform::Ios))]),
            ("b".to_string(), vec![]),
        ];
        let runs = expand_full(0, &app_reqs).expect("expand_full() SHALL succeed");
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].slots.len(),
            1,
            "app with zero requirements SHALL NOT create its own slot"
        );
        assert_eq!(runs[0].slots[0].apps, vec!["a".to_string()]);
    }

    // ── expand_min ───────────────────────────────────────────────────

    #[test]
    fn expand_min_reduces_via_cover_then_delegates_to_full() {
        let reqs = vec![
            AppRequirement {
                device_type: Some(DeviceType::Phone),
                ..req(Some(Platform::Ios))
            },
            AppRequirement {
                device_type: Some(DeviceType::Tablet),
                ..req(Some(Platform::Android))
            },
        ];
        let app_reqs = vec![("a".to_string(), reqs)];
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
        let runs = expand_min(0, &app_reqs, &snap).expect("expand_min() SHALL succeed");
        assert_eq!(
            runs.len(),
            2,
            "two platform-incompatible boxes each need their own device, \
             cycling into two separate FlowRuns"
        );
    }

    // Min's per-app underspec error SHALL be wrapped with the offending
    // app's name so a multi-app failure is diagnosable.
    #[test]
    fn expand_min_errors_with_app_name_context_when_uncovered() {
        let app_reqs = vec![("checkout".to_string(), vec![req(Some(Platform::Android))])];
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let result = expand_min(0, &app_reqs, &snap);
        let err = result.expect_err("uncovered box SHALL error under Min");
        let full: String = err
            .chain()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            full.contains("checkout"),
            "error chain SHALL name the offending app: {full}"
        );
        assert!(
            full.contains("cannot be satisfied"),
            "error chain SHALL explain the min-coverage failure: {full}"
        );
    }

    // ── reduce_app_reqs_via_cover_lenient ────────────────────────────

    // Lenient cover (used by `One`) SHALL silently drop boxes no device
    // reaches, while the strict variant (`Min`/`Smart`) errors on the
    // very same input.
    #[test]
    fn reduce_app_reqs_via_cover_lenient_drops_uncovered_boxes_silently() {
        let reqs = vec![req(Some(Platform::Ios)), req(Some(Platform::Android))];
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let lenient = reduce_app_reqs_via_cover_lenient(&reqs, &snap);
        assert_eq!(
            lenient.len(),
            1,
            "the unreachable android box SHALL be silently dropped"
        );
        assert_eq!(lenient[0].platform, Some(Platform::Ios));

        let strict = reduce_app_reqs_via_cover(&reqs, &snap);
        assert!(
            strict.is_err(),
            "the identical uncovered box SHALL error under the strict variant"
        );
    }

    // ── expand_jit ───────────────────────────────────────────────────

    // Smart's "no box reachable at all" error SHALL name the "smart"
    // strategy (distinct wording from "one"/"jit") and SHALL bail before
    // registering a coverage group.
    #[test]
    fn expand_jit_smart_errors_when_pool_entirely_empty() {
        let app_reqs: Vec<(String, Vec<AppRequirement>)> = vec![("a".to_string(), vec![])];
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let mut groups = Vec::new();
        let result = expand_jit(
            0,
            &app_reqs,
            &snap,
            &mut groups,
            CoverageStrategy::Smart,
            None,
        );
        let err = result.expect_err("empty pool SHALL error under Smart");
        assert!(
            format!("{err}").contains("smart"),
            "error SHALL name the smart strategy: {err}"
        );
        assert!(
            groups.is_empty(),
            "no coverage group SHALL be registered on this early-exit error path"
        );
    }

    #[test]
    fn expand_jit_one_errors_when_pool_entirely_empty() {
        let app_reqs: Vec<(String, Vec<AppRequirement>)> = vec![("a".to_string(), vec![])];
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let mut groups = Vec::new();
        let result = expand_jit(
            0,
            &app_reqs,
            &snap,
            &mut groups,
            CoverageStrategy::One,
            Some(1),
        );
        let err = result.expect_err("empty pool SHALL error under One");
        assert!(
            format!("{err}").contains("one"),
            "error SHALL name the one strategy: {err}"
        );
    }

    // A multi-app JIT run where one app has no reachable device at all
    // (lenient cover reduces it to an empty Vec) SHALL exclude that app's
    // pool entry from `covers_boxes` and from every slot — not push a
    // bogus pool index or panic on an empty reqs slice.
    #[test]
    fn expand_jit_app_with_no_matches_excluded_from_covers_boxes() {
        let app_reqs = vec![
            ("a".to_string(), vec![req(Some(Platform::Ios))]),
            ("b".to_string(), vec![req(Some(Platform::Android))]),
        ];
        let snap = vec![mk_device(
            "iPhone",
            "u1",
            Platform::Ios,
            26,
            DeviceType::Phone,
            true,
        )];
        let mut groups = Vec::new();
        let runs = expand_jit(
            0,
            &app_reqs,
            &snap,
            &mut groups,
            CoverageStrategy::One,
            Some(1),
        )
        .expect("expand_jit() SHALL succeed — app \"a\" still has a reachable box");
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].covers_boxes.len(),
            1,
            "app \"b\"'s unreachable box SHALL be excluded from covers_boxes"
        );
        assert_eq!(
            runs[0].slots[0].apps,
            vec!["a".to_string()],
            "app \"b\" SHALL be dropped from the slot — no device satisfies it"
        );
    }

    // ── expand_app_requirements: 3-axis partial combinations ──────────

    // type + hardware both multi-valued, os single-valued: multi_count
    // reaches the >=2 threshold WITHOUT os being one of the multi axes.
    // The (arguably surprising) actual behaviour: the single-valued os
    // axis is dropped entirely rather than pinned onto every box.
    #[test]
    fn expand_app_requirements_type_and_hardware_multi_drops_single_os() {
        let app = AppConfig {
            name: "a".into(),
            bundle: Some("com.a".into()),
            install_script: None,
            install_timeout_ms: None,
            install_env: None,
            profile: None,
            devices: vec![dc(
                Some(vec!["ios:26"]),
                Some(vec!["phone", "tablet"]),
                Some(vec!["virtual", "real"]),
            )],
        };
        let snap: Vec<DeviceInfo> = Vec::new();
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min)
            .expect("expand_app_requirements() SHALL succeed");
        assert_eq!(
            reqs.len(),
            4,
            "2 type values + 2 hardware values = 4 partial boxes"
        );
        assert!(
            reqs.iter().all(|r| r.os_version.is_none()),
            "single-valued os axis SHALL be dropped, not pinned onto every box"
        );
        let type_only = reqs
            .iter()
            .filter(|r| r.device_type.is_some() && r.physical.is_none())
            .count();
        let hw_only = reqs
            .iter()
            .filter(|r| r.physical.is_some() && r.device_type.is_none())
            .count();
        assert_eq!(type_only, 2, "SHALL emit 2 partial type boxes");
        assert_eq!(hw_only, 2, "SHALL emit 2 partial hardware boxes");
    }

    // os + hardware both multi-valued, type single-valued: same threshold
    // reached via the other pairing — the single-valued type axis is the
    // one dropped this time.
    #[test]
    fn expand_app_requirements_os_and_hardware_multi_drops_single_type() {
        let app = AppConfig {
            name: "a".into(),
            bundle: Some("com.a".into()),
            install_script: None,
            install_timeout_ms: None,
            install_env: None,
            profile: None,
            devices: vec![dc(
                Some(vec!["ios:18", "ios:26"]),
                None,
                Some(vec!["virtual", "real"]),
            )],
        };
        let snap: Vec<DeviceInfo> = Vec::new();
        let reqs = expand_app_requirements(&app, &snap, None, CoverageStrategy::Min)
            .expect("expand_app_requirements() SHALL succeed");
        assert_eq!(
            reqs.len(),
            4,
            "2 os values + 2 hardware values = 4 partial boxes"
        );
        assert!(
            reqs.iter().all(|r| r.device_type.is_none()),
            "single-valued type axis SHALL be dropped, not pinned onto every box"
        );
        let os_only = reqs
            .iter()
            .filter(|r| r.os_version.is_some() && r.physical.is_none())
            .count();
        let hw_only = reqs
            .iter()
            .filter(|r| r.physical.is_some() && r.os_version.is_none())
            .count();
        assert_eq!(os_only, 2, "SHALL emit 2 partial os boxes");
        assert_eq!(hw_only, 2, "SHALL emit 2 partial hardware boxes");
    }

    // ── expand_os_pairs: "N+" minimum-version suffix ──────────────────

    // "ios:17+" SHALL parse via the generic (non-"latest") branch into an
    // `OsVersionSpec::Minimum`, not an `Exact`.
    #[test]
    fn expand_os_pairs_minimum_version_suffix_parses_as_minimum_spec() {
        let d = dc(Some(vec!["ios:17+"]), None, None);
        let pairs = expand_os_pairs(&d, &[], None, CoverageStrategy::Min)
            .expect("minimum-version syntax SHALL parse");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, Platform::Ios);
        match &pairs[0].1 {
            Some(OsVersionSpec::Minimum { platform, major }) => {
                assert_eq!(*platform, Platform::Ios);
                assert_eq!(*major, 17);
            }
            other => panic!("expected Minimum spec, got {other:?}"),
        }
    }
}
