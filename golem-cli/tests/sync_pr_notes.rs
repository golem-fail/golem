//! Runs the trailer-sync suite (`scripts/tests/sync-pr-notes.test.sh`) under
//! `cargo t`. `sync-pr-notes.sh` REWRITES a PR description in place, so a bug
//! there mangles what an author wrote — and it had no tests at all until #213
//! required changing its marker matching.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn trailer_sync_merges_notes_without_eating_the_body() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-cli has a parent directory")
        .to_path_buf();
    let harness = repo_root.join("scripts/tests/sync-pr-notes.test.sh");

    let out = Command::new("bash")
        .arg(&harness)
        .current_dir(&repo_root)
        .output()
        .expect("the sync-pr-notes harness SHALL be runnable");

    assert!(
        out.status.success(),
        "sync-pr-notes.test.sh failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
