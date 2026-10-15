//! Which actions an LLM may run through `golem mcp`: each action belongs
//! to a class, and the user's policy sets each class, or one action, to
//! `allow`, `ask` or `deny`.
//!
//! Steps from the user's own files (a flow, a mixin, a subflow) never pass
//! through the policy: they run as `golem run` runs them.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// How strictly golem treats an action, from least to most strict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Allow,
    Ask,
    Deny,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Allow => "allow",
            Mode::Ask => "ask",
            Mode::Deny => "deny",
        }
    }

    pub fn parse(s: &str) -> Result<Mode> {
        match s {
            "allow" => Ok(Mode::Allow),
            "ask" => Ok(Mode::Ask),
            "deny" => Ok(Mode::Deny),
            _ => bail!("{s:?} is not a policy mode: use allow, ask or deny"),
        }
    }
}

/// The risk an action carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionClass {
    /// Reads the screen or the app: no change.
    Read,
    /// Changes the device only.
    Device,
    /// Deletes app data, or changes device state an app reads.
    DeviceData,
    /// Drives the test user in a temporary browser profile.
    Browser,
    /// Reads the test user's tokens, or runs a site's own actions.
    BrowserSecrets,
    /// Sends requests from the host, with the host's network access.
    Http,
    /// Reads email, which is untrusted input.
    Email,
    /// Runs a script in the project.
    Script,
    /// Runs any command on the host.
    Shell,
}

impl ActionClass {
    pub const ALL: [ActionClass; 9] = [
        ActionClass::Read,
        ActionClass::Device,
        ActionClass::DeviceData,
        ActionClass::Browser,
        ActionClass::BrowserSecrets,
        ActionClass::Http,
        ActionClass::Email,
        ActionClass::Script,
        ActionClass::Shell,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ActionClass::Read => "read",
            ActionClass::Device => "device",
            ActionClass::DeviceData => "device-data",
            ActionClass::Browser => "browser",
            ActionClass::BrowserSecrets => "browser-secrets",
            ActionClass::Http => "http",
            ActionClass::Email => "email",
            ActionClass::Script => "script",
            ActionClass::Shell => "shell",
        }
    }

    pub fn parse(s: &str) -> Option<ActionClass> {
        ActionClass::ALL.into_iter().find(|c| c.name() == s)
    }

    fn default_mode(self) -> Mode {
        match self {
            ActionClass::Http | ActionClass::Shell => Mode::Ask,
            _ => Mode::Allow,
        }
    }

    /// The class of `action`, or `None` for an action golem does not know.
    pub fn of(action: &str) -> Option<ActionClass> {
        Some(match action {
            "assert_alert" | "assert_not_visible" | "assert_visible" | "read" | "screenshot"
            | "load_fixture" | "load_mixin" | "fail" => ActionClass::Read,
            "tap" | "double_tap" | "type" | "backspace" | "clear_text" | "long_press" | "swipe"
            | "scroll" | "pinch" | "gesture" | "rotate" | "hide_keyboard" | "launch" | "stop"
            | "open_link" | "press" | "set_dark_mode" | "accept_alert" | "dismiss_alert" => {
                ActionClass::Device
            }
            "clear_data" | "set_location" | "push_notification" | "add_media" => {
                ActionClass::DeviceData
            }
            "browse_get_cookie"
            | "browse_get_local_storage"
            | "browse_get_session_storage"
            | "browse_execute_js"
            | "browse_mcp_call" => ActionClass::BrowserSecrets,
            a if a.starts_with("browse_") => ActionClass::Browser,
            "get_http" | "post_http" | "put_http" | "patch_http" | "delete_http" => {
                ActionClass::Http
            }
            "create_inbox" | "await_email" => ActionClass::Email,
            "run" => ActionClass::Script,
            "bash" => ActionClass::Shell,
            _ => return None,
        })
    }
}

/// A `[mcp.policy]` table: a mode per class, and `[mcp.policy.actions]`, a
/// mode per action.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicyConfig {
    #[serde(flatten)]
    pub classes: BTreeMap<String, Mode>,
    #[serde(default)]
    pub actions: BTreeMap<String, Mode>,
}

impl PolicyConfig {
    /// The `[mcp.policy]` table of a TOML file, or an empty one.
    pub fn from_toml(text: &str) -> Result<PolicyConfig> {
        let table: toml::Table = text.parse()?;
        let Some(policy) = table.get("mcp").and_then(|m| m.get("policy")) else {
            return Ok(PolicyConfig::default());
        };
        let config: PolicyConfig = policy.clone().try_into()?;
        config.check()?;
        Ok(config)
    }

    fn check(&self) -> Result<()> {
        if let Some(name) = self
            .classes
            .keys()
            .find(|k| ActionClass::parse(k).is_none())
        {
            bail!(
                "[mcp.policy] {name:?} is not an action class: use one of {}",
                class_names()
            );
        }
        if let Some(name) = self.actions.keys().find(|k| ActionClass::of(k).is_none()) {
            bail!("[mcp.policy.actions] {name:?} is not an action");
        }
        Ok(())
    }
}

