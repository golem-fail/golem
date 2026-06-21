//! Integration tests for advanced flow features: sub-flow variable passing,
//! data-driven row expansion, and fixture loading.
//!
//! These test the `golem-runner` modules working together through their public APIs.

use std::collections::HashMap;

use golem_parser::Block;
use golem_runner::data_driven::{apply_data_vars, expand_data_rows, get_runs};
use golem_runner::fixture_loader::load_fixture_into_store;
use golem_runner::subflow::{extract_subflow_config, prepare_child_vars, propagate_results};
use golem_vars::{Scope, ScopeLevel, VarValue, VariableStore};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn seeded_rng() -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(42)
}

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
        name: Some("run_child".to_string()),
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

/// Write a fixture TOML file under `__fixtures__/<name>.toml` within the given base directory.
fn write_fixture(base_dir: &std::path::Path, name: &str, content: &str) {
    let fixture_dir = base_dir.join("__fixtures__");
    let file_path = fixture_dir.join(format!("{name}.toml"));
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).expect("Failed to create fixture directory");
    }
    std::fs::write(&file_path, content).expect("Failed to write fixture file");
}

// ===========================================================================
// Sub-flow tests
// ===========================================================================

// ---------------------------------------------------------------------------
// 1. prepare_child_vars passes parent variables to child
// ---------------------------------------------------------------------------
#[test]
fn subflow_prepare_child_vars_passes_parent_vars() {
    let parent = make_parent_store();
    let overrides = HashMap::from([("env".to_string(), "staging".to_string())]);

    let child = prepare_child_vars(&parent, &overrides);

    // Parent variables are inherited
    assert_eq!(
        child.get("user_email"),
        Some(&VarValue::string("alice@example.com")),
    );
    assert_eq!(
        child.get("user_password"),
        Some(&VarValue::string("secret123")),
    );
    assert_eq!(
        child.get("base_url"),
        Some(&VarValue::string("https://example.com")),
    );
    // Override variable is present
    assert_eq!(child.get("env"), Some(&VarValue::string("staging")),);
}

// ---------------------------------------------------------------------------
// 2. propagate_results maps child vars back to parent
// ---------------------------------------------------------------------------
#[test]
fn subflow_propagate_results_maps_child_to_parent() {
    let mut child_store = VariableStore::new();
    let mut scope = Scope::new(ScopeLevel::Flow);
    scope.set("token", VarValue::string("jwt-abc-123"));
    scope.set("user_id", VarValue::string("42"));
    child_store.push_scope(scope);

    let mut parent = make_parent_store();
    let save_to = HashMap::from([
        ("token".to_string(), "session_token".to_string()),
        ("user_id".to_string(), "active_user_id".to_string()),
    ]);

    propagate_results(&child_store, &mut parent, &save_to).expect("propagation should succeed");

    assert_eq!(
        parent.get("session_token"),
        Some(&VarValue::string("jwt-abc-123")),
    );
    assert_eq!(parent.get("active_user_id"), Some(&VarValue::string("42")),);
    // Original parent vars are untouched
    assert_eq!(
        parent.get("user_email"),
        Some(&VarValue::string("alice@example.com")),
    );
}

// ---------------------------------------------------------------------------
// 3. SubFlowConfig extracted from block with run_flow field
// ---------------------------------------------------------------------------
#[test]
fn subflow_config_extracted_from_run_flow_block() {
    let vars = HashMap::from([("email".to_string(), "${user.email}".to_string())]);
    let save_to = HashMap::from([("token".to_string(), "session_token".to_string())]);

    let block = make_block_with_run_flow("auth/login.test.toml", vars.clone(), save_to.clone());

    let config = extract_subflow_config(&block).expect("should extract config");

    assert_eq!(config.flow_path, "auth/login.test.toml");
    assert_eq!(config.vars, vars);
    assert_eq!(config.save_to, save_to);
}

#[test]
fn subflow_config_returns_none_for_regular_block() {
    let block = make_regular_block();
    assert!(extract_subflow_config(&block).is_none());
}

// ===========================================================================
// Data-driven tests
// ===========================================================================

// ---------------------------------------------------------------------------
// 4. expand_data_rows creates one run per data row
// ---------------------------------------------------------------------------
#[test]
fn data_driven_expand_rows_creates_one_run_per_row() {
    let data = vec![
        HashMap::from([("payment".to_string(), "visa".to_string())]),
        HashMap::from([("payment".to_string(), "paypal".to_string())]),
        HashMap::from([("payment".to_string(), "crypto".to_string())]),
    ];

    let runs = expand_data_rows(&data);

    assert_eq!(runs.len(), 3);
    assert_eq!(
        runs[0].vars.get("payment").map(|s| s.as_str()),
        Some("visa")
    );
    assert_eq!(
        runs[1].vars.get("payment").map(|s| s.as_str()),
        Some("paypal")
    );
    assert_eq!(
        runs[2].vars.get("payment").map(|s| s.as_str()),
        Some("crypto")
    );
    // Indices match positions
    for (i, run) in runs.iter().enumerate() {
        assert_eq!(run.index, i);
    }
}

