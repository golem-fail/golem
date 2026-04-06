//! Auto-cleanup logic that runs after a flow completes (including teardown).
//!
//! Resets device state (orientation, dark mode), stops recordings, and optionally
//! shuts down booted emulators/simulators. All errors are collected as warnings
//! rather than propagated — cleanup is best-effort.

use golem_devices::DeviceInfo;
use golem_driver::PlatformDriver;

/// Options controlling what cleanup actions to perform.
#[derive(Default)]
pub struct CleanupOptions {
    /// When `true`, skip device shutdown but still reset orientation, dark mode, etc.
    pub keep_devices: bool,
}

/// The result of a cleanup run — contains any non-fatal warnings.
pub struct CleanupResult {
    /// Warnings from cleanup steps that failed (best-effort, never fatal).
    pub warnings: Vec<String>,
}

/// Run auto-cleanup after flow completion.
///
/// Performs the following steps in order:
/// 1. Reset device orientation to portrait
/// 2. Reset dark mode to disabled
/// 3. Clear mocked location (reset to 0,0)
/// 4. Remove port forwards (Android only)
/// 5. Stop any running screen recordings
/// 6. Shut down the device (unless `options.keep_devices` is set)
///
/// Every step is best-effort: failures are collected into `CleanupResult::warnings`
/// and never propagated as errors.
pub async fn auto_cleanup(
    driver: &dyn PlatformDriver,
    device: &DeviceInfo,
    options: &CleanupOptions,
) -> CleanupResult {
    let mut warnings = Vec::new();

    // 1. Reset orientation to portrait
    if let Err(e) = driver.set_orientation("portrait").await {
        warnings.push(format!("Failed to reset orientation: {e}"));
    }

    // 2. Reset dark mode to disabled
    if let Err(e) = driver.set_dark_mode(false).await {
        warnings.push(format!("Failed to reset dark mode: {e}"));
    }

    // 3. Clear mocked location (reset to 0,0)
    if let Err(e) = driver.set_location(0.0, 0.0).await {
        warnings.push(format!("Failed to reset location: {e}"));
    }

    // 4. Remove port forwards (Android only)
    if device.platform == golem_devices::Platform::Android {
        if let Err(e) = driver.remove_port_forwards().await {
            warnings.push(format!("Failed to remove port forwards: {e}"));
        }
    }

    // 5. Stop recording if running (ignore the result path or error)
    if let Err(e) = driver.stop_recording().await {
        warnings.push(format!("Failed to stop recording: {e}"));
    }

    // 6. Shutdown device (unless keep_devices is set)
    if !options.keep_devices {
        if let Err(e) = golem_devices::lifecycle::shutdown_device(device).await {
            warnings.push(format!("Failed to shutdown device: {e}"));
        }
    }

    CleanupResult { warnings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use golem_devices::{DeviceState, DeviceType, Platform};
    use golem_driver::{Direction, PlatformDriver, ScreenshotResult};
    use golem_element::{Bounds, Element};
    use std::sync::Mutex;

    /// A mock driver that can be configured to fail specific methods.
    struct FailableMockDriver {
        calls: Mutex<Vec<String>>,
        fail_orientation: bool,
        fail_dark_mode: bool,
        fail_stop_recording: bool,
        fail_set_location: bool,
        fail_remove_port_forwards: bool,
    }

    impl FailableMockDriver {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_orientation: false,
                fail_dark_mode: false,
                fail_stop_recording: false,
                fail_set_location: false,
                fail_remove_port_forwards: false,
            }
        }

        fn get_calls(&self) -> Vec<String> {
            self.calls.lock().expect("lock poisoned").clone()
        }

        fn record(&self, method: &str) {
            self.calls
                .lock()
                .expect("lock poisoned")
                .push(method.to_string());
        }
    }

    #[async_trait]
    impl PlatformDriver for FailableMockDriver {
        async fn get_hierarchy(&self) -> anyhow::Result<Element> {
            Ok(Element {
                element_type: "View".into(),
                text: None,
                accessibility_label: None,
                placeholder: None,
                enabled: true,
                checked: false,
                clickable: false,
                focused: false,
                bounds: Bounds::new(0, 0, 375, 812),
                children: vec![],
            })
        }

        async fn tap(&self, _x: i32, _y: i32) -> anyhow::Result<()> {
            Ok(())
        }

        async fn long_press(&self, _x: i32, _y: i32, _duration_ms: u64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn backspace(&self, _count: u32) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe(&self, _direction: Direction) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe_coords(
            &self,
            _from_x: i32,
            _from_y: i32,
            _to_x: i32,
            _to_y: i32,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn screenshot(&self) -> anyhow::Result<ScreenshotResult> {
            Ok(ScreenshotResult {
                path: "mock.png".into(),
                data: vec![],
            })
        }

        async fn hide_keyboard(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn launch_app(&self, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn stop_app(&self, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn clear_app_data(&self, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn press_button(&self, _button: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn set_orientation(&self, orientation: &str) -> anyhow::Result<()> {
            self.record(&format!("set_orientation:{orientation}"));
            if self.fail_orientation {
                anyhow::bail!("orientation reset failed");
            }
            Ok(())
        }

        async fn set_dark_mode(&self, enabled: bool) -> anyhow::Result<()> {
            self.record(&format!("set_dark_mode:{enabled}"));
            if self.fail_dark_mode {
                anyhow::bail!("dark mode reset failed");
            }
            Ok(())
        }

        async fn set_location(&self, lat: f64, lon: f64) -> anyhow::Result<()> {
            self.record(&format!("set_location:{lat},{lon}"));
            if self.fail_set_location {
                anyhow::bail!("set location failed");
            }
            Ok(())
        }

        async fn open_url(&self, _url: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn push_notification(
            &self,
            _title: &str,
            _body: &str,
            _payload: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn add_media(&self, _path: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn grant_permission(
            &self,
            _bundle_id: &str,
            _permission: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn revoke_permission(
            &self,
            _bundle_id: &str,
            _permission: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn start_recording(&self, _name: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn stop_recording(&self) -> anyhow::Result<String> {
            self.record("stop_recording");
            if self.fail_stop_recording {
                anyhow::bail!("stop recording failed");
            }
            Ok("recording.mp4".into())
        }

        async fn get_alert(&self) -> anyhow::Result<Option<Element>> {
            Ok(None)
        }

        async fn dismiss_alert(&self, _button: Option<&str>) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove_port_forwards(&self) -> anyhow::Result<()> {
            self.record("remove_port_forwards");
            if self.fail_remove_port_forwards {
                anyhow::bail!("remove port forwards failed");
            }
            Ok(())
        }
    }

    fn test_device() -> DeviceInfo {
        DeviceInfo {
            name: "iPhone 15".into(),
            udid: "TEST-UDID-1234".into(),
            platform: Platform::Ios,
            device_type: DeviceType::Phone,
            os_major: 17,
            os_version: "17.2".into(),
            state: DeviceState::Booted,
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        }
    }

    // 1. auto_cleanup resets orientation
    #[tokio::test]
    async fn auto_cleanup_resets_orientation_to_portrait() {
        let driver = FailableMockDriver::new();
        let device = test_device();
        let options = CleanupOptions {
            keep_devices: true, // skip shutdown so we don't call real commands
        };

        let result = auto_cleanup(&driver, &device, &options).await;

        let calls = driver.get_calls();
        assert!(
            calls.contains(&"set_orientation:portrait".to_string()),
            "Expected orientation reset call, got: {calls:?}"
        );
        assert!(result.warnings.is_empty());
    }

    // 2. auto_cleanup resets dark mode
    #[tokio::test]
    async fn auto_cleanup_resets_dark_mode_to_disabled() {
        let driver = FailableMockDriver::new();
        let device = test_device();
        let options = CleanupOptions {
            keep_devices: true,
        };

        let result = auto_cleanup(&driver, &device, &options).await;

        let calls = driver.get_calls();
        assert!(
            calls.contains(&"set_dark_mode:false".to_string()),
            "Expected dark mode reset call, got: {calls:?}"
        );
        assert!(result.warnings.is_empty());
    }

    // 3. auto_cleanup stops recording
    #[tokio::test]
    async fn auto_cleanup_stops_recording() {
        let driver = FailableMockDriver::new();
        let device = test_device();
        let options = CleanupOptions {
            keep_devices: true,
        };

        let result = auto_cleanup(&driver, &device, &options).await;

        let calls = driver.get_calls();
        assert!(
            calls.contains(&"stop_recording".to_string()),
            "Expected stop_recording call, got: {calls:?}"
        );
        assert!(result.warnings.is_empty());
    }

    // 4. auto_cleanup shuts down device when keep_devices=false
    //    Note: shutdown_device calls a real command, so we verify indirectly
    //    by checking that it attempts the shutdown (which will fail in test
    //    because xcrun/adb is not available, producing a warning).
    #[tokio::test]
    async fn auto_cleanup_attempts_shutdown_when_keep_devices_false() {
        let driver = FailableMockDriver::new();
        let device = test_device();
        let options = CleanupOptions {
            keep_devices: false,
        };

        let result = auto_cleanup(&driver, &device, &options).await;

        // shutdown_device will fail in a test env (no real simulator),
        // so we expect a warning about it.
        let has_shutdown_warning = result
            .warnings
            .iter()
            .any(|w| w.contains("shutdown device") || w.contains("shutdown"));
        assert!(
            has_shutdown_warning,
            "Expected a shutdown warning when keep_devices=false, got: {:?}",
            result.warnings
        );
    }

    // 5. auto_cleanup skips shutdown when keep_devices=true
    #[tokio::test]
    async fn auto_cleanup_skips_shutdown_when_keep_devices_true() {
        let driver = FailableMockDriver::new();
        let device = test_device();
        let options = CleanupOptions {
            keep_devices: true,
        };

        let result = auto_cleanup(&driver, &device, &options).await;

        // No shutdown warning because we skipped it
        let has_shutdown_warning = result
            .warnings
            .iter()
            .any(|w| w.contains("shutdown"));
        assert!(
            !has_shutdown_warning,
            "Should not have shutdown warnings when keep_devices=true, got: {:?}",
            result.warnings
        );
    }

    // 6. Cleanup failure is collected as warning, not error
    #[tokio::test]
    async fn cleanup_failure_collected_as_warning_not_error() {
        let driver = FailableMockDriver {
            fail_orientation: true,
            ..FailableMockDriver::new()
        };
        let device = test_device();
        let options = CleanupOptions {
            keep_devices: true,
        };

        // auto_cleanup returns normally (not Err) even when orientation fails
        let result = auto_cleanup(&driver, &device, &options).await;

        assert_eq!(result.warnings.len(), 1);
        assert!(
            result.warnings[0].contains("Failed to reset orientation"),
            "Expected orientation warning, got: {}",
            result.warnings[0]
        );
    }

    // 7. Multiple cleanup failures all collected
    #[tokio::test]
    async fn multiple_cleanup_failures_all_collected() {
        let driver = FailableMockDriver {
            fail_orientation: true,
            fail_dark_mode: true,
            fail_stop_recording: true,
            ..FailableMockDriver::new()
        };
        let device = test_device();
        let options = CleanupOptions {
            keep_devices: true,
        };

        let result = auto_cleanup(&driver, &device, &options).await;

        assert_eq!(
            result.warnings.len(),
            3,
            "Expected 3 warnings, got: {:?}",
            result.warnings
        );
        assert!(result.warnings[0].contains("orientation"));
        assert!(result.warnings[1].contains("dark mode"));
        assert!(result.warnings[2].contains("recording"));
    }

    // 8. Default CleanupOptions has keep_devices=false
    #[test]
    fn default_cleanup_options_has_keep_devices_false() {
        let options = CleanupOptions::default();
        assert!(!options.keep_devices);
    }

    fn android_test_device() -> DeviceInfo {
        DeviceInfo {
            name: "Pixel 8".into(),
            udid: "emulator-5554".into(),
            platform: Platform::Android,
            device_type: DeviceType::Phone,
            os_major: 14,
            os_version: "14.0".into(),
            state: DeviceState::Booted,
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        }
    }

    // 9. auto_cleanup resets mocked location
    #[tokio::test]
    async fn auto_cleanup_resets_location() {
        let driver = FailableMockDriver::new();
        let device = test_device();
        let options = CleanupOptions {
            keep_devices: true,
        };

        let result = auto_cleanup(&driver, &device, &options).await;

        let calls = driver.get_calls();
        assert!(
            calls.contains(&"set_location:0,0".to_string()),
            "SHALL reset location to 0,0, got: {calls:?}"
        );
        assert!(result.warnings.is_empty());
    }

    // 10. auto_cleanup location reset failure collected as warning
    #[tokio::test]
    async fn auto_cleanup_location_reset_failure_is_warning() {
        let driver = FailableMockDriver {
            fail_set_location: true,
            ..FailableMockDriver::new()
        };
        let device = test_device();
        let options = CleanupOptions {
            keep_devices: true,
        };

        let result = auto_cleanup(&driver, &device, &options).await;

        assert!(
            result.warnings.iter().any(|w| w.contains("location")),
            "SHALL collect location reset failure as warning, got: {:?}",
            result.warnings
        );
    }

    // 11. auto_cleanup attempts port forward removal for Android devices
    #[tokio::test]
    async fn auto_cleanup_attempts_port_forward_removal_for_android() {
        let mut driver = FailableMockDriver::new();
        driver.fail_remove_port_forwards = true;
        let device = android_test_device();
        let options = CleanupOptions {
            keep_devices: true,
        };

        let result = auto_cleanup(&driver, &device, &options).await;

        // Port forward removal will fail in test env (no adb), but should
        // be attempted and failure collected as a warning.
        let has_port_forward_warning = result
            .warnings
            .iter()
            .any(|w| w.contains("port forward"));
        assert!(
            has_port_forward_warning,
            "SHALL attempt port forward removal for Android and collect failure as warning, got: {:?}",
            result.warnings
        );
    }

    // 12. auto_cleanup skips port forward removal for iOS devices
    #[tokio::test]
    async fn auto_cleanup_skips_port_forward_removal_for_ios() {
        let driver = FailableMockDriver::new();
        let device = test_device(); // iOS device
        let options = CleanupOptions {
            keep_devices: true,
        };

        let result = auto_cleanup(&driver, &device, &options).await;

        let has_port_forward_warning = result
            .warnings
            .iter()
            .any(|w| w.contains("port forward"));
        assert!(
            !has_port_forward_warning,
            "SHALL NOT attempt port forward removal for iOS, got: {:?}",
            result.warnings
        );
    }
}
