use anyhow::Result;
use golem_driver::PlatformDriver;
use std::path::{Path, PathBuf};

/// Configuration for automatic screenshot and recording capture.
pub struct CaptureConfig {
    /// When true, automatically capture a screenshot on step failure or warning.
    pub screenshot_on_failure: bool,
    /// Directory where failure/warning screenshots are saved.
    pub screenshot_dir: PathBuf,
    /// When true, automatically start/stop screen recording per flow.
    pub record: bool,
    /// Directory where recordings are saved.
    pub recording_dir: PathBuf,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            screenshot_on_failure: true,
            screenshot_dir: PathBuf::from(".golem/screenshots"),
            record: false,
            recording_dir: PathBuf::from(".golem/recordings"),
        }
    }
}

/// Sanitize a string so it is safe for use as a filename component.
///
/// Replaces any character that is not alphanumeric, `_`, or `-` with `_`.
fn sanitize_filename(name: &str) -> String {
    name.replace(
        |c: char| !c.is_alphanumeric() && c != '_' && c != '-',
        "_",
    )
}

/// Build the screenshot path without actually capturing it.
pub fn build_screenshot_path(
    config: &CaptureConfig,
    flow_name: &str,
    block_name: &str,
    step_index: usize,
    failure_type: &str,
) -> PathBuf {
    let filename = format!(
        "{}_{}_step{}_{}.png",
        sanitize_filename(flow_name),
        sanitize_filename(block_name),
        step_index,
        failure_type,
    );
    config.screenshot_dir.join(filename)
}

/// Capture a screenshot on failure/warning and write it to disk.
///
/// Returns the path the screenshot was saved to, or an error if
/// `screenshot_on_failure` is disabled or the capture/write fails.
pub async fn capture_failure_screenshot(
    driver: &dyn PlatformDriver,
    config: &CaptureConfig,
    flow_name: &str,
    block_name: &str,
    step_index: usize,
    failure_type: &str,
) -> Result<PathBuf> {
    if !config.screenshot_on_failure {
        anyhow::bail!("Screenshot on failure is disabled");
    }

    let screenshot = driver.screenshot().await?;

    let path = build_screenshot_path(config, flow_name, block_name, step_index, failure_type);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&path, &screenshot.data)?;
    Ok(path)
}

/// Build the recording path without actually stopping a recording.
pub fn build_recording_path(
    config: &CaptureConfig,
    flow_name: &str,
    device_name: &str,
) -> PathBuf {
    let filename = format!(
        "{}_{}.mp4",
        sanitize_filename(flow_name),
        sanitize_filename(device_name),
    );
    config.recording_dir.join(filename)
}

/// Start screen recording for a device.
///
/// Does nothing (returns `Ok(())`) when `config.record` is `false`.
pub async fn start_recording(
    driver: &dyn PlatformDriver,
    config: &CaptureConfig,
    flow_name: &str,
    device_name: &str,
) -> Result<()> {
    if !config.record {
        return Ok(());
    }
    let name = format!(
        "{}_{}",
        sanitize_filename(flow_name),
        sanitize_filename(device_name),
    );
    driver.start_recording(&name).await
}

