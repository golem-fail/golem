//! The canonical step notation: one step as a one-line TOML inline table,
//! `{ action = "tap", on_text = "Sign in" }` — the same text as a step in
//! a flow file's `steps = [ … ]` array. `golem probe`, `golem session` and
//! the MCP tools take a step in this form.

use anyhow::{bail, Result};
use serde::Deserialize;
use toml_edit::{DocumentMut, InlineTable, Item, Key, Value};

use crate::validation::{
    missing_params, nearest_action, unknown_step_keys, validate_step, ValidationErrorKind,
};
use crate::Step;

/// The example every notation error shows.
pub const STEP_EXAMPLE: &str = r#"{ action = "tap", on_text = "Sign in" }"#;

/// The example a selector error shows.
pub const SELECTOR_EXAMPLE: &str = r#"{ on_text = "Sign in" }"#;

/// A step parsed from the canonical notation.
#[derive(Debug, Clone)]
pub struct InlineStep {
    pub step: Step,
    /// The step in canonical form: one line, `{ key = value, … }`, with the
    /// keys in the order written and each value spelled as written.
    pub line: String,
}

/// Parse one step written as a TOML inline table. The outer braces are
/// optional. The step passes the per-step checks a flow runs, and an
/// unknown `on_*` key, or a key one edit from a step field, is an error.
pub fn parse_step_inline(input: &str) -> Result<InlineStep> {
    let table = parse_table(input, STEP_EXAMPLE)?;
    let pairs = top_level_pairs(&table);
    let line = render(&pairs);
    let step = deserialize(&line, STEP_EXAMPLE)?;

    if step.action.is_empty() {
        bail!(
            "the step has no action; add one, e.g. {}",
            with_pair(&pairs, "action", r#""tap""#, true)
        );
    }

    let mut problems = Vec::new();
    for error in validate_step(&step) {
        problems.push(match error.kind {
            ValidationErrorKind::UnknownAction => unknown_action_message(&step.action, &pairs),
            ValidationErrorKind::MissingParam => {
                let mut fixed = pairs.clone();
                for param in missing_params(&step) {
                    fixed.push((Key::new(param).display_repr().into_owned(), "…".into()));
                }
                format!("{}; e.g. {}", error.message, render(&fixed))
            }
            _ => error.message,
        });
    }
    problems.extend(unknown_key_messages(&step, &pairs));
    if !problems.is_empty() {
        bail!("{}", problems.join("\n"));
    }
    Ok(InlineStep { step, line })
}

/// Parse a selector written in the step notation, for `probe`. The outer
/// braces are optional, `action` is dropped, and at least one element
/// criterion (`on_*`, or `on`/`to`) is required.
pub fn parse_selector_inline(input: &str) -> Result<InlineStep> {
    let mut table = parse_table(input, SELECTOR_EXAMPLE)?;
    table.remove("action");
    let pairs = top_level_pairs(&table);
    let line = render(&pairs);
    let step = deserialize(&line, SELECTOR_EXAMPLE)?;
    let problems = unknown_key_messages(&step, &pairs);
    if !problems.is_empty() {
        bail!("{}", problems.join("\n"));
    }
    if !step.has_element_selector() {
        bail!("the selector has no element criterion; add one, e.g. {SELECTOR_EXAMPLE}");
    }
    Ok(InlineStep { step, line })
}

fn parse_table(input: &str, example: &str) -> Result<InlineTable> {
    let text = input.trim();
    if text.is_empty() {
        bail!("the step is empty; write it as one TOML inline table, e.g. {example}");
    }
    if text.contains('\n') {
        bail!("{}", multi_line_message(text, example));
    }
    // TOML would accept a comment after the closing brace and drop it.
    if shape(text).comment {
        bail!("{}", comment_message(example));
    }
    let body = if text.starts_with('{') {
        text.to_string()
    } else {
        format!("{{ {text} }}")
    };
    let doc = match format!("step = {body}").parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(err) => bail!("{}", syntax_message(text, &err.to_string(), example)),
    };
    match doc.get("step") {
        Some(Item::Value(Value::InlineTable(table))) if doc.len() == 1 => Ok(table.clone()),
        _ => bail!("the step is not one inline table; write it as e.g. {example}"),
    }
}

fn deserialize(line: &str, example: &str) -> Result<Step> {
    #[derive(Deserialize)]
    struct Wrapper {
        step: Step,
    }
    match toml::from_str::<Wrapper>(&format!("step = {line}")) {
        Ok(w) => Ok(w.step),
        Err(err) => bail!("invalid step: {}; e.g. {example}", err.message()),
    }
}

