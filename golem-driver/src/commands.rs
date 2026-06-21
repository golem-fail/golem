//! Host-side command construction helpers for deep links, push notifications,
//! media operations, and permission management.
//!
//! Each function returns command arguments as `Vec<String>` so they are
//! testable without running actual `xcrun` or `adb` processes.

// ---------------------------------------------------------------------------
// Deep links
// ---------------------------------------------------------------------------

/// Build `xcrun simctl openurl <device_id> <url>`.
pub fn deep_link_command_ios(device_id: &str, url: &str) -> Vec<String> {
    vec![
        "xcrun".into(),
        "simctl".into(),
        "openurl".into(),
        device_id.into(),
        url.into(),
    ]
}

/// Build `adb -s <serial> shell am start -a android.intent.action.VIEW -d <url>`.
pub fn deep_link_command_android(serial: &str, url: &str) -> Vec<String> {
    vec![
        "adb".into(),
        "-s".into(),
        serial.into(),
        "shell".into(),
        "am".into(),
        "start".into(),
        "-a".into(),
        "android.intent.action.VIEW".into(),
        "-d".into(),
        url.into(),
    ]
}

// ---------------------------------------------------------------------------
// Push notifications
// ---------------------------------------------------------------------------

/// Build `xcrun simctl push <device_id> <bundle_id> <payload_file>`.
pub fn push_notification_command_ios(
    device_id: &str,
    bundle_id: &str,
    payload_path: &str,
) -> Vec<String> {
    vec![
        "xcrun".into(),
        "simctl".into(),
        "push".into(),
        device_id.into(),
        bundle_id.into(),
        payload_path.into(),
    ]
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

/// Build `xcrun simctl addmedia <device_id> <file_path>`.
pub fn add_media_command_ios(device_id: &str, file_path: &str) -> Vec<String> {
    vec![
        "xcrun".into(),
        "simctl".into(),
        "addmedia".into(),
        device_id.into(),
        file_path.into(),
    ]
}

/// Build `adb -s <serial> push <file_path> /sdcard/DCIM/`.
pub fn add_media_command_android(serial: &str, file_path: &str) -> Vec<String> {
    vec![
        "adb".into(),
        "-s".into(),
        serial.into(),
        "push".into(),
        file_path.into(),
        "/sdcard/DCIM/".into(),
    ]
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

/// Build `xcrun simctl privacy <device_id> <action> <permission> <bundle_id>`.
///
/// `action` is typically `"grant"` or `"revoke"`.
pub fn permission_command_ios(
    device_id: &str,
    action: &str,
    permission: &str,
    bundle_id: &str,
) -> Vec<String> {
    vec![
        "xcrun".into(),
        "simctl".into(),
        "privacy".into(),
        device_id.into(),
        action.into(),
        permission.into(),
        bundle_id.into(),
    ]
}

/// Build `adb -s <serial> shell pm <action> <package> <permission>`.
///
/// `action` is typically `"grant"` or `"revoke"`.
pub fn permission_command_android(
    serial: &str,
    action: &str,
    package: &str,
    permission: &str,
) -> Vec<String> {
    vec![
        "adb".into(),
        "-s".into(),
        serial.into(),
        "shell".into(),
        "pm".into(),
        action.into(),
        package.into(),
        permission.into(),
    ]
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // 1. iOS deep link command correct
    // -----------------------------------------------------------------------
    #[test]
    fn ios_deep_link_command() {
        let cmd = deep_link_command_ios("ABCD-1234", "myapp://home");
        assert_eq!(
            cmd,
            vec!["xcrun", "simctl", "openurl", "ABCD-1234", "myapp://home"]
        );
    }

    // -----------------------------------------------------------------------
    // 2. Android deep link command correct
    // -----------------------------------------------------------------------
    #[test]
    fn android_deep_link_command() {
        let cmd = deep_link_command_android("emulator-5554", "myapp://home");
        assert_eq!(
            cmd,
            vec![
                "adb",
                "-s",
                "emulator-5554",
                "shell",
                "am",
                "start",
                "-a",
                "android.intent.action.VIEW",
                "-d",
                "myapp://home",
            ]
        );
    }

    // -----------------------------------------------------------------------
    // 3. iOS push notification command correct
    // -----------------------------------------------------------------------
    #[test]
    fn ios_push_notification_command() {
        let cmd =
            push_notification_command_ios("ABCD-1234", "com.example.app", "/tmp/payload.json");
        assert_eq!(
            cmd,
            vec![
                "xcrun",
                "simctl",
                "push",
                "ABCD-1234",
                "com.example.app",
                "/tmp/payload.json",
            ]
        );
    }

    // -----------------------------------------------------------------------
    // 4. iOS add media command correct
    // -----------------------------------------------------------------------
    #[test]
    fn ios_add_media_command() {
        let cmd = add_media_command_ios("ABCD-1234", "/tmp/photo.jpg");
        assert_eq!(
            cmd,
            vec!["xcrun", "simctl", "addmedia", "ABCD-1234", "/tmp/photo.jpg"]
        );
    }

    // -----------------------------------------------------------------------
    // 5. Android add media command correct
    // -----------------------------------------------------------------------
    #[test]
    fn android_add_media_command() {
        let cmd = add_media_command_android("emulator-5554", "/tmp/photo.jpg");
        assert_eq!(
            cmd,
            vec![
                "adb",
                "-s",
                "emulator-5554",
                "push",
                "/tmp/photo.jpg",
                "/sdcard/DCIM/",
            ]
        );
    }

    // -----------------------------------------------------------------------
    // 6. iOS grant permission command correct
    // -----------------------------------------------------------------------
    #[test]
    fn ios_grant_permission_command() {
        let cmd = permission_command_ios("ABCD-1234", "grant", "camera", "com.example.app");
        assert_eq!(
            cmd,
            vec![
                "xcrun",
                "simctl",
                "privacy",
                "ABCD-1234",
                "grant",
                "camera",
                "com.example.app",
            ]
        );
    }

    // -----------------------------------------------------------------------
    // 7. iOS revoke permission command correct
    // -----------------------------------------------------------------------
    #[test]
    fn ios_revoke_permission_command() {
        let cmd = permission_command_ios("ABCD-1234", "revoke", "photos", "com.example.app");
        assert_eq!(
            cmd,
            vec![
                "xcrun",
                "simctl",
                "privacy",
                "ABCD-1234",
                "revoke",
                "photos",
                "com.example.app",
            ]
        );
    }

    // -----------------------------------------------------------------------
    // 8. Android grant permission command correct
    // -----------------------------------------------------------------------
    #[test]
    fn android_grant_permission_command() {
        let cmd = permission_command_android(
            "emulator-5554",
            "grant",
            "com.example.app",
            "android.permission.CAMERA",
        );
        assert_eq!(
            cmd,
            vec![
                "adb",
                "-s",
                "emulator-5554",
                "shell",
                "pm",
                "grant",
                "com.example.app",
                "android.permission.CAMERA",
            ]
        );
    }

    // -----------------------------------------------------------------------
    // 9. Android revoke permission command correct
    // -----------------------------------------------------------------------
    #[test]
    fn android_revoke_permission_command() {
        let cmd = permission_command_android(
            "emulator-5554",
            "revoke",
            "com.example.app",
            "android.permission.READ_CONTACTS",
        );
        assert_eq!(
            cmd,
            vec![
                "adb",
                "-s",
                "emulator-5554",
                "shell",
                "pm",
                "revoke",
                "com.example.app",
                "android.permission.READ_CONTACTS",
            ]
        );
    }

    // -----------------------------------------------------------------------
    // 10. Commands with special characters in URLs
    // -----------------------------------------------------------------------
    #[test]
    fn deep_link_with_special_chars_in_url() {
        let url = "myapp://search?q=hello%20world&lang=en&ref=home#section";
        let ios_cmd = deep_link_command_ios("SIM-99", url);
        assert_eq!(ios_cmd[4], url);

        let android_cmd = deep_link_command_android("emulator-5554", url);
        assert_eq!(android_cmd[9], url);
    }

    // -----------------------------------------------------------------------
    // 11. Commands with spaces in file paths
    // -----------------------------------------------------------------------
    #[test]
    fn add_media_with_spaces_in_path() {
        let path = "/Users/test user/Documents/my photo.jpg";
        let ios_cmd = add_media_command_ios("ABCD-1234", path);
        assert_eq!(ios_cmd[4], path);

        let android_cmd = add_media_command_android("emulator-5554", path);
        assert_eq!(android_cmd[4], path);
    }

    // -----------------------------------------------------------------------
    // 12. Push notification with spaces in payload path
    // -----------------------------------------------------------------------
    #[test]
    fn push_notification_with_spaces_in_path() {
        let payload = "/tmp/my payloads/notif.json";
        let cmd = push_notification_command_ios("DEV-1", "com.app.test", payload);
        assert_eq!(cmd[5], payload);
    }

    // -----------------------------------------------------------------------
    // 13. Android deep link with https URL
    // -----------------------------------------------------------------------
    #[test]
    fn android_deep_link_with_https_url() {
        let cmd = deep_link_command_android(
            "192.168.1.100:5555",
            "https://example.com/path?foo=bar&baz=1",
        );
        assert_eq!(cmd[2], "192.168.1.100:5555");
        assert_eq!(cmd[9], "https://example.com/path?foo=bar&baz=1");
    }

    // -----------------------------------------------------------------------
    // 14. Permission with network-connected serial
    // -----------------------------------------------------------------------
    #[test]
    fn android_permission_with_network_serial() {
        let cmd = permission_command_android(
            "192.168.1.50:5555",
            "grant",
            "com.test.pkg",
            "android.permission.ACCESS_FINE_LOCATION",
        );
        assert_eq!(cmd[2], "192.168.1.50:5555");
        assert_eq!(cmd[6], "com.test.pkg");
        assert_eq!(cmd[7], "android.permission.ACCESS_FINE_LOCATION");
    }

    // -----------------------------------------------------------------------
    // 18. iOS permission action is positional, not validated — an arbitrary
    //     action string is forwarded verbatim into the privacy slot
    // -----------------------------------------------------------------------
    #[test]
    fn ios_permission_forwards_arbitrary_action() {
        // 1. A non grant/revoke action ("reset") is accepted without validation.
        let cmd = permission_command_ios("DEV", "reset", "location", "com.x");
        // 2. The whole argv SHALL match the hand-written expected command, proving
        //    the arbitrary action lands in the privacy slot verbatim and every
        //    other position (prefix + device id) is left untouched.
        assert_eq!(
            cmd,
            vec!["xcrun", "simctl", "privacy", "DEV", "reset", "location", "com.x",],
            "arbitrary action SHALL be forwarded verbatim with no validation",
        );
    }
}
