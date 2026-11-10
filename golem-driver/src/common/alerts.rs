use golem_element::Element;

/// Heuristic detection of an Android ANR ("isn't responding") system
/// dialog occluding the test app. Looks for the dialog's title text
/// — case-insensitive substring match on "isn't responding". Combines
/// well with a `Close app` / `Wait` button check but title alone is
/// sufficient (those button labels localise; the title pattern is
/// the most stable cross-locale signal we can rely on without
/// shipping a translation matrix).
///
/// Not exact — false negatives are acceptable (we just don't auto-
/// recover in that case). False positives would auto-reboot a healthy
/// device unnecessarily, which is more expensive than missing a real
/// ANR, so the matcher is conservative.
///
/// Android-only by design. iOS has no equivalent "isn't responding"
/// system dialog — the watchdog kills a hung app silently — so there is
/// nothing analogous to match, and iOS device recovery instead rides the
/// platform-agnostic wedge paths (companion unresponsive at recovery time
/// / `DeviceCompanionWedged`). We deliberately do NOT treat iOS system
/// prompts (Touch ID, Face ID, location, notifications) as ANRs: those
/// are dismissable interrupts, not device wedges, so rebooting on them
/// would be wrong (spurious reboot + lost state). This matcher only keys
/// on the Android title text, so those prompts never trip it.
pub fn detect_anr(el: &Element) -> bool {
    fn has_anr_text(el: &Element) -> bool {
        if let Some(ref t) = el.text {
            let lower = t.to_lowercase();
            if lower.contains("isn't responding") || lower.contains("isn’t responding") {
                return true;
            }
        }
        el.children.iter().any(has_anr_text)
    }
    has_anr_text(el)
}

/// Put system `alerts` (iOS SpringBoard: permission prompts, "Open in …?")
/// over the app's tree. The user can only answer such an alert, so every
/// element of the app gets empty visible bounds: it leaves the visible
/// tree, which a step judges against, and stays in the full tree as a
/// hint. The alerts become the root's last children, drawn on top.
pub fn cover_with_alerts(root: &mut Element, alerts: Vec<Element>) {
    fn hide(e: &mut Element) {
        e.visible_bounds = Some(golem_element::Bounds::new(0, 0, 0, 0));
        e.hit_points.clear();
        e.children.iter_mut().for_each(hide);
    }
    if alerts.is_empty() {
        return;
    }
    root.children.iter_mut().for_each(hide);
    root.children.extend(alerts);
}

/// Find an alert dialog in the hierarchy.
/// iOS: an element of type `alert`. Android: the tree is the active
/// window's, so the root is the dialog when that window does not start at
/// the screen's origin and holds a button. An activity's window fills the
/// screen; a dialog's (an `AlertDialog`, a Compose dialog, the permission
/// prompt) is inset. Inner layouts are not checked: any screen has inset
/// frames that hold buttons.
pub fn find_alert(root: &Element) -> Option<Element> {
    if let Some(alert) = find_typed_alert(root) {
        return Some(alert);
    }
    if (root.bounds.x > 0 || root.bounds.y > 0) && has_button_descendant(root) {
        let mut alert = root.clone();
        alert.text = extract_alert_message(&alert);
        return Some(alert);
    }
    None
}

fn find_typed_alert(el: &Element) -> Option<Element> {
    if el.element_type.eq_ignore_ascii_case("alert") {
        let mut alert = el.clone();
        // Always extract the message body — the alert's own text is the title
        alert.text = extract_alert_message(&alert).or(alert.text);
        return Some(alert);
    }
    el.children.iter().find_map(find_typed_alert)
}

/// Extract the message text from an alert's descendants.
/// The first non-button text is the title, the second is the message.
fn extract_alert_message(el: &Element) -> Option<String> {
    let mut texts = Vec::new();
    collect_non_button_text(el, &mut texts);
    // Skip the title (first text), return the message (second text)
    if texts.len() >= 2 {
        Some(texts[1].clone())
    } else {
        texts.into_iter().next()
    }
}

