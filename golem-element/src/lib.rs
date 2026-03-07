pub mod glob;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub element_type: String,
    pub text: Option<String>,
    pub id: Option<String>,
    pub placeholder: Option<String>,
    pub enabled: bool,
    pub checked: bool,
    pub clickable: bool,
    pub focused: bool,
    pub bounds: Bounds,
    #[serde(default)]
    pub children: Vec<Element>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub fn center_x(&self) -> f64 {
        self.x + self.width / 2.0
    }
    pub fn center_y(&self) -> f64 {
        self.y + self.height / 2.0
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }
    pub fn right(&self) -> f64 {
        self.x + self.width
    }
}

#[derive(Debug, Clone)]
pub struct FindResult {
    pub element: Element,
    pub tap_x: f64,
    pub tap_y: f64,
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

    fn make_bounds(x: f64, y: f64, width: f64, height: f64) -> Bounds {
        Bounds::new(x, y, width, height)
    }

    fn make_element(element_type: &str, bounds: Bounds) -> Element {
        Element {
            element_type: element_type.to_string(),
            text: None,
            id: None,
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
        let b = make_bounds(10.0, 20.0, 100.0, 50.0);
        assert!((b.center_x() - 60.0).abs() < f64::EPSILON);
        assert!((b.center_y() - 45.0).abs() < f64::EPSILON);
    }

    #[test]
    fn bounds_bottom_and_right() {
        let b = make_bounds(10.0, 20.0, 100.0, 50.0);
        assert!((b.bottom() - 70.0).abs() < f64::EPSILON);
        assert!((b.right() - 110.0).abs() < f64::EPSILON);
    }

    #[test]
    fn find_result_computes_tap_coordinates() {
        let elem = make_element("Button", make_bounds(0.0, 0.0, 200.0, 80.0));
        let result = FindResult::new(elem);
        assert!((result.tap_x - 100.0).abs() < f64::EPSILON);
        assert!((result.tap_y - 40.0).abs() < f64::EPSILON);
    }

    #[test]
    fn element_serialization_round_trip() {
        let elem = Element {
            element_type: "TextField".to_string(),
            text: Some("hello".to_string()),
            id: Some("input-1".to_string()),
            placeholder: Some("Enter name".to_string()),
            enabled: true,
            checked: false,
            clickable: true,
            focused: true,
            bounds: make_bounds(5.0, 10.0, 300.0, 44.0),
            children: Vec::new(),
        };

        let json = serde_json::to_string(&elem).expect("serialization failed");
        let deserialized: Element = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.element_type, "TextField");
        assert_eq!(deserialized.text.as_deref(), Some("hello"));
        assert_eq!(deserialized.id.as_deref(), Some("input-1"));
        assert_eq!(deserialized.placeholder.as_deref(), Some("Enter name"));
        assert!(deserialized.enabled);
        assert!(!deserialized.checked);
        assert!(deserialized.clickable);
        assert!(deserialized.focused);
        assert_eq!(deserialized.bounds, make_bounds(5.0, 10.0, 300.0, 44.0));
        assert!(deserialized.children.is_empty());
    }

    #[test]
    fn element_with_children_serializes_and_deserializes() {
        let child = make_element("Label", make_bounds(10.0, 10.0, 80.0, 20.0));
        let mut parent = make_element("View", make_bounds(0.0, 0.0, 100.0, 100.0));
        parent.children.push(child);

        let json = serde_json::to_string(&parent).expect("serialization failed");
        let deserialized: Element = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.children.len(), 1);
        assert_eq!(deserialized.children[0].element_type, "Label");
        assert_eq!(
            deserialized.children[0].bounds,
            make_bounds(10.0, 10.0, 80.0, 20.0)
        );
    }

    #[test]
    fn bounds_with_zero_dimensions() {
        let b = make_bounds(50.0, 50.0, 0.0, 0.0);
        assert!((b.center_x() - 50.0).abs() < f64::EPSILON);
        assert!((b.center_y() - 50.0).abs() < f64::EPSILON);
        assert!((b.bottom() - 50.0).abs() < f64::EPSILON);
        assert!((b.right() - 50.0).abs() < f64::EPSILON);
    }
}
