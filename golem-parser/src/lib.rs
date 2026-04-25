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
    /// Bundle id. Optional in source TOML — inherited from a matching
    /// project-level `[[apps]]` entry by name, if present. After
    /// project-merge, must be populated (validated elsewhere).
    #[serde(default)]
    pub bundle: Option<String>,
    #[serde(default)]
    pub devices: Vec<DeviceConstraint>,
    /// Optional install script. Either a single path (cross-platform) or a
    /// platform-keyed table: `{ ios = "...", android = "..." }`. When unset
    /// AND not provided by the project `[[apps]]` registry, golem skips the
    /// install step entirely and assumes the app is already installed.
    pub install_script: Option<InstallScriptValue>,
    /// Timeout in ms for the install script. Override default when this
    /// app's build is known to take longer than the default.
    pub install_timeout_ms: Option<u64>,
}

/// An install script path, either a single cross-platform script or a
/// platform-keyed map (e.g. separate iOS / Android scripts).
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum InstallScriptValue {
    /// One script handles all platforms (e.g. Tauri, Expo).
    Single(String),
    /// Separate scripts per platform. Missing keys = no script for that platform.
    PerPlatform(InstallScriptPerPlatform),
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct InstallScriptPerPlatform {
    pub ios: Option<String>,
    pub android: Option<String>,
}

impl InstallScriptValue {
    /// Resolve to a script path for the given platform, or `None` if no
    /// script is configured for that platform.
    pub fn for_platform(&self, platform: &str) -> Option<&str> {
        match self {
            InstallScriptValue::Single(s) => Some(s.as_str()),
            InstallScriptValue::PerPlatform(p) => match platform {
                "ios" => p.ios.as_deref(),
                "android" => p.android.as_deref(),
                _ => None,
            },
        }
    }
}

/// Project-level app definition (`[[apps]]` in golem.toml).
///
/// Lets a project declare each app once — name, bundle, install_script,
/// optional timeout, optional device defaults — so flows can reference by
/// name and inherit:
///
/// ```toml
/// # golem.toml
/// [[apps]]
/// name = "app-b"
/// bundle = "fail.golem.testb"
/// install_script = { ios = "scripts/install-app-b-ios.sh", android = "scripts/install-app-b-android.sh" }
/// install_timeout_ms = 900000   # 15 min
///
/// # flow.toml
/// [[flow.apps]]
/// name = "app-b"        # inherits bundle + install_script from golem.toml
/// ```
///
/// Flow-level fields override matching project-level ones.
#[derive(Deserialize, Debug, Clone)]
pub struct ProjectAppConfig {
    pub name: String,
    pub bundle: Option<String>,
    #[serde(default)]
    pub devices: Vec<DeviceConstraint>,
    pub install_script: Option<InstallScriptValue>,
    pub install_timeout_ms: Option<u64>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct DeviceConstraint {
    pub os: Option<StringOrVec>,
    #[serde(rename = "type")]
    pub device_type: Option<StringOrVec>,
    pub name: Option<String>,
    pub accessibility_label: Option<String>,
    /// Hardware kind axis. Values: `"virtual"` (sim/emulator) or `"real"`
    /// (physical device). Supports array form for coverage — `["virtual",
    /// "real"]` emits two tick boxes, satisfiable independently. Omitted
    /// means virtual-only (safest default — physical devices require
    /// explicit opt-in).
    pub hardware: Option<StringOrVec>,
    pub playstore: Option<bool>,
    /// If set, only match devices in the requested boot state. Mostly used
    /// internally when a flow has no `[[flow.apps.devices]]` block: the plan
    /// emits one partial tick box per currently-booted platform with
    /// `booted = Some(true)` so the suite runs on whatever's already up.
    pub booted: Option<bool>,
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

/// An anchor for relational selectors — either a text pattern or a nested selector group.
///
/// Deserializes from either a plain string or an inline table:
/// ```toml
/// right_of = "Orientation:"                          # text pattern
/// right_of = { text = "Orientation:", enabled = true } # nested selector
/// ```
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Anchor {
    Text(String),
    Selector(Box<SelectorGroup>),
}

/// Grouped selector for `on = { ... }` / `to = { ... }` syntax.
///
/// All fields match the flat `on_*` fields on Step but without the prefix.
/// Relational fields (below, above, right_of, left_of) accept either a
/// text pattern or a nested selector group.
/// A coordinate value: either an absolute pixel or a percentage string.
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum CoordValue {
    Pixels(i32),
    Percent(String), // e.g. "50%"
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct SelectorGroup {
    pub text: Option<String>,
    pub accessibility_label: Option<String>,
    pub index: Option<usize>,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
    pub clickable: Option<bool>,
    pub below: Option<Anchor>,
    pub above: Option<Anchor>,
    pub right_of: Option<Anchor>,
    pub left_of: Option<Anchor>,
    /// Observable traits: ["button", "has_text", "square"], etc.
    #[serde(default)]
    pub traits: Vec<String>,
    /// X coordinate: absolute pixels, percentage of screen/element, or offset from center.
    pub x: Option<CoordValue>,
    /// Y coordinate: same as x.
    pub y: Option<CoordValue>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct Step {
    #[serde(default)]
    pub action: String,
    // Flat selectors (on_* prefix)
    pub on_text: Option<String>,
    pub on_accessibility_label: Option<String>,
    pub on_index: Option<usize>,
    pub on_enabled: Option<bool>,
    pub on_checked: Option<bool>,
    pub on_clickable: Option<bool>,
    pub on_below: Option<String>,
    pub on_above: Option<String>,
    pub on_right_of: Option<String>,
    pub on_left_of: Option<String>,
    // Grouped selector — accepts both `on = {}` and `to = {}` in TOML
    #[serde(alias = "to")]
    pub on: Option<SelectorGroup>,
    /// Value to type into an input field. Used by the `type` action.
    /// Separates the typed value from `text` which is always a selector.
    pub input: Option<String>,
    pub if_fail: Option<String>,
    pub save_to: Option<String>,
    pub timeout: Option<u64>,
    pub retry: Option<u32>,
    pub retry_delay: Option<u64>,
    pub app: Option<String>,
    pub restart: Option<bool>,
    pub auto_scroll: Option<bool>,
    pub max_scrolls: Option<u32>,
    pub scroll_timeout: Option<u64>,
    /// Constrain scrolling to within a specific element's bounds.
    pub within: Option<SelectorGroup>,
    /// Swipe start position (selector with optional coordinates).
    pub start: Option<SelectorGroup>,
    /// Swipe end position (selector with optional coordinates).
    pub end: Option<SelectorGroup>,
    /// Gesture path: array of points the finger travels through.
    /// start prepends, end appends. Internally everything becomes a path.
    /// Named `points` in TOML to avoid conflict with screenshot `path`.
    #[serde(default, alias = "points")]
    pub points: Vec<SelectorGroup>,
    /// Swipe/gesture duration in milliseconds (total or per-segment).
    pub duration: Option<u64>,
    /// Pinch scale factor (>1.0 = zoom in, <1.0 = zoom out).
    pub scale: Option<f64>,
    /// Rotation in degrees (positive = clockwise, negative = counter-clockwise).
    pub rotation: Option<f64>,
    /// Gesture velocity (scale/sec for pinch, degrees/sec for rotate).
    pub velocity: Option<f64>,
    /// Multi-touch gesture: array of finger paths.
    #[serde(default)]
    pub fingers: Vec<Finger>,
    #[serde(flatten)]
    pub params: HashMap<String, toml::Value>,
}

/// A single finger path in a multi-touch gesture.
#[derive(Deserialize, Debug, Clone)]
pub struct Finger {
    /// Points the finger travels through (each is a full selector).
    pub points: Vec<SelectorGroup>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct FlowOptions {
    pub max_concurrency: Option<u32>,
    pub min_free_ram_mb: Option<u64>,
    pub min_free_disk_mb: Option<u64>,
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
    /// Coverage strategy — how to expand multi-valued device constraints
    /// into FlowRuns. `full` = Cartesian (every combo); `min` = smallest
    /// device set ticking every axis value; `smart` = execute-time
    /// adaptive, uses more devices when free (default); `one` = single
    /// run for local smoke testing.
    pub coverage: Option<CoverageStrategy>,
    /// App lifecycle management before flow execution.
    /// - `"reset"` — stop all apps + launch first app (fresh state). Default.
    /// - `"launch"` — launch first app if not running; preserves state.
    /// - `"manual"` — do nothing; flow manages its own lifecycle.
    pub app_lifecycle: Option<AppLifecycle>,
    /// Enable/disable automatic performance capture. Default: true.
    pub perf: Option<bool>,
    /// Memory warning threshold in MB.
    pub perf_memory_warn_mb: Option<f64>,
    /// Memory error threshold in MB.
    pub perf_memory_error_mb: Option<f64>,
    /// CPU warning threshold as percentage.
    pub perf_cpu_warn_percent: Option<f64>,
    /// CPU error threshold as percentage.
    pub perf_cpu_error_percent: Option<f64>,
    /// Thread count warning threshold.
    pub perf_threads_warn: Option<u32>,
    /// Thread count error threshold.
    pub perf_threads_error: Option<u32>,
    /// File descriptor warning threshold.
    pub perf_fd_warn: Option<u32>,
    /// File descriptor error threshold.
    pub perf_fd_error: Option<u32>,
}

/// How to expand multi-valued device constraints into FlowRuns.
///
/// `Smart` is the default: plan produces the same fully-pinned slot set
/// as `Min` (via greedy set-cover) but registers a `CoverageGroup` so the
/// scheduler terminates the group once every pool box has been ticked.
/// `One` is the same machinery with `max_runs = Some(1)` for local smoke.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CoverageStrategy {
    /// FlowRuns share a group with `max_runs = 1`; first successful run
    /// ends the group. For local smoke runs where any match satisfies.
    One,
    /// Plan-time greedy set-cover: fewest devices ticking every box.
    /// No group registered — every emitted FlowRun runs unconditionally.
    Min,
    /// Default. Same plan output as `Min`, plus a `CoverageGroup` the
    /// scheduler consults to stop dispatching members once every pool
    /// box has been ticked (direct + bonus ticks from picked devices).
    Smart,
    /// Cartesian product — every (os, type, …) combination as its own run.
    Full,
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
on_text = "OK"
"#;
        let flow = parse_flow(toml_str).expect("minimal valid flow should parse");
        assert_eq!(flow.flow.name, "minimal test");
        assert_eq!(flow.flow.apps.len(), 1);
        assert_eq!(flow.flow.apps[0].bundle.as_deref(), Some("com.example.app"));
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
steps = [{action = "tap", on_text = "OK"}, {action = "wait"}]
"#;
        let verbose_toml = r#"
[flow]
name = "verbose"

[[block]]
name = "b1"

[[block.steps]]
action = "tap"
on_text = "OK"

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
            assert_eq!(c.on_text, v.on_text);
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
        assert_eq!(flow.flow.apps[0].bundle.as_deref(), Some("com.example.one"));
        assert_eq!(flow.flow.apps[1].name, "app2");
        assert_eq!(flow.flow.apps[1].bundle.as_deref(), Some("com.example.two"));
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
on_text = "Submit"
on_accessibility_label = "btn_submit"
on_index = 2
on_enabled = true
on_below = "Header"
"#;
        let flow = parse_flow(toml_str).expect("selectors should parse");
        let step = &flow.block[0].steps[0];
        assert_eq!(step.on_text.as_deref(), Some("Submit"));
        assert_eq!(step.on_accessibility_label.as_deref(), Some("btn_submit"));
        assert_eq!(step.on_index, Some(2));
        assert_eq!(step.on_enabled, Some(true));
        assert_eq!(step.on_below.as_deref(), Some("Header"));
    }

    // ---------------------------------------------------------------
    // 8b. Grouped on = {} selector syntax
    // ---------------------------------------------------------------
    #[test]
    fn step_with_grouped_on_selector() {
        let toml_str = r#"
[flow]
name = "grouped"

[[block]]
steps = [
  { action = "tap", on = { text = "Submit", below = "Counter", enabled = true } },
]
"#;
        let flow = parse_flow(toml_str).expect("grouped on syntax should parse");
        let step = &flow.block[0].steps[0];
        let g = step.on.as_ref().expect("on group should be present");
        assert_eq!(g.text.as_deref(), Some("Submit"));
        assert!(matches!(&g.below, Some(Anchor::Text(s)) if s == "Counter"));
        assert_eq!(g.enabled, Some(true));
    }

    // ---------------------------------------------------------------
    // 8c. to = {} alias for grouped selector
    // ---------------------------------------------------------------
    #[test]
    fn step_with_to_alias() {
        let toml_str = r#"
[flow]
name = "to alias"

[[block]]
steps = [
  { action = "scroll", to = { text = "Item 49" } },
]
"#;
        let flow = parse_flow(toml_str).expect("to alias should parse");
        let step = &flow.block[0].steps[0];
        let g = step.on.as_ref().expect("to should populate on field");
        assert_eq!(g.text.as_deref(), Some("Item 49"));
    }

    // ---------------------------------------------------------------
    // 8d. Nested anchor selector
    // ---------------------------------------------------------------
    #[test]
    fn step_with_nested_anchor() {
        let toml_str = r#"
[flow]
name = "nested"

[[block]]
steps = [
  { action = "tap", on = { text = "Portrait", right_of = { text = "Orientation:", enabled = true } } },
]
"#;
        let flow = parse_flow(toml_str).expect("nested anchor should parse");
        let step = &flow.block[0].steps[0];
        let g = step.on.as_ref().expect("on group should be present");
        assert_eq!(g.text.as_deref(), Some("Portrait"));
        match &g.right_of {
            Some(Anchor::Selector(nested)) => {
                assert_eq!(nested.text.as_deref(), Some("Orientation:"));
                assert_eq!(nested.enabled, Some(true));
            }
            other => panic!("expected nested Anchor::Selector, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // 8e. Observable traits in grouped selector
    // ---------------------------------------------------------------
    #[test]
    fn step_with_traits() {
        let toml_str = r#"
[flow]
name = "traits"

[[block]]
steps = [
  { action = "tap", on = { text = "Submit", traits = ["button", "has_text"] } },
]
"#;
        let flow = parse_flow(toml_str).expect("traits should parse");
        let step = &flow.block[0].steps[0];
        let g = step.on.as_ref().expect("on group should be present");
        assert_eq!(g.traits, vec!["button", "has_text"]);
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
if_fail = "skip"
save_to = "result_var"
timeout = 5000
retry = 3
retry_delay = 1000
"#;
        let flow = parse_flow(toml_str).expect("behavior fields should parse");
        let step = &flow.block[0].steps[0];
        assert_eq!(step.if_fail.as_deref(), Some("skip"));
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
    fn missing_app_bundle_parses_as_none() {
        // bundle is parse-optional; it may be supplied by a project-level
        // [[apps]] entry in golem.toml. Missing-at-execution-time is
        // validated elsewhere (suite runner).
        let toml_str = r#"
[flow]
name = "no bundle"

[[flow.apps]]
name = "needs_bundle"
"#;
        let flow = parse_flow(toml_str).expect("missing bundle SHALL parse");
        assert_eq!(flow.flow.apps[0].bundle, None);
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
        assert_eq!(step.duration, Some(500));
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

    // ---------------------------------------------------------------
    // 23. Perf options — disabled
    // ---------------------------------------------------------------
    #[test]
    fn perf_disabled() {
        let toml_str = r#"
[flow]
name = "perf off"

[flow.options]
perf = false
"#;
        let flow = parse_flow(toml_str).expect("perf disabled should parse");
        let opts = flow.flow.options.expect("options should be present");
        assert_eq!(opts.perf, Some(false));
    }

    // ---------------------------------------------------------------
    // 24. Perf options — thresholds
    // ---------------------------------------------------------------
    #[test]
    fn perf_thresholds() {
        let toml_str = r#"
[flow]
name = "perf thresholds"

[flow.options]
perf_memory_warn_mb = 200.0
perf_memory_error_mb = 500.0
perf_cpu_warn_percent = 80.0
perf_cpu_error_percent = 95.0
perf_threads_warn = 100
perf_threads_error = 200
perf_fd_warn = 200
perf_fd_error = 500
"#;
        let flow = parse_flow(toml_str).expect("thresholds should parse");
        let opts = flow.flow.options.expect("options should be present");
        assert_eq!(opts.perf_memory_warn_mb, Some(200.0));
        assert_eq!(opts.perf_memory_error_mb, Some(500.0));
        assert_eq!(opts.perf_cpu_warn_percent, Some(80.0));
        assert_eq!(opts.perf_cpu_error_percent, Some(95.0));
        assert_eq!(opts.perf_threads_warn, Some(100));
        assert_eq!(opts.perf_threads_error, Some(200));
        assert_eq!(opts.perf_fd_warn, Some(200));
        assert_eq!(opts.perf_fd_error, Some(500));
    }

    // ---------------------------------------------------------------
    // 25. Perf options — defaults are None
    // ---------------------------------------------------------------
    #[test]
    fn perf_defaults_none() {
        let toml_str = r#"
[flow]
name = "no perf opts"

[flow.options]
max_steps = 100
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let opts = flow.flow.options.expect("options should be present");
        assert!(opts.perf.is_none());
        assert!(opts.perf_memory_warn_mb.is_none());
        assert!(opts.perf_cpu_warn_percent.is_none());
        assert!(opts.perf_threads_warn.is_none());
        assert!(opts.perf_fd_warn.is_none());
    }
}