/// A multi-line step is rejected. If it is valid as `key = value` lines,
/// the message shows the same step on one line.
fn multi_line_message(text: &str, example: &str) -> String {
    if let Ok(doc) = text.parse::<DocumentMut>() {
        let mut table = InlineTable::new();
        let mut convertible = true;
        for (key, item) in doc.iter() {
            match item.as_value() {
                Some(value) => {
                    table.insert(key, value.clone());
                }
                None => convertible = false,
            }
        }
        if convertible && !table.is_empty() {
            return format!(
                "a step is one line, not one key per line; write it as {}",
                render(&top_level_pairs(&table))
            );
        }
    }
    format!("a step is one line; write it as one TOML inline table, e.g. {example}")
}

/// The shape of a step's text outside its quoted strings.
#[derive(Default)]
struct Shape {
    comment: bool,
    equals: usize,
    commas: usize,
    /// The last character outside a string before the outer `}`, or before
    /// the end of unbraced text.
    last: Option<char>,
}

fn shape(text: &str) -> Shape {
    let braced = text.starts_with('{');
    let base = usize::from(braced);
    let mut s = Shape::default();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in text.chars() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if q == '"' && c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '#' => s.comment = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth = depth.saturating_sub(1),
            '=' if depth == base => s.equals += 1,
            ',' if depth == base => s.commas += 1,
            _ => {}
        }
        // The outer `}` closes depth `base` back to 0.
        let outer_close = braced && c == '}' && depth == 0;
        if !(c.is_whitespace() || outer_close) {
            s.last = Some(c);
        }
    }
    s
}

fn comment_message(example: &str) -> String {
    format!("a step cannot contain a `#` comment; pass the comment separately, e.g. {example}")
}

fn syntax_message(text: &str, err: &str, example: &str) -> String {
    let s = shape(text);
    if s.last == Some(',') {
        return format!(
            "a step cannot end with a comma: TOML does not allow a trailing comma in an \
             inline table; e.g. {example}"
        );
    }
    if s.equals > s.commas + 1 {
        return format!("separate the key/value pairs with commas, e.g. {example}");
    }
    let reason = err
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("invalid TOML");
    format!("invalid step: {reason}; write it as one TOML inline table, e.g. {example}")
}

fn unknown_action_message(action: &str, pairs: &[(String, String)]) -> String {
    match nearest_action(action) {
        Some(near) => format!(
            "unknown action '{action}'; did you mean {}",
            with_pair(pairs, "action", &format!("\"{near}\""), false)
        ),
        None => format!("unknown action '{action}'; e.g. {STEP_EXAMPLE}"),
    }
}

fn unknown_key_messages(step: &Step, pairs: &[(String, String)]) -> Vec<String> {
    unknown_step_keys(step)
        .into_iter()
        .map(|(key, suggestion)| match suggestion {
            Some(field) => {
                let fixed: Vec<(String, String)> = pairs
                    .iter()
                    .map(|(k, v)| {
                        let k = if *k == Key::new(key.as_str()).display_repr() {
                            Key::new(field.as_str()).display_repr().into_owned()
                        } else {
                            k.clone()
                        };
                        (k, v.clone())
                    })
                    .collect();
                format!("unknown key '{key}'; did you mean {}", render(&fixed))
            }
            None => format!("unknown key '{key}'; e.g. {STEP_EXAMPLE}"),
        })
        .collect()
}

/// `pairs` with `key` set to `value`: replaced in place when present,
/// otherwise added first (`front`) or last.
fn with_pair(pairs: &[(String, String)], key: &str, value: &str, front: bool) -> String {
    let mut fixed = pairs.to_vec();
    if let Some(pair) = fixed.iter_mut().find(|(k, _)| k == key) {
        pair.1 = value.to_string();
    } else if front {
        fixed.insert(0, (key.to_string(), value.to_string()));
    } else {
        fixed.push((key.to_string(), value.to_string()));
    }
    render(&fixed)
}

fn top_level_pairs(table: &InlineTable) -> Vec<(String, String)> {
    table
        .iter()
        .map(|(k, v)| (Key::new(k).display_repr().into_owned(), render_value(v)))
        .collect()
}

fn render(pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return "{}".into();
    }
    let body: Vec<String> = pairs.iter().map(|(k, v)| format!("{k} = {v}")).collect();
    format!("{{ {} }}", body.join(", "))
}

