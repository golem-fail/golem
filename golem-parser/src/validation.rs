use crate::{BranchCondition, FlowFile, Step};
use std::collections::HashSet;

/// One structural problem found by [`validate_flow`]: a human-readable
/// `message` plus a `kind` a caller can match on to decide severity or
/// filter known-acceptable cases.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub message: String,
    pub kind: ValidationErrorKind,
}

/// Category of a [`ValidationError`], covering the structural checks
/// `validate_flow` runs: dangling references (unknown action, bad `goto`/
/// start block, duplicate block names), malformed branch conditions, and
/// invalid option values.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationErrorKind {
    MissingDevices,
    UnknownAction,
    InvalidGotoTarget,
    InvalidStartBlock,
    DuplicateBlockName,
    InvalidOnFail,
    MissingGoto,
    ConflictingBranchCondition,
    MissingComparison,
    InvalidConcurrency,
    /// A selector was given to an action that operates on the currently
    /// focused element (e.g. `backspace`), where it can't be honored reliably.
    SelectorNotAllowed,
    /// A `permissions` map entry has an invalid mode for its permission
    /// (e.g. `camera = "limited"`) or uses the removed `location-always` key.
    InvalidPermission,
}

const KNOWN_ACTIONS: &[&str] = &[
    "tap",
    "long_press",
    "type",
    "backspace",
    "swipe",
    "scroll",
    "read",
    "hide_keyboard",
    "press",
    "launch",
    "stop",
    "clear_data",
    "assert_visible",
    "assert_not_visible",
    "fail",
    "set_location",
    "set_dark_mode",
    "rotate",
    "push_notification",
    "open_link",
    "screenshot",
    "add_media",
    "load_fixture",
    "load_mixin",
    "run",
    "bash",
    "get_http",
    "post_http",
    "put_http",
    "patch_http",
    "delete_http",
    "await_email",
    "create_inbox",
    "assert_alert",
    "dismiss_alert",
];

const VALID_ON_FAIL: &[&str] = &["error", "warn", "ignore"];

/// A `within = { ... }` setting that won't actually constrain the step.
/// Returned by `lint_within_no_op` for both runtime warnings and a
/// future `--validate` mode (which should treat them as errors).
#[derive(Debug, Clone)]
pub struct WithinNoOpIssue {
    pub block_name: Option<String>,
    pub step_index: usize,
    pub action: String,
}

/// A `push_notification` step in a flow that *could* be scheduled
/// against a physical device — the action is sim/emu-only on both
/// platforms (see `golem-driver/src/{ios,android}.rs::push_notification`)
/// and would error at runtime there. Caller decides severity:
/// `--validate` rejects with error, runtime emits a warning.
#[derive(Debug, Clone)]
pub struct PushNotificationPhysIssue {
    pub block_name: Option<String>,
    pub step_index: usize,
    pub app_name: String,
}

/// A step param key that reads like a field golem would have used, but
/// isn't one — so serde swept it into the open `params` map and nothing
/// ever looked at it.
#[derive(Debug, Clone)]
pub struct UnknownStepFieldIssue {
    pub block_name: Option<String>,
    pub step_index: usize,
    pub action: String,
    /// The key as written.
    pub key: String,
    /// The single field within one edit of `key`, when there is exactly
    /// one. Absent for an `on_*` key that resembles nothing.
    pub suggestion: Option<String>,
}

/// Every named field on [`Step`], as spelled in TOML.
///
/// The destructure is the point: `Step` is matched exhaustively with no
/// `..`, so adding a field fails to compile until it is listed here. The
/// alternative — a hand-kept array — drifts the moment someone adds a
/// field, and it drifts silently, which is the exact failure this lint
/// exists to catch.
fn step_field_names() -> &'static [&'static str] {
    #[allow(clippy::no_effect_underscore_binding)]
    fn _exhaustive(step: &Step) {
        let Step {
            action: _,
            on_text: _,
            on_accessibility_label: _,
            on_index: _,
            on_enabled: _,
            on_checked: _,
            on_clickable: _,
            on_below: _,
            on_above: _,
            on_right_of: _,
            on_left_of: _,
            on: _,
            input: _,
            if_fail: _,
            save_to: _,
            timeout: _,
            retry: _,
            retry_delay: _,
            app: _,
            restart: _,
            auto_scroll: _,
            scroll_timeout: _,
            keep_keyboard: _,
            visibility_percentage: _,
            within: _,
            start: _,
            end: _,
            points: _,
            duration: _,
            scale: _,
            rotation: _,
            velocity: _,
            fingers: _,
            params: _,
        } = step;
    }
    &[
        "action",
        "on_text",
        "on_accessibility_label",
        "on_index",
        "on_enabled",
        "on_checked",
        "on_clickable",
        "on_below",
        "on_above",
        "on_right_of",
        "on_left_of",
        "on",
        // `on`'s TOML alias — a step may spell the grouped selector either way.
        "to",
        "input",
        "if_fail",
        "save_to",
        "timeout",
        "retry",
        "retry_delay",
        "app",
        "restart",
        "auto_scroll",
        "scroll_timeout",
        "keep_keyboard",
        "visibility_percentage",
        "within",
        "start",
        "end",
        "points",
        "duration",
        "scale",
        "rotation",
        "velocity",
        "fingers",
    ]
}

