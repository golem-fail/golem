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
