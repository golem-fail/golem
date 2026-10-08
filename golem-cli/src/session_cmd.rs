#![allow(clippy::disallowed_macros)] // a command renderer: stdout is its output
//! `golem session`: a named session that the daemon holds between commands.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use golem_orchestrator::{daemon, ipc, project};

use crate::cli::{SessionArgs, SessionCommands, TreeOutput};

/// How long one daemon call waits before it answers `pending`; the command
/// then asks again, so a long open can print what it is doing.
const POLL: Duration = Duration::from_secs(10);

/// Run a `golem session` command. Returns the exit code: `do` exits 1 when
/// its step fails.
pub async fn run(args: &SessionArgs) -> Result<i32> {
    match &args.command {
        SessionCommands::Start(a) => {
            let cwd = std::env::current_dir()?;
            let project_root = project::find_project_root(&cwd).unwrap_or(cwd);
            let mut msg = serde_json::json!({
                "query": {
                    "os": a.os,
                    "type": a.device_type,
                    "device": a.device,
                    "bundle": a.bundle,
                    "app": a.app,
                },
                "boot": !a.no_boot,
                "project_root": project_root.display().to_string(),
                "idle_timeout_s": a.idle_timeout,
                "flow": a.flow.as_ref().map(|f| absolute(f).display().to_string()),
                "stop_at": a.stop_at,
                "break_on_failure": a.break_on_failure,
                "teardown": !a.no_teardown,
                "vars": parse_vars(&a.vars)?,
            });
            if cfg!(debug_assertions) && a.stub {
                msg["stub"] = serde_json::json!(true);
            }
            ipc::add_client_context(&mut msg);
            let reply = call(&a.name.name, "session_open", msg, Connect::Start).await?;
            print!("{}", done_text(&reply)?);
            Ok(0)
        }
        SessionCommands::Do(a) => {
            golem_parser::inline::parse_step_inline(&a.step)?;
            let msg = serde_json::json!({ "step": a.step, "comment": a.comment, "tree": a.tree });
            let reply = call(&a.name.name, "session_act", msg, Connect::Attach).await?;
            let passed = reply["result"]["passed"].as_bool() == Some(true);
            match a.output {
                TreeOutput::Toon => print!("{}", done_text(&reply)?),
                TreeOutput::Json => println!("{}", json_text(&reply)?),
            }
            Ok(if passed { 0 } else { 1 })
        }
        SessionCommands::Probe(a) => {
            golem_parser::inline::parse_selector_inline(&a.selector)?;
            let msg = serde_json::json!({ "selector": a.selector, "timeout_ms": a.timeout });
            let reply = call(&a.name.name, "session_probe", msg, Connect::Attach).await?;
            match a.output {
                TreeOutput::Toon => print!("{}", done_text(&reply)?),
                TreeOutput::Json => println!("{}", json_text(&reply)?),
            }
            Ok(0)
        }
        SessionCommands::Tree(a) => {
            let json = a.output == TreeOutput::Json;
            let msg = serde_json::json!({ "full": a.full, "json": json });
            let reply = call(&a.name.name, "session_tree", msg, Connect::Attach).await?;
            print!("{}", done_text(&reply)?);
            Ok(0)
        }
        SessionCommands::Screenshot(a) => {
            let path = absolute(&a.path);
            let msg = serde_json::json!({ "path": path.display().to_string() });
            let reply = call(&a.name.name, "session_screenshot", msg, Connect::Attach).await?;
            done_text(&reply)?;
            // Only the first reply writes the file; a later `session_wait`
            // carries the PNG instead.
            if reply["result"]["saved"].is_null() {
                bail!("the screenshot took too long; run the command again");
            }
            println!(
                "{} · {} bytes",
                a.path.display(),
                reply["result"]["bytes"].as_u64().unwrap_or(0)
            );
            Ok(0)
        }
        SessionCommands::Logs(a) => {
            let msg = serde_json::json!({
                "since_secs": a.since,
                "filter": a.filter,
                "limit": a.limit,
                "app": a.app,
            });
            let reply = call(&a.name.name, "session_logs", msg, Connect::Attach).await?;
            println!("{}", reply["logs"].as_str().unwrap_or_default());
            Ok(0)
        }
        SessionCommands::Export(a) => {
            let msg = serde_json::json!({
                "path": absolute(&a.path).display().to_string(),
                "overwrite": a.overwrite,
            });
            let reply = call(&a.name.name, "session_export", msg, Connect::Attach).await?;
            print!("{}", done_text(&reply)?);
            Ok(0)
        }
        SessionCommands::Stop(a) => {
            let msg = serde_json::json!({ "teardown": !a.no_teardown });
            let reply = call(&a.name.name, "session_close", msg, Connect::Attach).await?;
            match reply["teardown"].as_str() {
                Some(notes) => println!("session {} stopped · {notes}", a.name.name),
                None => println!("session {} stopped", a.name.name),
            }
            Ok(0)
        }
        SessionCommands::List => {
            let reply = match attach().await {
                Ok(stream) => {
                    let msg = serde_json::json!({ "type": "session_list", "id": 1 });
                    ipc::request(stream, &msg).await?
                }
                // No daemon, so no session.
                Err(_) => serde_json::json!({ "sessions": [] }),
            };
            print!("{}", list_text(&reply));
            Ok(0)
        }
    }
}

