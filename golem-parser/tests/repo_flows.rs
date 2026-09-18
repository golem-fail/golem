//! Parses every `*.test.toml` checked into the repo.
//!
//! The selector structs reject unknown keys (#32), and the failure mode of
//! that strictness is the mirror of the bug it fixes: a key that is actually
//! valid but missing from a struct turns a working flow into a parse error.
//! The repo's own flows are the corpus that catches it — they exercise the
//! selector forms in the shapes people write, which the unit tests only
//! approximate. Reading files is cheap; no device, no build.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn every_checked_in_flow_still_parses() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-parser has a parent directory")
        .to_path_buf();

    // Tracked files only, not a filesystem walk: a contributor's scratch or
    // deliberately-malformed flow sitting untracked in their tree is not part
    // of the corpus and SHALL NOT redden this suite.
    let listed = Command::new("git")
        .args(["ls-files", "-z", "--", "*.test.toml"])
        .current_dir(&repo_root)
        .output()
        .expect("git ls-files SHALL run inside the repo");
    assert!(
        listed.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&listed.stderr)
    );

    let flows: Vec<PathBuf> = String::from_utf8_lossy(&listed.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(|p| repo_root.join(p))
        .collect();
    assert!(
        flows.len() >= 20,
        "expected the repo's e2e corpus, found {} flows — did the listing break?",
        flows.len()
    );

    let failures: Vec<String> = flows
        .iter()
        .filter_map(|path| {
            let body = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("{} SHALL be readable: {e}", path.display()));
            let err = golem_parser::parse_flow(&body).err()?;
            Some(format!("{}: {err}", path.display()))
        })
        .collect();

    assert!(
        failures.is_empty(),
        "checked-in flows SHALL parse:\n{}",
        failures.join("\n")
    );
}
