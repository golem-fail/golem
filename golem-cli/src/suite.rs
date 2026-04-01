use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
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
    /// Force a specific platform, overriding flow device constraints.
    pub platform: Option<Platform>,
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
    /// Flows are executed sequentially. Each flow may produce multiple
    /// [`FlowReport`]s when it targets more than one platform.
    pub async fn run_suite(&self, flow_paths: &[PathBuf]) -> Result<SuiteReport> {
        let start = Instant::now();
        let mut flow_reports = Vec::new();

        for path in flow_paths {
            let reports = self.run_single_flow(path).await;
            flow_reports.extend(reports);
        }

        Ok(SuiteReport {
            flows: flow_reports,
            total_duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Run a single flow file on all applicable platforms in parallel.
    ///
    /// Steps:
    /// 1. Parse the TOML flow file and expand mixins.
    /// 2. Detect target platforms from CLI override or flow device constraints.
    /// 3. Discover a device and ensure companion for each platform.
    /// 4. Spawn parallel `tokio::spawn` tasks — one per device.
    /// 5. Collect results into a `Vec<FlowReport>`.
    async fn run_single_flow(&self, path: &Path) -> Vec<FlowReport> {
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
                return vec![FlowReport {
                    flow_name,
                    success: false,
                    step_results: Vec::new(),
                    warnings: vec![format!("Parse/mixin error: {e}")],
                    duration_ms: start.elapsed().as_millis() as u64,
                    seed: self.config.seed,
                    screenshot_path: None,
                    device_name: None,
                }];
            }
        };

        // Detect target platforms from CLI override or flow's device constraints.
        let platforms = if let Some(p) = self.config.platform {
            vec![p]
        } else {
            detect_all_platforms(&flow)
        };

        // Discover devices and set up companions for each platform.
        let mut device_setups = Vec::new();
        for platform in &platforms {
            match discover_device(*platform).await {
                Ok(device) => {
                    eprintln!("  Platform: {platform}");
                    let port = match find_or_allocate_port(&device, *platform).await {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("  No available port for {platform}: {e:#}");
                            continue;
                        }
                    };
                    match ensure_companion(&device, *platform, port).await {
                        Ok(health) => {
                            eprintln!(
                                "  Companion: {} v{} on {} ({})",
                                health.platform, health.version, health.device_name, health.os_version
                            );
                            device_setups.push((device, *platform, port));
                        }
                        Err(e) => {
                            eprintln!("  Companion failed for {platform}: {e:#}");
                            // Don't fail the whole suite — skip this platform
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  No {platform} device available: {e:#}");
                    // Skip this platform
                }
            }
        }

        if device_setups.is_empty() {
            return vec![FlowReport {
                flow_name,
                success: false,
                step_results: Vec::new(),
                warnings: vec!["No devices available for any target platform".to_string()],
                duration_ms: start.elapsed().as_millis() as u64,
                seed: self.config.seed,
                screenshot_path: None,
                device_name: None,
            }];
        }

        // Spawn parallel execution tasks — one per device.
        let mut handles = Vec::new();
        for (device, platform, port) in device_setups {
            let flow = flow.clone();
            let flow_name = flow_name.clone();
            let flow_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let seed = self.config.seed;

            handles.push(tokio::spawn(async move {
                run_flow_on_device(flow, flow_name, flow_dir, device, platform, port, seed).await
            }));
        }

        // Collect results from all spawned tasks.
        let mut reports = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(report) => reports.push(report),
                Err(e) => {
                    reports.push(FlowReport {
                        flow_name: flow_name.clone(),
                        success: false,
                        step_results: Vec::new(),
                        warnings: vec![format!("Task panicked: {e}")],
                        duration_ms: start.elapsed().as_millis() as u64,
                        seed: self.config.seed,
                        screenshot_path: None,
                        device_name: None,
                    });
                }
            }
        }
        reports
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

/// Return the path to a companion lockfile for the given port.
fn companion_lockfile(port: u16) -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = std::path::PathBuf::from(home).join(".golem");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("companion-{port}.lock"))
}

