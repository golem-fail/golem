use std::collections::HashMap;

use golem_vars::{VarValue, VariableStore};

/// Manages device-scoped variable stores for parallel execution.
///
/// Each device gets its own [`VariableStore`] that persists across blocks.
/// A global store is shared across all devices for cross-cutting variables.
/// Cross-device reads are supported via [`read_device_var`](Self::read_device_var).
pub struct DeviceVarManager {
    device_stores: HashMap<String, VariableStore>,
    global_store: VariableStore,
}

impl DeviceVarManager {
    /// Create a new manager with no device stores and an empty global store.
    pub fn new() -> Self {
        Self {
            device_stores: HashMap::new(),
            global_store: VariableStore::new(),
        }
    }

    /// Get or create a device's variable store.
    ///
    /// If the device has not been seen before, a new empty store is created.
    pub fn get_device_store(&mut self, device_id: &str) -> &mut VariableStore {
        self.device_stores
            .entry(device_id.to_string())
            .or_default()
    }

    /// Get a mutable reference to the global store.
    pub fn global_store(&mut self) -> &mut VariableStore {
        &mut self.global_store
    }

    /// Read a variable from a specific device's store (immutable).
    ///
    /// Returns `None` if the device does not exist or the variable is not set.
    pub fn read_device_var(&self, device_id: &str, var_name: &str) -> Option<&VarValue> {
        self.device_stores.get(device_id)?.get(var_name)
    }

    /// Read a variable from the global store.
    ///
    /// Returns `None` if the variable is not set.
    pub fn read_global_var(&self, var_name: &str) -> Option<&VarValue> {
        self.global_store.get(var_name)
    }

    /// Initialize a device store by cloning variables from a base store.
    ///
    /// This replaces any existing store for the given device with a clone of
    /// `base_vars`, which is useful for seeding a device with flow-level or
    /// project-level variables at the start of execution.
    pub fn init_device(&mut self, device_id: &str, base_vars: &VariableStore) {
        self.device_stores
            .insert(device_id.to_string(), base_vars.clone());
    }

    /// List all registered device IDs, in arbitrary order.
    pub fn device_ids(&self) -> Vec<&str> {
        self.device_stores.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for DeviceVarManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_vars::{Scope, ScopeLevel};

    // ---------------------------------------------------------------
    // 1. Device stores are isolated
    // ---------------------------------------------------------------
    #[test]
    fn device_stores_are_isolated() {
        let mut mgr = DeviceVarManager::new();

        let store_a = mgr.get_device_store("device_a");
        store_a.push_scope(Scope::new(ScopeLevel::Flow));
        store_a.set("secret", VarValue::string("only_a"));

        let store_b = mgr.get_device_store("device_b");
        store_b.push_scope(Scope::new(ScopeLevel::Flow));
        store_b.set("secret", VarValue::string("only_b"));

        assert_eq!(
            mgr.read_device_var("device_a", "secret"),
            Some(&VarValue::string("only_a")),
        );
        assert_eq!(
            mgr.read_device_var("device_b", "secret"),
            Some(&VarValue::string("only_b")),
        );
    }

    // ---------------------------------------------------------------
    // 2. Device vars persist across operations (simulating blocks)
    // ---------------------------------------------------------------
    #[test]
    fn device_vars_persist_across_blocks() {
        let mut mgr = DeviceVarManager::new();

        // Block 1: set a variable on device_a
        {
            let store = mgr.get_device_store("device_a");
            store.push_scope(Scope::new(ScopeLevel::Flow));
            store.set("quote_ref", VarValue::string("QR-001"));
        }

        // Block 2: different work (no interaction with device_a)

        // Block 3: device_a's variable should still be available
        assert_eq!(
            mgr.read_device_var("device_a", "quote_ref"),
            Some(&VarValue::string("QR-001")),
        );
    }

    // ---------------------------------------------------------------
    // 3. Cross-device read works
    // ---------------------------------------------------------------
    #[test]
    fn cross_device_read_works() {
        let mut mgr = DeviceVarManager::new();

        // Device A sets a variable
        let store_a = mgr.get_device_store("client_ios17");
        store_a.push_scope(Scope::new(ScopeLevel::Flow));
        store_a.set("quote_ref", VarValue::string("QR-999"));

        // Device B reads device A's variable via cross-device access
        let cross_val = mgr.read_device_var("client_ios17", "quote_ref");
        assert_eq!(cross_val, Some(&VarValue::string("QR-999")));
    }

    // ---------------------------------------------------------------
    // 4. Global store is shared
    // ---------------------------------------------------------------
    #[test]
    fn global_store_is_shared() {
        let mut mgr = DeviceVarManager::new();

        // Write to global store
        let global = mgr.global_store();
        global.push_scope(Scope::new(ScopeLevel::Flow));
        global.set("api_base", VarValue::string("https://api.test.com"));

        // Read from global store (simulating different device context)
        assert_eq!(
            mgr.read_global_var("api_base"),
            Some(&VarValue::string("https://api.test.com")),
        );
    }

    // ---------------------------------------------------------------
    // 5. init_device clones base variables correctly
    // ---------------------------------------------------------------
    #[test]
    fn init_device_clones_base_variables() {
        let mut mgr = DeviceVarManager::new();

        // Prepare base store
        let mut base = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Project);
        scope.set("env", VarValue::string("staging"));
        scope.set("api_url", VarValue::string("https://staging.api.com"));
        base.push_scope(scope);

        mgr.init_device("device_x", &base);

        // device_x should have the base variables
        assert_eq!(
            mgr.read_device_var("device_x", "env"),
            Some(&VarValue::string("staging")),
        );
        assert_eq!(
            mgr.read_device_var("device_x", "api_url"),
            Some(&VarValue::string("https://staging.api.com")),
        );

        // Mutating the original base should NOT affect the cloned store
        base.set("env", VarValue::string("production"));
        assert_eq!(
            mgr.read_device_var("device_x", "env"),
            Some(&VarValue::string("staging")),
        );
    }

