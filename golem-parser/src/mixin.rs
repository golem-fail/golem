use crate::Step;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A parsed mixin file — contains only steps, no blocks or flow metadata
#[derive(Debug, Clone)]
pub struct Mixin {
    pub steps: Vec<Step>,
}

/// Internal representation for deserializing a mixin TOML file
#[derive(serde::Deserialize)]
struct MixinFile {
    #[serde(default)]
    step: Vec<Step>,
    // These fields exist only to detect and reject invalid mixin content
    #[serde(default)]
    block: Option<toml::Value>,
    #[serde(default)]
    flow: Option<toml::Value>,
    #[serde(default)]
    vars: Option<toml::Value>,
}

/// Parse a mixin TOML file. Mixin files contain only [[step]] entries.
pub fn parse_mixin(toml_str: &str) -> anyhow::Result<Mixin> {
    let mixin_file: MixinFile = toml::from_str(toml_str)?;

    if mixin_file.block.is_some() {
        anyhow::bail!("Mixin files cannot contain [[block]] sections");
    }
    if mixin_file.flow.is_some() {
        anyhow::bail!("Mixin files cannot contain [flow] sections");
    }
    if mixin_file.vars.is_some() {
        anyhow::bail!("Mixin files cannot contain [vars] sections");
    }

    Ok(Mixin {
        steps: mixin_file.step,
    })
}

/// Resolve a mixin name to a file path using __mixins__/ directory convention.
/// Searches from flow_dir up to project_root, closest wins.
pub fn resolve_mixin_path(
    mixin_name: &str,
    flow_dir: &Path,
    project_root: &Path,
) -> anyhow::Result<PathBuf> {
    // Reject path traversal
    if mixin_name.contains("..") {
        anyhow::bail!("Mixin name cannot contain path traversal: {mixin_name}");
    }

    // Append .toml if not already present
    let file_name = if mixin_name.ends_with(".toml") {
        mixin_name.to_string()
    } else {
        format!("{mixin_name}.toml")
    };

    // Walk from flow_dir up to project_root (inclusive)
    let project_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut current = flow_dir
        .canonicalize()
        .unwrap_or_else(|_| flow_dir.to_path_buf());

    loop {
        let candidate = current.join("__mixins__").join(&file_name);
        if candidate.is_file() {
            return Ok(candidate);
        }

        // Stop if we've reached or passed the project root
        if current == project_root {
            break;
        }

        // Walk up to parent
        match current.parent() {
            Some(parent) => {
                // Don't go above the project root
                if current == parent.to_path_buf() {
                    break;
                }
                current = parent.to_path_buf();
            }
            None => break,
        }
    }

    anyhow::bail!(
        "Mixin '{}' not found in __mixins__/ directories from {} to {}",
        mixin_name,
        flow_dir.display(),
        project_root.display()
    )
}

/// Expand all load_mixin steps in a block's step list.
/// Each load_mixin is replaced by the mixin's steps with variable remapping.
/// Returns error if a mixin contains load_mixin (no nesting).
pub fn expand_mixins(
    steps: &[Step],
    flow_dir: &Path,
    project_root: &Path,
) -> anyhow::Result<Vec<Step>> {
    let mut expanded = Vec::new();

    for step in steps {
        if step.action == "load_mixin" {
            let mixin_name = step
                .params
                .get("mixin")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("load_mixin step missing 'mixin' parameter"))?;

            let vars = extract_vars_from_step(step);

            let mixin_path = resolve_mixin_path(mixin_name, flow_dir, project_root)?;
            let mixin_content = std::fs::read_to_string(&mixin_path)
                .map_err(|e| anyhow::anyhow!("Failed to read mixin file {}: {e}", mixin_path.display()))?;
            let mixin = parse_mixin(&mixin_content)?;

            // Check for nested load_mixin
            for mixin_step in &mixin.steps {
                if mixin_step.action == "load_mixin" {
                    anyhow::bail!(
                        "Nested load_mixin is not allowed: mixin '{}' contains a load_mixin step",
                        mixin_name
                    );
                }
            }

            // Apply variable remapping and add steps
            for mixin_step in &mixin.steps {
                expanded.push(remap_step_vars(mixin_step, &vars));
            }
        } else {
            expanded.push(step.clone());
        }
    }

    Ok(expanded)
}

/// Extract variable mappings from a load_mixin step's params
fn extract_vars_from_step(step: &Step) -> HashMap<String, String> {
    let mut vars = HashMap::new();

    if let Some(toml::Value::Table(table)) = step.params.get("vars") {
        for (key, value) in table {
            if let Some(s) = value.as_str() {
                vars.insert(key.clone(), s.to_string());
            }
        }
    }

    vars
}

