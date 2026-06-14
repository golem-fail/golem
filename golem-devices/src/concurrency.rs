//! RAM-aware adaptive concurrency for device launches.
//!
//! Before launching a new simulator or emulator the system checks available
//! system RAM and enforces a maximum concurrency limit.  Launches are
//! staggered so the host is never overwhelmed.

use anyhow::Result;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Tunables for adaptive device concurrency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcurrencyConfig {
    /// Hard upper bound on simultaneously running devices.
    pub max_concurrency: usize,
    /// Minimum free RAM (MB) required before launching another device.
    pub min_free_ram_mb: u64,
    /// Minimum free disk space (MB) required before creating a new device.
    /// Default: 50,000 MB (50 GB) — safe for dev laptops. CI can lower this.
    pub min_free_disk_mb: u64,
    /// Minimum delay in milliseconds between successive launches.
    pub launch_delay_ms: u64,
}

impl Default for ConcurrencyConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            min_free_ram_mb: 2048,
            min_free_disk_mb: 50_000,
            launch_delay_ms: 2000,
        }
    }
}

// ---------------------------------------------------------------------------
// RAM provider (injectable for testing)
// ---------------------------------------------------------------------------

/// Trait that abstracts over reading available system memory.
///
/// Implement this for real OS queries *and* for deterministic test doubles.
pub trait RamProvider: Send + Sync {
    /// Return currently available RAM in megabytes.
    fn available_ram_mb(&self) -> Result<u64>;
}

/// Default provider that queries the real operating system.
pub struct SystemRamProvider;

impl RamProvider for SystemRamProvider {
    fn available_ram_mb(&self) -> Result<u64> {
        get_available_ram_mb()
    }
}

// ---------------------------------------------------------------------------
// Real RAM query
// ---------------------------------------------------------------------------

/// Check current available system RAM in megabytes.
///
/// * **macOS** – parses `vm_stat` page-size and free/inactive/speculative
///   counts.
/// * **Linux** – reads `MemAvailable` from `/proc/meminfo`.
/// * **Other** – returns a generous default so nothing blocks.
pub fn get_available_ram_mb() -> Result<u64> {
    #[cfg(target_os = "macos")]
    {
        get_available_ram_mb_macos()
    }
    #[cfg(target_os = "linux")]
    {
        get_available_ram_mb_linux()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // Generous fallback so we never block on unsupported platforms.
        Ok(16384)
    }
}

#[cfg(target_os = "macos")]
fn get_available_ram_mb_macos() -> Result<u64> {
    use std::process::Command;

    let output = Command::new("vm_stat").output()?;
    let text = String::from_utf8_lossy(&output.stdout);

    // First line: "Mach Virtual Memory Statistics: (page size of NNNN bytes)"
    let page_size: u64 = text
        .lines()
        .next()
        .and_then(|line| {
            line.split("page size of ")
                .nth(1)
                .and_then(|s| s.trim_end_matches(" bytes)").parse().ok())
        })
        .unwrap_or(16384);

    let page_value = |key: &str| -> u64 {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| {
                l.split(':')
                    .nth(1)
                    .map(|v| v.trim().trim_end_matches('.'))
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .unwrap_or(0)
    };

    let free = page_value("Pages free");
    let inactive = page_value("Pages inactive");
    let speculative = page_value("Pages speculative");

    let available_bytes = (free + inactive + speculative) * page_size;
    Ok(available_bytes / (1024 * 1024))
}

#[cfg(target_os = "linux")]
fn get_available_ram_mb_linux() -> Result<u64> {
    let contents = std::fs::read_to_string("/proc/meminfo")?;
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("MemAvailable:") {
            let kb: u64 = rest
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("failed to parse MemAvailable: {e}"))?;
            return Ok(kb / 1024);
        }
    }
    anyhow::bail!("MemAvailable not found in /proc/meminfo")
}

// ---------------------------------------------------------------------------
// Disk space query
// ---------------------------------------------------------------------------

