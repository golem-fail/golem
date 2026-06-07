use std::collections::HashMap;

use anyhow::Result;
use golem_parser::Block;
use golem_vars::{Scope, ScopeLevel, VarValue, VariableStore};

/// Configuration for a sub-flow execution.
pub struct SubFlowConfig {
    /// Path to the child flow file (relative to current flow).
    pub flow_path: String,
    /// Variable overrides to pass into the child flow.
    pub vars: HashMap<String, String>,
    /// Mapping of child variable names to parent variable names for result propagation.
    pub save_to: HashMap<String, String>,
}

/// Extract sub-flow configuration from a block, if the block has `run_flow` set.
///
/// Returns `None` for blocks that do not reference a sub-flow.
pub fn extract_subflow_config(block: &Block) -> Option<SubFlowConfig> {
    let flow_path = block.run_flow.as_ref()?;
    Some(SubFlowConfig {
        flow_path: flow_path.clone(),
        vars: block.vars.clone(),
        save_to: block.save_to.clone(),
    })
}

/// Prepare a child `VariableStore` by cloning the parent and applying overrides.
///
/// All parent variables are inherited. The `overrides` map provides additional
/// values (or replacements) that are set in a new `Flow`-level scope so they
/// take precedence over inherited values.
pub fn prepare_child_vars(
    parent: &VariableStore,
    overrides: &HashMap<String, String>,
) -> VariableStore {
    let mut child = parent.clone();
    if !overrides.is_empty() {
        let mut scope = Scope::new(ScopeLevel::Flow);
        for (key, value) in overrides {
            scope.set(key.clone(), VarValue::string(value.clone()));
        }
        child.push_scope(scope);
    }
    child
}

