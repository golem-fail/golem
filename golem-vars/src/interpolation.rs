use crate::{VarError, VarValue, VariableStore};
use std::collections::HashMap;

/// Context for variable resolution during interpolation.
pub struct InterpolationContext<'a> {
    /// The main variable store.
    pub store: &'a VariableStore,
    /// Current device name (for self: prefix and bare var resolution).
    pub device: Option<&'a str>,
    /// Device-scoped variable stores (device_name -> VariableStore).
    pub device_stores: Option<&'a HashMap<String, VariableStore>>,
    /// Global variable store (for global: prefix).
    pub global_store: Option<&'a VariableStore>,
    /// Current for_each iteration variables (for _each. prefix).
    pub each_vars: Option<&'a VariableStore>,
    /// Built-in variables (_device, _os, _platform, _type, _udid, _app, _loop).
    pub builtins: Option<&'a HashMap<String, String>>,
}

/// Resolve a dot-path against a `VarValue`, returning a string result.
///
/// `root_key` is the first segment (used for error messages), `rest` is the
/// remaining dot-separated segments, and `value` is the starting value.
fn resolve_dot_path(root_key: &str, rest: &[&str], value: &VarValue) -> Result<String, VarError> {
    let mut current = value;
    // Track the path we've traversed so far for error messages.
    let mut traversed = root_key.to_string();

    for &segment in rest {
        match current {
            VarValue::Object(map) => {
                current = map.get(segment).ok_or_else(|| VarError::PropertyNotFound {
                    object: traversed.clone(),
                    property: segment.to_string(),
                })?;
                traversed.push('.');
                traversed.push_str(segment);
            }
            VarValue::String(_) => {
                return Err(VarError::NotAnObject(traversed));
            }
        }
    }

    match current {
        VarValue::String(s) => Ok(s.clone()),
        VarValue::Object(_) => {
            // Attempting to interpolate an object directly is an error.
            Err(VarError::NotAnObject(traversed))
        }
    }
}

/// Resolve a key (possibly with dot-path) from a `VariableStore`.
fn resolve_from_store(full_key: &str, store: &VariableStore) -> Result<String, VarError> {
    let parts: Vec<&str> = full_key.split('.').collect();
    let root_key = parts[0];

    let value = store.resolve(root_key)?;

    if parts.len() == 1 {
        match value {
            VarValue::String(s) => Ok(s.clone()),
            VarValue::Object(_) => Err(VarError::NotAnObject(root_key.to_string())),
        }
    } else {
        resolve_dot_path(root_key, &parts[1..], value)
    }
}

