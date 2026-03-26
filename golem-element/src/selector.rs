use crate::glob::GlobMatcher;
use crate::{Element, FindResult};

/// Selector criteria for finding elements.
///
/// All non-None fields are combined with AND logic:
/// an element must satisfy every specified criterion to match.
#[derive(Debug, Clone, Default)]
pub struct Selector {
    pub text: Option<String>,
    pub accessibility_id: Option<String>,
    pub element_type: Option<String>,
    pub index: Option<usize>,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
    pub clickable: Option<bool>,
    pub placeholder: Option<String>,
    /// Keep only elements whose bounds.y > anchor.bottom()
    pub below: Option<String>,
    /// Keep only elements whose bounds.bottom() < anchor.y
    pub above: Option<String>,
    /// Keep only elements whose bounds.x > anchor.right()
    pub right_of: Option<String>,
    /// Keep only elements whose bounds.right() < anchor.x
    pub left_of: Option<String>,
    /// Keep only elements that are descendants of an element matching this text
    pub child_of: Option<String>,
}

/// Find all elements matching the selector in the hierarchy tree.
///
/// Traverses the entire tree recursively (depth-first), collecting all matches.
/// Then applies relational filters (below, above, right_of, left_of, child_of)
/// and the index filter if present.
pub fn find_elements(root: &Element, selector: &Selector) -> Vec<FindResult> {
    let mut results = Vec::new();
    collect_matches(root, selector, &mut results);

    // Apply relational filters
    results = apply_relational_filters(root, selector, results);

    if let Some(idx) = selector.index {
        if idx < results.len() {
            vec![results.swap_remove(idx)]
        } else {
            Vec::new()
        }
    } else {
        results
    }
}

/// Find the first element in the tree whose text matches the given glob pattern.
fn find_anchor<'a>(root: &'a Element, pattern: &str) -> Option<&'a Element> {
    let matcher = GlobMatcher::new(pattern);
    find_anchor_recursive(root, &matcher)
}

fn find_anchor_recursive<'a>(element: &'a Element, matcher: &GlobMatcher) -> Option<&'a Element> {
    if let Some(ref text) = element.text {
        if matcher.is_match(text) {
            return Some(element);
        }
    }
    for child in &element.children {
        if let Some(found) = find_anchor_recursive(child, matcher) {
            return Some(found);
        }
    }
    None
}

/// Check whether an element (identified by its bounds) is a descendant of
/// any element matching the `child_of` pattern in the tree.
/// We collect all elements under the anchor and check if the candidate is among them.
fn is_element_descendant_of_anchor(root: &Element, anchor_pattern: &str, candidate: &Element) -> bool {
    let matcher = GlobMatcher::new(anchor_pattern);
    // Find the anchor element
    if let Some(anchor) = find_anchor_recursive(root, &matcher) {
        // Check if candidate is a descendant of anchor
        element_exists_in_subtree(anchor, candidate)
    } else {
        false
    }
}

/// Check if an element with matching bounds/text/type/id exists in the subtree
/// (excluding the root itself, only checking descendants).
fn element_exists_in_subtree(subtree_root: &Element, candidate: &Element) -> bool {
    for child in &subtree_root.children {
        if elements_match(child, candidate) {
            return true;
        }
        if element_exists_in_subtree(child, candidate) {
            return true;
        }
    }
    false
}

/// Check if two elements are the same by comparing all their identifying fields.
fn elements_match(a: &Element, b: &Element) -> bool {
    a.element_type == b.element_type
        && a.text == b.text
        && a.accessibility_id == b.accessibility_id
        && a.bounds == b.bounds
        && a.enabled == b.enabled
        && a.checked == b.checked
        && a.clickable == b.clickable
        && a.focused == b.focused
        && a.placeholder == b.placeholder
}