/// Propagate variables from a completed child store back into the parent store.
///
/// For each entry in `save_to`, the child variable named by the key is read and
/// written into the parent store under the name given by the value.
///
/// Returns an error if any child variable referenced in `save_to` does not exist.
pub fn propagate_results(
    child: &VariableStore,
    parent: &mut VariableStore,
    save_to: &HashMap<String, String>,
) -> Result<()> {
    for (child_var, parent_var) in save_to {
        let value = match child.get(child_var) {
            Some(v) => v.clone(),
            None => crate::fail_code!(golem_events::FailureCode::ParseMissingReference, "sub-flow variable \"{child_var}\" not found for propagation"),
        };
        parent.set_in_scope(ScopeLevel::Flow, parent_var.clone(), value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_parser::Block;
    use golem_vars::{Scope, ScopeLevel, VarValue, VariableStore};
    use std::collections::HashMap;

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

    fn make_parent_store() -> VariableStore {
        let mut store = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Flow);
        scope.set("user_email", VarValue::string("alice@example.com"));
        scope.set("user_password", VarValue::string("secret123"));
        scope.set("base_url", VarValue::string("https://example.com"));
        store.push_scope(scope);
        store
    }

    fn make_block_with_run_flow(
        run_flow: &str,
        vars: HashMap<String, String>,
        save_to: HashMap<String, String>,
    ) -> Block {
        Block {
            name: Some("run_login".to_string()),
            app: None,
            steps: Vec::new(),
            next: None,
            branch: Vec::new(),
            for_each: None,
            r#where: None,
            run_flow: Some(run_flow.to_string()),
            max_iterations: None,
            vars,
            save_to,
            record: None,
        }
    }

    fn make_regular_block() -> Block {
        Block {
            name: Some("regular".to_string()),
            app: None,
            steps: Vec::new(),
            next: None,
            branch: Vec::new(),
            for_each: None,
            r#where: None,
            run_flow: None,
            max_iterations: None,
            vars: HashMap::new(),
            save_to: HashMap::new(),
            record: None,
        }
    }

    // ---------------------------------------------------------------
    // 1. prepare_child_vars inherits parent variables
    // ---------------------------------------------------------------
    #[test]
    fn prepare_child_vars_inherits_parent_variables() {
        let parent = make_parent_store();
        let overrides = HashMap::new();

        let child = prepare_child_vars(&parent, &overrides);

        assert_eq!(
            child.get("user_email"),
            Some(&VarValue::string("alice@example.com"))
        );
        assert_eq!(
            child.get("user_password"),
            Some(&VarValue::string("secret123"))
        );
        assert_eq!(
            child.get("base_url"),
            Some(&VarValue::string("https://example.com"))
        );
    }

    // ---------------------------------------------------------------
    // 2. prepare_child_vars overrides apply correctly
    // ---------------------------------------------------------------
    #[test]
    fn prepare_child_vars_overrides_apply() {
        let parent = make_parent_store();
        let mut overrides = HashMap::new();
        overrides.insert("user_email".to_string(), "bob@example.com".to_string());
        overrides.insert("extra_var".to_string(), "extra_value".to_string());

        let child = prepare_child_vars(&parent, &overrides);

        // Override replaces parent value
        assert_eq!(
            child.get("user_email"),
            Some(&VarValue::string("bob@example.com"))
        );
        // New variable from override is present
        assert_eq!(
            child.get("extra_var"),
            Some(&VarValue::string("extra_value"))
        );
        // Non-overridden parent variable is still inherited
        assert_eq!(
            child.get("user_password"),
            Some(&VarValue::string("secret123"))
        );
    }

    // ---------------------------------------------------------------
    // 3. propagate_results maps child vars to parent
    // ---------------------------------------------------------------
    #[test]
    fn propagate_results_maps_child_to_parent() {
        let mut child_store = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Flow);
        scope.set("token", VarValue::string("jwt-abc-123"));
        child_store.push_scope(scope);

        let mut parent = make_parent_store();
        let mut save_to = HashMap::new();
        save_to.insert("token".to_string(), "session_token".to_string());

        propagate_results(&child_store, &mut parent, &save_to)
            .expect("propagation should succeed");

        assert_eq!(
            parent.get("session_token"),
            Some(&VarValue::string("jwt-abc-123"))
        );
    }

    // ---------------------------------------------------------------
    // 4. propagate_results with missing child var returns error
    // ---------------------------------------------------------------
    #[test]
    fn propagate_results_missing_child_var_errors() {
        let child_store = VariableStore::new();
        let mut parent = make_parent_store();
        let mut save_to = HashMap::new();
        save_to.insert("nonexistent".to_string(), "parent_var".to_string());

        let result = propagate_results(&child_store, &mut parent, &save_to);

        assert!(result.is_err());
        let err_msg = result.expect_err("should be error").to_string();
        assert!(
            err_msg.contains("nonexistent"),
            "error should mention missing variable: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 5. extract_subflow_config from block with run_flow
    // ---------------------------------------------------------------
    #[test]
    fn extract_subflow_config_from_run_flow_block() {
        let mut vars = HashMap::new();
        vars.insert("email".to_string(), "${user.email}".to_string());
        let mut save_to = HashMap::new();
        save_to.insert("token".to_string(), "session_token".to_string());

        let block = make_block_with_run_flow("auth/login.test.toml", vars.clone(), save_to.clone());

        let config = extract_subflow_config(&block).expect("should extract config");

        assert_eq!(config.flow_path, "auth/login.test.toml");
        assert_eq!(config.vars, vars);
        assert_eq!(config.save_to, save_to);
    }

    // ---------------------------------------------------------------
    // 6. extract_subflow_config returns None for regular block
    // ---------------------------------------------------------------
    #[test]
    fn extract_subflow_config_returns_none_for_regular_block() {
        let block = make_regular_block();
        assert!(extract_subflow_config(&block).is_none());
    }

    // ---------------------------------------------------------------
    // 7. prepare_child_vars with empty overrides clones parent
    // ---------------------------------------------------------------
    #[test]
    fn prepare_child_vars_empty_overrides_clones_parent() {
        let parent = make_parent_store();
        let overrides = HashMap::new();

        let child = prepare_child_vars(&parent, &overrides);

        // All parent vars present in child
        assert_eq!(
            child.get("user_email"),
            Some(&VarValue::string("alice@example.com"))
        );
        assert_eq!(
            child.get("user_password"),
            Some(&VarValue::string("secret123"))
        );
        assert_eq!(
            child.get("base_url"),
            Some(&VarValue::string("https://example.com"))
        );

        // No additional scopes were pushed (empty overrides should not add a scope)
        assert_eq!(child.scopes().len(), parent.scopes().len());
    }

    // ---------------------------------------------------------------
    // 8. propagate_results with empty save_to is no-op
    // ---------------------------------------------------------------
    #[test]
    fn propagate_results_empty_save_to_is_noop() {
        let child_store = VariableStore::new();
        let mut parent = make_parent_store();
        let save_to = HashMap::new();

        propagate_results(&child_store, &mut parent, &save_to)
            .expect("empty save_to should succeed");

        // Parent is unchanged
        assert_eq!(
            parent.get("user_email"),
            Some(&VarValue::string("alice@example.com"))
        );
    }

    // ---------------------------------------------------------------
    // 9. Multiple var mappings all propagated
    // ---------------------------------------------------------------
    #[test]
    fn propagate_results_multiple_mappings() {
        let mut child_store = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Flow);
        scope.set("token", VarValue::string("jwt-token"));
        scope.set("user_id", VarValue::string("42"));
        scope.set("role", VarValue::string("admin"));
        child_store.push_scope(scope);

        let mut parent = make_parent_store();
        let mut save_to = HashMap::new();
        save_to.insert("token".to_string(), "session_token".to_string());
        save_to.insert("user_id".to_string(), "logged_in_user_id".to_string());
        save_to.insert("role".to_string(), "user_role".to_string());

        propagate_results(&child_store, &mut parent, &save_to)
            .expect("propagation should succeed");

        assert_eq!(
            parent.get("session_token"),
            Some(&VarValue::string("jwt-token"))
        );
        assert_eq!(
            parent.get("logged_in_user_id"),
            Some(&VarValue::string("42"))
        );
        assert_eq!(
            parent.get("user_role"),
            Some(&VarValue::string("admin"))
        );
    }

    // ---------------------------------------------------------------
    // 10. propagate_results propagates structured (object) values
    // ---------------------------------------------------------------
    #[test]
    fn propagate_results_propagates_object_values() {
        let mut child_store = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Flow);
        let user_obj = VarValue::object(vec![
            ("name", VarValue::string("Alice")),
            ("email", VarValue::string("alice@test.com")),
        ]);
        scope.set("user", user_obj.clone());
        child_store.push_scope(scope);

        let mut parent = make_parent_store();
        let mut save_to = HashMap::new();
        save_to.insert("user".to_string(), "logged_in_user".to_string());

        propagate_results(&child_store, &mut parent, &save_to)
            .expect("propagation should succeed");

        let result = parent.get("logged_in_user").expect("should exist");
        assert_eq!(result, &user_obj);
    }

    // ---------------------------------------------------------------
    // 11. Child overrides do not mutate the parent store
    // ---------------------------------------------------------------
    #[test]
    fn child_overrides_do_not_mutate_parent() {
        let parent = make_parent_store();
        let mut overrides = HashMap::new();
        overrides.insert("user_email".to_string(), "override@example.com".to_string());

        let _child = prepare_child_vars(&parent, &overrides);

        // Parent still has original value
        assert_eq!(
            parent.get("user_email"),
            Some(&VarValue::string("alice@example.com"))
        );
    }

    // ---------------------------------------------------------------
    // 12. extract_subflow_config captures empty vars and save_to
    // ---------------------------------------------------------------
    #[test]
    fn extract_subflow_config_empty_vars_and_save_to() {
        let block = make_block_with_run_flow("flows/helper.toml", HashMap::new(), HashMap::new());

        let config = extract_subflow_config(&block).expect("should extract config");

        assert_eq!(config.flow_path, "flows/helper.toml");
        assert!(config.vars.is_empty());
        assert!(config.save_to.is_empty());
    }
}
