//! Runs the install-script freshness suite (`scripts/tests/install-freshness.test.sh`)
//! so the shipped templates are covered by `cargo t` rather than only by an
//! e2e run that takes minutes and a device.
//!
//! The harness sources the freshness helpers out of each template into a
//! throwaway project with `npm install` and `expo prebuild` stubbed as
//! recorders, so it asserts on what the script DECIDED to do without running
//! either. That keeps it fast enough not to be nextest-SLOW, unlike the
//! release-notes harness beside it.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn install_templates_rebuild_when_their_inputs_change() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-cli has a parent directory")
        .to_path_buf();
    let harness = repo_root.join("scripts/tests/install-freshness.test.sh");

    let out = Command::new("bash")
        .arg(&harness)
        .current_dir(&repo_root)
        .output()
        .expect("the install-freshness test harness SHALL be runnable");

    assert!(
        out.status.success(),
        "install-freshness.test.sh failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// The repo's own install scripts are the templates with placeholders filled
/// in. They drift silently otherwise: a fix to the template would ship to
/// scaffolded projects while this repo's own e2e kept exercising the old
/// behaviour — which is exactly how the staleness bug survived.
#[test]
fn the_repos_install_scripts_match_the_templates_they_were_rendered_from() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("golem-cli has a parent directory")
        .to_path_buf();

    let cases: [(&str, &str, &[(&str, &str)]); 2] = [
        (
            "golem-cli/templates/install-scripts/expo.sh",
            "scripts/install-app-e.sh",
            &[
                ("EXPO_DIR", "test-app-e"),
                ("PM_RUNNER", "npx expo"),
                ("PM_INSTALL", "npm install"),
                ("IOS_SCHEME", ""),
            ],
        ),
        (
            "golem-cli/templates/install-scripts/tauri.sh",
            "scripts/install-app.sh",
            &[
                ("TAURI_DIR", "test-app"),
                ("IOS_SCHEME", "app_iOS"),
                ("TAURI_CMD", "cargo tauri"),
                ("PM_INSTALL", "npm install"),
            ],
        ),
    ];

    for (template, rendered, placeholders) in cases {
        let tmpl = std::fs::read_to_string(repo_root.join(template))
            .unwrap_or_else(|e| panic!("read {template}: {e}"));
        let actual = std::fs::read_to_string(repo_root.join(rendered))
            .unwrap_or_else(|e| panic!("read {rendered}: {e}"));

        let mut expected = tmpl;
        for (key, value) in placeholders {
            expected = expected.replace(&format!("{{{{{key}}}}}"), value);
        }

        assert!(
            !expected.contains("{{"),
            "{template} has a placeholder this test doesn't fill — add it here"
        );
        assert_eq!(
            expected, actual,
            "{rendered} is out of date with {template}; re-render it"
        );
    }
}
