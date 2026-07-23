//! Parses golem's `.test.toml` flow files (and the `golem.toml` project
//! config, `__fixtures__/*.toml` fixtures, and `__mixins__/*.toml` mixins
//! that go with them) into the typed structures the rest of golem executes.
//!
//! The entry point is [`parse_flow`], which deserializes a TOML string into
//! a [`FlowFile`] — a tree of [`Block`]s, each a list of [`Step`]s (the
//! atomic tap/type/assert/scroll/... unit) plus optional branching. Element
//! targeting on a step goes through [`SelectorGroup`] — text/attribute
//! filters plus relational anchors (`below`, `right_of`, `contains`, ...).
//!
//! Sibling modules extend the format without touching this core model:
//! [`config`] loads and merges the project-level `golem.toml`, [`fixture`]
//! and [`mixin`] resolve and parse the `__fixtures__`/`__mixins__` file
//! conventions, and [`validation`] runs structural lints (unknown actions,
//! dangling `goto`s, ...) over an already-parsed [`FlowFile`]. This crate
//! only parses and validates — it has no knowledge of how a flow actually
//! runs; that lives in the runner/driver crates.

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

/// The top-level result of [`parse_flow`] — everything in one `.test.toml`
/// file. `block` holds the flow's execution graph (blocks link to each
/// other via `next`/`branch`/`goto`), `data` supplies data-driven rows
/// (each row's key/value pairs are substituted per-iteration), and
/// `teardown` is a "finally"-style block list run after the flow regardless
/// of pass/fail.
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

/// A list of steps run after the flow finishes (success or failure), e.g.
/// to clean up state. See `FlowFile::teardown` and the project-level
/// equivalent in [`config::ProjectConfig::teardown`].
#[derive(Deserialize, Debug, Clone)]
pub struct TeardownBlock {
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// The `[flow]` section: name, entry point, and flow-wide settings.
#[derive(Deserialize, Debug, Clone)]
pub struct FlowMeta {
    pub name: String,
    pub start: Option<String>,
    pub seed: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// When `true`, this flow is skipped by the tag-less discovery sweep
    /// (bare `golem run` / `golem run <dir>`). It still runs when its path is
    /// given directly or when a `--tag` filter matches. Used for subflows —
    /// flows meant to run as a `run_flow` child, where a standalone run in the
    /// bulk sweep would be redundant.
    #[serde(default)]
    pub explicit_only: bool,
    #[serde(default)]
    pub vars: HashMap<String, String>,
    #[serde(default)]
    pub apps: Vec<AppConfig>,
    pub options: Option<FlowOptions>,
}

/// A flow-level `[[flow.apps]]` entry: one app under test, its bundle id,
/// device constraints, and how to install it. Merges with a matching
/// project-level [`ProjectAppConfig`](crate::ProjectAppConfig) by `name`
/// (flow-level fields win) so common apps only need to be declared once.
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
    /// Environment variables injected into the install script's process.
    /// Values interpolate `${var}` through the var engine at install time
    /// (Cli/`--var` + Project + Flow scopes only — install runs before any
    /// flow step, so device/`each`-scoped vars are unavailable and error).
    /// The install script inherits the parent env, so these are additive;
    /// scripts that ignore unknown vars stay backward-compatible.
    #[serde(default)]
    pub install_env: Option<HashMap<String, String>>,
    /// Profile tag. The same app `name` may appear in multiple entries
    /// disambiguated by `profile`; a `profile`-less entry is the catch-all
    /// default. `golem run --profile <name>` selects the matching entry per
    /// app, falling back to the catch-all. See `resolve_app_profiles`.
    #[serde(default)]
    pub profile: Option<String>,
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
    /// See [`AppConfig::install_env`]. Project-level default, gap-filled into
    /// a flow app when the flow doesn't set its own.
    #[serde(default)]
    pub install_env: Option<HashMap<String, String>>,
    /// See [`AppConfig::profile`].
    #[serde(default)]
    pub profile: Option<String>,
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

/// A named node in the flow's execution graph: a list of [`Step`]s plus
/// how control moves on from here — `next` (unconditional), `branch`
/// (conditional `goto`s), or `run_flow` (delegate to a child flow file
/// instead of running steps directly). `for_each` repeats the block once
/// per row of a named data set, binding each row's columns as vars.
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
    /// Per-block override for screen recording. When set, takes
    /// precedence over flow/project defaults but loses to `--no-record`.
    /// `None` = inherit; `Some(false)` = explicit opt-out.
    pub record: Option<bool>,
}

/// A block's `[block.where]` guard — skip this block unless the device it's
/// running on matches. Distinct from [`DeviceConstraint`] (which selects
/// *which* devices a flow runs on); this filters block execution *within*
/// an already-selected device.
#[derive(Deserialize, Debug, Clone)]
pub struct DeviceFilter {
    #[serde(rename = "type")]
    pub device_type: Option<String>,
    pub os: Option<String>,
    pub physical: Option<bool>,
}

