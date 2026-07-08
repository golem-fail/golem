//! Auto-create simulators/emulators when no matching device is available.

use anyhow::{bail, Result};

use crate::Platform;

use super::exec::{boot_device, create_simulator};

// ---------------------------------------------------------------------------
// Auto-create device
// ---------------------------------------------------------------------------

/// Estimated disk space needed per device (MB).
const IOS_DEVICE_SIZE_MB: u64 = 5_000;
const ANDROID_DEVICE_SIZE_MB: u64 = 4_000;

/// Auto-create and boot a simulator/emulator matching the given platform.
///
/// Discovers available runtimes/images, picks the best match, creates the
/// device, and boots it. Checks disk space before creation.
///
/// `os_version` narrows the runtime/image selection: `Exact(N)` picks a
/// specific major (erroring if not installed); anything else picks latest.
///
/// Returns the newly created and booted DeviceInfo.
pub async fn auto_create_device(
    platform: Platform,
    device_type: crate::DeviceType,
    os_version: Option<crate::OsVersionSpec>,
    playstore: Option<bool>,
    concurrency_config: &crate::concurrency::ConcurrencyConfig,
) -> Result<crate::DeviceInfo> {
    let want_phone = device_type == crate::DeviceType::Phone;

    // Check disk space
    let estimated_size = match platform {
        Platform::Ios => IOS_DEVICE_SIZE_MB,
        Platform::Android => ANDROID_DEVICE_SIZE_MB,
    };
    if !crate::concurrency::has_sufficient_disk(concurrency_config, estimated_size)? {
        bail!(
            "Insufficient disk space to create a new {} device. \
             Need {}MB free above min_free_disk_mb ({}MB).",
            platform,
            estimated_size,
            concurrency_config.min_free_disk_mb,
        );
    }

    match platform {
        Platform::Ios => auto_create_ios(want_phone, os_version.as_ref()).await,
        Platform::Android => auto_create_android(want_phone, os_version.as_ref(), playstore).await,
    }
}

async fn auto_create_ios(
    want_phone: bool,
    os_version: Option<&crate::OsVersionSpec>,
) -> Result<crate::DeviceInfo> {
    use crate::ios::{
        discover_ios_device_types, discover_ios_runtimes, pick_device_type, pick_runtime_for_spec,
    };

    let runtimes = discover_ios_runtimes().await?;
    if runtimes.is_empty() {
        return Err(golem_events::coded(
            golem_events::FailureCode::HostToolchainMissing,
            anyhow::anyhow!("No iOS runtimes installed. Install one via Xcode."),
        ));
    }
    let runtime = pick_runtime_for_spec(&runtimes, os_version).ok_or_else(|| {
        let requested = match os_version {
            Some(crate::OsVersionSpec::Exact { major, .. }) => format!("iOS {major}"),
            _ => "any iOS".to_string(),
        };
        let installed: Vec<String> = runtimes
            .iter()
            .map(|r| format!("iOS {}", r.major))
            .collect();
        golem_events::coded(
            golem_events::FailureCode::HostToolchainMissing,
            anyhow::anyhow!(
                "Requested {requested} runtime is not installed. Installed: {}. \
                     Add via Xcode > Settings > Platforms.",
                installed.join(", ")
            ),
        )
    })?;

    let device_types = discover_ios_device_types().await?;
    let device_type = pick_device_type(&device_types, want_phone).ok_or_else(|| {
        golem_events::coded(
            golem_events::FailureCode::HostToolchainMissing,
            anyhow::anyhow!(
                "No {} device type found. Install Xcode device support.",
                if want_phone { "iPhone" } else { "iPad" }
            ),
        )
    })?;

    let name = format!(
        "golem-{}-ios{}",
        device_type.name.replace(' ', "-"),
        runtime.major
    );
    eprintln!(
        "  Creating iOS simulator: {name} ({}, {})",
        device_type.name, runtime.name
    );

    let output = create_simulator(
        Platform::Ios,
        &name,
        &device_type.identifier,
        &runtime.identifier,
    )
    .await?;

    // xcrun simctl create returns the UDID on stdout
    let udid = output.trim().to_string();
    if udid.is_empty() {
        return Err(golem_events::coded(
            golem_events::FailureCode::DeviceCreateFailed,
            anyhow::anyhow!("Failed to create simulator: no UDID returned"),
        ));
    }

    let device = crate::DeviceInfo {
        name: name.clone(),
        udid: udid.clone(),
        platform: Platform::Ios,
        device_type: if want_phone {
            crate::DeviceType::Phone
        } else {
            crate::DeviceType::Tablet
        },
        os_major: runtime.major,
        os_version: runtime.version.clone(),
        state: crate::DeviceState::Shutdown,
        physical: false,
        playstore: false,
        screen_width: None,
        screen_height: None,
        screen_scale: None,
        last_booted: None,
        runtime_id: Some(runtime.identifier.clone()),
        device_type_id: Some(device_type.identifier.clone()),
    };

    eprintln!("  Booting {name}...");
    boot_device(&device).await
}