/// Resolve a single `${...}` reference using the interpolation context.
fn resolve_reference(reference: &str, ctx: &InterpolationContext) -> Result<String, VarError> {
    // Check for nested interpolation: ${${...}}
    if reference.contains("${") {
        return Err(VarError::Other(
            "nested interpolation is not supported".to_string(),
        ));
    }

    // Handle prefixed references
    if let Some(var_name) = reference.strip_prefix("self:") {
        // self: prefix -- only current device store, error if not found
        let device_name = ctx.device.ok_or_else(|| {
            VarError::Other("self: prefix used outside device context".to_string())
        })?;
        let device_stores = ctx.device_stores.ok_or_else(|| {
            VarError::Other("self: prefix used but no device stores available".to_string())
        })?;
        let device_store = device_stores.get(device_name).ok_or_else(|| {
            VarError::Undefined(format!("self:{var_name}"))
        })?;
        return resolve_from_store(var_name, device_store);
    }

    if let Some(var_name) = reference.strip_prefix("global:") {
        // global: prefix -- only global store
        let global_store = ctx.global_store.ok_or_else(|| {
            VarError::Other("global: prefix used but no global store available".to_string())
        })?;
        return resolve_from_store(var_name, global_store);
    }

    if let Some(var_name) = reference.strip_prefix("_each.") {
        // _each. prefix -- each_vars store
        let each_store = ctx.each_vars.ok_or_else(|| {
            VarError::Other("_each used outside of for_each context".to_string())
        })?;
        return resolve_from_store(var_name, each_store);
    }

    // Check for device_name: prefix (e.g., "iphone_17:var")
    if let Some(colon_pos) = reference.find(':') {
        let device_name = &reference[..colon_pos];
        let var_name = &reference[colon_pos + 1..];
        let device_stores = ctx.device_stores.ok_or_else(|| {
            VarError::Other(format!(
                "device prefix \"{device_name}:\" used but no device stores available"
            ))
        })?;
        let device_store = device_stores.get(device_name).ok_or_else(|| {
            VarError::Other(format!("device \"{device_name}\" not found"))
        })?;
        return resolve_from_store(var_name, device_store);
    }

    // Bare reference -- resolution order:
    // 1. Builtins
    // 2. Current device store
    // 3. Main store

    // 1. Check builtins
    let parts: Vec<&str> = reference.split('.').collect();
    let root_key = parts[0];

    if let Some(builtins) = ctx.builtins {
        if let Some(builtin_val) = builtins.get(root_key) {
            if parts.len() == 1 {
                return Ok(builtin_val.clone());
            }
            // Builtins are plain strings; dot access on them is an error.
            return Err(VarError::NotAnObject(root_key.to_string()));
        }
    }

    // 2. Check current device store
    if let (Some(device_name), Some(device_stores)) = (ctx.device, ctx.device_stores) {
        if let Some(device_store) = device_stores.get(device_name) {
            if let Ok(result) = resolve_from_store(reference, device_store) {
                return Ok(result);
            }
        }
    }

    // 3. Main store
    resolve_from_store(reference, ctx.store)
}