fn collect_non_button_text(el: &Element, texts: &mut Vec<String>) {
    // Skip buttons and the alert root — collect leaf text elements
    let et = el.element_type.to_lowercase();
    if et == "button" || is_compose_answer(el) {
        return;
    }
    if let Some(ref text) = el.text {
        if !text.is_empty() && et != "alert" {
            texts.push(text.clone());
        }
    }
    for child in &el.children {
        collect_non_button_text(child, texts);
    }
}

/// Find all buttons in an alert element.
pub fn find_alert_buttons(alert: &Element) -> Vec<Element> {
    let mut buttons = Vec::new();
    collect_buttons(alert, &mut buttons);
    buttons
}

fn collect_buttons(el: &Element, buttons: &mut Vec<Element>) {
    if let Some(answer) = compose_answer(el) {
        buttons.push(answer);
        return;
    }
    if el.element_type.eq_ignore_ascii_case("button") {
        buttons.push(el.clone());
    }
    for child in &el.children {
        collect_buttons(child, buttons);
    }
}

fn is_compose_answer(el: &Element) -> bool {
    let unlabelled_button = |c: &Element| {
        c.element_type.eq_ignore_ascii_case("button")
            && [&c.text, &c.accessibility_label]
                .into_iter()
                .flatten()
                .all(|t| t.trim().is_empty())
    };
    el.clickable && el.children.iter().any(unlabelled_button)
}

/// A Compose dialog button: a clickable node holding its label as a text
/// node and an unlabelled `Button` node that only carries the role. It is
/// one button, labelled with its text and tapped on the clickable node.
fn compose_answer(el: &Element) -> Option<Element> {
    if !is_compose_answer(el) {
        return None;
    }
    let mut texts = Vec::new();
    for child in &el.children {
        collect_non_button_text(child, &mut texts);
    }
    let mut answer = el.clone();
    answer.element_type = "Button".to_string();
    answer.text = Some(texts.join(" "));
    answer.children.clear();
    Some(answer)
}

fn has_button_descendant(el: &Element) -> bool {
    if el.element_type == "Button" {
        return true;
    }
    el.children.iter().any(has_button_descendant)
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_element::{filter_viewport, Bounds, Viewport};

    fn parse(json: serde_json::Value) -> Element {
        crate::common::parse_hierarchy(&json.to_string())
            .expect("tree")
            .0
    }

    #[test]
    fn a_system_alert_covers_the_app_in_the_visible_tree_only() {
        let mut root = parse(serde_json::json!({
            "element_type": "window", "bounds": {"x": 0, "y": 0, "width": 400, "height": 800},
            "children": [{"element_type": "button", "text": "Submit",
                          "bounds": {"x": 10, "y": 100, "width": 100, "height": 44}}]
        }));
        let alert = parse(serde_json::json!({
            "element_type": "alert", "text": "Open in “App”?",
            "bounds": {"x": 60, "y": 380, "width": 270, "height": 104},
            "children": [{"element_type": "button", "text": "Open",
                          "bounds": {"x": 200, "y": 440, "width": 130, "height": 44}}]
        }));
        cover_with_alerts(&mut root, vec![alert]);
        let visible = filter_viewport(&root, &Viewport::from_root(&root));
        let texts: Vec<&str> = visible
            .children
            .iter()
            .filter_map(|e| e.text.as_deref())
            .collect();
        assert_eq!(
            texts,
            vec!["Open in “App”?", "Open"],
            "the app's Submit SHALL be covered"
        );
        assert_eq!(
            root.children[0].bounds,
            Bounds::new(10, 100, 100, 44),
            "the full tree SHALL keep it, as a hint"
        );
        assert!(find_alert(&root).is_some());
    }

    #[test]
    fn no_system_alert_leaves_the_tree_as_it_is() {
        let mut root = parse(serde_json::json!({
            "element_type": "window", "bounds": {"x": 0, "y": 0, "width": 400, "height": 800},
            "children": [{"element_type": "button", "text": "Submit",
                          "bounds": {"x": 10, "y": 100, "width": 100, "height": 44}}]
        }));
        let before = format!("{root:?}");
        cover_with_alerts(&mut root, Vec::new());
        assert_eq!(format!("{root:?}"), before);
    }
}
