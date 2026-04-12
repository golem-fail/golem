pub mod glob;
pub mod selector;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub element_type: String,
    pub text: Option<String>,
    pub accessibility_label: Option<String>,
    pub placeholder: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub checked: bool,
    #[serde(default)]
    pub clickable: bool,
    #[serde(default)]
    pub focused: bool,
    pub bounds: Bounds,
    #[serde(default)]
    pub children: Vec<Element>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Bounds {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub fn center_x(&self) -> i32 {
        self.x + self.width / 2
    }
    pub fn center_y(&self) -> i32 {
        self.y + self.height / 2
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }
    pub fn right(&self) -> i32 {
        self.x + self.width
    }
}

#[derive(Debug, Clone)]
pub struct FindResult {
    pub element: Element,
    pub tap_x: i32,
    pub tap_y: i32,
}

/// Screen viewport for filtering visible elements.
/// Origin (x, y) handles windows not at (0,0) like alert dialogs.
/// Width and height are dimensions, not absolute coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Viewport {
    pub fn new(width: i32, height: i32) -> Self {
        Self { x: 0, y: 0, width, height }
    }

    /// Detect viewport from the root element's bounds.
    pub fn from_root(root: &Element) -> Self {
        Self {
            x: root.bounds.x,
            y: root.bounds.y,
            width: root.bounds.width,
            height: root.bounds.height,
        }
    }

    /// Check if an element's bounds intersect this viewport (partially or fully visible).
    pub fn contains(&self, bounds: &Bounds) -> bool {
        let right = self.x + self.width;
        let bottom = self.y + self.height;
        bounds.x + bounds.width > self.x
            && bounds.x < right
            && bounds.y + bounds.height > self.y
            && bounds.y < bottom
    }
}

/// Collect all elements whose bounds intersect the viewport into a flat list.
///
/// Uses absolute positions only — an element at y=389 is visible regardless
/// of where its parent is in the tree. This correctly handles fixed-position
/// overlays, scrolled content, and dynamic DOM insertions.
pub fn filter_viewport(root: &Element, viewport: &Viewport) -> Element {
    let mut visible = Vec::new();
    // Collect visible descendants (skip the root — it's the container).
    for child in &root.children {
        collect_visible(child, viewport, &mut visible);
    }
    // Return the root with visible elements as flat children.
    Element {
        element_type: root.element_type.clone(),
        text: root.text.clone(),
        accessibility_label: root.accessibility_label.clone(),
        placeholder: root.placeholder.clone(),
        enabled: root.enabled,
        checked: root.checked,
        clickable: root.clickable,
        focused: root.focused,
        bounds: root.bounds.clone(),
        children: visible,
    }
}

fn collect_visible(element: &Element, viewport: &Viewport, out: &mut Vec<Element>) {
    if viewport.contains(&element.bounds) {
        let mut leaf = element.clone();
        leaf.children = Vec::new();
        out.push(leaf);
    }
    for child in &element.children {
        collect_visible(child, viewport, out);
    }
}

