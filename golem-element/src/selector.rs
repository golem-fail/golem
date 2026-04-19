use crate::glob::GlobMatcher;
use crate::{Element, FindResult};

/// An anchor for relational selectors — either a simple text pattern or a full selector.
#[derive(Debug, Clone)]
pub enum AnchorSelector {
    /// Match anchor by text pattern (glob).
    Text(String),
    /// Match anchor using a full selector.
    Full(Box<Selector>),
}

/// Selector criteria for finding elements.
///
/// All non-None fields are combined with AND logic:
/// an element must satisfy every specified criterion to match.
#[derive(Debug, Clone, Default)]
pub struct Selector {
    pub text: Option<String>,
    pub accessibility_label: Option<String>,
    pub index: Option<usize>,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
    pub clickable: Option<bool>,
    /// Keep only elements whose bounds.y > anchor.bottom()
    pub below: Option<AnchorSelector>,
    /// Keep only elements whose bounds.bottom() < anchor.y
    pub above: Option<AnchorSelector>,
    /// Keep only elements whose bounds.x > anchor.right()
    pub right_of: Option<AnchorSelector>,
    /// Keep only elements whose bounds.right() < anchor.x
    pub left_of: Option<AnchorSelector>,
    /// Observable traits that the element must have. All must match (AND logic).
    /// E.g. ["button", "has_text", "square"]
    pub traits: Vec<String>,
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

/// Find the anchor element for a relational selector.
///
/// - `AnchorSelector::Text` — find first element matching the glob pattern.
/// - `AnchorSelector::Full` — find first element matching the full selector.
fn resolve_anchor<'a>(root: &'a Element, anchor: &AnchorSelector) -> Option<FindResult> {
    match anchor {
        AnchorSelector::Text(pattern) => {
            let matcher = GlobMatcher::new(pattern);
            find_by_text_recursive(root, &matcher).map(|el| FindResult::new(el.clone()))
        }
        AnchorSelector::Full(selector) => {
            let results = find_elements(root, selector);
            results.into_iter().next()
        }
    }
}

fn find_by_text_recursive<'a>(element: &'a Element, matcher: &GlobMatcher) -> Option<&'a Element> {
    if let Some(ref text) = element.text {
        if matcher.is_match(text) {
            return Some(element);
        }
    }
    for child in &element.children {
        if let Some(found) = find_by_text_recursive(child, matcher) {
            return Some(found);
        }
    }
    None
}

