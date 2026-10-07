//! A UI tree as TOON-style text: one line per selectable element, in few
//! tokens, for an LLM that writes selectors. `golem tree`, `golem probe`
//! and the MCP `tree` tool print it, and the `[n]` indexes are the same in
//! all three.
//!
//! ```text
//! tree visible 402x874 kb:336 n:3
//! [1] button "Log in" 40,900 360x80 @220,940  ·button·has_text·short_text·wide·
//! [2] other label=Settings 0,100 402x300 @201,250  ·no_text·
//!   [3] switch "Wi-Fi" 16,120 370x44 @201,142  ·has_text·short_text·wide· [checked]
//! ```
//!
//! An element is indented under the selectable element before it whose
//! bounds enclose it: that is the relation `contains` and `inside` select
//! on, and the visible tree carries no other hierarchy.

use crate::selector::element_has_trait;
use crate::{Bounds, Element};

/// Traits shown on a line, in this order: content type, text, shape, size.
/// `text` is an alias of `has_text`, so it is left out.
pub const RENDERED_TRAITS: &[&str] = &[
    "button",
    "has_text",
    "no_text",
    "short_text",
    "long_text",
    "square",
    "wide",
    "tall",
    "small",
    "large",
];

/// What the header says about the tree.
#[derive(Debug, Clone, Copy, Default)]
pub struct TreeHeader {
    /// The full tree, not the visible one. Its header says it is a hint.
    pub full: bool,
    /// The soft keyboard's height, when it is up.
    pub keyboard_height: i32,
}

/// Whether `e` gets a line: it has text, a label or an id, takes a tap, or
/// looks like a button. Pure layout containers do not.
pub fn is_selectable(e: &Element) -> bool {
    let has_text = e.text.as_deref().is_some_and(|s| !s.is_empty());
    let has_label = [&e.accessibility_label, &e.accessibility_id]
        .into_iter()
        .flatten()
        .any(|s| !s.is_empty());
    has_text || has_label || e.clickable || element_has_trait(e, "button")
}

