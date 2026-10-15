//! Integration: an interactive command without a terminal fails at once,
//! and names a way on.

use std::process::{Command, Stdio};

fn golem_without_a_terminal(args: &[&str]) -> (i32, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(env!("CARGO_BIN_EXE_golem"))
        .args(args)
        .current_dir(dir.path())
        .env("GOLEM_CONFIG", dir.path().join("config.toml"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("golem runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn each_interactive_command_names_a_way_on_without_a_terminal() {
    for (args, way_on) in [
        (&["install-script"][..], "golem install-script --print expo"),
        (&["mcp-config"][..], "golem mcp-config show"),
        (
            &["mcp-config", "set", "shell", "allow"][..],
            "golem mcp-config set shell allow",
        ),
    ] {
        let (code, stderr) = golem_without_a_terminal(args);
        assert_eq!(code, 1, "{args:?}: {stderr}");
        assert!(stderr.contains(way_on), "{args:?}: {stderr}");
        assert!(stderr.contains("in a terminal"), "{args:?}: {stderr}");
    }
}
