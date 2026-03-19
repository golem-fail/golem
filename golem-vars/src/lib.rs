// golem-vars: variable and data generation

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
        let map = entries
            .into_iter()
            .map(|(k, v)| (k.into(), v))
            .collect();
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
    pub fn push_scope(&mut self, scope: Scope) {
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
            let params_str = rest
                .get(paren_start + 1..)?
                .strip_suffix(')')?;

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
        assert_eq!(
            store.get("env"),
            Some(&VarValue::string("staging"))
        );
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
}
