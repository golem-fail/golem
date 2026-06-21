#![cfg(test)]

use golem_element::{Bounds, Element};
use golem_parser::Step;
use golem_vars::{Scope, ScopeLevel, VariableStore};

pub fn make_step(action: &str) -> Step {
    Step {
        action: action.to_string(),
        ..Default::default()
    }
}

pub fn make_element(element_type: &str, bounds: Bounds) -> Element {
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
        visible_bounds: None,
        hit_points: vec![],
        drawing_order: None,
        children: Vec::new(),
    }
}

pub fn make_element_with_text(element_type: &str, text: &str, bounds: Bounds) -> Element {
    let mut e = make_element(element_type, bounds);
    e.text = Some(text.to_string());
    e
}

pub fn make_element_with_id(element_type: &str, id: &str, bounds: Bounds) -> Element {
    let mut e = make_element(element_type, bounds);
    e.accessibility_label = Some(id.to_string());
    e
}

pub fn make_element_with_id_and_text(
    element_type: &str,
    id: &str,
    text: &str,
    bounds: Bounds,
) -> Element {
    let mut e = make_element(element_type, bounds);
    e.accessibility_label = Some(id.to_string());
    e.text = Some(text.to_string());
    e
}

pub fn make_vars() -> VariableStore {
    let mut store = VariableStore::new();
    store.push_scope(Scope::new(ScopeLevel::Flow));
    store
}

pub fn root_with_button(text: &str) -> Element {
    let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
    root.children.push(make_element_with_text(
        "Button",
        text,
        Bounds::new(100, 200, 100, 44),
    ));
    root
}

pub fn root_with_input(id: &str) -> Element {
    let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
    let mut input = make_element_with_id("TextField", id, Bounds::new(20, 100, 300, 44));
    // Mark the input as focused so handle_type's post-tap focus check
    // passes without forcing a retry. Real devices set `focused=true`
    // on the tapped input; the mock driver doesn't simulate state
    // changes, so the fixture pre-sets it for happy-path tests.
    input.focused = true;
    root.children.push(input);
    root
}
