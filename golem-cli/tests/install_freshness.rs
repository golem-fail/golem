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

use golem_cli::scaffold::{render_install_script, InstallFramework};

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

/// A template, the repo script rendered from it, and the placeholder values.
type RenderedScript = (
    InstallFramework,
    &'static str,
    &'static [(&'static str, &'static str)],
);

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

    let cases: [RenderedScript; 6] = [
        (
            InstallFramework::Expo,
            "scripts/install-app-e.sh",
            &[
                ("EXPO_DIR", "test-app-e"),
                ("PM_RUNNER", "npx expo"),
                ("PM_INSTALL", "npm install"),
                ("IOS_SCHEME", ""),
            ],
        ),
        (
            InstallFramework::Tauri,
            "scripts/install-app.sh",
            &[
                ("TAURI_DIR", "test-app"),
                ("IOS_SCHEME", "app_iOS"),
                ("TAURI_CMD", "cargo tauri"),
                ("PM_INSTALL", "npm install"),
            ],
        ),
        (
            InstallFramework::NativeAndroid,
            "scripts/install-app-b-android.sh",
            &[
                ("GRADLE_ROOT", "test-app-b/android"),
                ("MODULE_NAME", "app"),
            ],
        ),
        (
            InstallFramework::NativeIos,
            "scripts/install-app-b-ios.sh",
            &[
                ("XCODE_PROJECT", "test-app-b/ios/GolemTestB.xcodeproj"),
                ("XCODE_SCHEME", "GolemTestB"),
                ("CONFIGURATION", "Debug"),
            ],
        ),
        (
            InstallFramework::Capacitor,
            "scripts/install-app-c.sh",
            &[
                ("CAP_DIR", "test-app-c"),
                ("CAP_CMD", "npx cap"),
                ("PM_INSTALL", "npm install"),
                ("WEB_BUILD", "npm run build"),
                ("WEB_DIR", "www"),
            ],
        ),
        (
            InstallFramework::Kmp,
            "scripts/install-app-k.sh",
            &[
                ("KMP_DIR", "test-app-k"),
                ("ANDROID_MODULE", "androidApp"),
                ("IOS_DIR", "iosApp"),
                ("XCODE_SCHEME", "iosApp"),
            ],
        ),
    ];

    for (framework, rendered, placeholders) in cases {
        let template = framework.label();
        let actual = std::fs::read_to_string(repo_root.join(rendered))
            .unwrap_or_else(|e| panic!("read {rendered}: {e}"));

        let expected = render_install_script(framework, placeholders)
            .unwrap_or_else(|e| panic!("render {template}: {e}"));

        assert!(
            !expected.contains("{{"),
            "{template} has a placeholder this test doesn't fill — add it here"
        );
        assert_eq!(
            expected, actual,
            "{rendered} is out of date with the {template} template; re-render it"
        );
    }
}
