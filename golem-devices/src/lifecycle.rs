//! Device lifecycle management: boot, shutdown, install, clear data, create.
//!
//! Each operation is split into a command-construction function (returns `Vec<String>`,
//! fully testable without real devices) and an async execution function that runs
//! the constructed command via `tokio::process::Command`.

use std::path::Path;

use crate::{DeviceInfo, Platform};
use anyhow::{bail, Context, Result};
use golem_events::CodeExt;

/// Find the .xctestrun file in a directory of extracted iOS companion products.
fn find_xctestrun(dir: &Path) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "xctestrun")
        })
        .map(|e| e.path().to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Command construction
// ---------------------------------------------------------------------------

/// Construct the command to boot a device/emulator.
pub fn boot_command(device: &DeviceInfo) -> Vec<String> {
    match device.platform {
        Platform::Ios => vec![
            "xcrun".into(),
            "simctl".into(),
            "boot".into(),
            device.udid.clone(),
        ],
        Platform::Android => vec![
            "emulator".into(),
            "-avd".into(),
            // `emulator -avd` expects the on-disk AVD identifier (no
            // spaces / parens). For shutdown AVDs, `udid` carries that
            // value; `name` is the human display name from
            // `avd.ini.displayname` and may contain spaces.
            device.udid.clone(),
            "-no-window".into(),
            "-no-audio".into(),
        ],
    }
}

/// Construct the command to shut down a device/emulator.
pub fn shutdown_command(device: &DeviceInfo) -> Vec<String> {
    match device.platform {
        Platform::Ios => vec![
            "xcrun".into(),
            "simctl".into(),
            "shutdown".into(),
            device.udid.clone(),
        ],
        Platform::Android => vec![
            "adb".into(),
            "-s".into(),
            device.udid.clone(),
            "emu".into(),
            "kill".into(),
        ],
    }
}

/// Construct the command to install an application on a device.
pub fn install_app_command(device: &DeviceInfo, app_path: &str) -> Vec<String> {
    match device.platform {
        Platform::Ios => vec![
            "xcrun".into(),
            "simctl".into(),
            "install".into(),
            device.udid.clone(),
            app_path.into(),
        ],
        Platform::Android => vec![
            "adb".into(),
            "-s".into(),
            device.udid.clone(),
            "install".into(),
            "-r".into(),
            app_path.into(),
        ],
    }
}

/// Construct the command to build the iOS companion for testing.
pub fn build_companion_command(device: &DeviceInfo, companion_path: &str) -> Vec<String> {
    match device.platform {
        Platform::Ios => {
            if companion_path.ends_with(".xcodeproj") {
                // Source-based: build from Xcode project
                vec![
                    "xcodebuild".into(),
                    "build-for-testing".into(),
                    "-project".into(),
                    companion_path.into(),
                    "-scheme".into(),
                    "GolemRunnerUITests".into(),
                    "-destination".into(),
                    format!("id={}", device.udid),
                ]
            } else {
                // Embedded: already built, nothing to do
                vec![]
            }
        }
        Platform::Android => vec![],  // Android uses pre-built APK
    }
}

/// Construct the command to start the companion server process.
///
/// This command blocks forever (the companion stays alive). It must be
/// spawned as a background process, not awaited.
/// Construct the command to start the companion server.
///
/// If `reg_port` is provided, the companion will register with golem's
/// registration server to get its port allocation. Otherwise falls back
/// to the legacy `port` parameter.
pub fn start_companion_command(device: &DeviceInfo, companion_path: &str, port: u16) -> Vec<String> {
    start_companion_command_with_reg(device, companion_path, port, None)
}

pub fn start_companion_command_with_reg(
    device: &DeviceInfo,
    companion_path: &str,
    port: u16,
    reg_port: Option<u16>,
) -> Vec<String> {
    match device.platform {
        Platform::Ios => {
            if companion_path.ends_with(".xcodeproj") {
                vec![
                    "xcodebuild".into(),
                    "test-without-building".into(),
                    "-project".into(),
                    companion_path.into(),
                    "-scheme".into(),
                    "GolemRunnerUITests".into(),
                    "-destination".into(),
                    format!("id={}", device.udid),
                    "-parallel-testing-enabled".into(),
                    "NO".into(),
                    "-only-testing:GolemRunnerUITests/GolemRunnerUITests/testCompanionServer".into(),
                ]
            } else {
                let dir = std::path::Path::new(companion_path);
                let xctestrun = find_xctestrun(dir).unwrap_or_default();
                vec![
                    "xcodebuild".into(),
                    "test-without-building".into(),
                    "-xctestrun".into(),
                    xctestrun,
                    "-destination".into(),
                    format!("id={}", device.udid),
                    "-parallel-testing-enabled".into(),
                    "NO".into(),
                    "-only-testing:GolemRunnerUITests/GolemRunnerUITests/testCompanionServer".into(),
                ]
            }
        }
        Platform::Android => {
            let mut args = vec![
                "adb".into(),
                "-s".into(),
                device.udid.clone(),
                "shell".into(),
                "am".into(),
                "instrument".into(),
                "-w".into(),
                "-e".into(),
                "device_serial".into(),
                device.udid.clone(),
            ];
            if let Some(rp) = reg_port {
                args.extend(["-e".into(), "reg_port".into(), rp.to_string()]);
            } else {
                args.extend(["-e".into(), "port".into(), port.to_string()]);
            }
            args.push("fail.golem.companion.test/androidx.test.runner.AndroidJUnitRunner".into());
            args
        }
    }
}

/// Set up ADB reverse so the Android emulator can reach the host's
/// registration server.
pub async fn setup_adb_reverse(device: &DeviceInfo, host_port: u16) -> Result<()> {
    let output = tokio::process::Command::new("adb")
        .args([
            "-s",
            &device.udid,
            "reverse",
            &format!("tcp:{host_port}"),
            &format!("tcp:{host_port}"),
        ])
        .output()
        .await
        .context("failed to set up ADB reverse")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(golem_events::coded(
            golem_events::FailureCode::DeviceDriverOpFailed,
            anyhow::anyhow!("ADB reverse failed: {stderr}"),
        ));
    }
    Ok(())
}

