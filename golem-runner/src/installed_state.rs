//! Query the install state of a bundle on a device.
//!
//! Used by the persistent install cache to evaluate the integrity gates:
//!
//! 1. Is the bundle currently installed on the device?
//! 2. What was the device-reported install time? (compared against the
//!    cache's `device_install_time` to detect external reinstalls)
//! 3. What version string does the device report? (recorded for debugging
//!    / future CI scenarios; not used for cache decisions today)

use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use golem_devices::{DeviceInfo, Platform};
use tokio::process::Command;

/// Result of querying a device for a particular bundle's install state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInstallInfo {
    /// `true` when the device reports the bundle as installed.
    pub installed: bool,
    /// Device-reported install time. iOS sim: bundle directory mtime.
    /// Android: `lastUpdateTime` from `dumpsys package`. `None` when not
    /// installed or when the platform doesn't expose it (iOS physical
    /// devices via `devicectl` — not implemented yet).
    pub install_time: Option<DateTime<Utc>>,
    /// Best-effort version string. iOS: `CFBundleVersion`. Android:
    /// `versionName`. `None` when not installed or unavailable.
    pub version: Option<String>,
}

impl DeviceInstallInfo {
    pub fn not_installed() -> Self {
        Self {
            installed: false,
            install_time: None,
            version: None,
        }
    }
}

/// Query the device for the bundle's installed state. Never errors — any
/// underlying tool failure (adb missing, simctl error, parse failure)
/// degrades to "not installed", so the cache misses safely and the
/// install path runs as today.
///
/// Platform dispatch:
/// - iOS simulator: `xcrun simctl get_app_container <udid> <bundle>` for
///   presence + path; bundle dir mtime for install time; bundle Info.plist
///   for version (via `defaults read`)
/// - iOS physical: not implemented yet — returns `not_installed()`
/// - Android (emulator + physical): `adb shell pm path` for presence;
///   `dumpsys package` for install time + version
pub async fn query(device: &DeviceInfo, bundle_id: &str) -> DeviceInstallInfo {
    match device.platform {
        Platform::Ios if !device.physical => query_ios_sim(&device.udid, bundle_id)
            .await
            .unwrap_or_else(|_| DeviceInstallInfo::not_installed()),
        Platform::Ios => DeviceInstallInfo::not_installed(),
        Platform::Android => query_android(&device.udid, bundle_id)
            .await
            .unwrap_or_else(|_| DeviceInstallInfo::not_installed()),
    }
}

async fn query_ios_sim(udid: &str, bundle_id: &str) -> Result<DeviceInstallInfo> {
    // `get_app_container app <udid> <bundle>` prints the bundle path or
    // exits nonzero when the bundle isn't installed. Fast — ~50ms.
    let out = Command::new("xcrun")
        .args(["simctl", "get_app_container", udid, bundle_id, "app"])
        .output()
        .await
        .context("invoke simctl get_app_container")?;
    if !out.status.success() {
        return Ok(DeviceInstallInfo::not_installed());
    }
    let bundle_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if bundle_path.is_empty() {
        return Ok(DeviceInstallInfo::not_installed());
    }

    // mtime of the .app directory tracks install time on the simulator —
    // each `simctl install` rewrites the bundle so the dir's mtime jumps.
    let install_time = std::fs::metadata(&bundle_path)
        .and_then(|m| m.modified())
        .ok()
        .map(system_time_to_utc);

    let version = read_ios_bundle_version(Path::new(&bundle_path)).await;

    Ok(DeviceInstallInfo {
        installed: true,
        install_time,
        version,
    })
}

async fn read_ios_bundle_version(bundle_path: &Path) -> Option<String> {
    let info_plist = bundle_path.join("Info.plist");
    if !info_plist.exists() {
        return None;
    }
    // `defaults read` works for binary and XML plists. CFBundleShortVersionString
    // is the marketing version; CFBundleVersion the build number. We prefer
    // the user-facing marketing version and fall back to the build number.
    let plist_arg = info_plist
        .to_string_lossy()
        .trim_end_matches(".plist")
        .to_string();
    if let Some(v) = read_plist_key(&plist_arg, "CFBundleShortVersionString").await {
        return Some(v);
    }
    read_plist_key(&plist_arg, "CFBundleVersion").await
}

