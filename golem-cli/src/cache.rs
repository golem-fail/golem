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
