use std::collections::HashSet;

use anyhow::{bail, Context};

use crate::version::{matches_version, parse_os_version, resolve_latest};
use crate::{DeviceInfo, DeviceState, DeviceType, OsVersionSpec, Platform, ResolvedDevice};

/// Options controlling device resolution behavior.
#[derive(Default)]
pub struct ResolveOptions {
    /// When true, if no existing device matches a constraint but a
    /// simulator/emulator could be created, include a synthetic
    /// `NeedsCreation` device in the results instead of returning an error.
    pub create_if_missing: bool,
    /// When true, if a constraint requires a physical device type that is
    /// not connected, skip it silently instead of returning an error.
    pub ignore_missing_physical: bool,
}

/// Return a preference score for a device state (lower is better).
///
/// Preference order: Booted (0) > Shutdown (1) > Connected (2) > NeedsCreation (3).
fn state_preference(state: &DeviceState) -> u8 {
    match state {
        DeviceState::Booted => 0,
        DeviceState::Shutdown => 1,
        DeviceState::Connected => 2,
        DeviceState::NeedsCreation => 3,
    }
}

/// How to expand a device constraint with multiple values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExpandMode {
    /// Find the minimum set of devices covering all requirements (default).
    #[default]
    MinCoverage,
    /// Produce the full cartesian product of OS versions x device types.
    Full,
}

/// A device constraint from a flow file.
///
/// Each constraint describes one or more requirements that must be satisfied by
/// real devices. When `expand` is `MinCoverage` (the default), the resolver
/// picks the fewest devices that cover every listed OS version and device type.
/// When `expand` is `Full`, every combination of OS version and device type is
/// required.
#[derive(Debug, Clone)]
pub struct DeviceConstraint {
    /// If set, select the device with this exact name.
    pub name: Option<String>,
    /// OS version specs such as `"ios:18"` or `"android:34"`.
    pub os_versions: Vec<String>,
    /// Required device types (Phone, Tablet).
    pub device_types: Vec<DeviceType>,
    /// Expansion mode.
    pub expand: ExpandMode,
}

/// A single requirement that needs to be covered by at least one device.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Requirement {
    /// Must have a device whose OS version matches this spec string.
    OsVersion(String),
    /// Must have a device of this type.
    DeviceType(DeviceType),
}

/// Check whether a device satisfies a given requirement.
fn device_satisfies(device: &DeviceInfo, req: &Requirement) -> bool {
    match req {
        Requirement::OsVersion(spec_str) => {
            let spec = match parse_os_version(spec_str) {
                Ok(s) => s,
                Err(_) => return false,
            };
            // The spec's platform must match the device's platform.
            let spec_platform = match &spec {
                crate::OsVersionSpec::Exact { platform, .. }
                | crate::OsVersionSpec::Minimum { platform, .. }
                | crate::OsVersionSpec::Latest { platform, .. } => *platform,
            };
            if spec_platform != device.platform {
                return false;
            }
            matches_version(&spec, device.os_major)
        }
        Requirement::DeviceType(dt) => device.device_type == *dt,
    }
}

/// Compute the set of requirements a device covers.
fn covered_requirements(device: &DeviceInfo, requirements: &[Requirement]) -> HashSet<usize> {
    requirements
        .iter()
        .enumerate()
        .filter(|(_, req)| device_satisfies(device, req))
        .map(|(i, _)| i)
        .collect()
}

/// Resolve device constraints against available devices.
///
/// For each constraint, determines the set of real devices from `available`
/// that must be used. Named constraints resolve to a single device by name.
/// Multi-value constraints use either greedy minimum set cover or full
/// cartesian product expansion depending on the `expand` mode.
///
/// Previously resolved devices (from earlier constraints) receive credit --
/// if an already-selected device covers requirements from a later constraint,
/// it is not selected again, reducing the total device count.
pub fn resolve_devices(
    constraints: &[DeviceConstraint],
    available: &[DeviceInfo],
    options: &ResolveOptions,
) -> anyhow::Result<Vec<ResolvedDevice>> {
    let mut result: Vec<ResolvedDevice> = Vec::new();

    for constraint in constraints {
        let expanded = expand_latest_in_constraint(constraint, available);
        let constraint = &expanded;
        if let Some(ref name) = constraint.name {
            resolve_named(name, available, &mut result)?;
        } else {
            match constraint.expand {
                ExpandMode::Full => {
                    resolve_full(constraint, available, &mut result, options)?;
                }
                ExpandMode::MinCoverage => {
                    resolve_min_coverage(constraint, available, &mut result, options)?;
                }
            }
        }
    }

    Ok(result)
}

/// Rewrite any `:latest` / `:latest:N` entries in `os_versions` to the
/// concrete top-N `platform:major` strings picked from `available`.
/// Non-Latest entries pass through unchanged. If nothing matches the
/// platform, the original string is preserved so downstream error paths
/// still report the user-facing constraint.
///
/// Why: `matches_version(Latest, _)` returns `true` for any device — if
/// left in place, greedy min-coverage tie-breaks `:latest` by state
/// preference (booted > shutdown) and picks iOS 18 booted over iOS 26
/// shutdown. Expanding here pins the version first so state preference
/// only tie-breaks *within* the chosen version.
fn expand_latest_in_constraint(
    constraint: &DeviceConstraint,
    available: &[DeviceInfo],
) -> DeviceConstraint {
    let mut expanded: Vec<String> = Vec::with_capacity(constraint.os_versions.len());
    for s in &constraint.os_versions {
        match parse_os_version(s) {
            Ok(OsVersionSpec::Latest { platform, count }) => {
                let majors: Vec<u32> = available
                    .iter()
                    .filter(|d| d.platform == platform)
                    .map(|d| d.os_major)
                    .collect();
                let tops = resolve_latest(platform, count, &majors);
                if tops.is_empty() {
                    expanded.push(s.clone());
                } else {
                    for m in tops {
                        expanded.push(format!("{platform}:{m}"));
                    }
                }
            }
            _ => expanded.push(s.clone()),
        }
    }
    DeviceConstraint {
        name: constraint.name.clone(),
        os_versions: expanded,
        device_types: constraint.device_types.clone(),
        expand: constraint.expand,
    }
}

/// Resolve a constraint that names a specific device.
fn resolve_named(
    name: &str,
    available: &[DeviceInfo],
    result: &mut Vec<ResolvedDevice>,
) -> anyhow::Result<()> {
    // Don't add duplicates.
    if result.iter().any(|r| r.device.name == name) {
        return Ok(());
    }

    let device = available
        .iter()
        .find(|d| d.name == name)
        .with_context(|| format!("no available device named '{name}'"))?;

    result.push(ResolvedDevice {
        device: device.clone(),
        apps: Vec::new(),
        created: false,
    });

    Ok(())
}

