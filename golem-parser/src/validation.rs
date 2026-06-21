use crate::{BranchCondition, FlowFile};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub message: String,
    pub kind: ValidationErrorKind,
}

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
    "dark_mode",
    "rotate",
    "grant_permission",
    "revoke_permission",
    "push_notification",
    "open_link",
    "screenshot",
    "add_media",
    "load_fixture",
    "load_mixin",
    "run",
    "bash",
    "http_get",
    "http_post",
    "http_put",
    "http_patch",
    "http_delete",
    "await_email",
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
}
