//! Interactive `golem install-script` subcommand — scaffolds a bash
//! install script for a supported framework.
//!
//! Uses `dialoguer` for arrow-key selection, default-valued text input,
//! and y/n confirmation prompts. Non-TTY stdin falls back to line reader.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Select};

use crate::scaffold::{update_golem_toml_install_script, write_install_script, InstallFramework};

const OTHER_LABEL: &str = "Other (type manually)";

/// Entry point for `golem install-script`.
pub fn run() -> Result<()> {
    let theme = ColorfulTheme::default();

    let frameworks = [
        (
            "native-ios    (Xcode / xcodebuild)",
            InstallFramework::NativeIos,
        ),
        (
            "native-android (Gradle / adb)",
            InstallFramework::NativeAndroid,
        ),
        ("tauri          (Tauri 2.x mobile)", InstallFramework::Tauri),
    ];
    let idx = Select::with_theme(&theme)
        .with_prompt("Framework")
        .items(&frameworks.iter().map(|(l, _)| *l).collect::<Vec<_>>())
        .default(0)
        .interact()
        .context("framework selection cancelled")?;
    let framework = frameworks[idx].1;

    // app_name is used for: default output filename, iOS scheme default,
    // and as the name key when we scaffold an [[apps]] entry in golem.toml.
    // Bundle id is asked later, only if the user opts into updating golem.toml.
    let app_name: String = Input::with_theme(&theme)
        .with_prompt("App name (matches [[apps]] and [[flow.apps]] name)")
        .default("app".into())
        .interact_text()?;

    let mut placeholders: Vec<(&'static str, String)> = Vec::new();

    match framework {
        InstallFramework::NativeIos => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let found = discover_xcode_projects(&cwd, 5);
            let project = if found.is_empty() {
                Input::with_theme(&theme)
                    .with_prompt("Xcode project or workspace path (e.g. MyApp.xcodeproj)")
                    .validate_with(|s: &String| -> std::result::Result<(), &str> {
                        if s.trim().is_empty() {
                            Err("required")
                        } else {
                            Ok(())
                        }
                    })
                    .interact_text()?
            } else {
                let mut items: Vec<String> = found
                    .iter()
                    .map(|p| {
                        p.strip_prefix(&cwd)
                            .unwrap_or(p)
                            .to_string_lossy()
                            .to_string()
                    })
                    .collect();
                items.push(OTHER_LABEL.into());
                let idx = Select::with_theme(&theme)
                    .with_prompt("Xcode project or workspace")
                    .items(&items)
                    .default(0)
                    .interact()?;
                if idx == items.len() - 1 {
                    Input::with_theme(&theme)
                        .with_prompt("Enter path")
                        .interact_text()?
                } else {
                    items[idx].clone()
                }
            };

            let schemes = discover_xcode_schemes(&project);
            let scheme = if schemes.is_empty() {
                Input::with_theme(&theme)
                    .with_prompt("Xcode scheme")
                    .default(app_name.clone())
                    .interact_text()?
            } else if schemes.len() == 1 {
                println!("  using scheme: {}", schemes[0]);
                schemes[0].clone()
            } else {
                let mut items = schemes.clone();
                items.push(OTHER_LABEL.into());
                let idx = Select::with_theme(&theme)
                    .with_prompt("Xcode scheme")
                    .items(&items)
                    .default(0)
                    .interact()?;
                if idx == items.len() - 1 {
                    Input::with_theme(&theme)
                        .with_prompt("Enter scheme")
                        .interact_text()?
                } else {
                    items[idx].clone()
                }
            };
            let config_items = ["Debug", "Release"];
            let config_idx = Select::with_theme(&theme)
                .with_prompt("Configuration")
                .items(&config_items)
                .default(0)
                .interact()?;
            placeholders.push(("XCODE_PROJECT", project));
            placeholders.push(("XCODE_SCHEME", scheme));
            placeholders.push(("CONFIGURATION", config_items[config_idx].into()));
        }
        InstallFramework::NativeAndroid => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let found = discover_android_roots(&cwd, 5);
            let gradle_root = if found.is_empty() {
                Input::with_theme(&theme)
                    .with_prompt("Gradle project root (directory with settings.gradle)")
                    .default(".".into())
                    .interact_text()?
            } else {
                let mut items = found.clone();
                items.push(OTHER_LABEL.into());
                let idx = Select::with_theme(&theme)
                    .with_prompt("Gradle project root")
                    .items(&items)
                    .default(0)
                    .interact()?;
                if idx == items.len() - 1 {
                    Input::with_theme(&theme)
                        .with_prompt("Enter gradle root path")
                        .interact_text()?
                } else {
                    items[idx].clone()
                }
            };
            let module: String = Input::with_theme(&theme)
                .with_prompt("Module name (gradle submodule, e.g. 'app')")
                .default("app".into())
                .interact_text()?;
            let task: String = Input::with_theme(&theme)
                .with_prompt("Gradle task")
                .default("installDebug".into())
                .interact_text()?;
            placeholders.push(("GRADLE_ROOT", gradle_root));
            placeholders.push(("MODULE_NAME", module));
            placeholders.push(("GRADLE_TASK", task));
        }
        InstallFramework::Tauri => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let found = discover_tauri_dirs(&cwd, 5);
            let tauri_dir = if found.is_empty() {
                Input::with_theme(&theme)
                    .with_prompt("Tauri project directory (contains src-tauri/)")
                    .default(".".into())
                    .interact_text()?
            } else {
                let mut items = found.clone();
                items.push(OTHER_LABEL.into());
                let idx = Select::with_theme(&theme)
                    .with_prompt("Tauri project directory")
                    .items(&items)
                    .default(0)
                    .interact()?;
                if idx == items.len() - 1 {
                    Input::with_theme(&theme)
                        .with_prompt("Enter path")
                        .interact_text()?
                } else {
                    items[idx].clone()
                }
            };

            // Detect package manager from lockfile in the tauri dir.
            let pm_items = [
                ("npx tauri", "npm (npx)"),
                ("yarn tauri", "yarn"),
                ("pnpm tauri", "pnpm"),
                ("bun tauri", "bun"),
                ("cargo tauri", "cargo (direct)"),
            ];
            let default_idx = detect_tauri_command(&cwd.join(&tauri_dir), &pm_items);
            let pm_idx = Select::with_theme(&theme)
                .with_prompt("Tauri CLI runner")
                .items(&pm_items.iter().map(|(_, label)| *label).collect::<Vec<_>>())
                .default(default_idx)
                .interact()?;
            let tauri_cmd = pm_items[pm_idx].0.to_string();

            let ios_scheme: String = Input::with_theme(&theme)
                .with_prompt("iOS scheme name")
                .default(format!("{}_iOS", app_name))
                .interact_text()?;
            placeholders.push(("TAURI_DIR", tauri_dir));
            placeholders.push(("IOS_SCHEME", ios_scheme));
            placeholders.push(("TAURI_CMD", tauri_cmd));
        }
    }

    // For native-{ios,android}, include platform in default filename so the
    // same app can have separate scripts. Cross-platform frameworks don't.
    let default_output = default_output_path(framework, &app_name);
    let output_path_str: String = Input::with_theme(&theme)
        .with_prompt("Output path")
        .default(default_output)
        .interact_text()?;
    let output_path = PathBuf::from(&output_path_str);

    let ph_refs: Vec<(&str, &str)> = placeholders.iter().map(|(k, v)| (*k, v.as_str())).collect();
    write_install_script(&output_path, framework, &ph_refs).with_context(|| {
        format!(
            "failed to write install script to {}",
            output_path.display()
        )
    })?;
    println!("\n✓ wrote {}", output_path.display());

    let golem_toml = find_golem_toml();
    match golem_toml {
        Some(path) => {
            // Show relative path in the prompt rather than absolute noise.
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let display_path = pathdiff_relative(&path, &cwd).unwrap_or_else(|| path.clone());
            let yes = Confirm::with_theme(&theme)
                .with_prompt(format!("Add to [[apps]] in {}?", display_path.display()))
                .default(true)
                .interact()?;
            if yes {
                // Ask for bundle id only now — it's only needed when writing the
                // [[apps]] entry. Leave blank if the entry already has one.
                let bundle_input: String = Input::with_theme(&theme)
                    .with_prompt("Bundle id (optional — leave blank if already set in [[apps]])")
                    .allow_empty(true)
                    .interact_text()?;
                let bundle_id = if bundle_input.trim().is_empty() {
                    None
                } else {
                    Some(bundle_input)
                };

                let project_root = path.parent().unwrap_or(Path::new("."));
                let rel = pathdiff_relative(&output_path, project_root)
                    .unwrap_or_else(|| output_path.clone());
                let rel_str = rel.to_string_lossy().to_string();
                // Platform-specific for native, cross-platform for Tauri/Expo.
                let platform_key = platform_key_for(framework);
                update_golem_toml_install_script(
                    &path,
                    &app_name,
                    bundle_id.as_deref(),
                    &rel_str,
                    platform_key,
                )?;
                match platform_key {
                    Some(p) => println!(
                        "✓ updated {} with [[apps]] name=\"{}\" install_script.{} = \"{}\"",
                        display_path.display(),
                        app_name,
                        p,
                        rel_str
                    ),
                    None => println!(
                        "✓ updated {} with [[apps]] name=\"{}\" install_script = \"{}\"",
                        display_path.display(),
                        app_name,
                        rel_str
                    ),
                }
            } else {
                let snippet = install_script_snippet(framework, &output_path.display().to_string());
                println!(
                    "\nAdd to [[apps]] in golem.toml (or [[flow.apps]] in a flow file):\n  [[apps]]\n  name = \"{}\"\n  {}",
                    app_name, snippet
                );
            }
        }
        None => {
            println!(
                "\n(no golem.toml found — run `golem init` to create one, then add this app manually)"
            );
        }
    }

    Ok(())
}

