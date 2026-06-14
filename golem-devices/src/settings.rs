//! Per-platform device settings applied once per session, before any
//! flow runs. The orchestrator reads `[device_settings]` from
//! `golem.toml` and dispatches to platform-specific appliers below.
//!
//! Why this exists: emulator wipes / new system images reset OS-level
//! state that perturbs test runs — Android's stylus handwriting
//! overlay, Pixel onboarding tips, iOS first-run sheets. Manually
//! re-applying `adb shell settings put` after every wipe is fragile.
//! Centralising the knobs in `golem.toml` makes the desired state
//! declarative and reproducible.
//!
//! Keys are namespaced by the platform-native grouping: Android's
//! `<namespace>.<key>` (system / secure / global), iOS's
//! `<domain>.<key>` for `defaults write`. The applier translates the
//! TOML key to the right shell command.

use crate::{DeviceInfo, Platform};

/// Apply Android settings via `adb shell settings put <ns> <key> <value>`.
/// `entries` keys are `<namespace>.<key>` (system / secure / global).
/// Errors are best-effort: a single failed key emits a warning but
/// other keys still apply.
pub async fn apply_android_settings(
    device: &DeviceInfo,
    entries: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for (full_key, value) in entries {
        let (namespace, key) = match full_key.split_once('.') {
            Some(pair) => pair,
            None => {
                warnings.push(format!(
                    "device_settings.android key {full_key:?} missing namespace prefix \
                     (expected system. / secure. / global.)"
                ));
                continue;
            }
        };
        if !matches!(namespace, "system" | "secure" | "global") {
            warnings.push(format!(
                "device_settings.android namespace {namespace:?} not one of system|secure|global"
            ));
            continue;
        }
        let args = vec![
            "-s".to_string(),
            device.udid.clone(),
            "shell".to_string(),
            "settings".to_string(),
            "put".to_string(),
            namespace.to_string(),
            key.to_string(),
            value.clone(),
        ];
        let output = tokio::process::Command::new("adb")
            .args(&args)
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                warnings.push(format!(
                    "device_settings.android {namespace}.{key} = {value:?} failed: {stderr}"
                ));
            }
            Err(e) => {
                warnings.push(format!(
                    "device_settings.android {namespace}.{key} = {value:?} failed: {e}"
                ));
            }
        }
    }
    warnings
}

/// Apply iOS `defaults write` settings via `xcrun simctl spawn <udid>
/// defaults write <domain> <key> <value>`. Keys are
/// `<domain>.<key>` where dots in the domain itself are written with
/// underscores in the TOML key (e.g. `com_apple_springboard.foo`) —
/// the underscores are translated back to dots before invoking
/// `defaults write`.
pub async fn apply_ios_settings(
    device: &DeviceInfo,
    entries: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for (full_key, value) in entries {
        let (domain_us, key) = match full_key.split_once('.') {
            Some(pair) => pair,
            None => {
                warnings.push(format!(
                    "device_settings.ios key {full_key:?} missing domain prefix"
                ));
                continue;
            }
        };
        let domain = domain_us.replace('_', ".");
        let args = vec![
            "simctl".to_string(),
            "spawn".to_string(),
            device.udid.clone(),
            "defaults".to_string(),
            "write".to_string(),
            domain.clone(),
            key.to_string(),
            value.clone(),
        ];
        let output = tokio::process::Command::new("xcrun")
            .args(&args)
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                warnings.push(format!(
                    "device_settings.ios {domain}.{key} = {value:?} failed: {stderr}"
                ));
            }
            Err(e) => {
                warnings.push(format!(
                    "device_settings.ios {domain}.{key} = {value:?} failed: {e}"
                ));
            }
        }
    }
    warnings
}

