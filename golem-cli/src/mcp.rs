//! `golem mcp`: an MCP server over stdio, so an LLM client can drive a
//! device one step at a time.
//!
//! The server is a thin client of the daemon. A session runs in the daemon
//! and belongs to this server's connection: when the MCP client stops the
//! server, the connection drops and the daemon ends the session, without
//! the flow teardown. Stdout carries JSON-RPC only; logs go to stderr.
//!
//! Startup does no device work and does not touch the daemon: an MCP
//! client gives a server a few seconds to start. The first tool that needs
//! the daemon connects to it, or starts it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use golem_orchestrator::{daemon, ipc, target};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// How `golem mcp` reaches the daemon and the project.
#[derive(Debug, Clone)]
pub struct McpOptions {
    pub socket: PathBuf,
    /// The project `golem.toml` is read from, unless `session_open` names
    /// another.
    pub project_root: PathBuf,
    /// How long a tool waits for its operation before it answers `pending`.
    pub soft_timeout: Duration,
    /// Open sessions on the device-free stub driver. Debug builds only,
    /// for the integration tests.
    pub stub: bool,
}

/// The MCP server.
#[derive(Clone)]
pub struct GolemMcp {
    link: Arc<DaemonLink>,
    options: McpOptions,
    #[allow(dead_code)] // read by the `tool_handler` macro
    tool_router: ToolRouter<Self>,
}

/// One connection to the daemon, shared by every tool call. Replies are
/// matched to calls by `id`, so calls can run at the same time.
struct DaemonLink {
    socket: PathBuf,
    conn: tokio::sync::Mutex<Option<LinkConn>>,
    /// Why the session ended, when the daemon ended it on its own.
    ended: Arc<std::sync::Mutex<Option<String>>>,
}

struct LinkConn {
    writer: tokio::net::unix::OwnedWriteHalf,
    pending: Arc<std::sync::Mutex<HashMap<u64, tokio::sync::oneshot::Sender<serde_json::Value>>>>,
    next_id: u64,
}