// ---------------------------------------------------------------------------
// 5. apply_data_vars merges row variables into store
// ---------------------------------------------------------------------------
#[test]
fn data_driven_apply_vars_merges_into_store() {
    let mut store = VariableStore::new();
    let data_vars = HashMap::from([
        ("payment".to_string(), "credit_card".to_string()),
        ("expected_total".to_string(), "$29.99".to_string()),
    ]);

    apply_data_vars(&mut store, &data_vars);

    let payment = store.resolve("payment").expect("payment should resolve");
    assert_eq!(payment, &VarValue::String("credit_card".to_string()));

    let total = store
        .resolve("expected_total")
        .expect("expected_total should resolve");
    assert_eq!(total, &VarValue::String("$29.99".to_string()));
}

// ---------------------------------------------------------------------------
// 6. get_runs returns single default run when no data rows
// ---------------------------------------------------------------------------
#[test]
fn data_driven_get_runs_returns_default_when_empty() {
    let runs = get_runs(&[]);

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].label, "default");
    assert!(runs[0].vars.is_empty());
    assert_eq!(runs[0].index, 0);
}

// ---------------------------------------------------------------------------
// 7. Data vars override flow vars in store
// ---------------------------------------------------------------------------
#[test]
fn data_driven_vars_override_flow_vars() {
    let mut store = VariableStore::new();

    // Pre-populate a Flow-level variable
    store.set_in_scope(
        ScopeLevel::Flow,
        "payment",
        VarValue::String("cash".to_string()),
    );

    // Override it via data row (also at Flow level)
    let data_vars = HashMap::from([("payment".to_string(), "paypal".to_string())]);
    apply_data_vars(&mut store, &data_vars);

    let val = store.resolve("payment").expect("should resolve");
    assert_eq!(val, &VarValue::String("paypal".to_string()));
}

// ===========================================================================
// Fixture loading tests
// ===========================================================================

// ---------------------------------------------------------------------------
// 8. load_fixture_into_store loads vars under namespace
// ---------------------------------------------------------------------------
#[test]
fn fixture_loads_vars_under_namespace() {
    let tmp = TempDir::new().expect("temp dir");
    let dir = tmp.path();
    write_fixture(
        dir,
        "new_user",
        "[vars]\nemail = \"alice@example.com\"\npassword = \"s3cret\"\n",
    );

    let mut store = VariableStore::new();
    let mut rng = seeded_rng();

    load_fixture_into_store("new_user", "user", dir, dir, &mut store, &mut rng)
        .expect("should load fixture");

    let user = store.resolve("user").expect("user should exist in store");
    let obj = user.as_object().expect("user should be an object");
    assert_eq!(
        obj.get("email"),
        Some(&VarValue::string("alice@example.com")),
    );
    assert_eq!(obj.get("password"), Some(&VarValue::string("s3cret")),);
}

// ---------------------------------------------------------------------------
// 9. Fixture with fake:* generators evaluates them
// ---------------------------------------------------------------------------
#[test]
fn fixture_with_generators_evaluates_fake_values() {
    let tmp = TempDir::new().expect("temp dir");
    let dir = tmp.path();
    write_fixture(
        dir,
        "gen_user",
        "[vars]\nemail = \"${fake:email}\"\nfirst = \"${fake:first_name}\"\n",
    );

    let mut store = VariableStore::new();
    let mut rng = seeded_rng();

    load_fixture_into_store("gen_user", "user", dir, dir, &mut store, &mut rng)
        .expect("should load fixture");

    let user = store.resolve("user").expect("user should exist");
    let obj = user.as_object().expect("user should be an object");

    let email = obj
        .get("email")
        .and_then(|v| v.as_str())
        .expect("email should be a string");
    assert!(email.contains('@'), "email SHALL contain @, got: {email}");

    let first = obj
        .get("first")
        .and_then(|v| v.as_str())
        .expect("first should be a string");
    assert!(!first.is_empty(), "first name SHALL NOT be empty");
}

// ---------------------------------------------------------------------------
// 10. Fixture vars accessible via dot-path (namespace.var)
// ---------------------------------------------------------------------------
#[test]
fn fixture_vars_accessible_via_dot_path() {
    let tmp = TempDir::new().expect("temp dir");
    let dir = tmp.path();
    write_fixture(
        dir,
        "creds",
        "[vars]\nemail = \"bob@test.com\"\npassword = \"hunter2\"\n",
    );

    let mut store = VariableStore::new();
    let mut rng = seeded_rng();

    load_fixture_into_store("creds", "user", dir, dir, &mut store, &mut rng)
        .expect("should load fixture");

    let user_val = store.resolve("user").expect("user should exist");

    let email = user_val
        .get_path("email")
        .and_then(|v| v.as_str())
        .expect("user.email should resolve");
    assert_eq!(email, "bob@test.com");

    let password = user_val
        .get_path("password")
        .and_then(|v| v.as_str())
        .expect("user.password should resolve");
    assert_eq!(password, "hunter2");
}

