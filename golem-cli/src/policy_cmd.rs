//! The policy for what an LLM may run through `golem mcp`: where it is
//! read from, and `golem mcp-config`, which changes it.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use golem_parser::policy::{ActionClass, Mode, Policy, PolicyConfig};

/// Where `golem mcp` reads the policy from.
#[derive(Debug, Clone, Default)]
pub struct PolicySource {
    /// The user's config. `None` reads none, so the defaults apply: the
    /// tests must not read the config of the machine they run on.
    pub user_config: Option<PathBuf>,
    /// `--deny` and `--ask`: classes or actions, only stricter.
    pub deny: Vec<String>,
    pub ask: Vec<String>,
}

impl PolicySource {
    /// The policy for a session in `project_root`: the defaults, the
    /// user's config over them, then the project `golem.toml` and the
    /// arguments, which can only make it stricter.
    pub fn policy(&self, project_root: &Path) -> Result<Policy> {
        let user = match &self.user_config {
            Some(path) => read_config(path)?,
            None => PolicyConfig::default(),
        };
        let mut policy = Policy::from_user(&user);
        let project = project_root.join("golem.toml");
        if let Ok(text) = std::fs::read_to_string(&project) {
            policy.tighten(
                &PolicyConfig::from_toml(&text)
                    .with_context(|| format!("reading the policy in {}", project.display()))?,
            );
        }
        policy.tighten(&self.arguments()?);
        Ok(policy)
    }

    fn arguments(&self) -> Result<PolicyConfig> {
        let mut config = PolicyConfig::default();
        for (names, mode) in [(&self.ask, Mode::Ask), (&self.deny, Mode::Deny)] {
            for name in names {
                set(&mut config, name, mode)?;
            }
        }
        Ok(config)
    }
}

/// `~/.golem/config.toml`, or the file `GOLEM_CONFIG` names.
pub fn user_config_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("GOLEM_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    Ok(dirs::home_dir()
        .context("could not determine the home directory")?
        .join(".golem/config.toml"))
}

/// The `[mcp.policy]` of the file at `path`; none when there is no file.
fn read_config(path: &Path) -> Result<PolicyConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => PolicyConfig::from_toml(&text)
            .with_context(|| format!("reading the policy in {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PolicyConfig::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Set `name`, a class or an action, to `mode` in `config`.
fn set(config: &mut PolicyConfig, name: &str, mode: Mode) -> Result<()> {
    if ActionClass::parse(name).is_some() {
        config.classes.insert(name.to_string(), mode);
    } else if ActionClass::of(name).is_some() {
        config.actions.insert(name.to_string(), mode);
    } else {
        bail!(
            "{name:?} is not an action class or an action. Classes: {}",
            ActionClass::ALL.map(ActionClass::name).join(", ")
        );
    }
    Ok(())
}

/// Write `config` as the `[mcp.policy]` of the file at `path`, and keep
/// everything else in the file.
fn write_config(path: &Path, config: &PolicyConfig) -> Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("reading {}", path.display()))?;
    let mcp = doc
        .entry("mcp")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("[mcp] is not a table")?;
    mcp.set_implicit(true);
    let mut policy = toml_edit::Table::new();
    for (name, mode) in &config.classes {
        policy.insert(name, toml_edit::value(mode.name()));
    }
    if !config.actions.is_empty() {
        let mut actions = toml_edit::Table::new();
        for (name, mode) in &config.actions {
            actions.insert(name, toml_edit::value(mode.name()));
        }
        policy.insert("actions", toml_edit::Item::Table(actions));
    }
    mcp.insert("policy", toml_edit::Item::Table(policy));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, doc.to_string()).with_context(|| format!("writing {}", path.display()))
}

/// The policy as lines for a person: each class, each action setting, and
/// what golem changed.
pub fn describe(policy: &Policy) -> String {
    let mut out = String::new();
    for class in ActionClass::ALL {
        out.push_str(&format!(
            "{:<16} {}\n",
            class.name(),
            policy.class_mode(class).name()
        ));
    }
    for (action, mode) in policy.actions() {
        out.push_str(&format!("{action:<16} {} (action)\n", mode.name()));
    }
    for note in policy.notes() {
        out.push_str(&format!("note: {note}\n"));
    }
    out
}

