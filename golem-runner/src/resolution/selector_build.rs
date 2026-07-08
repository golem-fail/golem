use golem_element::selector::{AnchorSelector, Selector};
use golem_parser::{Anchor, Step};

/// Build a `Selector` from the fields of a parsed `Step`.
///
/// Maps each optional selector/filter field on the step to the
/// corresponding field on `Selector`. Fields that are `None` on the
/// step remain `None` on the selector (i.e. not constrained).
/// Build a `Selector` from the step's selector fields.
///
/// Supports three syntaxes:
/// - Flat: `on_text = "Submit"`, `on_below = "Counter"`
/// - Grouped: `on = { text = "Submit", below = "Counter" }`
/// - To alias: `to = { text = "Item 49" }`
///
/// Grouped fields take precedence over flat fields.
/// Convert a parser `Anchor` to a runtime `AnchorSelector`.
fn convert_anchor(anchor: &Anchor) -> AnchorSelector {
    match anchor {
        Anchor::Text(s) => AnchorSelector::Text(s.clone()),
        Anchor::Selector(group) => AnchorSelector::Full(Box::new(build_selector_from_group(group))),
    }
}

/// Convert a parser `ContainsAnchor` to a runtime `AnchorSelector` (the
/// `min_matches` count is carried separately on the `Selector`).
fn convert_contains_anchor(anchor: &golem_parser::ContainsAnchor) -> AnchorSelector {
    match anchor {
        golem_parser::ContainsAnchor::Text(s) => AnchorSelector::Text(s.clone()),
        golem_parser::ContainsAnchor::Spec(spec) => {
            AnchorSelector::Full(Box::new(build_selector_from_group(&spec.group)))
        }
    }
}

/// Build a `Selector` from a `SelectorGroup` (recursive for nested anchors).
pub fn build_selector_from_group(g: &golem_parser::SelectorGroup) -> Selector {
    Selector {
        text: g.text.clone(),
        accessibility_label: g.accessibility_label.clone(),
        index: g.index,
        enabled: g.enabled,
        checked: g.checked,
        clickable: g.clickable,
        below: g.below.as_ref().map(convert_anchor),
        above: g.above.as_ref().map(convert_anchor),
        right_of: g.right_of.as_ref().map(convert_anchor),
        left_of: g.left_of.as_ref().map(convert_anchor),
        contains: g.contains.as_ref().map(convert_contains_anchor),
        contains_min_matches: g.contains.as_ref().map(|c| c.min_matches()),
        inside: g.inside.as_ref().map(convert_anchor),
        traits: g.traits.clone(),
    }
}

/// Build a `Selector` from the step's selector fields.
///
/// Supports flat `on_*`, grouped `on = {}`, `to = {}`, and nested anchors.
/// Grouped fields take precedence over flat fields.
pub fn build_selector(step: &Step) -> Selector {
    let g = step.on.as_ref();
    Selector {
        text: g.and_then(|g| g.text.clone()).or(step.on_text.clone()),
        accessibility_label: g
            .and_then(|g| g.accessibility_label.clone())
            .or(step.on_accessibility_label.clone()),
        index: g.and_then(|g| g.index).or(step.on_index),
        enabled: g.and_then(|g| g.enabled).or(step.on_enabled),
        checked: g.and_then(|g| g.checked).or(step.on_checked),
        clickable: g.and_then(|g| g.clickable).or(step.on_clickable),
        below: g.and_then(|g| g.below.as_ref().map(convert_anchor)).or(step
            .on_below
            .as_ref()
            .map(|s| AnchorSelector::Text(s.clone()))),
        above: g.and_then(|g| g.above.as_ref().map(convert_anchor)).or(step
            .on_above
            .as_ref()
            .map(|s| AnchorSelector::Text(s.clone()))),
        right_of: g
            .and_then(|g| g.right_of.as_ref().map(convert_anchor))
            .or(step
                .on_right_of
                .as_ref()
                .map(|s| AnchorSelector::Text(s.clone()))),
        left_of: g
            .and_then(|g| g.left_of.as_ref().map(convert_anchor))
            .or(step
                .on_left_of
                .as_ref()
                .map(|s| AnchorSelector::Text(s.clone()))),
        contains: g.and_then(|g| g.contains.as_ref().map(convert_contains_anchor)),
        contains_min_matches: g.and_then(|g| g.contains.as_ref().map(|c| c.min_matches())),
        inside: g.and_then(|g| g.inside.as_ref().map(convert_anchor)),
        traits: g.map(|g| g.traits.clone()).unwrap_or_default(),
    }
}

/// Build a human-readable label for a selector (for event output).
pub(crate) fn selector_label(sel: &Selector) -> String {
    if let Some(ref t) = sel.text {
        return t.clone();
    }
    if let Some(ref a) = sel.accessibility_label {
        return a.clone();
    }
    "?".to_string()
}
