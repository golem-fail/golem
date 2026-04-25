use crate::{DeviceInfo, DeviceState, DeviceType, Platform};
use std::collections::HashMap;

/// Parse an AVD config.ini file contents into device metadata.
///
/// Reads key=value pairs from the config to extract screen dimensions,
/// density, architecture, API level, and Play Store availability.
/// State defaults to `Shutdown` (running state is detected separately).
pub fn parse_avd_config(avd_name: &str, config_contents: &str) -> anyhow::Result<DeviceInfo> {
    let props = parse_properties(config_contents);

    let width: u32 = props
        .get("hw.lcd.width")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let height: u32 = props
        .get("hw.lcd.height")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let density: u32 = props
        .get("hw.lcd.density")
        .and_then(|v| v.parse().ok())
        .unwrap_or(160);

    let device_type = classify_android_device_type(width, height, density);

    let tag_id = props.get("tag.id").map(String::as_str).unwrap_or("");
    let playstore = has_playstore(tag_id);

    let abi = props
        .get("abi.type")
        .cloned()
        .unwrap_or_default();

    let api_level = props
        .get("image.sysdir.1")
        .and_then(|p| extract_api_level(p))
        .unwrap_or(0);

    let display_name = props
        .get("avd.ini.displayname")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| avd_name.to_string());

    let os_version = format!("{api_level}");

    let screen_scale = f64::from(density) / 160.0;

    Ok(DeviceInfo {
        name: display_name,
        udid: avd_name.to_string(),
        platform: Platform::Android,
        device_type,
        os_major: api_level,
        os_version,
        state: DeviceState::Shutdown,
        physical: false,
        playstore,
        screen_width: if width > 0 { Some(width) } else { None },
        screen_height: if height > 0 { Some(height) } else { None },
        screen_scale: Some(screen_scale),
        last_booted: None,
        runtime_id: if abi.is_empty() { None } else { Some(abi) },
        device_type_id: None,
    })
}

/// Determine if an AVD is a tablet based on screen dimensions and density.
///
/// A device is classified as a tablet if its smallest screen dimension
/// is >= 600dp (the standard Android tablet threshold). The dp value is
/// computed as `pixels * 160 / density`.
fn classify_android_device_type(width: u32, height: u32, density: u32) -> DeviceType {
    if density == 0 {
        return DeviceType::Phone;
    }
    let min_px = width.min(height);
    let min_dp = min_px * 160 / density;
    if min_dp >= 600 {
        DeviceType::Tablet
    } else {
        DeviceType::Phone
    }
}

/// Check if an AVD has Google Play Store based on the tag.id value.
fn has_playstore(tag_id: &str) -> bool {
    tag_id.contains("playstore")
}

/// Extract API level from an image path or ABI path.
///
/// Looks for the pattern `android-NN` in the path and extracts the
/// numeric portion as the API level.
fn extract_api_level(image_path: &str) -> Option<u32> {
    for segment in image_path.split('/') {
        if let Some(rest) = segment.strip_prefix("android-") {
            if let Ok(level) = rest.parse::<u32>() {
                return Some(level);
            }
        }
    }
    None
}

/// Parse the output of `emulator -list-avds` to get AVD names.
///
/// Each non-empty line of output is treated as an AVD name.
pub fn parse_avd_list(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect()
}

