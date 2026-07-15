use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use golem_runner::installer::PersistedInstall;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

const CACHE_PATH: &str = ".golem/install-cache.json";

#[derive(Deserialize)]
struct CacheFileView {
    entries: HashMap<String, PersistedInstall>,
}

/// Pure, structured summary of a parsed install cache. Holds exactly the
/// figures `info()` renders, computed with no I/O so it can be unit-tested.
struct CacheSummary {
    total: usize,
    /// `(key, installed_at)` of the oldest/newest entries; `None` when empty.
    oldest: Option<(String, DateTime<Utc>)>,
    newest: Option<(String, DateTime<Utc>)>,
    with_install_time: usize,
}

impl CacheSummary {
    /// Compute the summary from a parsed cache view. Mirrors exactly what
    /// `info()` derives: entry count, oldest/newest by `installed_at`, and the
    /// number of entries carrying a `device_install_time`.
    fn from_view(view: &CacheFileView) -> Self {
        let total = view.entries.len();

        let mut times: Vec<(String, DateTime<Utc>)> = view
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.installed_at))
            .collect();
        times.sort_by_key(|(_, t)| *t);

        let oldest = times.first().cloned();
        let newest = times.last().cloned();

        let with_install_time = view
            .entries
            .values()
            .filter(|e| e.device_install_time.is_some())
            .count();

        CacheSummary {
            total,
            oldest,
            newest,
            with_install_time,
        }
    }
}

pub fn info() -> Result<()> {
    let path = PathBuf::from(CACHE_PATH);
    if !path.exists() {
        println!(
            "No install cache at {} (nothing built yet, or run from a different project root).",
            path.display()
        );
        return Ok(());
    }

    let bytes_len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let view: CacheFileView =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;

    let summary = CacheSummary::from_view(&view);

    println!("Install cache: {}", path.display());
    println!("  Size:    {} bytes", bytes_len);
    println!("  Entries: {}", summary.total);

    if summary.total == 0 {
        return Ok(());
    }

    if let (Some(oldest), Some(newest)) = (&summary.oldest, &summary.newest) {
        println!(
            "  Oldest:  {}  {}",
            oldest.1.format("%Y-%m-%d %H:%M:%SZ"),
            oldest.0
        );
        println!(
            "  Newest:  {}  {}",
            newest.1.format("%Y-%m-%d %H:%M:%SZ"),
            newest.0
        );
    }

    println!(
        "  With device install-time: {}/{}",
        summary.with_install_time, summary.total
    );

    Ok(())
}

/// Delete the install cache file at `path`. Returns `true` if a file was
/// removed, `false` if none existed. Pure I/O with no printing so it can be
/// unit-tested against a temp path.
fn remove_cache_at(path: &std::path::Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    Ok(true)
}

pub fn clear() -> Result<()> {
    let path = PathBuf::from(CACHE_PATH);
    if remove_cache_at(&path)? {
        println!("Removed install cache: {}", path.display());
    } else {
        println!(
            "No install cache at {} (nothing to remove).",
            path.display()
        );
    }
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
        Utc.timestamp_opt(secs, 0)
            .single()
            .expect("valid timestamp")
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

    // 3. `CacheFileView` has no `version` field, so serde silently ignores the
    //    on-disk `version` key — a differing version number SHALL NOT gate or
    //    corrupt the entries info() consumes. Proven by serializing with an
    //    arbitrary future version and confirming the entry's real fields (the
    //    data info() reads: installed_at, device_install_time) survive intact.
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

    fn view(entries: HashMap<String, PersistedInstall>) -> CacheFileView {
        CacheFileView { entries }
    }

    // 6. An empty cache summarizes to zero totals with no oldest/newest — the
    //    `total == 0` early-return path info() takes.
    #[test]
    fn summary_empty_cache_has_no_extremes() {
        let summary = CacheSummary::from_view(&view(HashMap::new()));

        assert_eq!(summary.total, 0, "empty cache SHALL report zero entries");
        assert!(summary.oldest.is_none(), "empty cache SHALL have no oldest");
        assert!(summary.newest.is_none(), "empty cache SHALL have no newest");
        assert_eq!(
            summary.with_install_time, 0,
            "empty cache SHALL count zero device install-times"
        );
    }

    // 7. A single-entry cache reports that entry as both oldest and newest.
    #[test]
    fn summary_single_entry_is_both_extremes() {
        let mut entries = HashMap::new();
        entries.insert("only:bundle".to_string(), entry(500, Some(400)));

        let summary = CacheSummary::from_view(&view(entries));

        assert_eq!(summary.total, 1, "single entry SHALL report total of 1");
        let (oldest_key, oldest_t) = summary.oldest.expect("oldest present");
        let (newest_key, newest_t) = summary.newest.expect("newest present");
        assert_eq!(oldest_key, "only:bundle", "sole entry SHALL be the oldest");
        assert_eq!(newest_key, "only:bundle", "sole entry SHALL be the newest");
        assert_eq!(oldest_t, ts(500), "oldest time SHALL match the entry");
        assert_eq!(newest_t, ts(500), "newest time SHALL match the entry");
        assert_eq!(
            summary.with_install_time, 1,
            "the entry's device install-time SHALL be counted"
        );
    }

    // 8. With several entries, oldest/newest are picked by installed_at (not
    //    insertion or key order), and device-install-time presence is counted.
    #[test]
    fn summary_picks_extremes_by_installed_at() {
        let mut entries = HashMap::new();
        entries.insert("mid".to_string(), entry(200, Some(1)));
        entries.insert("late".to_string(), entry(300, None));
        entries.insert("early".to_string(), entry(100, Some(2)));

        let summary = CacheSummary::from_view(&view(entries));

        assert_eq!(summary.total, 3, "all three entries SHALL be counted");
        let (oldest_key, oldest_t) = summary.oldest.expect("oldest present");
        let (newest_key, newest_t) = summary.newest.expect("newest present");
        assert_eq!(
            oldest_key, "early",
            "oldest SHALL be the earliest installed_at"
        );
        assert_eq!(oldest_t, ts(100), "oldest time SHALL be the earliest");
        assert_eq!(
            newest_key, "late",
            "newest SHALL be the latest installed_at"
        );
        assert_eq!(newest_t, ts(300), "newest time SHALL be the latest");
        assert_eq!(
            summary.with_install_time, 2,
            "only entries with a device install-time SHALL be counted"
        );
    }

    // 9. `remove_cache_at` deletes an existing cache file and reports it removed.
    #[test]
    fn remove_cache_deletes_existing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("install-cache.json");
        std::fs::write(&path, "{}").expect("write cache");

        let removed = remove_cache_at(&path).expect("remove");

        assert!(removed, "an existing cache file SHALL report removed=true");
        assert!(!path.exists(), "the file SHALL be gone after removal");
    }

    // 10. `remove_cache_at` is a no-op (removed=false) when no file exists.
    #[test]
    fn remove_cache_absent_is_noop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("install-cache.json");

        let removed = remove_cache_at(&path).expect("remove");

        assert!(!removed, "a missing cache file SHALL report removed=false");
    }
}
