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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // 1. has_android_companion agrees with the observable Android
    //    extraction branch: both gate on the same pair of embedded APKs.
    //    When the predicate is true, extract_android SHALL yield
    //    (Some, Some); when false, (None, None). Cross-checking the
    //    public predicate against the real extraction outcome (not
    //    against the consts it is defined from) catches a regression
    //    where the predicate and the extraction gate diverge.
    #[test]
    fn has_android_companion_requires_both_apks() {
        let base = tempdir().expect("tempdir");

        let (test_apk, main_apk) =
            extract_android(base.path()).expect("extract_android SHALL succeed");

        assert_eq!(
            has_android_companion(),
            test_apk.is_some(),
            "has_android_companion SHALL agree with the Android extraction branch (test APK)"
        );
        assert_eq!(
            has_android_companion(),
            main_apk.is_some(),
            "has_android_companion SHALL agree with the Android extraction branch (main APK)"
        );
    }

    // 2. has_ios_companion agrees with the observable iOS extraction
    //    branch: both gate on the same embedded archive. When the
    //    predicate is true, extract_ios SHALL yield Some(dir); when
    //    false, it SHALL yield None. Cross-checking the public predicate
    //    against the real extraction outcome (not against the const it is
    //    defined from) catches a regression where the two diverge.
    #[test]
    fn has_ios_companion_reflects_archive_presence() {
        let base = tempdir().expect("tempdir");

        let extracted = extract_ios(base.path()).expect("extract_ios SHALL succeed");

        assert_eq!(
            has_ios_companion(),
            extracted.is_some(),
            "has_ios_companion SHALL be true iff extract_ios yields an extracted dir"
        );
    }

    // 4. extract_ios short-circuits when the .extracted marker exists:
    //    it returns the ios dir (when embedded) without re-writing the
    //    archive, or None (when not embedded). Either way it must not
    //    leave a stray companion-ios.tar.gz behind.
    #[test]
    fn extract_ios_short_circuits_on_marker() {
        let base = tempdir().expect("tempdir");
        let ios_dir = base.path().join("ios");
        fs::create_dir_all(&ios_dir).expect("create ios dir");
        fs::write(ios_dir.join(".extracted"), "").expect("write marker");

        let result = extract_ios(base.path()).expect("extract_ios SHALL succeed");

        if IOS_COMPANION.is_empty() {
            assert!(
                result.is_none(),
                "extract_ios SHALL return None when no iOS companion is embedded"
            );
        } else {
            assert_eq!(
                result.as_deref(),
                Some(ios_dir.as_path()),
                "extract_ios SHALL return the marked ios dir without re-extracting"
            );
        }
        assert!(
            !ios_dir.join("companion-ios.tar.gz").exists(),
            "marker short-circuit SHALL NOT write the archive"
        );
    }

    // 5. extract_ios returns None and touches nothing when the iOS
    //    companion is not embedded (no-op fast path). Only meaningful in
    //    a build without an embedded iOS archive; otherwise skipped.
    #[test]
    fn extract_ios_none_when_not_embedded() {
        if !IOS_COMPANION.is_empty() {
            return;
        }
        let base = tempdir().expect("tempdir");

        let result = extract_ios(base.path()).expect("extract_ios SHALL succeed");

        assert!(result.is_none(), "extract_ios SHALL return None");
        assert!(
            !base.path().join("ios").exists(),
            "extract_ios SHALL NOT create the ios dir when not embedded"
        );
    }

    // 6. extract_android short-circuits when both APKs already exist:
    //    it returns those exact paths without overwriting their contents
    //    (when embedded), or (None, None) (when not embedded).
    #[test]
    fn extract_android_short_circuits_when_apks_present() {
        let base = tempdir().expect("tempdir");
        let android_dir = base.path().join("android");
        let test_apk = android_dir.join("app-debug-androidTest.apk");
        let main_apk = android_dir.join("app-debug.apk");
        fs::create_dir_all(&android_dir).expect("create android dir");
        fs::write(&test_apk, b"sentinel-test").expect("write test apk");
        fs::write(&main_apk, b"sentinel-main").expect("write main apk");

        let (got_test, got_main) =
            extract_android(base.path()).expect("extract_android SHALL succeed");

        if !has_android_companion() {
            assert!(
                got_test.is_none() && got_main.is_none(),
                "extract_android SHALL return (None, None) when not embedded"
            );
        } else {
            assert_eq!(
                got_test.as_deref(),
                Some(test_apk.as_path()),
                "extract_android SHALL return the existing test APK path"
            );
            assert_eq!(
                got_main.as_deref(),
                Some(main_apk.as_path()),
                "extract_android SHALL return the existing main APK path"
            );
            assert_eq!(
                fs::read(&test_apk).expect("read test apk"),
                b"sentinel-test",
                "short-circuit SHALL NOT overwrite the existing test APK"
            );
            assert_eq!(
                fs::read(&main_apk).expect("read main apk"),
                b"sentinel-main",
                "short-circuit SHALL NOT overwrite the existing main APK"
            );
        }
    }

    // 7. extract_android returns (None, None) and creates nothing when
    //    the Android companion is not embedded. Skipped in builds that
    //    embed real APKs.
    #[test]
    fn extract_android_none_when_not_embedded() {
        if has_android_companion() {
            return;
        }
        let base = tempdir().expect("tempdir");

        let (got_test, got_main) =
            extract_android(base.path()).expect("extract_android SHALL succeed");

        assert!(
            got_test.is_none() && got_main.is_none(),
            "extract_android SHALL return (None, None) when not embedded"
        );
        assert!(
            !base.path().join("android").exists(),
            "extract_android SHALL NOT create the android dir when not embedded"
        );
    }
}