async fn auto_create_android(
    want_phone: bool,
    os_version: Option<&crate::OsVersionSpec>,
    playstore: Option<bool>,
) -> Result<crate::DeviceInfo> {
    use crate::android::{
        discover_android_device_profiles, discover_android_system_images, pick_device_profile,
        pick_system_image,
    };

    let images = discover_android_system_images().await?;
    // Exact(N) → that API level; anything else → latest (preferred_api=0).
    let preferred_api = match os_version {
        Some(crate::OsVersionSpec::Exact { major, .. }) => *major,
        _ => 0,
    };
    let image = pick_system_image(&images, preferred_api, playstore).ok_or_else(|| {
        let requested = if preferred_api > 0 {
            format!("API {preferred_api}")
        } else {
            "any arm64 Android".to_string()
        };
        let store_hint = match playstore {
            Some(true) => " (playstore target required)",
            Some(false) => " (non-playstore target required)",
            None => "",
        };
        let installed: Vec<String> = images
            .iter()
            .map(|i| format!("API {} ({})", i.api_level, i.target))
            .collect();
        golem_events::coded(
            golem_events::FailureCode::HostToolchainMissing,
            anyhow::anyhow!(
                "Requested {requested}{store_hint} system image is not installed. \
                     Installed: {}. \
                     Add via: sdkmanager 'system-images;android-<N>;<target>;arm64-v8a'",
                installed.join(", ")
            ),
        )
    })?;

    let profiles = discover_android_device_profiles().await?;
    let profile = pick_device_profile(&profiles, want_phone).ok_or_else(|| {
        golem_events::coded(
            golem_events::FailureCode::HostToolchainMissing,
            anyhow::anyhow!(
                "No {} device profile found.",
                if want_phone { "phone" } else { "tablet" }
            ),
        )
    })?;

    let name = format!("golem-{}-api{}", profile.id, image.api_level);
    eprintln!(
        "  Creating Android emulator: {name} ({}, API {})",
        profile.name, image.api_level
    );

    create_simulator(Platform::Android, &name, &image.path, &profile.id).await?;

    let device = crate::DeviceInfo {
        name: name.clone(),
        udid: name.clone(), // Android uses AVD name as identifier
        platform: Platform::Android,
        device_type: if want_phone {
            crate::DeviceType::Phone
        } else {
            crate::DeviceType::Tablet
        },
        os_major: image.api_level,
        os_version: image.api_level.to_string(),
        state: crate::DeviceState::Shutdown,
        physical: false,
        playstore: image.target.contains("playstore"),
        screen_width: None,
        screen_height: None,
        screen_scale: None,
        last_booted: None,
        runtime_id: None,
        device_type_id: None,
    };

    eprintln!("  Booting {name}...");
    boot_device(&device).await
}
