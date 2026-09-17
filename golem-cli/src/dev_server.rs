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
