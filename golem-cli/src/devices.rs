use golem_devices::{DeviceInfo, DeviceState, Platform};
use std::fmt::Write;

/// Format a device state as a human-readable string.
fn state_label(state: DeviceState) -> &'static str {
    match state {
        DeviceState::Booted => "booted",
        DeviceState::Shutdown => "shutdown",
        DeviceState::Connected => "connected",
        DeviceState::NeedsCreation => "needs-creation",
    }
}

/// Format a list of devices into a human-readable table grouped by category.
///
/// Categories are: iOS Simulators, Android Emulators, Physical Devices.
/// Each device row shows: name, platform:version, device_type, state.
/// Columns are aligned within each category.
pub fn format_device_list(devices: &[DeviceInfo]) -> String {
    let ios_sims: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| d.platform == Platform::Ios && !d.physical)
        .collect();

    let android_emus: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| d.platform == Platform::Android && !d.physical)
        .collect();

    let physical: Vec<&DeviceInfo> = devices.iter().filter(|d| d.physical).collect();

    let mut out = String::new();

    write_section(&mut out, "iOS Simulators:", &ios_sims);
    write_section(&mut out, "Android Emulators:", &android_emus);
    write_section(&mut out, "Physical Devices:", &physical);

    // Remove the trailing newline if present
    if out.ends_with('\n') {
        out.truncate(out.len() - 1);
    }

    out
}