/// `golem mcp-config`: with no arguments, change the policy interactively;
/// `show` prints it; `set NAME MODE` sets one class or action. A change
/// that makes the policy looser needs a terminal: an LLM cannot easily
/// work a terminal prompt, so it cannot loosen its own policy.
pub fn run(args: &crate::cli::McpConfigArgs) -> Result<i32> {
    let path = user_config_path()?;
    let project_root = crate::project_root_for_policy();
    let mut config = read_config(&path)?;
    match &args.command {
        Some(crate::cli::McpConfigCommand::Show) => {
            let source = PolicySource {
                user_config: Some(path.clone()),
                ..PolicySource::default()
            };
            eprint!("{}", describe(&source.policy(&project_root)?));
            eprintln!("(user config: {})", path.display());
            Ok(0)
        }
        Some(crate::cli::McpConfigCommand::Set { name, mode }) => {
            let mode = Mode::parse(mode)?;
            // Compare what the file says, not the effective policy: a shell
            // that golem raises today would hide a looser value written now.
            let looser = mode < Policy::configured(&config, name);
            set(&mut config, name, mode)?;
            if looser && !crate::terminal::is_interactive() {
                bail!(
                    "making the policy looser needs a terminal. Without one:\n  \
                     golem mcp-config show    # read the policy\n  \
                     or ask the user to run `golem mcp-config set {name} {}` in a terminal",
                    mode.name()
                );
            }
            write_config(&path, &config)?;
            eprint!("{}", describe(&Policy::from_user(&config)));
            Ok(0)
        }
        None => interactive(&path, config),
    }
}

fn interactive(path: &Path, mut config: PolicyConfig) -> Result<i32> {
    use dialoguer::theme::ColorfulTheme;
    use dialoguer::Select;
    crate::terminal::require(
        "golem mcp-config",
        &[
            ("golem mcp-config show", "read the policy"),
            (
                "golem mcp-config set NAME MODE",
                "make the policy stricter (looser needs a terminal)",
            ),
        ],
    )?;
    let theme = ColorfulTheme::default();
    loop {
        let policy = Policy::from_user(&config);
        let mut items: Vec<String> = ActionClass::ALL
            .iter()
            .map(|c| format!("{:<16} {}", c.name(), policy.class_mode(*c).name()))
            .collect();
        items.push("save and exit".into());
        items.push("exit without saving".into());
        for note in policy.notes() {
            eprintln!("note: {note}");
        }
        let pick = Select::with_theme(&theme)
            .with_prompt("What an LLM may run through golem mcp")
            .items(&items)
            .default(0)
            .interact()?;
        match pick.checked_sub(ActionClass::ALL.len()) {
            Some(0) => {
                write_config(path, &config)?;
                eprintln!("saved to {}", path.display());
                return Ok(0);
            }
            Some(_) => return Ok(0),
            None => {
                let class = ActionClass::ALL[pick];
                let modes = [Mode::Allow, Mode::Ask, Mode::Deny];
                let current = policy.class_mode(class);
                let mode = Select::with_theme(&theme)
                    .with_prompt(class.name())
                    .items(&modes.map(Mode::name))
                    .default(modes.iter().position(|m| *m == current).unwrap_or(0))
                    .interact()?;
                config.classes.insert(class.name().to_string(), modes[mode]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_and_the_arguments_only_tighten_the_user_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let user = dir.path().join("config.toml");
        std::fs::write(&user, "[mcp.policy]\nshell = \"allow\"\nhttp = \"allow\"\n").expect("user");
        std::fs::write(
            dir.path().join("golem.toml"),
            "[mcp.policy]\nshell = \"allow\"\nemail = \"ask\"\n",
        )
        .expect("project");
        let source = PolicySource {
            user_config: Some(user),
            deny: vec!["delete_http".into()],
            ask: Vec::new(),
        };
        let policy = source.policy(dir.path()).expect("policy");
        assert_eq!(policy.mode("get_http"), Mode::Allow);
        assert_eq!(policy.mode("delete_http"), Mode::Deny);
        assert_eq!(policy.mode("create_inbox"), Mode::Ask);
        assert_eq!(
            policy.mode("bash"),
            Mode::Deny,
            "delete_http deny SHALL raise shell to deny"
        );
    }

    #[test]
    fn no_user_config_means_the_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let policy = PolicySource::default().policy(dir.path()).expect("policy");
        assert_eq!(policy, Policy::default());
    }

    #[test]
    fn writing_the_policy_keeps_the_rest_of_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# mine\n[other]\nkeep = true\n").expect("write");
        let mut config = PolicyConfig::default();
        set(&mut config, "shell", Mode::Deny).expect("class");
        set(&mut config, "run", Mode::Ask).expect("action");
        assert!(set(&mut config, "nope", Mode::Ask).is_err());
        write_config(&path, &config).expect("write");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(
            text.contains("# mine") && text.contains("keep = true"),
            "{text}"
        );
        assert_eq!(read_config(&path).expect("read back"), config);
    }
}
