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
    /// `None` picks it from the client that connects ([`soft_timeout_for`]).
    pub soft_timeout: Option<Duration>,
    /// Open sessions on the device-free stub driver. Debug builds only,
    /// for the integration tests.
    pub stub: bool,
}

/// The MCP server.
#[derive(Clone)]
pub struct GolemMcp {
    link: Arc<DaemonLink>,
    options: McpOptions,
    /// The soft timeout in force: `options.soft_timeout`, else the one
    /// picked for the client at `initialize`.
    soft_timeout: Arc<std::sync::Mutex<Duration>>,
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
#[serde(deny_unknown_fields)]
pub struct DevicesParams {
    /// "ios", "android", "ios:26", "ios:26+" or "ios:latest".
    pub os: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct OpenParams {
    /// "ios", "android", "ios:26", "ios:26+" or "ios:latest". Default: any.
    pub os: Option<String>,
    /// "phone" or "tablet".
    #[serde(rename = "type")]
    pub device_type: Option<String>,
    /// A UDID, a serial, a name, or part of one.
    pub device: Option<String>,
    /// Boot a device when no running one fits. Default true.
    pub boot: Option<bool>,
    /// The app's bundle id.
    pub bundle: Option<String>,
    /// The app's name in golem.toml [[apps]].
    pub app: Option<String>,
    /// The directory with golem.toml. Default: the server's project.
    pub project: Option<String>,
    /// End the session after this many idle seconds. Default 1800.
    pub idle_timeout_s: Option<u64>,
    /// Run this .test.toml first, as golem run does; the session opens
    /// where it stops. Its [[teardown]] runs at session_close.
    pub flow: Option<String>,
    /// With flow: stop before this step: block or block:step.
    pub stop_at: Option<String>,
    /// With flow: false does the setup but runs no steps.
    pub run: Option<bool>,
    /// With flow: open at a failed step instead of ending.
    #[serde(default)]
    pub break_on_failure: bool,
    /// With flow: false never runs its [[teardown]].
    pub teardown: Option<bool>,
    /// With flow: variables, as golem run --var.
    pub vars: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct CloseParams {
    /// Run the flow's [[teardown]]. Default true.
    pub teardown: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActParams {
    pub step: String,
    /// Goes above the step in the draft.
    pub comment: Option<String>,
    /// Also return the visible tree after the step.
    #[serde(default)]
    pub tree: bool,
    /// "toon" (compact text, default) or "json".
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProbeParams {
    /// { on_text = "Sign in" }; a whole step works too.
    pub selector: String,
    /// Poll this long while nothing matches. Default 0.
    pub timeout_ms: Option<u64>,
    /// "toon" (default) or "json".
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct TreeParams {
    /// Add off-screen elements: a hint, never proof.
    #[serde(default)]
    pub full: bool,
    /// "toon" (default) or "json".
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct WaitParams {
    /// Default: the server's soft timeout.
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct FlowSetParams {
    pub name: Option<String>,
    pub tags: Option<Vec<String>>,
    /// Merged into [flow] vars.
    pub vars: Option<std::collections::BTreeMap<String, String>>,
    /// The seed for fake: generators.
    pub seed: Option<u64>,
    /// true: golem run without a path skips this flow.
    pub explicit_only: Option<bool>,
    /// The first block.
    pub start: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AppsSetParams {
    /// { "name": "app", "bundle": "com.acme", "devices": [{ "os": "ios:latest" }] };
    /// also permissions, install_script. The same name replaces those fields.
    pub app: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockBeginParams {
    pub name: String,
    /// Its next block.
    pub next: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockLinkParams {
    pub block: String,
    pub next: Option<String>,
    /// [{ "if_visible": "Error", "goto": "retry" }]; or if_not_visible, or
    /// if_var with equals, matches or gte.
    pub branches: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StepNoteParams {
    pub step: String,
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StepEditParams {
    /// block:step
    pub at: String,
    /// The whole new step.
    pub step: Option<String>,
    /// "" removes it.
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StepAtParams {
    /// block:step
    pub at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StepMoveParams {
    /// block:step
    pub from: String,
    /// Its block:step after the move, counted without the moved step; one
    /// past a block's last step appends.
    pub to: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockRenameParams {
    pub name: String,
    pub to: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockNameParams {
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DataAddParams {
    /// { "email": "a@b.test", "name": "Ada" }
    pub row: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommentParams {
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportParams {
    /// Relative to the project, or absolute.
    pub path: String,
    /// Needed to replace a file this session did not open or export.
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct DraftRunParams {
    /// true: from the start. Default false: from the cursor.
    #[serde(default)]
    pub restart: bool,
    /// Stop before this step: block or block:step.
    pub stop_at: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct DraftStepsParams {
    /// "cursor" (default) or block:step.
    pub around: Option<String>,
    /// Steps on each side. Default 5.
    pub context: Option<usize>,
    /// List this block instead.
    pub block: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct HelpParams {
    /// Default: list every action.
    pub action: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct NoParams {}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct LogsParams {
    /// Seconds back. Default: since the session opened.
    pub since: Option<u64>,
    /// Only lines with this text, in any case.
    pub filter: Option<String>,
    /// Lines besides crash lines. Default 200, the newest.
    pub limit: Option<usize>,
    /// An app name or bundle id. Default: the session's app.
    pub app: Option<String>,
}

/// What every client keeps in context: what golem is for, and the rules
/// that many tools share, so that no tool repeats them.
const INSTRUCTIONS: &str = "\
golem drives iOS and Android devices (simulators, emulators) for mobile e2e tests. Use it to \
write or edit an e2e flow (.test.toml), or to debug an app live: reproduce a bug, read the \
screen and the app log.
Start: session_open. It boots a device if needed. With flow = path it runs that flow first; \
add run = false to edit the flow without running it.
Look: tree (the visible tree decides what is on screen; full is a hint only), probe (check a \
selector), screenshot, app_logs (crashes).
Act: act runs one step. A step is a one-line TOML inline table: \
{ action = \"tap\", on_text = \"OK\" }. actions_help lists the actions and their keys.
Draft: the session builds a flow draft: the flow file it opened, or a new one with the \
session's app whose first step starts block main. A flow is named blocks of steps. A step that passes in act goes into the draft at the cursor; a failed step \
does not. A ${var} stays a reference. Address a step as block:step, from 1. draft_steps \
shows each step's status: ✓ passed here, · not run here, ? unverified (# unverified in the \
file), ~ stale. A change (a new, edited, moved or deleted step) makes the next step ? and \
later steps ~. Change the draft with step_edit, step_delete, step_move, the block_ tools and \
record_only (no run). draft_run runs the draft again: steps that pass become ✓. export_flow \
writes the file.
A long call answers pending: call wait. busy: another operation is running; status, wait, \
cancel, app_logs and session_close still answer.";

/// The tools' input schemas without what costs a client tokens and tells
/// it nothing: the `$schema` dialect (MCP's default) and the `null` that
/// each optional field allows (leaving a field out says the same).
fn compact_schemas<S>(mut router: ToolRouter<S>) -> ToolRouter<S> {
    fn compact(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                map.remove("$schema");
                if let Some(serde_json::Value::Array(types)) = map.get("type") {
                    let kept: Vec<serde_json::Value> =
                        types.iter().filter(|t| *t != "null").cloned().collect();
                    if let [one] = kept.as_slice() {
                        map.insert("type".into(), one.clone());
                    }
                }
                if let Some(serde_json::Value::String(f)) = map.get("format") {
                    if f.starts_with("uint") || f.starts_with("int") {
                        map.remove("format");
                    }
                }
                for v in map.values_mut() {
                    compact(v);
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(compact),
            _ => {}
        }
    }
    for route in router.map.values_mut() {
        let mut schema = serde_json::Value::Object((*route.attr.input_schema).clone());
        compact(&mut schema);
        if let serde_json::Value::Object(map) = schema {
            route.attr.input_schema = Arc::new(map);
        }
    }
    router
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
            soft_timeout: Arc::new(std::sync::Mutex::new(
                options.soft_timeout.unwrap_or(DEFAULT_SOFT_TIMEOUT),
            )),
            options,
            tool_router: compact_schemas(Self::tool_router()),
        }
    }

    #[tool(
        description = "Devices in any state: platform, id, name, OS, state, live companion port."
    )]
    async fn devices(
        &self,
        Parameters(p): Parameters<DevicesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let query = target::TargetQuery {
            os: match p.os.as_deref().map(target::OsQuery::parse).transpose() {
                Ok(os) => os,
                Err(e) => return tool_error(format!("{e:#}")),
            },
            ..Default::default()
        };
        let mut entries = target::list_devices(query.platform()).await;
        let all: Vec<golem_devices::DeviceInfo> =
            entries.iter().map(|e| e.device.clone()).collect();
        entries.retain(|e| query.fits(&e.device, &all));
        if entries.is_empty() {
            return text(match &query.os {
                Some(os) => format!("no {} device found", os.text),
                None => "no devices found; is adb or xcrun on PATH?".to_string(),
            });
        }
        text(target::format_device_entries(&entries))
    }

    #[tool(
        description = "Open a session on one device and app. A running device that fits wins; else golem boots one. The session ends at session_close, when this server stops, or after idle_timeout_s idle."
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
                "os": p.os,
                "type": p.device_type,
                "device": p.device,
                "bundle": p.bundle,
                "app": p.app,
            },
            "project_root": project_root.display().to_string(),
            "idle_timeout_s": p.idle_timeout_s,
            "boot": p.boot.unwrap_or(true),
            "flow": p.flow,
            "stop_at": p.stop_at,
            "run": p.run,
            "break_on_failure": p.break_on_failure,
            "teardown": p.teardown,
            "vars": p.vars,
            "wait_ms": self.soft_timeout().as_millis() as u64,
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

    #[tool(description = "Close the session; release the device.")]
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
        description = "Run one step on the device. If it passes, it goes into the draft at the cursor, and the cursor moves after it. Examples: { action = \"tap\", on_text = \"Sign in\" } · { action = \"type\", on_text = \"Email\", input = \"a@b.test\" } · { action = \"assert_visible\", on_text = \"Welcome\" }. Warns when a step passes in half its timeout or more."
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
        description = "What a selector matches, without acting: each visible match, which one act picks, each anchor; on a miss, which clause removed the candidates. Never fails."
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
        description = "The screen: one indexed line per element you can target. Visible elements only, unless full. Target an element with selector keys (actions_help), not its index."
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

    #[tool(description = "The screen as a PNG.")]
    async fn screenshot(
        &self,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self.op("session_screenshot", serde_json::json!({})).await?;
        self.render(&reply, false)
    }

    #[tool(
        description = "Wait for the running operation, and return its result: the last result when none runs. It can answer pending again."
    )]
    async fn wait(
        &self,
        Parameters(p): Parameters<WaitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let wait = p.timeout_s.map_or(self.soft_timeout(), Duration::from_secs);
        let reply = self
            .call(
                "session_wait",
                serde_json::json!({ "wait_ms": wait.as_millis() as u64 }),
            )
            .await?;
        self.render(&reply, false)
    }

    #[tool(
        description = "Idle or busy, without waiting: the running operation and its phase, or the last result."
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

    #[tool(description = "Stop the running operation. No teardown runs.")]
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
        description = "The app's device log (Android logcat, iOS simulator; not physical iOS): crash lines first, then the newest. Answers while another operation runs."
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

    #[tool(description = "The draft as .test.toml text.")]
    async fn draft_show(
        &self,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self.op("session_draft_show", serde_json::json!({})).await?;
        self.render(&reply, false)
    }

    #[tool(
        description = "The draft's steps near the cursor, or one block's: block:step, status, step, comment. Block headers show next and branches."
    )]
    async fn draft_steps(
        &self,
        Parameters(p): Parameters<DraftStepsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self
            .op(
                "session_draft_steps",
                serde_json::json!({ "around": p.around, "context": p.context, "block": p.block, "limit": p.limit }),
            )
            .await?;
        self.render(&reply, false)
    }

    #[tool(
        description = "Run the draft on the device: no setup, no teardown, the app as it is. Steps that pass become ✓. Stops before stop_at, at a failed step, or at the end; the cursor goes there."
    )]
    async fn draft_run(
        &self,
        Parameters(p): Parameters<DraftRunParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let reply = self
            .op(
                "session_draft_run",
                serde_json::json!({ "restart": p.restart, "stop_at": p.stop_at }),
            )
            .await?;
        self.render(&reply, false)
    }

    #[tool(
        description = "Validate the draft as golem run does; if valid, write it. Reports the status counts and the unverified steps."
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

    #[tool(description = "Set [flow] fields of the draft.")]
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
        description = "Add or replace a [[flow.apps]] entry in the draft. The session does not change."
    )]
    async fn apps_set(
        &self,
        Parameters(p): Parameters<AppsSetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "apps_set", "app": p.app }))
            .await
    }

    #[tool(
        description = "Move the cursor to the end of a block; a new name adds the block at the end."
    )]
    async fn block_begin(
        &self,
        Parameters(p): Parameters<BlockBeginParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "block_begin", "name": p.name, "next": p.next }))
            .await
    }

    #[tool(
        description = "Set a block's next, and add branches. After a block's last step the flow takes the first branch whose condition holds, else next, else the next block in the file; after the last block it ends."
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

    #[tool(description = "Add a step to the draft's [[teardown]]. It does not run.")]
    async fn teardown_add(
        &self,
        Parameters(p): Parameters<StepNoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(
            serde_json::json!({ "edit": "teardown_add", "step": p.step, "comment": p.comment }),
        )
        .await
    }

    #[tool(
        description = "Add a [[data]] row. A block with for_each = \"data\" runs once per row; its steps read ${_each.field}."
    )]
    async fn data_add(
        &self,
        Parameters(p): Parameters<DataAddParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "data_add", "row": p.row }))
            .await
    }

    #[tool(description = "Add a comment line at the cursor.")]
    async fn comment_add(
        &self,
        Parameters(p): Parameters<CommentParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "comment_add", "text": p.text }))
            .await
    }

    #[tool(
        description = "Record a step at the cursor without running it, for a path the session does not take. It is ?."
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
        description = "Change a step without running it. Only a new comment or a larger timeout keeps its status; any other change makes it ?."
    )]
    async fn step_edit(
        &self,
        Parameters(p): Parameters<StepEditParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "step_edit", "at": p.at, "step": p.step, "comment": p.comment }))
            .await
    }

    #[tool(description = "Remove a step and its comment.")]
    async fn step_delete(
        &self,
        Parameters(p): Parameters<StepAtParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "step_delete", "at": p.at }))
            .await
    }

    #[tool(description = "Move a step. Where it was counts as a delete; where it lands it is ?.")]
    async fn step_move(
        &self,
        Parameters(p): Parameters<StepMoveParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "step_move", "from": p.from, "to": p.to }))
            .await
    }

    #[tool(description = "Rename a block, and each next, goto and start that names it.")]
    async fn block_rename(
        &self,
        Parameters(p): Parameters<BlockRenameParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "block_rename", "name": p.name, "to": p.to }))
            .await
    }

    #[tool(
        description = "Remove a block and its steps. Refused while a next, goto or start names it."
    )]
    async fn block_delete(
        &self,
        Parameters(p): Parameters<BlockNameParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.edit(serde_json::json!({ "edit": "block_delete", "name": p.name }))
            .await
    }

    #[tool(
        description = "The project's mixins and the vars each uses. Run one with act: { action = \"load_mixin\", mixin = \"name\", vars = { … } }."
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

    #[tool(description = "Every action, or one action's keys and examples.")]
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
    /// The soft timeout in force: after `initialize`, the one picked for
    /// the client.
    pub fn soft_timeout(&self) -> Duration {
        *self.soft_timeout.lock().unwrap_or_else(|e| e.into_inner())
    }

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
        msg["wait_ms"] = serde_json::json!(self.soft_timeout().as_millis() as u64);
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
            .with_instructions(INSTRUCTIONS)
    }

    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());
        let client = &request.client_info;
        if self.options.soft_timeout.is_none() {
            let picked = soft_timeout_for(&client.name);
            *self.soft_timeout.lock().unwrap_or_else(|e| e.into_inner()) = picked;
            eprintln!(
                "golem mcp: client {} {} · soft timeout {}s (--soft-timeout sets it)",
                client.name,
                client.version,
                picked.as_secs()
            );
        }
        self.negotiate_initialize(&request)
    }
}