/// Resolve a constraint using full cartesian product expansion.
///
/// Every combination of OS version x device type is required. If either
/// dimension is empty, the other dimension alone is iterated.
fn resolve_full(
    constraint: &DeviceConstraint,
    available: &[DeviceInfo],
    result: &mut Vec<ResolvedDevice>,
    options: &ResolveOptions,
) -> anyhow::Result<()> {
    let os_versions = &constraint.os_versions;
    let device_types = &constraint.device_types;

    if os_versions.is_empty() && device_types.is_empty() {
        return Ok(());
    }

    // Build the cartesian product of (os, type) pairs.
    let combos: Vec<(Option<&String>, Option<&DeviceType>)> = if os_versions.is_empty() {
        device_types.iter().map(|dt| (None, Some(dt))).collect()
    } else if device_types.is_empty() {
        os_versions.iter().map(|os| (Some(os), None)).collect()
    } else {
        let mut c = Vec::new();
        for os in os_versions {
            for dt in device_types {
                c.push((Some(os), Some(dt)));
            }
        }
        c
    };

    for (os_opt, dt_opt) in &combos {
        // Check if an already-selected device covers this combo.
        let already_covered = result.iter().any(|r| {
            let os_ok = match os_opt {
                Some(os_str) => {
                    device_satisfies(&r.device, &Requirement::OsVersion((*os_str).clone()))
                }
                None => true,
            };
            let dt_ok = match dt_opt {
                Some(dt) => r.device.device_type == **dt,
                None => true,
            };
            os_ok && dt_ok
        });

        if already_covered {
            continue;
        }

        // Find matching available devices that aren't already selected, sorted by preference.
        let selected_udids: HashSet<&str> =
            result.iter().map(|r| r.device.udid.as_str()).collect();

        let mut candidates: Vec<&DeviceInfo> = available
            .iter()
            .filter(|d| !selected_udids.contains(d.udid.as_str()))
            .filter(|d| {
                let os_ok = match os_opt {
                    Some(os_str) => {
                        device_satisfies(d, &Requirement::OsVersion((*os_str).clone()))
                    }
                    None => true,
                };
                let dt_ok = match dt_opt {
                    Some(dt) => d.device_type == **dt,
                    None => true,
                };
                os_ok && dt_ok
            })
            .collect();

        // Sort by state preference: Booted > Shutdown > Connected > NeedsCreation.
        candidates.sort_by_key(|d| state_preference(&d.state));

        if let Some(device) = candidates.first() {
            result.push(ResolvedDevice {
                device: (*device).clone(),
                apps: Vec::new(),
                created: false,
            });
        } else if requires_physical(os_opt, dt_opt, available) && options.ignore_missing_physical {
            // Physical device required but not connected -- skip silently.
            continue;
        } else if let (true, Some(synthetic)) =
            (options.create_if_missing, make_synthetic_device(os_opt, dt_opt))
        {
            result.push(ResolvedDevice {
                device: synthetic,
                apps: Vec::new(),
                created: true,
            });
        } else {
            bail!(
                "no available device matching os={:?}, type={:?}",
                os_opt,
                dt_opt
            );
        }
    }

    Ok(())
}

/// Resolve a constraint using greedy minimum set cover.
///
/// 1. List all requirements (each OS version, each device type).
/// 2. Give credit to devices already in `result`.
/// 3. Greedily pick the available device covering the most uncovered requirements.
/// 4. Repeat until all requirements are covered.
fn resolve_min_coverage(
    constraint: &DeviceConstraint,
    available: &[DeviceInfo],
    result: &mut Vec<ResolvedDevice>,
    options: &ResolveOptions,
) -> anyhow::Result<()> {
    // Build the full set of requirements.
    let mut requirements: Vec<Requirement> = Vec::new();
    for os in &constraint.os_versions {
        requirements.push(Requirement::OsVersion(os.clone()));
    }
    for dt in &constraint.device_types {
        requirements.push(Requirement::DeviceType(*dt));
    }

    if requirements.is_empty() {
        return Ok(());
    }

    // Track which requirements are already covered by previously resolved devices.
    let mut covered: HashSet<usize> = HashSet::new();
    for resolved in result.iter() {
        let c = covered_requirements(&resolved.device, &requirements);
        covered = covered.union(&c).copied().collect();
    }

    // Greedy set cover loop.
    let selected_udids: HashSet<String> =
        result.iter().map(|r| r.device.udid.clone()).collect();

    // Build a pool of candidates (available devices not already selected),
    // sorted by state preference so ties favor better states.
    let mut candidates: Vec<&DeviceInfo> = available
        .iter()
        .filter(|d| !selected_udids.contains(&d.udid))
        .collect();
    candidates.sort_by_key(|d| state_preference(&d.state));

    while covered.len() < requirements.len() {
        // For each candidate, compute how many *uncovered* requirements it covers.
        // When there is a tie in coverage count, prefer the device with the
        // better state (lower state_preference value). Because candidates are
        // already sorted by state preference, max_by_key with stable ordering
        // on (new_covers.len()) would pick the *last* equal element.  To
        // prefer the *first* (best state), we use a tuple that also includes
        // (new_count, inverse_state_preference) so the best state wins ties.
        let best = candidates
            .iter()
            .filter(|d| !result.iter().any(|r| r.device.udid == d.udid))
            .map(|d| {
                let device_covers = covered_requirements(d, &requirements);
                let new_covers: HashSet<usize> =
                    device_covers.difference(&covered).copied().collect();
                let new_count = new_covers.len();
                // Higher inverse_pref => better state => preferred in tie.
                let inverse_pref = 255 - state_preference(&d.state);
                (d, new_covers, new_count, inverse_pref)
            })
            .filter(|(_, _, count, _)| *count > 0)
            .max_by_key(|(_, _, count, inv_pref)| (*count, *inv_pref));

        match best {
            Some((device, new_covers, _, _)) => {
                covered = covered.union(&new_covers).copied().collect();
                result.push(ResolvedDevice {
                    device: (*device).clone(),
                    apps: Vec::new(),
                    created: false,
                });
            }
            None => {
                // Collect uncovered requirements for the error message.
                let uncovered_reqs: Vec<&Requirement> = requirements
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !covered.contains(i))
                    .map(|(_, req)| req)
                    .collect();

                // Check if all uncovered requirements involve physical devices.
                if options.ignore_missing_physical
                    && uncovered_reqs_are_physical_only(&uncovered_reqs, available)
                {
                    break;
                }

                // Try create_if_missing: generate synthetic devices for remaining requirements.
                if options.create_if_missing {
                    let mut created_any = false;
                    for (i, req) in requirements.iter().enumerate() {
                        if covered.contains(&i) {
                            continue;
                        }
                        if let Some(synthetic) = make_synthetic_for_requirement(req) {
                            // Check if a previously created synthetic already covers this.
                            let syn_covers = covered_requirements(&synthetic, &requirements);
                            let new: HashSet<usize> =
                                syn_covers.difference(&covered).copied().collect();
                            if !new.is_empty() {
                                covered = covered.union(&new).copied().collect();
                                result.push(ResolvedDevice {
                                    device: synthetic,
                                    apps: Vec::new(),
                                    created: true,
                                });
                                created_any = true;
                            }
                        }
                    }
                    if created_any {
                        continue;
                    }
                }

                let uncovered: Vec<String> = requirements
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !covered.contains(i))
                    .map(|(_, req)| format!("{req:?}"))
                    .collect();
                bail!(
                    "no available devices can satisfy remaining requirements: {}",
                    uncovered.join(", ")
                );
            }
        }
    }

    Ok(())
}

