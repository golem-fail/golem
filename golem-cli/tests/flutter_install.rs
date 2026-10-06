//! Runs the rendered Flutter install script against stubbed `flutter`, `adb`
//! and `xcrun`, so its decisions are covered by `cargo t` with no Flutter
//! SDK, no platform SDKs and no devices.
//!
//! One project and one set of stubs serve each test, with the behaviour
//! picked through env vars: a freshly written executable costs ~350ms of exec
//! overhead on macOS, and per case that would double the runtime.

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

    // Stands in for `flutter build <apk|ios> …`: writes the artifact where
    // Flutter does, and records the signing team Flutter would hand xcodebuild.
    write_exe(
        &bin.join("fakeflutter"),
        r#"#!/usr/bin/env bash
echo "flutter $*${FLUTTER_XCODE_DEVELOPMENT_TEAM:+ team=$FLUTTER_XCODE_DEVELOPMENT_TEAM}" >> "$REC"
mode=debug; flavor=""; sim=0; prev=""
for a in "$@"; do
  case "$a" in
    --profile) mode=profile ;;
    --release) mode=release ;;
    --simulator) sim=1 ;;
  esac
  [[ "$prev" == --flavor ]] && flavor="$a"
  prev="$a"
done
case "$2" in
  apk)
    if [[ -n "$flavor" ]]; then d="build/app/outputs/apk/$flavor/$mode"; f="app-$flavor-$mode.apk"
    else d="build/app/outputs/apk/$mode"; f="app-$mode.apk"; fi
    mkdir -p "$d"; : > "$d/$f"
    if [[ "${FAKE_STALE:-}" == 1 ]]; then touch -t 202001010000 "$d/$f"; fi ;;
  ios)
    if [[ "${FAKE_NO_APP:-}" == 1 ]]; then exit 0; fi
    if [[ $sim == 1 ]]; then mkdir -p build/ios/iphonesimulator/Runner.app
    else mkdir -p build/ios/iphoneos/Runner.app; fi ;;
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
            InstallFramework::Flutter,
            &[("FLUTTER_DIR", "."), ("FLUTTER_CMD", "fakeflutter")],
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
    fn run(&self, platform: &str, device: &str, args: &[&str], env: &[(&str, &str)]) -> Outcome {
        let rec = self.root.join("rec.log");
        let _ = fs::remove_file(&rec);
        let path = format!(
            "{}:{}",
            self.root.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new("bash");
        cmd.arg(&self.script)
            .args([platform, device, "fail.golem.testd"])
            .args(args)
            .current_dir(&self.root)
            .env("PATH", path)
            .env("REC", &rec);
        for k in [
            "BUILD_TYPE",
            "FLAVOR",
            "DART_DEFINES",
            "DART_DEFINE_FILE",
            "FLUTTER_BUILD_ARGS",
            "DEVELOPMENT_TEAM",
            "FLUTTER_XCODE_DEVELOPMENT_TEAM",
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
fn flutter_android_builds_the_variant_and_installs_its_fresh_apk() {
    let fx = fixture();

    let o = fx.run("android", "emulator-5554", &[], &[]);
    assert!(o.ok, "a debug build SHALL install:\n{}", o.stderr);
    assert_eq!(
        o.log,
        "flutter build apk --debug\n\
         adb -s emulator-5554 install -r build/app/outputs/apk/debug/app-debug.apk\n"
    );

    let o = fx.run(
        "android",
        "emulator-5554",
        &[],
        &[
            ("BUILD_TYPE", "release"),
            ("FLAVOR", "free"),
            ("DART_DEFINES", "API=https://x ENV=staging"),
            ("DART_DEFINE_FILE", "env.json"),
            ("FLUTTER_BUILD_ARGS", "--obfuscate --split-debug-info=sym"),
        ],
    );
    assert!(o.ok, "{}", o.stderr);
    assert_eq!(
        o.log,
        "flutter build apk --release --flavor free --dart-define=API=https://x \
         --dart-define=ENV=staging --dart-define-from-file=env.json --obfuscate \
         --split-debug-info=sym\n\
         adb -s emulator-5554 install -r build/app/outputs/apk/free/release/app-free-release.apk\n",
        "the flavor's own variant SHALL be built and installed"
    );

    let o = fx.run("android", "emulator-5554", &["install-only"], &[]);
    assert!(o.ok, "{}", o.stderr);
    assert_eq!(
        o.log, "adb -s emulator-5554 install -r build/app/outputs/apk/debug/app-debug.apk\n",
        "install-only SHALL reuse the previous APK without building"
    );

    let o = fx.run("android", "emulator-5554", &[], &[("FAKE_STALE", "1")]);
    assert!(!o.ok, "an APK the build did not write SHALL NOT install");
    assert!(
        o.stderr.contains("not refreshed by this build"),
        "{}",
        o.stderr
    );
    assert!(!o.log.contains("adb"), "{}", o.log);

    let o = fx.run(
        "android",
        "emulator-5554",
        &[],
        &[("BUILD_TYPE", "staging")],
    );
    assert!(!o.ok, "an unknown BUILD_TYPE SHALL fail");
    assert!(o.log.is_empty(), "nothing SHALL be built: {}", o.log);
}

#[test]
fn flutter_ios_builds_debug_for_a_simulator_and_release_for_a_device() {
    let fx = fixture();

    let o = fx.run("ios", "SIM", &[], &[]);
    assert!(o.ok, "{}", o.stderr);
    assert_eq!(
        o.log,
        "flutter build ios --debug --simulator\n\
         simctl install build/ios/iphonesimulator/Runner.app\n"
    );

    let o = fx.run("ios", "SIM", &[], &[("BUILD_TYPE", "release")]);
    assert!(!o.ok, "a release build for a simulator SHALL fail");
    assert!(
        o.stderr.contains("only debug for a simulator"),
        "{}",
        o.stderr
    );
    assert!(o.log.is_empty(), "nothing SHALL be built: {}", o.log);

    let o = fx.run("ios", "PHYS", &[], &[]);
    assert!(!o.ok, "a debug build for a physical device SHALL fail");
    assert!(o.stderr.contains("BUILD_TYPE=release"), "{}", o.stderr);

    let o = fx.run(
        "ios",
        "PHYS",
        &[],
        &[
            ("BUILD_TYPE", "release"),
            ("DEVELOPMENT_TEAM", "ABCDE12345"),
        ],
    );
    assert!(o.ok, "{}", o.stderr);
    assert_eq!(
        o.log,
        "flutter build ios --release team=ABCDE12345\n\
         devicectl install build/ios/iphoneos/Runner.app\n",
        "the team SHALL reach xcodebuild through FLUTTER_XCODE_DEVELOPMENT_TEAM"
    );

    let o = fx.run("ios", "SIM", &[], &[("FAKE_NO_APP", "1")]);
    assert!(!o.ok, "a build with no .app SHALL fail");
    assert!(o.stderr.contains("no .app under"), "{}", o.stderr);
}
