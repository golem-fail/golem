use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Default content for golem.toml.
const GOLEM_TOML_TEMPLATE: &str = r#"# GOLEM project configuration
# See documentation for all available options.
#
# Optional sections:
#   [options]              — defaults for flows (step_timeout, screenshot_on_failure, ...)
#   [vars]                 — project-level variables (referenced in flows as ${name})
#   [[apps]]               — app registry (bundle, install_script, install_timeout_ms, devices)
#   [device_settings]      — per-platform OS-level tweaks applied before flows run
#   [[teardown]]           — steps appended to every flow's teardown
#
# A flow always wins over the project for anything it states itself.
# See docs/test-structure.md §Project config.
#
# Run `golem install-script` to add an app and install script interactively.

# Per-platform device settings applied once per session before any
# flow runs. Useful for suppressing OS-level interruptions that bias
# test results — system pop-ups, first-run sheets, gesture tutorials.
# Idempotent across runs; survives emulator wipes (re-applied next
# `golem run`).
#
# [device_settings.android]
# # Suppress Android 14+ stylus handwriting overlay (otherwise a
# # slow tap on a focused input opens a full-screen handwriting
# # receiver that steals subsequent touches).
# "secure.stylus_handwriting_enabled" = "0"
# "secure.stylus_handwriting_default_value" = "0"
# # Suppress heads-up notifications during test runs.
# "global.heads_up_notifications_enabled" = "0"
#
# [device_settings.ios]
# # iOS `defaults write` — domain dots become underscores in the
# # TOML key (translated back before invoking `defaults write`).
# # "com_apple_springboard.SBHomeScreenHintWelcomeShown" = "true"
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

// ── Install-script scaffolding ─────────────────────────────────────

/// Supported framework templates for `golem install-script`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallFramework {
    NativeIos,
    NativeAndroid,
    Tauri,
    Expo,
}

impl InstallFramework {
    pub fn label(self) -> &'static str {
        match self {
            InstallFramework::NativeIos => "native-ios",
            InstallFramework::NativeAndroid => "native-android",
            InstallFramework::Tauri => "tauri",
            InstallFramework::Expo => "expo",
        }
    }

    fn template(self) -> &'static str {
        match self {
            InstallFramework::NativeIos => {
                include_str!("../templates/install-scripts/native-ios.sh")
            }
            InstallFramework::NativeAndroid => {
                include_str!("../templates/install-scripts/native-android.sh")
            }
            InstallFramework::Tauri => include_str!("../templates/install-scripts/tauri.sh"),
            InstallFramework::Expo => include_str!("../templates/install-scripts/expo.sh"),
        }
    }
}

fn install_partial(name: &str) -> Option<&'static str> {
    match name {
        "helpers" => Some(include_str!("../templates/install-scripts/_helpers.sh")),
        _ => None,
    }
}

/// Splice each `{{>name}}` include with the named partial's body.
///
/// Runs before placeholder substitution, so a placeholder value that happens
/// to contain `{{>…}}` stays literal. Partials can't include partials: one
/// level keeps every splice visible in the template that asks for it.
fn resolve_partials(template: &str) -> Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{>") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        let end = after
            .find("}}")
            .ok_or_else(|| anyhow::anyhow!("unterminated partial include `{{{{>`"))?;
        let name = after[..end].trim();
        let body = install_partial(name)
            .ok_or_else(|| anyhow::anyhow!("unknown install-script partial `{name}`"))?;
        if body.contains("{{>") {
            bail!("install-script partial `{name}` includes another partial");
        }
        out.push_str(body.trim_end_matches('\n'));
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Replace `{{PLACEHOLDER}}` tokens in a template with provided values.
fn render_template(template: &str, placeholders: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in placeholders {
        out = out.replace(&format!("{{{{{}}}}}", key), value);
    }
    out
}

/// Render `framework`'s install script: partials spliced, then placeholders
/// filled. The result is standalone bash.
pub fn render_install_script(
    framework: InstallFramework,
    placeholders: &[(&str, &str)],
) -> Result<String> {
    Ok(render_template(
        &resolve_partials(framework.template())?,
        placeholders,
    ))
}

/// Write a rendered install-script template to `output_path`. Creates
/// parent directories and sets the script executable on Unix.
pub fn write_install_script(
    output_path: &Path,
    framework: InstallFramework,
    placeholders: &[(&str, &str)],
) -> Result<()> {
    let rendered = render_install_script(framework, placeholders)?;

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }

    fs::write(output_path, rendered)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(output_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(output_path, perms)?;
    }

    Ok(())
}

