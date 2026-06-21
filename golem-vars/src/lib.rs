// golem-vars: variable and data generation

mod card_loader;
pub mod evaluate;
pub mod generators;
pub mod geo;
pub mod geo_loader;
pub mod interpolation;
pub mod seed;
pub mod structured;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Errors that can occur during variable resolution and interpolation.
#[derive(Error, Debug)]
pub enum VarError {
    #[error("undefined variable: {0}")]
    Undefined(String),
    #[error("property \"{property}\" not found on \"{object}\"")]
    PropertyNotFound { object: String, property: String },
    #[error("\"{0}\" is not a structured object")]
    NotAnObject(String),
    #[error("unclosed variable reference")]
    UnclosedReference,
    #[error("{0}")]
    Other(String),
}

/// A variable value — either a plain string or a nested object map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VarValue {
    String(String),
    Object(HashMap<String, VarValue>),
}

impl VarValue {
    /// Create a string value.
    pub fn string(s: impl Into<String>) -> Self {
        VarValue::String(s.into())
    }

    /// Create an object value from a vec of key-value entries.
    pub fn object(entries: Vec<(impl Into<String>, VarValue)>) -> Self {
        let map = entries.into_iter().map(|(k, v)| (k.into(), v)).collect();
        VarValue::Object(map)
    }

    /// Returns the inner string if this is a `String` variant, or `None`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            VarValue::String(s) => Some(s),
            VarValue::Object(_) => None,
        }
    }

    /// Returns the inner map if this is an `Object` variant, or `None`.
    pub fn as_object(&self) -> Option<&HashMap<String, VarValue>> {
        match self {
            VarValue::Object(map) => Some(map),
            VarValue::String(_) => None,
        }
    }

    /// Navigate a dot-separated path (e.g. "user.address.city") into nested objects.
    /// Returns `None` if any segment is missing or if a non-object is encountered mid-path.
    pub fn get_path(&self, path: &str) -> Option<&VarValue> {
        let mut current = self;
        for segment in path.split('.') {
            match current {
                VarValue::Object(map) => {
                    current = map.get(segment)?;
                }
                VarValue::String(_) => return None,
            }
        }
        Some(current)
    }
}

/// Priority levels for variable scopes, ordered from highest to lowest priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeLevel {
    Cli,
    Flow,
    Fixture,
    Project,
    Generator,
}

impl ScopeLevel {
    /// Returns the numeric priority for this scope level.
    /// Higher values indicate higher priority.
    fn priority(self) -> u8 {
        match self {
            ScopeLevel::Cli => 5,
            ScopeLevel::Flow => 4,
            ScopeLevel::Fixture => 3,
            ScopeLevel::Project => 2,
            ScopeLevel::Generator => 1,
        }
    }
}

/// A named scope containing variables at a given priority level.
#[derive(Debug, Clone)]
pub struct Scope {
    pub level: ScopeLevel,
    pub name: Option<String>,
    pub vars: HashMap<String, VarValue>,
}

impl Scope {
    /// Create an unnamed scope at the given level.
    pub fn new(level: ScopeLevel) -> Self {
        Self {
            level,
            name: None,
            vars: HashMap::new(),
        }
    }

    /// Create a named scope at the given level.
    pub fn with_name(level: ScopeLevel, name: impl Into<String>) -> Self {
        Self {
            level,
            name: Some(name.into()),
            vars: HashMap::new(),
        }
    }

    /// Set a variable in this scope.
    pub fn set(&mut self, key: impl Into<String>, value: VarValue) {
        self.vars.insert(key.into(), value);
    }

    /// Get a variable from this scope.
    pub fn get(&self, key: &str) -> Option<&VarValue> {
        self.vars.get(key)
    }
}

/// A store of variable scopes, supporting priority-based resolution.
/// Scopes at the front of the list have highest priority.
#[derive(Debug, Clone)]
pub struct VariableStore {
    scopes: Vec<Scope>,
}

impl VariableStore {
    /// Create an empty variable store.
    pub fn new() -> Self {
        Self { scopes: Vec::new() }
    }

