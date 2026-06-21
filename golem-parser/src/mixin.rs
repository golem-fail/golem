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
        on_text: step.on_text.as_ref().map(|s| remap_string(s, vars)),
        on_accessibility_label: step.on_accessibility_label.as_ref().map(|s| remap_string(s, vars)),
        on_index: step.on_index,
        on_enabled: step.on_enabled,
        on_checked: step.on_checked,
        on_clickable: step.on_clickable,
        on_below: step.on_below.as_ref().map(|s| remap_string(s, vars)),
        on_above: step.on_above.as_ref().map(|s| remap_string(s, vars)),
        on_right_of: step.on_right_of.as_ref().map(|s| remap_string(s, vars)),
        on_left_of: step.on_left_of.as_ref().map(|s| remap_string(s, vars)),
        on: step.on.as_ref().map(|g| remap_selector_group(g, vars)),
        input: step.input.as_ref().map(|s| remap_string(s, vars)),
        if_fail: step.if_fail.clone(),
        save_to: step.save_to.clone(),
        timeout: step.timeout,
        retry: step.retry,
        retry_delay: step.retry_delay,
        app: step.app.as_ref().map(|s| remap_string(s, vars)),
        restart: step.restart,
        auto_scroll: step.auto_scroll,
        scroll_timeout: step.scroll_timeout,
        within: step.within.clone(),
        start: step.start.clone(),
        end: step.end.clone(),
        points: step.points.clone(),
        duration: step.duration,
        scale: step.scale,
        rotation: step.rotation,
        velocity: step.velocity,
        fingers: step.fingers.clone(),
        params: remap_params(&step.params, vars),
    }
}

/// Remap variables in an Anchor.
fn remap_anchor(a: &crate::Anchor, vars: &HashMap<String, String>) -> crate::Anchor {
    match a {
        crate::Anchor::Text(s) => crate::Anchor::Text(remap_string(s, vars)),
        crate::Anchor::Selector(g) => crate::Anchor::Selector(Box::new(remap_selector_group(g, vars))),
    }
}

/// Remap variables in a ContainsAnchor (preserves `min_matches`).
fn remap_contains_anchor(a: &crate::ContainsAnchor, vars: &HashMap<String, String>) -> crate::ContainsAnchor {
    match a {
        crate::ContainsAnchor::Text(s) => crate::ContainsAnchor::Text(remap_string(s, vars)),
        crate::ContainsAnchor::Spec(s) => crate::ContainsAnchor::Spec(Box::new(crate::ContainsSpec {
            group: remap_selector_group(&s.group, vars),
            min_matches: s.min_matches,
        })),
    }
}

/// Remap variables in a grouped SelectorGroup.
fn remap_selector_group(g: &crate::SelectorGroup, vars: &HashMap<String, String>) -> crate::SelectorGroup {
    crate::SelectorGroup {
        text: g.text.as_ref().map(|s| remap_string(s, vars)),
        accessibility_label: g.accessibility_label.as_ref().map(|s| remap_string(s, vars)),
        index: g.index,
        enabled: g.enabled,
        checked: g.checked,
        clickable: g.clickable,
        below: g.below.as_ref().map(|a| remap_anchor(a, vars)),
        above: g.above.as_ref().map(|a| remap_anchor(a, vars)),
        right_of: g.right_of.as_ref().map(|a| remap_anchor(a, vars)),
        left_of: g.left_of.as_ref().map(|a| remap_anchor(a, vars)),
        contains: g.contains.as_ref().map(|a| remap_contains_anchor(a, vars)),
        inside: g.inside.as_ref().map(|a| remap_anchor(a, vars)),
        traits: g.traits.clone(),
        x: g.x.clone(),
        y: g.y.clone(),
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
            params,
            ..Default::default()
        }
    }

    /// Helper: create a simple step
    fn simple_step(action: &str) -> Step {
        Step {
            action: action.to_string(),
            ..Default::default()
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
on_accessibility_label = "email-input"

[[step]]
action = "type"
on_text = "hello"
"#,
        );

        let steps = vec![load_mixin_step("login", None)];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 2);
        assert_eq!(expanded[0].action, "tap");
        assert_eq!(expanded[0].on_accessibility_label.as_deref(), Some("email-input"));
        assert_eq!(expanded[1].action, "type");
        assert_eq!(expanded[1].on_text.as_deref(), Some("hello"));
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
on_text = "${email}"
"#,
        );

        let mut vars = HashMap::new();
        vars.insert("email".to_string(), "${user.email}".to_string());

        let steps = vec![load_mixin_step("login", Some(vars))];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].on_text.as_deref(), Some("${user.email}"));
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
on_accessibility_label = "${email_field}"
on_text = "${email}"