/// Apply variable remapping to a step: replace ${key} with the mapped value
/// in all string fields.
fn remap_step_vars(step: &Step, vars: &HashMap<String, String>) -> Step {
    if vars.is_empty() {
        return step.clone();
    }

    Step {
        action: remap_string(&step.action, vars),
        text: step.text.as_ref().map(|s| remap_string(s, vars)),
        accessibility_id: step.accessibility_id.as_ref().map(|s| remap_string(s, vars)),
        element_type: step.element_type.as_ref().map(|s| remap_string(s, vars)),
        index: step.index,
        enabled: step.enabled,
        checked: step.checked,
        clickable: step.clickable,
        below: step.below.as_ref().map(|s| remap_string(s, vars)),
        above: step.above.as_ref().map(|s| remap_string(s, vars)),
        right_of: step.right_of.as_ref().map(|s| remap_string(s, vars)),
        left_of: step.left_of.as_ref().map(|s| remap_string(s, vars)),
        child_of: step.child_of.as_ref().map(|s| remap_string(s, vars)),
        placeholder: step.placeholder.as_ref().map(|s| remap_string(s, vars)),
        on_fail: step.on_fail.clone(),
        save_to: step.save_to.clone(),
        timeout: step.timeout,
        retry: step.retry,
        retry_delay: step.retry_delay,
        app: step.app.as_ref().map(|s| remap_string(s, vars)),
        auto_scroll: step.auto_scroll,
        params: remap_params(&step.params, vars),
    }
}

/// Replace all ${key} occurrences in a string with the mapped value
fn remap_string(s: &str, vars: &HashMap<String, String>) -> String {
    let mut result = s.to_string();
    for (key, value) in vars {
        let pattern = format!("${{{key}}}");
        result = result.replace(&pattern, value);
    }
    result
}

/// Remap variables in a params HashMap
fn remap_params(
    params: &HashMap<String, toml::Value>,
    vars: &HashMap<String, String>,
) -> HashMap<String, toml::Value> {
    params
        .iter()
        .map(|(k, v)| (k.clone(), remap_toml_value(v, vars)))
        .collect()
}