#[derive(Clone, Copy)]
enum Connect {
    /// Start a daemon, or replace one from another build.
    Start,
    /// Use the daemon that is running: the session lives in it, and
    /// replacing it would end the session.
    Attach,
}

async fn attach() -> Result<tokio::net::UnixStream> {
    ipc::attach(
        &ipc::socket_path(),
        &ipc::Identity::current(),
        ipc::HELLO_TIMEOUT,
    )
    .await
}

/// Send one `session_*` message for the session `name`, then ask again
/// while it is pending.
async fn call(
    name: &str,
    kind: &str,
    mut msg: serde_json::Value,
    connect: Connect,
) -> Result<serde_json::Value> {
    msg["type"] = serde_json::json!(kind);
    msg["session"] = serde_json::json!(name);
    msg["id"] = serde_json::json!(1);
    msg["wait_ms"] = serde_json::json!(POLL.as_millis() as u64);
    let stream = match connect {
        Connect::Start => {
            daemon::connect_or_start(
                &ipc::socket_path(),
                crate::daemon_starter().as_ref(),
                &daemon::ClientOptions::current(),
            )
            .await?
        }
        Connect::Attach => attach().await?,
    };
    let mut reply = ipc::request(stream, &msg).await?;
    let mut phase = String::new();
    while reply["status"] == "pending" {
        let now = reply["phase"].as_str().unwrap_or_default();
        if now != phase {
            eprintln!("  {now}");
            phase = now.to_string();
        }
        let wait = serde_json::json!({
            "type": "session_wait",
            "session": name,
            "id": 1,
            "wait_ms": POLL.as_millis() as u64,
        });
        reply = ipc::request(attach().await?, &wait).await?;
    }
    match reply["status"].as_str() {
        Some("error") => bail!("{}", reply["message"].as_str().unwrap_or("unknown error")),
        Some("ended") => bail!(
            "session {name} ended ({})",
            reply["reason"].as_str().unwrap_or("closed")
        ),
        Some("busy") => bail!(
            "session {name} is busy: {} is still running · {}",
            reply["op"].as_str().unwrap_or_default(),
            reply["phase"].as_str().unwrap_or_default()
        ),
        _ => Ok(reply),
    }
}

/// A finished operation as text; an operation that failed to run is an
/// error.
fn done_text(reply: &serde_json::Value) -> Result<String> {
    let r = &reply["result"];
    if let Some(err) = r["error"].as_str() {
        bail!("{err}");
    }
    Ok(note_text(reply).unwrap_or_else(|| toon_text(r)))
}

fn json_text(reply: &serde_json::Value) -> Result<String> {
    let r = &reply["result"];
    if let Some(err) = r["error"].as_str() {
        bail!("{err}");
    }
    let body = serde_json::json!({
        "passed": r["passed"],
        "result": r.get("step").or_else(|| r.get("report")),
        "tree": r["tree"],
    });
    serde_json::to_string_pretty(&body).context("json")
}

