pub mod cli;
pub mod companion_paths;
pub mod companions;
pub mod devices;
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

use clap::Parser;

use anyhow::Context;

use cli::{Cli, Commands};
use discovery::TagFilter;
use suite::SuiteConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(args) => {
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
                std::process::exit(1);
            }

            // Build suite config
            let platform_override = args.platform.as_deref().map(|p| match p {
                "android" => golem_devices::Platform::Android,
                "ios" => golem_devices::Platform::Ios,
                other => {
                    eprintln!("Unknown platform: {other}. Use 'ios' or 'android'.");
                    std::process::exit(1);
                }
            });

            let coverage_override = args.coverage.as_deref().map(|c| match c {
                "one" => golem_parser::CoverageStrategy::One,
                "min" => golem_parser::CoverageStrategy::Min,
                "smart" => golem_parser::CoverageStrategy::Smart,
                "full" => golem_parser::CoverageStrategy::Full,
                other => {
                    eprintln!("Unknown coverage: {other}. Use 'one', 'min', 'smart', or 'full'.");
                    std::process::exit(1);
                }
            });

            // Stream human output unless user explicitly chose non-human format.
            // Default (no --output) = human, so stream is on.
            let has_human_output = args.outputs.is_empty()
                || args.outputs.iter().any(|s| s == "human" || s.starts_with("human:"));

            let cli_vars = cli::parse_var_args(&args.vars)?;

            // Load project config from golem.toml (walk up from cwd).
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let (project_config, project_toml_path) = project::ProjectConfig::load_from(&cwd)?;
            let project_root = project_toml_path
                .as_ref()
                .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
                .unwrap_or(cwd);

            let output_dir = args.output_dir
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from(".golem/results"));

            // Parse stdout output formats.
            let stdout_formats: Vec<golem_report::output::OutputFormat> = args
                .outputs
                .iter()
                .map(|s| golem_report::output::parse_output_format(s))
                .collect::<Result<Vec<_>, _>>()?;

            let include_junit = stdout_formats.iter().any(|f| matches!(f, golem_report::output::OutputFormat::Junit));

            // Conflicting flags: --no-build wins over --rebuild because
            // skipping work entirely is the more concrete intent. Warn
            // loudly so the user knows their --rebuild was ignored.
            let rebuild = args.rebuild;
            let no_build = args.no_build;
            if rebuild && no_build {
                eprintln!(
                    "  [install] both --rebuild and --no-build passed — \
                     --no-build wins (skipping build+install)"
                );
            }

            if args.record && args.no_record {
                eprintln!(
                    "  [record] both --record and --no-record passed — \
                     --no-record wins (recording disabled)"
                );
            }

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
                rebuild,
                no_build,
                device_settings: project_config.device_settings,
                record: args.record,
                no_record: args.no_record,
                project_record: project_config.options.record,
                trace: args.trace,
                repeat: args.repeat,
            };

            // Wire shape (always identical, whether we're talking to
            // an in-process orchestrator we just started or to an
            // existing daemon): config_json carries everything the
            // server needs to reconstruct SuiteConfig. ProjectAppConfig
            // isn't Serialize, so the server re-parses golem.toml from
            // `project_root`.
            let config_json = serde_json::json!({
                "platform": args.platform,
                "seed": args.seed,
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
                "coverage": args.coverage,
                "rebuild": config.rebuild,
                "no_build": config.no_build,
                "record": config.record,
                "no_record": config.no_record,
                "trace": config.trace,
                "repeat": config.repeat,
                "include_junit": include_junit,
            });

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
                    let s = orchestrator::try_connect().await
                        .context("failed to connect to in-process orchestrator")?;
                    (s, Some(server))
                }
            };

            let outcome = orchestrator::submit_and_wait(
                stream, &flow_paths, &config_json,
                config.verbose, config.debug, has_human_output,
            ).await?;
            let report = outcome.report;

            // Drain in-process server: wait for any other clients, then
            // shut down sims/emulators we booted (respects --keep-devices).
            if let Some(server) = local_server {
                server.wait_for_clients().await;
                let warnings = server.resource_mgr.shutdown_golem_booted(args.keep_devices).await;
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
            let flake_summary = build_flake_summary(&report.flows);
            if !flake_summary.is_empty() {
                eprint!("{}", render_flake_summary(&flake_summary));
            }

            // Exit with appropriate code. Skipped flows (coverage-group
            // reclassify + install preconditions) don't fail the suite;
            // only genuine failures do. Use `is_failed` (not the
            // simpler `all_passed` from the submit-wire) so install-
            // precondition skips are correctly fatal and coverage-group
            // skips are correctly tolerated.
            let any_failed = report.flows.iter().any(|f| f.is_failed());
            if any_failed {
                std::process::exit(1);
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
    }

    Ok(())
}

