use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use golem_devices::{DeviceInfo, DeviceState, Platform};
use golem_driver::android::AndroidDriver;
use golem_driver::ios::IosDriver;
use golem_driver::PlatformDriver;
use golem_parser::{parse_flow, FlowFile, StringOrVec};
use golem_parser::mixin::expand_mixins;
use golem_report::{FlowReport, SuiteReport};
use golem_runner::capture::CaptureConfig;
use golem_runner::context::ExecutionContext;
use golem_runner::executor::execute_flow;
use golem_vars::VariableStore;

/// Configuration for a suite run.
#[derive(Default)]
pub struct SuiteConfig {
    /// Skip cleaning device state between flows.
    pub no_clean: bool,
    /// Skip teardown steps after each flow.
    pub no_teardown: bool,
    /// Keep device connections alive across flows.
    pub keep_devices: bool,
    /// Fixed random seed to use for all flows. When `None`, each flow
    /// gets an independent random seed.
    pub seed: Option<u64>,
}

/// Orchestrates the execution of a suite of test flows.
pub struct SuiteRunner {
    pub config: SuiteConfig,
}

impl SuiteRunner {
    pub fn new(config: SuiteConfig) -> Self {
        Self { config }
    }

    /// Run a suite of flow files and return aggregated results.
    ///
    /// Flows are executed sequentially. Each flow produces a [`FlowReport`]
    /// which is collected into the final [`SuiteReport`].
    pub async fn run_suite(&self, flow_paths: &[std::path::PathBuf]) -> Result<SuiteReport> {
        let start = Instant::now();
        let mut flow_reports = Vec::new();

        for path in flow_paths {
            let report = self.run_single_flow(path).await;
            flow_reports.push(report);
        }

        Ok(SuiteReport {
            flows: flow_reports,
            total_duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Run a single flow file and return its report.
    ///
    /// Steps:
    /// 1. Read the TOML flow file.
    /// 2. Parse it with [`parse_flow`].
    /// 3. Expand mixins on each block's steps.
    /// 4. TODO: merge config, connect to device, execute blocks.
    ///
    /// Sub-flow execution (via `run_flow` on a block) should also call
    /// [`expand_mixins`] after parsing the sub-flow.
    async fn run_single_flow(&self, path: &Path) -> FlowReport {
        let flow_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let start = Instant::now();

        let flow = match self.parse_and_expand(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("  Parse error: {e:#}");
                return FlowReport {
                    flow_name,
                    success: false,
                    step_results: Vec::new(),
                    warnings: vec![format!("Parse/mixin error: {e}")],
                    duration_ms: start.elapsed().as_millis() as u64,
                    seed: self.config.seed,
                    screenshot_path: None,
                    device_name: None,
                };
            }
        };

        // Detect target platform from the flow's device constraints.
        let platform = detect_platform(&flow);
        eprintln!("  Platform: {platform}");

        // Discover a device for the target platform.
        let device = match discover_device(platform).await {
            Ok(d) => d,
            Err(e) => {
                eprintln!("  Device discovery failed: {e:#}");
                return FlowReport {
                    flow_name,
                    success: false,
                    step_results: Vec::new(),
                    warnings: vec![format!("Device discovery failed: {e}")],
                    duration_ms: start.elapsed().as_millis() as u64,
                    seed: self.config.seed,
                    screenshot_path: None,
                    device_name: None,
                };
            }
        };

        let device_name = device.name.clone();

        // Extract bundle ID from the first app in the flow
        let bundle_id = flow
            .flow
            .apps
            .first()
            .map(|a| a.bundle.clone())
            .unwrap_or_else(|| "com.golem.test".to_string());

        // Create platform-appropriate driver
        let driver: Box<dyn PlatformDriver> = match platform {
            Platform::Ios => {
                Box::new(IosDriver::new(device.udid.clone(), bundle_id, 8222))
            }
            Platform::Android => {
                Box::new(AndroidDriver::new(device.udid.clone(), bundle_id, 8223))
            }
        };

        // Set up variable store
        let mut vars = VariableStore::new();

        // Build execution context
        let flow_dir = path.parent().unwrap_or(Path::new("."));
        let capture_config = CaptureConfig::default();
        let mut ctx = ExecutionContext {
            flow_dir,
            project_root: flow_dir,
            capture_config: &capture_config,
            flow_name: &flow_name,
            block_name: None,
            step_index: 0,
        };

        // Execute the flow
        eprintln!("  Executing on {} ({})", device_name, device.udid);
        match execute_flow(&flow, driver.as_ref(), &mut vars, None, 10_000, &mut ctx).await {
            Ok(result) => {
                if !result.success {
                    if let Some(ref block) = result.failed_block {
                        eprintln!("  Failed in block: {block}");
                    }
                    if let Some(step) = result.failed_step {
                        eprintln!("  Failed at step: {step}");
                    }
                }
                for w in &result.warnings {
                    eprintln!("  Warning: {w}");
                }
                FlowReport {
                    flow_name,
                    success: result.success,
                    step_results: Vec::new(),
                    warnings: result.warnings,
                    duration_ms: start.elapsed().as_millis() as u64,
                    seed: self.config.seed,
                    screenshot_path: None,
                    device_name: Some(device_name),
                }
            }
            Err(e) => {
                eprintln!("  Error: {e:#}");
                FlowReport {
                    flow_name,
                    success: false,
                    step_results: Vec::new(),
                    warnings: vec![format!("Execution error: {e}")],
                    duration_ms: start.elapsed().as_millis() as u64,
                    seed: self.config.seed,
                    screenshot_path: None,
                    device_name: Some(device_name),
                }
            }
        }
    }

    /// Read, parse, and expand mixins in a flow file.
    ///
    /// Returns the fully-expanded [`FlowFile`] ready for execution.
    fn parse_and_expand(&self, path: &Path) -> Result<FlowFile> {
        let content = std::fs::read_to_string(path)?;
        let mut flow = parse_flow(&content)?;

        let flow_dir = path.parent().unwrap_or(Path::new("."));
        // Use the flow directory as the project root when not explicitly configured.
        // TODO: discover the actual project root from config or directory traversal.
        let project_root = flow_dir;

        for block in &mut flow.block {
            block.steps = expand_mixins(&block.steps, flow_dir, project_root)?;
        }

        Ok(flow)
    }
}

/// Detect the target platform from the flow's device constraints.
/// Looks at the first app's first device constraint `os` field.
/// Defaults to iOS if not specified.
fn detect_platform(flow: &FlowFile) -> Platform {
    for app in &flow.flow.apps {
        for constraint in &app.devices {
            if let Some(ref os) = constraint.os {
                let os_str = match os {
                    StringOrVec::Single(s) => s.as_str(),
                    StringOrVec::Multiple(v) => v.first().map(|s| s.as_str()).unwrap_or(""),
                };
                if os_str.starts_with("android") {
                    return Platform::Android;
                }
                if os_str.starts_with("ios") {
                    return Platform::Ios;
                }
            }
        }
    }
    Platform::Ios
}

/// Discover a suitable device for the given platform.
async fn discover_device(platform: Platform) -> Result<DeviceInfo> {
    match platform {
        Platform::Ios => {
            let devices = golem_devices::ios::discover_ios_devices().await?;
            let booted_count = devices.iter().filter(|d| d.state == DeviceState::Booted).count();
            eprintln!("  Found {booted_count} booted iOS device(s)");
            devices
                .into_iter()
                .find(|d| d.state == DeviceState::Booted)
                .ok_or_else(|| anyhow::anyhow!("No booted iOS simulator found"))
        }
        Platform::Android => {
            // For Android, check `adb devices` for connected/running devices.
            let output = tokio::process::Command::new("adb")
                .args(["devices"])
                .output()
                .await?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut devices = Vec::new();
            for line in stdout.lines().skip(1) {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 2 && parts[1] == "device" {
                    devices.push(DeviceInfo {
                        name: parts[0].to_string(),
                        udid: parts[0].to_string(),
                        platform: Platform::Android,
                        device_type: golem_devices::DeviceType::Phone,
                        os_major: 0,
                        os_version: String::new(),
                        state: DeviceState::Booted,
                        physical: false,
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
            eprintln!("  Found {} connected Android device(s)", devices.len());
            devices
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("No connected Android device/emulator found"))
        }
    }
}

/// Summary statistics for a completed suite run.
pub struct SuiteStats {
    /// Total number of flows in the suite.
    pub total: usize,
    /// Number of flows that passed.
    pub passed: usize,
    /// Number of flows that failed.
    pub failed: usize,
}

/// Compute aggregate statistics from a [`SuiteReport`].
pub fn suite_stats(report: &SuiteReport) -> SuiteStats {
    SuiteStats {
        total: report.flows.len(),
        passed: report.flows.iter().filter(|f| f.success).count(),
        failed: report.flows.iter().filter(|f| !f.success).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Helper to create a passing FlowReport with the given name.
    fn passing_flow(name: &str) -> FlowReport {
        FlowReport {
            flow_name: name.to_string(),
            success: true,
            step_results: Vec::new(),
            warnings: Vec::new(),
            duration_ms: 0,
            seed: None,
            screenshot_path: None,
            device_name: None,
        }
    }

    /// Helper to create a failing FlowReport with the given name.
    fn failing_flow(name: &str) -> FlowReport {
        FlowReport {
            flow_name: name.to_string(),
            success: false,
            step_results: Vec::new(),
            warnings: Vec::new(),
            duration_ms: 0,
            seed: None,
            screenshot_path: None,
            device_name: None,
        }
    }

    // ---------------------------------------------------------------
    // 1. SuiteConfig defaults are correct
    // ---------------------------------------------------------------
    #[test]
    fn suite_config_defaults() {
        let config = SuiteConfig::default();
        assert!(!config.no_clean);
        assert!(!config.no_teardown);
        assert!(!config.keep_devices);
        assert!(config.seed.is_none());
    }

    // ---------------------------------------------------------------
    // 2. suite_stats counts passed flows
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_counts_passed() {
        let report = SuiteReport {
            flows: vec![passing_flow("a"), passing_flow("b"), passing_flow("c")],
            total_duration_ms: 100,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.passed, 3);
    }

    // ---------------------------------------------------------------
    // 3. suite_stats counts failed flows
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_counts_failed() {
        let report = SuiteReport {
            flows: vec![failing_flow("a"), failing_flow("b")],
            total_duration_ms: 100,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.failed, 2);
    }

    // ---------------------------------------------------------------
    // 4. suite_stats with mixed results
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_mixed_results() {
        let report = SuiteReport {
            flows: vec![
                passing_flow("a"),
                failing_flow("b"),
                passing_flow("c"),
                failing_flow("d"),
                passing_flow("e"),
            ],
            total_duration_ms: 500,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.total, 5);
        assert_eq!(stats.passed, 3);
        assert_eq!(stats.failed, 2);
    }

    // ---------------------------------------------------------------
    // 5. Empty suite produces empty report
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn empty_suite_produces_empty_report() {
        let runner = SuiteRunner::new(SuiteConfig::default());
        let report = runner.run_suite(&[]).await.expect("run_suite");
        assert!(report.flows.is_empty());
    }

    // ---------------------------------------------------------------
    // 6. run_suite returns correct number of flow reports
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn run_suite_returns_correct_count() {
        let runner = SuiteRunner::new(SuiteConfig::default());
        let paths = vec![
            PathBuf::from("login.test.toml"),
            PathBuf::from("checkout.test.toml"),
            PathBuf::from("signup.test.toml"),
        ];
        let report = runner.run_suite(&paths).await.expect("run_suite");
        assert_eq!(report.flows.len(), 3);
    }

    // ---------------------------------------------------------------
    // 7. Suite duration is tracked
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn suite_duration_is_tracked() {
        let runner = SuiteRunner::new(SuiteConfig::default());
        let paths = vec![PathBuf::from("a.test.toml")];
        let report = runner.run_suite(&paths).await.expect("run_suite");
        // Duration should be a non-negative value (stub flows are instant,
        // so total_duration_ms will be 0 or very small).
        // We just verify the field is populated and the report succeeds.
        assert!(report.total_duration_ms < 1000);
    }

    // ---------------------------------------------------------------
    // 8. Seed is propagated to flow reports
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn seed_propagated_to_flow_reports() {
        let config = SuiteConfig {
            seed: Some(42),
            ..SuiteConfig::default()
        };
        let runner = SuiteRunner::new(config);
        let paths = vec![
            PathBuf::from("a.test.toml"),
            PathBuf::from("b.test.toml"),
        ];
        let report = runner.run_suite(&paths).await.expect("run_suite");
        for flow in &report.flows {
            assert_eq!(flow.seed, Some(42));
        }
    }

    // ---------------------------------------------------------------
    // 9. SuiteStats with all passing
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_all_passing() {
        let report = SuiteReport {
            flows: vec![
                passing_flow("a"),
                passing_flow("b"),
                passing_flow("c"),
                passing_flow("d"),
            ],
            total_duration_ms: 200,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.total, 4);
        assert_eq!(stats.passed, 4);
        assert_eq!(stats.failed, 0);
    }

    // ---------------------------------------------------------------
    // 10. SuiteStats with all failing
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_all_failing() {
        let report = SuiteReport {
            flows: vec![
                failing_flow("a"),
                failing_flow("b"),
                failing_flow("c"),
            ],
            total_duration_ms: 300,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.total, 3);
        assert_eq!(stats.passed, 0);
        assert_eq!(stats.failed, 3);
    }

    // ---------------------------------------------------------------
    // 11. Flow names are extracted from file paths
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn flow_names_extracted_from_paths() {
        let runner = SuiteRunner::new(SuiteConfig::default());
        let paths = vec![
            PathBuf::from("flows/auth/login.test.toml"),
            PathBuf::from("checkout.test.toml"),
        ];
        let report = runner.run_suite(&paths).await.expect("run_suite");
        assert_eq!(report.flows[0].flow_name, "login.test");
        assert_eq!(report.flows[1].flow_name, "checkout.test");
    }

    // ---------------------------------------------------------------
    // 12. Empty suite stats
    // ---------------------------------------------------------------
    #[test]
    fn suite_stats_empty_suite() {
        let report = SuiteReport {
            flows: Vec::new(),
            total_duration_ms: 0,
        };
        let stats = suite_stats(&report);
        assert_eq!(stats.total, 0);
        assert_eq!(stats.passed, 0);
        assert_eq!(stats.failed, 0);
    }

    // ---------------------------------------------------------------
    // 13. parse_and_expand reads flow and expands mixins
    // ---------------------------------------------------------------
    #[test]
    fn parse_and_expand_reads_flow_file() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let flow_toml = r#"
[flow]
name = "basic flow"

[[block]]
name = "block1"
steps = [
  { action = "tap", text = "Hello" },
]
"#;
        let flow_path = tmp.path().join("basic.test.toml");
        std::fs::write(&flow_path, flow_toml).expect("write flow");

        let runner = SuiteRunner::new(SuiteConfig::default());
        let flow = runner
            .parse_and_expand(&flow_path)
            .expect("parse_and_expand SHALL succeed");

        assert_eq!(flow.flow.name, "basic flow");
        assert_eq!(flow.block.len(), 1);
        assert_eq!(flow.block[0].steps.len(), 1);
        assert_eq!(flow.block[0].steps[0].action, "tap");
    }

    // ---------------------------------------------------------------
    // 14. parse_and_expand expands load_mixin steps
    // ---------------------------------------------------------------
    #[test]
    fn parse_and_expand_expands_mixins() {
        let tmp = tempfile::tempdir().expect("temp dir");

        // Create mixin file
        let mixins_dir = tmp.path().join("__mixins__");
        std::fs::create_dir_all(&mixins_dir).expect("create mixins dir");
        std::fs::write(
            mixins_dir.join("login.toml"),
            r#"
[[step]]
action = "type"
id = "email_field"
text = "{{email}}"

[[step]]
action = "tap"
text = "Submit"
"#,
        )
        .expect("write mixin");

        // Create flow file referencing the mixin
        let flow_toml = r#"
[flow]
name = "mixin flow"

[[block]]
name = "login"
steps = [
  { action = "load_mixin", mixin = "login" },
  { action = "screenshot" },
]
"#;
        let flow_path = tmp.path().join("mixin_flow.test.toml");
        std::fs::write(&flow_path, flow_toml).expect("write flow");

        let runner = SuiteRunner::new(SuiteConfig::default());
        let flow = runner
            .parse_and_expand(&flow_path)
            .expect("parse_and_expand with mixins SHALL succeed");

        // The load_mixin step should be replaced by the mixin's 2 steps + the screenshot step
        assert_eq!(
            flow.block[0].steps.len(),
            3,
            "load_mixin SHALL be expanded to the mixin's steps"
        );
        assert_eq!(flow.block[0].steps[0].action, "type");
        assert_eq!(flow.block[0].steps[1].action, "tap");
        assert_eq!(flow.block[0].steps[2].action, "screenshot");
    }

    // ---------------------------------------------------------------
    // 15. run_single_flow fails gracefully for missing file
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn run_single_flow_fails_for_missing_file() {
        let runner = SuiteRunner::new(SuiteConfig::default());
        let path = PathBuf::from("nonexistent_flow.test.toml");
        let report = runner.run_single_flow(&path).await;

        assert!(!report.success, "report SHALL indicate failure for missing file");
        assert!(
            !report.warnings.is_empty(),
            "warnings SHALL contain the parse error"
        );
    }

    // ---------------------------------------------------------------
    // 16. run_single_flow fails gracefully for invalid TOML
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn run_single_flow_fails_for_invalid_toml() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let flow_path = tmp.path().join("bad.test.toml");
        std::fs::write(&flow_path, "this is not [[[valid toml").expect("write bad flow");

        let runner = SuiteRunner::new(SuiteConfig::default());
        let report = runner.run_single_flow(&flow_path).await;

        assert!(!report.success, "report SHALL indicate failure for invalid TOML");
        assert!(
            !report.warnings.is_empty(),
            "warnings SHALL contain the parse error"
        );
    }
}
