//! Pure command-construction functions for device lifecycle operations.
//!
//! Each function returns a `Vec<String>` (or `Vec<Vec<String>>`), fully
//! testable without real devices. The async execution layer (`super::exec`)
//! runs the constructed commands.

use std::path::Path;

use crate::{DeviceInfo, Platform};

/// Find the .xctestrun file in a directory of extracted iOS companion products.
pub(crate) fn find_xctestrun(dir: &Path) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|ext| ext == "xctestrun"))
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
        Platform::Android => vec![], // Android uses pre-built APK
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
pub fn start_companion_command(
    device: &DeviceInfo,
    companion_path: &str,
    port: u16,
) -> Vec<String> {
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
                    "-only-testing:GolemRunnerUITests/GolemRunnerUITests/testCompanionServer"
                        .into(),
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
                    "-only-testing:GolemRunnerUITests/GolemRunnerUITests/testCompanionServer"
                        .into(),
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
