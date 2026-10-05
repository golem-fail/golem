//! Runs the rendered NativeScript install script against stubbed `ns`,
//! `adb` and `xcrun`, so its decisions are covered by `cargo t` with no
//! NativeScript CLI, no SDKs and no devices.
//!
//! One project and one set of stubs serve each test, with the behaviour
//! picked through env vars: a freshly written executable costs ~350ms of exec
//! overhead on macOS, and per case that would double the runtime.
//!
//! The iOS test can be nextest-SLOW (>2s) under the full parallel suite: it
//! is 5 runs of the script, each one bash plus its stubs, so the cost is
//! process spawning under contention. Same shape as native_ios_install.rs.

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
}

struct Outcome {
    ok: bool,
    stderr: String,
    /// Every stub call, in order, one per line.
    log: String,
}

fn write_exe(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, body).expect("write stub");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

fn fixture() -> Fixture {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path().to_path_buf();
    let bin = root.join("bin");

    // Stands in for `ns build <platform> …`: writes the artifact where the
    // CLI does, iOS products under the casing $FAKE_IOS_DIR names.
    write_exe(
        &bin.join("fakens"),
        r#"#!/usr/bin/env bash
echo "ns $*" >> "$REC"
cfg=debug; sdk=iphonesimulator
for a in "$@"; do
  [[ "$a" == --release ]] && cfg=release
  [[ "$a" == --for-device ]] && sdk=iphoneos
done
case "$2" in
  android)
    d="platforms/android/app/build/outputs/apk/$cfg"
    mkdir -p "$d"; : > "$d/app-$cfg.apk"
    if [[ "${FAKE_STALE:-}" == 1 ]]; then touch -t 202001010000 "$d/app-$cfg.apk"; fi ;;
  ios)
    if [[ "${FAKE_NO_APP:-}" == 1 ]]; then exit 0; fi
    Cfg=Debug; [[ $cfg == release ]] && Cfg=Release
    mkdir -p "platforms/ios/${FAKE_IOS_DIR:-build}/$Cfg-$sdk/App.app" ;;
esac
"#,
    );
    write_exe(
        &bin.join("adb"),
        "#!/usr/bin/env bash\necho \"adb $*\" >> \"$REC\"\n",
    );
    write_exe(
        &bin.join("xcrun"),
        r#"#!/usr/bin/env bash
case "$1 $2" in
  "simctl list") printf '{ "devices" : { "r" : [ { "udid" : "SIM" } ] } }\n' ;;
  "simctl install") echo "simctl install $4" >> "$REC" ;;
  "devicectl --version") exit 0 ;;
  "devicectl device") echo "devicectl install $7" >> "$REC" ;;
esac
"#,
    );

    let script = root.join("install.sh");
    fs::write(
        &script,
        render_install_script(
            InstallFramework::NativeScript,
            &[("NS_DIR", "."), ("NS_CMD", "fakens"), ("PM_INSTALL", "")],
        )
        .expect("render"),
    )
    .expect("write script");

    Fixture {
        _tmp: tmp,
        root,
        script,
    }
}

impl Fixture {
    fn run(&self, platform: &str, device: &str, env: &[(&str, &str)]) -> Outcome {
        let rec = self.root.join("rec.log");
        let _ = fs::remove_file(&rec);
        let path = format!(
            "{}:{}",
            self.root.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new("bash");
        cmd.arg(&self.script)
            .args([platform, device, "fail.golem.testn"])
            .current_dir(&self.root)
            .env("PATH", path)
            .env("REC", &rec);
        for k in [
            "BUILD_TYPE",
            "NS_BUILD_ARGS",
            "DEVELOPMENT_TEAM",
            "PROVISION",
        ] {
            cmd.env_remove(k);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("bash runs");
        Outcome {
            ok: out.status.success(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            log: fs::read_to_string(&rec).unwrap_or_default(),
        }
    }
}

#[test]
fn nativescript_android_builds_and_installs_the_fresh_apk() {
    let fx = fixture();

    let o = fx.run("android", "emulator-5554", &[]);
    assert!(o.ok, "a debug build SHALL install:\n{}", o.stderr);
    assert_eq!(
        o.log,
        "ns build android\n\
         adb -s emulator-5554 install -r platforms/android/app/build/outputs/apk/debug/app-debug.apk\n"
    );

    let o = fx.run(
        "android",
        "emulator-5554",
        &[
            ("BUILD_TYPE", "release"),
            ("NS_BUILD_ARGS", "--env.uglify --key-store-path ks.jks"),
        ],
    );
    assert!(o.ok, "{}", o.stderr);
    assert!(
        o.log
            .starts_with("ns build android --release --env.uglify --key-store-path ks.jks\n"),
        "release and NS_BUILD_ARGS SHALL reach ns build, got:\n{}",
        o.log
    );
    assert!(o.log.contains("apk/release/app-release.apk"), "{}", o.log);

    let o = fx.run("android", "emulator-5554", &[("FAKE_STALE", "1")]);
    assert!(!o.ok, "an APK the build did not write SHALL NOT install");
    assert!(
        o.stderr.contains("not refreshed by this build"),
        "{}",
        o.stderr
    );
    assert!(!o.log.contains("adb"), "{}", o.log);
}

#[test]
fn nativescript_ios_finds_the_app_under_either_casing() {
    let fx = fixture();

    let o = fx.run("ios", "SIM", &[]);
    assert!(o.ok, "{}", o.stderr);
    assert_eq!(
        o.log,
        "ns build ios\nsimctl install platforms/ios/build/Debug-iphonesimulator/App.app\n"
    );

    // nativescript-cli#5055: some versions write `Build/`. A fresh fixture,
    // so no `build/` is left over. On a case-insensitive filesystem (macOS)
    // the two spellings are one directory, so either spelling is accepted.
    let fx2 = fixture();
    let o = fx2.run("ios", "SIM", &[("FAKE_IOS_DIR", "Build")]);
    assert!(o.ok, "{}", o.stderr);
    assert!(
        o.log
            .to_lowercase()
            .ends_with("simctl install platforms/ios/build/debug-iphonesimulator/app.app\n"),
        "the `Build/` casing SHALL be found too, got:\n{}",
        o.log
    );

    let o = fx.run("ios", "PHYS", &[("DEVELOPMENT_TEAM", "ABCDE12345")]);
    assert!(o.ok, "{}", o.stderr);
    assert!(
        o.log
            .starts_with("ns build ios --for-device --team-id ABCDE12345\n")
            && o.log
                .contains("devicectl install platforms/ios/build/Debug-iphoneos/App.app"),
        "a physical device SHALL build --for-device and sign with the team, got:\n{}",
        o.log
    );

    let o = fx.run("ios", "SIM", &[("FAKE_NO_APP", "1")]);
    assert!(!o.ok, "a build with no .app SHALL fail");
    assert!(o.stderr.contains("no .app under"), "{}", o.stderr);
}
