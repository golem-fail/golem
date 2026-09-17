//! Readiness probe for a developer-run JS dev server (Expo/Metro) under
//! `golem run --dev`.
//!
//! golem never starts the bundler. The developer runs `npx expo start`; this
//! module only blocks until that server answers, so a suite fails with "no
//! dev server" instead of a redbox misread as a missing selector.

use anyhow::Result;
use std::time::{Duration, Instant};

/// Metro's readiness endpoint. It answers `packager-status:running` once the
/// server can serve bundles.
const STATUS_PATH: &str = "status";
const READY_BODY: &str = "packager-status:running";

/// How long `wait_until_ready` polls before giving up.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(30);

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// `http://127.0.0.1:<port>/status` — the URL the probe polls.
pub fn status_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/{STATUS_PATH}")
}

/// Whether a `/status` body means the server can serve bundles.
pub fn body_is_ready(body: &str) -> bool {
    body.trim_start().starts_with(READY_BODY)
}

/// The message shown when no dev server answered in time.
pub fn unavailable_message(port: u16, waited: Duration) -> String {
    format!(
        "no dev server answered {} after {}s — `--dev` expects a bundler you \
         started yourself. Run `npx expo start` in your Expo project (or pass \
         `--dev-port` if it listens elsewhere), then re-run.",
        status_url(port),
        waited.as_secs(),
    )
}