fn find_golem_toml() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    crate::project::find_project_root(&cwd).map(|p| p.join("golem.toml"))
}

fn pathdiff_relative(path: &Path, base: &Path) -> Option<PathBuf> {
    let abs_path = path
        .canonicalize()
        .ok()
        .or_else(|| Some(path.to_path_buf()))?;
    let abs_base = base
        .canonicalize()
        .ok()
        .or_else(|| Some(base.to_path_buf()))?;

    let mut path_components = abs_path.components().peekable();
    let mut base_components = abs_base.components().peekable();

    while path_components.peek() == base_components.peek() && path_components.peek().is_some() {
        path_components.next();
        base_components.next();
    }

    let mut result = PathBuf::new();
    for _ in base_components {
        result.push("..");
    }
    for c in path_components {
        result.push(c.as_os_str());
    }
    if result.as_os_str().is_empty() {
        Some(PathBuf::from("."))
    } else {
        Some(result)
    }
}

/// Walk up to `max_depth` from `root` looking for `.xcworkspace` and
/// `.xcodeproj` bundles. Skips common noise (node_modules, build, .git, target).
fn discover_xcode_projects(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_for(
        root,
        max_depth,
        &mut |p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.ends_with(".xcworkspace") || name.ends_with(".xcodeproj")
        },
        &mut out,
    );
    // Prefer .xcworkspace over .xcodeproj in same dir.
    out.sort_by(|a, b| {
        let a_ws = a.extension().map(|e| e == "xcworkspace").unwrap_or(false);
        let b_ws = b.extension().map(|e| e == "xcworkspace").unwrap_or(false);
        b_ws.cmp(&a_ws).then_with(|| a.cmp(b))
    });
    out
}

