use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "golem", about = "Mobile UI testing framework")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run test flows
    Run(RunArgs),
    /// List available devices
    Devices,
    /// Initialize a new project
    Init,
    /// Create a new flow template
    Create(CreateArgs),
}

#[derive(clap::Args, Debug)]
pub struct RunArgs {
    /// Flow files or directories to run
    pub files: Vec<PathBuf>,

    /// Filter by tag (repeatable; use | within a value for OR logic)
    #[arg(long = "tag")]
    pub tags: Vec<String>,

    /// Set variable (KEY=VALUE, repeatable)
    #[arg(long = "var")]
    pub vars: Vec<String>,

    /// Deterministic seed for fake data
    #[arg(long)]
    pub seed: Option<u64>,

    /// Start at named block
    #[arg(long)]
    pub start: Option<String>,

    /// Output format (repeatable): human, json:<file>, junit:<file>, toon
    #[arg(long = "output", default_value = "human")]
    pub outputs: Vec<String>,

    /// Enable auto-recording
    #[arg(long)]
    pub record: bool,

    /// Skip app data clear between flows
    #[arg(long)]
    pub no_clean: bool,

    /// Skip teardown blocks
    #[arg(long)]
    pub no_teardown: bool,

    /// Keep devices running after flow
    #[arg(long)]
    pub keep_devices: bool,

    /// Max parallel devices
    #[arg(long)]
    pub max_concurrency: Option<usize>,
}

#[derive(clap::Args, Debug)]
pub struct CreateArgs {
    /// Name of the flow to create
    pub name: String,
}

/// Parse CLI `--var` arguments into key-value pairs.
///
/// Each element must be in `KEY=VALUE` format.
pub fn parse_var_args(vars: &[String]) -> Result<Vec<(String, String)>, anyhow::Error> {
    vars.iter()
        .map(|v| {
            let (key, value) = v.split_once('=').ok_or_else(|| {
                anyhow::anyhow!("Invalid --var format: '{}'. Expected KEY=VALUE", v)
            })?;
            Ok((key.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Helper: parse a command line, prepending the binary name.
    fn parse(args: &[&str]) -> Cli {
        let mut full = vec!["golem"];
        full.extend_from_slice(args);
        Cli::parse_from(full)
    }

    // 1. `run` with no extra args
    #[test]
    fn run_no_args() {
        let cli = parse(&["run"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert!(run.files.is_empty());
        assert!(run.tags.is_empty());
        assert!(!run.record);
    }

    // 2. `run file.test.toml`
    #[test]
    fn run_single_file() {
        let cli = parse(&["run", "file.test.toml"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(run.files, vec![PathBuf::from("file.test.toml")]);
    }

    // 3. `run --tag smoke --tag critical`
    #[test]
    fn run_multiple_tags() {
        let cli = parse(&["run", "--tag", "smoke", "--tag", "critical"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(run.tags, vec!["smoke", "critical"]);
    }

    // 4. `run --var email=test@example.com`
    #[test]
    fn run_var() {
        let cli = parse(&["run", "--var", "email=test@example.com"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(run.vars, vec!["email=test@example.com"]);
    }

    // 5. `run --seed 12345`
    #[test]
    fn run_seed() {
        let cli = parse(&["run", "--seed", "12345"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(run.seed, Some(12345));
    }

    // 6. `run --start block_name`
    #[test]
    fn run_start() {
        let cli = parse(&["run", "--start", "block_name"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(run.start.as_deref(), Some("block_name"));
    }

    // 7. `run --output json:report.json`
    #[test]
    fn run_output() {
        let cli = parse(&["run", "--output", "json:report.json"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(run.outputs, vec!["json:report.json"]);
    }

    // 8. `run --no-teardown --keep-devices`
    #[test]
    fn run_bool_flags() {
        let cli = parse(&["run", "--no-teardown", "--keep-devices"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert!(run.no_teardown);
        assert!(run.keep_devices);
        assert!(!run.no_clean);
        assert!(!run.record);
    }

    // 9. `devices` subcommand
    #[test]
    fn devices_subcommand() {
        let cli = parse(&["devices"]);
        assert!(matches!(cli.command, Commands::Devices));
    }

    // 10. `init` subcommand
    #[test]
    fn init_subcommand() {
        let cli = parse(&["init"]);
        assert!(matches!(cli.command, Commands::Init));
    }

    // 11. `create my_flow`
    #[test]
    fn create_subcommand() {
        let cli = parse(&["create", "my_flow"]);
        let Commands::Create(create) = cli.command else {
            panic!("expected Create");
        };
        assert_eq!(create.name, "my_flow");
    }

    // 12. parse_var_args valid input
    #[test]
    fn parse_var_args_valid() {
        let vars = vec![
            "email=test@example.com".to_string(),
            "name=John Doe".to_string(),
        ];
        let result = parse_var_args(&vars).expect("should parse");
        assert_eq!(
            result,
            vec![
                ("email".to_string(), "test@example.com".to_string()),
                ("name".to_string(), "John Doe".to_string()),
            ]
        );
    }

    // 13. parse_var_args invalid input (no =)
    #[test]
    fn parse_var_args_invalid() {
        let vars = vec!["no_equals_sign".to_string()];
        let result = parse_var_args(&vars);
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be an error"));
        assert!(err_msg.contains("no_equals_sign"));
    }

    // 14. Multiple --output flags
    #[test]
    fn run_multiple_outputs() {
        let cli = parse(&[
            "run",
            "--output",
            "human",
            "--output",
            "json:report.json",
            "--output",
            "junit:results.xml",
        ]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(
            run.outputs,
            vec!["human", "json:report.json", "junit:results.xml"]
        );
    }

    // 15. Multiple files
    #[test]
    fn run_multiple_files() {
        let cli = parse(&["run", "login.test.toml", "signup.test.toml", "flows/"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(
            run.files,
            vec![
                PathBuf::from("login.test.toml"),
                PathBuf::from("signup.test.toml"),
                PathBuf::from("flows/"),
            ]
        );
    }
}
