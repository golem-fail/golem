//! Guards the two test-app declarations that Tauri gives us no supported
//! place to put, and that a regeneration is happy to delete.
//!
//! The `golem-test://` scheme the `deep_link` flows drive has to be declared
//! per platform, and neither declaration can live in `tauri.conf.json`: a
//! `plugins.deep-link.mobile` entry is a universal-link associated domain
//! (`host`/`pathPrefix`), and the `schemes` key `tauri-plugin-deep-link`
//! actually reads sits under `desktop`. A `src-tauri/Info.ios.plist` is not
//! merged by tauri-cli 2.11 either — verified by building with one; the key
//! never reached the bundle. So both declarations live in generated files
//! that are tracked by a `.gitignore` carve-out.
//!
//! What makes them fragile is different per platform, and the Android half
//! is the subtle one: `tauri-plugin-deep-link`'s build script rewrites
//! everything between the two `DEEP LINK PLUGIN. AUTO-GENERATED` markers
//! from `tauri.conf.json` every time that crate is rebuilt — a fresh clone,
//! a `cargo clean`, a dependency bump. The scheme used to be declared inside
//! that block, so it was deleted without warning and `deep_link.test.toml`
//! started failing on a machine that had done nothing wrong. Being in the
//! file is therefore not enough; being OUTSIDE the block is the property
//! that holds.
//!
//! Reading two files, no device, no build.

use std::path::PathBuf;

const MARKER: &str = "DEEP LINK PLUGIN. AUTO-GENERATED";
const SCHEME: &str = "golem-test";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-cli has a parent directory")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} SHALL be tracked and readable: {e}", path.display()))
}

#[test]
fn android_manifest_declares_the_custom_scheme_outside_the_generated_block() {
    let manifest = read("test-app/src-tauri/gen/android/app/src/main/AndroidManifest.xml");

    let open = manifest
        .find(MARKER)
        .expect("the deep-link plugin's opening marker SHALL still be present");
    let close = manifest[open + MARKER.len()..]
        .find(MARKER)
        .map(|i| open + MARKER.len() + i)
        .expect("the deep-link plugin's closing marker SHALL still be present");

    let declaration = format!(r#"android:scheme="{SCHEME}""#);
    let at = manifest
        .find(&declaration)
        .unwrap_or_else(|| panic!("the manifest SHALL declare {declaration}"));

    assert!(
        at < open || at > close,
        "the {SCHEME} scheme is declared INSIDE the plugin's auto-generated \
         block (bytes {open}..{close}); the plugin's build script rewrites \
         that region from tauri.conf.json and will delete it. Move the \
         declaration to its own <intent-filter> outside the markers."
    );
}

#[test]
fn android_manifest_declares_the_permissions_the_grant_flows_need() {
    let manifest = read("test-app/src-tauri/gen/android/app/src/main/AndroidManifest.xml");
    // `pm grant` refuses a permission the package never declared, so these
    // are what `e2e/links/permissions_*.test.toml` actually rests on.
    for permission in [
        "android.permission.CAMERA",
        "android.permission.RECORD_AUDIO",
        "android.permission.ACCESS_FINE_LOCATION",
        "android.permission.ACCESS_COARSE_LOCATION",
        "android.permission.READ_MEDIA_IMAGES",
    ] {
        assert!(
            manifest.contains(&format!(r#"android:name="{permission}""#)),
            "the manifest SHALL declare {permission} — pm grant refuses an \
             undeclared permission"
        );
    }
}

#[test]
fn ios_plist_registers_the_custom_url_scheme() {
    let plist = read("test-app/src-tauri/gen/apple/golem-test-app_iOS/Info.plist");

    let types = plist
        .find("CFBundleURLTypes")
        .expect("the plist SHALL declare CFBundleURLTypes");
    let scheme = plist
        .find(&format!("<string>{SCHEME}</string>"))
        .unwrap_or_else(|| {
            panic!(
                "the plist SHALL register the {SCHEME} scheme — without it \
                 `simctl openurl` fails with LSApplicationWorkspaceErrorDomain 115"
            )
        });
    assert!(
        scheme > types,
        "the {SCHEME} string SHALL sit inside the CFBundleURLTypes array"
    );
}
