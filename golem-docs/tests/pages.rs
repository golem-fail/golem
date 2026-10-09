//! The pages in `docs/` match their parts in `docs/src/`, and their links
//! resolve.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn docs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs")
}

fn rendered() -> BTreeMap<String, String> {
    golem_docs::pages(&docs().join("src"))
        .expect("docs/src reads")
        .iter()
        .map(|p| (format!("{}.md", p.name), p.render()))
        .collect()
}

/// The error code registry part, from `FailureCode`.
fn registry() -> String {
    use golem_events::FailureCode;
    let mut out = String::from(
        "## Registry\n\n\
         <!-- Generated from FailureCode in golem-events/src/code.rs (meaning, fix). -->\n\n\
         | Code | Meaning | Fix |\n|------|---------|-----|\n",
    );
    for code in FailureCode::ALL {
        out.push_str(&format!(
            "| `{}` | {} | {} |\n",
            code.fragment(),
            code.meaning(),
            code.fix()
        ));
    }
    out
}

const REGISTRY_PART: &str = "src/error-codes/10-registry.md";

#[test]
fn each_generated_page_matches_its_parts() {
    let update = std::env::var_os("GOLEM_UPDATE_DOCS").is_some();
    let mut stale = Vec::new();
    let part = docs().join(REGISTRY_PART);
    if std::fs::read_to_string(&part).ok() != Some(registry()) {
        if update {
            std::fs::write(&part, registry()).expect("write the registry part");
        } else {
            stale.push(REGISTRY_PART.to_string());
        }
    }
    for (file, text) in rendered() {
        let path = docs().join(&file);
        if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            continue;
        }
        if update {
            std::fs::write(&path, text).expect("write the page");
        } else {
            stale.push(file);
        }
    }
    assert!(
        stale.is_empty(),
        "docs/{stale:?} differ from their source. Edit the parts in docs/src (the error \
         code registry: FailureCode), not the page, then run: \
         GOLEM_UPDATE_DOCS=1 cargo nextest run -p golem-docs"
    );
}

#[test]
fn each_link_in_a_generated_page_resolves() {
    let broken = golem_docs::broken_links(&docs(), &rendered());
    assert!(broken.is_empty(), "broken links:\n{}", broken.join("\n"));
}

#[test]
fn each_part_has_its_own_address() {
    let entries = golem_docs::help_entries(&docs().join("src")).expect("docs/src reads");
    let mut seen = BTreeSet::new();
    for (address, _) in &entries {
        assert!(seen.insert(address), "two parts have the address {address}");
    }
}