/// Apply all relational filters to the results.
fn apply_relational_filters(
    root: &Element,
    selector: &Selector,
    mut results: Vec<FindResult>,
) -> Vec<FindResult> {
    if let Some(ref anchor) = selector.below {
        if let Some(found) = resolve_anchor(root, anchor) {
            let anchor_bottom = found.element.effective_bounds().bottom();
            results.retain(|r| r.element.effective_bounds().y > anchor_bottom);
        } else {
            return Vec::new();
        }
    }

    if let Some(ref anchor) = selector.above {
        if let Some(found) = resolve_anchor(root, anchor) {
            let anchor_y = found.element.effective_bounds().y;
            results.retain(|r| r.element.effective_bounds().bottom() < anchor_y);
        } else {
            return Vec::new();
        }
    }

    if let Some(ref anchor) = selector.right_of {
        if let Some(found) = resolve_anchor(root, anchor) {
            let anchor_right = found.element.effective_bounds().right();
            results.retain(|r| r.element.effective_bounds().x > anchor_right);
        } else {
            return Vec::new();
        }
    }

    if let Some(ref anchor) = selector.left_of {
        if let Some(found) = resolve_anchor(root, anchor) {
            let anchor_x = found.element.effective_bounds().x;
            results.retain(|r| r.element.effective_bounds().right() < anchor_x);
        } else {
            return Vec::new();
        }
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

    if let Some(ref pattern) = selector.accessibility_label {
        match &element.accessibility_label {
            Some(aid) => {
                if !GlobMatcher::new(pattern).is_match(aid) {
                    return false;
                }
            }
            None => return false,
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

    // Check observable traits
    for trait_name in &selector.traits {
        if !element_has_trait(element, trait_name) {
            return false;
        }
    }

    true
}

/// Check whether an element has a given observable trait.
///
/// Traits are computed from existing element data — no companion changes needed.
/// Element types are compared case-insensitively to handle iOS (lowercase)
/// vs Android (PascalCase) differences.
fn element_has_trait(element: &Element, trait_name: &str) -> bool {
    let et = element.element_type.to_lowercase();
    let text_len = element.text.as_ref().map_or(0, |t| t.len());
    let w = element.bounds.width;
    let h = element.bounds.height;

    match trait_name {
        // Content type traits
        "button" => et == "button" || et == "link",
        "input" => matches!(
            et.as_str(),
            "text_field" | "secure_text_field" | "search_field" | "text_view"
                | "edittext" | "autocompletetextview"
        ),
        "toggle" => matches!(
            et.as_str(),
            "switch" | "toggle" | "checkbox" | "radio_button"
                | "togglebutton" | "radiobutton" | "compoundbutton"
        ),

        // Text traits
        "has_text" | "text" => text_len > 0,
        "no_text" => text_len == 0,
        "short_text" => text_len > 0 && text_len <= 10,
        "long_text" => text_len > 50,

        // Shape/size traits
        "square" => {
            if w == 0 || h == 0 { return false; }
            let ratio = w as f64 / h as f64;
            ratio > 0.8 && ratio < 1.2
        }
        "wide" => w > 0 && h > 0 && w > 2 * h,
        "tall" => w > 0 && h > 0 && h > 2 * w,
        "small" => {
            let area = w as u64 * h as u64;
            area > 0 && area < 2500
        }
        "large" => {
            let area = w as u64 * h as u64;
            area > 100_000
        }

        _ => false, // Unknown trait — doesn't match
    }
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
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds: bounds(0, 0, 100, 40),
            visible_bounds: None,
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
        btn.accessibility_label = Some("btn-submit".to_string());
        root.children.push(btn);

        let s = Selector {
            accessibility_label: Some("btn-submit".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.accessibility_label.as_deref(), Some("btn-submit"));
    }

    // ── 6. ID glob ──────────────────────────────────────────────────

    #[test]
    fn id_glob() {
        let mut root = elem("View");
        let mut btn1 = elem("Button");
        btn1.accessibility_label = Some("btn-submit".to_string());
        let mut btn2 = elem("Button");
        btn2.accessibility_label = Some("btn-cancel".to_string());
        let mut lbl = elem("Label");
        lbl.accessibility_label = Some("lbl-title".to_string());
        root.children.push(btn1);
        root.children.push(btn2);
        root.children.push(lbl);

        let s = Selector {
            accessibility_label: Some("btn-*".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 2);
    }

    // ── 8. Index selection ──────────────────────────────────────────

    #[test]
    fn index_selection() {
        let mut root = elem("View");
        root.children.push(elem_with_text("Button", "A"));
        root.children.push(elem_with_text("Button", "B"));
        root.children.push(elem_with_text("Button", "C"));

        let s = Selector {
            text: Some("*".to_string()),
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
            text: Some("*".to_string()),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Enabled"));
    }

    // ── 11. State filter clickable ──────────────────────────────────

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
            below: Some(AnchorSelector::Text("Header".to_string())),
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
            above: Some(AnchorSelector::Text("Footer".to_string())),
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
            right_of: Some(AnchorSelector::Text("Label".to_string())),
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
            left_of: Some(AnchorSelector::Text("Button".to_string())),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].element.text.as_deref(), Some("Icon"));
    }

    // ── 30. Relational anchor not found — empty results ─────────────

    #[test]
    fn relational_anchor_not_found() {
        let mut root = elem("View");
        root.children.push(elem_at("Button", "Submit", 0, 100, 100, 40));

        let s = Selector {
            below: Some(AnchorSelector::Text("Nonexistent".to_string())),
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
            text: Some("*Btn".to_string()),
            below: Some(AnchorSelector::Text("Header".to_string())),
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
            text: Some("Btn *".to_string()),
            below: Some(AnchorSelector::Text("Header".to_string())),
            ..sel()
        };
        let results = find_elements(&root, &s);
        assert_eq!(results.len(), 3);
    }

}
