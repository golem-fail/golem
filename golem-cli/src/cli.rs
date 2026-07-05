use clap::{Parser, Subcommand};
use std::path::PathBuf;

// These clap structs are the source of truth for docs/cli-reference.md. When you add,
// remove, or rename a command or flag, update that doc to match.

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
    /// Show the UI element tree from a running companion
    Tree(TreeArgs),
    /// List available devices
    Devices,
    /// Initialize a new project
    Init,
    /// Create a new flow template
    Create(CreateArgs),
    /// Interactively scaffold an app install script
    InstallScript,
    /// Inspect the persistent install cache
    Cache(CacheArgs),
    /// Read the embedded audit out of an annotated a11y screenshot: list the
    /// findings and print the command to replay that run.
    A11yExtract(A11yExtractArgs),
}

#[derive(clap::Args, Debug)]
pub struct A11yExtractArgs {
    /// Path to an annotated a11y screenshot PNG (`*_a11y.png`).
    pub png: PathBuf,

    /// Print the raw embedded `Golem-Audit` JSON instead of the human report.
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct CacheArgs {
    #[command(subcommand)]
    pub command: CacheCommands,
}

#[derive(Subcommand, Debug)]
pub enum CacheCommands {
    /// Show entry count, status breakdown, and recency stats
    Info,
}

#[derive(clap::Args, Debug)]
pub struct TreeArgs {
    /// Filter by platform (ios or android)
    #[arg(long)]
    pub platform: Option<String>,

    /// Filter by device name or UDID
    #[arg(long)]
    pub device: Option<String>,

    /// Show full tree (no viewport filtering)
    #[arg(long)]
    pub full: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// App bundle ID for iOS (needed to target the right app)
    #[arg(long)]
    pub bundle: Option<String>,

    /// Show verbose metadata (CDP status, enrichment source, etc.)
    #[arg(long)]
    pub verbose: bool,
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

    /// Output format for stdout (repeatable): human, json, junit, toon
    #[arg(long = "output", default_value = "human")]
    pub outputs: Vec<String>,

    /// Results directory (default: .golem/results)
    #[arg(long = "output-dir")]
    pub output_dir: Option<String>,

    /// Disable writing results to disk (no json/toon/screenshots/recordings)
    #[arg(long)]
    pub no_results: bool,

    /// Repeat the entire suite N times. Each run writes to
    /// `{output-dir}/run_{i}/`. A flake summary is printed at the end
    /// listing tests that didn't pass every run. Useful for chasing
    /// intermittents — `--repeat 5 --trace` gives 5 fully-traced
    /// passes. Capped at 100; the daemon schedules all N runs as
    /// independent FlowRuns, so they parallelise across available
    /// devices for free.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=100))]
    pub repeat: u32,

    /// Enable auto-recording for every block. Beats per-block opt-out
    /// only when paired without `--no-record` (which always wins).
    #[arg(long)]
    pub record: bool,

    /// Force-disable auto-recording everywhere. Beats `--record`,
    /// `[flow.options].record`, `[options].record`, and per-block opts.
    #[arg(long)]
    pub no_record: bool,

    /// Forensic capture mode for intermittent investigation. Forces
    /// recording on (beats `--no-record`) and captures a screenshot +
    /// accessibility tree at every step boundary. ~200ms/step
    /// overhead — not for regular CI runs.
    #[arg(long)]
    pub trace: bool,

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

    /// Force target platform (ios or android). Overrides flow device constraints.
    #[arg(long)]
    pub platform: Option<String>,

    /// Override every flow's coverage strategy (one | min | smart | full).
    /// Handy for `--coverage one` to run a suite as a quick smoke with a
    /// single FlowRun per flow.
    #[arg(long)]
    pub coverage: Option<String>,

    /// Disable automatic performance capture
    #[arg(long)]
    pub no_perf: bool,

    /// Override every flow's accessibility audit level
    /// (off | critical | relaxed | strict). Default: relaxed.
    #[arg(long)]
    pub a11y: Option<String>,

    /// Override every flow's `a11y_min_confidence` (0.0–1.0): drop a11y
    /// findings below this confidence. 0.0 surfaces every heuristic finding;
    /// higher keeps only confident ones. Wins over `[flow.options]`.
    #[arg(long = "a11y-min-confidence")]
    pub a11y_min_confidence: Option<f32>,

    /// Verbose output: show swipe coordinates, scroll strategy, fingerprints
    #[arg(long)]
    pub verbose: bool,

    /// Debug output: show driver-level diagnostics (WebKit/CDP connection, errors)
    #[arg(long)]
    pub debug: bool,

    /// Bypass the persistent install cache for this run — every (device,
    /// bundle) pair is rebuilt + reinstalled. The cache is still updated
    /// after, so the next run benefits.
    #[arg(long)]
    pub rebuild: bool,

    /// Skip build+install entirely. If a device already has the bundle
    /// installed, golem trusts it and runs flows; if not, the flow fails
    /// loudly with an actionable message. The cache is left untouched.
    #[arg(long = "no-build")]
    pub no_build: bool,

