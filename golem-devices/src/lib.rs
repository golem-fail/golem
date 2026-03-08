// golem-devices: device management

pub mod android;
pub mod ios;
pub mod version;

use serde::{Deserialize, Serialize};
use std::fmt;

/// Target mobile platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Ios,
    Android,
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Platform::Ios => write!(f, "ios"),
            Platform::Android => write!(f, "android"),
        }
    }
}

/// Physical form factor of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    Phone,
    Tablet,
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceType::Phone => write!(f, "phone"),
            DeviceType::Tablet => write!(f, "tablet"),
        }
    }
}

/// Lifecycle state of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceState {
    Booted,
    Shutdown,
    Connected,
    NeedsCreation,
}

/// Full metadata for a discovered or configured device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub udid: String,
    pub platform: Platform,
    pub device_type: DeviceType,
    pub os_major: u32,
    pub os_version: String,
    pub state: DeviceState,
    pub physical: bool,
    pub playstore: bool,
    pub screen_width: Option<u32>,
    pub screen_height: Option<u32>,
    pub screen_scale: Option<f64>,
    pub last_booted: Option<String>,
    pub runtime_id: Option<String>,
    pub device_type_id: Option<String>,
}

/// Specification for which OS versions to target.
#[derive(Debug, Clone, PartialEq)]
pub enum OsVersionSpec {
    Exact { platform: Platform, major: u32 },
    Minimum { platform: Platform, major: u32 },
    Latest { platform: Platform, count: u32 },
}

/// A resolved device paired with its installed apps and creation status.
#[derive(Debug, Clone)]
pub struct ResolvedDevice {
    pub device: DeviceInfo,
    pub apps: Vec<String>,
    pub created: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_display_outputs_correct_strings() {
        assert_eq!(Platform::Ios.to_string(), "ios");
        assert_eq!(Platform::Android.to_string(), "android");
    }

    #[test]
    fn device_type_display_outputs_correct_strings() {
        assert_eq!(DeviceType::Phone.to_string(), "phone");
        assert_eq!(DeviceType::Tablet.to_string(), "tablet");
    }

    #[test]
    fn device_info_can_be_constructed_with_all_fields() {
        let info = DeviceInfo {
            name: "iPhone 15".to_string(),
            udid: "ABC-123".to_string(),
            platform: Platform::Ios,
            device_type: DeviceType::Phone,
            os_major: 17,
            os_version: "17.2".to_string(),
            state: DeviceState::Booted,
            physical: false,
            playstore: false,
            screen_width: Some(1179),
            screen_height: Some(2556),
            screen_scale: Some(3.0),
            last_booted: Some("2026-03-21T00:00:00Z".to_string()),
            runtime_id: Some("com.apple.CoreSimulator.SimRuntime.iOS-17-2".to_string()),
            device_type_id: Some("com.apple.CoreSimulator.SimDeviceType.iPhone-15".to_string()),
        };

        assert_eq!(info.name, "iPhone 15");
        assert_eq!(info.udid, "ABC-123");
        assert_eq!(info.platform, Platform::Ios);
        assert_eq!(info.device_type, DeviceType::Phone);
        assert_eq!(info.os_major, 17);
        assert_eq!(info.os_version, "17.2");
        assert_eq!(info.state, DeviceState::Booted);
        assert!(!info.physical);
        assert!(!info.playstore);
        assert_eq!(info.screen_width, Some(1179));
        assert_eq!(info.screen_height, Some(2556));
        assert_eq!(info.screen_scale, Some(3.0));
        assert!(info.last_booted.is_some());
        assert!(info.runtime_id.is_some());
        assert!(info.device_type_id.is_some());
    }

    #[test]
    fn os_version_spec_variants_hold_correct_data() {
        let exact = OsVersionSpec::Exact {
            platform: Platform::Ios,
            major: 17,
        };
        assert_eq!(
            exact,
            OsVersionSpec::Exact {
                platform: Platform::Ios,
                major: 17
            }
        );

        let minimum = OsVersionSpec::Minimum {
            platform: Platform::Android,
            major: 14,
        };
        assert_eq!(
            minimum,
            OsVersionSpec::Minimum {
                platform: Platform::Android,
                major: 14
            }
        );

        let latest = OsVersionSpec::Latest {
            platform: Platform::Ios,
            count: 3,
        };
        assert_eq!(
            latest,
            OsVersionSpec::Latest {
                platform: Platform::Ios,
                count: 3
            }
        );
    }

    #[test]
    fn device_state_equality_comparisons_work() {
        assert_eq!(DeviceState::Booted, DeviceState::Booted);
        assert_eq!(DeviceState::Shutdown, DeviceState::Shutdown);
        assert_eq!(DeviceState::Connected, DeviceState::Connected);
        assert_eq!(DeviceState::NeedsCreation, DeviceState::NeedsCreation);
        assert_ne!(DeviceState::Booted, DeviceState::Shutdown);
        assert_ne!(DeviceState::Connected, DeviceState::NeedsCreation);
    }

    #[test]
    fn platform_serialization_round_trip() {
        let ios = Platform::Ios;
        let json = serde_json::to_string(&ios).expect("serialize platform");
        let back: Platform = serde_json::from_str(&json).expect("deserialize platform");
        assert_eq!(ios, back);

        let android = Platform::Android;
        let json = serde_json::to_string(&android).expect("serialize platform");
        let back: Platform = serde_json::from_str(&json).expect("deserialize platform");
        assert_eq!(android, back);
    }

    #[test]
    fn device_info_serialization_round_trip() {
        let info = DeviceInfo {
            name: "Pixel 8".to_string(),
            udid: "emulator-5554".to_string(),
            platform: Platform::Android,
            device_type: DeviceType::Phone,
            os_major: 14,
            os_version: "14.0".to_string(),
            state: DeviceState::Connected,
            physical: true,
            playstore: true,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        };

        let json = serde_json::to_string(&info).expect("serialize device info");
        let back: DeviceInfo = serde_json::from_str(&json).expect("deserialize device info");
        assert_eq!(back.name, "Pixel 8");
        assert_eq!(back.udid, "emulator-5554");
        assert_eq!(back.platform, Platform::Android);
        assert_eq!(back.device_type, DeviceType::Phone);
        assert_eq!(back.os_major, 14);
        assert_eq!(back.os_version, "14.0");
        assert_eq!(back.state, DeviceState::Connected);
        assert!(back.physical);
        assert!(back.playstore);
        assert!(back.screen_width.is_none());
        assert!(back.screen_height.is_none());
        assert!(back.screen_scale.is_none());
        assert!(back.last_booted.is_none());
        assert!(back.runtime_id.is_none());
        assert!(back.device_type_id.is_none());
    }

    #[test]
    fn resolved_device_holds_device_and_app_list() {
        let device = DeviceInfo {
            name: "iPhone 15".to_string(),
            udid: "ABC-123".to_string(),
            platform: Platform::Ios,
            device_type: DeviceType::Phone,
            os_major: 17,
            os_version: "17.2".to_string(),
            state: DeviceState::Booted,
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        };

        let resolved = ResolvedDevice {
            device,
            apps: vec!["com.example.app".to_string(), "com.test.runner".to_string()],
            created: true,
        };

        assert_eq!(resolved.device.name, "iPhone 15");
        assert_eq!(resolved.apps.len(), 2);
        assert_eq!(resolved.apps[0], "com.example.app");
        assert_eq!(resolved.apps[1], "com.test.runner");
        assert!(resolved.created);
    }
}
