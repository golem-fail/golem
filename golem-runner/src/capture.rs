use anyhow::Result;
use golem_driver::PlatformDriver;
use std::path::{Path, PathBuf};

/// Configuration for automatic screenshot and recording capture.
///
/// Paths are structured as `{output_dir}/{flow_name}/{device_name}/screenshots/`
/// and `{output_dir}/{flow_name}/{device_name}/recordings/`.
pub struct CaptureConfig {
    /// When true, automatically capture a screenshot on step failure or warning.
    pub screenshot_on_failure: bool,
    /// CLI override on the per-block effective record. `Some(true)` =
    /// `--record` (force on for every block, overriding explicit
    /// `[[block]] record = false`); `Some(false)` = `--no-record`
    /// (force off everywhere, beats `--record` if both passed);
    /// `None` = no CLI override, fall through to block / flow /
    /// project defaults.
    pub cli_force_record: Option<bool>,
    /// `golem.toml` `[options].record` — project-wide fallback for
    /// blocks where neither flow nor block sets a value.
    pub project_record: Option<bool>,
    /// When true, write results to disk. When false, skip all file output.
    pub write_to_disk: bool,
    /// Root output directory (default: `.golem/results`).
    pub output_dir: PathBuf,
    /// Flow name (sanitized for filesystem).
    pub flow_name: String,
    /// Device name (sanitized for filesystem).
    pub device_name: String,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            screenshot_on_failure: true,
            cli_force_record: None,
            project_record: None,
            write_to_disk: true,
            output_dir: PathBuf::from(".golem/results"),
            flow_name: String::new(),
            device_name: String::new(),
        }
    }
}

/// Sanitize a string so it is safe for use as a filename component.
///
/// Replaces any character that is not alphanumeric, `_`, or `-` with `_`.
pub fn sanitize_filename(name: &str) -> String {
    name.replace(
        |c: char| !c.is_alphanumeric() && c != '_' && c != '-',
        "_",
    )
}

/// Build the screenshot directory for this flow/device.
fn screenshot_dir(config: &CaptureConfig) -> PathBuf {
    config.output_dir
        .join(sanitize_filename(&config.flow_name))
        .join(sanitize_filename(&config.device_name))
        .join("screenshots")
}

/// Build the screenshot path.
///
/// Filename follows the `[global][block:iter][step]` output pattern:
/// `{global}_{block}_{iter}_{step}_{type}.png`
pub fn build_screenshot_path(
    config: &CaptureConfig,
    block_name: &str,
    global_step_index: u64,
    block_iteration: u32,
    step_index: usize,
    failure_type: &str,
) -> PathBuf {
    let filename = format!(
        "{}_{}_{}_{}_{}.png",
        global_step_index,
        sanitize_filename(block_name),
        block_iteration,
        step_index,
        failure_type,
    );
    screenshot_dir(config).join(filename)
}


/// Capture a screenshot on failure/warning and write it to disk.
///
/// Returns the path the screenshot was saved to, or an error if
/// capture is disabled.
pub async fn capture_failure_screenshot(
    driver: &dyn PlatformDriver,
    config: &CaptureConfig,
    block_name: &str,
    global_step_index: u64,
    block_iteration: u32,
    step_index: usize,
    failure_type: &str,
) -> Result<PathBuf> {
    if !config.screenshot_on_failure || !config.write_to_disk {
        anyhow::bail!("Screenshot capture is disabled");
    }

    let screenshot = driver.screenshot().await?;

    let path = build_screenshot_path(config, block_name, global_step_index, block_iteration, step_index, failure_type);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&path, &screenshot.data)?;
    Ok(path)
}

/// Build the path for the hierarchy-tree dump that accompanies a
/// failure screenshot. Same naming scheme + directory, `.json`
/// extension. Lives next to the PNG so a reader can pair them
/// without guessing.
pub fn build_tree_path(
    config: &CaptureConfig,
    block_name: &str,
    global_step_index: u64,
    block_iteration: u32,
    step_index: usize,
    failure_type: &str,
) -> PathBuf {
    let filename = format!(
        "{}_{}_{}_{}_{}_tree.json",
        global_step_index,
        sanitize_filename(block_name),
        block_iteration,
        step_index,
        failure_type,
    );
    screenshot_dir(config).join(filename)
}

