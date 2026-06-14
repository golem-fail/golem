use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A parsed fixture file — contains only [vars], no blocks or steps
#[derive(Debug, Clone)]
pub struct Fixture {
    pub vars: HashMap<String, String>,
}

/// Internal representation for deserializing a fixture TOML file
#[derive(serde::Deserialize)]
struct FixtureFile {
    #[serde(default)]
    vars: HashMap<String, String>,
}

/// Parse a fixture TOML file. Fixture files contain only [vars].
pub fn parse_fixture(toml_str: &str) -> anyhow::Result<Fixture> {
    let fixture_file: FixtureFile = toml::from_str(toml_str)?;
    Ok(Fixture {
        vars: fixture_file.vars,
    })
}

/// Resolve a fixture name to a file path using __fixtures__/ directory convention.
/// Searches from flow_dir up to project_root, closest wins.
pub fn resolve_fixture_path(
    fixture_name: &str,
    flow_dir: &Path,
    project_root: &Path,
) -> anyhow::Result<PathBuf> {
    // Reject path traversal
    if fixture_name.contains("..") {
        anyhow::bail!("Fixture name cannot contain path traversal: {fixture_name}");
    }

    // Append .toml if not already present
    let file_name = if fixture_name.ends_with(".toml") {
        fixture_name.to_string()
    } else {
        format!("{fixture_name}.toml")
    };

    // Walk from flow_dir up to project_root (inclusive)
    let project_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut current = flow_dir
        .canonicalize()
        .unwrap_or_else(|_| flow_dir.to_path_buf());

    loop {
        let candidate = current.join("__fixtures__").join(&file_name);
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
        "Fixture '{}' not found in __fixtures__/ directories from {} to {}",
        fixture_name,
        flow_dir.display(),
        project_root.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: create a __fixtures__ directory and write a fixture file
    fn write_fixture(base_dir: &Path, name: &str, content: &str) {
        let fixture_dir = base_dir.join("__fixtures__");
        // name may contain subdirectories
        let file_path = fixture_dir.join(format!("{name}.toml"));
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create fixture directory");
        }
        fs::write(&file_path, content).expect("Failed to write fixture file");
    }

    // ---------------------------------------------------------------
    // 1. Fixture found in same directory
    // ---------------------------------------------------------------
    #[test]
    fn fixture_found_in_same_directory() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_fixture(flow_dir, "user", "[vars]\nemail = \"test@example.com\"\n");

        let result = resolve_fixture_path("user", flow_dir, project_root);
        assert!(result.is_ok(), "Should find fixture in same directory");
        let path = result.expect("fixture path SHALL resolve");
        assert!(path.ends_with("__fixtures__/user.toml"));
    }

    // ---------------------------------------------------------------
    // 2. Fixture found in parent directory
    // ---------------------------------------------------------------
    #[test]
    fn fixture_found_in_parent_directory() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let project_root = tmp.path();
        let flow_dir = project_root.join("flows").join("auth");
        fs::create_dir_all(&flow_dir).expect("Failed to create flow dir");

        write_fixture(
            &project_root.join("flows"),
            "user",
            "[vars]\nemail = \"test@example.com\"\n",
        );

        let result = resolve_fixture_path("user", &flow_dir, project_root);
        assert!(result.is_ok(), "Should find fixture in parent directory");
    }

    // ---------------------------------------------------------------
    // 3. Fixture found at project root
    // ---------------------------------------------------------------
    #[test]
    fn fixture_found_at_project_root() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let project_root = tmp.path();
        let flow_dir = project_root.join("flows").join("auth").join("deep");
        fs::create_dir_all(&flow_dir).expect("Failed to create flow dir");

        write_fixture(
            project_root,
            "user",
            "[vars]\nemail = \"test@example.com\"\n",
        );

        let result = resolve_fixture_path("user", &flow_dir, project_root);
        assert!(result.is_ok(), "Should find fixture at project root");
    }

    // ---------------------------------------------------------------
    // 4. Closest wins (override)
    // ---------------------------------------------------------------
    #[test]
    fn closest_wins_override() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let project_root = tmp.path();
        let flow_dir = project_root.join("flows").join("auth");
        fs::create_dir_all(&flow_dir).expect("Failed to create flow dir");

        // Write fixture at project root
        write_fixture(
            project_root,
            "user",
            "[vars]\nemail = \"root@example.com\"\n",
        );

        // Write fixture closer to flow_dir (in flows/)
        write_fixture(
            &project_root.join("flows"),
            "user",
            "[vars]\nemail = \"flows@example.com\"\n",
        );

        let result = resolve_fixture_path("user", &flow_dir, project_root);
        assert!(result.is_ok(), "Should find closest fixture");
        // The closest one is in flows/__fixtures__/user.toml
        let path = result.expect("fixture path SHALL resolve");
        assert!(
            path.to_string_lossy().contains("flows/__fixtures__/user.toml")
                || path.to_string_lossy().contains("flows\\__fixtures__\\user.toml"),
            "Should resolve to the closest fixture, got: {}",
            path.display()
        );
    }

    // ---------------------------------------------------------------
    // 5. Subfolder path
    // ---------------------------------------------------------------
    #[test]
    fn subfolder_path() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_fixture(
            flow_dir,
            "payments/stripe_card",
            "[vars]\ncard = \"4242424242424242\"\n",
        );

        let result = resolve_fixture_path("payments/stripe_card", flow_dir, project_root);
        assert!(result.is_ok(), "Should find fixture in subfolder");
        let path = result.expect("fixture path SHALL resolve");
        assert!(path.ends_with("__fixtures__/payments/stripe_card.toml"));
    }

    // ---------------------------------------------------------------
    // 6. Not found — error
    // ---------------------------------------------------------------
    #[test]
    fn not_found_error() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        let result = resolve_fixture_path("nonexistent", flow_dir, project_root);
        assert!(result.is_err(), "Should error when fixture not found");
        let err_msg = format!("{}", result.expect_err("should be an error"));
        assert!(
            err_msg.contains("not found"),
            "Error should mention not found, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 7. Stops at project root
    // ---------------------------------------------------------------
    #[test]
    fn stops_at_project_root() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let project_root = tmp.path().join("project");
        fs::create_dir_all(&project_root).expect("Failed to create project root");

        let flow_dir = project_root.join("flows");
        fs::create_dir_all(&flow_dir).expect("Failed to create flow dir");

        // Write fixture ABOVE the project root (should not be found)
        write_fixture(tmp.path(), "secret", "[vars]\nkey = \"value\"\n");

        let result = resolve_fixture_path("secret", &flow_dir, &project_root);
        assert!(
            result.is_err(),
            "Should not find fixture above project root"
        );
    }

    // ---------------------------------------------------------------
    // 8. Path traversal rejected
    // ---------------------------------------------------------------
    #[test]
    fn path_traversal_rejected() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        let result = resolve_fixture_path("../../../etc/passwd", flow_dir, project_root);
        assert!(result.is_err(), "Should reject path traversal");
        let err_msg = format!("{}", result.expect_err("should be an error"));
        assert!(
            err_msg.contains("path traversal"),
            "Error should mention path traversal, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 9. Extension not required in name
    // ---------------------------------------------------------------
    #[test]
    fn extension_not_required_in_name() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_fixture(flow_dir, "user", "[vars]\nemail = \"test@example.com\"\n");

        // Resolve without .toml extension
        let result = resolve_fixture_path("user", flow_dir, project_root);
        assert!(result.is_ok(), "Should find fixture without .toml extension");
    }

    // ---------------------------------------------------------------
    // 10. Extension in name still works
    // ---------------------------------------------------------------
    #[test]
    fn extension_in_name_still_works() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let flow_dir = tmp.path();
        let project_root = tmp.path();

        write_fixture(flow_dir, "user", "[vars]\nemail = \"test@example.com\"\n");

        // Resolve with .toml extension
        let result = resolve_fixture_path("user.toml", flow_dir, project_root);
        assert!(result.is_ok(), "Should find fixture with .toml extension");
    }

    // ---------------------------------------------------------------
    // 11. Parse fixture with vars
    // ---------------------------------------------------------------
    #[test]
    fn parse_fixture_with_vars() {
        let toml_str = r#"
[vars]
email = "fake:email(prefix=test,domain=acme-qa.com)"
password = "fake:password(length=12)"
first = "fake:first_name"
"#;
        let fixture = parse_fixture(toml_str).expect("Should parse fixture with vars");
        assert_eq!(fixture.vars.len(), 3);
        assert_eq!(
            fixture.vars.get("email").map(|s| s.as_str()),
            Some("fake:email(prefix=test,domain=acme-qa.com)")
        );
        assert_eq!(
            fixture.vars.get("password").map(|s| s.as_str()),
            Some("fake:password(length=12)")
        );
        assert_eq!(
            fixture.vars.get("first").map(|s| s.as_str()),
            Some("fake:first_name")
        );
    }

    // ---------------------------------------------------------------
    // 12. Parse fixture ignores non-vars content
    // ---------------------------------------------------------------
    #[test]
    fn parse_fixture_ignores_non_vars_content() {
        let toml_str = r#"
[vars]
email = "test@example.com"

[other_section]
key = "value"
"#;
        // Non-vars sections are silently ignored — only [vars] is extracted.
        let fixture = parse_fixture(toml_str).expect("Should parse, ignoring non-vars content");
        assert_eq!(fixture.vars.len(), 1);
        assert_eq!(
            fixture.vars.get("email").map(|s| s.as_str()),
            Some("test@example.com")
        );
    }

    // ---------------------------------------------------------------
    // 13. Empty vars section — valid
    // ---------------------------------------------------------------
    #[test]
    fn empty_vars_section_valid() {
        let toml_str = r#"
[vars]
"#;
        let fixture = parse_fixture(toml_str).expect("Empty vars section should be valid");
        assert!(fixture.vars.is_empty());
    }

    // ---------------------------------------------------------------
    // 14. No [vars] section at all — defaults to empty (serde default)
    // ---------------------------------------------------------------
    #[test]
    fn parse_fixture_no_vars_section_defaults_empty() {
        // A file with no [vars] table SHALL parse to an empty vars map via #[serde(default)].
        let fixture = parse_fixture("").expect("empty file SHALL parse to empty fixture");
        assert!(
            fixture.vars.is_empty(),
            "missing [vars] SHALL default to empty map"
        );
    }

    // ---------------------------------------------------------------
    // 16. Malformed TOML — propagates parse error
    // ---------------------------------------------------------------
    #[test]
    fn parse_fixture_malformed_toml_errors() {
        // Unterminated string is invalid TOML and SHALL bubble up as an error.
        let result = parse_fixture("[vars]\nemail = \"unterminated\n");
        assert!(result.is_err(), "malformed TOML SHALL produce an error");
    }

    // ---------------------------------------------------------------
    // 17. Wrong type for a var value — non-string rejected
    // ---------------------------------------------------------------
    #[test]
    fn parse_fixture_non_string_var_value_errors() {
        // vars is HashMap<String, String>; an integer value SHALL fail deserialization.
        let result = parse_fixture("[vars]\ncount = 5\n");
        assert!(
            result.is_err(),
            "non-string var value SHALL fail to deserialize"
        );
    }

    // ---------------------------------------------------------------
    // 18. Wrong type for [vars] itself — non-table rejected
    // ---------------------------------------------------------------
    #[test]
    fn parse_fixture_vars_not_a_table_errors() {
        // vars declared as a scalar instead of a table SHALL fail.
        let result = parse_fixture("vars = \"oops\"\n");
        assert!(
            result.is_err(),
            "scalar vars value SHALL fail to deserialize into a map"
        );
    }

    // ---------------------------------------------------------------
    // 19. Not-found error message names both flow_dir and project_root
    // ---------------------------------------------------------------
    #[test]
    fn not_found_error_mentions_search_bounds() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let project_root = tmp.path();
        let flow_dir = project_root.join("flows").join("auth");
        fs::create_dir_all(&flow_dir).expect("Failed to create flow dir");

        let result = resolve_fixture_path("missing", &flow_dir, project_root);
        let err_msg = format!("{}", result.expect_err("missing fixture SHALL error"));
        assert!(
            err_msg.contains("missing"),
            "error SHALL name the fixture, got: {err_msg}"
        );
        assert!(
            err_msg.contains("auth"),
            "error SHALL name the starting flow_dir, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 20. Path traversal rejected even with a bare ".." segment
    // ---------------------------------------------------------------
    #[test]
    fn path_traversal_rejected_bare_dotdot() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let dir = tmp.path();

        let result = resolve_fixture_path("..", dir, dir);
        let err_msg = format!("{}", result.expect_err("bare .. SHALL be rejected"));
        assert!(
            err_msg.contains("path traversal"),
            "bare .. SHALL be rejected as traversal, got: {err_msg}"
        );
    }

    // ---------------------------------------------------------------
    // 21. flow_dir below project_root that does not exist still errors
    //     cleanly (canonicalize falls back to the literal path)
    // ---------------------------------------------------------------
    #[test]
    fn nonexistent_dirs_resolve_to_not_found() {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let project_root = tmp.path().join("nope_root");
        let flow_dir = project_root.join("nope_flow");
        // Neither directory exists; canonicalize fails and falls back to literal paths.
        let result = resolve_fixture_path("user", &flow_dir, &project_root);
        assert!(
            result.is_err(),
            "absent dirs with no fixture SHALL yield not-found, not a panic"
        );
    }

    // ---------------------------------------------------------------
    // 22. parse_fixture performs no value interpretation — values with
    //     special chars (=, comma, parens) are stored verbatim. Guards
    //     against a future contributor adding value-transformation logic.
    // ---------------------------------------------------------------
    #[test]
    fn parse_fixture_stores_values_without_interpretation() {
        let toml_str = "[vars]\nraw = \"a=b,c=d (x)\"\n";
        let fixture = parse_fixture(toml_str).expect("SHALL parse value with special chars");
        assert_eq!(
            fixture.vars.get("raw").map(|s| s.as_str()),
            Some("a=b,c=d (x)"),
            "value SHALL be stored verbatim without interpretation"
        );
    }
}
