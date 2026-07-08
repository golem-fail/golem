use anyhow::{Context, Result};

// ---------------------------------------------------------------------------
// HTTP client wrapper shared by both platform drivers
// ---------------------------------------------------------------------------

/// HTTP status the companion returns when its own main-thread watchdog
/// (`runOnMain` deadline) fires — alive but wedged. Mapped to
/// `FailureCode::DeviceCompanionWedged` (D503).
const COMPANION_WATCHDOG_HTTP_STATUS: u16 = 504;

/// Per-request timeout, in seconds, for the companion `/health` check.
const HEALTH_CHECK_TIMEOUT_SECS: u64 = 5;

/// Poll interval, in seconds, used by `wait_for_health` while waiting for
/// the companion to come up.
const HEALTH_POLL_INTERVAL_SECS: u64 = 2;

/// Tag a companion request transport error with a failure code where the kind
/// is unambiguous, so it stops rendering as an opaque `EX000` (`Uncoded`).
/// `ctx` is preserved as the human message. Only cleanly-attributable kinds
/// are coded; anything else stays uncoded on purpose.
///
/// - connect failure (connection refused) → `DeviceCompanionUnreachable`
///   (D505): the companion process is gone / not accepting — death, or a
///   cold-start drop before the socket is up.
/// - client-side timeout (request exceeded its per-request budget) →
///   `DeviceCompanionWedged` (D503): the companion is up but didn't answer in
///   time — a stall.
fn coded_transport_err(e: reqwest::Error, ctx: String) -> anyhow::Error {
    let code = if e.is_connect() {
        Some(golem_events::FailureCode::DeviceCompanionUnreachable)
    } else if e.is_timeout() {
        Some(golem_events::FailureCode::DeviceCompanionWedged)
    } else {
        None
    };
    let err = anyhow::Error::new(e).context(ctx);
    match code {
        Some(c) => golem_events::coded(c, err),
        None => err,
    }
}

/// Tag a non-success companion HTTP status where the meaning is unambiguous.
/// A `504` is the companion's own main-thread watchdog (`runOnMain` deadline)
/// firing — alive but wedged → `DeviceCompanionWedged` (D503). Other statuses
/// stay uncoded.
pub(crate) fn coded_status_err(status: reqwest::StatusCode, msg: String) -> anyhow::Error {
    if status.as_u16() == COMPANION_WATCHDOG_HTTP_STATUS {
        golem_events::coded(
            golem_events::FailureCode::DeviceCompanionWedged,
            anyhow::anyhow!(msg),
        )
    } else {
        anyhow::anyhow!(msg)
    }
}

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
    /// AVC-encoder max width/height the device reports (Android only, via
    /// MediaCodec `VideoCapabilities`). `None` when unknown/unreported (older
    /// companion, iOS, or query failed) — callers fall back to a heuristic cap.
    pub max_recording_width: Option<u32>,
    pub max_recording_height: Option<u32>,
}

impl CompanionClient {
    /// Create a client for the companion server forwarded to `localhost:port`
    /// (`adb forward` on Android, a direct simulator port on iOS). No I/O
    /// happens here — the base URL is just recorded for later requests.
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

    pub(crate) fn current_request_timeout(&self) -> Option<std::time::Duration> {
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
            .timeout(std::time::Duration::from_secs(HEALTH_CHECK_TIMEOUT_SECS))
            .send()
            .await
            .with_context(|| {
                format!(
                    "Companion server not reachable at {}. Is it running?",
                    self.base_url
                )
            })?;

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
            device_name: json["device_name"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            os_version: json["os_version"].as_str().unwrap_or("unknown").to_string(),
            device_id: json["device_id"].as_str().unwrap_or("unknown").to_string(),
            // 0 / missing → None (unknown); positive → the reported ceiling.
            max_recording_width: json["max_recording_width"]
                .as_u64()
                .filter(|&v| v > 0)
                .map(|v| v as u32),
            max_recording_height: json["max_recording_height"]
                .as_u64()
                .filter(|&v| v > 0)
                .map(|v| v as u32),
        })
    }

    /// Poll `/health` until the companion responds or timeout expires.
    ///
    /// Polls every 2 seconds. Returns the health info on success.
    pub async fn wait_for_health(&self, timeout: std::time::Duration) -> Result<CompanionHealth> {
        let deadline = tokio::time::Instant::now() + timeout;
        let poll_interval = std::time::Duration::from_secs(HEALTH_POLL_INTERVAL_SECS);

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
    pub(crate) fn url(&self, path: &str) -> String {
        let dq = self
            .default_query
            .read()
            .map(|q| q.clone())
            .unwrap_or_default();
        let sep = if path.contains('?') { "&" } else { "?" };
        if dq.is_empty() {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}{}{}{}", self.base_url, path, sep, dq)
        }
    }

    /// `POST` a JSON body to `path` on the companion (the default query
    /// string and any configured per-request timeout are applied
    /// automatically) and return the response body as text. A non-2xx
    /// status becomes an `Err` carrying the status and body; a 504 is
    /// tagged as `FailureCode::DeviceCompanionWedged` so callers can tell a
    /// wedged companion apart from an ordinary request failure.
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
            .map_err(|e| coded_transport_err(e, format!("POST {url} failed")))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .with_context(|| format!("reading response body from POST {url}"))?;

        if !status.is_success() {
            return Err(coded_status_err(
                status,
                format!("POST {url} returned {status}: {text}"),
            ));
        }

        Ok(text)
    }

    /// `GET` `path` on the companion and return the response body as text.
    /// Same query-string, timeout, and error-mapping behaviour as
    /// [`post_json`](Self::post_json).
    pub async fn get_text(&self, path: &str) -> Result<String> {
        let url = self.url(path);
        let mut req = self.client.get(&url);
        if let Some(t) = self.current_request_timeout() {
            req = req.timeout(t);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| coded_transport_err(e, format!("GET {url} failed")))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .with_context(|| format!("reading response body from GET {url}"))?;

        if !status.is_success() {
            return Err(coded_status_err(
                status,
                format!("GET {url} returned {status}: {text}"),
            ));
        }

        Ok(text)
    }

    /// `GET` `path` on the companion and return the raw response bytes
    /// (used for screenshot/recording payloads). Same query-string,
    /// timeout, and error-mapping behaviour as
    /// [`post_json`](Self::post_json).
    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let url = self.url(path);
        let mut req = self.client.get(&url);
        if let Some(t) = self.current_request_timeout() {
            req = req.timeout(t);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| coded_transport_err(e, format!("GET {url} failed")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable>".to_string());
            return Err(coded_status_err(
                status,
                format!("GET {url} returned {status}: {text}"),
            ));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .with_context(|| format!("reading bytes from GET {url}"))
    }
}
