//! Project-level configuration loaded from `golem.toml`.
//!
//! Walks up from the current directory looking for a `golem.toml` file.
//! Missing file is not an error — sections default to empty.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use golem_parser::ProjectAppConfig;
use serde::Deserialize;

/// Full `golem.toml` schema (parsed on demand; sections are optional).
#[derive(Deserialize, Debug, Default, Clone)]
pub struct ProjectConfig {
    /// App definitions — registry of known apps. Flows reference by name
    /// and inherit bundle/install_script/install_timeout_ms/devices.
    #[serde(default)]
    pub apps: Vec<ProjectAppConfig>,
    /// Per-platform OS-level tweaks applied once per device session
    /// before any flow runs. Lets a project pin emulator/sim state
    /// (suppress system pop-ups, first-run sheets, gesture
    /// tutorials) so tests aren't perturbed by defaults that change
    /// across wipes.
    #[serde(default)]
    pub device_settings: DeviceSettings,
    /// Project-wide defaults from `[options]`. CLI defines the full
    /// option surface in golem-parser; here we only pull the fields
    /// the CLI itself consumes (today: recording cascade).
    #[serde(default)]
    pub options: ProjectOptions,
}

/// Subset of `golem.toml` `[options]` consumed by the CLI.
#[derive(Deserialize, Debug, Default, Clone)]
pub struct ProjectOptions {
    /// Project-wide default for per-block screen recording. Loses
    /// to flow-level, block-level, and CLI flags.
    #[serde(default)]
    pub record: Option<bool>,
}

/// Per-platform device settings, keyed by namespace.
#[derive(Deserialize, Debug, Default, Clone)]
pub struct DeviceSettings {
    /// Android `settings put <ns> <key> <value>` keys, formatted as
    /// `"<namespace>.<key>" = "<value>"`. Namespaces: system,
    /// secure, global.
    #[serde(default)]
    pub android: std::collections::HashMap<String, String>,
    /// iOS `defaults write <domain> <key> <value>` keys. The domain
    /// goes in the TOML key with dots-replaced-by-underscores
    /// (translated back before invoking `defaults`).
    #[serde(default)]
    pub ios: std::collections::HashMap<String, String>,
}

impl ProjectConfig {
    /// Discover and load `golem.toml` from the given directory, walking up
    /// the filesystem tree. Returns default config if no file is found.
    pub fn load_from(dir: &Path) -> Result<(Self, Option<PathBuf>)> {
        let path = match find_project_root(dir) {
            Some(root) => root.join("golem.toml"),
            None => return Ok((Self::default(), None)),
        };
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: ProjectConfig = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok((config, Some(path)))
    }
}

/// Walk up from `start` looking for a directory containing `golem.toml`.
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.canonicalize().ok().or_else(|| Some(start.to_path_buf()))?;
    loop {
        if current.join("golem.toml").is_file() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_returns_default() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (cfg, path) = ProjectConfig::load_from(tmp.path()).expect("load");
        assert!(path.is_none(), "no golem.toml → no path");
        assert!(cfg.apps.is_empty());
    }

    #[test]
    fn load_parses_apps_registry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("golem.toml"),
            r#"
[[apps]]
name = "app-b"
bundle = "com.example.b"
install_script = { ios = "scripts/install-b-ios.sh", android = "scripts/install-b-android.sh" }
install_timeout_ms = 900000
"#,
        ).unwrap();
        let (cfg, path) = ProjectConfig::load_from(tmp.path()).expect("load");
        assert!(path.is_some());
        assert_eq!(cfg.apps.len(), 1);
        assert_eq!(cfg.apps[0].name, "app-b");
        assert_eq!(cfg.apps[0].bundle.as_deref(), Some("com.example.b"));
        assert_eq!(cfg.apps[0].install_timeout_ms, Some(900000));
        assert_eq!(
            cfg.apps[0].install_script.as_ref().and_then(|v| v.for_platform("ios")),
            Some("scripts/install-b-ios.sh")
        );
    }

    #[test]
    fn load_walks_up_to_find_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let nested = root.join("flows").join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            root.join("golem.toml"),
            r#"[[apps]]
