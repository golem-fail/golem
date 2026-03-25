use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Default content for golem.toml.
const GOLEM_TOML_TEMPLATE: &str = r#"# GOLEM project configuration
# See documentation for all available options

[options]
# step_timeout = 10000
# screenshot_on_failure = true

[vars]
# api_token = "your-token-here"
"#;

/// Generate the content for a new flow file template.
fn flow_template(name: &str) -> String {
    format!(
        r#"[flow]
name = "{name}"
# tags = ["smoke"]

[[flow.apps]]
name = "app"
bundle = "com.example.app"

# Uncomment to specify device requirements:
# [[flow.devices]]
# os = "ios:latest"
# type = "phone"

[[block]]
name = "main"
steps = [
  {{ action = "launch" }},
  # Add your test steps here
  # {{ action = "tap", text = "Button" }},
  # {{ action = "assert_visible", text = "Expected Text" }},
]
"#
    )
}

/// Standard directories created by `golem init`.
const PROJECT_DIRS: &[&str] = &["flows", "__fixtures__", "__mixins__", ".golem"];

/// Initialize a new GOLEM project in the given directory.
///
/// Creates `golem.toml`, `flows/`, `__fixtures__/`, `__mixins__/`, and `.golem/`.
/// Existing files and directories are skipped (idempotent).
pub fn init_project(dir: &Path) -> Result<()> {
    // Create the project root if it doesn't exist
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create project directory: {}", dir.display()))?;

    // Create golem.toml (skip if it already exists)
    let config_path = dir.join("golem.toml");
    if !config_path.exists() {
        fs::write(&config_path, GOLEM_TOML_TEMPLATE)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
    }

    // Create standard directories
    for dir_name in PROJECT_DIRS {
        let path = dir.join(dir_name);
        if !path.exists() {
            fs::create_dir_all(&path)
                .with_context(|| format!("failed to create directory: {}", path.display()))?;
        }
    }

    Ok(())
}

