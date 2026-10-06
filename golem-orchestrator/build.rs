use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let workspace_root = manifest_dir.parent().expect("workspace root");

    build_ios_companion(workspace_root, &out_dir);
    build_android_companion(workspace_root, &out_dir);
}

fn build_ios_companion(workspace_root: &Path, out_dir: &Path) {
    println!("cargo:rerun-if-changed=../companions/ios/GolemRunnerUITests");
    println!("cargo:rerun-if-changed=../companions/ios/GolemRunnerApp");
    println!("cargo:rerun-if-changed=../companions/ios/GolemRunner.xcodeproj");

    // Gate on the *target* OS, not the host. iOS simulators exist only on macOS
    // and the XCTest bundle is unusable elsewhere, so a non-macOS target (e.g. a
    // Linux cross-build from a mac host that has Xcode installed) must embed zero
    // iOS bytes deterministically — never rely on "xcodebuild happened to be
    // absent." CARGO_CFG_TARGET_OS is the target being compiled for; a build
    // script's own cfg!(target_os) would be the host and get this wrong.
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        println!("cargo:warning=iOS companion skipped for non-macOS target ({target_os})");
        write_empty_marker(out_dir, "companion-ios.tar.gz");
        return;
    }

    let project = workspace_root.join("companions/ios/GolemRunner.xcodeproj");
    let archive = out_dir.join("companion-ios.tar.gz");

    if !project.exists() {
        println!(
            "cargo:warning=iOS companion project not found at {}, skipping",
            project.display()
        );
        write_empty_marker(out_dir, "companion-ios.tar.gz");
        return;
    }

    // Check if xcodebuild is available
    if Command::new("xcodebuild").arg("-version").output().is_err() {
        println!("cargo:warning=xcodebuild not found, skipping iOS companion build");
        write_empty_marker(out_dir, "companion-ios.tar.gz");
        return;
    }

    // Check if source changed (hash of Swift/project files only, not build artifacts).
    // Store hash in stable location (cargo assigns fresh out_dir on recompile).
    let ios_dir = workspace_root.join("companions/ios");
    let source_hash = hash_directories(&[
        ios_dir.join("GolemRunnerUITests"),
        ios_dir.join("GolemRunnerApp"),
        ios_dir.join("GolemRunner.xcodeproj"),
    ]);
    let stable_hash_dir = workspace_root.join("target").join("companion-cache");
    let _ = fs::create_dir_all(&stable_hash_dir);
    let hash_file = stable_hash_dir.join("ios.hash");
    if hash_file.exists() {
        if let Ok(prev_hash) = fs::read_to_string(&hash_file) {
            if prev_hash.trim() == source_hash {
                let cached_archive = stable_hash_dir.join("companion-ios.tar.gz");
                if cached_archive.exists() {
                    let _ = fs::copy(&cached_archive, &archive);
                    println!("cargo:warning=iOS companion unchanged, skipping rebuild");
                    return;
                }
            }
        }
    }

    println!("cargo:warning=Building iOS companion (universal simulator)...");

    // Build for both arm64 and x86_64 simulator architectures
    let status = Command::new("xcodebuild")
        .arg("build-for-testing")
        .arg("-project")
        .arg(&project)
        .arg("-scheme")
        .arg("GolemRunnerUITests")
        .arg("-sdk")
        .arg("iphonesimulator")
        .arg("-destination")
        .arg("generic/platform=iOS Simulator")
        .arg("ARCHS=arm64 x86_64")
        .arg("ONLY_ACTIVE_ARCH=NO")
        .arg("-derivedDataPath")
        .arg(out_dir.join("ios-derived"))
        .output();

    match status {
        Ok(output) if output.status.success() => {
            // Package the build products into a tar.gz
            let products_dir = out_dir.join("ios-derived/Build/Products");
            if package_ios_products(&products_dir, &archive) {
                let _ = fs::write(&hash_file, &source_hash);
                let _ = fs::copy(&archive, stable_hash_dir.join("companion-ios.tar.gz"));
                println!(
                    "cargo:warning=iOS companion built and packaged: {}",
                    archive.display()
                );
            } else {
                println!("cargo:warning=Failed to package iOS companion products");
                write_empty_marker(out_dir, "companion-ios.tar.gz");
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!(
                "cargo:warning=iOS companion build failed: {}",
                stderr.lines().last().unwrap_or("unknown error")
            );
            write_empty_marker(out_dir, "companion-ios.tar.gz");
        }
        Err(e) => {
            println!("cargo:warning=Failed to run xcodebuild: {e}");
            write_empty_marker(out_dir, "companion-ios.tar.gz");
        }
    }
}