/// Read a single string-valued plist key via `defaults read`. Returns
/// `None` for missing keys, command failures, or empty values.
async fn read_plist_key(plist_arg: &str, key: &str) -> Option<String> {
    let out = Command::new("defaults")
        .args(["read", plist_arg, key])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

async fn query_android(serial: &str, bundle_id: &str) -> Result<DeviceInstallInfo> {
    // `pm path` exits 1 when the package isn't installed. Fast presence check.
    let out = Command::new("adb")
        .args(["-s", serial, "shell", "pm", "path", bundle_id])
        .output()
        .await
        .context("invoke adb pm path")?;
    if !out.status.success() {
        return Ok(DeviceInstallInfo::not_installed());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.contains("package:") {
        return Ok(DeviceInstallInfo::not_installed());
    }

    // `dumpsys package <bundle>` exposes lastUpdateTime + versionName.
    let dump = Command::new("adb")
        .args(["-s", serial, "shell", "dumpsys", "package", bundle_id])
        .output()
        .await;
    let (install_time, version) = match dump {
        Ok(o) if o.status.success() => parse_android_dumpsys(&String::from_utf8_lossy(&o.stdout)),
        _ => (None, None),
    };

    Ok(DeviceInstallInfo {
        installed: true,
        install_time,
        version,
    })
}

/// Parse `lastUpdateTime=YYYY-MM-DD HH:MM:SS` and `versionName=X.Y.Z` from
/// `dumpsys package`. Both can be missing; both are best-effort.
fn parse_android_dumpsys(text: &str) -> (Option<DateTime<Utc>>, Option<String>) {
    let mut install_time: Option<DateTime<Utc>> = None;
    let mut version: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("lastUpdateTime=") {
            install_time = parse_android_timestamp(rest);
        } else if let Some(rest) = line.strip_prefix("versionName=") {
            let v = rest.trim().to_string();
            if !v.is_empty() {
                version = Some(v);
            }
        }
    }
    (install_time, version)
}

/// Android's `dumpsys` prints local-time `YYYY-MM-DD HH:MM:SS` without a
/// timezone. Treat as UTC for comparison purposes — the cache's
/// `device_install_time` is round-tripped through the same parser, so the
/// comparison is internally consistent regardless of the device's zone.
fn parse_android_timestamp(s: &str) -> Option<DateTime<Utc>> {
    use chrono::NaiveDateTime;
    let trimmed = s.trim();
    let naive = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S").ok()?;
    Some(naive.and_utc())
}

fn system_time_to_utc(t: SystemTime) -> DateTime<Utc> {
    let dur = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    DateTime::<Utc>::from_timestamp(dur.as_secs() as i64, dur.subsec_nanos())
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dumpsys_extracts_version_and_time() {
        let sample = "
            Packages:
              Package [com.x] (deadbeef):
                versionName=0.5.3
                versionCode=5003 minSdk=24 targetSdk=36
                lastUpdateTime=2026-04-27 09:12:45
                  firstInstallTime=2026-04-04 16:56:14
        ";
        let (t, v) = parse_android_dumpsys(sample);
        assert_eq!(v.as_deref(), Some("0.5.3"));
        assert!(t.is_some(), "lastUpdateTime SHALL parse");
        let t = t.unwrap();
        assert_eq!(
            t.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-04-27 09:12:45"
        );
    }

    #[test]
    fn parse_dumpsys_handles_missing_fields() {
        let (t, v) = parse_android_dumpsys("Packages:\n  versionCode=1\n");
        assert!(t.is_none());
        assert!(v.is_none());
    }

    #[test]
    fn parse_dumpsys_skips_first_install_time() {
        let sample = "lastUpdateTime=2026-04-27 09:12:45\nfirstInstallTime=2026-04-04 16:56:14";
        let (t, _) = parse_android_dumpsys(sample);
        let t = t.unwrap();
        assert_eq!(
            t.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-04-27 09:12:45",
            "lastUpdateTime SHALL win over firstInstallTime"
        );
    }

    #[test]
    fn android_timestamp_invalid_returns_none() {
        assert!(parse_android_timestamp("garbage").is_none());
        assert!(parse_android_timestamp("2026-04-27").is_none());
    }

    // 5. not_installed() SHALL produce the negative sentinel: not installed,
    //    no time, no version.
    #[test]
    fn not_installed_is_negative_sentinel() {
        let info = DeviceInstallInfo::not_installed();
        assert!(
            !info.installed,
            "not_installed SHALL report installed=false"
        );
        assert!(
            info.install_time.is_none(),
            "not_installed SHALL have no install_time"
        );
        assert!(
            info.version.is_none(),
            "not_installed SHALL have no version"
        );
    }

    // 6. A valid lastUpdateTime line SHALL parse, surrounding whitespace and
    //    trailing content stripped by the line trim.
    #[test]
    fn android_timestamp_valid_round_trips() {
        let t =
            parse_android_timestamp("2020-01-02 03:04:05").expect("valid timestamp SHALL parse");
        assert_eq!(
            t.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2020-01-02 03:04:05"
        );
    }

    // 7. Empty / whitespace-only timestamp input SHALL NOT parse.
    #[test]
    fn android_timestamp_empty_returns_none() {
        assert!(
            parse_android_timestamp("").is_none(),
            "empty SHALL NOT parse"
        );
        assert!(
            parse_android_timestamp("   ").is_none(),
            "whitespace SHALL NOT parse"
        );
    }

    // 8. An empty versionName value SHALL be treated as absent (None), not as
    //    an empty string.
    #[test]
    fn parse_dumpsys_empty_version_name_is_none() {
        let (_, v) = parse_android_dumpsys("versionName=\nlastUpdateTime=2026-04-27 09:12:45");
        assert!(v.is_none(), "empty versionName SHALL yield None");
    }

    // 9. versionName surrounding whitespace SHALL be trimmed.
    #[test]
    fn parse_dumpsys_version_name_trimmed() {
        let (_, v) = parse_android_dumpsys("versionName=   1.2.3   ");
        assert_eq!(v.as_deref(), Some("1.2.3"), "versionName SHALL be trimmed");
    }

    // 10. When lastUpdateTime appears multiple times, the last occurrence
    //     SHALL win (loop assigns unconditionally).
    #[test]
    fn parse_dumpsys_last_update_time_last_wins() {
        let sample = "lastUpdateTime=2020-01-01 00:00:00\nlastUpdateTime=2021-02-02 11:11:11";
        let (t, _) = parse_android_dumpsys(sample);
        let t = t.expect("a lastUpdateTime SHALL parse");
        assert_eq!(
            t.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2021-02-02 11:11:11",
            "last lastUpdateTime SHALL win"
        );
    }

    // 11. An unparseable lastUpdateTime value SHALL reset install_time to None
    //     even if an earlier valid one was seen.
    #[test]
    fn parse_dumpsys_bad_last_update_time_clears_time() {
        let sample = "lastUpdateTime=2020-01-01 00:00:00\nlastUpdateTime=garbage";
        let (t, _) = parse_android_dumpsys(sample);
        assert!(
            t.is_none(),
            "trailing unparseable lastUpdateTime SHALL clear the value"
        );
    }

    // 12. versionName containing '=' SHALL keep everything after the first '='.
    #[test]
    fn parse_dumpsys_version_name_with_equals() {
        let (_, v) = parse_android_dumpsys("versionName=1.0=beta");
        assert_eq!(
            v.as_deref(),
            Some("1.0=beta"),
            "value after first '=' SHALL be preserved"
        );
    }

    // 13. Empty dumpsys text SHALL yield no time and no version.
    #[test]
    fn parse_dumpsys_empty_text() {
        let (t, v) = parse_android_dumpsys("");
        assert!(t.is_none());
        assert!(v.is_none());
    }

    // 14. The UNIX epoch SystemTime SHALL map to the epoch UTC instant.
    #[test]
    fn system_time_to_utc_epoch() {
        let dt = system_time_to_utc(SystemTime::UNIX_EPOCH);
        assert_eq!(dt.timestamp(), 0, "epoch SHALL map to timestamp 0");
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "1970-01-01 00:00:00"
        );
    }

    // 15. A post-epoch SystemTime SHALL round-trip its second + nanosecond
    //     components into the resulting DateTime.
    #[test]
    fn system_time_to_utc_post_epoch() {
        use std::time::Duration;
        let t = SystemTime::UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789);
        let dt = system_time_to_utc(t);
        assert_eq!(dt.timestamp(), 1_700_000_000, "seconds SHALL round-trip");
        assert_eq!(
            dt.timestamp_subsec_nanos(),
            123_456_789,
            "nanos SHALL round-trip"
        );
    }

    // 16. A pre-epoch SystemTime SHALL saturate to default (zero) duration,
    //     mapping to the epoch rather than panicking.
    #[test]
    fn system_time_to_utc_pre_epoch_saturates() {
        use std::time::Duration;
        let t = SystemTime::UNIX_EPOCH - Duration::from_secs(5);
        let dt = system_time_to_utc(t);
        assert_eq!(
            dt.timestamp(),
            0,
            "pre-epoch SHALL saturate to epoch, not panic"
        );
    }
}
