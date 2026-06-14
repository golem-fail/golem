use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use golem_runner::installer::PersistedInstall;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

const CACHE_PATH: &str = ".golem/install-cache.json";

#[derive(Deserialize)]
struct CacheFileView {
    #[allow(dead_code)]
    version: u32,
    entries: HashMap<String, PersistedInstall>,
}

pub fn info() -> Result<()> {
    let path = PathBuf::from(CACHE_PATH);
    if !path.exists() {
        println!("No install cache at {} (nothing built yet, or run from a different project root).", path.display());
        return Ok(());
    }

    let bytes_len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let view: CacheFileView = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {}", path.display()))?;

    let total = view.entries.len();
    println!("Install cache: {}", path.display());
    println!("  Size:    {} bytes", bytes_len);
    println!("  Entries: {total}");

    if total == 0 {
        return Ok(());
    }

    let mut times: Vec<(String, DateTime<Utc>)> = view
        .entries
        .iter()
        .map(|(k, v)| (k.clone(), v.installed_at))
        .collect();
    times.sort_by_key(|(_, t)| *t);

    let oldest = &times[0];
    let newest = &times[times.len() - 1];
    println!("  Oldest:  {}  {}", oldest.1.format("%Y-%m-%d %H:%M:%SZ"), oldest.0);
    println!("  Newest:  {}  {}", newest.1.format("%Y-%m-%d %H:%M:%SZ"), newest.0);

    let with_install_time = view
        .entries
        .values()
        .filter(|e| e.device_install_time.is_some())
        .count();
    println!("  With device install-time: {with_install_time}/{total}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use golem_runner::fingerprint::Fingerprint;
    use golem_runner::installer::PersistedInstall;
    use serde::Serialize;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().expect("valid timestamp")
    }

    fn entry(installed_at: i64, device_install_time: Option<i64>) -> PersistedInstall {
        PersistedInstall {
            fingerprint: Fingerprint::Git {
                rev: "abc123".into(),
                porcelain: "x".into(),
            },
            device_install_time: device_install_time.map(ts),
            installed_version: None,
            installed_at: ts(installed_at),
        }
    }

    // The on-disk file shape `CacheFileView` must read is whatever the
    // runner writes. We mirror it here with a serialisable twin so the
    // serialized bytes match the real cache file format exactly.
    #[derive(Serialize)]
    struct CacheFileWrite {
        version: u32,
        entries: HashMap<String, PersistedInstall>,
    }

    fn serialize_cache(version: u32, entries: HashMap<String, PersistedInstall>) -> String {
        serde_json::to_string(&CacheFileWrite { version, entries }).expect("serialize cache")
    }

    // 1. A populated cache round-trips through CacheFileView: version is
    //    ignored, all entries are preserved by key.
    #[test]
    fn cache_file_view_reads_populated_entries() {
        let mut entries = HashMap::new();
        entries.insert("udid1:bundle.a".to_string(), entry(100, Some(90)));
        entries.insert("udid2:bundle.b".to_string(), entry(200, None));
        let raw = serialize_cache(1, entries);

        let view: CacheFileView = serde_json::from_str(&raw).expect("parse cache");

        assert_eq!(view.entries.len(), 2, "both entries SHALL be read");
        assert!(
            view.entries.contains_key("udid1:bundle.a"),
            "first key SHALL be present"
        );
        assert!(
            view.entries.contains_key("udid2:bundle.b"),
            "second key SHALL be present"
        );
    }

    // 2. An empty-entries cache parses to a zero-length map (the `total == 0`
    //    early-return branch in info()).
    #[test]
    fn cache_file_view_reads_empty_entries() {
        let raw = serialize_cache(1, HashMap::new());

        let view: CacheFileView = serde_json::from_str(&raw).expect("parse empty cache");

        assert_eq!(view.entries.len(), 0, "empty cache SHALL yield no entries");
    }

    // 3. The `version` field is deserialized but unused — a differing version
    //    number SHALL NOT gate or corrupt the entries info() consumes. Proven
    //    by serializing with an arbitrary future version and confirming the
    //    entry's real fields (the data info() reads: installed_at,
    //    device_install_time) survive intact.
    #[test]
    fn cache_file_view_ignores_version_value() {
        let mut entries = HashMap::new();
        entries.insert("k".to_string(), entry(1234, Some(900)));
        let raw = serialize_cache(99, entries);

        let view: CacheFileView = serde_json::from_str(&raw).expect("parse future-version cache");

        assert_eq!(
            view.entries.len(),
            1,
            "entries SHALL parse regardless of version"
        );
        let parsed = view.entries.get("k").expect("entry under key 'k'");
        assert_eq!(
            parsed.installed_at,
            ts(1234),
            "installed_at SHALL survive a future version verbatim"
        );
        assert_eq!(
            parsed.device_install_time,
            Some(ts(900)),
            "device_install_time SHALL survive a future version verbatim"
        );
    }

    // 5. A cache missing the `entries` field SHALL fail (it has no default);
    //    `version` alone is not a valid cache file for info().
    #[test]
    fn cache_file_view_requires_entries_field() {
        let result: Result<CacheFileView, _> = serde_json::from_str(r#"{"version":1}"#);

        assert!(result.is_err(), "missing entries field SHALL fail to parse");
    }

}