/// The soft timeout for a client golem does not know.
pub const DEFAULT_SOFT_TIMEOUT: Duration = Duration::from_secs(45);

/// The most golem waits before `pending`, whatever the client allows: golem
/// knows a client's default limit, not the one its user set.
const MAX_SOFT_TIMEOUT: Duration = Duration::from_secs(120);

/// A client with no limit for one call.
const NO_LIMIT: Duration = Duration::from_secs(24 * 3600);

/// Each client's default limit for one tool call, by the `clientInfo.name`
/// it sends; a name ending in `*` matches a prefix. Only names read in the
/// client's source are listed: a guessed name would match nothing, or the
/// wrong client. Where the client's docs and its source disagree, the lower
/// limit is listed, so that golem answers `pending` before the client gives
/// up in either case (Codex: docs 60 s, source 300 s; Copilot CLI: docs
/// 30 s, source 180 s; LibreChat: docs 30 s, source 60 s).
const CLIENT_LIMITS: &[(&str, Duration)] = &[
    ("claude-code", NO_LIMIT),
    ("codex-mcp-client", Duration::from_secs(60)),
    ("gemini-cli-mcp-client", Duration::from_secs(600)),
    ("opencode", Duration::from_secs(60)),
    ("github-copilot-developer", Duration::from_secs(30)),
    ("goose-cli", Duration::from_secs(300)),
    ("goose-desktop", Duration::from_secs(300)),
    ("Cline", Duration::from_secs(60)),
    ("continue-client", Duration::from_secs(60)),
    ("Zed", Duration::from_secs(60)),
    ("claude-desktop-3p", Duration::from_secs(60)),
    ("local-agent-mode-*", Duration::from_secs(60)),
    ("@librechat/api-client", Duration::from_secs(30)),
    ("mcpc", Duration::from_secs(60)),
    ("ai-sdk-mcp-client", NO_LIMIT),
    ("spring-ai-mcp-client*", Duration::from_secs(20)),
    ("Visual Studio Code", NO_LIMIT),
    ("Code - OSS", NO_LIMIT),
];