/// True when `a` and `b` are within one insertion, deletion or
/// substitution. Bounded rather than a full edit distance because that is
/// all the lint acts on, and a length gap over one settles it immediately.
fn within_one_edit(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    let mut i = 0;
    let mut j = 0;
    let mut edited = false;
    while i < long.len() && j < short.len() {
        if long[i] == short[j] {
            i += 1;
            j += 1;
            continue;
        }
        if edited {
            return false;
        }
        edited = true;
        // Same length → substitution advances both; otherwise the extra
        // character belongs to the longer string alone.
        if long.len() == short.len() {
            j += 1;
        }
        i += 1;
    }
    true
}

/// Flag step keys that landed in the catch-all `params` map but look like
/// they were meant to be step fields.
///
/// `Step` ends in `#[serde(flatten)] params`, the open map action
/// parameters arrive through, so `deny_unknown_fields` is not available
/// here the way it is on the selector structs (#32) — an unrecognised key
/// is indistinguishable, to serde, from a parameter golem does not model.
/// The cost of that is silent: `on_text = "Save", on_indx = 2` drops the
/// disambiguator and resolves against every "Save" on screen, and
/// `on_tex = "Save"` alone leaves the step with no criteria at all —
/// which matches the FIRST element in the tree, because
/// `matches_selector` only tests the criteria that are set. Both pass.
///
/// Two rules, in decreasing confidence:
///
/// 1. `on_` is golem's prefix. Every action is in `KNOWN_ACTIONS` and the
///    parameter vocabulary is golem's own, so an `on_*` key that is not a
///    flat selector cannot be a legitimate parameter — it is a typo.
/// 2. Any other key within one edit of a field name, when exactly one
///    field is that close. Ties are left alone rather than guessed at.
pub fn lint_unknown_step_fields(flow: &FlowFile) -> Vec<UnknownStepFieldIssue> {
    let fields = step_field_names();
    let mut issues = Vec::new();
    for block in &flow.block {
        for (idx, step) in block.steps.iter().enumerate() {
            // Sorted so the warnings a flow produces don't reorder between
            // runs — params is a HashMap.
            let mut keys: Vec<&String> = step.params.keys().collect();
            keys.sort();
            for key in keys {
                let near: Vec<&str> = fields
                    .iter()
                    .copied()
                    .filter(|f| within_one_edit(f, key))
                    .collect();
                let suggestion = match near.as_slice() {
                    [only] => Some((*only).to_string()),
                    _ => None,
                };
                // A correctly spelled flat selector is a named field, so
                // serde consumed it and it is not in `params` at all — the
                // bare prefix is enough, and an exemption list here would be
                // a condition that can never be false.
                let is_stray_on = key.starts_with("on_");
                if !is_stray_on && suggestion.is_none() {
                    continue;
                }
                issues.push(UnknownStepFieldIssue {
                    block_name: block.name.clone(),
                    step_index: idx,
                    action: step.action.clone(),
                    key: key.clone(),
                    suggestion,
                });
            }
        }
    }
    issues
}

