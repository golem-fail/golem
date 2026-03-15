use std::collections::HashMap;

use golem_devices::{DeviceInfo, DeviceType};
use golem_vars::VarValue;

/// Represents the `_each` context for one iteration of a `for_each` block.
///
/// When a block specifies `for_each = "client"`, the runner iterates over each
/// device assigned to the "client" app. Each iteration produces an `EachContext`
/// that makes the target device's variables available via `${_each.*}` references.
pub struct EachContext {
    /// The UDID / unique identifier of the iteration device.
    pub device_id: String,
    /// The human-readable name of the iteration device.
    pub device_name: String,
    /// The OS version string of the iteration device (e.g. "17.2").
    pub os_version: String,
    /// Snapshot of the iteration device's variable store at the time of iteration.
    pub vars: HashMap<String, VarValue>,
}

/// Build the list of [`EachContext`] entries for a `for_each` block.
///
/// One context is created per device in `target_app_devices`, in the same order.
/// Each context captures the device's current variables from `device_vars`.
pub fn build_each_contexts(
    target_app_devices: &[DeviceInfo],
    device_vars: &HashMap<String, HashMap<String, VarValue>>,
) -> Vec<EachContext> {
    target_app_devices
        .iter()
        .map(|device| {
            let vars = device_vars
                .get(&device.udid)
                .cloned()
                .unwrap_or_default();
            EachContext {
                device_id: device.udid.clone(),
                device_name: device.name.clone(),
                os_version: device.os_version.clone(),
                vars,
            }
        })
        .collect()
}

/// A constraint that filters which devices of an app execute a block.
///
/// Parsed from the `[block.where]` table in a flow file. Only devices matching
/// **all** specified constraints will run the block; unspecified fields match
/// everything.
pub struct WhereFilter {
    /// If set, only devices of this type run the block.
    pub device_type: Option<DeviceType>,
    /// If set, only devices whose `os_version` starts with this string run the block.
    pub os: Option<String>,
    /// If set, only devices with this exact name run the block.
    pub name: Option<String>,
    /// If set, only devices matching this physical flag run the block.
    pub physical: Option<bool>,
}

impl WhereFilter {
    /// Parse a `WhereFilter` from a string-keyed map (as deserialized from TOML).
    ///
    /// Recognised keys:
    /// - `"type"` → mapped to [`DeviceType`] (`"phone"` or `"tablet"`)
    /// - `"os"`   → stored as a prefix-match string
    /// - `"name"` → exact device name match
    /// - `"physical"` → `"true"` or `"false"`
    ///
    /// Unknown keys are silently ignored.
    pub fn from_map(map: &HashMap<String, String>) -> Self {
        let device_type = map.get("type").and_then(|t| match t.as_str() {
            "phone" => Some(DeviceType::Phone),
            "tablet" => Some(DeviceType::Tablet),
            _ => None,
        });

        let os = map.get("os").cloned();
        let name = map.get("name").cloned();
        let physical = map.get("physical").and_then(|v| v.parse::<bool>().ok());

        Self {
            device_type,
            os,
            name,
            physical,
        }
    }

    /// Check whether a device satisfies all constraints in this filter.
    ///
    /// A `None` constraint is treated as "match any".
    pub fn matches(&self, device: &DeviceInfo) -> bool {
        if let Some(dt) = self.device_type {
            if device.device_type != dt {
                return false;
            }
        }
        if let Some(ref os) = self.os {
            if !device.os_version.starts_with(os.as_str()) {
                return false;
            }
        }
        if let Some(ref name) = self.name {
            if device.name != *name {
                return false;
            }
        }
        if let Some(physical) = self.physical {
            if device.physical != physical {
                return false;
            }
        }
        true
    }
}

