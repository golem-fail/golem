use anyhow::{bail, ensure, Context};

use crate::{OsVersionSpec, Platform};

/// Parse an OS version string into an OsVersionSpec.
///
/// Supports: "ios:18", "ios:17+", "ios:latest", "ios:latest:2", "android:34", etc.
pub fn parse_os_version(input: &str) -> anyhow::Result<OsVersionSpec> {
    // Must contain a colon separator
    let colon_pos = input
        .find(':')
        .context("invalid format: expected 'platform:version' (missing ':')")?;

    let platform_str = &input[..colon_pos];
    let rest = &input[colon_pos + 1..];

    // Parse platform
    let platform = match platform_str {
        "ios" => Platform::Ios,
        "android" => Platform::Android,
        other => bail!("unsupported platform: {other}"),
    };

    // Rest must not be empty
    ensure!(!rest.is_empty(), "missing version after '{platform_str}:'");

    // Handle "latest" variants
    if rest == "latest" {
        return Ok(OsVersionSpec::Latest {
            platform,
            count: 1,
        });
    }

    if let Some(suffix) = rest.strip_prefix("latest:") {
        let count: u32 = suffix
            .parse()
            .context("invalid count after 'latest:'")?;
        ensure!(count > 0, "latest count must be >= 1, got {count}");
        return Ok(OsVersionSpec::Latest { platform, count });
    }

    // Reject "latest+" or "latest<anything else>"
    if rest.starts_with("latest") {
        bail!("invalid latest syntax: '{rest}' (use 'latest' or 'latest:N')");
    }

    // Handle numeric versions, possibly with "+" suffix
    let (version_str, is_minimum) = if let Some(stripped) = rest.strip_suffix('+') {
        (stripped, true)
    } else {
        (rest, false)
    };

    // Strip optional minor version (e.g., "18.6" -> "18")
    let major_str = version_str.split('.').next().unwrap_or(version_str);

    let major: u32 = major_str
        .parse()
        .with_context(|| format!("invalid version number: '{version_str}'"))?;

    if is_minimum {
        Ok(OsVersionSpec::Minimum { platform, major })
    } else {
        Ok(OsVersionSpec::Exact { platform, major })
    }
}

/// Check if a device's OS major version matches this spec.
///
/// For Exact: device_major == spec_major
/// For Minimum: device_major >= spec_major
/// For Latest: always returns true (resolved elsewhere)
pub fn matches_version(spec: &OsVersionSpec, device_major: u32) -> bool {
    match spec {
        OsVersionSpec::Exact { major, .. } => device_major == *major,
        OsVersionSpec::Minimum { major, .. } => device_major >= *major,
        OsVersionSpec::Latest { .. } => true,
    }
}