/// Create a new flow file template.
///
/// If a `flows/` directory exists under `dir`, the file is created there;
/// otherwise it is created directly in `dir`. The file name is `<name>.test.toml`.
///
/// Returns the path of the created file. Errors if the file already exists.
pub fn create_flow(name: &str, dir: &Path) -> Result<PathBuf> {
    let file_name = format!("{name}.test.toml");

    let flows_dir = dir.join("flows");
    let target_dir = if flows_dir.is_dir() {
        flows_dir
    } else {
        dir.to_path_buf()
    };

    let file_path = target_dir.join(&file_name);

    if file_path.exists() {
        bail!("flow file already exists: {}", file_path.display());
    }

    fs::write(&file_path, flow_template(name))
        .with_context(|| format!("failed to write flow file: {}", file_path.display()))?;

    Ok(file_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ---------------------------------------------------------------
    // 1. init creates golem.toml
    // ---------------------------------------------------------------
    #[test]
    fn init_creates_golem_toml() {
        let tmp = TempDir::new().expect("tempdir");
        init_project(tmp.path()).expect("init");

        let config_path = tmp.path().join("golem.toml");
        assert!(config_path.exists(), "golem.toml SHALL exist");

        let content = fs::read_to_string(&config_path).expect("read golem.toml");
        assert!(content.contains("[options]"));
        assert!(content.contains("[vars]"));
    }

    // ---------------------------------------------------------------
    // 2. init creates all directories
    // ---------------------------------------------------------------
    #[test]
    fn init_creates_all_directories() {
        let tmp = TempDir::new().expect("tempdir");
        init_project(tmp.path()).expect("init");

        for dir_name in &["flows", "__fixtures__", "__mixins__", ".golem"] {
            let dir_path = tmp.path().join(dir_name);
            assert!(
                dir_path.is_dir(),
                "{dir_name} directory should exist"
            );
        }
    }

    // ---------------------------------------------------------------
    // 3. init doesn't overwrite existing golem.toml
    // ---------------------------------------------------------------
    #[test]
    fn init_does_not_overwrite_existing_golem_toml() {
        let tmp = TempDir::new().expect("tempdir");
        let config_path = tmp.path().join("golem.toml");

        let custom_content = "# custom config\n[options]\nstep_timeout = 5000\n";
        fs::write(&config_path, custom_content).expect("write custom config");

        init_project(tmp.path()).expect("init");

        let content = fs::read_to_string(&config_path).expect("read golem.toml");
        assert_eq!(content, custom_content, "golem.toml SHALL NOT be overwritten");
    }

    // ---------------------------------------------------------------
    // 4. init is idempotent (running twice is safe)
    // ---------------------------------------------------------------
    #[test]
    fn init_is_idempotent() {
        let tmp = TempDir::new().expect("tempdir");

        init_project(tmp.path()).expect("first init");
        let content_after_first = fs::read_to_string(tmp.path().join("golem.toml"))
            .expect("read golem.toml");

        init_project(tmp.path()).expect("second init");
        let content_after_second = fs::read_to_string(tmp.path().join("golem.toml"))
            .expect("read golem.toml");

        assert_eq!(content_after_first, content_after_second);

        for dir_name in PROJECT_DIRS {
            assert!(tmp.path().join(dir_name).is_dir());
        }
    }

    // ---------------------------------------------------------------
    // 5. create makes a valid .test.toml file
    // ---------------------------------------------------------------
    #[test]
    fn create_makes_test_toml_file() {
        let tmp = TempDir::new().expect("tempdir");
        let path = create_flow("login", tmp.path()).expect("create");

        assert!(path.exists(), "flow file SHALL exist");
        assert!(
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == "login.test.toml"),
            "file should be named login.test.toml"
        );

        let content = fs::read_to_string(&path).expect("read flow file");
        assert!(content.contains("name = \"login\""));
    }

    // ---------------------------------------------------------------
    // 6. create puts file in flows/ if it exists
    // ---------------------------------------------------------------
    #[test]
    fn create_puts_file_in_flows_dir() {
        let tmp = TempDir::new().expect("tempdir");
        fs::create_dir(tmp.path().join("flows")).expect("create flows dir");

        let path = create_flow("checkout", tmp.path()).expect("create");

        assert!(
            path.starts_with(tmp.path().join("flows")),
            "flow file should be in flows/ directory"
        );
        assert!(path.exists());
    }

    // ---------------------------------------------------------------
    // 7. create puts file in current dir if no flows/
    // ---------------------------------------------------------------
    #[test]
    fn create_puts_file_in_current_dir_without_flows() {
        let tmp = TempDir::new().expect("tempdir");

        let path = create_flow("signup", tmp.path()).expect("create");

        assert_eq!(
            path.parent().expect("parent"),
            tmp.path(),
            "flow file should be directly in the given directory"
        );
        assert!(path.exists());
    }

    // ---------------------------------------------------------------
    // 8. create errors if file already exists
    // ---------------------------------------------------------------
    #[test]
    fn create_errors_if_file_already_exists() {
        let tmp = TempDir::new().expect("tempdir");

        create_flow("duplicate", tmp.path()).expect("first create");
        let result = create_flow("duplicate", tmp.path());

        assert!(result.is_err(), "SHALL error on duplicate");
        let err_msg = format!("{}", result.expect_err("expected error"));
        assert!(
            err_msg.contains("already exists"),
            "error should mention 'already exists', got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 9. created flow file is valid TOML (parseable)
    // ---------------------------------------------------------------
    #[test]
    fn created_flow_file_is_valid_toml() {
        let tmp = TempDir::new().expect("tempdir");
        let path = create_flow("my_flow", tmp.path()).expect("create");

        let content = fs::read_to_string(&path).expect("read flow file");
        let parsed: Result<toml::Value, _> = toml::from_str(&content);
        assert!(
            parsed.is_ok(),
            "flow file should be valid TOML, parse error: {:?}",
            parsed.err()
        );
    }

    // ---------------------------------------------------------------
    // 10. golem.toml template is valid TOML
    // ---------------------------------------------------------------
    #[test]
    fn golem_toml_template_is_valid_toml() {
        let parsed: Result<toml::Value, _> = toml::from_str(GOLEM_TOML_TEMPLATE);
        assert!(
            parsed.is_ok(),
            "golem.toml template should be valid TOML, parse error: {:?}",
            parsed.err()
        );
    }

    // ---------------------------------------------------------------
    // 11. create flow includes expected sections
    // ---------------------------------------------------------------
    #[test]
    fn create_flow_includes_expected_sections() {
        let tmp = TempDir::new().expect("tempdir");
        let path = create_flow("full_check", tmp.path()).expect("create");

        let content = fs::read_to_string(&path).expect("read flow file");
        assert!(content.contains("[flow]"), "SHALL contain [flow] section");
        assert!(content.contains("[[flow.apps]]"), "SHALL contain [[flow.apps]]");
        assert!(content.contains("[[block]]"), "SHALL contain [[block]]");
        assert!(content.contains("action = \"launch\""), "SHALL contain launch action");
    }

    // ---------------------------------------------------------------
    // 12. init then create uses flows/ directory
    // ---------------------------------------------------------------
    #[test]
    fn init_then_create_uses_flows_dir() {
        let tmp = TempDir::new().expect("tempdir");

        init_project(tmp.path()).expect("init");
        let path = create_flow("after_init", tmp.path()).expect("create");

        assert!(
            path.starts_with(tmp.path().join("flows")),
            "after init, create should put files in flows/"
        );
    }
}
