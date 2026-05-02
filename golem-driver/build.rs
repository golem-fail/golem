use std::path::Path;

fn main() {
    // Self-reference first so edits to this script (different minifier
    // options, output path changes, etc.) trigger a rebuild even when
    // the JS source hasn't moved.
    println!("cargo:rerun-if-changed=build.rs");
    let src_path = "src/dom_traversal.js";
    println!("cargo:rerun-if-changed={src_path}");

    let source = std::fs::read_to_string(src_path)
        .expect("src/dom_traversal.js missing");
    let minified = minifier::js::minify(&source).to_string();

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_path = Path::new(&out_dir).join("dom_traversal.min.js");
    std::fs::write(&out_path, minified).expect("failed to write minified JS");
}