/// One flow's pass/fail count across N repeat runs. Sorted by
/// "flakiest first" — most failed runs at the top, then most warn
/// runs, then alphabetical for stable output.
struct FlakeEntry {
    flow: String,
    passed: u32,
    failed: u32,
    skipped: u32,
    total: u32,
}

/// Tally pass/fail per (flow_name, device) across all repeat runs.
/// `flows` is the flat `SuiteReport.flows` list — each entry already
/// carries a `repeat: Option<RepeatContext>` populated by the runner
/// when `--repeat > 1`. Returns empty when no entry has repeat set
/// (single-run suites — no flake summary needed).
fn build_flake_summary(flows: &[golem_report::FlowReport]) -> Vec<FlakeEntry> {
    if !flows.iter().any(|f| f.repeat.is_some()) {
        return Vec::new();
    }
    let mut acc: std::collections::BTreeMap<String, FlakeEntry> = std::collections::BTreeMap::new();
    for f in flows {
        let key = match &f.device_name {
            Some(d) => format!("{} ({})", f.flow_name, d),
            None => f.flow_name.clone(),
        };
        let entry = acc.entry(key.clone()).or_insert_with(|| FlakeEntry {
            flow: key,
            passed: 0,
            failed: 0,
            skipped: 0,
            total: 0,
        });
        entry.total += 1;
        if f.is_skipped() {
            entry.skipped += 1;
        } else if f.success {
            entry.passed += 1;
        } else {
            entry.failed += 1;
        }
    }
    let mut out: Vec<FlakeEntry> = acc.into_values().collect();
    out.sort_by(|a, b| {
        let a_flake = a.passed > 0 && a.failed > 0;
        let b_flake = b.passed > 0 && b.failed > 0;
        b_flake.cmp(&a_flake)
            .then(b.failed.cmp(&a.failed))
            .then(a.flow.cmp(&b.flow))
    });
    out
}

fn render_flake_summary(entries: &[FlakeEntry]) -> String {
    use std::fmt::Write;
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());
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
    let flakes = entries.iter().filter(|e| e.passed > 0 && e.failed > 0).count();
    let stable_fails = entries.iter().filter(|e| e.passed == 0 && e.failed > 0).count();
    let stable_passes = entries.iter()
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
            let body_color = if e.failed == 0 && e.passed > 0 { DIM } else { "" };
            let body_reset = if e.failed == 0 && e.passed > 0 { RESET } else { "" };
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

#[cfg(test)]
mod tests {
    use super::*;
    use golem_events::RepeatContext;
    use golem_report::FlowReport;

    fn flow(name: &str, success: bool, repeat: Option<RepeatContext>, skipped: bool) -> FlowReport {
        // `is_skipped` requires success=true + skipped_reason=Some
        // (a coverage-group skip). Install-precondition skips keep
        // success=false and classify as failures — we use the
        // coverage-skip shape here so `skipped` rows actually count.
        FlowReport {
            flow_name: name.to_string(),
            success: if skipped { true } else { success },
            skipped_reason: if skipped { Some("coverage group satisfied".into()) } else { None },
            device_name: Some("iPhone 17".into()),
            repeat,
            ..FlowReport::default()
        }
    }

    #[test]
    fn flake_summary_empty_when_no_repeat() {
        // Single-run suites — every flow has repeat=None — get no flake summary.
        let flows = vec![flow("a.test", true, None, false), flow("b.test", false, None, false)];
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
            vec!["a_flake.test (iPhone 17)", "m_fail.test (iPhone 17)", "z_pass.test (iPhone 17)"],
            "SHALL sort flakes first, then stable failures, then stable passes",
        );
    }
}