/// Apply all relational filters to the results.
fn apply_relational_filters(
    root: &Element,
    selector: &Selector,
    mut results: Vec<FindResult>,
) -> Vec<FindResult> {
    if let Some(ref pattern) = selector.below {
        if let Some(anchor) = find_anchor(root, pattern) {
            let anchor_bottom = anchor.bounds.bottom();
            results.retain(|r| r.element.bounds.y > anchor_bottom);
        } else {
            return Vec::new();
        }
    }

    if let Some(ref pattern) = selector.above {
        if let Some(anchor) = find_anchor(root, pattern) {
            let anchor_y = anchor.bounds.y;
            results.retain(|r| r.element.bounds.bottom() < anchor_y);
        } else {
            return Vec::new();
        }
    }

    if let Some(ref pattern) = selector.right_of {
        if let Some(anchor) = find_anchor(root, pattern) {
            let anchor_right = anchor.bounds.right();
            results.retain(|r| r.element.bounds.x > anchor_right);
        } else {
            return Vec::new();
        }
    }

    if let Some(ref pattern) = selector.left_of {
        if let Some(anchor) = find_anchor(root, pattern) {
            let anchor_x = anchor.bounds.x;
            results.retain(|r| r.element.bounds.right() < anchor_x);
        } else {
            return Vec::new();
        }
    }

    if let Some(ref pattern) = selector.child_of {
        results.retain(|r| is_element_descendant_of_anchor(root, pattern, &r.element));
    }

    results
}

/// Recursively traverse the element tree depth-first, collecting matching elements.
fn collect_matches(element: &Element, selector: &Selector, results: &mut Vec<FindResult>) {
    if matches_selector(element, selector) {
        results.push(FindResult::new(element.clone()));
    }
    for child in &element.children {
        collect_matches(child, selector, results);
    }
}

