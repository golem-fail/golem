use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

/// Represents a temporary Ethereal email account for testing.
#[derive(Debug, Clone)]
pub struct EtherealAccount {
    pub user: String,
    pub pass: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub imap_host: String,
    pub imap_port: u16,
}

/// Raw response from the Ethereal / Nodemailer API.
#[derive(Debug, Deserialize)]
pub struct EtherealApiResponse {
    pub user: String,
    pub pass: String,
    pub smtp: EtherealSmtp,
    pub imap: EtherealImap,
}

#[derive(Debug, Deserialize)]
pub struct EtherealSmtp {
    pub host: String,
    pub port: u16,
    #[allow(dead_code)]
    pub secure: bool,
}

#[derive(Debug, Deserialize)]
pub struct EtherealImap {
    pub host: String,
    pub port: u16,
    #[allow(dead_code)]
    pub secure: bool,
}

impl EtherealAccount {
    /// Parse an `EtherealAccount` from the raw API response JSON.
    pub fn from_api_response(json: &str) -> Result<Self> {
        let resp: EtherealApiResponse =
            serde_json::from_str(json).context("failed to parse Ethereal API response")?;
        Ok(Self {
            user: resp.user,
            pass: resp.pass,
            smtp_host: resp.smtp.host,
            smtp_port: resp.smtp.port,
            imap_host: resp.imap.host,
            imap_port: resp.imap.port,
        })
    }
}

// ---------------------------------------------------------------------------
// Injectable HTTP client trait (allows mocking in tests)
// ---------------------------------------------------------------------------

/// Trait abstracting the HTTP call so we can inject a mock in tests.
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// POST to the given URL and return the response body as a `String`.
    async fn post(&self, url: &str) -> Result<String>;
}

/// Production HTTP client backed by `reqwest`.
pub struct ReqwestHttpClient {
    client: reqwest::Client,
}

impl ReqwestHttpClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn post(&self, url: &str) -> Result<String> {
        let resp = self
            .client
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body("requestor=golem&version=1")
            .send()
            .await
            .context("Ethereal API request failed")?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .context("failed to read Ethereal API response body")?;

        if !status.is_success() {
            anyhow::bail!("Ethereal API returned HTTP {}: {}", status.as_u16(), body);
        }

        Ok(body)
    }
}

// ---------------------------------------------------------------------------
// EtherealClient
// ---------------------------------------------------------------------------

const ETHEREAL_API_URL: &str = "https://api.nodemailer.com/user";

/// Client for creating temporary Ethereal email accounts.
pub struct EtherealClient {
    http: Box<dyn HttpClient>,
}

impl EtherealClient {
    /// Create a new `EtherealClient` using the real HTTP backend.
    pub fn new() -> Self {
        Self {
            http: Box::new(ReqwestHttpClient::new()),
        }
    }

    /// Create a new `EtherealClient` with an injected HTTP client (for tests).
    pub fn with_http_client(http: Box<dyn HttpClient>) -> Self {
        Self { http }
    }

    /// Create a new Ethereal test email account.
    ///
    /// POSTs to `https://api.nodemailer.com/user` and parses the JSON
    /// response into an [`EtherealAccount`].
    pub async fn create_account(&self) -> Result<EtherealAccount> {
        let body = self
            .http
            .post(ETHEREAL_API_URL)
            .await
            .context("failed to create Ethereal account")?;

        EtherealAccount::from_api_response(&body)
    }
}

