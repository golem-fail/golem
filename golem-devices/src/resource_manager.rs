//! Resource manager for device and companion port allocation.
//!
//! Tracks which devices are in use and enforces RAM/concurrency limits
//! before allocating new devices. Port scanning and companion discovery
//! are handled by the CLI layer (which has HTTP access via golem-driver).

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;

use crate::concurrency::{can_launch_device, ConcurrencyConfig, RamProvider, SystemRamProvider};
use crate::DeviceInfo;

/// Port range for companion servers.
/// Wide range to support 100+ devices on high-end machines.
pub const PORT_RANGE_START: u16 = 8222;
pub const PORT_RANGE_END: u16 = 8999;

/// An allocated device with its companion port.
#[derive(Debug, Clone)]
pub struct DeviceAllocation {
    pub device: DeviceInfo,
    pub port: u16,
}

/// Manages device allocation and companion port assignment.
///
/// Enforces concurrency and RAM limits. Port scanning is external —
/// the caller provides used ports when requesting a free port.
pub struct ResourceManager {
    config: ConcurrencyConfig,
    ram_provider: Box<dyn RamProvider>,
    /// Active allocations: device UDID → port
    allocations: Mutex<HashMap<String, u16>>,
    /// Devices this suite booted (vs user-booted). Only these get shut down
    /// at suite end. UDID → DeviceInfo so shutdown has what it needs without
    /// a second discovery pass.
    golem_booted: Mutex<HashMap<String, DeviceInfo>>,
    /// Devices currently unavailable — typically rebooting after an ANR
    /// recovery. `find_available_device` filters these out so no new
    /// FlowRun gets allocated while the device is mid-reboot.
    unhealthy: Mutex<std::collections::HashSet<String>>,
}

