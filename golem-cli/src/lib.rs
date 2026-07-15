pub mod a11y_extract;
pub mod cache;
pub mod cli;
pub mod companion_paths;
pub mod companions;
pub mod devices;
pub mod doctor;
pub mod discovery;
pub mod install_cache;
pub mod install_script_cmd;
pub mod orchestrator;
pub mod project;
pub mod registration;
pub mod scaffold;
pub mod suite;
pub mod tree;

use std::path::{Path, PathBuf};

use anyhow::Context;

use cli::{Cli, Commands};
use discovery::TagFilter;
use suite::SuiteConfig;

/// Run the CLI to completion, returning the process exit code (`0` = ok,
/// `1` = a flow failed or setup errored). Split out of `main` so
/// integration tests in `tests/` can drive the whole pipeline in-process
/// (real orchestrator + renderer + result files) against the stub driver,
/// without spawning a subprocess. `main` is a thin wrapper that parses
/// argv, calls this, and exits with the returned code.
pub async fn run_cli(cli: Cli) -> anyhow::Result<i32> {
    match cli.command {
        Commands::Run(args) => {
            // Restore any device keyboards golem swapped for its invisible
            // Unicode IME if the user interrupts mid-run. Normal completion
            // restores them via suite teardown; a Ctrl-C skips that, leaving
            // golem's IME active until the next run self-heals. In the default
            // in-process topology the driver's activation registry lives in
            // this process, so we can restore it directly. (A daemon owns its
            // own teardown — interrupting the client doesn't stop the daemon.)
            tokio::spawn(async {
                if tokio::signal::ctrl_c().await.is_ok() {
                    eprintln!("\n  [ime] interrupted — restoring device keyboards...");
                    golem_driver::ime::restore_all().await;
                    std::process::exit(130);
                }
            });

            // Resolve flow paths
            let tag_filters: Vec<TagFilter> =
                args.tags.iter().map(|t| TagFilter::parse(t)).collect();

            let flow_paths: Vec<PathBuf> = if args.files.is_empty() {
                // Discover from current directory
                let flows = discovery::discover_flows(Path::new("."), &tag_filters)?;
                flows.into_iter().map(|f| f.path).collect()
            } else {
                // Expand directories in the file list
                let mut paths = Vec::new();
                for file in &args.files {
                    if file.is_dir() {
                        let flows = discovery::discover_flows(file, &tag_filters)?;
                        paths.extend(flows.into_iter().map(|f| f.path));
                    } else {
                        paths.push(file.clone());
                    }
                }
                paths
            };

            if flow_paths.is_empty() {
                eprintln!("No flow files found.");
                return Ok(1);
            }

            // Build suite config. On a non-macOS host golem can't drive iOS
            // (no simctl, no embedded iOS companion), so default a bare run to
            // Android rather than planning iOS legs that can't run. An explicit
            // --platform is always respected (an explicit --platform ios then
            // fails loudly, which is the right signal on Linux).
            let (effective_platform, platform_defaulted) =
                resolve_platform_default(args.platform.as_deref(), cfg!(target_os = "macos"));
            if platform_defaulted {
                eprintln!(
                    "note: non-macOS host — defaulting to `--platform android` (iOS is unsupported here; pass --platform to override)."
                );
            }
            let platform_override = match parse_platform_override(effective_platform.as_deref()) {
                Ok(p) => p,
                Err(msg) => {
                    eprintln!("{msg}");
                    return Ok(1);
                }
            };

            let coverage_override = match parse_coverage_override(args.coverage.as_deref()) {
                Ok(c) => c,
                Err(msg) => {
                    eprintln!("{msg}");
                    return Ok(1);
                }
            };

            let a11y_override = match parse_a11y_override(args.a11y.as_deref()) {
                Ok(a) => a,
                Err(msg) => {
                    eprintln!("{msg}");
                    return Ok(1);
                }
            };

            let a11y_min_confidence_override =
                match validate_a11y_min_confidence(args.a11y_min_confidence) {
                    Ok(c) => c,
                    Err(msg) => {
                        eprintln!("{msg}");
                        return Ok(1);
                    }
                };

            // Stream human output unless user explicitly chose non-human format.
            // Default (no --output) = human, so stream is on.
            let has_human_output = detect_human_output(&args.outputs);

            let cli_vars = cli::parse_var_args(&args.vars)?;

            // Load project config from golem.toml (walk up from cwd).
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let (project_config, project_toml_path) = project::ProjectConfig::load_from(&cwd)?;
            let project_root = project_toml_path
                .as_ref()
                .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
                .unwrap_or(cwd);

            let output_dir = args
                .output_dir
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from(".golem/results"));

            // Parse stdout output formats.
            let stdout_formats: Vec<golem_report::output::OutputFormat> = args
                .outputs
                .iter()
                .map(|s| golem_report::output::parse_output_format(s))
                .collect::<Result<Vec<_>, _>>()?;

            let include_junit = stdout_formats
                .iter()
                .any(|f| matches!(f, golem_report::output::OutputFormat::Junit));

            // Conflicting flags: --no-build wins over --rebuild because
            // skipping work entirely is the more concrete intent. Warn
            // loudly so the user knows their --rebuild was ignored.
            let rebuild = args.rebuild;
            let no_build = args.no_build;
            if let Some(msg) = rebuild_no_build_conflict_warning(rebuild, no_build) {
                eprintln!("{msg}");
            }

            if let Some(msg) = record_no_record_conflict_warning(args.record, args.no_record) {
                eprintln!("{msg}");
            }

            // `--stub <path>`: resolve to the stub fail-on-runs list, which
            // activates device-free stub mode. `None` in release builds (the
            // stub driver is compiled out) and when the flag is absent.
            let stub_fail_on_runs = resolve_stub(args.stub.as_deref())?;

            let config = SuiteConfig {
                no_clean: args.no_clean,
                no_teardown: args.no_teardown,
                keep_devices: args.keep_devices,
                seed: args.seed,
                platform: platform_override,
                no_perf: args.no_perf,
                verbose: args.verbose,
                debug: args.debug,
                stream_human: has_human_output,
                start: args.start,
                vars: cli_vars,
                output_dir: output_dir.clone(),
                no_results: args.no_results,
                project_root,
                project_apps: project_config.apps,
                coverage_override,
                a11y_override,
                a11y_min_confidence_override,
                rebuild,
                no_build,
                device_settings: project_config.device_settings,
                record: args.record,
                no_record: args.no_record,
                project_record: project_config.options.record,
                trace: args.trace,
                repeat: args.repeat,
                max_device_wait: args
                    .max_wait
                    .as_deref()
                    .and_then(golem_runner::executor::parse_duration),
                stub_fail_on_runs,
                profile: args.profile.clone(),
            };

            // Parse `--max-wait` into milliseconds for the wire. An
            // unparseable value is dropped (None) with a loud warning so
            // the user knows their flag was ignored.
            let max_device_wait_ms = args.max_wait.as_deref().and_then(|s| {
                let d = golem_runner::executor::parse_duration(s);
                if d.is_none() {
                    eprintln!("warning: ignoring --max-wait '{s}' (expected e.g. 30m, 1h, 90s)");
                }
                d.map(|d| d.as_millis() as u64)
            });

            // Wire shape (always identical, whether we're talking to
            // an in-process orchestrator we just started or to an
            // existing daemon): config_json carries everything the
            // server needs to reconstruct SuiteConfig. ProjectAppConfig
            // isn't Serialize, so the server re-parses golem.toml from
            // `project_root`.
            let config_json = build_config_json(
                &config,
                effective_platform.as_deref(),
                args.coverage.as_deref(),
                args.a11y.as_deref(),
                max_device_wait_ms,
                include_junit,
            );

            // Unified submit path: connect to an existing daemon if
            // there is one, otherwise spin up an in-process server and
            // self-connect. `local_server` is `Some(...)` only in the
            // in-process case — that's the marker for "I own the
            // device pool, I must clean it up afterwards." External
            // daemons own their own device lifecycle.
            let (stream, local_server) = match orchestrator::try_connect().await {
                Ok(s) => (s, None),
                Err(_) => {
                    let server = orchestrator::start_server().await?;
                    let s = orchestrator::try_connect()
                        .await
                        .context("failed to connect to in-process orchestrator")?;
                    (s, Some(server))
                }
            };

            let outcome = orchestrator::submit_and_wait(
                stream,
                &flow_paths,
                &config_json,
                config.verbose,
                config.debug,
                has_human_output,
            )
            .await?;
            let report = outcome.report;

            // Drain in-process server: wait for any other clients, then
            // shut down sims/emulators we booted (respects --keep-devices).
            if let Some(server) = local_server {
                server.wait_for_clients().await;
                let warnings = server
                    .resource_mgr
                    .shutdown_golem_booted(args.keep_devices)
                    .await;
                for w in &warnings {
                    eprintln!("  [devices] {w}");
                }
            }

            // Stdout non-human formats from the accumulated report
            // (human formats stream live to stderr via stream renderer).
            for fmt in &stdout_formats {
                if !matches!(fmt, golem_report::output::OutputFormat::Human) {
                    let content = golem_report::output::render(&report, fmt)?;
                    println!("{content}");
                }
            }

            // `--repeat` flake summary: tally per (flow, device) across
            // all repeat runs. Empty (no-op) for single-run suites.
            // Only emit to stderr when human output is active — for
            // toon/json/junit consumers the flake info is part of the
            // structured output itself.
            if has_human_output {
                let flake_summary = build_flake_summary(&report.flows);
                if !flake_summary.is_empty() {
                    eprint!("{}", render_flake_summary(&flake_summary));
                }

                // Host-queue congestion: only surfaces when two same-class
                // heavy ops actually contended (multi-device). Silent at zero
                // so single-device runs stay uncluttered.
                let queue_wait = golem_common::host_queue::queue_wait_stats();
                if !queue_wait.is_zero() {
                    eprint!("{}", render_queue_wait(&queue_wait));
                }
            }

            // Exit with appropriate code. Skipped flows (coverage-group
            // reclassify + install preconditions) don't fail the suite;
            // only genuine failures do. Use `is_failed` (not the
            // simpler `all_passed` from the submit-wire) so install-
            // precondition skips are correctly fatal and coverage-group
            // skips are correctly tolerated.
            let any_failed = report.flows.iter().any(|f| f.is_failed());
            if any_failed {
                // Auto-invoke doctor when the run never acquired a device (a
                // missing-runtime-dep dead-end: no adb/simctl, no booted device),
                // so the user sees *why* instead of a bare failure. Scoped to the
                // never-ran no-device shape to avoid noise on real test failures.
                if has_human_output
                    && report.flows.iter().any(|f| {
                        f.step_results.is_empty()
                            && matches!(
                                f.first_failure_code,
                                Some(golem_events::FailureCode::DeviceNotFound)
                            )
                    })
                {
                    doctor::hint_no_device().await;
                }
                return Ok(1);
            }
            // Quiet the unused-binding warnings for variables now only
            // used in the in-process path. `has_human_output` had been
            // shaping the legacy server-mode "Results:" line; the
            // server side now owns that print.
            let _ = (has_human_output, &output_dir);
        }

        Commands::Tree(args) => {
            tree::run(&args).await?;
        }

        Commands::Devices => {
            let mut all_devices = golem_devices::ios::discover_ios_devices().await?;
            if let Ok(android) = golem_devices::android::discover_android_devices().await {
                all_devices.extend(android);
            }
            let output = devices::format_device_list(&all_devices);
            println!("{output}");
        }

        Commands::Init => {
            scaffold::init_project(Path::new("."))?;
            println!("Project initialized.");
        }

        Commands::Create(args) => {
            let path = scaffold::create_flow(&args.name, Path::new("."))?;
            println!("Created flow: {}", path.display());
        }

        Commands::InstallScript => {
            install_script_cmd::run()?;
        }

        Commands::Cache(args) => match args.command {
            cli::CacheCommands::Info => cache::info()?,
            cli::CacheCommands::Clear => cache::clear()?,
        },

        Commands::A11yExtract(args) => {
            a11y_extract::run(&args)?;
        }

        Commands::Doctor(args) => {
            // doctor owns its own exit code (non-zero when a checked mode can't
            // proceed) so CI can gate on it — return it directly.
            return doctor::run(&args).await;
        }
    }

    Ok(0)
}

