//! The detached daemon as real processes: `golem run` starts `golem daemon`,
//! returns without waiting for it, and the daemon exits once idle.
//!
//! Runs the built binary rather than the in-process harness: what this
//! covers is the process boundary itself — a detached child that does not
//! hold the client's stdout open, and that outlives the client.

#![cfg(unix)]

mod common;

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn project() -> tempfile::TempDir {
    // Short: macOS caps a unix socket path near 104 bytes.
    let dir = tempfile::Builder::new()
        .prefix("gdp")
        .tempdir_in("/tmp")
        .expect("tempdir");
    std::fs::write(dir.path().join("golem.toml"), common::golem_toml()).expect("golem.toml");
    std::fs::write(dir.path().join("f.test.toml"), common::fixture_flow()).expect("flow");
    std::fs::write(dir.path().join("stub.toml"), "").expect("stub");
    dir
}

fn golem(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_golem"));
    cmd.current_dir(dir)
        .env("HOME", dir)
        .env("GOLEM_SOCKET", dir.join("d.sock"))
        .env("GOLEM_DAEMON_IDLE_SECS", "1")
        .env_remove("GOLEM_DAEMON_IN_PROCESS")
        .stdin(Stdio::null());
    cmd
}

fn wait_until(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < limit, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_run_starts_a_detached_daemon_that_exits_when_idle() {
    let dir = project();
    let socket = dir.path().join("d.sock");

    // `output()` pipes stdout and stderr and reads them to EOF: a daemon
    // that inherited either pipe would hold this call open until it exits.
    let started = Instant::now();
    let out = golem(dir.path())
        .args([
            "run",
            "f.test.toml",
            "--stub",
            "stub.toml",
            "--platform",
            "android",
        ])
        .args(["--output", "toon"])
        .output()
        .expect("golem run");
    let took = started.elapsed();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("PASS"),
        "the run's TOON report SHALL be on stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(socket.exists(), "the daemon SHALL outlive the run");

    let log = std::fs::read_to_string(dir.path().join("d.log")).unwrap_or_default();
    assert!(log.contains("listening on"), "daemon log: {log}");

    wait_until("the idle daemon to exit", Duration::from_secs(10), || {
        !socket.exists()
    });
    assert!(
        took < Duration::from_secs(20),
        "the run SHALL not wait for the daemon: took {took:?}"
    );
}

#[test]
fn concurrent_runs_share_one_daemon() {
    let dir = project();
    let children: Vec<_> = (0..3)
        .map(|_| {
            golem(dir.path())
                .args([
                    "run",
                    "f.test.toml",
                    "--stub",
                    "stub.toml",
                    "--platform",
                    "android",
                ])
                .args(["--output", "toon", "--no-results"])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn")
        })
        .collect();
    for child in children {
        let out = child.wait_with_output().expect("wait");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let log = std::fs::read_to_string(dir.path().join("d.log")).unwrap_or_default();
    assert_eq!(
        log.matches("listening on").count(),
        1,
        "one daemon SHALL serve every run: {log}"
    );
}
