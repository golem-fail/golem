use anyhow::{Context, Result};
use golem_element::Element;
use serde::Serialize;

use crate::ios_display;

// ---------------------------------------------------------------------------
// Request / response DTOs for companion server communication
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub(crate) struct TapRequest {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Serialize)]
pub(crate) struct TypeRequest<'a> {
    pub text: &'a str,
}

#[derive(Debug, Serialize)]
pub(crate) struct BackspaceRequest {
    pub count: u32,
}

#[derive(Debug, Serialize)]
pub(crate) struct LongPressRequest {
    pub x: i32,
    pub y: i32,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SwipeRequest {
    pub from_x: i32,
    pub from_y: i32,
    pub to_x: i32,
    pub to_y: i32,
    pub duration_ms: u64,
}


// ---------------------------------------------------------------------------
// Parsing helpers (testable without HTTP)
// ---------------------------------------------------------------------------

/// Parse a hierarchy JSON response body into an `Element` tree.
///
/// Metadata returned alongside the hierarchy.
#[derive(Debug, Default, Clone)]
pub struct HierarchyMeta {
    /// Height of the on-screen keyboard (0 if hidden).
    pub keyboard_height: i32,
    /// Safe area inset from top (status bar / notch area, in device units).
    pub safe_area_top: i32,
    /// Safe area inset from bottom (navigation bar / home indicator, in device units).
    pub safe_area_bottom: i32,
    /// Safe area inset from left (Android back-from-edge gesture, iOS swipe-from-edge zones).
    /// Default 0 until the companion populates it.
    pub safe_area_left: i32,
    /// Safe area inset from right (mirror of left).
    pub safe_area_right: i32,
    /// Display cutout regions where physical pixels don't exist (notch, punch-hole).
    pub cutouts: Vec<CutoutRect>,
    /// Rounded display corners where physical pixels don't exist.
    pub rounded_corners: Vec<RoundedCorner>,
    /// Number of nodes in the hierarchy tree (for verbose stats).
    pub node_count: u32,
}

/// A rectangular region of the display with no physical pixels (notch, Dynamic Island, etc.).
/// Coordinates are in the device's native coordinate space (pixels on Android, points on iOS).
#[derive(Debug, Clone)]
pub struct CutoutRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A rounded display corner where physical pixels don't exist outside the curve.
#[derive(Debug, Clone)]
pub struct RoundedCorner {
    pub position: CornerPosition,
    pub radius: i32,
    pub center_x: i32,
    pub center_y: i32,
}

/// Which corner of the display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CornerPosition {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

/// Parse a hierarchy JSON response from a companion server.
///
/// Handles three formats:
/// - `{ "tree": [...], "keyboard_height": N }` — wrapper with metadata
/// - `[{...}, ...]` — array of root elements
/// - `{...}` — single root element
pub fn parse_hierarchy(json: &str) -> Result<(Element, HierarchyMeta)> {
    let mut val: serde_json::Value =
        serde_json::from_str(json).context("failed to parse hierarchy JSON")?;

    // Extract metadata from wrapper format
    let mut meta = HierarchyMeta::default();
    let mut device_model: Option<String> = None;
    if let Some(obj) = val.as_object() {
        if obj.contains_key("tree") {
            meta.keyboard_height = obj
                .get("keyboard_height")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            meta.safe_area_top = obj
                .get("safe_area_top")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            meta.safe_area_bottom = obj
                .get("safe_area_bottom")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            meta.safe_area_left = obj
                .get("safe_area_left")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            meta.safe_area_right = obj
                .get("safe_area_right")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            meta.cutouts = parse_cutouts_json(obj.get("cutouts"));
            meta.rounded_corners = parse_corners_json(obj.get("rounded_corners"));
            device_model = obj.get("device_model").and_then(|v| v.as_str()).map(String::from);
            val = obj.get("tree").cloned().unwrap_or(val);
        }
    }

    // Handle array wrapper: some companions return `[{...}]` instead of `{...}`.
    if let serde_json::Value::Array(ref mut arr) = val {
        for item in arr.iter_mut() {
            normalize_json(item);
        }
        if arr.len() == 1 {
            val = arr.remove(0);
        } else {
            // Multiple roots — wrap in a synthetic container.
            // Compute bounding box from children so viewport filtering works.
            let (mut max_w, mut max_h) = (0i64, 0i64);
            for item in arr.iter() {
                if let Some(b) = item.get("bounds") {
                    let w = b.get("x").and_then(|v| v.as_i64()).unwrap_or(0)
                        + b.get("width").and_then(|v| v.as_i64()).unwrap_or(0);
                    let h = b.get("y").and_then(|v| v.as_i64()).unwrap_or(0)
                        + b.get("height").and_then(|v| v.as_i64()).unwrap_or(0);
                    max_w = max_w.max(w);
                    max_h = max_h.max(h);
                }
            }
            let wrapped = serde_json::json!({
                "element_type": "other",
                "text": null,
                "accessibility_label": null,
                "placeholder": null,
                "enabled": true,
                "checked": false,
                "clickable": false,
                "focused": false,
                "bounds": { "x": 0, "y": 0, "width": max_w, "height": max_h },
                "children": arr
            });
            val = wrapped;
        }
    } else {
        normalize_json(&mut val);
    }

    let element: Element = serde_json::from_value(val).context("failed to deserialize hierarchy into Element")?;
    meta.node_count = count_nodes(&element);

    // iOS: look up display data from device model using screen dimensions from the parsed tree
    if let Some(model) = device_model {
        if let Some(display) = ios_display::lookup(
            &model,
            element.bounds.width,
            element.bounds.height,
        ) {
            if meta.cutouts.is_empty() {
                meta.cutouts = display.cutouts;
            }
            if meta.rounded_corners.is_empty() {
                meta.rounded_corners = display.rounded_corners;
            }
            // Companion doesn't probe gesture insets — populate from the
            // static toml only when the parsed response didn't include them.
            if meta.safe_area_left == 0 {
                meta.safe_area_left = display.gesture_inset_left;
            }
            if meta.safe_area_right == 0 {
                meta.safe_area_right = display.gesture_inset_right;
            }
        }
    }

    Ok((element, meta))
}

/// Count total nodes in an element tree.
fn count_nodes(el: &Element) -> u32 {
    1 + el.children.iter().map(count_nodes).sum::<u32>()
}

/// Recursively normalize a JSON hierarchy node to match the `Element` schema.
///
/// Handles two companion server formats:
/// - **iOS (mobile-bench):** `label` field instead of `text`/`id`
/// - **Android:** `class` instead of `element_type`, `contentDescription` instead of `id`,
///   `bounds` with `left/top/right/bottom` instead of `x/y/width/height`
fn normalize_json(val: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = val {
        // Rename raw `id` → `accessibility_label` (GOLEM companion format)
        if map.contains_key("id") && !map.contains_key("accessibility_label") {
            if let Some(v) = map.remove("id") {
                map.insert("accessibility_label".to_string(), v);
            }
        }

        // Android: rename `class` → `element_type`
        if map.contains_key("class") && !map.contains_key("element_type") {
            if let Some(class) = map.remove("class") {
                // Simplify Android class names: "android.widget.Button" → "Button"
                let simplified = class
                    .as_str()
                    .and_then(|s| s.rsplit('.').next())
                    .unwrap_or("")
                    .to_string();
                map.insert("element_type".to_string(), serde_json::Value::String(simplified));
            }
        }

        // Android: use `contentDescription` as `id` when present and non-empty.
        if let Some(cd) = map.get("contentDescription").and_then(|v| v.as_str()) {
            if !cd.is_empty() {
                let id_empty = map
                    .get("accessibility_label")
                    .and_then(|v| v.as_str())
                    .is_none_or(|s| s.is_empty());
                if id_empty {
                    map.insert("accessibility_label".to_string(), serde_json::Value::String(cd.to_string()));
                }
            }
        }

        // Fix checked state for switches/toggles: iOS reports state via value "0"/"1"
        // rather than isSelected. Normalize to checked = true/false.
        let is_switch = map.get("element_type")
            .and_then(|v| v.as_str())
            .is_some_and(|et| {
                let lower = et.to_lowercase();
                lower == "switch" || lower == "toggle" || lower == "checkbox"
            });
        if is_switch {
            let value_is_on = map.get("value")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
            if value_is_on {
                map.insert("checked".to_string(), serde_json::Value::Bool(true));
            }
        }

        // Android: convert bounds and visible_bounds from {left,top,right,bottom} → {x,y,width,height}
        for key in ["bounds", "visible_bounds"] {
            if let Some(serde_json::Value::Object(rect)) = map.get_mut(key) {
                normalize_android_rect(rect);
            }
        }

        // Build `text` reflecting what the user sees on screen.
        //
        // For inputs: value → placeholder → label → text content
        //   (value = what's typed, placeholder = hint when empty)
        // For everything else: placeholder → label → text content
        //   (value is unreliable — iOS reports internal state for non-inputs,
        //    switches report "0"/"1" toggle state)

        // Promote label → accessibility_label (always, regardless of text chain)
        promote_label_to_id(map);

        let current_text = map
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let value = map
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let label = map
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let placeholder = map
            .get("placeholder")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let is_input = matches!(
            map.get("element_type").and_then(|v| v.as_str()),
            Some("text_field" | "secure_text_field" | "search_field" | "text_view"
                | "EditText" | "AutoCompleteTextView")
        );

        let resolved_text = if is_input && !value.is_empty() {
            value
        } else if !placeholder.is_empty() {
            placeholder
        } else if !label.is_empty() {
            label
        } else {
            current_text
        };

        if !resolved_text.is_empty() {
            map.insert("text".to_string(), serde_json::Value::String(resolved_text));
        }

        // Recurse into children
        if let Some(serde_json::Value::Array(arr)) = map.get_mut("children") {
            for child in arr {
                normalize_json(child);
            }
        }
    }
}

/// Convert Android rect format {left,top,right,bottom} → {x,y,width,height}.
/// Clamps dimensions to 0 — Android WebView clips bottom/right to the visible
/// area, causing negative dimensions for off-screen elements.
fn normalize_android_rect(rect: &mut serde_json::Map<String, serde_json::Value>) {
    if rect.contains_key("left") && !rect.contains_key("x") {
        let left = rect.get("left").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let top = rect.get("top").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let right = rect.get("right").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let bottom = rect.get("bottom").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        rect.insert("x".to_string(), serde_json::json!(left));
        rect.insert("y".to_string(), serde_json::json!(top));
        rect.insert("width".to_string(), serde_json::json!((right - left).max(0)));
        rect.insert("height".to_string(), serde_json::json!((bottom - top).max(0)));
    }
}

/// Promote `label` (aria-label) to `accessibility_label` when id is absent/empty.
fn promote_label_to_id(map: &mut serde_json::Map<String, serde_json::Value>) {
    let label_str = map
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let id_empty = match map.get("accessibility_label") {
        Some(serde_json::Value::String(s)) => s.is_empty(),
        Some(serde_json::Value::Null) | None => true,
        _ => false,
    };
    if id_empty && !label_str.is_empty() {
        map.insert("accessibility_label".to_string(), serde_json::Value::String(label_str));
    }
}

/// Build the JSON body for a tap request.
pub(crate) fn build_tap_body(x: i32, y: i32) -> Result<String> {
    serde_json::to_string(&TapRequest { x, y }).context("failed to serialize tap request")
}

/// Build the JSON body for a type-text request.
pub(crate) fn build_type_body(text: &str) -> Result<String> {
    serde_json::to_string(&TypeRequest { text }).context("failed to serialize type request")
}

/// Build the JSON body for a backspace request.
pub(crate) fn build_backspace_body(count: u32) -> Result<String> {
    serde_json::to_string(&BackspaceRequest { count })
        .context("failed to serialize backspace request")
}

/// Build the JSON body for a long-press request.
pub(crate) fn build_long_press_body(x: i32, y: i32, duration_ms: u64) -> Result<String> {
    serde_json::to_string(&LongPressRequest {
        x,
        y,
        duration_ms,
    })
    .context("failed to serialize long press request")
}

/// Build the JSON body for a swipe request.
pub(crate) fn build_swipe_body(
    from_x: i32,
    from_y: i32,
    to_x: i32,
    to_y: i32,
    duration_ms: u64,
) -> Result<String> {
    serde_json::to_string(&SwipeRequest {
        from_x,
        from_y,
        to_x,
        to_y,
        duration_ms,
    })
    .context("failed to serialize swipe request")
}

/// Build the JSON body for a gesture request.
pub(crate) fn build_gesture_body(fingers: &[crate::GestureFinger]) -> Result<String> {
    let fingers_json: Vec<serde_json::Value> = fingers
        .iter()
        .map(|f| {
            let points: Vec<Vec<i32>> = f.points.iter().map(|(x, y)| vec![*x, *y]).collect();
            serde_json::json!({
                "points": points,
                "duration_ms": f.duration_ms,
            })
        })
        .collect();
    serde_json::to_string(&serde_json::json!({ "fingers": fingers_json }))
        .context("failed to serialize gesture request")
}


// ---------------------------------------------------------------------------
// Cutout / corner JSON parsing helpers
// ---------------------------------------------------------------------------

/// Parse cutout rects from a JSON array: `[{"x":N,"y":N,"width":N,"height":N}, ...]`
fn parse_cutouts_json(val: Option<&serde_json::Value>) -> Vec<CutoutRect> {
    let arr = match val.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|item| {
            let x = item.get("x")?.as_i64()? as i32;
            let y = item.get("y")?.as_i64()? as i32;
            let w = item.get("width")?.as_i64()? as i32;
            let h = item.get("height")?.as_i64()? as i32;
            if w > 0 && h > 0 {
                Some(CutoutRect { x, y, width: w, height: h })
            } else {
                None
            }
        })
        .collect()
}

