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

/// The same corpus, against the step-field lint (#216).
///
/// That lint calls a key a typo partly on a one-edit heuristic, so its
/// failure mode is the mirror of the bug: a real action parameter that
/// happens to sit one letter from a field name would warn on every correct
/// flow that uses it. The repo's own flows are the only place the actual
/// parameter vocabulary is exercised in the shapes people write, so they
/// are what says whether the heuristic is safe.
///
/// Scoped to this one lint on purpose — `validate_flow` and the other lints
/// are not asserted clean here, because a flow legitimately failing an
/// unrelated check would then block this one from ever being trusted.
#[test]
fn no_checked_in_flow_trips_the_step_field_lint() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-parser has a parent directory")
        .to_path_buf();

    let listed = Command::new("git")
        .args(["ls-files", "-z", "--", "*.test.toml"])
        .current_dir(&repo_root)
        .output()
        .expect("git ls-files SHALL run inside the repo");

    let mut warnings = Vec::new();
    for rel in String::from_utf8_lossy(&listed.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
    {
        let path = repo_root.join(rel);
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(flow) = golem_parser::parse_flow(&body) else {
            continue;
        };
        for issue in golem_parser::validation::lint_unknown_step_fields(&flow) {
            warnings.push(format!(
                "{rel}:{}::{} `{}` (action `{}`)",
                issue.block_name.as_deref().unwrap_or("<unnamed>"),
                issue.step_index,
                issue.key,
                issue.action,
            ));
        }
    }

    assert!(
        warnings.is_empty(),
        "the step-field lint SHALL NOT fire on checked-in flows:\n{}",
        warnings.join("\n")
    );
}
