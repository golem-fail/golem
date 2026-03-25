use anyhow::{Context, Result};
use async_trait::async_trait;
use regex::Regex;

/// A parsed email message.
#[derive(Debug, Clone)]
pub struct EmailMessage {
    pub from: String,
    pub to: String,
    pub subject: String,
    pub body: String,
    pub date: String,
}

impl EmailMessage {
    /// Parse an `EmailMessage` from a raw RFC-2822-style string.
    ///
    /// This is intentionally simple — it handles the subset of headers
    /// produced by Ethereal and similar test SMTP servers. Production
    /// email parsing would need a full MIME library.
    pub fn from_raw(raw: &str) -> Result<Self> {
        let from = Self::extract_header(raw, "From")
            .context("missing From header")?;
        let to = Self::extract_header(raw, "To")
            .context("missing To header")?;
        let subject = Self::extract_header(raw, "Subject")
            .context("missing Subject header")?;
        let date = Self::extract_header(raw, "Date")
            .unwrap_or_default();

        // Body is everything after the first blank line.
        let body = raw
            .split_once("\r\n\r\n")
            .or_else(|| raw.split_once("\n\n"))
            .map(|(_, b)| b.trim().to_string())
            .unwrap_or_default();

        Ok(Self {
            from,
            to,
            subject,
            body,
            date,
        })
    }

    fn extract_header(raw: &str, name: &str) -> Option<String> {
        for line in raw.lines() {
            if let Some(value) = line.strip_prefix(&format!("{name}: ")) {
                return Some(value.trim().to_string());
            }
            // Also try case-insensitive prefix (common in real mail).
            let lower = line.to_lowercase();
            let prefix = format!("{}: ", name.to_lowercase());
            if lower.starts_with(&prefix) {
                return Some(line[prefix.len()..].trim().to_string());
            }
        }
        None
    }
}

/// Check whether a subject line matches the given pattern.
///
/// The pattern supports simple glob-style wildcards (`*`).
/// `*` matches any sequence of characters (including empty).
pub fn subject_matches(subject: &str, pattern: &str) -> bool {
    // Convert the glob pattern to a regex.
    let mut re_str = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => re_str.push_str(".*"),
            '?' => re_str.push('.'),
            c => {
                // Escape regex-special characters.
                if "\\+()[]{}|^$.".contains(c) {
                    re_str.push('\\');
                }
                re_str.push(c);
            }
        }
    }
    re_str.push('$');
    Regex::new(&re_str)
        .map(|re| re.is_match(subject))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Injectable IMAP connection trait
// ---------------------------------------------------------------------------

/// Trait abstracting the IMAP connection so we can inject a mock in tests.
#[async_trait]
pub trait ImapConnection: Send + Sync {
    /// Fetch all messages currently in the INBOX and return them as
    /// a list of [`EmailMessage`] values.
    async fn fetch_inbox(&self) -> Result<Vec<EmailMessage>>;
}

// ---------------------------------------------------------------------------
// ImapPoller
// ---------------------------------------------------------------------------

/// Polls an IMAP inbox waiting for an email whose subject matches a pattern.
pub struct ImapPoller {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    connector: Box<dyn ImapConnection>,
}

impl ImapPoller {
    /// Create a new `ImapPoller` with the real IMAP backend.
    pub fn new(host: String, port: u16, user: String, pass: String) -> Self {
        let connector = RealImapConnection {
            host: host.clone(),
            port,
            user: user.clone(),
            pass: pass.clone(),
        };
        Self {
            host,
            port,
            user,
            pass,
            connector: Box::new(connector),
        }
    }

    /// Create a new `ImapPoller` with an injected IMAP connection (for tests).
    pub fn with_connection(
        host: String,
        port: u16,
        user: String,
        pass: String,
        connector: Box<dyn ImapConnection>,
    ) -> Self {
        Self {
            host,
            port,
            user,
            pass,
            connector,
        }
    }

