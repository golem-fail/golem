//! Recognise a React Native error overlay in a captured tree, so a step that
//! failed against one is reported as a broken app rather than a missing
//! selector.
//!
//! React Native has two, and which one appears says *when* the app broke:
//!
//! - The **native redbox**, when the JS bundle never ran — the bundler was
//!   unreachable, the bundle didn't compile, or evaluating it threw. Native
//!   views, so it renders with no JS at all. Android only in practice: the
//!   same failures leave an iOS simulator showing a blank screen (#201).
//! - The **JS LogBox**, when the app mounted and then threw — a press
//!   handler, an effect, a render. Being React, it needs a working bundle,
//!   and it renders identically on both platforms.
//!
//! Both matched on their chrome rather than their message, because the
//! message is the error and changes every time while the chrome does not.
//! Every string here was read off a real overlay on an Android emulator and
//! an iOS 26 simulator (Expo 57 / RN 0.86) and cross-checked against React
//! Native's own source.

use golem_element::Element;

/// Whether `text` is one of the redbox's footer buttons. The keyboard hint is
/// required: an app of its own with a "Dismiss" or "Reload" button is
/// ordinary, one with "DISMISS\n(ESC)" is React Native's overlay. RN writes
/// the reload hint with a non-breaking space ("(R,\u{a0}R)"), so the match
/// stops before it.
fn is_dismiss(text: &str) -> bool {
    let t = text.trim();
    t.starts_with("DISMISS") && t.contains("(ESC)")
}

fn is_reload(text: &str) -> bool {
    let t = text.trim();
    t.starts_with("RELOAD") && t.contains("(R")
}

/// What the app is showing instead of its UI, if that's an error overlay.
///
/// `None` when the app is rendering normally — including when a LogBox is
/// minimised to its badge, or is open on a console warning, both of which
/// leave a working app underneath.
pub fn dev_bundle_error(root: &Element) -> Option<String> {
    native_redbox(root).or_else(|| js_logbox(root))
}

/// The native redbox's message.
fn native_redbox(root: &Element) -> Option<String> {
    // Narrow to the overlay before reading any text. Searching the whole tree
    // would let the app's own content behind the overlay supply the "title".
    let overlay = overlay_root(root)?;
    let title = first_text(overlay)?;
    Some(first_line(title))
}

// ---------------------------------------------------------------------------
// JS LogBox
// ---------------------------------------------------------------------------

/// The titles LogBox gives a log that means the app itself is broken
/// (`headerTitleMap` in `LogBoxInspectorBody.js`).
///
/// `Console Warning` and `Console Error` are deliberately absent: those open
/// over an app that still works, and a flow failing behind one is failing on
/// its own merits.
const FATAL_TITLES: [&str; 3] = ["Uncaught Error", "Syntax Error", "Render Error"];

/// The header a compile failure gets instead of "Log N of M"
/// (`LogBoxInspectorHeader.js`). Its footer carries no buttons at all, so this
/// string is the only thing identifying that variant.
const SYNTAX_HEADER: &str = "Failed to compile";

/// Chrome: section labels and controls, never the error.
const CHROME: [&str; 6] = [
    "Source",
    "Call Stack",
    "Dismiss",
    "Minimize",
    "Copy",
    "This error cannot be dismissed.",
];

