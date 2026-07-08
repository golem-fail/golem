//! Device lifecycle management: boot, shutdown, install, clear data, create.
//!
//! Each operation is split into a command-construction function (returns `Vec<String>`,
//! fully testable without real devices) and an async execution function that runs
//! the constructed command via `tokio::process::Command`.
//!
//! Submodules:
//! - [`commands`]: pure sync command builders.
//! - [`exec`]: async execution layer that runs the built commands.
//! - [`auto_create`]: auto-create simulators/emulators when none is available.

mod auto_create;
mod commands;
mod exec;

pub use auto_create::auto_create_device;
pub use commands::{
    boot_command, build_companion_command, clear_app_data_commands, create_device_command,
    install_app_command, install_companion_command, port_forward_command, shutdown_command,
    start_companion_command, start_companion_command_with_reg,
};
pub use exec::{
    boot_device, build_companion, clear_app_data, create_simulator, install_android_companion,
    install_android_companion_with_main, install_app, run_command_public, setup_adb_reverse,
    shutdown_device, spawn_companion_with_reg,
};

// Re-imported so the inline test module below (which reaches everything via
// `use super::*`) keeps compiling unchanged after the split — these items
// are not part of the public API, only test-visible.
#[cfg(test)]
use crate::{DeviceInfo, Platform};
#[cfg(test)]
use commands::find_xctestrun;
#[cfg(test)]
use exec::{is_already_booted_error, parse_emulator_serials, run_command};