/// `push_notification` is sim/emu-only on both platforms. Flag any
/// flow whose app explicitly opts into `hardware = "real"` (or
/// `["virtual", "real"]`) AND uses the action — the runtime would
/// bail on the phys-device run. Apps with `hardware` absent default
/// to virtual-only today and don't trigger the lint; if the default
/// changes (see roadmap), this trigger condition flips to include
/// the absent case too.
pub fn lint_push_notification_phys(flow: &FlowFile) -> Vec<PushNotificationPhysIssue> {
    // Apps whose device constraints permit real hardware.
    let phys_capable_apps: Vec<&str> = flow
        .flow
        .apps
        .iter()
        .filter(|app| {
            app.devices.iter().any(|dc| {
                dc.hardware
                    .as_ref()
                    .is_some_and(|h| h.to_vec().iter().any(|v| v == "real"))
            })
        })
        .map(|app| app.name.as_str())
        .collect();
    if phys_capable_apps.is_empty() {
        return Vec::new();
    }
    let mut issues = Vec::new();
    for block in &flow.block {
        for (idx, step) in block.steps.iter().enumerate() {
            if step.action != "push_notification" {
                continue;
            }
            // Resolve step's app target — `step.app` overrides flow-
            // default (the first [[flow.apps]] entry).
            let target = step
                .app
                .as_deref()
                .or_else(|| flow.flow.apps.first().map(|a| a.name.as_str()));
            let Some(target) = target else {
                continue;
            };
            if phys_capable_apps.contains(&target) {
                issues.push(PushNotificationPhysIssue {
                    block_name: block.name.clone(),
                    step_index: idx,
                    app_name: target.to_string(),
                });
            }
        }
    }
    issues
}

/// `within` is consumed by `scroll` and by any step that has
/// `auto_scroll = true` (the resolver uses it to constrain scroll-into-
/// view to the container). On any other step it's silently dropped —
/// most often a footgun for swipes ported from a scroll snippet.
pub fn lint_within_no_op(flow: &FlowFile) -> Vec<WithinNoOpIssue> {
    let mut issues = Vec::new();
    for block in &flow.block {
        for (idx, step) in block.steps.iter().enumerate() {
            if step.within.is_none() {
                continue;
            }
            let consumed = step.action == "scroll" || step.auto_scroll == Some(true);
            if !consumed {
                issues.push(WithinNoOpIssue {
                    block_name: block.name.clone(),
                    step_index: idx,
                    action: step.action.clone(),
                });
            }
        }
    }
    issues
}

/// Validate a parsed FlowFile for structural correctness.
pub fn validate_flow(flow: &FlowFile) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // 1. Missing devices on app
    for app in &flow.flow.apps {
        if app.devices.is_empty() {
            errors.push(ValidationError {
                message: format!("App '{}' has no device constraints", app.name),
                kind: ValidationErrorKind::MissingDevices,
            });
        }
        // Launch-time permission map: mode must be valid for the permission.
        for (permission, mode) in &app.permissions {
            if let Err(message) = crate::permissions::validate_permission_entry(permission, mode) {
                errors.push(ValidationError {
                    message: format!("App '{}': {message}", app.name),
                    kind: ValidationErrorKind::InvalidPermission,
                });
            }
        }
    }

    // 10. Concurrency options, if set, SHALL be >= 1 — a value of 0 would
    //     deadlock the parallel executor's semaphore (zero permits).
    if let Some(ref opts) = flow.flow.options {
        if opts.max_concurrency == Some(0) {
            errors.push(ValidationError {
                message: "max_concurrency SHALL be >= 1 (0 would deadlock the executor)"
                    .to_string(),
                kind: ValidationErrorKind::InvalidConcurrency,
            });
        }
        if opts.suite_concurrency == Some(0) {
            errors.push(ValidationError {
                message: "suite_concurrency SHALL be >= 1 (0 would deadlock the executor)"
                    .to_string(),
                kind: ValidationErrorKind::InvalidConcurrency,
            });
        }
    }

    // Collect named blocks for reference checks
    let mut block_names: HashSet<&str> = HashSet::new();
    let mut seen_names: HashSet<&str> = HashSet::new();

    // 5. Duplicate block names
    for block in &flow.block {
        if let Some(ref name) = block.name {
            if !seen_names.insert(name.as_str()) {
                errors.push(ValidationError {
                    message: format!("Duplicate block name '{name}'"),
                    kind: ValidationErrorKind::DuplicateBlockName,
                });
            }
            block_names.insert(name.as_str());
        }
    }

    // 4. Start block doesn't exist
    if let Some(ref start) = flow.flow.start {
        if !block_names.contains(start.as_str()) {
            errors.push(ValidationError {
                message: format!("Start block '{start}' does not exist"),
                kind: ValidationErrorKind::InvalidStartBlock,
            });
        }
    }

    // Iterate blocks for step and branch validation
    for block in &flow.block {
        // 2. Unknown action
        for step in &block.steps {
            if !KNOWN_ACTIONS.contains(&step.action.as_str()) {
                errors.push(ValidationError {
                    message: format!("Unknown action '{}'", step.action),
                    kind: ValidationErrorKind::UnknownAction,
                });
            }

            // 6. Invalid if_fail
            if let Some(ref if_fail) = step.if_fail {
                if !VALID_ON_FAIL.contains(&if_fail.as_str()) {
                    errors.push(ValidationError {
                        message: format!("Invalid if_fail value '{if_fail}', expected one of: error, warn, ignore"),
                        kind: ValidationErrorKind::InvalidOnFail,
                    });
                }
            }

            // 11. `backspace` operates on the focused field — a selector
            //     can't be honored reliably (a tap-to-focus mis-places the
            //     caret; there's no cross-platform move-to-end), so reject it.
            if step.action == "backspace" && step.has_element_selector() {
                errors.push(ValidationError {
                    message: "backspace does not take a selector — it deletes from the \
                         currently focused field; type or tap the field first"
                        .to_string(),
                    kind: ValidationErrorKind::SelectorNotAllowed,
                });
            }
        }

        // Branch validation
        for branch in &block.branch {
            validate_branch(branch, &block_names, &mut errors);
        }
    }

    errors
}

