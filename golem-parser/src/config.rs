use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{FlowFile, FlowOptions, TeardownBlock};

/// Project-level options from golem.toml `[options]`.
/// All fields are optional — missing fields are left as `None`.
#[derive(Deserialize, Debug, Clone, Default)]
pub struct ProjectOptions {
    pub max_concurrency: Option<u32>,
    pub min_free_ram_mb: Option<u64>,
    pub min_free_disk_mb: Option<u64>,
    pub create_if_missing: Option<bool>,
    pub ignore_missing_physical: Option<bool>,
    pub step_timeout: Option<u64>,
    pub screenshot_on_failure: Option<bool>,
    pub screenshot_dir: Option<String>,
    pub record: Option<bool>,
    pub max_steps: Option<u64>,
    pub max_runtime: Option<String>,
    pub suite_concurrency: Option<u32>,
    pub keep_devices: Option<bool>,
    pub coverage: Option<crate::CoverageStrategy>,
    pub app_lifecycle: Option<crate::AppLifecycle>,
    pub perf: Option<bool>,
    pub perf_memory_warn_mb: Option<f64>,
    pub perf_memory_error_mb: Option<f64>,
    pub perf_cpu_warn_percent: Option<f64>,
    pub perf_cpu_error_percent: Option<f64>,
    pub perf_threads_warn: Option<u32>,
    pub perf_threads_error: Option<u32>,
    pub perf_fd_warn: Option<u32>,
    pub perf_fd_error: Option<u32>,
    pub a11y: Option<crate::A11yLevel>,
    pub a11y_max_errors: Option<usize>,
    pub a11y_max_warnings: Option<usize>,
    pub a11y_min_confidence: Option<f32>,
}

/// Internal deserialization target for `golem.toml`.
#[derive(Deserialize, Debug, Clone, Default)]
struct ProjectConfigRaw {
    #[serde(default)]
    options: ProjectOptions,
    #[serde(default)]
    vars: HashMap<String, String>,
    #[serde(default)]
    teardown: Vec<TeardownBlock>,
}

/// Public project configuration with options and vars.
#[derive(Debug, Clone, Default)]
pub struct ProjectConfig {
    pub options: ProjectOptions,
    pub vars: HashMap<String, String>,
    /// Optional project-level teardown block to append to every flow.
    pub teardown: Option<TeardownBlock>,
}

/// Parse a golem.toml string into a `ProjectConfig`.
pub fn parse_project_config(toml_str: &str) -> anyhow::Result<ProjectConfig> {
    let raw: ProjectConfigRaw = toml::from_str(toml_str)?;
    // Take the first teardown block if present (TOML `[[teardown]]` yields a vec).
    let teardown = raw.teardown.into_iter().next();
    Ok(ProjectConfig {
        options: raw.options,
        vars: raw.vars,
        teardown,
    })
}

/// Walk up from `start_dir` looking for `golem.toml`, similar to how git finds `.git`.
/// Returns `Some(path)` to the `golem.toml` file if found, or `None`.
pub fn find_project_config(start_dir: &Path) -> Option<PathBuf> {
    let mut current = start_dir
        .canonicalize()
        .unwrap_or_else(|_| start_dir.to_path_buf());

    loop {
        let candidate = current.join("golem.toml");
        if candidate.is_file() {
            return Some(candidate);
        }

        match current.parent() {
            Some(parent) => {
                let parent = parent.to_path_buf();
                if parent == current {
                    // Reached filesystem root
                    break;
                }
                current = parent;
            }
            None => break,
        }
    }

    None
}

/// Find and parse `golem.toml` by walking up from `start_dir`.
/// Returns `Ok(Some(config))` if found and parsed, `Ok(None)` if not found,
/// or `Err` if found but failed to parse.
pub fn load_project_config(start_dir: &Path) -> anyhow::Result<Option<ProjectConfig>> {
    match find_project_config(start_dir) {
        Some(path) => {
            let content = std::fs::read_to_string(&path)?;
            let config = parse_project_config(&content)?;
            Ok(Some(config))
        }
        None => Ok(None),
    }
}

