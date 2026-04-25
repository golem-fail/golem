use crate::{DeviceInfo, DeviceState, DeviceType, Platform};
use serde::Deserialize;

/// Top-level structure for parsing `xcrun simctl list devices -j` output.
#[derive(Deserialize)]
struct SimctlOutput {
    devices: std::collections::HashMap<String, Vec<SimctlDevice>>,
}

/// A single simulator device as reported by simctl.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimctlDevice {
    name: String,
    udid: String,
    state: String,
    is_available: bool,
    device_type_identifier: Option<String>,
    last_booted_at: Option<String>,
}

/// Parse simctl JSON into a list of `DeviceInfo`.
///
/// Iterates over all runtime keys and their device lists, skipping
/// unavailable devices and extracting OS version from the runtime string.
pub fn parse_simctl_output(json: &str) -> anyhow::Result<Vec<DeviceInfo>> {
    let output: SimctlOutput = serde_json::from_str(json)?;
    let mut devices = Vec::new();

    for (runtime, sim_devices) in &output.devices {
        // Only process iOS runtimes
        if !runtime.contains("iOS") && !runtime.contains("ios") {
            continue;
        }

        let (os_major, os_version) = match parse_runtime_version(runtime) {
            Some(v) => v,
            None => continue,
        };

        for sim in sim_devices {
            if !sim.is_available {
                continue;
            }

            let device_type = sim
                .device_type_identifier
                .as_deref()
                .map_or(DeviceType::Phone, classify_device_type);

            let state = match sim.state.as_str() {
                "Booted" => DeviceState::Booted,
                _ => DeviceState::Shutdown,
            };

            devices.push(DeviceInfo {
                name: sim.name.clone(),
                udid: sim.udid.clone(),
                platform: Platform::Ios,
                device_type,
                os_major,
                os_version: os_version.clone(),
                state,
                physical: false,
                playstore: false,
                screen_width: None,
                screen_height: None,
                screen_scale: None,
                last_booted: sim.last_booted_at.clone(),
                runtime_id: Some(runtime.clone()),
                device_type_id: sim.device_type_identifier.clone(),
            });
        }
    }

    Ok(devices)
}

/// Determine if a device type ID represents an iPad (tablet) or iPhone (phone).
///
/// Device type identifiers look like:
/// - `com.apple.CoreSimulator.SimDeviceType.iPhone-15`
/// - `com.apple.CoreSimulator.SimDeviceType.iPad-Pro-13-inch-M4`
fn classify_device_type(device_type_id: &str) -> DeviceType {
    if device_type_id.contains("iPad") {
        DeviceType::Tablet
    } else {
        DeviceType::Phone
    }
}

/// Extract OS major version and full version string from a runtime identifier.
///
/// Runtime strings look like `com.apple.CoreSimulator.SimRuntime.iOS-18-6`.
/// This returns `Some((18, "18.6"))` for that input.
fn parse_runtime_version(runtime: &str) -> Option<(u32, String)> {
    // Find the iOS version portion after "iOS-"
    let ios_prefix = "iOS-";
    let idx = runtime.find(ios_prefix)?;
    let version_part = &runtime[idx + ios_prefix.len()..];

    // Split on hyphens: e.g. "18-6" -> ["18", "6"]
    let parts: Vec<&str> = version_part.split('-').collect();
    if parts.is_empty() {
        return None;
    }

    let major: u32 = parts[0].parse().ok()?;

    // Build the dotted version string
    let full_version = parts.join(".");

    Some((major, full_version))
}

/// Discover iOS simulator devices by running `xcrun simctl list devices -j`.
///
/// This performs an actual shell call and parses the resulting JSON.
pub async fn discover_ios_devices() -> anyhow::Result<Vec<DeviceInfo>> {
    let output = tokio::process::Command::new("xcrun")
        .args(["simctl", "list", "devices", "-j"])
        .output()
        .await?;

    anyhow::ensure!(
        output.status.success(),
        "xcrun simctl failed with status {}",
        output.status
    );

    let json = String::from_utf8(output.stdout)?;
    parse_simctl_output(&json)
}

