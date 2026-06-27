use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use golem_parser::fixture::{parse_fixture, resolve_fixture_path};
use golem_vars::evaluate::evaluate_generators;
use golem_vars::{ScopeLevel, VarValue, VariableStore};
use rand::Rng;

/// Load a fixture file and merge its variables into the store under the given namespace.
///
/// Steps:
/// 1. Resolve the fixture file path using the `__fixtures__/` directory convention
/// 2. Read and parse the TOML fixture file
/// 3. Evaluate any `fake:*` generators in the fixture vars
/// 4. Scope all variables under the given namespace as a `VarValue::Object`
/// 5. Store the object in the `VariableStore` at `ScopeLevel::Fixture`
pub fn load_fixture_into_store(
    fixture_name: &str,
    namespace: &str,
    flow_dir: &Path,
    project_root: &Path,
    store: &mut VariableStore,
    rng: &mut impl Rng,
) -> Result<()> {
    // 1. Resolve path
    let path = resolve_fixture_path(fixture_name, flow_dir, project_root)?;

    // 2. Read and parse
    let content = std::fs::read_to_string(&path)?;
    let fixture = parse_fixture(&content)?;

    // 3. Evaluate generators — convert HashMap to Vec of pairs for evaluate_generators
    let vars: Vec<(String, String)> = fixture.vars.into_iter().collect();
    let evaluated = evaluate_generators(&vars, rng)?;

    // 4. Build object: wrap all evaluated VarValues under the namespace
    let mut object = HashMap::new();
    for (key, value) in evaluated {
        object.insert(key, value);
    }

    // 5. Store under namespace at Fixture scope level
    store.set_in_scope(ScopeLevel::Fixture, namespace, VarValue::Object(object));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_vars::Scope;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::fs;
    use tempfile::TempDir;

    fn seeded_rng() -> ChaCha8Rng {
        ChaCha8Rng::seed_from_u64(42)
    }

    /// Helper: create a __fixtures__ directory and write a fixture file
    fn write_fixture(base_dir: &Path, name: &str, content: &str) {
        let fixture_dir = base_dir.join("__fixtures__");
        let file_path = fixture_dir.join(format!("{name}.toml"));
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create fixture directory");
        }
        fs::write(&file_path, content).expect("Failed to write fixture file");
    }

    // ---------------------------------------------------------------
    // 1. Load fixture with static vars — vars available under namespace
    // ---------------------------------------------------------------
    #[test]
    fn load_fixture_static_vars_under_namespace() {
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
            Some(&VarValue::string("alice@example.com"))
        );
        assert_eq!(obj.get("password"), Some(&VarValue::string("s3cret")));
    }

    // ---------------------------------------------------------------
    // 2. Load fixture with fake:* generators — values generated
    // ---------------------------------------------------------------
    #[test]
    fn load_fixture_with_generators() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();
        write_fixture(
            dir,
            "gen_user",
            "[vars]\nemail = \"${fake:email}\"\nfirst = \"${fake:city}\"\n",
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

    // ---------------------------------------------------------------
    // 3. Fixture vars scoped under namespace (user.email, user.password)
    // ---------------------------------------------------------------
    #[test]
    fn fixture_vars_accessible_via_namespace() {
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

        // Access via the VarValue dot-path navigation
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

    // ---------------------------------------------------------------
    // 4. Fixture not found returns error
    // ---------------------------------------------------------------
    #[test]
    fn fixture_not_found_returns_error() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();

        let mut store = VariableStore::new();
        let mut rng = seeded_rng();

        let result = load_fixture_into_store("nonexistent", "user", dir, dir, &mut store, &mut rng);
        assert!(result.is_err(), "SHALL error when fixture not found");
        let err_msg = format!("{}", result.expect_err("should be an error"));
        assert!(
            err_msg.contains("not found"),
            "error should mention 'not found', got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 5. Loading same fixture twice with different names creates
    //    independent copies
    // ---------------------------------------------------------------
    #[test]
    fn loading_same_fixture_twice_creates_independent_copies() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();
        write_fixture(
            dir,
            "person",
            "[vars]\nname = \"Charlie\"\nrole = \"admin\"\n",
        );

        let mut store = VariableStore::new();
        let mut rng = seeded_rng();

        load_fixture_into_store("person", "admin", dir, dir, &mut store, &mut rng)
            .expect("first load");
        load_fixture_into_store("person", "viewer", dir, dir, &mut store, &mut rng)
            .expect("second load");

        let admin = store.resolve("admin").expect("admin should exist");
        let viewer = store.resolve("viewer").expect("viewer should exist");

        let admin_name = admin
            .get_path("name")
            .and_then(|v| v.as_str())
            .expect("admin.name");
        let viewer_name = viewer
            .get_path("name")
            .and_then(|v| v.as_str())
            .expect("viewer.name");

        assert_eq!(admin_name, "Charlie");
        assert_eq!(viewer_name, "Charlie");
        // They should be equal values but stored independently
        assert_eq!(admin, viewer);
    }

    // ---------------------------------------------------------------
    // 6. Fixture vars accessible via dot-path in store
    // ---------------------------------------------------------------
    #[test]
    fn fixture_vars_accessible_via_dot_path() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();
        write_fixture(
            dir,
            "config",
            "[vars]\nhost = \"localhost\"\nport = \"8080\"\n",
        );

        let mut store = VariableStore::new();
        // Also push a flow scope so we can verify fixture scope works alongside others
        let flow_scope = Scope::new(ScopeLevel::Flow);
        store.push_scope(flow_scope);

        let mut rng = seeded_rng();

        load_fixture_into_store("config", "srv", dir, dir, &mut store, &mut rng)
            .expect("should load fixture");

        let srv = store.resolve("srv").expect("srv should exist");
        assert_eq!(srv.get_path("host"), Some(&VarValue::string("localhost")));
        assert_eq!(srv.get_path("port"), Some(&VarValue::string("8080")));
    }

    // ---------------------------------------------------------------
    // 7. Empty fixture produces empty object
    // ---------------------------------------------------------------
    #[test]
    fn empty_fixture_produces_empty_object() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();
        write_fixture(dir, "empty", "[vars]\n");

        let mut store = VariableStore::new();
        let mut rng = seeded_rng();

        load_fixture_into_store("empty", "data", dir, dir, &mut store, &mut rng)
            .expect("should load empty fixture");

        let data = store.resolve("data").expect("data should exist");
        let obj = data.as_object().expect("data should be an object");
        assert!(obj.is_empty(), "empty fixture SHALL produce empty object");
    }

    // ---------------------------------------------------------------
    // 8. Invalid fixture TOML returns error
    // ---------------------------------------------------------------
    #[test]
    fn invalid_fixture_toml_returns_error() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();
        write_fixture(dir, "broken", "this is not [[[valid toml");

        let mut store = VariableStore::new();
        let mut rng = seeded_rng();

        let result = load_fixture_into_store("broken", "data", dir, dir, &mut store, &mut rng);
        assert!(result.is_err(), "SHALL error on invalid TOML");
    }

    // ---------------------------------------------------------------
    // 9. Same seed produces same generated value (deterministic threading
    //    of the caller-supplied rng through evaluate_generators)
    // ---------------------------------------------------------------
    #[test]
    fn same_seed_produces_same_generated_value() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();
        write_fixture(dir, "gen", "[vars]\nemail = \"${fake:email}\"\n");

        let load_email = || {
            let mut store = VariableStore::new();
            let mut rng = seeded_rng();
            load_fixture_into_store("gen", "user", dir, dir, &mut store, &mut rng)
                .expect("should load fixture");
            let user = store.resolve("user").expect("user should exist");
            user.get_path("email")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .expect("user.email should resolve to a string")
        };

        let first = load_email();
        let second = load_email();
        assert_eq!(
            first, second,
            "same seed SHALL produce same generated value"
        );
    }

    // ---------------------------------------------------------------
    // 10. Loading a second fixture into the SAME namespace replaces the
    //     prior object at that namespace (no merge, last write wins)
    // ---------------------------------------------------------------
    #[test]
    fn same_namespace_second_load_replaces_first() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();
        write_fixture(
            dir,
            "first",
            "[vars]\nname = \"Alice\"\nonly_first = \"x\"\n",
        );
        write_fixture(dir, "second", "[vars]\nname = \"Bob\"\n");

        let mut store = VariableStore::new();
        let mut rng = seeded_rng();

        load_fixture_into_store("first", "user", dir, dir, &mut store, &mut rng)
            .expect("first load");
        load_fixture_into_store("second", "user", dir, dir, &mut store, &mut rng)
            .expect("second load");

        let user = store.resolve("user").expect("user should exist");
        let obj = user.as_object().expect("user should be an object");
        assert_eq!(
            obj.get("name"),
            Some(&VarValue::string("Bob")),
            "second load SHALL overwrite name"
        );
        assert!(
            obj.get("only_first").is_none(),
            "keys from the first fixture SHALL NOT survive a same-namespace replace"
        );
    }

    // ---------------------------------------------------------------
    // 11. Fixture resolved by walking up from flow_dir to project_root
    //     when it lives in an ancestor's __fixtures__ directory
    // ---------------------------------------------------------------
    #[test]
    fn fixture_resolved_from_ancestor_directory() {
        let tmp = TempDir::new().expect("temp dir");
        let project_root = tmp.path();
        let flow_dir = project_root.join("flows").join("auth");
        fs::create_dir_all(&flow_dir).expect("create flow dir");

        // Fixture lives at project_root, flow runs two levels deeper.
        write_fixture(project_root, "shared", "[vars]\nkey = \"value\"\n");

        let mut store = VariableStore::new();
        let mut rng = seeded_rng();

        load_fixture_into_store(
            "shared",
            "cfg",
            &flow_dir,
            project_root,
            &mut store,
            &mut rng,
        )
        .expect("should resolve fixture from ancestor");

        let cfg = store.resolve("cfg").expect("cfg should exist");
        assert_eq!(cfg.get_path("key"), Some(&VarValue::string("value")));
    }

    // ---------------------------------------------------------------
    // 12. Subfolder fixture name loads into store under namespace
    // ---------------------------------------------------------------
    #[test]
    fn subfolder_fixture_name_loads() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();
        write_fixture(dir, "payments/card", "[vars]\npan = \"4242424242424242\"\n");

        let mut store = VariableStore::new();
        let mut rng = seeded_rng();

        load_fixture_into_store("payments/card", "card", dir, dir, &mut store, &mut rng)
            .expect("should load subfolder fixture");

        let card = store.resolve("card").expect("card should exist");
        assert_eq!(
            card.get_path("pan"),
            Some(&VarValue::string("4242424242424242"))
        );
    }

    // ---------------------------------------------------------------
    // 13. Non-string var value propagates parse error (vars is
    //     HashMap<String, String>; integer fails deserialization)
    // ---------------------------------------------------------------
    #[test]
    fn non_string_var_value_propagates_error() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();
        write_fixture(dir, "typed", "[vars]\ncount = 5\n");

        let mut store = VariableStore::new();
        let mut rng = seeded_rng();

        let result = load_fixture_into_store("typed", "data", dir, dir, &mut store, &mut rng);
        assert!(
            result.is_err(),
            "non-string var value SHALL propagate as a load error"
        );
        assert!(
            store.resolve("data").is_err(),
            "no namespace SHALL be stored when parsing fails"
        );
    }

    // ---------------------------------------------------------------
    // 14. Unknown generator name propagates as a load error
    // ---------------------------------------------------------------
    #[test]
    fn invalid_generator_propagates_error() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();
        // `fake:` parses fine (GeneratorDef with an empty name); the error originates in
        // generate_simple, whose `_` arm rejects an empty/unknown name as "unknown generator".
        write_fixture(dir, "badgen", "[vars]\nx = \"${fake:}\"\n");

        let mut store = VariableStore::new();
        let mut rng = seeded_rng();

        let result = load_fixture_into_store("badgen", "data", dir, dir, &mut store, &mut rng);
        assert!(
            result.is_err(),
            "an unknown generator name SHALL propagate as a load error"
        );
        // Lock in WHICH branch fired: the unknown-generator arm, not a parse failure.
        let err_msg = format!("{}", result.expect_err("should be an error"));
        assert!(
            err_msg.contains("unknown generator"),
            "error SHALL come from the unknown-generator branch, got: {err_msg}"
        );
        // Nothing SHALL be stored when generation fails.
        assert!(
            store.resolve("data").is_err(),
            "no namespace SHALL be stored when generation fails"
        );
    }

    // ---------------------------------------------------------------
    // 15. Distinct generated keys advance the rng (independent values
    //     within a single load call)
    // ---------------------------------------------------------------
    #[test]
    fn distinct_generated_keys_advance_rng() {
        let tmp = TempDir::new().expect("temp dir");
        let dir = tmp.path();
        write_fixture(
            dir,
            "multi",
            "[vars]\na = \"${fake:email}\"\nb = \"${fake:email}\"\n",
        );

        let mut store = VariableStore::new();
        let mut rng = seeded_rng();

        load_fixture_into_store("multi", "user", dir, dir, &mut store, &mut rng)
            .expect("should load fixture");

        let user = store.resolve("user").expect("user should exist");
        let a = user
            .get_path("a")
            .and_then(|v| v.as_str())
            .expect("a should be a string");
        let b = user
            .get_path("b")
            .and_then(|v| v.as_str())
            .expect("b should be a string");
        assert_ne!(
            a, b,
            "two generators in one fixture SHALL draw from the advancing rng, not repeat"
        );
    }
}