fn class_names() -> String {
    ActionClass::ALL
        .iter()
        .map(|c| c.name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The policy that applies: the defaults, the user's config over them,
/// then each stricter source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    classes: BTreeMap<ActionClass, Mode>,
    actions: BTreeMap<String, Mode>,
    /// What golem changed so the rules hold, for example shell raised.
    #[serde(default)]
    notes: Vec<String>,
}

impl Default for Policy {
    fn default() -> Self {
        let mut policy = Policy {
            classes: ActionClass::ALL
                .into_iter()
                .map(|c| (c, c.default_mode()))
                .collect(),
            actions: BTreeMap::new(),
            notes: Vec::new(),
        };
        policy.hold_shell();
        policy
    }
}

impl Policy {
    /// The defaults with the user's own config over them. The user's
    /// config can make a class looser than its default.
    pub fn from_user(config: &PolicyConfig) -> Policy {
        let mut policy = Policy::default();
        policy.notes.clear();
        for (name, mode) in &config.classes {
            if let Some(class) = ActionClass::parse(name) {
                policy.classes.insert(class, *mode);
            }
        }
        policy.actions.extend(config.actions.clone());
        policy.hold_shell();
        policy
    }

    /// Apply `config` where it is stricter. A project `golem.toml` and the
    /// arguments of `golem mcp` can only make the policy stricter: a cloned
    /// repository must not allow `bash` for itself.
    pub fn tighten(&mut self, config: &PolicyConfig) {
        for (name, mode) in &config.classes {
            if let Some(class) = ActionClass::parse(name) {
                let current = self.classes.entry(class).or_insert(Mode::Allow);
                *current = (*current).max(*mode);
            }
        }
        for (action, mode) in &config.actions {
            let current = self.mode(action);
            self.actions.insert(action.clone(), current.max(*mode));
        }
        self.hold_shell();
    }

    /// Make shell, and `bash`, at least as strict as the strictest class or
    /// action: `bash` can do the work of each other class (`./script.sh`,
    /// `curl`, `adb shell pm clear`), so a stricter class would not hold.
    fn hold_shell(&mut self) {
        let strictest = self
            .classes
            .iter()
            .filter(|(c, _)| **c != ActionClass::Shell)
            .map(|(c, m)| (*m, c.name().to_string()))
            .chain(
                self.actions
                    .iter()
                    .filter(|(a, _)| a.as_str() != "bash")
                    .map(|(a, m)| (*m, a.clone())),
            )
            .max_by_key(|(m, _)| *m);
        let Some((floor, because)) = strictest else {
            return;
        };
        let shell = self.mode("bash");
        if shell < floor {
            self.notes.push(format!(
                "shell: {} → {} ({because} is {}, and bash can do its work)",
                shell.name(),
                floor.name(),
                floor.name()
            ));
            self.classes.insert(ActionClass::Shell, floor);
            if self.actions.contains_key("bash") {
                self.actions.insert("bash".into(), floor);
            }
        }
    }

    /// The mode for `action`: its own setting, else its class's. An action
    /// golem does not know is denied.
    pub fn mode(&self, action: &str) -> Mode {
        if let Some(mode) = self.actions.get(action) {
            return *mode;
        }
        ActionClass::of(action)
            .and_then(|c| self.classes.get(&c).copied())
            .unwrap_or(Mode::Deny)
    }

    /// The mode of a class.
    pub fn class_mode(&self, class: ActionClass) -> Mode {
        self.classes.get(&class).copied().unwrap_or(Mode::Allow)
    }

    /// The per-action settings.
    pub fn actions(&self) -> &BTreeMap<String, Mode> {
        &self.actions
    }

    /// What golem changed so the rules hold.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// The mode that `config` itself gives `name`, a class or an action,
    /// before golem raises shell: what a person wrote, or the default.
    pub fn configured(config: &PolicyConfig, name: &str) -> Mode {
        let class_mode = |class: ActionClass| {
            config
                .classes
                .get(class.name())
                .copied()
                .unwrap_or(class.default_mode())
        };
        if let Some(class) = ActionClass::parse(name) {
            return class_mode(class);
        }
        config
            .actions
            .get(name)
            .copied()
            .or_else(|| ActionClass::of(name).map(class_mode))
            .unwrap_or(Mode::Deny)
    }

    /// One line for the model: each class and action that is not `allow`,
    /// then what golem changed. `None` when everything is allowed.
    pub fn summary_line(&self) -> Option<String> {
        let mut parts: Vec<String> = ActionClass::ALL
            .into_iter()
            .filter(|c| self.class_mode(*c) != Mode::Allow)
            .map(|c| format!("{} {}", c.name(), self.class_mode(c).name()))
            .collect();
        parts.extend(
            self.actions
                .iter()
                .filter(|(a, m)| ActionClass::of(a).map(|c| self.class_mode(c)) != Some(**m))
                .map(|(a, m)| format!("{a} {}", m.name())),
        );
        if parts.is_empty() && self.notes.is_empty() {
            return None;
        }
        let mut line = format!("policy: {}", parts.join(", "));
        for note in &self.notes {
            line.push_str(&format!(" · {note}"));
        }
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_has_a_class() {
        let missing: Vec<&str> = crate::validation::known_actions()
            .iter()
            .copied()
            .filter(|a| ActionClass::of(a).is_none())
            .collect();
        assert!(missing.is_empty(), "actions with no class: {missing:?}");
    }

    #[test]
    fn the_defaults_ask_for_shell_and_http_and_allow_the_rest() {
        let p = Policy::default();
        assert_eq!(p.mode("bash"), Mode::Ask);
        assert_eq!(p.mode("post_http"), Mode::Ask);
        assert_eq!(p.mode("run"), Mode::Allow);
        assert_eq!(p.mode("browse_execute_js"), Mode::Allow);
        assert_eq!(p.mode("tap"), Mode::Allow);
        assert_eq!(
            p.summary_line().as_deref(),
            Some("policy: http ask, shell ask")
        );
        assert!(p.notes().is_empty());
    }

    #[test]
    fn an_action_setting_wins_over_its_class() {
        let config = PolicyConfig::from_toml(
            "[mcp.policy]\nhttp = \"allow\"\n[mcp.policy.actions]\ndelete_http = \"deny\"\n",
        )
        .expect("config");
        let p = Policy::from_user(&config);
        assert_eq!(p.mode("get_http"), Mode::Allow);
        assert_eq!(p.mode("delete_http"), Mode::Deny);
    }

    #[test]
    fn shell_is_never_looser_than_another_class_or_action() {
        let config = PolicyConfig::from_toml(
            "[mcp.policy]\nshell = \"allow\"\nhttp = \"allow\"\nscript = \"ask\"\n",
        )
        .expect("config");
        let p = Policy::from_user(&config);
        assert_eq!(
            p.mode("bash"),
            Mode::Ask,
            "script is ask, so shell SHALL be ask"
        );
        assert_eq!(
            p.mode("run"),
            Mode::Ask,
            "script SHALL keep the stricter value"
        );
        assert_eq!(
            p.notes(),
            ["shell: allow → ask (script is ask, and bash can do its work)"]
        );
        let by_action = Policy::from_user(
            &PolicyConfig::from_toml(
                "[mcp.policy]\nshell = \"allow\"\nhttp = \"allow\"\n[mcp.policy.actions]\nrun = \"deny\"\nbash = \"allow\"\n",
            )
            .expect("config"),
        );
        assert_eq!(by_action.mode("bash"), Mode::Deny);
        let looser_run = Policy::from_user(
            &PolicyConfig::from_toml("[mcp.policy]\nscript = \"allow\"\nshell = \"ask\"\n")
                .expect("config"),
        );
        assert_eq!(
            looser_run.mode("run"),
            Mode::Allow,
            "run allow with bash ask is valid"
        );
        assert!(looser_run.notes().is_empty());
    }

    #[test]
    fn a_project_or_an_argument_only_tightens() {
        let mut p = Policy::from_user(
            &PolicyConfig::from_toml("[mcp.policy]\nshell = \"allow\"\nhttp = \"allow\"\n")
                .expect("config"),
        );
        assert_eq!(p.mode("bash"), Mode::Allow);
        p.tighten(
            &PolicyConfig::from_toml("[mcp.policy]\nshell = \"allow\"\nemail = \"deny\"\n")
                .expect("project"),
        );
        assert_eq!(p.mode("bash"), Mode::Deny, "email deny SHALL raise shell");
        assert_eq!(p.mode("create_inbox"), Mode::Deny);
        p.tighten(&PolicyConfig::from_toml("[mcp.policy]\nemail = \"allow\"\n").expect("project"));
        assert_eq!(
            p.mode("create_inbox"),
            Mode::Deny,
            "tighten SHALL NOT loosen"
        );
    }

    #[test]
    fn a_policy_names_only_real_classes_and_actions() {
        let err =
            PolicyConfig::from_toml("[mcp.policy]\nshel = \"deny\"\n").expect_err("unknown class");
        assert!(err.to_string().contains("\"shel\""), "{err}");
        let err = PolicyConfig::from_toml("[mcp.policy.actions]\nbsh = \"deny\"\n")
            .expect_err("unknown action");
        assert!(err.to_string().contains("\"bsh\""), "{err}");
        assert_eq!(
            PolicyConfig::from_toml("[apps]\n").expect("no policy"),
            PolicyConfig::default()
        );
    }

    #[test]
    fn the_configured_mode_ignores_the_raised_shell() {
        let config = PolicyConfig::from_toml("[mcp.policy]\nhttp = \"deny\"\nshell = \"allow\"\n")
            .expect("config");
        assert_eq!(Policy::configured(&config, "shell"), Mode::Allow);
        assert_eq!(Policy::from_user(&config).mode("bash"), Mode::Deny);
        assert_eq!(Policy::configured(&config, "run"), Mode::Allow);
        assert_eq!(
            Policy::configured(&PolicyConfig::default(), "bash"),
            Mode::Ask
        );
    }
}
