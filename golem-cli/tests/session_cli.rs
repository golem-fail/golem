//! `golem session` as real processes: each command is a new process and a
//! new daemon connection, and the named session lives in the daemon
//! between them.

#![cfg(unix)]

mod common;

use std::process::{Command, Output};

fn golem(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_golem"))
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .env("GOLEM_SOCKET", dir.join("d.sock"))
        .env("GOLEM_DAEMON_IDLE_SECS", "1")
        .env_remove("GOLEM_DAEMON_IN_PROCESS")
        .output()
        .expect("golem")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn a_session_carries_state_across_commands() {
    let dir = tempfile::Builder::new()
        .prefix("gsc")
        .tempdir_in("/tmp")
        .expect("tempdir");
    std::fs::write(dir.path().join("golem.toml"), common::golem_toml()).expect("golem.toml");
    let d = dir.path();

    // A failure before `stop` leaves the daemon up until the idle timeout.
    let start = golem(d, &["session", "start", "--stub", "--idle-timeout", "10"]);
    assert!(start.status.success(), "{start:?}");
    assert!(
        stdout(&start).starts_with("session open · android/Stub Device"),
        "{start:?}"
    );

    let read = golem(
        d,
        &[
            "session",
            "do",
            r#"{ action = "read", on_text = "Submit", save_to = "label" }"#,
        ],
    );
    assert!(read.status.success(), "{read:?}");
    let tap = golem(
        d,
        &[
            "session",
            "do",
            r#"{ action = "tap", on_text = "${label}" }"#,
            "--comment",
            "Send",
        ],
    );
    assert!(tap.status.success(), "the var SHALL carry over: {tap:?}");
    assert!(stdout(&tap).starts_with("+tap:"), "{tap:?}");

    let missing = golem(
        d,
        &[
            "session",
            "do",
            r#"{ action = "tap", on_text = "Nope", timeout = 100 }"#,
        ],
    );
    assert_eq!(
        missing.status.code(),
        Some(1),
        "a failed step SHALL exit 1: {missing:?}"
    );

    let list = golem(d, &["session", "list"]);
    assert!(
        stdout(&list).starts_with("default · android/Stub Device · idle"),
        "{list:?}"
    );

    let out = d.join("flows/session.test.toml");
    let export = golem(d, &["session", "export", "flows/session.test.toml"]);
    assert!(export.status.success(), "{export:?}");
    let flow = std::fs::read_to_string(&out).expect("exported");
    assert!(flow.contains("# Send"), "{flow}");

    let shot = golem(d, &["session", "screenshot", "shot.png"]);
    assert!(shot.status.success(), "{shot:?}");
    assert!(d.join("shot.png").exists());

    let stop = golem(d, &["session", "stop"]);
    assert!(stop.status.success(), "{stop:?}");
    assert_eq!(stdout(&stop), "session default stopped\n");

    let after = golem(d, &["session", "tree"]);
    assert!(!after.status.success());
    assert!(
        String::from_utf8_lossy(&after.stderr).contains("golem session start"),
        "{after:?}"
    );
}

#[test]
fn a_command_without_a_session_says_how_to_start_one() {
    let dir = tempfile::Builder::new()
        .prefix("gsn")
        .tempdir_in("/tmp")
        .expect("tempdir");
    let tree = golem(dir.path(), &["session", "tree", "--name", "other"]);
    assert!(!tree.status.success());
    assert!(
        String::from_utf8_lossy(&tree.stderr).contains("golem session start"),
        "{tree:?}"
    );
    let list = golem(dir.path(), &["session", "list"]);
    assert_eq!(stdout(&list), "no sessions\n");
}
