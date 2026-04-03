use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Embedded iOS companion archive (tar.gz of XCTest build products).
/// Empty if build was unavailable.
const IOS_COMPANION: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/companion-ios.tar.gz"));

/// Embedded Android companion APK.
/// Empty if build was unavailable.
const ANDROID_COMPANION: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/companion-android.apk"));

/// Paths to extracted companion artifacts.
pub struct CompanionPaths {
    /// Directory containing iOS build products (Debug-iphonesimulator/ + .xctestrun)
    pub ios_products: Option<PathBuf>,
    /// Path to the Android APK
    pub android_apk: Option<PathBuf>,
}

/// Extract embedded companions to ~/.golem/companions/{version}/.
///
/// Skips extraction if already present for this version. Returns paths
/// to the extracted artifacts, or None for platforms where the companion
/// wasn't embedded.
pub fn ensure_extracted() -> Result<CompanionPaths> {
    let version = env!("CARGO_PKG_VERSION");
    let base_dir = home_dir()?.join(".golem/companions").join(version);

    let ios_products = extract_ios(&base_dir)?;
    let android_apk = extract_android(&base_dir)?;

    Ok(CompanionPaths {
        ios_products,
        android_apk,
    })
}

/// Check if embedded companions are available.
pub fn has_ios_companion() -> bool {
    !IOS_COMPANION.is_empty()
}

pub fn has_android_companion() -> bool {
    !ANDROID_COMPANION.is_empty()
}

fn extract_ios(base_dir: &Path) -> Result<Option<PathBuf>> {
    if IOS_COMPANION.is_empty() {
        return Ok(None);
    }

    let ios_dir = base_dir.join("ios");
    let marker = ios_dir.join(".extracted");

    // Already extracted for this version
    if marker.exists() {
        return Ok(Some(ios_dir));
    }

    eprintln!("  Extracting embedded iOS companion...");

    // Clean and create directory
    if ios_dir.exists() {
        fs::remove_dir_all(&ios_dir).context("failed to clean iOS companion directory")?;
    }
    fs::create_dir_all(&ios_dir).context("failed to create iOS companion directory")?;

    // Write the tar.gz and extract it
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

    // Clean up the archive
    let _ = fs::remove_file(&archive_path);

    // Write extraction marker
    fs::write(&marker, "").context("failed to write extraction marker")?;

    Ok(Some(ios_dir))
}

fn extract_android(base_dir: &Path) -> Result<Option<PathBuf>> {
    if ANDROID_COMPANION.is_empty() {
        return Ok(None);
    }

    let android_dir = base_dir.join("android");
    let apk_path = android_dir.join("app-debug-androidTest.apk");

    // Already extracted for this version
    if apk_path.exists() {
        return Ok(Some(apk_path));
    }

    eprintln!("  Extracting embedded Android companion...");

    fs::create_dir_all(&android_dir).context("failed to create Android companion directory")?;
    fs::write(&apk_path, ANDROID_COMPANION).context("failed to write Android companion APK")?;

    Ok(Some(apk_path))
}

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("could not determine home directory")
}