/// One entry in a block's `[[block.branch]]` list: an optional condition
/// (visibility check or `if_var` comparison) and where to jump if it holds.
/// A branch with no condition fields set is an unconditional `goto` —
/// typically the last entry, acting as a default/else case.
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
/// right_of = "Theme:"                          # text pattern
/// right_of = { text = "Theme:", enabled = true } # nested selector
/// ```
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Anchor {
    Text(String),
    Selector(Box<SelectorGroup>),
}

/// Anchor for the `contains` predicate. Like [`Anchor`] but the group form
/// also accepts `min_matches`, so a `within = { contains = { text = "Row *",
/// min_matches = 2 } }` resolves to the smallest element enclosing **≥N**
/// matches (the repeated-item *container*, e.g. the `<ul>`, not a single
/// `<li>` wrapper). `min_matches` lives only here — it is meaningless on the
/// main selector or the positional anchors, so it is structurally
/// impossible to write there.
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ContainsAnchor {
    Text(String),
    Spec(Box<ContainsSpec>),
}

/// The group form of a `contains` anchor: every [`SelectorGroup`] field plus
/// `min_matches` (default 1 = the smallest single enclosing box, today's
/// behaviour).
#[derive(Deserialize, Debug, Clone)]
pub struct ContainsSpec {
    #[serde(flatten)]
    pub group: SelectorGroup,
    #[serde(default, deserialize_with = "deserialize_min_matches")]
    pub min_matches: Option<usize>,
}

/// Sanity cap on `min_matches`. Disambiguating a container from a per-item
/// wrapper only ever needs 2–3 enclosed matches; a value beyond this is a
/// mistake, and a huge one would just silently never resolve (every candidate
/// encloses far fewer matches). Reject it at parse time with a clear message
/// instead. Generous so no realistic on-screen list is constrained.
pub const MAX_CONTAINS_MIN_MATCHES: usize = 100;

fn deserialize_min_matches<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<usize>::deserialize(deserializer)?;
    if let Some(n) = value {
        if n == 0 {
            return Err(serde::de::Error::custom("min_matches must be at least 1"));
        }
        if n > MAX_CONTAINS_MIN_MATCHES {
            return Err(serde::de::Error::custom(format!(
                "min_matches = {n} is unreasonably large (max {MAX_CONTAINS_MIN_MATCHES}); 2–3 is typical for disambiguating a container"
            )));
        }
    }
    Ok(value)
}

impl ContainsAnchor {
    /// Minimum number of anchor matches a container must enclose (default 1).
    pub fn min_matches(&self) -> usize {
        match self {
            ContainsAnchor::Text(_) => 1,
            ContainsAnchor::Spec(s) => s.min_matches.unwrap_or(1),
        }
    }
}

/// A coordinate value: either an absolute pixel offset or a percentage
/// string (e.g. `"50%"`, resolved against screen or element bounds at
/// match time).
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum CoordValue {
    Pixels(i32),
    Percent(String), // e.g. "50%"
}

/// Grouped selector for `on = { ... }` / `to = { ... }` syntax.
///
/// All fields match the flat `on_*` fields on [`Step`] but without the
/// prefix. Relational fields (`below`, `above`, `right_of`, `left_of`)
/// accept either a text pattern or a nested selector group.
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
    /// Geometric containment: match the element whose bounds enclose the
    /// anchor ("the box that holds X"). Accepts a text pattern or a nested
    /// group; the group form also accepts `min_matches` to target the
    /// smallest container of ≥N matches (the repeated-item container).
    pub contains: Option<ContainsAnchor>,
    /// Inverse of `contains`: match the element fully enclosed by the anchor.
    pub inside: Option<Anchor>,
    /// Observable traits: ["button", "has_text", "square"], etc.
    #[serde(default)]
    pub traits: Vec<String>,
    /// X coordinate: absolute pixels, percentage of screen/element, or offset from center.
    pub x: Option<CoordValue>,
    /// Y coordinate: same as x.
    pub y: Option<CoordValue>,
}

/// A single flow instruction: an `action` (e.g. `"tap"`, `"type_text"`,
/// `"assert_visible"`, `"scroll"` — validated against golem's known-action
/// list at [`validation::validate_flow`] time, not at parse time) plus
/// whichever of the many optional fields that action uses. Element
/// targeting is either the flat `on_*` fields or the grouped `on`/`to`
/// [`SelectorGroup`]; action-specific parameters not modeled as a named
/// field land in `params` via `#[serde(flatten)]`.
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
    pub scroll_timeout: Option<u64>,
    /// Opt out of the pre-tap keyboard dismissal (`tap`/`long_press`). Default
    /// (unset) auto-dismisses an open keyboard on iOS so the focused input
    /// can't absorb the tap (#83); set true when the tap targets an
    /// input-accessory/toolbar control meant to act on the focused field.
    pub keep_keyboard: Option<bool>,
    /// How much of the target must be visible for auto-scroll to stop, 0–100.
    /// Unset = the engine default (maximise visibility, best-effort). Lower it
    /// to stop earlier on elements taller/wider than the viewport.
    pub visibility_percentage: Option<u8>,
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