[[step]]
action = "type"
on_accessibility_label = "${password_field}"
on_text = "${password}"
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
        assert_eq!(expanded[0].on_accessibility_label.as_deref(), Some("login-email"));
        assert_eq!(expanded[0].on_text.as_deref(), Some("alice@example.com"));
        assert_eq!(expanded[1].on_accessibility_label.as_deref(), Some("login-password"));
        assert_eq!(expanded[1].on_text.as_deref(), Some("secret123"));
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
on_text = "${greeting} ${name}"
"#,
        );

        let mut vars = HashMap::new();
        vars.insert("greeting".to_string(), "Hello".to_string());
        // "name" is not mapped — should pass through as ${name}

        let steps = vec![load_mixin_step("greet", Some(vars))];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded[0].on_text.as_deref(), Some("Hello ${name}"));
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
on_accessibility_label = "price-label"
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
on_text = "Login"
"#,
        );

        write_mixin(
            flow_dir,
            "logout",
            r#"
[[step]]
action = "tap"
on_text = "Logout"
"#,
        );

        let steps = vec![
            simple_step("screenshot"),
            load_mixin_step("login", None),
            simple_step("screenshot"),
            load_mixin_step("logout", None),
            simple_step("screenshot"),
        ];

        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 5);
        assert_eq!(expanded[0].action, "screenshot");
        assert_eq!(expanded[1].action, "tap");
        assert_eq!(expanded[1].on_text.as_deref(), Some("Login"));
        assert_eq!(expanded[2].action, "screenshot");
        assert_eq!(expanded[3].action, "tap");
        assert_eq!(expanded[3].on_text.as_deref(), Some("Logout"));
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
            simple_step("screenshot"),
        ];

        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 2);
        assert_eq!(expanded[0].action, "tap");
        assert_eq!(expanded[1].action, "screenshot");
    }

    // ---------------------------------------------------------------
    // 8. Mixin with if_fail preserved
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
on_text = "Maybe"
if_fail = "ignore"
"#,
        );

        let steps = vec![load_mixin_step("fragile", None)];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].if_fail.as_deref(), Some("ignore"));
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
on_text = "${some_var}"
"#,
        );

        let steps = vec![load_mixin_step("raw", None)];
        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        assert_eq!(expanded.len(), 1);
        assert_eq!(
            expanded[0].on_text.as_deref(),
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
on_text = "mixin-step-1"

[[step]]
action = "tap"
on_text = "mixin-step-2"

[[step]]
action = "swipe"
on_text = "mixin-step-3"
"#,
        );

        let steps = vec![
            simple_step("screenshot"),
            load_mixin_step("middle", None),
            simple_step("screenshot"),
        ];

        let expanded =
            expand_mixins(&steps, flow_dir, project_root).expect("expansion should succeed");

        let actions: Vec<&str> = expanded.iter().map(|s| s.action.as_str()).collect();
        assert_eq!(
            actions,
            vec!["screenshot", "type", "tap", "swipe", "screenshot"],
            "Steps should maintain order: before, mixin steps in order, after"
        );

        // Verify mixin step content preserved
        assert_eq!(expanded[1].on_text.as_deref(), Some("mixin-step-1"));
        assert_eq!(expanded[2].on_text.as_deref(), Some("mixin-step-2"));
        assert_eq!(expanded[3].on_text.as_deref(), Some("mixin-step-3"));
    }

    // ---------------------------------------------------------------
    // 15. resolve_mixin_path rejects path traversal
    // ---------------------------------------------------------------
    #[test]
    fn resolve_mixin_path_rejects_traversal() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let result = resolve_mixin_path("../escape", tmp.path(), tmp.path());

        assert!(result.is_err(), "traversal SHALL be rejected");
        let err_msg = format!("{}", result.expect_err("SHALL be an error"));
        assert!(
            err_msg.contains("path traversal"),
            "error SHALL mention path traversal, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 16. resolve_mixin_path appends .toml when missing
    // ---------------------------------------------------------------
    #[test]
    fn resolve_mixin_path_appends_toml_suffix() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        write_mixin(tmp.path(), "thing", "");

        let resolved = resolve_mixin_path("thing", tmp.path(), tmp.path())
            .expect("SHALL resolve when file exists");
        assert!(
            resolved.ends_with("thing.toml"),
            "resolved path SHALL end with thing.toml, got: {}",
            resolved.display()
        );
    }

    // ---------------------------------------------------------------
    // 17. resolve_mixin_path accepts explicit .toml without doubling
    // ---------------------------------------------------------------
    #[test]
    fn resolve_mixin_path_explicit_toml_not_doubled() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        write_mixin(tmp.path(), "thing", "");

        let resolved = resolve_mixin_path("thing.toml", tmp.path(), tmp.path())
            .expect("SHALL resolve with explicit .toml");
        assert!(
            resolved.ends_with("thing.toml"),
            "SHALL NOT double the suffix, got: {}",
            resolved.display()
        );
        assert!(
            !resolved.to_string_lossy().contains("thing.toml.toml"),
            "SHALL NOT produce thing.toml.toml"
        );
    }

    // ---------------------------------------------------------------
    // 18. resolve_mixin_path not found returns descriptive error
    // ---------------------------------------------------------------
    #[test]
    fn resolve_mixin_path_not_found_error() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let result = resolve_mixin_path("missing", tmp.path(), tmp.path());

        assert!(result.is_err(), "missing mixin SHALL error");
        let err_msg = format!("{}", result.expect_err("SHALL be an error"));
        assert!(
            err_msg.contains("not found") && err_msg.contains("missing"),
            "error SHALL name the mixin and say not found, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 19. resolve_mixin_path walks up from nested flow_dir, closest wins
    // ---------------------------------------------------------------
    #[test]
    fn resolve_mixin_path_closest_wins() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let project_root = tmp.path();
        let nested = project_root.join("a").join("b");
        fs::create_dir_all(&nested).expect("Failed to create nested dirs");

        // Same-named mixin at both project root and nested flow_dir.
        write_mixin(project_root, "shared", "[[step]]\naction = \"root\"\n");
        write_mixin(&nested, "shared", "[[step]]\naction = \"nested\"\n");

        let resolved = resolve_mixin_path("shared", &nested, project_root)
            .expect("SHALL resolve from nested dir");
        let content = fs::read_to_string(&resolved).expect("SHALL read resolved file");
        assert!(
            content.contains("nested"),
            "closest (nested) mixin SHALL win, got: {content}"
        );
    }

    // ---------------------------------------------------------------
    // 20. resolve_mixin_path finds mixin at an ancestor dir
    // ---------------------------------------------------------------
    #[test]
    fn resolve_mixin_path_found_at_ancestor() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let project_root = tmp.path();
        let nested = project_root.join("a").join("b");
        fs::create_dir_all(&nested).expect("Failed to create nested dirs");

        // Mixin only at project root; flow_dir is nested.
        write_mixin(project_root, "only_root", "[[step]]\naction = \"root\"\n");

        let resolved = resolve_mixin_path("only_root", &nested, project_root)
            .expect("SHALL walk up to ancestor");
        let content = fs::read_to_string(&resolved).expect("SHALL read resolved file");
        assert!(content.contains("root"), "SHALL find ancestor mixin, got: {content}");
    }

    // ---------------------------------------------------------------
    // 21. load_mixin missing 'mixin' param errors
    // ---------------------------------------------------------------
    #[test]
    fn load_mixin_missing_mixin_param_errors() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let step = Step {
            action: "load_mixin".to_string(),
            ..Default::default()
        };

        let result = expand_mixins(&[step], tmp.path(), tmp.path());
        assert!(result.is_err(), "missing mixin param SHALL error");
        let err_msg = format!("{}", result.expect_err("SHALL be an error"));
        assert!(
            err_msg.contains("missing 'mixin' parameter"),
            "error SHALL mention missing mixin param, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 22. load_mixin with non-string 'mixin' value errors (not a str)
    // ---------------------------------------------------------------
    #[test]
    fn load_mixin_non_string_mixin_param_errors() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let mut params = HashMap::new();
        params.insert("mixin".to_string(), toml::Value::Integer(42));
        let step = Step {
            action: "load_mixin".to_string(),
            params,
            ..Default::default()
        };

        let result = expand_mixins(&[step], tmp.path(), tmp.path());
        assert!(
            result.is_err(),
            "non-string mixin param SHALL be treated as missing and error"
        );
        let err_msg = format!("{}", result.expect_err("SHALL be an error"));
        assert!(
            err_msg.contains("missing 'mixin' parameter"),
            "error SHALL mention missing mixin param, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 23. load_mixin referencing a missing file errors on read
    // ---------------------------------------------------------------
    #[test]
    fn load_mixin_unresolvable_name_errors() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let steps = vec![load_mixin_step("does_not_exist", None)];

        let result = expand_mixins(&steps, tmp.path(), tmp.path());
        assert!(result.is_err(), "unresolvable mixin SHALL error");
        let err_msg = format!("{}", result.expect_err("SHALL be an error"));
        assert!(
            err_msg.contains("not found"),
            "error SHALL say not found, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 24. extract_vars_from_step ignores non-string var values
    // ---------------------------------------------------------------
    #[test]
    fn extract_vars_ignores_non_string_values() {
        let mut table = toml::map::Map::new();
        table.insert("good".to_string(), toml::Value::String("ok".to_string()));
        table.insert("num".to_string(), toml::Value::Integer(7));
        table.insert("flag".to_string(), toml::Value::Boolean(true));
        let mut params = HashMap::new();
        params.insert("vars".to_string(), toml::Value::Table(table));
        let step = Step {
            action: "load_mixin".to_string(),
            params,
            ..Default::default()
        };

        let vars = extract_vars_from_step(&step);
        assert_eq!(vars.len(), 1, "only string var SHALL be extracted");
        assert_eq!(vars.get("good").map(String::as_str), Some("ok"));
        assert!(!vars.contains_key("num"), "integer var SHALL be ignored");
        assert!(!vars.contains_key("flag"), "boolean var SHALL be ignored");
    }

    // ---------------------------------------------------------------
    // 25. extract_vars_from_step returns empty when vars absent
    // ---------------------------------------------------------------
    #[test]
    fn extract_vars_empty_when_absent() {
        let step = load_mixin_step("x", None);
        let vars = extract_vars_from_step(&step);
        assert!(vars.is_empty(), "no vars param SHALL yield empty map");
    }

    // ---------------------------------------------------------------
    // 26. remap_string replaces multiple distinct keys
    // ---------------------------------------------------------------
    #[test]
    fn remap_string_multiple_keys() {
        let mut vars = HashMap::new();
        vars.insert("a".to_string(), "X".to_string());
        vars.insert("b".to_string(), "Y".to_string());

        let out = remap_string("${a}-${b}-${a}", &vars);
        assert_eq!(out, "X-Y-X", "every occurrence of each key SHALL be replaced");
    }

    // ---------------------------------------------------------------
    // 27. remap remaps the grouped `on` selector and nested fields
    // ---------------------------------------------------------------
    #[test]
    fn remap_grouped_on_selector() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        write_mixin(
            tmp.path(),
            "grouped",
            r#"
[[step]]
action = "tap"

[step.on]
text = "${label}"
accessibility_label = "${aid}"
below = "${anchor_text}"
"#,
        );

        let mut vars = HashMap::new();
        vars.insert("label".to_string(), "Submit".to_string());
        vars.insert("aid".to_string(), "submit-btn".to_string());
        vars.insert("anchor_text".to_string(), "Header".to_string());

        let steps = vec![load_mixin_step("grouped", Some(vars))];
        let expanded =
            expand_mixins(&steps, tmp.path(), tmp.path()).expect("expansion should succeed");

        let group = expanded[0].on.as_ref().expect("on group SHALL be present");
        assert_eq!(group.text.as_deref(), Some("Submit"));
        assert_eq!(group.accessibility_label.as_deref(), Some("submit-btn"));
        match group.below.as_ref().expect("below anchor SHALL be present") {
            crate::Anchor::Text(s) => assert_eq!(s, "Header"),
            other => panic!("below SHALL be a Text anchor, got: {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // 28. remap descends into a nested Selector anchor
    // ---------------------------------------------------------------
    #[test]
    fn remap_nested_selector_anchor() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        write_mixin(
            tmp.path(),
            "nested_anchor",
            r#"
[[step]]
action = "tap"

[step.on]
text = "${label}"

[step.on.right_of]
text = "${anchor_label}"
"#,
        );

        let mut vars = HashMap::new();
        vars.insert("label".to_string(), "Field".to_string());
        vars.insert("anchor_label".to_string(), "Caption".to_string());

        let steps = vec![load_mixin_step("nested_anchor", Some(vars))];
        let expanded =
            expand_mixins(&steps, tmp.path(), tmp.path()).expect("expansion should succeed");

        let group = expanded[0].on.as_ref().expect("on group SHALL be present");
        match group.right_of.as_ref().expect("right_of SHALL be present") {
            crate::Anchor::Selector(inner) => {
                assert_eq!(inner.text.as_deref(), Some("Caption"));
            }
            other => panic!("right_of SHALL be a Selector anchor, got: {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // 29. remap recurses into nested params (table + array)
    // ---------------------------------------------------------------
    #[test]
    fn remap_recursive_params() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        write_mixin(
            tmp.path(),
            "params",
            r#"
[[step]]
action = "custom"
list = ["${a}", "literal"]

[step.nested]
inner = "${b}"
"#,
        );

        let mut vars = HashMap::new();
        vars.insert("a".to_string(), "AA".to_string());
        vars.insert("b".to_string(), "BB".to_string());

        let steps = vec![load_mixin_step("params", Some(vars))];
        let expanded =
            expand_mixins(&steps, tmp.path(), tmp.path()).expect("expansion should succeed");

        let list = expanded[0]
            .params
            .get("list")
            .and_then(|v| v.as_array())
            .expect("list param SHALL be an array");
        assert_eq!(list[0].as_str(), Some("AA"), "array string SHALL be remapped");
        assert_eq!(list[1].as_str(), Some("literal"), "non-var literal SHALL stay");

        let inner = expanded[0]
            .params
            .get("nested")
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("inner"))
            .and_then(|v| v.as_str())
            .expect("nested.inner SHALL be a string");
        assert_eq!(inner, "BB", "nested table string SHALL be remapped");
    }

    // ---------------------------------------------------------------
    // 30. remap leaves non-string params (integers) untouched
    // ---------------------------------------------------------------
    #[test]
    fn remap_preserves_non_string_params() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        write_mixin(
            tmp.path(),
            "typed",
            r#"
[[step]]
action = "custom"
count = 5
label = "${name}"
"#,
        );

        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Bob".to_string());

        let steps = vec![load_mixin_step("typed", Some(vars))];
        let expanded =
            expand_mixins(&steps, tmp.path(), tmp.path()).expect("expansion should succeed");

        assert_eq!(
            expanded[0].params.get("count").and_then(|v| v.as_integer()),
            Some(5),
            "integer param SHALL be untouched"
        );
        assert_eq!(
            expanded[0].params.get("label").and_then(|v| v.as_str()),
            Some("Bob"),
            "string param SHALL be remapped"
        );
    }

    // ---------------------------------------------------------------
    // 31. remap rewrites flat relational on_* fields and action
    // ---------------------------------------------------------------
    #[test]
    fn remap_flat_relational_and_action() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        write_mixin(
            tmp.path(),
            "flat",
            r#"
[[step]]
action = "${act}"
on_below = "${below_anchor}"
on_above = "${above_anchor}"
input = "${value}"
app = "${app_id}"
"#,
        );

        let mut vars = HashMap::new();
        vars.insert("act".to_string(), "tap".to_string());
        vars.insert("below_anchor".to_string(), "Footer".to_string());
        vars.insert("above_anchor".to_string(), "Banner".to_string());
        vars.insert("value".to_string(), "typed-text".to_string());
        vars.insert("app_id".to_string(), "com.example".to_string());

        let steps = vec![load_mixin_step("flat", Some(vars))];
        let expanded =
            expand_mixins(&steps, tmp.path(), tmp.path()).expect("expansion should succeed");

        let step = &expanded[0];
        assert_eq!(step.action, "tap", "action SHALL be remapped");
        assert_eq!(step.on_below.as_deref(), Some("Footer"));
        assert_eq!(step.on_above.as_deref(), Some("Banner"));
        assert_eq!(step.input.as_deref(), Some("typed-text"));
        assert_eq!(step.app.as_deref(), Some("com.example"));
    }

    // ---------------------------------------------------------------
    // 32. parse_mixin defaults to empty steps for empty input
    // ---------------------------------------------------------------
    #[test]
    fn parse_mixin_empty_yields_no_steps() {
        let mixin = parse_mixin("").expect("empty mixin SHALL parse");
        assert!(mixin.steps.is_empty(), "empty mixin SHALL have no steps");
    }

    // ---------------------------------------------------------------
    // 33. parse_mixin rejects malformed TOML
    // ---------------------------------------------------------------
    #[test]
    fn parse_mixin_malformed_toml_errors() {
        let result = parse_mixin("this is = = not toml");
        assert!(result.is_err(), "malformed TOML SHALL error");
    }
}
