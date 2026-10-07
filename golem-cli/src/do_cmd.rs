#![allow(clippy::disallowed_macros)] // a command renderer: stdout is its output
//! `golem do` and `golem probe`: one step, or one selector, on a device,
//! through the daemon.

use anyhow::{bail, Result};
use golem_orchestrator::{daemon, interactive, ipc, project, target::TargetQuery};

use crate::cli::{DoArgs, ProbeArgs, TreeOutput};

/// Run `golem do`. Returns the exit code: 0 when the step passes, 1 when it
/// fails.
pub async fn run(args: &DoArgs) -> Result<i32> {
    // Parse here too, so a malformed step fails before a daemon starts.
    golem_parser::inline::parse_step_inline(&args.step)?;
    let query = TargetQuery {
        platform: parse_platform(args.platform.as_deref())?,
        device: args.device.clone(),
        bundle: args.bundle.clone(),
        app: args.app.clone(),
    };
    let cwd = std::env::current_dir()?;
    let project_root = project::find_project_root(&cwd).unwrap_or(cwd);
    let mut msg = interactive::do_request_json(&args.step, &query, &project_root, args.tree);
    ipc::add_client_context(&mut msg);

    let stream = daemon::connect_or_start(
        &ipc::socket_path(),
        crate::daemon_starter().as_ref(),
        &daemon::ClientOptions::current(),
    )
    .await?;
    let reply = ipc::request(stream, &msg).await?;
    print!("{}", render(&reply, args.output));
    Ok(if reply["passed"].as_bool() == Some(true) {
        0
    } else {
        1
    })
}

/// Run `golem probe`. Always exits 0: a probe reports, it does not judge.
pub async fn probe(args: &ProbeArgs) -> Result<i32> {
    golem_parser::inline::parse_selector_inline(&args.selector)?;
    let query = TargetQuery {
        platform: parse_platform(args.platform.as_deref())?,
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

fn parse_platform(platform: Option<&str>) -> Result<Option<golem_devices::Platform>> {
    match platform {
        None => Ok(None),
        Some("ios") => Ok(Some(golem_devices::Platform::Ios)),
        Some("android") => Ok(Some(golem_devices::Platform::Android)),
        Some(p) => bail!("unknown platform: {p}. Use 'ios' or 'android'."),
    }
}

/// The reply to a `do`, as the user asked to see it.
fn render(reply: &serde_json::Value, output: TreeOutput) -> String {
    match output {
        TreeOutput::Json => {
            let out = serde_json::json!({
                "device": reply["device"],
                "passed": reply["passed"],
                "step": reply["step"],
                "tree": reply["tree"],
            });
            format!(
                "{}\n",
                serde_json::to_string_pretty(&out).unwrap_or_default()
            )
        }
        TreeOutput::Toon => {
            let mut out = format!(
                "{} {}\n",
                reply["device"].as_str().unwrap_or_default(),
                reply["toon"].as_str().unwrap_or_default()
            );
            if let Some(tree) = reply["tree"].as_str() {
                out.push_str(tree);
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(passed: bool, tree: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "type": "do_result",
            "passed": passed,
            "device": "android/Pixel 8",
            "toon": "+tap:on_text=\"+\" d:812 @243,732 b180,666,126,132",
            "step": { "action": "tap", "outcome": "success" },
            "tree": tree,
        })
    }

    #[test]
    fn toon_prints_the_device_and_the_step_line() {
        assert_eq!(
            render(&reply(true, None), TreeOutput::Toon),
            "android/Pixel 8 +tap:on_text=\"+\" d:812 @243,732 b180,666,126,132\n"
        );
    }

    #[test]
    fn toon_appends_the_tree_when_asked() {
        let out = render(
            &reply(true, Some("tree visible 1080x2400 n:1\n[1] button \"+\"\n")),
            TreeOutput::Toon,
        );
        assert!(
            out.ends_with("tree visible 1080x2400 n:1\n[1] button \"+\"\n"),
            "{out}"
        );
    }

    #[test]
    fn json_keeps_the_step_object() {
        let out = render(&reply(false, None), TreeOutput::Json);
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["passed"], false);
        assert_eq!(v["step"]["action"], "tap");
        assert!(v["tree"].is_null());
    }

    #[test]
    fn a_platform_must_be_ios_or_android() {
        assert!(parse_platform(Some("web")).is_err());
        assert_eq!(
            parse_platform(Some("ios")).expect("ios"),
            Some(golem_devices::Platform::Ios)
        );
    }
}