/// Parse rounded corners from a JSON array:
/// `[{"position":"top_left","radius":N,"center_x":N,"center_y":N}, ...]`
fn parse_corners_json(val: Option<&serde_json::Value>) -> Vec<RoundedCorner> {
    let arr = match val.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|item| {
            let pos_str = item.get("position")?.as_str()?;
            let position = match pos_str {
                "top_left" => CornerPosition::TopLeft,
                "top_right" => CornerPosition::TopRight,
                "bottom_right" => CornerPosition::BottomRight,
                "bottom_left" => CornerPosition::BottomLeft,
                _ => return None,
            };
            let radius = item.get("radius")?.as_i64()? as i32;
            let center_x = item.get("center_x")?.as_i64()? as i32;
            let center_y = item.get("center_y")?.as_i64()? as i32;
            Some(RoundedCorner { position, radius, center_x, center_y })
        })
        .collect()
}

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

// ---------------------------------------------------------------------------
// HTTP client wrapper shared by both platform drivers
// ---------------------------------------------------------------------------

/// Thin HTTP client wrapper used by both `AndroidDriver` and `IosDriver`.
pub struct CompanionClient {
    pub base_url: String,
    /// Default query string appended to every request (e.g. `"bundle_id=fail.golem.test"`).
    default_query: std::sync::RwLock<String>,
    pub client: reqwest::Client,
    /// Per-request timeout (ms) applied to `post_json`/`get_text`/`get_bytes`.
    /// 0 = no per-request timeout. Updated by the runner before each step
    /// so a wedged companion surfaces as a clean network error before the
    /// outer `tokio::time::timeout` cancels the future.
    request_timeout_ms: std::sync::atomic::AtomicU64,
}

