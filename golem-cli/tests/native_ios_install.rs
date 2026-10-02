//! Runs the rendered native-ios install script against stubbed `xcodebuild`
//! and `xcrun`, so the build/install decisions are covered by `cargo t` with
//! no Xcode, no simulator and no device.
//!
//! One project and one set of stubs serve every case, with the behaviour
//! picked through env vars: a freshly written executable costs ~350ms of exec
//! overhead on macOS, and per case that would double the runtime.
//!
//! Still nextest-SLOW (>2s) under the full parallel suite, ~2s on its own:
//! 8 cases, each one bash run of the script plus its stubs, so the cost is
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
}

struct Outcome {
    ok: bool,
    stderr: String,
    build_args: String,
    install: String,
}

fn write_exe(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, body).expect("write stub");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

fn fixture() -> Fixture {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path().to_path_buf();

    // Stands in for `xcodebuild … build`: writes App.app where Xcode would,
    // in the shape $FAKE_XCODEBUILD asks for.
    write_exe(
        &root.join("bin/xcodebuild"),
        r#"#!/usr/bin/env bash
echo "$*" > "$REC/build"
cfg=""; dd=""; sdk=iphoneos
while [[ $# -gt 0 ]]; do
  case "$1" in
    -configuration) cfg="$2"; shift ;;
    -derivedDataPath) dd="$2"; shift ;;
    -destination) [[ "$2" == *Simulator* ]] && sdk=iphonesimulator; shift ;;
  esac
  shift
done
app="$dd/Build/Products/$cfg-$sdk/App.app"
case "${FAKE_XCODEBUILD:-ok}" in
  ok)    mkdir -p "$app"; : > "$app/Info.plist" ;;
  stale) mkdir -p "$app"; : > "$app/Info.plist"; touch -t 202001010000 "$app" ;;
  noop)  : ;;
  fail)  echo "error: Signing for \"App\" requires a development team." >&2; exit 65 ;;
esac
"#,
    );
    // The device list names "PHYS" outside a "udid" field, so only an
    // anchored match tells the simulator from the physical device.
    write_exe(
        &root.join("bin/xcrun"),
        r#"#!/usr/bin/env bash
case "$1 $2" in
  "simctl list")
    printf '{ "devices" : { "r" : [ { "name" : "PHYS", "udid" : "SIM" } ] } }\n' ;;
  "simctl install") echo "simctl $3 $4" > "$REC/install" ;;
  "devicectl --version") exit 0 ;;
  "devicectl device") echo "devicectl $6 $7" > "$REC/install" ;;
esac
"#,
    );

    let script = root.join("install.sh");
    fs::write(
        &script,
        render_install_script(
            InstallFramework::NativeIos,
            &[
                ("XCODE_PROJECT", "App.xcodeproj"),
                ("XCODE_SCHEME", "App"),
                ("CONFIGURATION", "Debug"),
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
    fn products(&self, dir: &str) -> PathBuf {
        self.root.join("build/DerivedData/Build/Products").join(dir)
    }

    fn run(&self, udid: &str, env: &[(&str, &str)]) -> Outcome {
        let rec = self.root.join("rec");
        let _ = fs::remove_dir_all(&rec);
        fs::create_dir_all(&rec).expect("rec dir");

        let path = format!(
            "{}:{}",
            self.root.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new("bash");
        cmd.arg(&self.script)
            .args(["ios", udid, "com.example.app"])
            .current_dir(&self.root)
            .env("PATH", path)
            .env("REC", &rec);
        for k in [
            "CONFIGURATION",
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
        let read = |name: &str| {
            fs::read_to_string(rec.join(name))
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        Outcome {
            ok: out.status.success(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            build_args: read("build"),
            install: read("install"),
        }
    }
}

#[test]
fn native_ios_builds_with_env_overrides_and_installs_only_a_fresh_app() {
    let fx = fixture();

    let o = fx.run("SIM", &[]);
    assert!(o.ok, "a simulator build SHALL install:\n{}", o.stderr);
    assert!(
        o.build_args.contains("-scheme App -configuration Debug")
            && o.build_args.contains("platform=iOS Simulator,id=SIM"),
        "got {}",
        o.build_args
    );
    assert!(
        !o.build_args.contains("-quiet"),
        "build output SHALL NOT be silenced"
    );
    assert_eq!(
        o.install,
        "simctl SIM ./build/DerivedData/Build/Products/Debug-iphonesimulator/App.app"
    );

    let o = fx.run(
        "SIM",
        &[
            ("CONFIGURATION", "Release"),
            ("XCODE_SCHEME", "Other"),
            ("XCCONFIG", "ci.xcconfig"),
        ],
    );
    assert!(o.ok, "{}", o.stderr);
    assert!(
        o.build_args
            .contains("-scheme Other -configuration Release")
            && o.build_args.contains("-xcconfig ci.xcconfig"),
        "env SHALL override scheme and configuration and add the xcconfig, got {}",
        o.build_args
    );
    assert!(o.install.ends_with("Release-iphonesimulator/App.app"));

    let o = fx.run("SIM", &[("FAKE_XCODEBUILD", "stale")]);
    assert!(!o.ok, "a .app the build did not write SHALL fail");
    assert!(
        o.stderr.contains("not refreshed by this build"),
        "{}",
        o.stderr
    );
    assert_eq!(o.install, "");

    // A no-op build must not let the previous .app through.
    fs::create_dir_all(fx.products("Debug-iphonesimulator/App.app")).expect("leftover");
    let o = fx.run("SIM", &[("FAKE_XCODEBUILD", "noop")]);
    assert!(!o.ok, "a leftover .app SHALL NOT be installed");
    assert_eq!(o.install, "");

    let o = fx.run("PHYS", &[]);
    assert!(o.ok, "{}", o.stderr);
    assert!(
        o.build_args.contains("platform=iOS,id=PHYS"),
        "a UDID outside a \"udid\" field SHALL NOT count as a simulator, got {}",
        o.build_args
    );
    assert!(!o.build_args.contains("-allowProvisioningUpdates"));
    assert!(o.install.starts_with("devicectl PHYS"), "{}", o.install);

    let o = fx.run("PHYS", &[("DEVELOPMENT_TEAM", "ABCDE12345")]);
    assert!(o.ok, "{}", o.stderr);
    assert!(
        o.build_args
            .contains("-allowProvisioningUpdates DEVELOPMENT_TEAM=ABCDE12345"),
        "got {}",
        o.build_args
    );

    let o = fx.run("PHYS", &[("FAKE_XCODEBUILD", "fail")]);
    assert!(!o.ok);
    assert!(
        o.stderr.contains("requires a development team")
            && o.stderr.contains("set DEVELOPMENT_TEAM"),
        "a physical build failure SHALL show xcodebuild's error and name the fix:\n{}",
        o.stderr
    );

    let o = fx.run("SIM", &[("FAKE_XCODEBUILD", "fail")]);
    assert!(!o.ok);
    assert!(
        !o.stderr.contains("set DEVELOPMENT_TEAM"),
        "a simulator build needs no team, so SHALL NOT be told to set one"
    );
}