/// Dump the accessibility tree alongside the failure screenshot.
///
/// Cheap to run (~30KB JSON, single hierarchy fetch) and dramatically
/// improves post-mortem signal for intermittents — the screenshot
/// shows what was on-screen, the tree shows whether golem could see
/// the expected element at all. Errors are non-fatal: failure-time
/// capture is best-effort.
pub async fn capture_failure_tree(
    driver: &dyn PlatformDriver,
    config: &CaptureConfig,
    block_name: &str,
    global_step_index: u64,
    block_iteration: u32,
    step_index: usize,
    failure_type: &str,
) -> Result<PathBuf> {
    if !config.screenshot_on_failure || !config.write_to_disk {
        anyhow::bail!("Tree dump is disabled");
    }

    let (root, _meta) = driver.get_hierarchy().await?;
    let json = serde_json::to_string_pretty(&root)?;

    let path = build_tree_path(config, block_name, global_step_index, block_iteration, step_index, failure_type);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&path, json)?;
    Ok(path)
}

/// Build the recording directory for this flow/device.
fn recording_dir(config: &CaptureConfig) -> PathBuf {
    config.output_dir
        .join(sanitize_filename(&config.flow_name))
        .join(sanitize_filename(&config.device_name))
        .join("recordings")
}

/// Build the recording path for one block + iteration.
///
/// Naming: `{block}_{iter}.mp4`. Loops produce one file per iteration
/// so timestamps line up with the per-block execution boundary.
//
// TODO: Android `screenrecord` truncates at ~3 min. Auto-rotate into
// `{block}_{iter}_part1.mp4`, `_part2.mp4` once duration crosses 2:55.
// Tracked in roadmap "Recording: per-block default with cascading config".
pub fn build_recording_path(
    config: &CaptureConfig,
    block_name: &str,
    iteration: u32,
) -> PathBuf {
    let filename = format!("{}_{}.mp4", sanitize_filename(block_name), iteration);
    recording_dir(config).join(filename)
}

/// Start screen recording for a single block on a device.
///
/// Caller is responsible for evaluating the cascade (CLI flag, project,
/// flow, block) and only invoking when the effective value is `true`.
/// Returns `Ok(())` without driver contact when `write_to_disk` is off.
pub async fn start_recording(
    driver: &dyn PlatformDriver,
    config: &CaptureConfig,
    block_name: &str,
    iteration: u32,
) -> Result<()> {
    if !config.write_to_disk {
        return Ok(());
    }
    let name = format!(
        "{}_{}_{}_{}",
        sanitize_filename(&config.flow_name),
        sanitize_filename(&config.device_name),
        sanitize_filename(block_name),
        iteration,
    );
    driver.start_recording(&name).await
}