    /// Push a scope to the front (highest priority position).
    ///
    /// Deduplicates by level: if a scope with the same `ScopeLevel` already
    /// exists, the incoming scope's vars are merged into it (consistent with
    /// `merge_scope`) rather than creating a second scope at that level. This
    /// keeps `resolve()` (first-of-level wins) and `get()` (front scope) in
    /// agreement.
    pub fn push_scope(&mut self, scope: Scope) {
        for existing in &mut self.scopes {
            if existing.level == scope.level {
                for (key, value) in scope.vars {
                    existing.vars.insert(key, value);
                }
                return;
            }
        }
        self.scopes.insert(0, scope);
    }

    /// Set a variable in the first (highest priority) scope.
    /// If no scopes exist, this is a no-op.
    pub fn set(&mut self, key: impl Into<String>, value: VarValue) {
        if let Some(scope) = self.scopes.first_mut() {
            scope.set(key, value);
        }
    }

    /// Resolve a variable by searching all scopes in priority order.
    /// Returns the first match found.
    pub fn get(&self, key: &str) -> Option<&VarValue> {
        for scope in &self.scopes {
            if let Some(val) = scope.get(key) {
                return Some(val);
            }
        }
        None
    }

    /// Resolve a variable by searching scopes in priority order (highest first).
    /// Returns an error if the variable is not found in any scope.
    pub fn resolve(&self, key: &str) -> Result<&VarValue, VarError> {
        // Sort conceptually by priority: iterate all scopes and pick the one
        // with the highest ScopeLevel priority that contains the key.
        let mut best: Option<(&VarValue, u8)> = None;
        for scope in &self.scopes {
            if let Some(val) = scope.get(key) {
                let prio = scope.level.priority();
                match best {
                    Some((_, current_prio)) if prio <= current_prio => {}
                    _ => best = Some((val, prio)),
                }
            }
        }
        best.map(|(val, _)| val)
            .ok_or_else(|| VarError::Undefined(key.to_string()))
    }

    /// Set a variable in a specific scope level.
    /// If a scope with the given level exists, the variable is added to it.
    /// If no scope with that level exists, a new scope is created and pushed.
    pub fn set_in_scope(&mut self, level: ScopeLevel, key: impl Into<String>, value: VarValue) {
        let key = key.into();
        for scope in &mut self.scopes {
            if scope.level == level {
                scope.set(key, value);
                return;
            }
        }
        // No scope with this level exists; create one and push it
        let mut scope = Scope::new(level);
        scope.set(key, value);
        self.push_scope(scope);
    }

    /// Check if a variable exists in any scope.
    pub fn has(&self, key: &str) -> bool {
        self.scopes.iter().any(|scope| scope.get(key).is_some())
    }

    /// Remove a variable from all scopes.
    pub fn remove(&mut self, key: &str) {
        for scope in &mut self.scopes {
            scope.vars.remove(key);
        }
    }

    /// Merge variables from a scope into an existing scope of the same level.
    /// If no scope with the same level exists, the scope is pushed as a new entry.
    pub fn merge_scope(&mut self, incoming: Scope) {
        for scope in &mut self.scopes {
            if scope.level == incoming.level {
                for (key, value) in incoming.vars {
                    scope.vars.insert(key, value);
                }
                return;
            }
        }
        // No matching level found; push as new scope
        self.push_scope(incoming);
    }

    /// Returns a slice of all scopes, in priority order (highest first).
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }
}

impl Default for VariableStore {
    fn default() -> Self {
        Self::new()
    }
}

/// A parsed generator definition in the form "fake:name(param=value,...)".
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratorDef {
    pub name: String,
    pub params: HashMap<String, String>,
}

