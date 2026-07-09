//! `golem doctor` — diagnose the runtime environment.
//!
//! The `golem` binary is self-contained: the companions are baked in, so
//! *consuming* golem needs no Rust/Xcode/Gradle. But *driving* a device still
//! needs host CLIs (`adb`, `xcrun`/`simctl`), a booted device, and a writable
//! `~/.golem`. doctor probes each, prints a copy-paste remediation for every
//! miss, and exits non-zero when the host can drive **no** platform — so CI can
//! gate on it.
//!
//! Design for testability (see the roadmap's I/O-seam note): every external
//! probe goes through the `golem_common::command` seam, and the decision logic
//! is split into pure functions — [`evaluate`] (facts → checks), [`exit_code`],
//! [`render_with_color`], and the `adb devices` / `simctl booted` parsers — so
//! the check/report behaviour is exhaustively unit-testable without real tools.

use std::io::IsTerminal;
use std::path::Path;

use anyhow::Result;
use golem_common::command;

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

/// Probed facts about the host — the sole input to [`evaluate`]. Plain data so
/// evaluation and rendering stay pure and testable without touching real tools.
#[derive(Debug, Clone)]
struct Facts {
    is_macos: bool,
    adb: bool,
    xcrun: bool,
    simctl: bool,
    ffmpeg: bool,
    /// Physical Android devices currently connected (running emulators are AVDs,
    /// counted separately, so they are excluded here to avoid double counting).
    android_connected: usize,
    /// Bootable Android AVDs defined on the host.
    android_avds: usize,
    /// Bootable iOS simulators available on the host.
    ios_sims: usize,
    ios_companion: bool,
    android_companion: bool,
    /// `Ok` if `~/.golem` is writable, else a human-readable reason.
    golem_writable: std::result::Result<(), String>,
}

/// Can golem drive Android on this host? (device CLI present *and* companion
/// embedded).
fn android_drivable(f: &Facts) -> bool {
    f.adb && f.android_companion
}

/// Can golem drive iOS on this host? iOS is macOS-only.
fn ios_drivable(f: &Facts) -> bool {
    f.is_macos && f.xcrun && f.simctl && f.ios_companion
}