/// Check current available disk space in megabytes on the volume containing
/// the current working directory.
///
/// Uses `statvfs` on macOS and Linux.
pub fn get_available_disk_mb() -> Result<u64> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let path = CString::new(".").expect("CString::new failed");
    let mut stat = MaybeUninit::<libc::statvfs>::uninit();

    let result = unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) };
    if result != 0 {
        anyhow::bail!("statvfs failed: {}", std::io::Error::last_os_error());
    }

    let stat = unsafe { stat.assume_init() };
    let available_bytes = stat.f_bavail as u64 * stat.f_frsize;
    Ok(available_bytes / (1024 * 1024))
}

/// Check whether there is sufficient disk space to create a new device.
///
/// Returns `true` if available disk minus `estimated_device_size_mb` is
/// still above `config.min_free_disk_mb`.
pub fn has_sufficient_disk(config: &ConcurrencyConfig, estimated_device_size_mb: u64) -> Result<bool> {
    let available = get_available_disk_mb()?;
    Ok(available >= config.min_free_disk_mb + estimated_device_size_mb)
}

// ---------------------------------------------------------------------------
// Resource snapshot
// ---------------------------------------------------------------------------

/// System-resource availability at a point in time. Distinct from
/// `PerfSnapshot` (which measures *app footprint* — how much RAM/disk
/// the app under test is consuming). `ResourceSnapshot` measures *system
/// headroom* — how much capacity remains on the host and on the device.
///
/// Used by:
/// - Pre-boot gating (do we have room for another sim/emu?)
/// - Recovery messaging (was low disk a contributing cause?)
/// - On-failure capture (host pressure when a step timed out?)
///
/// All fields are `Option` so partial captures (e.g. host-only, device
/// unreachable) still record the data we *can* get.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceSnapshot {
    pub host_free_disk_mb: Option<u64>,
    pub host_free_ram_mb: Option<u64>,
    /// `/data` free space on a specific Android device when udid passed.
    /// None for host-only captures or unreachable devices.
    pub device_free_disk_mb: Option<u64>,
}

impl ResourceSnapshot {
    /// Capture host metrics only. Used pre-boot and at host-level failure
    /// points (no device context).
    pub async fn capture_host() -> Self {
        Self {
            host_free_disk_mb: get_available_disk_mb().ok(),
            host_free_ram_mb: get_available_ram_mb().ok(),
            device_free_disk_mb: None,
        }
    }

    /// Capture host + the given device's `/data` partition free space.
    /// Android-only for now; iOS recovery can extend with a separate path
    /// (`xcrun simctl` doesn't expose free space directly).
    pub async fn capture_with_android_device(udid: &str) -> Self {
        let mut snap = Self::capture_host().await;
        snap.device_free_disk_mb = android_device_free_disk_mb(udid).await;
        snap
    }

    /// iOS simulator data lives on the host filesystem (under
    /// `~/Library/Developer/CoreSimulator/Devices/<UDID>/data`), so the
    /// simulator's "free space" equals the host's. Mirror the host
    /// value into `device_free_disk_mb` so consumers can read the same
    /// field regardless of platform. Physical iOS is a different story
    /// (needs `xcrun devicectl device info storage` or similar) and is
    /// roadmapped separately.
    pub async fn capture_with_ios_simulator() -> Self {
        let snap = Self::capture_host().await;
        Self {
            device_free_disk_mb: snap.host_free_disk_mb,
            ..snap
        }
    }
}

/// Free MiB on `/data` on the given Android device, via `adb shell df`.
/// Returns None if the device is unreachable or output unparseable.
async fn android_device_free_disk_mb(udid: &str) -> Option<u64> {
    let out = tokio::process::Command::new("adb")
        .args(["-s", udid, "shell", "df", "-k", "/data"])
        .output()
        .await
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().nth(1)?;
    let avail_kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(avail_kb / 1024)
}

// ---------------------------------------------------------------------------
// Decision logic
// ---------------------------------------------------------------------------

