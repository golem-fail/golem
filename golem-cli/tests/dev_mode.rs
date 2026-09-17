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

impl FakeBundler {
    fn start() -> Self {
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
                        let mut buf = [0u8; 1024];
                        let _ = sock.set_nonblocking(false);
                        let _ = sock.read(&mut buf);
                        let _ = sock.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 23\r\nConnection: close\r\n\r\n\
                              packager-status:running",
                        );
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
