//! Tag the values a step or a fixture marks secret, so golem masks them in
//! all output ([`golem_common::secrets`]).

use golem_parser::Step;
use golem_vars::{VarValue, VariableStore};

/// Mask `value` from now on, shown as `${reference}`. An object's leaves
/// are each masked, shown by their own path.
pub fn register_value(value: &VarValue, reference: &str) {
    match value {
        VarValue::String(s) => golem_common::secrets::register(s, &format!("${{{reference}}}")),
        VarValue::Object(fields) => {
            for (key, field) in fields {
                register_value(field, &format!("{reference}.{key}"));
            }
        }
    }
}

/// Tag what `step` saved under `save_to`, as its `secret` says: `true` for
/// the whole value, or a list of paths in it. Returns the paths that name
/// nothing in the saved value.
pub fn register_step(step: &Step, vars: &VariableStore) -> Vec<String> {
    let (Some(secret), Some(name)) = (step.params.get("secret"), step.save_to.as_deref()) else {
        return Vec::new();
    };
    let Some(saved) = vars.get(name) else {
        return Vec::new();
    };
    match secret {
        toml::Value::Boolean(true) => {
            register_value(saved, name);
            Vec::new()
        }
        toml::Value::Array(paths) => paths
            .iter()
            .filter_map(toml::Value::as_str)
            .filter(|path| match saved.get_path(path) {
                Some(value) => {
                    register_value(value, &format!("{name}.{path}"));
                    false
                }
                None => true,
            })
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_vars::ScopeLevel;
    use std::collections::HashMap;

    fn step(secret: &str) -> Step {
        toml::from_str(&format!(
            "action = \"post_http\"\nsave_to = \"user\"\nsecret = {secret}\n"
        ))
        .expect("step")
    }

    fn store() -> VariableStore {
        let mut vars = VariableStore::new();
        let mut user = HashMap::new();
        user.insert("id".to_string(), VarValue::string("u-42"));
        user.insert("token".to_string(), VarValue::string("tok-abc-123"));
        vars.set_in_scope(ScopeLevel::Flow, "user", VarValue::Object(user));
        vars
    }

    #[test]
    fn named_paths_are_masked_and_the_rest_is_not() {
        let missing = register_step(&step("[\"token\", \"tokn\"]"), &store());
        assert_eq!(missing, vec!["tokn".to_string()]);
        assert_eq!(
            golem_common::secrets::mask("id u-42 token tok-abc-123"),
            "id u-42 token ${user.token}"
        );
    }

    #[test]
    fn secret_true_masks_every_leaf() {
        assert!(register_step(&step("true"), &store()).is_empty());
        assert_eq!(
            golem_common::secrets::mask("u-42 tok-abc-123"),
            "${user.id} ${user.token}"
        );
    }
}