/// Add or update an `[[apps]]` entry's `install_script` field in golem.toml,
/// preserving existing content/comments via toml_edit.
///
/// - `app_name`: matches `[[apps]] name = "..."` (created if not present)
/// - `bundle_id`: written on first creation; left alone on updates
/// - `script_relative_path`: path to the install script, relative to project root
/// - `platform`:
///   - `None` for cross-platform (Tauri, Expo) — sets `install_script` to a bare string
///   - `Some("ios")` / `Some("android")` for native — writes/merges an inline table
///     `install_script = { ios = "...", android = "..." }`.
pub fn update_golem_toml_install_script(
    golem_toml_path: &Path,
    app_name: &str,
    bundle_id: Option<&str>,
    script_relative_path: &str,
    platform: Option<&str>,
) -> Result<()> {
    let current = fs::read_to_string(golem_toml_path)
        .with_context(|| format!("read {}", golem_toml_path.display()))?;
    let mut doc: toml_edit::DocumentMut = current
        .parse()
        .with_context(|| format!("parse {}", golem_toml_path.display()))?;

    // Ensure `apps` is an array-of-tables.
    if !doc.contains_key("apps") {
        doc["apps"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    let apps = doc["apps"].as_array_of_tables_mut().ok_or_else(|| {
        anyhow::anyhow!(
            "`apps` in {} is not an array of tables",
            golem_toml_path.display()
        )
    })?;

    // Find existing entry by name, or append a new one.
    let idx = (0..apps.len()).find(|i| {
        apps.get(*i)
            .and_then(|t| t.get("name"))
            .and_then(|v| v.as_str())
            == Some(app_name)
    });

    let app_table = match idx {
        Some(i) => apps.get_mut(i).expect("existing entry"),
        None => {
            let mut t = toml_edit::Table::new();
            t["name"] = toml_edit::value(app_name);
            apps.push(t);
            apps.get_mut(apps.len() - 1).expect("just pushed")
        }
    };

    // Set bundle if provided and the entry doesn't already have one.
    // (Never overwrites a user-set bundle.)
    if let Some(b) = bundle_id {
        if !app_table.contains_key("bundle") {
            app_table["bundle"] = toml_edit::value(b);
        }
    }

    // Update install_script on that entry, merging platform keys.
    match platform {
        None => {
            app_table["install_script"] = toml_edit::value(script_relative_path);
        }
        Some(plat) => {
            let mut inline = toml_edit::InlineTable::new();
            if let Some(existing) = app_table.get("install_script") {
                if let Some(t) = existing.as_inline_table() {
                    for (k, v) in t.iter() {
                        if k != plat {
                            inline.insert(k, v.clone());
                        }
                    }
                }
            }
            inline.insert(plat, script_relative_path.into());
            app_table["install_script"] = toml_edit::value(inline);
        }
    }

    fs::write(golem_toml_path, doc.to_string())
        .with_context(|| format!("write {}", golem_toml_path.display()))?;
    Ok(())
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
            assert!(dir_path.is_dir(), "{dir_name} directory should exist");
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
        assert_eq!(
            content, custom_content,
            "golem.toml SHALL NOT be overwritten"
        );
    }

    // ---------------------------------------------------------------
    // 4. init is idempotent (running twice is safe)
    // ---------------------------------------------------------------
    #[test]
    fn init_is_idempotent() {
        let tmp = TempDir::new().expect("tempdir");

        init_project(tmp.path()).expect("first init");
        let content_after_first =
            fs::read_to_string(tmp.path().join("golem.toml")).expect("read golem.toml");

        init_project(tmp.path()).expect("second init");
        let content_after_second =
            fs::read_to_string(tmp.path().join("golem.toml")).expect("read golem.toml");

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
        assert!(
            content.contains("[[flow.apps]]"),
            "SHALL contain [[flow.apps]]"
        );
        assert!(content.contains("[[block]]"), "SHALL contain [[block]]");
        assert!(
            content.contains("action = \"launch\""),
            "SHALL contain launch action"
        );
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

    // ── install-script tests ───────────────────────────────────────

    #[test]
    fn render_template_replaces_placeholders() {
        let tmpl = "#!/bin/sh\necho {{BUNDLE_ID}} {{APP_NAME}}";
        let rendered = render_template(tmpl, &[("BUNDLE_ID", "com.x"), ("APP_NAME", "myapp")]);
        assert_eq!(rendered, "#!/bin/sh\necho com.x myapp");
    }

    #[test]
    fn write_install_script_writes_executable_file() {
        let tmp = TempDir::new().expect("tempdir");
        let out = tmp.path().join("scripts").join("install.sh");
        write_install_script(
            &out,
            InstallFramework::NativeAndroid,
            &[
                ("GRADLE_ROOT", "android"),
                ("MODULE_NAME", "app"),
                ("GRADLE_TASK", "installDebug"),
            ],
        )
        .expect("write");
        assert!(out.exists());
        let content = fs::read_to_string(&out).expect("read");
        assert!(content.contains("MODULE_NAME=\"app\""));
        assert!(content.contains("GRADLE_TASK=\"installDebug\""));
        assert!(content.contains("GRADLE_ROOT=\"android\""));
        assert!(!content.contains("{{"), "no placeholders remain");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&out)
                .expect("metadata() SHALL succeed")
                .permissions()
                .mode();
            assert_eq!(mode & 0o755, 0o755, "SHALL be executable: {:o}", mode);
        }
    }

    #[test]
    fn update_golem_toml_adds_app_entry_cross_platform() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("golem.toml");
        fs::write(&path, "[options]\n").expect("write() SHALL succeed");
        update_golem_toml_install_script(
            &path,
            "app-b",
            Some("com.example.b"),
            "scripts/install-b.sh",
            None,
        )
        .expect("update");
        let content = fs::read_to_string(&path).expect("read_to_string() SHALL succeed");
        assert!(content.contains("[[apps]]"));
        assert!(content.contains(r#"name = "app-b""#));
        assert!(content.contains(r#"bundle = "com.example.b""#));
        assert!(content.contains(r#"install_script = "scripts/install-b.sh""#));
    }

    #[test]
    fn update_golem_toml_updates_existing_app_entry() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("golem.toml");
        fs::write(
            &path,
            r#"
[[apps]]
name = "app-b"
bundle = "com.example.b"
install_script = "scripts/old.sh"
"#,
        )
        .expect("value SHALL be present");
        update_golem_toml_install_script(&path, "app-b", None, "scripts/new.sh", None)
            .expect("update");
        let content = fs::read_to_string(&path).expect("read_to_string() SHALL succeed");
        assert!(content.contains(r#"install_script = "scripts/new.sh""#));
        assert!(!content.contains("scripts/old.sh"));
        // bundle preserved
        assert!(content.contains(r#"bundle = "com.example.b""#));
    }

    #[test]
    fn update_golem_toml_backfills_missing_bundle_on_update() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("golem.toml");
        // Existing entry has no bundle — a second scaffold pass supplying one
        // SHALL fill it in rather than silently discard.
        fs::write(
            &path,
            r#"
[[apps]]
name = "app-b"
install_script = { ios = "scripts/ios.sh" }
"#,
        )
        .expect("value SHALL be present");
        update_golem_toml_install_script(
            &path,
            "app-b",
            Some("com.x"),
            "scripts/android.sh",
            Some("android"),
        )
        .expect("update");
        let content = fs::read_to_string(&path).expect("read_to_string() SHALL succeed");
        assert!(
            content.contains(r#"bundle = "com.x""#),
            "SHALL backfill missing bundle, got:\n{content}"
        );
    }

    #[test]
    fn update_golem_toml_preserves_existing_bundle() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("golem.toml");
        // A subsequent scaffold with a different bundle SHALL NOT overwrite.
        fs::write(
            &path,
            r#"
[[apps]]
name = "app-b"
bundle = "com.kept"
install_script = { ios = "scripts/ios.sh" }
"#,
        )
        .expect("value SHALL be present");
        update_golem_toml_install_script(
            &path,
            "app-b",
            Some("com.other"),
            "scripts/android.sh",
            Some("android"),
        )
        .expect("update");
        let content = fs::read_to_string(&path).expect("read_to_string() SHALL succeed");
        assert!(
            content.contains(r#"bundle = "com.kept""#),
            "SHALL preserve existing bundle, got:\n{content}"
        );
        assert!(
            !content.contains("com.other"),
            "SHALL NOT write the supplied bundle when one already exists"
        );
    }

    #[test]
    fn update_golem_toml_writes_per_platform_merges_keys() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("golem.toml");
        fs::write(&path, "[options]\n").expect("write() SHALL succeed");
        update_golem_toml_install_script(
            &path,
            "app-b",
            Some("com.example.b"),
            "scripts/ios.sh",
            Some("ios"),
        )
        .expect("update ios");
        update_golem_toml_install_script(
            &path,
            "app-b",
            Some("com.example.b"),
            "scripts/android.sh",
            Some("android"),
        )
        .expect("update android");
        let content = fs::read_to_string(&path).expect("read_to_string() SHALL succeed");
        assert!(content.contains("scripts/ios.sh"));
        assert!(content.contains("scripts/android.sh"));
        // Should have one [[apps]] entry (not two)
        assert_eq!(content.matches("[[apps]]").count(), 1);
    }

    // 13. label() returns the stable string for every framework variant.
    #[test]
    fn install_framework_label_all_variants() {
        assert_eq!(InstallFramework::NativeIos.label(), "native-ios");
        assert_eq!(InstallFramework::NativeAndroid.label(), "native-android");
        assert_eq!(InstallFramework::Tauri.label(), "tauri");
        assert_eq!(InstallFramework::Expo.label(), "expo");
    }

    // 14. render_template leaves text untouched when no placeholder matches.
    #[test]
    fn render_template_no_match_is_noop() {
        let tmpl = "echo {{NOT_PROVIDED}}";
        let rendered = render_template(tmpl, &[("OTHER", "x")]);
        assert_eq!(
            rendered, "echo {{NOT_PROVIDED}}",
            "unmatched placeholders SHALL be left in place"
        );
    }

    // 15. render_template replaces every occurrence of a repeated placeholder.
    #[test]
    fn render_template_replaces_all_occurrences() {
        let tmpl = "{{X}}-{{X}}-{{X}}";
        let rendered = render_template(tmpl, &[("X", "z")]);
        assert_eq!(
            rendered, "z-z-z",
            "all occurrences of a placeholder SHALL be replaced"
        );
    }

    // 16. render_template with an empty placeholder list returns the template verbatim.
    #[test]
    fn render_template_empty_placeholders_returns_template() {
        let tmpl = "literal {{KEEP}}";
        assert_eq!(render_template(tmpl, &[]), "literal {{KEEP}}");
    }

    // 17. write_install_script renders the Tauri template into an existing
    //     parent dir, substituting every Tauri-specific placeholder.
    #[test]
    fn write_install_script_renders_tauri_placeholders() {
        let tmp = TempDir::new().expect("tempdir");
        // Output sits directly in the tempdir, whose parent already exists —
        // create_dir_all is a no-op and the rendered content is what we verify.
        let out = tmp.path().join("install.sh");
        write_install_script(
            &out,
            InstallFramework::Tauri,
            &[
                ("TAURI_DIR", "./app"),
                ("IOS_SCHEME", "MyApp_iOS"),
                ("TAURI_CMD", "pnpm tauri"),
                ("PM_INSTALL", "pnpm install"),
            ],
        )
        .expect("write");

        let content = fs::read_to_string(&out).expect("read");
        // 17a. Each placeholder SHALL be substituted with its supplied value.
        assert!(
            content.contains(r#"TAURI_DIR="./app""#),
            "TAURI_DIR SHALL be substituted, got:\n{content}"
        );
        assert!(
            content.contains(r#"IOS_SCHEME="MyApp_iOS""#),
            "IOS_SCHEME SHALL be substituted, got:\n{content}"
        );
        assert!(
            content.contains(r#"TAURI_CMD="pnpm tauri""#),
            "TAURI_CMD SHALL be substituted, got:\n{content}"
        );
        // The Tauri template installs dependencies too, so it carries the
        // package manager as well as the CLI runner — the two are chosen
        // independently.
        assert!(
            content.contains(r#"PM_INSTALL="pnpm install""#),
            "PM_INSTALL SHALL be substituted, got:\n{content}"
        );
        // 17b. No placeholder tokens SHALL remain in the rendered Tauri script.
        assert!(
            !content.contains("{{"),
            "no placeholders SHALL remain, got:\n{content}"
        );
    }

    // 17c. write_install_script renders the Expo template, substituting every
    //      Expo-specific placeholder.
    #[test]
    fn write_install_script_renders_expo_placeholders() {
        let tmp = TempDir::new().expect("tempdir");
        let out = tmp.path().join("install.sh");
        write_install_script(
            &out,
            InstallFramework::Expo,
            &[
                ("EXPO_DIR", "./mobile"),
                ("PM_RUNNER", "pnpm expo"),
                ("PM_INSTALL", "pnpm install"),
                ("IOS_SCHEME", "MyApp"),
            ],
        )
        .expect("write");

        let content = fs::read_to_string(&out).expect("read");
        assert!(
            content.contains(r#"EXPO_DIR="./mobile""#),
            "EXPO_DIR SHALL be substituted, got:\n{content}"
        );
        assert!(
            content.contains(r#"PM_RUNNER="pnpm expo""#),
            "PM_RUNNER SHALL be substituted, got:\n{content}"
        );
        assert!(
            content.contains(r#"PM_INSTALL="pnpm install""#),
            "PM_INSTALL SHALL be substituted, got:\n{content}"
        );
        assert!(
            content.contains(r#"IOS_SCHEME="MyApp""#),
            "IOS_SCHEME SHALL be substituted, got:\n{content}"
        );
        assert!(
            !content.contains("{{"),
            "no placeholders SHALL remain, got:\n{content}"
        );
    }

    // 18. update on a missing golem.toml surfaces a read error (not a panic).
    #[test]
    fn update_golem_toml_missing_file_errors() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("does-not-exist.toml");
        let result =
            update_golem_toml_install_script(&path, "app", Some("com.x"), "scripts/x.sh", None);
        assert!(result.is_err(), "missing golem.toml SHALL error");
        let msg = format!("{}", result.expect_err("expected error"));
        assert!(
            msg.contains("read"),
            "error should mention the read step, got: {msg}"
        );
    }

    // 19. update on a malformed golem.toml surfaces a parse error.
    #[test]
    fn update_golem_toml_invalid_toml_errors() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("golem.toml");
        fs::write(&path, "this is = = not valid toml [[[").expect("write");
        let result =
            update_golem_toml_install_script(&path, "app", Some("com.x"), "scripts/x.sh", None);
        assert!(result.is_err(), "invalid TOML SHALL error");
        let msg = format!("{}", result.expect_err("expected error"));
        assert!(
            msg.contains("parse"),
            "error should mention the parse step, got: {msg}"
        );
    }

    // 20. update errors when `apps` exists but is not an array-of-tables.
    #[test]
    fn update_golem_toml_apps_wrong_type_errors() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("golem.toml");
        fs::write(&path, "apps = \"not-a-table-array\"\n").expect("write");
        let result =
            update_golem_toml_install_script(&path, "app", Some("com.x"), "scripts/x.sh", None);
        assert!(result.is_err(), "wrong `apps` type SHALL error");
        let msg = format!("{}", result.expect_err("expected error"));
        assert!(
            msg.contains("not an array of tables"),
            "error should explain the type mismatch, got: {msg}"
        );
    }

    // 21. re-writing the same platform key overwrites only that key, keeping the other.
    #[test]
    fn update_golem_toml_same_platform_overwrites_keeps_other() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("golem.toml");
        fs::write(
            &path,
            r#"
[[apps]]
name = "app-b"
install_script = { ios = "scripts/ios-old.sh", android = "scripts/android.sh" }
"#,
        )
        .expect("write");
        update_golem_toml_install_script(&path, "app-b", None, "scripts/ios-new.sh", Some("ios"))
            .expect("update ios");
        let content = fs::read_to_string(&path).expect("read");
        assert!(
            content.contains("scripts/ios-new.sh"),
            "ios key SHALL be updated"
        );
        assert!(
            !content.contains("scripts/ios-old.sh"),
            "old ios value SHALL be replaced"
        );
        assert!(
            content.contains("scripts/android.sh"),
            "android key SHALL be preserved"
        );
    }

    // 22. cross-platform (None) path backfills a missing bundle on an existing entry.
    #[test]
    fn update_golem_toml_cross_platform_backfills_bundle() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("golem.toml");
        fs::write(
            &path,
            r#"
[[apps]]
name = "app-b"
install_script = "scripts/old.sh"
"#,
        )
        .expect("write");
        update_golem_toml_install_script(
            &path,
            "app-b",
            Some("com.backfill"),
            "scripts/new.sh",
            None,
        )
        .expect("update");
        let content = fs::read_to_string(&path).expect("read");
        assert!(
            content.contains(r#"bundle = "com.backfill""#),
            "cross-platform update SHALL backfill missing bundle, got:\n{content}"
        );
        assert!(content.contains(r#"install_script = "scripts/new.sh""#));
    }

    // 23. flow_template embeds the given name verbatim into the [flow] name field.
    #[test]
    fn flow_template_embeds_name() {
        let rendered = flow_template("my-special_flow");
        assert!(
            rendered.contains(r#"name = "my-special_flow""#),
            "flow template SHALL embed the supplied name, got:\n{rendered}"
        );
    }

    #[test]
    fn all_templates_render_without_leftover_placeholders() {
        let tmp = TempDir::new().expect("tempdir");
        let placeholders = [
            ("XCODE_PROJECT", "X.xcodeproj"),
            ("XCODE_SCHEME", "X"),
            ("CONFIGURATION", "Debug"),
            ("GRADLE_ROOT", "android"),
            ("MODULE_NAME", "app"),
            ("GRADLE_TASK", "installDebug"),
            ("TAURI_DIR", "."),
            ("IOS_SCHEME", "X_iOS"),
            ("TAURI_CMD", "npx tauri"),
            ("EXPO_DIR", "."),
            ("PM_RUNNER", "npx expo"),
            ("PM_INSTALL", "npm install"),
        ];
        for fw in [
            InstallFramework::NativeIos,
            InstallFramework::NativeAndroid,
            InstallFramework::Tauri,
            InstallFramework::Expo,
        ] {
            let out = tmp.path().join(format!("{}.sh", fw.label()));
            write_install_script(&out, fw, &placeholders).expect("write");
            let content = fs::read_to_string(&out).expect("read");
            assert!(
                !content.contains("{{"),
                "{}: has leftover placeholder",
                fw.label()
            );
            let syntax = std::process::Command::new("bash")
                .arg("-n")
                .arg(&out)
                .output()
                .expect("bash runs");
            assert!(
                syntax.status.success(),
                "{}: rendered script SHALL be valid bash:\n{}",
                fw.label(),
                String::from_utf8_lossy(&syntax.stderr)
            );
        }
    }

    #[test]
    fn resolve_partials_splices_the_helpers_body_inline() {
        let out = resolve_partials("#!/usr/bin/env bash\nA=1\n{{>helpers}}\nmain\n").expect("ok");
        let helpers = install_partial("helpers").expect("helpers partial exists");
        assert!(
            out.contains(helpers.trim_end_matches('\n')),
            "the helpers body SHALL be spliced inline, got:\n{out}"
        );
        assert!(out.starts_with("#!/usr/bin/env bash\nA=1\n"));
        assert!(
            out.ends_with("\nmain\n"),
            "text after the include SHALL follow on its own line"
        );
        assert!(!out.contains("{{>"));
    }

    #[test]
    fn resolve_partials_rejects_an_unknown_partial() {
        let err = resolve_partials("{{>nope}}\n").expect_err("unknown partial");
        assert!(
            err.to_string().contains("nope"),
            "the error SHALL name the partial, got: {err}"
        );
    }

    #[test]
    fn resolve_partials_rejects_an_unterminated_include() {
        assert!(resolve_partials("{{>helpers\n").is_err());
    }

    #[test]
    fn partials_resolve_before_placeholders() {
        let out = render_template(
            &resolve_partials("X={{X}}\n").expect("ok"),
            &[("X", "{{>helpers}}")],
        );
        assert_eq!(
            out, "X={{>helpers}}\n",
            "a placeholder value SHALL never be read as an include"
        );
    }

    #[test]
    fn every_template_includes_the_helpers_partial() {
        for fw in [
            InstallFramework::NativeIos,
            InstallFramework::NativeAndroid,
            InstallFramework::Tauri,
            InstallFramework::Expo,
        ] {
            let out = render_install_script(fw, &[]).expect("render");
            assert!(
                out.contains("golem_pick_apk()") && out.contains("golem_mtime()"),
                "{}: rendered script SHALL carry the helpers inline",
                fw.label()
            );
            assert!(!out.contains("{{>"), "{}: unresolved include", fw.label());
        }
    }

    /// Runs `body` in bash under the templates' strict mode, with the
    /// helpers partial defined and `dir` as cwd. Returns stdout.
    fn run_helpers(dir: &Path, body: &str) -> String {
        let script = format!(
            "set -euo pipefail\n{}\n{body}\n",
            install_partial("helpers").expect("helpers")
        );
        let out = std::process::Command::new("bash")
            .arg("-c")
            .arg(script)
            .current_dir(dir)
            .output()
            .expect("bash runs");
        assert!(
            out.status.success(),
            "helpers script failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn touch_aged(path: &Path, secs_ago: u64) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        let f = fs::File::create(path).expect("create");
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago);
        f.set_modified(t).expect("set mtime");
    }

    #[test]
    fn pick_apk_takes_the_newest_installable_apk() {
        let tmp = TempDir::new().expect("tempdir");
        let apk = tmp.path().join("apk");
        touch_aged(&apk.join("debug/app-debug.apk"), 300);
        touch_aged(&apk.join("release/app-release.apk"), 200);
        touch_aged(&apk.join("release/app-release-unsigned.apk"), 10);
        touch_aged(&apk.join("androidTest/debug/app-debug-androidTest.apk"), 10);
        assert_eq!(
            run_helpers(tmp.path(), "golem_pick_apk apk"),
            "apk/release/app-release.apk",
            "test and unsigned APKs SHALL never be picked, however new"
        );
        assert_eq!(
            run_helpers(tmp.path(), "golem_pick_apk apk debug"),
            "apk/debug/app-debug.apk",
            "a variant SHALL narrow the pick to that variant's directory"
        );
    }

    #[test]
    fn pick_app_takes_the_newest_bundle_within_depth() {
        let tmp = TempDir::new().expect("tempdir");
        fs::create_dir_all(tmp.path().join("p/Old.app")).expect("mkdir");
        fs::create_dir_all(tmp.path().join("p/New.app")).expect("mkdir");
        fs::create_dir_all(tmp.path().join("p/deep/Deeper.app")).expect("mkdir");
        for (dir, age) in [
            ("p/Old.app", 300),
            ("p/New.app", 100),
            ("p/deep/Deeper.app", 1),
        ] {
            let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age);
            fs::File::open(tmp.path().join(dir))
                .expect("open dir")
                .set_modified(t)
                .expect("set mtime");
        }
        assert_eq!(run_helpers(tmp.path(), "golem_pick_app p"), "p/New.app");
        assert_eq!(
            run_helpers(tmp.path(), "golem_pick_app p 2"),
            "p/deep/Deeper.app"
        );
    }

    #[test]
    fn pick_app_never_returns_a_bundle_nested_in_another() {
        let tmp = TempDir::new().expect("tempdir");
        fs::create_dir_all(tmp.path().join("p/Outer.app/Watch/Inner.app")).expect("mkdir");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(300);
        fs::File::open(tmp.path().join("p/Outer.app"))
            .expect("open dir")
            .set_modified(old)
            .expect("set mtime");
        assert_eq!(run_helpers(tmp.path(), "golem_pick_app p 3"), "p/Outer.app");
    }

    #[test]
    fn require_fresh_fails_only_for_an_artifact_older_than_the_build() {
        let tmp = TempDir::new().expect("tempdir");
        touch_aged(&tmp.path().join("old.apk"), 300);
        touch_aged(&tmp.path().join("new.apk"), 0);
        let out = run_helpers(
            tmp.path(),
            "start=$(( $(date +%s) - 60 ))\n\
             golem_require_fresh new.apk \"$start\" && echo new-ok\n\
             golem_require_fresh old.apk \"$start\" 2>/dev/null || echo old-rejected",
        );
        assert_eq!(out, "new-ok\nold-rejected\n");
    }

    #[test]
    fn picks_from_a_missing_dir_are_empty_not_fatal() {
        let tmp = TempDir::new().expect("tempdir");
        assert_eq!(
            run_helpers(
                tmp.path(),
                "A=$(golem_pick_apk nope); B=$(golem_pick_app nope); echo \"[$A$B]\""
            ),
            "[]\n",
            "an assignment from a missing root SHALL survive set -e with an empty value"
        );
    }
}