    // ---------------------------------------------------------------
    // 6. get_device_store creates new store if doesn't exist
    // ---------------------------------------------------------------
    #[test]
    fn get_device_store_creates_if_absent() {
        let mut mgr = DeviceVarManager::new();
        assert!(mgr.device_ids().is_empty());

        let _store = mgr.get_device_store("new_device");

        let ids = mgr.device_ids();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&"new_device"));
    }

    // ---------------------------------------------------------------
    // 7. read_device_var returns None for unknown device
    // ---------------------------------------------------------------
    #[test]
    fn read_device_var_returns_none_for_unknown_device() {
        let mgr = DeviceVarManager::new();
        assert_eq!(mgr.read_device_var("nonexistent", "any_var"), None);
    }

    // ---------------------------------------------------------------
    // 8. read_device_var returns None for unknown variable
    // ---------------------------------------------------------------
    #[test]
    fn read_device_var_returns_none_for_unknown_variable() {
        let mut mgr = DeviceVarManager::new();
        let store = mgr.get_device_store("device_a");
        store.push_scope(Scope::new(ScopeLevel::Flow));
        store.set("known", VarValue::string("yes"));

        assert_eq!(mgr.read_device_var("device_a", "unknown"), None);
    }

    // ---------------------------------------------------------------
    // 9. Multiple devices can be initialized independently
    // ---------------------------------------------------------------
    #[test]
    fn multiple_devices_initialized_independently() {
        let mut mgr = DeviceVarManager::new();

        let mut base_ios = VariableStore::new();
        let mut scope_ios = Scope::new(ScopeLevel::Flow);
        scope_ios.set("platform", VarValue::string("ios"));
        base_ios.push_scope(scope_ios);

        let mut base_android = VariableStore::new();
        let mut scope_android = Scope::new(ScopeLevel::Flow);
        scope_android.set("platform", VarValue::string("android"));
        base_android.push_scope(scope_android);

        mgr.init_device("iphone14", &base_ios);
        mgr.init_device("pixel7", &base_android);

        assert_eq!(
            mgr.read_device_var("iphone14", "platform"),
            Some(&VarValue::string("ios")),
        );
        assert_eq!(
            mgr.read_device_var("pixel7", "platform"),
            Some(&VarValue::string("android")),
        );
    }

    // ---------------------------------------------------------------
    // 10. device_ids returns all registered devices
    // ---------------------------------------------------------------
    #[test]
    fn device_ids_returns_all_registered() {
        let mut mgr = DeviceVarManager::new();
        let _ = mgr.get_device_store("alpha");
        let _ = mgr.get_device_store("beta");
        let _ = mgr.get_device_store("gamma");

        let mut ids = mgr.device_ids();
        ids.sort();
        assert_eq!(ids, vec!["alpha", "beta", "gamma"]);
    }

    // ---------------------------------------------------------------
    // 11. read_global_var returns None for unknown variable
    // ---------------------------------------------------------------
    #[test]
    fn read_global_var_returns_none_for_unknown() {
        let mgr = DeviceVarManager::new();
        assert_eq!(mgr.read_global_var("missing"), None);
    }

    // ---------------------------------------------------------------
    // 12. init_device replaces existing store
    // ---------------------------------------------------------------
    #[test]
    fn init_device_replaces_existing_store() {
        let mut mgr = DeviceVarManager::new();

        // First init
        let mut base1 = VariableStore::new();
        let mut scope1 = Scope::new(ScopeLevel::Flow);
        scope1.set("version", VarValue::string("v1"));
        base1.push_scope(scope1);
        mgr.init_device("dev", &base1);

        assert_eq!(
            mgr.read_device_var("dev", "version"),
            Some(&VarValue::string("v1")),
        );

        // Re-init with different base
        let mut base2 = VariableStore::new();
        let mut scope2 = Scope::new(ScopeLevel::Flow);
        scope2.set("version", VarValue::string("v2"));
        base2.push_scope(scope2);
        mgr.init_device("dev", &base2);

        assert_eq!(
            mgr.read_device_var("dev", "version"),
            Some(&VarValue::string("v2")),
        );
    }

    // ---------------------------------------------------------------
    // 13. Default trait works
    // ---------------------------------------------------------------
    #[test]
    fn default_creates_empty_manager() {
        let mgr = DeviceVarManager::default();
        assert!(mgr.device_ids().is_empty());
        assert_eq!(mgr.read_global_var("anything"), None);
    }

    // ---------------------------------------------------------------
    // 14. get_device_store returns the SAME store on repeat calls
    //     (or_default must not overwrite an existing store)
    // ---------------------------------------------------------------
    #[test]
    fn get_device_store_is_idempotent_and_preserves_state() {
        let mut mgr = DeviceVarManager::new();

        // First access: seed a variable.
        {
            let store = mgr.get_device_store("device_a");
            store.push_scope(Scope::new(ScopeLevel::Flow));
            store.set("kept", VarValue::string("v1"));
        }

        // Second access of the same device must return the existing store,
        // not a fresh empty one.
        {
            let store = mgr.get_device_store("device_a");
            store.set("added", VarValue::string("v2"));
        }

        assert_eq!(
            mgr.read_device_var("device_a", "kept"),
            Some(&VarValue::string("v1")),
            "repeat get_device_store SHALL preserve earlier variables",
        );
        assert_eq!(
            mgr.read_device_var("device_a", "added"),
            Some(&VarValue::string("v2")),
            "repeat get_device_store SHALL accept new variables on the same store",
        );
        assert_eq!(
            mgr.device_ids().len(),
            1,
            "repeat get_device_store SHALL NOT register a duplicate device id",
        );
    }

    // ---------------------------------------------------------------
    // 15. get_device_store after init_device returns the seeded store,
    //     not a freshly created empty one
    // ---------------------------------------------------------------
    #[test]
    fn get_device_store_after_init_returns_seeded_store() {
        let mut mgr = DeviceVarManager::new();

        let mut base = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Flow);
        scope.set("seeded", VarValue::string("from_base"));
        base.push_scope(scope);
        mgr.init_device("dev", &base);

        // Touching the store via get_device_store must not wipe seeded vars.
        let store = mgr.get_device_store("dev");
        store.set("late", VarValue::string("after_init"));

        assert_eq!(
            mgr.read_device_var("dev", "seeded"),
            Some(&VarValue::string("from_base")),
            "get_device_store SHALL return the init_device-seeded store",
        );
        assert_eq!(
            mgr.read_device_var("dev", "late"),
            Some(&VarValue::string("after_init")),
        );
    }

    // ---------------------------------------------------------------
    // 16. Device and global stores do not leak into each other
    // ---------------------------------------------------------------
    #[test]
    fn device_and_global_stores_are_independent() {
        let mut mgr = DeviceVarManager::new();

        {
            let store = mgr.get_device_store("device_a");
            store.push_scope(Scope::new(ScopeLevel::Flow));
            store.set("only_device", VarValue::string("d"));
        }
        {
            let global = mgr.global_store();
            global.push_scope(Scope::new(ScopeLevel::Flow));
            global.set("only_global", VarValue::string("g"));
        }

        // Device var is not visible globally.
        assert_eq!(
            mgr.read_global_var("only_device"),
            None,
            "a device variable SHALL NOT leak into the global store",
        );
        // Global var is not visible on the device.
        assert_eq!(
            mgr.read_device_var("device_a", "only_global"),
            None,
            "a global variable SHALL NOT leak into a device store",
        );
        // Writing to the global store SHALL NOT register a device id.
        assert_eq!(
            mgr.device_ids(),
            vec!["device_a"],
            "global writes SHALL NOT create device entries",
        );
    }

    // ---------------------------------------------------------------
    // 17. global_store() returns the one persistent store across
    //     separate accesses, and read_global_var observes the same store
    // ---------------------------------------------------------------
    #[test]
    fn global_store_persists_across_accesses() {
        let mut mgr = DeviceVarManager::new();

        // First access: seed a value, then drop the borrow.
        {
            let global = mgr.global_store();
            global.push_scope(Scope::new(ScopeLevel::Flow));
            global.set("token", VarValue::string("first"));
        }

        // Separate access must hand back the SAME store, so the value seeded
        // in the first access is still visible. This is the module-level
        // behavior under test: global_store() exposes the single persistent
        // global field, not a fresh store per call.
        {
            let global = mgr.global_store();
            assert_eq!(
                global.get("token"),
                Some(&VarValue::string("first")),
                "global_store SHALL return the same persistent store across separate accesses",
            );
        }

        // read_global_var reads that same persistent store (not a copy).
        assert_eq!(
            mgr.read_global_var("token"),
            Some(&VarValue::string("first")),
            "read_global_var SHALL observe the store mutated via global_store",
        );
    }

    // ---------------------------------------------------------------
    // 18. init_device on a fresh device registers it in device_ids
    // ---------------------------------------------------------------
    #[test]
    fn init_device_registers_new_device_id() {
        let mut mgr = DeviceVarManager::new();
        assert!(mgr.device_ids().is_empty());

        let base = VariableStore::new();
        mgr.init_device("seeded_dev", &base);

        assert_eq!(
            mgr.device_ids(),
            vec!["seeded_dev"],
            "init_device SHALL register the device id even with an empty base",
        );
    }
}