/// Check whether uncovered requirements involve only physical devices.
fn uncovered_reqs_are_physical_only(reqs: &[&Requirement], available: &[DeviceInfo]) -> bool {
    // If available has any physical devices, the missing ones are physical.
    // We consider a requirement "physical-only" when it's a DeviceType requirement
    // and the only devices of that type in `available` are physical, OR there are
    // no matching devices at all (meaning the constraint targets a physical device).
    // For simplicity: if no available device (including virtual) satisfies any of the
    // uncovered reqs, and the constraint could reference a physical device, treat as physical.
    for req in reqs {
        let any_virtual_match = available.iter().any(|d| !d.physical && device_satisfies(d, req));
        if any_virtual_match {
            return false;
        }
    }
    true
}

/// Check whether a particular combo targets a physical device.
fn requires_physical(
    _os_opt: &Option<&String>,
    _dt_opt: &Option<&DeviceType>,
    available: &[DeviceInfo],
) -> bool {
    // If all devices in the pool are physical, we treat it as physical-required.
    available.iter().all(|d| d.physical)
}

/// Build a synthetic `NeedsCreation` device from OS + type combo (for `resolve_full`).
fn make_synthetic_device(
    os_opt: &Option<&String>,
    dt_opt: &Option<&DeviceType>,
) -> Option<DeviceInfo> {
    let (platform, os_major) = match os_opt {
        Some(os_str) => {
            let spec = parse_os_version(os_str).ok()?;
            let (p, m) = match spec {
                crate::OsVersionSpec::Exact { platform, major } => (platform, major),
                crate::OsVersionSpec::Minimum { platform, major } => (platform, major),
                crate::OsVersionSpec::Latest { .. } => {
                    unreachable!(
                        "Latest is expanded to a concrete major before make_synthetic_* runs"
                    )
                }
            };
            (p, m)
        }
        None => (Platform::Ios, 0),
    };

    let device_type = match dt_opt {
        Some(dt) => **dt,
        None => DeviceType::Phone,
    };

    Some(DeviceInfo {
        name: format!("synthetic-{}-{}-{}", platform, device_type, os_major),
        udid: format!("synthetic-{}-{}-{}", platform, device_type, os_major),
        platform,
        device_type,
        os_major,
        os_version: format!("{os_major}.0"),
        state: DeviceState::NeedsCreation,
        physical: false,
        playstore: false,
        screen_width: None,
        screen_height: None,
        screen_scale: None,
        last_booted: None,
        runtime_id: None,
        device_type_id: None,
    })
}