/// The selectable elements of `root`, in pre-order. Element `[n]` in the
/// text is `selectable(root)[n - 1]`.
pub fn selectable(root: &Element) -> Vec<&Element> {
    fn walk<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
        if is_selectable(e) {
            out.push(e);
        }
        for child in &e.children {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

/// `root` as TOON tree text: a header line, then one line per selectable
/// element.
pub fn encode_tree(root: &Element, header: &TreeHeader) -> String {
    let nodes = selectable(root);
    let b = root.bounds;
    let mut out = format!(
        "tree {} {}x{}",
        if header.full { "full" } else { "visible" },
        b.width,
        b.height
    );
    if header.keyboard_height > 0 {
        out.push_str(&format!(" kb:{}", header.keyboard_height));
    }
    out.push_str(&format!(" n:{}", nodes.len()));
    if header.full {
        out.push_str(" · hint only: not what the user sees");
    }
    out.push('\n');
    if nodes.is_empty() {
        out.push_str("(no selectable elements)\n");
        return out;
    }
    let mut enclosing: Vec<Bounds> = Vec::new();
    for (i, e) in nodes.iter().enumerate() {
        let bounds = *e.effective_bounds();
        while enclosing
            .last()
            .is_some_and(|outer| !encloses(outer, &bounds))
        {
            enclosing.pop();
        }
        out.push_str(&"  ".repeat(enclosing.len()));
        out.push_str(&format_line(i + 1, e));
        out.push('\n');
        enclosing.push(bounds);
    }
    out
}

/// One element's line, without indent or newline.
pub fn format_line(index: usize, e: &Element) -> String {
    let b = e.effective_bounds();
    let mut line = format!("[{index}] {}", short_type(&e.element_type));
    let text = e.text.as_deref().filter(|t| !t.is_empty());
    if let Some(t) = text {
        line.push_str(&format!(" \"{}\"", one_line(t)));
    }
    for (key, value) in [
        ("label", e.accessibility_label.as_deref()),
        ("id", e.accessibility_id.as_deref()),
    ] {
        if let Some(v) = value.filter(|v| !v.is_empty() && Some(*v) != text) {
            line.push_str(&format!(" {key}={}", one_line(v)));
        }
    }
    line.push_str(&format!(
        " {},{} {}x{} @{},{}",
        b.x,
        b.y,
        b.width,
        b.height,
        b.x + b.width / 2,
        b.y + b.height / 2
    ));
    let traits: Vec<&str> = RENDERED_TRAITS
        .iter()
        .copied()
        .filter(|t| element_has_trait(e, t))
        .collect();
    if !traits.is_empty() {
        line.push_str(&format!("  ·{}·", traits.join("·")));
    }
    let state: Vec<&str> = [
        (!e.enabled, "disabled"),
        (e.checked, "checked"),
        (e.focused, "focused"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    if !state.is_empty() {
        line.push_str(&format!(" [{}]", state.join(", ")));
    }
    line
}

/// `android.widget.Button` → `Button`; an iOS or DOM type stays as it is.
fn short_type(element_type: &str) -> &str {
    element_type.rsplit('.').next().unwrap_or(element_type)
}

/// Text on one line: a merged Flutter label carries newlines.
fn one_line(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// `outer` holds `inner` and is larger. Equal bounds do not nest: a stack
/// of full-screen wrappers would otherwise indent every line below it.
fn encloses(outer: &Bounds, inner: &Bounds) -> bool {
    outer != inner
        && inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{filter_viewport, Viewport};

    fn el(element_type: &str, bounds: (i32, i32, i32, i32)) -> Element {
        Element {
            element_type: element_type.into(),
            text: None,
            accessibility_label: None,
            accessibility_id: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: false,
            focused: false,
            bounds: Bounds::new(bounds.0, bounds.1, bounds.2, bounds.3),
            visible_bounds: None,
            hit_points: vec![],
            drawing_order: None,
            children: Vec::new(),
        }
    }

    fn text(mut e: Element, t: &str) -> Element {
        e.text = Some(t.into());
        e
    }

    fn visible(root: &Element, keyboard: i32) -> Element {
        let mut vp = Viewport::from_root(root);
        vp.height -= keyboard;
        filter_viewport(root, &vp)
    }

    /// A native Android settings screen: a toolbar title, a group with a
    /// label holding two rows, a disabled button, and an off-screen row.
    fn native_screen() -> Element {
        let mut root = el("android.widget.FrameLayout", (0, 0, 1080, 2400));
        root.children.push(text(
            el("android.widget.TextView", (48, 120, 600, 80)),
            "Settings",
        ));
        let mut group = el("android.widget.LinearLayout", (0, 300, 1080, 600));
        group.accessibility_label = Some("Network".into());
        let mut wifi = text(el("android.widget.Switch", (48, 340, 984, 120)), "Wi-Fi");
        wifi.checked = true;
        wifi.clickable = true;
        group.children.push(wifi);
        let mut bt = text(
            el("android.widget.Switch", (48, 500, 984, 120)),
            "Bluetooth",
        );
        bt.clickable = true;
        bt.accessibility_id = Some("bt_toggle".into());
        group.children.push(bt);
        root.children.push(group);
        let mut save = text(el("android.widget.Button", (48, 1000, 984, 140)), "Save");
        save.enabled = false;
        save.clickable = true;
        root.children.push(save);
        root.children
            .push(el("android.view.View", (0, 1200, 1080, 400)));
        root.children.push(text(
            el("android.widget.TextView", (48, 2600, 600, 80)),
            "About",
        ));
        root
    }

    #[test]
    fn a_native_screen() {
        assert_eq!(
            encode_tree(&visible(&native_screen(), 0), &TreeHeader::default()),
            "tree visible 1080x2400 n:5\n\
             [1] TextView \"Settings\" 48,120 600x80 @348,160  ·has_text·short_text·wide·\n\
             [2] LinearLayout label=Network 0,300 1080x600 @540,600  ·no_text·\n\
             \x20 [3] Switch \"Wi-Fi\" 48,340 984x120 @540,400  ·has_text·short_text·wide· [checked]\n\
             \x20 [4] Switch \"Bluetooth\" id=bt_toggle 48,500 984x120 @540,560  ·has_text·short_text·wide·\n\
             [5] Button \"Save\" 48,1000 984x140 @540,1070  ·has_text·short_text·wide· [disabled]\n"
        );
    }

    #[test]
    fn a_webview_screen() {
        let mut root = el("android.widget.FrameLayout", (0, 0, 1080, 2400));
        let mut web = el("android.webkit.WebView", (0, 200, 1080, 2200));
        let mut form = el("form", (40, 300, 1000, 600));
        let mut email = el("input", (80, 340, 920, 120));
        email.accessibility_label = Some("Email".into());
        email.clickable = true;
        form.children.push(email);
        let mut go = text(el("button", (80, 700, 920, 140)), "Sign \"in\"");
        go.clickable = true;
        form.children.push(go);
        web.children.push(form);
        root.children.push(web);
        assert_eq!(
            encode_tree(&visible(&root, 0), &TreeHeader::default()),
            "tree visible 1080x2400 n:2\n\
             [1] input label=Email 80,340 920x120 @540,400  ·no_text·wide·\n\
             [2] button \"Sign \\\"in\\\"\" 80,700 920x140 @540,770  ·button·has_text·short_text·wide·\n"
        );
    }

    #[test]
    fn a_screen_with_the_keyboard_up() {
        let root = native_screen();
        let header = TreeHeader {
            full: false,
            keyboard_height: 1400,
        };
        assert_eq!(
            encode_tree(&visible(&root, 1400), &header),
            "tree visible 1080x2400 kb:1400 n:4\n\
             [1] TextView \"Settings\" 48,120 600x80 @348,160  ·has_text·short_text·wide·\n\
             [2] LinearLayout label=Network 0,300 1080x600 @540,600  ·no_text·\n\
             \x20 [3] Switch \"Wi-Fi\" 48,340 984x120 @540,400  ·has_text·short_text·wide· [checked]\n\
             \x20 [4] Switch \"Bluetooth\" id=bt_toggle 48,500 984x120 @540,560  ·has_text·short_text·wide·\n",
            "the Save button sits under the keyboard"
        );
    }

    #[test]
    fn the_full_tree_says_it_is_a_hint() {
        let text = encode_tree(
            &native_screen(),
            &TreeHeader {
                full: true,
                keyboard_height: 0,
            },
        );
        let first = text.lines().next().unwrap_or_default();
        assert_eq!(
            first,
            "tree full 1080x2400 n:6 · hint only: not what the user sees"
        );
        assert!(text.contains("[6] TextView \"About\" 48,2600"), "{text}");
    }

    #[test]
    fn an_empty_screen_says_so() {
        let root = el("android.widget.FrameLayout", (0, 0, 1080, 2400));
        assert_eq!(
            encode_tree(&visible(&root, 0), &TreeHeader::default()),
            "tree visible 1080x2400 n:0\n(no selectable elements)\n"
        );
    }

    #[test]
    fn indexes_follow_selectable_order() {
        let root = visible(&native_screen(), 0);
        let nodes = selectable(&root);
        assert_eq!(nodes[3].accessibility_id.as_deref(), Some("bt_toggle"));
        assert!(encode_tree(&root, &TreeHeader::default()).contains("[4] Switch \"Bluetooth\""));
    }

    #[test]
    fn wrappers_with_equal_bounds_do_not_nest() {
        let mut root = el("window", (0, 0, 400, 800));
        for t in ["other", "web_view"] {
            let mut w = el(t, (0, 0, 400, 800));
            w.clickable = true;
            root.children.push(w);
        }
        root.children
            .push(text(el("button", (10, 10, 100, 40)), "Go"));
        let lines: Vec<String> = encode_tree(&visible(&root, 0), &TreeHeader::default())
            .lines()
            .map(str::to_string)
            .collect();
        assert!(lines[1].starts_with("[1] other"), "{lines:?}");
        assert!(lines[2].starts_with("[2] web_view"), "{lines:?}");
        assert!(lines[3].starts_with("  [3] button"), "{lines:?}");
    }

    #[test]
    fn multi_line_text_stays_on_one_line() {
        let e = text(el("flt-semantics", (0, 0, 100, 100)), "Total\n$12");
        assert_eq!(
            format_line(1, &e),
            "[1] flt-semantics \"Total\\n$12\" 0,0 100x100 @50,50  ·has_text·short_text·square·"
        );
    }

    fn elem(element_type: &str) -> Element {
        el(element_type, (0, 0, 100, 40))
    }

    fn elem_text(element_type: &str, t: &str) -> Element {
        text(elem(element_type), t)
    }

    // ── is_selectable ─────────────────────────────────────────────────

    // 5. Plain layout container with no text/label/click/trait is not selectable.
    #[test]
    fn plain_container_not_selectable() {
        let e = elem("View");
        assert!(
            !is_selectable(&e),
            "layout container with no affordance SHALL NOT be selectable"
        );
    }

    // 6. Non-empty text makes an element selectable.
    #[test]
    fn element_with_text_is_selectable() {
        let e = elem_text("Label", "Hello");
        assert!(is_selectable(&e), "element with text SHALL be selectable");
    }

    // 7. Empty-string text does NOT make an element selectable.
    #[test]
    fn element_with_empty_text_not_selectable() {
        let mut e = elem("Label");
        e.text = Some(String::new());
        assert!(
            !is_selectable(&e),
            "element with empty text SHALL NOT be selectable"
        );
    }

    // 8. Non-empty accessibility_label makes an element selectable.
    #[test]
    fn element_with_label_is_selectable() {
        let mut e = elem("View");
        e.accessibility_label = Some("Submit".to_string());
        assert!(
            is_selectable(&e),
            "element with accessibility label SHALL be selectable"
        );
    }

    // 8a. Non-empty accessibility_id makes an element selectable.
    #[test]
    fn element_with_accessibility_id_is_selectable() {
        let mut e = elem("View");
        e.accessibility_id = Some("tagged-text".to_string());
        assert!(
            is_selectable(&e),
            "element with an accessibility id SHALL be selectable"
        );
    }

    // 9. Empty accessibility_label does NOT make an element selectable.
    #[test]
    fn element_with_empty_label_not_selectable() {
        let mut e = elem("View");
        e.accessibility_label = Some(String::new());
        assert!(
            !is_selectable(&e),
            "element with empty label SHALL NOT be selectable"
        );
    }

    // 10. clickable flag alone makes an element selectable.
    #[test]
    fn clickable_element_is_selectable() {
        let mut e = elem("View");
        e.clickable = true;
        assert!(is_selectable(&e), "clickable element SHALL be selectable");
    }

    // 11. A button-type element (via the "button" trait) is selectable even
    //     with no text/label/click.
    #[test]
    fn button_trait_is_selectable() {
        let e = elem("Button");
        assert!(
            is_selectable(&e),
            "button-trait element SHALL be selectable"
        );
    }

    // ── selectable ────────────────────────────────────────────

    // 14. Collects selectable nodes in pre-order (parent before children),
    //     skipping non-selectable containers but still descending into them.
    #[test]
    fn selectable_preorder_skips_containers() {
        let mut root = elem("View"); // not selectable
        let mut wrapper = elem("Group"); // not selectable
        wrapper.children.push(elem_text("Label", "Deep"));
        root.children.push(elem_text("Button", "Top"));
        root.children.push(wrapper);

        let out = selectable(&root);

        assert_eq!(
            out.len(),
            2,
            "two selectable descendants SHALL be collected"
        );
        assert_eq!(
            out[0].text.as_deref(),
            Some("Top"),
            "pre-order SHALL visit the earlier sibling first"
        );
        assert_eq!(
            out[1].text.as_deref(),
            Some("Deep"),
            "recursion SHALL descend into non-selectable containers"
        );
    }

    // 15. A selectable root includes itself before its children.
    #[test]
    fn selectable_includes_selectable_root() {
        let mut root = elem_text("Button", "Root");
        root.children.push(elem_text("Label", "Child"));

        let out = selectable(&root);

        assert_eq!(
            out.len(),
            2,
            "selectable root and child SHALL both be collected"
        );
        assert_eq!(
            out[0].text.as_deref(),
            Some("Root"),
            "selectable root SHALL be collected before its children"
        );
    }

    // 16. A tree of only non-selectable containers yields an empty list.
    #[test]
    fn selectable_empty_for_pure_containers() {
        let mut root = elem("View");
        root.children.push(elem("Group"));
        root.children.push(elem("Stack"));

        let out = selectable(&root);

        assert!(out.is_empty(), "pure-container tree SHALL collect nothing");
    }
}