/// Resolve the `--stub <path>` flag to the stub script's `fail_on_runs`
/// list. `Some(_)` activates device-free stub mode. In release builds the
/// stub driver is compiled out, so this is always `None` regardless of the
/// flag (the hidden flag is inert there).
#[cfg(debug_assertions)]
fn resolve_stub(path: Option<&Path>) -> anyhow::Result<Option<Vec<u32>>> {
    match path {
        None => Ok(None),
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .with_context(|| format!("failed to read stub script {}", p.display()))?;
            let script = golem_driver::stub::StubScript::from_toml_str(&text)
                .with_context(|| format!("failed to parse stub script {}", p.display()))?;
            Ok(Some(script.fail_on_runs))
        }
    }
}

#[cfg(not(debug_assertions))]
fn resolve_stub(_path: Option<&Path>) -> anyhow::Result<Option<Vec<u32>>> {
    Ok(None)
}

/// Map the raw `--platform` string to a [`Platform`]. `None` (flag
/// absent) maps to `Ok(None)`; an unrecognized value is reported as an
/// error message for the caller to print before exiting.
fn parse_platform_override(
    platform: Option<&str>,
) -> Result<Option<golem_devices::Platform>, String> {
    match platform {
        None => Ok(None),
        Some("android") => Ok(Some(golem_devices::Platform::Android)),
        Some("ios") => Ok(Some(golem_devices::Platform::Ios)),
        Some(other) => Err(format!(
            "Unknown platform: {other}. Use 'ios' or 'android'."
        )),
    }
}