/// Walk for Gradle project roots (directories with `settings.gradle[.kts]`).
/// Returns relative path strings. Does NOT return submodule dirs — modules
/// are named by the user separately.
fn discover_android_roots(root: &Path, max_depth: usize) -> Vec<String> {
    let mut dirs = Vec::<PathBuf>::new();
    walk_for(
        root,
        max_depth,
        &mut |p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name == "settings.gradle" || name == "settings.gradle.kts"
        },
        &mut dirs,
    );
    let mut out: Vec<String> = dirs
        .iter()
        .filter_map(|p| p.parent())
        .map(|p| {
            p.strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .to_string()
        })
        .map(|s| if s.is_empty() { ".".to_string() } else { s })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Walk for directories containing `src-tauri/`.
fn discover_tauri_dirs(root: &Path, max_depth: usize) -> Vec<String> {
    let mut tauri_children = Vec::<PathBuf>::new();
    walk_for(
        root,
        max_depth,
        &mut |p| p.is_dir() && p.file_name().and_then(|n| n.to_str()) == Some("src-tauri"),
        &mut tauri_children,
    );
    let mut out: Vec<String> = tauri_children
        .iter()
        .filter_map(|p| p.parent())
        .map(|p| {
            p.strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .to_string()
        })
        .map(|s| if s.is_empty() { ".".to_string() } else { s })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Invoke `xcodebuild -list` on a project/workspace and parse scheme names.
/// Returns empty Vec if xcodebuild fails or produces no schemes.
fn discover_xcode_schemes(project_path: &str) -> Vec<String> {
    let flag = if project_path.ends_with(".xcworkspace") {
        "-workspace"
    } else {
        "-project"
    };
    let output = Command::new("xcodebuild")
        .args(["-list", flag, project_path])
        .output();
    let Ok(out) = output else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_xcode_schemes(&text)
}

/// Parse scheme names out of `xcodebuild -list` output. Schemes are listed
/// one-per-line under a `Schemes:` header, terminated by a blank line.
fn parse_xcode_schemes(text: &str) -> Vec<String> {
    let mut schemes = Vec::new();
    let mut in_schemes = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Schemes:") {
            in_schemes = true;
            continue;
        }
        if in_schemes {
            if t.is_empty() {
                break;
            }
            schemes.push(t.to_string());
        }
    }
    schemes
}