/// Build a synthetic `NeedsCreation` device for a single requirement
/// (used in `resolve_min_coverage`).
fn make_synthetic_for_requirement(req: &Requirement) -> Option<DeviceInfo> {
    match req {
        Requirement::OsVersion(os_str) => {
            let spec = parse_os_version(os_str).ok()?;
            let (platform, major) = match spec {
                crate::OsVersionSpec::Exact { platform, major } => (platform, major),
                crate::OsVersionSpec::Minimum { platform, major } => (platform, major),
                crate::OsVersionSpec::Latest { .. } => {
                    unreachable!(
                        "Latest is expanded to a concrete major before make_synthetic_* runs"
                    )
                }
            };
            Some(DeviceInfo {
                name: format!("synthetic-{}-{}", platform, major),
                udid: format!("synthetic-{}-{}", platform, major),
                platform,
                device_type: DeviceType::Phone,
                os_major: major,
                os_version: format!("{major}.0"),
                state: DeviceState::NeedsCreation,
                physical: false,
                playstore: false,
                screen_width: None,
                screen_height: None,
                screen_scale: None,
                last_booted: None,
                runtime_id: None,
                device_type_id: None,
            })
        }
        Requirement::DeviceType(dt) => Some(DeviceInfo {
            name: format!("synthetic-{dt}"),
            udid: format!("synthetic-{dt}"),
            platform: Platform::Ios,
            device_type: *dt,
            os_major: 0,
            os_version: "0.0".to_string(),
            state: DeviceState::NeedsCreation,
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceState, Platform};

    /// Helper: build a DeviceInfo with common defaults.
    fn make_device(
        name: &str,
        udid: &str,
        platform: Platform,
        device_type: DeviceType,
        os_major: u32,
    ) -> DeviceInfo {
        DeviceInfo {
            name: name.to_string(),
            udid: udid.to_string(),
            platform,
            device_type,
            os_major,
            os_version: format!("{os_major}.0"),
            state: DeviceState::Shutdown,
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

    // 1. Single explicit device by name
    #[test]
    fn single_explicit_device_by_name() {
        let available = vec![
            make_device("iPhone 15 Pro", "uid-1", Platform::Ios, DeviceType::Phone, 18),
            make_device("iPad Air", "uid-2", Platform::Ios, DeviceType::Tablet, 18),
        ];
        let constraints = vec![DeviceConstraint {
            name: Some("iPhone 15 Pro".to_string()),
            os_versions: vec![],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let opts = ResolveOptions::default();
        let result = resolve_devices(&constraints, &available, &opts).expect("should resolve");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].device.name, "iPhone 15 Pro");
        assert_eq!(result[0].device.udid, "uid-1");
    }

    // 2. Single OS constraint matches device
    #[test]
    fn single_os_constraint_matches_device() {
        let available = vec![
            make_device("iPhone 15", "uid-1", Platform::Ios, DeviceType::Phone, 17),
            make_device("iPhone 16", "uid-2", Platform::Ios, DeviceType::Phone, 18),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:18".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].device.os_major, 18);
    }

    // 3. Single type constraint matches device
    #[test]
    fn single_type_constraint_matches_device() {
        let available = vec![
            make_device("iPhone 15", "uid-1", Platform::Ios, DeviceType::Phone, 18),
            make_device("iPad Air", "uid-2", Platform::Ios, DeviceType::Tablet, 18),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec![],
            device_types: vec![DeviceType::Tablet],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].device.device_type, DeviceType::Tablet);
    }

    // 4. Multiple OS versions -- minimum coverage picks 2 devices
    #[test]
    fn multiple_os_versions_min_coverage_picks_two() {
        let available = vec![
            make_device("iPhone 15", "uid-1", Platform::Ios, DeviceType::Phone, 17),
            make_device("iPhone 16", "uid-2", Platform::Ios, DeviceType::Phone, 18),
            make_device("iPhone 14", "uid-3", Platform::Ios, DeviceType::Phone, 16),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:17".to_string(), "ios:18".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        assert_eq!(result.len(), 2);
        let majors: HashSet<u32> = result.iter().map(|r| r.device.os_major).collect();
        assert!(majors.contains(&17));
        assert!(majors.contains(&18));
    }

    // 5. Multiple types -- minimum coverage picks 2 devices
    #[test]
    fn multiple_types_min_coverage_picks_two() {
        let available = vec![
            make_device("iPhone 16", "uid-1", Platform::Ios, DeviceType::Phone, 18),
            make_device("iPad Air", "uid-2", Platform::Ios, DeviceType::Tablet, 18),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec![],
            device_types: vec![DeviceType::Phone, DeviceType::Tablet],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        assert_eq!(result.len(), 2);
        let types: HashSet<DeviceType> = result.iter().map(|r| r.device.device_type).collect();
        assert!(types.contains(&DeviceType::Phone));
        assert!(types.contains(&DeviceType::Tablet));
    }

    // 6. Combined OS + type -- minimum 2 covers both dimensions
    #[test]
    fn combined_os_and_type_min_coverage() {
        // Need: ios:17, ios:18, phone, tablet
        // iPhone 16 (ios:18, phone) covers ios:18 + phone
        // iPad 17 (ios:17, tablet) covers ios:17 + tablet
        // Minimum: 2 devices
        let available = vec![
            make_device("iPhone 16", "uid-1", Platform::Ios, DeviceType::Phone, 18),
            make_device("iPad 17", "uid-2", Platform::Ios, DeviceType::Tablet, 17),
            make_device("iPhone 15", "uid-3", Platform::Ios, DeviceType::Phone, 17),
            make_device("iPad 18", "uid-4", Platform::Ios, DeviceType::Tablet, 18),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:17".to_string(), "ios:18".to_string()],
            device_types: vec![DeviceType::Phone, DeviceType::Tablet],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        assert_eq!(result.len(), 2);

        // Verify all requirements are satisfied.
        let has_ios17 = result.iter().any(|r| r.device.os_major == 17);
        let has_ios18 = result.iter().any(|r| r.device.os_major == 18);
        let has_phone = result
            .iter()
            .any(|r| r.device.device_type == DeviceType::Phone);
        let has_tablet = result
            .iter()
            .any(|r| r.device.device_type == DeviceType::Tablet);
        assert!(has_ios17);
        assert!(has_ios18);
        assert!(has_phone);
        assert!(has_tablet);
    }

    // 7. expand=full produces cartesian product
    #[test]
    fn expand_full_produces_cartesian_product() {
        let available = vec![
            make_device("iPhone 16", "uid-1", Platform::Ios, DeviceType::Phone, 18),
            make_device("iPad 18", "uid-2", Platform::Ios, DeviceType::Tablet, 18),
            make_device("iPhone 15", "uid-3", Platform::Ios, DeviceType::Phone, 17),
            make_device("iPad 17", "uid-4", Platform::Ios, DeviceType::Tablet, 17),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:17".to_string(), "ios:18".to_string()],
            device_types: vec![DeviceType::Phone, DeviceType::Tablet],
            expand: ExpandMode::Full,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        // 2 OS versions x 2 types = 4 devices
        assert_eq!(result.len(), 4);
    }

    // 8. Explicit device gets credit toward coverage
    #[test]
    fn explicit_device_gets_credit_toward_coverage() {
        // iPhone 15 Pro is ios:18 + phone.
        // Second constraint needs ios:17, ios:18, phone.
        // iPhone 15 Pro already covers ios:18 and phone, so only need one more for ios:17.
        let available = vec![
            make_device("iPhone 15 Pro", "uid-1", Platform::Ios, DeviceType::Phone, 18),
            make_device("iPhone 14", "uid-2", Platform::Ios, DeviceType::Phone, 17),
            make_device("iPhone 16", "uid-3", Platform::Ios, DeviceType::Phone, 18),
        ];
        let constraints = vec![
            DeviceConstraint {
                name: Some("iPhone 15 Pro".to_string()),
                os_versions: vec![],
                device_types: vec![],
                expand: ExpandMode::MinCoverage,
            },
            DeviceConstraint {
                name: None,
                os_versions: vec!["ios:17".to_string(), "ios:18".to_string()],
                device_types: vec![DeviceType::Phone],
                expand: ExpandMode::MinCoverage,
            },
        ];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        // iPhone 15 Pro covers ios:18 + phone. Only need one more for ios:17.
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|r| r.device.name == "iPhone 15 Pro"));
        assert!(result.iter().any(|r| r.device.os_major == 17));
    }

    // 9. No matching device returns error
    #[test]
    fn no_matching_device_returns_error() {
        let available = vec![make_device(
            "iPhone 16",
            "uid-1",
            Platform::Ios,
            DeviceType::Phone,
            18,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["android:34".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default());
        assert!(result.is_err());
    }

    // 10. Empty constraints returns empty result
    #[test]
    fn empty_constraints_returns_empty_result() {
        let available = vec![make_device(
            "iPhone 16",
            "uid-1",
            Platform::Ios,
            DeviceType::Phone,
            18,
        )];
        let constraints: Vec<DeviceConstraint> = vec![];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        assert!(result.is_empty());
    }

    // 11. Duplicate devices are not selected twice
    #[test]
    fn duplicate_devices_not_selected_twice() {
        let available = vec![make_device(
            "iPhone 15 Pro",
            "uid-1",
            Platform::Ios,
            DeviceType::Phone,
            18,
        )];
        // Two constraints that both name the same device.
        let constraints = vec![
            DeviceConstraint {
                name: Some("iPhone 15 Pro".to_string()),
                os_versions: vec![],
                device_types: vec![],
                expand: ExpandMode::MinCoverage,
            },
            DeviceConstraint {
                name: Some("iPhone 15 Pro".to_string()),
                os_versions: vec![],
                device_types: vec![],
                expand: ExpandMode::MinCoverage,
            },
        ];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        assert_eq!(result.len(), 1);
    }

    // 12. Greedy picks device covering most requirements first
    #[test]
    fn greedy_picks_device_covering_most_requirements() {
        // Requirements: ios:17, ios:18, phone
        // Device A (ios:18, phone) covers ios:18 + phone = 2 requirements
        // Device B (ios:17, phone) covers ios:17 + phone = 2 requirements
        // Device C (ios:17, phone) covers ios:17 + phone = 2 requirements (duplicate)
        //
        // Greedy picks A (2 covers), then after A is selected, B covers ios:17
        // (the only remaining). Total: 2 devices (not 3).
        // Without greedy (naive), you might pick B first, then still need A -> also 2.
        // Key point: greedy never picks more than necessary.
        let available = vec![
            make_device("iPhone 16", "uid-1", Platform::Ios, DeviceType::Phone, 18),
            make_device("iPhone 15", "uid-2", Platform::Ios, DeviceType::Phone, 17),
            make_device("iPhone 15b", "uid-3", Platform::Ios, DeviceType::Phone, 17),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:17".to_string(), "ios:18".to_string()],
            device_types: vec![DeviceType::Phone],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        // Greedy should find a 2-device solution (not 3).
        assert_eq!(result.len(), 2);
        // Both OS versions covered.
        assert!(result.iter().any(|r| r.device.os_major == 17));
        assert!(result.iter().any(|r| r.device.os_major == 18));
    }

    // 13. OS version matching uses matches_version logic
    #[test]
    fn os_version_matching_uses_version_logic() {
        // "ios:17+" should match both 17 and 18.
        let available = vec![
            make_device("iPhone 15", "uid-1", Platform::Ios, DeviceType::Phone, 17),
            make_device("iPhone 16", "uid-2", Platform::Ios, DeviceType::Phone, 18),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:17+".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        // Only one requirement (ios:17+), so one device suffices.
        assert_eq!(result.len(), 1);
        // The picked device should have os_major >= 17.
        assert!(result[0].device.os_major >= 17);
    }

    // 14. Multiple constraints produce combined results
    #[test]
    fn multiple_constraints_produce_combined_results() {
        let available = vec![
            make_device("iPhone 16", "uid-1", Platform::Ios, DeviceType::Phone, 18),
            make_device("Pixel 8", "uid-2", Platform::Android, DeviceType::Phone, 34),
        ];
        let constraints = vec![
            DeviceConstraint {
                name: None,
                os_versions: vec!["ios:18".to_string()],
                device_types: vec![],
                expand: ExpandMode::MinCoverage,
            },
            DeviceConstraint {
                name: None,
                os_versions: vec!["android:34".to_string()],
                device_types: vec![],
                expand: ExpandMode::MinCoverage,
            },
        ];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        assert_eq!(result.len(), 2);
        let platforms: HashSet<Platform> = result.iter().map(|r| r.device.platform).collect();
        assert!(platforms.contains(&Platform::Ios));
        assert!(platforms.contains(&Platform::Android));
    }

    // 15. Named device not found returns error
    #[test]
    fn named_device_not_found_returns_error() {
        let available = vec![make_device(
            "iPhone 16",
            "uid-1",
            Platform::Ios,
            DeviceType::Phone,
            18,
        )];
        let constraints = vec![DeviceConstraint {
            name: Some("Galaxy S24".to_string()),
            os_versions: vec![],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default());
        assert!(result.is_err());
        let err_msg = result.expect_err("should fail").to_string();
        assert!(
            err_msg.contains("Galaxy S24"),
            "error should mention the device name: {err_msg}"
        );
    }

    // 16. expand=full with only OS versions (no types)
    #[test]
    fn expand_full_os_only() {
        let available = vec![
            make_device("iPhone 15", "uid-1", Platform::Ios, DeviceType::Phone, 17),
            make_device("iPhone 16", "uid-2", Platform::Ios, DeviceType::Phone, 18),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:17".to_string(), "ios:18".to_string()],
            device_types: vec![],
            expand: ExpandMode::Full,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        assert_eq!(result.len(), 2);
    }

    // 17. expand=full with only types (no OS versions)
    #[test]
    fn expand_full_types_only() {
        let available = vec![
            make_device("iPhone 16", "uid-1", Platform::Ios, DeviceType::Phone, 18),
            make_device("iPad Air", "uid-2", Platform::Ios, DeviceType::Tablet, 18),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec![],
            device_types: vec![DeviceType::Phone, DeviceType::Tablet],
            expand: ExpandMode::Full,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        assert_eq!(result.len(), 2);
    }

    // 18. Constraint with empty os_versions and empty device_types produces nothing
    #[test]
    fn empty_os_and_types_produces_nothing() {
        let available = vec![make_device(
            "iPhone 16",
            "uid-1",
            Platform::Ios,
            DeviceType::Phone,
            18,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec![],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default()).expect("should resolve");
        assert!(result.is_empty());
    }

    // ─── Preference ordering tests ───────────────────────────────────

    /// Helper: build a DeviceInfo with a specific state.
    fn make_device_with_state(
        name: &str,
        udid: &str,
        platform: Platform,
        device_type: DeviceType,
        os_major: u32,
        state: DeviceState,
    ) -> DeviceInfo {
        DeviceInfo {
            name: name.to_string(),
            udid: udid.to_string(),
            platform,
            device_type,
            os_major,
            os_version: format!("{os_major}.0"),
            state,
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

    /// Helper: build a physical DeviceInfo.
    fn make_physical_device(
        name: &str,
        udid: &str,
        platform: Platform,
        device_type: DeviceType,
        os_major: u32,
        state: DeviceState,
    ) -> DeviceInfo {
        let mut d = make_device_with_state(name, udid, platform, device_type, os_major, state);
        d.physical = true;
        d
    }

    // 19. Prefer running (booted) device over shutdown device
    #[test]
    fn prefer_running_over_shutdown() {
        let available = vec![
            make_device_with_state(
                "iPhone Shutdown",
                "uid-1",
                Platform::Ios,
                DeviceType::Phone,
                18,
                DeviceState::Shutdown,
            ),
            make_device_with_state(
                "iPhone Booted",
                "uid-2",
                Platform::Ios,
                DeviceType::Phone,
                18,
                DeviceState::Booted,
            ),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:18".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default())
            .expect("should resolve");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].device.name, "iPhone Booted");
        assert_eq!(result[0].device.state, DeviceState::Booted);
    }

    // 20. Prefer shutdown device over needs-creation
    #[test]
    fn prefer_shutdown_over_needs_creation() {
        let available = vec![
            make_device_with_state(
                "iPhone NeedsCreation",
                "uid-1",
                Platform::Ios,
                DeviceType::Phone,
                18,
                DeviceState::NeedsCreation,
            ),
            make_device_with_state(
                "iPhone Shutdown",
                "uid-2",
                Platform::Ios,
                DeviceType::Phone,
                18,
                DeviceState::Shutdown,
            ),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:18".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default())
            .expect("should resolve");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].device.name, "iPhone Shutdown");
        assert_eq!(result[0].device.state, DeviceState::Shutdown);
    }

    // 21. create_if_missing=true adds synthetic device when no match exists
    #[test]
    fn create_if_missing_true_adds_synthetic() {
        let available = vec![make_device(
            "iPhone 16",
            "uid-1",
            Platform::Ios,
            DeviceType::Phone,
            18,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["android:34".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];
        let opts = ResolveOptions {
            create_if_missing: true,
            ignore_missing_physical: false,
        };

        let result = resolve_devices(&constraints, &available, &opts).expect("should resolve");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].device.state, DeviceState::NeedsCreation);
        assert_eq!(result[0].device.platform, Platform::Android);
        assert!(result[0].created);
    }

    // 22. create_if_missing=false errors when no match exists
    #[test]
    fn create_if_missing_false_errors_on_no_match() {
        let available = vec![make_device(
            "iPhone 16",
            "uid-1",
            Platform::Ios,
            DeviceType::Phone,
            18,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["android:34".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];
        let opts = ResolveOptions {
            create_if_missing: false,
            ignore_missing_physical: false,
        };

        let result = resolve_devices(&constraints, &available, &opts);
        assert!(result.is_err());
    }

    // 23. ignore_missing_physical=true skips missing physical devices
    #[test]
    fn ignore_missing_physical_true_skips() {
        // Only physical devices in available, but none match android:34.
        // With ignore_missing_physical=true, should skip silently.
        let available = vec![make_physical_device(
            "Pixel 8",
            "uid-1",
            Platform::Android,
            DeviceType::Phone,
            33,
            DeviceState::Connected,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["android:34".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];
        let opts = ResolveOptions {
            create_if_missing: false,
            ignore_missing_physical: true,
        };

        let result = resolve_devices(&constraints, &available, &opts).expect("should resolve");
        // No device matched, but skipped silently instead of erroring.
        assert!(result.is_empty());
    }

    // 24. ignore_missing_physical=false errors on missing physical device
    #[test]
    fn ignore_missing_physical_false_errors() {
        let available = vec![make_physical_device(
            "Pixel 8",
            "uid-1",
            Platform::Android,
            DeviceType::Phone,
            33,
            DeviceState::Connected,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["android:34".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];
        let opts = ResolveOptions {
            create_if_missing: false,
            ignore_missing_physical: false,
        };

        let result = resolve_devices(&constraints, &available, &opts);
        assert!(result.is_err());
    }

    // 25. Mixed states: running + shutdown + needs-creation, picks optimally
    #[test]
    fn mixed_states_picks_optimal() {
        // Two ios:18 phones available: one booted, one shutdown, one needs-creation.
        // Also need ios:17 -- only shutdown available.
        let available = vec![
            make_device_with_state(
                "iPhone NeedsCreation",
                "uid-1",
                Platform::Ios,
                DeviceType::Phone,
                18,
                DeviceState::NeedsCreation,
            ),
            make_device_with_state(
                "iPhone Shutdown 18",
                "uid-2",
                Platform::Ios,
                DeviceType::Phone,
                18,
                DeviceState::Shutdown,
            ),
            make_device_with_state(
                "iPhone Booted 18",
                "uid-3",
                Platform::Ios,
                DeviceType::Phone,
                18,
                DeviceState::Booted,
            ),
            make_device_with_state(
                "iPhone Shutdown 17",
                "uid-4",
                Platform::Ios,
                DeviceType::Phone,
                17,
                DeviceState::Shutdown,
            ),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:17".to_string(), "ios:18".to_string()],
            device_types: vec![DeviceType::Phone],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default())
            .expect("should resolve");
        assert_eq!(result.len(), 2);

        // The ios:18 device should be the booted one.
        let ios18 = result
            .iter()
            .find(|r| r.device.os_major == 18)
            .expect("should have ios:18 device");
        assert_eq!(ios18.device.state, DeviceState::Booted);
        assert_eq!(ios18.device.name, "iPhone Booted 18");

        // ios:17 should be the shutdown one (only option).
        let ios17 = result
            .iter()
            .find(|r| r.device.os_major == 17)
            .expect("should have ios:17 device");
        assert_eq!(ios17.device.state, DeviceState::Shutdown);
    }

    // 26. ResolveOptions defaults to false for both fields
    #[test]
    fn resolve_options_default_is_false() {
        let opts = ResolveOptions::default();
        assert!(!opts.create_if_missing);
        assert!(!opts.ignore_missing_physical);
    }

    // 27. Preference ordering works in expand=Full mode too
    #[test]
    fn preference_ordering_in_full_mode() {
        let available = vec![
            make_device_with_state(
                "iPhone Shutdown",
                "uid-1",
                Platform::Ios,
                DeviceType::Phone,
                18,
                DeviceState::Shutdown,
            ),
            make_device_with_state(
                "iPhone Booted",
                "uid-2",
                Platform::Ios,
                DeviceType::Phone,
                18,
                DeviceState::Booted,
            ),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:18".to_string()],
            device_types: vec![DeviceType::Phone],
            expand: ExpandMode::Full,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default())
            .expect("should resolve");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].device.state, DeviceState::Booted);
        assert_eq!(result[0].device.name, "iPhone Booted");
    }

    // 28. create_if_missing with expand=Full adds synthetic device
    #[test]
    fn create_if_missing_full_mode() {
        let available: Vec<DeviceInfo> = vec![];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:18".to_string()],
            device_types: vec![],
            expand: ExpandMode::Full,
        }];
        let opts = ResolveOptions {
            create_if_missing: true,
            ignore_missing_physical: false,
        };

        let result = resolve_devices(&constraints, &available, &opts).expect("should resolve");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].device.state, DeviceState::NeedsCreation);
        assert!(result[0].created);
    }

    // 29. `:latest` picks highest version, not most-booted version.
    //
    // Regression: `matches_version(Latest, _) == true` made every iOS
    // device a candidate, and greedy tie-break then picked booted iOS 18
    // over shutdown iOS 26. Expected semantics: "latest = highest major",
    // state preference tie-breaks only within the chosen major.
    #[test]
    fn latest_picks_highest_version_over_booted_older() {
        let available = vec![
            make_device_with_state(
                "iPhone 18 Booted",
                "uid-1",
                Platform::Ios,
                DeviceType::Phone,
                18,
                DeviceState::Booted,
            ),
            make_device_with_state(
                "iPhone 26 Shutdown",
                "uid-2",
                Platform::Ios,
                DeviceType::Phone,
                26,
                DeviceState::Shutdown,
            ),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:latest".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default())
            .expect("should resolve");
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].device.os_major, 26,
            "`:latest` SHALL resolve to highest version regardless of state"
        );
    }

    // 30. Within the resolved `:latest` major, state preference still wins.
    #[test]
    fn latest_tiebreaks_by_state_within_same_version() {
        let available = vec![
            make_device_with_state(
                "iPhone 26 Shutdown",
                "uid-1",
                Platform::Ios,
                DeviceType::Phone,
                26,
                DeviceState::Shutdown,
            ),
            make_device_with_state(
                "iPhone 26 Booted",
                "uid-2",
                Platform::Ios,
                DeviceType::Phone,
                26,
                DeviceState::Booted,
            ),
            make_device_with_state(
                "iPhone 18",
                "uid-3",
                Platform::Ios,
                DeviceType::Phone,
                18,
                DeviceState::Booted,
            ),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:latest".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default())
            .expect("should resolve");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].device.os_major, 26);
        assert_eq!(
            result[0].device.state,
            DeviceState::Booted,
            "within the `:latest` version, booted SHALL beat shutdown"
        );
        assert_eq!(result[0].device.name, "iPhone 26 Booted");
    }

    // 31. `:latest:N` expands to the N highest versions.
    #[test]
    fn latest_n_expands_to_top_n_versions() {
        let available = vec![
            make_device("iPhone 16", "uid-1", Platform::Ios, DeviceType::Phone, 16),
            make_device("iPhone 18", "uid-2", Platform::Ios, DeviceType::Phone, 18),
            make_device("iPhone 26", "uid-3", Platform::Ios, DeviceType::Phone, 26),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:latest:2".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default())
            .expect("should resolve");
        assert_eq!(result.len(), 2);
        let majors: HashSet<u32> = result.iter().map(|r| r.device.os_major).collect();
        assert!(majors.contains(&26));
        assert!(majors.contains(&18));
        assert!(
            !majors.contains(&16),
            "`:latest:2` SHALL exclude the oldest version"
        );
    }

    // 32. `:latest` with no matching platform devices falls back gracefully.
    //
    // When the snapshot has zero devices for the spec's platform, we can't
    // expand Latest to a concrete version — leave it as-is so the normal
    // "no matching device" error path produces a meaningful message.
    #[test]
    fn latest_with_no_matching_platform_errors_cleanly() {
        let available = vec![make_device(
            "Pixel",
            "uid-1",
            Platform::Android,
            DeviceType::Phone,
            34,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:latest".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default());
        assert!(result.is_err());
    }

    // 33. In expand=Full mode, a device selected by an earlier constraint gets
    //     credit for a later constraint's combo (the `already_covered` branch),
    //     so it is not selected a second time.
    #[test]
    fn full_mode_earlier_device_credits_later_constraint() {
        let available = vec![make_device(
            "iPhone 16",
            "uid-1",
            Platform::Ios,
            DeviceType::Phone,
            18,
        )];
        // First constraint pulls in the iPhone 16. The second (Full) constraint
        // asks for the same (ios:18, phone) combo — already covered.
        let constraints = vec![
            DeviceConstraint {
                name: Some("iPhone 16".to_string()),
                os_versions: vec![],
                device_types: vec![],
                expand: ExpandMode::Full,
            },
            DeviceConstraint {
                name: None,
                os_versions: vec!["ios:18".to_string()],
                device_types: vec![DeviceType::Phone],
                expand: ExpandMode::Full,
            },
        ];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default())
            .expect("should resolve");
        assert_eq!(
            result.len(),
            1,
            "an earlier-selected device SHALL credit a later Full combo"
        );
        assert_eq!(result[0].device.name, "iPhone 16");
    }

    // 34. expand=Full errors when no device matches and create_if_missing is off.
    #[test]
    fn full_mode_no_match_errors() {
        let available = vec![make_device(
            "iPhone 16",
            "uid-1",
            Platform::Ios,
            DeviceType::Phone,
            18,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["android:34".to_string()],
            device_types: vec![DeviceType::Tablet],
            expand: ExpandMode::Full,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default());
        assert!(result.is_err());
    }

    // 35. expand=Full with ignore_missing_physical skips a combo when the pool
    //     holds only physical devices that don't match (the requires_physical
    //     skip branch in resolve_full).
    #[test]
    fn full_mode_ignore_missing_physical_skips() {
        let available = vec![make_physical_device(
            "Pixel 8",
            "uid-1",
            Platform::Android,
            DeviceType::Phone,
            33,
            DeviceState::Connected,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["android:34".to_string()],
            device_types: vec![],
            expand: ExpandMode::Full,
        }];
        let opts = ResolveOptions {
            create_if_missing: false,
            ignore_missing_physical: true,
        };

        let result = resolve_devices(&constraints, &available, &opts).expect("should resolve");
        assert!(
            result.is_empty(),
            "unmatched physical combo SHALL be skipped silently in Full mode"
        );
    }

    // 36. expand=Full with create_if_missing on a type-only combo synthesizes a
    //     device with that type and the default iOS platform (os_opt is None).
    #[test]
    fn full_mode_create_if_missing_type_only_synthetic() {
        let available: Vec<DeviceInfo> = vec![];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec![],
            device_types: vec![DeviceType::Tablet],
            expand: ExpandMode::Full,
        }];
        let opts = ResolveOptions {
            create_if_missing: true,
            ignore_missing_physical: false,
        };

        let result = resolve_devices(&constraints, &available, &opts).expect("should resolve");
        assert_eq!(result.len(), 1);
        assert!(result[0].created);
        assert_eq!(result[0].device.state, DeviceState::NeedsCreation);
        assert_eq!(result[0].device.device_type, DeviceType::Tablet);
        assert_eq!(
            result[0].device.platform,
            Platform::Ios,
            "type-only synthetic SHALL default to the iOS platform"
        );
    }

    // 37. min_coverage with create_if_missing on a type-only requirement
    //     synthesizes via make_synthetic_for_requirement's DeviceType branch.
    #[test]
    fn min_coverage_create_if_missing_type_requirement_synthetic() {
        let available: Vec<DeviceInfo> = vec![];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec![],
            device_types: vec![DeviceType::Tablet],
            expand: ExpandMode::MinCoverage,
        }];
        let opts = ResolveOptions {
            create_if_missing: true,
            ignore_missing_physical: false,
        };

        let result = resolve_devices(&constraints, &available, &opts).expect("should resolve");
        assert_eq!(result.len(), 1);
        assert!(result[0].created);
        assert_eq!(result[0].device.device_type, DeviceType::Tablet);
        assert_eq!(result[0].device.state, DeviceState::NeedsCreation);
    }

    // 38. min_coverage with create_if_missing synthesizes one device per
    //     uncovered OS requirement when nothing is available.
    #[test]
    fn min_coverage_create_if_missing_multiple_os_synthetics() {
        let available: Vec<DeviceInfo> = vec![];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:17".to_string(), "ios:18".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];
        let opts = ResolveOptions {
            create_if_missing: true,
            ignore_missing_physical: false,
        };

        let result = resolve_devices(&constraints, &available, &opts).expect("should resolve");
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|r| r.created));
        let majors: HashSet<u32> = result.iter().map(|r| r.device.os_major).collect();
        assert!(majors.contains(&17));
        assert!(majors.contains(&18));
    }

    // 39. min_coverage with ignore_missing_physical breaks when the only
    //     remaining requirement can't be satisfied by any virtual device
    //     (uncovered_reqs_are_physical_only is true), leaving a partial result.
    #[test]
    fn min_coverage_ignore_missing_physical_partial_result() {
        // A virtual iOS phone covers ios:18; the android:34 requirement has no
        // virtual candidate, so it is treated as physical-only and skipped.
        let available = vec![make_device(
            "iPhone 16",
            "uid-1",
            Platform::Ios,
            DeviceType::Phone,
            18,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:18".to_string(), "android:34".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];
        let opts = ResolveOptions {
            create_if_missing: false,
            ignore_missing_physical: true,
        };

        let result = resolve_devices(&constraints, &available, &opts).expect("should resolve");
        assert_eq!(
            result.len(),
            1,
            "only the satisfiable requirement SHALL resolve; the physical-only one is skipped"
        );
        assert_eq!(result[0].device.os_major, 18);
    }

    // 40. expand=Full with `:latest` pins to the highest available major before
    //     building the cartesian product.
    #[test]
    fn full_mode_latest_pins_highest_major() {
        let available = vec![
            make_device("iPhone 18", "uid-1", Platform::Ios, DeviceType::Phone, 18),
            make_device("iPhone 26", "uid-2", Platform::Ios, DeviceType::Phone, 26),
        ];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:latest".to_string()],
            device_types: vec![DeviceType::Phone],
            expand: ExpandMode::Full,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default())
            .expect("should resolve");
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].device.os_major, 26,
            "Full + `:latest` SHALL pin the highest available major"
        );
    }

    // 41. An unparseable OS spec satisfies no device — a constraint built on it
    //     resolves to nothing matchable and errors.
    #[test]
    fn unparseable_os_spec_matches_nothing() {
        let available = vec![make_device(
            "iPhone 16",
            "uid-1",
            Platform::Ios,
            DeviceType::Phone,
            18,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["garbage-no-colon".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default());
        assert!(
            result.is_err(),
            "an unparseable OS spec SHALL satisfy no device and error"
        );
    }

    // 42. A spec whose platform differs from the device's platform does not
    //     match (the spec_platform != device.platform guard in device_satisfies).
    #[test]
    fn os_spec_platform_mismatch_does_not_match() {
        // Device is Android API 18; spec asks for iOS 18. Same major, wrong platform.
        let available = vec![make_device(
            "Pixel",
            "uid-1",
            Platform::Android,
            DeviceType::Phone,
            18,
        )];
        let constraints = vec![DeviceConstraint {
            name: None,
            os_versions: vec!["ios:18".to_string()],
            device_types: vec![],
            expand: ExpandMode::MinCoverage,
        }];

        let result = resolve_devices(&constraints, &available, &ResolveOptions::default());
        assert!(
            result.is_err(),
            "a same-major different-platform device SHALL NOT satisfy the spec"
        );
    }

    // 43. The reachable synthetic-device builders produce the expected concrete
    //     devices for Exact and Minimum specs. The Latest arm is unreachable
    //     (expand_latest_in_constraint rewrites it first), so it is not exercised
    //     here — calling it would panic via unreachable!().
    #[test]
    fn synthetic_builders_cover_reachable_specs() {
        // 1. make_synthetic_device with an Exact OS spec yields that major.
        let os = "android:34".to_string();
        let dt = DeviceType::Tablet;
        let synthetic = make_synthetic_device(&Some(&os), &Some(&dt))
            .expect("Exact spec SHALL build a synthetic device");
        assert_eq!(synthetic.platform, Platform::Android);
        assert_eq!(synthetic.os_major, 34);
        assert_eq!(synthetic.device_type, DeviceType::Tablet);
        assert_eq!(synthetic.state, DeviceState::NeedsCreation);

        // 2. make_synthetic_device with a Minimum OS spec keeps the floor major.
        let min_os = "ios:17+".to_string();
        let min_synthetic = make_synthetic_device(&Some(&min_os), &None)
            .expect("Minimum spec SHALL build a synthetic device");
        assert_eq!(min_synthetic.platform, Platform::Ios);
        assert_eq!(min_synthetic.os_major, 17);
        assert_eq!(min_synthetic.device_type, DeviceType::Phone);

        // 3. make_synthetic_for_requirement handles the OsVersion (Exact) branch.
        let req = Requirement::OsVersion("ios:18".to_string());
        let req_synthetic = make_synthetic_for_requirement(&req)
            .expect("OsVersion requirement SHALL build a synthetic device");
        assert_eq!(req_synthetic.platform, Platform::Ios);
        assert_eq!(req_synthetic.os_major, 18);

        // 4. make_synthetic_for_requirement handles the DeviceType branch.
        let type_req = Requirement::DeviceType(DeviceType::Tablet);
        let type_synthetic = make_synthetic_for_requirement(&type_req)
            .expect("DeviceType requirement SHALL build a synthetic device");
        assert_eq!(type_synthetic.device_type, DeviceType::Tablet);
        assert_eq!(type_synthetic.state, DeviceState::NeedsCreation);
    }
}