/// Construct the command to install the Android companion APK.
pub fn install_companion_command(device: &DeviceInfo, apk_path: &str) -> Vec<String> {
    vec![
        "adb".into(),
        "-s".into(),
        device.udid.clone(),
        "install".into(),
        "-r".into(),
        apk_path.into(),
    ]
}

/// Construct the command to set up Android port forwarding.
pub fn port_forward_command(device: &DeviceInfo, port: u16) -> Vec<String> {
    vec![
        "adb".into(),
        "-s".into(),
        device.udid.clone(),
        "forward".into(),
        format!("tcp:{port}"),
        format!("tcp:{port}"),
    ]
}

/// Construct the commands to clear application data on a device.
///
/// iOS: reset privacy settings and uninstall then reinstall the app.
/// Android: `pm clear <package>`.
///
/// Returns a `Vec` of commands.
pub fn clear_app_data_commands(
    device: &DeviceInfo,
    bundle_or_package: &str,
    app_path: Option<&str>,
) -> Vec<Vec<String>> {
    match device.platform {
        Platform::Ios => {
            let mut cmds = vec![
                vec![
                    "xcrun".into(),
                    "simctl".into(),
                    "privacy".into(),
                    device.udid.clone(),
                    "reset".into(),
                    "all".into(),
                ],
                vec![
                    "xcrun".into(),
                    "simctl".into(),
                    "uninstall".into(),
                    device.udid.clone(),
                    bundle_or_package.into(),
                ],
            ];
            if let Some(path) = app_path {
                cmds.push(vec![
                    "xcrun".into(),
                    "simctl".into(),
                    "install".into(),
                    device.udid.clone(),
                    path.into(),
                ]);
            }
            cmds
        }
        Platform::Android => vec![vec![
            "adb".into(),
            "-s".into(),
            device.udid.clone(),
            "shell".into(),
            "pm".into(),
            "clear".into(),
            bundle_or_package.into(),
        ]],
    }
}

