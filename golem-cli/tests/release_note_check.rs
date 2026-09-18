//! Runs the release-note gate's own suite (`scripts/tests/release-note-check.test.sh`)
//! so `cargo t` covers the logic behind a REQUIRED status check.
//!
//! It used to live inline in `.github/workflows/release-note-check.yml`, where
//! nothing could exercise it — and it was silently broken: under
//! `set -euo pipefail` a body with no marker aborted the script at the first
//! `grep`, so the most common mistake (no block at all) failed the check with
//! no error message. The first run of this harness caught it.
//!
//! nextest-SLOW, deliberately. The harness runs in ~0.7s alone and crosses the
//! threshold only through process-spawn contention under the full parallel
//! suite — the same effect measured in #66. The cases already avoid per-case
//! spawns by sourcing the script and calling it in a subshell; what remains is
//! the one `bash` the driver starts, which is irreducible for shell-under-test.
//! Same shape, and same trade, as `install_freshness` and `release_notes`.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn release_note_gate_accepts_and_rejects_the_right_pr_bodies() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-cli has a parent directory")
        .to_path_buf();
    let harness = repo_root.join("scripts/tests/release-note-check.test.sh");

    let out = Command::new("bash")
        .arg(&harness)
        .current_dir(&repo_root)
        .output()
        .expect("the release-note-check harness SHALL be runnable");

    assert!(
        out.status.success(),
        "release-note-check.test.sh failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
