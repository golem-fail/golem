//! Recognise a React Native dev-server error overlay ("redbox") in a captured
//! tree, so a step that failed against it is reported as a broken bundle
//! rather than a missing selector.
//!
//! Matched on the overlay's own dismiss/reload controls, not on its title: the
//! title is the error text and differs per failure ("Unable to load script…"
//! when the bundler is unreachable, "SyntaxError: …" for a transform error,
//! "[runtime not ready]: …" for a throw at module scope), while the two
//! buttons and their keyboard hints are the same in all three. Verified
//! against all three on an Android emulator (Expo 57 / RN 0.86).

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

/// The overlay's message, if this tree is showing one.
///
/// `None` when the app is rendering normally — including when it shows a
/// non-fatal LogBox notice, which has neither button and leaves the app's own
/// UI on screen.
pub fn dev_bundle_error(root: &Element) -> Option<String> {
    // Narrow to the overlay before reading any text. Searching the whole tree
    // would let the app's own content behind the overlay supply the "title".
    let overlay = overlay_root(root)?;
    let title = first_text(overlay)?;
    Some(first_line(title))
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
