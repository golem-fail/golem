//! Async execution layer for device lifecycle operations.
//!
//! Runs the commands constructed in `super::commands` via
//! `tokio::process::Command`.

use std::path::Path;

use crate::{DeviceInfo, Platform};
use anyhow::{bail, Context, Result};
use golem_events::CodeExt;

use super::commands::{
    boot_command, build_companion_command, clear_app_data_commands, create_device_command,
    find_xctestrun, install_app_command, install_companion_command, port_forward_command,
    shutdown_command, start_companion_command_with_reg,
};

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
pub(crate) fn is_already_booted_error(e: &anyhow::Error) -> bool {
    let s = format!("{e:#}");
    s.contains("Unable to boot device in current state: Booted") || s.contains("code=405")
}

pub(crate) async fn run_command(args: &[String], context: &str) -> Result<String> {
    let Some((program, arguments)) = args.split_first() else {
        bail!("{context}: empty command");
    };

    let output = golem_common::command::output(program, arguments)
        .await
        .with_context(|| format!("{context}: failed to run {program}"))?;

    if !output.status.success() {
        bail!("{context}: {}", String::from_utf8_lossy(&output.stderr));
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

/// Deadline for the entire Android boot sequence (serial discovery +
/// `sys.boot_completed` polling), in seconds. Well above typical cold-boot
/// times (~60-120s) — see [`boot_device`] docs.
const ANDROID_BOOT_DEADLINE_SECS: u64 = 180;

/// Poll interval while waiting for a newly spawned Android emulator's
/// serial to appear in `adb devices`, in milliseconds.
const ANDROID_SERIAL_POLL_INTERVAL_MS: u64 = 500;

/// Poll interval while waiting for `sys.boot_completed` to flip to `"1"`
/// on a booting Android emulator, in seconds.
const ANDROID_BOOT_COMPLETED_POLL_INTERVAL_SECS: u64 = 1;

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
    golem_common::command::spawn_detached(program, arguments)
        .await
        .with_context(|| format!("spawn emulator for {}", device.name))?;

    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(ANDROID_BOOT_DEADLINE_SECS);
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
        tokio::time::sleep(std::time::Duration::from_millis(
            ANDROID_SERIAL_POLL_INTERVAL_MS,
        ))
        .await;
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
        tokio::time::sleep(std::time::Duration::from_secs(
            ANDROID_BOOT_COMPLETED_POLL_INTERVAL_SECS,
        ))
        .await;
    }
}

/// Snapshot every emulator serial currently visible to adb. Physical
/// devices are excluded — they don't show as `emulator-NNNN`.
async fn list_running_emulator_serials() -> Vec<String> {
    let out = match golem_common::command::output("adb", &["devices".to_string()]).await {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_emulator_serials(&stdout)
}

/// Parse the serials of running emulators out of `adb devices` stdout.
///
/// The first line is the `List of devices attached` header and is skipped.
/// Each subsequent line is `<serial>\t<state>`; only `emulator-NNNN`
/// serials in the `device` state are returned (physical devices and
/// offline/unauthorized entries are excluded).
pub(crate) fn parse_emulator_serials(stdout: &str) -> Vec<String> {
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
///
/// Serialized host-wide via [`OpClass::Install`]: `simctl install` /
/// `adb install` funnel through the one per-host device daemon
/// (`CoreSimulatorService` / `adb server`), which concurrent installs across
/// devices thrash. The permit wraps only the install command, not any settle
/// loop, so it drains as fast as the install completes.
pub async fn install_app(device: &DeviceInfo, app_path: &str) -> Result<()> {
    let args = install_app_command(device, app_path);
    golem_common::host_queue::acquire_then_run(
        golem_common::host_queue::OpClass::Install,
        run_command(&args, &format!("install app on {}", device.name)),
    )
    .await?;
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

    // iOS source-based: pass the port via env var (the .xcodeproj launcher
    // reads it rather than taking it as an argument).
    let mut env: Vec<(String, String)> = Vec::new();
    if device.platform == Platform::Ios && companion_path.ends_with(".xcodeproj") {
        if let Some(rp) = reg_port {
            env.push(("GOLEM_REG_PORT".to_string(), rp.to_string()));
        } else {
            env.push(("GOLEM_PORT".to_string(), port.to_string()));
        }
    }

    golem_common::command::spawn_detached_with(
        program,
        arguments,
        &golem_common::command::CommandOpts::with_env(env),
    )
    .await
    .with_context(|| format!("failed to spawn companion on {} (port {port})", device.name))?;
    Ok(())
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
