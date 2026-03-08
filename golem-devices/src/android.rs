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
    let output = tokio::process::Command::new("emulator")
        .arg("-list-avds")
        .output()
        .await?;

    if !output.status.success() {
        // emulator command not available or failed — return empty list
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8(output.stdout)?;
    let avd_names = parse_avd_list(&stdout);

    let home = std::env::var("HOME").unwrap_or_default();
    let avd_dir = std::path::PathBuf::from(&home).join(".android").join("avd");

    let mut devices = Vec::new();

    for name in &avd_names {
        let config_path = avd_dir.join(format!("{name}.avd")).join("config.ini");
        if let Ok(contents) = tokio::fs::read_to_string(&config_path).await {
            match parse_avd_config(name, &contents) {
                Ok(device) => devices.push(device),
                Err(_) => continue,
            }
        }
    }

    Ok(devices)
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

#[cfg(test)]
mod tests {
    use super::*;

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