/// Recursively remap variables in a toml::Value
fn remap_toml_value(value: &toml::Value, vars: &HashMap<String, String>) -> toml::Value {
    match value {
        toml::Value::String(s) => toml::Value::String(remap_string(s, vars)),
        toml::Value::Array(arr) => {
            toml::Value::Array(arr.iter().map(|v| remap_toml_value(v, vars)).collect())
        }
        toml::Value::Table(table) => {
            let mut new_table = toml::map::Map::new();
            for (k, v) in table {
                new_table.insert(k.clone(), remap_toml_value(v, vars));
            }
            toml::Value::Table(new_table)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: create a __mixins__ directory and write a mixin file
    fn write_mixin(base_dir: &Path, name: &str, content: &str) {
        let mixin_dir = base_dir.join("__mixins__");
        // name may contain subdirectories
        let file_path = mixin_dir.join(format!("{name}.toml"));
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create mixin directory");
        }
        fs::write(&file_path, content).expect("Failed to write mixin file");
    }

    /// Helper: create a load_mixin step
    fn load_mixin_step(mixin_name: &str, vars: Option<HashMap<String, String>>) -> Step {
        let mut params = HashMap::new();
        params.insert(
            "mixin".to_string(),
            toml::Value::String(mixin_name.to_string()),
        );
        if let Some(v) = vars {
            let mut table = toml::map::Map::new();
            for (key, value) in v {
                table.insert(key, toml::Value::String(value));
            }
            params.insert("vars".to_string(), toml::Value::Table(table));
        }
        Step {
            action: "load_mixin".to_string(),
            text: None,
            accessibility_id: None,
            element_type: None,
            index: None,
            enabled: None,
            checked: None,
            clickable: None,
            below: None,
            above: None,
            right_of: None,
            left_of: None,
            child_of: None,
            placeholder: None,
            on_fail: None,
            save_to: None,
            timeout: None,
            retry: None,
            retry_delay: None,
            app: None,
            auto_scroll: None,
            params,
        }
    }

    /// Helper: create a simple step
    fn simple_step(action: &str) -> Step {
        Step {
            action: action.to_string(),
            text: None,
            accessibility_id: None,
            element_type: None,
            index: None,
            enabled: None,
            checked: None,
            clickable: None,
            below: None,
            above: None,
            right_of: None,
            left_of: None,
            child_of: None,
            placeholder: None,
            on_fail: None,
            save_to: None,
            timeout: None,
            retry: None,
            retry_delay: None,
            app: None,
            auto_scroll: None,
            params: HashMap::new(),
        }
    }

    // ---------------------------------------------------------------
    // 1. Basic expansion — steps replace load_mixin
    // ---------------------------------------------------------------
    #[test]
    fn basic_expansion_replaces_load_mixin() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_mixin(
            flow_dir,
            "login",
            r#"
[[step]]
action = "tap"
accessibility_id = "email-input"

[[step]]
action = "type"
text = "hello"
"#,
        );

        let steps = vec![load_mixin_step("login", None)];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 2);
        assert_eq!(expanded[0].action, "tap");
        assert_eq!(expanded[0].accessibility_id.as_deref(), Some("email-input"));
        assert_eq!(expanded[1].action, "type");
        assert_eq!(expanded[1].text.as_deref(), Some("hello"));
    }

    // ---------------------------------------------------------------
    // 2. Variable mapping — vars passed to mixin remapped
    // ---------------------------------------------------------------
    #[test]
    fn variable_mapping_single_var() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_mixin(
            flow_dir,
            "login",
            r#"
[[step]]
action = "type"
text = "${email}"
"#,
        );

        let mut vars = HashMap::new();
        vars.insert("email".to_string(), "${user.email}".to_string());

        let steps = vec![load_mixin_step("login", Some(vars))];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].text.as_deref(), Some("${user.email}"));
    }

    // ---------------------------------------------------------------
    // 3. Variable mapping — multiple vars
    // ---------------------------------------------------------------
    #[test]
    fn variable_mapping_multiple_vars() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_mixin(
            flow_dir,
            "login",
            r#"
[[step]]
action = "type"
accessibility_id = "${email_field}"
text = "${email}"

[[step]]
action = "type"
accessibility_id = "${password_field}"
text = "${password}"
"#,
        );

        let mut vars = HashMap::new();
        vars.insert("email".to_string(), "alice@example.com".to_string());
        vars.insert("password".to_string(), "secret123".to_string());
        vars.insert("email_field".to_string(), "login-email".to_string());
        vars.insert("password_field".to_string(), "login-password".to_string());

        let steps = vec![load_mixin_step("login", Some(vars))];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 2);
        assert_eq!(expanded[0].accessibility_id.as_deref(), Some("login-email"));
        assert_eq!(expanded[0].text.as_deref(), Some("alice@example.com"));
        assert_eq!(expanded[1].accessibility_id.as_deref(), Some("login-password"));
        assert_eq!(expanded[1].text.as_deref(), Some("secret123"));
    }

    // ---------------------------------------------------------------
    // 4. Unmapped variables pass through
    // ---------------------------------------------------------------
    #[test]
    fn unmapped_variables_pass_through() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_mixin(
            flow_dir,
            "greet",
            r#"
[[step]]
action = "type"
text = "${greeting} ${name}"
"#,
        );

        let mut vars = HashMap::new();
        vars.insert("greeting".to_string(), "Hello".to_string());
        // "name" is not mapped — should pass through as ${name}

        let steps = vec![load_mixin_step("greet", Some(vars))];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded[0].text.as_deref(), Some("Hello ${name}"));
    }

    // ---------------------------------------------------------------
    // 5. Mixin with save_to preserved
    // ---------------------------------------------------------------
    #[test]
    fn mixin_with_save_to_preserved() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_mixin(
            flow_dir,
            "read_value",
            r#"
[[step]]
action = "read"
accessibility_id = "price-label"
save_to = "captured_price"
"#,
        );

        let steps = vec![load_mixin_step("read_value", None)];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].save_to.as_deref(), Some("captured_price"));
    }

    // ---------------------------------------------------------------
    // 6. Multiple load_mixin in one block
    // ---------------------------------------------------------------
    #[test]
    fn multiple_load_mixin_in_one_block() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_mixin(
            flow_dir,
            "login",
            r#"
[[step]]
action = "tap"
text = "Login"
"#,
        );

        write_mixin(
            flow_dir,
            "logout",
            r#"
[[step]]
action = "tap"
text = "Logout"
"#,
        );

        let steps = vec![
            simple_step("screenshot"),
            load_mixin_step("login", None),
            simple_step("wait"),
            load_mixin_step("logout", None),
            simple_step("screenshot"),
        ];

        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 5);
        assert_eq!(expanded[0].action, "screenshot");
        assert_eq!(expanded[1].action, "tap");
        assert_eq!(expanded[1].text.as_deref(), Some("Login"));
        assert_eq!(expanded[2].action, "wait");
        assert_eq!(expanded[3].action, "tap");
        assert_eq!(expanded[3].text.as_deref(), Some("Logout"));
        assert_eq!(expanded[4].action, "screenshot");
    }

    // ---------------------------------------------------------------
    // 7. Empty mixin — no steps inserted
    // ---------------------------------------------------------------
    #[test]
    fn empty_mixin_no_steps_inserted() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_mixin(flow_dir, "empty", "");

        let steps = vec![
            simple_step("tap"),
            load_mixin_step("empty", None),
            simple_step("wait"),
        ];

        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 2);
        assert_eq!(expanded[0].action, "tap");
        assert_eq!(expanded[1].action, "wait");
    }

    // ---------------------------------------------------------------
    // 8. Mixin with on_fail preserved
    // ---------------------------------------------------------------
    #[test]
    fn mixin_with_on_fail_preserved() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_mixin(
            flow_dir,
            "fragile",
            r#"
[[step]]
action = "tap"
text = "Maybe"
on_fail = "ignore"
"#,
        );

        let steps = vec![load_mixin_step("fragile", None)];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].on_fail.as_deref(), Some("ignore"));
    }

    // ---------------------------------------------------------------
    // 9. Nested mixin — error (mixin containing load_mixin)
    // ---------------------------------------------------------------
    #[test]
    fn nested_mixin_error() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_mixin(
            flow_dir,
            "outer",
            r#"
[[step]]
action = "load_mixin"
mixin = "inner"
"#,
        );

        let steps = vec![load_mixin_step("outer", None)];
        let result = expand_mixins(&steps, flow_dir, project_root);

        assert!(result.is_err(), "SHALL reject nested load_mixin in mixin files");
        let err_msg = format!("{}", result.expect_err("SHALL be an error"));
        assert!(
            err_msg.contains("Nested load_mixin"),
            "SHALL mention nested load_mixin in error, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 10. Mixin contains block — error
    // ---------------------------------------------------------------
    #[test]
    fn mixin_contains_block_error() {
        let toml_str = r#"
[[block]]
name = "forbidden"

[[step]]
action = "tap"
"#;
        let result = parse_mixin(toml_str);
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be an error"));
        assert!(
            err_msg.contains("block"),
            "Error should mention block, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 11. Mixin contains flow section — error
    // ---------------------------------------------------------------
    #[test]
    fn mixin_contains_flow_error() {
        let toml_str = r#"
[flow]
name = "forbidden"

[[step]]
action = "tap"
"#;
        let result = parse_mixin(toml_str);
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be an error"));
        assert!(
            err_msg.contains("flow"),
            "Error should mention flow, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 12. Mixin contains vars section — error
    // ---------------------------------------------------------------
    #[test]
    fn mixin_contains_vars_error() {
        let toml_str = r#"
[vars]
foo = "bar"

[[step]]
action = "tap"
"#;
        let result = parse_mixin(toml_str);
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be an error"));
        assert!(
            err_msg.contains("vars"),
            "Error should mention vars, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 13. load_mixin without vars — no mapping
    // ---------------------------------------------------------------
    #[test]
    fn load_mixin_without_vars_no_mapping() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_mixin(
            flow_dir,
            "raw",
            r#"
[[step]]
action = "type"
text = "${some_var}"
"#,
        );

        let steps = vec![load_mixin_step("raw", None)];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 1);
        assert_eq!(
            expanded[0].text.as_deref(),
            Some("${some_var}"),
            "Without vars mapping, variables should pass through unchanged"
        );
    }

    // ---------------------------------------------------------------
    // 14. Expansion preserves step order
    // ---------------------------------------------------------------
    #[test]
    fn expansion_preserves_step_order() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_mixin(
            flow_dir,
            "middle",
            r#"
[[step]]
action = "type"
text = "mixin-step-1"

[[step]]
action = "tap"
text = "mixin-step-2"

[[step]]
action = "swipe"
text = "mixin-step-3"
"#,
        );

        let steps = vec![
            simple_step("screenshot"),
            load_mixin_step("middle", None),
            simple_step("wait"),
        ];

        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        let actions: Vec<&str> = expanded.iter().map(|s| s.action.as_str()).collect();
        assert_eq!(
            actions,
            vec!["screenshot", "type", "tap", "swipe", "wait"],
            "Steps should maintain order: before, mixin steps in order, after"
        );

        // Verify mixin step content preserved
        assert_eq!(expanded[1].text.as_deref(), Some("mixin-step-1"));
        assert_eq!(expanded[2].text.as_deref(), Some("mixin-step-2"));
        assert_eq!(expanded[3].text.as_deref(), Some("mixin-step-3"));
    }
}