/// Recursive directory walk with a predicate and depth limit.
/// Does not descend into matched Xcode bundles (`.xcodeproj`, `.xcworkspace`)
/// or common noise (node_modules, build artifacts, etc.).
fn walk_for(
    dir: &Path,
    depth: usize,
    predicate: &mut dyn FnMut(&Path) -> bool,
    out: &mut Vec<PathBuf>,
) {
    if depth == 0 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // Skip common noise.
        if matches!(
            name,
            "node_modules"
                | "target"
                | ".git"
                | ".golem"
                | "build"
                | "DerivedData"
                | ".gradle"
                | "dist"
                | ".next"
                | ".cache"
        ) {
            continue;
        }
        if name.starts_with('.') && name != "." && name != ".." {
            continue;
        }
        if predicate(&path) {
            out.push(path.clone());
        }
        // Don't descend into Xcode bundles — their internals (e.g.
        // `.xcodeproj/project.xcworkspace`) are implementation details.
        if path.is_dir() && !name.ends_with(".xcodeproj") && !name.ends_with(".xcworkspace") {
            walk_for(&path, depth - 1, predicate, out);
        }
    }
}

/// Detect package manager from lockfiles present in the Tauri project dir
/// (or its immediate parent — often the lockfile lives at repo root while
/// tauri lives in a subdir). Returns index into `pm_items` matching the
/// preferred command, or 0 (npm) when nothing detected.
fn detect_tauri_command(tauri_dir: &Path, pm_items: &[(&str, &str)]) -> usize {
    let candidates = [tauri_dir.to_path_buf(), tauri_dir.join("..")];
    for dir in &candidates {
        if dir.join("bun.lockb").exists() || dir.join("bun.lock").exists() {
            return pm_items
                .iter()
                .position(|(cmd, _)| cmd.starts_with("bun"))
                .unwrap_or(0);
        }
        if dir.join("pnpm-lock.yaml").exists() {
            return pm_items
                .iter()
                .position(|(cmd, _)| cmd.starts_with("pnpm"))
                .unwrap_or(0);
        }
        if dir.join("yarn.lock").exists() {
            return pm_items
                .iter()
                .position(|(cmd, _)| cmd.starts_with("yarn"))
                .unwrap_or(0);
        }
        if dir.join("package-lock.json").exists() || dir.join("package.json").exists() {
            return pm_items
                .iter()
                .position(|(cmd, _)| cmd.starts_with("npx"))
                .unwrap_or(0);
        }
    }
    0
}

fn sanitize_app_name_for_path(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Default suggested output path for the install script. Native frameworks
/// embed the platform so an app can keep separate ios/android scripts;
/// cross-platform frameworks (Tauri) use a single filename.
fn default_output_path(framework: InstallFramework, app_name: &str) -> String {
    let slug = sanitize_app_name_for_path(app_name);
    match framework {
        InstallFramework::NativeIos => format!("scripts/install-{slug}-ios.sh"),
        InstallFramework::NativeAndroid => format!("scripts/install-{slug}-android.sh"),
        InstallFramework::Tauri => format!("scripts/install-{slug}.sh"),
    }
}

/// The `install_script.<platform>` key for a framework, or `None` for
/// cross-platform frameworks that write a bare `install_script` value.
fn platform_key_for(framework: InstallFramework) -> Option<&'static str> {
    match framework {
        InstallFramework::NativeIos => Some("ios"),
        InstallFramework::NativeAndroid => Some("android"),
        InstallFramework::Tauri => None,
    }
}