/// The soft timeout for the client named `name`: two thirds of its limit
/// for one call, at most [`MAX_SOFT_TIMEOUT`] and at least 5 s, so that
/// golem answers `pending` before the client gives up. A client golem does
/// not know gets [`DEFAULT_SOFT_TIMEOUT`].
pub fn soft_timeout_for(name: &str) -> Duration {
    CLIENT_LIMITS
        .iter()
        .find(|(known, _)| match known.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => *known == name,
        })
        .map_or(DEFAULT_SOFT_TIMEOUT, |(_, limit)| {
            (*limit * 2 / 3).clamp(Duration::from_secs(5), MAX_SOFT_TIMEOUT)
        })
}

/// An MCP client that `golem mcp --print-config` writes a config block for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Client {
    /// Claude Code: `.mcp.json` at the project root.
    Claude,
    /// Codex CLI: `~/.codex/config.toml`, or `.codex/config.toml` in a
    /// trusted project.
    Codex,
    /// Claude Desktop and other GUI clients: `claude_desktop_config.json`.
    Desktop,
    /// OpenCode: `opencode.json` at the project root.
    Opencode,
    /// Gemini CLI: `.gemini/settings.json` in the project.
    Gemini,
    /// GitHub Copilot CLI: `.mcp.json`, `.github/mcp.json` or
    /// `~/.copilot/mcp-config.json`.
    Copilot,
    /// Goose: `~/.config/goose/config.yaml`.
    Goose,
    /// Zed: `.zed/settings.json` or `~/.config/zed/settings.json`.
    Zed,
    /// Continue: `.continue/mcpServers/golem.yaml`.
    Continue,
}