/// Resolve "latest:N" against a list of available major versions.
///
/// Returns the N highest versions, sorted ascending.
/// For "latest" (count=1), returns the single highest.
/// If count exceeds available versions, returns all available versions.
pub fn resolve_latest(_platform: Platform, count: u32, available: &[u32]) -> Vec<u32> {
    let mut sorted: Vec<u32> = available.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let take = (count as usize).min(sorted.len());
    sorted[sorted.len() - take..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1. iOS exact major version -- "ios:18" matches any 18.x
    #[test]
    fn ios_exact_major_version() {
        let spec = parse_os_version("ios:18").expect("should parse");
        assert_eq!(
            spec,
            OsVersionSpec::Exact {
                platform: Platform::Ios,
                major: 18,
            }
        );
        assert!(matches_version(&spec, 18));
        assert!(!matches_version(&spec, 17));
        assert!(!matches_version(&spec, 19));
    }

    // 2. iOS minimum version -- "ios:17+" matches 17 and 18 from [16, 17, 18]
    #[test]
    fn ios_minimum_version() {
        let spec = parse_os_version("ios:17+").expect("should parse");
        assert_eq!(
            spec,
            OsVersionSpec::Minimum {
                platform: Platform::Ios,
                major: 17,
            }
        );
        assert!(!matches_version(&spec, 16));
        assert!(matches_version(&spec, 17));
        assert!(matches_version(&spec, 18));
    }

    // 3. iOS latest -- "ios:latest" from [16, 17, 18] selects 18
    #[test]
    fn ios_latest() {
        let spec = parse_os_version("ios:latest").expect("should parse");
        assert_eq!(
            spec,
            OsVersionSpec::Latest {
                platform: Platform::Ios,
                count: 1,
            }
        );
        let resolved = resolve_latest(Platform::Ios, 1, &[16, 17, 18]);
        assert_eq!(resolved, vec![18]);
    }

    // 4. iOS latest:N -- "ios:latest:2" from [16, 17, 18] expands to [17, 18]
    #[test]
    fn ios_latest_n() {
        let spec = parse_os_version("ios:latest:2").expect("should parse");
        assert_eq!(
            spec,
            OsVersionSpec::Latest {
                platform: Platform::Ios,
                count: 2,
            }
        );
        let resolved = resolve_latest(Platform::Ios, 2, &[16, 17, 18]);
        assert_eq!(resolved, vec![17, 18]);
    }

    // 5. iOS latest:3 -- from [16, 17, 18] expands to [16, 17, 18]
    #[test]
    fn ios_latest_3() {
        let resolved = resolve_latest(Platform::Ios, 3, &[16, 17, 18]);
        assert_eq!(resolved, vec![16, 17, 18]);
    }

    // 6. iOS latest:N exceeds available -- "ios:latest:5" from [17, 18] -> [17, 18]
    #[test]
    fn ios_latest_n_exceeds_available() {
        let spec = parse_os_version("ios:latest:5").expect("should parse");
        assert_eq!(
            spec,
            OsVersionSpec::Latest {
                platform: Platform::Ios,
                count: 5,
            }
        );
        let resolved = resolve_latest(Platform::Ios, 5, &[17, 18]);
        assert_eq!(resolved, vec![17, 18]);
    }

    // 7. Android exact API level -- "android:34" matches 34
    #[test]
    fn android_exact_api_level() {
        let spec = parse_os_version("android:34").expect("should parse");
        assert_eq!(
            spec,
            OsVersionSpec::Exact {
                platform: Platform::Android,
                major: 34,
            }
        );
        assert!(matches_version(&spec, 34));
        assert!(!matches_version(&spec, 33));
    }

    // 8. Android minimum API -- "android:31+" matches 31, 33, 34 from [28, 31, 33, 34]
    #[test]
    fn android_minimum_api() {
        let spec = parse_os_version("android:31+").expect("should parse");
        assert_eq!(
            spec,
            OsVersionSpec::Minimum {
                platform: Platform::Android,
                major: 31,
            }
        );
        assert!(!matches_version(&spec, 28));
        assert!(matches_version(&spec, 31));
        assert!(matches_version(&spec, 33));
        assert!(matches_version(&spec, 34));
    }

    // 9. Android latest -- "android:latest" from [31, 33, 34] selects 34
    #[test]
    fn android_latest() {
        let spec = parse_os_version("android:latest").expect("should parse");
        assert_eq!(
            spec,
            OsVersionSpec::Latest {
                platform: Platform::Android,
                count: 1,
            }
        );
        let resolved = resolve_latest(Platform::Android, 1, &[31, 33, 34]);
        assert_eq!(resolved, vec![34]);
    }

    // 10. Android latest:N -- "android:latest:2" from [31, 33, 34] -> [33, 34]
    #[test]
    fn android_latest_n() {
        let spec = parse_os_version("android:latest:2").expect("should parse");
        assert_eq!(
            spec,
            OsVersionSpec::Latest {
                platform: Platform::Android,
                count: 2,
            }
        );
        let resolved = resolve_latest(Platform::Android, 2, &[31, 33, 34]);
        assert_eq!(resolved, vec![33, 34]);
    }

    // 11. Invalid platform -- "windows:11" -> error
    #[test]
    fn invalid_platform() {
        let err = parse_os_version("windows:11")
            .expect_err("windows:11 SHALL be rejected")
            .to_string();
        assert!(
            err.contains("unsupported platform"),
            "expected 'unsupported platform' in: {err}"
        );
    }

    // 12. Missing version -- "ios:" -> error
    #[test]
    fn missing_version() {
        let err = parse_os_version("ios:")
            .expect_err("`ios:` alone SHALL be rejected")
            .to_string();
        assert!(
            err.contains("missing version"),
            "expected 'missing version' in: {err}"
        );
    }

    // 13. Missing colon -- "ios18" -> error
    #[test]
    fn missing_colon() {
        let err = parse_os_version("ios18")
            .expect_err("missing colon SHALL be rejected")
            .to_string();
        assert!(
            err.contains("missing ':'"),
            "expected \"missing ':'\" in: {err}"
        );
    }

    // 14. Negative version -- "android:-1" -> error
    #[test]
    fn negative_version() {
        let result = parse_os_version("android:-1");
        assert!(result.is_err());
    }

    // 15. latest:0 -- "ios:latest:0" -> error
    #[test]
    fn latest_zero() {
        let err = parse_os_version("ios:latest:0")
            .expect_err("`:latest:0` SHALL be rejected")
            .to_string();
        assert!(
            err.contains("latest count must be >= 1"),
            "expected 'latest count must be >= 1' in: {err}"
        );
    }

    // 16. Version with minor -- "ios:18.6" -> treated as major 18
    #[test]
    fn version_with_minor() {
        let spec = parse_os_version("ios:18.6").expect("should parse");
        assert_eq!(
            spec,
            OsVersionSpec::Exact {
                platform: Platform::Ios,
                major: 18,
            }
        );
        assert!(matches_version(&spec, 18));
    }

    // 17. Plus on latest -- "ios:latest+" -> error
    #[test]
    fn plus_on_latest() {
        let err = parse_os_version("ios:latest+")
            .expect_err("`:latest+` SHALL be rejected")
            .to_string();
        assert!(
            err.contains("invalid latest syntax"),
            "expected 'invalid latest syntax' in: {err}"
        );
    }
}