name = "x"
bundle = "com.x"
"#,
        ).unwrap();
        let (cfg, path) = ProjectConfig::load_from(&nested).expect("load");
        assert!(path.is_some(), "SHALL find golem.toml in ancestor");
        assert_eq!(cfg.apps.len(), 1);
    }

    // 4. Malformed TOML SHALL surface a parse error (not a silent default),
    //    and the error context SHALL name the offending file path.
    #[test]
    fn load_invalid_toml_errors_with_path_context() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("golem.toml");
        std::fs::write(&path, "this is = = not valid toml").expect("write");
        let err = ProjectConfig::load_from(tmp.path()).expect_err("parse SHALL fail");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("failed to parse"),
            "error SHALL mention parse failure, got: {msg}"
        );
        assert!(
            msg.contains("golem.toml"),
            "error SHALL name the offending file, got: {msg}"
        );
    }

    // 5. An empty golem.toml SHALL load as the default config (all sections
    //    empty) while still reporting the discovered path.
    #[test]
    fn load_empty_file_yields_defaults_with_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("golem.toml"), "").expect("write");
        let (cfg, path) = ProjectConfig::load_from(tmp.path()).expect("load");
        assert!(path.is_some(), "discovered file SHALL report its path");
        assert!(cfg.apps.is_empty(), "default apps SHALL be empty");
        assert!(cfg.options.record.is_none(), "default record SHALL be None");
        assert!(
            cfg.device_settings.android.is_empty(),
            "default android settings SHALL be empty"
        );
        assert!(
            cfg.device_settings.ios.is_empty(),
            "default ios settings SHALL be empty"
        );
    }

    // 6. The [options] section's `record` flag SHALL parse into
    //    ProjectOptions::record.
    #[test]
    fn load_parses_options_record() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("golem.toml"),
            "[options]\nrecord = true\n",
        )
        .expect("write");
        let (cfg, _) = ProjectConfig::load_from(tmp.path()).expect("load");
        assert_eq!(
            cfg.options.record,
            Some(true),
            "record = true SHALL parse to Some(true)"
        );
    }

    // 7. The [device_settings] section SHALL populate per-platform key/value
    //    maps for both android and ios namespaces.
    #[test]
    fn load_parses_device_settings_both_platforms() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("golem.toml"),
            r#"
[device_settings.android]
"system.show_touches" = "1"
"global.window_animation_scale" = "0"

[device_settings.ios]
"com_apple_keyboard.KeyboardAutocorrection" = "0"
"#,
        )
        .expect("write");
        let (cfg, _) = ProjectConfig::load_from(tmp.path()).expect("load");
        assert_eq!(
            cfg.device_settings.android.get("system.show_touches"),
            Some(&"1".to_string()),
            "android key SHALL parse"
        );
        assert_eq!(
            cfg.device_settings.android.get("global.window_animation_scale"),
            Some(&"0".to_string()),
            "second android key SHALL parse"
        );
        assert_eq!(
            cfg.device_settings.ios.get("com_apple_keyboard.KeyboardAutocorrection"),
            Some(&"0".to_string()),
            "ios key SHALL parse"
        );
    }

    // 8. install_script with an android entry SHALL resolve via for_platform,
    //    covering the non-ios branch of the platform map.
    #[test]
    fn load_install_script_resolves_android_platform() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("golem.toml"),
            r#"
[[apps]]
name = "a"
bundle = "com.a"
install_script = { ios = "i.sh", android = "a.sh" }
"#,
        )
        .expect("write");
        let (cfg, _) = ProjectConfig::load_from(tmp.path()).expect("load");
        assert_eq!(
            cfg.apps[0]
                .install_script
                .as_ref()
                .and_then(|v| v.for_platform("android")),
            Some("a.sh"),
            "android install_script SHALL resolve"
        );
    }

    // 9. find_project_root SHALL return None when no ancestor (up to the
    //    filesystem root) contains a golem.toml. A freshly created tempdir
    //    tree has no golem.toml in it; using its real (canonicalized) path
    //    exercises the walk-to-root-then-None path.
    #[test]
    fn find_project_root_none_when_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let nested = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&nested).expect("mkdir");
        // No golem.toml anywhere under tmp, and the system temp dir's
        // ancestors (/var/folders/... on macOS, /tmp/... on Linux) carry
        // none either, so the walk SHALL terminate at the FS root with None.
        let found = find_project_root(&nested);
        assert!(
            found.is_none(),
            "no golem.toml from tmp up to FS root → SHALL be None, got {found:?}"
        );
    }

    // 10. find_project_root SHALL return the directory itself when it
    //     directly contains golem.toml (no walking required).
    #[test]
    fn find_project_root_self_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("golem.toml"), "").expect("write");
        let found = find_project_root(tmp.path()).expect("SHALL find root");
        let canon = tmp.path().canonicalize().expect("canonicalize");
        assert_eq!(found, canon, "root SHALL be the canonicalized self dir");
    }

    // 11. find_project_root SHALL tolerate a non-existent start path
    //     (canonicalize fails) by falling back to the literal path and
    //     walking up its lexical ancestors.
    #[test]
    fn find_project_root_nonexistent_start_does_not_panic() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bogus = tmp.path().join("does").join("not").join("exist");
        // canonicalize() fails on the non-existent path, so the fallback at
        // line 75 keeps the literal path and walks its lexical ancestors.
        // No golem.toml exists anywhere up to the FS root, so the walk SHALL
        // terminate with None (and SHALL NOT panic).
        let found = find_project_root(&bogus);
        assert!(
            found.is_none(),
            "non-existent start with no golem.toml ancestor → SHALL be None, got {found:?}"
        );
    }

    // 12. A nested golem.toml SHALL win over a higher ancestor: the walk
    //     stops at the *closest* directory containing the file.
    #[test]
    fn find_project_root_stops_at_closest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let outer = tmp.path();
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).expect("mkdir");
        std::fs::write(outer.join("golem.toml"), "").expect("write outer");
        std::fs::write(inner.join("golem.toml"), "").expect("write inner");
        let found = find_project_root(&inner).expect("SHALL find root");
        assert_eq!(
            found,
            inner.canonicalize().expect("canonicalize"),
            "closest ancestor with golem.toml SHALL win"
        );
    }
}