/// Construct the command to create a new simulator or emulator.
pub fn create_device_command(
    platform: Platform,
    name: &str,
    type_or_image: &str,
    runtime_or_device: &str,
) -> Vec<String> {
    match platform {
        Platform::Ios => vec![
            "xcrun".into(),
            "simctl".into(),
            "create".into(),
            name.into(),
            type_or_image.into(),
            runtime_or_device.into(),
        ],
        Platform::Android => vec![
            "avdmanager".into(),
            "create".into(),
            "avd".into(),
            "-n".into(),
            name.into(),
            "-k".into(),
            type_or_image.into(),
            "-d".into(),
            runtime_or_device.into(),
        ],
    }
}

// ---------------------------------------------------------------------------
// Async execution helpers
// ---------------------------------------------------------------------------

/// Execute a single command described by `args` and return an error on non-zero exit.
pub async fn run_command_public(args: &[String], context: &str) -> Result<String> {
    run_command(args, context).await
}

/// Detect simctl's "already-booted" error so concurrent boot calls
/// against the same sim can treat the loser as success rather than
/// failing the slot. Apple's exact message:
/// `An error was encountered processing the command (domain=com.apple.CoreSimulator.SimError, code=405): Unable to boot device in current state: Booted`
fn is_already_booted_error(e: &anyhow::Error) -> bool {
    let s = format!("{e:#}");
    s.contains("Unable to boot device in current state: Booted")
        || s.contains("code=405")
}

