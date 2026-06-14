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
    let tag = extraction_tag(IOS_COMPANION, ANDROID_TEST_APK, ANDROID_MAIN_APK);
    let base_dir = home_dir()?.join(".golem/companions").join(&tag);

    let ios_products = extract_ios(&base_dir)?;
    let (android_apk, android_main_apk) = extract_android(&base_dir)?;

    Ok(CompanionPaths {
        ios_products,
        android_apk,
        android_main_apk,
    })
}

/// Compute the version+content-hash tag used for the extraction directory.
///
/// Include a content hash of the embedded companions in the dir name so a
/// rebuilt companion (same version, different bytes) gets a fresh extraction
/// directory. Avoids the "I changed companion code but `golem` keeps using
/// the old cached extraction" gotcha.
fn extraction_tag(ios: &[u8], test_apk: &[u8], main_apk: &[u8]) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let mut hasher = DefaultHasher::new();
    hasher.write(ios);
    hasher.write(test_apk);
    hasher.write(main_apk);
    // 32-bit truncation: collision risk within a single version is
    // ~1 in 4B, negligible for our use. Keeps the dir name tidy.
    format!("{}-{:08x}", version, hasher.finish() as u32)
}

/// Check if embedded companions are available.
pub fn has_ios_companion() -> bool {
    !IOS_COMPANION.is_empty()
}

pub fn has_android_companion() -> bool {
    android_available(!ANDROID_TEST_APK.is_empty(), !ANDROID_MAIN_APK.is_empty())
}

/// Both Android APKs (instrumentation test + main) must be present for the
/// Android companion to be usable.
fn android_available(test_present: bool, main_present: bool) -> bool {
    test_present && main_present
}

fn extract_ios(base_dir: &Path) -> Result<Option<PathBuf>> {
    extract_ios_from(IOS_COMPANION, base_dir)
}

fn extract_ios_from(ios: &[u8], base_dir: &Path) -> Result<Option<PathBuf>> {
    if ios.is_empty() {
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
    fs::write(&archive_path, ios).context("failed to write iOS companion archive")?;

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
    extract_android_from(ANDROID_TEST_APK, ANDROID_MAIN_APK, base_dir)
}

fn extract_android_from(
    test_bytes: &[u8],
    main_bytes: &[u8],
    base_dir: &Path,
) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    if test_bytes.is_empty() || main_bytes.is_empty() {
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
    fs::write(&test_apk, test_bytes).context("failed to write Android test APK")?;
    fs::write(&main_apk, main_bytes).context("failed to write Android main APK")?;

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

    // 8. android_available encodes the AND gate that has_android_companion
    //    relies on: both APKs must be present. Exercise all four truth-table
    //    combinations of (test_present, main_present).
    #[test]
    fn android_available_requires_both_present() {
        assert!(
            android_available(true, true),
            "android_available SHALL be true when both APKs are present"
        );
        assert!(
            !android_available(true, false),
            "android_available SHALL be false when the main APK is missing"
        );
        assert!(
            !android_available(false, true),
            "android_available SHALL be false when the test APK is missing"
        );
        assert!(
            !android_available(false, false),
            "android_available SHALL be false when both APKs are missing"
        );
    }

    // 9. has_android_companion delegates to android_available: the public
    //    predicate SHALL equal the helper applied to the embedded APK
    //    presence flags it is defined from.
    #[test]
    fn has_android_companion_delegates_to_helper() {
        assert_eq!(
            has_android_companion(),
            android_available(!ANDROID_TEST_APK.is_empty(), !ANDROID_MAIN_APK.is_empty()),
            "has_android_companion SHALL equal android_available of the embedded flags"
        );
    }

    // 10. extraction_tag is deterministic and content-addressed: identical
    //     inputs SHALL produce the identical tag, and any byte difference in
    //     any of the three slices SHALL change the tag. The tag SHALL also
    //     carry the crate version as a prefix.
    #[test]
    fn extraction_tag_is_content_addressed() {
        let base = extraction_tag(b"ios", b"test", b"main");

        // Determinism.
        assert_eq!(
            base,
            extraction_tag(b"ios", b"test", b"main"),
            "extraction_tag SHALL be deterministic for identical inputs"
        );

        // Version prefix.
        assert!(
            base.starts_with(&format!("{}-", env!("CARGO_PKG_VERSION"))),
            "extraction_tag SHALL be prefixed with the crate version"
        );

        // A change in any slice SHALL change the tag.
        assert_ne!(
            base,
            extraction_tag(b"IOS", b"test", b"main"),
            "extraction_tag SHALL change when the iOS bytes change"
        );
        assert_ne!(
            base,
            extraction_tag(b"ios", b"TEST", b"main"),
            "extraction_tag SHALL change when the test APK bytes change"
        );
        assert_ne!(
            base,
            extraction_tag(b"ios", b"test", b"MAIN"),
            "extraction_tag SHALL change when the main APK bytes change"
        );
    }

    // 11. extract_ios_from honors the injected bytes: empty bytes are a
    //     no-op (None, no dir created) regardless of the embedded const;
    //     non-empty bytes that are not a valid archive SHALL surface the
    //     tar failure as an error.
    #[test]
    fn extract_ios_from_empty_bytes_is_noop() {
        let base = tempdir().expect("tempdir");

        let result = extract_ios_from(&[], base.path()).expect("extract_ios_from SHALL succeed");

        assert!(
            result.is_none(),
            "extract_ios_from SHALL return None for empty bytes"
        );
        assert!(
            !base.path().join("ios").exists(),
            "extract_ios_from SHALL NOT create the ios dir for empty bytes"
        );
    }

    #[test]
    fn extract_ios_from_invalid_archive_errors() {
        let base = tempdir().expect("tempdir");

        let result = extract_ios_from(b"not a real tar.gz", base.path());

        assert!(
            result.is_err(),
            "extract_ios_from SHALL error when the injected bytes are not a valid archive"
        );
        assert!(
            !base.path().join("ios").join(".extracted").exists(),
            "extract_ios_from SHALL NOT write the completion marker on a failed extraction"
        );
    }

    // 12. extract_android_from honors the injected bytes: it writes exactly
    //     those bytes to the two APK paths, and short-circuits without
    //     overwriting when both already exist.
    #[test]
    fn extract_android_from_writes_injected_bytes() {
        let base = tempdir().expect("tempdir");

        let (test_apk, main_apk) =
            extract_android_from(b"test-bytes", b"main-bytes", base.path())
                .expect("extract_android_from SHALL succeed");

        let test_apk = test_apk.expect("test APK path SHALL be Some");
        let main_apk = main_apk.expect("main APK path SHALL be Some");
        assert_eq!(
            fs::read(&test_apk).expect("read test apk"),
            b"test-bytes",
            "extract_android_from SHALL write the injected test bytes"
        );
        assert_eq!(
            fs::read(&main_apk).expect("read main apk"),
            b"main-bytes",
            "extract_android_from SHALL write the injected main bytes"
        );
    }

    #[test]
    fn extract_android_from_empty_bytes_is_noop() {
        let base = tempdir().expect("tempdir");

        // Either slice empty SHALL yield (None, None) and create nothing.
        let (test_apk, main_apk) =
            extract_android_from(&[], b"main-bytes", base.path())
                .expect("extract_android_from SHALL succeed");

        assert!(
            test_apk.is_none() && main_apk.is_none(),
            "extract_android_from SHALL return (None, None) when the test slice is empty"
        );
        assert!(
            !base.path().join("android").exists(),
            "extract_android_from SHALL NOT create the android dir when a slice is empty"
        );
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
