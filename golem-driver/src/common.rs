use anyhow::{Context, Result};
use golem_element::Element;
use serde::Serialize;

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
    if let Some(obj) = val.as_object() {
        if obj.contains_key("tree") {
            meta.keyboard_height = obj
                .get("keyboard_height")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
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

    let element = serde_json::from_value(val).context("failed to deserialize hierarchy into Element")?;
    Ok((element, meta))
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

        // Android: convert bounds from {left,top,right,bottom} → {x,y,width,height}
        if let Some(serde_json::Value::Object(bounds)) = map.get_mut("bounds") {
            if bounds.contains_key("left") && !bounds.contains_key("x") {
                let left = bounds.get("left").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let top = bounds.get("top").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let right = bounds.get("right").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let bottom = bounds.get("bottom").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                bounds.insert("x".to_string(), serde_json::json!(left));
                bounds.insert("y".to_string(), serde_json::json!(top));
                // Clamp to 0 — Android WebView clips bottom/right to the visible
                // area, causing negative dimensions for off-screen elements.
                bounds.insert("width".to_string(), serde_json::json!((right - left).max(0)));
                bounds.insert("height".to_string(), serde_json::json!((bottom - top).max(0)));
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
    if el.element_type == "FrameLayout" && el.bounds.y > 0 {
        if has_button_descendant(el) {
            let mut alert = el.clone();
            alert.text = extract_alert_message(&alert);
            return Some(alert);
        }
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
    el.children.iter().any(|c| has_button_descendant(c))
}

// ---------------------------------------------------------------------------
// HTTP client wrapper shared by both platform drivers
// ---------------------------------------------------------------------------

/// Thin HTTP client wrapper used by both `AndroidDriver` and `IosDriver`.
pub struct CompanionClient {
    pub base_url: String,
    /// Default query string appended to every request (e.g. `"?bundle_id=fail.golem.test"`).
    pub default_query: String,
    pub client: reqwest::Client,
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
            default_query: String::new(),
            client: reqwest::Client::new(),
        }
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

        let text = resp.text().await.context("reading health response")?;
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