/// Health information returned by the companion server.
#[derive(Debug)]
pub struct CompanionHealth {
    pub platform: String,
    pub version: String,
    pub device_name: String,
    pub os_version: String,
    pub device_id: String,
}

impl CompanionClient {
    pub fn new(port: u16) -> Self {
        Self {
            base_url: format!("http://localhost:{port}"),
            default_query: std::sync::RwLock::new(String::new()),
            client: reqwest::Client::new(),
            request_timeout_ms: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Update the default query string (e.g. on app switch).
    pub fn set_default_query(&self, query: &str) {
        if let Ok(mut q) = self.default_query.write() {
            *q = query.to_string();
        }
    }

    /// Set the per-request timeout for `post_json`/`get_text`/`get_bytes`.
    /// Stored with whole-millisecond precision (`as_millis` truncation).
    /// Pass `Duration::ZERO`, or any value that truncates to 0ms, to clear
    /// the timeout (`current_request_timeout` then returns `None`).
    pub fn set_request_timeout(&self, timeout: std::time::Duration) {
        let ms = timeout.as_millis().min(u64::MAX as u128) as u64;
        self.request_timeout_ms
            .store(ms, std::sync::atomic::Ordering::Relaxed);
    }

    fn current_request_timeout(&self) -> Option<std::time::Duration> {
        let ms = self
            .request_timeout_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        (ms > 0).then(|| std::time::Duration::from_millis(ms))
    }

    /// Check companion health and return device info.
    ///
    /// Returns `Ok(health)` if the companion is running and responsive.
    /// Returns `Err` if the companion is not reachable or returns unexpected data.
    pub async fn check_health(&self) -> Result<CompanionHealth> {
        let url = format!("{}/health", self.base_url);
        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .with_context(|| format!(
                "Companion server not reachable at {}. Is it running?",
                self.base_url
            ))?;

        // 503 = companion is up but its UiAutomation accessibility-
        // service binding hasn't warmed up yet (Android first-flow
        // race). Treat as not-ready so `wait_for_health` keeps polling
        // instead of declaring readiness on the HTTP socket alone.
        let status = resp.status();
        let text = resp.text().await.context("reading health response")?;
        if !status.is_success() {
            anyhow::bail!("companion /health returned {}: {}", status.as_u16(), text);
        }
        let json: serde_json::Value =
            serde_json::from_str(&text).context("parsing health response")?;

        Ok(CompanionHealth {
            platform: json["platform"].as_str().unwrap_or("unknown").to_string(),
            version: json["version"].as_str().unwrap_or("unknown").to_string(),
            device_name: json["device_name"].as_str().unwrap_or("unknown").to_string(),
            os_version: json["os_version"].as_str().unwrap_or("unknown").to_string(),
            device_id: json["device_id"].as_str().unwrap_or("unknown").to_string(),
        })
    }

    /// Poll `/health` until the companion responds or timeout expires.
    ///
    /// Polls every 2 seconds. Returns the health info on success.
    pub async fn wait_for_health(&self, timeout: std::time::Duration) -> Result<CompanionHealth> {
        let deadline = tokio::time::Instant::now() + timeout;
        let poll_interval = std::time::Duration::from_secs(2);

        loop {
            match self.check_health().await {
                Ok(health) => return Ok(health),
                Err(_) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(poll_interval).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Build the full URL for a request, appending the default query string.
    fn url(&self, path: &str) -> String {
        let dq = self.default_query.read().map(|q| q.clone()).unwrap_or_default();
        let sep = if path.contains('?') { "&" } else { "?" };
        if dq.is_empty() {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}{}{}{}", self.base_url, path, sep, dq)
        }
    }

    pub async fn post_json(&self, path: &str, body: &str) -> Result<String> {
        let url = self.url(path);
        let mut req = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body.to_string());
        if let Some(t) = self.current_request_timeout() {
            req = req.timeout(t);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("POST {url} failed"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .with_context(|| format!("reading response body from POST {url}"))?;

        if !status.is_success() {
            anyhow::bail!("POST {url} returned {status}: {text}");
        }

        Ok(text)
    }

    pub async fn get_text(&self, path: &str) -> Result<String> {
        let url = self.url(path);
        let mut req = self.client.get(&url);
        if let Some(t) = self.current_request_timeout() {
            req = req.timeout(t);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .with_context(|| format!("reading response body from GET {url}"))?;

        if !status.is_success() {
            anyhow::bail!("GET {url} returned {status}: {text}");
        }

        Ok(text)
    }

    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let url = self.url(path);
        let mut req = self.client.get(&url);
        if let Some(t) = self.current_request_timeout() {
            req = req.timeout(t);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable>".to_string());
            anyhow::bail!("GET {url} returned {status}: {text}");
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .with_context(|| format!("reading bytes from GET {url}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- parse_hierarchy ----

    // 1. A single root object parses into an Element with node_count 1.
    #[test]
    fn parse_hierarchy_single_root_object() {
        let json = r#"{
            "element_type": "other", "text": null, "accessibility_label": null,
            "placeholder": null, "enabled": true, "checked": false,
            "clickable": false, "focused": false,
            "bounds": { "x": 0, "y": 0, "width": 100, "height": 200 },
            "children": []
        }"#;
        let (el, meta) = parse_hierarchy(json).expect("single root SHALL parse");
        assert_eq!(el.element_type, "other", "element_type SHALL round-trip");
        assert_eq!(meta.node_count, 1, "single node SHALL count as 1");
        assert_eq!(meta.keyboard_height, 0, "absent keyboard_height SHALL default 0");
    }

    // 2. A single-element array unwraps to that element (no synthetic container).
    #[test]
    fn parse_hierarchy_single_element_array_unwraps() {
        let json = r#"[{
            "element_type": "Button", "text": "OK", "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 10, "height": 10 },
            "children": []
        }]"#;
        let (el, meta) = parse_hierarchy(json).expect("array of one SHALL parse");
        assert_eq!(el.element_type, "Button", "lone array element SHALL be unwrapped");
        assert_eq!(meta.node_count, 1, "unwrapped lone element SHALL count as 1");
    }

    // 3. A multi-root array is wrapped in a synthetic `other` container whose
    //    bounds span the union of children right/bottom edges.
    #[test]
    fn parse_hierarchy_multi_root_array_wraps_with_bounding_box() {
        let json = r#"[
            { "element_type": "A", "text": null, "accessibility_label": null,
              "placeholder": null,
              "bounds": { "x": 0, "y": 0, "width": 50, "height": 60 }, "children": [] },
            { "element_type": "B", "text": null, "accessibility_label": null,
              "placeholder": null,
              "bounds": { "x": 100, "y": 200, "width": 30, "height": 40 }, "children": [] }
        ]"#;
        let (el, meta) = parse_hierarchy(json).expect("multi-root SHALL parse");
        assert_eq!(el.element_type, "other", "wrapper SHALL be synthetic `other`");
        assert_eq!(el.children.len(), 2, "wrapper SHALL hold both roots");
        assert_eq!(el.bounds.width, 130, "wrapper width SHALL be max child right edge");
        assert_eq!(el.bounds.height, 240, "wrapper height SHALL be max child bottom edge");
        assert_eq!(meta.node_count, 3, "two roots plus wrapper SHALL count as 3");
    }

    // 4. The `{ "tree": ..., ...meta }` wrapper extracts metadata and uses `tree`.
    #[test]
    fn parse_hierarchy_tree_wrapper_extracts_meta() {
        let json = r#"{
            "tree": { "element_type": "other", "text": null,
                      "accessibility_label": null, "placeholder": null,
                      "bounds": { "x": 0, "y": 0, "width": 4, "height": 8 },
                      "children": [] },
            "keyboard_height": 300,
            "safe_area_top": 47,
            "safe_area_bottom": 34,
            "safe_area_left": 5,
            "safe_area_right": 6
        }"#;
        let (_el, meta) = parse_hierarchy(json).expect("tree wrapper SHALL parse");
        assert_eq!(meta.keyboard_height, 300, "keyboard_height SHALL be read from wrapper");
        assert_eq!(meta.safe_area_top, 47, "safe_area_top SHALL be read");
        assert_eq!(meta.safe_area_bottom, 34, "safe_area_bottom SHALL be read");
        assert_eq!(meta.safe_area_left, 5, "safe_area_left SHALL be read");
        assert_eq!(meta.safe_area_right, 6, "safe_area_right SHALL be read");
    }

    // 5. Invalid JSON surfaces a contextual parse error.
    #[test]
    fn parse_hierarchy_invalid_json_errors() {
        let err = parse_hierarchy("not json").expect_err("garbage SHALL error");
        assert!(
            err.to_string().contains("failed to parse hierarchy JSON"),
            "error SHALL carry parse context, got: {err}"
        );
    }

    // 6. Valid JSON that is not a valid Element (missing bounds) errors at deserialize.
    #[test]
    fn parse_hierarchy_wrong_shape_errors() {
        let err = parse_hierarchy(r#"{"element_type": "x"}"#)
            .expect_err("missing bounds SHALL fail deserialize");
        assert!(
            err.to_string().contains("failed to deserialize hierarchy into Element"),
            "error SHALL carry deserialize context, got: {err}"
        );
    }

    // 7. Android `class` + bounds get normalized through parse_hierarchy end to end.
    #[test]
    fn parse_hierarchy_normalizes_android_node() {
        let json = r#"{
            "class": "android.widget.Button",
            "text": "Tap",
            "bounds": { "left": 10, "top": 20, "right": 60, "bottom": 120 },
            "children": []
        }"#;
        let (el, _meta) = parse_hierarchy(json).expect("android node SHALL parse");
        assert_eq!(el.element_type, "Button", "class SHALL simplify to last segment");
        assert_eq!(el.bounds.x, 10, "left SHALL map to x");
        assert_eq!(el.bounds.width, 50, "right-left SHALL be width");
        assert_eq!(el.bounds.height, 100, "bottom-top SHALL be height");
    }

    // ---- normalize_json ----

    // 8. `id` is renamed to accessibility_label when none present.
    #[test]
    fn normalize_renames_id_to_accessibility_label() {
        let mut v = json!({ "id": "save_btn" });
        normalize_json(&mut v);
        assert_eq!(v["accessibility_label"], "save_btn", "id SHALL become accessibility_label");
        assert!(v.get("id").is_none(), "raw id SHALL be removed");
    }

    // 9. `id` rename is skipped when accessibility_label already present.
    #[test]
    fn normalize_keeps_existing_accessibility_label_over_id() {
        let mut v = json!({ "id": "a", "accessibility_label": "b" });
        normalize_json(&mut v);
        assert_eq!(v["accessibility_label"], "b", "existing label SHALL win over id");
    }

    // 10. Android `class` simplifies to the final dotted segment.
    #[test]
    fn normalize_simplifies_android_class() {
        let mut v = json!({ "class": "android.widget.EditText" });
        normalize_json(&mut v);
        assert_eq!(v["element_type"], "EditText", "class SHALL simplify to last segment");
    }

    // 11. Non-empty contentDescription fills an absent accessibility_label.
    #[test]
    fn normalize_uses_content_description_for_label() {
        let mut v = json!({ "contentDescription": "Close" });
        normalize_json(&mut v);
        assert_eq!(v["accessibility_label"], "Close", "contentDescription SHALL fill label");
    }

    // 12. Empty contentDescription does not set a label.
    #[test]
    fn normalize_ignores_empty_content_description() {
        let mut v = json!({ "contentDescription": "" });
        normalize_json(&mut v);
        assert!(
            v.get("accessibility_label").is_none(),
            "empty contentDescription SHALL NOT set a label"
        );
    }

    // 13. Switch with value "1" is normalized to checked = true.
    #[test]
    fn normalize_switch_value_one_sets_checked() {
        let mut v = json!({ "element_type": "Switch", "value": "1" });
        normalize_json(&mut v);
        assert_eq!(v["checked"], true, "switch value \"1\" SHALL set checked true");
    }

    // 14. Switch with value "true" (case-insensitive) sets checked.
    #[test]
    fn normalize_switch_value_true_sets_checked() {
        let mut v = json!({ "element_type": "checkbox", "value": "TRUE" });
        normalize_json(&mut v);
        assert_eq!(v["checked"], true, "value \"TRUE\" SHALL set checked true");
    }

    // 15. Switch with value "0" leaves checked unset (does not force false).
    #[test]
    fn normalize_switch_value_zero_does_not_set_checked() {
        let mut v = json!({ "element_type": "toggle", "value": "0" });
        normalize_json(&mut v);
        assert!(v.get("checked").is_none(), "value \"0\" SHALL NOT insert checked");
    }

    // 16. Input element prefers `value` over placeholder/label for text.
    #[test]
    fn normalize_input_prefers_value_for_text() {
        let mut v = json!({
            "element_type": "text_field", "value": "typed",
            "placeholder": "hint", "label": "field"
        });
        normalize_json(&mut v);
        assert_eq!(v["text"], "typed", "input SHALL surface typed value as text");
    }

    // 17. Empty-value input falls back to placeholder.
    #[test]
    fn normalize_empty_input_falls_back_to_placeholder() {
        let mut v = json!({
            "element_type": "text_field", "value": "",
            "placeholder": "Enter name", "label": "field"
        });
        normalize_json(&mut v);
        assert_eq!(v["text"], "Enter name", "empty input SHALL fall back to placeholder");
    }

    // 18. Non-input prefers placeholder → label over the value field.
    #[test]
    fn normalize_non_input_ignores_value_for_text() {
        let mut v = json!({ "element_type": "other", "value": "internal", "label": "Visible" });
        normalize_json(&mut v);
        assert_eq!(v["text"], "Visible", "non-input SHALL prefer label over value");
    }

    // 19. With nothing else, existing `text` is preserved.
    #[test]
    fn normalize_preserves_text_when_no_overrides() {
        let mut v = json!({ "element_type": "other", "text": "keep" });
        normalize_json(&mut v);
        assert_eq!(v["text"], "keep", "existing text SHALL be preserved");
    }

    // 20. normalize_json recurses into children.
    #[test]
    fn normalize_recurses_into_children() {
        let mut v = json!({
            "element_type": "other",
            "children": [ { "class": "android.widget.TextView", "id": "child" } ]
        });
        normalize_json(&mut v);
        assert_eq!(v["children"][0]["element_type"], "TextView", "child class SHALL normalize");
        assert_eq!(v["children"][0]["accessibility_label"], "child", "child id SHALL normalize");
    }

    // ---- normalize_android_rect ----

    // 21. left/top/right/bottom convert to x/y/width/height.
    #[test]
    fn android_rect_converts_edges_to_xywh() {
        let mut rect = serde_json::Map::new();
        rect.insert("left".into(), json!(10));
        rect.insert("top".into(), json!(20));
        rect.insert("right".into(), json!(40));
        rect.insert("bottom".into(), json!(60));
        normalize_android_rect(&mut rect);
        assert_eq!(rect["x"], 10, "x SHALL equal left");
        assert_eq!(rect["y"], 20, "y SHALL equal top");
        assert_eq!(rect["width"], 30, "width SHALL equal right-left");
        assert_eq!(rect["height"], 40, "height SHALL equal bottom-top");
    }

    // 22. Inverted edges clamp width/height to 0 (off-screen WebView clip).
    #[test]
    fn android_rect_clamps_negative_dims_to_zero() {
        let mut rect = serde_json::Map::new();
        rect.insert("left".into(), json!(100));
        rect.insert("top".into(), json!(100));
        rect.insert("right".into(), json!(50));
        rect.insert("bottom".into(), json!(50));
        normalize_android_rect(&mut rect);
        assert_eq!(rect["width"], 0, "negative width SHALL clamp to 0");
        assert_eq!(rect["height"], 0, "negative height SHALL clamp to 0");
    }

    // 23. Rect already in x/y form is left untouched.
    #[test]
    fn android_rect_skips_when_x_already_present() {
        let mut rect = serde_json::Map::new();
        rect.insert("x".into(), json!(7));
        rect.insert("left".into(), json!(99));
        normalize_android_rect(&mut rect);
        assert_eq!(rect["x"], 7, "existing x SHALL be untouched");
        assert!(rect.get("width").is_none(), "no conversion SHALL run when x present");
    }

    // ---- promote_label_to_id ----

    // 24. label promotes to accessibility_label when label slot is null.
    #[test]
    fn promote_label_fills_null_label() {
        let mut map = serde_json::Map::new();
        map.insert("label".into(), json!("Submit"));
        map.insert("accessibility_label".into(), serde_json::Value::Null);
        promote_label_to_id(&mut map);
        assert_eq!(map["accessibility_label"], "Submit", "null label SHALL be filled from label");
    }

    // 25. label does not overwrite a non-empty accessibility_label.
    #[test]
    fn promote_label_keeps_existing_nonempty() {
        let mut map = serde_json::Map::new();
        map.insert("label".into(), json!("ignored"));
        map.insert("accessibility_label".into(), json!("kept"));
        promote_label_to_id(&mut map);
        assert_eq!(map["accessibility_label"], "kept", "non-empty label SHALL NOT be overwritten");
    }

    // ---- build_* request bodies ----

    // 26. Tap body serializes x/y.
    #[test]
    fn build_tap_body_serializes_coords() {
        let body = build_tap_body(3, 7).expect("tap body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["x"], 3, "x SHALL serialize");
        assert_eq!(v["y"], 7, "y SHALL serialize");
    }

    // 27. Type body serializes text.
    #[test]
    fn build_type_body_serializes_text() {
        let body = build_type_body("héllo").expect("type body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["text"], "héllo", "text SHALL serialize verbatim");
    }

    // 28. Backspace body serializes count.
    #[test]
    fn build_backspace_body_serializes_count() {
        let body = build_backspace_body(5).expect("backspace body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["count"], 5, "count SHALL serialize");
    }

    // 29. Long-press body serializes coords and duration.
    #[test]
    fn build_long_press_body_serializes_fields() {
        let body = build_long_press_body(1, 2, 800).expect("long press body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["x"], 1, "x SHALL serialize");
        assert_eq!(v["y"], 2, "y SHALL serialize");
        assert_eq!(v["duration_ms"], 800, "duration_ms SHALL serialize");
    }

    // 30. Swipe body serializes all five fields under their own names, so a
    //     from_y/to_x (or any) field swap in the wire contract is caught.
    //     Distinct argument values per field make a transposition observable.
    #[test]
    fn build_swipe_body_serializes_fields() {
        let body = build_swipe_body(1, 2, 3, 4, 250).expect("swipe body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["from_x"], 1, "from_x SHALL serialize");
        assert_eq!(v["from_y"], 2, "from_y SHALL serialize");
        assert_eq!(v["to_x"], 3, "to_x SHALL serialize");
        assert_eq!(v["to_y"], 4, "to_y SHALL serialize");
        assert_eq!(v["duration_ms"], 250, "duration_ms SHALL serialize");
    }

    // 31. Gesture body serializes finger points as [x,y] pairs plus duration.
    #[test]
    fn build_gesture_body_serializes_fingers() {
        let fingers = vec![
            crate::GestureFinger { points: vec![(0, 0), (5, 5)], duration_ms: 300 },
            crate::GestureFinger { points: vec![(9, 9)], duration_ms: 100 },
        ];
        let body = build_gesture_body(&fingers).expect("gesture body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["fingers"][0]["points"][1], json!([5, 5]), "point SHALL be [x,y]");
        assert_eq!(v["fingers"][0]["duration_ms"], 300, "duration SHALL serialize");
        assert_eq!(v["fingers"][1]["points"][0], json!([9, 9]), "second finger SHALL serialize");
    }

    // 32. Empty gesture serializes an empty fingers array.
    #[test]
    fn build_gesture_body_empty_fingers() {
        let body = build_gesture_body(&[]).expect("empty gesture SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["fingers"], json!([]), "empty input SHALL yield empty array");
    }

    // ---- parse_cutouts_json ----

    // 33. None input yields an empty cutout vec.
    #[test]
    fn parse_cutouts_none_is_empty() {
        assert!(parse_cutouts_json(None).is_empty(), "None SHALL yield empty cutouts");
    }

    // 34. Valid cutouts parse; zero/negative-area entries are filtered out.
    #[test]
    fn parse_cutouts_filters_zero_area() {
        let v = json!([
            { "x": 10, "y": 0, "width": 100, "height": 30 },
            { "x": 0, "y": 0, "width": 0, "height": 30 }
        ]);
        let cutouts = parse_cutouts_json(Some(&v));
        assert_eq!(cutouts.len(), 1, "zero-width cutout SHALL be filtered");
        assert_eq!(cutouts[0].width, 100, "valid cutout SHALL retain width");
    }

    // 35. Entries missing a required field are skipped.
    #[test]
    fn parse_cutouts_skips_missing_fields() {
        let v = json!([ { "x": 1, "y": 2, "width": 3 } ]);
        assert!(parse_cutouts_json(Some(&v)).is_empty(), "missing height SHALL skip entry");
    }

    // ---- parse_corners_json ----

    // 36. Each corner position string maps to its enum variant.
    #[test]
    fn parse_corners_maps_positions() {
        let v = json!([
            { "position": "top_left", "radius": 5, "center_x": 5, "center_y": 5 },
            { "position": "top_right", "radius": 5, "center_x": 1, "center_y": 5 },
            { "position": "bottom_right", "radius": 5, "center_x": 1, "center_y": 1 },
            { "position": "bottom_left", "radius": 5, "center_x": 5, "center_y": 1 }
        ]);
        let corners = parse_corners_json(Some(&v));
        assert_eq!(corners.len(), 4, "all four corners SHALL parse");
        assert_eq!(corners[0].position, CornerPosition::TopLeft, "first SHALL be TopLeft");
        assert_eq!(corners[3].position, CornerPosition::BottomLeft, "last SHALL be BottomLeft");
    }

    // 37. Unknown position string skips the entry.
    #[test]
    fn parse_corners_skips_unknown_position() {
        let v = json!([ { "position": "middle", "radius": 5, "center_x": 1, "center_y": 1 } ]);
        assert!(parse_corners_json(Some(&v)).is_empty(), "unknown position SHALL be skipped");
    }

    // 38. None input yields empty corners.
    #[test]
    fn parse_corners_none_is_empty() {
        assert!(parse_corners_json(None).is_empty(), "None SHALL yield empty corners");
    }

    // ---- element-tree helpers (detect_anr / find_alert / buttons) ----
    // These build Elements via JSON since Element has no Default.

    fn el(json: serde_json::Value) -> Element {
        serde_json::from_value(json).expect("test element SHALL deserialize")
    }

    fn leaf(element_type: &str, text: Option<&str>) -> serde_json::Value {
        json!({
            "element_type": element_type,
            "text": text,
            "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 1, "height": 1 },
            "children": []
        })
    }

    // 39. detect_anr matches the straight-apostrophe title.
    #[test]
    fn detect_anr_matches_straight_apostrophe() {
        let tree = el(json!({
            "element_type": "FrameLayout", "text": null,
            "accessibility_label": null, "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 1, "height": 1 },
            "children": [ leaf("TextView", Some("App isn't responding")) ]
        }));
        assert!(detect_anr(&tree), "straight-apostrophe ANR title SHALL match");
    }

    // 40. detect_anr matches the curly-apostrophe (Unicode) title.
    #[test]
    fn detect_anr_matches_curly_apostrophe() {
        let tree = el(leaf("TextView", Some("App isn\u{2019}t responding")));
        assert!(detect_anr(&tree), "curly-apostrophe ANR title SHALL match");
    }

    // 41. detect_anr is false for unrelated text.
    #[test]
    fn detect_anr_false_for_normal_ui() {
        let tree = el(leaf("TextView", Some("Welcome")));
        assert!(!detect_anr(&tree), "non-ANR text SHALL NOT match");
    }

    // 42. find_alert returns an iOS native alert and extracts message as text.
    #[test]
    fn find_alert_ios_extracts_message() {
        let tree = el(json!({
            "element_type": "alert", "text": "Title", "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 10, "height": 10 },
            "children": [
                leaf("StaticText", Some("Alert Title")),
                leaf("StaticText", Some("This is the body")),
                leaf("Button", Some("OK"))
            ]
        }));
        let alert = find_alert(&tree).expect("alert SHALL be found");
        assert_eq!(alert.element_type, "alert", "found element SHALL be the alert");
        assert_eq!(
            alert.text.as_deref(), Some("This is the body"),
            "alert text SHALL be the message (second non-button text)"
        );
    }

    // 43. find_alert detects the Android FrameLayout-with-Button dialog pattern.
    #[test]
    fn find_alert_android_dialog_pattern() {
        let tree = el(json!({
            "element_type": "FrameLayout", "text": null, "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 100, "width": 200, "height": 200 },
            "children": [
                leaf("TextView", Some("Permission")),
                leaf("Button", Some("Allow"))
            ]
        }));
        let alert = find_alert(&tree).expect("android dialog SHALL be found");
        assert_eq!(alert.element_type, "FrameLayout", "android alert SHALL be the frame");
    }

    // 44. Android FrameLayout at y == 0 is NOT treated as an alert.
    #[test]
    fn find_alert_skips_top_anchored_frame() {
        let tree = el(json!({
            "element_type": "FrameLayout", "text": null, "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 200, "height": 200 },
            "children": [ leaf("Button", Some("X")) ]
        }));
        assert!(find_alert(&tree).is_none(), "y==0 frame SHALL NOT be an alert");
    }

    // 45. find_alert returns None when no alert is present.
    #[test]
    fn find_alert_none_when_absent() {
        let tree = el(leaf("other", Some("content")));
        assert!(find_alert(&tree).is_none(), "tree without alert SHALL yield None");
    }

    // 46. find_alert_buttons collects all buttons recursively, case-insensitively.
    #[test]
    fn find_alert_buttons_collects_all() {
        let alert = el(json!({
            "element_type": "alert", "text": "t", "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 10, "height": 10 },
            "children": [
                leaf("Button", Some("Cancel")),
                json!({
                    "element_type": "other", "text": null, "accessibility_label": null,
                    "placeholder": null,
                    "bounds": { "x": 0, "y": 0, "width": 1, "height": 1 },
                    "children": [ leaf("button", Some("OK")) ]
                })
            ]
        }));
        let buttons = find_alert_buttons(&alert);
        assert_eq!(buttons.len(), 2, "both buttons SHALL be collected across depths");
    }

    // 47. extract_alert_message returns the single text when only one non-button text exists.
    #[test]
    fn find_alert_single_text_becomes_message() {
        let tree = el(json!({
            "element_type": "alert", "text": "ignored-root", "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 10, "height": 10 },
            "children": [ leaf("StaticText", Some("Only line")) ]
        }));
        let alert = find_alert(&tree).expect("alert SHALL be found");
        assert_eq!(
            alert.text.as_deref(), Some("Only line"),
            "single non-button text SHALL be used as message"
        );
    }

    // ---- find_webview_bounds ----

    // 48. Android WebView bounds read from {left,top}.
    #[test]
    fn find_webview_bounds_android() {
        let v = json!({
            "class": "android.webkit.WebView",
            "bounds": { "left": 5, "top": 15, "right": 100, "bottom": 200 }
        });
        assert_eq!(find_webview_bounds(&v), Some((5, 15)), "android webview SHALL read left/top");
    }

    // 49. iOS web_view bounds read from {x,y}; array root is traversed.
    #[test]
    fn find_webview_bounds_ios_array_root() {
        let v = json!([
            { "element_type": "window", "children": [] },
            { "element_type": "web_view", "bounds": { "x": 7, "y": 8 } }
        ]);
        assert_eq!(find_webview_bounds(&v), Some((7, 8)), "ios webview SHALL read x/y from array");
    }

    // 50. find_webview_bounds recurses into children.
    #[test]
    fn find_webview_bounds_nested_child() {
        let v = json!({
            "element_type": "root",
            "children": [
                { "element_type": "web_view", "bounds": { "x": 1, "y": 2 } }
            ]
        });
        assert_eq!(find_webview_bounds(&v), Some((1, 2)), "nested webview SHALL be found");
    }

    // 51. No webview yields None.
    #[test]
    fn find_webview_bounds_none() {
        let v = json!({ "element_type": "other", "children": [] });
        assert_eq!(find_webview_bounds(&v), None, "no webview SHALL yield None");
    }

    // ---- replace_webview_children ----

    // 52. Replacing webview children swaps them for the DOM payload and returns true.
    #[test]
    fn replace_webview_children_swaps_dom() {
        let mut v = json!({
            "element_type": "web_view",
            "children": [ { "element_type": "stale" } ]
        });
        let dom = json!({ "element_type": "dom_root" });
        let replaced = replace_webview_children(&mut v, dom);
        assert!(replaced, "replacement SHALL report success");
        assert_eq!(v["children"].as_array().map(|a| a.len()), Some(1), "children SHALL be exactly the DOM");
        assert_eq!(v["children"][0]["element_type"], "dom_root", "children SHALL be the DOM node");
    }

    // 53. No webview present returns false and leaves the tree unchanged.
    #[test]
    fn replace_webview_children_no_webview() {
        let mut v = json!({ "element_type": "other", "children": [] });
        let replaced = replace_webview_children(&mut v, json!({}));
        assert!(!replaced, "no webview SHALL report failure");
    }

    // 54. Array root is traversed for replacement.
    #[test]
    fn replace_webview_children_array_root() {
        let mut v = json!([
            { "element_type": "window", "children": [] },
            { "class": "android.webkit.WebView", "children": [ {} ] }
        ]);
        let replaced = replace_webview_children(&mut v, json!({ "element_type": "dom" }));
        assert!(replaced, "array-root webview SHALL be replaced");
        assert_eq!(v[1]["children"][0]["element_type"], "dom", "DOM SHALL replace android webview kids");
    }

    // ---- CompanionClient URL / timeout (non-network) ----

    // 55. url() with no default query produces base_url + path.
    #[test]
    fn companion_url_without_query() {
        let c = CompanionClient::new(1234);
        assert_eq!(c.url("/tap"), "http://localhost:1234/tap", "bare path SHALL append to base");
    }

    // 56. url() appends default query with `?` when path has none.
    #[test]
    fn companion_url_appends_query_with_question_mark() {
        let c = CompanionClient::new(1234);
        c.set_default_query("bundle_id=fail.golem.test");
        assert_eq!(
            c.url("/hierarchy"),
            "http://localhost:1234/hierarchy?bundle_id=fail.golem.test",
            "query SHALL be joined with ?"
        );
    }

    // 57. url() uses `&` when the path already contains a query string.
    #[test]
    fn companion_url_appends_query_with_ampersand() {
        let c = CompanionClient::new(1234);
        c.set_default_query("a=b");
        assert_eq!(
            c.url("/x?foo=1"),
            "http://localhost:1234/x?foo=1&a=b",
            "existing query SHALL be extended with &"
        );
    }

    // 58. Request timeout round-trips; ZERO clears it.
    #[test]
    fn companion_request_timeout_roundtrip() {
        let c = CompanionClient::new(1);
        assert!(c.current_request_timeout().is_none(), "default SHALL be no timeout");
        c.set_request_timeout(std::time::Duration::from_millis(250));
        assert_eq!(
            c.current_request_timeout(),
            Some(std::time::Duration::from_millis(250)),
            "set timeout SHALL round-trip"
        );
        c.set_request_timeout(std::time::Duration::ZERO);
        assert!(c.current_request_timeout().is_none(), "ZERO SHALL clear the timeout");
    }

    // 59. Sub-millisecond Durations truncate to 0ms (whole-ms precision),
    //     which clears the timeout rather than clamping up to 1ms.
    #[test]
    fn companion_request_timeout_sub_millisecond_truncates_to_zero() {
        let c = CompanionClient::new(1);
        c.set_request_timeout(std::time::Duration::from_millis(250));
        c.set_request_timeout(std::time::Duration::from_micros(500));
        assert!(
            c.current_request_timeout().is_none(),
            "sub-millisecond timeout SHALL truncate to 0ms and clear the timeout"
        );
    }
}