/// Discover Android devices by listing AVDs and parsing their configs.
///
/// Runs `emulator -list-avds` to get names, then reads each AVD's
/// `config.ini` from `~/.android/avd/<name>.avd/config.ini`.
pub async fn discover_android_devices() -> anyhow::Result<Vec<DeviceInfo>> {
    // Get running devices from adb
    let running = discover_running_android_devices().await;

    // Get AVD definitions from emulator -list-avds + config files
    let mut devices = Vec::new();
    if let Ok(output) = tokio::process::Command::new("emulator")
        .arg("-list-avds")
        .output()
        .await
    {
        if output.status.success() {
            let stdout = String::from_utf8(output.stdout)?;
            let avd_names = parse_avd_list(&stdout);
            let home = std::env::var("HOME").unwrap_or_default();
            let avd_dir = std::path::PathBuf::from(&home).join(".android").join("avd");

            for name in &avd_names {
                let config_path = avd_dir.join(format!("{name}.avd")).join("config.ini");
                if let Ok(contents) = tokio::fs::read_to_string(&config_path).await {
                    if let Ok(mut device) = parse_avd_config(name, &contents) {
                        // Check if this AVD is running by matching against adb devices
                        if let Some(serial) = running.iter().find(|(_, avd)| avd.as_deref() == Some(name)) {
                            device.state = DeviceState::Booted;
                            device.udid = serial.0.clone();
                        }
                        devices.push(device);
                    }
                }
            }
        }
    }

    // Add any running devices not found via AVD configs (e.g. physical devices,
    // emulators with unparseable configs). Query `getprop ro.build.version.sdk`
    // over adb so os_major is correct (was 0 in the fallback path, which
    // leaked into displayed labels like `android/v0/phone`).
    for (serial, _) in &running {
        if !devices.iter().any(|d| d.udid == *serial) {
            let (os_major, os_version) = fetch_adb_version(serial).await;
            devices.push(DeviceInfo {
                name: serial.clone(),
                udid: serial.clone(),
                platform: Platform::Android,
                device_type: DeviceType::Phone,
                os_major,
                os_version,
                state: DeviceState::Booted,
                physical: !serial.starts_with("emulator-"),
                playstore: false,
                screen_width: None,
                screen_height: None,
                screen_scale: None,
                last_booted: None,
                runtime_id: None,
                device_type_id: None,
            });
        }
    }

    Ok(devices)
}

/// Query `adb shell getprop` for a running device's Android API level and
/// release string. Returns `(0, "")` if adb is unreachable — caller treats
/// 0 as "unknown version".
async fn fetch_adb_version(serial: &str) -> (u32, String) {
    let sdk = tokio::process::Command::new("adb")
        .args(["-s", serial, "shell", "getprop", "ro.build.version.sdk"])
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let release = tokio::process::Command::new("adb")
        .args(["-s", serial, "shell", "getprop", "ro.build.version.release"])
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let major = sdk.parse::<u32>().unwrap_or(0);
    (major, release)
}

/// Get running Android devices from `adb devices`, with optional AVD name.
/// Returns: Vec<(serial, Option<avd_name>)>
async fn discover_running_android_devices() -> Vec<(String, Option<String>)> {
    let output = match tokio::process::Command::new("adb")
        .args(["devices"])
        .output()
        .await
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut result = Vec::new();

    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1] == "device" {
            let serial = parts[0].to_string();
            // Try to get AVD name for emulators
            let avd_name = get_emulator_avd_name(&serial).await;
            result.push((serial, avd_name));
        }
    }
    result
}