/// Stop screen recording, copy the file to `recording_dir`, and return the
/// destination path.
///
/// The underlying driver's `stop_recording` returns a source path string.  We
/// copy the file from that source into the configured recording directory so
/// all recordings live in one predictable place.
pub async fn stop_recording(
    driver: &dyn PlatformDriver,
    config: &CaptureConfig,
    flow_name: &str,
    device_name: &str,
) -> Result<PathBuf> {
    let source_path_str = driver.stop_recording().await?;
    let source = Path::new(&source_path_str);

    let dest = build_recording_path(config, flow_name, device_name);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::copy(source, &dest)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::{Bounds, Element};

    fn default_hierarchy() -> Element {
        Element {
            element_type: "View".to_string(),
            text: None,
            accessibility_id: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds: Bounds::new(0, 0, 375, 812),
            children: Vec::new(),
        }
    }

    // ---------------------------------------------------------------
    // 1. build_screenshot_path generates correct filename
    // ---------------------------------------------------------------
    #[test]
    fn build_screenshot_path_generates_correct_filename() {
        let config = CaptureConfig::default();
        let path = build_screenshot_path(&config, "login_flow", "verify_block", 3, "error");

        assert_eq!(
            path,
            PathBuf::from(".golem/screenshots/login_flow_verify_block_step3_error.png")
        );
    }

    // ---------------------------------------------------------------
    // 2. build_screenshot_path sanitizes special characters
    // ---------------------------------------------------------------
    #[test]
    fn build_screenshot_path_sanitizes_special_characters() {
        let config = CaptureConfig::default();
        let path = build_screenshot_path(&config, "my flow!", "block #1", 0, "warn");

        assert_eq!(
            path,
            PathBuf::from(".golem/screenshots/my_flow__block__1_step0_warn.png")
        );
    }

    // ---------------------------------------------------------------
    // 3. CaptureConfig defaults are correct
    // ---------------------------------------------------------------
    #[test]
    fn capture_config_defaults() {
        let config = CaptureConfig::default();
        assert!(config.screenshot_on_failure);
        assert_eq!(config.screenshot_dir, PathBuf::from(".golem/screenshots"));
        assert!(!config.record);
        assert_eq!(config.recording_dir, PathBuf::from(".golem/recordings"));
    }

    // ---------------------------------------------------------------
    // 4. capture_failure_screenshot returns error when disabled
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn capture_failure_screenshot_disabled() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        let config = CaptureConfig {
            screenshot_on_failure: false,
            ..CaptureConfig::default()
        };

        let result =
            capture_failure_screenshot(&driver, &config, "flow", "block", 1, "error").await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be an error"));
        assert!(
            err_msg.contains("disabled"),
            "expected 'disabled' in error message, got: {err_msg}"
        );

        // Driver should NOT have been called
        assert!(driver.get_calls().is_empty());
    }

    // ---------------------------------------------------------------
    // 5. sanitize_filename handles special characters
    // ---------------------------------------------------------------
    #[test]
    fn sanitize_filename_handles_special_chars() {
        assert_eq!(sanitize_filename("hello world"), "hello_world");
        assert_eq!(sanitize_filename("test/flow.v2"), "test_flow_v2");
        assert_eq!(sanitize_filename("keep-dashes_and_underscores"), "keep-dashes_and_underscores");
        assert_eq!(sanitize_filename("emoji\u{1F600}name"), "emoji_name");
        assert_eq!(sanitize_filename(""), "");
    }

    // ---------------------------------------------------------------
    // 6. Screenshot path includes flow name, block name, step index, type
    // ---------------------------------------------------------------
    #[test]
    fn screenshot_path_components() {
        let config = CaptureConfig {
            screenshot_dir: PathBuf::from("/tmp/shots"),
            ..CaptureConfig::default()
        };
        let path = build_screenshot_path(&config, "checkout", "payment", 7, "warn");

        let filename = path
            .file_name()
            .expect("should have filename")
            .to_str()
            .expect("should be valid utf8");

        assert!(filename.contains("checkout"), "missing flow name");
        assert!(filename.contains("payment"), "missing block name");
        assert!(filename.contains("step7"), "missing step index");
        assert!(filename.contains("warn"), "missing failure type");
        assert!(filename.ends_with(".png"), "missing .png extension");
        assert_eq!(
            path.parent().expect("should have parent"),
            Path::new("/tmp/shots")
        );
    }

    // ---------------------------------------------------------------
    // 7. Recording path includes flow name and device name
    // ---------------------------------------------------------------
    #[test]
    fn recording_path_components() {
        let config = CaptureConfig {
            recording_dir: PathBuf::from("/tmp/recordings"),
            ..CaptureConfig::default()
        };
        let path = build_recording_path(&config, "signup", "Pixel-6");

        let filename = path
            .file_name()
            .expect("should have filename")
            .to_str()
            .expect("should be valid utf8");

        assert!(filename.contains("signup"), "missing flow name");
        assert!(filename.contains("Pixel-6"), "missing device name");
        assert!(filename.ends_with(".mp4"), "missing .mp4 extension");
        assert_eq!(
            path.parent().expect("should have parent"),
            Path::new("/tmp/recordings")
        );
    }

    // ---------------------------------------------------------------
    // 8. start_recording does nothing when record=false
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn start_recording_noop_when_disabled() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        let config = CaptureConfig {
            record: false,
            ..CaptureConfig::default()
        };

        let result = start_recording(&driver, &config, "flow", "device").await;
        assert!(result.is_ok());
        assert!(
            driver.get_calls().is_empty(),
            "driver should not have been called"
        );
    }

    // ---------------------------------------------------------------
    // 9. start_recording calls driver when record=true
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn start_recording_calls_driver_when_enabled() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        let config = CaptureConfig {
            record: true,
            ..CaptureConfig::default()
        };

        let result = start_recording(&driver, &config, "my-flow", "iPhone14").await;
        assert!(result.is_ok());

        let calls = driver.get_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "start_recording");
        assert_eq!(calls[0].1, vec!["my-flow_iPhone14"]);
    }

    // ---------------------------------------------------------------
    // 10. capture_failure_screenshot writes file to disk
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn capture_failure_screenshot_writes_file() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        let tmp = tempfile::tempdir().expect("failed to create tempdir");
        let config = CaptureConfig {
            screenshot_on_failure: true,
            screenshot_dir: tmp.path().to_path_buf(),
            ..CaptureConfig::default()
        };

        let path =
            capture_failure_screenshot(&driver, &config, "login", "auth", 2, "error")
                .await
                .expect("capture should succeed");

        assert!(path.exists(), "screenshot file SHALL exist on disk");
        let data = std::fs::read(&path).expect("should read file");
        // MockPlatformDriver returns PNG magic bytes
        assert_eq!(&data[..4], &[0x89, 0x50, 0x4E, 0x47]);

        let calls = driver.get_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "screenshot");
    }
}