/// Stop screen recording, copy the file into the per-block recording path.
pub async fn stop_recording(
    driver: &dyn PlatformDriver,
    config: &CaptureConfig,
    block_name: &str,
    iteration: u32,
) -> Result<PathBuf> {
    let source_path_str = driver.stop_recording().await?;
    if source_path_str.is_empty() {
        anyhow::bail!("driver returned empty recording path — recording was not active");
    }
    let source = Path::new(&source_path_str);

    let dest = build_recording_path(config, block_name, iteration);
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
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds: Bounds::new(0, 0, 375, 812),
            visible_bounds: None,
            children: Vec::new(),
        }
    }

    fn test_config() -> CaptureConfig {
        CaptureConfig {
            flow_name: "login_flow".to_string(),
            device_name: "iPhone_16e".to_string(),
            ..CaptureConfig::default()
        }
    }

    // ---------------------------------------------------------------
    // 1. build_screenshot_path generates correct structured path
    // ---------------------------------------------------------------
    #[test]
    fn build_screenshot_path_generates_correct_filename() {
        let config = test_config();
        let path = build_screenshot_path(&config, "verify_block", 3, 0, 1, "error");

        assert_eq!(
            path,
            PathBuf::from(".golem/results/login_flow/iPhone_16e/screenshots/3_verify_block_0_1_error.png")
        );
    }

    // ---------------------------------------------------------------
    // 2. build_screenshot_path sanitizes special characters
    // ---------------------------------------------------------------
    #[test]
    fn build_screenshot_path_sanitizes_special_characters() {
        let config = CaptureConfig {
            flow_name: "my flow!".to_string(),
            device_name: "iPhone 16 Pro".to_string(),
            ..CaptureConfig::default()
        };
        let path = build_screenshot_path(&config, "block #1", 5, 2, 0, "warn");

        assert_eq!(
            path,
            PathBuf::from(".golem/results/my_flow_/iPhone_16_Pro/screenshots/5_block__1_2_0_warn.png")
        );
    }

    // ---------------------------------------------------------------
    // 3. CaptureConfig defaults are correct
    // ---------------------------------------------------------------
    #[test]
    fn capture_config_defaults() {
        let config = CaptureConfig::default();
        assert!(config.screenshot_on_failure);
        assert!(config.write_to_disk);
        assert!(config.cli_force_record.is_none());
        assert!(config.project_record.is_none());
        assert_eq!(config.output_dir, PathBuf::from(".golem/results"));
    }

    // ---------------------------------------------------------------
    // 4. capture_failure_screenshot returns error when disabled
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn capture_failure_screenshot_disabled() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        let config = CaptureConfig {
            screenshot_on_failure: false,
            ..test_config()
        };

        let result =
            capture_failure_screenshot(&driver, &config, "block", 1, 0, 0, "error").await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be an error"));
        assert!(
            err_msg.contains("disabled"),
            "expected 'disabled' in error message, got: {err_msg}"
        );

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
    // 6. Screenshot path includes all components
    // ---------------------------------------------------------------
    #[test]
    fn screenshot_path_components() {
        let config = CaptureConfig {
            output_dir: PathBuf::from("/tmp/out"),
            flow_name: "checkout".to_string(),
            device_name: "Pixel-6".to_string(),
            ..CaptureConfig::default()
        };
        let path = build_screenshot_path(&config, "payment", 7, 1, 3, "warn");

        let filename = path
            .file_name()
            .expect("should have filename")
            .to_str()
            .expect("should be valid utf8");

        assert_eq!(filename, "7_payment_1_3_warn.png");
        assert_eq!(
            path.parent().expect("should have parent"),
            Path::new("/tmp/out/checkout/Pixel-6/screenshots")
        );
    }

    // ---------------------------------------------------------------
    // 7. Recording path uses structured directory + per-block naming
    // ---------------------------------------------------------------
    #[test]
    fn recording_path_components() {
        let config = CaptureConfig {
            output_dir: PathBuf::from("/tmp/out"),
            flow_name: "signup".to_string(),
            device_name: "Pixel-6".to_string(),
            ..CaptureConfig::default()
        };
        let path = build_recording_path(&config, "login", 0);

        assert_eq!(path, PathBuf::from("/tmp/out/signup/Pixel-6/recordings/login_0.mp4"));
    }

    // ---------------------------------------------------------------
    // 8. start_recording skips driver when write_to_disk=false
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn start_recording_noop_when_no_results() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        let config = CaptureConfig {
            write_to_disk: false,
            ..test_config()
        };

        let result = start_recording(&driver, &config, "login", 0).await;
        assert!(result.is_ok());
        assert!(
            driver.get_calls().is_empty(),
            "driver should not have been called"
        );
    }

    // ---------------------------------------------------------------
    // 9. start_recording calls driver with structured name
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn start_recording_calls_driver_with_block_name() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        let config = CaptureConfig {
            flow_name: "my-flow".to_string(),
            device_name: "iPhone14".to_string(),
            ..CaptureConfig::default()
        };

        let result = start_recording(&driver, &config, "login", 2).await;
        assert!(result.is_ok());

        let calls = driver.get_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "start_recording");
        assert_eq!(calls[0].1, vec!["my-flow_iPhone14_login_2"]);
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
            output_dir: tmp.path().to_path_buf(),
            flow_name: "login".to_string(),
            device_name: "test_device".to_string(),
            ..CaptureConfig::default()
        };

        let path =
            capture_failure_screenshot(&driver, &config, "auth", 2, 0, 1, "error")
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

    // ---------------------------------------------------------------
    // 11. write_to_disk=false skips capture
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn no_results_skips_capture() {
        let driver = MockPlatformDriver::new(default_hierarchy());
        let config = CaptureConfig {
            write_to_disk: false,
            ..test_config()
        };

        let result =
            capture_failure_screenshot(&driver, &config, "block", 1, 0, 0, "error").await;
        assert!(result.is_err());
        assert!(driver.get_calls().is_empty());
    }
}
