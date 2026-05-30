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
}
