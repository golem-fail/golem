use anyhow::{Context, Result};
use golem_element::Element;
use serde::Serialize;
#[cfg(test)]
use serde::Deserialize;

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

#[derive(Debug, Serialize)]
pub(crate) struct AlertRequest<'a> {
    pub action: &'a str,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
pub(crate) struct AlertResponse {
    pub alert: Option<Element>,
}

// ---------------------------------------------------------------------------
// Parsing helpers (testable without HTTP)
// ---------------------------------------------------------------------------

/// Parse a hierarchy JSON response body into an `Element` tree.
///
/// The companion server may return either a single root element or an array
/// of root elements. When an array is returned, wrap them under a synthetic
/// root element so callers always see a single tree.
///
/// After parsing, promotes `label` → `text` for elements where `text` is absent,
/// to normalise across different companion server implementations.
pub(crate) fn parse_hierarchy(json: &str) -> Result<Element> {
    // Parse as generic JSON first so we can normalise label → text.
    let mut val: serde_json::Value =
        serde_json::from_str(json).context("failed to parse hierarchy JSON")?;

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
                "accessibility_id": null,
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

    serde_json::from_value(val).context("failed to deserialize hierarchy into Element")
}

/// Recursively normalize a JSON hierarchy node to match the `Element` schema.
///
/// Handles two companion server formats:
/// - **iOS (mobile-bench):** `label` field instead of `text`/`id`
/// - **Android:** `class` instead of `element_type`, `contentDescription` instead of `id`,
///   `bounds` with `left/top/right/bottom` instead of `x/y/width/height`
fn normalize_json(val: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = val {
        // Rename raw `id` → `accessibility_id` (GOLEM companion format)
        if map.contains_key("id") && !map.contains_key("accessibility_id") {
            if let Some(v) = map.remove("id") {
                map.insert("accessibility_id".to_string(), v);
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
                    .get("accessibility_id")
                    .and_then(|v| v.as_str())
                    .is_none_or(|s| s.is_empty());
                if id_empty {
                    map.insert("accessibility_id".to_string(), serde_json::Value::String(cd.to_string()));
                }
            }
        }

        // Android: convert bounds from {left,top,right,bottom} → {x,y,width,height}
        if let Some(serde_json::Value::Object(bounds)) = map.get_mut("bounds") {
            if bounds.contains_key("left") && !bounds.contains_key("x") {
                let left = bounds.get("left").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let top = bounds.get("top").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let right = bounds.get("right").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let bottom = bounds.get("bottom").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                bounds.insert("x".to_string(), serde_json::json!(left));
                bounds.insert("y".to_string(), serde_json::json!(top));
                bounds.insert("width".to_string(), serde_json::json!(right - left));
                bounds.insert("height".to_string(), serde_json::json!(bottom - top));
            }
        }

        // iOS: promote label → id and label → text (existing logic)
        promote_labels_json_inner(map);

        // Recurse into children
        if let Some(serde_json::Value::Array(arr)) = map.get_mut("children") {
            for child in arr {
                normalize_json(child);
            }
        }
    }
}

/// Promote `label` to `id` and `text` for iOS companion servers.
/// Called on a single node's map — recursion is handled by `normalize_json`.
fn promote_labels_json_inner(map: &mut serde_json::Map<String, serde_json::Value>) {
    let label_str = map
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Promote label → id when id is absent/empty.
    let id_empty = match map.get("accessibility_id") {
        Some(serde_json::Value::String(s)) => s.is_empty(),
        Some(serde_json::Value::Null) | None => true,
        _ => false,
    };
    if id_empty && !label_str.is_empty() {
        map.insert("accessibility_id".to_string(), serde_json::Value::String(label_str.clone()));
    }

    // Promote label → text always when text is absent/empty.
    // Per aria-label spec: "overrides any other native labeling mechanism."
    // This matches Android behavior where getText() returns aria-label.
    let text_empty = match map.get("text") {
        Some(serde_json::Value::String(s)) => s.is_empty(),
        Some(serde_json::Value::Null) | None => true,
        _ => false,
    };
    if text_empty && !label_str.is_empty() {
        map.insert("text".to_string(), serde_json::Value::String(label_str));
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

/// Build the JSON body for an alert action request.
pub(crate) fn build_alert_body(action: &str) -> Result<String> {
    serde_json::to_string(&AlertRequest { action }).context("failed to serialize alert request")
}

/// Parse an alert response body.
#[cfg(test)]
pub(crate) fn parse_alert_response(json: &str) -> Result<Option<Element>> {
    let resp: AlertResponse =
        serde_json::from_str(json).context("failed to parse alert response")?;
    Ok(resp.alert)
}

/// Walk an element tree looking for an alert-type element.
pub(crate) fn find_alert(el: &Element) -> Option<Element> {
    if el.element_type == "Alert" {
        return Some(el.clone());
    }
    for child in &el.children {
        if let Some(alert) = find_alert(child) {
            return Some(alert);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// HTTP client wrapper shared by both platform drivers
// ---------------------------------------------------------------------------

/// Thin HTTP client wrapper used by both `AndroidDriver` and `IosDriver`.
pub(crate) struct CompanionClient {
    pub base_url: String,
    /// Default query string appended to every request (e.g. `"?bundle_id=com.golem.test"`).
    pub default_query: String,
    pub client: reqwest::Client,
}

impl CompanionClient {
    pub fn new(port: u16) -> Self {
        Self {
            base_url: format!("http://localhost:{port}"),
            default_query: String::new(),
            client: reqwest::Client::new(),
        }
    }

    /// Build the full URL for a request, appending the default query string.
    fn url(&self, path: &str) -> String {
        let sep = if path.contains('?') { "&" } else { "?" };
        if self.default_query.is_empty() {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}{}{}{}", self.base_url, path, sep, self.default_query)
        }
    }

    pub async fn post_json(&self, path: &str, body: &str) -> Result<String> {
        let url = self.url(path);
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body.to_string())
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
        let resp = self
            .client
            .get(&url)
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
        let resp = self
            .client
            .get(&url)
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
