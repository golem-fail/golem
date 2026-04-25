pub mod cli;
pub mod companions;
pub mod devices;
pub mod discovery;
pub mod install_script_cmd;
pub mod orchestrator;
pub mod project;
pub mod registration;
pub mod scaffold;
pub mod suite;
pub mod tree;

use std::path::{Path, PathBuf};

use clap::Parser;

use cli::{Cli, Commands};
use discovery::TagFilter;
use suite::{SuiteConfig, SuiteRunner};

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
            };

            // Check if an orchestrator is already running
            if let Ok(stream) = orchestrator::try_connect().await {
                // Client mode: submit to existing orchestrator. The
                // server reloads golem.toml from `project_root` so app
                // bundle IDs and device defaults match what the CLI saw
                // locally (ProjectAppConfig isn't Serialize, so we pass
                // the path and let the server re-parse).
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
                });
                let all_passed = orchestrator::submit_and_wait(stream, &flow_paths, &config_json, config.verbose, config.debug).await?;
                if !all_passed {
                    std::process::exit(1);
                }
                return Ok(());
            }

            // Server mode: start orchestrator + run suite with shared ResourceManager
            let server = orchestrator::start_server().await?;

            let mut runner = SuiteRunner::with_resource_manager(
                config,
                server.resource_mgr.clone(),
                server.install_cache.clone(),
            );
            let report = runner.run_suite(&flow_paths).await?;

            // Wait for any active client connections to finish before exiting
            server.wait_for_clients().await;

            // Shut down any sims/emulators golem booted this run (unless
            // --keep-devices). User-booted devices are not tracked, so never
            // shut down.
            let shutdown_warnings = server
                .resource_mgr
                .shutdown_golem_booted(args.keep_devices)
                .await;
            for w in &shutdown_warnings {
                eprintln!("  [devices] {w}");
            }

            // Write results to output dir (json + toon always, junit if requested).
            if !args.no_results {
                golem_report::output::write_results_to_dir(&report, &output_dir, include_junit)?;
            }

            // Write to stdout (non-human formats only — human streams live to stderr).
            for fmt in &stdout_formats {
                if !matches!(fmt, golem_report::output::OutputFormat::Human) {
                    let content = golem_report::output::render(&report, fmt)?;
                    println!("{content}");
                }
            }

            // Exit with appropriate code. Skipped flows (coverage-group
            // reclassify + install preconditions) don't fail the suite;
            // only genuine failures do.
            let any_failed = report.flows.iter().any(|f| f.is_failed());
            if any_failed {
                std::process::exit(1);
            }
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