/// Get the AVD name for a running emulator via `adb emu avd name`.
async fn get_emulator_avd_name(serial: &str) -> Option<String> {
    let output = tokio::process::Command::new("adb")
        .args(["-s", serial, "emu", "avd", "name"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .next()
        .filter(|s| !s.is_empty() && !s.starts_with("OK"))
        .map(|s| s.trim().to_string())
}

/// Parse key=value properties from config.ini content.
fn parse_properties(contents: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    map
}

// ---------------------------------------------------------------------------
// System image and device profile discovery
// ---------------------------------------------------------------------------

/// Information about an installed Android system image.
#[derive(Debug, Clone)]
pub struct SystemImageInfo {
    /// API level (e.g., 34)
    pub api_level: u32,
    /// ABI (e.g., "arm64-v8a", "x86_64")
    pub abi: String,
    /// Target variant (e.g., "google_apis", "google_apis_playstore", "default")
    pub target: String,
    /// Full path for avdmanager (e.g., "system-images;android-34;google_apis;arm64-v8a")
    pub path: String,
}

/// Information about an available Android device profile.
#[derive(Debug, Clone)]
pub struct DeviceProfileInfo {
    /// Profile ID for avdmanager (e.g., "pixel_9")
    pub id: String,
    /// Human-readable name (e.g., "Pixel 9")
    pub name: String,
    /// Whether this is a phone (true) or tablet/other (false)
    pub is_phone: bool,
}

/// Discover installed Android system images by scanning $ANDROID_HOME/system-images/.
pub async fn discover_android_system_images() -> anyhow::Result<Vec<SystemImageInfo>> {
    let android_home = std::env::var("ANDROID_HOME")
        .or_else(|_| std::env::var("ANDROID_SDK_ROOT"))
        .map_err(|_| anyhow::anyhow!("ANDROID_HOME not set"))?;

    let images_dir = std::path::PathBuf::from(&android_home).join("system-images");
    if !images_dir.exists() {
        return Ok(Vec::new());
    }

    let mut images = Vec::new();

    // Structure: system-images/android-{api}/{target}/{abi}/
    if let Ok(api_dirs) = std::fs::read_dir(&images_dir) {
        for api_entry in api_dirs.flatten() {
            let api_name = api_entry.file_name().to_string_lossy().to_string();
            let api_level: u32 = api_name
                .strip_prefix("android-")
                .and_then(|s| s.split('-').next()) // handle "android-34-ext8"
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            if api_level == 0 {
                continue;
            }

            if let Ok(target_dirs) = std::fs::read_dir(api_entry.path()) {
                for target_entry in target_dirs.flatten() {
                    let target = target_entry.file_name().to_string_lossy().to_string();

                    if let Ok(abi_dirs) = std::fs::read_dir(target_entry.path()) {
                        for abi_entry in abi_dirs.flatten() {
                            let abi = abi_entry.file_name().to_string_lossy().to_string();
                            images.push(SystemImageInfo {
                                api_level,
                                abi: abi.clone(),
                                target: target.clone(),
                                path: format!("system-images;{api_name};{target};{abi}"),
                            });
                        }
                    }
                }
            }
        }
    }

    // Sort by API level descending (latest first), prefer google_apis
    images.sort_by(|a, b| {
        b.api_level.cmp(&a.api_level)
            .then(a.target.cmp(&b.target)) // google_apis before google_apis_playstore
    });

    Ok(images)
}

/// Discover available Android device profiles via avdmanager.
pub async fn discover_android_device_profiles() -> anyhow::Result<Vec<DeviceProfileInfo>> {
    let android_home = std::env::var("ANDROID_HOME")
        .or_else(|_| std::env::var("ANDROID_SDK_ROOT"))
        .map_err(|_| anyhow::anyhow!("ANDROID_HOME not set"))?;

    let avdmanager = std::path::PathBuf::from(&android_home)
        .join("cmdline-tools/latest/bin/avdmanager");

    if !avdmanager.exists() {
        anyhow::bail!("avdmanager not found at {}", avdmanager.display());
    }

    let output = tokio::process::Command::new(&avdmanager)
        .args(["list", "device"])
        .output()
        .await?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut profiles = Vec::new();

    // Parse output: lines like 'id: 44 or "pixel_8"' followed by '    Name: Pixel 8'
    let mut current_id: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("id: ") {
            // Extract the quoted ID: '44 or "pixel_8"' → "pixel_8"
            if let Some(start) = rest.find('"') {
                if let Some(end) = rest[start + 1..].find('"') {
                    current_id = Some(rest[start + 1..start + 1 + end].to_string());
                }
            }
        } else if let Some(rest) = line.strip_prefix("    Name: ") {
            if let Some(id) = current_id.take() {
                let name = rest.trim().to_string();
                let is_phone = name.contains("Pixel") && !name.contains("Tablet")
                    && !name.contains("Fold") && !name.contains("C");
                profiles.push(DeviceProfileInfo { id, name, is_phone });
            }
        }
    }

    Ok(profiles)
}

/// Pick the best system image for the given API level (or latest if 0)
/// and optional Play Store preference.
///
/// - `preferred_api == 0` → any API level
/// - `want_playstore = Some(true)` → require `google_apis_playstore` target
/// - `want_playstore = Some(false)` → require `google_apis` target (excludes
///    playstore images — useful when flows need unrestricted system access)
/// - `want_playstore = None` → prefer `google_apis` (sorted first), fall
///   back to any matching target
///
/// Always requires `arm64-v8a` ABI.
pub fn pick_system_image(
    images: &[SystemImageInfo],
    preferred_api: u32,
    want_playstore: Option<bool>,
) -> Option<&SystemImageInfo> {
    images.iter().find(|img| {
        if img.abi != "arm64-v8a" {
            return false;
        }
        if preferred_api != 0 && img.api_level != preferred_api {
            return false;
        }
        match want_playstore {
            Some(true) => img.target.contains("playstore"),
            Some(false) => !img.target.contains("playstore"),
            None => true,
        }
    })
    // already sorted by api_level desc, google_apis first (non-playstore
    // wins when both are installed and want_playstore is None).
}

/// Pick the best device profile for a phone or tablet.
/// Prefers the latest Pixel model.
pub fn pick_device_profile(profiles: &[DeviceProfileInfo], want_phone: bool) -> Option<&DeviceProfileInfo> {
    profiles
        .iter()
        .rfind(|p| {
            if want_phone {
                p.is_phone
            } else {
                p.name.contains("Tablet")
            }
        })
    // rfind = latest model (avdmanager lists chronologically)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(api: u32, target: &str, abi: &str) -> SystemImageInfo {
        SystemImageInfo {
            api_level: api,
            abi: abi.into(),
            target: target.into(),
            path: format!("system-images/android-{api}/{target}/{abi}/"),
        }
    }

    // pick_system_image — playstore preference ───────────────────────

    #[test]
    fn pick_system_image_want_playstore_true_picks_playstore_target() {
        let images = vec![
            img(34, "google_apis", "arm64-v8a"),
            img(34, "google_apis_playstore", "arm64-v8a"),
        ];
        let pick = pick_system_image(&images, 34, Some(true))
            .expect("SHALL find playstore image");
        assert!(pick.target.contains("playstore"));
    }

    #[test]
    fn pick_system_image_want_playstore_false_excludes_playstore() {
        let images = vec![
            img(34, "google_apis_playstore", "arm64-v8a"),
            img(34, "google_apis", "arm64-v8a"),
        ];
        let pick = pick_system_image(&images, 34, Some(false))
            .expect("SHALL find non-playstore image");
        assert!(!pick.target.contains("playstore"));
    }

    #[test]
    fn pick_system_image_none_prefers_google_apis_first() {
        // Input order simulates sort: google_apis before google_apis_playstore.
        let images = vec![
            img(34, "google_apis", "arm64-v8a"),
            img(34, "google_apis_playstore", "arm64-v8a"),
        ];
        let pick = pick_system_image(&images, 34, None)
            .expect("SHALL find image");
        assert_eq!(pick.target, "google_apis",
            "None SHALL pick the first match (google_apis by sort order)");
    }

    #[test]
    fn pick_system_image_want_playstore_but_only_non_playstore_installed() {
        let images = vec![img(34, "google_apis", "arm64-v8a")];
        let pick = pick_system_image(&images, 34, Some(true));
        assert!(pick.is_none(),
            "SHALL return None when playstore requested but only non-playstore installed");
    }

    /// Helper: build a config.ini string from key-value pairs.
    fn make_config(pairs: &[(&str, &str)]) -> String {
        pairs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // 1. Parse a complete config.ini -> correct DeviceInfo with all fields
    #[test]
    fn parse_complete_config_produces_correct_device_info() {
        let config = make_config(&[
            ("hw.lcd.width", "1080"),
            ("hw.lcd.height", "2400"),
            ("hw.lcd.density", "440"),
            ("tag.id", "google_apis_playstore"),
            ("image.sysdir.1", "system-images/android-34/google_apis_playstore/x86_64/"),
            ("avd.ini.displayname", "Pixel 4 API 34"),
            ("abi.type", "x86_64"),
        ]);

        let device = parse_avd_config("Pixel_4_API_34", &config)
            .expect("should parse config");

        assert_eq!(device.name, "Pixel 4 API 34");
        assert_eq!(device.udid, "Pixel_4_API_34");
        assert_eq!(device.platform, Platform::Android);
        assert_eq!(device.device_type, DeviceType::Phone);
        assert_eq!(device.os_major, 34);
        assert_eq!(device.os_version, "34");
        assert_eq!(device.state, DeviceState::Shutdown);
        assert!(!device.physical);
        assert!(device.playstore);
        assert_eq!(device.screen_width, Some(1080));
        assert_eq!(device.screen_height, Some(2400));
        assert!(device.screen_scale.is_some());
        assert_eq!(device.runtime_id, Some("x86_64".to_string()));
    }

    // 2. Phone classification (1080x2400 at 440dpi -> phone)
    #[test]
    fn phone_classification_1080x2400_at_440dpi() {
        let device_type = classify_android_device_type(1080, 2400, 440);
        // min(1080, 2400) = 1080, dp = 1080 * 160 / 440 = 392 < 600
        assert_eq!(device_type, DeviceType::Phone);
    }

    // 3. Tablet classification (1600x2560 -> tablet)
    #[test]
    fn tablet_classification_1600x2560() {
        // Using 320 dpi: min(1600, 2560) = 1600, dp = 1600 * 160 / 320 = 800 >= 600
        let device_type = classify_android_device_type(1600, 2560, 320);
        assert_eq!(device_type, DeviceType::Tablet);
    }

    // 4. Playstore detection from tag.id
    #[test]
    fn playstore_detected_from_tag_id() {
        assert!(has_playstore("google_apis_playstore"));
    }

    // 5. No playstore when tag.id is "google_apis" (without _playstore)
    #[test]
    fn no_playstore_when_tag_id_is_google_apis() {
        assert!(!has_playstore("google_apis"));
    }

    // 6. API level extraction from image path
    #[test]
    fn api_level_extraction_from_image_path() {
        let level = extract_api_level(
            "system-images/android-34/google_apis_playstore/x86_64/",
        );
        assert_eq!(level, Some(34));

        let level = extract_api_level(
            "system-images/android-30/google_apis/arm64-v8a/",
        );
        assert_eq!(level, Some(30));

        // No android- prefix
        let level = extract_api_level("some/random/path/");
        assert_eq!(level, None);
    }

    // 7. AVD list parsing (multiple lines -> vec of names)
    #[test]
    fn avd_list_parsing_multiple_lines() {
        let output = "Pixel_4_API_34\nPixel_6_API_33\nNexus_5X_API_30\n";
        let names = parse_avd_list(output);
        assert_eq!(names, vec![
            "Pixel_4_API_34",
            "Pixel_6_API_33",
            "Nexus_5X_API_30",
        ]);
    }

    // 8. Missing display name falls back to avd_name
    #[test]
    fn missing_display_name_falls_back_to_avd_name() {
        let config = make_config(&[
            ("hw.lcd.width", "1080"),
            ("hw.lcd.height", "2400"),
            ("hw.lcd.density", "440"),
            ("image.sysdir.1", "system-images/android-34/google_apis/x86_64/"),
        ]);

        let device = parse_avd_config("My_Custom_AVD", &config)
            .expect("should parse config");

        assert_eq!(device.name, "My_Custom_AVD");
    }

    // 9. Minimal config.ini with only required fields
    #[test]
    fn minimal_config_with_defaults() {
        let config = "";
        let device = parse_avd_config("Minimal_AVD", config)
            .expect("should parse empty config");

        assert_eq!(device.name, "Minimal_AVD");
        assert_eq!(device.udid, "Minimal_AVD");
        assert_eq!(device.platform, Platform::Android);
        assert_eq!(device.device_type, DeviceType::Phone);
        assert_eq!(device.os_major, 0);
        assert_eq!(device.state, DeviceState::Shutdown);
        assert!(!device.physical);
        assert!(!device.playstore);
        assert_eq!(device.screen_width, None);
        assert_eq!(device.screen_height, None);
    }
}
