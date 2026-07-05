//! Thin binary wrapper. All logic lives in the `golem_cli` library crate
//! (`lib.rs`) so integration tests in `tests/` can drive the whole pipeline
//! in-process. `main` parses argv, runs the CLI, and exits with its code.

use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = golem_cli::cli::Cli::parse();
    match golem_cli::run_cli(cli).await {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("Error: {e:#}");
            std::process::exit(1);
        }
    }
}