fn build_android_companion(workspace_root: &Path, out_dir: &Path) {
    println!("cargo:rerun-if-changed=../companions/android/app/src");

    let android_dir = workspace_root.join("companions/android");
    let target_test_apk = out_dir.join("companion-android-test.apk");
    let target_main_apk = out_dir.join("companion-android-main.apk");

    if !android_dir.exists() {
        println!("cargo:warning=Android companion directory not found, skipping");
        write_empty_marker(out_dir, "companion-android-test.apk");
        write_empty_marker(out_dir, "companion-android-main.apk");
        return;
    }

    // Check if source changed. Store hash in workspace target dir (stable across
    // build script recompilations — cargo assigns fresh out_dir each time).
    let source_hash = hash_directory(&android_dir.join("app/src"));
    let stable_hash_dir = workspace_root.join("target").join("companion-cache");
    let _ = fs::create_dir_all(&stable_hash_dir);
    let hash_file = stable_hash_dir.join("android.hash");
    // Copy cached APKs into current out_dir if source unchanged
    if hash_file.exists() {
        if let Ok(prev_hash) = fs::read_to_string(&hash_file) {
            if prev_hash.trim() == source_hash {
                let cached_test = stable_hash_dir.join("android-test.apk");
                let cached_main = stable_hash_dir.join("android-main.apk");
                if cached_test.exists() && cached_main.exists() {
                    let _ = fs::copy(&cached_test, &target_test_apk);
                    let _ = fs::copy(&cached_main, &target_main_apk);
                    println!("cargo:warning=Android companion unchanged, skipping rebuild");
                    return;
                }
            }
        }
    }

    // Pre-built APK output paths (used after gradle build)
    let prebuilt_test =
        android_dir.join("app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk");
    let prebuilt_main = android_dir.join("app/build/outputs/apk/debug/app-debug.apk");

    // Try building with gradlew
    let gradlew = android_dir.join("gradlew");
    if !gradlew.exists() {
        println!("cargo:warning=gradlew not found, skipping Android companion build");
        write_empty_marker(out_dir, "companion-android-test.apk");
        write_empty_marker(out_dir, "companion-android-main.apk");
        return;
    }

    println!("cargo:warning=Building Android companion...");

    // Build both debug and androidTest APKs
    let status = Command::new(&gradlew)
        .args(["assembleDebug", "assembleAndroidTest"])
        .current_dir(&android_dir)
        .output();

    match status {
        Ok(output) if output.status.success() => {
            let mut ok = true;
            if prebuilt_test.exists() {
                if fs::copy(&prebuilt_test, &target_test_apk).is_err() {
                    ok = false;
                }
            } else {
                ok = false;
            }
            if prebuilt_main.exists() {
                if fs::copy(&prebuilt_main, &target_main_apk).is_err() {
                    ok = false;
                }
            } else {
                ok = false;
            }

            if ok {
                let _ = fs::write(&hash_file, &source_hash);
                let _ = fs::copy(&target_test_apk, stable_hash_dir.join("android-test.apk"));
                let _ = fs::copy(&target_main_apk, stable_hash_dir.join("android-main.apk"));
                println!("cargo:warning=Android companion built (both APKs)");
            } else {
                println!("cargo:warning=Android companion build succeeded but APKs not found");
                write_empty_marker(out_dir, "companion-android-test.apk");
                write_empty_marker(out_dir, "companion-android-main.apk");
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!(
                "cargo:warning=Android companion build failed: {}",
                stderr.lines().last().unwrap_or("unknown error")
            );
            write_empty_marker(out_dir, "companion-android-test.apk");
            write_empty_marker(out_dir, "companion-android-main.apk");
        }
        Err(e) => {
            println!("cargo:warning=Failed to run gradlew: {e}");
            write_empty_marker(out_dir, "companion-android-test.apk");
            write_empty_marker(out_dir, "companion-android-main.apk");
        }
    }
}

/// Package iOS build products into a tar.gz archive.
fn package_ios_products(products_dir: &Path, archive: &Path) -> bool {
    let sim_dir = products_dir.join("Debug-iphonesimulator");
    if !sim_dir.exists() {
        return false;
    }

    // Find the .xctestrun file
    let xctestrun = fs::read_dir(products_dir).ok().and_then(|entries| {
        entries
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().is_some_and(|ext| ext == "xctestrun"))
            .map(|e| e.path())
    });

    if xctestrun.is_none() {
        return false;
    }

    // Use tar to create the archive
    let status = Command::new("tar")
        .arg("czf")
        .arg(archive)
        .arg("-C")
        .arg(products_dir)
        .arg("Debug-iphonesimulator")
        .arg(
            xctestrun
                .expect("checked above")
                .file_name()
                .expect("has filename"),
        )
        .output();

    matches!(status, Ok(output) if output.status.success())
}

/// Write an empty file as a marker when companion build is unavailable.
/// This prevents include_bytes! from failing at compile time.
fn write_empty_marker(out_dir: &Path, name: &str) {
    let _ = fs::write(out_dir.join(name), b"");
}

/// Compute a hash of all files in a directory plus the golem version.
/// Version is included so that version bumps trigger companion rebuilds.
fn hash_directories(dirs: &[PathBuf]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    env::var("CARGO_PKG_VERSION")
        .unwrap_or_default()
        .hash(&mut hasher);
    for dir in dirs {
        if let Ok(entries) = walkdir(dir) {
            for path in entries {
                if let Ok(contents) = fs::read(&path) {
                    path.to_string_lossy().hash(&mut hasher);
                    contents.hash(&mut hasher);
                }
            }
        }
    }
    format!("{:016x}", hasher.finish())
}

fn hash_directory(dir: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    // Include version so bumps trigger rebuild
    env::var("CARGO_PKG_VERSION")
        .unwrap_or_default()
        .hash(&mut hasher);
    if let Ok(entries) = walkdir(dir) {
        for path in entries {
            if let Ok(contents) = fs::read(&path) {
                path.to_string_lossy().hash(&mut hasher);
                contents.hash(&mut hasher);
            }
        }
    }
    format!("{:016x}", hasher.finish())
}

/// Recursively list files in a directory, sorted for deterministic hashing.
fn walkdir(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut files = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                // Skip build artifacts and hidden directories
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name.starts_with('.') || name == "build" || name == "DerivedData" {
                    continue;
                }
                files.extend(walkdir(&path)?);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}