// ---------------------------------------------------------------------------
// Runtime and device type discovery
// ---------------------------------------------------------------------------

/// Information about an installed iOS runtime.
#[derive(Debug, Clone)]
pub struct RuntimeInfo {
    /// Xcode runtime identifier (e.g., "com.apple.CoreSimulator.SimRuntime.iOS-18-6")
    pub identifier: String,
    /// Human-readable name (e.g., "iOS 18.6")
    pub name: String,
    /// Dotted version string (e.g., "18.6")
    pub version: String,
    /// Major version number (e.g., 18)
    pub major: u32,
}

/// Information about an available iOS device type.
#[derive(Debug, Clone)]
pub struct DeviceTypeInfo {
    /// Xcode device type identifier (e.g., "com.apple.CoreSimulator.SimDeviceType.iPhone-16")
    pub identifier: String,
    /// Human-readable name (e.g., "iPhone 16")
    pub name: String,
    /// Whether this is a phone (true) or tablet/other (false)
    pub is_phone: bool,
}

/// Discover available iOS runtimes via `xcrun simctl list runtimes -j`.
pub async fn discover_ios_runtimes() -> anyhow::Result<Vec<RuntimeInfo>> {
    let output = tokio::process::Command::new("xcrun")
        .args(["simctl", "list", "runtimes", "-j"])
        .output()
        .await?;

    anyhow::ensure!(output.status.success(), "xcrun simctl list runtimes failed");

    let text = String::from_utf8_lossy(&output.stdout);

    #[derive(Deserialize)]
    struct RuntimesOutput {
        runtimes: Vec<RuntimeEntry>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RuntimeEntry {
        identifier: String,
        name: String,
        version: String,
        is_available: bool,
    }

    let parsed: RuntimesOutput = serde_json::from_str(&text)?;

    let mut runtimes: Vec<RuntimeInfo> = parsed
        .runtimes
        .into_iter()
        .filter(|r| r.is_available && r.name.starts_with("iOS"))
        .filter_map(|r| {
            let major: u32 = r.version.split('.').next()?.parse().ok()?;
            Some(RuntimeInfo {
                identifier: r.identifier,
                name: r.name,
                version: r.version,
                major,
            })
        })
        .collect();

    // Sort by major version descending (latest first)
    runtimes.sort_by(|a, b| b.major.cmp(&a.major));
    Ok(runtimes)
}

/// Discover available iOS device types via `xcrun simctl list devicetypes -j`.
pub async fn discover_ios_device_types() -> anyhow::Result<Vec<DeviceTypeInfo>> {
    let output = tokio::process::Command::new("xcrun")
        .args(["simctl", "list", "devicetypes", "-j"])
        .output()
        .await?;

    anyhow::ensure!(output.status.success(), "xcrun simctl list devicetypes failed");

    let text = String::from_utf8_lossy(&output.stdout);

    #[derive(Deserialize)]
    struct DeviceTypesOutput {
        devicetypes: Vec<DeviceTypeEntry>,
    }
    #[derive(Deserialize)]
    struct DeviceTypeEntry {
        identifier: String,
        name: String,
    }

    let parsed: DeviceTypesOutput = serde_json::from_str(&text)?;

    Ok(parsed
        .devicetypes
        .into_iter()
        .map(|dt| {
            let is_phone = dt.name.contains("iPhone");
            DeviceTypeInfo {
                identifier: dt.identifier,
                name: dt.name,
                is_phone,
            }
        })
        .collect())
}

/// Pick an iOS runtime matching the requested OS-version spec.
///
/// - `None` / `Latest` / `Minimum` → latest (runtimes are sorted major
///   descending, so `first()` is latest).
/// - `Exact { major }` → the runtime with matching `major`, or `None` if
///   not installed. The caller turns `None` into an actionable error.
pub fn pick_runtime_for_spec<'a>(
    runtimes: &'a [RuntimeInfo],
    os_version: Option<&crate::OsVersionSpec>,
) -> Option<&'a RuntimeInfo> {
    match os_version {
        Some(crate::OsVersionSpec::Exact { major, .. }) => {
            runtimes.iter().find(|r| r.major == *major)
        }
        _ => runtimes.first(),
    }
}