    /// Hard cap on how long a FlowRun blocks in the device queue before
    /// failing with "no device available". Format: `30m`, `1h`, `90s`,
    /// `1h30m`. Default: unbounded — the per-flow `max_runtime` breaker
    /// guarantees forward progress by freeing wedged devices. Set for
    /// CI usage with a wall-clock budget.
    #[arg(long = "max-wait")]
    pub max_wait: Option<String>,

    /// Hidden: drive the suite against the device-free StubDriver using the
    /// TOML stub script at this path (in-process integration tests only).
    /// The stub driver is compiled out of release builds, so this flag is a
    /// no-op there. See `golem-cli/tests/`.
    #[arg(long, hide = true)]
    pub stub: Option<PathBuf>,
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

    // 7. `run --output json`
    #[test]
    fn run_output() {
        let cli = parse(&["run", "--output", "json"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(run.outputs, vec!["json"]);
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
            "run", "--output", "human", "--output", "json", "--output", "junit",
        ]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(run.outputs, vec!["human", "json", "junit"]);
    }

    // 15. `run --no-perf`
    #[test]
    fn run_no_perf() {
        let cli = parse(&["run", "--no-perf"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert!(run.no_perf);
    }

    // 15b. no-perf defaults to false
    #[test]
    fn run_no_perf_default() {
        let cli = parse(&["run"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert!(!run.no_perf);
    }

    // 16. Multiple files
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

    // 17. `--output` defaults to ["human"] when omitted
    #[test]
    fn run_output_default_is_human() {
        let cli = parse(&["run"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(
            run.outputs,
            vec!["human"],
            "outputs SHALL default to [human] when --output omitted"
        );
    }

    // 18. `--repeat` defaults to 1
    #[test]
    fn run_repeat_default_is_one() {
        let cli = parse(&["run"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(run.repeat, 1, "repeat SHALL default to 1");
    }

    // 19. `--repeat` accepts the documented boundaries 1 and 100
    #[test]
    fn run_repeat_boundaries_accepted() {
        let lo = parse(&["run", "--repeat", "1"]);
        let Commands::Run(run_lo) = lo.command else {
            panic!("expected Run");
        };
        assert_eq!(run_lo.repeat, 1, "repeat=1 SHALL be accepted");

        let hi = parse(&["run", "--repeat", "100"]);
        let Commands::Run(run_hi) = hi.command else {
            panic!("expected Run");
        };
        assert_eq!(run_hi.repeat, 100, "repeat=100 SHALL be accepted");
    }

    // 20. `--repeat 0` is below the range and SHALL be rejected
    #[test]
    fn run_repeat_zero_rejected() {
        let res = Cli::try_parse_from(["golem", "run", "--repeat", "0"]);
        assert!(
            res.is_err(),
            "repeat=0 SHALL be rejected (range starts at 1)"
        );
    }

    // 21. `--repeat 101` is above the range and SHALL be rejected
    #[test]
    fn run_repeat_above_cap_rejected() {
        let res = Cli::try_parse_from(["golem", "run", "--repeat", "101"]);
        assert!(
            res.is_err(),
            "repeat=101 SHALL be rejected (range capped at 100)"
        );
    }

    // 22. recording flags all default to false
    #[test]
    fn run_recording_flags_default_false() {
        let cli = parse(&["run"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert!(!run.record, "record SHALL default false");
        assert!(!run.no_record, "no_record SHALL default false");
        assert!(!run.trace, "trace SHALL default false");
    }

    // 23. `--record --no-record --trace` all set independently (no parse-time conflict)
    #[test]
    fn run_recording_flags_coexist() {
        let cli = parse(&["run", "--record", "--no-record", "--trace"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert!(run.record, "record SHALL be set");
        assert!(run.no_record, "no_record SHALL be set");
        assert!(run.trace, "trace SHALL be set");
    }

    // 24. `--output-dir` and `--no-results`
    #[test]
    fn run_output_dir_and_no_results() {
        let cli = parse(&["run", "--output-dir", "out/dir", "--no-results"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(run.output_dir.as_deref(), Some("out/dir"));
        assert!(run.no_results, "no_results SHALL be set");
    }

    // 25. cache/build flags: --rebuild, --no-build, --max-concurrency, --max-wait
    #[test]
    fn run_build_and_queue_flags() {
        let cli = parse(&[
            "run",
            "--rebuild",
            "--no-build",
            "--max-concurrency",
            "4",
            "--max-wait",
            "1h30m",
        ]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert!(run.rebuild, "rebuild SHALL be set");
        assert!(run.no_build, "no_build SHALL be set");
        assert_eq!(run.max_concurrency, Some(4));
        assert_eq!(run.max_wait.as_deref(), Some("1h30m"));
    }

    // 26. --platform and --coverage overrides
    #[test]
    fn run_platform_and_coverage() {
        let cli = parse(&["run", "--platform", "ios", "--coverage", "one"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(run.platform.as_deref(), Some("ios"));
        assert_eq!(run.coverage.as_deref(), Some("one"));
    }

    // 27. seed/start/output-dir/platform default to None
    #[test]
    fn run_optionals_default_none() {
        let cli = parse(&["run"]);
        let Commands::Run(run) = cli.command else {
            panic!("expected Run");
        };
        assert!(run.seed.is_none(), "seed SHALL default None");
        assert!(run.start.is_none(), "start SHALL default None");
        assert!(run.output_dir.is_none(), "output_dir SHALL default None");
        assert!(run.platform.is_none(), "platform SHALL default None");
        assert!(run.coverage.is_none(), "coverage SHALL default None");
        assert!(
            run.max_concurrency.is_none(),
            "max_concurrency SHALL default None"
        );
        assert!(run.max_wait.is_none(), "max_wait SHALL default None");
    }

    // 28. `tree` with no args: all options None/false
    #[test]
    fn tree_defaults() {
        let cli = parse(&["tree"]);
        let Commands::Tree(tree) = cli.command else {
            panic!("expected Tree");
        };
        assert!(tree.platform.is_none());
        assert!(tree.device.is_none());
        assert!(!tree.full);
        assert!(!tree.json);
        assert!(tree.bundle.is_none());
        assert!(!tree.verbose);
    }

    // 29. `tree` with all options populated
    #[test]
    fn tree_all_options() {
        let cli = parse(&[
            "tree",
            "--platform",
            "android",
            "--device",
            "emulator-5554",
            "--full",
            "--json",
            "--bundle",
            "com.example.app",
            "--verbose",
        ]);
        let Commands::Tree(tree) = cli.command else {
            panic!("expected Tree");
        };
        assert_eq!(tree.platform.as_deref(), Some("android"));
        assert_eq!(tree.device.as_deref(), Some("emulator-5554"));
        assert!(tree.full, "full SHALL be set");
        assert!(tree.json, "json SHALL be set");
        assert_eq!(tree.bundle.as_deref(), Some("com.example.app"));
        assert!(tree.verbose, "verbose SHALL be set");
    }

    // 30. `installscript` subcommand parses
    #[test]
    fn install_script_subcommand() {
        let cli = parse(&["install-script"]);
        assert!(matches!(cli.command, Commands::InstallScript));
    }

    // 31. `cache info` nested subcommand
    #[test]
    fn cache_info_subcommand() {
        let cli = parse(&["cache", "info"]);
        let Commands::Cache(cache) = cli.command else {
            panic!("expected Cache");
        };
        assert!(matches!(cache.command, CacheCommands::Info));
    }

    // 32. `cache` without a subcommand SHALL fail (subcommand is required)
    #[test]
    fn cache_requires_subcommand() {
        let res = Cli::try_parse_from(["golem", "cache"]);
        assert!(res.is_err(), "cache without subcommand SHALL be rejected");
    }

    // 33. `create` without a name SHALL fail (positional is required)
    #[test]
    fn create_requires_name() {
        let res = Cli::try_parse_from(["golem", "create"]);
        assert!(res.is_err(), "create without name SHALL be rejected");
    }

    // 34. an unknown subcommand SHALL fail
    #[test]
    fn unknown_subcommand_rejected() {
        let res = Cli::try_parse_from(["golem", "frobnicate"]);
        assert!(res.is_err(), "unknown subcommand SHALL be rejected");
    }

    // 35. parse_var_args: empty input yields empty vec
    #[test]
    fn parse_var_args_empty() {
        let result = parse_var_args(&[]).expect("empty input should parse");
        assert!(result.is_empty(), "empty input SHALL yield empty vec");
    }

    // 36. parse_var_args: value containing '=' splits only on the first '='
    #[test]
    fn parse_var_args_value_with_equals() {
        let vars = vec!["token=a=b=c".to_string()];
        let result = parse_var_args(&vars).expect("should parse");
        assert_eq!(
            result,
            vec![("token".to_string(), "a=b=c".to_string())],
            "split SHALL occur only on the first '='"
        );
    }

    // 37. parse_var_args: empty value (KEY=) is allowed and yields empty string
    #[test]
    fn parse_var_args_empty_value() {
        let vars = vec!["key=".to_string()];
        let result = parse_var_args(&vars).expect("should parse");
        assert_eq!(result, vec![("key".to_string(), String::new())]);
    }

    // 38. parse_var_args: empty key (=value) is allowed and yields empty key
    #[test]
    fn parse_var_args_empty_key() {
        let vars = vec!["=value".to_string()];
        let result = parse_var_args(&vars).expect("should parse");
        assert_eq!(result, vec![(String::new(), "value".to_string())]);
    }

    // 39. parse_var_args: one invalid among valid entries fails the whole parse
    #[test]
    fn parse_var_args_fails_fast_on_any_invalid() {
        let vars = vec![
            "good=1".to_string(),
            "bad".to_string(),
            "alsogood=2".to_string(),
        ];
        let result = parse_var_args(&vars);
        assert!(result.is_err(), "any invalid entry SHALL fail the parse");
        let msg = format!("{}", result.expect_err("should be error"));
        assert!(msg.contains("bad"), "error SHALL name the offending value");
        assert!(
            msg.contains("KEY=VALUE"),
            "error SHALL mention the expected KEY=VALUE format"
        );
    }
}
