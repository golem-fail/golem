//! Runs the test-app project-patch suite (`scripts/tests/patch-test-app-projects.test.sh`)
//! under `cargo t`, and pins the wiring that makes it run at all.
//!
//! The test app needs two declarations Tauri has no config for — the
//! `uses-permission` lines `pm grant` requires, and the `golem-test://`
//! registration the deep_link flows open. They used to be committed files
//! inside an ignored `gen/`, which handed a clone a partial project tree that
//! `tauri android build` refuses outright (#22). They are re-applied after
//! generation instead, which moves the risk into a shell script and a
//! golem.toml line — both covered here.
//!
//! The harness run is nextest-SLOW (>2s) under the full parallel suite and
//! ~0.7s on its own: it is 25 assertions, each forking bash/awk/grep against
//! a throwaway project dir, so the cost is process spawning under contention
//! rather than anything the test does. Same shape as the install-freshness
//! harness beside it. Trimming assertions would buy back only spawn count,
//! at the price of the states that actually matter — post-init, post-build,
//! and the broken-scheme-inside-the-block case #22 came from.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-cli has a parent directory")
        .to_path_buf()
}

#[test]
fn the_patcher_restores_both_declarations_in_every_project_state() {
    let repo_root = repo_root();
    let harness = repo_root.join("scripts/tests/patch-test-app-projects.test.sh");

    let out = Command::new("bash")
        .arg(&harness)
        .current_dir(&repo_root)
        .output()
        .expect("the patch-test-app-projects harness SHALL be runnable");

    assert!(
        out.status.success(),
        "patch-test-app-projects.test.sh failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// The patcher only protects anything if something calls it. Pointing the app
/// back at `scripts/install-app.sh` would still build and still pass e2e on a
/// machine whose `gen/` tree is already patched — and break silently for
/// everyone else, which is the failure this whole arrangement exists to stop.
#[test]
fn the_tauri_test_app_installs_through_the_wrapper() {
    let config = std::fs::read_to_string(repo_root().join("golem.toml"))
        .expect("golem.toml SHALL be readable");
    assert!(
        config.contains(r#"install_script = "scripts/install-test-app.sh""#),
        "the `app` entry in golem.toml SHALL install through \
         scripts/install-test-app.sh, which generates the native project and \
         re-applies the declarations before handing off to the rendered \
         template"
    );
}