/// `Log 3 of 7` — the inspector's header for every level but syntax.
fn is_log_counter(text: &str) -> bool {
    let rest = match text.trim().strip_prefix("Log ") {
        Some(r) => r,
        None => return false,
    };
    match rest.split_once(" of ") {
        Some((n, total)) => {
            !n.is_empty()
                && !total.is_empty()
                && n.chars().all(|c| c.is_ascii_digit())
                && total.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// A code-frame line. LogBox prefixes each with U+200E (LEFT-TO-RIGHT MARK) to
/// stop the gutter reordering under RTL, which makes them easy to skip.
fn is_code_frame(text: &str) -> bool {
    text.starts_with('\u{200e}')
}

/// `App.tsx (35:43)` — the source position LogBox renders under the code
/// frame. The redbox has no equivalent, so this is the one thing the LogBox
/// path can report that the redbox path cannot.
fn is_source_position(text: &str) -> bool {
    let t = text.trim();
    let Some(open) = t.rfind(" (") else {
        return false;
    };
    let Some(inner) = t[open + 2..].strip_suffix(')') else {
        return false;
    };
    match inner.split_once(':') {
        Some((line, col)) => {
            !line.is_empty()
                && !col.is_empty()
                && line.chars().all(|c| c.is_ascii_digit())
                && col.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// The JS LogBox inspector's message, when it is open on a fatal log.
///
/// Identified by its header plus a title that means the app is broken, rather
/// than by the Dismiss/Minimize/Copy trio — a syntax-error LogBox renders no
/// buttons at all, and an app could plausibly have a Dismiss/Copy pair of its
/// own.
fn js_logbox(root: &Element) -> Option<String> {
    let mut texts = Vec::new();
    all_texts(root, &mut texts);

    let title_at = texts.iter().position(|t| {
        let t = t.trim();
        FATAL_TITLES.contains(&t) || t == SYNTAX_HEADER
    })?;
    let header_present = texts
        .iter()
        .any(|t| is_log_counter(t) || t.trim() == SYNTAX_HEADER);
    if !header_present {
        return None;
    }

    // The message is the first real text after the title; everything between
    // is chrome or code frame.
    let message = texts[title_at + 1..].iter().find(|t| {
        let s = t.trim();
        !s.is_empty()
            && !CHROME.contains(&s)
            && !is_code_frame(t)
            && !is_log_counter(s)
            && !FATAL_TITLES.contains(&s)
    })?;

    let position = texts[title_at..]
        .iter()
        .find(|t| is_source_position(t) && !is_code_frame(t));

    let message = first_line(message);
    Some(match position {
        Some(pos) => format!("{message} at {}", pos.trim()),
        None => message,
    })
}

/// Every non-empty text and accessibility label, in document order.
fn all_texts<'a>(node: &'a Element, out: &mut Vec<&'a str>) {
    for candidate in [node.text.as_deref(), node.accessibility_label.as_deref()]
        .into_iter()
        .flatten()
    {
        if !candidate.is_empty() && out.last() != Some(&candidate) {
            out.push(candidate);
        }
    }
    for child in &node.children {
        all_texts(child, out);
    }
}

/// The innermost node holding the whole overlay — both footer buttons *and*
/// the message — rather than whatever window it was mounted in.
///
/// RN puts the buttons in their own row beside the message, so "innermost node
/// containing both buttons" is that row and has no message in it. Requiring a
/// message too stops the descent one level higher, at the overlay itself.
fn overlay_root(node: &Element) -> Option<&Element> {
    if !is_overlay(node) {
        return None;
    }
    for child in &node.children {
        if let Some(inner) = overlay_root(child) {
            return Some(inner);
        }
    }
    Some(node)
}

/// Whether this subtree holds a complete overlay: both buttons and a message.
fn is_overlay(node: &Element) -> bool {
    let (mut dismiss, mut reload) = (false, false);
    let mut texts = Vec::new();
    collect(node, &mut dismiss, &mut reload, &mut texts);
    dismiss
        && reload
        && texts
            .iter()
            .any(|t| !is_dismiss(t) && !is_reload(t) && !t.trim().is_empty())
}

/// First non-button text in document order. RN renders the title above the
/// stack trace, so the first one is the message.
fn first_text(node: &Element) -> Option<&str> {
    for candidate in [node.text.as_deref(), node.accessibility_label.as_deref()]
        .into_iter()
        .flatten()
    {
        if !candidate.is_empty() && !is_dismiss(candidate) && !is_reload(candidate) {
            return Some(candidate);
        }
    }
    node.children.iter().find_map(first_text)
}

/// The overlay title's first meaningful line — the message, without the stack
/// trace RN appends after a blank line.
fn first_line(title: &str) -> String {
    title
        .split('\n')
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

fn collect<'a>(node: &'a Element, dismiss: &mut bool, reload: &mut bool, texts: &mut Vec<&'a str>) {
    for candidate in [node.text.as_deref(), node.accessibility_label.as_deref()]
        .into_iter()
        .flatten()
    {
        if candidate.is_empty() {
            continue;
        }
        *dismiss |= is_dismiss(candidate);
        *reload |= is_reload(candidate);
        texts.push(candidate);
    }
    for child in &node.children {
        collect(child, dismiss, reload, texts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_element::Bounds;

    fn node(kind: &str, text: Option<&str>, children: Vec<Element>) -> Element {
        Element {
            element_type: kind.into(),
            text: text.map(str::to_string),
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: false,
            focused: false,
            bounds: Bounds {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            visible_bounds: None,
            hit_points: Vec::new(),
            drawing_order: None,
            children,
        }
    }

    /// An overlay in the shape a device actually produces (Expo 57 / RN 0.86,
    /// Android emulator): the message in a ListView, the buttons in their own
    /// row beside it, both under the overlay container. The nesting is
    /// load-bearing — a flat fixture cannot tell `overlay_root` from the
    /// button row.
    fn redbox(title: &str) -> Element {
        node(
            "LinearLayout",
            None,
            vec![
                node(
                    "ListView",
                    None,
                    vec![
                        node("TextView", Some(title), vec![]),
                        node("TextView", Some("loadJSBundleFromAssets"), vec![]),
                        node("TextView", Some("ReactInstance.kt:86"), vec![]),
                    ],
                ),
                node("LinearLayout", None, vec![]),
                node(
                    "LinearLayout",
                    None,
                    vec![
                        node("Button", Some("DISMISS\n(ESC)"), vec![]),
                        node("Button", Some("RELOAD\n(R,\u{a0}R)"), vec![]),
                    ],
                ),
            ],
        )
    }

    #[test]
    fn an_unreachable_bundler_is_a_dev_bundle_error() {
        let tree = redbox(
            "Unable to load script.\n\nMake sure you're running Metro or that your \
             bundle 'index.android.bundle' is packaged correctly for release.",
        );
        assert_eq!(
            dev_bundle_error(&tree).as_deref(),
            Some("Unable to load script."),
            "the title's first line SHALL be the reported message"
        );
    }

    #[test]
    fn a_transform_error_is_a_dev_bundle_error() {
        let tree =
            redbox("SyntaxError: /p/App.tsx: Unexpected token (9:15)\n\n   7 | // a comment\n");
        assert_eq!(
            dev_bundle_error(&tree).as_deref(),
            Some("SyntaxError: /p/App.tsx: Unexpected token (9:15)"),
            "the code frame SHALL be dropped, the message kept"
        );
    }

    #[test]
    fn a_runtime_throw_is_a_dev_bundle_error() {
        let tree = redbox("[runtime not ready]: Error: boom, stack:\nanonymous@92902:17");
        assert_eq!(
            dev_bundle_error(&tree).as_deref(),
            Some("[runtime not ready]: Error: boom, stack:")
        );
    }

    // -- JS LogBox ---------------------------------------------------

    /// The inspector as a device renders it, in document order, taken from a
    /// real `*_error_tree.json`: header, title, message, the Source section
    /// with its code frame, the position, the call stack, then the controls.
    /// `level` picks the title; `extra` appends anything a variant adds.
    fn logbox(header: &str, title: &str, message: &str, controls: Vec<&str>) -> Element {
        let mut kids = vec![
            node("text", Some(header), vec![]),
            node("text", Some(title), vec![]),
            node("text", Some(message), vec![]),
            node("text", Some("Source"), vec![]),
            node(
                "text",
                Some("\u{200e}> 35 |  onPress={() => { throw new Error('boom'); }}"),
                vec![],
            ),
            node(
                "text",
                Some("\u{200e}     |                        ^"),
                vec![],
            ),
            node("other", Some("App.tsx (35:43)"), vec![]),
            node("text", Some("Call Stack"), vec![]),
            node("other", Some("Pressable.props.onPress"), vec![]),
            node("other", Some("App.tsx:35:43"), vec![]),
            node("other", Some("See 11 more frames"), vec![]),
        ];
        kids.extend(controls.into_iter().map(|c| node("other", Some(c), vec![])));
        node("other", None, vec![node("other", None, kids)])
    }

    fn fatal_logbox() -> Element {
        logbox(
            "Log 1 of 1",
            "Uncaught Error",
            "golem-deferred-boom",
            vec!["Dismiss", "Minimize", "Copy"],
        )
    }

    #[test]
    fn a_fatal_logbox_reports_its_message_and_source_position() {
        // The position is what makes this better than the redbox path, which
        // has no file or line to report.
        assert_eq!(
            dev_bundle_error(&fatal_logbox()).as_deref(),
            Some("golem-deferred-boom at App.tsx (35:43)")
        );
    }

    #[test]
    fn a_compile_failure_is_found_by_its_header_alone() {
        // A syntax-level LogBox renders no buttons at all — its footer is the
        // sentence below — so the header is the only thing identifying it.
        let tree = logbox(
            "Failed to compile",
            "Syntax Error",
            "App.tsx: Unexpected token (9:15)",
            vec!["This error cannot be dismissed."],
        );
        assert!(
            dev_bundle_error(&tree)
                .is_some_and(|m| m.starts_with("App.tsx: Unexpected token (9:15)")),
            "got {:?}",
            dev_bundle_error(&tree)
        );
    }

    #[test]
    fn a_render_error_is_fatal_too() {
        let tree = logbox(
            "Log 1 of 1",
            "Render Error",
            "Objects are not valid as a React child",
            vec!["Dismiss", "Minimize", "Copy"],
        );
        assert!(dev_bundle_error(&tree).is_some());
    }

    #[test]
    fn a_console_log_left_open_over_a_working_app_is_not_fatal() {
        // LogBox opens on warnings and console errors too, but the app under
        // them still works — a flow failing there is failing on its own
        // merits, and calling it a broken bundle would be a lie.
        for title in ["Console Warning", "Console Error"] {
            let tree = logbox(
                "Log 1 of 1",
                title,
                "something noisy",
                vec!["Dismiss", "Minimize", "Copy"],
            );
            assert_eq!(
                dev_bundle_error(&tree),
                None,
                "{title} SHALL NOT read as a broken app"
            );
        }
    }

    #[test]
    fn a_minimised_logbox_badge_is_not_fatal() {
        // Observed on device: the badge sits over a fully working app. It
        // carries the message but none of the inspector's chrome.
        let tree = node(
            "other",
            None,
            vec![
                node("text", Some("Counter"), vec![]),
                node("text", Some("0"), vec![]),
                node("other", Some("+"), vec![]),
                node("other", Some("!, golem-deferred-boom"), vec![]),
            ],
        );
        assert_eq!(dev_bundle_error(&tree), None);
    }

    #[test]
    fn an_apps_own_dismiss_and_copy_buttons_are_not_a_logbox() {
        // Identified by header + title rather than the control trio precisely
        // so this can't trip it.
        let tree = node(
            "other",
            None,
            vec![
                node("text", Some("Share this receipt"), vec![]),
                node("other", Some("Copy"), vec![]),
                node("other", Some("Dismiss"), vec![]),
            ],
        );
        assert_eq!(dev_bundle_error(&tree), None);
    }

    #[test]
    fn a_title_without_the_inspector_header_is_not_enough() {
        // An app that happens to render the words "Uncaught Error" is not a
        // LogBox; the header is what says an inspector is on screen.
        let tree = node(
            "other",
            None,
            vec![
                node("text", Some("Uncaught Error"), vec![]),
                node("text", Some("in our own error screen"), vec![]),
            ],
        );
        assert_eq!(dev_bundle_error(&tree), None);
    }

    #[test]
    fn the_log_counter_is_recognised_only_in_its_real_shape() {
        assert!(is_log_counter("Log 1 of 1"));
        assert!(is_log_counter("Log 3 of 7"));
        assert!(!is_log_counter("Log in"));
        assert!(!is_log_counter("Log of 7"));
        assert!(!is_log_counter("Catalog 1 of 1"));
        assert!(!is_log_counter("Log one of two"));
    }

    #[test]
    fn a_source_position_is_recognised_only_in_its_real_shape() {
        assert!(is_source_position("App.tsx (35:43)"));
        assert!(is_source_position("src/screens/Home.tsx (1:0)"));
        // A stack frame renders the position without parentheses.
        assert!(!is_source_position("App.tsx:35:43"));
        assert!(!is_source_position("Pressable.props.onPress"));
        assert!(!is_source_position("See 11 more frames"));
        assert!(!is_source_position("Vertical scroll bar, 2 pages"));
    }

    #[test]
    fn an_app_rendering_normally_is_not() {
        let tree = node(
            "FrameLayout",
            None,
            vec![
                node("TextView", Some("Counter"), vec![]),
                node("TextView", Some("0"), vec![]),
                node("Button", Some("+"), vec![]),
            ],
        );
        assert_eq!(dev_bundle_error(&tree), None);
    }

    #[test]
    fn a_non_fatal_logbox_notice_is_not() {
        // Observed on device: a stray-text warning renders a badge over an app
        // that is otherwise working. The bundle built; the run should carry on
        // failing (or passing) on its own merits.
        let tree = node(
            "FrameLayout",
            None,
            vec![
                node("TextView", Some("Counter"), vec![]),
                node("TextView", Some("0"), vec![]),
                node("TextView", Some("!"), vec![]),
                node(
                    "TextView",
                    Some("Text strings must be rendered within a <Text> component."),
                    vec![],
                ),
            ],
        );
        assert_eq!(
            dev_bundle_error(&tree),
            None,
            "a LogBox notice over a working app SHALL NOT read as a broken bundle"
        );
    }

    #[test]
    fn one_button_alone_is_not_enough() {
        // An app may legitimately have a reload control of its own; only the
        // pair, with RN's keyboard hints, identifies the overlay.
        let only_reload = node(
            "FrameLayout",
            None,
            vec![
                node("TextView", Some("Something went wrong"), vec![]),
                node("Button", Some("RELOAD\n(R,\u{a0}R)"), vec![]),
            ],
        );
        assert_eq!(dev_bundle_error(&only_reload), None);

        let plain_words = node(
            "FrameLayout",
            None,
            vec![
                node("Button", Some("Dismiss"), vec![]),
                node("Button", Some("Reload"), vec![]),
            ],
        );
        assert_eq!(
            dev_bundle_error(&plain_words),
            None,
            "an app's own Dismiss/Reload buttons SHALL NOT read as a redbox"
        );
    }

    #[test]
    fn app_content_behind_the_overlay_does_not_supply_the_message() {
        // The overlay is mounted over the app, so the app's own text is still
        // in the tree and comes first in document order. Reporting "Counter"
        // as the bundle error would be worse than reporting nothing.
        let tree = node(
            "Root",
            None,
            vec![
                node("TextView", Some("Counter"), vec![]),
                node("TextView", Some("0"), vec![]),
                redbox("SyntaxError: /p/App.tsx: Unexpected token (9:15)"),
            ],
        );
        assert_eq!(
            dev_bundle_error(&tree).as_deref(),
            Some("SyntaxError: /p/App.tsx: Unexpected token (9:15)")
        );
    }

    #[test]
    fn the_overlay_is_found_however_deeply_it_is_nested() {
        // The real tree wraps it in four containers before the overlay itself.
        let deep = node(
            "FrameLayout",
            None,
            vec![node(
                "LinearLayout",
                None,
                vec![node(
                    "FrameLayout",
                    None,
                    vec![node("LinearLayout", None, vec![redbox("SyntaxError: x")])],
                )],
            )],
        );
        assert_eq!(dev_bundle_error(&deep).as_deref(), Some("SyntaxError: x"));
    }
}