impl Step {
    /// Whether the step carries any element-selector criterion — the `on_*`
    /// fields or the grouped `on`/`to`. Used to reject a selector on actions
    /// that operate on the currently-focused element (e.g. `backspace`), so a
    /// misplaced selector fails loudly instead of silently mis-targeting.
    pub fn has_element_selector(&self) -> bool {
        self.on_text.is_some()
            || self.on_accessibility_label.is_some()
            || self.on_index.is_some()
            || self.on_enabled.is_some()
            || self.on_checked.is_some()
            || self.on_clickable.is_some()
            || self.on_below.is_some()
            || self.on_above.is_some()
            || self.on_right_of.is_some()
            || self.on_left_of.is_some()
            || self.on.is_some()
    }
}

/// A single finger path in a multi-touch gesture.
#[derive(Deserialize, Debug, Clone)]
pub struct Finger {
    /// Points the finger travels through (each is a full selector).
    pub points: Vec<SelectorGroup>,
}

/// The `[flow.options]` table: per-flow tunables for concurrency,
/// resource gating, timeouts, recording, and accessibility auditing. Every
/// field is optional here; [`config::merge_config`] fills gaps from the
/// project-level `golem.toml` `[options]` table, with flow-level values
/// always winning.
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
    /// Accessibility audit strictness. `off` disables; `critical` runs
    /// tree checks only; `relaxed` (default) adds opportunistic contrast;
    /// `strict` forces a per-block screenshot + AAA bands. The `--a11y`
    /// CLI flag overrides this.
    pub a11y: Option<A11yLevel>,
    /// Fail the flow when the cumulative a11y error count exceeds this.
    pub a11y_max_errors: Option<usize>,
    /// Fail the flow when the cumulative a11y warning count exceeds this.
    pub a11y_max_warnings: Option<usize>,
    /// Drop a11y findings whose confidence (0.0–1.0) is below this. Lets
    /// flows tune out noisy heuristic findings (e.g. contrast); deterministic
    /// checks are confidence 1.0 and always pass. Default: keep all.
    pub a11y_min_confidence: Option<f32>,
}