/// Check if a process with the given PID is still alive.
fn is_pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Try to acquire an exclusive lock. Returns true if acquired, false if another
/// process holds it and is still alive.
fn try_acquire_lock(port: u16) -> bool {
    let path = companion_lockfile(port);

    // Check existing lock
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(pid) = content.trim().parse::<u32>() {
            if is_pid_alive(pid) {
                return false; // Another process is starting the companion
            }
        }
    }

    // Write our PID
    let _ = std::fs::write(&path, std::process::id().to_string());
    true
}

/// Release the companion lock.
fn release_lock(port: u16) {
    let _ = std::fs::remove_file(companion_lockfile(port));
}

/// Scan ports for running companion servers.
///
/// Checks ports in the companion range concurrently for a responding
/// /health endpoint. Returns a list of (port, health) for all found.
/// Fast — unused ports return "connection refused" instantly.
async fn scan_companions() -> Vec<(u16, golem_driver::CompanionHealth)> {
    use golem_devices::resource_manager::{PORT_RANGE_START, PORT_RANGE_END};
    use golem_driver::common::CompanionClient;

    let mut handles = Vec::new();
    for port in PORT_RANGE_START..=PORT_RANGE_END {
        handles.push(tokio::spawn(async move {
            let client = CompanionClient::new(port);
            match client.check_health().await {
                Ok(health) => Some((port, health)),
                Err(_) => None,
            }
        }));
    }

    let mut found = Vec::new();
    for handle in handles {
        if let Ok(Some(result)) = handle.await {
            found.push(result);
        }
    }
    found
}

/// Find a port for a device: reuse an existing matching companion or allocate a free port.
async fn find_or_allocate_port(device: &DeviceInfo, platform: Platform) -> Result<u16> {
    let golem_version = env!("CARGO_PKG_VERSION");
    let platform_str = match platform {
        Platform::Ios => "ios",
        Platform::Android => "android",
    };

    let companions = scan_companions().await;

    // Try to find an existing companion for this device with matching version
    for (port, health) in &companions {
        if health.platform == platform_str
            && health.version == golem_version
            && health.device_name == device.name
        {
            return Ok(*port);
        }
    }

    // No match — find first free port
    let used_ports: Vec<u16> = companions.iter().map(|(p, _)| *p).collect();
    use golem_devices::resource_manager::{PORT_RANGE_START, PORT_RANGE_END};
    for port in PORT_RANGE_START..=PORT_RANGE_END {
        if !used_ports.contains(&port) {
            return Ok(port);
        }
    }

    anyhow::bail!("No free companion ports in range {PORT_RANGE_START}-{PORT_RANGE_END}")
}