fn validate_branch(
    branch: &BranchCondition,
    block_names: &HashSet<&str>,
    errors: &mut Vec<ValidationError>,
) {
    // 3. Goto target doesn't exist
    if !block_names.contains(branch.goto.as_str()) {
        errors.push(ValidationError {
            message: format!("Goto target '{}' does not exist", branch.goto),
            kind: ValidationErrorKind::InvalidGotoTarget,
        });
    }

    // 7. Missing goto — already required by struct, but validate non-empty
    if branch.goto.is_empty() {
        errors.push(ValidationError {
            message: "Branch condition has empty goto".to_string(),
            kind: ValidationErrorKind::MissingGoto,
        });
    }

    // 8. Conflicting branch condition — should not have both if_visible/if_not_visible and if_var
    let has_visibility = branch.if_visible.is_some() || branch.if_not_visible.is_some();
    let has_var = branch.if_var.is_some();
    if has_visibility && has_var {
        errors.push(ValidationError {
            message: "Branch condition has both visibility check and if_var".to_string(),
            kind: ValidationErrorKind::ConflictingBranchCondition,
        });
    }

    // 9. Missing comparison — if_var without equals, matches, or gte
    if branch.if_var.is_some()
        && branch.equals.is_none()
        && branch.matches.is_none()
        && branch.gte.is_none()
    {
        errors.push(ValidationError {
            message: "if_var without a comparison (equals, matches, or gte)".to_string(),
            kind: ValidationErrorKind::MissingComparison,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_flow;

    // 1. Valid flow passes validation (empty errors)
    #[test]
    fn valid_flow_passes_validation() {
        let toml_str = r#"
[flow]
name = "valid flow"

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

[[block]]
name = "second"

[[block.steps]]
action = "swipe"

[[block.branch]]
if_visible = "Welcome"
goto = "first"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    // 2. Missing devices on app
    #[test]
    fn missing_devices_on_app() {
        let toml_str = r#"
[flow]
name = "no devices"

[[flow.apps]]
name = "myapp"
bundle = "com.example.app"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ValidationErrorKind::MissingDevices);
        assert!(errors[0].message.contains("myapp"));
    }

    // 3. Unknown action "explode"
    #[test]
    fn unknown_action() {
        let toml_str = r#"
[flow]
name = "bad action"

[[block]]

[[block.steps]]
action = "explode"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ValidationErrorKind::UnknownAction);
        assert!(errors[0].message.contains("explode"));
    }

    // 4. Goto to nonexistent block
    #[test]
    fn goto_nonexistent_block() {
        let toml_str = r#"
[flow]
name = "bad goto"

[[block]]
name = "first"

[[block.branch]]
goto = "nowhere"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ValidationErrorKind::InvalidGotoTarget);
        assert!(errors[0].message.contains("nowhere"));
    }

    // 5. Start block doesn't exist
    #[test]
    fn start_block_doesnt_exist() {
        let toml_str = r#"
[flow]
name = "bad start"
start = "nonexistent"

[[block]]
name = "first"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ValidationErrorKind::InvalidStartBlock);
        assert!(errors[0].message.contains("nonexistent"));
    }

    // 6. Duplicate block names
    #[test]
    fn duplicate_block_names() {
        let toml_str = r#"
[flow]
name = "duplicates"

[[block]]
name = "login"

[[block]]
name = "login"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ValidationErrorKind::DuplicateBlockName);
        assert!(errors[0].message.contains("login"));
    }

    // 7. Invalid if_fail "crash"
    #[test]
    fn invalid_on_fail() {
        let toml_str = r#"
[flow]
name = "bad if_fail"

[[block]]

[[block.steps]]
action = "tap"
if_fail = "crash"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ValidationErrorKind::InvalidOnFail);
        assert!(errors[0].message.contains("crash"));
    }

    // 8. if_var without comparison
    #[test]
    fn if_var_without_comparison() {
        let toml_str = r#"
[flow]
name = "missing comparison"

[[block]]
name = "check"

[[block.branch]]
if_var = "count"
goto = "check"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ValidationErrorKind::MissingComparison);
    }

    // 9. Multiple errors returned at once
    #[test]
    fn multiple_errors_at_once() {
        let toml_str = r#"
[flow]
name = "many errors"
start = "nonexistent"

[[flow.apps]]
name = "myapp"
bundle = "com.example.app"

[[block]]
name = "first"

[[block.steps]]
action = "explode"
if_fail = "crash"

[[block]]
name = "first"

[[block.branch]]
goto = "nowhere"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);

        let kinds: Vec<&ValidationErrorKind> = errors.iter().map(|e| &e.kind).collect();

        // Should have: MissingDevices, DuplicateBlockName, InvalidStartBlock,
        // UnknownAction, InvalidOnFail, InvalidGotoTarget
        assert!(
            kinds.contains(&&ValidationErrorKind::MissingDevices),
            "expected MissingDevices, got: {kinds:?}"
        );
        assert!(
            kinds.contains(&&ValidationErrorKind::DuplicateBlockName),
            "expected DuplicateBlockName, got: {kinds:?}"
        );
        assert!(
            kinds.contains(&&ValidationErrorKind::InvalidStartBlock),
            "expected InvalidStartBlock, got: {kinds:?}"
        );
        assert!(
            kinds.contains(&&ValidationErrorKind::UnknownAction),
            "expected UnknownAction, got: {kinds:?}"
        );
        assert!(
            kinds.contains(&&ValidationErrorKind::InvalidOnFail),
            "expected InvalidOnFail, got: {kinds:?}"
        );
        assert!(
            kinds.contains(&&ValidationErrorKind::InvalidGotoTarget),
            "expected InvalidGotoTarget, got: {kinds:?}"
        );

        assert!(
            errors.len() >= 6,
            "expected at least 6 errors, got {}",
            errors.len()
        );
    }

    // 10. Empty goto string is flagged as MissingGoto (and also InvalidGotoTarget,
    //     since "" is never a registered block name)
    #[test]
    fn empty_goto_is_missing_goto() {
        let toml_str = r#"
[flow]
name = "empty goto"

[[block]]
name = "first"

[[block.branch]]
if_visible = "X"
goto = ""
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        let kinds: Vec<&ValidationErrorKind> = errors.iter().map(|e| &e.kind).collect();
        assert!(
            kinds.contains(&&ValidationErrorKind::MissingGoto),
            "empty goto SHALL produce MissingGoto, got: {kinds:?}"
        );
        assert!(
            kinds.contains(&&ValidationErrorKind::InvalidGotoTarget),
            "empty goto SHALL also be an unknown target, got: {kinds:?}"
        );
    }

    // 11. Branch with both a visibility check and if_var conflicts
    #[test]
    fn conflicting_branch_condition_if_visible_and_if_var() {
        let toml_str = r#"
[flow]
name = "conflict"

[[block]]
name = "first"

[[block.branch]]
if_visible = "Welcome"
if_var = "count"
equals = "1"
goto = "first"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        let kinds: Vec<&ValidationErrorKind> = errors.iter().map(|e| &e.kind).collect();
        assert!(
            kinds.contains(&&ValidationErrorKind::ConflictingBranchCondition),
            "visibility + if_var SHALL conflict, got: {kinds:?}"
        );
        // A valid comparison was supplied, so no MissingComparison.
        assert!(
            !kinds.contains(&&ValidationErrorKind::MissingComparison),
            "comparison present SHALL not flag MissingComparison, got: {kinds:?}"
        );
    }

    // 12. if_not_visible also counts as a visibility check for the conflict path
    #[test]
    fn conflicting_branch_condition_if_not_visible_and_if_var() {
        let toml_str = r#"
[flow]
name = "conflict not visible"

[[block]]
name = "first"

[[block.branch]]
if_not_visible = "Spinner"
if_var = "count"
gte = 3
goto = "first"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        let kinds: Vec<&ValidationErrorKind> = errors.iter().map(|e| &e.kind).collect();
        assert!(
            kinds.contains(&&ValidationErrorKind::ConflictingBranchCondition),
            "if_not_visible + if_var SHALL conflict, got: {kinds:?}"
        );
    }

    // 13. if_var with a matches comparison is accepted (no MissingComparison)
    #[test]
    fn if_var_with_matches_comparison_ok() {
        let toml_str = r#"
[flow]
name = "matches ok"

[[block]]
name = "check"

[[block.branch]]
if_var = "status"
matches = "^done$"
goto = "check"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        assert!(
            errors.is_empty(),
            "if_var + matches SHALL be valid, got: {errors:?}"
        );
    }

    // 14. if_var with a gte comparison is accepted
    #[test]
    fn if_var_with_gte_comparison_ok() {
        let toml_str = r#"
[flow]
name = "gte ok"

[[block]]
name = "check"

[[block.branch]]
if_var = "count"
gte = 5
goto = "check"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        assert!(
            errors.is_empty(),
            "if_var + gte SHALL be valid, got: {errors:?}"
        );
    }

    // 15. lint_within_no_op flags `within` on a non-scroll, non-auto_scroll step
    #[test]
    fn lint_within_flags_non_consuming_step() {
        let toml_str = r#"
[flow]
name = "within no-op"

[[block]]
name = "first"

[[block.steps]]
action = "swipe"
within = { text = "Container" }
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let issues = lint_within_no_op(&flow);
        assert_eq!(issues.len(), 1, "swipe with within SHALL be flagged");
        assert_eq!(issues[0].action, "swipe");
        assert_eq!(issues[0].step_index, 0);
        assert_eq!(issues[0].block_name.as_deref(), Some("first"));
    }

    // 16. lint_within_no_op ignores `within` on a scroll step (it's consumed)
    #[test]
    fn lint_within_ignores_scroll_step() {
        let toml_str = r#"
[flow]
name = "scroll within"

[[block]]

[[block.steps]]
action = "scroll"
within = { text = "Container" }
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let issues = lint_within_no_op(&flow);
        assert!(
            issues.is_empty(),
            "scroll consumes within, SHALL not be flagged: {issues:?}"
        );
    }

    // 17. lint_within_no_op ignores `within` when auto_scroll = true
    #[test]
    fn lint_within_ignores_auto_scroll_step() {
        let toml_str = r#"
[flow]
name = "auto scroll within"

[[block]]

[[block.steps]]
action = "tap"
auto_scroll = true
within = { text = "Container" }
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let issues = lint_within_no_op(&flow);
        assert!(
            issues.is_empty(),
            "auto_scroll consumes within, SHALL not be flagged: {issues:?}"
        );
    }

    // 18. lint_within_no_op skips steps without a within at all
    #[test]
    fn lint_within_skips_steps_without_within() {
        let toml_str = r#"
[flow]
name = "no within"

[[block]]

[[block.steps]]
action = "tap"
text = "OK"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let issues = lint_within_no_op(&flow);
        assert!(issues.is_empty(), "no within means no issue: {issues:?}");
    }

    // 19. lint_push_notification_phys: no phys-capable app => no issues even
    //     when push_notification is used
    #[test]
    fn lint_push_notif_no_phys_capable_apps() {
        let toml_str = r#"
[flow]
name = "virtual only"

[[flow.apps]]
name = "myapp"
bundle = "com.example.app"

[[flow.apps.devices]]
os = "android"

[[block]]

[[block.steps]]
action = "push_notification"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let issues = lint_push_notification_phys(&flow);
        assert!(
            issues.is_empty(),
            "absent hardware defaults to virtual, SHALL not flag: {issues:?}"
        );
    }

    // 20. lint_push_notification_phys: app opts into real hardware AND uses the
    //     action => flagged, resolving target via flow-default first app
    #[test]
    fn lint_push_notif_real_hardware_default_app() {
        let toml_str = r#"
[flow]
name = "phys push"

[[flow.apps]]
name = "myapp"
bundle = "com.example.app"

[[flow.apps.devices]]
os = "android"
hardware = "real"

[[block]]
name = "notify"

[[block.steps]]
action = "push_notification"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let issues = lint_push_notification_phys(&flow);
        assert_eq!(issues.len(), 1, "real-hardware push SHALL be flagged");
        assert_eq!(issues[0].app_name, "myapp");
        assert_eq!(issues[0].step_index, 0);
        assert_eq!(issues[0].block_name.as_deref(), Some("notify"));
    }

    // 21. lint_push_notification_phys: array form ["virtual", "real"] also
    //     counts as phys-capable
    #[test]
    fn lint_push_notif_array_hardware_includes_real() {
        let toml_str = r#"
[flow]
name = "phys push array"

[[flow.apps]]
name = "myapp"
bundle = "com.example.app"

[[flow.apps.devices]]
os = "android"
hardware = ["virtual", "real"]

[[block]]

[[block.steps]]
action = "push_notification"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let issues = lint_push_notification_phys(&flow);
        assert_eq!(issues.len(), 1, "array including real SHALL be flagged");
        assert_eq!(issues[0].app_name, "myapp");
    }

    // 22. lint_push_notification_phys: step.app override targets a non-phys app =>
    //     not flagged even though another app is phys-capable
    #[test]
    fn lint_push_notif_step_app_override_to_virtual() {
        let toml_str = r#"
[flow]
name = "override target"

[[flow.apps]]
name = "physapp"
bundle = "com.example.phys"

[[flow.apps.devices]]
os = "android"
hardware = "real"

[[flow.apps]]
name = "virtualapp"
bundle = "com.example.virtual"

[[flow.apps.devices]]
os = "android"

[[block]]

[[block.steps]]
action = "push_notification"
app = "virtualapp"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let issues = lint_push_notification_phys(&flow);
        assert!(
            issues.is_empty(),
            "push targeting virtual app SHALL not be flagged: {issues:?}"
        );
    }

    // 24. max_concurrency = 0 is rejected (would deadlock the executor)
    #[test]
    fn zero_max_concurrency_rejected() {
        let toml_str = r#"
[flow]
name = "zero concurrency"

[flow.options]
max_concurrency = 0
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        let kinds: Vec<&ValidationErrorKind> = errors.iter().map(|e| &e.kind).collect();
        assert!(
            kinds.contains(&&ValidationErrorKind::InvalidConcurrency),
            "max_concurrency = 0 SHALL be rejected, got: {kinds:?}"
        );
    }

    // 25. suite_concurrency = 0 is rejected (would deadlock the executor)
    #[test]
    fn zero_suite_concurrency_rejected() {
        let toml_str = r#"
[flow]
name = "zero suite concurrency"

[flow.options]
suite_concurrency = 0
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        let kinds: Vec<&ValidationErrorKind> = errors.iter().map(|e| &e.kind).collect();
        assert!(
            kinds.contains(&&ValidationErrorKind::InvalidConcurrency),
            "suite_concurrency = 0 SHALL be rejected, got: {kinds:?}"
        );
    }

    // 26. Concurrency options >= 1 are accepted (no InvalidConcurrency)
    #[test]
    fn positive_concurrency_accepted() {
        let toml_str = r#"
[flow]
name = "valid concurrency"

[flow.options]
max_concurrency = 1
suite_concurrency = 4
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let errors = validate_flow(&flow);
        let kinds: Vec<&ValidationErrorKind> = errors.iter().map(|e| &e.kind).collect();
        assert!(
            !kinds.contains(&&ValidationErrorKind::InvalidConcurrency),
            "concurrency >= 1 SHALL be accepted, got: {kinds:?}"
        );
    }

    // 23. lint_push_notification_phys: non-push steps are ignored even when the
    //     app is phys-capable
    #[test]
    fn lint_push_notif_ignores_non_push_steps() {
        let toml_str = r#"
[flow]
name = "no push action"

[[flow.apps]]
name = "myapp"
bundle = "com.example.app"

[[flow.apps.devices]]
os = "android"
hardware = "real"

[[block]]

[[block.steps]]
action = "tap"
text = "OK"
"#;
        let flow = parse_flow(toml_str).expect("should parse");
        let issues = lint_push_notification_phys(&flow);
        assert!(
            issues.is_empty(),
            "no push_notification step SHALL produce no issues: {issues:?}"
        );
    }

    // ---------------------------------------------------------------
    // lint_unknown_step_fields — typos that serde swept into `params`
    // ---------------------------------------------------------------

    fn lint_keys(step_body: &str) -> Vec<(String, Option<String>)> {
        let toml_str = format!(
            "[flow]\nname = \"t\"\n\n[[block]]\nname = \"b\"\n\n[[block.steps]]\n{step_body}\n"
        );
        let flow = parse_flow(&toml_str).expect("fixture SHALL parse");
        lint_unknown_step_fields(&flow)
            .into_iter()
            .map(|i| (i.key, i.suggestion))
            .collect()
    }

    #[test]
    fn lint_flags_a_misspelled_flat_selector() {
        assert_eq!(
            lint_keys("action = \"tap\"\non_tex = \"Save\""),
            vec![("on_tex".to_string(), Some("on_text".to_string()))]
        );
    }

    #[test]
    fn lint_flags_a_misspelled_modifier_beside_a_working_selector() {
        // The quiet one: the step still resolves, against every "Save" on
        // screen instead of the second.
        assert_eq!(
            lint_keys("action = \"tap\"\non_text = \"Save\"\non_indx = 2"),
            vec![("on_indx".to_string(), Some("on_index".to_string()))]
        );
    }

    #[test]
    fn lint_flags_an_on_prefixed_key_that_resembles_nothing() {
        // No suggestion to offer, but `on_` is golem's prefix, so it is
        // still a typo and not somebody's action parameter.
        assert_eq!(
            lint_keys("action = \"tap\"\non_wibble = \"Save\""),
            vec![("on_wibble".to_string(), None)]
        );
    }

    #[test]
    fn lint_flags_a_misspelled_non_selector_field() {
        assert_eq!(
            lint_keys("action = \"type\"\ninpu = \"hello\""),
            vec![("inpu".to_string(), Some("input".to_string()))]
        );
    }

    #[test]
    fn lint_leaves_real_action_parameters_alone() {
        // Every key an action actually consumes, plus the ones mixin
        // expansion injects. A warning on any of these fires on a correct
        // flow, which is worse than the typo it is looking for.
        for key in [
            "url",
            "body",
            "headers",
            "extract",
            "session",
            "count",
            "direction",
            "label",
            "message",
            "path",
            "payload",
            "permissions",
            "vars",
            "x",
            "y",
            "args",
            "script",
            "run",
            "fixture",
            "inbox",
            "provider",
            "subject",
            "title",
            "button",
            "enabled",
            "latitude",
            "longitude",
            "as",
            "mixin",
        ] {
            let found = lint_keys(&format!("action = \"tap\"\n{key} = \"v\""));
            assert!(
                found.is_empty(),
                "`{key}` is a real parameter and SHALL NOT be flagged, got {found:?}"
            );
        }
    }

    #[test]
    fn a_correct_flat_selector_never_reaches_the_lint() {
        // Why the `on_` rule needs no exemption list: serde binds the real
        // ones to named fields, so only a misspelling can reach `params`.
        let flow = parse_flow(
            "[flow]\nname = \"t\"\n\n[[block]]\nname = \"b\"\n\n[[block.steps]]\n\
             action = \"tap\"\non_text = \"Save\"\non_index = 2\non_enabled = true\n",
        )
        .expect("fixture SHALL parse");
        assert!(
            flow.block[0].steps[0].params.is_empty(),
            "spelled-correctly selectors SHALL be fields, not params: {:?}",
            flow.block[0].steps[0].params
        );
        assert!(lint_unknown_step_fields(&flow).is_empty());
    }

    #[test]
    fn lint_stays_quiet_when_two_fields_are_equally_close() {
        // `tn` is one substitution from both `to` and `on`. Naming one of
        // them would be a coin flip, and warning without a name would be
        // noise on a key that may well be a parameter — so say nothing.
        assert!(
            lint_keys("action = \"tap\"\ntn = \"X\"").is_empty(),
            "an ambiguous near-miss SHALL NOT be guessed at"
        );
    }

    #[test]
    fn within_one_edit_is_bounded_at_one() {
        assert!(within_one_edit("on_text", "on_tex")); // deletion
        assert!(within_one_edit("on_index", "on_indx")); // substitution + shift
        assert!(within_one_edit("input", "inputs")); // insertion
        assert!(within_one_edit("scale", "scale")); // identical
        assert!(!within_one_edit("on_text", "on_txet")); // two substitutions
        assert!(!within_one_edit("app", "as"));
        assert!(!within_one_edit("on", "to"));
        assert!(!within_one_edit("points", "path"));
    }
}
