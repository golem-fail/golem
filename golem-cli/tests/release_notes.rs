//! Runs the release-notes lockfile-parser suite (`scripts/tests/release-notes.test.sh`)
//! so repo tooling is covered by `cargo t` rather than living outside the gate.
//!
//! The shell harness builds a throwaway git repo, tags two commits and diffs
//! them through the real `release-notes.sh` entry point — hence nextest-SLOW
//! (a `git init`, two commits, two tags and a bash run). Testing the script's
//! parsers any faster would mean sourcing it, which would require restructuring
//! release-critical tooling for the benefit of the test.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn release_notes_parses_every_lockfile_format() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-cli has a parent directory")
        .to_path_buf();
    let harness = repo_root.join("scripts/tests/release-notes.test.sh");

    let out = Command::new("bash")
        .arg(&harness)
        .current_dir(&repo_root)
        .output()
        .expect("the release-notes test harness SHALL be runnable");

    assert!(
        out.status.success(),
        "release-notes.test.sh failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
