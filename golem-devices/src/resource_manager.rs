//! Resource manager for device and companion port allocation.
//!
//! Tracks which devices are in use and enforces RAM/concurrency limits
//! before allocating new devices. Port scanning and companion discovery
//! are handled by the CLI layer (which has HTTP access via golem-driver).

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{bail, Result};

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
}

impl ResourceManager {
    /// Create a new ResourceManager with the given concurrency config.
    pub fn new(config: ConcurrencyConfig) -> Self {
        Self {
            config,
            ram_provider: Box::new(SystemRamProvider),
            allocations: Mutex::new(HashMap::new()),
        }
    }

    /// Create a ResourceManager with a custom RAM provider (for testing).
    pub fn with_ram_provider(config: ConcurrencyConfig, ram_provider: Box<dyn RamProvider>) -> Self {
        Self {
            config,
            ram_provider,
            allocations: Mutex::new(HashMap::new()),
        }
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

        bail!("No free ports in range {PORT_RANGE_START}-{PORT_RANGE_END}")
    }

    /// Try to allocate a device. Checks that the device isn't already
    /// allocated, and that RAM and concurrency limits allow it.
    pub fn try_allocate(&self, device: &DeviceInfo, port: u16) -> Result<()> {
        let mut allocations = self.allocations.lock().expect("lock poisoned");

        // Check if device is already allocated (another flow is using it)
        if allocations.contains_key(&device.udid) {
            bail!(
                "Device {} ({}) is already in use by another flow",
                device.name,
                device.udid,
            );
        }

        if !can_launch_device(&self.config, allocations.len(), self.ram_provider.as_ref())? {
            bail!(
                "Cannot allocate device {}: concurrency or RAM limit reached ({} active, min_free_ram={}MB)",
                device.name,
                allocations.len(),
                self.config.min_free_ram_mb,
            );
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
        self.allocations.lock().expect("lock poisoned").get(device_udid).copied()
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

        rm.try_allocate(&device, 8222).expect("allocation SHALL succeed");
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
        rm.try_allocate(&d2, 8223).expect("second SHALL succeed after release");
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
}
