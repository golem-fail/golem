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
    pub x: f64,
    pub y: f64,
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
    pub x: f64,
    pub y: f64,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SwipeRequest {
    pub from_x: f64,
    pub from_y: f64,
    pub to_x: f64,
    pub to_y: f64,
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
            promote_labels_json(item);
        }
        if arr.len() == 1 {
            val = arr.remove(0);
        } else {
            // Multiple roots — wrap in a synthetic container.
            let wrapped = serde_json::json!({
                "element_type": "other",
                "text": null,
                "id": null,
                "placeholder": null,
                "enabled": true,
                "checked": false,
                "clickable": false,
                "focused": false,
                "bounds": { "x": 0.0, "y": 0.0, "width": 0.0, "height": 0.0 },
                "children": arr
            });
            val = wrapped;
        }
    } else {
        promote_labels_json(&mut val);
    }

    serde_json::from_value(val).context("failed to deserialize hierarchy into Element")
}

/// Recursively promote `label` to `text` in a JSON value before deserializing
/// into `Element`. This handles companion servers that use `label` instead of `text`.
fn promote_labels_json(val: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = val {
        let label_str = map
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Promote label → id when id is absent/empty.
        // On iOS WKWebView, the HTML element's `id` attribute becomes the
        // accessibility label reported by XCUITest.
        let id_empty = match map.get("id") {
            Some(serde_json::Value::String(s)) => s.is_empty(),
            Some(serde_json::Value::Null) | None => true,
            _ => false,
        };
        if id_empty && !label_str.is_empty() {
            map.insert("id".to_string(), serde_json::Value::String(label_str.clone()));
        }

        // Promote label → text only when text is absent/empty AND the element
        // type is a leaf that carries visible text (StaticText, Button, etc.).
        // For container elements, the label is typically the accessibility ID,
        // not the visible content.
        let text_empty = match map.get("text") {
            Some(serde_json::Value::String(s)) => s.is_empty(),
            Some(serde_json::Value::Null) | None => true,
            _ => false,
        };
        if text_empty && !label_str.is_empty() {
            let etype = map
                .get("element_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let is_leaf = matches!(
                etype,
                "StaticText" | "Button" | "TextField" | "SecureTextField"
                    | "SearchField" | "TextView" | "Switch" | "Link"
            );
            if is_leaf {
                map.insert("text".to_string(), serde_json::Value::String(label_str));
            }
        }
        // Recurse into children
        if let Some(children) = map.get_mut("children") {
            if let serde_json::Value::Array(arr) = children {
                for child in arr {
                    promote_labels_json(child);
                }
            }
        }
    }
}

/// Build the JSON body for a tap request.
pub(crate) fn build_tap_body(x: f64, y: f64) -> Result<String> {
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
pub(crate) fn build_long_press_body(x: f64, y: f64, duration_ms: u64) -> Result<String> {
    serde_json::to_string(&LongPressRequest {
        x,
        y,
        duration_ms,
    })
    .context("failed to serialize long press request")
}

/// Build the JSON body for a swipe request.
pub(crate) fn build_swipe_body(
    from_x: f64,
    from_y: f64,
    to_x: f64,
    to_y: f64,
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
    pub client: reqwest::Client,
}

impl CompanionClient {
    pub fn new(port: u16) -> Self {
        Self {
            base_url: format!("http://localhost:{port}"),
            client: reqwest::Client::new(),
        }
    }

    pub async fn post_json(&self, path: &str, body: &str) -> Result<String> {
        let url = format!("{}{}", self.base_url, path);
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
        let url = format!("{}{}", self.base_url, path);
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
        let url = format!("{}{}", self.base_url, path);
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
