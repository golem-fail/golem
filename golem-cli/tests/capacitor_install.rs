//! Runs the rendered Capacitor install script against stubbed web build,
//! `cap`, `./gradlew`, `adb`, `xcodebuild` and `xcrun`, so the order of the
//! steps and the gates between them are covered by `cargo t` with no Node,
//! no SDKs and no devices.
//!
//! One project and one set of stubs serve every case, with the behaviour
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

    // The web build: writes www/index.html fresh, or leaves an old one.
    write_exe(
        &bin.join("fakeweb"),
        r#"#!/usr/bin/env bash
echo "web" >> "$REC"
mkdir -p www
case "${FAKE_WEB:-ok}" in
  ok)    echo '<h1>Counter</h1>' > www/index.html ;;
  stale) : ;;
esac
"#,
    );
    write_exe(
        &bin.join("fakecap"),
        "#!/usr/bin/env bash\necho \"cap $*\" >> \"$REC\"\n",
    );
    write_exe(
        &root.join("android/gradlew"),
        r#"#!/usr/bin/env bash
echo "gradle $1" >> "$REC"
case "$1" in
  *Release) d=release ;;
  *)        d=debug ;;
esac
mkdir -p "app/build/outputs/apk/$d"
: > "app/build/outputs/apk/$d/app-$d.apk"
"#,
    );
    write_exe(
        &bin.join("adb"),
        "#!/usr/bin/env bash\necho \"adb $*\" >> \"$REC\"\n",
    );
    write_exe(
        &bin.join("xcodebuild"),
        r#"#!/usr/bin/env bash
echo "xcodebuild $*" >> "$REC"
cfg=""; dd=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -configuration) cfg="$2"; shift ;;
    -derivedDataPath) dd="$2"; shift ;;
  esac
  shift
done
mkdir -p "$dd/Build/Products/$cfg-iphonesimulator/App.app"
"#,
    );
    write_exe(
        &bin.join("xcrun"),
        r#"#!/usr/bin/env bash
case "$1 $2" in
  "simctl list") printf '{ "devices" : { "r" : [ { "udid" : "SIM" } ] } }\n' ;;
  "simctl install") echo "simctl install $3 $4" >> "$REC" ;;
esac
"#,
    );

    let script = root.join("install.sh");
    fs::write(
        &script,
        render_install_script(
            InstallFramework::Capacitor,
            &[
                ("CAP_DIR", "."),
                ("CAP_CMD", "fakecap"),
                ("PM_INSTALL", ""),
                ("WEB_BUILD", "fakeweb"),
                ("WEB_DIR", "www"),
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
    fn run(&self, platform: &str, mode: &str, env: &[(&str, &str)]) -> Outcome {
        let rec = self.root.join("rec.log");
        let _ = fs::remove_file(&rec);
        // An old web bundle, so a web build that writes nothing is visible.
        fs::create_dir_all(self.root.join("www")).expect("www");
        let old = fs::File::create(self.root.join("www/index.html")).expect("old index");
        old.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(600))
            .expect("age");

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
            ])
            .arg("fail.golem.testc")
            .arg(mode)
            .current_dir(&self.root)
            .env("PATH", path)
            .env("REC", &rec)
            .env_remove("ANDROID_HOME")
            .env_remove("ANDROID_SDK_ROOT");
        for k in [
            "BUILD_TYPE",
            "FLAVOR",
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
fn capacitor_android_builds_the_web_app_syncs_and_installs_in_order() {
    let fx = fixture();

    let o = fx.run("android", "", &[]);
    assert!(o.ok, "an Android build SHALL install:\n{}", o.stderr);
    assert_eq!(
        o.log,
        "web\ncap sync android\ngradle :app:assembleDebug\n\
         adb -s emulator-5554 install -r android/app/build/outputs/apk/debug/app-debug.apk\n",
        "web build, then cap sync, then gradle, then install"
    );

    let o = fx.run("android", "", &[("FAKE_WEB", "stale")]);
    assert!(!o.ok, "a web build that wrote nothing SHALL fail");
    assert!(
        o.stderr.contains("not refreshed by this build"),
        "{}",
        o.stderr
    );
    assert!(
        !o.log.contains("cap sync"),
        "the stale bundle SHALL NOT be synced"
    );

    let o = fx.run("android", "install-only", &[]);
    assert!(o.ok, "{}", o.stderr);
    assert!(
        o.log.starts_with("adb "),
        "install-only SHALL skip the web build, sync and gradle, got:\n{}",
        o.log
    );

    let o = fx.run("android", "", &[("BUILD_TYPE", "profile")]);
    assert!(!o.ok);
    assert!(o.stderr.contains("unknown BUILD_TYPE"), "{}", o.stderr);
    assert_eq!(o.log, "", "nothing SHALL run before the config is valid");
}

#[test]
fn capacitor_ios_picks_the_project_and_configuration() {
    let fx = fixture();
    fs::create_dir_all(fx.root.join("ios")).expect("ios dir");

    let o = fx.run("ios", "", &[("BUILD_TYPE", "release")]);
    assert!(o.ok, "an iOS build SHALL install:\n{}", o.stderr);
    assert!(
        o.log.contains(
            "xcodebuild -project ios/App/App.xcodeproj -scheme App -configuration Release"
        ),
        "BUILD_TYPE=release SHALL select the Release configuration, got:\n{}",
        o.log
    );
    assert!(o.log.ends_with(
        "simctl install SIM ./build/DerivedData/Build/Products/Release-iphonesimulator/App.app\n"
    ));

    fs::create_dir_all(fx.root.join("ios/App/App.xcworkspace")).expect("workspace");
    let o = fx.run("ios", "", &[]);
    assert!(o.ok, "{}", o.stderr);
    assert!(
        o.log
            .contains("xcodebuild -workspace ios/App/App.xcworkspace"),
        "a CocoaPods workspace SHALL win over the bare project, got:\n{}",
        o.log
    );

    fs::remove_dir_all(fx.root.join("ios")).expect("rm ios");
    let o = fx.run("ios", "", &[]);
    assert!(!o.ok, "a missing native project SHALL fail");
    assert!(
        o.stderr.contains("'fakecap add ios'"),
        "the error SHALL name the command that creates it:\n{}",
        o.stderr
    );
    assert_eq!(o.log, "");
}
