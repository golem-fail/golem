//! HTTP transport seam for the `http` action.
//!
//! Mirrors the subprocess seam in `golem-common`: a process-global
//! [`HttpTransport`] that production backs with `reqwest` and tests replace
//! with a [`FakeHttpTransport`] returning canned responses keyed on
//! `(method, url)`. This makes `handle_http`'s request construction, status
//! handling, and response capture hermetically testable — no live server.
//!
//! Isolation follows the same model as the command seam: the transport is
//! process-global, so override tests must not run concurrently in one process
//! (the workspace runs under `cargo nextest`, process-per-test). The guard
//! returned by [`set_test_transport`] restores the previous transport on drop.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use anyhow::Result;
use async_trait::async_trait;

/// A completed HTTP response reduced to what `handle_http` needs.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Abstraction over issuing a single HTTP request.
#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) -> Result<HttpResponse>;
}

// ---------------------------------------------------------------------------
// Real implementation (reqwest)
// ---------------------------------------------------------------------------

/// Production transport backed by a shared `reqwest::Client`.
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) -> Result<HttpResponse> {
        let mut request = match method {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            "PATCH" => self.client.patch(url),
            "DELETE" => self.client.delete(url),
            other => anyhow::bail!("Unsupported HTTP method: {other}"),
        };
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body.to_string());
        }
        for (key, value) in headers {
            request = request.header(key.as_str(), value.as_str());
        }
        let response = request.send().await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        Ok(HttpResponse { status, body })
    }
}

// ---------------------------------------------------------------------------
// Process-global transport + test override
// ---------------------------------------------------------------------------

static OVERRIDE: RwLock<Option<Arc<dyn HttpTransport>>> = RwLock::new(None);

fn default_transport() -> &'static Arc<dyn HttpTransport> {
    static DEFAULT: OnceLock<Arc<dyn HttpTransport>> = OnceLock::new();
    DEFAULT.get_or_init(|| Arc::new(ReqwestTransport::default()))
}

/// The currently active transport (test override if set, else reqwest).
pub fn transport() -> Arc<dyn HttpTransport> {
    let guard = OVERRIDE.read().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(t) => Arc::clone(t),
        None => Arc::clone(default_transport()),
    }
}

/// Restores the previous transport when dropped.
#[must_use = "dropping the guard immediately restores the previous transport"]
pub struct TestTransportGuard {
    prev: Option<Arc<dyn HttpTransport>>,
}

impl Drop for TestTransportGuard {
    fn drop(&mut self) {
        *OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = self.prev.take();
    }
}

/// Install `t` as the process-global transport until the guard drops.
pub fn set_test_transport(t: Arc<dyn HttpTransport>) -> TestTransportGuard {
    let mut w = OVERRIDE.write().unwrap_or_else(|e| e.into_inner());
    let prev = w.take();
    *w = Some(t);
    TestTransportGuard { prev }
}

/// Issue a request via the active transport.
pub async fn request(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&str>,
) -> Result<HttpResponse> {
    transport().request(method, url, headers, body).await
}

// ---------------------------------------------------------------------------
// Fake implementation (for hermetic tests)
// ---------------------------------------------------------------------------

/// A canned HTTP outcome for one `(method, url)` invocation.
pub enum CannedHttp {
    /// Return this status + body.
    Response { status: u16, body: String },
    /// Fail the request (e.g. connection refused, DNS failure).
    Err(String),
}

impl CannedHttp {
    pub fn ok(body: impl Into<String>) -> Self {
        CannedHttp::Response {
            status: 200,
            body: body.into(),
        }
    }

    pub fn status(status: u16, body: impl Into<String>) -> Self {
        CannedHttp::Response {
            status,
            body: body.into(),
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        CannedHttp::Err(msg.into())
    }

    fn realize(&self) -> Result<HttpResponse> {
        match self {
            CannedHttp::Response { status, body } => Ok(HttpResponse {
                status: *status,
                body: body.clone(),
            }),
            CannedHttp::Err(msg) => Err(anyhow::anyhow!(msg.clone())),
        }
    }
}

/// A recorded HTTP request (for test assertions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// Test double for [`HttpTransport`]. Responses are keyed on `(method, url)`,
/// consumed FIFO with the last repeating once drained. Every request is
/// recorded. An un-scripted `(method, url)` errors so gaps surface.
pub struct FakeHttpTransport {
    responses: Mutex<HashMap<(String, String), VecDeque<CannedHttp>>>,
    calls: Mutex<Vec<RecordedRequest>>,
}

impl Default for FakeHttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeHttpTransport {
    pub fn new() -> Self {
        Self {
            responses: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Queue a response for a `(method, url)` pair.
    pub fn expect(&self, method: &str, url: &str, resp: CannedHttp) -> &Self {
        self.responses
            .lock()
            .expect("responses lock poisoned")
            .entry((method.to_string(), url.to_string()))
            .or_default()
            .push_back(resp);
        self
    }

    /// Every request seen so far, in order.
    pub fn recorded(&self) -> Vec<RecordedRequest> {
        self.calls.lock().expect("calls lock poisoned").clone()
    }
}

#[async_trait]
impl HttpTransport for FakeHttpTransport {
    async fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) -> Result<HttpResponse> {
        self.calls
            .lock()
            .expect("calls lock poisoned")
            .push(RecordedRequest {
                method: method.to_string(),
                url: url.to_string(),
                headers: headers.to_vec(),
                body: body.map(|b| b.to_string()),
            });

        let key = (method.to_string(), url.to_string());
        let mut map = self.responses.lock().expect("responses lock poisoned");
        match map.get_mut(&key) {
            Some(queue) if queue.len() > 1 => {
                queue.pop_front().expect("non-empty by guard").realize()
            }
            Some(queue) => queue.front().expect("non-empty by presence").realize(),
            None => Err(anyhow::anyhow!(
                "FakeHttpTransport: no canned response for {method} {url}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_matches_on_method_and_url() {
        let fake = FakeHttpTransport::new();
        fake.expect("GET", "https://x/api", CannedHttp::ok("hello"));
        let r = fake
            .request("GET", "https://x/api", &[], None)
            .await
            .expect("scripted");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, "hello");
        assert!(r.is_success());
    }

    #[tokio::test]
    async fn fake_records_headers_and_body() {
        let fake = FakeHttpTransport::new();
        fake.expect("POST", "https://x/api", CannedHttp::status(201, "created"));
        let headers = vec![("X-Token".to_string(), "abc".to_string())];
        fake.request("POST", "https://x/api", &headers, Some("{}"))
            .await
            .expect("scripted");
        let rec = fake.recorded();
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].method, "POST");
        assert_eq!(rec[0].headers, headers);
        assert_eq!(rec[0].body.as_deref(), Some("{}"));
    }

    #[tokio::test]
    async fn fake_unscripted_errors() {
        let fake = FakeHttpTransport::new();
        let err = fake
            .request("GET", "https://x/none", &[], None)
            .await
            .expect_err("unscripted SHALL error");
        assert!(format!("{err:#}").contains("no canned response"));
    }

    #[tokio::test]
    async fn fake_error_response_propagates() {
        let fake = FakeHttpTransport::new();
        fake.expect("GET", "https://x", CannedHttp::error("connection refused"));
        let err = fake
            .request("GET", "https://x", &[], None)
            .await
            .expect_err("scripted error SHALL surface");
        assert!(format!("{err:#}").contains("connection refused"));
    }
}
