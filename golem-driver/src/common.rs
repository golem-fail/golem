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
pub(crate) fn parse_hierarchy(json: &str) -> Result<Element> {
    serde_json::from_str::<Element>(json).context("failed to parse hierarchy JSON")
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
