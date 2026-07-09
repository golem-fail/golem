//! `golem doctor` — diagnose the environment.
//!
//! Two modes (combinable; runtime is the default):
//! - **runtime**: what's needed to *drive* a device — host CLIs (`adb`,
//!   `xcrun`/`simctl`), an available device, the embedded companions, a writable
//!   `~/.golem`. Exits non-zero when no platform is drivable.
//! - **build** (`--build`): what's needed to *build* golem from source — a Rust
//!   toolchain, plus the companion build deps (`xcodebuild` for iOS; JDK +
//!   Android SDK for Android). Exits non-zero when it can't build.
//!
//! Where a tool exposes one, the detected version is shown (`found 6.1.1`).
//!
//! Design for testability (see the roadmap's I/O-seam note): every external
//! probe goes through the `golem_common::command` seam, and the decision logic
//! is split into pure functions — `evaluate_runtime` / `evaluate_build`
//! (facts → checks), [`exit_code`], [`render_with_color`], [`parse_version`],
//! and the device-list parsers — so behaviour is unit-testable without real
//! tools.

use std::io::IsTerminal;
use std::path::Path;

use anyhow::Result;
use golem_common::command;

use crate::cli::DoctorArgs;

/// Severity of a single diagnostic line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Ok,
    Warn,
    Fail,
}

/// One diagnostic line: what was checked, the outcome, and (on non-Ok) a
/// copy-paste remedy.
#[derive(Debug)]
struct Check {
    label: String,
    status: Status,
    detail: String,
    remedy: Option<String>,
}

impl Check {
    fn ok(label: &str, detail: &str) -> Self {
        Self {
            label: label.to_string(),
            status: Status::Ok,
            detail: detail.to_string(),
            remedy: None,
        }
    }
    fn warn(label: &str, detail: &str, remedy: &str) -> Self {
        Self {
            label: label.to_string(),
            status: Status::Warn,
            detail: detail.to_string(),
            remedy: Some(remedy.to_string()),
        }
    }
    fn fail(label: &str, detail: &str, remedy: &str) -> Self {
        Self {
            label: label.to_string(),
            status: Status::Fail,
            detail: detail.to_string(),
            remedy: Some(remedy.to_string()),
        }
    }
}

/// A titled group of checks (used to render runtime + build sections together).
struct Section {
    title: &'static str,
    checks: Vec<Check>,
}

/// Probed facts about the host — the sole input to the evaluators. Plain data so
/// evaluation and rendering stay pure and testable without touching real tools.
///
/// Tools carry `Option<String>`: `None` = absent, `Some(v)` = present (`v` is the
/// detected version, or empty when it couldn't be parsed).
#[derive(Debug, Clone, Default)]
struct Facts {
    is_macos: bool,
    // runtime
    adb: Option<String>,
    xcrun: Option<String>,
    simctl: bool, // needs a working invocation, no clean version
    ffmpeg: Option<String>,
    /// Physical Android devices currently connected (running emulators are AVDs,
    /// counted separately, so they are excluded here to avoid double counting).
    android_connected: usize,
    /// Bootable Android AVDs defined on the host.
    android_avds: usize,
    /// Bootable iOS simulators available on the host.
    ios_sims: usize,
    /// Embedded companion size in bytes, or `None` if not embedded.
    ios_companion: Option<usize>,
    android_companion: Option<usize>,
    /// `Ok` if `~/.golem` is writable, else a human-readable reason.
    golem_writable: Option<std::result::Result<(), String>>,
    // build
    cargo: Option<String>,
    xcodebuild: Option<String>,
    jdk: Option<String>,
    /// The Android SDK path (`ANDROID_HOME`/`ANDROID_SDK_ROOT`) if it exists.
    android_sdk: Option<String>,
}

