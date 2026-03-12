use std::collections::HashSet;

use anyhow::{bail, Context};

use crate::version::{matches_version, parse_os_version};
use crate::{DeviceInfo, DeviceType, ResolvedDevice};

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
) -> anyhow::Result<Vec<ResolvedDevice>> {
    let mut result: Vec<ResolvedDevice> = Vec::new();

    for constraint in constraints {
        if let Some(ref name) = constraint.name {
            resolve_named(name, available, &mut result)?;
        } else {
            match constraint.expand {
                ExpandMode::Full => {
                    resolve_full(constraint, available, &mut result)?;
                }
                ExpandMode::MinCoverage => {
                    resolve_min_coverage(constraint, available, &mut result)?;
                }
            }
        }
    }

    Ok(result)
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

        // Find a matching available device that isn't already selected.
        let selected_udids: HashSet<&str> =
            result.iter().map(|r| r.device.udid.as_str()).collect();

        let device = available
            .iter()
            .filter(|d| !selected_udids.contains(d.udid.as_str()))
            .find(|d| {
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
            .with_context(|| {
                format!(
                    "no available device matching os={:?}, type={:?}",
                    os_opt, dt_opt
                )
            })?;

        result.push(ResolvedDevice {
            device: device.clone(),
            apps: Vec::new(),
            created: false,
        });
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

    // Build a pool of candidates (available devices not already selected).
    let candidates: Vec<&DeviceInfo> = available
        .iter()
        .filter(|d| !selected_udids.contains(&d.udid))
        .collect();

    while covered.len() < requirements.len() {
        // For each candidate, compute how many *uncovered* requirements it covers.
        let best = candidates
            .iter()
            .filter(|d| !result.iter().any(|r| r.device.udid == d.udid))
            .map(|d| {
                let device_covers = covered_requirements(d, &requirements);
                let new_covers: HashSet<usize> =
                    device_covers.difference(&covered).copied().collect();
                (d, new_covers)
            })
            .filter(|(_, new)| !new.is_empty())
            .max_by_key(|(_, new)| new.len());

        match best {
            Some((device, new_covers)) => {
                covered = covered.union(&new_covers).copied().collect();
                result.push(ResolvedDevice {
                    device: (*device).clone(),
                    apps: Vec::new(),
                    created: false,
                });
            }
            None => {
                // Collect uncovered requirements for the error message.
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available);
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available);
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
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

        let result = resolve_devices(&constraints, &available).expect("should resolve");
        assert!(result.is_empty());
    }
}
