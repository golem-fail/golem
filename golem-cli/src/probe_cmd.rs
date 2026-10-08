#![allow(clippy::disallowed_macros)] // a command renderer: stdout is its output
//! `golem probe`: what one selector matches on a device, through the
//! daemon.

use anyhow::{bail, Result};
use golem_orchestrator::{daemon, interactive, ipc, project, target::TargetQuery};

use crate::cli::{ProbeArgs, TreeOutput};

/// Run `golem probe`. Always exits 0: a probe reports, it does not judge.
pub async fn run(args: &ProbeArgs) -> Result<i32> {
    golem_parser::inline::parse_selector_inline(&args.selector)?;
    let query = TargetQuery {
        os: platform_os(args.platform.as_deref())?,
        device_type: None,
        device: args.device.clone(),
        bundle: args.bundle.clone(),
        app: args.app.clone(),
    };
    let cwd = std::env::current_dir()?;
    let project_root = project::find_project_root(&cwd).unwrap_or(cwd);
    let msg = interactive::probe_request_json(&args.selector, &query, &project_root, args.timeout);
    let stream = daemon::connect_or_start(
        &ipc::socket_path(),
        crate::daemon_starter().as_ref(),
        &daemon::ClientOptions::current(),
    )
    .await?;
    let reply = ipc::request(stream, &msg).await?;
    match args.output {
        TreeOutput::Toon => print!(
            "{} {}",
            reply["device"].as_str().unwrap_or_default(),
            reply["toon"].as_str().unwrap_or_default()
        ),
        TreeOutput::Json => println!(
            "{}",
            serde_json::to_string_pretty(&reply["report"]).unwrap_or_default()
        ),
    }
    Ok(0)
}

/// A `--platform` flag as the target query's `os`, any version.
pub(crate) fn platform_os(
    platform: Option<&str>,
) -> Result<Option<golem_orchestrator::target::OsQuery>> {
    match platform {
        None => Ok(None),
        Some(p @ ("ios" | "android")) => Ok(Some(golem_orchestrator::target::OsQuery::parse(p)?)),
        Some(p) => bail!("unknown platform: {p}. Use 'ios' or 'android'."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_platform_must_be_ios_or_android() {
        assert!(platform_os(Some("web")).is_err());
        assert!(platform_os(Some("ios:26")).is_err());
        assert_eq!(
            platform_os(Some("ios")).expect("ios").map(|o| o.platform),
            Some(golem_devices::Platform::Ios)
        );
    }
}