/// Poll the dev server until it reports ready, or `timeout` elapses.
///
/// Returns how long the wait took, so the caller can tell "already up" from
/// "we waited". The error is tagged `H503`.
pub async fn wait_until_ready(port: u16, timeout: Duration) -> Result<Duration> {
    let client = reqwest::Client::builder()
        .timeout(POLL_INTERVAL)
        .build()
        .map_err(|e| {
            golem_events::coded(
                golem_events::FailureCode::HostDevServerUnavailable,
                anyhow::anyhow!("could not build the dev-server probe client: {e}"),
            )
        })?;
    let url = status_url(port);
    let start = Instant::now();
    loop {
        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(body) = resp.text().await {
                if body_is_ready(&body) {
                    return Ok(start.elapsed());
                }
            }
        }
        if start.elapsed() >= timeout {
            return Err(golem_events::coded(
                golem_events::FailureCode::HostDevServerUnavailable,
                anyhow::anyhow!("{}", unavailable_message(port, timeout)),
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

// ---------------------------------------------------------------------------
// bundle health
// ---------------------------------------------------------------------------

/// Expo's virtual entry point. Asking for it makes Metro transform the app's
/// module graph, so a syntax error anywhere in it answers 500 with a JSON
/// `TransformError` instead of a bundle.
///
/// Expo-specific on purpose. `/index.bundle` exists too but was observed
/// serving a healthy 200 for a project whose `App.tsx` did not parse, so it
/// cannot be used to tell health from breakage.
const EXPO_ENTRY: &str = ".expo/.virtual-metro-entry.bundle";

/// `http://127.0.0.1:<port>/<expo entry>?platform=<platform>…` — the URL the
/// app itself fetches, minus the parameters that only affect output shape.
pub fn bundle_url(port: u16, platform: &str) -> String {
    format!("http://127.0.0.1:{port}/{EXPO_ENTRY}?platform={platform}&dev=true&minify=false")
}

/// The one-line summary of a Metro build failure, from its JSON body.
///
/// Metro's `message` holds the summary, a blank line, then an ANSI-coloured
/// code frame; only the first line is wanted. Falls back to the structured
/// fields when the message is missing, and to `None` when the body isn't a
/// Metro error at all — a 500 from something else on the port is not evidence
/// about the app.
pub fn parse_bundle_error(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if v.get("type")?.as_str()? != "TransformError" {
        return None;
    }
    if let Some(first) = v
        .get("message")
        .and_then(|m| m.as_str())
        .and_then(|m| m.lines().find(|l| !l.trim().is_empty()))
    {
        return Some(strip_ansi(first).trim().to_string());
    }
    let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("Error");
    let file = v.get("filename").and_then(|f| f.as_str()).unwrap_or("?");
    let line = v.get("lineNumber").and_then(|l| l.as_u64()).unwrap_or(0);
    let col = v.get("column").and_then(|c| c.as_u64()).unwrap_or(0);
    Some(format!("{name}: {file} ({line}:{col})"))
}

/// Drop ANSI SGR sequences. Metro colours its code frames, and those escapes
/// would otherwise land in a report file.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // CSI … final byte in @-~; anything else we skip one char and resync.
        if chars.next() == Some('[') {
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

/// Ask the dev server whether the app's bundle builds.
///
/// `Ok(())` means "no evidence of a problem", which covers a healthy build and
/// every case where the answer isn't interpretable — a non-Expo entry point
/// (404), a body that isn't Metro's, a request that fails outright. The probe
/// exists to turn a known breakage into a good message, never to invent one.
pub async fn check_bundle(port: u16, platform: &str) -> Result<()> {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
    {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    let Ok(resp) = client.get(bundle_url(port, platform)).send().await else {
        return Ok(());
    };
    if resp.status() != reqwest::StatusCode::INTERNAL_SERVER_ERROR {
        // A healthy build answers 200 with megabytes of JavaScript. Dropping
        // the response here means none of it is ever read off the socket.
        return Ok(());
    }
    let Ok(body) = resp.text().await else {
        return Ok(());
    };
    match parse_bundle_error(&body) {
        Some(message) => Err(golem_events::coded(
            golem_events::FailureCode::AppDevBundleError,
            anyhow::anyhow!("the dev server cannot build the app's bundle: {message}"),
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_url_targets_loopback_on_the_given_port() {
        assert_eq!(status_url(8081), "http://127.0.0.1:8081/status");
        assert_eq!(status_url(19000), "http://127.0.0.1:19000/status");
    }

    #[test]
    fn running_packager_reads_as_ready() {
        assert!(body_is_ready("packager-status:running"));
        // Metro answers without a trailing newline today, but a proxy that
        // adds whitespace must not read as "not ready".
        assert!(body_is_ready("  packager-status:running\n"));
    }

    #[test]
    fn anything_else_reads_as_not_ready() {
        assert!(!body_is_ready(""));
        assert!(!body_is_ready("packager-status:stopped"));
        // A different server on the port (a web app, a proxy error page)
        // must not be mistaken for a bundler.
        assert!(!body_is_ready(
            "<!doctype html><title>something else</title>"
        ));
    }

    #[test]
    fn the_unavailable_message_names_the_url_the_port_and_the_fix() {
        let msg = unavailable_message(8081, Duration::from_secs(30));
        assert!(msg.contains("http://127.0.0.1:8081/status"), "got: {msg}");
        assert!(msg.contains("30s"), "got: {msg}");
        assert!(msg.contains("npx expo start"), "got: {msg}");
        assert!(msg.contains("--dev-port"), "got: {msg}");
    }

    #[tokio::test]
    async fn a_port_with_nothing_on_it_fails_with_h503() {
        // Port 1 is privileged and unbound: the connection is refused
        // immediately, so this exercises the timeout arm without waiting.
        let err = wait_until_ready(1, Duration::from_millis(1))
            .await
            .expect_err("an unbound port SHALL NOT read as ready");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::HostDevServerUnavailable),
            "the failure SHALL be tagged H503: {err:#}"
        );
    }

    // -- bundle health ---------------------------------------------

    /// Metro's real 500 body for a parse error, captured from `test-app-e`
    /// on Expo 57 (code frame truncated, escapes left as Metro sends them).
    /// Metro's real 500 body for a parse error, captured from
    /// `test-app-e` on Expo 57. The ANSI escapes are Metro's own, and are
    /// why the parser has to strip them.
    const REAL_TRANSFORM_ERROR: &str = "{\"type\":\"TransformError\",\"lineNumber\":9,\"column\":15,\"filename\":\"App.tsx\",\"name\":\"SyntaxError\",\"message\":\"SyntaxError: /p/test-app-e/App.tsx: Unexpected token (9:15)\\n\\n\\u001b[0m \\u001b[90m  7 |\\u001b[39m a comment\\n\"}";

    #[test]
    fn a_metro_transform_error_yields_its_first_line_without_escapes() {
        let msg = parse_bundle_error(REAL_TRANSFORM_ERROR)
            .expect("a TransformError body SHALL be recognised");
        assert_eq!(
            msg, "SyntaxError: /p/test-app-e/App.tsx: Unexpected token (9:15)",
            "the code frame SHALL be dropped"
        );
        assert!(
            !msg.contains('\u{1b}'),
            "ANSI escapes SHALL NOT reach a report: {msg:?}"
        );
    }

    #[test]
    fn a_body_that_is_not_a_metro_error_is_no_evidence() {
        // Something else answering 500 on the port says nothing about the app.
        assert_eq!(parse_bundle_error("<html>502 Bad Gateway</html>"), None);
        assert_eq!(parse_bundle_error("{}"), None);
        assert_eq!(parse_bundle_error(r#"{"type":"SomethingElse"}"#), None);
        assert_eq!(parse_bundle_error(""), None);
    }

    #[test]
    fn a_transform_error_without_a_message_falls_back_to_its_fields() {
        let body = r#"{"type":"TransformError","name":"SyntaxError","filename":"App.tsx","lineNumber":9,"column":15}"#;
        assert_eq!(
            parse_bundle_error(body).as_deref(),
            Some("SyntaxError: App.tsx (9:15)")
        );
    }

    #[test]
    fn the_bundle_url_is_the_entry_the_app_itself_fetches() {
        let url = bundle_url(8081, "ios");
        assert!(url.starts_with("http://127.0.0.1:8081/.expo/.virtual-metro-entry.bundle"));
        assert!(url.contains("platform=ios"), "got: {url}");
        assert!(url.contains("dev=true"), "got: {url}");
    }

    #[tokio::test]
    async fn a_healthy_bundle_is_not_reported_as_broken() {
        let (port, _guard) =
            serve_once("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await;
        check_bundle(port, "android")
            .await
            .expect("a 200 SHALL NOT be reported as a broken bundle");
    }

    #[tokio::test]
    async fn a_broken_bundle_fails_with_a501_and_the_real_message() {
        let body = REAL_TRANSFORM_ERROR;
        let resp = format!(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (port, _guard) = serve_once(&resp).await;
        let err = check_bundle(port, "ios")
            .await
            .expect_err("a Metro 500 SHALL fail the run");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::AppDevBundleError),
            "SHALL be tagged A501: {err:#}"
        );
        assert!(
            format!("{err:#}").contains("Unexpected token (9:15)"),
            "the developer's own error SHALL be quoted: {err:#}"
        );
    }

    #[tokio::test]
    async fn a_404_entry_point_is_not_treated_as_a_broken_bundle() {
        // A bare React Native project has no Expo virtual entry. Unknown is
        // not the same as broken.
        let (port, _guard) =
            serve_once("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        check_bundle(port, "android")
            .await
            .expect("a 404 SHALL NOT be reported as a broken bundle");
    }

    #[tokio::test]
    async fn an_unreachable_port_is_not_treated_as_a_broken_bundle() {
        // The readiness probe already owns "no dev server"; this must not
        // report the same problem a second time under a different code.
        check_bundle(1, "android")
            .await
            .expect("an unreachable server SHALL NOT read as a broken bundle");
    }

    /// Serve one canned HTTP response per connection until the task is dropped.
    async fn serve_once(response: &str) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        let body = response.to_string();
        let handle = tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(body.as_bytes()).await;
                let _ = sock.flush().await;
            }
        });
        (port, handle)
    }

    #[tokio::test]
    async fn a_server_answering_packager_status_reads_as_ready() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                use tokio::io::AsyncWriteExt;
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 23\r\n\r\npackager-status:running",
                    )
                    .await;
                let _ = sock.flush().await;
            }
        });
        wait_until_ready(port, Duration::from_secs(5))
            .await
            .expect("a packager-status:running server SHALL read as ready");
    }
}