/// The text of a result that is not a step, probe or tree: an open, a
/// cancel, a draft, an edit or an export. Shared with `golem mcp`.
pub(crate) fn note_text(reply: &serde_json::Value) -> Option<String> {
    let r = &reply["result"];
    if r["cancelled"] == true {
        return Some(format!(
            "op {} {} was cancelled",
            reply["op_id"],
            reply["op"].as_str().unwrap_or_default()
        ));
    }
    if let Some(edited) = r["edited"].as_str() {
        return Some(edited.to_string());
    }
    if let Some(text) = r["draft"].as_str() {
        return Some(text.to_string());
    }
    if let Some(path) = r["exported"].as_str() {
        let mut out = format!("exported {path} · {} steps · valid\n", r["steps"]);
        if let Some(counts) = r["counts"].as_str().filter(|c| !c.is_empty()) {
            out.push_str(&format!("{counts}\n"));
        }
        if let Some(list) = r["unverified"].as_array().filter(|l| !l.is_empty()) {
            out.push_str("unverified (never run in this form):\n");
            for step in list {
                out.push_str(&format!("  {}\n", step.as_str().unwrap_or_default()));
            }
        }
        return Some(out);
    }
    if r["opened"] == true {
        let mut out = format!(
            "session open · {} ({}) · app {}\n",
            r["device"].as_str().unwrap_or_default(),
            r["udid"].as_str().unwrap_or_default(),
            r["bundle"]
                .as_str()
                .filter(|b| !b.is_empty())
                .unwrap_or("(last launched)"),
        );
        if let Some(flow) = r["flow"].as_str() {
            out.push_str(flow);
        }
        return Some(out);
    }
    None
}

/// A step, probe or tree result: its TOON line, then the tree. Shared with
/// `golem mcp`.
pub(crate) fn toon_text(r: &serde_json::Value) -> String {
    let mut out = String::new();
    for key in ["toon", "tree"] {
        if let Some(s) = r[key].as_str() {
            out.push_str(s);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

fn list_text(reply: &serde_json::Value) -> String {
    let sessions = reply["sessions"].as_array().cloned().unwrap_or_default();
    if sessions.is_empty() {
        return "no sessions\n".into();
    }
    let mut out = String::new();
    for s in sessions {
        let state = match s["status"].as_str() {
            Some("idle") if s["device"].is_null() => "opening".to_string(),
            Some("idle") => format!("idle {}s", s["idle_ms"].as_u64().unwrap_or(0) / 1000),
            Some("busy") => format!(
                "busy · {} · {}",
                s["op"].as_str().unwrap_or_default(),
                s["phase"].as_str().unwrap_or_default()
            ),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        out.push_str(&format!(
            "{} · {} · {state}\n",
            s["name"].as_str().unwrap_or_default(),
            s["device"].as_str().unwrap_or("(opening)"),
        ));
    }
    out
}

/// `KEY=VALUE` pairs as a JSON object of strings.
fn parse_vars(vars: &[String]) -> Result<serde_json::Map<String, serde_json::Value>> {
    vars.iter()
        .map(|kv| {
            let (k, v) = kv
                .split_once('=')
                .with_context(|| format!("--var {kv:?} needs KEY=VALUE"))?;
            Ok((k.to_string(), serde_json::json!(v)))
        })
        .collect()
}

/// The daemon resolves paths against its own directory, not the shell's.
fn absolute(path: &std::path::Path) -> std::path::PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_text_shows_each_session() {
        let reply = serde_json::json!({ "sessions": [
            { "name": "default", "device": "android/Pixel 8", "status": "idle", "idle_ms": 12_000 },
            { "name": "b", "device": "ios/iPhone 17", "status": "busy", "op": "act", "phase": "act tap" },
        ]});
        assert_eq!(
            list_text(&reply),
            "default · android/Pixel 8 · idle 12s\nb · ios/iPhone 17 · busy · act · act tap\n"
        );
        assert_eq!(
            list_text(&serde_json::json!({ "sessions": [] })),
            "no sessions\n"
        );
    }

    #[test]
    fn vars_need_key_and_value() {
        let vars = parse_vars(&["user=ada".into(), "q=a=b".into()]).expect("vars");
        assert_eq!(vars["user"], "ada");
        assert_eq!(vars["q"], "a=b");
        assert!(parse_vars(&["nokey".into()]).is_err());
    }

    #[test]
    fn a_failed_operation_is_an_error() {
        let reply = serde_json::json!({ "status": "done", "result": { "error": "no device" } });
        assert_eq!(
            done_text(&reply).expect_err("error").to_string(),
            "no device"
        );
    }

    #[test]
    fn a_step_prints_its_line_then_the_tree() {
        let reply = serde_json::json!({ "status": "done", "result": {
            "passed": true, "toon": "+tap:on_text=\"OK\" d:10", "tree": "tree visible 1x1 n:0",
        }});
        assert_eq!(
            done_text(&reply).expect("text"),
            "+tap:on_text=\"OK\" d:10\ntree visible 1x1 n:0\n"
        );
    }
}