impl ResourceManager {
    /// Create a new ResourceManager with the given concurrency config.
    pub fn new(config: ConcurrencyConfig) -> Self {
        Self {
            config,
            ram_provider: Box::new(SystemRamProvider),
            allocations: Mutex::new(HashMap::new()),
            golem_booted: Mutex::new(HashMap::new()),
            unhealthy: Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Create a ResourceManager with a custom RAM provider (for testing).
    pub fn with_ram_provider(
        config: ConcurrencyConfig,
        ram_provider: Box<dyn RamProvider>,
    ) -> Self {
        Self {
            config,
            ram_provider,
            allocations: Mutex::new(HashMap::new()),
            golem_booted: Mutex::new(HashMap::new()),
            unhealthy: Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Mark a device unhealthy (e.g. mid-reboot after ANR recovery).
    /// `find_available_device` callers should filter via [`is_unhealthy`].
    pub fn mark_unhealthy(&self, udid: &str) {
        self.unhealthy
            .lock()
            .expect("lock poisoned")
            .insert(udid.to_string());
    }

    /// Mark a previously-unhealthy device as healthy again.
    pub fn mark_healthy(&self, udid: &str) {
        self.unhealthy.lock().expect("lock poisoned").remove(udid);
    }

    /// Whether a device is currently flagged unhealthy.
    pub fn is_unhealthy(&self, udid: &str) -> bool {
        self.unhealthy.lock().expect("lock poisoned").contains(udid)
    }

    /// Find the first port in range not in `used_ports` or already allocated.
    pub fn find_free_port(&self, used_ports: &[u16]) -> Result<u16> {
        let allocations = self.allocations.lock().expect("lock poisoned");
        let allocated_ports: std::collections::HashSet<u16> =
            allocations.values().copied().collect();

        for port in PORT_RANGE_START..=PORT_RANGE_END {
            if !used_ports.contains(&port) && !allocated_ports.contains(&port) {
                return Ok(port);
            }
        }

        Err(golem_events::coded(
            golem_events::FailureCode::HostPortsExhausted,
            anyhow::anyhow!("No free ports in range {PORT_RANGE_START}-{PORT_RANGE_END}"),
        ))
    }

    /// Try to allocate a device. Checks that the device isn't already
    /// allocated, and that RAM and concurrency limits allow it.
    pub fn try_allocate(&self, device: &DeviceInfo, port: u16) -> Result<()> {
        let mut allocations = self.allocations.lock().expect("lock poisoned");

        // Check if device is already allocated (another flow is using it)
        if allocations.contains_key(&device.udid) {
            return Err(golem_events::coded(
                golem_events::FailureCode::DeviceBusy,
                anyhow::anyhow!(
                    "Device {} ({}) is already in use by another flow",
                    device.name,
                    device.udid,
                ),
            ));
        }

        if !can_launch_device(&self.config, allocations.len(), self.ram_provider.as_ref())? {
            return Err(golem_events::coded(
                golem_events::FailureCode::DeviceBusy,
                anyhow::anyhow!(
                    "Cannot allocate device {}: concurrency or RAM limit reached ({} active, min_free_ram={}MB)",
                    device.name,
                    allocations.len(),
                    self.config.min_free_ram_mb,
                ),
            ));
        }

        allocations.insert(device.udid.clone(), port);
        Ok(())
    }

    /// Release a device and its port.
    pub fn release(&self, device_udid: &str) {
        let mut allocations = self.allocations.lock().expect("lock poisoned");
        allocations.remove(device_udid);
    }

    /// How many devices are currently allocated.
    pub fn active_count(&self) -> usize {
        self.allocations.lock().expect("lock poisoned").len()
    }

    /// Get the port for an allocated device.
    pub fn port_for(&self, device_udid: &str) -> Option<u16> {
        self.allocations
            .lock()
            .expect("lock poisoned")
            .get(device_udid)
            .copied()
    }

    /// Record that golem booted (or created) this device. Only tracked
    /// devices are shut down at suite end; user-booted devices are never
    /// touched.
    pub fn mark_golem_booted(&self, device: DeviceInfo) {
        let mut booted = self.golem_booted.lock().expect("lock poisoned");
        booted.insert(device.udid.clone(), device);
    }

    /// Shut down every device this suite booted. Drains the tracking map
    /// so repeat calls are idempotent. Respects `keep_devices`: when true,
    /// the map is drained but no shutdown commands run.
    ///
    /// Errors are collected as warning strings rather than propagated —
    /// shutdown is best-effort (a sim may have died on its own already).
    pub async fn shutdown_golem_booted(&self, keep_devices: bool) -> Vec<String> {
        let devices: Vec<DeviceInfo> = {
            let mut booted = self.golem_booted.lock().expect("lock poisoned");
            booted.drain().map(|(_, d)| d).collect()
        };

        if keep_devices || devices.is_empty() {
            return Vec::new();
        }

        let mut warnings = Vec::new();
        for device in devices {
            if let Err(e) = crate::lifecycle::shutdown_device(&device).await {
                warnings.push(format!("Failed to shutdown {}: {e}", device.name));
            }
        }
        warnings
    }

    /// How many devices are currently tracked as golem-booted. Testing.
    pub fn golem_booted_count(&self) -> usize {
        self.golem_booted.lock().expect("lock poisoned").len()
    }

    /// Read-only snapshot of the retained golem-booted [`DeviceInfo`] for a
    /// UDID, or `None` if that device is not tracked. Testing only.
    #[cfg(test)]
    fn golem_booted_device(&self, udid: &str) -> Option<DeviceInfo> {
        self.golem_booted
            .lock()
            .expect("lock poisoned")
            .get(udid)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concurrency::RamProvider;
    use crate::{DeviceState, DeviceType, Platform};

    struct FixedRamProvider(u64);
    impl RamProvider for FixedRamProvider {
        fn available_ram_mb(&self) -> Result<u64> {
            Ok(self.0)
        }
    }

    fn test_device(name: &str, udid: &str, platform: Platform) -> DeviceInfo {
        DeviceInfo {
            name: name.to_string(),
            udid: udid.to_string(),
            platform,
            device_type: DeviceType::Phone,
            os_major: 17,
            os_version: "17.2".to_string(),
            state: DeviceState::Booted,
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

    #[test]
    fn allocate_and_release() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        let device = test_device("iPhone 15", "uid-1", Platform::Ios);

        rm.try_allocate(&device, 8222)
            .expect("allocation SHALL succeed");
        assert_eq!(rm.active_count(), 1);
        assert_eq!(rm.port_for("uid-1"), Some(8222));

        rm.release("uid-1");
        assert_eq!(rm.active_count(), 0);
        assert_eq!(rm.port_for("uid-1"), None);
    }

    #[test]
    fn allocation_respects_concurrency_limit() {
        let config = ConcurrencyConfig {
            max_concurrency: 2,
            ..ConcurrencyConfig::default()
        };
        let rm = ResourceManager::with_ram_provider(config, Box::new(FixedRamProvider(8192)));

        let d1 = test_device("Device 1", "uid-1", Platform::Ios);
        let d2 = test_device("Device 2", "uid-2", Platform::Android);
        let d3 = test_device("Device 3", "uid-3", Platform::Ios);

        rm.try_allocate(&d1, 8222).expect("first SHALL succeed");
        rm.try_allocate(&d2, 8223).expect("second SHALL succeed");
        let result = rm.try_allocate(&d3, 8224);
        assert!(result.is_err(), "third SHALL fail at max_concurrency=2");
    }

    #[test]
    fn allocation_respects_ram_limit() {
        let config = ConcurrencyConfig {
            min_free_ram_mb: 4096,
            ..ConcurrencyConfig::default()
        };
        let rm = ResourceManager::with_ram_provider(config, Box::new(FixedRamProvider(2048)));
        let device = test_device("Device", "uid-1", Platform::Ios);

        let result = rm.try_allocate(&device, 8222);
        assert!(result.is_err(), "SHALL fail when RAM is below threshold");
    }

    #[test]
    fn release_frees_slot_for_new_allocation() {
        let config = ConcurrencyConfig {
            max_concurrency: 1,
            ..ConcurrencyConfig::default()
        };
        let rm = ResourceManager::with_ram_provider(config, Box::new(FixedRamProvider(8192)));

        let d1 = test_device("Device 1", "uid-1", Platform::Ios);
        let d2 = test_device("Device 2", "uid-2", Platform::Android);

        rm.try_allocate(&d1, 8222).expect("first SHALL succeed");
        assert!(rm.try_allocate(&d2, 8223).is_err(), "second SHALL fail");

        rm.release("uid-1");
        rm.try_allocate(&d2, 8223)
            .expect("second SHALL succeed after release");
    }

    #[tokio::test]
    async fn mark_golem_booted_tracks_device() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        assert_eq!(rm.golem_booted_count(), 0);
        rm.mark_golem_booted(test_device("sim", "uid-1", Platform::Ios));
        assert_eq!(rm.golem_booted_count(), 1);
        rm.mark_golem_booted(test_device("emu", "uid-2", Platform::Android));
        assert_eq!(rm.golem_booted_count(), 2);
    }

    #[tokio::test]
    async fn shutdown_golem_booted_keep_devices_drains_without_running() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        rm.mark_golem_booted(test_device("sim", "uid-1", Platform::Ios));
        rm.mark_golem_booted(test_device("emu", "uid-2", Platform::Android));

        let warnings = rm.shutdown_golem_booted(true).await;
        assert!(
            warnings.is_empty(),
            "keep_devices=true SHALL skip shutdown calls"
        );
        assert_eq!(
            rm.golem_booted_count(),
            0,
            "map SHALL drain even with keep_devices=true"
        );
    }

    #[tokio::test]
    async fn shutdown_golem_booted_empty_is_noop() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        let warnings = rm.shutdown_golem_booted(false).await;
        assert!(warnings.is_empty(), "empty map SHALL produce no warnings");
    }

    #[tokio::test]
    async fn mark_golem_booted_is_independent_of_allocations() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        let d = test_device("sim", "uid-1", Platform::Ios);
        rm.mark_golem_booted(d.clone());
        // Marking as golem-booted SHALL NOT consume an allocation slot.
        assert_eq!(rm.active_count(), 0);
        // Can still allocate the same device afterward.
        rm.try_allocate(&d, 8222).expect("allocate SHALL succeed");
        assert_eq!(rm.active_count(), 1);
        assert_eq!(rm.golem_booted_count(), 1);
    }

    #[test]
    fn multiple_devices_track_different_ports() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );

        let d1 = test_device("iPhone", "uid-1", Platform::Ios);
        let d2 = test_device("Pixel", "uid-2", Platform::Android);

        rm.try_allocate(&d1, 8222).expect("allocate d1");
        rm.try_allocate(&d2, 8225).expect("allocate d2");

        assert_eq!(rm.port_for("uid-1"), Some(8222));
        assert_eq!(rm.port_for("uid-2"), Some(8225));
        assert_eq!(rm.active_count(), 2);
    }

