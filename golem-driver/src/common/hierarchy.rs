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
            device_model = obj
                .get("device_model")
                .and_then(|v| v.as_str())
                .map(String::from);
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

    let mut element: Element =
        serde_json::from_value(val).context("failed to deserialize hierarchy into Element")?;
    // Native occlusion: hit-test tap targets against the tree's paint order so
    // `tap_point()` routes around occluders the same way it does for webview
    // (where hit_points arrive pre-computed from the DOM). No-op for nodes that
    // already carry hit_points (webview) and for trees with no native tap
    // targets. Heuristic — see `compute_native_hit_points`.
    element.compute_native_hit_points();
    meta.node_count = count_nodes(&element);

    // iOS: look up display data from device model using screen dimensions from the parsed tree
    if let Some(model) = device_model {
        if let Some(display) =
            ios_display::lookup(&model, element.bounds.width, element.bounds.height)
        {
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
pub(crate) fn normalize_json(val: &mut serde_json::Value) {
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
                map.insert(
                    "element_type".to_string(),
                    serde_json::Value::String(simplified),
                );
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
                    map.insert(
                        "accessibility_label".to_string(),
                        serde_json::Value::String(cd.to_string()),
                    );
                }
            }
        }

        // Fix checked state for switches/toggles: iOS reports state via value "0"/"1"
        // rather than isSelected. Normalize to checked = true/false.
        let is_switch = map
            .get("element_type")
            .and_then(|v| v.as_str())
            .is_some_and(|et| {
                let lower = et.to_lowercase();
                lower == "switch" || lower == "toggle" || lower == "checkbox"
            });
        if is_switch {
            let value_is_on = map
                .get("value")
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
            Some(
                "text_field"
                    | "secure_text_field"
                    | "search_field"
                    | "text_view"
                    | "EditText"
                    | "AutoCompleteTextView"
            )
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
pub(crate) fn normalize_android_rect(rect: &mut serde_json::Map<String, serde_json::Value>) {
    if rect.contains_key("left") && !rect.contains_key("x") {
        let left = rect.get("left").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let top = rect.get("top").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let right = rect.get("right").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let bottom = rect.get("bottom").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        rect.insert("x".to_string(), serde_json::json!(left));
        rect.insert("y".to_string(), serde_json::json!(top));
        rect.insert(
            "width".to_string(),
            serde_json::json!((right - left).max(0)),
        );
        rect.insert(
            "height".to_string(),
            serde_json::json!((bottom - top).max(0)),
        );
    }
}

/// Promote `label` (aria-label) to `accessibility_label` when id is absent/empty.
pub(crate) fn promote_label_to_id(map: &mut serde_json::Map<String, serde_json::Value>) {
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
        map.insert(
            "accessibility_label".to_string(),
            serde_json::Value::String(label_str),
        );
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

/// Interpret a /type or /backspace companion response as the
/// post-mutation check. The companion sets `"text_unchanged": true` when
/// its single post-dispatch read saw no change (slow IME). Returns
/// `Some(true)` in that case, `Some(false)` when the companion responded
/// without the flag (change observed / field unreadable). Always
/// `Some(_)` — a companion that ran the check answered one way or the
/// other; malformed/absent JSON degrades to `Some(false)` (no extend).
pub(crate) fn parse_text_unchanged(resp: &str) -> Option<bool> {
    Some(
        serde_json::from_str::<serde_json::Value>(resp)
            .ok()
            .and_then(|v| v.get("text_unchanged").and_then(serde_json::Value::as_bool))
            .unwrap_or(false),
    )
}

/// Build the JSON body for a long-press request.
pub(crate) fn build_long_press_body(x: i32, y: i32, duration_ms: u64) -> Result<String> {
    serde_json::to_string(&LongPressRequest { x, y, duration_ms })
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
pub(crate) fn parse_cutouts_json(val: Option<&serde_json::Value>) -> Vec<CutoutRect> {
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
                Some(CutoutRect {
                    x,
                    y,
                    width: w,
                    height: h,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Parse rounded corners from a JSON array:
/// `[{"position":"top_left","radius":N,"center_x":N,"center_y":N}, ...]`
pub(crate) fn parse_corners_json(val: Option<&serde_json::Value>) -> Vec<RoundedCorner> {
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
            Some(RoundedCorner {
                position,
                radius,
                center_x,
                center_y,
            })
        })
        .collect()
}