/// Interpolate all `${...}` references in a template string.
///
/// - `${var}` resolves bare variables (builtins, then device store, then main store).
/// - `${self:var}` resolves from the current device store only.
/// - `${global:var}` resolves from the global store only.
/// - `${device_name:var}` resolves from a specific device's store.
/// - `${_each.var}` resolves from the for_each iteration variables.
/// - `$$` produces a literal `$` (i.e. `$${...}` becomes `${...}` literally).
/// - Dot paths like `${obj.field.sub}` traverse nested objects.
///
/// Returns an error for undefined variables, unclosed braces, nested
/// interpolation, and non-string object access.
pub fn interpolate(template: &str, ctx: &InterpolationContext) -> Result<String, VarError> {
    let mut result = String::with_capacity(template.len());
    let chars: Vec<char> = template.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '$' {
            // Check for escaped dollar: $$
            if i + 1 < len && chars[i + 1] == '$' {
                result.push('$');
                i += 2;
                continue;
            }

            // Check for ${...} pattern
            if i + 1 < len && chars[i + 1] == '{' {
                // Find closing brace
                let start = i + 2;
                let mut depth = 1;
                let mut j = start;
                let mut has_nested = false;

                while j < len && depth > 0 {
                    if chars[j] == '{' && j > 0 && chars[j - 1] == '$' {
                        has_nested = true;
                        depth += 1;
                    } else if chars[j] == '}' {
                        depth -= 1;
                    }
                    if depth > 0 {
                        j += 1;
                    }
                }

                if depth > 0 {
                    return Err(VarError::UnclosedReference);
                }

                let reference: String = chars[start..j].iter().collect();

                if has_nested {
                    return Err(VarError::Other(
                        "nested interpolation is not supported".to_string(),
                    ));
                }

                let resolved = resolve_reference(&reference, ctx)?;
                result.push_str(&resolved);

                i = j + 1;
            } else {
                // Lone $ not followed by $ or {
                result.push('$');
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Scope, ScopeLevel, VarValue, VariableStore};

    /// Helper to build a simple store with one scope at a given level.
    fn store_with(level: ScopeLevel, vars: Vec<(&str, VarValue)>) -> VariableStore {
        let mut store = VariableStore::new();
        let mut scope = Scope::new(level);
        for (k, v) in vars {
            scope.set(k, v);
        }
        store.push_scope(scope);
        store
    }

    /// Minimal context pointing at a single store with no device/global/each.
    fn simple_ctx(store: &VariableStore) -> InterpolationContext<'_> {
        InterpolationContext {
            store,
            device: None,
            device_stores: None,
            global_store: None,
            each_vars: None,
            builtins: None,
        }
    }

    // 1. Simple substitution
    #[test]
    fn simple_substitution() {
        let store = store_with(
            ScopeLevel::Project,
            vec![("email", VarValue::string("alice@example.com"))],
        );
        let ctx = simple_ctx(&store);
        let result = interpolate("${email}", &ctx).unwrap();
        assert_eq!(result, "alice@example.com");
    }

    // 2. Multiple substitutions
    #[test]
    fn multiple_substitutions() {
        let store = store_with(
            ScopeLevel::Project,
            vec![
                ("first", VarValue::string("Sarah")),
                ("last", VarValue::string("Johnson")),
            ],
        );
        let ctx = simple_ctx(&store);
        let result = interpolate("${first} ${last}", &ctx).unwrap();
        assert_eq!(result, "Sarah Johnson");
    }

    // 3. No variables -- passthrough unchanged
    #[test]
    fn no_variables_passthrough() {
        let store = VariableStore::new();
        let ctx = simple_ctx(&store);
        let result = interpolate("plain text here", &ctx).unwrap();
        assert_eq!(result, "plain text here");
    }

    // 4. Dot access one level
    #[test]
    fn dot_access_one_level() {
        let store = store_with(
            ScopeLevel::Project,
            vec![(
                "person",
                VarValue::object(vec![("first", VarValue::string("Sarah"))]),
            )],
        );
        let ctx = simple_ctx(&store);
        let result = interpolate("${person.first}", &ctx).unwrap();
        assert_eq!(result, "Sarah");
    }

    // 5. Dot access multiple levels
    #[test]
    fn dot_access_multiple_levels() {
        let store = store_with(
            ScopeLevel::Project,
            vec![(
                "config",
                VarValue::object(vec![(
                    "db",
                    VarValue::object(vec![
                        ("host", VarValue::string("localhost")),
                        ("port", VarValue::string("5432")),
                    ]),
                )]),
            )],
        );
        let ctx = simple_ctx(&store);
        let result = interpolate("${config.db.host}:${config.db.port}", &ctx).unwrap();
        assert_eq!(result, "localhost:5432");
    }

    // 6. Dot access property not found
    #[test]
    fn dot_access_property_not_found() {
        let store = store_with(
            ScopeLevel::Project,
            vec![(
                "person",
                VarValue::object(vec![("first", VarValue::string("Sarah"))]),
            )],
        );
        let ctx = simple_ctx(&store);
        let result = interpolate("${person.missing}", &ctx);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, VarError::PropertyNotFound { ref object, ref property }
                     if object == "person" && property == "missing"),
            "expected PropertyNotFound, got: {err}"
        );
    }

    // 7. Dot access intermediate not object
    #[test]
    fn dot_access_intermediate_not_object() {
        let store = store_with(
            ScopeLevel::Project,
            vec![("name", VarValue::string("Alice"))],
        );
        let ctx = simple_ctx(&store);
        let result = interpolate("${name.first}", &ctx);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, VarError::NotAnObject(ref s) if s == "name"),
            "expected NotAnObject, got: {err}"
        );
    }

    // 8. Self prefix
    #[test]
    fn self_prefix() {
        let main_store = VariableStore::new();
        let device_store = store_with(
            ScopeLevel::Project,
            vec![("email", VarValue::string("device@example.com"))],
        );
        let mut device_stores = HashMap::new();
        device_stores.insert("pixel_9".to_string(), device_store);

        let ctx = InterpolationContext {
            store: &main_store,
            device: Some("pixel_9"),
            device_stores: Some(&device_stores),
            global_store: None,
            each_vars: None,
            builtins: None,
        };
        let result = interpolate("${self:email}", &ctx).unwrap();
        assert_eq!(result, "device@example.com");
    }

    // 9. Self prefix not found -- error (no fallthrough to global)
    #[test]
    fn self_prefix_not_found_no_fallthrough() {
        let main_store = store_with(
            ScopeLevel::Project,
            vec![("email", VarValue::string("global@example.com"))],
        );
        let device_store = VariableStore::new();
        let mut device_stores = HashMap::new();
        device_stores.insert("pixel_9".to_string(), device_store);

        let ctx = InterpolationContext {
            store: &main_store,
            device: Some("pixel_9"),
            device_stores: Some(&device_stores),
            global_store: None,
            each_vars: None,
            builtins: None,
        };
        let result = interpolate("${self:email}", &ctx);
        assert!(result.is_err());
    }

    // 10. Global prefix
    #[test]
    fn global_prefix() {
        let main_store = VariableStore::new();
        let global_store = store_with(
            ScopeLevel::Project,
            vec![("email", VarValue::string("global@example.com"))],
        );

        let ctx = InterpolationContext {
            store: &main_store,
            device: None,
            device_stores: None,
            global_store: Some(&global_store),
            each_vars: None,
            builtins: None,
        };
        let result = interpolate("${global:email}", &ctx).unwrap();
        assert_eq!(result, "global@example.com");
    }

    // 11. Cross-device prefix
    #[test]
    fn cross_device_prefix() {
        let main_store = VariableStore::new();
        let iphone_store = store_with(
            ScopeLevel::Project,
            vec![("quote_ref", VarValue::string("QR-12345"))],
        );
        let mut device_stores = HashMap::new();
        device_stores.insert("iphone_17".to_string(), iphone_store);

        let ctx = InterpolationContext {
            store: &main_store,
            device: Some("pixel_9"),
            device_stores: Some(&device_stores),
            global_store: None,
            each_vars: None,
            builtins: None,
        };
        let result = interpolate("${iphone_17:quote_ref}", &ctx).unwrap();
        assert_eq!(result, "QR-12345");
    }

    // 12. Cross-device device not found
    #[test]
    fn cross_device_not_found() {
        let main_store = VariableStore::new();
        let device_stores = HashMap::new();

        let ctx = InterpolationContext {
            store: &main_store,
            device: None,
            device_stores: Some(&device_stores),
            global_store: None,
            each_vars: None,
            builtins: None,
        };
        let result = interpolate("${nonexistent_device:var}", &ctx);
        assert!(result.is_err());
    }

    // 13. _each prefix inside for_each
    #[test]
    fn each_prefix_inside_for_each() {
        let main_store = VariableStore::new();
        let each_store = store_with(
            ScopeLevel::Project,
            vec![("item", VarValue::string("apple"))],
        );

        let ctx = InterpolationContext {
            store: &main_store,
            device: None,
            device_stores: None,
            global_store: None,
            each_vars: Some(&each_store),
            builtins: None,
        };
        let result = interpolate("${_each.item}", &ctx).unwrap();
        assert_eq!(result, "apple");
    }

    // 14. _each prefix outside for_each
    #[test]
    fn each_prefix_outside_for_each() {
        let main_store = VariableStore::new();

        let ctx = InterpolationContext {
            store: &main_store,
            device: None,
            device_stores: None,
            global_store: None,
            each_vars: None,
            builtins: None,
        };
        let result = interpolate("${_each.item}", &ctx);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, VarError::Other(ref msg) if msg.contains("for_each")),
            "expected for_each error, got: {err}"
        );
    }

    // 15. Undefined variable
    #[test]
    fn undefined_variable_error() {
        let store = VariableStore::new();
        let ctx = simple_ctx(&store);
        let result = interpolate("${nonexistent}", &ctx);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, VarError::Undefined(ref name) if name == "nonexistent"),
            "expected Undefined, got: {err}"
        );
    }

    // 16. Empty string is valid
    #[test]
    fn empty_string_is_valid() {
        let store = store_with(
            ScopeLevel::Project,
            vec![("name", VarValue::string(""))],
        );
        let ctx = simple_ctx(&store);
        let result = interpolate("${name}", &ctx).unwrap();
        assert_eq!(result, "");
    }

    // 17. Built-in variables
    #[test]
    fn builtin_variables() {
        let store = VariableStore::new();
        let mut builtins = HashMap::new();
        builtins.insert("_device".to_string(), "Pixel 9".to_string());

        let ctx = InterpolationContext {
            store: &store,
            device: None,
            device_stores: None,
            global_store: None,
            each_vars: None,
            builtins: Some(&builtins),
        };
        let result = interpolate("${_device}", &ctx).unwrap();
        assert_eq!(result, "Pixel 9");
    }

    // 18. Built-in _loop
    #[test]
    fn builtin_loop() {
        let store = VariableStore::new();
        let mut builtins = HashMap::new();
        builtins.insert("_loop".to_string(), "3".to_string());

        let ctx = InterpolationContext {
            store: &store,
            device: None,
            device_stores: None,
            global_store: None,
            each_vars: None,
            builtins: Some(&builtins),
        };
        let result = interpolate("${_loop}", &ctx).unwrap();
        assert_eq!(result, "3");
    }

    // 19. Resolution order: CLI wins
    #[test]
    fn resolution_order_cli_wins() {
        let mut store = VariableStore::new();

        let mut flow = Scope::new(ScopeLevel::Flow);
        flow.set("env", VarValue::string("flow-env"));
        store.push_scope(flow);

        let mut cli = Scope::new(ScopeLevel::Cli);
        cli.set("env", VarValue::string("cli-env"));
        store.push_scope(cli);

        let ctx = simple_ctx(&store);
        let result = interpolate("${env}", &ctx).unwrap();
        assert_eq!(result, "cli-env");
    }

    // 20. Resolution order: flow wins over fixture
    #[test]
    fn resolution_order_flow_wins_over_fixture() {
        let mut store = VariableStore::new();

        let mut fixture = Scope::new(ScopeLevel::Fixture);
        fixture.set("env", VarValue::string("fixture-env"));
        store.push_scope(fixture);

        let mut flow = Scope::new(ScopeLevel::Flow);
        flow.set("env", VarValue::string("flow-env"));
        store.push_scope(flow);

        let ctx = simple_ctx(&store);
        let result = interpolate("${env}", &ctx).unwrap();
        assert_eq!(result, "flow-env");
    }

    // 21. Resolution order: fixture wins over project
    #[test]
    fn resolution_order_fixture_wins_over_project() {
        let mut store = VariableStore::new();

        let mut project = Scope::new(ScopeLevel::Project);
        project.set("env", VarValue::string("project-env"));
        store.push_scope(project);

        let mut fixture = Scope::new(ScopeLevel::Fixture);
        fixture.set("env", VarValue::string("fixture-env"));
        store.push_scope(fixture);

        let ctx = simple_ctx(&store);
        let result = interpolate("${env}", &ctx).unwrap();
        assert_eq!(result, "fixture-env");
    }

    // 22. Resolution order: device before global (main store)
    #[test]
    fn resolution_order_device_before_global() {
        let main_store = store_with(
            ScopeLevel::Project,
            vec![("email", VarValue::string("main@example.com"))],
        );
        let device_store = store_with(
            ScopeLevel::Project,
            vec![("email", VarValue::string("device@example.com"))],
        );
        let mut device_stores = HashMap::new();
        device_stores.insert("pixel_9".to_string(), device_store);

        let ctx = InterpolationContext {
            store: &main_store,
            device: Some("pixel_9"),
            device_stores: Some(&device_stores),
            global_store: None,
            each_vars: None,
            builtins: None,
        };
        let result = interpolate("${email}", &ctx).unwrap();
        assert_eq!(result, "device@example.com");
    }

    // 23. Nested interpolation not supported
    #[test]
    fn nested_interpolation_not_supported() {
        let store = store_with(
            ScopeLevel::Project,
            vec![("key", VarValue::string("value"))],
        );
        let ctx = simple_ctx(&store);
        let result = interpolate("${${key}}", &ctx);
        assert!(result.is_err());
    }

    // 24. Escaped dollar
    #[test]
    fn escaped_dollar() {
        let store = VariableStore::new();
        let ctx = simple_ctx(&store);
        let result = interpolate("$${not_a_var}", &ctx).unwrap();
        assert_eq!(result, "${not_a_var}");
    }

    // 25. Adjacent substitutions
    #[test]
    fn adjacent_substitutions() {
        let store = store_with(
            ScopeLevel::Project,
            vec![
                ("a", VarValue::string("hello")),
                ("b", VarValue::string("world")),
            ],
        );
        let ctx = simple_ctx(&store);
        let result = interpolate("${a}${b}", &ctx).unwrap();
        assert_eq!(result, "helloworld");
    }

    // 26. Unclosed brace
    #[test]
    fn unclosed_brace_error() {
        let store = VariableStore::new();
        let ctx = simple_ctx(&store);
        let result = interpolate("${name", &ctx);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, VarError::UnclosedReference),
            "expected UnclosedReference, got: {err}"
        );
    }

    // 27. Variable in variable value (pre-resolved) -- tested via store setup
    #[test]
    fn variable_in_variable_value_pre_resolved() {
        // Variables whose values themselves contain ${...} should have been
        // resolved *before* being stored, so they come out as plain strings.
        let store = store_with(
            ScopeLevel::Project,
            vec![
                ("greeting", VarValue::string("Hello, Alice")),
                ("name", VarValue::string("Alice")),
            ],
        );
        let ctx = simple_ctx(&store);
        // greeting was pre-resolved before being stored
        let result = interpolate("${greeting}", &ctx).unwrap();
        assert_eq!(result, "Hello, Alice");
    }

    // 28. self: prefix used with no device set -- distinct Other error
    #[test]
    fn self_prefix_without_device_context() {
        let store = VariableStore::new();
        let ctx = simple_ctx(&store);
        let result = interpolate("${self:email}", &ctx);
        let err = result.expect_err("self: without device SHALL error");
        assert!(
            matches!(err, VarError::Other(ref msg) if msg.contains("outside device context")),
            "expected device context error, got: {err}"
        );
    }

    // 29. self: prefix when device set but no device stores available
    #[test]
    fn self_prefix_without_device_stores() {
        let store = VariableStore::new();
        let ctx = InterpolationContext {
            store: &store,
            device: Some("pixel_9"),
            device_stores: None,
            global_store: None,
            each_vars: None,
            builtins: None,
        };
        let result = interpolate("${self:email}", &ctx);
        let err = result.expect_err("self: without device stores SHALL error");
        assert!(
            matches!(err, VarError::Other(ref msg) if msg.contains("no device stores available")),
            "expected no-device-stores error, got: {err}"
        );
    }

    // 30. self: prefix when current device is absent from the device stores map
    #[test]
    fn self_prefix_device_missing_from_stores() {
        let store = VariableStore::new();
        let device_stores: HashMap<String, VariableStore> = HashMap::new();
        let ctx = InterpolationContext {
            store: &store,
            device: Some("pixel_9"),
            device_stores: Some(&device_stores),
            global_store: None,
            each_vars: None,
            builtins: None,
        };
        let result = interpolate("${self:email}", &ctx);
        let err = result.expect_err("self: for absent device SHALL error");
        assert!(
            matches!(err, VarError::Undefined(ref name) if name == "self:email"),
            "expected Undefined(self:email), got: {err}"
        );
    }

    // 31. global: prefix used but no global store provided
    #[test]
    fn global_prefix_without_global_store() {
        let store = VariableStore::new();
        let ctx = simple_ctx(&store);
        let result = interpolate("${global:email}", &ctx);
        let err = result.expect_err("global: without global store SHALL error");
        assert!(
            matches!(err, VarError::Other(ref msg) if msg.contains("no global store available")),
            "expected no-global-store error, got: {err}"
        );
    }

    // 32. device_name: prefix used but no device stores available
    #[test]
    fn device_prefix_without_device_stores() {
        let store = VariableStore::new();
        let ctx = simple_ctx(&store);
        let result = interpolate("${iphone_17:var}", &ctx);
        let err = result.expect_err("device prefix without stores SHALL error");
        assert!(
            matches!(err, VarError::Other(ref msg)
                     if msg.contains("iphone_17:") && msg.contains("no device stores available")),
            "expected no-device-stores error naming the device, got: {err}"
        );
    }

    // 33. Bare reference falls through a non-matching device store to the main store
    #[test]
    fn bare_reference_falls_through_device_store_to_main() {
        let main_store = store_with(
            ScopeLevel::Project,
            vec![("only_in_main", VarValue::string("main-value"))],
        );
        // Device store exists but does NOT contain the key.
        let device_store = store_with(
            ScopeLevel::Project,
            vec![("something_else", VarValue::string("x"))],
        );
        let mut device_stores = HashMap::new();
        device_stores.insert("pixel_9".to_string(), device_store);

        let ctx = InterpolationContext {
            store: &main_store,
            device: Some("pixel_9"),
            device_stores: Some(&device_stores),
            global_store: None,
            each_vars: None,
            builtins: None,
        };
        let result = interpolate("${only_in_main}", &ctx).expect("SHALL fall through to main");
        assert_eq!(result, "main-value");
    }

    // 34. Builtin matched as root but accessed with a dot path -- NotAnObject
    #[test]
    fn builtin_with_dot_access_is_not_an_object() {
        let store = VariableStore::new();
        let mut builtins = HashMap::new();
        builtins.insert("_device".to_string(), "Pixel 9".to_string());

        let ctx = InterpolationContext {
            store: &store,
            device: None,
            device_stores: None,
            global_store: None,
            each_vars: None,
            builtins: Some(&builtins),
        };
        let result = interpolate("${_device.field}", &ctx);
        let err = result.expect_err("dot access on a builtin SHALL error");
        assert!(
            matches!(err, VarError::NotAnObject(ref s) if s == "_device"),
            "expected NotAnObject(_device), got: {err}"
        );
    }

    // 35. Resolving a bare top-level object (no dot path) directly is NotAnObject
    #[test]
    fn bare_object_reference_is_not_an_object() {
        let store = store_with(
            ScopeLevel::Project,
            vec![(
                "person",
                VarValue::object(vec![("first", VarValue::string("Sarah"))]),
            )],
        );
        let ctx = simple_ctx(&store);
        let result = interpolate("${person}", &ctx);
        let err = result.expect_err("interpolating an object directly SHALL error");
        assert!(
            matches!(err, VarError::NotAnObject(ref s) if s == "person"),
            "expected NotAnObject(person), got: {err}"
        );
    }

    // 36. Resolving a dot path down to an object (not a leaf string) is NotAnObject
    #[test]
    fn dot_path_to_object_is_not_an_object() {
        let store = store_with(
            ScopeLevel::Project,
            vec![(
                "config",
                VarValue::object(vec![(
                    "db",
                    VarValue::object(vec![("host", VarValue::string("localhost"))]),
                )]),
            )],
        );
        let ctx = simple_ctx(&store);
        // config.db is itself an object, not a string leaf.
        let result = interpolate("${config.db}", &ctx);
        let err = result.expect_err("dot path landing on an object SHALL error");
        assert!(
            matches!(err, VarError::NotAnObject(ref s) if s == "config.db"),
            "expected NotAnObject(config.db), got: {err}"
        );
    }

    // 37. Lone $ not followed by { or $ is preserved literally
    #[test]
    fn lone_dollar_preserved() {
        let store = VariableStore::new();
        let ctx = simple_ctx(&store);
        let result = interpolate("cost is $5 today", &ctx).expect("lone $ SHALL pass through");
        assert_eq!(result, "cost is $5 today");
    }

    // 38. Trailing $ at end of template is preserved literally
    #[test]
    fn trailing_dollar_preserved() {
        let store = VariableStore::new();
        let ctx = simple_ctx(&store);
        let result = interpolate("price$", &ctx).expect("trailing $ SHALL pass through");
        assert_eq!(result, "price$");
    }

    // 39. Empty template yields empty string
    #[test]
    fn empty_template() {
        let store = VariableStore::new();
        let ctx = simple_ctx(&store);
        let result = interpolate("", &ctx).expect("empty template SHALL be empty");
        assert_eq!(result, "");
    }

    // 40. Empty reference ${} resolves as an undefined bare variable
    #[test]
    fn empty_reference_is_undefined() {
        let store = VariableStore::new();
        let ctx = simple_ctx(&store);
        let result = interpolate("${}", &ctx);
        let err = result.expect_err("empty reference SHALL error");
        assert!(
            matches!(err, VarError::Undefined(ref name) if name.is_empty()),
            "expected Undefined(\"\"), got: {err}"
        );
    }

    // 41. _each. prefix with a dot path into a nested object
    #[test]
    fn each_prefix_with_dot_path() {
        let main_store = VariableStore::new();
        let each_store = store_with(
            ScopeLevel::Project,
            vec![(
                "item",
                VarValue::object(vec![("name", VarValue::string("apple"))]),
            )],
        );
        let ctx = InterpolationContext {
            store: &main_store,
            device: None,
            device_stores: None,
            global_store: None,
            each_vars: Some(&each_store),
            builtins: None,
        };
        let result = interpolate("${_each.item.name}", &ctx).expect("nested _each SHALL resolve");
        assert_eq!(result, "apple");
    }

    // 42. Cross-device prefix when the named device exists but the var does not
    #[test]
    fn cross_device_var_not_found_in_existing_device() {
        let main_store = VariableStore::new();
        let iphone_store = store_with(
            ScopeLevel::Project,
            vec![("present", VarValue::string("yes"))],
        );
        let mut device_stores = HashMap::new();
        device_stores.insert("iphone_17".to_string(), iphone_store);

        let ctx = InterpolationContext {
            store: &main_store,
            device: None,
            device_stores: Some(&device_stores),
            global_store: None,
            each_vars: None,
            builtins: None,
        };
        let result = interpolate("${iphone_17:absent}", &ctx);
        let err = result.expect_err("missing cross-device var SHALL error");
        assert!(
            matches!(err, VarError::Undefined(ref name) if name == "absent"),
            "expected Undefined(absent), got: {err}"
        );
    }

    // 43. Literal text surrounding a reference is preserved on both sides
    #[test]
    fn surrounding_literal_text_preserved() {
        let store = store_with(
            ScopeLevel::Project,
            vec![("name", VarValue::string("Alice"))],
        );
        let ctx = simple_ctx(&store);
        let result = interpolate("Hi ${name}, welcome!", &ctx).expect("SHALL interpolate");
        assert_eq!(result, "Hi Alice, welcome!");
    }

    // 44. Builtins take precedence over a same-named device store entry
    #[test]
    fn builtin_wins_over_device_store() {
        let main_store = VariableStore::new();
        let device_store = store_with(
            ScopeLevel::Project,
            vec![("_device", VarValue::string("from-device-store"))],
        );
        let mut device_stores = HashMap::new();
        device_stores.insert("pixel_9".to_string(), device_store);
        let mut builtins = HashMap::new();
        builtins.insert("_device".to_string(), "builtin-value".to_string());

        let ctx = InterpolationContext {
            store: &main_store,
            device: Some("pixel_9"),
            device_stores: Some(&device_stores),
            global_store: None,
            each_vars: None,
            builtins: Some(&builtins),
        };
        let result = interpolate("${_device}", &ctx).expect("SHALL resolve");
        assert_eq!(
            result, "builtin-value",
            "builtin SHALL win over device store"
        );
    }
}