/// Check whether a single element matches all non-None selector criteria.
fn matches_selector(element: &Element, selector: &Selector) -> bool {
    if let Some(ref pattern) = selector.text {
        match &element.text {
            Some(text) => {
                if !GlobMatcher::new(pattern).is_match(text) {
                    return false;
                }
            }
            None => return false,
        }
    }

    if let Some(ref pattern) = selector.accessibility_id {
        match &element.accessibility_id {
            Some(aid) => {
                if !GlobMatcher::new(pattern).is_match(aid) {
                    return false;
                }
            }
            None => return false,
        }
    }

    if let Some(ref expected_type) = selector.element_type {
        if element.element_type != *expected_type {
            return false;
        }
    }

    if let Some(expected) = selector.enabled {
        if element.enabled != expected {
            return false;
        }
    }

    if let Some(expected) = selector.checked {
        if element.checked != expected {
            return false;
        }
    }

    if let Some(expected) = selector.clickable {
        if element.clickable != expected {
            return false;
        }
    }

    if let Some(ref pattern) = selector.placeholder {
        match &element.placeholder {
            Some(placeholder) => {
                if !GlobMatcher::new(pattern).is_match(placeholder) {
                    return false;
                }
            }
            None => return false,
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bounds;

    // ── Test helpers ──────────────────────────────────────────────────

    fn bounds(x: i32, y: i32, w: i32, h: i32) -> Bounds {
        Bounds::new(x, y, w, h)
    }

    fn elem(element_type: &str) -> Element {
        Element {
            element_type: element_type.to_string(),
            text: None,
            accessibility_id: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds: bounds(0, 0, 100, 40),
            children: Vec::new(),
        }
    }

    fn elem_with_text(element_type: &str, text: &str) -> Element {
        let mut e = elem(element_type);
        e.text = Some(text.to_string());
        e
    }

    fn sel() -> Selector {
        Selector::default()
    }

    // ── 1. Exact text match ──────────────────────────────────────────

    #[test]
    fn exact_text_match() {
        let mut root = elem("View");
        root.children.push(elem_with_text("Button", "Submit"));
        root.children.push(elem_with_text("Button", "Cancel"));

        let s = Selector {
            text: Some("Submit".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Submit"));
    }

    // ── 2. Glob pattern wildcard ─────────────────────────────────────

    #[test]
    fn glob_pattern_wildcard() {
        let mut root = elem("View");
        root.children.push(elem_with_text("Label", "Item 1"));
        root.children.push(elem_with_text("Label", "Item 2"));
        root.children.push(elem_with_text("Label", "Other"));

        let s = Selector {
            text: Some("Item *".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 2);
    }

    // ── 3. Glob single char ─────────────────────────────────────────

    #[test]
    fn glob_single_char() {
        let mut root = elem("View");
        root.children.push(elem_with_text("Tab", "Tab 1"));
        root.children.push(elem_with_text("Tab", "Tab 10"));

        let s = Selector {
            text: Some("Tab ?".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Tab 1"));
    }

    // ── 4. Glob literal asterisk ─────────────────────────────────────

    #[test]
    fn glob_literal_asterisk() {
        let mut root = elem("View");
        root.children.push(elem_with_text("Label", "5 * 3"));
        root.children.push(elem_with_text("Label", "5 x 3"));

        let s = Selector {
            text: Some("5 \\* 3".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("5 * 3"));
    }

    // ── 5. ID match ─────────────────────────────────────────────────

    #[test]
    fn id_match() {
        let mut root = elem("View");
        let mut btn = elem("Button");
        btn.accessibility_id = Some("btn-submit".to_string());
        root.children.push(btn);

        let s = Selector {
            accessibility_id: Some("btn-submit".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.accessibility_id.as_deref(), Some("btn-submit"));
    }

    // ── 6. ID glob ──────────────────────────────────────────────────

    #[test]
    fn id_glob() {
        let mut root = elem("View");
        let mut btn1 = elem("Button");
        btn1.accessibility_id = Some("btn-submit".to_string());
        let mut btn2 = elem("Button");
        btn2.accessibility_id = Some("btn-cancel".to_string());
        let mut lbl = elem("Label");
        lbl.accessibility_id = Some("lbl-title".to_string());
        root.children.push(btn1);
        root.children.push(btn2);
        root.children.push(lbl);

        let s = Selector {
            accessibility_id: Some("btn-*".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 2);
    }

    // ── 7. Type filter ──────────────────────────────────────────────

    #[test]
    fn type_filter() {
        let mut root = elem("View");
        root.children.push(elem("Button"));
        root.children.push(elem("Label"));
        root.children.push(elem("Button"));

        let s = Selector {
            element_type: Some("Button".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 2);
        assert!(results
            .iter()
            .all(|r| r.element.element_type == "Button"));
    }

    // ── 8. Index selection ──────────────────────────────────────────

    #[test]
    fn index_selection() {
        let mut root = elem("View");
        root.children.push(elem_with_text("Button", "A"));
        root.children.push(elem_with_text("Button", "B"));
        root.children.push(elem_with_text("Button", "C"));

        let s = Selector {
            element_type: Some("Button".to_string()),
            index: Some(1),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("B"));
    }

    // ── 9. Index out of range ───────────────────────────────────────

    #[test]
    fn index_out_of_range() {
        let mut root = elem("View");
        root.children.push(elem("Button"));

        let s = Selector {
            element_type: Some("Button".to_string()),
            index: Some(99),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert!(results.is_empty());
    }

    // ── 10. State filter enabled ────────────────────────────────────

    #[test]
    fn state_filter_enabled() {
        let mut root = elem("View");
        let mut enabled_btn = elem_with_text("Button", "Enabled");
        enabled_btn.enabled = true;
        let mut disabled_btn = elem_with_text("Button", "Disabled");
        disabled_btn.enabled = false;
        root.children.push(enabled_btn);
        root.children.push(disabled_btn);

        let s = Selector {
            enabled: Some(true),
            element_type: Some("Button".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Enabled"));
    }

    // ── 11. State filter checked ────────────────────────────────────

    #[test]
    fn state_filter_checked() {
        let mut root = elem("View");
        let mut checked = elem("Checkbox");
        checked.checked = true;
        let unchecked = elem("Checkbox");
        root.children.push(checked);
        root.children.push(unchecked);

        let s = Selector {
            checked: Some(false),
            element_type: Some("Checkbox".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert!(!results[0].element.checked);
    }

    // ── 12. State filter clickable ──────────────────────────────────

    #[test]
    fn state_filter_clickable() {
        let mut root = elem("View");
        let clickable = elem("Button"); // clickable=true by default
        let mut non_clickable = elem("Label");
        non_clickable.clickable = false;
        root.children.push(clickable);
        root.children.push(non_clickable);

        let s = Selector {
            clickable: Some(true),
            ..sel()
        };
        // root itself is clickable, plus the Button child
        let results = find_elements(&root, &s);
        assert!(results.iter().all(|r| r.element.clickable));
    }

    // ── 13. Placeholder match ───────────────────────────────────────

    #[test]
    fn placeholder_match() {
        let mut root = elem("View");
        let mut input1 = elem("TextField");
        input1.placeholder = Some("Enter name".to_string());
        let mut input2 = elem("TextField");
        input2.placeholder = Some("Enter email".to_string());
        let mut input3 = elem("TextField");
        input3.placeholder = Some("Search".to_string());
        root.children.push(input1);
        root.children.push(input2);
        root.children.push(input3);

        let s = Selector {
            placeholder: Some("Enter *".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 2);
    }

    // ── 14. AND combination text + state ─────────────────────────────

    #[test]
    fn and_combination_text_and_state() {
        let mut root = elem("View");
        let mut save_enabled = elem_with_text("Button", "Save");
        save_enabled.enabled = true;
        let mut save_disabled = elem_with_text("Button", "Save");
        save_disabled.enabled = false;
        root.children.push(save_enabled);
        root.children.push(save_disabled);

        let s = Selector {
            text: Some("Save".to_string()),
            enabled: Some(true),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert!(results[0].element.enabled);
    }

    // ── 15. AND combination type + enabled ──────────────────────────

    #[test]
    fn and_combination_type_and_enabled() {
        let mut root = elem("View");
        let enabled_btn = elem("Button"); // enabled=true by default
        let mut disabled_btn = elem("Button");
        disabled_btn.enabled = false;
        let label = elem("Label");
        root.children.push(enabled_btn);
        root.children.push(disabled_btn);
        root.children.push(label);

        let s = Selector {
            element_type: Some("Button".to_string()),
            enabled: Some(true),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.element_type, "Button");
        assert!(results[0].element.enabled);
    }

    // ── 16. Deep tree traversal ─────────────────────────────────────

    #[test]
    fn deep_tree_traversal() {
        // Build a 5-level deep tree
        let target = elem_with_text("Button", "Deep");
        let mut level4 = elem("View");
        level4.children.push(target);
        let mut level3 = elem("View");
        level3.children.push(level4);
        let mut level2 = elem("View");
        level2.children.push(level3);
        let mut root = elem("View");
        root.children.push(level2);

        let s = Selector {
            text: Some("Deep".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Deep"));
    }

    // ── 17. No matches ──────────────────────────────────────────────

    #[test]
    fn no_matches() {
        let mut root = elem("View");
        root.children.push(elem_with_text("Button", "Submit"));

        let s = Selector {
            text: Some("Nonexistent".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert!(results.is_empty());
    }

    // ── 18. Multiple matches ────────────────────────────────────────

    #[test]
    fn multiple_matches() {
        let mut root = elem("View");
        root.children.push(elem_with_text("Button", "OK"));
        root.children.push(elem_with_text("Button", "OK"));
        root.children.push(elem_with_text("Button", "OK"));

        let s = Selector {
            text: Some("OK".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 3);
    }

    // ── 19. Empty tree ──────────────────────────────────────────────

    #[test]
    fn empty_tree() {
        let root = elem("View");

        let s = Selector {
            text: Some("Anything".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert!(results.is_empty());
    }

    // ── 20. Glob wildcard only ──────────────────────────────────────

    #[test]
    fn glob_wildcard_only() {
        let mut root = elem("View");
        root.children.push(elem_with_text("Button", "A"));
        root.children.push(elem_with_text("Button", "B"));

        let s = Selector {
            text: Some("*".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        // Root has no text, so it won't match. The two children match.
        assert_eq!(results.len(), 2);
    }

    // ── 21. Empty selector matches everything ───────────────────────

    #[test]
    fn empty_selector_matches_everything() {
        let mut root = elem("View");
        root.children.push(elem("Button"));
        root.children.push(elem("Label"));

        let s = sel();
        let results = find_elements(&root, &s);
        // root + 2 children = 3
        assert_eq!(results.len(), 3);
    }

    // ── 22. Case-sensitive text ─────────────────────────────────────

    #[test]
    fn case_sensitive_text() {
        let mut root = elem("View");
        root.children.push(elem_with_text("Button", "Submit"));
        root.children.push(elem_with_text("Button", "submit"));

        let s = Selector {
            text: Some("Submit".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Submit"));
    }

    // ── 23. Unicode text matching ───────────────────────────────────

    #[test]
    fn unicode_text_matching() {
        let mut root = elem("View");
        root.children
            .push(elem_with_text("Button", "\u{9001}\u{4FE1}\u{30DC}\u{30BF}\u{30F3}"));
        root.children
            .push(elem_with_text("Button", "\u{30AD}\u{30E3}\u{30F3}\u{30BB}\u{30EB}"));

        let s = Selector {
            text: Some("\u{9001}\u{4FE1}*".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].element.text.as_deref(),
            Some("\u{9001}\u{4FE1}\u{30DC}\u{30BF}\u{30F3}")
        );
    }

    // ── 24. Multiple matches with index ─────────────────────────────

    #[test]
    fn multiple_matches_with_index() {
        let mut root = elem("View");
        root.children.push(elem_with_text("Button", "Item A"));
        root.children.push(elem_with_text("Button", "Item B"));
        root.children.push(elem_with_text("Button", "Item C"));

        let s = Selector {
            text: Some("Item *".to_string()),
            index: Some(2),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Item C"));
    }

    // ── Relational filter helpers ───────────────────────────────────

    fn elem_at(element_type: &str, text: &str, x: i32, y: i32, w: i32, h: i32) -> Element {
        let mut e = elem(element_type);
        e.text = Some(text.to_string());
        e.bounds = bounds(x, y, w, h);
        e
    }

    // ── 25. below — elements below "Header" returned ────────────────

    #[test]
    fn relational_below() {
        let mut root = elem("View");
        root.bounds = bounds(0, 0, 400, 600);
        // Header at top: y=0, height=50 => bottom=50
        root.children.push(elem_at("Label", "Header", 0, 0, 400, 50));
        // Content below header: y=60 > 50
        root.children.push(elem_at("Button", "Content", 0, 60, 400, 40));
        // Sidebar at same level as header: y=10 (not below)
        root.children.push(elem_at("Label", "Sidebar", 0, 10, 100, 40));

        let s = Selector {
            below: Some("Header".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Content"));
    }

    // ── 26. above — elements above "Footer" returned ────────────────

    #[test]
    fn relational_above() {
        let mut root = elem("View");
        root.bounds = bounds(0, 0, 400, 600);
        // Title at top: y=0, height=30 => bottom=30
        root.children.push(elem_at("Label", "Title", 0, 0, 400, 30));
        // Footer at bottom: y=500
        root.children.push(elem_at("Label", "Footer", 0, 500, 400, 50));
        // Body in middle: y=100, height=200 => bottom=300 < 500
        root.children.push(elem_at("Label", "Body", 0, 100, 400, 200));

        let s = Selector {
            above: Some("Footer".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        // Title (bottom=30 < 500) and Body (bottom=300 < 500) qualify
        assert_eq!(results.len(), 2);
        let texts: Vec<_> = results.iter().filter_map(|r| r.element.text.as_deref()).collect();
        assert!(texts.contains(&"Title"));
        assert!(texts.contains(&"Body"));
    }

    // ── 27. right_of — elements right of "Label" returned ───────────

    #[test]
    fn relational_right_of() {
        let mut root = elem("View");
        root.bounds = bounds(0, 0, 800, 100);
        // Label on the left: x=0, width=100 => right=100
        root.children.push(elem_at("Label", "Label", 0, 0, 100, 40));
        // Input to the right: x=120 > 100
        root.children.push(elem_at("TextField", "Input", 120, 0, 200, 40));
        // Another label overlapping: x=50 (not to the right)
        root.children.push(elem_at("Label", "Other", 50, 0, 80, 40));

        let s = Selector {
            right_of: Some("Label".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Input"));
    }

    // ── 28. left_of — elements left of "Button" returned ────────────

    #[test]
    fn relational_left_of() {
        let mut root = elem("View");
        root.bounds = bounds(0, 0, 800, 100);
        // Icon on the left: x=0, width=30 => right=30
        root.children.push(elem_at("Image", "Icon", 0, 0, 30, 30));
        // Button on the right: x=200
        root.children.push(elem_at("Button", "Button", 200, 0, 100, 40));
        // Another element overlapping: x=180, width=50 => right=230 (not to the left)
        root.children.push(elem_at("Label", "Near", 180, 0, 50, 40));

        let s = Selector {
            left_of: Some("Button".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Icon"));
    }

    // ── 29. child_of — only children of "List" returned ─────────────

    #[test]
    fn relational_child_of() {
        let mut root = elem("View");
        root.bounds = bounds(0, 0, 400, 600);

        let mut list = elem("View");
        list.text = Some("List".to_string());
        list.bounds = bounds(0, 100, 400, 300);
        list.children.push(elem_at("Label", "Item 1", 0, 100, 400, 40));
        list.children.push(elem_at("Label", "Item 2", 0, 150, 400, 40));
        root.children.push(list);

        // Sibling of list, not a child
        root.children.push(elem_at("Label", "Item 3", 0, 450, 400, 40));

        let s = Selector {
            element_type: Some("Label".to_string()),
            child_of: Some("List".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 2);
        let texts: Vec<_> = results.iter().filter_map(|r| r.element.text.as_deref()).collect();
        assert!(texts.contains(&"Item 1"));
        assert!(texts.contains(&"Item 2"));
    }

    // ── 30. Relational anchor not found — empty results ─────────────

    #[test]
    fn relational_anchor_not_found() {
        let mut root = elem("View");
        root.children.push(elem_at("Button", "Submit", 0, 100, 100, 40));

        let s = Selector {
            below: Some("Nonexistent".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert!(results.is_empty());
    }

    // ── 31. Combined type + below + enabled ─────────────────────────

    #[test]
    fn combined_type_below_enabled() {
        let mut root = elem("View");
        root.bounds = bounds(0, 0, 400, 600);
        // Header at top
        root.children.push(elem_at("Label", "Header", 0, 0, 400, 50));

        // Enabled button below header
        let mut btn1 = elem_at("Button", "Enabled Btn", 0, 60, 200, 40);
        btn1.enabled = true;
        root.children.push(btn1);

        // Disabled button below header
        let mut btn2 = elem_at("Button", "Disabled Btn", 0, 110, 200, 40);
        btn2.enabled = false;
        root.children.push(btn2);

        // Enabled label below header (wrong type)
        root.children.push(elem_at("Label", "Info", 0, 160, 200, 40));

        let s = Selector {
            element_type: Some("Button".to_string()),
            below: Some("Header".to_string()),
            enabled: Some(true),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Enabled Btn"));
    }

    // ── 32. below with multiple results ─────────────────────────────

    #[test]
    fn below_with_multiple_results() {
        let mut root = elem("View");
        root.bounds = bounds(0, 0, 400, 600);
        // Header: bottom = 50
        root.children.push(elem_at("Label", "Header", 0, 0, 400, 50));
        // Three buttons below header
        root.children.push(elem_at("Button", "Btn A", 0, 60, 200, 40));
        root.children.push(elem_at("Button", "Btn B", 0, 110, 200, 40));
        root.children.push(elem_at("Button", "Btn C", 0, 160, 200, 40));

        let s = Selector {
            element_type: Some("Button".to_string()),
            below: Some("Header".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 3);
    }

    // ── 33. child_of excludes siblings ──────────────────────────────

    #[test]
    fn child_of_excludes_siblings() {
        let mut root = elem("View");
        root.bounds = bounds(0, 0, 400, 600);

        // Container "Panel"
        let mut panel = elem("View");
        panel.text = Some("Panel".to_string());
        panel.bounds = bounds(0, 0, 200, 300);
        panel.children.push(elem_at("Button", "Inside", 10, 10, 80, 30));
        root.children.push(panel);

        // Sibling button (not inside Panel)
        root.children.push(elem_at("Button", "Outside", 210, 10, 80, 30));

        let s = Selector {
            element_type: Some("Button".to_string()),
            child_of: Some("Panel".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Inside"));
    }

    // ── 34. Deep descendant found by child_of ───────────────────────

    #[test]
    fn deep_descendant_found_by_child_of() {
        let mut root = elem("View");
        root.bounds = bounds(0, 0, 400, 600);

        // Container "Wrapper" > View > View > deep Button
        let deep_btn = elem_at("Button", "Deep Button", 10, 10, 80, 30);
        let mut inner2 = elem("View");
        inner2.bounds = bounds(5, 5, 190, 190);
        inner2.children.push(deep_btn);
        let mut inner1 = elem("View");
        inner1.bounds = bounds(0, 0, 195, 195);
        inner1.children.push(inner2);
        let mut wrapper = elem("View");
        wrapper.text = Some("Wrapper".to_string());
        wrapper.bounds = bounds(0, 0, 200, 200);
        wrapper.children.push(inner1);
        root.children.push(wrapper);

        // Sibling button (not inside Wrapper)
        root.children.push(elem_at("Button", "Shallow Button", 300, 10, 80, 30));

        let s = Selector {
            element_type: Some("Button".to_string()),
            child_of: Some("Wrapper".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Deep Button"));
    }
}