/// Pick the best device type for the given form factor.
///
/// Prefers the latest model (last in the list from simctl, which tends
/// to be sorted chronologically). For phones, picks the latest iPhone.
/// For tablets, picks the latest iPad.
pub fn pick_device_type(device_types: &[DeviceTypeInfo], want_phone: bool) -> Option<&DeviceTypeInfo> {
    device_types
        .iter()
        .rfind(|dt| {
            if want_phone {
                dt.is_phone
            } else {
                dt.name.contains("iPad")
            }
        })
    // rfind = latest model (simctl lists chronologically)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OsVersionSpec, Platform};

    fn rt(major: u32) -> RuntimeInfo {
        RuntimeInfo {
            identifier: format!("iOS-{major}"),
            name: format!("iOS {major}.0"),
            version: format!("{major}.0"),
            major,
        }
    }

    // pick_runtime_for_spec: None → latest ----------------------------

    #[test]
    fn pick_runtime_none_returns_latest() {
        let runtimes = vec![rt(26), rt(18), rt(17)];
        let pick = pick_runtime_for_spec(&runtimes, None)
            .expect("SHALL return latest when spec is None");
        assert_eq!(pick.major, 26);
    }

    // pick_runtime_for_spec: Exact(18) → runtime with major==18 -------

    #[test]
    fn pick_runtime_exact_returns_matching_major() {
        let runtimes = vec![rt(26), rt(18), rt(17)];
        let pick = pick_runtime_for_spec(
            &runtimes,
            Some(&OsVersionSpec::Exact { platform: Platform::Ios, major: 18 }),
        )
        .expect("SHALL return the iOS 18 runtime");
        assert_eq!(pick.major, 18);
    }

    // pick_runtime_for_spec: Exact(99) not installed → None -----------

    #[test]
    fn pick_runtime_exact_missing_returns_none() {
        let runtimes = vec![rt(26), rt(18)];
        let pick = pick_runtime_for_spec(
            &runtimes,
            Some(&OsVersionSpec::Exact { platform: Platform::Ios, major: 99 }),
        );
        assert!(pick.is_none(),
            "SHALL return None when requested major is not installed");
    }

    // pick_runtime_for_spec: Latest → latest --------------------------

    #[test]
    fn pick_runtime_latest_returns_first() {
        let runtimes = vec![rt(26), rt(18)];
        let pick = pick_runtime_for_spec(
            &runtimes,
            Some(&OsVersionSpec::Latest { platform: Platform::Ios, count: 1 }),
        )
        .expect("SHALL return latest");
        assert_eq!(pick.major, 26);
    }

    /// Helper: build a simctl JSON string from a list of (runtime, devices) pairs.
    fn make_simctl_json(
        entries: &[(&str, &str)],
    ) -> String {
        let mut runtime_entries = Vec::new();
        for (runtime, devices_json) in entries {
            runtime_entries.push(format!("    \"{runtime}\": {devices_json}"));
        }
        format!(
            "{{\n  \"devices\": {{\n{}\n  }}\n}}",
            runtime_entries.join(",\n")
        )
    }

    fn make_device_json(
        name: &str,
        udid: &str,
        state: &str,
        is_available: bool,
        device_type_id: Option<&str>,
        last_booted_at: Option<&str>,
    ) -> String {
        let dti = match device_type_id {
            Some(id) => format!("\"deviceTypeIdentifier\": \"{id}\","),
            None => String::new(),
        };
        let lba = match last_booted_at {
            Some(ts) => format!("\"lastBootedAt\": \"{ts}\","),
            None => String::new(),
        };
        format!(
            r#"{{
        "name": "{name}",
        "udid": "{udid}",
        "state": "{state}",
        "isAvailable": {is_available},
        {dti}
        {lba}
        "dataPath": "/tmp/fake",
        "logPath": "/tmp/fake"
      }}"#
        )
    }

    // 1. Parse simctl JSON with 2 iOS 18 devices (one booted, one shutdown)
    #[test]
    fn parse_two_ios18_devices_booted_and_shutdown() {
        let dev1 = make_device_json(
            "iPhone 16 Pro",
            "AAAA-1111",
            "Booted",
            true,
            Some("com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro"),
            Some("2026-03-21T10:00:00Z"),
        );
        let dev2 = make_device_json(
            "iPhone 16",
            "BBBB-2222",
            "Shutdown",
            true,
            Some("com.apple.CoreSimulator.SimDeviceType.iPhone-16"),
            None,
        );
        let json = make_simctl_json(&[(
            "com.apple.CoreSimulator.SimRuntime.iOS-18-4",
            &format!("[{dev1}, {dev2}]"),
        )]);

        let devices = parse_simctl_output(&json).expect("should parse");
        assert_eq!(devices.len(), 2);

        let booted = devices
            .iter()
            .find(|d| d.udid == "AAAA-1111")
            .expect("should find booted device");
        assert_eq!(booted.name, "iPhone 16 Pro");
        assert_eq!(booted.platform, Platform::Ios);
        assert_eq!(booted.device_type, DeviceType::Phone);
        assert_eq!(booted.os_major, 18);
        assert_eq!(booted.os_version, "18.4");
        assert_eq!(booted.state, DeviceState::Booted);
        assert!(!booted.physical);
        assert_eq!(
            booted.last_booted,
            Some("2026-03-21T10:00:00Z".to_string())
        );

        let shutdown = devices
            .iter()
            .find(|d| d.udid == "BBBB-2222")
            .expect("should find shutdown device");
        assert_eq!(shutdown.name, "iPhone 16");
        assert_eq!(shutdown.state, DeviceState::Shutdown);
        assert_eq!(shutdown.os_version, "18.4");
        assert!(shutdown.last_booted.is_none());
    }

    // 2. Parse simctl JSON with mixed iOS versions (17 and 18)
    #[test]
    fn parse_mixed_ios_versions() {
        let dev17 = make_device_json(
            "iPhone 15",
            "CCCC-3333",
            "Shutdown",
            true,
            Some("com.apple.CoreSimulator.SimDeviceType.iPhone-15"),
            None,
        );
        let dev18 = make_device_json(
            "iPhone 16",
            "DDDD-4444",
            "Booted",
            true,
            Some("com.apple.CoreSimulator.SimDeviceType.iPhone-16"),
            None,
        );
        let json = make_simctl_json(&[
            (
                "com.apple.CoreSimulator.SimRuntime.iOS-17-5",
                &format!("[{dev17}]"),
            ),
            (
                "com.apple.CoreSimulator.SimRuntime.iOS-18-2",
                &format!("[{dev18}]"),
            ),
        ]);

        let devices = parse_simctl_output(&json).expect("should parse");
        assert_eq!(devices.len(), 2);

        let d17 = devices
            .iter()
            .find(|d| d.udid == "CCCC-3333")
            .expect("should find iOS 17 device");
        assert_eq!(d17.os_major, 17);
        assert_eq!(d17.os_version, "17.5");

        let d18 = devices
            .iter()
            .find(|d| d.udid == "DDDD-4444")
            .expect("should find iOS 18 device");
        assert_eq!(d18.os_major, 18);
        assert_eq!(d18.os_version, "18.2");
    }

    // 3. iPad device type classified correctly
    #[test]
    fn ipad_classified_as_tablet() {
        let dev = make_device_json(
            "iPad Pro 13-inch (M4)",
            "EEEE-5555",
            "Shutdown",
            true,
            Some("com.apple.CoreSimulator.SimDeviceType.iPad-Pro-13-inch-M4"),
            None,
        );
        let json = make_simctl_json(&[(
            "com.apple.CoreSimulator.SimRuntime.iOS-18-4",
            &format!("[{dev}]"),
        )]);

        let devices = parse_simctl_output(&json).expect("should parse");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device_type, DeviceType::Tablet);
        assert_eq!(devices[0].name, "iPad Pro 13-inch (M4)");
    }

    // 4. iPhone device type classified correctly
    #[test]
    fn iphone_classified_as_phone() {
        let dev = make_device_json(
            "iPhone SE (3rd generation)",
            "FFFF-6666",
            "Shutdown",
            true,
            Some("com.apple.CoreSimulator.SimDeviceType.iPhone-SE-3rd-generation"),
            None,
        );
        let json = make_simctl_json(&[(
            "com.apple.CoreSimulator.SimRuntime.iOS-17-0",
            &format!("[{dev}]"),
        )]);

        let devices = parse_simctl_output(&json).expect("should parse");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device_type, DeviceType::Phone);
    }

    // 5. Unavailable device is filtered out
    #[test]
    fn unavailable_device_is_filtered() {
        let available = make_device_json(
            "iPhone 16",
            "GGGG-7777",
            "Shutdown",
            true,
            Some("com.apple.CoreSimulator.SimDeviceType.iPhone-16"),
            None,
        );
        let unavailable = make_device_json(
            "iPhone 14",
            "HHHH-8888",
            "Shutdown",
            false,
            Some("com.apple.CoreSimulator.SimDeviceType.iPhone-14"),
            None,
        );
        let json = make_simctl_json(&[(
            "com.apple.CoreSimulator.SimRuntime.iOS-18-4",
            &format!("[{available}, {unavailable}]"),
        )]);

        let devices = parse_simctl_output(&json).expect("should parse");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].udid, "GGGG-7777");
    }

    // 6. Runtime version parsing extracts major and full version
    #[test]
    fn runtime_version_parsing() {
        let result =
            parse_runtime_version("com.apple.CoreSimulator.SimRuntime.iOS-18-6");
        assert_eq!(result, Some((18, "18.6".to_string())));

        let result =
            parse_runtime_version("com.apple.CoreSimulator.SimRuntime.iOS-17-0");
        assert_eq!(result, Some((17, "17.0".to_string())));

        let result =
            parse_runtime_version("com.apple.CoreSimulator.SimRuntime.iOS-16-4");
        assert_eq!(result, Some((16, "16.4".to_string())));

        // Non-iOS runtime should return None
        let result =
            parse_runtime_version("com.apple.CoreSimulator.SimRuntime.tvOS-18-0");
        assert_eq!(result, None);
    }

    // 7. Empty devices list returns empty vec
    #[test]
    fn empty_devices_returns_empty_vec() {
        let json = make_simctl_json(&[(
            "com.apple.CoreSimulator.SimRuntime.iOS-18-4",
            "[]",
        )]);

        let devices = parse_simctl_output(&json).expect("should parse");
        assert!(devices.is_empty());
    }

    // 8. Device state mapping (Booted/Shutdown)
    #[test]
    fn device_state_mapping() {
        let booted = make_device_json(
            "iPhone 16",
            "IIII-9999",
            "Booted",
            true,
            Some("com.apple.CoreSimulator.SimDeviceType.iPhone-16"),
            None,
        );
        let shutdown = make_device_json(
            "iPhone 15",
            "JJJJ-0000",
            "Shutdown",
            true,
            Some("com.apple.CoreSimulator.SimDeviceType.iPhone-15"),
            None,
        );
        let creating = make_device_json(
            "iPhone 14",
            "KKKK-1111",
            "Creating",
            true,
            Some("com.apple.CoreSimulator.SimDeviceType.iPhone-14"),
            None,
        );
        let json = make_simctl_json(&[(
            "com.apple.CoreSimulator.SimRuntime.iOS-18-0",
            &format!("[{booted}, {shutdown}, {creating}]"),
        )]);

        let devices = parse_simctl_output(&json).expect("should parse");
        assert_eq!(devices.len(), 3);

        let d_booted = devices
            .iter()
            .find(|d| d.udid == "IIII-9999")
            .expect("should find booted");
        assert_eq!(d_booted.state, DeviceState::Booted);

        let d_shutdown = devices
            .iter()
            .find(|d| d.udid == "JJJJ-0000")
            .expect("should find shutdown");
        assert_eq!(d_shutdown.state, DeviceState::Shutdown);

        // Any state other than "Booted" maps to Shutdown
        let d_creating = devices
            .iter()
            .find(|d| d.udid == "KKKK-1111")
            .expect("should find creating");
        assert_eq!(d_creating.state, DeviceState::Shutdown);
    }
}