/// Resolve the effective `--platform` for a run. On a non-macOS host, an unset
/// platform defaults to `android` (golem can't drive iOS there — no simctl, no
/// embedded iOS companion), avoiding iOS legs that can't run. An explicit value
/// always passes through unchanged. Returns `(effective, defaulted?)` so the
/// caller can print a one-line notice only when it actually defaulted.
fn resolve_platform_default(explicit: Option<&str>, is_macos: bool) -> (Option<String>, bool) {
    match explicit {
        Some(p) => (Some(p.to_string()), false),
        None if !is_macos => (Some("android".to_string()), true),
        None => (None, false),
    }
}

/// Map the raw `--coverage` string to a [`CoverageStrategy`]. `None`
/// (flag absent) maps to `Ok(None)`; an unrecognized value is reported
/// as an error message for the caller to print before exiting.
fn parse_coverage_override(
    coverage: Option<&str>,
) -> Result<Option<golem_parser::CoverageStrategy>, String> {
    match coverage {
        None => Ok(None),
        Some("one") => Ok(Some(golem_parser::CoverageStrategy::One)),
        Some("min") => Ok(Some(golem_parser::CoverageStrategy::Min)),
        Some("smart") => Ok(Some(golem_parser::CoverageStrategy::Smart)),
        Some("full") => Ok(Some(golem_parser::CoverageStrategy::Full)),
        Some(other) => Err(format!(
            "Unknown coverage: {other}. Use 'one', 'min', 'smart', or 'full'."
        )),
    }
}

fn parse_a11y_override(a11y: Option<&str>) -> Result<Option<golem_parser::A11yLevel>, String> {
    match a11y {
        None => Ok(None),
        Some("off") => Ok(Some(golem_parser::A11yLevel::Off)),
        Some("critical") => Ok(Some(golem_parser::A11yLevel::Critical)),
        Some("relaxed") => Ok(Some(golem_parser::A11yLevel::Relaxed)),
        Some("strict") => Ok(Some(golem_parser::A11yLevel::Strict)),
        Some(other) => Err(format!(
            "Unknown a11y level: {other}. Use 'off', 'critical', 'relaxed', or 'strict'."
        )),
    }
}

/// Validate the `--a11y-min-confidence` flag: must be in `0.0..=1.0` (a
/// confidence is a probability). `None` (absent flag) passes through.
fn validate_a11y_min_confidence(c: Option<f32>) -> Result<Option<f32>, String> {
    match c {
        Some(v) if !(0.0..=1.0).contains(&v) => Err(format!(
            "--a11y-min-confidence must be between 0.0 and 1.0 (got {v})"
        )),
        other => Ok(other),
    }
}

/// Whether human (streamed) output is active. Default (no `--output`)
/// is human, so an empty list streams; an explicit list streams only
/// if it contains a `human` or `human:`-prefixed format.
fn detect_human_output(outputs: &[String]) -> bool {
    outputs.is_empty()
        || outputs
            .iter()
            .any(|s| s == "human" || s.starts_with("human:"))
}

/// The conflict warning shown when both `--rebuild` and `--no-build`
/// are passed. `--no-build` wins; `None` when there is no conflict.
fn rebuild_no_build_conflict_warning(rebuild: bool, no_build: bool) -> Option<&'static str> {
    if rebuild && no_build {
        Some(
            "  [install] both --rebuild and --no-build passed — \
             --no-build wins (skipping build+install)",
        )
    } else {
        None
    }
}

