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
        let from = Self::extract_header(raw, "From").context("missing From header")?;
        let to = Self::extract_header(raw, "To").context("missing To header")?;
        let subject = Self::extract_header(raw, "Subject").context("missing Subject header")?;
        let date = Self::extract_header(raw, "Date").unwrap_or_default();

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
            // Split on the first colon: everything before is the field name,
            // everything after is the value (which may itself contain colons,
            // e.g. a Date). Matching the name is ASCII-case-insensitive (RFC
            // 5322 header names are ASCII). No byte-slicing, so multibyte
            // values can't trip a char-boundary panic.
            if let Some((key, value)) = line.split_once(':') {
                if key.trim().eq_ignore_ascii_case(name) {
                    return Some(value.trim().to_string());
                }
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
    /// Fetch the most recent messages in the INBOX, in IMAP sequence order
    /// (oldest first), as a list of [`EmailMessage`] values. The real backend
    /// caps this to the latest window rather than the entire mailbox.
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
    /// When several emails match, the **most recent** one is returned —
    /// `fetch_inbox` yields messages in IMAP sequence order (oldest first), so
    /// we scan from the back. This is what verification/OTP flows want: a stale
    /// code left in the inbox from an earlier run never shadows the fresh one.
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
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);

        loop {
            let messages = self
                .connector
                .fetch_inbox()
                .await
                .context("failed to fetch IMAP inbox")?;

            // Newest-first: the last message in sequence order is the most recent.
            for msg in messages.into_iter().rev() {
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
// Real IMAP connection (rustls TLS + the blocking `imap` crate)
// ---------------------------------------------------------------------------

struct RealImapConnection {
    host: String,
    port: u16,
    user: String,
    pass: String,
}

#[async_trait]
impl ImapConnection for RealImapConnection {
    async fn fetch_inbox(&self) -> Result<Vec<EmailMessage>> {
        // The `imap` crate is blocking and `rustls` here is the synchronous
        // API, so run the whole connect→login→fetch cycle on a blocking
        // thread to keep the async poll loop responsive.
        let host = self.host.clone();
        let port = self.port;
        let user = self.user.clone();
        let pass = self.pass.clone();
        tokio::task::spawn_blocking(move || fetch_inbox_blocking(&host, port, &user, &pass))
            .await
            .context("IMAP fetch task failed to join")?
    }
}

/// Open a TLS IMAP connection, log in, EXAMINE INBOX (read-only, so messages
/// stay unseen), FETCH every message, and parse each into an [`EmailMessage`].
///
/// One full connection per call: poll frequency is low (seconds) and test
/// inboxes are tiny, so a persistent session isn't worth the added state.
fn fetch_inbox_blocking(
    host: &str,
    port: u16,
    user: &str,
    pass: &str,
) -> Result<Vec<EmailMessage>> {
    use std::sync::Arc;

    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .with_context(|| format!("invalid IMAP host name {host:?}"))?;
    let conn = rustls::ClientConnection::new(Arc::new(config), server_name)
        .context("failed to initialise TLS client")?;
    let tcp = std::net::TcpStream::connect((host, port))
        .with_context(|| format!("failed to connect to IMAP {host}:{port}"))?;
    let tls = rustls::StreamOwned::new(conn, tcp);

    let mut client = imap::Client::new(tls);
    client
        .read_greeting()
        .context("IMAP server greeting failed")?;
    let mut session = client
        .login(user, pass)
        .map_err(|(e, _)| anyhow::anyhow!("IMAP login failed: {e}"))?;

    let mailbox = session
        .examine("INBOX")
        .context("IMAP EXAMINE INBOX failed")?;

    let mut out = Vec::new();
    if mailbox.exists > 0 {
        // Only the most-recent window of messages, not the whole mailbox:
        // `await_email` returns the newest match and re-fetches every poll, so
        // pulling full RFC822 bodies for the entire inbox each time is wasted
        // bandwidth. A verification/OTP mail is always among the latest few; a
        // match older than the window is intentionally out of scope.
        const WINDOW: u32 = 30;
        let start = mailbox.exists.saturating_sub(WINDOW - 1).max(1);
        let seq = format!("{start}:*");
        let fetches = session.fetch(&seq, "RFC822").context("IMAP FETCH failed")?;
        for fetch in fetches.iter() {
            if let Some(body) = fetch.body() {
                let raw = String::from_utf8_lossy(body);
                if let Ok(msg) = EmailMessage::from_raw(&raw) {
                    out.push(msg);
                }
            }
        }
    }

    // Best-effort logout; the result doesn't change what we fetched.
    let _ = session.logout();
    Ok(out)
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

    /// A mock connection whose `fetch_inbox` always fails.
    struct FailingInboxConnection;

    #[async_trait]
    impl ImapConnection for FailingInboxConnection {
        async fn fetch_inbox(&self) -> Result<Vec<EmailMessage>> {
            anyhow::bail!("connection refused")
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

    // 1. A missing To header SHALL surface as an error carrying the
    //    "missing To header" context, distinct from the From failure.
    #[test]
    fn parse_email_missing_to_is_error() {
        let raw = "From: a@b.com\nSubject: hi\n\nBody text";
        let err = EmailMessage::from_raw(raw).expect_err("missing To SHALL error");
        let msg = format!("{err:#}");
        assert!(msg.contains("missing To header"), "unexpected error: {msg}");
    }

    // 2. A missing Subject header SHALL surface as an error.
    #[test]
    fn parse_email_missing_subject_is_error() {
        let raw = "From: a@b.com\nTo: c@d.com\n\nBody text";
        let err = EmailMessage::from_raw(raw).expect_err("missing Subject SHALL error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("missing Subject header"),
            "unexpected error: {msg}"
        );
    }

    // 3. Date is optional: when absent the field SHALL default to empty
    //    while parsing still succeeds.
    #[test]
    fn parse_email_missing_date_defaults_empty() {
        let raw = "From: a@b.com\nTo: c@d.com\nSubject: hi\n\nBody text";
        let msg = EmailMessage::from_raw(raw).expect("SHALL parse without Date");
        assert_eq!(msg.date, "", "absent Date SHALL be empty string");
        assert_eq!(msg.body, "Body text");
    }

    // 4. With no blank-line separator there is no body, so the body
    //    SHALL default to empty (headers still parse).
    #[test]
    fn parse_email_no_blank_line_has_empty_body() {
        let raw = "From: a@b.com\nTo: c@d.com\nSubject: hi";
        let msg = EmailMessage::from_raw(raw).expect("SHALL parse header-only message");
        assert_eq!(msg.body, "", "no blank line SHALL yield empty body");
        assert_eq!(msg.subject, "hi");
    }

    // 5. Headers SHALL be matched case-insensitively (real mail may use
    //    lowercase keys like "from:").
    #[test]
    fn parse_email_case_insensitive_headers() {
        let raw = "from: a@b.com\nto: c@d.com\nsubject: hi\n\nBody";
        let msg = EmailMessage::from_raw(raw).expect("SHALL parse lowercase headers");
        assert_eq!(msg.from, "a@b.com");
        assert_eq!(msg.to, "c@d.com");
        assert_eq!(msg.subject, "hi");
    }

    // 6. Header values SHALL be trimmed of surrounding whitespace, and a
    //    CRLF-separated body SHALL be preferred and trimmed.
    #[test]
    fn parse_email_trims_header_and_body() {
        let raw = "From: a@b.com  \r\nTo: c@d.com\r\nSubject: spaced  \r\n\r\n  Body  ";
        let msg = EmailMessage::from_raw(raw).expect("SHALL parse");
        assert_eq!(msg.from, "a@b.com", "From SHALL be trimmed");
        assert_eq!(msg.subject, "spaced", "Subject SHALL be trimmed");
        assert_eq!(msg.body, "Body", "body SHALL be trimmed");
    }

    // Multibyte header values SHALL parse without panicking. The old
    // implementation byte-sliced the original line at an offset measured on a
    // lowercased copy, which could land mid-UTF-8-char; split_once + trim has
    // no such slice. A value containing colons (the Date) SHALL also survive.
    #[test]
    fn parse_email_handles_multibyte_and_colon_values() {
        let raw = "From: 山田太郎 <a@b.com>\r\n\
                   To: c@d.com\r\n\
                   Subject: 日本語の verify code\r\n\
                   Date: Mon, 23 Mar 2026 10:00:00 +0000\r\n\
                   \r\n\
                   コードは 123456 です。";
        let msg = EmailMessage::from_raw(raw).expect("multibyte headers SHALL parse");
        assert_eq!(msg.from, "山田太郎 <a@b.com>");
        assert_eq!(msg.subject, "日本語の verify code");
        assert_eq!(msg.date, "Mon, 23 Mar 2026 10:00:00 +0000");
        assert_eq!(msg.body, "コードは 123456 です。");
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

    // 7. `?` requires exactly one character, so it SHALL NOT match an
    //    empty position.
    #[test]
    fn subject_matches_question_mark_requires_one_char() {
        assert!(
            !subject_matches("code-", "code-?"),
            "? SHALL NOT match empty"
        );
        assert!(subject_matches("code-X", "code-?"));
    }

    // 8. Regex-special characters in the pattern SHALL be treated as
    //    literals, not as regex metacharacters.
    #[test]
    fn subject_matches_escapes_regex_special_chars() {
        // '.' is escaped, so it matches a literal dot only.
        assert!(subject_matches("a.b", "a.b"));
        assert!(
            !subject_matches("axb", "a.b"),
            "'.' SHALL be literal, not any-char"
        );
        // '+' is escaped: literal plus, not "one-or-more".
        assert!(subject_matches("a+b", "a+b"));
        assert!(!subject_matches("aaab", "a+b"));
        // Parentheses and brackets are literal.
        assert!(subject_matches("code (1) [x]", "code (1) [x]"));
    }

    // 9. Multiple wildcards SHALL each expand independently and the whole
    //    pattern is anchored at both ends.
    #[test]
    fn subject_matches_multiple_wildcards_anchored() {
        assert!(subject_matches("alpha-beta-gamma", "alpha*gamma"));
        assert!(subject_matches("a1b2c", "a*b*c"));
        // Anchored: a trailing-only match is not enough without a wildcard.
        assert!(!subject_matches("prefix-verification", "verification"));
    }

    // 10. An invalid regex (unbalanced escape) cannot arise from glob
    //     translation, but an empty pattern SHALL only match an empty
    //     subject because of the ^$ anchors.
    #[test]
    fn subject_matches_empty_pattern_only_matches_empty() {
        assert!(subject_matches("", ""));
        assert!(
            !subject_matches("x", ""),
            "empty pattern SHALL NOT match non-empty"
        );
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
        let msgs = vec![make_email("Welcome aboard"), make_email("Your invoice")];
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

    // 11. A fetch failure SHALL propagate immediately, wrapped with the
    //     "failed to fetch IMAP inbox" context (not a timeout).
    #[tokio::test]
    async fn await_email_propagates_fetch_error() {
        let poller = ImapPoller::with_connection(
            "host".into(),
            993,
            "user".into(),
            "pass".into(),
            Box::new(FailingInboxConnection),
        );

        let err = poller
            .await_email("*verification*", 5000, 50)
            .await
            .expect_err("fetch failure SHALL error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("failed to fetch IMAP inbox"),
            "SHALL carry fetch context: {msg}"
        );
        assert!(
            msg.contains("connection refused"),
            "SHALL preserve underlying cause: {msg}"
        );
        assert!(
            !msg.contains("timed out"),
            "SHALL fail fast, not time out: {msg}"
        );
    }

    // 12. When several messages match, the MOST RECENT one (last in IMAP
    //     sequence order) SHALL be returned, so a stale match from an earlier
    //     run never shadows the fresh email.
    #[tokio::test]
    async fn await_email_returns_most_recent_matching() {
        let msgs = vec![
            make_email("Welcome aboard"),
            make_email("verification one"),
            make_email("verification two"),
        ];
        let conn = MockImapConnection { messages: msgs };
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
            .expect("SHALL find a match");
        assert_eq!(
            result.subject, "verification two",
            "SHALL return the most recent matching email"
        );
    }

    // 13. The real IMAP backend is now wired (rustls + the `imap` crate).
    //     Pointing it at a closed local port proves it actually attempts a
    //     TCP connection — the fetch error carries the connect context and is
    //     no longer the old "not yet implemented" stub — without touching the
    //     network. Port 1 has nothing listening, so connect fails fast.
    #[tokio::test]
    async fn await_email_real_backend_attempts_connection() {
        let poller = ImapPoller::new(
            "127.0.0.1".into(),
            1,
            "user@example.com".into(),
            "pass".into(),
        );

        let err = poller
            .await_email("*verification*", 3000, 50)
            .await
            .expect_err("offline connect SHALL error");
        let msg = format!("{err:#}");
        assert!(
            !msg.contains("not yet implemented"),
            "real backend SHALL be wired, not a stub: {msg}"
        );
        assert!(
            msg.contains("failed to fetch IMAP inbox"),
            "SHALL surface via the fetch context: {msg}"
        );
        assert!(
            msg.contains("failed to connect to IMAP 127.0.0.1:1"),
            "SHALL report the connect attempt: {msg}"
        );
    }

    // 14. Live smoke against a real IMAP server (Ethereal). Ignored by
    //     default — it needs the network and real credentials. Run with:
    //       GOLEM_IMAP_HOST=imap.ethereal.email GOLEM_IMAP_PORT=993 \
    //       GOLEM_IMAP_USER=… GOLEM_IMAP_PASS=… \
    //       cargo nextest run -p golem-email --run-ignored all live_receive
    //     Provision via the Nodemailer API; a fresh Ethereal inbox already
    //     contains a welcome message, so no SMTP send is needed (leave
    //     GOLEM_IMAP_SUBJECT unset to match it with "*"). Asserts the real
    //     rustls+IMAP path connects, authenticates, and fetches a parsed message.
    #[tokio::test]
    #[ignore = "live network + real IMAP credentials required"]
    async fn live_receive() {
        let host = std::env::var("GOLEM_IMAP_HOST").expect("GOLEM_IMAP_HOST");
        let port: u16 = std::env::var("GOLEM_IMAP_PORT")
            .expect("GOLEM_IMAP_PORT")
            .parse()
            .expect("port");
        let user = std::env::var("GOLEM_IMAP_USER").expect("GOLEM_IMAP_USER");
        let pass = std::env::var("GOLEM_IMAP_PASS").expect("GOLEM_IMAP_PASS");
        let pattern = std::env::var("GOLEM_IMAP_SUBJECT").unwrap_or_else(|_| "*".into());

        let poller = ImapPoller::new(host, port, user, pass);
        let msg = poller
            .await_email(&pattern, 30000, 2000)
            .await
            .expect("SHALL receive a live message");
        println!("LIVE subject={:?}\nLIVE body={:?}", msg.subject, msg.body);
        assert!(!msg.subject.is_empty(), "subject SHALL be populated");
    }
}
