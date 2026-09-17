//! Integration: `golem run --dev` — the dev-server preflight and the fact
//! that the flag survives the daemon socket.
//!
//! The bundler is faked with a plain TCP listener answering Metro's
//! `/status`, so these stay device-free and need no `expo start`. What they
//! can't cover is a real bundle download; that lives on the e2e sweep.
//!
//! Deliberately *not* here: a daemon-parity case. Stub mode returns before
//! the install pipeline, which is the only place `--dev` changes what the
//! server does, so such a test would pass whether or not the flag crossed
//! the socket — it was written, sabotage-checked, and found vacuous. The
//! wire is pinned instead by the paired unit tests
//! `build_config_json_carries_dev_mode` and `parse_submit_config_carries_dev_mode`,
//! which do fail when a key is dropped from either half.
//!
//! nextest-SLOW by nature: each case drives a full run.

mod common;

use common::{read_results_json, run_stub};

/// A listener answering `packager-status:running` on a loopback port, for as
/// long as the returned guard is alive. Metro's readiness probe is a plain
/// GET, so a hand-rolled response is a faithful stand-in.
struct FakeBundler {
    port: u16,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Metro's 500 body for a file that doesn't parse, as captured from
/// `test-app-e`. Kept here in the shape the CLI will actually receive.
const TRANSFORM_ERROR_BODY: &str = "{\"type\":\"TransformError\",\"lineNumber\":9,\"column\":15,\"filename\":\"App.tsx\",\"name\":\"SyntaxError\",\"message\":\"SyntaxError: /p/App.tsx: Unexpected token (9:15)\"}";

impl FakeBundler {
    /// A bundler whose app builds cleanly.
    fn start() -> Self {
        Self::with_bundle_health(true)
    }

    /// A bundler that is up and healthy but cannot build the app's bundle.
    fn start_with_broken_bundle() -> Self {
        Self::with_bundle_health(false)
    }

    fn with_bundle_health(bundle_ok: bool) -> Self {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        // Non-blocking so the accept loop can notice the shutdown flag
        // instead of parking forever on a port nobody probes again.
        listener
            .set_nonblocking(true)
            .expect("listener SHALL be non-blocking");
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = shutdown.clone();
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut sock, _)) => {
                        let mut buf = [0u8; 2048];
                        let _ = sock.set_nonblocking(false);
                        let n = sock.read(&mut buf).unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]).to_string();
                        let resp = if req.contains("/status") {
                            "HTTP/1.1 200 OK\r\nContent-Length: 23\r\nConnection: close\r\n\r\n\
                             packager-status:running"
                                .to_string()
                        } else if bundle_ok {
                            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                                .to_string()
                        } else {
                            format!(
                                "HTTP/1.1 500 Internal Server Error\r\n\
                                 Content-Type: application/json\r\nContent-Length: {}\r\n\
                                 Connection: close\r\n\r\n{TRANSFORM_ERROR_BODY}",
                                TRANSFORM_ERROR_BODY.len()
                            )
                        };
                        let _ = sock.write_all(resp.as_bytes());
                        let _ = sock.flush();
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            }
        });
        Self { port, shutdown }
    }

    fn port_arg(&self) -> String {
        self.port.to_string()
    }
}

impl Drop for FakeBundler {
    fn drop(&mut self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A loopback port with nothing listening on it.
fn unbound_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = l.local_addr().expect("addr").port();
    drop(l);
    port
}

#[test]
fn dev_runs_the_suite_when_the_dev_server_answers() {
    let bundler = FakeBundler::start();
    let r = run_stub("", &["--dev", "--dev-port", &bundler.port_arg()]);
    assert_eq!(r.code, 0, "a --dev run SHALL pass; stderr={}", r.stderr);

    let v = read_results_json(&r, "");
    assert_eq!(
        v["suite"]["passed"], 1,
        "the flow SHALL have actually run; json={v}"
    );
    // The install pipeline is skipped the way `--no-build` skips it, but says
    // so in the user's own words.
    assert!(
        !r.stderr.contains("--no-build"),
        "a --dev run SHALL NOT mention a flag the user never passed; stderr={}",
        r.stderr
    );
}

#[test]
fn dev_fails_with_an_actionable_message_when_no_dev_server_answers() {
    let port = unbound_port();
    // A short wait: the subject is the message, not the patience.
    let r = run_stub(
        "",
        &["--dev", "--dev-port", &port.to_string(), "--dev-wait", "1s"],
    );

    assert_eq!(r.code, 1, "a --dev run without a bundler SHALL fail");
    // The whole value of this failure is naming what to start, so the message
    // is asserted rather than just the exit code.
    assert!(
        r.stderr.contains(&format!("127.0.0.1:{port}/status")),
        "the failure SHALL name the URL it probed; stderr={}",
        r.stderr
    );
    assert!(
        r.stderr.contains("npx expo start"),
        "the failure SHALL name the command that fixes it; stderr={}",
        r.stderr
    );
    // It must not be reported as a flow problem — that misdiagnosis is the
    // reason this mode exists.
    assert!(
        !r.stderr.contains("EF408"),
        "a missing dev server SHALL NOT read as a step timeout; stderr={}",
        r.stderr
    );
}

#[test]
fn a_bundle_that_does_not_build_fails_before_any_flow_runs() {
    // The iOS case has no other signal: there the error overlay renders
    // nothing golem's tree or a screenshot can see, so the bundler's own
    // answer is the only evidence that exists.
    let bundler = FakeBundler::start_with_broken_bundle();
    let r = run_stub("", &["--dev", "--dev-port", &bundler.port_arg()]);

    assert_eq!(r.code, 1, "a broken bundle SHALL fail the run");
    assert!(
        r.stderr.contains("Unexpected token (9:15)"),
        "the developer's own error SHALL be quoted; stderr={}",
        r.stderr
    );
    assert!(
        r.stderr.contains("cannot build"),
        "the message SHALL say what went wrong; stderr={}",
        r.stderr
    );
    // Failing up front is the point: running every flow against a blank app
    // would bury the one line that explains it.
    assert!(
        !r.stderr.contains("PASS") && !r.stderr.contains("NG "),
        "no flow SHALL have run; stderr={}",
        r.stderr
    );
}