/// The conflict warning shown when both `--record` and `--no-record`
/// are passed. `--no-record` wins; `None` when there is no conflict.
fn record_no_record_conflict_warning(record: bool, no_record: bool) -> Option<&'static str> {
    if record && no_record {
        Some(
            "  [record] both --record and --no-record passed — \
             --no-record wins (recording disabled)",
        )
    } else {
        None
    }
}

/// Build the wire `config_json` from a resolved [`SuiteConfig`] plus the
/// raw `--platform` / `--coverage` strings (carried verbatim for the
/// server to re-derive), the already-parsed `--max-wait` milliseconds,
/// and whether JUnit output is requested.
fn build_config_json(
    config: &SuiteConfig,
    platform: Option<&str>,
    coverage: Option<&str>,
    a11y: Option<&str>,
    max_device_wait_ms: Option<u64>,
    include_junit: bool,
) -> serde_json::Value {
    serde_json::json!({
        "platform": platform,
        "seed": config.seed,
        "verbose": config.verbose,
        "debug": config.debug,
        "no_perf": config.no_perf,
        "no_clean": config.no_clean,
        "no_teardown": config.no_teardown,
        "keep_devices": config.keep_devices,
        "no_results": config.no_results,
        "start": config.start,
        "vars": config.vars,
        "output_dir": config.output_dir.display().to_string(),
        "project_root": config.project_root.display().to_string(),
        "coverage": coverage,
        "a11y": a11y,
        "a11y_min_confidence": config.a11y_min_confidence_override,
        "rebuild": config.rebuild,
        "no_build": config.no_build,
        "record": config.record,
        "no_record": config.no_record,
        "trace": config.trace,
        "repeat": config.repeat,
        "max_device_wait_ms": max_device_wait_ms,
        "profile": config.profile,
        "include_junit": include_junit,
        // Stub mode (in-process integration tests): an array (possibly
        // empty) activates it; null/absent = real devices. Always null in a
        // release binary. Carried inline so the server needs no file access.
        "stub_fail_on_runs": config.stub_fail_on_runs,
    })
}

use golem_report::flake::{build_summary as build_flake_summary, FlakeEntry};

fn render_flake_summary(entries: &[FlakeEntry]) -> String {
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());
    render_flake_summary_with_color(entries, use_color)
}

fn render_flake_summary_with_color(entries: &[FlakeEntry], use_color: bool) -> String {
    use std::fmt::Write;
    // Match the stream renderer's continuation indent — `Summary` /
    // `Results:` lines start at column 13 (after the timestamp + space),
    // so the flake block aligns when piped beside them.
    const INDENT: &str = "             ";
    const DIM: &str = "\x1b[2m";
    const CYAN: &str = "\x1b[36m";
    const RESET: &str = "\x1b[0m";
    const BOLD_RED: &str = "\x1b[1;31m";
    const BOLD_YELLOW: &str = "\x1b[1;33m";
    const BOLD_GREEN: &str = "\x1b[1;32m";

    let total_runs = entries.first().map(|e| e.total).unwrap_or(0);
    let flakes = entries
        .iter()
        .filter(|e| e.passed > 0 && e.failed > 0)
        .count();
    let stable_fails = entries
        .iter()
        .filter(|e| e.passed == 0 && e.failed > 0)
        .count();
    let stable_passes = entries
        .iter()
        .filter(|e| e.failed == 0 && e.passed > 0)
        .count();

    let header_body = format!(
        "── {flakes} flake{}, {stable_fails} fail{}, {stable_passes} stable across {total_runs} runs ──",
        if flakes == 1 { "" } else { "s" },
        if stable_fails == 1 { "" } else { "s" },
    );

    let mut out = String::new();
    if use_color {
        let _ = writeln!(out, "\n{INDENT}{CYAN}{header_body}{RESET}");
    } else {
        let _ = writeln!(out, "\n{INDENT}{header_body}");
    }

    if flakes == 0 && stable_fails == 0 {
        let line = "all flows passed in every run";
        if use_color {
            let _ = writeln!(out, "{INDENT}{DIM}{line}{RESET}");
        } else {
            let _ = writeln!(out, "{INDENT}{line}");
        }
        return out;
    }

    for e in entries {
        let (label, label_color) = if e.passed > 0 && e.failed > 0 {
            ("FLAKE", BOLD_YELLOW)
        } else if e.failed > 0 {
            ("FAIL ", BOLD_RED)
        } else {
            ("PASS ", BOLD_GREEN)
        };
        let body = format!("{:>3}/{:<3}  {}", e.passed, e.total, e.flow);
        if use_color {
            // Dim stable PASS rows — they're informational once flakes
            // and stable fails are listed above; eye should land on the
            // problematic rows first.
            let body_color = if e.failed == 0 && e.passed > 0 {
                DIM
            } else {
                ""
            };
            let body_reset = if e.failed == 0 && e.passed > 0 {
                RESET
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "{INDENT}{label_color}{label}{RESET}  {body_color}{body}{body_reset}",
            );
        } else {
            let _ = writeln!(out, "{INDENT}{label}  {body}");
        }
    }
    out
}

fn render_queue_wait(stats: &golem_common::host_queue::QueueWaitStats) -> String {
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());
    render_queue_wait_with_color(stats, use_color)
}

