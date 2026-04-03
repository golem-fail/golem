use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let workspace_root = manifest_dir.parent().expect("workspace root");

    build_ios_companion(workspace_root, &out_dir);
    build_android_companion(workspace_root, &out_dir);
}

fn build_ios_companion(workspace_root: &Path, out_dir: &Path) {
    let project = workspace_root.join("companions/ios/GolemRunner.xcodeproj");
    let archive = out_dir.join("companion-ios.tar.gz");

    if !project.exists() {
        println!("cargo:warning=iOS companion project not found at {}, skipping", project.display());
        write_empty_marker(out_dir, "companion-ios.tar.gz");
        return;
    }

    // Check if xcodebuild is available
    if Command::new("xcodebuild").arg("-version").output().is_err() {
        println!("cargo:warning=xcodebuild not found, skipping iOS companion build");
        write_empty_marker(out_dir, "companion-ios.tar.gz");
        return;
    }

    // Check if source changed (hash of Swift files)
    let source_hash = hash_directory(&workspace_root.join("companions/ios"));
    let hash_file = out_dir.join("companion-ios.hash");
    if archive.exists() && hash_file.exists() {
        if let Ok(prev_hash) = fs::read_to_string(&hash_file) {
            if prev_hash.trim() == source_hash {
                println!("cargo:warning=iOS companion unchanged, skipping rebuild");
                return;
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
                println!("cargo:warning=iOS companion built and packaged: {}", archive.display());
            } else {
                println!("cargo:warning=Failed to package iOS companion products");
                write_empty_marker(out_dir, "companion-ios.tar.gz");
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("cargo:warning=iOS companion build failed: {}", stderr.lines().last().unwrap_or("unknown error"));
            write_empty_marker(out_dir, "companion-ios.tar.gz");
        }
        Err(e) => {
            println!("cargo:warning=Failed to run xcodebuild: {e}");
            write_empty_marker(out_dir, "companion-ios.tar.gz");
        }
    }

    // Rerun if companion source changes
    println!("cargo:rerun-if-changed=../companions/ios");
}

fn build_android_companion(workspace_root: &Path, out_dir: &Path) {
    let android_dir = workspace_root.join("companions/android");
    let target_apk = out_dir.join("companion-android.apk");

    if !android_dir.exists() {
        println!("cargo:warning=Android companion directory not found, skipping");
        write_empty_marker(out_dir, "companion-android.apk");
        return;
    }

    // Check if source changed
    let source_hash = hash_directory(&android_dir.join("app/src"));
    let hash_file = out_dir.join("companion-android.hash");
    if target_apk.exists() && hash_file.exists() {
        if let Ok(prev_hash) = fs::read_to_string(&hash_file) {
            if prev_hash.trim() == source_hash {
                println!("cargo:warning=Android companion unchanged, skipping rebuild");
                return;
            }
        }
    }

    // Try to find a pre-built APK first
    let prebuilt_apk = android_dir.join("app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk");
    if prebuilt_apk.exists() {
        if fs::copy(&prebuilt_apk, &target_apk).is_ok() {
            let _ = fs::write(&hash_file, &source_hash);
            println!("cargo:warning=Android companion: using pre-built APK");
            println!("cargo:rerun-if-changed=../companions/android/app/src");
            return;
        }
    }

    // Try building with gradlew
    let gradlew = android_dir.join("gradlew");
    if !gradlew.exists() {
        println!("cargo:warning=gradlew not found, skipping Android companion build");
        write_empty_marker(out_dir, "companion-android.apk");
        return;
    }

    println!("cargo:warning=Building Android companion...");

    let status = Command::new(&gradlew)
        .arg("assembleAndroidTest")
        .current_dir(&android_dir)
        .output();

    match status {
        Ok(output) if output.status.success() => {
            if prebuilt_apk.exists() {
                if fs::copy(&prebuilt_apk, &target_apk).is_ok() {
                    let _ = fs::write(&hash_file, &source_hash);
                    println!("cargo:warning=Android companion built: {}", target_apk.display());
                }
            } else {
                println!("cargo:warning=Android companion build succeeded but APK not found");
                write_empty_marker(out_dir, "companion-android.apk");
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("cargo:warning=Android companion build failed: {}", stderr.lines().last().unwrap_or("unknown error"));
            write_empty_marker(out_dir, "companion-android.apk");
        }
        Err(e) => {
            println!("cargo:warning=Failed to run gradlew: {e}");
            write_empty_marker(out_dir, "companion-android.apk");
        }
    }

    println!("cargo:rerun-if-changed=../companions/android/app/src");
}

/// Package iOS build products into a tar.gz archive.
fn package_ios_products(products_dir: &Path, archive: &Path) -> bool {
    let sim_dir = products_dir.join("Debug-iphonesimulator");
    if !sim_dir.exists() {
        return false;
    }

    // Find the .xctestrun file
    let xctestrun = fs::read_dir(products_dir)
        .ok()
        .and_then(|entries| {
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
        .arg(xctestrun.expect("checked above").file_name().expect("has filename"))
        .output();

    matches!(status, Ok(output) if output.status.success())
}

/// Write an empty file as a marker when companion build is unavailable.
/// This prevents include_bytes! from failing at compile time.
fn write_empty_marker(out_dir: &Path, name: &str) {
    let _ = fs::write(out_dir.join(name), b"");
}

/// Compute a simple hash of all files in a directory (for change detection).
fn hash_directory(dir: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
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