/// The `install_script` TOML snippet to print when the user declines the
/// automatic golem.toml update. Platform-specific frameworks emit an inline
/// table keyed by platform; cross-platform frameworks emit a bare string.
fn install_script_snippet(framework: InstallFramework, output_path: &str) -> String {
    match platform_key_for(framework) {
        Some(p) => format!("install_script = {{ {p} = \"{output_path}\" }}"),
        None => format!("install_script = \"{output_path}\""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pathdiff_basic() {
        let tmp = tempfile::tempdir().expect("tempdir() SHALL succeed");
        let root = tmp.path();
        let script = root.join("scripts").join("install.sh");
        std::fs::create_dir_all(script.parent().expect("parent() SHALL succeed"))
            .expect("value SHALL be present");
        std::fs::write(&script, "").expect("write() SHALL succeed");
        let rel = pathdiff_relative(&script, root).expect("pathdiff_relative() SHALL succeed");
        assert_eq!(rel, PathBuf::from("scripts/install.sh"));
    }

    #[test]
    fn discover_xcode_projects_finds_nested() {
        let tmp = tempfile::tempdir().expect("tempdir() SHALL succeed");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("ios/MyApp.xcodeproj")).expect("value SHALL be present");
        std::fs::create_dir_all(root.join("other/Sub.xcworkspace"))
            .expect("value SHALL be present");
        std::fs::create_dir_all(root.join("node_modules/bogus.xcodeproj"))
            .expect("value SHALL be present");
        let found = discover_xcode_projects(root, 5);
        assert_eq!(found.len(), 2, "SHALL skip node_modules; got {:?}", found);
        // .xcworkspace preferred first
        assert!(found[0].to_string_lossy().ends_with(".xcworkspace"));
    }

    #[test]
    fn discover_xcode_projects_skips_internal_workspace() {
        // Every real .xcodeproj has a `project.xcworkspace` inside it.
        // That's Xcode internals and SHALL NOT be surfaced to the user.
        let tmp = tempfile::tempdir().expect("tempdir() SHALL succeed");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("ios/MyApp.xcodeproj/project.xcworkspace"))
            .expect("value SHALL be present");
        let found = discover_xcode_projects(root, 5);
        assert_eq!(
            found.len(),
            1,
            "SHALL not recurse into .xcodeproj: {:?}",
            found
        );
        assert!(found[0].to_string_lossy().ends_with("MyApp.xcodeproj"));
    }

    #[test]
    fn discover_android_roots_finds_settings_gradle_dirs() {
        let tmp = tempfile::tempdir().expect("tempdir() SHALL succeed");
        let root = tmp.path();
        // project-a has a settings.gradle → is a root
        std::fs::create_dir_all(root.join("project-a/app")).expect("value SHALL be present");
        std::fs::write(root.join("project-a/settings.gradle"), "").expect("value SHALL be present");
        std::fs::write(root.join("project-a/build.gradle"), "").expect("value SHALL be present");
        std::fs::write(root.join("project-a/app/build.gradle"), "")
            .expect("value SHALL be present");
        // project-b uses .kts
        std::fs::create_dir_all(root.join("project-b")).expect("value SHALL be present");
        std::fs::write(root.join("project-b/settings.gradle.kts"), "")
            .expect("value SHALL be present");
        let found = discover_android_roots(root, 5);
        assert_eq!(
            found,
            vec!["project-a".to_string(), "project-b".to_string()]
        );
    }

    #[test]
    fn discover_tauri_dirs_finds_parents_of_src_tauri() {
        let tmp = tempfile::tempdir().expect("tempdir() SHALL succeed");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("app-a/src-tauri")).expect("value SHALL be present");
        std::fs::create_dir_all(root.join("app-b/src-tauri")).expect("value SHALL be present");
        let found = discover_tauri_dirs(root, 5);
        assert_eq!(found, vec!["app-a".to_string(), "app-b".to_string()]);
    }

    #[test]
    fn detect_tauri_command_prefers_lockfile() {
        let items = [
            ("npx tauri", "npm"),
            ("yarn tauri", "yarn"),
            ("pnpm tauri", "pnpm"),
            ("bun tauri", "bun"),
            ("cargo tauri", "cargo"),
        ];

        // no lockfile → npx default
        let tmp = tempfile::tempdir().expect("tempdir() SHALL succeed");
        assert_eq!(detect_tauri_command(tmp.path(), &items), 0);

        // yarn.lock → yarn
        let tmp = tempfile::tempdir().expect("tempdir() SHALL succeed");
        std::fs::write(tmp.path().join("yarn.lock"), "").expect("value SHALL be present");
        assert_eq!(detect_tauri_command(tmp.path(), &items), 1);

        // pnpm-lock.yaml → pnpm
        let tmp = tempfile::tempdir().expect("tempdir() SHALL succeed");
        std::fs::write(tmp.path().join("pnpm-lock.yaml"), "").expect("value SHALL be present");
        assert_eq!(detect_tauri_command(tmp.path(), &items), 2);

        // bun.lockb → bun
        let tmp = tempfile::tempdir().expect("tempdir() SHALL succeed");
        std::fs::write(tmp.path().join("bun.lockb"), "").expect("value SHALL be present");
        assert_eq!(detect_tauri_command(tmp.path(), &items), 3);

        // parent dir lockfile is checked (tauri subdir case)
        let tmp = tempfile::tempdir().expect("tempdir() SHALL succeed");
        let sub = tmp.path().join("app");
        std::fs::create_dir(&sub).expect("create_dir() SHALL succeed");
        std::fs::write(tmp.path().join("pnpm-lock.yaml"), "").expect("value SHALL be present");
        assert_eq!(detect_tauri_command(&sub, &items), 2);
    }

    #[test]
    fn sanitize_app_name_for_path_basic() {
        assert_eq!(sanitize_app_name_for_path("myapp"), "myapp");
        assert_eq!(sanitize_app_name_for_path("my-app_2"), "my-app_2");
        assert_eq!(sanitize_app_name_for_path("my app!"), "my-app-");
    }

    // 1. Empty name sanitizes to empty; non-ASCII alphanumerics are kept
    //    (char::is_alphanumeric is Unicode-aware), other symbols become '-'.
    #[test]
    fn sanitize_app_name_for_path_edge_cases() {
        assert_eq!(sanitize_app_name_for_path(""), "", "empty SHALL stay empty");
        assert_eq!(
            sanitize_app_name_for_path("café9"),
            "café9",
            "unicode alphanumerics SHALL be preserved"
        );
        assert_eq!(
            sanitize_app_name_for_path("a/b\\c.d"),
            "a-b-c-d",
            "path separators and dots SHALL become dashes"
        );
        assert_eq!(
            sanitize_app_name_for_path("!@#"),
            "---",
            "all-symbol input SHALL map each char to a dash"
        );
    }

    // 2. Identical path and base resolve to "." rather than an empty path.
    #[test]
    fn pathdiff_identical_is_dot() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let rel = pathdiff_relative(root, root).expect("relative");
        assert_eq!(rel, PathBuf::from("."), "same path SHALL yield \".\"");
    }

    // 3. A sibling directory must be reached by going up then down ("../sibling").
    #[test]
    fn pathdiff_sibling_uses_parent_traversal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let a = root.join("a");
        let b = root.join("b");
        std::fs::create_dir_all(&a).expect("mkdir a");
        std::fs::create_dir_all(&b).expect("mkdir b");
        let rel = pathdiff_relative(&b, &a).expect("relative");
        assert_eq!(
            rel,
            PathBuf::from("../b"),
            "sibling SHALL traverse up then into target"
        );
    }

    // 4. A base nested below the target yields one ".." per extra base level.
    #[test]
    fn pathdiff_base_below_target() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let deep = root.join("x").join("y");
        std::fs::create_dir_all(&deep).expect("mkdir deep");
        let rel = pathdiff_relative(root, &deep).expect("relative");
        assert_eq!(
            rel,
            PathBuf::from("../.."),
            "ascending two levels SHALL produce two parent hops"
        );
    }

    // 5. No matching project/workspace anywhere returns an empty Vec.
    #[test]
    fn discover_xcode_projects_empty_when_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        let found = discover_xcode_projects(tmp.path(), 5);
        assert!(
            found.is_empty(),
            "no xcode artifacts SHALL yield empty: {found:?}"
        );
    }

    // 6. When two .xcodeproj exist they are ordered alphabetically by path
    //    (workspace-preference tie-breaker falls through to path compare).
    #[test]
    fn discover_xcode_projects_sorts_alphabetically_within_type() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("zeta/Z.xcodeproj")).expect("mkdir z");
        std::fs::create_dir_all(root.join("alpha/A.xcodeproj")).expect("mkdir a");
        let found = discover_xcode_projects(root, 5);
        assert_eq!(found.len(), 2);
        assert!(
            found[0].to_string_lossy().contains("alpha"),
            "alphabetical order SHALL place alpha first: {found:?}"
        );
    }

    // 7. A settings.gradle at the search root itself surfaces as "." (empty
    //    relative path is normalized to dot).
    #[test]
    fn discover_android_roots_root_itself_is_dot() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::write(root.join("settings.gradle"), "").expect("write");
        let found = discover_android_roots(root, 5);
        assert_eq!(found, vec![".".to_string()], "root gradle SHALL be \".\"");
    }

    // 8. Both settings.gradle and settings.gradle.kts in the same dir dedupe
    //    to a single entry for that directory.
    #[test]
    fn discover_android_roots_dedupes_same_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("proj")).expect("mkdir");
        std::fs::write(root.join("proj/settings.gradle"), "").expect("write");
        std::fs::write(root.join("proj/settings.gradle.kts"), "").expect("write");
        let found = discover_android_roots(root, 5);
        assert_eq!(found, vec!["proj".to_string()], "same dir SHALL dedupe");
    }

    // 9. A file (not directory) named src-tauri is ignored — only directories
    //    count, per the predicate's is_dir() guard.
    #[test]
    fn discover_tauri_dirs_ignores_file_named_src_tauri() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::write(root.join("src-tauri"), "").expect("write file");
        let found = discover_tauri_dirs(root, 5);
        assert!(
            found.is_empty(),
            "file named src-tauri SHALL be ignored: {found:?}"
        );
    }

    // 10. A src-tauri directly under the search root surfaces its parent as ".".
    #[test]
    fn discover_tauri_dirs_root_parent_is_dot() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src-tauri")).expect("mkdir");
        let found = discover_tauri_dirs(root, 5);
        assert_eq!(
            found,
            vec![".".to_string()],
            "root src-tauri parent SHALL be \".\""
        );
    }

    // 11. bun is detected ahead of all others when bun.lock(b) co-exists with
    //    a pnpm/yarn/npm lockfile in the same dir (check order is bun-first).
    #[test]
    fn detect_tauri_command_bun_wins_over_others() {
        let items = [
            ("npx tauri", "npm"),
            ("yarn tauri", "yarn"),
            ("pnpm tauri", "pnpm"),
            ("bun tauri", "bun"),
            ("cargo tauri", "cargo"),
        ];
        let tmp = tempfile::tempdir().expect("tempdir");
        let d = tmp.path();
        std::fs::write(d.join("bun.lock"), "").expect("write");
        std::fs::write(d.join("pnpm-lock.yaml"), "").expect("write");
        std::fs::write(d.join("yarn.lock"), "").expect("write");
        std::fs::write(d.join("package-lock.json"), "").expect("write");
        assert_eq!(
            detect_tauri_command(d, &items),
            3,
            "bun SHALL take priority"
        );
    }

    // 12. package-lock.json (npm) and bare package.json both fall back to npx.
    #[test]
    fn detect_tauri_command_npm_lockfiles() {
        let items = [
            ("npx tauri", "npm"),
            ("yarn tauri", "yarn"),
            ("pnpm tauri", "pnpm"),
            ("bun tauri", "bun"),
            ("cargo tauri", "cargo"),
        ];
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("package-lock.json"), "").expect("write");
        assert_eq!(
            detect_tauri_command(tmp.path(), &items),
            0,
            "package-lock SHALL pick npx"
        );

        let tmp2 = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp2.path().join("package.json"), "{}").expect("write");
        assert_eq!(
            detect_tauri_command(tmp2.path(), &items),
            0,
            "package.json SHALL pick npx"
        );
    }

    // 13. The dir's own lockfile takes precedence over a parent dir lockfile
    //    (candidates are scanned dir-first, parent-second).
    #[test]
    fn detect_tauri_command_own_dir_beats_parent() {
        let items = [
            ("npx tauri", "npm"),
            ("yarn tauri", "yarn"),
            ("pnpm tauri", "pnpm"),
            ("bun tauri", "bun"),
            ("cargo tauri", "cargo"),
        ];
        let tmp = tempfile::tempdir().expect("tempdir");
        let sub = tmp.path().join("app");
        std::fs::create_dir(&sub).expect("mkdir");
        std::fs::write(sub.join("yarn.lock"), "").expect("write child");
        std::fs::write(tmp.path().join("pnpm-lock.yaml"), "").expect("write parent");
        assert_eq!(
            detect_tauri_command(&sub, &items),
            1,
            "own-dir lockfile SHALL win"
        );
    }

    // 14. walk_for honours its depth budget: at depth 1 only direct children
    //    are examined, so a nested match is not found.
    #[test]
    fn walk_for_respects_depth_limit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("nested")).expect("mkdir");
        std::fs::write(root.join("nested/target.txt"), "").expect("write");
        let mut found = Vec::new();
        walk_for(
            root,
            1,
            &mut |p| p.file_name().and_then(|n| n.to_str()) == Some("target.txt"),
            &mut found,
        );
        assert!(
            found.is_empty(),
            "depth 1 SHALL not reach a grandchild: {found:?}"
        );

        let mut found2 = Vec::new();
        walk_for(
            root,
            5,
            &mut |p| p.file_name().and_then(|n| n.to_str()) == Some("target.txt"),
            &mut found2,
        );
        assert_eq!(found2.len(), 1, "deeper budget SHALL reach the match");
    }

    // 15. walk_for skips dot-prefixed entries (hidden dirs/files) entirely.
    #[test]
    fn walk_for_skips_hidden_entries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".hidden")).expect("mkdir");
        std::fs::write(root.join(".hidden/marker"), "").expect("write nested");
        std::fs::write(root.join(".secret"), "").expect("write hidden file");
        let mut found = Vec::new();
        walk_for(root, 5, &mut |_p| true, &mut found);
        assert!(
            found.iter().all(|p| {
                let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                !n.starts_with('.')
            }),
            "hidden entries SHALL be skipped: {found:?}"
        );
    }

    // 16. walk_for returns silently when handed a path that is not a readable
    //    directory (read_dir error branch).
    #[test]
    fn walk_for_nonexistent_dir_is_noop() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("does-not-exist");
        let mut found = Vec::new();
        walk_for(&missing, 5, &mut |_p| true, &mut found);
        assert!(found.is_empty(), "missing dir SHALL produce no matches");
    }

    // 17. parse_xcode_schemes reads names under the `Schemes:` header and stops
    //    at the first blank line that follows them.
    #[test]
    fn parse_xcode_schemes_extracts_block() {
        let text = "Information about project \"MyApp\":\n    \
                    Targets:\n        MyApp\n\n    \
                    Schemes:\n        MyApp\n        MyAppTests\n\n";
        let schemes = parse_xcode_schemes(text);
        assert_eq!(
            schemes,
            vec!["MyApp".to_string(), "MyAppTests".to_string()],
            "SHALL collect scheme names until the terminating blank line"
        );
    }

    // 18. No `Schemes:` header anywhere yields an empty Vec.
    #[test]
    fn parse_xcode_schemes_no_header_is_empty() {
        let text = "Information about workspace:\n    Targets:\n        App\n";
        assert!(
            parse_xcode_schemes(text).is_empty(),
            "absence of Schemes header SHALL yield no schemes"
        );
    }

    // 19. Empty input yields no schemes.
    #[test]
    fn parse_xcode_schemes_empty_input() {
        assert!(
            parse_xcode_schemes("").is_empty(),
            "empty text SHALL yield no schemes"
        );
    }

    // 20. A Schemes header with no following blank line (EOF instead) still
    //    terminates cleanly with whatever was collected.
    #[test]
    fn parse_xcode_schemes_header_at_eof() {
        let text = "Schemes:\n        OnlyOne";
        assert_eq!(
            parse_xcode_schemes(text),
            vec!["OnlyOne".to_string()],
            "trailing scheme without blank line SHALL still be collected"
        );
    }

    // 21. default_output_path embeds the platform for native frameworks and a
    //    bare filename for Tauri, slugging the app name in all cases.
    #[test]
    fn default_output_path_per_framework() {
        assert_eq!(
            default_output_path(InstallFramework::NativeIos, "My App"),
            "scripts/install-My-App-ios.sh",
            "iOS default SHALL embed -ios and slug the name"
        );
        assert_eq!(
            default_output_path(InstallFramework::NativeAndroid, "My App"),
            "scripts/install-My-App-android.sh",
            "Android default SHALL embed -android"
        );
        assert_eq!(
            default_output_path(InstallFramework::Tauri, "My App"),
            "scripts/install-My-App.sh",
            "Tauri default SHALL omit a platform suffix"
        );
    }

    // 22. platform_key_for maps native frameworks to their platform and Tauri
    //    to None (cross-platform).
    #[test]
    fn platform_key_for_maps_frameworks() {
        assert_eq!(platform_key_for(InstallFramework::NativeIos), Some("ios"));
        assert_eq!(
            platform_key_for(InstallFramework::NativeAndroid),
            Some("android")
        );
        assert_eq!(
            platform_key_for(InstallFramework::Tauri),
            None,
            "Tauri SHALL have no platform key"
        );
    }

    // 23. install_script_snippet emits an inline table for native frameworks
    //    and a bare string for cross-platform ones.
    #[test]
    fn install_script_snippet_per_framework() {
        assert_eq!(
            install_script_snippet(InstallFramework::NativeIos, "scripts/install-app-ios.sh"),
            "install_script = { ios = \"scripts/install-app-ios.sh\" }",
            "iOS SHALL emit an ios-keyed inline table"
        );
        assert_eq!(
            install_script_snippet(InstallFramework::NativeAndroid, "scripts/x.sh"),
            "install_script = { android = \"scripts/x.sh\" }",
            "Android SHALL emit an android-keyed inline table"
        );
        assert_eq!(
            install_script_snippet(InstallFramework::Tauri, "scripts/install-app.sh"),
            "install_script = \"scripts/install-app.sh\"",
            "Tauri SHALL emit a bare install_script string"
        );
    }
}