/// One-line host-queue congestion summary, aligned under the `Summary` block.
/// Callers only invoke this when `stats` is non-zero.
fn render_queue_wait_with_color(
    stats: &golem_common::host_queue::QueueWaitStats,
    use_color: bool,
) -> String {
    use std::fmt::Write;
    const INDENT: &str = "             ";
    const DIM: &str = "\x1b[2m";
    const RESET: &str = "\x1b[0m";

    let secs = |d: std::time::Duration| format!("{:.1}s", d.as_secs_f64());
    let breakdown = stats
        .per_class
        .iter()
        .map(|c| format!("{} {} ×{}", c.class.label(), secs(c.waited), c.count))
        .collect::<Vec<_>>()
        .join(", ");
    let body = format!("queue wait: {} total  ({breakdown})", secs(stats.total));

    let mut out = String::new();
    if use_color {
        let _ = writeln!(out, "{INDENT}{DIM}{body}{RESET}");
    } else {
        let _ = writeln!(out, "{INDENT}{body}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_common::host_queue::{ClassWait, OpClass, QueueWaitStats};
    use golem_events::RepeatContext;
    use golem_report::FlowReport;

    #[test]
    fn queue_wait_line_lists_classes_worst_first() {
        let stats = QueueWaitStats {
            total: std::time::Duration::from_millis(3900),
            per_class: vec![
                ClassWait {
                    class: OpClass::Screenshot,
                    waited: std::time::Duration::from_millis(2800),
                    count: 63,
                },
                ClassWait {
                    class: OpClass::Dumpsys,
                    waited: std::time::Duration::from_millis(1100),
                    count: 40,
                },
            ],
        };
        let line = render_queue_wait_with_color(&stats, false);
        assert!(line.contains("queue wait: 3.9s total"), "got: {line}");
        assert!(line.contains("screenshot 2.8s ×63"), "got: {line}");
        assert!(line.contains("dumpsys 1.1s ×40"), "got: {line}");
        // Worst-first ordering: screenshot precedes dumpsys.
        let (s, d) = (
            line.find("screenshot").expect("screenshot listed"),
            line.find("dumpsys").expect("dumpsys listed"),
        );
        assert!(s < d, "classes SHALL be listed worst-first: {line}");
    }

    fn flow(name: &str, success: bool, repeat: Option<RepeatContext>, skipped: bool) -> FlowReport {
        // `is_skipped` requires success=true + skipped_reason=Some
        // (a coverage-group skip). Install-precondition skips keep
        // success=false and classify as failures — we use the
        // coverage-skip shape here so `skipped` rows actually count.
        FlowReport {
            flow_name: name.to_string(),
            success: if skipped { true } else { success },
            skipped_reason: if skipped {
                Some("coverage group satisfied".into())
            } else {
                None
            },
            device_name: Some("iPhone 17".into()),
            repeat,
            ..FlowReport::default()
        }
    }

    #[test]
    fn flake_summary_empty_when_no_repeat() {
        // Single-run suites — every flow has repeat=None — get no flake summary.
        let flows = vec![
            flow("a.test", true, None, false),
            flow("b.test", false, None, false),
        ];
        assert!(build_flake_summary(&flows).is_empty());
    }

    #[test]
    fn flake_summary_tallies_pass_fail_skip() {
        let r = |i| Some(RepeatContext { index: i, total: 3 });
        let flows = vec![
            flow("a.test", true, r(0), false),
            flow("a.test", false, r(1), false),
            flow("a.test", true, r(2), false),
            flow("b.test", true, r(0), false),
            flow("b.test", true, r(1), false),
            flow("b.test", true, r(2), false),
            flow("c.test", false, r(0), true),
            flow("c.test", false, r(1), true),
            flow("c.test", false, r(2), true),
        ];
        let summary = build_flake_summary(&flows);
        let by_name: std::collections::HashMap<_, _> =
            summary.iter().map(|e| (e.flow.clone(), e)).collect();

        let a = by_name.get("a.test (iPhone 17)").expect("a entry");
        assert_eq!((a.passed, a.failed, a.skipped, a.total), (2, 1, 0, 3));

        let b = by_name.get("b.test (iPhone 17)").expect("b entry");
        assert_eq!((b.passed, b.failed, b.skipped, b.total), (3, 0, 0, 3));

        let c = by_name.get("c.test (iPhone 17)").expect("c entry");
        // Skipped flows count as skipped (success-false-with-skipped-reason
        // is still a skip for tally purposes).
        assert_eq!((c.passed, c.failed, c.skipped, c.total), (0, 0, 3, 3));
    }

    #[test]
    fn flake_summary_sort_is_flakes_first_then_failures_then_passes() {
        let r = |i| Some(RepeatContext { index: i, total: 3 });
        let flows = vec![
            // pass-all
            flow("z_pass.test", true, r(0), false),
            flow("z_pass.test", true, r(1), false),
            flow("z_pass.test", true, r(2), false),
            // stable fail
            flow("m_fail.test", false, r(0), false),
            flow("m_fail.test", false, r(1), false),
            flow("m_fail.test", false, r(2), false),
            // flake (1/3)
            flow("a_flake.test", true, r(0), false),
            flow("a_flake.test", false, r(1), false),
            flow("a_flake.test", false, r(2), false),
        ];
        let summary = build_flake_summary(&flows);
        let names: Vec<&str> = summary.iter().map(|e| e.flow.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "a_flake.test (iPhone 17)",
                "m_fail.test (iPhone 17)",
                "z_pass.test (iPhone 17)"
            ],
            "SHALL sort flakes first, then stable failures, then stable passes",
        );
    }

    fn entry(flow: &str, passed: u32, failed: u32, skipped: u32, total: u32) -> FlakeEntry {
        FlakeEntry {
            flow: flow.to_string(),
            passed,
            failed,
            skipped,
            total,
        }
    }

    // render_flake_summary runs under nextest with a non-terminal stderr,
    // so `use_color` is false and the assertions below target the plain
    // (no-ANSI) branch deterministically.

    // 1. Empty entries → header reports 0/0/0 across 0 runs and the
    //    "all flows passed" line (no flakes, no stable fails).
    #[test]
    fn render_empty_entries_reports_all_passed() {
        let out = render_flake_summary(&[]);
        assert!(
            out.contains("0 flakes, 0 fails, 0 stable across 0 runs"),
            "empty input SHALL render a zeroed header, got: {out}",
        );
        assert!(
            out.contains("all flows passed in every run"),
            "no flakes + no stable fails SHALL emit the all-passed line, got: {out}",
        );
    }

    // 2. All-stable-pass entries → all-passed line, total taken from the
    //    first entry's `total`, never the count of ANSI codes.
    #[test]
    fn render_all_stable_pass_uses_first_entry_total() {
        let entries = vec![entry("a (dev)", 3, 0, 0, 3), entry("b (dev)", 3, 0, 0, 3)];
        let out = render_flake_summary(&entries);
        assert!(
            out.contains("0 flakes, 0 fails, 2 stable across 3 runs"),
            "two stable passes over 3 runs SHALL be summarized, got: {out}",
        );
        assert!(
            out.contains("all flows passed in every run"),
            "all-stable-pass SHALL short-circuit to the all-passed line, got: {out}",
        );
        // The early-return path SHALL NOT list any per-flow PASS rows.
        assert!(
            !out.contains("PASS"),
            "all-passed short-circuit SHALL omit per-flow rows, got: {out}",
        );
    }

    // 3. Singular pluralization: exactly one flake and one stable fail →
    //    "flake" and "fail" with no trailing 's'.
    #[test]
    fn render_header_singular_pluralization() {
        let entries = vec![
            entry("flaky (dev)", 1, 2, 0, 3),
            entry("broken (dev)", 0, 3, 0, 3),
        ];
        let out = render_flake_summary(&entries);
        assert!(
            out.contains("1 flake, 1 fail, 0 stable across 3 runs"),
            "single flake + single fail SHALL render singular nouns, got: {out}",
        );
    }

    // 4. Plural pluralization: two flakes and two stable fails → "flakes"
    //    and "fails" with trailing 's'.
    #[test]
    fn render_header_plural_pluralization() {
        let entries = vec![
            entry("f1 (dev)", 1, 2, 0, 3),
            entry("f2 (dev)", 2, 1, 0, 3),
            entry("x1 (dev)", 0, 3, 0, 3),
            entry("x2 (dev)", 0, 3, 0, 3),
        ];
        let out = render_flake_summary(&entries);
        assert!(
            out.contains("2 flakes, 2 fails, 0 stable across 3 runs"),
            "two flakes + two fails SHALL render plural nouns, got: {out}",
        );
    }

    // 5. Per-entry labels: a flake row, a stable-fail row, and a stable-pass
    //    row each get their distinct label and `passed/total  flow` body.
    #[test]
    fn render_rows_label_each_category() {
        let entries = vec![
            entry("flaky (dev)", 1, 2, 0, 3),
            entry("broken (dev)", 0, 3, 0, 3),
            entry("good (dev)", 3, 0, 0, 3),
        ];
        let out = render_flake_summary(&entries);
        assert!(
            out.contains("FLAKE    1/3    flaky (dev)"),
            "flake row SHALL be labelled FLAKE with passed/total body, got: {out}"
        );
        assert!(
            out.contains("FAIL     0/3    broken (dev)"),
            "stable-fail row SHALL be labelled FAIL, got: {out}"
        );
        assert!(
            out.contains("PASS     3/3    good (dev)"),
            "stable-pass row SHALL be labelled PASS, got: {out}"
        );
        // Header counts the categories correctly: 1 flake, 1 fail, 1 stable.
        assert!(
            out.contains("1 flake, 1 fail, 1 stable across 3 runs"),
            "header SHALL tally one of each category, got: {out}"
        );
    }

    // 6. A skipped-only entry (passed=0, failed=0) is neither flake nor
    //    stable-fail, so the header counts no flakes/fails, but because a
    //    stable-pass also exists the per-flow rows still render (no early
    //    return) — the skipped row is labelled PASS (the else branch).
    #[test]
    fn render_skipped_only_entry_takes_pass_label_branch() {
        let entries = vec![
            entry("skip (dev)", 0, 0, 3, 3),
            entry("flaky (dev)", 1, 2, 0, 3),
        ];
        let out = render_flake_summary(&entries);
        // total_runs comes from the FIRST entry.
        assert!(
            out.contains("1 flake, 0 fails,"),
            "one flake, no stable fails SHALL be tallied, got: {out}"
        );
        // skip row: passed==0 && failed==0 → else branch → PASS label.
        assert!(
            out.contains("PASS     0/3    skip (dev)"),
            "a fully-skipped entry SHALL fall to the PASS label branch, got: {out}"
        );
        assert!(
            out.contains("FLAKE    1/3    flaky (dev)"),
            "flake row SHALL render, got: {out}"
        );
    }

    // 8. Color injection: the same sample flake summary SHALL render with
    //    ANSI escapes when use_color=true and without any when false, while
    //    the human-readable content (header tally + per-flow bodies) stays
    //    identical between the two.
    #[test]
    fn render_with_color_emits_ansi_only_when_enabled() {
        let entries = vec![
            entry("flaky (dev)", 1, 2, 0, 3),
            entry("broken (dev)", 0, 3, 0, 3),
            entry("good (dev)", 3, 0, 0, 3),
        ];

        let plain = render_flake_summary_with_color(&entries, false);
        let colored = render_flake_summary_with_color(&entries, true);

        // The plain path SHALL be byte-identical to the TTY-derived default
        // when stderr is not a terminal (as it isn't under nextest).
        assert_eq!(
            plain,
            render_flake_summary(&entries),
            "non-colored path SHALL match the real (non-TTY) render verbatim",
        );

        // 1. use_color=false SHALL emit no ANSI escape sequences at all.
        assert!(
            !plain.contains('\x1b'),
            "non-colored render SHALL contain no ESC byte, got: {plain:?}",
        );
        // 2. use_color=true SHALL emit ANSI escape sequences.
        assert!(
            colored.contains('\x1b'),
            "colored render SHALL contain ANSI escapes, got: {colored:?}",
        );
        // 3. The colored render SHALL carry the category label colors.
        assert!(
            colored.contains("\x1b[1;33mFLAKE"),
            "FLAKE row SHALL be bold-yellow, got: {colored:?}"
        );
        assert!(
            colored.contains("\x1b[1;31mFAIL "),
            "FAIL row SHALL be bold-red, got: {colored:?}"
        );
        assert!(
            colored.contains("\x1b[1;32mPASS "),
            "PASS row SHALL be bold-green, got: {colored:?}"
        );

        // 4. Stripping ANSI from the colored output SHALL reproduce the
        //    plain output exactly — color is purely decorative.
        let stripped = strip_ansi(&colored);
        assert_eq!(
            stripped, plain,
            "colored output with escapes removed SHALL equal the plain output",
        );
    }

    // Color injection over the all-passed short-circuit: the dim "all flows
    // passed" line is wrapped in ANSI only when use_color=true.
    #[test]
    fn render_with_color_dims_all_passed_line() {
        let entries = vec![entry("a (dev)", 3, 0, 0, 3)];

        let plain = render_flake_summary_with_color(&entries, false);
        let colored = render_flake_summary_with_color(&entries, true);

        assert!(plain.contains("all flows passed in every run"));
        assert!(
            !plain.contains('\x1b'),
            "plain all-passed SHALL have no ANSI, got: {plain:?}"
        );
        // \x1b[2m is the DIM code wrapping the all-passed line.
        assert!(
            colored.contains("\x1b[2mall flows passed in every run\x1b[0m"),
            "colored all-passed line SHALL be dimmed, got: {colored:?}",
        );
        assert_eq!(
            strip_ansi(&colored),
            plain,
            "stripping color SHALL match plain"
        );
    }

    // Minimal ANSI stripper for the color-injection assertions above —
    // removes CSI sequences of the form ESC '[' ... 'm'.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Skip until (and including) the terminating 'm'.
                for n in chars.by_ref() {
                    if n == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    // 7. parse_platform_override: absent flag → None; known values map to
    //    the matching Platform; an unknown value SHALL be an error message.
    #[test]
    fn parse_platform_override_maps_known_and_rejects_unknown() {
        // 1. Absent flag SHALL be None (no override).
        assert_eq!(
            parse_platform_override(None).expect("none SHALL be Ok"),
            None
        );
        // 2. Known values SHALL map to the matching Platform.
        assert_eq!(
            parse_platform_override(Some("android")).expect("android SHALL be Ok"),
            Some(golem_devices::Platform::Android),
        );
        assert_eq!(
            parse_platform_override(Some("ios")).expect("ios SHALL be Ok"),
            Some(golem_devices::Platform::Ios),
        );
        // 3. Unknown value (incl. wrong case) SHALL be a descriptive error.
        let err = parse_platform_override(Some("IOS")).expect_err("uppercase SHALL be rejected");
        assert!(
            err.contains("Unknown platform: IOS") && err.contains("'ios' or 'android'"),
            "error SHALL name the bad value and the valid options, got: {err}",
        );
    }

    // 7b. resolve_platform_default: non-macOS defaults an unset platform to
    //     android (and flags it), macOS leaves it None, and an explicit value
    //     always passes through unchanged (even --platform ios on Linux).
    #[test]
    fn resolve_platform_default_defaults_android_off_macos() {
        // Unset on Linux → android, defaulted.
        assert_eq!(
            resolve_platform_default(None, false),
            (Some("android".to_string()), true)
        );
        // Unset on macOS → no override, not defaulted.
        assert_eq!(resolve_platform_default(None, true), (None, false));
        // Explicit always passes through, never flagged as defaulted.
        assert_eq!(
            resolve_platform_default(Some("ios"), false),
            (Some("ios".to_string()), false),
            "explicit --platform ios SHALL pass through on Linux (fails loudly later)"
        );
        assert_eq!(
            resolve_platform_default(Some("android"), true),
            (Some("android".to_string()), false)
        );
    }

    // 8. parse_coverage_override: absent flag → None; each strategy maps;
    //    an unknown value SHALL be an error message.
    #[test]
    fn parse_coverage_override_maps_known_and_rejects_unknown() {
        use golem_parser::CoverageStrategy;
        // 1. Absent flag SHALL be None.
        assert_eq!(
            parse_coverage_override(None).expect("none SHALL be Ok"),
            None
        );
        // 2. Each known strategy SHALL map to its variant.
        for (s, want) in [
            ("one", CoverageStrategy::One),
            ("min", CoverageStrategy::Min),
            ("smart", CoverageStrategy::Smart),
            ("full", CoverageStrategy::Full),
        ] {
            assert_eq!(
                parse_coverage_override(Some(s)).expect("known coverage SHALL be Ok"),
                Some(want),
                "{s} SHALL map to {want:?}",
            );
        }
        // 3. Unknown value SHALL be a descriptive error.
        let err =
            parse_coverage_override(Some("none")).expect_err("bad coverage SHALL be rejected");
        assert!(
            err.contains("Unknown coverage: none")
                && err.contains("'one', 'min', 'smart', or 'full'"),
            "error SHALL name the bad value and the valid options, got: {err}",
        );
    }

    // 8b. validate_a11y_min_confidence: absent → None; in-range passes;
    //     out-of-range (below 0 or above 1) SHALL be a descriptive error.
    #[test]
    fn validate_a11y_min_confidence_range() {
        assert_eq!(
            validate_a11y_min_confidence(None).expect("absent SHALL be Ok"),
            None
        );
        for v in [0.0_f32, 0.5, 1.0] {
            assert_eq!(
                validate_a11y_min_confidence(Some(v)).expect("in-range SHALL be Ok"),
                Some(v),
                "{v} SHALL pass through",
            );
        }
        for bad in [-0.1_f32, 1.1] {
            let err = validate_a11y_min_confidence(Some(bad))
                .expect_err("out-of-range SHALL be rejected");
            assert!(
                err.contains("between 0.0 and 1.0"),
                "error SHALL name the valid range, got: {err}",
            );
        }
    }

    // 9. detect_human_output: empty list defaults to human; explicit list
    //    streams only when a human (or human:-prefixed) format is present.
    #[test]
    fn detect_human_output_default_and_explicit() {
        // 1. Empty (no --output) SHALL default to human streaming.
        assert!(detect_human_output(&[]), "empty outputs SHALL stream human");
        // 2. Explicit human (bare or with suboptions) SHALL stream.
        assert!(
            detect_human_output(&["human".to_string()]),
            "bare human SHALL stream"
        );
        assert!(
            detect_human_output(&["human:color".to_string()]),
            "human: prefixed SHALL stream",
        );
        assert!(
            detect_human_output(&["json".to_string(), "human".to_string()]),
            "human alongside others SHALL stream",
        );
        // 3. Explicit non-human-only SHALL NOT stream.
        assert!(
            !detect_human_output(&["json".to_string()]),
            "json-only SHALL NOT stream human",
        );
        assert!(
            !detect_human_output(&["toon".to_string(), "junit".to_string()]),
            "no human format SHALL NOT stream human",
        );
        // 4. A format that merely contains 'human' but is not 'human' nor
        //    'human:'-prefixed SHALL NOT stream (prefix match, not substring).
        assert!(
            !detect_human_output(&["nonhuman".to_string()]),
            "substring 'human' SHALL NOT count as human output",
        );
    }

    // 10. Conflict warnings: present only when BOTH opposing flags are set,
    //     and absent otherwise.
    #[test]
    fn conflict_warnings_only_when_both_flags_set() {
        // 1. rebuild + no_build SHALL warn that --no-build wins.
        let w = rebuild_no_build_conflict_warning(true, true).expect("both build flags SHALL warn");
        assert!(
            w.contains("--no-build wins") && w.contains("[install]"),
            "rebuild/no-build warning SHALL state --no-build wins, got: {w}",
        );
        // 2. No conflict for any single/neither flag.
        assert!(rebuild_no_build_conflict_warning(true, false).is_none());
        assert!(rebuild_no_build_conflict_warning(false, true).is_none());
        assert!(rebuild_no_build_conflict_warning(false, false).is_none());
        // 3. record + no_record SHALL warn that --no-record wins.
        let r =
            record_no_record_conflict_warning(true, true).expect("both record flags SHALL warn");
        assert!(
            r.contains("--no-record wins") && r.contains("[record]"),
            "record/no-record warning SHALL state --no-record wins, got: {r}",
        );
        // 4. No conflict for any single/neither flag.
        assert!(record_no_record_conflict_warning(true, false).is_none());
        assert!(record_no_record_conflict_warning(false, true).is_none());
        assert!(record_no_record_conflict_warning(false, false).is_none());
    }

    // 11. build_config_json: every wire key carries the resolved value;
    //     raw platform/coverage strings and the parsed max-wait pass through.
    #[test]
    fn build_config_json_carries_resolved_values() {
        let config = SuiteConfig {
            seed: Some(42),
            verbose: true,
            debug: false,
            no_perf: true,
            no_clean: false,
            no_teardown: true,
            keep_devices: false,
            no_results: true,
            start: Some("home".to_string()),
            rebuild: true,
            no_build: false,
            record: false,
            no_record: true,
            trace: true,
            repeat: 3,
            a11y_min_confidence_override: Some(0.7),
            output_dir: std::path::PathBuf::from(".golem/results"),
            project_root: std::path::PathBuf::from("/proj"),
            ..SuiteConfig::default()
        };
        let json = build_config_json(
            &config,
            Some("ios"),
            Some("smart"),
            Some("strict"),
            Some(1800000),
            true,
        );

        // 1. Raw flag strings SHALL pass through verbatim.
        assert_eq!(json["platform"], "ios", "platform SHALL be the raw string");
        assert_eq!(json["a11y"], "strict", "a11y SHALL be the raw string");
        assert!(
            json["a11y_min_confidence"]
                .as_f64()
                .is_some_and(|v| (v - 0.7).abs() < 1e-6),
            "a11y_min_confidence SHALL be mirrored onto the wire, got {}",
            json["a11y_min_confidence"],
        );
        assert_eq!(
            json["coverage"], "smart",
            "coverage SHALL be the raw string"
        );
        // 2. Scalar config fields SHALL be mirrored onto the wire.
        assert_eq!(json["seed"], 42);
        assert_eq!(json["verbose"], true);
        assert_eq!(json["no_perf"], true);
        assert_eq!(json["no_teardown"], true);
        assert_eq!(json["no_results"], true);
        assert_eq!(json["start"], "home");
        assert_eq!(json["rebuild"], true);
        assert_eq!(json["no_record"], true);
        assert_eq!(json["trace"], true);
        assert_eq!(json["repeat"], 3);
        // 3. Paths SHALL be rendered as display strings.
        assert_eq!(json["project_root"], "/proj");
        assert_eq!(json["output_dir"], ".golem/results");
        // 4. Parsed max-wait and JUnit flag SHALL pass through.
        assert_eq!(json["max_device_wait_ms"], 1800000u64);
        assert_eq!(json["include_junit"], true);
    }

    // 12. build_config_json: absent optional inputs serialize to JSON null.
    #[test]
    fn build_config_json_absent_options_are_null() {
        let config = SuiteConfig::default();
        let json = build_config_json(&config, None, None, None, None, false);
        assert!(
            json["platform"].is_null(),
            "absent --platform SHALL be null"
        );
        assert!(
            json["coverage"].is_null(),
            "absent --coverage SHALL be null"
        );
        assert!(
            json["max_device_wait_ms"].is_null(),
            "absent --max-wait SHALL be null",
        );
        assert_eq!(json["include_junit"], false);
    }
}