/// Write a single section (header + rows or "(none)") into the output buffer.
fn write_section(out: &mut String, header: &str, devices: &[&DeviceInfo]) {
    let _ = writeln!(out, "{header}");

    if devices.is_empty() {
        let _ = writeln!(out, "  (none)");
        let _ = writeln!(out);
        return;
    }

    // Compute column widths for alignment
    let name_width = devices
        .iter()
        .map(|d| d.name.len())
        .max()
        .unwrap_or(0);

    let version_strings: Vec<String> = devices
        .iter()
        .map(|d| format!("{}:{}", d.platform, d.os_version))
        .collect();

    let version_width = version_strings
        .iter()
        .map(|s| s.len())
        .max()
        .unwrap_or(0);

    let type_width = devices
        .iter()
        .map(|d| d.device_type.to_string().len())
        .max()
        .unwrap_or(0);

    for (device, ver_str) in devices.iter().zip(version_strings.iter()) {
        let dtype = device.device_type.to_string();
        let state = state_label(device.state);
        let _ = writeln!(
            out,
            "  {:<name_w$}  {:<ver_w$}  {:<type_w$}  {state}",
            device.name,
            ver_str,
            dtype,
            name_w = name_width,
            ver_w = version_width,
            type_w = type_width,
        );
    }

    let _ = writeln!(out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_devices::{DeviceType, Platform};

    /// Helper to build a DeviceInfo with minimal boilerplate.
    fn make_device(
        name: &str,
        platform: Platform,
        device_type: DeviceType,
        os_version: &str,
        os_major: u32,
        state: DeviceState,
        physical: bool,
    ) -> DeviceInfo {
        DeviceInfo {
            name: name.to_string(),
            udid: format!("udid-{name}"),
            platform,
            device_type,
            os_major,
            os_version: os_version.to_string(),
            state,
            physical,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        }
    }

    #[test]
    fn format_ios_simulators_with_correct_columns() {
        let devices = vec![make_device(
            "iPhone 15 Pro",
            Platform::Ios,
            DeviceType::Phone,
            "18.0",
            18,
            DeviceState::Booted,
            false,
        )];

        let output = format_device_list(&devices);
        assert!(output.contains("iOS Simulators:"));
        assert!(output.contains("iPhone 15 Pro"));
        assert!(output.contains("ios:18.0"));
        assert!(output.contains("phone"));
        assert!(output.contains("booted"));
    }

    #[test]
    fn format_android_emulators_with_correct_columns() {
        let devices = vec![make_device(
            "Pixel_7_API_34",
            Platform::Android,
            DeviceType::Phone,
            "34",
            34,
            DeviceState::Shutdown,
            false,
        )];

        let output = format_device_list(&devices);
        assert!(output.contains("Android Emulators:"));
        assert!(output.contains("Pixel_7_API_34"));
        assert!(output.contains("android:34"));
        assert!(output.contains("phone"));
        assert!(output.contains("shutdown"));
    }

    #[test]
    fn format_physical_devices_section() {
        let devices = vec![make_device(
            "Pixel 8",
            Platform::Android,
            DeviceType::Phone,
            "14.0",
            14,
            DeviceState::Connected,
            true,
        )];

        let output = format_device_list(&devices);
        assert!(output.contains("Physical Devices:"));
        assert!(output.contains("Pixel 8"));
        assert!(output.contains("android:14.0"));
        assert!(output.contains("connected"));
    }

    #[test]
    fn empty_device_list_shows_none_for_each_category() {
        let output = format_device_list(&[]);

        assert!(output.contains("iOS Simulators:"));
        assert!(output.contains("Android Emulators:"));
        assert!(output.contains("Physical Devices:"));
        // Each category should show "(none)"
        let none_count = output.matches("(none)").count();
        assert_eq!(none_count, 3, "Expected 3 '(none)' markers, got {none_count}");
    }

    #[test]
    fn mixed_ios_and_android_devices_grouped_correctly() {
        let devices = vec![
            make_device(
                "iPhone 15",
                Platform::Ios,
                DeviceType::Phone,
                "18.0",
                18,
                DeviceState::Shutdown,
                false,
            ),
            make_device(
                "Pixel_7_API_34",
                Platform::Android,
                DeviceType::Phone,
                "34",
                34,
                DeviceState::Shutdown,
                false,
            ),
        ];

        let output = format_device_list(&devices);

        // iOS section should come before Android section
        let ios_pos = output
            .find("iOS Simulators:")
            .expect("iOS header should exist");
        let android_pos = output
            .find("Android Emulators:")
            .expect("Android header should exist");
        assert!(ios_pos < android_pos, "iOS section SHALL precede Android section");

        // iPhone should appear under iOS, not under Android
        let iphone_pos = output.find("iPhone 15").expect("iPhone should appear");
        assert!(
            iphone_pos > ios_pos && iphone_pos < android_pos,
            "iPhone should be listed under iOS Simulators"
        );

        // Pixel should appear under Android
        let pixel_pos = output.find("Pixel_7_API_34").expect("Pixel should appear");
        assert!(
            pixel_pos > android_pos,
            "Pixel should be listed under Android Emulators"
        );
    }

    #[test]
    fn device_state_displayed_correctly() {
        let devices = vec![
            make_device(
                "iPhone 15 Pro",
                Platform::Ios,
                DeviceType::Phone,
                "18.0",
                18,
                DeviceState::Booted,
                false,
            ),
            make_device(
                "iPhone 14",
                Platform::Ios,
                DeviceType::Phone,
                "17.5",
                17,
                DeviceState::Shutdown,
                false,
            ),
            make_device(
                "Real iPhone",
                Platform::Ios,
                DeviceType::Phone,
                "18.0",
                18,
                DeviceState::Connected,
                true,
            ),
        ];

        let output = format_device_list(&devices);
        assert!(output.contains("booted"), "Should show booted state");
        assert!(output.contains("shutdown"), "Should show shutdown state");
        assert!(output.contains("connected"), "Should show connected state");
    }

    #[test]
    fn device_type_displayed_correctly() {
        let devices = vec![
            make_device(
                "iPhone 15",
                Platform::Ios,
                DeviceType::Phone,
                "18.0",
                18,
                DeviceState::Shutdown,
                false,
            ),
            make_device(
                "iPad Air (5th gen)",
                Platform::Ios,
                DeviceType::Tablet,
                "18.0",
                18,
                DeviceState::Shutdown,
                false,
            ),
        ];

        let output = format_device_list(&devices);
        assert!(output.contains("phone"), "Should show phone type");
        assert!(output.contains("tablet"), "Should show tablet type");
    }

    #[test]
    fn columns_are_aligned() {
        let devices = vec![
            make_device(
                "iPhone 15 Pro",
                Platform::Ios,
                DeviceType::Phone,
                "18.0",
                18,
                DeviceState::Booted,
                false,
            ),
            make_device(
                "iPhone 15",
                Platform::Ios,
                DeviceType::Phone,
                "18.0",
                18,
                DeviceState::Shutdown,
                false,
            ),
            make_device(
                "iPad Air (5th gen)",
                Platform::Ios,
                DeviceType::Tablet,
                "18.0",
                18,
                DeviceState::Shutdown,
                false,
            ),
        ];

        let output = format_device_list(&devices);
        let ios_lines: Vec<&str> = output
            .lines()
            .skip(1) // skip header
            .take_while(|line| !line.is_empty())
            .collect();

        assert_eq!(ios_lines.len(), 3, "Should have 3 iOS device lines");

        // All lines should have the same length after trimming trailing whitespace is
        // not applicable because state strings differ in length.
        // Instead, verify that the platform:version column starts at the same position.
        let version_positions: Vec<Option<usize>> = ios_lines
            .iter()
            .map(|line| line.find("ios:18.0"))
            .collect();

        // All positions should be Some and equal
        let first = version_positions[0].expect("version column should exist in first line");
        for (i, pos) in version_positions.iter().enumerate() {
            let p = pos.unwrap_or_else(|| panic!("version column missing in line {i}"));
            assert_eq!(
                p, first,
                "Version column in line {i} at position {p} differs from first at {first}"
            );
        }
    }

    #[test]
    fn physical_devices_shows_none_when_only_simulators_present() {
        let devices = vec![make_device(
            "iPhone 15",
            Platform::Ios,
            DeviceType::Phone,
            "18.0",
            18,
            DeviceState::Shutdown,
            false,
        )];

        let output = format_device_list(&devices);

        // Physical devices section should show (none)
        let phys_pos = output
            .find("Physical Devices:")
            .expect("Physical Devices header should exist");
        let after_phys = &output[phys_pos..];
        assert!(
            after_phys.contains("(none)"),
            "Physical Devices section should show (none) when no physical devices"
        );
    }

    #[test]
    fn needs_creation_state_is_displayed() {
        let devices = vec![make_device(
            "Pixel_8_API_35",
            Platform::Android,
            DeviceType::Phone,
            "35",
            35,
            DeviceState::NeedsCreation,
            false,
        )];

        let output = format_device_list(&devices);
        assert!(
            output.contains("needs-creation"),
            "NeedsCreation state should render as 'needs-creation'"
        );
    }
}