fn render_value(value: &Value) -> String {
    match value {
        Value::InlineTable(t) => render(&top_level_pairs(t)),
        Value::Array(a) => {
            let items: Vec<String> = a.iter().map(render_value).collect();
            format!("[{}]", items.join(", "))
        }
        scalar => {
            let mut v = scalar.clone();
            v.decor_mut().clear();
            v.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::known_actions;

    fn err(input: &str) -> String {
        parse_step_inline(input)
            .expect_err("SHALL be rejected")
            .to_string()
    }

    #[test]
    fn braces_are_optional() {
        let braced = parse_step_inline(r#"{ action = "tap", on_text = "OK" }"#).expect("braced");
        let bare = parse_step_inline(r#"action = "tap", on_text = "OK""#).expect("bare");
        assert_eq!(braced.line, bare.line);
        assert_eq!(bare.line, r#"{ action = "tap", on_text = "OK" }"#);
        assert_eq!(bare.step.action, "tap");
        assert_eq!(bare.step.on_text.as_deref(), Some("OK"));
    }

    #[test]
    fn unbraced_text_that_ends_in_a_nested_table_gets_outer_braces() {
        let s = parse_step_inline(r#"action = "load_mixin", mixin = "m", vars = { a = "1" }"#)
            .expect("SHALL parse");
        assert_eq!(
            s.line,
            r#"{ action = "load_mixin", mixin = "m", vars = { a = "1" } }"#
        );
    }

    #[test]
    fn messy_spacing_renders_canonically() {
        let s = parse_step_inline(
            "  {action=\"assert_visible\",on_text = \"Hi\" ,   auto_scroll=true}  ",
        )
        .expect("SHALL parse");
        assert_eq!(
            s.line,
            r#"{ action = "assert_visible", on_text = "Hi", auto_scroll = true }"#
        );
    }

    #[test]
    fn nested_tables_and_arrays_parse_and_render() {
        let s = parse_step_inline(
            r#"{ action = "tap", on = { text = "Save", traits = ["button", "large"], below = { text = "Name" } } }"#,
        )
        .expect("SHALL parse");
        let on = s.step.on.as_ref().expect("on");
        assert_eq!(on.text.as_deref(), Some("Save"));
        assert_eq!(on.traits, ["button", "large"]);
        assert_eq!(
            s.line,
            r#"{ action = "tap", on = { text = "Save", traits = ["button", "large"], below = { text = "Name" } } }"#
        );
    }

    #[test]
    fn unknown_action_suggests_the_near_one() {
        let e = err(r#"{ action = "tapp", on_text = "OK" }"#);
        assert_eq!(
            e,
            r#"unknown action 'tapp'; did you mean { action = "tap", on_text = "OK" }"#
        );
        let e = err(r#"{ action = "explode" }"#);
        assert_eq!(e, format!("unknown action 'explode'; e.g. {STEP_EXAMPLE}"));
    }

    #[test]
    fn missing_action_is_rejected() {
        let e = err(r#"{ on_text = "OK" }"#);
        assert_eq!(
            e,
            r#"the step has no action; add one, e.g. { action = "tap", on_text = "OK" }"#
        );
    }

    #[test]
    fn missing_required_param_shows_where_it_goes() {
        let e = err(r#"{ action = "open_link" }"#);
        assert_eq!(
            e,
            r#"open_link requires 'url'; e.g. { action = "open_link", url = … }"#
        );
        let e = err(r#"{ action = "set_location", latitude = 35.6 }"#);
        assert_eq!(
            e,
            r#"set_location requires 'longitude'; e.g. { action = "set_location", latitude = 35.6, longitude = … }"#
        );
    }

    #[test]
    fn unknown_key_suggests_the_field() {
        let e = err(r#"{ action = "tap", on_txt = "OK" }"#);
        assert_eq!(
            e,
            r#"unknown key 'on_txt'; did you mean { action = "tap", on_text = "OK" }"#
        );
        let e = err(r#"{ action = "tap", on_whatever = "OK" }"#);
        assert_eq!(e, format!("unknown key 'on_whatever'; e.g. {STEP_EXAMPLE}"));
    }

    #[test]
    fn other_step_checks_still_apply() {
        assert_eq!(
            err(r#"{ action = "tap", on_text = "OK", if_fail = "crash" }"#),
            "Invalid if_fail value 'crash', expected one of: error, warn, ignore"
        );
        assert!(err(r#"{ action = "backspace", on_text = "OK" }"#)
            .starts_with("backspace does not take a selector"));
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let e = err(r#"{ action = "open_link", on_txt = "x" }"#);
        assert_eq!(e.lines().count(), 2, "{e}");
    }

    #[test]
    fn a_multi_line_step_is_rejected_with_its_one_line_form() {
        let e = err("action = \"tap\"\non_text = \"OK\"\n# why\n");
        assert_eq!(
            e,
            r#"a step is one line, not one key per line; write it as { action = "tap", on_text = "OK" }"#
        );
        let e = err("{ action = \"tap\",\n  on_text = \"OK\" }");
        assert_eq!(
            e,
            format!("a step is one line; write it as one TOML inline table, e.g. {STEP_EXAMPLE}")
        );
    }

    #[test]
    fn a_comment_inside_the_step_is_rejected() {
        let e = err(r#"{ action = "tap", on_text = "OK" } # open it"#);
        assert_eq!(
            e,
            format!(
                "a step cannot contain a `#` comment; pass the comment separately, e.g. {STEP_EXAMPLE}"
            )
        );
        // A `#` inside a string is text, not a comment.
        parse_step_inline(r##"{ action = "tap", on_text = "#1" }"##).expect("SHALL parse");
    }

    #[test]
    fn space_separated_pairs_are_rejected() {
        let e = err(r#"action="tap" on_text="OK""#);
        assert_eq!(
            e,
            format!("separate the key/value pairs with commas, e.g. {STEP_EXAMPLE}")
        );
    }

    #[test]
    fn a_trailing_comma_is_rejected() {
        for input in [
            r#"{ action = "tap", on_text = "OK", }"#,
            r#"action = "tap", on_text = "OK","#,
        ] {
            assert!(
                err(input).starts_with("a step cannot end with a comma"),
                "{input}: {}",
                err(input)
            );
        }
    }

    #[test]
    fn other_syntax_errors_name_the_toml_reason() {
        let e = err(r#"{ action = "tap", on_text = }"#);
        assert!(e.starts_with("invalid step: "), "{e}");
        assert!(e.ends_with(&format!("e.g. {STEP_EXAMPLE}")), "{e}");
        assert!(!e.contains('\n'), "{e}");
    }

    #[test]
    fn empty_input_is_rejected() {
        assert_eq!(
            err("   "),
            format!("the step is empty; write it as one TOML inline table, e.g. {STEP_EXAMPLE}")
        );
    }

    #[test]
    fn a_wrong_value_type_is_rejected() {
        let e = err(r#"{ action = "tap", on_index = "first" }"#);
        assert!(e.starts_with("invalid step: "), "{e}");
    }

    #[test]
    fn selector_drops_the_action_and_needs_a_criterion() {
        let s = parse_selector_inline(r#"action = "tap", on_text = "OK", on_index = 1"#)
            .expect("SHALL parse");
        assert_eq!(s.line, r#"{ on_text = "OK", on_index = 1 }"#);
        assert_eq!(s.step.action, "");
        parse_selector_inline(r#"{ on = { traits = ["button"] } }"#).expect("grouped");

        let e = parse_selector_inline(r#"{ action = "tap" }"#)
            .expect_err("no criterion")
            .to_string();
        assert_eq!(
            e,
            format!("the selector has no element criterion; add one, e.g. {SELECTOR_EXAMPLE}")
        );
        let e = parse_selector_inline(r#"{ on_txt = "OK" }"#)
            .expect_err("typo")
            .to_string();
        assert_eq!(
            e,
            r#"unknown key 'on_txt'; did you mean { on_text = "OK" }"#
        );
    }

    /// One canonical line per action. Each SHALL parse, pass validation,
    /// and render back to itself.
    const SAMPLES: &[&str] = &[
        r#"{ action = "accept_alert" }"#,
        r#"{ action = "add_media", path = "fixtures/cat.jpg" }"#,
        r#"{ action = "assert_alert", on_text = "Delete?" }"#,
        r#"{ action = "assert_not_visible", on_text = "Loading" }"#,
        r#"{ action = "assert_visible", on_text = "Hi, Test", auto_scroll = true }"#,
        r#"{ action = "await_email", inbox = "${inbox}", save_to = "mail", timeout = 60000 }"#,
        r#"{ action = "backspace", count = 3 }"#,
        r#"{ action = "bash", run = "echo ok", save_to = "out" }"#,
        r##"{ action = "browse_assert_exists", selector = "#login" }"##,
        r##"{ action = "browse_assert_not_exists", selector = "#error" }"##,
        r#"{ action = "browse_assert_text", selector = "h1", text = "Welcome" }"#,
        r#"{ action = "browse_close" }"#,
        r#"{ action = "browse_execute_js", script = "return document.title", save_to = "title" }"#,
        r#"{ action = "browse_get_cookie", name = "sid", save_to = "sid" }"#,
        r#"{ action = "browse_get_local_storage", key = "token", save_to = "token" }"#,
        r#"{ action = "browse_get_session_storage", key = "cart", save_to = "cart" }"#,
        r#"{ action = "browse_mcp_call", tool = "search", args = { q = "golem" } }"#,
        r#"{ action = "browse_mcp_list_tools", save_to = "tools" }"#,
        r#"{ action = "browse_navigate", url = "https://example.test/login" }"#,
        r#"{ action = "browse_read", selector = ".total", save_to = "total" }"#,
        r#"{ action = "browse_screenshot", path = "shots/web.png" }"#,
        r#"{ action = "browse_scroll_by", y = 400 }"#,
        r##"{ action = "browse_scroll_to", selector = "#footer" }"##,
        r##"{ action = "browse_select", selector = "#country", value = "JP" }"##,
        r#"{ action = "browse_set_cookie", name = "sid", value = "abc" }"#,
        r#"{ action = "browse_set_local_storage", key = "token", value = "t" }"#,
        r#"{ action = "browse_set_session_storage", key = "cart", value = "[]" }"#,
        r##"{ action = "browse_tap", selector = "#submit" }"##,
        r##"{ action = "browse_type", selector = "#email", text = "a@b.test" }"##,
        r#"{ action = "browse_wait_exists", selector = ".done", timeout = 5000 }"#,
        r#"{ action = "browse_wait_not_exists", selector = ".spinner" }"#,
        r#"{ action = "clear_data", app = "app" }"#,
        r#"{ action = "clear_text" }"#,
        r#"{ action = "create_inbox", provider = "mailpit", save_to = "inbox" }"#,
        r#"{ action = "delete_http", url = "https://api.test/items/1" }"#,
        r#"{ action = "dismiss_alert" }"#,
        r#"{ action = "double_tap", on_text = "Photo" }"#,
        r#"{ action = "fail", message = "unreachable" }"#,
        r#"{ action = "gesture", fingers = [{ points = [{ x = "40%", y = "50%" }, { x = "20%", y = "50%" }] }, { points = [{ x = "60%", y = "50%" }, { x = "80%", y = "50%" }] }] }"#,
        r#"{ action = "get_http", url = "https://api.test/items", save_to = "items" }"#,
        r#"{ action = "hide_keyboard" }"#,
        r#"{ action = "launch", app = "app", restart = true }"#,
        r#"{ action = "load_fixture", fixture = "users", as = "user" }"#,
        r#"{ action = "load_mixin", mixin = "launch_and_wait", vars = { app_bundle = "app", wait_element = "Submit" } }"#,
        r#"{ action = "long_press", on_text = "Item 1", duration = 1500 }"#,
        r#"{ action = "open_link", url = "golem://settings" }"#,
        r#"{ action = "patch_http", url = "https://api.test/items/1", body = { done = true } }"#,
        r#"{ action = "pinch", on_text = "Map", scale = 2.0 }"#,
        r#"{ action = "post_http", url = "https://api.test/items", body = { name = "x" } }"#,
        r#"{ action = "press", button = "back" }"#,
        r#"{ action = "push_notification", title = "Hello", body = "World" }"#,
        r#"{ action = "put_http", url = "https://api.test/items/1" }"#,
        r#"{ action = "read", on_below = "Total", save_to = "total" }"#,
        r#"{ action = "rotate", on_text = "Map", rotation = 90.0 }"#,
        r#"{ action = "run", script = "scripts/seed.sh" }"#,
        r#"{ action = "screenshot", path = "shots/home.png" }"#,
        r#"{ action = "scroll", direction = "down", within = { text = "List" } }"#,
        r#"{ action = "set_dark_mode", enabled = true }"#,
        r#"{ action = "set_location", latitude = 35.68, longitude = 139.76 }"#,
        r#"{ action = "stop", app = "app" }"#,
        r#"{ action = "swipe", start = { x = "50%", y = "80%" }, end = { x = "50%", y = "20%" } }"#,
        r#"{ action = "tap", on_text = "Sign in" }"#,
        r#"{ action = "type", on_accessibility_label = "Email", input = "test@acme.com" }"#,
    ];

    #[test]
    fn every_action_round_trips() {
        let mut sampled: Vec<String> = Vec::new();
        for line in SAMPLES {
            let parsed =
                parse_step_inline(line).unwrap_or_else(|e| panic!("{line} SHALL parse: {e}"));
            assert_eq!(&parsed.line, line, "SHALL render back to itself");
            sampled.push(parsed.step.action);
        }
        sampled.sort();
        let mut known: Vec<String> = known_actions().iter().map(|a| a.to_string()).collect();
        known.sort();
        assert_eq!(
            sampled, known,
            "SAMPLES SHALL cover every known action once"
        );
    }
}