/// Determine whether it is safe to launch another device.
///
/// Returns `false` when either:
/// * `current_running` already equals `config.max_concurrency`, or
/// * the `ram_provider` reports less free RAM than `config.min_free_ram_mb`.
pub fn can_launch_device(
    config: &ConcurrencyConfig,
    current_running: usize,
    ram_provider: &dyn RamProvider,
) -> Result<bool> {
    if current_running >= config.max_concurrency {
        return Ok(false);
    }
    let available = ram_provider.available_ram_mb()?;
    Ok(available >= config.min_free_ram_mb)
}

// ---------------------------------------------------------------------------
// Launch planning
// ---------------------------------------------------------------------------

/// A computed plan describing the order and parallelism of device launches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// Device IDs in the order they should be launched.
    pub device_order: Vec<String>,
    /// Maximum number of devices to run in parallel.
    pub max_parallel: usize,
}

/// Build a [`LaunchPlan`] from a list of device identifiers and a
/// [`ConcurrencyConfig`].
///
/// The plan preserves the caller-supplied ordering and caps parallelism at the
/// lesser of `config.max_concurrency` and the total device count.
pub fn plan_launches(device_ids: &[String], config: &ConcurrencyConfig) -> LaunchPlan {
    LaunchPlan {
        device_order: device_ids.to_vec(),
        max_parallel: config.max_concurrency.min(device_ids.len()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Test doubles -------------------------------------------------------

    /// A [`RamProvider`] that always returns a fixed value.
    struct FixedRamProvider(u64);

    impl RamProvider for FixedRamProvider {
        fn available_ram_mb(&self) -> Result<u64> {
            Ok(self.0)
        }
    }

    /// A [`RamProvider`] that always fails.
    struct FailingRamProvider;

    impl RamProvider for FailingRamProvider {
        fn available_ram_mb(&self) -> Result<u64> {
            anyhow::bail!("simulated RAM query failure")
        }
    }

    // -- Helpers ------------------------------------------------------------

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    // -- Tests --------------------------------------------------------------

    // 1. Default configuration values.
    #[test]
    fn default_config_has_expected_values() {
        let cfg = ConcurrencyConfig::default();
        assert_eq!(cfg.max_concurrency, 4);
        assert_eq!(cfg.min_free_ram_mb, 2048);
        assert_eq!(cfg.min_free_disk_mb, 50_000);
        assert_eq!(cfg.launch_delay_ms, 2000);
    }

    // 2. At max concurrency → cannot launch.
    #[test]
    fn cannot_launch_when_at_max_concurrency() {
        let cfg = ConcurrencyConfig::default();
        let ram = FixedRamProvider(8192);
        let result = can_launch_device(&cfg, 4, &ram).expect("should not error");
        assert!(!result);
    }

    // 3. RAM too low → cannot launch.
    #[test]
    fn cannot_launch_when_ram_too_low() {
        let cfg = ConcurrencyConfig {
            min_free_ram_mb: 4096,
            ..ConcurrencyConfig::default()
        };
        let ram = FixedRamProvider(2048); // below threshold
        let result = can_launch_device(&cfg, 0, &ram).expect("should not error");
        assert!(!result);
    }

    // 4. Enough RAM and capacity → can launch.
    #[test]
    fn can_launch_when_capacity_and_ram_available() {
        let cfg = ConcurrencyConfig::default();
        let ram = FixedRamProvider(8192);
        let result = can_launch_device(&cfg, 2, &ram).expect("should not error");
        assert!(result);
    }

    // 5. plan_launches respects max_concurrency.
    #[test]
    fn plan_launches_caps_at_max_concurrency() {
        let cfg = ConcurrencyConfig {
            max_concurrency: 2,
            ..ConcurrencyConfig::default()
        };
        let plan = plan_launches(&ids(&["a", "b", "c", "d"]), &cfg);
        assert_eq!(plan.max_parallel, 2);
    }

    // 6. Fewer devices than max_concurrency.
    #[test]
    fn plan_launches_with_fewer_devices_than_max() {
        let cfg = ConcurrencyConfig {
            max_concurrency: 8,
            ..ConcurrencyConfig::default()
        };
        let plan = plan_launches(&ids(&["x", "y"]), &cfg);
        assert_eq!(plan.max_parallel, 2);
    }

    // 7. Device order is preserved.
    #[test]
    fn plan_launches_preserves_device_order() {
        let cfg = ConcurrencyConfig::default();
        let input = ids(&["delta", "alpha", "charlie", "bravo"]);
        let plan = plan_launches(&input, &cfg);
        assert_eq!(plan.device_order, input);
    }

    // 8. Empty device list produces empty plan.
    #[test]
    fn plan_launches_empty_device_list() {
        let cfg = ConcurrencyConfig::default();
        let plan = plan_launches(&[], &cfg);
        assert!(plan.device_order.is_empty());
        assert_eq!(plan.max_parallel, 0);
    }

    // 9. max_parallel is min(max_concurrency, device_count).
    #[test]
    fn max_parallel_is_min_of_concurrency_and_count() {
        let cfg = ConcurrencyConfig {
            max_concurrency: 3,
            ..ConcurrencyConfig::default()
        };
        // Exactly at max
        let plan = plan_launches(&ids(&["a", "b", "c"]), &cfg);
        assert_eq!(plan.max_parallel, 3);

        // Below max
        let plan = plan_launches(&ids(&["a"]), &cfg);
        assert_eq!(plan.max_parallel, 1);

        // Above max
        let plan = plan_launches(&ids(&["a", "b", "c", "d", "e"]), &cfg);
        assert_eq!(plan.max_parallel, 3);
    }

    // 10. RAM provider error propagates through can_launch_device.
    #[test]
    fn ram_provider_error_propagates() {
        let cfg = ConcurrencyConfig::default();
        let result = can_launch_device(&cfg, 0, &FailingRamProvider);
        assert!(result.is_err());
    }

    // 11. Exactly at RAM threshold → can launch (boundary).
    #[test]
    fn can_launch_at_exact_ram_boundary() {
        let cfg = ConcurrencyConfig {
            min_free_ram_mb: 2048,
            ..ConcurrencyConfig::default()
        };
        let ram = FixedRamProvider(2048); // exactly at threshold
        let result = can_launch_device(&cfg, 0, &ram).expect("should not error");
        assert!(result);
    }

    // 12. One below RAM threshold → cannot launch (boundary).
    #[test]
    fn cannot_launch_one_below_ram_boundary() {
        let cfg = ConcurrencyConfig {
            min_free_ram_mb: 2048,
            ..ConcurrencyConfig::default()
        };
        let ram = FixedRamProvider(2047);
        let result = can_launch_device(&cfg, 0, &ram).expect("should not error");
        assert!(!result);
    }

    // 13. Over max concurrency also blocked (not just equal).
    #[test]
    fn cannot_launch_when_over_max_concurrency() {
        let cfg = ConcurrencyConfig {
            max_concurrency: 2,
            ..ConcurrencyConfig::default()
        };
        let ram = FixedRamProvider(8192);
        let result = can_launch_device(&cfg, 5, &ram).expect("should not error");
        assert!(!result);
    }

    // 14. get_available_ram_mb returns a plausible value on the host OS.
    #[test]
    fn get_available_ram_mb_returns_positive_value() {
        let mb = get_available_ram_mb().expect("should query system RAM");
        assert!(mb > 0, "available RAM SHALL be positive, got {mb}");
    }

    // 15. SystemRamProvider implements RamProvider correctly.
    #[test]
    fn system_ram_provider_returns_positive_value() {
        let provider = SystemRamProvider;
        let mb = provider.available_ram_mb().expect("should query system RAM");
        assert!(mb > 0, "available RAM SHALL be positive, got {mb}");
    }

    // 16. ConcurrencyConfig equality.
    #[test]
    fn concurrency_config_equality() {
        let a = ConcurrencyConfig::default();
        let b = ConcurrencyConfig::default();
        assert_eq!(a, b);

        let c = ConcurrencyConfig {
            max_concurrency: 8,
            ..ConcurrencyConfig::default()
        };
        assert_ne!(a, c);
    }

    // 17. LaunchPlan equality.
    #[test]
    fn launch_plan_equality() {
        let cfg = ConcurrencyConfig::default();
        let p1 = plan_launches(&ids(&["a", "b"]), &cfg);
        let p2 = plan_launches(&ids(&["a", "b"]), &cfg);
        assert_eq!(p1, p2);
    }

    // 18. plan_launches threads the caller's device order verbatim into
    //     device_order (not sorted/dedup'd), while max_parallel stays equal
    //     across both inputs of the same length.
    #[test]
    fn launch_plan_inequality_on_different_order() {
        let cfg = ConcurrencyConfig::default();
        let p1 = plan_launches(&ids(&["a", "b"]), &cfg);
        let p2 = plan_launches(&ids(&["b", "a"]), &cfg);
        // 1. Each plan's device_order matches its own input verbatim.
        assert_eq!(p1.device_order, ids(&["a", "b"]));
        assert_eq!(p2.device_order, ids(&["b", "a"]));
        // 2. Same length → identical parallelism cap regardless of order.
        assert_eq!(p1.max_parallel, p2.max_parallel);
    }

    // 19. get_available_disk_mb returns a plausible value on the host OS.
    #[test]
    fn get_available_disk_mb_returns_positive_value() {
        let mb = get_available_disk_mb().expect("should query disk space");
        assert!(mb > 0, "available disk SHALL be positive, got {mb}");
    }

    // 20. has_sufficient_disk is true when threshold + estimate is trivially
    //     small (zero minimum, zero estimated device size).
    #[test]
    fn has_sufficient_disk_true_when_requirement_trivial() {
        let cfg = ConcurrencyConfig {
            min_free_disk_mb: 0,
            ..ConcurrencyConfig::default()
        };
        let ok = has_sufficient_disk(&cfg, 0).expect("should query disk space");
        assert!(ok, "any nonzero free disk SHALL satisfy a zero requirement");
    }

    // 21. has_sufficient_disk is false when the minimum requirement exceeds
    //     any conceivable free space. min_free_disk_mb + estimate sums to
    //     exactly u64::MAX here (no overflow), so the comparison
    //     `available >= u64::MAX` is what fails the check.
    #[test]
    fn has_sufficient_disk_false_when_requirement_unsatisfiable() {
        let cfg = ConcurrencyConfig {
            min_free_disk_mb: u64::MAX - 1,
            ..ConcurrencyConfig::default()
        };
        let ok = has_sufficient_disk(&cfg, 1).expect("should query disk space");
        assert!(
            !ok,
            "a requirement larger than the volume SHALL fail the check"
        );
    }

    // 23. capture_host populates host fields and leaves device field None.
    #[tokio::test]
    async fn capture_host_records_host_only() {
        let snap = ResourceSnapshot::capture_host().await;
        assert!(
            snap.host_free_disk_mb.is_some(),
            "host disk SHALL be captured"
        );
        assert!(
            snap.host_free_ram_mb.is_some(),
            "host RAM SHALL be captured"
        );
        assert_eq!(
            snap.device_free_disk_mb, None,
            "host-only capture SHALL leave device field None"
        );
    }

    // 24. capture_with_ios_simulator mirrors host disk into the device field.
    #[tokio::test]
    async fn capture_with_ios_simulator_mirrors_host_disk() {
        let snap = ResourceSnapshot::capture_with_ios_simulator().await;
        assert_eq!(
            snap.device_free_disk_mb, snap.host_free_disk_mb,
            "iOS simulator device disk SHALL mirror host disk"
        );
    }
}