/// Merge project config into a flow file. Flow-level values take precedence
/// over project-level values.
///
/// - `project.vars` are inherited by `flow.vars`, but flow vars override.
/// - `project.options` are inherited by `flow.options`, but flow options override.
pub fn merge_config(project: &ProjectConfig, flow: &FlowFile) -> FlowFile {
    let mut merged = flow.clone();

    // Merge vars: start with project vars, then overlay flow vars
    let mut merged_vars = project.vars.clone();
    for (key, value) in &flow.flow.vars {
        merged_vars.insert(key.clone(), value.clone());
    }
    merged.flow.vars = merged_vars;

    // Merge options: start with project options, then overlay flow options
    let flow_opts = flow.flow.options.clone().unwrap_or_default();
    let proj_opts = &project.options;

    let merged_opts = FlowOptions {
        max_concurrency: flow_opts.max_concurrency.or(proj_opts.max_concurrency),
        min_free_ram_mb: flow_opts.min_free_ram_mb.or(proj_opts.min_free_ram_mb),
        min_free_disk_mb: flow_opts.min_free_disk_mb.or(proj_opts.min_free_disk_mb),
        create_if_missing: flow_opts.create_if_missing.or(proj_opts.create_if_missing),
        ignore_missing_physical: flow_opts
            .ignore_missing_physical
            .or(proj_opts.ignore_missing_physical),
        step_timeout: flow_opts.step_timeout.or(proj_opts.step_timeout),
        screenshot_on_failure: flow_opts
            .screenshot_on_failure
            .or(proj_opts.screenshot_on_failure),
        screenshot_dir: flow_opts
            .screenshot_dir
            .clone()
            .or_else(|| proj_opts.screenshot_dir.clone()),
        record: flow_opts.record.or(proj_opts.record),
        max_steps: flow_opts.max_steps.or(proj_opts.max_steps),
        max_runtime: flow_opts
            .max_runtime
            .clone()
            .or_else(|| proj_opts.max_runtime.clone()),
        suite_concurrency: flow_opts.suite_concurrency.or(proj_opts.suite_concurrency),
        keep_devices: flow_opts.keep_devices.or(proj_opts.keep_devices),
        coverage: flow_opts.coverage.or(proj_opts.coverage),
        app_lifecycle: flow_opts.app_lifecycle.or(proj_opts.app_lifecycle),
        perf: flow_opts.perf.or(proj_opts.perf),
        perf_memory_warn_mb: flow_opts
            .perf_memory_warn_mb
            .or(proj_opts.perf_memory_warn_mb),
        perf_memory_error_mb: flow_opts
            .perf_memory_error_mb
            .or(proj_opts.perf_memory_error_mb),
        perf_cpu_warn_percent: flow_opts
            .perf_cpu_warn_percent
            .or(proj_opts.perf_cpu_warn_percent),
        perf_cpu_error_percent: flow_opts
            .perf_cpu_error_percent
            .or(proj_opts.perf_cpu_error_percent),
        perf_threads_warn: flow_opts.perf_threads_warn.or(proj_opts.perf_threads_warn),
        perf_threads_error: flow_opts
            .perf_threads_error
            .or(proj_opts.perf_threads_error),
        perf_fd_warn: flow_opts.perf_fd_warn.or(proj_opts.perf_fd_warn),
        perf_fd_error: flow_opts.perf_fd_error.or(proj_opts.perf_fd_error),
        a11y: flow_opts.a11y.or(proj_opts.a11y),
        a11y_max_errors: flow_opts.a11y_max_errors.or(proj_opts.a11y_max_errors),
        a11y_max_warnings: flow_opts.a11y_max_warnings.or(proj_opts.a11y_max_warnings),
        a11y_min_confidence: flow_opts
            .a11y_min_confidence
            .or(proj_opts.a11y_min_confidence),
    };

    // Only set options if at least one field is Some
    let has_any_option = merged_opts.max_concurrency.is_some()
        || merged_opts.min_free_ram_mb.is_some()
        || merged_opts.min_free_disk_mb.is_some()
        || merged_opts.create_if_missing.is_some()
        || merged_opts.ignore_missing_physical.is_some()
        || merged_opts.step_timeout.is_some()
        || merged_opts.screenshot_on_failure.is_some()
        || merged_opts.screenshot_dir.is_some()
        || merged_opts.record.is_some()
        || merged_opts.max_steps.is_some()
        || merged_opts.max_runtime.is_some()
        || merged_opts.suite_concurrency.is_some()
        || merged_opts.keep_devices.is_some()
        || merged_opts.coverage.is_some()
        || merged_opts.app_lifecycle.is_some()
        || merged_opts.perf.is_some()
        || merged_opts.perf_memory_warn_mb.is_some()
        || merged_opts.perf_memory_error_mb.is_some()
        || merged_opts.perf_cpu_warn_percent.is_some()
        || merged_opts.perf_cpu_error_percent.is_some()
        || merged_opts.perf_threads_warn.is_some()
        || merged_opts.perf_threads_error.is_some()
        || merged_opts.perf_fd_warn.is_some()
        || merged_opts.perf_fd_error.is_some()
        || merged_opts.a11y.is_some()
        || merged_opts.a11y_max_errors.is_some()
        || merged_opts.a11y_max_warnings.is_some()
        || merged_opts.a11y_min_confidence.is_some();

    merged.flow.options = if has_any_option {
        Some(merged_opts)
    } else {
        None
    };

    // Merge teardown: flow teardown first, then project teardown
    if let Some(ref project_teardown) = project.teardown {
        merged.teardown.push(project_teardown.clone());
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_flow;
    use std::fs;
    use tempfile::TempDir;

    // ---------------------------------------------------------------
    // 1. Parse empty golem.toml
    // ---------------------------------------------------------------
    #[test]
    fn parse_empty_project_config() {
        let toml_str = "";
        let config = parse_project_config(toml_str).expect("empty golem.toml should parse");
        assert!(config.vars.is_empty());
        assert!(config.options.max_concurrency.is_none());
        assert!(config.options.step_timeout.is_none());
    }

    // ---------------------------------------------------------------
    // 2. Parse options only
    // ---------------------------------------------------------------
    #[test]
    fn parse_options_only() {
        let toml_str = r#"
[options]
max_concurrency = 4
step_timeout = 10000
screenshot_on_failure = true
screenshot_dir = ".golem/screenshots"
"#;
        let config = parse_project_config(toml_str).expect("options-only config should parse");
        assert!(config.vars.is_empty());
        assert_eq!(config.options.max_concurrency, Some(4));
        assert_eq!(config.options.step_timeout, Some(10000));
        assert_eq!(config.options.screenshot_on_failure, Some(true));
        assert_eq!(
            config.options.screenshot_dir.as_deref(),
            Some(".golem/screenshots")
        );
    }

    // ---------------------------------------------------------------
    // 3. Parse vars only
    // ---------------------------------------------------------------
    #[test]
    fn parse_vars_only() {
        let toml_str = r#"
[vars]
api_token = "sk-test-abc123"
staging_url = "https://api.staging.example.com"
default_country = "JP"
"#;
        let config = parse_project_config(toml_str).expect("vars-only config should parse");
        assert_eq!(
            config.vars.get("api_token").map(|s| s.as_str()),
            Some("sk-test-abc123")
        );
        assert_eq!(
            config.vars.get("staging_url").map(|s| s.as_str()),
            Some("https://api.staging.example.com")
        );
        assert_eq!(
            config.vars.get("default_country").map(|s| s.as_str()),
            Some("JP")
        );
        assert!(config.options.max_concurrency.is_none());
    }

    // ---------------------------------------------------------------
    // 4. Parse full config (options + vars)
    // ---------------------------------------------------------------
    #[test]
    fn parse_full_config() {
        let toml_str = r#"
[options]
max_concurrency = 4
min_free_ram_mb = 2048
create_if_missing = true
ignore_missing_physical = true
step_timeout = 10000
screenshot_on_failure = true
screenshot_dir = ".golem/screenshots"

[vars]
api_token = "sk-test-abc123"
staging_url = "https://api.staging.example.com"
default_country = "JP"
"#;
        let config = parse_project_config(toml_str).expect("full config should parse");

        assert_eq!(config.options.max_concurrency, Some(4));
        assert_eq!(config.options.min_free_ram_mb, Some(2048));
        assert_eq!(config.options.create_if_missing, Some(true));
        assert_eq!(config.options.ignore_missing_physical, Some(true));
        assert_eq!(config.options.step_timeout, Some(10000));
        assert_eq!(config.options.screenshot_on_failure, Some(true));
        assert_eq!(
            config.options.screenshot_dir.as_deref(),
            Some(".golem/screenshots")
        );

        assert_eq!(config.vars.len(), 3);
        assert_eq!(
            config.vars.get("api_token").map(|s| s.as_str()),
            Some("sk-test-abc123")
        );
    }

    // ---------------------------------------------------------------
    // 5. Unknown options fields are ignored
    // ---------------------------------------------------------------
    #[test]
    fn unknown_options_fields_ignored() {
        // ProjectOptions uses default serde behavior (no deny_unknown_fields
        // on the options struct), so unknown fields in [options] are ignored.
        // However, the top-level ProjectConfigRaw uses deny_unknown_fields
        // for unknown top-level sections. We test that unknown option *fields*
        // within [options] are fine.
        let toml_str = r#"
[options]
max_concurrency = 4
future_flag = true
experimental_mode = "beta"
"#;
        let config = parse_project_config(toml_str);
        // Since ProjectOptions doesn't deny unknown fields, this should work
        assert!(
            config.is_ok(),
            "Unknown option fields should be ignored: {:?}",
            config.err()
        );
        let config = config.expect("should parse");
        assert_eq!(config.options.max_concurrency, Some(4));
    }

    // ---------------------------------------------------------------
    // 6. find_project_config finds golem.toml in current dir
    // ---------------------------------------------------------------
    #[test]
    fn find_config_in_current_dir() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let config_path = tmp.path().join("golem.toml");
        fs::write(&config_path, "[options]\nmax_concurrency = 2\n")
            .expect("Failed to write golem.toml");

        let result = find_project_config(tmp.path());
        assert!(result.is_some(), "Should find golem.toml in current dir");
        let found = result.expect("should be Some");
        assert!(
            found.ends_with("golem.toml"),
            "Found path should end with golem.toml, got: {}",
            found.display()
        );
    }

    // ---------------------------------------------------------------
    // 7. find_project_config finds golem.toml in parent dir
    // ---------------------------------------------------------------
    #[test]
    fn find_config_in_parent_dir() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let config_path = tmp.path().join("golem.toml");
        fs::write(&config_path, "[options]\nmax_concurrency = 2\n")
            .expect("Failed to write golem.toml");

        let child_dir = tmp.path().join("flows");
        fs::create_dir_all(&child_dir).expect("Failed to create child dir");

        let result = find_project_config(&child_dir);
        assert!(result.is_some(), "Should find golem.toml in parent dir");
    }

    // ---------------------------------------------------------------
    // 8. find_project_config finds golem.toml in grandparent dir
    // ---------------------------------------------------------------
    #[test]
    fn find_config_in_grandparent_dir() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let config_path = tmp.path().join("golem.toml");
        fs::write(&config_path, "[options]\nmax_concurrency = 2\n")
            .expect("Failed to write golem.toml");

        let deep_dir = tmp.path().join("flows").join("auth").join("login");
        fs::create_dir_all(&deep_dir).expect("Failed to create deep dir");

        let result = find_project_config(&deep_dir);
        assert!(
            result.is_some(),
            "Should find golem.toml in grandparent dir"
        );
    }

    // ---------------------------------------------------------------
    // 9. find_project_config returns None when no golem.toml exists
    // ---------------------------------------------------------------
    #[test]
    fn find_config_returns_none_when_missing() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let result = find_project_config(tmp.path());
        assert!(
            result.is_none(),
            "Should return None when no golem.toml exists"
        );
    }

    // ---------------------------------------------------------------
    // Helper: build a minimal flow for merge tests
    // ---------------------------------------------------------------
    fn minimal_flow(flow_toml: &str) -> FlowFile {
        parse_flow(flow_toml).expect("flow should parse")
    }

    // ---------------------------------------------------------------
    // 10. merge_config with empty project config changes nothing
    // ---------------------------------------------------------------
    #[test]
    fn merge_empty_project_changes_nothing() {
        let project = ProjectConfig::default();
        let flow = minimal_flow(
            r#"
[flow]
name = "test"

[flow.vars]
user = "alice"
"#,
        );

        let merged = merge_config(&project, &flow);
        assert_eq!(merged.flow.name, "test");
        assert_eq!(merged.flow.vars.len(), 1);
        assert_eq!(
            merged.flow.vars.get("user").map(|s| s.as_str()),
            Some("alice")
        );
        assert!(merged.flow.options.is_none());
    }

    // ---------------------------------------------------------------
    // 11. merge_config project vars are inherited by flow
    // ---------------------------------------------------------------
    #[test]
    fn merge_project_vars_inherited() {
        let project = parse_project_config(
            r#"
[vars]
api_token = "sk-test-abc123"
staging_url = "https://staging.example.com"
"#,
        )
        .expect("project config should parse");

        let flow = minimal_flow(
            r#"
[flow]
name = "test"
"#,
        );

        let merged = merge_config(&project, &flow);
        assert_eq!(merged.flow.vars.len(), 2);
        assert_eq!(
            merged.flow.vars.get("api_token").map(|s| s.as_str()),
            Some("sk-test-abc123")
        );
        assert_eq!(
            merged.flow.vars.get("staging_url").map(|s| s.as_str()),
            Some("https://staging.example.com")
        );
    }

    // ---------------------------------------------------------------
    // 12. merge_config flow vars override project vars
    // ---------------------------------------------------------------
    #[test]
    fn merge_flow_vars_override_project() {
        let project = parse_project_config(
            r#"
[vars]
api_token = "sk-project-token"
staging_url = "https://staging.example.com"
"#,
        )
        .expect("project config should parse");

        let flow = minimal_flow(
            r#"
[flow]
name = "test"

[flow.vars]
api_token = "sk-flow-override"
extra_var = "only_in_flow"
"#,
        );

        let merged = merge_config(&project, &flow);
        // Flow overrides project
        assert_eq!(
            merged.flow.vars.get("api_token").map(|s| s.as_str()),
            Some("sk-flow-override")
        );
        // Project var inherited
        assert_eq!(
            merged.flow.vars.get("staging_url").map(|s| s.as_str()),
            Some("https://staging.example.com")
        );
        // Flow-only var preserved
        assert_eq!(
            merged.flow.vars.get("extra_var").map(|s| s.as_str()),
            Some("only_in_flow")
        );
        assert_eq!(merged.flow.vars.len(), 3);
    }

    // ---------------------------------------------------------------
    // 13. merge_config project options are inherited
    // ---------------------------------------------------------------
    #[test]
    fn merge_project_options_inherited() {
        let project = parse_project_config(
            r#"
[options]
max_concurrency = 4
step_timeout = 10000
screenshot_on_failure = true
screenshot_dir = ".golem/screenshots"
"#,
        )
        .expect("project config should parse");

        let flow = minimal_flow(
            r#"
[flow]
name = "test"
"#,
        );

        let merged = merge_config(&project, &flow);
        let opts = merged
            .flow
            .options
            .expect("merged flow should have options");
        assert_eq!(opts.max_concurrency, Some(4));
        assert_eq!(opts.step_timeout, Some(10000));
        assert_eq!(opts.screenshot_on_failure, Some(true));
        assert_eq!(opts.screenshot_dir.as_deref(), Some(".golem/screenshots"));
    }

    // ---------------------------------------------------------------
    // 14. merge_config flow options override project options
    // ---------------------------------------------------------------
    #[test]
    fn merge_flow_options_override_project() {
        let project = parse_project_config(
            r#"
[options]
max_concurrency = 4
step_timeout = 10000
"#,
        )
        .expect("project config should parse");

        let flow = minimal_flow(
            r#"
[flow]
name = "test"

[flow.options]
max_concurrency = 8
step_timeout = 5000
"#,
        );

        let merged = merge_config(&project, &flow);
        let opts = merged
            .flow
            .options
            .expect("merged flow should have options");
        // Flow overrides both
        assert_eq!(opts.max_concurrency, Some(8));
        assert_eq!(opts.step_timeout, Some(5000));
    }

    // ---------------------------------------------------------------
    // 15. Project config with teardown parses
    // ---------------------------------------------------------------
    #[test]
    fn parse_project_config_with_teardown() {
        let toml_str = r#"
[vars]
token = "abc"

[[teardown]]

[[teardown.steps]]
action = "screenshot"

[[teardown.steps]]
action = "back"
"#;
        let config =
            parse_project_config(toml_str).expect("project config with teardown should parse");
        assert_eq!(config.vars.get("token").map(|s| s.as_str()), Some("abc"));
        let teardown = config.teardown.as_ref().expect("SHALL have teardown block");
        assert_eq!(teardown.steps.len(), 2, "SHALL contain both teardown steps");
        assert_eq!(teardown.steps[0].action, "screenshot");
        assert_eq!(teardown.steps[1].action, "back");
    }

    // ---------------------------------------------------------------
    // 16. Project teardown merges into flow
    // ---------------------------------------------------------------
    #[test]
    fn merge_project_teardown_into_flow() {
        let project = parse_project_config(
            r#"
[[teardown]]

[[teardown.steps]]
action = "screenshot"
"#,
        )
        .expect("project config should parse");

        let flow = minimal_flow(
            r#"
[flow]
name = "test"
"#,
        );

        let merged = merge_config(&project, &flow);
        assert_eq!(
            merged.teardown.len(),
            1,
            "SHALL have one teardown block from project"
        );
        assert_eq!(merged.teardown[0].steps[0].action, "screenshot");
    }

    // ---------------------------------------------------------------
    // 17. Flow's own teardown comes before project teardown
    // ---------------------------------------------------------------
    #[test]
    fn merge_flow_teardown_before_project_teardown() {
        let project = parse_project_config(
            r#"
[[teardown]]

[[teardown.steps]]
action = "screenshot"
"#,
        )
        .expect("project config should parse");

        let flow = minimal_flow(
            r#"
[flow]
name = "test"

[[teardown]]

[[teardown.steps]]
action = "back"
"#,
        );

        let merged = merge_config(&project, &flow);
        assert_eq!(
            merged.teardown.len(),
            2,
            "SHALL have flow teardown + project teardown"
        );
        assert_eq!(
            merged.teardown[0].steps[0].action, "back",
            "SHALL place flow teardown first"
        );
        assert_eq!(
            merged.teardown[1].steps[0].action, "screenshot",
            "SHALL place project teardown after flow teardown"
        );
    }

    // ---------------------------------------------------------------
    // 18. No project teardown leaves flow teardown unchanged
    // ---------------------------------------------------------------
    #[test]
    fn merge_no_project_teardown_leaves_flow_unchanged() {
        let project = parse_project_config(
            r#"
[vars]
token = "abc"
"#,
        )
        .expect("project config should parse");

        let flow = minimal_flow(
            r#"
[flow]
name = "test"

[[teardown]]

[[teardown.steps]]
action = "back"
"#,
        );

        let merged = merge_config(&project, &flow);
        assert_eq!(
            merged.teardown.len(),
            1,
            "SHALL preserve flow teardown when project has none"
        );
        assert_eq!(merged.teardown[0].steps[0].action, "back");
    }

    // ---------------------------------------------------------------
    // 19. merge_config partial option override
    // ---------------------------------------------------------------
    #[test]
    fn merge_partial_option_override() {
        let project = parse_project_config(
            r#"
[options]
max_concurrency = 4
step_timeout = 10000
screenshot_on_failure = true
"#,
        )
        .expect("project config should parse");

        let flow = minimal_flow(
            r#"
[flow]
name = "test"

[flow.options]
step_timeout = 5000
record = true
"#,
        );

        let merged = merge_config(&project, &flow);
        let opts = merged
            .flow
            .options
            .expect("merged flow should have options");
        // From project (not overridden)
        assert_eq!(opts.max_concurrency, Some(4));
        assert_eq!(opts.screenshot_on_failure, Some(true));
        // Overridden by flow
        assert_eq!(opts.step_timeout, Some(5000));
        // Only in flow
        assert_eq!(opts.record, Some(true));
    }

    // ---------------------------------------------------------------
    // 20. Invalid TOML surfaces a parse error
    // ---------------------------------------------------------------
    #[test]
    fn parse_invalid_toml_errors() {
        // Unterminated string / malformed key-value is not valid TOML.
        let toml_str = "this is = = not valid toml";
        let result = parse_project_config(toml_str);
        assert!(
            result.is_err(),
            "Malformed TOML SHALL surface a parse error"
        );
    }

    // ---------------------------------------------------------------
    // 21. Wrong value type for a known option surfaces an error
    // ---------------------------------------------------------------
    #[test]
    fn parse_wrong_option_type_errors() {
        // max_concurrency is u32; a string SHALL fail deserialization.
        let toml_str = r#"
[options]
max_concurrency = "not a number"
"#;
        let result = parse_project_config(toml_str);
        assert!(
            result.is_err(),
            "Wrong-typed option value SHALL surface a deserialization error"
        );
    }

    // ---------------------------------------------------------------
    // 22. Only the first [[teardown]] block is taken
    // ---------------------------------------------------------------
    #[test]
    fn parse_only_first_teardown_block_taken() {
        let toml_str = r#"
[[teardown]]

[[teardown.steps]]
action = "screenshot"

[[teardown]]

[[teardown.steps]]
action = "back"
"#;
        let config = parse_project_config(toml_str).expect("multi-teardown config SHALL parse");
        let teardown = config
            .teardown
            .as_ref()
            .expect("SHALL retain a teardown block");
        assert_eq!(
            teardown.steps.len(),
            1,
            "SHALL keep only the first teardown block"
        );
        assert_eq!(
            teardown.steps[0].action, "screenshot",
            "SHALL keep the first block, not the second"
        );
    }

    // ---------------------------------------------------------------
    // 23. coverage and app_lifecycle options parse into enums
    // ---------------------------------------------------------------
    #[test]
    fn parse_coverage_and_app_lifecycle() {
        let toml_str = r#"
[options]
coverage = "full"
app_lifecycle = "manual"
"#;
        let config =
            parse_project_config(toml_str).expect("coverage/app_lifecycle config SHALL parse");
        assert_eq!(config.options.coverage, Some(crate::CoverageStrategy::Full));
        assert_eq!(
            config.options.app_lifecycle,
            Some(crate::AppLifecycle::Manual)
        );
    }

    // ---------------------------------------------------------------
    // 24. Invalid coverage enum value surfaces an error
    // ---------------------------------------------------------------
    #[test]
    fn parse_invalid_coverage_value_errors() {
        let toml_str = r#"
[options]
coverage = "sideways"
"#;
        let result = parse_project_config(toml_str);
        assert!(
            result.is_err(),
            "Unknown coverage variant SHALL surface a deserialization error"
        );
    }

    // ---------------------------------------------------------------
    // 25. load_project_config returns Ok(None) when no golem.toml exists
    // ---------------------------------------------------------------
    #[test]
    fn load_config_none_when_missing() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let result =
            load_project_config(tmp.path()).expect("absent config SHALL be Ok(None), not Err");
        assert!(
            result.is_none(),
            "SHALL return None when no golem.toml is found"
        );
    }

    // ---------------------------------------------------------------
    // 26. load_project_config reads and parses a found golem.toml
    // ---------------------------------------------------------------
    #[test]
    fn load_config_reads_and_parses() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        fs::write(
            tmp.path().join("golem.toml"),
            "[options]\nmax_concurrency = 7\n\n[vars]\nfoo = \"bar\"\n",
        )
        .expect("Failed to write golem.toml");

        let config = load_project_config(tmp.path())
            .expect("present config SHALL parse without error")
            .expect("SHALL return Some when golem.toml exists");
        assert_eq!(config.options.max_concurrency, Some(7));
        assert_eq!(config.vars.get("foo").map(|s| s.as_str()), Some("bar"));
    }

    // ---------------------------------------------------------------
    // 27. load_project_config surfaces parse errors on malformed file
    // ---------------------------------------------------------------
    #[test]
    fn load_config_propagates_parse_error() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        fs::write(
            tmp.path().join("golem.toml"),
            "[options]\nmax_concurrency = \"oops\"\n",
        )
        .expect("Failed to write golem.toml");

        let result = load_project_config(tmp.path());
        assert!(
            result.is_err(),
            "A found-but-malformed golem.toml SHALL surface an Err, not Ok(None)"
        );
    }

    // ---------------------------------------------------------------
    // 28. find_project_config tolerates a non-existent start dir
    //     (canonicalize fails, falls back to the literal path)
    // ---------------------------------------------------------------
    #[test]
    fn find_config_nonexistent_start_dir_returns_none() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let missing = tmp.path().join("does").join("not").join("exist");
        let result = find_project_config(&missing);
        assert!(
            result.is_none(),
            "SHALL return None (not panic) when start dir cannot be canonicalized"
        );
    }

    // ---------------------------------------------------------------
    // 29. merge_config flows coverage/app_lifecycle through when another
    //     counted option is also present
    // ---------------------------------------------------------------
    #[test]
    fn merge_coverage_and_app_lifecycle_inherited() {
        let project = parse_project_config(
            r#"
[options]
max_concurrency = 2
coverage = "min"
app_lifecycle = "launch"
"#,
        )
        .expect("project config SHALL parse");

        let flow = minimal_flow(
            r#"
[flow]
name = "test"
"#,
        );

        let merged = merge_config(&project, &flow);
        let opts = merged.flow.options.expect("merged flow SHALL have options");
        assert_eq!(opts.coverage, Some(crate::CoverageStrategy::Min));
        assert_eq!(opts.app_lifecycle, Some(crate::AppLifecycle::Launch));
    }

    // ---------------------------------------------------------------
    // 30. merge_config inherits numeric/perf project options when a
    //     counted option is present; flow value wins per-field
    // ---------------------------------------------------------------
    #[test]
    fn merge_perf_and_numeric_options_inherited() {
        let project = parse_project_config(
            r#"
[options]
min_free_ram_mb = 1024
min_free_disk_mb = 4096
max_steps = 500
max_runtime = "10m"
suite_concurrency = 3
keep_devices = true
perf_memory_warn_mb = 128.5
perf_threads_warn = 64
"#,
        )
        .expect("project config SHALL parse");

        let flow = minimal_flow(
            r#"
[flow]
name = "test"

[flow.options]
max_runtime = "5m"
"#,
        );

        let merged = merge_config(&project, &flow);
        let opts = merged.flow.options.expect("merged flow SHALL have options");
        // Inherited from project.
        assert_eq!(opts.min_free_ram_mb, Some(1024));
        assert_eq!(opts.min_free_disk_mb, Some(4096));
        assert_eq!(opts.max_steps, Some(500));
        assert_eq!(opts.suite_concurrency, Some(3));
        assert_eq!(opts.keep_devices, Some(true));
        assert_eq!(opts.perf_memory_warn_mb, Some(128.5));
        assert_eq!(opts.perf_threads_warn, Some(64));
        // Flow value wins.
        assert_eq!(opts.max_runtime.as_deref(), Some("5m"));
    }

    // ---------------------------------------------------------------
    // 31. merge_config inherits a perf-only project option even when no
    //     other counted option (and no flow options) is present —
    //     regression: perf fields were omitted from has_any_option, so the
    //     merged options were silently dropped to None.
    // ---------------------------------------------------------------
    #[test]
    fn merge_perf_only_project_option_inherited() {
        // 1. Project sets ONLY a perf threshold; flow has no options at all.
        let project = parse_project_config(
            r#"
[options]
perf_memory_error_mb = 512.0
"#,
        )
        .expect("project config SHALL parse");

        let flow = minimal_flow(
            r#"
[flow]
name = "test"
"#,
        );

        // 2. Merge SHALL retain the inherited perf threshold rather than None.
        let merged = merge_config(&project, &flow);
        let opts = merged
            .flow
            .options
            .expect("perf-only project option SHALL survive the merge");
        assert_eq!(
            opts.perf_memory_error_mb,
            Some(512.0),
            "SHALL preserve the inherited perf threshold"
        );
    }

    // ---------------------------------------------------------------
    // 32. merge_config preserves flow options even when project is empty
    // ---------------------------------------------------------------
    #[test]
    fn merge_empty_project_preserves_flow_options() {
        let project = ProjectConfig::default();
        let flow = minimal_flow(
            r#"
[flow]
name = "test"

[flow.options]
max_concurrency = 9
"#,
        );

        let merged = merge_config(&project, &flow);
        let opts = merged
            .flow
            .options
            .expect("flow-only options SHALL survive an empty-project merge");
        assert_eq!(opts.max_concurrency, Some(9));
    }
}
