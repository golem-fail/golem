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
    /// Run this flow file first (setup, apps, launch, steps), and open the
    /// session where it stops. Its [[teardown]] runs at session_close.
    pub flow: Option<String>,
    /// With flow: stop before this step, "block" or "block:step" (steps count from 1).
    pub stop_at: Option<String>,
    /// With flow: keep the session open at a failed step. Without it, a
    /// failed flow ends as golem run would, and no session opens.
    #[serde(default)]
    pub break_on_failure: bool,
    /// With flow: false skips its [[teardown]] however the session ends.
    pub teardown: Option<bool>,
    /// With flow: variables to set, as with golem run --var.
    pub vars: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct CloseParams {
    /// Run the [[teardown]] of the flow the session opened from (default true).
    pub teardown: Option<bool>,
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
pub struct FlowSetParams {
    pub name: Option<String>,
    pub tags: Option<Vec<String>>,
    /// Merged into [flow] vars.
    pub vars: Option<std::collections::BTreeMap<String, String>>,
    pub seed: Option<u64>,
    pub explicit_only: Option<bool>,
    /// The block the flow starts at.
    pub start: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AppsSetParams {
    /// One [[flow.apps]] entry: { "name": "app", "bundle": "com.acme",
    /// "devices": [{ "os": "ios:latest", "type": "phone" }],
    /// "permissions": { "camera": "allow" }, "install_script": "scripts/install.sh" }.
    /// An entry with an existing name replaces those fields.
    pub app: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockBeginParams {
    /// The block to record into next; created when the draft has none by this name.
    pub name: String,
    /// The block that follows it.
    pub next: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockLinkParams {
    pub block: String,
    pub next: Option<String>,
    /// Branches to add: [{ "if_visible": "Error", "goto": "retry" }], or
    /// if_not_visible, or if_var with equals, matches or gte.
    pub branches: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StepNoteParams {
    /// One step as a TOML inline table. It does not run.
    pub step: String,
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DataAddParams {
    /// One [[data]] row: { "email": "a@b.test", "name": "Ada" }.
    pub row: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommentParams {
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportParams {
    /// Where to write the .test.toml, relative to the project directory or absolute.
    pub path: String,
    /// Replace a file the session did not open from.
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct HelpParams {
    /// One action to describe. Without it, every action is listed.
    pub action: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct NoParams {}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct LogsParams {
    /// Seconds back from now. Without it, lines since the session opened.
    pub since: Option<u64>,
    /// Only lines whose tag or message contains this text, in any case.
    pub filter: Option<String>,
    /// The most lines to show besides crash lines. Default 200, the newest.
    pub limit: Option<usize>,
    /// The app's name in the flow, or its bundle id. Without it, the session's app.
    pub app: Option<String>,
}

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
            "flow": p.flow,
            "stop_at": p.stop_at,
            "break_on_failure": p.break_on_failure,
            "teardown": p.teardown,
            "vars": p.vars,
            "wait_ms": self.options.soft_timeout.as_millis() as u64,
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
        self.render(&reply, false)
    }

    #[tool(description = "Close the session and release its device.")]
    async fn session_close(
        &self,
        Parameters(p): Parameters<CloseParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self
            .call(
                "session_close",
                serde_json::json!({ "teardown": p.teardown.unwrap_or(true) }),
            )
            .await?;
        match reply["status"].as_str() {
            Some("closed") => match reply["teardown"].as_str() {
                Some(notes) => text(format!("session closed · {notes}")),
                None => text("session closed"),
            },
            _ => self.not_done(&reply),
        }
    }

    #[tool(
        description = "Run one step and return its result. The step is a TOML inline table, the same text as a step in a flow file: { action = \"tap\", on_text = \"Sign in\" }. Call actions_help for the actions."
    )]
    async fn act(&self, Parameters(p): Parameters<ActParams>) -> Result<CallToolResult, ErrorData> {
        let reply = self
            .op(
                "session_act",
                serde_json::json!({ "step": p.step, "tree": p.tree, "comment": p.comment }),
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
        description = "The app's device log (Android logcat, iOS simulator unified log): crash lines first, then the newest lines. Works while an act is still running, so use it when the app hangs or crashes."
    )]
    async fn app_logs(
        &self,
        Parameters(p): Parameters<LogsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self
            .call(
                "session_logs",
                serde_json::json!({ "since_secs": p.since, "filter": p.filter, "limit": p.limit, "app": p.app }),
            )
            .await?;
        match reply["logs"].as_str() {
            Some(logs) => text(logs),
            None => self.not_done(&reply),
        }
    }

    #[tool(
        description = "The flow draft: every step that passed in act, at its insertion point, as .test.toml text."
    )]
    async fn draft_show(
        &self,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self.op("session_draft_show", serde_json::json!({})).await?;
        self.render(&reply, false)
    }

    #[tool(
        description = "Check the flow draft as golem run would, then write it to path. A file the session did not open from needs overwrite = true."
    )]
    async fn export_flow(
        &self,
        Parameters(p): Parameters<ExportParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self
            .op(
                "session_export",
                serde_json::json!({ "path": p.path, "overwrite": p.overwrite }),
            )
            .await?;
        self.render(&reply, false)
    }

    #[tool(
        description = "Set [flow] fields of the draft: name, tags, vars, seed, explicit_only, start."
    )]
    async fn flow_set(
        &self,
        Parameters(p): Parameters<FlowSetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({
            "edit": "flow_set", "name": p.name, "tags": p.tags, "vars": p.vars,
            "seed": p.seed, "explicit_only": p.explicit_only, "start": p.start,
        }))
        .await
    }

    #[tool(
        description = "Add or replace a [[flow.apps]] entry in the draft. It need not match the session's device."
    )]
    async fn apps_set(
        &self,
        Parameters(p): Parameters<AppsSetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "apps_set", "app": p.app }))
            .await
    }

    #[tool(
        description = "Record the next steps into block name, creating it at the end of the draft if it does not exist."
    )]
    async fn block_begin(
        &self,
        Parameters(p): Parameters<BlockBeginParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "block_begin", "name": p.name, "next": p.next }))
            .await
    }

    #[tool(
        description = "Set a block's next block and add branches (if_visible / if_not_visible / if_var, then goto)."
    )]
    async fn block_link(
        &self,
        Parameters(p): Parameters<BlockLinkParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({
            "edit": "block_link", "block": p.block, "next": p.next, "branches": p.branches,
        }))
        .await
    }

    #[tool(description = "Add a step to the draft's [[teardown]]. The step does not run now.")]
    async fn teardown_add(
        &self,
        Parameters(p): Parameters<StepNoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(
            serde_json::json!({ "edit": "teardown_add", "step": p.step, "comment": p.comment }),
        )
        .await
    }

    #[tool(description = "Add a [[data]] row to the draft, for a for_each block.")]
    async fn data_add(
        &self,
        Parameters(p): Parameters<DataAddParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "data_add", "row": p.row }))
            .await
    }

    #[tool(description = "Add a comment line to the draft where the next step goes.")]
    async fn comment_add(
        &self,
        Parameters(p): Parameters<CommentParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "comment_add", "text": p.text }))
            .await
    }

    #[tool(
        description = "Record a step without running it, for a path the session does not take. It is marked # unverified, and export_flow lists it."
    )]
    async fn record_only(
        &self,
        Parameters(p): Parameters<StepNoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(
            serde_json::json!({ "edit": "record_only", "step": p.step, "comment": p.comment }),
        )
        .await
    }

    #[tool(
        description = "The project's mixins and the vars each expects. Use one with act('{ action = \"load_mixin\", mixin = \"name\", vars = { … } }')."
    )]
    async fn mixins_list(
        &self,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let found = golem_orchestrator::draft::mixins(&self.options.project_root);
        if found.is_empty() {
            return text("no mixins: the project has no __mixins__/ directory");
        }
        let mut out = String::new();
        for m in found {
            let rel = m
                .path
                .strip_prefix(&self.options.project_root)
                .unwrap_or(&m.path)
                .display()
                .to_string();
            out.push_str(&format!(
                "{} ({rel}){}\n",
                m.name,
                if m.vars.is_empty() {
                    String::new()
                } else {
                    format!(" · vars: {}", m.vars.join(", "))
                }
            ));
        }
        text(out)
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

    async fn edit(&self, msg: serde_json::Value) -> Result<CallToolResult, ErrorData> {
        let reply = self.op("session_edit", msg).await?;
        self.render(&reply, false)
    }

    fn render(&self, reply: &serde_json::Value, json: bool) -> Result<CallToolResult, ErrorData> {
        if reply["status"] != "done" {
            return self.not_done(reply);
        }
        let r = &reply["result"];
        if let Some(err) = r["error"].as_str() {
            return tool_error(err.to_string());
        }
        if let Some(note) = crate::session_cmd::note_text(reply) {
            return text(note);
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
        text(crate::session_cmd::toon_text(r))
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