impl GeneratorDef {
    /// Parse a generator definition string like "fake:email" or "fake:name(prefix=test,domain=acme.com)".
    /// Returns `None` if the input doesn't start with "fake:".
    pub fn parse(input: &str) -> Option<Self> {
        let rest = input.strip_prefix("fake:")?;

        // Check if there are parameters in parentheses
        if let Some(paren_start) = rest.find('(') {
            let name = rest[..paren_start].to_string();
            let params_str = rest.get(paren_start + 1..)?.strip_suffix(')')?;

            let mut params = HashMap::new();
            if !params_str.is_empty() {
                for pair in params_str.split(',') {
                    let mut parts = pair.splitn(2, '=');
                    let key = parts.next()?.trim().to_string();
                    let value = parts.next()?.trim().to_string();
                    params.insert(key, value);
                }
            }

            Some(Self { name, params })
        } else {
            Some(Self {
                name: rest.to_string(),
                params: HashMap::new(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn var_value_string_as_str_returns_some() {
        let val = VarValue::string("hello");
        assert_eq!(val.as_str(), Some("hello"));
    }

    #[test]
    fn var_value_string_as_object_returns_none() {
        let val = VarValue::string("hello");
        assert!(val.as_object().is_none());
    }

    #[test]
    fn var_value_object_as_object_returns_some() {
        let val = VarValue::object(vec![("key", VarValue::string("value"))]);
        assert!(val.as_object().is_some());
    }

    #[test]
    fn var_value_object_as_str_returns_none() {
        let val = VarValue::object(vec![("key", VarValue::string("value"))]);
        assert!(val.as_str().is_none());
    }

    #[test]
    fn var_value_get_path_navigates_nested_objects() {
        let val = VarValue::object(vec![(
            "user",
            VarValue::object(vec![(
                "address",
                VarValue::object(vec![("city", VarValue::string("Portland"))]),
            )]),
        )]);

        let result = val.get_path("user.address.city");
        assert_eq!(result, Some(&VarValue::string("Portland")));
    }

    #[test]
    fn var_value_get_path_returns_none_for_missing_keys() {
        let val = VarValue::object(vec![("name", VarValue::string("Alice"))]);
        assert!(val.get_path("missing.key").is_none());
    }

    #[test]
    fn var_value_get_path_returns_none_when_traversing_string() {
        let val = VarValue::object(vec![("name", VarValue::string("Alice"))]);
        assert!(val.get_path("name.first").is_none());
    }

    #[test]
    fn scope_set_and_get() {
        let mut scope = Scope::new(ScopeLevel::Project);
        scope.set("url", VarValue::string("https://example.com"));
        assert_eq!(
            scope.get("url"),
            Some(&VarValue::string("https://example.com"))
        );
    }

    #[test]
    fn variable_store_resolves_highest_priority_first() {
        let mut store = VariableStore::new();

        let mut project_scope = Scope::new(ScopeLevel::Project);
        project_scope.set("env", VarValue::string("production"));
        store.push_scope(project_scope);

        let mut cli_scope = Scope::new(ScopeLevel::Cli);
        cli_scope.set("env", VarValue::string("staging"));
        store.push_scope(cli_scope);

        // CLI scope was pushed last, so it's at the front (highest priority)
        assert_eq!(store.get("env"), Some(&VarValue::string("staging")));
    }

    #[test]
    fn variable_store_returns_none_for_missing() {
        let store = VariableStore::new();
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn generator_def_parse_no_params() {
        let def = GeneratorDef::parse("fake:email");
        assert_eq!(
            def,
            Some(GeneratorDef {
                name: "email".to_string(),
                params: HashMap::new(),
            })
        );
    }

    #[test]
    fn generator_def_parse_with_params() {
        let def = GeneratorDef::parse("fake:email(prefix=test,domain=acme.com)");
        let gen = def.expect("should parse");
        assert_eq!(gen.name, "email");
        assert_eq!(gen.params.get("prefix"), Some(&"test".to_string()));
        assert_eq!(gen.params.get("domain"), Some(&"acme.com".to_string()));
    }

    #[test]
    fn generator_def_parse_returns_none_for_non_fake() {
        assert!(GeneratorDef::parse("not_fake:email").is_none());
        assert!(GeneratorDef::parse("just a string").is_none());
        assert!(GeneratorDef::parse("").is_none());
    }

    #[test]
    fn var_value_serialization_round_trip() {
        let val = VarValue::object(vec![
            ("name", VarValue::string("Alice")),
            (
                "address",
                VarValue::object(vec![("city", VarValue::string("Portland"))]),
            ),
        ]);

        let json = serde_json::to_string(&val).expect("should serialize");
        let deserialized: VarValue = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(val, deserialized);
    }

    // --- resolve tests ---

    #[test]
    fn resolve_returns_value_from_highest_priority_scope() {
        let mut store = VariableStore::new();

        let mut project = Scope::new(ScopeLevel::Project);
        project.set("url", VarValue::string("project-url"));
        store.push_scope(project);

        let mut cli = Scope::new(ScopeLevel::Cli);
        cli.set("url", VarValue::string("cli-url"));
        store.push_scope(cli);

        let result = store.resolve("url").expect("should resolve");
        assert_eq!(result, &VarValue::string("cli-url"));
    }

    #[test]
    fn resolve_returns_error_for_undefined_variable() {
        let store = VariableStore::new();
        let result = store.resolve("missing");
        assert!(result.is_err());
        let err = result.expect_err("should be error");
        assert!(
            matches!(err, VarError::Undefined(ref name) if name == "missing"),
            "expected Undefined error, got: {err}"
        );
    }

    // --- set_in_scope tests ---

    #[test]
    fn set_in_scope_creates_scope_if_not_present() {
        let mut store = VariableStore::new();
        store.set_in_scope(ScopeLevel::Flow, "token", VarValue::string("abc123"));

        let result = store.resolve("token").expect("should resolve");
        assert_eq!(result, &VarValue::string("abc123"));
        assert_eq!(store.scopes().len(), 1);
        assert_eq!(store.scopes()[0].level, ScopeLevel::Flow);
    }

    #[test]
    fn set_in_scope_adds_to_existing_scope() {
        let mut store = VariableStore::new();
        let scope = Scope::new(ScopeLevel::Project);
        store.push_scope(scope);

        store.set_in_scope(ScopeLevel::Project, "key1", VarValue::string("val1"));
        store.set_in_scope(ScopeLevel::Project, "key2", VarValue::string("val2"));

        // Should still only have one scope
        assert_eq!(store.scopes().len(), 1);
        assert_eq!(
            store.resolve("key1").expect("should resolve"),
            &VarValue::string("val1")
        );
        assert_eq!(
            store.resolve("key2").expect("should resolve"),
            &VarValue::string("val2")
        );
    }

    // --- has tests ---

    #[test]
    fn has_returns_true_for_existing_false_for_missing() {
        let mut store = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Project);
        scope.set("host", VarValue::string("localhost"));
        store.push_scope(scope);

        assert!(store.has("host"));
        assert!(!store.has("port"));
    }

    // --- remove tests ---

    #[test]
    fn remove_clears_from_all_scopes() {
        let mut store = VariableStore::new();

        let mut project = Scope::new(ScopeLevel::Project);
        project.set("env", VarValue::string("production"));
        store.push_scope(project);

        let mut cli = Scope::new(ScopeLevel::Cli);
        cli.set("env", VarValue::string("staging"));
        store.push_scope(cli);

        assert!(store.has("env"));
        store.remove("env");
        assert!(!store.has("env"));
        assert!(store.resolve("env").is_err());
    }

    // --- merge_scope tests ---

    #[test]
    fn merge_scope_merges_into_existing_scope_of_same_level() {
        let mut store = VariableStore::new();

        let mut existing = Scope::new(ScopeLevel::Project);
        existing.set("key1", VarValue::string("val1"));
        store.push_scope(existing);

        let mut incoming = Scope::new(ScopeLevel::Project);
        incoming.set("key2", VarValue::string("val2"));
        store.merge_scope(incoming);

        // Should still be one scope
        assert_eq!(store.scopes().len(), 1);
        assert_eq!(
            store.resolve("key1").expect("should resolve"),
            &VarValue::string("val1")
        );
        assert_eq!(
            store.resolve("key2").expect("should resolve"),
            &VarValue::string("val2")
        );
    }

    #[test]
    fn merge_scope_pushes_new_scope_if_no_matching_level() {
        let mut store = VariableStore::new();

        let mut project = Scope::new(ScopeLevel::Project);
        project.set("key1", VarValue::string("val1"));
        store.push_scope(project);

        let mut cli = Scope::new(ScopeLevel::Cli);
        cli.set("key2", VarValue::string("val2"));
        store.merge_scope(cli);

        assert_eq!(store.scopes().len(), 2);
        assert!(store.has("key1"));
        assert!(store.has("key2"));
    }

    // --- scope priority ordering tests ---

    #[test]
    fn cli_scope_wins_over_flow_scope() {
        let mut store = VariableStore::new();

        let mut flow = Scope::new(ScopeLevel::Flow);
        flow.set("x", VarValue::string("flow"));
        store.push_scope(flow);

        let mut cli = Scope::new(ScopeLevel::Cli);
        cli.set("x", VarValue::string("cli"));
        store.push_scope(cli);

        assert_eq!(
            store.resolve("x").expect("should resolve"),
            &VarValue::string("cli")
        );
    }

    #[test]
    fn flow_scope_wins_over_fixture_scope() {
        let mut store = VariableStore::new();

        let mut fixture = Scope::new(ScopeLevel::Fixture);
        fixture.set("x", VarValue::string("fixture"));
        store.push_scope(fixture);

        let mut flow = Scope::new(ScopeLevel::Flow);
        flow.set("x", VarValue::string("flow"));
        store.push_scope(flow);

        assert_eq!(
            store.resolve("x").expect("should resolve"),
            &VarValue::string("flow")
        );
    }

    #[test]
    fn fixture_scope_wins_over_project_scope() {
        let mut store = VariableStore::new();

        let mut project = Scope::new(ScopeLevel::Project);
        project.set("x", VarValue::string("project"));
        store.push_scope(project);

        let mut fixture = Scope::new(ScopeLevel::Fixture);
        fixture.set("x", VarValue::string("fixture"));
        store.push_scope(fixture);

        assert_eq!(
            store.resolve("x").expect("should resolve"),
            &VarValue::string("fixture")
        );
    }

    #[test]
    fn project_scope_wins_over_generator_scope() {
        let mut store = VariableStore::new();

        let mut generator = Scope::new(ScopeLevel::Generator);
        generator.set("x", VarValue::string("generator"));
        store.push_scope(generator);

        let mut project = Scope::new(ScopeLevel::Project);
        project.set("x", VarValue::string("project"));
        store.push_scope(project);

        assert_eq!(
            store.resolve("x").expect("should resolve"),
            &VarValue::string("project")
        );
    }

    // --- VarValue::object and get_path edge cases ---

    // 1. object with no entries produces an empty Object map.
    #[test]
    fn var_value_object_empty_entries_is_empty_map() {
        let val = VarValue::object(Vec::<(&str, VarValue)>::new());
        let map = val.as_object().expect("object SHALL yield a map");
        assert!(map.is_empty(), "empty entries SHALL produce empty map");
    }

    // 2. object with multiple entries retains all keys.
    #[test]
    fn var_value_object_retains_all_entries() {
        let val = VarValue::object(vec![
            ("a", VarValue::string("1")),
            ("b", VarValue::string("2")),
        ]);
        let map = val.as_object().expect("object SHALL yield a map");
        assert_eq!(map.get("a"), Some(&VarValue::string("1")));
        assert_eq!(map.get("b"), Some(&VarValue::string("2")));
    }

    // 3. get_path with a single segment returns the direct child.
    #[test]
    fn var_value_get_path_single_segment_returns_child() {
        let val = VarValue::object(vec![("name", VarValue::string("Bob"))]);
        assert_eq!(val.get_path("name"), Some(&VarValue::string("Bob")));
    }

    // 4. A scalar String root rejects any path: the first segment hits the
    //    `VarValue::String(_) => return None` arm before any lookup. This holds
    //    both for a real segment and for the empty-string path (whose single
    //    empty segment still meets the String arm first).
    #[test]
    fn var_value_get_path_on_string_root_returns_none() {
        let val = VarValue::string("scalar");
        assert!(
            val.get_path("anything").is_none(),
            "string root SHALL NOT resolve a real path segment"
        );
        assert!(
            val.get_path("").is_none(),
            "string root SHALL NOT resolve the empty path"
        );
    }

    // 5. get_path that bottoms out on a nested object returns the object itself.
    #[test]
    fn var_value_get_path_can_return_an_object() {
        let inner = VarValue::object(vec![("city", VarValue::string("Reno"))]);
        let val = VarValue::object(vec![("addr", inner.clone())]);
        assert_eq!(val.get_path("addr"), Some(&inner));
    }

    // --- Scope name / construction ---

    // 6. new() yields an unnamed scope; with_name() carries the name through.
    #[test]
    fn scope_new_is_unnamed_with_name_carries_name() {
        let unnamed = Scope::new(ScopeLevel::Cli);
        assert_eq!(unnamed.name, None, "new() SHALL be unnamed");

        let named = Scope::with_name(ScopeLevel::Fixture, "login");
        assert_eq!(named.name.as_deref(), Some("login"));
        assert_eq!(named.level, ScopeLevel::Fixture);
    }

    // 7. Scope::get returns None for a key that was never set.
    #[test]
    fn scope_get_returns_none_for_missing_key() {
        let scope = Scope::new(ScopeLevel::Project);
        assert!(scope.get("nope").is_none(), "missing key SHALL be None");
    }

    // --- VariableStore::set / push_scope ordering ---

    // 8. set() writes into the first (front) scope only.
    #[test]
    fn store_set_writes_into_front_scope() {
        let mut store = VariableStore::new();
        store.push_scope(Scope::new(ScopeLevel::Project));
        store.push_scope(Scope::new(ScopeLevel::Cli));

        store.set("k", VarValue::string("v"));

        // Front scope is the Cli scope (pushed last).
        assert_eq!(store.scopes()[0].level, ScopeLevel::Cli);
        assert_eq!(store.scopes()[0].get("k"), Some(&VarValue::string("v")));
        assert!(
            store.scopes()[1].get("k").is_none(),
            "set SHALL NOT touch lower scopes"
        );
    }

    // 9. set() on an empty store is a silent no-op.
    #[test]
    fn store_set_on_empty_store_is_noop() {
        let mut store = VariableStore::new();
        store.set("k", VarValue::string("v"));
        assert!(
            store.get("k").is_none(),
            "set with no scopes SHALL be a no-op"
        );
        assert!(store.scopes().is_empty(), "no scope SHALL be created");
    }

    // 10. push_scope places the newest scope at the front of the list.
    #[test]
    fn push_scope_inserts_at_front() {
        let mut store = VariableStore::new();
        store.push_scope(Scope::new(ScopeLevel::Project));
        store.push_scope(Scope::new(ScopeLevel::Flow));

        assert_eq!(store.scopes()[0].level, ScopeLevel::Flow);
        assert_eq!(store.scopes()[1].level, ScopeLevel::Project);
    }

    // 10b. push_scope dedups by level: pushing a scope whose level already
    //      exists merges into it rather than creating a duplicate, keeping
    //      resolve() and get() in agreement.
    #[test]
    fn push_scope_dedups_by_level() {
        let mut store = VariableStore::new();

        let mut first = Scope::new(ScopeLevel::Cli);
        first.set("x", VarValue::string("first"));
        store.push_scope(first);

        let mut second = Scope::new(ScopeLevel::Cli);
        second.set("x", VarValue::string("second"));
        second.set("y", VarValue::string("extra"));
        store.push_scope(second);

        // Only one Cli scope SHALL exist.
        assert_eq!(
            store.scopes().len(),
            1,
            "push_scope SHALL NOT create a duplicate level"
        );
        // Incoming vars SHALL be merged in (overwriting collisions).
        assert_eq!(store.get("x"), Some(&VarValue::string("second")));
        assert_eq!(store.get("y"), Some(&VarValue::string("extra")));

        // resolve() and get() SHALL agree (no diverging duplicate level).
        assert_eq!(
            store.resolve("x").expect("should resolve"),
            store.get("x").expect("should get"),
            "resolve() and get() SHALL agree after dedup"
        );
    }

    // --- resolve uses ScopeLevel priority, not list position ---

    // 11. resolve picks the highest-priority scope even when a lower-priority
    //     scope sits at the front of the list (diverging from get()).
    #[test]
    fn resolve_uses_priority_not_list_order() {
        let mut store = VariableStore::new();

        // Cli (highest priority) pushed first => ends up at the back.
        let mut cli = Scope::new(ScopeLevel::Cli);
        cli.set("x", VarValue::string("cli"));
        store.push_scope(cli);

        // Generator (lowest priority) pushed last => sits at the front.
        let mut generator = Scope::new(ScopeLevel::Generator);
        generator.set("x", VarValue::string("generator"));
        store.push_scope(generator);

        // get() follows list order and returns the front (generator) value.
        assert_eq!(store.get("x"), Some(&VarValue::string("generator")));
        // resolve() follows priority and returns the Cli value.
        assert_eq!(
            store.resolve("x").expect("should resolve"),
            &VarValue::string("cli"),
            "resolve SHALL pick highest ScopeLevel priority"
        );
    }

    // 12. resolve from a single scope returns that scope's value.
    #[test]
    fn resolve_single_scope_returns_value() {
        let mut store = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Project);
        scope.set("only", VarValue::string("here"));
        store.push_scope(scope);

        assert_eq!(
            store.resolve("only").expect("should resolve"),
            &VarValue::string("here")
        );
    }

    // --- ScopeLevel priority ordering (private fn, reachable in-module) ---

    // 13. priority is strictly descending Cli > Flow > Fixture > Project > Generator.
    #[test]
    fn scope_level_priority_is_strictly_descending() {
        assert!(ScopeLevel::Cli.priority() > ScopeLevel::Flow.priority());
        assert!(ScopeLevel::Flow.priority() > ScopeLevel::Fixture.priority());
        assert!(ScopeLevel::Fixture.priority() > ScopeLevel::Project.priority());
        assert!(ScopeLevel::Project.priority() > ScopeLevel::Generator.priority());
    }

    // --- remove on a store that never had the key ---

    // 14. remove() of an absent key leaves the store untouched and is safe.
    #[test]
    fn remove_absent_key_is_safe() {
        let mut store = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Project);
        scope.set("kept", VarValue::string("v"));
        store.push_scope(scope);

        store.remove("never_set");
        assert!(store.has("kept"), "unrelated key SHALL remain after remove");
    }

    // --- merge_scope overwrites colliding keys ---

    // 15. merging into an existing same-level scope overwrites colliding keys
    //     with the incoming value.
    #[test]
    fn merge_scope_overwrites_colliding_keys() {
        let mut store = VariableStore::new();

        let mut existing = Scope::new(ScopeLevel::Project);
        existing.set("shared", VarValue::string("old"));
        store.push_scope(existing);

        let mut incoming = Scope::new(ScopeLevel::Project);
        incoming.set("shared", VarValue::string("new"));
        store.merge_scope(incoming);

        assert_eq!(store.scopes().len(), 1, "SHALL stay a single scope");
        assert_eq!(
            store.resolve("shared").expect("should resolve"),
            &VarValue::string("new"),
            "incoming value SHALL win on collision"
        );
    }

    // --- VariableStore::default ---

    // 16. Default produces an empty store equivalent to new().
    #[test]
    fn variable_store_default_is_empty() {
        let store = VariableStore::default();
        assert!(
            store.scopes().is_empty(),
            "default store SHALL have no scopes"
        );
        assert!(store.get("any").is_none());
    }

    // --- GeneratorDef::parse edge cases ---

    // 17. empty parens yield a named generator with no params.
    #[test]
    fn generator_def_parse_empty_parens_no_params() {
        let gen = GeneratorDef::parse("fake:uuid()").expect("should parse");
        assert_eq!(gen.name, "uuid");
        assert!(gen.params.is_empty(), "empty parens SHALL yield no params");
    }

    // 18. an unclosed paren (no trailing ')') fails to parse.
    #[test]
    fn generator_def_parse_unclosed_paren_returns_none() {
        assert!(
            GeneratorDef::parse("fake:name(prefix=x").is_none(),
            "missing closing paren SHALL fail to parse"
        );
    }

    // 19. a param with no '=' fails to parse (missing value side).
    #[test]
    fn generator_def_parse_param_without_equals_returns_none() {
        assert!(
            GeneratorDef::parse("fake:name(bare)").is_none(),
            "param lacking '=' SHALL fail to parse"
        );
    }

    // 20. surrounding whitespace in params is trimmed from keys and values.
    #[test]
    fn generator_def_parse_trims_param_whitespace() {
        let gen = GeneratorDef::parse("fake:name( prefix = test )").expect("should parse");
        assert_eq!(gen.params.get("prefix"), Some(&"test".to_string()));
    }

    // 21. an empty generator name after "fake:" is still accepted.
    #[test]
    fn generator_def_parse_empty_name_is_accepted() {
        let gen = GeneratorDef::parse("fake:").expect("should parse");
        assert_eq!(gen.name, "", "empty name SHALL parse to empty string");
        assert!(gen.params.is_empty());
    }

    // 22. a value containing '=' keeps everything after the first '=' (splitn(2)).
    #[test]
    fn generator_def_parse_value_with_equals_is_preserved() {
        let gen = GeneratorDef::parse("fake:token(seed=a=b=c)").expect("should parse");
        assert_eq!(gen.params.get("seed"), Some(&"a=b=c".to_string()));
    }
}
