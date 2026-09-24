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

/// The generator has to map each commit back to the PR whose body holds its
/// authored notes. It reads the "(#N)" a squash merge appends — and when that
/// is absent, asks the API. Getting this wrong is silent: the notes are not
/// missing, they are replaced by the commit subject under whatever bucket its
/// conventional type maps to, which is how #219 shipped as a `fix(` subject
/// under Fixed when its block said `internal`.
///
/// ~1.1s alone, nextest-SLOW (>2s) under the full parallel suite: it builds a
/// four-commit git repo and runs the real generator over it, so the cost is
/// process spawning under contention. Driving the entry point is the point —
/// the bug it pins was in which PR the loop asked for, which a parser-level
/// test never reaches.
#[test]
fn release_notes_take_their_text_from_the_right_pr() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-cli has a parent directory")
        .to_path_buf();
    let harness = repo_root.join("scripts/tests/release-notes-pr-lookup.test.sh");

    let out = Command::new("bash")
        .arg(&harness)
        .current_dir(&repo_root)
        .output()
        .expect("the release-notes PR-lookup harness SHALL be runnable");

    assert!(
        out.status.success(),
        "release-notes-pr-lookup.test.sh failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
