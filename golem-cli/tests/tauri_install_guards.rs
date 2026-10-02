//! Runs the Tauri install-script guard suite
//! (`scripts/tests/tauri-install-guards.test.sh`) under `cargo t`: the iOS
//! stale-bundle guards, the Android APK pick and freshness guard, and the
//! `BUILD_TYPE` / `TAURI_BUILD_*` mapping onto tauri's build flags.
//!
//! The guards exist because the pipeline once installed weeks-old bundles
//! silently (#189). Two of the three holes that remained live in the shell
//! script: the `tauri-cli` failure tolerance was wide enough to accept a
//! genuine build failure, and nothing checked that the web assets the
//! `.app` embeds were the ones this run produced.
//!
//! Tested at the shell level because that is where they live, and against
//! the TEMPLATE rather than the rendered copy: `{{TAURI_CMD}}` is the seam
//! that lets a case script an arbitrary build outcome with no Xcode, no
//! simulator and no Tauri. `install_freshness.rs` pins the rendered
//! `scripts/install-app.sh` to that same template, so both are covered.
//!
//! Nextest-SLOW (>2s) under the full parallel suite, ~1.8s on its own: it
//! is 7 cases, each forking bash/sed/find/grep against a throwaway project
//! dir, so the cost is process spawning under contention. Same shape as
//! the install-freshness and patch-test-app-projects harnesses beside it.
//! The stubs and the rendered script are already created once rather than
//! per case, which took it from ~4s to ~1.8s — writing a fresh executable
//! per case cost ~350ms each in exec overhead alone. What remains is one
//! bash per outcome, and the outcomes are the point.

use std::path::PathBuf;
use std::process::Command;

use golem_cli::scaffold::{render_install_script, InstallFramework};

#[test]
fn the_stale_artifact_guards_hold_in_every_build_outcome() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-cli has a parent directory")
        .to_path_buf();
    let harness = repo_root.join("scripts/tests/tauri-install-guards.test.sh");

    // Partials resolved, placeholders left for the harness to fill: the
    // harness must test what `golem install-script` emits, and a second
    // splice implementation in shell could drift from this one.
    let template = tempfile::NamedTempFile::new().expect("temp template");
    std::fs::write(
        template.path(),
        render_install_script(InstallFramework::Tauri, &[]).expect("render tauri template"),
    )
    .expect("write temp template");

    let out = Command::new("bash")
        .arg(&harness)
        .env("TAURI_TEMPLATE", template.path())
        .current_dir(&repo_root)
        .output()
        .expect("the tauri-install-guards harness SHALL be runnable");

    assert!(
        out.status.success(),
        "tauri-install-guards.test.sh failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
