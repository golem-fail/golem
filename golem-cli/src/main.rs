pub mod cli;
pub mod devices;
pub mod discovery;
pub mod orchestrator;
pub mod scaffold;
pub mod suite;

use std::path::{Path, PathBuf};

use clap::Parser;

use cli::{Cli, Commands};
use discovery::TagFilter;
use golem_report::output::OutputTarget;
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

            let config = SuiteConfig {
                no_clean: args.no_clean,
                no_teardown: args.no_teardown,
                keep_devices: args.keep_devices,
                seed: args.seed,
                platform: platform_override,
            };

            // Check if an orchestrator is already running
            if let Ok(stream) = orchestrator::try_connect().await {
                // Client mode: submit to existing orchestrator
                let config_json = serde_json::json!({
                    "platform": args.platform,
                    "seed": args.seed,
                });
                let all_passed = orchestrator::submit_and_wait(stream, &flow_paths, &config_json).await?;
                if !all_passed {
                    std::process::exit(1);
                }
                return Ok(());
            }

            // Server mode: start orchestrator + run suite with shared ResourceManager
            let server = orchestrator::start_server().await?;

            let runner = SuiteRunner::with_resource_manager(config, server.resource_mgr.clone());
            let report = runner.run_suite(&flow_paths).await?;

            // Wait for any active client connections to finish before exiting
            server.wait_for_clients().await;

            // Parse output targets
            let targets: Vec<OutputTarget> = args
                .outputs
                .iter()
                .map(|s| OutputTarget::parse(s))
                .collect::<Result<Vec<_>, _>>()?;

            // Write outputs
            let (_written_files, stdout_contents) =
                golem_report::output::write_outputs(&report, &targets)?;

            for content in &stdout_contents {
                println!("{content}");
            }

            // Exit with appropriate code
            let all_passed = report.flows.iter().all(|f| f.success);
            if !all_passed {
                std::process::exit(1);
            }
        }

        Commands::Devices => {
            let ios_devices = golem_devices::ios::discover_ios_devices().await?;
            let output = devices::format_device_list(&ios_devices);
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
    }

    Ok(())
}
