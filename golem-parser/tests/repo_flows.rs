//! Parses every `*.test.toml` checked into the repo.
//!
//! The selector structs reject unknown keys (#32), and the failure mode of
//! that strictness is the mirror of the bug it fixes: a key that is actually
//! valid but missing from a struct turns a working flow into a parse error.
//! The repo's own flows are the corpus that catches it — they exercise the
//! selector forms in the shapes people write, which the unit tests only
//! approximate. Reading files is cheap; no device, no build.

use std::path::{Path, PathBuf};

fn collect_flows(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            // Build output and vendored deps hold no flows of ours.
            if !matches!(
                name.to_string_lossy().as_ref(),
                "target" | "node_modules" | ".git"
            ) {
                collect_flows(&path, out);
            }
        } else if path.to_string_lossy().ends_with(".test.toml") {
            out.push(path);
        }
    }
}

#[test]
fn every_checked_in_flow_still_parses() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-parser has a parent directory")
        .to_path_buf();

    let mut flows = Vec::new();
    collect_flows(&repo_root, &mut flows);
    flows.sort();
    assert!(
        flows.len() >= 20,
        "expected the repo's e2e corpus, found {} flows — did the walk break?",
        flows.len()
    );

    let failures: Vec<String> = flows
        .iter()
        .filter_map(|path| {
            let body = std::fs::read_to_string(path).ok()?;
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