impl Default for EtherealClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Fixture data -------------------------------------------------------

    const FIXTURE_RESPONSE: &str = r#"{
        "user": "abc123@ethereal.email",
        "pass": "s3cretP4ss",
        "smtp": {
            "host": "smtp.ethereal.email",
            "port": 587,
            "secure": false
        },
        "imap": {
            "host": "imap.ethereal.email",
            "port": 993,
            "secure": true
        }
    }"#;

    // -- Mock HTTP client ---------------------------------------------------

    struct MockHttpClient {
        response: String,
    }

    impl MockHttpClient {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
            }
        }
    }

    #[async_trait]
    impl HttpClient for MockHttpClient {
        async fn post(&self, _url: &str) -> Result<String> {
            Ok(self.response.clone())
        }
    }

    struct FailingHttpClient;

    #[async_trait]
    impl HttpClient for FailingHttpClient {
        async fn post(&self, _url: &str) -> Result<String> {
            anyhow::bail!("network unavailable")
        }
    }

    // -- Tests --------------------------------------------------------------

    #[test]
    fn parse_ethereal_api_response() {
        let account = EtherealAccount::from_api_response(FIXTURE_RESPONSE).expect("should parse");
        assert_eq!(account.user, "abc123@ethereal.email");
        assert_eq!(account.pass, "s3cretP4ss");
    }

    #[test]
    fn ethereal_account_has_correct_hosts_and_ports() {
        let account = EtherealAccount::from_api_response(FIXTURE_RESPONSE).expect("should parse");
        assert_eq!(account.smtp_host, "smtp.ethereal.email");
        assert_eq!(account.smtp_port, 587);
        assert_eq!(account.imap_host, "imap.ethereal.email");
        assert_eq!(account.imap_port, 993);
    }

    #[test]
    fn parse_invalid_json_returns_error() {
        let result = EtherealAccount::from_api_response("{invalid}");
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_fields_returns_error() {
        let json = r#"{"user": "x"}"#;
        let result = EtherealAccount::from_api_response(json);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn create_account_with_mock_http_client() {
        let mock = MockHttpClient::new(FIXTURE_RESPONSE);
        let client = EtherealClient::with_http_client(Box::new(mock));
        let account = client.create_account().await.expect("should succeed");
        assert_eq!(account.user, "abc123@ethereal.email");
        assert_eq!(account.pass, "s3cretP4ss");
        assert_eq!(account.smtp_host, "smtp.ethereal.email");
        assert_eq!(account.smtp_port, 587);
        assert_eq!(account.imap_host, "imap.ethereal.email");
        assert_eq!(account.imap_port, 993);
    }

    #[tokio::test]
    async fn create_account_propagates_http_error() {
        let client = EtherealClient::with_http_client(Box::new(FailingHttpClient));
        let result = client.create_account().await;
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("network unavailable"),
            "unexpected error: {err_msg}"
        );
    }

    // 7. An HTTP failure SHALL be wrapped with the create_account context so the
    //    chained message identifies the failed operation, not just the cause.
    #[tokio::test]
    async fn create_account_http_error_carries_create_context() {
        let client = EtherealClient::with_http_client(Box::new(FailingHttpClient));
        let err = client
            .create_account()
            .await
            .expect_err("network failure SHALL surface as error");
        let chained = format!("{err:#}");
        assert!(
            chained.contains("failed to create Ethereal account"),
            "error SHALL carry create_account context, got: {chained}"
        );
    }

    // 8. A successful HTTP response whose body is not valid JSON SHALL fail at
    //    the parse step (not the HTTP step) with the parse context message.
    #[tokio::test]
    async fn create_account_invalid_json_body_returns_parse_error() {
        let mock = MockHttpClient::new("not json at all");
        let client = EtherealClient::with_http_client(Box::new(mock));
        let err = client
            .create_account()
            .await
            .expect_err("invalid body SHALL fail to parse");
        let chained = format!("{err:#}");
        assert!(
            chained.contains("failed to parse Ethereal API response"),
            "parse failure SHALL carry parse context, got: {chained}"
        );
    }

    // 9. Unknown/extra JSON fields SHALL be tolerated (serde ignores them) so
    //    the account is parsed from the recognised fields only.
    #[test]
    fn parse_ignores_unknown_extra_fields() {
        let json = r#"{
            "user": "u@ethereal.email",
            "pass": "pw",
            "web": "https://ethereal.email/messages",
            "smtp": { "host": "smtp.ethereal.email", "port": 587, "secure": false, "extra": 1 },
            "imap": { "host": "imap.ethereal.email", "port": 993, "secure": true },
            "pop3": { "host": "pop3.ethereal.email", "port": 995, "secure": true }
        }"#;
        let account =
            EtherealAccount::from_api_response(json).expect("extra fields SHALL be ignored");
        // 9a. Every recognised field SHALL be populated from the input despite the
        //     surrounding unknown fields (web/pop3/extra), proving the parse keys
        //     off the known fields rather than positionally.
        assert_eq!(account.user, "u@ethereal.email");
        assert_eq!(account.pass, "pw");
        assert_eq!(account.smtp_host, "smtp.ethereal.email");
        assert_eq!(account.smtp_port, 587);
        assert_eq!(account.imap_host, "imap.ethereal.email");
        assert_eq!(account.imap_port, 993);
    }

    // 10. A port value outside the u16 range SHALL fail to parse rather than
    //     silently truncating or wrapping.
    #[test]
    fn parse_out_of_range_port_returns_error() {
        let json = r#"{
            "user": "u@ethereal.email",
            "pass": "pw",
            "smtp": { "host": "smtp.ethereal.email", "port": 70000, "secure": false },
            "imap": { "host": "imap.ethereal.email", "port": 993, "secure": true }
        }"#;
        let result = EtherealAccount::from_api_response(json);
        assert!(result.is_err(), "port > u16::MAX SHALL be rejected");
    }

    // 11. An empty body SHALL fail to parse (no fields present at all).
    #[test]
    fn parse_empty_body_returns_error() {
        let result = EtherealAccount::from_api_response("");
        assert!(result.is_err(), "empty body SHALL fail to parse");
    }
}
