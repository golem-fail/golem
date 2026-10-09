//! Embeds the docs parts (`docs/src`) for the MCP help, by address.

#![allow(clippy::disallowed_macros)] // cargo reads build-script directives from stdout

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/src");
    println!("cargo:rerun-if-changed={}", src.display());
    let entries = golem_docs::help_entries(&src).expect("docs/src parts");
    let mut out = String::from("pub static HELP_PARTS: &[(&str, &str)] = &[\n");
    for (address, path) in entries {
        let path = path.canonicalize().expect("a docs part path");
        let _ = writeln!(
            out,
            "    ({address:?}, include_str!({:?})),",
            path.display().to_string()
        );
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("help_parts.rs");
    std::fs::write(dest, out).expect("write help_parts.rs");
}
