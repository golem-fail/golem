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

/// Walk an element tree looking for an alert-type element.
/// Find an alert dialog in the hierarchy.
/// iOS: element_type == "alert". Android: detects the dialog pattern
/// (a top-level window containing a title + message + button).
pub fn find_alert(el: &Element) -> Option<Element> {
    // iOS: native alert element type
    if el.element_type.eq_ignore_ascii_case("alert") {
        let mut alert = el.clone();
        // Always extract the message body — the alert's own text is the title
        alert.text = extract_alert_message(&alert).or(alert.text);
        return Some(alert);
    }
    // Android: dialog window pattern — FrameLayout at non-zero y with
    // a Button child (native alert dialogs have this structure)
    if el.element_type == "FrameLayout" && el.bounds.y > 0 && has_button_descendant(el) {
        let mut alert = el.clone();
        alert.text = extract_alert_message(&alert);
        return Some(alert);
    }
    for child in &el.children {
        if let Some(alert) = find_alert(child) {
            return Some(alert);
        }
    }
    None
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
    if et == "button" {
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
    let et = el.element_type.to_lowercase();
    if et == "button" {
        buttons.push(el.clone());
    }
    for child in &el.children {
        collect_buttons(child, buttons);
    }
}

fn has_button_descendant(el: &Element) -> bool {
    if el.element_type == "Button" {
        return true;
    }
    el.children.iter().any(has_button_descendant)
}
