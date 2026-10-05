//! Runs the rendered Kotlin Multiplatform install script against stubbed
//! `./gradlew`, `adb`, `xcodebuild`, `xcrun` and `pod`, so its decisions are
//! covered by `cargo t` with no SDKs, no Kotlin/Native and no devices.
//!
//! One project and one set of stubs serve each test, with the behaviour
//! picked through env vars: a freshly written executable costs ~350ms of exec
//! overhead on macOS, and per case that would double the runtime.
//!
//! The iOS test can be nextest-SLOW (>2s) under the full parallel suite: it
//! is 4 runs of the script, each one bash plus its stubs, so the cost is
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

    write_exe(
        &root.join("gradlew"),
        r#"#!/usr/bin/env bash
echo "gradle $1" >> "$REC"
mkdir -p androidApp/build/outputs/apk/debug
: > androidApp/build/outputs/apk/debug/androidApp-debug.apk
"#,
    );
    write_exe(
        &bin.join("adb"),
        "#!/usr/bin/env bash\necho \"adb $*\" >> \"$REC\"\n",
    );
    write_exe(
        &bin.join("pod"),
        "#!/usr/bin/env bash\necho \"pod $* in ${PWD##*/}\" >> \"$REC\"\n",
    );
    // Writes the .app with or without the Kotlin/Native runtime in it, as a
    // project with or without its framework build step would.
    write_exe(
        &bin.join("xcodebuild"),
        r#"#!/usr/bin/env bash
echo "xcodebuild $1 $2" >> "$REC"
cfg=""; dd=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -configuration) cfg="$2"; shift ;;
    -derivedDataPath) dd="$2"; shift ;;
  esac
  shift
done
app="$dd/Build/Products/$cfg-iphonesimulator/App.app"
mkdir -p "$app"
case "${FAKE_KOTLIN:-yes}" in
  yes) printf 'swift…Konan_init…' > "$app/App.debug.dylib" ;;
  no)  printf 'swift only' > "$app/App" ;;
esac
"#,
    );
    write_exe(
        &bin.join("xcrun"),
        r#"#!/usr/bin/env bash
case "$1 $2" in
  "simctl list") printf '{ "devices" : { "r" : [ { "udid" : "SIM" } ] } }\n' ;;
  "simctl install") echo "simctl install $4" >> "$REC" ;;
esac
"#,
    );
    fs::create_dir_all(root.join("iosApp/iosApp.xcodeproj")).expect("xcodeproj");

    let script = root.join("install.sh");
    fs::write(
        &script,
        render_install_script(
            InstallFramework::Kmp,
            &[
                ("KMP_DIR", "."),
                ("ANDROID_MODULE", "androidApp"),
                ("IOS_DIR", "iosApp"),
                ("XCODE_SCHEME", "iosApp"),
            ],
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
    fn run(&self, platform: &str, env: &[(&str, &str)]) -> Outcome {
        let rec = self.root.join("rec.log");
        let _ = fs::remove_file(&rec);
        let path = format!(
            "{}:{}",
            self.root.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new("bash");
        cmd.arg(&self.script)
            .args([
                platform,
                if platform == "ios" {
                    "SIM"
                } else {
                    "emulator-5554"
                },
                "fail.golem.testk",
            ])
            .current_dir(&self.root)
            .env("PATH", path)
            .env("REC", &rec)
            .env_remove("ANDROID_HOME")
            .env_remove("ANDROID_SDK_ROOT");
        for k in [
            "BUILD_TYPE",
            "FLAVOR",
            "XCODE_SCHEME",
            "XCCONFIG",
            "DEVELOPMENT_TEAM",
            "DERIVED_DATA",
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
fn kmp_android_builds_the_app_module_and_installs_its_apk() {
    let fx = fixture();
    let o = fx.run("android", &[]);
    assert!(o.ok, "an Android build SHALL install:\n{}", o.stderr);
    assert_eq!(
        o.log,
        "gradle :androidApp:assembleDebug\n\
         adb -s emulator-5554 install -r ./androidApp/build/outputs/apk/debug/androidApp-debug.apk\n"
    );
}

#[test]
fn kmp_ios_installs_only_an_app_that_holds_the_kotlin_code() {
    let fx = fixture();

    let o = fx.run("ios", &[("BUILD_TYPE", "release")]);
    assert!(
        o.ok,
        "an iOS build with Kotlin code SHALL install:\n{}",
        o.stderr
    );
    assert_eq!(
        o.log,
        "xcodebuild -project iosApp/iosApp.xcodeproj\n\
         simctl install ./build/DerivedData/Build/Products/Release-iphonesimulator/App.app\n"
    );

    let o = fx.run("ios", &[("FAKE_KOTLIN", "no")]);
    assert!(
        !o.ok,
        "an .app without the Kotlin framework SHALL NOT install"
    );
    assert!(
        o.stderr.contains("binaries.framework")
            && o.stderr.contains("embedAndSignAppleFrameworkForXcode"),
        "the error SHALL name both prerequisites:\n{}",
        o.stderr
    );
    assert!(!o.log.contains("simctl install"), "{}", o.log);

    // CocoaPods integration: build the workspace; `pod install` only when
    // what is installed (Pods/Manifest.lock) differs from Podfile.lock.
    fs::create_dir_all(fx.root.join("iosApp/iosApp.xcworkspace")).expect("workspace");
    fs::create_dir_all(fx.root.join("iosApp/Pods")).expect("pods");
    fs::write(fx.root.join("iosApp/Podfile.lock"), "shared 1.0").expect("lock");
    fs::write(fx.root.join("iosApp/Pods/Manifest.lock"), "shared 0.9").expect("manifest");
    let o = fx.run("ios", &[]);
    assert!(o.ok, "{}", o.stderr);
    assert!(
        o.log.starts_with(
            "pod install in iosApp\nxcodebuild -workspace iosApp/iosApp.xcworkspace\n"
        ),
        "a stale pod install SHALL re-run before the workspace build, got:\n{}",
        o.log
    );
    fs::write(fx.root.join("iosApp/Pods/Manifest.lock"), "shared 1.0").expect("manifest");
    let o = fx.run("ios", &[]);
    assert!(o.ok, "{}", o.stderr);
    assert!(
        !o.log.contains("pod install"),
        "current pods SHALL NOT reinstall:\n{}",
        o.log
    );
}