    /// Poll the inbox until an email whose subject matches `subject_pattern`
    /// arrives, or until `timeout_ms` elapses.
    ///
    /// - `subject_pattern`: glob pattern (e.g. `"*verification*"`).
    /// - `timeout_ms`: maximum time to wait, in milliseconds.
    /// - `poll_interval_ms`: sleep between successive polls, in milliseconds.
    pub async fn await_email(
        &self,
        subject_pattern: &str,
        timeout_ms: u64,
        poll_interval_ms: u64,
    ) -> Result<EmailMessage> {
        let deadline =
            tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);

        loop {
            let messages = self
                .connector
                .fetch_inbox()
                .await
                .context("failed to fetch IMAP inbox")?;

            for msg in messages {
                if subject_matches(&msg.subject, subject_pattern) {
                    return Ok(msg);
                }
            }

            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out after {timeout_ms}ms waiting for email matching \"{subject_pattern}\""
                );
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(poll_interval_ms)).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Real IMAP connection (placeholder — needs the `imap` crate or similar)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct RealImapConnection {
    host: String,
    port: u16,
    user: String,
    pass: String,
}

#[async_trait]
impl ImapConnection for RealImapConnection {
    async fn fetch_inbox(&self) -> Result<Vec<EmailMessage>> {
        // In a full implementation this would open a TLS connection to
        // self.host:self.port, authenticate with self.user/self.pass,
        // SELECT INBOX, and FETCH all messages.
        //
        // For now we return an error indicating that real IMAP is not yet
        // wired up — tests use the mock connector instead.
        anyhow::bail!(
            "real IMAP connection to {}:{} is not yet implemented (user: {})",
            self.host,
            self.port,
            self.user
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // -- Fixture data -------------------------------------------------------

    const RAW_EMAIL: &str = "From: sender@example.com\r\n\
                             To: receiver@example.com\r\n\
                             Subject: Your verification code\r\n\
                             Date: Mon, 23 Mar 2026 10:00:00 +0000\r\n\
                             \r\n\
                             Your code is 123456.";

    // -- Mock IMAP connection -----------------------------------------------

    struct MockImapConnection {
        messages: Vec<EmailMessage>,
    }

    #[async_trait]
    impl ImapConnection for MockImapConnection {
        async fn fetch_inbox(&self) -> Result<Vec<EmailMessage>> {
            Ok(self.messages.clone())
        }
    }

    struct EmptyInboxConnection;

    #[async_trait]
    impl ImapConnection for EmptyInboxConnection {
        async fn fetch_inbox(&self) -> Result<Vec<EmailMessage>> {
            Ok(vec![])
        }
    }

    /// A mock connection that returns an empty inbox for the first N calls,
    /// then returns the provided messages.
    struct DelayedInboxConnection {
        messages: Vec<EmailMessage>,
        call_count: Arc<AtomicUsize>,
        empty_for: usize,
    }

    #[async_trait]
    impl ImapConnection for DelayedInboxConnection {
        async fn fetch_inbox(&self) -> Result<Vec<EmailMessage>> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n < self.empty_for {
                Ok(vec![])
            } else {
                Ok(self.messages.clone())
            }
        }
    }

    // -- Helper -------------------------------------------------------------

    fn make_email(subject: &str) -> EmailMessage {
        EmailMessage {
            from: "sender@example.com".into(),
            to: "receiver@example.com".into(),
            subject: subject.into(),
            body: "hello".into(),
            date: "Mon, 23 Mar 2026 10:00:00 +0000".into(),
        }
    }

    // -- Tests: parsing -----------------------------------------------------

    #[test]
    fn parse_email_from_raw() {
        let msg = EmailMessage::from_raw(RAW_EMAIL).expect("should parse");
        assert_eq!(msg.from, "sender@example.com");
        assert_eq!(msg.to, "receiver@example.com");
        assert_eq!(msg.subject, "Your verification code");
        assert_eq!(msg.body, "Your code is 123456.");
        assert_eq!(msg.date, "Mon, 23 Mar 2026 10:00:00 +0000");
    }

    #[test]
    fn parse_email_unix_line_endings() {
        let raw = "From: a@b.com\nTo: c@d.com\nSubject: hi\n\nBody text";
        let msg = EmailMessage::from_raw(raw).expect("should parse");
        assert_eq!(msg.subject, "hi");
        assert_eq!(msg.body, "Body text");
    }

    #[test]
    fn parse_email_missing_from_is_error() {
        let raw = "To: c@d.com\nSubject: hi\n\nBody text";
        assert!(EmailMessage::from_raw(raw).is_err());
    }

    // -- Tests: subject matching --------------------------------------------

    #[test]
    fn subject_matches_exact() {
        assert!(subject_matches("hello world", "hello world"));
        assert!(!subject_matches("hello world!", "hello world"));
    }

    #[test]
    fn subject_matches_wildcard() {
        assert!(subject_matches("Your verification code", "*verification*"));
        assert!(subject_matches("verification", "*verification*"));
        assert!(!subject_matches("Your code", "*verification*"));
    }

    #[test]
    fn subject_matches_question_mark() {
        assert!(subject_matches("code-A", "code-?"));
        assert!(!subject_matches("code-AB", "code-?"));
    }

    #[test]
    fn subject_matches_leading_trailing_wildcard() {
        assert!(subject_matches("abc", "*"));
        assert!(subject_matches("", "*"));
    }

    // -- Tests: ImapPoller configuration ------------------------------------

    #[test]
    fn imap_poller_stores_config() {
        let poller = ImapPoller::new(
            "imap.ethereal.email".into(),
            993,
            "user@ethereal.email".into(),
            "pass123".into(),
        );
        assert_eq!(poller.host, "imap.ethereal.email");
        assert_eq!(poller.port, 993);
        assert_eq!(poller.user, "user@ethereal.email");
        assert_eq!(poller.pass, "pass123");
    }

    // -- Tests: await_email -------------------------------------------------

    #[tokio::test]
    async fn await_email_returns_immediately_when_present() {
        let msg = make_email("Your verification code");
        let conn = MockImapConnection {
            messages: vec![msg.clone()],
        };
        let poller = ImapPoller::with_connection(
            "host".into(),
            993,
            "user".into(),
            "pass".into(),
            Box::new(conn),
        );

        let result = poller
            .await_email("*verification*", 5000, 100)
            .await
            .expect("should find email");
        assert_eq!(result.subject, "Your verification code");
    }

    #[tokio::test]
    async fn await_email_empty_inbox_times_out() {
        let conn = EmptyInboxConnection;
        let poller = ImapPoller::with_connection(
            "host".into(),
            993,
            "user".into(),
            "pass".into(),
            Box::new(conn),
        );

        let result = poller.await_email("*verification*", 200, 50).await;
        assert!(result.is_err());
        let err = format!("{:#}", result.expect_err("should timeout"));
        assert!(err.contains("timed out"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn await_email_polls_until_message_arrives() {
        let msg = make_email("Your verification code");
        let call_count = Arc::new(AtomicUsize::new(0));
        let conn = DelayedInboxConnection {
            messages: vec![msg],
            call_count: Arc::clone(&call_count),
            empty_for: 2, // first 2 polls return empty
        };
        let poller = ImapPoller::with_connection(
            "host".into(),
            993,
            "user".into(),
            "pass".into(),
            Box::new(conn),
        );

        let result = poller
            .await_email("*verification*", 5000, 50)
            .await
            .expect("should eventually find email");
        assert_eq!(result.subject, "Your verification code");

        // Verify it actually polled multiple times.
        let calls = call_count.load(Ordering::SeqCst);
        assert!(calls >= 3, "expected at least 3 polls, got {calls}");
    }

    #[tokio::test]
    async fn await_email_ignores_non_matching_subjects() {
        let msgs = vec![
            make_email("Welcome aboard"),
            make_email("Your invoice"),
        ];
        let conn = MockImapConnection { messages: msgs };
        let poller = ImapPoller::with_connection(
            "host".into(),
            993,
            "user".into(),
            "pass".into(),
            Box::new(conn),
        );

        let result = poller.await_email("*verification*", 200, 50).await;
        assert!(result.is_err(), "SHALL NOT match any email");
    }
}
