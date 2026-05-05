use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};


/// Embedded iOS companion archive (tar.gz of XCTest build products).
/// Empty if build was unavailable.
const IOS_COMPANION: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/companion-ios.tar.gz"));

/// Embedded Android companion test APK (instrumentation).
/// Empty if build was unavailable.
const ANDROID_TEST_APK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/companion-android-test.apk"));

/// Embedded Android companion main APK (required for instrumentation).
/// Empty if build was unavailable.
const ANDROID_MAIN_APK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/companion-android-main.apk"));

/// Paths to extracted companion artifacts.
pub struct CompanionPaths {
    /// Directory containing iOS build products (Debug-iphonesimulator/ + .xctestrun)
    pub ios_products: Option<PathBuf>,
    /// Path to the Android test APK (instrumentation)
    pub android_apk: Option<PathBuf>,
    /// Path to the Android main APK (required for instrumentation to work)
    pub android_main_apk: Option<PathBuf>,
}

/// Extract embedded companions to ~/.golem/companions/{version}/.
///
/// Skips extraction if already present for this version. Returns paths
/// to the extracted artifacts, or None for platforms where the companion
/// wasn't embedded.
pub fn ensure_extracted() -> Result<CompanionPaths> {
    let version = env!("CARGO_PKG_VERSION");
    // Include a content hash of the embedded companions in the dir name
    // so a rebuilt companion (same version, different bytes) gets a
    // fresh extraction directory. Avoids the "I changed companion code
    // but `golem` keeps using the old cached extraction" gotcha.
    let mut hasher = DefaultHasher::new();
    hasher.write(IOS_COMPANION);
    hasher.write(ANDROID_TEST_APK);
    hasher.write(ANDROID_MAIN_APK);
    // 32-bit truncation: collision risk within a single version is
    // ~1 in 4B, negligible for our use. Keeps the dir name tidy.
    let tag = format!("{}-{:08x}", version, hasher.finish() as u32);
    let base_dir = home_dir()?.join(".golem/companions").join(&tag);

    let ios_products = extract_ios(&base_dir)?;
    let (android_apk, android_main_apk) = extract_android(&base_dir)?;

    Ok(CompanionPaths {
        ios_products,
        android_apk,
        android_main_apk,
    })
}

/// Check if embedded companions are available.
pub fn has_ios_companion() -> bool {
    !IOS_COMPANION.is_empty()
}

pub fn has_android_companion() -> bool {
    !ANDROID_TEST_APK.is_empty() && !ANDROID_MAIN_APK.is_empty()
}

fn extract_ios(base_dir: &Path) -> Result<Option<PathBuf>> {
    if IOS_COMPANION.is_empty() {
        return Ok(None);
    }

    let ios_dir = base_dir.join("ios");
    let marker = ios_dir.join(".extracted");

    if marker.exists() {
        return Ok(Some(ios_dir));
    }

    eprintln!("  Extracting embedded iOS companion...");

    if ios_dir.exists() {
        fs::remove_dir_all(&ios_dir).context("failed to clean iOS companion directory")?;
    }
    fs::create_dir_all(&ios_dir).context("failed to create iOS companion directory")?;

    let archive_path = ios_dir.join("companion-ios.tar.gz");
    fs::write(&archive_path, IOS_COMPANION).context("failed to write iOS companion archive")?;

    let output = Command::new("tar")
        .arg("xzf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&ios_dir)
        .output()
        .context("failed to extract iOS companion archive")?;

    if !output.status.success() {
        bail!(
            "Failed to extract iOS companion: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let _ = fs::remove_file(&archive_path);
    fs::write(&marker, "").context("failed to write extraction marker")?;

    Ok(Some(ios_dir))
}

fn extract_android(base_dir: &Path) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    if ANDROID_TEST_APK.is_empty() || ANDROID_MAIN_APK.is_empty() {
        return Ok((None, None));
    }

    let android_dir = base_dir.join("android");
    let test_apk = android_dir.join("app-debug-androidTest.apk");
    let main_apk = android_dir.join("app-debug.apk");

    if test_apk.exists() && main_apk.exists() {
        return Ok((Some(test_apk), Some(main_apk)));
    }

    eprintln!("  Extracting embedded Android companion...");

    fs::create_dir_all(&android_dir).context("failed to create Android companion directory")?;
    fs::write(&test_apk, ANDROID_TEST_APK).context("failed to write Android test APK")?;
    fs::write(&main_apk, ANDROID_MAIN_APK).context("failed to write Android main APK")?;

    Ok((Some(test_apk), Some(main_apk)))
}

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("could not determine home directory")
}