/// "found 1.2.3" when a version was detected, else "found".
fn found(v: &Option<String>) -> String {
    match v {
        Some(ver) if !ver.is_empty() => format!("found {ver}"),
        _ => "found".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Capability predicates (pure)
// ---------------------------------------------------------------------------

/// Can golem drive Android on this host? (device CLI present *and* companion
/// embedded).
fn android_drivable(f: &Facts) -> bool {
    f.adb.is_some() && f.android_companion.is_some()
}

/// Can golem drive iOS on this host? iOS is macOS-only.
fn ios_drivable(f: &Facts) -> bool {
    f.is_macos && f.xcrun.is_some() && f.simctl && f.ios_companion.is_some()
}

/// Human-readable byte size in IEC binary units, e.g. `12.3 MiB` / `512 KiB`.
fn human_size(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    let b = bytes as f64;
    if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.0} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// Can this host build the Android companion? (JDK + Android SDK).
fn android_buildable(f: &Facts) -> bool {
    f.jdk.is_some() && f.android_sdk.is_some()
}

/// Can this host build the iOS companion? (macOS + Xcode).
fn ios_buildable(f: &Facts) -> bool {
    f.is_macos && f.xcodebuild.is_some()
}

// ---------------------------------------------------------------------------
// Evaluators (pure): facts → checks
// ---------------------------------------------------------------------------

/// Runtime checks: what's needed to drive a device.
fn evaluate_runtime(f: &Facts) -> Vec<Check> {
    let mut checks = Vec::new();

    // State dir — a hard requirement: companions extract here.
    match &f.golem_writable {
        Some(Ok(())) | None => checks.push(Check::ok("~/.golem writable", "yes")),
        Some(Err(reason)) => checks.push(Check::fail(
            "~/.golem writable",
            reason,
            "fix permissions on ~/.golem (golem extracts embedded companions there)",
        )),
    }

    // Android CLI + companion. Individual misses are warnings; the "drivable
    // platform" summary escalates to Fail if nothing is drivable overall.
    if f.adb.is_some() {
        checks.push(Check::ok("adb (Android)", &found(&f.adb)));
    } else {
        checks.push(Check::warn(
            "adb (Android)",
            "not on PATH",
            "install platform-tools: `brew install --cask android-platform-tools` (macOS) / `apt-get install android-tools-adb` (Linux)",
        ));
    }
    if let Some(bytes) = f.android_companion {
        checks.push(Check::ok(
            "Android companion",
            &format!("embedded ({})", human_size(bytes)),
        ));
    } else {
        checks.push(Check::warn(
            "Android companion",
            "not embedded",
            "this binary shipped without the Android companion — reinstall a release built on a host with the Android SDK",
        ));
    }

    // iOS CLI + companion — macOS only.
    if f.is_macos {
        if f.xcrun.is_some() {
            checks.push(Check::ok("xcrun (iOS)", &found(&f.xcrun)));
        } else {
            checks.push(Check::warn(
                "xcrun (iOS)",
                "not on PATH",
                "install Xcode command-line tools: `xcode-select --install`",
            ));
        }
        if f.simctl {
            checks.push(Check::ok("simctl (iOS)", "found"));
        } else {
            checks.push(Check::warn(
                "simctl (iOS)",
                "unavailable",
                "install full Xcode from the App Store (simctl ships with it)",
            ));
        }
        if let Some(bytes) = f.ios_companion {
            checks.push(Check::ok(
                "iOS companion",
                &format!("embedded ({})", human_size(bytes)),
            ));
        } else {
            checks.push(Check::warn(
                "iOS companion",
                "not embedded",
                "this binary shipped without the iOS companion — reinstall a macOS release built with Xcode",
            ));
        }
    } else {
        checks.push(Check::ok("iOS driving", "n/a — requires macOS"));
    }

    // golem boots a device on demand, so what matters is whether at least one is
    // *available* to boot (an AVD / simulator) or already connected — not whether
    // one happens to be booted right now.
    let avail_android = f.android_avds + f.android_connected;
    let avail_ios = if f.is_macos { f.ios_sims } else { 0 };
    let avail = avail_android + avail_ios;
    if avail > 0 {
        checks.push(Check::ok(
            "device available",
            &format!("{avail} ({avail_android} android, {avail_ios} ios)"),
        ));
    } else {
        checks.push(Check::warn(
            "device available",
            "none",
            "no emulator/simulator or connected device found — golem boots one on demand, so create one: Android `avdmanager create avd` (or Android Studio Device Manager); iOS via Xcode > Settings > Components",
        ));
    }

    // ffmpeg — optional. Recording itself uses native screenrecord/simctl and
    // works without it; ffmpeg only lets the a11y audit and `--trace` reuse a
    // frame from an existing recording instead of taking an extra live shot.
    if f.ffmpeg.is_some() {
        checks.push(Check::ok("ffmpeg (optional)", &found(&f.ffmpeg)));
    } else {
        checks.push(Check::warn(
            "ffmpeg (optional)",
            "not on PATH",
            "optional — lets a11y/`--trace` reuse recording frames instead of extra live screenshots (recording works without it): `brew install ffmpeg` / `apt-get install ffmpeg`",
        ));
    }

    // The gate: can golem drive at least one platform end to end?
    let mut drivable = Vec::new();
    if android_drivable(f) {
        drivable.push("android");
    }
    if ios_drivable(f) {
        drivable.push("ios");
    }
    if drivable.is_empty() {
        checks.push(Check::fail(
            "drivable platform",
            "none",
            "install a device toolchain above and ensure its companion is embedded",
        ));
    } else {
        checks.push(Check::ok("drivable platform", &drivable.join(", ")));
    }

    checks
}

/// Build checks: what's needed to build golem + its companions from source.
fn evaluate_build(f: &Facts) -> Vec<Check> {
    let mut checks = Vec::new();

    // Rust — hard requirement to build anything.
    if f.cargo.is_some() {
        checks.push(Check::ok("Rust (cargo)", &found(&f.cargo)));
    } else {
        checks.push(Check::fail(
            "Rust (cargo)",
            "not on PATH",
            "install the Rust toolchain: https://rustup.rs",
        ));
    }

    // Android companion build deps: JDK + Android SDK.
    if f.jdk.is_some() {
        checks.push(Check::ok("JDK (java)", &found(&f.jdk)));
    } else {
        checks.push(Check::warn(
            "JDK (java)",
            "not on PATH",
            "install a JDK 17 (AGP 8.x needs it) to build the Android companion",
        ));
    }
    if let Some(path) = &f.android_sdk {
        checks.push(Check::ok("Android SDK", path));
    } else {
        checks.push(Check::warn(
            "Android SDK",
            "not found",
            "install the Android SDK (cmdline-tools + build-tools) and set ANDROID_HOME to build the Android companion",
        ));
    }

    // iOS companion build deps — macOS only.
    if f.is_macos {
        if f.xcodebuild.is_some() {
            checks.push(Check::ok("Xcode (xcodebuild)", &found(&f.xcodebuild)));
        } else {
            checks.push(Check::warn(
                "Xcode (xcodebuild)",
                "not on PATH",
                "install Xcode from the App Store to build the iOS companion",
            ));
        }
    } else {
        checks.push(Check::ok("iOS build", "n/a — requires macOS"));
    }

    // The gate: Rust present AND at least one companion buildable.
    let mut buildable = Vec::new();
    if android_buildable(f) {
        buildable.push("android");
    }
    if ios_buildable(f) {
        buildable.push("ios");
    }
    if f.cargo.is_none() {
        checks.push(Check::fail(
            "buildable companion",
            "blocked — no Rust toolchain",
            "install Rust (above); it's required to build golem at all",
        ));
    } else if buildable.is_empty() {
        checks.push(Check::fail(
            "buildable companion",
            "none",
            "install a companion's build deps above (JDK + Android SDK, or Xcode) — a build with neither embeds a driverless binary",
        ));
    } else {
        checks.push(Check::ok("buildable companion", &buildable.join(", ")));
    }

    checks
}

/// Process exit code: non-zero if any check failed.
fn exit_code(checks: &[Check]) -> i32 {
    if checks.iter().any(|c| c.status == Status::Fail) {
        1
    } else {
        0
    }
}

/// Render the report. Split on `use_color` so both branches are unit-testable.
/// A section title is only shown when more than one section is present.
fn render_with_color(sections: &[Section], use_color: bool) -> String {
    use std::fmt::Write;

    const GREEN: &str = "\x1b[32m";
    const YELLOW: &str = "\x1b[33m";
    const RED: &str = "\x1b[31m";
    const DIM: &str = "\x1b[2m";
    const BOLD: &str = "\x1b[1m";
    const RESET: &str = "\x1b[0m";

    let paint = |s: &str, code: &str| -> String {
        if use_color {
            format!("{code}{s}{RESET}")
        } else {
            s.to_string()
        }
    };

    let mut out = String::new();
    let _ = writeln!(out, "{}", paint("golem doctor", BOLD));
    let show_titles = sections.len() > 1;

    for section in sections {
        if show_titles {
            let _ = writeln!(out, "{}", paint(&format!("  [{}]", section.title), BOLD));
        }
        for c in &section.checks {
            let (sym, color) = match c.status {
                Status::Ok => ("✓", GREEN),
                Status::Warn => ("!", YELLOW),
                Status::Fail => ("✗", RED),
            };
            let _ = writeln!(out, "  {} {} — {}", paint(sym, color), c.label, c.detail);
            if let Some(remedy) = &c.remedy {
                let _ = writeln!(out, "{}", paint(&format!("      ↳ {remedy}"), DIM));
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Parsers (pure)
// ---------------------------------------------------------------------------

/// Extract the first version-looking token (`1.2`, `6.1.1`, …) from arbitrary
/// `--version` output. Requires ≥2 dot-separated all-numeric components, so it
/// skips bare years/counts. Returns `None` when nothing matches.
fn parse_version(text: &str) -> Option<String> {
    // Split on any char that isn't a digit or dot → maximal `[0-9.]` runs.
    for run in text.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        let comps: Vec<&str> = run.split('.').collect();
        if comps.len() >= 2 && comps.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
            return Some(run.to_string());
        }
    }
    None
}

/// Count *physical* online devices in `adb devices` output — online (`device`
/// state) entries whose serial is not an `emulator-*` (a running AVD, counted
/// via the AVD list instead), ignoring the header and offline/unauthorized rows.
fn count_adb_physical(stdout: &str) -> usize {
    stdout
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter(|(serial, state)| state.trim() == "device" && !serial.starts_with("emulator-"))
        .count()
}

/// Count bootable simulators in `xcrun simctl list devices available` output —
/// device lines carry a state, `(Booted)` or `(Shutdown)`.
fn count_sim_devices(stdout: &str) -> usize {
    stdout
        .lines()
        .filter(|l| l.contains("(Booted)") || l.contains("(Shutdown)"))
        .count()
}

/// Count Android AVDs by their `<name>.ini` marker in the AVD home directory
/// (needs no `emulator` binary on PATH, which often isn't).
fn count_avds_in(avd_home: &Path) -> usize {
    match std::fs::read_dir(avd_home) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "ini"))
            .count(),
        Err(_) => 0,
    }
}

/// The AVD home directory: `$ANDROID_AVD_HOME` if set, else `~/.android/avd`.
fn avd_home() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("ANDROID_AVD_HOME") {
        return std::path::PathBuf::from(dir);
    }
    dirs::home_dir().unwrap_or_default().join(".android/avd")
}

