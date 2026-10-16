//! What a project still needs before golem can install its apps: a
//! `golem.toml`, and for each of its `[[apps]]` an install script that
//! exists and runs. `session_open` and `golem doctor` both report it.

use std::path::{Path, PathBuf};

use golem_parser::InstallScriptValue;

/// The setup of the project that `start` is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setup {
    /// No `golem.toml` in `start` or above.
    NoProject,
    /// `golem.toml` does not load.
    Unreadable {
        toml: PathBuf,
        error: String,
    },
    Project {
        toml: PathBuf,
        apps: Vec<App>,
    },
}

/// One `[[apps]]` entry: each install script, by its path as written.
/// No scripts means the entry has no `install_script`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub name: String,
    pub scripts: Vec<(String, Script)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Ok,
    Missing,
    NotExecutable,
}

pub fn check(start: &Path) -> Setup {
    let Some(root) = golem_orchestrator::project::find_project_root(start) else {
        return Setup::NoProject;
    };
    let toml = root.join("golem.toml");
    let project = match golem_orchestrator::project::ProjectConfig::load_from(&root) {
        Ok((project, _)) => project,
        Err(e) => {
            return Setup::Unreadable {
                toml,
                error: format!("{e:#}"),
            }
        }
    };
    let apps = project
        .apps
        .iter()
        .map(|app| {
            let mut paths: Vec<&String> = match &app.install_script {
                None => Vec::new(),
                Some(InstallScriptValue::Single(p)) => vec![p],
                Some(InstallScriptValue::PerPlatform(p)) => {
                    p.ios.iter().chain(p.android.iter()).collect()
                }
            };
            paths.dedup();
            App {
                name: app.name.clone(),
                scripts: paths
                    .into_iter()
                    .map(|p| (p.clone(), script(&root.join(p))))
                    .collect(),
            }
        })
        .collect();
    Setup::Project { toml, apps }
}

fn script(path: &Path) -> Script {
    let Ok(meta) = std::fs::metadata(path) else {
        return Script::Missing;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Script::NotExecutable;
        }
    }
    let _ = meta;
    Script::Ok
}

/// What is missing, one line for each part, for the `session_open` result.
pub fn session_lines(setup: &Setup) -> String {
    let mut out = String::new();
    match setup {
        Setup::NoProject => out.push_str("setup: no golem.toml · golem init creates one\n"),
        Setup::Unreadable { toml, error } => out.push_str(&format!(
            "setup: {} does not load: {error}\n",
            toml.display()
        )),
        Setup::Project { apps, .. } => {
            for app in apps {
                if app.scripts.is_empty() {
                    out.push_str(&format!(
                        "setup: app {:?} has no install_script · help(\"install\")\n",
                        app.name
                    ));
                }
                for (path, state) in &app.scripts {
                    match state {
                        Script::Ok => {}
                        Script::Missing => out.push_str(&format!(
                            "setup: app {:?} install_script {path} does not exist · help(\"install\")\n",
                            app.name
                        )),
                        Script::NotExecutable => out.push_str(&format!(
                            "setup: app {:?} install_script {path} is not executable · chmod +x {path}\n",
                            app.name
                        )),
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, text: &str, mode: u32) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("dir");
        std::fs::write(&p, text).expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).expect("mode");
        }
    }

    #[test]
    fn outside_a_project_the_only_line_names_golem_init() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(check(dir.path()), Setup::NoProject);
        assert_eq!(
            session_lines(&Setup::NoProject),
            "setup: no golem.toml · golem init creates one\n"
        );
    }

    #[test]
    fn each_app_without_a_working_script_gets_a_line_and_a_working_one_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "golem.toml",
            r#"
[[apps]]
name = "ready"
bundle = "a"
install_script = "scripts/ready.sh"

[[apps]]
name = "bare"
bundle = "b"

[[apps]]
name = "split"
bundle = "c"
install_script = { ios = "scripts/ios.sh", android = "scripts/gone.sh" }
"#,
            0o644,
        );
        write(dir.path(), "scripts/ready.sh", "#!/bin/sh\n", 0o755);
        write(dir.path(), "scripts/ios.sh", "#!/bin/sh\n", 0o644);
        let setup = check(&dir.path().join("scripts"));
        assert_eq!(
            session_lines(&setup),
            "setup: app \"bare\" has no install_script · help(\"install\")\n\
             setup: app \"split\" install_script scripts/ios.sh is not executable · chmod +x scripts/ios.sh\n\
             setup: app \"split\" install_script scripts/gone.sh does not exist · help(\"install\")\n"
        );
    }

    #[test]
    fn a_project_with_every_script_in_place_has_no_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "golem.toml",
            "[[apps]]\nname = \"a\"\nbundle = \"a\"\ninstall_script = \"i.sh\"\n",
            0o644,
        );
        write(dir.path(), "i.sh", "#!/bin/sh\n", 0o755);
        assert_eq!(session_lines(&check(dir.path())), "");
    }

    #[test]
    fn one_script_for_both_platforms_is_checked_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "golem.toml",
            "[[apps]]\nname = \"a\"\nbundle = \"a\"\ninstall_script = { ios = \"i.sh\", android = \"i.sh\" }\n",
            0o644,
        );
        match check(dir.path()) {
            Setup::Project { apps, .. } => assert_eq!(apps[0].scripts.len(), 1),
            other => panic!("{other:?}"),
        }
    }
}
