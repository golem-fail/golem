// golem-parser: test file parser

pub mod config;
pub mod fixture;
pub mod mixin;
pub mod validation;

use serde::Deserialize;
use std::collections::HashMap;

/// Parse a TOML string into a FlowFile.
pub fn parse_flow(toml_str: &str) -> anyhow::Result<FlowFile> {
    let flow_file: FlowFile = toml::from_str(toml_str)?;
    Ok(flow_file)
}

#[derive(Deserialize, Debug, Clone)]
pub struct FlowFile {
    pub flow: FlowMeta,
    #[serde(default)]
    pub block: Vec<Block>,
    #[serde(default)]
    pub data: Vec<HashMap<String, String>>,
    #[serde(default)]
    pub teardown: Vec<TeardownBlock>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TeardownBlock {
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct FlowMeta {
    pub name: String,
    pub start: Option<String>,
    pub seed: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub vars: HashMap<String, String>,
    #[serde(default)]
    pub apps: Vec<AppConfig>,
    pub options: Option<FlowOptions>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AppConfig {
    pub name: String,
    pub bundle: String,
    #[serde(default)]
    pub devices: Vec<DeviceConstraint>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct DeviceConstraint {
    pub os: Option<StringOrVec>,
    #[serde(rename = "type")]
    pub device_type: Option<StringOrVec>,
    pub name: Option<String>,
    pub accessibility_id: Option<String>,
    pub physical: Option<bool>,
    pub playstore: Option<bool>,
    pub expand: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum StringOrVec {
    Single(String),
    Multiple(Vec<String>),
}

impl StringOrVec {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            StringOrVec::Single(s) => vec![s.clone()],
            StringOrVec::Multiple(v) => v.clone(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct Block {
    pub name: Option<String>,
    pub app: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
    pub next: Option<String>,
    #[serde(default)]
    pub branch: Vec<BranchCondition>,
    pub for_each: Option<String>,
    pub r#where: Option<DeviceFilter>,
    pub run_flow: Option<String>,
    pub max_iterations: Option<u64>,
    #[serde(default)]
    pub vars: HashMap<String, String>,
    #[serde(default)]
    pub save_to: HashMap<String, String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct DeviceFilter {
    #[serde(rename = "type")]
    pub device_type: Option<String>,
    pub os: Option<String>,
    pub physical: Option<bool>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BranchCondition {
    pub if_visible: Option<String>,
    pub if_not_visible: Option<String>,
    pub if_var: Option<String>,
    pub equals: Option<String>,
    pub matches: Option<String>,
    pub gte: Option<i64>,
    pub goto: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Step {
    pub action: String,
    pub text: Option<String>,
    pub accessibility_id: Option<String>,
    #[serde(rename = "type")]
    pub element_type: Option<String>,
    pub index: Option<usize>,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
    pub clickable: Option<bool>,
    pub below: Option<String>,
    pub above: Option<String>,
    pub right_of: Option<String>,
    pub left_of: Option<String>,
    pub child_of: Option<String>,
    pub placeholder: Option<String>,
    pub on_fail: Option<String>,
    pub save_to: Option<String>,
    pub timeout: Option<u64>,
    pub retry: Option<u32>,
    pub retry_delay: Option<u64>,
    pub app: Option<String>,
    pub auto_scroll: Option<bool>,
    #[serde(flatten)]
    pub params: HashMap<String, toml::Value>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct FlowOptions {
    pub max_concurrency: Option<u32>,
    pub min_free_ram_mb: Option<u64>,
    pub create_if_missing: Option<bool>,
    pub ignore_missing_physical: Option<bool>,
    pub step_timeout: Option<u64>,
    pub screenshot_on_failure: Option<bool>,
    pub screenshot_dir: Option<String>,
    pub recording_dir: Option<String>,
    pub record: Option<bool>,
    pub max_steps: Option<u64>,
    pub max_runtime: Option<String>,
    pub suite_concurrency: Option<u32>,
    pub keep_devices: Option<bool>,
    /// App lifecycle management before flow execution.
    /// - `"reset"` — stop + launch (fresh state). Default for top-level flows.
    /// - `"launch"` — launch if not running. Default for subflows.
    /// - `"manual"` — do nothing; flow manages its own lifecycle.
    pub app_lifecycle: Option<AppLifecycle>,
}

/// Controls how the runner manages the app before executing a flow.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AppLifecycle {
    /// Stop and relaunch the app for fresh state.
    Reset,
    /// Launch the app if not already running; no-op if running.
    Launch,
    /// Do nothing — the flow manages its own app lifecycle.
    Manual,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // 1. Minimal valid flow
    // ---------------------------------------------------------------
    #[test]
    fn minimal_valid_flow() {
        let toml_str = r#"
[flow]
name = "minimal test"

[[flow.apps]]
name = "myapp"
bundle = "com.example.app"

[[flow.apps.devices]]
os = "android"

[[block]]
name = "first"

[[block.steps]]
action = "tap"
text = "OK"
"#;
        let flow = parse_flow(toml_str).expect("minimal valid flow should parse");
        assert_eq!(flow.flow.name, "minimal test");
        assert_eq!(flow.flow.apps.len(), 1);
        assert_eq!(flow.flow.apps[0].bundle, "com.example.app");
        assert_eq!(flow.flow.apps[0].devices.len(), 1);
        assert_eq!(flow.block.len(), 1);
        assert_eq!(flow.block[0].steps.len(), 1);
        assert_eq!(flow.block[0].steps[0].action, "tap");
    }

    // ---------------------------------------------------------------
    // 2. All step actions parse
    // ---------------------------------------------------------------
    #[test]
    fn all_step_actions_parse() {
        let toml_str = r#"
[flow]
name = "actions test"

[[block]]
name = "actions"

[[block.steps]]
action = "tap"

[[block.steps]]
action = "long_press"

[[block.steps]]
action = "type_text"

[[block.steps]]
action = "swipe"

[[block.steps]]
action = "scroll"

[[block.steps]]
action = "wait"

[[block.steps]]
action = "assert_visible"

[[block.steps]]
action = "assert_not_visible"

[[block.steps]]
action = "back"

[[block.steps]]
action = "screenshot"

[[block.steps]]
action = "launch"

[[block.steps]]
action = "clear_text"
"#;
        let flow = parse_flow(toml_str).expect("all actions should parse");
        let actions: Vec<&str> = flow.block[0].steps.iter().map(|s| s.action.as_str()).collect();
        assert_eq!(
            actions,
            vec![
                "tap",
                "long_press",
                "type_text",
                "swipe",
                "scroll",
                "wait",
                "assert_visible",
                "assert_not_visible",
                "back",
                "screenshot",
                "launch",
                "clear_text",
            ]
        );
    }

    // ---------------------------------------------------------------
    // 3. Compact vs verbose step syntax
    // ---------------------------------------------------------------
    #[test]
    fn compact_vs_verbose_step_syntax() {
        let compact_toml = r#"
[flow]
name = "compact"

[[block]]
name = "b1"
steps = [{action = "tap", text = "OK"}, {action = "wait"}]
"#;
        let verbose_toml = r#"
[flow]
name = "verbose"

[[block]]
name = "b1"

[[block.steps]]
action = "tap"
text = "OK"

[[block.steps]]
action = "wait"
"#;
        let compact = parse_flow(compact_toml).expect("compact syntax should parse");
        let verbose = parse_flow(verbose_toml).expect("verbose syntax should parse");

        assert_eq!(compact.block[0].steps.len(), verbose.block[0].steps.len());
        for (c, v) in compact.block[0]
            .steps
            .iter()
            .zip(verbose.block[0].steps.iter())
        {
            assert_eq!(c.action, v.action);
            assert_eq!(c.text, v.text);
        }
    }

    // ---------------------------------------------------------------
    // 4. Flow-level tags
    // ---------------------------------------------------------------
    #[test]
    fn flow_level_tags() {
        let toml_str = r#"
[flow]
name = "tagged"
tags = ["smoke", "critical"]
"#;
        let flow = parse_flow(toml_str).expect("tags should parse");
        assert_eq!(flow.flow.tags, vec!["smoke", "critical"]);
    }

    // ---------------------------------------------------------------
    // 5. Multiple apps with devices
    // ---------------------------------------------------------------
    #[test]
    fn multiple_apps_with_devices() {
        let toml_str = r#"
[flow]
name = "multi app"

[[flow.apps]]
name = "app1"
bundle = "com.example.one"

[[flow.apps.devices]]
os = "android"

[[flow.apps]]
name = "app2"
bundle = "com.example.two"

[[flow.apps.devices]]
os = "ios:17"
"#;
        let flow = parse_flow(toml_str).expect("multiple apps should parse");
        assert_eq!(flow.flow.apps.len(), 2);
        assert_eq!(flow.flow.apps[0].name, "app1");
        assert_eq!(flow.flow.apps[0].bundle, "com.example.one");
        assert_eq!(flow.flow.apps[1].name, "app2");
        assert_eq!(flow.flow.apps[1].bundle, "com.example.two");
        assert_eq!(flow.flow.apps[0].devices.len(), 1);
        assert_eq!(flow.flow.apps[1].devices.len(), 1);
    }

    // ---------------------------------------------------------------
    // 6. Block with branching
    // ---------------------------------------------------------------
    #[test]
    fn block_with_branching() {
        let toml_str = r#"
[flow]
name = "branching"

[[block]]
name = "check"

[[block.branch]]
if_visible = "Welcome"
goto = "dashboard"

[[block.branch]]
if_var = "retry_count"
gte = 3
goto = "fail_block"

[[block.branch]]
goto = "login"
"#;
        let flow = parse_flow(toml_str).expect("branching should parse");
        let branches = &flow.block[0].branch;
        assert_eq!(branches.len(), 3);

        assert_eq!(branches[0].if_visible.as_deref(), Some("Welcome"));
        assert_eq!(branches[0].goto, "dashboard");

        assert_eq!(branches[1].if_var.as_deref(), Some("retry_count"));
        assert_eq!(branches[1].gte, Some(3));
        assert_eq!(branches[1].goto, "fail_block");

        // default branch: no conditions, just goto
        assert!(branches[2].if_visible.is_none());
        assert!(branches[2].if_not_visible.is_none());
        assert!(branches[2].if_var.is_none());
        assert_eq!(branches[2].goto, "login");
    }

    // ---------------------------------------------------------------
    // 7. Block with all optional fields
    // ---------------------------------------------------------------
    #[test]
    fn block_with_all_optional_fields() {
        let toml_str = r#"
[flow]
name = "full block"

[[block]]
name = "login"
app = "myapp"
next = "dashboard"
for_each = "users"
max_iterations = 10

[block.where]
type = "phone"
os = "android"
physical = true

[block.vars]
username = "admin"
password = "secret"

[[block.steps]]
action = "tap"
"#;
        let flow = parse_flow(toml_str).expect("full block should parse");
        let block = &flow.block[0];
        assert_eq!(block.name.as_deref(), Some("login"));
        assert_eq!(block.app.as_deref(), Some("myapp"));
        assert_eq!(block.next.as_deref(), Some("dashboard"));
        assert_eq!(block.for_each.as_deref(), Some("users"));
        assert_eq!(block.max_iterations, Some(10));

        let w = block.r#where.as_ref().expect("where should be present");
        assert_eq!(w.device_type.as_deref(), Some("phone"));
        assert_eq!(w.os.as_deref(), Some("android"));
        assert_eq!(w.physical, Some(true));

        assert_eq!(block.vars.get("username").map(|s| s.as_str()), Some("admin"));
        assert_eq!(block.vars.get("password").map(|s| s.as_str()), Some("secret"));
    }

    // ---------------------------------------------------------------
    // 8. Step with selectors and filters
    // ---------------------------------------------------------------
    #[test]
    fn step_with_selectors_and_filters() {
        let toml_str = r#"
[flow]
name = "selectors"

[[block]]

[[block.steps]]
action = "tap"
text = "Submit"
accessibility_id = "btn_submit"
type = "Button"
index = 2
enabled = true
below = "Header"
child_of = "FormContainer"
placeholder = "Enter name"
"#;
        let flow = parse_flow(toml_str).expect("selectors should parse");
        let step = &flow.block[0].steps[0];
        assert_eq!(step.text.as_deref(), Some("Submit"));
        assert_eq!(step.accessibility_id.as_deref(), Some("btn_submit"));
        assert_eq!(step.element_type.as_deref(), Some("Button"));
        assert_eq!(step.index, Some(2));
        assert_eq!(step.enabled, Some(true));
        assert_eq!(step.below.as_deref(), Some("Header"));
        assert_eq!(step.child_of.as_deref(), Some("FormContainer"));
        assert_eq!(step.placeholder.as_deref(), Some("Enter name"));
    }

    // ---------------------------------------------------------------
    // 9. Step with behavior fields
    // ---------------------------------------------------------------
    #[test]
    fn step_with_behavior_fields() {
        let toml_str = r#"
[flow]
name = "behavior"

[[block]]

[[block.steps]]
action = "tap"
on_fail = "skip"
save_to = "result_var"
timeout = 5000
retry = 3
retry_delay = 1000
"#;
        let flow = parse_flow(toml_str).expect("behavior fields should parse");
        let step = &flow.block[0].steps[0];
        assert_eq!(step.on_fail.as_deref(), Some("skip"));
        assert_eq!(step.save_to.as_deref(), Some("result_var"));
        assert_eq!(step.timeout, Some(5000));
        assert_eq!(step.retry, Some(3));
        assert_eq!(step.retry_delay, Some(1000));
    }

    // ---------------------------------------------------------------
    // 10. Flow options
    // ---------------------------------------------------------------
    #[test]
    fn flow_options() {
        let toml_str = r#"
[flow]
name = "options test"

[flow.options]
max_concurrency = 4
min_free_ram_mb = 512
step_timeout = 30000
screenshot_on_failure = true
screenshot_dir = "/tmp/shots"
recording_dir = "/tmp/recordings"
record = true
max_steps = 200
max_runtime = "30m"
suite_concurrency = 2
keep_devices = false
create_if_missing = true
ignore_missing_physical = false
"#;
        let flow = parse_flow(toml_str).expect("options should parse");
        let opts = flow.flow.options.expect("options should be present");
        assert_eq!(opts.max_concurrency, Some(4));
        assert_eq!(opts.min_free_ram_mb, Some(512));
        assert_eq!(opts.step_timeout, Some(30000));
        assert_eq!(opts.screenshot_on_failure, Some(true));
        assert_eq!(opts.screenshot_dir.as_deref(), Some("/tmp/shots"));
        assert_eq!(opts.recording_dir.as_deref(), Some("/tmp/recordings"));
        assert_eq!(opts.record, Some(true));
        assert_eq!(opts.max_steps, Some(200));
        assert_eq!(opts.max_runtime.as_deref(), Some("30m"));
        assert_eq!(opts.suite_concurrency, Some(2));
        assert_eq!(opts.keep_devices, Some(false));
        assert_eq!(opts.create_if_missing, Some(true));
        assert_eq!(opts.ignore_missing_physical, Some(false));
    }

    // ---------------------------------------------------------------
    // 11. Data-driven rows
    // ---------------------------------------------------------------
    #[test]
    fn data_driven_rows() {
        let toml_str = r#"
[flow]
name = "data driven"

[[data]]
username = "alice"
password = "pass1"

[[data]]
username = "bob"
password = "pass2"

[[data]]
username = "charlie"
password = "pass3"
"#;
        let flow = parse_flow(toml_str).expect("data rows should parse");
        assert_eq!(flow.data.len(), 3);
        assert_eq!(flow.data[0].get("username").map(|s| s.as_str()), Some("alice"));
        assert_eq!(flow.data[1].get("username").map(|s| s.as_str()), Some("bob"));
        assert_eq!(flow.data[2].get("username").map(|s| s.as_str()), Some("charlie"));
        assert_eq!(flow.data[2].get("password").map(|s| s.as_str()), Some("pass3"));
    }

    // ---------------------------------------------------------------
    // 12. Seed in flow
    // ---------------------------------------------------------------
    #[test]
    fn seed_in_flow() {
        let toml_str = r#"
[flow]
name = "seeded"
seed = 847291036
"#;
        let flow = parse_flow(toml_str).expect("seed should parse");
        assert_eq!(flow.flow.seed, Some(847_291_036));
    }

    // ---------------------------------------------------------------
    // 13. Start block
    // ---------------------------------------------------------------
    #[test]
    fn start_block() {
        let toml_str = r#"
[flow]
name = "with start"
start = "login"
"#;
        let flow = parse_flow(toml_str).expect("start should parse");
        assert_eq!(flow.flow.start.as_deref(), Some("login"));
    }

    // ---------------------------------------------------------------
    // 14. Missing flow section — error
    // ---------------------------------------------------------------
    #[test]
    fn missing_flow_section_error() {
        let toml_str = r#"
[[block]]
name = "orphan"

[[block.steps]]
action = "tap"
"#;
        let result = parse_flow(toml_str);
        assert!(result.is_err(), "missing [flow] SHALL produce an error");
    }

    // ---------------------------------------------------------------
    // 15. Missing app bundle — error
    // ---------------------------------------------------------------
    #[test]
    fn missing_app_bundle_error() {
        let toml_str = r#"
[flow]
name = "bad app"

[[flow.apps]]
name = "no_bundle"
"#;
        let result = parse_flow(toml_str);
        assert!(result.is_err(), "missing bundle SHALL produce an error");
    }

    // ---------------------------------------------------------------
    // 16. Teardown block parses
    // ---------------------------------------------------------------
    #[test]
    fn teardown_block_parses() {
        let toml_str = r#"
[flow]
name = "with teardown"

[[teardown]]

[[teardown.steps]]
action = "back"

[[teardown.steps]]
action = "screenshot"
"#;
        let flow = parse_flow(toml_str).expect("teardown should parse");
        assert_eq!(flow.teardown.len(), 1);
        assert_eq!(flow.teardown[0].steps.len(), 2);
        assert_eq!(flow.teardown[0].steps[0].action, "back");
        assert_eq!(flow.teardown[0].steps[1].action, "screenshot");
    }

    // ---------------------------------------------------------------
    // 17. No teardown — optional
    // ---------------------------------------------------------------
    #[test]
    fn no_teardown_optional() {
        let toml_str = r#"
[flow]
name = "no teardown"
"#;
        let flow = parse_flow(toml_str).expect("no teardown should parse");
        assert!(flow.teardown.is_empty());
    }

    // ---------------------------------------------------------------
    // 18. Block with run_flow
    // ---------------------------------------------------------------
    #[test]
    fn block_with_run_flow() {
        let toml_str = r#"
[flow]
name = "delegating"

[[block]]
name = "helper"
run_flow = "helper.test.toml"
"#;
        let flow = parse_flow(toml_str).expect("run_flow should parse");
        assert_eq!(
            flow.block[0].run_flow.as_deref(),
            Some("helper.test.toml")
        );
    }

    // ---------------------------------------------------------------
    // 19. DeviceConstraint with string os
    // ---------------------------------------------------------------
    #[test]
    fn device_constraint_string_os() {
        let toml_str = r#"
[flow]
name = "string os"

[[flow.apps]]
name = "app1"
bundle = "com.example"

[[flow.apps.devices]]
os = "ios:18"
"#;
        let flow = parse_flow(toml_str).expect("string os should parse");
        let os = flow.flow.apps[0].devices[0]
            .os
            .as_ref()
            .expect("os should be present");
        match os {
            StringOrVec::Single(s) => assert_eq!(s, "ios:18"),
            StringOrVec::Multiple(_) => panic!("expected Single variant"),
        }
    }

    // ---------------------------------------------------------------
    // 20. DeviceConstraint with array os
    // ---------------------------------------------------------------
    #[test]
    fn device_constraint_array_os() {
        let toml_str = r#"
[flow]
name = "array os"

[[flow.apps]]
name = "app1"
bundle = "com.example"

[[flow.apps.devices]]
os = ["ios:17", "ios:18"]
"#;
        let flow = parse_flow(toml_str).expect("array os should parse");
        let os = flow.flow.apps[0].devices[0]
            .os
            .as_ref()
            .expect("os should be present");
        match os {
            StringOrVec::Multiple(v) => {
                assert_eq!(v, &["ios:17", "ios:18"]);
            }
            StringOrVec::Single(_) => panic!("expected Multiple variant"),
        }
    }

    // ---------------------------------------------------------------
    // 21. Step params flatten
    // ---------------------------------------------------------------
    #[test]
    fn step_params_flatten() {
        let toml_str = r#"
[flow]
name = "params"

[[block]]

[[block.steps]]
action = "swipe"
direction = "down"
duration = 500
"#;
        let flow = parse_flow(toml_str).expect("params flatten should parse");
        let step = &flow.block[0].steps[0];
        assert_eq!(step.action, "swipe");
        assert_eq!(
            step.params.get("direction"),
            Some(&toml::Value::String("down".to_string()))
        );
        assert_eq!(
            step.params.get("duration"),
            Some(&toml::Value::Integer(500))
        );
    }

    // ---------------------------------------------------------------
    // 22. Empty flow (blocks optional)
    // ---------------------------------------------------------------
    #[test]
    fn empty_flow_blocks_optional() {
        let toml_str = r#"
[flow]
name = "empty"
"#;
        let flow = parse_flow(toml_str).expect("empty flow should parse");
        assert!(flow.block.is_empty());
        assert!(flow.data.is_empty());
        assert!(flow.teardown.is_empty());
    }
}