/// Ensure the companion server is running and version-compatible.
///
/// Three-tier startup:
/// 1. Health check passes → reuse existing companion
/// 2. Health check fails → try to restart the server process (no rebuild)
/// 3. Restart fails → full rebuild + install + start
async fn ensure_companion(
    device: &DeviceInfo,
    platform: Platform,
    port: u16,
) -> Result<golem_driver::CompanionHealth> {
    use golem_driver::common::CompanionClient;

    let client = CompanionClient::new(port);
    let golem_version = env!("CARGO_PKG_VERSION");

    // Tier 1: check if companion is already running
    if let Ok(health) = client.check_health().await {
        if health.version == golem_version {
            return Ok(health);
        }
        eprintln!(
            "  Companion version {} does not match golem {}. Rebuilding...",
            health.version, golem_version
        );
        // Fall through to tier 3 (rebuild)
    } else if try_acquire_lock(port) {
        // We hold the lock — start the companion ourselves.

        // Tier 2: try to restart without rebuilding
        eprintln!("  Companion not running. Restarting...");
        let companion_path = find_companion_path(platform)?;
        if let Err(e) = golem_devices::lifecycle::spawn_companion(device, &companion_path, port).await {
            eprintln!("  Restart failed: {e:#}");
        } else {
            // Android needs port forwarding
            if platform == Platform::Android {
                let fwd = golem_devices::lifecycle::port_forward_command(device, port);
                let _ = golem_devices::lifecycle::run_command_public(&fwd, "port forward").await;
            }
            if let Ok(health) = client.wait_for_health(std::time::Duration::from_secs(15)).await {
                release_lock(port);
                if health.version == golem_version {
                    return Ok(health);
                }
                eprintln!(
                    "  Restarted companion has wrong version {}. Rebuilding...",
                    health.version
                );
            }
        }

        // Tier 3: full rebuild + install + start
        eprintln!("  Building companion...");
        let companion_path = find_companion_path(platform)?;

        match platform {
            Platform::Ios => {
                golem_devices::lifecycle::build_companion(device, &companion_path).await?;
                golem_devices::lifecycle::spawn_companion(device, &companion_path, port).await?;
            }
            Platform::Android => {
                let apk_path = find_android_apk()?;
                golem_devices::lifecycle::install_android_companion(device, &apk_path, port).await?;
                golem_devices::lifecycle::spawn_companion(device, &companion_path, port).await?;
            }
        }

        let health = client
            .wait_for_health(std::time::Duration::from_secs(60))
            .await
            .context("Companion did not start within 60 seconds")?;

        release_lock(port);

        if health.version != golem_version {
            anyhow::bail!(
                "Companion version {} does not match golem version {}",
                health.version,
                golem_version
            );
        }

        return Ok(health);
    } else {
        // Another process is starting the companion — wait for it.
        eprintln!("  Another golem instance is starting the companion. Waiting...");
        let health = client
            .wait_for_health(std::time::Duration::from_secs(60))
            .await
            .context("Companion did not become available (started by another process)")?;

        if health.version != golem_version {
            anyhow::bail!(
                "Companion version {} does not match golem version {}",
                health.version,
                golem_version
            );
        }

        return Ok(health);
    }

    // Version mismatch from tier 1 falls through here — need tier 3 rebuild.
    // But we need the lock for rebuilding too.
    if try_acquire_lock(port) {
        eprintln!("  Building companion...");
        let companion_path = find_companion_path(platform)?;

        match platform {
            Platform::Ios => {
                golem_devices::lifecycle::build_companion(device, &companion_path).await?;
                golem_devices::lifecycle::spawn_companion(device, &companion_path, port).await?;
            }
            Platform::Android => {
                let apk_path = find_android_apk()?;
                golem_devices::lifecycle::install_android_companion(device, &apk_path, port).await?;
                golem_devices::lifecycle::spawn_companion(device, &companion_path, port).await?;
            }
        }

        let health = client
            .wait_for_health(std::time::Duration::from_secs(60))
            .await
            .context("Companion did not start within 60 seconds")?;

        release_lock(port);

        if health.version != golem_version {
            anyhow::bail!(
                "Companion version {} does not match golem version {}",
                health.version,
                golem_version
            );
        }

        Ok(health)
    } else {
        eprintln!("  Another golem instance is rebuilding the companion. Waiting...");
        let health = client
            .wait_for_health(std::time::Duration::from_secs(60))
            .await
            .context("Companion did not become available (rebuilt by another process)")?;

        if health.version != golem_version {
            anyhow::bail!(
                "Companion version {} does not match golem version {}",
                health.version,
                golem_version
            );
        }

        Ok(health)
    }
}

/// Find the companion project path for the given platform.
fn find_companion_path(platform: Platform) -> Result<String> {
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
        "Companion project not found at {}. Run golem from the project root.",
        relative
    )
}

/// Find the pre-built Android companion APK.
fn find_android_apk() -> Result<String> {
    let relative = "companions/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk";
    if std::path::Path::new(relative).exists() {
        return Ok(relative.to_string());
    }
    anyhow::bail!(
        "Android companion APK not found. Run: cd companions/android && ./gradlew assembleAndroidTest"
    )
}

/// Detect the target platform from the flow's device constraints.
/// Looks at the first app's first device constraint `os` field.
/// Defaults to iOS if not specified.
#[allow(dead_code)]
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

