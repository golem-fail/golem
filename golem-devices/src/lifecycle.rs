//! Device lifecycle management: boot, shutdown, install, clear data, create.
//!
//! Each operation is split into a command-construction function (returns `Vec<String>`,
//! fully testable without real devices) and an async execution function that runs
//! the constructed command via `tokio::process::Command`.

use crate::{DeviceInfo, Platform};
use anyhow::{bail, Result};

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
            device.name.clone(),
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

/// Construct the commands to install and launch a test companion/runner.
///
/// iOS requires a two-step process: build-for-testing then test-without-building.
/// Android requires installing the companion APK then starting instrumentation.
///
/// Returns a `Vec` of commands (each command is itself a `Vec<String>`).
pub fn install_companion_commands(
    device: &DeviceInfo,
    companion_path: &str,
    test_runner_class: &str,
) -> Vec<Vec<String>> {
    match device.platform {
        Platform::Ios => vec![
            vec![
                "xcodebuild".into(),
                "build-for-testing".into(),
                "-project".into(),
                companion_path.into(),
                "-scheme".into(),
                "GolemRunnerUITests".into(),
                "-destination".into(),
                format!("id={}", device.udid),
            ],
            vec![
                "xcodebuild".into(),
                "test-without-building".into(),
                "-project".into(),
                companion_path.into(),
                "-scheme".into(),
                "GolemRunnerUITests".into(),
                "-destination".into(),
                format!("id={}", device.udid),
            ],
        ],
        Platform::Android => vec![
            vec![
                "adb".into(),
                "-s".into(),
                device.udid.clone(),
                "install".into(),
                "-r".into(),
                companion_path.into(),
            ],
            vec![
                "adb".into(),
                "-s".into(),
                device.udid.clone(),
                "shell".into(),
                "am".into(),
                "instrument".into(),
                "-w".into(),
                test_runner_class.into(),
            ],
        ],
    }
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

/// Boot a device (simulator or emulator).
pub async fn boot_device(device: &DeviceInfo) -> Result<()> {
    let args = boot_command(device);
    run_command(&args, &format!("boot {}", device.name)).await?;
    Ok(())
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

/// Install and launch a test companion on a device.
pub async fn install_companion(
    device: &DeviceInfo,
    companion_path: &str,
    test_runner_class: &str,
) -> Result<()> {
    let cmds = install_companion_commands(device, companion_path, test_runner_class);
    run_commands(&cmds, &format!("install companion on {}", device.name)).await
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
    run_command(&args, &format!("create device {name}")).await
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

    // 2. Android boot
    #[test]
    fn android_boot_command_is_correct() {
        let d = android_device();
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

    // 11. iOS install companion
    #[test]
    fn ios_install_companion_commands_are_correct() {
        let d = ios_device();
        let cmds = install_companion_commands(
            &d,
            "/path/to/Companion.xcodeproj",
            "CompanionUITests",
        );
        assert_eq!(cmds.len(), 2);
        assert_eq!(
            cmds[0],
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
        assert_eq!(
            cmds[1],
            vec![
                "xcodebuild",
                "test-without-building",
                "-project",
                "/path/to/Companion.xcodeproj",
                "-scheme",
                "GolemRunnerUITests",
                "-destination",
                "id=AAAA-BBBB-CCCC",
            ]
        );
    }

    // 12. Android install companion
    #[test]
    fn android_install_companion_commands_are_correct() {
        let d = android_device();
        let cmds = install_companion_commands(
            &d,
            "/path/to/companion.apk",
            "com.example.test/androidx.test.runner.AndroidJUnitRunner",
        );
        assert_eq!(cmds.len(), 2);
        assert_eq!(
            cmds[0],
            vec![
                "adb",
                "-s",
                "emulator-5554",
                "install",
                "-r",
                "/path/to/companion.apk",
            ]
        );
        assert_eq!(
            cmds[1],
            vec![
                "adb",
                "-s",
                "emulator-5554",
                "shell",
                "am",
                "instrument",
                "-w",
                "com.example.test/androidx.test.runner.AndroidJUnitRunner",
            ]
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
