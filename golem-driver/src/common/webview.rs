// ---------------------------------------------------------------------------
// WebView detection helpers shared by Android and iOS drivers
// ---------------------------------------------------------------------------

/// Find the first WebView element in a JSON hierarchy and return its bounds (x, y).
///
/// Recognizes both Android (`class == "android.webkit.WebView"`) and iOS
/// (`element_type == "web_view"`) formats. Handles array roots (iOS companion
/// sends `[window, window]`).
pub(crate) fn find_webview_bounds(val: &serde_json::Value) -> Option<(i32, i32)> {
    // Handle array root
    if let Some(arr) = val.as_array() {
        for item in arr {
            if let Some(bounds) = find_webview_bounds(item) {
                return Some(bounds);
            }
        }
        return None;
    }
    // Android: class field
    let is_webview = val
        .get("class")
        .and_then(|v| v.as_str())
        .is_some_and(|c| c == "android.webkit.WebView")
        || val
            .get("element_type")
            .and_then(|v| v.as_str())
            .is_some_and(|e| e == "web_view");
    if is_webview {
        let bounds = val.get("bounds")?;
        // Support both {left,top} (Android) and {x,y} (iOS) formats
        let x = bounds
            .get("left")
            .or_else(|| bounds.get("x"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let y = bounds
            .get("top")
            .or_else(|| bounds.get("y"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        return Some((x, y));
    }
    if let Some(children) = val.get("children").and_then(|c| c.as_array()) {
        for child in children {
            if let Some(bounds) = find_webview_bounds(child) {
                return Some(bounds);
            }
        }
    }
    None
}

/// Replace the first WebView element's children with DOM data from CDP/WebKit Inspector.
///
/// Recognizes both Android and iOS WebView element types. Handles array roots.
pub(crate) fn replace_webview_children(
    val: &mut serde_json::Value,
    dom: serde_json::Value,
) -> bool {
    if let Some(arr) = val.as_array_mut() {
        for item in arr {
            if replace_webview_children(item, dom.clone()) {
                return true;
            }
        }
        return false;
    }
    let is_webview = val
        .get("class")
        .and_then(|v| v.as_str())
        .is_some_and(|c| c == "android.webkit.WebView")
        || val
            .get("element_type")
            .and_then(|v| v.as_str())
            .is_some_and(|e| e == "web_view");
    if is_webview {
        if let Some(children) = val.get_mut("children").and_then(|c| c.as_array_mut()) {
            children.clear();
            children.push(dom);
        }
        return true;
    }
    if let Some(children) = val.get_mut("children").and_then(|c| c.as_array_mut()) {
        for child in children {
            if replace_webview_children(child, dom.clone()) {
                return true;
            }
        }
    }
    false
}