/// Detect ALL platforms referenced in the flow's device constraints.
/// Returns a deduplicated list. Defaults to `[Platform::Ios]` when no
/// constraints are specified.
fn detect_all_platforms(flow: &FlowFile) -> Vec<Platform> {
    let mut platforms = Vec::new();
    for app in &flow.flow.apps {
        for constraint in &app.devices {
            if let Some(ref os) = constraint.os {
                for os_str in os.to_vec() {
                    let p = if os_str.starts_with("android") {
                        Platform::Android
                    } else {
                        Platform::Ios
                    };
                    if !platforms.contains(&p) {
                        platforms.push(p);
                    }
                }
            }
        }
    }
    if platforms.is_empty() {
        platforms.push(Platform::Ios);
    }
    platforms
}

/// Execute a flow on a single device. This is a free function (not a method)
/// so it can be used with `tokio::spawn` which requires `'static` futures.
/// All parameters are owned values.
async fn run_flow_on_device(
    flow: FlowFile,
    flow_name: String,
    flow_dir: PathBuf,
    device: DeviceInfo,
    platform: Platform,
    port: u16,
    seed: Option<u64>,
) -> FlowReport {
    let start = Instant::now();
    let device_name = device.name.clone();
    let device_label = format!("{platform}/{device_name}");

    let bundle_id = flow
        .flow
        .apps
        .first()
        .map(|a| a.bundle.clone())
        .unwrap_or_else(|| "com.golem.test".to_string());

    let driver: Box<dyn PlatformDriver> = match platform {
        Platform::Ios => Box::new(IosDriver::new(device.udid.clone(), bundle_id, port)),
        Platform::Android => Box::new(AndroidDriver::new(device.udid.clone(), bundle_id, port)),
    };

    let mut vars = VariableStore::new();
    let capture_config = CaptureConfig::default();
    let mut ctx = ExecutionContext {
        flow_dir: &flow_dir,
        project_root: &flow_dir,
        capture_config: &capture_config,
        flow_name: &flow_name,
        block_name: None,
        step_index: 0,
        device: Some(&device),
    };

    eprintln!("  Executing on {device_label}");
    match execute_flow(&flow, driver.as_ref(), &mut vars, None, 10_000, &mut ctx).await {
        Ok(result) => {
            if !result.success {
                if let Some(ref block) = result.failed_block {
                    eprintln!("  [{device_label}] Failed in block: {block}");
                }
                if let Some(step) = result.failed_step {
                    eprintln!("  [{device_label}] Failed at step: {step}");
                }
            }
            for w in &result.warnings {
                eprintln!("  [{device_label}] Warning: {w}");
            }
            FlowReport {
                flow_name,
                success: result.success,
                step_results: Vec::new(),
                warnings: result.warnings,
                duration_ms: start.elapsed().as_millis() as u64,
                seed,
                screenshot_path: None,
                device_name: Some(device_label),
            }
        }
        Err(e) => {
            eprintln!("  [{device_label}] Error: {e:#}");
            FlowReport {
                flow_name,
                success: false,
                step_results: Vec::new(),
                warnings: vec![format!("Execution error: {e}")],
                duration_ms: start.elapsed().as_millis() as u64,
                seed,
                screenshot_path: None,
                device_name: Some(device_label),
            }
        }
    }
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
        let reports = runner.run_single_flow(&path).await;

        assert_eq!(reports.len(), 1, "missing file SHALL produce exactly one report");
        assert!(!reports[0].success, "report SHALL indicate failure for missing file");
        assert!(
            !reports[0].warnings.is_empty(),
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
        let reports = runner.run_single_flow(&flow_path).await;

        assert_eq!(reports.len(), 1, "invalid TOML SHALL produce exactly one report");
        assert!(!reports[0].success, "report SHALL indicate failure for invalid TOML");
        assert!(
            !reports[0].warnings.is_empty(),
            "warnings SHALL contain the parse error"
        );
    }
}