impl DaemonLink {
    /// Send a `session_*` message and wait for its reply.
    async fn call(&self, kind: &str, mut msg: serde_json::Value) -> Result<serde_json::Value> {
        let rx = {
            let mut guard = self.conn.lock().await;
            if guard.is_none() {
                *guard = Some(self.connect().await?);
            }
            let conn = guard.as_mut().context("no daemon connection")?;
            conn.next_id += 1;
            let id = conn.next_id;
            msg["type"] = serde_json::json!(kind);
            msg["id"] = serde_json::json!(id);
            let (tx, rx) = tokio::sync::oneshot::channel();
            conn.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id, tx);
            if let Err(e) = conn.writer.write_all(format!("{msg}\n").as_bytes()).await {
                *guard = None;
                return Err(e).context("lost the connection to the golem daemon");
            }
            rx
        };
        rx.await
            .context("the golem daemon closed the connection; call session_open again")
    }

    async fn connect(&self) -> Result<LinkConn> {
        let stream = daemon::connect_or_start(
            &self.socket,
            crate::daemon_starter().as_ref(),
            &daemon::ClientOptions::current(),
        )
        .await?;
        let (read, writer) = stream.into_split();
        let pending: Arc<
            std::sync::Mutex<HashMap<u64, tokio::sync::oneshot::Sender<serde_json::Value>>>,
        > = Arc::default();
        let ended = self.ended.clone();
        let replies = pending.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if v["type"] == "session_ended" {
                    *ended.lock().unwrap_or_else(|e| e.into_inner()) =
                        v["reason"].as_str().map(str::to_string);
                    continue;
                }
                let Some(id) = v["id"].as_u64() else { continue };
                if let Some(tx) = replies
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id)
                {
                    let _ = tx.send(v);
                }
            }
            // Dropping the senders fails every call still waiting.
            replies.lock().unwrap_or_else(|e| e.into_inner()).clear();
        });
        Ok(LinkConn {
            writer,
            pending,
            next_id: 0,
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct DevicesParams {
    /// Only devices on this platform: "ios" or "android".
    pub platform: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct OpenParams {
    /// "ios" or "android".
    pub platform: Option<String>,
    /// The device: a UDID or serial, a name, or part of either. Needed when
    /// more than one device is booted.
    pub device: Option<String>,
    /// The bundle ID of the app.
    pub bundle: Option<String>,
    /// The app, by its name in the golem.toml [[apps]] registry.
    pub app: Option<String>,
    /// The project directory that holds golem.toml.
    pub project: Option<String>,
    /// End the session after this many seconds with no operation (default 1800).
    pub idle_timeout_s: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActParams {
    /// One step as a TOML inline table: { action = "tap", on_text = "Sign in" }
    pub step: String,
    /// Why this step, for the flow draft.
    pub comment: Option<String>,
    /// Also return the visible tree after the screen settles.
    #[serde(default)]
    pub tree: bool,
    /// "toon" (default) or "json".
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProbeParams {
    /// A selector as a TOML inline table: { on_text = "Sign in" }. A whole
    /// step works too; its action is ignored.
    pub selector: String,
    /// Poll up to this many milliseconds while nothing visible matches (default 0).
    pub timeout_ms: Option<u64>,
    /// "toon" (default) or "json".
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct TreeParams {
    /// The full tree, not only what is on screen: a hint only.
    #[serde(default)]
    pub full: bool,
    /// "toon" (default) or "json".
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct WaitParams {
    /// Wait up to this many seconds (default: the server's soft timeout).
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct HelpParams {
    /// One action to describe. Without it, every action is listed.
    pub action: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct NoParams {}

fn text(s: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(s)]))
}

fn tool_error(s: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![ContentBlock::text(s)]))
}

fn json_format(format: Option<&str>) -> bool {
    format == Some("json")
}

#[tool_router]
impl GolemMcp {
    pub fn new(options: McpOptions) -> Self {
        GolemMcp {
            link: Arc::new(DaemonLink {
                socket: options.socket.clone(),
                conn: tokio::sync::Mutex::new(None),
                ended: Arc::default(),
            }),
            options,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "List devices: platform, id, name, OS, state, and the port of a live companion."
    )]
    async fn devices(
        &self,
        Parameters(p): Parameters<DevicesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let platform = match p.platform.as_deref() {
            None => None,
            Some("ios") => Some(golem_devices::Platform::Ios),
            Some("android") => Some(golem_devices::Platform::Android),
            Some(other) => {
                return tool_error(format!("unknown platform: {other}; use ios or android"))
            }
        };
        let entries = target::list_devices(platform).await;
        if entries.is_empty() {
            return text(
                "no devices found; is adb or xcrun on PATH? Start a simulator or emulator",
            );
        }
        text(target::format_device_entries(&entries))
    }

    #[tool(
        description = "Open a session on one device and app. The session holds the device until session_close, until this server stops, or until it is idle for idle_timeout_s."
    )]
    async fn session_open(
        &self,
        Parameters(p): Parameters<OpenParams>,
        meta: rmcp::model::RequestMetaObject,
        client: rmcp::Peer<rmcp::RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let project_root = p
            .project
            .map(PathBuf::from)
            .unwrap_or_else(|| self.options.project_root.clone());
        let mut msg = serde_json::json!({
            "query": {
                "platform": p.platform,
                "device": p.device,
                "bundle": p.bundle,
                "app": p.app,
            },
            "project_root": project_root.display().to_string(),
            "idle_timeout_s": p.idle_timeout_s,
        });
        if self.options.stub {
            msg["stub"] = serde_json::json!(true);
        }
        ipc::add_client_context(&mut msg);
        *self.link.ended.lock().unwrap_or_else(|e| e.into_inner()) = None;
        // Starting a companion can take most of a minute: keep a client
        // that sent a progress token from timing the call out.
        let ticker = meta.get_progress_token().map(|token| {
            tokio::spawn(async move {
                let mut elapsed = 0u64;
                loop {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    elapsed += 5;
                    let _ = client
                        .notify_progress(
                            rmcp::model::ProgressNotificationParam::new(
                                token.clone(),
                                elapsed as f64,
                            )
                            .with_message(format!("opening the session · {elapsed}s")),
                        )
                        .await;
                }
            })
        });
        let reply = self.call("session_open", msg).await;
        if let Some(t) = ticker {
            t.abort();
        }
        let reply = reply?;
        match reply["status"].as_str() {
            Some("open") => text(format!(
                "session open · {} ({}) · app {} · idle timeout {}s",
                reply["device"].as_str().unwrap_or_default(),
                reply["udid"].as_str().unwrap_or_default(),
                non_empty(reply["bundle"].as_str()).unwrap_or("(last launched)"),
                reply["idle_timeout_s"]
            )),
            _ => self.not_done(&reply),
        }
    }

    #[tool(description = "Close the session and release its device.")]
    async fn session_close(
        &self,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self.call("session_close", serde_json::json!({})).await?;
        match reply["status"].as_str() {
            Some("closed") => text("session closed"),
            _ => self.not_done(&reply),
        }
    }

    #[tool(
        description = "Run one step and return its result. The step is a TOML inline table, the same text as a step in a flow file: { action = \"tap\", on_text = \"Sign in\" }. Call actions_help for the actions."
    )]
    async fn act(&self, Parameters(p): Parameters<ActParams>) -> Result<CallToolResult, ErrorData> {
        let _ = &p.comment;
        let reply = self
            .op(
                "session_act",
                serde_json::json!({ "step": p.step, "tree": p.tree }),
            )
            .await?;
        self.render(&reply, json_format(p.format.as_deref()))
    }

    #[tool(
        description = "Show what a selector matches on screen without acting: every visible match, which one act would pick, each anchor, and on a miss which clause removed the candidates. Never fails."
    )]
    async fn probe(
        &self,
        Parameters(p): Parameters<ProbeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self
            .op(
                "session_probe",
                serde_json::json!({ "selector": p.selector, "timeout_ms": p.timeout_ms.unwrap_or(0) }),
            )
            .await?;
        self.render(&reply, json_format(p.format.as_deref()))
    }

    #[tool(
        description = "The UI tree: one indexed line per element you can target. The visible tree by default; full = true adds what is off-screen, as a hint only."
    )]
    async fn tree(
        &self,
        Parameters(p): Parameters<TreeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let json = json_format(p.format.as_deref());
        let reply = self
            .op(
                "session_tree",
                serde_json::json!({ "full": p.full, "json": json }),
            )
            .await?;
        self.render(&reply, json)
    }

    #[tool(description = "A screenshot of the device, as a PNG image.")]
    async fn screenshot(
        &self,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self.op("session_screenshot", serde_json::json!({})).await?;
        self.render(&reply, false)
    }

    #[tool(
        description = "Wait for the running operation and return its result, or the last result when none is running."
    )]
    async fn wait(
        &self,
        Parameters(p): Parameters<WaitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let wait = p
            .timeout_s
            .map_or(self.options.soft_timeout, Duration::from_secs);
        let reply = self
            .call(
                "session_wait",
                serde_json::json!({ "wait_ms": wait.as_millis() as u64 }),
            )
            .await?;
        self.render(&reply, false)
    }

    #[tool(
        description = "Whether the session is idle or busy, with the running operation, or the last result."
    )]
    async fn status(
        &self,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self.call("session_status", serde_json::json!({})).await?;
        match reply["status"].as_str() {
            Some("idle") => {
                let mut out = format!("idle {}s", reply["idle_ms"].as_u64().unwrap_or(0) / 1000);
                if let Some(last) = reply.get("last").filter(|l| !l.is_null()) {
                    out.push_str(&format!(
                        " · last: op {} {}",
                        last["op_id"],
                        last["op"].as_str().unwrap_or_default()
                    ));
                }
                text(out)
            }
            Some("busy") => text(running_line("busy", &reply)),
            _ => self.not_done(&reply),
        }
    }

    #[tool(description = "Stop the running operation. The teardown does not run.")]
    async fn cancel(
        &self,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self.call("session_cancel", serde_json::json!({})).await?;
        match reply["cancelled"].as_bool() {
            Some(true) => text("cancelled"),
            Some(false) => text("nothing was running"),
            None => self.not_done(&reply),
        }
    }

    #[tool(
        description = "How to write steps. Without an action: the notation and every action. With an action: its description and examples."
    )]
    async fn actions_help(
        &self,
        Parameters(p): Parameters<HelpParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match p.action {
            None => text(actions_overview()),
            Some(action) => match action_section(&action) {
                Some(section) => text(section),
                None => tool_error(format!(
                    "unknown action: {action}; call actions_help without an action for the list"
                )),
            },
        }
    }
}

