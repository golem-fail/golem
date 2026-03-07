// golem-vars: variable and data generation

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
}