impl FindResult {
    pub fn new(element: Element) -> Self {
        let tap_x = element.bounds.center_x();
        let tap_y = element.bounds.center_y();
        Self {
            element,
            tap_x,
            tap_y,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bounds(x: i32, y: i32, width: i32, height: i32) -> Bounds {
        Bounds::new(x, y, width, height)
    }

    fn make_element(element_type: &str, bounds: Bounds) -> Element {
        Element {
            element_type: element_type.to_string(),
            text: None,
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds,
            children: Vec::new(),
        }
    }

    #[test]
    fn bounds_center_x_and_center_y() {
        let b = make_bounds(10, 20, 100, 50);
        assert_eq!(b.center_x(), 60);
        assert_eq!(b.center_y(), 45);
    }

    #[test]
    fn bounds_bottom_and_right() {
        let b = make_bounds(10, 20, 100, 50);
        assert_eq!(b.bottom(), 70);
        assert_eq!(b.right(), 110);
    }

    #[test]
    fn find_result_computes_tap_coordinates() {
        let elem = make_element("Button", make_bounds(0, 0, 200, 80));
        let result = FindResult::new(elem);
        assert_eq!(result.tap_x, 100);
        assert_eq!(result.tap_y, 40);
    }

    #[test]
    fn element_serialization_round_trip() {
        let elem = Element {
            element_type: "TextField".to_string(),
            text: Some("hello".to_string()),
            accessibility_label: Some("input-1".to_string()),
            placeholder: Some("Enter name".to_string()),
            enabled: true,
            checked: false,
            clickable: true,
            focused: true,
            bounds: make_bounds(5, 10, 300, 44),
            children: Vec::new(),
        };

        let json = serde_json::to_string(&elem).expect("serialization failed");
        let deserialized: Element = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.element_type, "TextField");
        assert_eq!(deserialized.text.as_deref(), Some("hello"));
        assert_eq!(deserialized.accessibility_label.as_deref(), Some("input-1"));
        assert_eq!(deserialized.placeholder.as_deref(), Some("Enter name"));
        assert!(deserialized.enabled);
        assert!(!deserialized.checked);
        assert!(deserialized.clickable);
        assert!(deserialized.focused);
        assert_eq!(deserialized.bounds, make_bounds(5, 10, 300, 44));
        assert!(deserialized.children.is_empty());
    }

    #[test]
    fn element_with_children_serializes_and_deserializes() {
        let child = make_element("Label", make_bounds(10, 10, 80, 20));
        let mut parent = make_element("View", make_bounds(0, 0, 100, 100));
        parent.children.push(child);

        let json = serde_json::to_string(&parent).expect("serialization failed");
        let deserialized: Element = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.children.len(), 1);
        assert_eq!(deserialized.children[0].element_type, "Label");
        assert_eq!(
            deserialized.children[0].bounds,
            make_bounds(10, 10, 80, 20)
        );
    }

    #[test]
    fn bounds_with_zero_dimensions() {
        let b = make_bounds(50, 50, 0, 0);
        assert_eq!(b.center_x(), 50);
        assert_eq!(b.center_y(), 50);
        assert_eq!(b.bottom(), 50);
        assert_eq!(b.right(), 50);
    }

    // ── Viewport filtering tests ──────────────────────────────────────

    #[test]
    fn viewport_contains_fully_visible() {
        let vp = Viewport::new(375, 812);
        assert!(vp.contains(&Bounds::new(10, 10, 100, 44)));
    }

    #[test]
    fn viewport_contains_partially_visible() {
        let vp = Viewport::new(375, 812);
        // Element straddles bottom edge
        assert!(vp.contains(&Bounds::new(10, 790, 100, 44)));
    }

    #[test]
    fn viewport_excludes_fully_below() {
        let vp = Viewport::new(375, 812);
        assert!(!vp.contains(&Bounds::new(10, 900, 100, 44)));
    }

    #[test]
    fn viewport_excludes_fully_above() {
        let vp = Viewport::new(375, 812);
        assert!(!vp.contains(&Bounds::new(10, -100, 100, 44)));
    }

    #[test]
    fn filter_viewport_keeps_visible_removes_offscreen() {
        let vp = Viewport::new(375, 812);
        let mut root = make_element("View", make_bounds(0, 0, 375, 2000));
        root.children.push(make_element("Button", make_bounds(10, 100, 100, 44))); // visible
        root.children.push(make_element("Button", make_bounds(10, 900, 100, 44))); // offscreen
        root.children.push(make_element("Button", make_bounds(10, 400, 100, 44))); // visible

        let filtered = filter_viewport(&root, &vp);
        assert_eq!(
            filtered.children.len(),
            2,
            "SHALL keep 2 visible, remove 1 offscreen"
        );
    }

    #[test]
    fn filter_viewport_from_root_uses_root_bounds() {
        let root = make_element("Window", make_bounds(0, 0, 390, 844));
        let vp = Viewport::from_root(&root);
        assert_eq!(vp.width, 390);
        assert_eq!(vp.height, 844);
    }
}