impl GolemMcp {
    async fn call(
        &self,
        kind: &str,
        msg: serde_json::Value,
    ) -> Result<serde_json::Value, ErrorData> {
        self.link
            .call(kind, msg)
            .await
            .map_err(|e| ErrorData::internal_error(format!("{e:#}"), None))
    }

    /// A session operation, waiting up to the soft timeout.
    async fn op(
        &self,
        kind: &str,
        mut msg: serde_json::Value,
    ) -> Result<serde_json::Value, ErrorData> {
        msg["wait_ms"] = serde_json::json!(self.options.soft_timeout.as_millis() as u64);
        self.call(kind, msg).await
    }

    fn render(&self, reply: &serde_json::Value, json: bool) -> Result<CallToolResult, ErrorData> {
        if reply["status"] != "done" {
            return self.not_done(reply);
        }
        let r = &reply["result"];
        if let Some(err) = r["error"].as_str() {
            return tool_error(err.to_string());
        }
        if r["cancelled"] == true {
            return text(format!(
                "op {} {} was cancelled",
                reply["op_id"],
                reply["op"].as_str().unwrap_or_default()
            ));
        }
        if let Some(png) = r["png_base64"].as_str() {
            return Ok(CallToolResult::success(vec![ContentBlock::image(
                png,
                "image/png",
            )]));
        }
        if json {
            let body = r
                .get("step")
                .or_else(|| r.get("report"))
                .filter(|v| !v.is_null())
                .cloned()
                .map(|v| serde_json::json!({ "op_id": reply["op_id"], "result": v, "tree": r["tree"] }))
                .unwrap_or_else(|| serde_json::json!({ "op_id": reply["op_id"], "tree": r["tree"] }));
            return text(serde_json::to_string_pretty(&body).unwrap_or_default());
        }
        let mut out = String::new();
        for key in ["toon", "tree"] {
            if let Some(s) = r[key].as_str() {
                out.push_str(s);
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        text(out)
    }

    /// A reply that is not a finished operation.
    fn not_done(&self, reply: &serde_json::Value) -> Result<CallToolResult, ErrorData> {
        match reply["status"].as_str() {
            Some("pending") => text(format!(
                "{}\n→ call wait() for the result",
                running_line("pending", reply)
            )),
            Some("busy") => tool_error(format!("{} · call wait()", running_line("busy", reply))),
            Some("ended") => tool_error(format!(
                "the session ended ({}); call session_open",
                reply["reason"].as_str().unwrap_or("closed")
            )),
            Some("idle") => text("idle: nothing has run yet"),
            _ => {
                let mut message = reply["message"]
                    .as_str()
                    .unwrap_or("unknown error")
                    .to_string();
                if let Some(reason) = self
                    .link
                    .ended
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                {
                    message = format!("the session ended ({reason}); call session_open");
                }
                tool_error(message)
            }
        }
    }
}

fn running_line(status: &str, r: &serde_json::Value) -> String {
    format!(
        "{status} · op {} {} · phase: {} · {}s",
        r["op_id"],
        r["op"].as_str().unwrap_or_default(),
        r["phase"].as_str().unwrap_or_default(),
        r["elapsed_ms"].as_u64().unwrap_or(0) / 1000
    )
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.is_empty())
}

const ACTIONS_REFERENCE: &str = include_str!("../../docs/actions-reference.md");

/// The notation and every action with its one-line summary.
fn actions_overview() -> String {
    let mut out = String::from(
        "A step is one TOML inline table, the same text as a step in a flow file:\n\
         { action = \"tap\", on_text = \"Sign in\" }\n\
         Selectors: on_text, on_accessibility_label (label or id), on_index, on_below, on_above, \
         on_right_of, on_left_of, or a group on = { text = …, contains = …, inside = …, traits = [...] }.\n\
         Common options: timeout (ms), auto_scroll = true, if_fail = \"warn\", retry.\n\
         Call actions_help(action) for one action's fields and examples.\n\nActions:\n",
    );
    let summaries = section_titles();
    for action in golem_parser::validation::known_actions() {
        let summary = summaries
            .iter()
            .find(|(names, _)| names.iter().any(|n| n == action))
            .map(|(_, s)| s.as_str())
            .unwrap_or("");
        out.push_str(&format!(
            "- {action}{}\n",
            if summary.is_empty() {
                String::new()
            } else {
                format!(": {summary}")
            }
        ));
    }
    out
}

/// Each `### \`action\` — summary` heading: its action names and summary.
fn section_titles() -> Vec<(Vec<String>, String)> {
    ACTIONS_REFERENCE
        .lines()
        .filter_map(|l| l.strip_prefix("### "))
        .filter(|l| l.starts_with('`'))
        .map(|l| {
            let (names, summary) = l.split_once(" — ").unwrap_or((l, ""));
            let names = names
                .split('`')
                .enumerate()
                .filter(|(i, _)| i % 2 == 1)
                .map(|(_, n)| n.to_string())
                .collect();
            (names, summary.to_string())
        })
        .collect()
}

/// The reference section for `action`, from its heading to the next.
fn action_section(action: &str) -> Option<String> {
    let lines: Vec<&str> = ACTIONS_REFERENCE.lines().collect();
    let start = lines.iter().position(|l| {
        l.strip_prefix("### ").is_some_and(|h| {
            h.split(" — ")
                .next()
                .unwrap_or("")
                .contains(&format!("`{action}`"))
        })
    })?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("### ") || l.starts_with("## "))
        .map_or(lines.len(), |i| start + 1 + i);
    Some(lines[start..end].join("\n").trim_end().to_string() + "\n")
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for GolemMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("golem", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "golem drives an iOS or Android device. Call session_open first (devices lists \
                 them). Then use tree to see the screen, probe to check a selector, and act to \
                 run one step. Steps are TOML inline tables: { action = \"tap\", on_text = \"OK\" }. \
                 A long operation returns pending: call wait.",
            )
    }
}

/// Run the server on stdio until the client closes it.
pub async fn serve(options: McpOptions) -> Result<()> {
    use rmcp::ServiceExt;
    let service = GolemMcp::new(options)
        .serve(rmcp::transport::stdio())
        .await
        .context("MCP initialisation failed")?;
    service.waiting().await.context("MCP server failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_action_has_a_reference_section() {
        for action in golem_parser::validation::known_actions() {
            assert!(action_section(action).is_some(), "no section for {action}");
        }
    }

    #[test]
    fn a_section_starts_at_its_heading_and_stops_at_the_next() {
        let tap = action_section("tap").expect("tap");
        assert!(tap.starts_with("### `tap`"), "{tap}");
        assert!(
            tap.contains(r#"{ action = "tap", on_text = "Submit" }"#),
            "{tap}"
        );
        assert!(!tap.contains("### `double_tap`"), "{tap}");
        assert!(action_section("explode").is_none());
    }

    #[test]
    fn the_overview_lists_every_action_with_its_summary() {
        let o = actions_overview();
        assert!(o.contains("- tap: Tap an element"), "{o}");
        for action in golem_parser::validation::known_actions() {
            assert!(o.contains(&format!("- {action}")), "{action}");
        }
    }
}
