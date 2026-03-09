use crate::glob::GlobMatcher;
use crate::{Element, FindResult};

/// Selector criteria for finding elements.
///
/// All non-None fields are combined with AND logic:
/// an element must satisfy every specified criterion to match.
#[derive(Debug, Clone, Default)]
pub struct Selector {
    pub text: Option<String>,
    pub id: Option<String>,
    pub element_type: Option<String>,
    pub index: Option<usize>,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
    pub clickable: Option<bool>,
    pub placeholder: Option<String>,
    // Relational fields will be added in Wave 8
}

/// Find all elements matching the selector in the hierarchy tree.
///
/// Traverses the entire tree recursively (depth-first), collecting all matches.
/// Then applies the index filter if present.
pub fn find_elements(root: &Element, selector: &Selector) -> Vec<FindResult> {
    let mut results = Vec::new();
    collect_matches(root, selector, &mut results);

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

    if let Some(ref pattern) = selector.id {
        match &element.id {
            Some(id) => {
                if !GlobMatcher::new(pattern).is_match(id) {
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

    fn bounds(x: f64, y: f64, w: f64, h: f64) -> Bounds {
        Bounds::new(x, y, w, h)
    }

    fn elem(element_type: &str) -> Element {
        Element {
            element_type: element_type.to_string(),
            text: None,
            id: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds: bounds(0.0, 0.0, 100.0, 40.0),
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
        btn.id = Some("btn-submit".to_string());
        root.children.push(btn);

        let s = Selector {
            id: Some("btn-submit".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.id.as_deref(), Some("btn-submit"));
    }

    // ── 6. ID glob ──────────────────────────────────────────────────

    #[test]
    fn id_glob() {
        let mut root = elem("View");
        let mut btn1 = elem("Button");
        btn1.id = Some("btn-submit".to_string());
        let mut btn2 = elem("Button");
        btn2.id = Some("btn-cancel".to_string());
        let mut lbl = elem("Label");
        lbl.id = Some("lbl-title".to_string());
        root.children.push(btn1);
        root.children.push(btn2);
        root.children.push(lbl);

        let s = Selector {
            id: Some("btn-*".to_string()),
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
}
