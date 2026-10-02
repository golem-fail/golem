//! Runs the rendered native-android install script against stubbed
//! `./gradlew`, `adb` and `aapt2`, so the build/install decisions are covered
//! by `cargo t` with no SDK, no emulator and no Gradle.
//!
//! One project and one set of stubs serve every case, with the behaviour
//! picked through env vars: a freshly written executable costs ~350ms of exec
//! overhead on macOS, and per case that would double the runtime.
//!
//! Still nextest-SLOW (>2s) under the full parallel suite, ~0.9s on its own:
//! 7 cases, each one bash run of the script plus its stubs, so the cost is
//! process spawning under contention. Same shape as tauri_install_guards.rs.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use golem_cli::scaffold::{render_install_script, InstallFramework};
use tempfile::TempDir;

struct Fixture {
    _tmp: TempDir,
    root: PathBuf,
    script: PathBuf,
    sdk: PathBuf,
}

struct Outcome {
    ok: bool,
    stderr: String,
    gradle_task: String,
    adb_args: String,
}

fn write_exe(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, body).expect("write stub");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

fn fixture() -> Fixture {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path().to_path_buf();

    // Stands in for `./gradlew :app:assemble<Variant>`: writes the APK AGP
    // would, in the shape $FAKE_GRADLE asks for.
    write_exe(
        &root.join("android/gradlew"),
        r#"#!/usr/bin/env bash
echo "$1" > "$REC/gradle"
variant="${1#:app:assemble}"
case "$variant" in
  FreeRelease) dir=free/release; name=app-free-release ;;
  Release)     dir=release;      name=app-release ;;
  *)           dir=debug;        name=app-debug ;;
esac
out="app/build/outputs/apk/$dir"
mkdir -p "$out"
case "${FAKE_GRADLE:-ok}" in
  ok)       : > "$out/$name.apk" ;;
  unsigned) : > "$out/$name-unsigned.apk" ;;
  stale)    : > "$out/$name.apk"; touch -t 202001010000 "$out/$name.apk" ;;
  fail)     echo "FAILURE: Build failed with an exception." >&2; exit 1 ;;
esac
"#,
    );
    write_exe(
        &root.join("bin/adb"),
        "#!/usr/bin/env bash\necho \"$*\" > \"$REC/adb\"\n",
    );
    let sdk = root.join("sdk");
    write_exe(
        &sdk.join("build-tools/36.0.0/aapt2"),
        "#!/usr/bin/env bash\necho \"${FAKE_APP_ID:-com.example.app}\"\n",
    );

    let script = root.join("install.sh");
    fs::write(
        &script,
        render_install_script(
            InstallFramework::NativeAndroid,
            &[("GRADLE_ROOT", "android"), ("MODULE_NAME", "app")],
        )
        .expect("render"),
    )
    .expect("write script");

    Fixture {
        _tmp: tmp,
        root,
        script,
        sdk,
    }
}

impl Fixture {
    fn run(&self, env: &[(&str, &str)], with_sdk: bool) -> Outcome {
        let rec = self.root.join("rec");
        let _ = fs::remove_dir_all(&rec);
        let _ = fs::remove_dir_all(self.root.join("android/app"));
        fs::create_dir_all(&rec).expect("rec dir");

        let path = format!(
            "{}:{}",
            self.root.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new("bash");
        cmd.arg(&self.script)
            .args(["android", "emulator-5554", "com.example.app"])
            .current_dir(&self.root)
            .env("PATH", path)
            .env("REC", &rec)
            .env_remove("ANDROID_SDK_ROOT")
            .env_remove("BUILD_TYPE")
            .env_remove("FLAVOR");
        if with_sdk {
            cmd.env("ANDROID_HOME", &self.sdk);
        } else {
            cmd.env_remove("ANDROID_HOME");
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("bash runs");
        let read = |name: &str| {
            fs::read_to_string(rec.join(name))
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        Outcome {
            ok: out.status.success(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            gradle_task: read("gradle"),
            adb_args: read("adb"),
        }
    }
}

#[test]
fn native_android_builds_the_variant_and_installs_only_the_apk_it_wrote() {
    let fx = fixture();

    let o = fx.run(&[], true);
    assert!(o.ok, "a default build SHALL install:\n{}", o.stderr);
    assert_eq!(o.gradle_task, ":app:assembleDebug", "debug is the default");
    assert_eq!(
        o.adb_args, "-s emulator-5554 install -r android/app/build/outputs/apk/debug/app-debug.apk",
        "the script SHALL install the built APK itself, on the target serial"
    );

    let o = fx.run(&[("FLAVOR", "free"), ("BUILD_TYPE", "release")], true);
    assert!(
        o.ok,
        "a flavored release build SHALL install:\n{}",
        o.stderr
    );
    assert_eq!(o.gradle_task, ":app:assembleFreeRelease");
    assert!(
        o.adb_args
            .ends_with("apk/free/release/app-free-release.apk"),
        "the pick SHALL come from the variant's own directory, got {}",
        o.adb_args
    );

    let o = fx.run(
        &[("BUILD_TYPE", "release"), ("FAKE_GRADLE", "unsigned")],
        true,
    );
    assert!(!o.ok, "an unsigned-only build SHALL fail");
    assert!(
        o.stderr.contains("only unsigned APKs"),
        "the error SHALL name the signing problem:\n{}",
        o.stderr
    );
    assert_eq!(o.adb_args, "", "nothing SHALL be installed");

    let o = fx.run(&[("FAKE_GRADLE", "stale")], true);
    assert!(!o.ok, "an APK the build did not write SHALL fail");
    assert!(
        o.stderr.contains("not refreshed by this build"),
        "{}",
        o.stderr
    );
    assert_eq!(o.adb_args, "");

    let o = fx.run(&[("FAKE_GRADLE", "fail")], true);
    assert!(!o.ok, "a failed gradle build SHALL fail the install");
    assert!(o.stderr.contains("Build failed"), "{}", o.stderr);
    assert_eq!(o.adb_args, "");

    let o = fx.run(&[("FAKE_APP_ID", "com.example.app.debug")], true);
    assert!(!o.ok, "an applicationId other than the bundle SHALL fail");
    assert!(
        o.stderr.contains("'com.example.app.debug'") && o.stderr.contains("'com.example.app'"),
        "the error SHALL name both ids:\n{}",
        o.stderr
    );
    assert_eq!(o.adb_args, "");

    let o = fx.run(&[("FAKE_APP_ID", "com.other")], false);
    assert!(
        o.ok,
        "without SDK build-tools the id check SHALL be skipped, not fail:\n{}",
        o.stderr
    );
}