/// Apply all platform-relevant settings for a device. Idempotent —
/// safe to re-apply across flows on the same session, though the
/// orchestrator only calls this once per device per `golem run`.
pub async fn apply_device_settings(
    device: &DeviceInfo,
    android: &std::collections::HashMap<String, String>,
    ios: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    match device.platform {
        Platform::Android => apply_android_settings(device, android).await,
        Platform::Ios => apply_ios_settings(device, ios).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceState, DeviceType};
    use std::collections::HashMap;

    // Build a minimal device for a given platform. The settings appliers
    // only read `udid` and (for the dispatcher) `platform`.
    fn device(platform: Platform) -> DeviceInfo {
        DeviceInfo {
            name: "test-device".to_string(),
            udid: "UDID-XYZ".to_string(),
            platform,
            device_type: DeviceType::Phone,
            os_major: 17,
            os_version: "17.2".to_string(),
            state: DeviceState::Booted,
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

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // 1. Empty Android entries spawn no process and yield no warnings.
    #[tokio::test]
    async fn android_empty_entries_yields_no_warnings() {
        let warnings = apply_android_settings(&device(Platform::Android), &HashMap::new()).await;
        assert!(
            warnings.is_empty(),
            "empty android entries SHALL produce no warnings, got {warnings:?}"
        );
    }

    // 2. An Android key with no namespace prefix is rejected (no `.` to split).
    #[tokio::test]
    async fn android_missing_namespace_prefix_warns() {
        let entries = map(&[("nodotkey", "1")]);
        let warnings = apply_android_settings(&device(Platform::Android), &entries).await;
        assert_eq!(
            warnings.len(),
            1,
            "missing-namespace key SHALL produce exactly one warning"
        );
        assert!(
            warnings[0].contains("missing namespace prefix") && warnings[0].contains("nodotkey"),
            "warning SHALL name the offending key and reason, got {:?}",
            warnings[0]
        );
    }

    // 3. An Android key whose namespace is not system|secure|global is rejected.
    #[tokio::test]
    async fn android_invalid_namespace_warns() {
        let entries = map(&[("bogus.some_key", "1")]);
        let warnings = apply_android_settings(&device(Platform::Android), &entries).await;
        assert_eq!(
            warnings.len(),
            1,
            "invalid-namespace key SHALL produce exactly one warning"
        );
        assert!(
            warnings[0].contains("not one of system|secure|global")
                && warnings[0].contains("bogus"),
            "warning SHALL name the bad namespace, got {:?}",
            warnings[0]
        );
    }

    // 5. Multiple invalid Android keys each produce their own warning.
    #[tokio::test]
    async fn android_multiple_invalid_keys_each_warn() {
        let entries = map(&[("nodot", "1"), ("wrongns.k", "2")]);
        let warnings = apply_android_settings(&device(Platform::Android), &entries).await;
        assert_eq!(
            warnings.len(),
            2,
            "two invalid keys SHALL produce two warnings, got {warnings:?}"
        );
    }

    // 6. Empty iOS entries spawn no process and yield no warnings.
    #[tokio::test]
    async fn ios_empty_entries_yields_no_warnings() {
        let warnings = apply_ios_settings(&device(Platform::Ios), &HashMap::new()).await;
        assert!(
            warnings.is_empty(),
            "empty ios entries SHALL produce no warnings, got {warnings:?}"
        );
    }

    // 7. An iOS key with no domain prefix (no `.`) is rejected before spawn.
    #[tokio::test]
    async fn ios_missing_domain_prefix_warns() {
        let entries = map(&[("nodotdomain", "1")]);
        let warnings = apply_ios_settings(&device(Platform::Ios), &entries).await;
        assert_eq!(
            warnings.len(),
            1,
            "missing-domain key SHALL produce exactly one warning"
        );
        assert!(
            warnings[0].contains("missing domain prefix") && warnings[0].contains("nodotdomain"),
            "warning SHALL name the offending key and reason, got {:?}",
            warnings[0]
        );
    }

    // 8. The Android dispatcher routes to the Android applier and ignores the
    //    iOS map: an invalid android key warns, an (ignored) ios entry does not.
    #[tokio::test]
    async fn dispatch_android_uses_android_map_only() {
        let android = map(&[("nodot", "1")]);
        let ios = map(&[("would.be.ios", "x")]);
        let warnings = apply_device_settings(&device(Platform::Android), &android, &ios).await;
        assert_eq!(
            warnings.len(),
            1,
            "android dispatch SHALL apply only the android map, got {warnings:?}"
        );
        assert!(
            warnings[0].contains("device_settings.android"),
            "android dispatch warning SHALL come from the android applier, got {:?}",
            warnings[0]
        );
    }

    // 9. The iOS dispatcher routes to the iOS applier and ignores the
    //    Android map.
    #[tokio::test]
    async fn dispatch_ios_uses_ios_map_only() {
        let android = map(&[("secure.would_be_android", "1")]);
        let ios = map(&[("nodot", "x")]);
        let warnings = apply_device_settings(&device(Platform::Ios), &android, &ios).await;
        assert_eq!(
            warnings.len(),
            1,
            "ios dispatch SHALL apply only the ios map, got {warnings:?}"
        );
        assert!(
            warnings[0].contains("device_settings.ios"),
            "ios dispatch warning SHALL come from the ios applier, got {:?}",
            warnings[0]
        );
    }
}