async fn run_command(args: &[String], context: &str) -> Result<String> {
    let Some((program, arguments)) = args.split_first() else {
        bail!("{context}: empty command");
    };

    let output = tokio::process::Command::new(program)
        .args(arguments)
        .output()
        .await?;

    if !output.status.success() {
        bail!(
            "{context}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Execute a sequence of commands, stopping at the first failure.
async fn run_commands(commands: &[Vec<String>], context: &str) -> Result<()> {
    for cmd in commands {
        run_command(cmd, context).await?;
    }
    Ok(())
}

/// Boot a device (simulator or emulator) and **wait until it's ready**
/// for subsequent operations. Returns the post-boot [`DeviceInfo`].
///
/// The naive `simctl boot` / `emulator -avd` commands return as soon as the
/// boot is initiated — not when the OS is actually up. A subsequent
/// `simctl install` / `adb shell` then blocks internally until readiness,
/// making downstream operations look slow when the real cost is boot.
/// We chain a readiness gate per platform so the timing reported by
/// `boot_device` matches actual ready-for-use.
///
/// Per-platform gates:
/// - **iOS**: `xcrun simctl bootstatus <udid> -b` blocks until the sim
///   reports `Booted` with system services up. The returned [`DeviceInfo`]
///   keeps the same `udid` (sims are addressed by the static UDID).
/// - **Android**: spawn `emulator` detached (it runs forever), then
///   `adb wait-for-device` + poll `getprop sys.boot_completed` for `"1"`.
///   The returned [`DeviceInfo`] has its `udid` rewritten from the AVD
///   identifier (`Pixel_3a_3GB_API_34`) to the dynamic emulator serial
///   (`emulator-5554`) so subsequent `adb -s <udid>` calls work.
///   Timeout 180s — well above typical cold-boot times (~60-120s).
pub async fn boot_device(device: &DeviceInfo) -> Result<DeviceInfo> {
    match device.platform {
        Platform::Ios => boot_ios_and_wait(device).await,
        Platform::Android => boot_android_and_wait(device).await,
    }
}

async fn boot_ios_and_wait(device: &DeviceInfo) -> Result<DeviceInfo> {
    let args = boot_command(device);
    // Older simctl treated `boot` on an already-booted device as a no-op.
    // Recent versions (Xcode 26 / iOS 26) error out with
    // `Unable to boot device in current state: Booted` — which races
    // when multiple flow slots independently boot the same shared sim
    // (one wins, the rest see the error and tear down their slot).
    // Treat that specific error as success; bootstatus below confirms
    // the device is actually ready.
    if let Err(e) = run_command(&args, &format!("boot {}", device.name)).await {
        if !is_already_booted_error(&e) {
            return Err(e);
        }
    }
    // `bootstatus -b` blocks until the sim is fully booted (services up).
    let status = vec![
        "xcrun".into(),
        "simctl".into(),
        "bootstatus".into(),
        device.udid.clone(),
        "-b".into(),
    ];
    run_command(&status, &format!("bootstatus {}", device.name)).await?;
    Ok(DeviceInfo {
        state: crate::DeviceState::Booted,
        ..device.clone()
    })
}

async fn boot_android_and_wait(device: &DeviceInfo) -> Result<DeviceInfo> {
    // Snapshot serials *before* spawning so we can identify the new emulator
    // afterwards. Multiple emulators booting in parallel would otherwise
    // race for the same `emulator-NNNN` slot.
    let pre_serials = list_running_emulator_serials().await;

    // `emulator -avd` runs as long as the emulator does, so we spawn it
    // detached. Stdout/stderr are discarded; user-facing diagnostics come
    // from the readiness probes below.
    let args = boot_command(device);
    let Some((program, arguments)) = args.split_first() else {
        bail!("boot {}: empty command", device.name);
    };
    tokio::process::Command::new(program)
        .args(arguments)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("spawn emulator for {}", device.name))?;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(180);
    let avd_name = device.udid.clone();

    // Phase 1: find the new serial. Loop scanning `adb devices` for a
    // serial that wasn't there before AND reports our AVD name when
    // queried via `adb emu avd name`. Distinguishes "our boot" from any
    // unrelated emulator booting concurrently.
    let serial: String = 'find_serial: loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(golem_events::coded(
                golem_events::FailureCode::DeviceBootTimeout,
                anyhow::anyhow!(
                    "boot {}: timed out waiting for emulator to appear in adb devices",
                    device.name
                ),
            ));
        }
        let current = list_running_emulator_serials().await;
        for s in &current {
            if pre_serials.contains(s) {
                continue;
            }
            // New serial — query its AVD name. The console isn't always
            // responsive on first boot; if `emu avd name` errors we retry
            // on the next iteration.
            if let Some(name) = crate::android::get_emulator_avd_name(s).await {
                if name == avd_name {
                    break 'find_serial s.clone();
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    };

    // Phase 2: poll sys.boot_completed on the resolved serial.
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(golem_events::coded(
                golem_events::FailureCode::DeviceBootTimeout,
                anyhow::anyhow!(
                    "boot {}: timed out waiting for sys.boot_completed on {serial}",
                    device.name
                ),
            ));
        }
        let probe = vec![
            "adb".into(),
            "-s".into(),
            serial.clone(),
            "shell".into(),
            "getprop".into(),
            "sys.boot_completed".into(),
        ];
        if let Ok(out) = run_command(&probe, &format!("getprop boot_completed {serial}")).await {
            if out.trim() == "1" {
                return Ok(DeviceInfo {
                    udid: serial,
                    state: crate::DeviceState::Booted,
                    ..device.clone()
                });
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Snapshot every emulator serial currently visible to adb. Physical
/// devices are excluded — they don't show as `emulator-NNNN`.
async fn list_running_emulator_serials() -> Vec<String> {
    let out = match tokio::process::Command::new("adb")
        .args(["devices"])
        .output()
        .await
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .skip(1)
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == "device" && parts[0].starts_with("emulator-") {
                Some(parts[0].to_string())
            } else {
                None
            }
        })
        .collect()
}


/// Shut down a device (simulator or emulator).
pub async fn shutdown_device(device: &DeviceInfo) -> Result<()> {
    let args = shutdown_command(device);
    run_command(&args, &format!("shutdown {}", device.name)).await?;
    Ok(())
}

/// Install an application on a device.
pub async fn install_app(device: &DeviceInfo, app_path: &str) -> Result<()> {
    let args = install_app_command(device, app_path);
    run_command(&args, &format!("install app on {}", device.name)).await?;
    Ok(())
}

/// Start the companion server process in the background.
///
/// The server command blocks forever, so it is spawned as a detached process.
/// Returns once the process has been spawned (does NOT wait for /health).
/// Spawn companion with registration support.
/// If `reg_port` is provided, the companion registers with golem to get its port.
/// Otherwise uses the legacy port parameter.
pub async fn spawn_companion_with_reg(
    device: &DeviceInfo,
    companion_path: &str,
    port: u16,
    reg_port: Option<u16>,
) -> Result<()> {
    // For Android with registration: set up ADB reverse so companion can reach host
    if device.platform == Platform::Android {
        if let Some(rp) = reg_port {
            setup_adb_reverse(device, rp).await?;
        }
    }

    // For embedded iOS companion: inject env vars into xctestrun plist
    if device.platform == Platform::Ios && !companion_path.ends_with(".xcodeproj") {
        let dir = Path::new(companion_path);
        if let Some(xctestrun) = find_xctestrun(dir) {
            if let Some(rp) = reg_port {
                inject_env_into_xctestrun(&xctestrun, "GOLEM_REG_PORT", &rp.to_string())?;
            } else {
                inject_env_into_xctestrun(&xctestrun, "GOLEM_PORT", &port.to_string())?;
            }
        }
    }

    let args = start_companion_command_with_reg(device, companion_path, port, reg_port);
    let Some((program, arguments)) = args.split_first() else {
        bail!("empty companion start command");
    };
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(arguments)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    // iOS source-based: pass via env var
    if device.platform == Platform::Ios && companion_path.ends_with(".xcodeproj") {
        if let Some(rp) = reg_port {
            cmd.env("GOLEM_REG_PORT", rp.to_string());
        } else {
            cmd.env("GOLEM_PORT", port.to_string());
        }
    }

    cmd.spawn()
        .with_context(|| format!("failed to spawn companion on {} (port {port})", device.name))?;
    Ok(())
}

/// Legacy spawn without registration.
pub async fn spawn_companion(device: &DeviceInfo, companion_path: &str, port: u16) -> Result<()> {
    spawn_companion_with_reg(device, companion_path, port, None).await
}

/// Inject an environment variable into the xctestrun plist.
fn inject_env_into_xctestrun(xctestrun_path: &str, key: &str, value: &str) -> Result<()> {
    for section in [
        "TestConfigurations.0.TestTargets.0.EnvironmentVariables",
        "TestConfigurations.0.TestTargets.0.TestingEnvironmentVariables",
    ] {
        let full_key = format!("{section}.{key}");
        let _ = std::process::Command::new("plutil")
            .args(["-replace", &full_key, "-string", value, xctestrun_path])
            .output();
    }
    Ok(())
}

/// Build the companion (iOS only — Android uses pre-built APK).
pub async fn build_companion(device: &DeviceInfo, companion_path: &str) -> Result<()> {
    let args = build_companion_command(device, companion_path);
    if args.is_empty() {
        return Ok(()); // Android: nothing to build
    }
    run_command(&args, &format!("build companion for {}", device.name)).await?;
    Ok(())
}

/// Install the Android companion APKs and set up port forwarding.
///
/// Installs the main APK first (required for instrumentation), then the
/// test APK. `main_apk_path` is optional — if None, only the test APK
/// is installed (assumes the main APK is already on the device).
pub async fn install_android_companion(
    device: &DeviceInfo,
    apk_path: &str,
    port: u16,
) -> Result<()> {
    install_android_companion_with_main(device, apk_path, None, port).await
}

/// Install both Android companion APKs and set up port forwarding.
pub async fn install_android_companion_with_main(
    device: &DeviceInfo,
    test_apk_path: &str,
    main_apk_path: Option<&str>,
    port: u16,
) -> Result<()> {
    if let Some(main_path) = main_apk_path {
        let install_main = install_companion_command(device, main_path);
        run_command(&install_main, "install companion main APK").await?;
    }
    let install_test = install_companion_command(device, test_apk_path);
    run_command(&install_test, "install companion test APK").await?;
    let forward = port_forward_command(device, port);
    run_command(&forward, "set up port forwarding").await?;
    Ok(())
}

/// Clear application data on a device.
pub async fn clear_app_data(
    device: &DeviceInfo,
    bundle_or_package: &str,
    app_path: Option<&str>,
) -> Result<()> {
    let cmds = clear_app_data_commands(device, bundle_or_package, app_path);
    run_commands(&cmds, &format!("clear data on {}", device.name)).await
}

/// Create a new simulator or emulator.
pub async fn create_simulator(
    platform: Platform,
    name: &str,
    type_or_image: &str,
    runtime_or_device: &str,
) -> Result<String> {
    let args = create_device_command(platform, name, type_or_image, runtime_or_device);
    run_command(&args, &format!("create device {name}"))
        .await
        .code(golem_events::FailureCode::DeviceCreateFailed)
}

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
        Platform::Android => {
            auto_create_android(want_phone, os_version.as_ref(), playstore).await
        }
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
    let runtime = pick_runtime_for_spec(&runtimes, os_version)
        .ok_or_else(|| {
            let requested = match os_version {
                Some(crate::OsVersionSpec::Exact { major, .. }) => format!("iOS {major}"),
                _ => "any iOS".to_string(),
            };
            let installed: Vec<String> = runtimes.iter().map(|r| format!("iOS {}", r.major)).collect();
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
    let device_type = pick_device_type(&device_types, want_phone)
        .ok_or_else(|| golem_events::coded(
            golem_events::FailureCode::HostToolchainMissing,
            anyhow::anyhow!(
                "No {} device type found. Install Xcode device support.",
                if want_phone { "iPhone" } else { "iPad" }
            ),
        ))?;

    let name = format!("golem-{}-ios{}", device_type.name.replace(' ', "-"), runtime.major);
    eprintln!("  Creating iOS simulator: {name} ({}, {})", device_type.name, runtime.name);

    let output = create_simulator(
        Platform::Ios,
        &name,
        &device_type.identifier,
        &runtime.identifier,
    ).await?;

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
        device_type: if want_phone { crate::DeviceType::Phone } else { crate::DeviceType::Tablet },
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
    use crate::android::{discover_android_device_profiles, discover_android_system_images,
                          pick_device_profile, pick_system_image};

    let images = discover_android_system_images().await?;
    // Exact(N) → that API level; anything else → latest (preferred_api=0).
    let preferred_api = match os_version {
        Some(crate::OsVersionSpec::Exact { major, .. }) => *major,
        _ => 0,
    };
    let image = pick_system_image(&images, preferred_api, playstore)
        .ok_or_else(|| {
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
    let profile = pick_device_profile(&profiles, want_phone)
        .ok_or_else(|| golem_events::coded(
            golem_events::FailureCode::HostToolchainMissing,
            anyhow::anyhow!(
                "No {} device profile found.",
                if want_phone { "phone" } else { "tablet" }
            ),
        ))?;

    let name = format!("golem-{}-api{}", profile.id, image.api_level);
    eprintln!("  Creating Android emulator: {name} ({}, API {})", profile.name, image.api_level);

    create_simulator(
        Platform::Android,
        &name,
        &image.path,
        &profile.id,
    ).await?;

    let device = crate::DeviceInfo {
        name: name.clone(),
        udid: name.clone(), // Android uses AVD name as identifier
        platform: Platform::Android,
        device_type: if want_phone { crate::DeviceType::Phone } else { crate::DeviceType::Tablet },
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

// ---------------------------------------------------------------------------
// Tests — command construction only (no real devices needed)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceState, DeviceType};

    fn ios_device() -> DeviceInfo {
        DeviceInfo {
            name: "iPhone 15".into(),
            udid: "AAAA-BBBB-CCCC".into(),
            platform: Platform::Ios,
            device_type: DeviceType::Phone,
            os_major: 17,
            os_version: "17.2".into(),
            state: DeviceState::Shutdown,
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: Some("com.apple.CoreSimulator.SimRuntime.iOS-17-2".into()),
            device_type_id: Some(
                "com.apple.CoreSimulator.SimDeviceType.iPhone-15".into(),
            ),
        }
    }

    /// Booted Android emulator fixture — `udid` is the dynamic
    /// `emulator-NNNN` serial used in `adb -s ...` commands. Used by
    /// install / port-forward / instrument tests.
    fn android_device() -> DeviceInfo {
        DeviceInfo {
            name: "Pixel_8_API_34".into(),
            udid: "emulator-5554".into(),
            platform: Platform::Android,
            device_type: DeviceType::Phone,
            os_major: 14,
            os_version: "14.0".into(),
            state: DeviceState::Shutdown,
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

    // 1. iOS boot
    #[test]
    fn ios_boot_command_is_correct() {
        let d = ios_device();
        let cmd = boot_command(&d);
        assert_eq!(cmd, vec!["xcrun", "simctl", "boot", "AAAA-BBBB-CCCC"]);
    }

    // 2. Android boot — uses the AVD identifier (in `udid` for shutdown
    //    AVDs, per `parse_avd_config`), not the display name.
    #[test]
    fn android_boot_command_is_correct() {
        let d = DeviceInfo {
            name: "Pixel 8 API 34".into(),       // display name with spaces
            udid: "Pixel_8_API_34".into(),       // on-disk AVD identifier
            state: DeviceState::Shutdown,
            ..android_device()
        };
        let cmd = boot_command(&d);
        assert_eq!(
            cmd,
            vec!["emulator", "-avd", "Pixel_8_API_34", "-no-window", "-no-audio"]
        );
    }

    // 3. iOS shutdown
    #[test]
    fn ios_shutdown_command_is_correct() {
        let d = ios_device();
        let cmd = shutdown_command(&d);
        assert_eq!(cmd, vec!["xcrun", "simctl", "shutdown", "AAAA-BBBB-CCCC"]);
    }

    // 4. Android shutdown
    #[test]
    fn android_shutdown_command_is_correct() {
        let d = android_device();
        let cmd = shutdown_command(&d);
        assert_eq!(
            cmd,
            vec!["adb", "-s", "emulator-5554", "emu", "kill"]
        );
    }

    // 5. iOS install app
    #[test]
    fn ios_install_app_command_is_correct() {
        let d = ios_device();
        let cmd = install_app_command(&d, "/path/to/MyApp.app");
        assert_eq!(
            cmd,
            vec![
                "xcrun",
                "simctl",
                "install",
                "AAAA-BBBB-CCCC",
                "/path/to/MyApp.app"
            ]
        );
    }

    // 6. Android install app
    #[test]
    fn android_install_app_command_is_correct() {
        let d = android_device();
        let cmd = install_app_command(&d, "/path/to/app.apk");
        assert_eq!(
            cmd,
            vec![
                "adb",
                "-s",
                "emulator-5554",
                "install",
                "-r",
                "/path/to/app.apk"
            ]
        );
    }

    // 7. iOS create simulator
    #[test]
    fn ios_create_device_command_is_correct() {
        let cmd = create_device_command(
            Platform::Ios,
            "TestSim",
            "com.apple.CoreSimulator.SimDeviceType.iPhone-15",
            "com.apple.CoreSimulator.SimRuntime.iOS-17-2",
        );
        assert_eq!(
            cmd,
            vec![
                "xcrun",
                "simctl",
                "create",
                "TestSim",
                "com.apple.CoreSimulator.SimDeviceType.iPhone-15",
                "com.apple.CoreSimulator.SimRuntime.iOS-17-2",
            ]
        );
    }

    // 8. Android create emulator
    #[test]
    fn android_create_device_command_is_correct() {
        let cmd = create_device_command(
            Platform::Android,
            "Pixel_8_API_34",
            "system-images;android-34;google_apis;arm64-v8a",
            "pixel_8",
        );
        assert_eq!(
            cmd,
            vec![
                "avdmanager",
                "create",
                "avd",
                "-n",
                "Pixel_8_API_34",
                "-k",
                "system-images;android-34;google_apis;arm64-v8a",
                "-d",
                "pixel_8",
            ]
        );
    }

    // 9. iOS clear data
    #[test]
    fn ios_clear_app_data_commands_are_correct() {
        let d = ios_device();
        let cmds =
            clear_app_data_commands(&d, "com.example.MyApp", Some("/path/to/MyApp.app"));
        assert_eq!(cmds.len(), 3);
        assert_eq!(
            cmds[0],
            vec!["xcrun", "simctl", "privacy", "AAAA-BBBB-CCCC", "reset", "all"]
        );
        assert_eq!(
            cmds[1],
            vec!["xcrun", "simctl", "uninstall", "AAAA-BBBB-CCCC", "com.example.MyApp"]
        );
        assert_eq!(
            cmds[2],
            vec![
                "xcrun",
                "simctl",
                "install",
                "AAAA-BBBB-CCCC",
                "/path/to/MyApp.app"
            ]
        );
    }

    // 10. Android clear data
    #[test]
    fn android_clear_app_data_commands_are_correct() {
        let d = android_device();
        let cmds = clear_app_data_commands(&d, "com.example.myapp", None);
        assert_eq!(cmds.len(), 1);
        assert_eq!(
            cmds[0],
            vec![
                "adb",
                "-s",
                "emulator-5554",
                "shell",
                "pm",
                "clear",
                "com.example.myapp"
            ]
        );
    }

    // 11. iOS build companion command
    #[test]
    fn ios_build_companion_command_is_correct() {
        let d = ios_device();
        let cmd = build_companion_command(&d, "/path/to/Companion.xcodeproj");
        assert_eq!(
            cmd,
            vec![
                "xcodebuild",
                "build-for-testing",
                "-project",
                "/path/to/Companion.xcodeproj",
                "-scheme",
                "GolemRunnerUITests",
                "-destination",
                "id=AAAA-BBBB-CCCC",
            ]
        );
    }

    // 12. iOS start companion command includes no-clone flags
    #[test]
    fn ios_start_companion_command_is_correct() {
        let d = ios_device();
        let cmd = start_companion_command(&d, "/path/to/Companion.xcodeproj", 8222);
        assert_eq!(
            cmd,
            vec![
                "xcodebuild",
                "test-without-building",
                "-project",
                "/path/to/Companion.xcodeproj",
                "-scheme",
                "GolemRunnerUITests",
                "-destination",
                "id=AAAA-BBBB-CCCC",
                "-parallel-testing-enabled",
                "NO",
                "-only-testing:GolemRunnerUITests/GolemRunnerUITests/testCompanionServer",
            ]
        );
    }

    // 13. Android start companion command includes port arg
    #[test]
    fn android_start_companion_command_is_correct() {
        let d = android_device();
        let cmd = start_companion_command(&d, "/path/to/companion.apk", 8225);
        assert_eq!(
            cmd,
            vec![
                "adb",
                "-s",
                "emulator-5554",
                "shell",
                "am",
                "instrument",
                "-w",
                "-e",
                "device_serial",
                "emulator-5554",
                "-e",
                "port",
                "8225",
                "fail.golem.companion.test/androidx.test.runner.AndroidJUnitRunner",
            ]
        );
    }

    // 14. Android install companion command
    #[test]
    fn android_install_companion_command_is_correct() {
        let d = android_device();
        let cmd = install_companion_command(&d, "/path/to/companion.apk");
        assert_eq!(
            cmd,
            vec![
                "adb",
                "-s",
                "emulator-5554",
                "install",
                "-r",
                "/path/to/companion.apk",
            ]
        );
    }

    // 15. Android port forward command
    #[test]
    fn android_port_forward_command_is_correct() {
        let d = android_device();
        let cmd = port_forward_command(&d, 8223);
        assert_eq!(
            cmd,
            vec!["adb", "-s", "emulator-5554", "forward", "tcp:8223", "tcp:8223"]
        );
    }

    // Edge: iOS clear data without reinstall path
    #[test]
    fn ios_clear_app_data_without_reinstall_has_two_commands() {
        let d = ios_device();
        let cmds = clear_app_data_commands(&d, "com.example.MyApp", None);
        assert_eq!(cmds.len(), 2);
    }
}