/// `s` as a YAML scalar: a JSON string is valid YAML.
fn yaml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// The config block that starts `exe mcp` from `client`.
///
/// Claude Code, OpenCode, Gemini CLI and Copilot CLI read these blocks
/// from a project file and start the server in the project directory, so
/// their blocks need no `--project`. Codex, Goose and the editors can
/// start the server anywhere. A GUI client also does not get the shell `PATH`, so its block
/// carries `env`: without it the daemon cannot find `adb` or `xcrun`.
pub fn client_config(
    client: Client,
    exe: &std::path::Path,
    project_root: &std::path::Path,
    env: &[(String, String)],
) -> String {
    let exe = exe.display().to_string();
    let project = project_root.display().to_string();
    match client {
        Client::Claude | Client::Gemini => {
            let v = serde_json::json!({
                "mcpServers": { "golem": { "command": exe, "args": ["mcp"] } }
            });
            serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
        }
        Client::Codex => {
            let mut server = toml_edit::Table::new();
            server.insert("command", toml_edit::value(exe));
            let mut args = toml_edit::Array::new();
            for a in ["mcp", "--project", project.as_str()] {
                args.push(a);
            }
            server.insert("args", toml_edit::value(args));
            let mut servers = toml_edit::Table::new();
            servers.set_implicit(true);
            servers.insert("golem", toml_edit::Item::Table(server));
            let mut doc = toml_edit::DocumentMut::new();
            doc.insert("mcp_servers", toml_edit::Item::Table(servers));
            doc.to_string()
        }
        Client::Copilot => {
            // Copilot's default limit for one call is 30 s, under golem's
            // 45 s soft timeout.
            let v = serde_json::json!({
                "mcpServers": { "golem": {
                    "type": "stdio",
                    "command": exe,
                    "args": ["mcp"],
                    "tools": ["*"],
                    "timeout": 120000,
                } }
            });
            serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
        }
        Client::Goose => format!(
            "extensions:\n  golem:\n    type: stdio\n    name: golem\n    enabled: true\n    \
             cmd: {}\n    args: [\"mcp\", \"--project\", {}]\n    envs: {{}}\n    timeout: 300\n",
            yaml_str(&exe),
            yaml_str(&project)
        ),
        Client::Zed => {
            let env: serde_json::Map<String, serde_json::Value> = env
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                .collect();
            let v = serde_json::json!({
                "context_servers": { "golem": {
                    "command": exe,
                    "args": ["mcp", "--project", project],
                    "env": env,
                } }
            });
            serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
        }
        Client::Continue => format!(
            "name: golem\nversion: 0.0.1\nschema: v1\nmcpServers:\n  - name: golem\n    \
             type: stdio\n    command: {}\n    args: [\"mcp\", \"--project\", {}]\n",
            yaml_str(&exe),
            yaml_str(&project)
        ),
        Client::Opencode => {
            let v = serde_json::json!({
                "$schema": "https://opencode.ai/config.json",
                "mcp": { "golem": { "type": "local", "command": [exe, "mcp"], "enabled": true } }
            });
            serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
        }
        Client::Desktop => {
            let env: serde_json::Map<String, serde_json::Value> = env
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                .collect();
            let v = serde_json::json!({
                "mcpServers": { "golem": {
                    "command": exe,
                    "args": ["mcp", "--project", project],
                    "env": env,
                } }
            });
            serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
        }
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

    /// Every client keeps the instructions and the tool list in context,
    /// also in sessions that never use golem: the text stays short.
    #[test]
    fn the_instructions_and_tool_text_stay_short() {
        let words = |s: &str| s.split_whitespace().count();
        assert!(
            words(INSTRUCTIONS) <= 270,
            "instructions: {}",
            words(INSTRUCTIONS)
        );
        let router = compact_schemas(GolemMcp::tool_router());
        for tool in router.list_all() {
            let name = tool.name.to_string();
            let d = tool.description.as_deref().unwrap_or_default();
            assert!(words(d) <= 70, "{name}: {} words", words(d));
            let schema = serde_json::Value::Object((*tool.input_schema).clone());
            assert!(schema.get("$schema").is_none(), "{name}");
            let props = schema["properties"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            for (key, p) in props {
                let pd = p["description"].as_str().unwrap_or_default();
                assert!(words(pd) <= 25, "{name}.{key}: {} words", words(pd));
                assert!(!p["type"].is_array(), "{name}.{key}: {}", p["type"]);
            }
        }
    }

    #[test]
    fn the_soft_timeout_stays_under_the_clients_own_limit() {
        let secs = |name: &str| soft_timeout_for(name).as_secs();
        assert_eq!(secs("github-copilot-developer"), 20, "30 s limit");
        assert_eq!(
            secs("spring-ai-mcp-client - golem"),
            13,
            "20 s limit, by prefix"
        );
        assert_eq!(secs("Zed"), 40, "60 s limit");
        assert_eq!(secs("goose-cli"), 120, "300 s limit, capped");
        assert_eq!(secs("claude-code"), 120, "no limit, capped");
        assert_eq!(secs("some-new-client"), 45, "unknown: the default");
        assert_eq!(secs("zed"), 45, "names match exactly, as clients send them");
        for (name, limit) in CLIENT_LIMITS {
            let name = name.trim_end_matches('*');
            assert!(soft_timeout_for(name) < *limit, "{name}");
        }
    }

    #[test]
    fn each_client_config_starts_this_binary_with_mcp() {
        let exe = std::path::Path::new("/opt/golem/bin/golem");
        let root = std::path::Path::new("/work/app");
        let env = vec![
            (
                "PATH".to_string(),
                "/usr/bin:/sdk/platform-tools".to_string(),
            ),
            ("ANDROID_HOME".to_string(), "/sdk".to_string()),
        ];

        let claude: serde_json::Value =
            serde_json::from_str(&client_config(Client::Claude, exe, root, &env)).expect("json");
        assert_eq!(
            claude,
            serde_json::json!({ "mcpServers": { "golem": {
                "command": "/opt/golem/bin/golem", "args": ["mcp"] } } })
        );

        let gemini: serde_json::Value =
            serde_json::from_str(&client_config(Client::Gemini, exe, root, &env)).expect("json");
        assert_eq!(gemini, claude, "Gemini CLI reads the same mcpServers block");

        let opencode: serde_json::Value =
            serde_json::from_str(&client_config(Client::Opencode, exe, root, &env)).expect("json");
        assert_eq!(
            opencode["mcp"]["golem"],
            serde_json::json!({ "type": "local", "command": ["/opt/golem/bin/golem", "mcp"], "enabled": true })
        );

        let copilot: serde_json::Value =
            serde_json::from_str(&client_config(Client::Copilot, exe, root, &env)).expect("json");
        let golem = &copilot["mcpServers"]["golem"];
        assert_eq!(golem["args"], serde_json::json!(["mcp"]));
        assert_eq!(golem["tools"], serde_json::json!(["*"]));
        assert!(
            golem["timeout"].as_u64() > Some(45_000),
            "Copilot's limit SHALL exceed golem's soft timeout"
        );

        let zed: serde_json::Value =
            serde_json::from_str(&client_config(Client::Zed, exe, root, &env)).expect("json");
        let golem = &zed["context_servers"]["golem"];
        assert_eq!(golem["command"], "/opt/golem/bin/golem");
        assert_eq!(
            golem["args"],
            serde_json::json!(["mcp", "--project", "/work/app"])
        );
        assert_eq!(golem["env"]["ANDROID_HOME"], "/sdk");

        let goose = client_config(Client::Goose, exe, root, &env);
        assert!(
            goose.starts_with("extensions:\n  golem:\n    type: stdio\n"),
            "{goose}"
        );
        assert!(
            goose.contains("    cmd: \"/opt/golem/bin/golem\"\n"),
            "{goose}"
        );
        assert!(
            goose.contains("    args: [\"mcp\", \"--project\", \"/work/app\"]\n"),
            "{goose}"
        );

        let cont = client_config(Client::Continue, exe, root, &env);
        assert!(
            cont.starts_with(
                "name: golem\nversion: 0.0.1\nschema: v1\nmcpServers:\n  - name: golem\n"
            ),
            "{cont}"
        );
        assert!(
            cont.contains("    command: \"/opt/golem/bin/golem\"\n"),
            "{cont}"
        );

        let codex: toml::Value =
            toml::from_str(&client_config(Client::Codex, exe, root, &env)).expect("toml");
        let golem = &codex["mcp_servers"]["golem"];
        assert_eq!(golem["command"].as_str(), Some("/opt/golem/bin/golem"));
        assert_eq!(
            golem["args"],
            toml::Value::Array(vec!["mcp".into(), "--project".into(), "/work/app".into()])
        );

        let desktop: serde_json::Value =
            serde_json::from_str(&client_config(Client::Desktop, exe, root, &env)).expect("json");
        let golem = &desktop["mcpServers"]["golem"];
        assert_eq!(
            golem["args"],
            serde_json::json!(["mcp", "--project", "/work/app"])
        );
        assert_eq!(golem["env"]["ANDROID_HOME"], "/sdk");
        assert_eq!(golem["env"]["PATH"], "/usr/bin:/sdk/platform-tools");
    }

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
