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
    "assert_text",
    "assert_enabled",
    "assert_checked",
    "fail",
    "set_location",
    "dark_mode",
    "rotate",
    "grant_permission",
    "revoke_permission",
    "push_notification",
    "open_link",
    "screenshot",
    "start_recording",
    "stop_recording",
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
}