// ---------------------------------------------------------------------------
// Tests — command construction only (no real devices needed)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceState, DeviceType};
    use golem_common::command::{set_test_runner, Canned, FakeCommandRunner};

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
            device_type_id: Some("com.apple.CoreSimulator.SimDeviceType.iPhone-15".into()),
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
            name: "Pixel 8 API 34".into(), // display name with spaces
            udid: "Pixel_8_API_34".into(), // on-disk AVD identifier
            state: DeviceState::Shutdown,
            ..android_device()
        };
        let cmd = boot_command(&d);
        assert_eq!(
            cmd,
            vec![
                "emulator",
                "-avd",
                "Pixel_8_API_34",
                "-no-window",
                "-no-audio"
            ]
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
        assert_eq!(cmd, vec!["adb", "-s", "emulator-5554", "emu", "kill"]);
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
        let cmds = clear_app_data_commands(&d, "com.example.MyApp", Some("/path/to/MyApp.app"));
        assert_eq!(cmds.len(), 3);
        assert_eq!(
            cmds[0],
            vec![
                "xcrun",
                "simctl",
                "privacy",
                "AAAA-BBBB-CCCC",
                "reset",
                "all"
            ]
        );
        assert_eq!(
            cmds[1],
            vec![
                "xcrun",
                "simctl",
                "uninstall",
                "AAAA-BBBB-CCCC",
                "com.example.MyApp"
            ]
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
            vec![
                "adb",
                "-s",
                "emulator-5554",
                "forward",
                "tcp:8223",
                "tcp:8223"
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

    // 16. Android build companion is a no-op (pre-built APK).
    #[test]
    fn android_build_companion_command_is_empty() {
        let d = android_device();
        let cmd = build_companion_command(&d, "/path/to/companion.apk");
        assert!(cmd.is_empty(), "Android build SHALL produce no command");
    }

    // 17. iOS build companion for an embedded (non-.xcodeproj) path is a
    //     no-op — the products are already built.
    #[test]
    fn ios_build_companion_command_embedded_is_empty() {
        let d = ios_device();
        let cmd = build_companion_command(&d, "/path/to/products");
        assert!(
            cmd.is_empty(),
            "embedded iOS build SHALL produce no command"
        );
    }

    // 18. Android start companion with reg_port uses reg_port (not legacy
    //     port). The legacy port arg SHALL NOT appear.
    #[test]
    fn android_start_companion_with_reg_uses_reg_port() {
        let d = android_device();
        let cmd = start_companion_command_with_reg(&d, "/path/to/companion.apk", 8225, Some(9999));
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
                "reg_port",
                "9999",
                "fail.golem.companion.test/androidx.test.runner.AndroidJUnitRunner",
            ]
        );
    }

    // 19. iOS source-based start companion ignores reg_port — the
    //     xcodebuild invocation is identical whether or not reg_port is set
    //     (the port is passed via env var elsewhere, not on the cmdline).
    #[test]
    fn ios_source_start_companion_with_reg_matches_legacy() {
        let d = ios_device();
        let with_reg =
            start_companion_command_with_reg(&d, "/path/to/Companion.xcodeproj", 8222, Some(9999));
        let legacy = start_companion_command(&d, "/path/to/Companion.xcodeproj", 8222);
        assert_eq!(
            with_reg, legacy,
            "iOS source command SHALL be reg_port-independent"
        );
    }

    // 20. iOS embedded start companion with no .xctestrun in the directory
    //     falls back to an empty -xctestrun argument (find_xctestrun → None
    //     → unwrap_or_default).
    #[test]
    fn ios_embedded_start_companion_empty_xctestrun_when_missing() {
        let dir = unique_temp_dir("golem-no-xctestrun");
        std::fs::create_dir_all(&dir).expect("create temp dir SHALL succeed");
        let path = dir.to_string_lossy().into_owned();

        let cmd = start_companion_command(&ios_device(), &path, 8222);

        std::fs::remove_dir_all(&dir).ok();
        // The arg following "-xctestrun" SHALL be empty when none is found.
        let idx = cmd
            .iter()
            .position(|a| a == "-xctestrun")
            .expect("command SHALL contain -xctestrun");
        assert_eq!(cmd[idx + 1], "", "missing xctestrun SHALL yield empty arg");
    }

    // 21. find_xctestrun locates a .xctestrun file when present.
    #[test]
    fn find_xctestrun_finds_present_file() {
        let dir = unique_temp_dir("golem-find-xctestrun");
        std::fs::create_dir_all(&dir).expect("create temp dir SHALL succeed");
        let file = dir.join("Companion.xctestrun");
        std::fs::write(&file, b"x").expect("write fixture SHALL succeed");

        let found = find_xctestrun(&dir);

        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(
            found.as_deref(),
            Some(file.to_string_lossy().as_ref()),
            "find_xctestrun SHALL return the .xctestrun path"
        );
    }

    // 22. find_xctestrun returns None when no .xctestrun is present.
    #[test]
    fn find_xctestrun_none_when_absent() {
        let dir = unique_temp_dir("golem-find-xctestrun-empty");
        std::fs::create_dir_all(&dir).expect("create temp dir SHALL succeed");
        std::fs::write(dir.join("other.txt"), b"x").expect("write fixture SHALL succeed");

        let found = find_xctestrun(&dir);

        std::fs::remove_dir_all(&dir).ok();
        assert!(found.is_none(), "no .xctestrun SHALL yield None");
    }

    // 23. find_xctestrun returns None for a nonexistent directory
    //     (read_dir errors → ok()? short-circuits).
    #[test]
    fn find_xctestrun_none_for_missing_dir() {
        let dir = unique_temp_dir("golem-find-xctestrun-missing");
        assert!(
            find_xctestrun(&dir).is_none(),
            "missing dir SHALL yield None"
        );
    }

    // 24. is_already_booted_error recognises the exact CoreSimulator
    //     message and the code=405 form, but not unrelated errors.
    #[test]
    fn is_already_booted_error_detection() {
        let booted = anyhow::anyhow!("Unable to boot device in current state: Booted");
        assert!(
            is_already_booted_error(&booted),
            "the Booted-state message SHALL be recognised"
        );

        let code405 = anyhow::anyhow!("SimError, code=405): something");
        assert!(is_already_booted_error(&code405), "code=405 SHALL match");

        let other = anyhow::anyhow!("Unable to boot device: no such device");
        assert!(
            !is_already_booted_error(&other),
            "unrelated boot errors SHALL NOT match"
        );
    }

    // 25. is_already_booted_error inspects the full error chain, so a
    //     wrapped/contextualised cause still matches.
    #[test]
    fn is_already_booted_error_matches_wrapped_cause() {
        let wrapped = anyhow::anyhow!("Unable to boot device in current state: Booted")
            .context("boot iPhone 15");
        assert!(
            is_already_booted_error(&wrapped),
            "wrapped Booted cause SHALL be recognised via {{:#}}"
        );
    }

    // 26. run_command on an empty arg slice fails with an "empty command"
    //     error rather than panicking on split_first.
    #[tokio::test]
    async fn run_command_empty_args_errors() {
        let err = run_command(&[], "ctx")
            .await
            .expect_err("empty args SHALL error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("empty command"),
            "error SHALL mention empty command, got: {msg}"
        );
    }

    // 27. run_command surfaces a non-zero exit as an error carrying the
    //     context prefix and the child's stderr (hermetic via the fake).
    #[tokio::test]
    async fn run_command_nonzero_exit_errors() {
        let fake = std::sync::Arc::new(FakeCommandRunner::new());
        fake.expect(&["some-tool", "--bad"], Canned::exit(1, "", "boom"));
        let _g = set_test_runner(fake);

        let err = run_command(&["some-tool".into(), "--bad".into()], "deliberate failure")
            .await
            .expect_err("a non-zero exit SHALL error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("deliberate failure") && msg.contains("boom"),
            "error SHALL carry the context and stderr, got: {msg}"
        );
    }

    // 28. run_command returns captured stdout on success (hermetic).
    #[tokio::test]
    async fn run_command_success_returns_stdout() {
        let fake = std::sync::Arc::new(FakeCommandRunner::new());
        fake.expect(&["echo", "hello"], Canned::ok_stdout("hello\n"));
        let _g = set_test_runner(fake);

        let out = run_command(&["echo".into(), "hello".into()], "echo")
            .await
            .expect("echo SHALL succeed");
        assert_eq!(out.trim(), "hello", "stdout SHALL be captured verbatim");
    }

    // 28a. boot_device on iOS runs `simctl boot` then blocks on
    //      `bootstatus -b`, returning a Booted DeviceInfo with the udid intact.
    #[tokio::test]
    async fn boot_ios_waits_for_bootstatus() {
        let device = ios_device();
        let fake = std::sync::Arc::new(FakeCommandRunner::new());
        fake.expect(
            &["xcrun", "simctl", "boot", "AAAA-BBBB-CCCC"],
            Canned::ok_stdout(""),
        );
        fake.expect(
            &["xcrun", "simctl", "bootstatus", "AAAA-BBBB-CCCC", "-b"],
            Canned::ok_stdout(""),
        );
        let _g = set_test_runner(fake.clone());

        let booted = boot_device(&device).await.expect("boot SHALL succeed");
        assert_eq!(booted.state, DeviceState::Booted);
        assert_eq!(booted.udid, device.udid, "iOS keeps its static udid");
        assert_eq!(
            fake.recorded(),
            vec![
                vec![
                    "xcrun".to_string(),
                    "simctl".into(),
                    "boot".into(),
                    "AAAA-BBBB-CCCC".into()
                ],
                vec![
                    "xcrun".to_string(),
                    "simctl".into(),
                    "bootstatus".into(),
                    "AAAA-BBBB-CCCC".into(),
                    "-b".into()
                ],
            ],
            "boot then bootstatus, in order"
        );
    }

    // 28b. The "already Booted" (code=405) race — two slots booting the same
    //      shared sim — is treated as success; bootstatus still confirms ready.
    #[tokio::test]
    async fn boot_ios_treats_already_booted_race_as_success() {
        let device = ios_device();
        let fake = std::sync::Arc::new(FakeCommandRunner::new());
        fake.expect(
            &["xcrun", "simctl", "boot", "AAAA-BBBB-CCCC"],
            Canned::exit(
                149,
                "",
                "An error was encountered processing the command (domain=com.apple.CoreSimulator.SimError, code=405): Unable to boot device in current state: Booted",
            ),
        );
        fake.expect(
            &["xcrun", "simctl", "bootstatus", "AAAA-BBBB-CCCC", "-b"],
            Canned::ok_stdout(""),
        );
        let _g = set_test_runner(fake);

        let booted = boot_device(&device)
            .await
            .expect("the already-booted race SHALL be treated as success");
        assert_eq!(booted.state, DeviceState::Booted);
    }

    // 28c. A genuine boot error (not the 405 race) fails fast, before
    //      bootstatus is attempted.
    #[tokio::test]
    async fn boot_ios_propagates_real_boot_error_without_bootstatus() {
        let device = ios_device();
        let fake = std::sync::Arc::new(FakeCommandRunner::new());
        fake.expect(
            &["xcrun", "simctl", "boot", "AAAA-BBBB-CCCC"],
            Canned::exit(1, "", "Invalid device: AAAA-BBBB-CCCC"),
        );
        let _g = set_test_runner(fake.clone());

        let err = boot_device(&device)
            .await
            .expect_err("a real boot error SHALL propagate");
        assert!(format!("{err:#}").contains("Invalid device"));
        assert_eq!(
            fake.call_count(),
            1,
            "bootstatus SHALL NOT run after a fatal boot error"
        );
    }

    // 28d. A bootstatus failure after a successful boot propagates.
    #[tokio::test]
    async fn boot_ios_propagates_bootstatus_failure() {
        let device = ios_device();
        let fake = std::sync::Arc::new(FakeCommandRunner::new());
        fake.expect(
            &["xcrun", "simctl", "boot", "AAAA-BBBB-CCCC"],
            Canned::ok_stdout(""),
        );
        fake.expect(
            &["xcrun", "simctl", "bootstatus", "AAAA-BBBB-CCCC", "-b"],
            Canned::exit(1, "", "bootstatus failed"),
        );
        let _g = set_test_runner(fake);

        let err = boot_device(&device)
            .await
            .expect_err("bootstatus failure SHALL propagate");
        assert!(format!("{err:#}").contains("bootstatus"));
    }

    // 28e. boot_device on Android spawns the emulator, resolves the new serial
    //      by matching AVD name, polls sys.boot_completed until "1", and
    //      rewrites udid from the AVD id to the dynamic emulator serial.
    //      start_paused advances the poll sleeps instantly.
    #[tokio::test(start_paused = true)]
    async fn boot_android_resolves_serial_and_waits_for_boot_completed() {
        let mut device = android_device();
        device.udid = "Pixel_8_API_34".into(); // AVD identifier for a shutdown device
        device.state = DeviceState::Shutdown;

        let fake = std::sync::Arc::new(FakeCommandRunner::new());
        // `adb devices`: empty before spawn, then the new serial appears.
        fake.expect(
            &["adb", "devices"],
            Canned::ok_stdout("List of devices attached\n"),
        );
        fake.expect(
            &["adb", "devices"],
            Canned::ok_stdout("List of devices attached\nemulator-5554\tdevice\n"),
        );
        // The new serial reports our AVD name.
        fake.expect(
            &["adb", "-s", "emulator-5554", "emu", "avd", "name"],
            Canned::ok_stdout("Pixel_8_API_34\n"),
        );
        // Readiness poll flips 0 → 1.
        let getprop = [
            "adb",
            "-s",
            "emulator-5554",
            "shell",
            "getprop",
            "sys.boot_completed",
        ];
        fake.expect(&getprop, Canned::ok_stdout("0\n"));
        fake.expect(&getprop, Canned::ok_stdout("1\n"));
        let _g = set_test_runner(fake);

        let booted = boot_device(&device)
            .await
            .expect("android boot SHALL succeed");
        assert_eq!(booted.state, DeviceState::Booted);
        assert_eq!(
            booted.udid, "emulator-5554",
            "udid SHALL be rewritten from AVD id to the dynamic serial"
        );
    }

    // 29. parse_emulator_serials extracts only emulator serials in the
    //     `device` state, skipping the header line and excluding physical
    //     devices and non-`device` states.
    #[test]
    fn parse_emulator_serials_extracts_device_state_emulators() {
        let stdout = "List of devices attached\n\
                      emulator-5554\tdevice\n\
                      emulator-5556\tdevice\n";
        let serials = parse_emulator_serials(stdout);
        assert_eq!(
            serials,
            vec!["emulator-5554", "emulator-5556"],
            "both booted emulator serials SHALL be returned"
        );
    }

    // 30. parse_emulator_serials skips the header line — a serial that
    //     looked like the header SHALL not leak through (skip(1)).
    #[test]
    fn parse_emulator_serials_skips_header() {
        // The header is always line 1; with no devices the result is empty.
        let serials = parse_emulator_serials("List of devices attached\n");
        assert!(
            serials.is_empty(),
            "header-only output SHALL yield no serials"
        );
    }

    // 31. parse_emulator_serials excludes physical devices (serials that
    //     do not start with `emulator-`) and non-`device` states.
    #[test]
    fn parse_emulator_serials_excludes_physical_and_offline() {
        let stdout = "List of devices attached\n\
                      emulator-5554\tdevice\n\
                      R5CT70ABCDE\tdevice\n\
                      emulator-5556\toffline\n\
                      emulator-5558\tunauthorized\n";
        let serials = parse_emulator_serials(stdout);
        assert_eq!(
            serials,
            vec!["emulator-5554"],
            "only emulator serials in the device state SHALL be returned"
        );
    }

    // 32. parse_emulator_serials on empty input yields no serials (no
    //     header, nothing to skip past, no panic).
    #[test]
    fn parse_emulator_serials_empty_input() {
        assert!(
            parse_emulator_serials("").is_empty(),
            "empty stdout SHALL yield no serials"
        );
    }

    // Helper: a process-unique temp directory path under the system temp
    // dir. Avoids a tempfile dev-dependency while keeping tests isolated.
    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()))
    }
}