// ===========================================================================
// Cross-module integration: combine sub-flow + data-driven + fixture
// ===========================================================================

// ---------------------------------------------------------------------------
// 11. End-to-end: fixture vars available to child sub-flow via prepare_child_vars
// ---------------------------------------------------------------------------
#[test]
fn fixture_vars_available_to_child_subflow() {
    let tmp = TempDir::new().expect("temp dir");
    let dir = tmp.path();
    write_fixture(
        dir,
        "login_creds",
        "[vars]\nemail = \"admin@corp.com\"\npassword = \"admin_pass\"\n",
    );

    let mut parent = VariableStore::new();
    let mut rng = seeded_rng();

    // Load fixture into parent store
    load_fixture_into_store("login_creds", "creds", dir, dir, &mut parent, &mut rng)
        .expect("should load fixture");

    // Prepare child store with an override
    let overrides = HashMap::from([("env".to_string(), "test".to_string())]);
    let child = prepare_child_vars(&parent, &overrides);

    // Child should have inherited fixture vars
    let creds = child.resolve("creds").expect("creds should be in child");
    let email = creds
        .get_path("email")
        .and_then(|v| v.as_str())
        .expect("creds.email should exist");
    assert_eq!(email, "admin@corp.com");

    // Child should also have the override
    assert_eq!(child.get("env"), Some(&VarValue::string("test")),);
}

// ---------------------------------------------------------------------------
// 12. End-to-end: data-driven rows feed into sub-flow variable overrides
// ---------------------------------------------------------------------------
#[test]
fn data_rows_feed_into_subflow_overrides() {
    let data = vec![
        HashMap::from([
            ("user".to_string(), "alice".to_string()),
            ("role".to_string(), "admin".to_string()),
        ]),
        HashMap::from([
            ("user".to_string(), "bob".to_string()),
            ("role".to_string(), "viewer".to_string()),
        ]),
    ];

    let runs = expand_data_rows(&data);
    assert_eq!(runs.len(), 2);

    let parent = make_parent_store();

    // For each data run, prepare a child store using the run's vars as overrides
    for run in &runs {
        let child = prepare_child_vars(&parent, &run.vars);

        // Child has the data-driven override
        let user_val = child.get("user").expect("user should be in child");
        assert_eq!(
            user_val.as_str(),
            Some(run.vars.get("user").expect("user in vars").as_str()),
        );

        // Child inherits parent variables
        assert_eq!(
            child.get("base_url"),
            Some(&VarValue::string("https://example.com")),
        );
    }
}

// ---------------------------------------------------------------------------
// 13. Data vars at Flow level do not override CLI-level vars
// ---------------------------------------------------------------------------
#[test]
fn data_vars_do_not_override_cli_vars() {
    let mut store = VariableStore::new();

    // CLI-level variable has highest priority
    let mut cli_scope = Scope::new(ScopeLevel::Cli);
    cli_scope.set("env", VarValue::String("staging".to_string()));
    store.push_scope(cli_scope);

    // Data row tries to set the same variable at Flow level
    let data_vars = HashMap::from([("env".to_string(), "production".to_string())]);
    apply_data_vars(&mut store, &data_vars);

    // CLI should still win (priority: Cli > Flow)
    let val = store.resolve("env").expect("should resolve");
    assert_eq!(val, &VarValue::String("staging".to_string()));
}

// ---------------------------------------------------------------------------
// 14. propagate_results errors on missing child variable
// ---------------------------------------------------------------------------
#[test]
fn subflow_propagate_results_errors_on_missing_var() {
    let child_store = VariableStore::new();
    let mut parent = make_parent_store();
    let save_to = HashMap::from([("nonexistent".to_string(), "parent_var".to_string())]);

    let result = propagate_results(&child_store, &mut parent, &save_to);

    assert!(result.is_err());
    let err_msg = result.expect_err("should be error").to_string();
    assert!(
        err_msg.contains("nonexistent"),
        "error should mention missing variable: {err_msg}",
    );
}

// ---------------------------------------------------------------------------
// 15. Child overrides do not mutate the parent store
// ---------------------------------------------------------------------------
#[test]
fn child_overrides_do_not_mutate_parent() {
    let parent = make_parent_store();
    let overrides = HashMap::from([("user_email".to_string(), "override@example.com".to_string())]);

    let _child = prepare_child_vars(&parent, &overrides);

    // Parent still has original value
    assert_eq!(
        parent.get("user_email"),
        Some(&VarValue::string("alice@example.com")),
    );
}