/// Turn probed facts into an ordered list of diagnostic lines. Pure.
fn evaluate(f: &Facts) -> Vec<Check> {
    let mut checks = Vec::new();

    // State dir — a hard requirement: companions extract here.
    match &f.golem_writable {
        Ok(()) => checks.push(Check::ok("~/.golem writable", "yes")),
        Err(reason) => checks.push(Check::fail(
            "~/.golem writable",
            reason,
            "fix permissions on ~/.golem (golem extracts embedded companions there)",
        )),
    }

    // Android toolchain + companion. Individual misses are warnings; the
    // "can drive a platform" summary below escalates to Fail if nothing is
    // drivable overall.
    if f.adb {
        checks.push(Check::ok("adb (Android)", "found"));
    } else {
        checks.push(Check::warn(
            "adb (Android)",
            "not on PATH",
            "install platform-tools: `brew install --cask android-platform-tools` (macOS) / `apt-get install android-tools-adb` (Linux)",
        ));
    }
    if f.android_companion {
        checks.push(Check::ok("Android companion", "embedded"));
    } else {
        checks.push(Check::warn(
            "Android companion",
            "not embedded",
            "this binary shipped without the Android companion — reinstall a release built on a host with the Android SDK",
        ));
    }

    // iOS toolchain + companion — macOS only.
    if f.is_macos {
        if f.xcrun {
            checks.push(Check::ok("xcrun (iOS)", "found"));
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
        if f.ios_companion {
            checks.push(Check::ok("iOS companion", "embedded"));
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
    if f.ffmpeg {
        checks.push(Check::ok("ffmpeg (optional)", "found"));
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

/// Process exit code: non-zero if any check failed.
fn exit_code(checks: &[Check]) -> i32 {
    if checks.iter().any(|c| c.status == Status::Fail) {
        1
    } else {
        0
    }
}

/// Render the report. Split on `use_color` so both branches are unit-testable.
fn render_with_color(checks: &[Check], use_color: bool) -> String {
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

    for c in checks {
        let (sym, color) = match c.status {
            Status::Ok => ("✓", GREEN),
            Status::Warn => ("!", YELLOW),
            Status::Fail => ("✗", RED),
        };
        let _ = writeln!(
            out,
            "  {} {} — {}",
            paint(sym, color),
            c.label,
            c.detail
        );
        if let Some(remedy) = &c.remedy {
            let _ = writeln!(out, "{}", paint(&format!("      ↳ {remedy}"), DIM));
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Parsers (pure)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Probing (I/O — via the command seam + filesystem)
// ---------------------------------------------------------------------------

/// True if `program args…` could be spawned at all (Err ⇒ binary not found).
async fn spawns(program: &str, args: &[&str]) -> bool {
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    command::output(program, &args).await.is_ok()
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

/// Gather all facts about the host.
async fn probe() -> Facts {
    let is_macos = cfg!(target_os = "macos");

    let adb = spawns("adb", &["--version"]).await;
    let ffmpeg = spawns("ffmpeg", &["-version"]).await;

    // iOS tooling is macOS-only; skip the probes entirely elsewhere.
    let (xcrun, simctl) = if is_macos {
        (
            spawns("xcrun", &["--version"]).await,
            runs_ok("xcrun", &["simctl", "help"]).await,
        )
    } else {
        (false, false)
    };

    let android_connected = if adb {
        count_adb_physical(&stdout_of("adb", &["devices"]).await)
    } else {
        0
    };
    let android_avds = count_avds_in(&avd_home());
    let ios_sims = if simctl {
        count_sim_devices(&stdout_of("xcrun", &["simctl", "list", "devices", "available"]).await)
    } else {
        0
    };

    Facts {
        is_macos,
        adb,
        xcrun,
        simctl,
        ffmpeg,
        android_connected,
        android_avds,
        ios_sims,
        ios_companion: crate::companions::has_ios_companion(),
        android_companion: crate::companions::has_android_companion(),
        golem_writable: probe_golem_writable(),
    }
}

/// Run `golem doctor`: probe, report to stderr, and return the exit code.
pub async fn run() -> Result<i32> {
    let checks = evaluate(&probe().await);
    let use_color = std::io::stderr().is_terminal();
    eprint!("{}", render_with_color(&checks, use_color));
    Ok(exit_code(&checks))
}

/// Auto-invoke hook: when a run/command hits a "no device" dead-end, print the
/// doctor lines that explain *why* (missing CLI, no booted device, absent
/// companion) instead of a bare error. A thin reuse of the same probe + report
/// logic; scoped to the runtime-dep failure paths so it doesn't balloon.
pub async fn hint_no_device() {
    let checks = evaluate(&probe().await);
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
    eprint!("{}", render_with_color(&actionable, use_color));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_facts() -> Facts {
        Facts {
            is_macos: true,
            adb: true,
            xcrun: true,
            simctl: true,
            ffmpeg: true,
            android_connected: 0,
            android_avds: 1,
            ios_sims: 2,
            ios_companion: true,
            android_companion: true,
            golem_writable: Ok(()),
        }
    }

    // 1. A fully-provisioned macOS host: every line Ok, both platforms drivable,
    //    exit 0.
    #[test]
    fn healthy_macos_is_all_ok() {
        let checks = evaluate(&base_facts());
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
        f.adb = false;
        f.xcrun = false;
        f.simctl = false;
        let checks = evaluate(&f);
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
        f.xcrun = false;
        f.simctl = false;
        f.ios_companion = false;
        let checks = evaluate(&f);
        assert_eq!(exit_code(&checks), 0, "android-only host is still drivable");
    }

    // 4. Toolchain present but its companion missing ⇒ that platform is NOT
    //    drivable. adb without the Android companion, on a non-macOS host, means
    //    nothing is drivable.
    #[test]
    fn missing_companion_makes_platform_undrivable() {
        let mut f = base_facts();
        f.is_macos = false;
        f.xcrun = false;
        f.simctl = false;
        f.ios_companion = false;
        f.android_companion = false; // adb present, but no companion
        let checks = evaluate(&f);
        assert_eq!(exit_code(&checks), 1);
    }

    // 5. A non-writable state dir fails the gate regardless of drivability.
    #[test]
    fn unwritable_state_dir_fails() {
        let mut f = base_facts();
        f.golem_writable = Err("cannot write to /root/.golem: permission denied".to_string());
        let checks = evaluate(&f);
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
        let checks = evaluate(&f);
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
        f.adb = false; // produces a warning with a remedy
        let checks = evaluate(&f);

        let plain = render_with_color(&checks, false);
        assert!(!plain.contains('\x1b'), "no-color output SHALL be escape-free");
        assert!(plain.contains("golem doctor"));
        assert!(
            plain.contains("android-platform-tools"),
            "the remedy for a missing CLI SHALL be shown"
        );

        let colored = render_with_color(&checks, true);
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
        let checks = evaluate(&f);
        let dev = checks
            .iter()
            .find(|c| c.label == "device available")
            .expect("device line");
        assert_eq!(dev.status, Status::Warn, "no device SHALL warn, not fail");
        assert_eq!(exit_code(&checks), 0, "device availability SHALL NOT gate the exit code");
    }

    // 9. adb parser counts only physical online devices — emulators (running
    //    AVDs, counted via the AVD list) and offline/unauthorized rows excluded.
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

    // 10. simctl available parser: counts every bootable device line (Booted or
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

    // 11. AVD counter: counts `<name>.ini` markers in the AVD home, ignoring
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
}