/// Accessibility audit strictness (the TOML `a11y` value). The runner maps
/// this onto its own `A11yLevel` (which carries the threshold behaviour).
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum A11yLevel {
    Off,
    Critical,
    Relaxed,
    Strict,
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
        let actions: Vec<&str> = flow.block[0]
            .steps
            .iter()
            .map(|s| s.action.as_str())
            .collect();
        assert_eq!(
            actions,
            vec![
                "tap",
                "long_press",
                "type_text",
                "swipe",
                "scroll",
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
steps = [{action = "tap", on_text = "OK"}, {action = "assert_visible"}]
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
action = "assert_visible"
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

        assert_eq!(
            block.vars.get("username").map(|s| s.as_str()),
            Some("admin")
        );
        assert_eq!(
            block.vars.get("password").map(|s| s.as_str()),
            Some("secret")
        );
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
  { action = "tap", on = { text = "Light", right_of = { text = "Theme:", enabled = true } } },
]
"#;
        let flow = parse_flow(toml_str).expect("nested anchor should parse");
        let step = &flow.block[0].steps[0];
        let g = step.on.as_ref().expect("on group should be present");
        assert_eq!(g.text.as_deref(), Some("Light"));
        match &g.right_of {
            Some(Anchor::Selector(nested)) => {
                assert_eq!(nested.text.as_deref(), Some("Theme:"));
                assert_eq!(nested.enabled, Some(true));
            }
            other => panic!("expected nested Anchor::Selector, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // 8d2. `contains` anchor: bare text, group, and group + min_matches
    // ---------------------------------------------------------------
    #[test]
    fn step_with_contains_text_and_min_matches() {
        let toml_str = r#"
[flow]
name = "contains"

[[block]]
steps = [
  { action = "tap", on = { contains = "Row 0" } },
  { action = "tap", on = { contains = { text = "Row *" } } },
  { action = "scroll", to = { text = "Row 9" }, within = { contains = { text = "Row *", min_matches = 2 } } },
]
"#;
        let flow = parse_flow(toml_str).expect("contains forms should parse");
        let steps = &flow.block[0].steps;

        // Bare string → Text, default min_matches 1.
        let c0 = steps[0]
            .on
            .as_ref()
            .expect("as_ref() SHALL succeed")
            .contains
            .as_ref()
            .expect("as_ref() SHALL succeed");
        assert!(matches!(c0, ContainsAnchor::Text(s) if s == "Row 0"));
        assert_eq!(c0.min_matches(), 1, "bare text contains SHALL default to 1");

        // Group without min_matches → Spec, default min_matches 1, flatten keeps `text`.
        let c1 = steps[1]
            .on
            .as_ref()
            .expect("as_ref() SHALL succeed")
            .contains
            .as_ref()
            .expect("as_ref() SHALL succeed");
        match c1 {
            ContainsAnchor::Spec(s) => {
                assert_eq!(
                    s.group.text.as_deref(),
                    Some("Row *"),
                    "flatten SHALL keep group fields"
                );
                assert_eq!(
                    c1.min_matches(),
                    1,
                    "group without min_matches SHALL default to 1"
                );
            }
            other => panic!("expected Spec, got {other:?}"),
        }

        // Group with min_matches → parsed.
        let c2 = steps[2]
            .within
            .as_ref()
            .expect("as_ref() SHALL succeed")
            .contains
            .as_ref()
            .expect("as_ref() SHALL succeed");
        match c2 {
            ContainsAnchor::Spec(s) => {
                assert_eq!(s.group.text.as_deref(), Some("Row *"));
                assert_eq!(c2.min_matches(), 2, "min_matches SHALL parse");
            }
            other => panic!("expected Spec, got {other:?}"),
        }
    }

    // min_matches outside [1, MAX] is a parse error (clear message, fail-fast).
    #[test]
    fn contains_min_matches_rejects_zero_and_huge() {
        for bad in [0usize, MAX_CONTAINS_MIN_MATCHES + 1, 1_000_000] {
            let toml_str = format!(
                r#"
[flow]
name = "bad"

[[block]]
steps = [
  {{ action = "scroll", to = {{ text = "Row 9" }}, within = {{ contains = {{ text = "Row *", min_matches = {bad} }} }} }},
]
"#
            );
            let err =
                parse_flow(&toml_str).expect_err(&format!("min_matches = {bad} SHALL be rejected"));
            assert!(
                err.to_string().contains("min_matches"),
                "error SHALL name min_matches: {err}"
            );
        }
        // The boundary value is accepted.
        let ok = format!(
            r#"
[flow]
name = "ok"

[[block]]
steps = [
  {{ action = "scroll", to = {{ text = "Row 9" }}, within = {{ contains = {{ text = "Row *", min_matches = {} }} }} }},
]
"#,
            MAX_CONTAINS_MIN_MATCHES
        );
        parse_flow(&ok).expect("min_matches at the cap SHALL parse");
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
        assert_eq!(opts.record, Some(true));
        assert_eq!(opts.max_steps, Some(200));
        assert_eq!(opts.max_runtime.as_deref(), Some("30m"));
        assert_eq!(opts.suite_concurrency, Some(2));
        assert_eq!(opts.keep_devices, Some(false));
        assert_eq!(opts.create_if_missing, Some(true));
        assert_eq!(opts.ignore_missing_physical, Some(false));
    }

    // ---------------------------------------------------------------
    // 10b. Accessibility options
    // ---------------------------------------------------------------
    fn parse_a11y_opts(body: &str) -> FlowOptions {
        let toml_str = format!("[flow]\nname = \"a\"\n\n[flow.options]\n{body}\n");
        parse_flow(&toml_str)
            .expect("a11y options should parse")
            .flow
            .options
            .expect("options present")
    }

    #[test]
    fn a11y_levels_parse() {
        assert_eq!(parse_a11y_opts("a11y = \"off\"").a11y, Some(A11yLevel::Off));
        assert_eq!(
            parse_a11y_opts("a11y = \"critical\"").a11y,
            Some(A11yLevel::Critical)
        );
        assert_eq!(
            parse_a11y_opts("a11y = \"relaxed\"").a11y,
            Some(A11yLevel::Relaxed)
        );
        assert_eq!(
            parse_a11y_opts("a11y = \"strict\"").a11y,
            Some(A11yLevel::Strict)
        );
    }

    #[test]
    fn a11y_default_absent_is_none() {
        // No a11y key → None at parse; the suite supplies the relaxed default.
        assert_eq!(parse_a11y_opts("max_steps = 1").a11y, None);
    }

    #[test]
    fn a11y_invalid_value_rejected() {
        let toml_str = "[flow]\nname = \"a\"\n\n[flow.options]\na11y = \"banana\"\n";
        assert!(
            parse_flow(toml_str).is_err(),
            "unknown a11y level SHALL be a parse error"
        );
    }

    #[test]
    fn a11y_thresholds_parse() {
        let opts = parse_a11y_opts("a11y_max_errors = 0\na11y_max_warnings = 10");
        assert_eq!(opts.a11y_max_errors, Some(0));
        assert_eq!(opts.a11y_max_warnings, Some(10));
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
        assert_eq!(
            flow.data[0].get("username").map(|s| s.as_str()),
            Some("alice")
        );
        assert_eq!(
            flow.data[1].get("username").map(|s| s.as_str()),
            Some("bob")
        );
        assert_eq!(
            flow.data[2].get("username").map(|s| s.as_str()),
            Some("charlie")
        );
        assert_eq!(
            flow.data[2].get("password").map(|s| s.as_str()),
            Some("pass3")
        );
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

    #[test]
    fn explicit_only_parses() {
        let toml_str = r#"
[flow]
name = "subflow"
explicit_only = true
"#;
        let flow = parse_flow(toml_str).expect("explicit_only should parse");
        assert!(flow.flow.explicit_only);
    }

    #[test]
    fn explicit_only_defaults_false() {
        let toml_str = r#"
[flow]
name = "normal"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        assert!(
            !flow.flow.explicit_only,
            "absent explicit_only SHALL default to false"
        );
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
        assert_eq!(flow.block[0].run_flow.as_deref(), Some("helper.test.toml"));
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

    // ---------------------------------------------------------------
    // 26. InstallScriptValue::for_platform — Single resolves every platform
    // ---------------------------------------------------------------
    #[test]
    fn install_script_single_resolves_all_platforms() {
        let v = InstallScriptValue::Single("scripts/install.sh".to_string());
        assert_eq!(
            v.for_platform("ios"),
            Some("scripts/install.sh"),
            "Single SHALL resolve for ios"
        );
        assert_eq!(
            v.for_platform("android"),
            Some("scripts/install.sh"),
            "Single SHALL resolve for android"
        );
        assert_eq!(
            v.for_platform("web"),
            Some("scripts/install.sh"),
            "Single SHALL resolve for any platform string"
        );
    }

    // ---------------------------------------------------------------
    // 27. InstallScriptValue::for_platform — PerPlatform keyed lookup
    // ---------------------------------------------------------------
    #[test]
    fn install_script_per_platform_resolves_by_key() {
        let v = InstallScriptValue::PerPlatform(InstallScriptPerPlatform {
            ios: Some("ios.sh".to_string()),
            android: Some("android.sh".to_string()),
        });
        assert_eq!(
            v.for_platform("ios"),
            Some("ios.sh"),
            "ios key SHALL resolve"
        );
        assert_eq!(
            v.for_platform("android"),
            Some("android.sh"),
            "android key SHALL resolve"
        );
        assert_eq!(
            v.for_platform("windows"),
            None,
            "unknown platform SHALL resolve to None"
        );
    }

    // ---------------------------------------------------------------
    // 28. InstallScriptValue::for_platform — PerPlatform missing key is None
    // ---------------------------------------------------------------
    #[test]
    fn install_script_per_platform_missing_key_is_none() {
        let v = InstallScriptValue::PerPlatform(InstallScriptPerPlatform {
            ios: Some("ios.sh".to_string()),
            android: None,
        });
        assert_eq!(
            v.for_platform("ios"),
            Some("ios.sh"),
            "present ios key SHALL resolve"
        );
        assert_eq!(
            v.for_platform("android"),
            None,
            "absent android key SHALL resolve to None"
        );
    }

    // ---------------------------------------------------------------
    // 29. install_script deserializes from a single string (untagged)
    // ---------------------------------------------------------------
    #[test]
    fn install_script_deserializes_single_string() {
        let toml_str = r#"
[flow]
name = "single install"

[[flow.apps]]
name = "app1"
bundle = "com.example"
install_script = "scripts/install.sh"
install_timeout_ms = 900000
"#;
        let flow = parse_flow(toml_str).expect("single install_script SHALL parse");
        let app = &flow.flow.apps[0];
        let script = app.install_script.as_ref().expect("install_script present");
        assert!(
            matches!(script, InstallScriptValue::Single(s) if s == "scripts/install.sh"),
            "string form SHALL deserialize to Single"
        );
        assert_eq!(app.install_timeout_ms, Some(900_000));
    }

    #[test]
    fn install_env_deserializes_table() {
        let toml_str = r#"
[flow]
name = "install env"

[[flow.apps]]
name = "app1"
bundle = "com.example"
install_script = "scripts/install.sh"
install_env = { APP_ENV = "staging", SANDBOX_ID = "${sandbox_id}" }
"#;
        let flow = parse_flow(toml_str).expect("install_env SHALL parse");
        let env = flow.flow.apps[0]
            .install_env
            .as_ref()
            .expect("install_env present");
        assert_eq!(env.get("APP_ENV").map(String::as_str), Some("staging"));
        assert_eq!(
            env.get("SANDBOX_ID").map(String::as_str),
            Some("${sandbox_id}"),
            "raw `${{...}}` SHALL survive parsing (interpolated later, at install time)"
        );
    }

    #[test]
    fn install_env_absent_is_none() {
        let toml_str = r#"
[flow]
name = "no install env"

[[flow.apps]]
name = "app1"
bundle = "com.example"
"#;
        let flow = parse_flow(toml_str).expect("SHALL parse");
        assert!(flow.flow.apps[0].install_env.is_none());
    }

    // ---------------------------------------------------------------
    // 30. install_script deserializes from a platform-keyed table (untagged)
    // ---------------------------------------------------------------
    #[test]
    fn install_script_deserializes_per_platform_table() {
        let toml_str = r#"
[flow]
name = "per-platform install"

[[flow.apps]]
name = "app1"
bundle = "com.example"
install_script = { ios = "ios.sh", android = "android.sh" }
"#;
        let flow = parse_flow(toml_str).expect("table install_script SHALL parse");
        let script = flow.flow.apps[0]
            .install_script
            .as_ref()
            .expect("install_script present");
        match script {
            InstallScriptValue::PerPlatform(p) => {
                assert_eq!(p.ios.as_deref(), Some("ios.sh"));
                assert_eq!(p.android.as_deref(), Some("android.sh"));
            }
            InstallScriptValue::Single(_) => panic!("table SHALL deserialize to PerPlatform"),
        }
    }

    // ---------------------------------------------------------------
    // 31. StringOrVec::to_vec — Single wraps in a one-element vec
    // ---------------------------------------------------------------
    #[test]
    fn string_or_vec_to_vec_single() {
        let s = StringOrVec::Single("phone".to_string());
        assert_eq!(
            s.to_vec(),
            vec!["phone".to_string()],
            "Single SHALL yield one element"
        );
    }

    // ---------------------------------------------------------------
    // 32. StringOrVec::to_vec — Multiple clones the vec
    // ---------------------------------------------------------------
    #[test]
    fn string_or_vec_to_vec_multiple() {
        let s = StringOrVec::Multiple(vec!["a".to_string(), "b".to_string()]);
        assert_eq!(
            s.to_vec(),
            vec!["a".to_string(), "b".to_string()],
            "Multiple SHALL yield all elements"
        );
    }

    // ---------------------------------------------------------------
    // 33. CoverageStrategy parses all lowercase variants
    // ---------------------------------------------------------------
    #[test]
    fn coverage_strategy_lowercase_variants() {
        for (literal, expected) in [
            ("one", CoverageStrategy::One),
            ("min", CoverageStrategy::Min),
            ("smart", CoverageStrategy::Smart),
            ("full", CoverageStrategy::Full),
        ] {
            let toml_str = format!(
                r#"
[flow]
name = "coverage"

[flow.options]
coverage = "{literal}"
"#
            );
            let flow = parse_flow(&toml_str).expect("coverage variant SHALL parse");
            let opts = flow.flow.options.expect("options present");
            assert_eq!(
                opts.coverage,
                Some(expected),
                "coverage `{literal}` SHALL map to its variant"
            );
        }
    }

    // ---------------------------------------------------------------
    // 34. CoverageStrategy rejects unknown / wrong-case values
    // ---------------------------------------------------------------
    #[test]
    fn coverage_strategy_rejects_unknown() {
        let toml_str = r#"
[flow]
name = "bad coverage"

[flow.options]
coverage = "Full"
"#;
        let result = parse_flow(toml_str);
        assert!(
            result.is_err(),
            "uppercase / unknown coverage SHALL fail to parse (rename_all = lowercase)"
        );
    }

    // ---------------------------------------------------------------
    // 35. AppLifecycle parses all lowercase variants
    // ---------------------------------------------------------------
    #[test]
    fn app_lifecycle_lowercase_variants() {
        for (literal, expected) in [
            ("reset", AppLifecycle::Reset),
            ("launch", AppLifecycle::Launch),
            ("manual", AppLifecycle::Manual),
        ] {
            let toml_str = format!(
                r#"
[flow]
name = "lifecycle"

[flow.options]
app_lifecycle = "{literal}"
"#
            );
            let flow = parse_flow(&toml_str).expect("lifecycle variant SHALL parse");
            let opts = flow.flow.options.expect("options present");
            assert_eq!(
                opts.app_lifecycle,
                Some(expected),
                "app_lifecycle `{literal}` SHALL map to its variant"
            );
        }
    }

    // ---------------------------------------------------------------
    // 36. AppLifecycle rejects unknown value
    // ---------------------------------------------------------------
    #[test]
    fn app_lifecycle_rejects_unknown() {
        let toml_str = r#"
[flow]
name = "bad lifecycle"

[flow.options]
app_lifecycle = "restart"
"#;
        let result = parse_flow(toml_str);
        assert!(result.is_err(), "unknown app_lifecycle SHALL fail to parse");
    }

    // ---------------------------------------------------------------
    // 37. Branch with if_not_visible / equals / matches fields
    // ---------------------------------------------------------------
    #[test]
    fn branch_with_remaining_condition_fields() {
        let toml_str = r#"
[flow]
name = "branch fields"

[[block]]

[[block.branch]]
if_not_visible = "Spinner"
goto = "ready"

[[block.branch]]
if_var = "status"
equals = "done"
goto = "finish"

[[block.branch]]
if_var = "name"
matches = "^a.*"
goto = "matched"
"#;
        let flow = parse_flow(toml_str).expect("branch fields SHALL parse");
        let b = &flow.block[0].branch;
        assert_eq!(b[0].if_not_visible.as_deref(), Some("Spinner"));
        assert_eq!(b[1].equals.as_deref(), Some("done"));
        assert_eq!(b[2].matches.as_deref(), Some("^a.*"));
    }

    // ---------------------------------------------------------------
    // 38. CoordValue — absolute pixels vs percentage string
    // ---------------------------------------------------------------
    #[test]
    fn coord_value_pixels_and_percent() {
        let toml_str = r#"
[flow]
name = "coords"

[[block]]
steps = [
  { action = "tap", on = { x = 100, y = "50%" } },
]
"#;
        let flow = parse_flow(toml_str).expect("coord values SHALL parse");
        let g = flow.block[0].steps[0]
            .on
            .as_ref()
            .expect("on group present");
        assert!(
            matches!(g.x, Some(CoordValue::Pixels(100))),
            "integer SHALL deserialize to Pixels"
        );
        assert!(
            matches!(&g.y, Some(CoordValue::Percent(s)) if s == "50%"),
            "string SHALL deserialize to Percent"
        );
    }

    // ---------------------------------------------------------------
    // 39. Anchor on every relational field (below/above/left_of)
    // ---------------------------------------------------------------
    #[test]
    fn anchor_on_all_relational_fields() {
        let toml_str = r#"
[flow]
name = "relational"

[[block]]
steps = [
  { action = "tap", on = { text = "X", below = "B", above = "A", left_of = "L" } },
]
"#;
        let flow = parse_flow(toml_str).expect("relational anchors SHALL parse");
        let g = flow.block[0].steps[0]
            .on
            .as_ref()
            .expect("on group present");
        assert!(matches!(&g.below, Some(Anchor::Text(s)) if s == "B"));
        assert!(matches!(&g.above, Some(Anchor::Text(s)) if s == "A"));
        assert!(matches!(&g.left_of, Some(Anchor::Text(s)) if s == "L"));
    }

    // ---------------------------------------------------------------
    // 40. Multi-touch fingers gesture parses into Finger paths
    // ---------------------------------------------------------------
    #[test]
    fn multi_touch_fingers_parse() {
        let toml_str = r#"
[flow]
name = "multitouch"

[[block]]

[[block.steps]]
action = "pinch"
scale = 2.0
velocity = 1.5
rotation = 90.0

[[block.steps.fingers]]
points = [{ x = 0, y = 0 }, { x = 100, y = 100 }]

[[block.steps.fingers]]
points = [{ x = 200, y = 200 }, { x = 50, y = 50 }]
"#;
        let flow = parse_flow(toml_str).expect("multi-touch SHALL parse");
        let step = &flow.block[0].steps[0];
        assert_eq!(step.scale, Some(2.0));
        assert_eq!(step.velocity, Some(1.5));
        assert_eq!(step.rotation, Some(90.0));
        assert_eq!(step.fingers.len(), 2, "two finger paths SHALL parse");
        assert_eq!(step.fingers[0].points.len(), 2);
        assert!(matches!(
            step.fingers[1].points[0].x,
            Some(CoordValue::Pixels(200))
        ));
    }

    // ---------------------------------------------------------------
    // 41. Gesture `points` alias for the path field
    // ---------------------------------------------------------------
    #[test]
    fn gesture_points_alias() {
        let toml_str = r#"
[flow]
name = "path"

[[block]]
steps = [
  { action = "swipe", points = [{ x = 0, y = 0 }, { x = 100, y = 100 }] },
]
"#;
        let flow = parse_flow(toml_str).expect("points path SHALL parse");
        let step = &flow.block[0].steps[0];
        assert_eq!(step.points.len(), 2, "points alias SHALL populate the path");
    }

    // ---------------------------------------------------------------
    // 42. Swipe start/end/within selector blocks
    // ---------------------------------------------------------------
    #[test]
    fn swipe_start_end_within_blocks() {
        let toml_str = r#"
[flow]
name = "swipe blocks"

[[block]]

[[block.steps]]
action = "swipe"
duration = 300

[block.steps.start]
text = "TopAnchor"

[block.steps.end]
text = "BottomAnchor"

[block.steps.within]
text = "ScrollContainer"
"#;
        let flow = parse_flow(toml_str).expect("start/end/within SHALL parse");
        let step = &flow.block[0].steps[0];
        assert_eq!(
            step.start.as_ref().expect("start present").text.as_deref(),
            Some("TopAnchor")
        );
        assert_eq!(
            step.end.as_ref().expect("end present").text.as_deref(),
            Some("BottomAnchor")
        );
        assert_eq!(
            step.within
                .as_ref()
                .expect("within present")
                .text
                .as_deref(),
            Some("ScrollContainer")
        );
        assert_eq!(step.duration, Some(300));
    }

    // ---------------------------------------------------------------
    // 43. Step app/restart/auto_scroll/scroll_timeout/input fields
    // ---------------------------------------------------------------
    #[test]
    fn step_app_restart_autoscroll_input_fields() {
        let toml_str = r#"
[flow]
name = "step misc"

[[block]]

[[block.steps]]
action = "type_text"
input = "hello world"
app = "other-app"
restart = true
auto_scroll = false
scroll_timeout = 8000
visibility_percentage = 40
"#;
        let flow = parse_flow(toml_str).expect("misc step fields SHALL parse");
        let step = &flow.block[0].steps[0];
        assert_eq!(step.input.as_deref(), Some("hello world"));
        assert_eq!(step.app.as_deref(), Some("other-app"));
        assert_eq!(step.restart, Some(true));
        assert_eq!(step.auto_scroll, Some(false));
        assert_eq!(step.scroll_timeout, Some(8000));
        assert_eq!(step.visibility_percentage, Some(40));
    }

    // ---------------------------------------------------------------
    // 44. Block record override and save_to map
    // ---------------------------------------------------------------
    #[test]
    fn block_record_and_save_to() {
        let toml_str = r#"
[flow]
name = "block record"

[[block]]
name = "b"
record = false

[block.save_to]
total = "counter_value"

[[block.steps]]
action = "tap"
"#;
        let flow = parse_flow(toml_str).expect("record/save_to SHALL parse");
        let block = &flow.block[0];
        assert_eq!(
            block.record,
            Some(false),
            "explicit record opt-out SHALL parse"
        );
        assert_eq!(
            block.save_to.get("total").map(|s| s.as_str()),
            Some("counter_value")
        );
    }

    // ---------------------------------------------------------------
    // 45. ProjectAppConfig deserializes from [[apps]]-style table
    // ---------------------------------------------------------------
    #[test]
    fn project_app_config_deserializes() {
        let toml_str = r#"
name = "app-b"
bundle = "fail.golem.testb"
install_timeout_ms = 900000
install_script = { ios = "ios.sh", android = "android.sh" }

[[devices]]
os = "android"
"#;
        let cfg: ProjectAppConfig =
            toml::from_str(toml_str).expect("ProjectAppConfig SHALL deserialize");
        assert_eq!(cfg.name, "app-b");
        assert_eq!(cfg.bundle.as_deref(), Some("fail.golem.testb"));
        assert_eq!(cfg.install_timeout_ms, Some(900_000));
        assert_eq!(cfg.devices.len(), 1);
        assert!(matches!(
            cfg.install_script,
            Some(InstallScriptValue::PerPlatform(_))
        ));
    }

    // ---------------------------------------------------------------
    // 46. DeviceConstraint full axis set (hardware/playstore/booted/expand)
    // ---------------------------------------------------------------
    #[test]
    fn device_constraint_full_axes() {
        let toml_str = r#"
[flow]
name = "device axes"

[[flow.apps]]
name = "app1"
bundle = "com.example"

[[flow.apps.devices]]
type = ["phone", "tablet"]
name = "Pixel 7a"
accessibility_label = "device-label"
hardware = ["virtual", "real"]
playstore = true
booted = false
expand = "matrix"
"#;
        let flow = parse_flow(toml_str).expect("full device axes SHALL parse");
        let d = &flow.flow.apps[0].devices[0];
        assert_eq!(
            d.device_type.as_ref().expect("type present").to_vec(),
            vec!["phone".to_string(), "tablet".to_string()]
        );
        assert_eq!(d.name.as_deref(), Some("Pixel 7a"));
        assert_eq!(d.accessibility_label.as_deref(), Some("device-label"));
        assert_eq!(
            d.hardware.as_ref().expect("hardware present").to_vec(),
            vec!["virtual".to_string(), "real".to_string()]
        );
        assert_eq!(d.playstore, Some(true));
        assert_eq!(d.booted, Some(false));
        assert_eq!(d.expand.as_deref(), Some("matrix"));
    }

    // ---------------------------------------------------------------
    // 47. Malformed TOML — syntax error surfaces as Err
    // ---------------------------------------------------------------
    #[test]
    fn malformed_toml_is_err() {
        let toml_str = "this is = = not valid toml [[[";
        assert!(
            parse_flow(toml_str).is_err(),
            "invalid TOML SHALL produce an error"
        );
    }

    // ---------------------------------------------------------------
    // 48. Flow-level vars map parses
    // ---------------------------------------------------------------
    #[test]
    fn flow_level_vars() {
        let toml_str = r#"
[flow]
name = "vars"

[flow.vars]
base_url = "https://example.com"
token = "abc123"
"#;
        let flow = parse_flow(toml_str).expect("flow vars SHALL parse");
        assert_eq!(
            flow.flow.vars.get("base_url").map(|s| s.as_str()),
            Some("https://example.com")
        );
        assert_eq!(
            flow.flow.vars.get("token").map(|s| s.as_str()),
            Some("abc123")
        );
    }
}