    #[test]
    fn mark_unhealthy_sets_and_clears_flag() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        assert!(!rm.is_unhealthy("uid-1"));
        rm.mark_unhealthy("uid-1");
        assert!(rm.is_unhealthy("uid-1"));
        assert!(!rm.is_unhealthy("uid-2"));
        rm.mark_healthy("uid-1");
        assert!(!rm.is_unhealthy("uid-1"));
    }

    #[test]
    fn unhealthy_persists_across_arc_clones() {
        let rm = std::sync::Arc::new(ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        ));
        let clone = rm.clone();
        clone.mark_unhealthy("uid-1");
        assert!(rm.is_unhealthy("uid-1"));
    }

    // 1. find_free_port returns the very first port in range when nothing is used.
    #[test]
    fn find_free_port_returns_range_start_when_empty() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        let port = rm.find_free_port(&[]).expect("free port SHALL be found");
        assert_eq!(
            port, PORT_RANGE_START,
            "first free port SHALL be range start"
        );
    }

    // 2. find_free_port skips ports listed in used_ports.
    #[test]
    fn find_free_port_skips_used_ports() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        let used = vec![PORT_RANGE_START, PORT_RANGE_START + 1];
        let port = rm.find_free_port(&used).expect("free port SHALL be found");
        assert_eq!(
            port,
            PORT_RANGE_START + 2,
            "SHALL skip externally-used ports"
        );
    }

    // 3. find_free_port skips ports already allocated by this manager.
    #[test]
    fn find_free_port_skips_allocated_ports() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        let d = test_device("dev", "uid-1", Platform::Ios);
        rm.try_allocate(&d, PORT_RANGE_START)
            .expect("allocate SHALL succeed");
        let port = rm.find_free_port(&[]).expect("free port SHALL be found");
        assert_eq!(
            port,
            PORT_RANGE_START + 1,
            "SHALL skip already-allocated ports"
        );
    }

    // 4. find_free_port considers both used_ports and allocated ports together.
    #[test]
    fn find_free_port_skips_used_and_allocated_combined() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        let d = test_device("dev", "uid-1", Platform::Ios);
        rm.try_allocate(&d, PORT_RANGE_START + 1)
            .expect("allocate SHALL succeed");
        // start is externally used, start+1 is allocated → first free is start+2.
        let port = rm
            .find_free_port(&[PORT_RANGE_START])
            .expect("free port SHALL be found");
        assert_eq!(
            port,
            PORT_RANGE_START + 2,
            "SHALL combine used and allocated exclusions"
        );
    }

    // 5. find_free_port errors with HostPortsExhausted when whole range is used.
    #[test]
    fn find_free_port_exhausted_returns_coded_error() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        let all_ports: Vec<u16> = (PORT_RANGE_START..=PORT_RANGE_END).collect();
        let err = rm
            .find_free_port(&all_ports)
            .expect_err("SHALL error when no ports remain");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::HostPortsExhausted),
            "exhaustion SHALL carry HostPortsExhausted code",
        );
    }

    // 6. try_allocate of an already-allocated device fails with DeviceBusy.
    #[test]
    fn try_allocate_duplicate_device_is_device_busy() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        let d = test_device("dev", "uid-1", Platform::Ios);
        rm.try_allocate(&d, 8222)
            .expect("first allocate SHALL succeed");
        let err = rm
            .try_allocate(&d, 8223)
            .expect_err("re-allocating same device SHALL fail");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::DeviceBusy),
            "duplicate allocation SHALL carry DeviceBusy code",
        );
        // The original allocation SHALL be untouched (still on its first port).
        assert_eq!(rm.port_for("uid-1"), Some(8222));
        assert_eq!(
            rm.active_count(),
            1,
            "failed re-allocate SHALL NOT add a slot"
        );
    }

    // 7. try_allocate failing on the concurrency limit carries DeviceBusy code.
    #[test]
    fn try_allocate_concurrency_limit_carries_device_busy_code() {
        let config = ConcurrencyConfig {
            max_concurrency: 1,
            ..ConcurrencyConfig::default()
        };
        let rm = ResourceManager::with_ram_provider(config, Box::new(FixedRamProvider(8192)));
        let d1 = test_device("d1", "uid-1", Platform::Ios);
        let d2 = test_device("d2", "uid-2", Platform::Android);
        rm.try_allocate(&d1, 8222).expect("first SHALL succeed");
        let err = rm.try_allocate(&d2, 8223).expect_err("second SHALL fail");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::DeviceBusy),
            "limit rejection SHALL carry DeviceBusy code",
        );
    }

    // 8. port_for an unknown device returns None.
    #[test]
    fn port_for_unknown_device_is_none() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        assert_eq!(
            rm.port_for("nope"),
            None,
            "unknown device SHALL have no port"
        );
    }

    // 9. release of a device that was never allocated is a no-op.
    #[test]
    fn release_unknown_device_is_noop() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        let d = test_device("dev", "uid-1", Platform::Ios);
        rm.try_allocate(&d, 8222).expect("allocate SHALL succeed");
        rm.release("not-allocated");
        assert_eq!(
            rm.active_count(),
            1,
            "releasing unknown udid SHALL NOT drop a slot"
        );
        assert_eq!(rm.port_for("uid-1"), Some(8222));
    }

    // 10. mark_golem_booted with a repeated UDID overwrites, not duplicates.
    #[test]
    fn mark_golem_booted_same_udid_does_not_double_count() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        rm.mark_golem_booted(test_device("sim-a", "uid-1", Platform::Ios));
        rm.mark_golem_booted(test_device("sim-b", "uid-1", Platform::Ios));
        assert_eq!(
            rm.golem_booted_count(),
            1,
            "same UDID SHALL overwrite, not duplicate"
        );
    }

    // 11. mark_healthy removes only the named udid, leaving other unhealthy entries intact.
    #[test]
    fn mark_healthy_unknown_device_removes_only_named_key() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        // A genuinely-unhealthy device must survive a mark_healthy targeting some other udid.
        rm.mark_unhealthy("uid-1");
        rm.mark_healthy("never-seen");
        assert!(
            rm.is_unhealthy("uid-1"),
            "mark_healthy of an unrelated udid SHALL NOT clear other unhealthy entries",
        );
        assert!(
            !rm.is_unhealthy("never-seen"),
            "unknown device SHALL remain healthy",
        );
    }

    // 13. golem_booted_device returns the retained DeviceInfo for a tracked udid.
    #[test]
    fn golem_booted_device_returns_retained_info() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        rm.mark_golem_booted(test_device("sim", "uid-1", Platform::Ios));
        let retained = rm
            .golem_booted_device("uid-1")
            .expect("tracked device SHALL be retrievable");
        assert_eq!(retained.udid, "uid-1", "retained udid SHALL match");
        assert_eq!(retained.name, "sim", "retained name SHALL match");
        assert_eq!(
            retained.platform,
            Platform::Ios,
            "retained platform SHALL match"
        );
    }

    // 14. golem_booted_device returns None for a udid that was never booted.
    #[test]
    fn golem_booted_device_unknown_udid_is_none() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        rm.mark_golem_booted(test_device("sim", "uid-1", Platform::Ios));
        assert!(
            rm.golem_booted_device("nope").is_none(),
            "untracked udid SHALL have no retained DeviceInfo",
        );
    }

    // 15. golem_booted_device reflects overwrite semantics for a repeated udid.
    #[test]
    fn golem_booted_device_reflects_overwrite() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        rm.mark_golem_booted(test_device("sim-a", "uid-1", Platform::Ios));
        rm.mark_golem_booted(test_device("sim-b", "uid-1", Platform::Android));
        let retained = rm
            .golem_booted_device("uid-1")
            .expect("tracked device SHALL be retrievable");
        assert_eq!(
            retained.name, "sim-b",
            "overwrite SHALL return the latest DeviceInfo"
        );
        assert_eq!(
            retained.platform,
            Platform::Android,
            "overwrite SHALL return the latest platform",
        );
    }

    // 16. shutdown_golem_booted drains the map so the accessor reports None after.
    #[tokio::test]
    async fn golem_booted_device_none_after_drain() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        rm.mark_golem_booted(test_device("sim", "uid-1", Platform::Ios));
        assert!(
            rm.golem_booted_device("uid-1").is_some(),
            "device SHALL be tracked pre-drain"
        );
        let _ = rm.shutdown_golem_booted(true).await;
        assert!(
            rm.golem_booted_device("uid-1").is_none(),
            "drained map SHALL retain no DeviceInfo",
        );
    }

    // 12. unhealthy flag is independent from allocation state.
    #[test]
    fn unhealthy_flag_independent_of_allocation() {
        let rm = ResourceManager::with_ram_provider(
            ConcurrencyConfig::default(),
            Box::new(FixedRamProvider(8192)),
        );
        let d = test_device("dev", "uid-1", Platform::Ios);
        rm.try_allocate(&d, 8222).expect("allocate SHALL succeed");
        rm.mark_unhealthy("uid-1");
        // Marking unhealthy SHALL NOT alter allocation bookkeeping.
        assert_eq!(rm.active_count(), 1);
        assert_eq!(rm.port_for("uid-1"), Some(8222));
        assert!(rm.is_unhealthy("uid-1"));
    }
}