/// Filter a slice of devices, returning only those that match the given filter.
///
/// The returned references preserve the input ordering.
pub fn filter_devices<'a>(
    devices: &'a [DeviceInfo],
    filter: &WhereFilter,
) -> Vec<&'a DeviceInfo> {
    devices.iter().filter(|d| filter.matches(d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_devices::{DeviceState, Platform};

    /// Helper to build a [`DeviceInfo`] with sensible defaults that can be
    /// overridden per-test via the returned mutable value.
    fn make_device(name: &str, udid: &str) -> DeviceInfo {
        DeviceInfo {
            name: name.to_string(),
            udid: udid.to_string(),
            platform: Platform::Ios,
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

    // ---------------------------------------------------------------
    // 1. build_each_contexts creates one context per device
    // ---------------------------------------------------------------
    #[test]
    fn build_each_contexts_creates_one_context_per_device() {
        let devices = vec![
            make_device("iPhone 15", "uid-1"),
            make_device("iPhone 14", "uid-2"),
            make_device("iPhone 13", "uid-3"),
        ];
        let device_vars: HashMap<String, HashMap<String, VarValue>> = HashMap::new();

        let contexts = build_each_contexts(&devices, &device_vars);
        assert_eq!(contexts.len(), 3);
    }

    // ---------------------------------------------------------------
    // 2. build_each_contexts includes device vars
    // ---------------------------------------------------------------
    #[test]
    fn build_each_contexts_includes_device_vars() {
        let devices = vec![make_device("iPhone 15", "uid-1")];
        let mut device_vars: HashMap<String, HashMap<String, VarValue>> = HashMap::new();
        let mut vars = HashMap::new();
        vars.insert("quote_ref".to_string(), VarValue::string("QR-001"));
        vars.insert("amount".to_string(), VarValue::string("500"));
        device_vars.insert("uid-1".to_string(), vars);

        let contexts = build_each_contexts(&devices, &device_vars);
        assert_eq!(contexts.len(), 1);
        assert_eq!(
            contexts[0].vars.get("quote_ref"),
            Some(&VarValue::string("QR-001"))
        );
        assert_eq!(
            contexts[0].vars.get("amount"),
            Some(&VarValue::string("500"))
        );
    }

    // ---------------------------------------------------------------
    // 3. build_each_contexts with empty vars creates empty var maps
    // ---------------------------------------------------------------
    #[test]
    fn build_each_contexts_with_empty_vars_creates_empty_maps() {
        let devices = vec![
            make_device("Pixel 8", "uid-a"),
            make_device("Pixel 7", "uid-b"),
        ];
        let device_vars: HashMap<String, HashMap<String, VarValue>> = HashMap::new();

        let contexts = build_each_contexts(&devices, &device_vars);
        assert!(contexts[0].vars.is_empty());
        assert!(contexts[1].vars.is_empty());
    }

    // ---------------------------------------------------------------
    // 4. WhereFilter matches device type phone
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_matches_device_type_phone() {
        let filter = WhereFilter {
            device_type: Some(DeviceType::Phone),
            os: None,
            name: None,
            physical: None,
        };

        let phone = make_device("iPhone 15", "uid-1");
        assert!(filter.matches(&phone));

        let mut tablet = make_device("iPad Pro", "uid-2");
        tablet.device_type = DeviceType::Tablet;
        assert!(!filter.matches(&tablet));
    }

    // ---------------------------------------------------------------
    // 5. WhereFilter matches device type tablet
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_matches_device_type_tablet() {
        let filter = WhereFilter {
            device_type: Some(DeviceType::Tablet),
            os: None,
            name: None,
            physical: None,
        };

        let mut tablet = make_device("iPad Pro", "uid-2");
        tablet.device_type = DeviceType::Tablet;
        assert!(filter.matches(&tablet));

        let phone = make_device("iPhone 15", "uid-1");
        assert!(!filter.matches(&phone));
    }

    // ---------------------------------------------------------------
    // 6. WhereFilter with no constraints matches all
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_no_constraints_matches_all() {
        let filter = WhereFilter {
            device_type: None,
            os: None,
            name: None,
            physical: None,
        };

        let phone = make_device("iPhone 15", "uid-1");
        let mut tablet = make_device("iPad Pro", "uid-2");
        tablet.device_type = DeviceType::Tablet;

        assert!(filter.matches(&phone));
        assert!(filter.matches(&tablet));
    }

    // ---------------------------------------------------------------
    // 7. WhereFilter with os constraint filters correctly
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_os_constraint_filters_correctly() {
        let filter = WhereFilter {
            device_type: None,
            os: Some("17".to_string()),
            name: None,
            physical: None,
        };

        let mut ios17 = make_device("iPhone 15", "uid-1");
        ios17.os_version = "17.2".to_string();
        assert!(filter.matches(&ios17));

        let mut ios16 = make_device("iPhone 14", "uid-2");
        ios16.os_version = "16.4".to_string();
        assert!(!filter.matches(&ios16));
    }

    // ---------------------------------------------------------------
    // 8. WhereFilter with name constraint filters correctly
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_name_constraint_filters_correctly() {
        let filter = WhereFilter {
            device_type: None,
            os: None,
            name: Some("iPad Pro".to_string()),
            physical: None,
        };

        let mut ipad = make_device("iPad Pro", "uid-1");
        ipad.device_type = DeviceType::Tablet;
        assert!(filter.matches(&ipad));

        let iphone = make_device("iPhone 15", "uid-2");
        assert!(!filter.matches(&iphone));
    }

    // ---------------------------------------------------------------
    // 9. filter_devices returns only matching devices
    // ---------------------------------------------------------------
    #[test]
    fn filter_devices_returns_only_matching() {
        let mut tablet = make_device("iPad Pro", "uid-1");
        tablet.device_type = DeviceType::Tablet;
        let phone = make_device("iPhone 15", "uid-2");

        let devices = vec![tablet, phone];
        let filter = WhereFilter {
            device_type: Some(DeviceType::Phone),
            os: None,
            name: None,
            physical: None,
        };

        let result = filter_devices(&devices, &filter);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "iPhone 15");
    }

    // ---------------------------------------------------------------
    // 10. filter_devices with no matches returns empty
    // ---------------------------------------------------------------
    #[test]
    fn filter_devices_no_matches_returns_empty() {
        let devices = vec![
            make_device("iPhone 15", "uid-1"),
            make_device("iPhone 14", "uid-2"),
        ];
        let filter = WhereFilter {
            device_type: Some(DeviceType::Tablet),
            os: None,
            name: None,
            physical: None,
        };

        let result = filter_devices(&devices, &filter);
        assert!(result.is_empty());
    }

    // ---------------------------------------------------------------
    // 11. EachContext has correct device name and id
    // ---------------------------------------------------------------
    #[test]
    fn each_context_has_correct_device_name_and_id() {
        let devices = vec![make_device("Pixel 8 Pro", "emulator-5554")];
        let device_vars: HashMap<String, HashMap<String, VarValue>> = HashMap::new();

        let contexts = build_each_contexts(&devices, &device_vars);
        assert_eq!(contexts[0].device_id, "emulator-5554");
        assert_eq!(contexts[0].device_name, "Pixel 8 Pro");
    }

    // ---------------------------------------------------------------
    // 12. build_each_contexts ordering matches input order
    // ---------------------------------------------------------------
    #[test]
    fn build_each_contexts_ordering_matches_input() {
        let devices = vec![
            make_device("Alpha", "uid-a"),
            make_device("Beta", "uid-b"),
            make_device("Gamma", "uid-c"),
        ];
        let device_vars: HashMap<String, HashMap<String, VarValue>> = HashMap::new();

        let contexts = build_each_contexts(&devices, &device_vars);
        assert_eq!(contexts[0].device_name, "Alpha");
        assert_eq!(contexts[1].device_name, "Beta");
        assert_eq!(contexts[2].device_name, "Gamma");
    }

    // ---------------------------------------------------------------
    // 13. WhereFilter from_map parses type phone
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_from_map_parses_type_phone() {
        let mut map = HashMap::new();
        map.insert("type".to_string(), "phone".to_string());

        let filter = WhereFilter::from_map(&map);
        assert_eq!(filter.device_type, Some(DeviceType::Phone));
        assert!(filter.os.is_none());
        assert!(filter.name.is_none());
        assert!(filter.physical.is_none());
    }

    // ---------------------------------------------------------------
    // 14. WhereFilter from_map parses type tablet
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_from_map_parses_type_tablet() {
        let mut map = HashMap::new();
        map.insert("type".to_string(), "tablet".to_string());

        let filter = WhereFilter::from_map(&map);
        assert_eq!(filter.device_type, Some(DeviceType::Tablet));
    }

    // ---------------------------------------------------------------
    // 15. WhereFilter from_map parses os and name
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_from_map_parses_os_and_name() {
        let mut map = HashMap::new();
        map.insert("os".to_string(), "17".to_string());
        map.insert("name".to_string(), "iPhone 15".to_string());

        let filter = WhereFilter::from_map(&map);
        assert_eq!(filter.os, Some("17".to_string()));
        assert_eq!(filter.name, Some("iPhone 15".to_string()));
    }

    // ---------------------------------------------------------------
    // 16. WhereFilter from_map with unknown type yields None
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_from_map_unknown_type_yields_none() {
        let mut map = HashMap::new();
        map.insert("type".to_string(), "watch".to_string());

        let filter = WhereFilter::from_map(&map);
        assert!(filter.device_type.is_none());
    }

    // ---------------------------------------------------------------
    // 17. WhereFilter with physical constraint
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_physical_constraint() {
        let filter = WhereFilter {
            device_type: None,
            os: None,
            name: None,
            physical: Some(true),
        };

        let simulator = make_device("iPhone 15", "uid-1");
        assert!(!filter.matches(&simulator)); // physical = false by default

        let mut physical = make_device("iPhone 15", "uid-2");
        physical.physical = true;
        assert!(filter.matches(&physical));
    }

    // ---------------------------------------------------------------
    // 18. WhereFilter from_map parses physical field
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_from_map_parses_physical() {
        let mut map = HashMap::new();
        map.insert("physical".to_string(), "true".to_string());

        let filter = WhereFilter::from_map(&map);
        assert_eq!(filter.physical, Some(true));
    }

    // ---------------------------------------------------------------
    // 19. WhereFilter multiple constraints must all match (AND logic)
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_multiple_constraints_and_logic() {
        let filter = WhereFilter {
            device_type: Some(DeviceType::Phone),
            os: Some("17".to_string()),
            name: None,
            physical: None,
        };

        // Phone + iOS 17 → match
        let mut device = make_device("iPhone 15", "uid-1");
        device.os_version = "17.2".to_string();
        assert!(filter.matches(&device));

        // Tablet + iOS 17 → no match (wrong type)
        let mut tablet = make_device("iPad", "uid-2");
        tablet.device_type = DeviceType::Tablet;
        tablet.os_version = "17.0".to_string();
        assert!(!filter.matches(&tablet));

        // Phone + iOS 16 → no match (wrong OS)
        let mut old_phone = make_device("iPhone 14", "uid-3");
        old_phone.os_version = "16.4".to_string();
        assert!(!filter.matches(&old_phone));
    }

    // ---------------------------------------------------------------
    // 20. build_each_contexts captures os_version
    // ---------------------------------------------------------------
    #[test]
    fn build_each_contexts_captures_os_version() {
        let mut device = make_device("iPhone 15", "uid-1");
        device.os_version = "18.0".to_string();
        let devices = vec![device];
        let device_vars: HashMap<String, HashMap<String, VarValue>> = HashMap::new();

        let contexts = build_each_contexts(&devices, &device_vars);
        assert_eq!(contexts[0].os_version, "18.0");
    }

    // ---------------------------------------------------------------
    // 21. filter_devices preserves ordering
    // ---------------------------------------------------------------
    #[test]
    fn filter_devices_preserves_ordering() {
        let devices = vec![
            make_device("Alpha", "uid-a"),
            make_device("Beta", "uid-b"),
            make_device("Gamma", "uid-c"),
        ];
        let filter = WhereFilter {
            device_type: None,
            os: None,
            name: None,
            physical: None,
        };

        let result = filter_devices(&devices, &filter);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].name, "Alpha");
        assert_eq!(result[1].name, "Beta");
        assert_eq!(result[2].name, "Gamma");
    }

    // ---------------------------------------------------------------
    // 22. WhereFilter from_map empty map matches everything
    // ---------------------------------------------------------------
    #[test]
    fn where_filter_from_map_empty_map_matches_everything() {
        let map: HashMap<String, String> = HashMap::new();
        let filter = WhereFilter::from_map(&map);
        let device = make_device("iPhone 15", "uid-1");
        assert!(filter.matches(&device));
    }
}
