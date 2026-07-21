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

/// Tag a companion request transport error with a failure code (via
/// [`classify_transport`]) so it stops rendering as an opaque `EX000`. `ctx`
/// is preserved as the human message.
fn coded_transport_err(e: reqwest::Error, ctx: String) -> anyhow::Error {
    let code = classify_transport(e.is_timeout(), e.is_connect(), e.is_request(), e.is_body());
    let err = anyhow::Error::new(e).context(ctx);
    match code {
        Some(c) => golem_events::coded(c, err),
        None => err,
    }
}

/// Pure classifier for a reqwest transport failure, split out so it's
/// testable without constructing a `reqwest::Error` (which has no public
/// constructor).
///
/// - timeout → `DeviceCompanionWedged` (D503): alive but didn't answer in time.
/// - connect / request / body drop → `DeviceCompanionUnreachable` (D505): the
///   exchange couldn't complete. `is_connect` is a cold/gone socket; `is_request`
///   / `is_body` are a socket that dropped *mid-exchange* — the companion
///   process died while serving (e.g. "connection closed before message
///   completed" mid-`/hierarchy`), which previously fell through to an opaque
///   `EX000`.
/// - anything else (decode/builder) → uncoded on purpose.
fn classify_transport(
    is_timeout: bool,
    is_connect: bool,
    is_request: bool,
    is_body: bool,
) -> Option<golem_events::FailureCode> {
    if is_timeout {
        Some(golem_events::FailureCode::DeviceCompanionWedged)
    } else if is_connect || is_request || is_body {
        Some(golem_events::FailureCode::DeviceCompanionUnreachable)
    } else {
        None
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
    /// Adaptive `/hierarchy` pacing state: `(finished_at, last_latency)` of the
    /// most recent `/hierarchy` fetch. `/hierarchy` runs a snapshot on the
    /// companion's main thread; back-to-back fetches on a heavy tree pile up
    /// and can get the host killed. Level-1 prevention: before each fetch,
    /// hold off finish-to-start for up to the *previous* fetch's latency
    /// (capped) so a stressed companion — which answers slower — is given
    /// proportionally more breathing room, while a healthy one (fast answers)
    /// is barely delayed.
    hierarchy_pace: std::sync::Mutex<Option<(std::time::Instant, std::time::Duration)>>,
}

/// Cap on the adaptive `/hierarchy` finish-to-start gap. Bounds the slowdown so
/// pacing can relieve pressure without stalling a genuinely-slow-but-alive
/// companion indefinitely.
const HIERARCHY_PACE_CAP: std::time::Duration = std::time::Duration::from_millis(1000);

/// The one path that is adaptively paced (the heavy, high-frequency snapshot).
const HIERARCHY_PATH: &str = "/hierarchy";

/// Attempts for an idempotent GET read (1 retry). A mid-exchange drop is
/// sometimes a transient socket hiccup a fresh request clears; a genuinely
/// dead companion fails again fast (the D505 still surfaces). GET only —
/// mutations (`post_json`) are never retried.
const GET_MAX_ATTEMPTS: u32 = 2;

/// Backoff before the single GET retry — small, to stay within the step's
/// per-request timeout budget.
const GET_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

/// Whether a failed read should be retried: only a companion *drop*
/// (`DeviceCompanionUnreachable`). A wedge (`DeviceCompanionWedged`, from a
/// timeout) is likely to repeat and burn the budget, and a status error is
/// deterministic — neither is retried.
fn is_retryable_read(e: &anyhow::Error) -> bool {
    golem_events::extract_code(e) == Some(golem_events::FailureCode::DeviceCompanionUnreachable)
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
            hierarchy_pace: std::sync::Mutex::new(None),
        }
    }

    /// Finish-to-start delay to apply before the next `/hierarchy` fetch, from
    /// the recorded `(finished_at, last_latency)`. The target gap is the last
    /// fetch's latency (capped); we sleep only the part not already elapsed
    /// since it finished. Pure + `Instant`-parameterised so it's unit-testable.
    fn hierarchy_pace_delay(
        state: Option<(std::time::Instant, std::time::Duration)>,
        now: std::time::Instant,
    ) -> std::time::Duration {
        match state {
            Some((finished_at, last_latency)) => {
                let target = last_latency.min(HIERARCHY_PACE_CAP);
                target.saturating_sub(now.saturating_duration_since(finished_at))
            }
            None => std::time::Duration::ZERO,
        }
    }

    /// Sleep the adaptive pace before a `/hierarchy` fetch (no-op for other
    /// paths / first fetch). Reads pace state under the lock, drops it, then
    /// awaits — never holding the lock across `.await`.
    async fn pace_hierarchy(&self, path: &str) {
        if path != HIERARCHY_PATH {
            return;
        }
        let delay = {
            let g = self.hierarchy_pace.lock().expect("hierarchy_pace poisoned");
            Self::hierarchy_pace_delay(*g, std::time::Instant::now())
        };
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }

    /// Record a completed `/hierarchy` fetch's latency for the next pace.
    fn record_hierarchy_latency(&self, path: &str, latency: std::time::Duration) {
        if path != HIERARCHY_PATH {
            return;
        }
        if let Ok(mut g) = self.hierarchy_pace.lock() {
            *g = Some((std::time::Instant::now(), latency));
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
        let text = resp.text().await.map_err(|e| {
            coded_transport_err(e, format!("reading response body from POST {url}"))
        })?;

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
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            // Level-1 prevent: adaptive hold-off before a `/hierarchy` fetch.
            self.pace_hierarchy(path).await;
            let started = std::time::Instant::now();
            let result = self.get_text_once(&url).await;
            self.record_hierarchy_latency(path, started.elapsed());
            match result {
                Ok(text) => return Ok(text),
                // Level-2 recover: one retry on a transient companion drop.
                Err(e) if attempt < GET_MAX_ATTEMPTS && is_retryable_read(&e) => {
                    tokio::time::sleep(GET_RETRY_BACKOFF).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// One `GET`-and-read-text attempt with the shared timeout + error coding.
    async fn get_text_once(&self, url: &str) -> Result<String> {
        let mut req = self.client.get(url);
        if let Some(t) = self.current_request_timeout() {
            req = req.timeout(t);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| coded_transport_err(e, format!("GET {url} failed")))?;

        let status = resp.status();
        // A drop while reading the body ("connection closed before message
        // completed" mid-`/hierarchy`) is the companion dying while serving —
        // code it as a transport failure, not an opaque uncoded error.
        let text = resp
            .text()
            .await
            .map_err(|e| coded_transport_err(e, format!("reading response body from GET {url}")))?;

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
            .map_err(|e| coded_transport_err(e, format!("reading bytes from GET {url}")))
    }
}

#[cfg(test)]
mod tests {
    use super::classify_transport;
    use golem_events::FailureCode;

    #[test]
    fn timeout_is_wedged() {
        assert_eq!(
            classify_transport(true, false, false, false),
            Some(FailureCode::DeviceCompanionWedged),
            "a client timeout SHALL code as wedged (alive but slow)"
        );
    }

    #[test]
    fn connect_is_unreachable() {
        assert_eq!(
            classify_transport(false, true, false, false),
            Some(FailureCode::DeviceCompanionUnreachable),
            "connect failure (refused) SHALL code as unreachable"
        );
    }

    #[test]
    fn mid_exchange_drop_is_unreachable() {
        // "connection closed before message completed" while reading a body
        // surfaces as a request/body error — the companion died while serving.
        assert_eq!(
            classify_transport(false, false, true, false),
            Some(FailureCode::DeviceCompanionUnreachable),
            "a request-side mid-exchange drop SHALL code as unreachable, not uncoded"
        );
        assert_eq!(
            classify_transport(false, false, false, true),
            Some(FailureCode::DeviceCompanionUnreachable),
            "a body-read mid-exchange drop SHALL code as unreachable, not uncoded"
        );
    }

    #[test]
    fn other_errors_stay_uncoded() {
        assert_eq!(
            classify_transport(false, false, false, false),
            None,
            "a decode/builder error SHALL stay uncoded (not attributable to companion death)"
        );
    }

    #[test]
    fn timeout_wins_over_drop() {
        // A timed-out request can also report request-kind; timeout is the
        // more specific signal (wedged, not gone) and SHALL win.
        assert_eq!(
            classify_transport(true, false, true, false),
            Some(FailureCode::DeviceCompanionWedged),
            "timeout SHALL take precedence over a request-kind flag"
        );
    }

    // ---- adaptive /hierarchy pacing ----

    use super::{CompanionClient, HIERARCHY_PACE_CAP};
    use std::time::{Duration, Instant};

    #[test]
    fn pace_delay_zero_on_first_fetch() {
        assert_eq!(
            CompanionClient::hierarchy_pace_delay(None, Instant::now()),
            Duration::ZERO,
            "no prior fetch SHALL impose no delay"
        );
    }

    #[test]
    fn pace_delay_is_last_latency_minus_elapsed() {
        let now = Instant::now();
        // Last fetch took 800ms, finished 500ms ago → 300ms still owed.
        let finished = now - Duration::from_millis(500);
        let delay = CompanionClient::hierarchy_pace_delay(
            Some((finished, Duration::from_millis(800))),
            now,
        );
        assert_eq!(delay, Duration::from_millis(300));
    }

    #[test]
    fn pace_delay_zero_when_gap_already_elapsed() {
        let now = Instant::now();
        // Last fetch took 200ms but finished 900ms ago → nothing owed.
        let finished = now - Duration::from_millis(900);
        let delay = CompanionClient::hierarchy_pace_delay(
            Some((finished, Duration::from_millis(200))),
            now,
        );
        assert_eq!(delay, Duration::ZERO);
    }

    #[test]
    fn pace_delay_caps_slow_latency() {
        let now = Instant::now();
        // A 5s snapshot is capped so pacing can't stall indefinitely.
        let finished = now - Duration::from_millis(1);
        let delay =
            CompanionClient::hierarchy_pace_delay(Some((finished, Duration::from_secs(5))), now);
        assert!(
            delay <= HIERARCHY_PACE_CAP && delay > HIERARCHY_PACE_CAP - Duration::from_millis(10),
            "SHALL cap at {HIERARCHY_PACE_CAP:?}; got {delay:?}"
        );
    }

    #[test]
    fn retryable_only_on_companion_drop() {
        let drop = golem_events::coded(
            FailureCode::DeviceCompanionUnreachable,
            anyhow::anyhow!("connection closed"),
        );
        let wedged = golem_events::coded(
            FailureCode::DeviceCompanionWedged,
            anyhow::anyhow!("timed out"),
        );
        let plain = anyhow::anyhow!("some 404");
        assert!(super::is_retryable_read(&drop), "a drop SHALL be retried");
        assert!(
            !super::is_retryable_read(&wedged),
            "a wedge SHALL NOT be retried (would burn the budget)"
        );
        assert!(
            !super::is_retryable_read(&plain),
            "an uncoded/status error SHALL NOT be retried"
        );
    }
}
