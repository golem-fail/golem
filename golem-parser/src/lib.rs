// golem-parser: test file parser

use serde::Deserialize;
use std::collections::HashMap;

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
    pub id: Option<String>,
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
    pub id: Option<String>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_flow_parses() {
        let toml_str = r#"
            [flow]
            name = "minimal test"
        "#;
        let flow: FlowFile =
            toml::from_str(toml_str).expect("minimal flow TOML should parse");
        assert_eq!(flow.flow.name, "minimal test");
        assert!(flow.block.is_empty());
        assert!(flow.data.is_empty());
        assert!(flow.teardown.is_empty());
    }

    #[test]
    fn string_or_vec_single_and_multiple() {
        // Single string
        let toml_str = r#"
            [flow]
            name = "sov test"

            [[flow.apps]]
            name = "app1"
            bundle = "com.example"

            [[flow.apps.devices]]
            os = "android"
            type = ["phone", "tablet"]
        "#;
        let flow: FlowFile =
            toml::from_str(toml_str).expect("StringOrVec TOML should parse");
        let device = &flow.flow.apps[0].devices[0];

        let os = device.os.as_ref().expect("os should be present");
        assert_eq!(os.to_vec(), vec!["android"]);

        let dtype = device.device_type.as_ref().expect("type should be present");
        assert_eq!(dtype.to_vec(), vec!["phone", "tablet"]);
    }

    #[test]
    fn step_flatten_captures_extra_fields() {
        let toml_str = r#"
            [flow]
            name = "flatten test"

            [[block]]
            name = "b1"

            [[block.steps]]
            action = "tap"
            text = "OK"
            custom_field = "custom_value"
            another = 42
        "#;
        let flow: FlowFile =
            toml::from_str(toml_str).expect("flatten params TOML should parse");
        let step = &flow.block[0].steps[0];
        assert_eq!(step.action, "tap");
        assert_eq!(
            step.params.get("custom_field"),
            Some(&toml::Value::String("custom_value".to_string()))
        );
        assert_eq!(
            step.params.get("another"),
            Some(&toml::Value::Integer(42))
        );
    }

    #[test]
    fn all_optional_fields_can_be_omitted() {
        let toml_str = r#"
            [flow]
            name = "optional test"

            [[block]]

            [[block.steps]]
            action = "wait"
        "#;
        let flow: FlowFile =
            toml::from_str(toml_str).expect("optional fields TOML should parse");
        let block = &flow.block[0];
        assert!(block.name.is_none());
        assert!(block.app.is_none());
        assert!(block.next.is_none());
        assert!(block.for_each.is_none());
        assert!(block.r#where.is_none());
        assert!(block.run_flow.is_none());
        assert!(block.max_iterations.is_none());
        assert!(block.branch.is_empty());
        assert!(block.vars.is_empty());

        let step = &block.steps[0];
        assert_eq!(step.action, "wait");
        assert!(step.text.is_none());
        assert!(step.id.is_none());
        assert!(step.element_type.is_none());
        assert!(step.index.is_none());
        assert!(step.enabled.is_none());
        assert!(step.checked.is_none());
        assert!(step.clickable.is_none());
        assert!(step.below.is_none());
        assert!(step.above.is_none());
        assert!(step.right_of.is_none());
        assert!(step.left_of.is_none());
        assert!(step.child_of.is_none());
        assert!(step.placeholder.is_none());
        assert!(step.on_fail.is_none());
        assert!(step.save_to.is_none());
        assert!(step.timeout.is_none());
        assert!(step.retry.is_none());
        assert!(step.retry_delay.is_none());
        assert!(step.app.is_none());

        assert!(flow.flow.start.is_none());
        assert!(flow.flow.seed.is_none());
        assert!(flow.flow.options.is_none());
        assert!(flow.flow.tags.is_empty());
        assert!(flow.flow.vars.is_empty());
        assert!(flow.flow.apps.is_empty());
    }
}