/// The Android SDK dir from `ANDROID_HOME`/`ANDROID_SDK_ROOT`, if it exists.
fn android_sdk_dir() -> Option<String> {
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(dir) = std::env::var(var) {
            if !dir.is_empty() && Path::new(&dir).is_dir() {
                return Some(dir);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Probing (I/O — via the command seam + filesystem)
// ---------------------------------------------------------------------------

/// Spawn `program args…`; return `Some(version)` if it ran at all (version parsed
/// from stdout+stderr, empty if unparseable), or `None` if it couldn't be spawned.
async fn tool(program: &str, args: &[&str]) -> Option<String> {
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    match command::output(program, &args).await {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            Some(parse_version(&text).unwrap_or_default())
        }
        Err(_) => None,
    }
}

/// True if `program args…` ran and exited 0.
async fn runs_ok(program: &str, args: &[&str]) -> bool {
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    matches!(command::output(program, &args).await, Ok(o) if o.status.success())
}

/// Captured stdout of `program args…`, or empty on any failure.
async fn stdout_of(program: &str, args: &[&str]) -> String {
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    match command::output(program, &args).await {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

/// Probe `~/.golem`: ensure it exists and is writable by round-tripping a probe
/// file. Returns `Ok(())` or a human-readable reason.
fn probe_golem_writable() -> std::result::Result<(), String> {
    let home = dirs::home_dir().ok_or_else(|| "could not determine home directory".to_string())?;
    let dir = home.join(".golem");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let probe = dir.join(".doctor-write-probe");
    std::fs::write(&probe, b"").map_err(|e| format!("cannot write to {}: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Gather the facts needed for the requested modes.
async fn probe(run_runtime: bool, run_build: bool) -> Facts {
    let is_macos = cfg!(target_os = "macos");
    let mut f = Facts {
        is_macos,
        ..Default::default()
    };

    if run_runtime {
        f.adb = tool("adb", &["--version"]).await;
        f.ffmpeg = tool("ffmpeg", &["-version"]).await;
        // iOS tooling is macOS-only; skip the probes entirely elsewhere.
        if is_macos {
            f.xcrun = tool("xcrun", &["--version"]).await;
            f.simctl = runs_ok("xcrun", &["simctl", "help"]).await;
        }
        f.android_connected = if f.adb.is_some() {
            count_adb_physical(&stdout_of("adb", &["devices"]).await)
        } else {
            0
        };
        f.android_avds = count_avds_in(&avd_home());
        f.ios_sims = if f.simctl {
            count_sim_devices(&stdout_of("xcrun", &["simctl", "list", "devices", "available"]).await)
        } else {
            0
        };
        f.ios_companion =
            crate::companions::has_ios_companion().then(crate::companions::ios_companion_size);
        f.android_companion = crate::companions::has_android_companion()
            .then(crate::companions::android_companion_size);
        f.golem_writable = Some(probe_golem_writable());
    }

    if run_build {
        f.cargo = tool("cargo", &["--version"]).await;
        f.jdk = tool("java", &["-version"]).await;
        f.android_sdk = android_sdk_dir();
        if is_macos {
            f.xcodebuild = tool("xcodebuild", &["-version"]).await;
        }
    }

    f
}

/// Run `golem doctor`: probe the requested modes, report to stderr, and return
/// the exit code. `--build` selects build mode; `--runtime` (or no flag) selects
/// runtime; both flags check everything.
pub async fn run(args: &DoctorArgs) -> Result<i32> {
    let run_build = args.build;
    let run_runtime = args.runtime || !args.build;

    let facts = probe(run_runtime, run_build).await;

    let mut sections = Vec::new();
    if run_runtime {
        sections.push(Section {
            title: "runtime",
            checks: evaluate_runtime(&facts),
        });
    }
    if run_build {
        sections.push(Section {
            title: "build",
            checks: evaluate_build(&facts),
        });
    }

    // Non-zero if any section has a failing check.
    let code = sections
        .iter()
        .map(|s| exit_code(&s.checks))
        .max()
        .unwrap_or(0);

    let use_color = std::io::stderr().is_terminal();
    eprint!("{}", render_with_color(&sections, use_color));
    Ok(code)
}

/// Auto-invoke hook: when a run/command hits a "no device" dead-end, print the
/// runtime doctor lines that explain *why* (missing CLI, no device, absent
/// companion) instead of a bare error. A thin reuse of the same probe + report
/// logic; scoped to the runtime-dep failure paths so it doesn't balloon.
pub async fn hint_no_device() {
    let checks = evaluate_runtime(&probe(true, false).await);
    // Only the actionable (non-Ok) lines — this is a hint, not the full report.
    let actionable: Vec<Check> = checks
        .into_iter()
        .filter(|c| c.status != Status::Ok)
        .collect();
    if actionable.is_empty() {
        return;
    }
    let use_color = std::io::stderr().is_terminal();
    eprintln!("\nEnvironment diagnostics (run `golem doctor` for the full report):");
    eprint!(
        "{}",
        render_with_color(
            &[Section {
                title: "runtime",
                checks: actionable,
            }],
            use_color,
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_facts() -> Facts {
        Facts {
            is_macos: true,
            adb: Some("1.0.41".to_string()),
            xcrun: Some(String::new()),
            simctl: true,
            ffmpeg: Some("6.1.1".to_string()),
            android_connected: 0,
            android_avds: 1,
            ios_sims: 2,
            ios_companion: Some(9_400_000),
            android_companion: Some(12_000_000),
            golem_writable: Some(Ok(())),
            cargo: Some("1.83.0".to_string()),
            xcodebuild: Some("15.4".to_string()),
            jdk: Some("17.0.10".to_string()),
            android_sdk: Some("/opt/android-sdk".to_string()),
        }
    }

    // 1. A fully-provisioned macOS host: every runtime line Ok, both platforms
    //    drivable, exit 0.
    #[test]
    fn healthy_macos_is_all_ok() {
        let checks = evaluate_runtime(&base_facts());
        assert!(
            checks.iter().all(|c| c.status == Status::Ok),
            "healthy host SHALL produce only Ok lines"
        );
        assert_eq!(exit_code(&checks), 0);
        let drivable = checks
            .iter()
            .find(|c| c.label == "drivable platform")
            .expect("drivable line present");
        assert_eq!(drivable.detail, "android, ios");
    }

    // 2. No toolchains at all → nothing drivable → the summary is Fail and the
    //    exit code is non-zero, even though individual CLI misses are warnings.
    #[test]
    fn no_toolchains_fails_the_gate() {
        let mut f = base_facts();
        f.adb = None;
        f.xcrun = None;
        f.simctl = false;
        let checks = evaluate_runtime(&f);
        assert_eq!(exit_code(&checks), 1, "no drivable platform SHALL exit non-zero");
        let adb = checks.iter().find(|c| c.label == "adb (Android)").expect("adb line");
        assert_eq!(adb.status, Status::Warn, "a single missing CLI is a warning");
        let drivable = checks
            .iter()
            .find(|c| c.label == "drivable platform")
            .expect("drivable line");
        assert_eq!(drivable.status, Status::Fail);
    }

    // 3. One platform drivable is enough to pass the gate: Android tooling
    //    present, iOS tooling absent → exit 0.
    #[test]
    fn one_drivable_platform_passes() {
        let mut f = base_facts();
        f.xcrun = None;
        f.simctl = false;
        f.ios_companion = None;
        let checks = evaluate_runtime(&f);
        assert_eq!(exit_code(&checks), 0, "android-only host is still drivable");
    }

    // 4. Toolchain present but its companion missing ⇒ that platform is NOT
    //    drivable. adb without the Android companion, on a non-macOS host, means
    //    nothing is drivable.
    #[test]
    fn missing_companion_makes_platform_undrivable() {
        let mut f = base_facts();
        f.is_macos = false;
        f.xcrun = None;
        f.simctl = false;
        f.ios_companion = None;
        f.android_companion = None; // adb present, but no companion
        let checks = evaluate_runtime(&f);
        assert_eq!(exit_code(&checks), 1);
    }

    // 5. A non-writable state dir fails the gate regardless of drivability.
    #[test]
    fn unwritable_state_dir_fails() {
        let mut f = base_facts();
        f.golem_writable = Some(Err("cannot write to /root/.golem: permission denied".to_string()));
        let checks = evaluate_runtime(&f);
        assert_eq!(exit_code(&checks), 1);
        let line = checks
            .iter()
            .find(|c| c.label == "~/.golem writable")
            .expect("writable line");
        assert_eq!(line.status, Status::Fail);
        assert!(line.remedy.is_some(), "a failure SHALL carry a remedy");
    }

    // 6. Off macOS, iOS is reported as n/a (not a warning) and no iOS CLI lines
    //    appear.
    #[test]
    fn non_macos_reports_ios_not_applicable() {
        let mut f = base_facts();
        f.is_macos = false;
        let checks = evaluate_runtime(&f);
        assert!(
            checks.iter().all(|c| c.label != "xcrun (iOS)"),
            "no iOS CLI lines off macOS"
        );
        let ios = checks
            .iter()
            .find(|c| c.label == "iOS driving")
            .expect("iOS n/a line");
        assert_eq!(ios.status, Status::Ok);
        assert!(ios.detail.contains("macOS"));
    }

    // 7. Rendering: without color, no ANSI escapes; with color, escapes appear
    //    and remedies are shown for non-Ok lines.
    #[test]
    fn render_color_branches() {
        let mut f = base_facts();
        f.adb = None; // produces a warning with a remedy
        let sections = [Section {
            title: "runtime",
            checks: evaluate_runtime(&f),
        }];

        let plain = render_with_color(&sections, false);
        assert!(!plain.contains('\x1b'), "no-color output SHALL be escape-free");
        assert!(plain.contains("golem doctor"));
        assert!(
            plain.contains("android-platform-tools"),
            "the remedy for a missing CLI SHALL be shown"
        );

        let colored = render_with_color(&sections, true);
        assert!(colored.contains('\x1b'), "color output SHALL contain ANSI escapes");
    }

    // 8. No device available is a WARNING, not a gate failure — golem boots one
    //    on demand, and the platform is still "drivable" (toolchain + companion).
    #[test]
    fn no_device_available_is_warning_not_failure() {
        let mut f = base_facts();
        f.android_connected = 0;
        f.android_avds = 0;
        f.ios_sims = 0;
        let checks = evaluate_runtime(&f);
        let dev = checks
            .iter()
            .find(|c| c.label == "device available")
            .expect("device line");
        assert_eq!(dev.status, Status::Warn, "no device SHALL warn, not fail");
        assert_eq!(exit_code(&checks), 0, "device availability SHALL NOT gate the exit code");
    }

    // 9. Detected versions surface in the detail (e.g. "found 1.0.41").
    #[test]
    fn versions_render_in_detail() {
        let checks = evaluate_runtime(&base_facts());
        let adb = checks.iter().find(|c| c.label == "adb (Android)").expect("adb line");
        assert_eq!(adb.detail, "found 1.0.41");
        let ff = checks.iter().find(|c| c.label == "ffmpeg (optional)").expect("ffmpeg line");
        assert_eq!(ff.detail, "found 6.1.1");
        // xcrun present but no parsed version → plain "found".
        let xc = checks.iter().find(|c| c.label == "xcrun (iOS)").expect("xcrun line");
        assert_eq!(xc.detail, "found");
    }

    // 9b. Embedded companion sizes surface in the detail (sanity signal).
    #[test]
    fn companion_sizes_render_in_detail() {
        let checks = evaluate_runtime(&base_facts());
        let a = checks.iter().find(|c| c.label == "Android companion").expect("android line");
        assert_eq!(a.detail, "embedded (11.4 MiB)");
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(2048), "2 KiB");
        assert_eq!(human_size(9_400_000), "9.0 MiB");
    }

    // 10. Build mode: a fully-provisioned host builds both companions, exit 0.
    #[test]
    fn build_mode_healthy_all_ok() {
        let checks = evaluate_build(&base_facts());
        assert!(checks.iter().all(|c| c.status == Status::Ok), "all build lines Ok");
        assert_eq!(exit_code(&checks), 0);
        let b = checks.iter().find(|c| c.label == "buildable companion").expect("summary");
        assert_eq!(b.detail, "android, ios");
    }

    // 11. Build mode: no Rust ⇒ hard fail regardless of companion deps.
    #[test]
    fn build_mode_without_rust_fails() {
        let mut f = base_facts();
        f.cargo = None;
        let checks = evaluate_build(&f);
        assert_eq!(exit_code(&checks), 1);
        let rust = checks.iter().find(|c| c.label == "Rust (cargo)").expect("rust line");
        assert_eq!(rust.status, Status::Fail);
    }

    // 12. Build mode: Rust present but neither companion buildable ⇒ fail.
    #[test]
    fn build_mode_no_buildable_companion_fails() {
        let mut f = base_facts();
        f.is_macos = false; // no iOS build
        f.jdk = None; // no android build
        let checks = evaluate_build(&f);
        assert_eq!(exit_code(&checks), 1);
        // On non-macOS the iOS build line is n/a (Ok), not a warning.
        let ios = checks.iter().find(|c| c.label == "iOS build").expect("ios build line");
        assert_eq!(ios.status, Status::Ok);
    }

    // 13. Build mode on Linux with the Android build deps present ⇒ exit 0
    //     (android buildable is enough).
    #[test]
    fn build_mode_linux_android_only_passes() {
        let mut f = base_facts();
        f.is_macos = false;
        let checks = evaluate_build(&f);
        assert_eq!(exit_code(&checks), 0);
    }

    // 14. adb parser counts only physical online devices — emulators (running
    //     AVDs, counted via the AVD list) and offline/unauthorized rows excluded.
    #[test]
    fn adb_parser_counts_physical_only() {
        let out = "List of devices attached\n\
                   emulator-5554\tdevice\n\
                   R58N12345\tunauthorized\n\
                   emulator-5556\toffline\n\
                   192.168.0.2:5555\tdevice\n";
        assert_eq!(count_adb_physical(out), 1, "only the physical device counts");
        assert_eq!(count_adb_physical("List of devices attached\n\n"), 0);
    }

    // 15. simctl available parser: counts every bootable device line (Booted or
    //     Shutdown), ignoring headers.
    #[test]
    fn simctl_parser_counts_available_devices() {
        let out = "== Devices ==\n\
                   -- iOS 26.5 --\n    \
                   iPhone 16 (UDID) (Booted) \n    \
                   iPhone 17 (UDID2) (Shutdown) \n";
        assert_eq!(count_sim_devices(out), 2);
        assert_eq!(count_sim_devices("== Devices ==\n-- iOS 26.5 --\n"), 0);
    }

    // 16. AVD counter: counts `<name>.ini` markers in the AVD home, ignoring
    //     the `.avd` payload dirs and other files.
    #[test]
    fn avd_counter_counts_ini_markers() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Pixel_8.ini"), "").expect("write");
        std::fs::write(dir.path().join("Pixel_Tablet.ini"), "").expect("write");
        std::fs::create_dir(dir.path().join("Pixel_8.avd")).expect("mkdir");
        std::fs::write(dir.path().join("README.txt"), "").expect("write");
        assert_eq!(count_avds_in(dir.path()), 2);
        assert_eq!(count_avds_in(&dir.path().join("does-not-exist")), 0);
    }

    // 17. Version parser: pulls a dotted version from noisy tool output and
    //     rejects non-versions.
    #[test]
    fn version_parser_extracts_dotted_versions() {
        assert_eq!(parse_version("ffmpeg version 6.1.1 Copyright").as_deref(), Some("6.1.1"));
        assert_eq!(
            parse_version("Android Debug Bridge version 1.0.41").as_deref(),
            Some("1.0.41")
        );
        assert_eq!(parse_version("cargo 1.83.0 (abc 2024)").as_deref(), Some("1.83.0"));
        assert_eq!(parse_version("openjdk version \"17.0.10\" 2024").as_deref(), Some("17.0.10"));
        // "xcrun version 66." — trailing dot ⇒ not a clean N.N version.
        assert_eq!(parse_version("xcrun version 66."), None);
        assert_eq!(parse_version("no version here"), None);
    }
}
