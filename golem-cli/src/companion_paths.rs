//! Locate companion artifacts (iOS xctestrun directory, Android APKs) on
//! disk. Checks the embedded-companion extraction first, then falls back
//! to a relative path next to the binary or in the current working
//! directory.

use anyhow::Result;
use golem_devices::Platform;

/// Find the companion project path for the given platform.
pub(crate) fn find_companion_path(platform: Platform) -> Result<String> {
    // Check extracted embedded companions first
    if let Ok(paths) = crate::companions::ensure_extracted() {
        match platform {
            Platform::Ios => {
                if let Some(ref ios_dir) = paths.ios_products {
                    // For iOS, return the directory containing the .xctestrun file
                    return Ok(ios_dir.to_string_lossy().into_owned());
                }
            }
            Platform::Android => {
                if let Some(ref apk) = paths.android_apk {
                    if let Some(parent) = apk.parent() {
                        return Ok(parent.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }

    let relative = match platform {
        Platform::Ios => "companions/ios/GolemRunner.xcodeproj",
        Platform::Android => "companions/android",
    };

    // Check relative to current working directory
    if std::path::Path::new(relative).exists() {
        return Ok(relative.to_string());
    }

    // Check relative to golem binary location
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let path = dir.join(relative);
            if path.exists() {
                return Ok(path.to_string_lossy().into_owned());
            }
        }
    }

    anyhow::bail!(
        "Companion not found. Embedded companions may not have been built."
    )
}

/// Find the Android companion test APK.
pub(crate) fn find_android_apk() -> Result<String> {
    if let Ok(paths) = crate::companions::ensure_extracted() {
        if let Some(ref apk) = paths.android_apk {
            if apk.exists() {
                return Ok(apk.to_string_lossy().into_owned());
            }
        }
    }

    let relative = "companions/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk";
    if std::path::Path::new(relative).exists() {
        return Ok(relative.to_string());
    }
    anyhow::bail!("Android companion test APK not found.")
}

/// Find the Android companion main APK (optional, needed for fresh installs).
pub(crate) fn find_android_main_apk() -> Option<String> {
    if let Ok(paths) = crate::companions::ensure_extracted() {
        if let Some(ref apk) = paths.android_main_apk {
            if apk.exists() {
                return Some(apk.to_string_lossy().into_owned());
            }
        }
    }

    let relative = "companions/android/app/build/outputs/apk/debug/app-debug.apk";
    if std::path::Path::new(relative).exists() {
        return Some(relative.to_string());
    }
    None
}
