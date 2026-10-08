//! The `session_*` messages on a daemon connection.
//!
//! A message without a `session` name uses the connection's session: a connection
//! holds at most one, and it ends with the connection. A message with a
//! `name` uses the daemon's session of that name, which outlives each
//! connection, so a shell can use it across commands. Each message carries
//! an `id` that its reply echoes, and each runs in its own task: `wait`,
//! `status` and `cancel` must answer while an operation runs, and a client
//! may send several calls at once.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use crate::session::{
    Begin, LogsRequest, Op, OpResult, OpenRequest, Outcome, Running, Session, Status, Waited,
};

/// How long a call waits for its operation before it answers `pending`.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(45);

type Writer = Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>;

/// The session of one connection, if it opened one.
#[derive(Default, Clone)]
pub(crate) struct ConnSession {
    slot: Arc<tokio::sync::Mutex<Option<Arc<Session>>>>,
}

impl ConnSession {
    /// End the session when its connection drops: no teardown.
    pub(crate) async fn disconnect(&self) {
        if let Some(session) = self.slot.lock().await.take() {
            session.close("client disconnected", false).await;
        }
    }
}

/// The daemon's sessions opened by name.
#[derive(Default, Clone)]
pub(crate) struct Named {
    map: Arc<tokio::sync::Mutex<BTreeMap<String, Arc<Session>>>>,
}

/// Where a session lives: on its connection, or under a name.
#[derive(Clone)]
enum Home {
    Conn(ConnSession, Writer),
    Named(Named, String),
}

impl Home {
    fn of(msg: &serde_json::Value, conn: &ConnSession, named: &Named, writer: &Writer) -> Home {
        match msg["session"].as_str() {
            Some(name) => Home::Named(named.clone(), name.to_string()),
            None => Home::Conn(conn.clone(), writer.clone()),
        }
    }

    async fn get(&self) -> Option<Arc<Session>> {
        match self {
            Home::Conn(conn, _) => conn.slot.lock().await.clone(),
            Home::Named(named, name) => named.map.lock().await.get(name).cloned(),
        }
    }

    /// Forget `session` if it is still the one here.
    async fn forget(&self, session: &Arc<Session>) {
        match self {
            Home::Conn(conn, _) => {
                let mut slot = conn.slot.lock().await;
                if slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, session)) {
                    *slot = None;
                }
            }
            Home::Named(named, name) => {
                let mut map = named.map.lock().await;
                if map.get(name).is_some_and(|s| Arc::ptr_eq(s, session)) {
                    map.remove(name);
                }
            }
        }
    }

    fn missing(&self) -> String {
        match self {
            Home::Conn(..) => {
                "no session is open on this connection; call session_open first".into()
            }
            Home::Named(_, name) => format!(
                "no session named {name:?} is open; start one with `golem session start{}`",
                if name == "default" {
                    String::new()
                } else {
                    format!(" --name {name}")
                }
            ),
        }
    }
}

/// What the daemon shares with every session: devices, the install
/// cache, the device cap and the session count.
#[derive(Clone)]
pub(crate) struct Resources {
    pub resource_mgr: Arc<golem_devices::resource_manager::ResourceManager>,
    pub install_cache: golem_runner::installer::InstallCache,
    pub slots: Arc<crate::session::Slots>,
    /// Open sessions, for the daemon's idle check.
    pub sessions: Arc<std::sync::atomic::AtomicU64>,
}

/// Answer one `session_*` message in its own task.
pub(crate) fn spawn(
    msg: serde_json::Value,
    conn: ConnSession,
    resources: Resources,
    named: Named,
    writer: Writer,
) {
    tokio::spawn(async move {
        let id = msg["id"].clone();
        let mut reply = handle(&msg, &conn, &resources, &named, &writer).await;
        reply["type"] = serde_json::json!("session_reply");
        reply["id"] = id;
        let mut w = writer.lock().await;
        let _ = w.write_all(format!("{reply}\n").as_bytes()).await;
    });
}

async fn handle(
    msg: &serde_json::Value,
    conn: &ConnSession,
    resources: &Resources,
    named: &Named,
    writer: &Writer,
) -> serde_json::Value {
    let kind = msg["type"].as_str().unwrap_or_default();
    if kind == "session_list" {
        return list_json(named).await;
    }
    let home = Home::of(msg, conn, named, writer);
    if kind == "session_open" {
        return open(msg, &home, resources).await;
    }
    let Some(session) = home.get().await else {
        return error(&home.missing());
    };
    let wait = msg["wait_ms"]
        .as_u64()
        .map_or(DEFAULT_WAIT, Duration::from_millis);
    let op = match kind {
        "session_act" => Some(Op::Act {
            step: msg["step"].as_str().unwrap_or_default().to_string(),
            tree: msg["tree"].as_bool().unwrap_or(false),
            comment: msg["comment"].as_str().map(str::to_string),
        }),
        "session_draft_show" => Some(Op::DraftShow),
        "session_draft_steps" => Some(Op::DraftSteps(crate::draft::StepsQuery {
            around: msg["around"].as_str().map(str::to_string),
            context: msg["context"].as_u64().map(|n| n as usize),
            block: msg["block"].as_str().map(str::to_string),
            limit: msg["limit"].as_u64().map(|n| n as usize),
        })),
        "session_edit" => match parse_edit(msg) {
            Ok(edit) => Some(Op::Edit(edit)),
            Err(e) => return error(&format!("{e:#}")),
        },
        "session_export" => Some(Op::Export {
            path: std::path::PathBuf::from(msg["path"].as_str().unwrap_or_default()),
            overwrite: msg["overwrite"].as_bool().unwrap_or(false),
        }),
        "session_tree" => Some(Op::Tree {
            full: msg["full"].as_bool().unwrap_or(false),
            json: msg["json"].as_bool().unwrap_or(false),
        }),
        "session_probe" => Some(Op::Probe {
            selector: msg["selector"].as_str().unwrap_or_default().to_string(),
            timeout_ms: msg["timeout_ms"].as_u64().unwrap_or(0),
        }),
        "session_screenshot" => Some(Op::Screenshot),
        _ => None,
    };
    if let Some(op) = op {
        return match session.begin(op) {
            Begin::Started(_) => match (session.wait(wait).await, msg["path"].as_str()) {
                (
                    Waited::Done(Outcome {
                        op_id,
                        result: OpResult::Screenshot { png },
                        ..
                    }),
                    Some(path),
                ) => match std::fs::write(path, &png) {
                    Ok(()) => serde_json::json!({
                        "status": "done",
                        "op_id": op_id,
                        "op": "screenshot",
                        "result": { "saved": path, "bytes": png.len() },
                    }),
                    Err(e) => error(&format!("could not write {path}: {e}")),
                },
                (waited, _) => waited_json(waited),
            },
            Begin::Busy(running) => running_json("busy", &running),
            Begin::Ended(reason) => ended_json(&reason),
        };
    }
    match kind {
        "session_logs" => {
            let req = LogsRequest {
                since_secs: msg["since_secs"].as_u64(),
                filter: msg["filter"].as_str().map(str::to_string),
                limit: msg["limit"].as_u64().map(|n| n as usize),
                app: msg["app"].as_str().map(str::to_string),
            };
            match session.app_logs(&req).await {
                Ok(logs) => serde_json::json!({ "status": "ok", "logs": logs }),
                Err(e) => error(&format!("{e:#}")),
            }
        }
        "session_wait" => waited_json(session.wait(wait).await),
        "session_status" => status_json(&session.status()),
        "session_cancel" => {
            let cancelled = session.cancel().await;
            serde_json::json!({ "status": "ok", "cancelled": cancelled })
        }
        "session_close" => {
            let teardown = msg["teardown"].as_bool().unwrap_or(true);
            let notes = session.close("closed", teardown).await;
            home.forget(&session).await;
            serde_json::json!({ "status": "closed", "teardown": notes })
        }
        other => error(&format!("unknown session message: {other}")),
    }
}

async fn open(msg: &serde_json::Value, home: &Home, resources: &Resources) -> serde_json::Value {
    let Resources {
        resource_mgr,
        install_cache,
        slots,
        sessions,
    } = resources;
    // Held until the new session is in place, so two opens cannot race.
    let mut conn_slot = None;
    let mut named_map = None;
    match home {
        Home::Conn(conn, _) => {
            let slot = conn.slot.clone().lock_owned().await;
            if slot.is_some() {
                return error("a session is already open on this connection; close it first");
            }
            conn_slot = Some(slot);
        }
        Home::Named(named, name) => {
            let map = named.map.clone().lock_owned().await;
            if map.contains_key(name) {
                return error(&format!(
                    "a session named {name:?} is already open; stop it first, or pass another --name"
                ));
            }
            named_map = Some(map);
        }
    }
    let (query, project_root) = match crate::interactive::parse_query(&msg["query"])
        .and_then(|q| Ok((q, crate::interactive::project_root(msg)?)))
    {
        Ok(r) => r,
        Err(e) => return error(&format!("{e:#}")),
    };
    let child_env = crate::ipc::parse_child_env(msg);
    let flow = match parse_flow_open(msg, &project_root) {
        Ok(f) => f,
        Err(e) => return error(&format!("{e:#}")),
    };
    let idle_timeout = msg["idle_timeout_s"]
        .as_u64()
        .map_or(crate::session::DEFAULT_IDLE_TIMEOUT, Duration::from_secs);
    let wait = msg["wait_ms"]
        .as_u64()
        .map_or(DEFAULT_WAIT, Duration::from_millis);
    let stub = cfg!(debug_assertions) && msg["stub"].as_bool() == Some(true) && flow.is_none();
    let session = Arc::new(Session::start(
        OpenRequest {
            query,
            project_root,
            child_env,
            idle_timeout,
            flow,
            boot: msg["boot"].as_bool().unwrap_or(true),
            stub,
        },
        resource_mgr.clone(),
        install_cache.clone(),
        slots.clone(),
    ));
    if let Some(mut slot) = conn_slot {
        *slot = Some(session.clone());
    }
    if let (Some(mut map), Home::Named(_, name)) = (named_map, home) {
        map.insert(name.clone(), session.clone());
    }
    sessions.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    watch_idle(session.clone(), home.clone(), sessions.clone());
    forget_failed_open(session.clone(), home.clone());
    let waited = session.wait(wait).await;
    // An open that failed leaves nothing to keep. `forget_failed_open` can
    // close it first, so this wait may see only the end: the end reason
    // carries the error too.
    if let Waited::Done(Outcome {
        result: OpResult::Failed(e),
        ..
    }) = &waited
    {
        session.close(&format!("the open failed: {e}"), false).await;
        home.forget(&session).await;
    }
    waited_json(waited)
}

/// The flow part of a `session_open`: `flow`, `stop_at`,
/// `break_on_failure`, `teardown` and `vars`.
/// A relative flow path is in the project, as `export_flow`'s is.
fn parse_flow_open(
    msg: &serde_json::Value,
    project_root: &std::path::Path,
) -> anyhow::Result<Option<crate::session::FlowOpen>> {
    let Some(path) = msg["flow"].as_str() else {
        if !msg["stop_at"].is_null() {
            anyhow::bail!("stop_at needs a flow");
        }
        return Ok(None);
    };
    let stop_at = msg["stop_at"]
        .as_str()
        .map(golem_runner::context::StopAt::parse)
        .transpose()?;
    let vars = msg["vars"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str().map_or_else(|| v.to_string(), str::to_string),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(crate::session::FlowOpen {
        path: project_root.join(path),
        stop_at,
        break_on_failure: msg["break_on_failure"].as_bool().unwrap_or(false),
        no_teardown: msg["teardown"].as_bool() == Some(false),
        vars,
        stub: cfg!(debug_assertions) && msg["stub"].as_bool() == Some(true),
    }))
}

/// Close and forget a session whose open fails, however long the open
/// takes: the client may stop waiting first, and a failed named session
/// would hold its name and keep the daemon up.
fn forget_failed_open(session: Arc<Session>, home: Home) {
    tokio::spawn(async move {
        loop {
            match session.wait(Duration::from_secs(3600)).await {
                Waited::Pending(_) => continue,
                Waited::Done(Outcome {
                    op: "session_open",
                    result: OpResult::Failed(e),
                    ..
                }) => {
                    session.close(&format!("the open failed: {e}"), false).await;
                    home.forget(&session).await;
                }
                _ => {}
            }
            break;
        }
    });
}

/// End the session when its idle timeout passes, and tell a connection's
/// client why. Also drops the session count when the session ends for any
/// reason.
fn watch_idle(session: Arc<Session>, home: Home, sessions: Arc<std::sync::atomic::AtomicU64>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if let Status::Ended(_) = session.status() {
                break;
            }
            if session.expired() {
                let reason = format!(
                    "idle timeout after {}m",
                    session.idle_timeout().as_secs() / 60
                );
                session.close(&reason, false).await;
                home.forget(&session).await;
                if let Home::Conn(_, writer) = &home {
                    let notice = serde_json::json!({ "type": "session_ended", "reason": reason });
                    let mut w = writer.lock().await;
                    let _ = w.write_all(format!("{notice}\n").as_bytes()).await;
                }
                break;
            }
        }
        home.forget(&session).await;
        sessions.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    });
}

/// A `session_edit`: `edit` names the change, the other fields carry it.
fn parse_edit(msg: &serde_json::Value) -> anyhow::Result<crate::session::DraftEdit> {
    use crate::session::DraftEdit;
    let text = |k: &str| msg[k].as_str().map(str::to_string);
    let need = |k: &str| text(k).ok_or_else(|| anyhow::anyhow!("this edit needs `{k}`"));
    let object = |k: &str| match &msg[k] {
        serde_json::Value::Object(m) => Ok(m.clone()),
        _ => Err(anyhow::anyhow!("this edit needs `{k}` as an object")),
    };
    Ok(match msg["edit"].as_str().unwrap_or_default() {
        "flow_set" => DraftEdit::FlowSet(crate::draft::FlowSet {
            name: text("name"),
            tags: msg["tags"].as_array().map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(str::to_string))
                    .collect()
            }),
            vars: msg["vars"].as_object().cloned(),
            seed: msg["seed"].as_u64(),
            explicit_only: msg["explicit_only"].as_bool(),
            start: text("start"),
        }),
        "apps_set" => DraftEdit::AppSet(object("app")?),
        "block_begin" => DraftEdit::BlockBegin {
            name: need("name")?,
            next: text("next"),
        },
        "block_link" => DraftEdit::BlockLink {
            block: need("block")?,
            next: text("next"),
            branches: msg["branches"].as_array().cloned().unwrap_or_default(),
        },
        "teardown_add" => DraftEdit::TeardownAdd {
            step: need("step")?,
            comment: text("comment"),
        },
        "data_add" => DraftEdit::DataAdd(object("row")?),
        "comment_add" => DraftEdit::CommentAdd(need("text")?),
        "record_only" => DraftEdit::RecordOnly {
            step: need("step")?,
            comment: text("comment"),
        },
        other => anyhow::bail!("unknown draft edit: {other:?}"),
    })
}

fn error(message: &str) -> serde_json::Value {
    serde_json::json!({ "status": "error", "message": message })
}

fn ended_json(reason: &str) -> serde_json::Value {
    serde_json::json!({ "status": "ended", "reason": reason })
}

fn running_json(status: &str, r: &Running) -> serde_json::Value {
    serde_json::json!({
        "status": status,
        "op_id": r.op_id,
        "op": r.op,
        "phase": r.phase,
        "elapsed_ms": r.started.elapsed().as_millis() as u64,
    })
}

fn waited_json(w: Waited) -> serde_json::Value {
    match w {
        Waited::Done(o) => outcome_json(&o),
        Waited::Pending(r) => running_json("pending", &r),
        Waited::Nothing => serde_json::json!({ "status": "idle" }),
        Waited::Ended(reason) => ended_json(&reason),
    }
}

/// The named sessions: each one's name, device and state.
async fn list_json(named: &Named) -> serde_json::Value {
    let map = named.map.lock().await;
    let sessions: Vec<serde_json::Value> = map
        .iter()
        .map(|(name, session)| {
            let mut entry = status_json(&session.status());
            entry["name"] = serde_json::json!(name);
            entry["device"] = serde_json::json!(session.device());
            entry
        })
        .collect();
    serde_json::json!({ "status": "ok", "sessions": sessions })
}

fn status_json(s: &Status) -> serde_json::Value {
    match s {
        Status::Idle { since, last } => serde_json::json!({
            "status": "idle",
            "idle_ms": since.elapsed().as_millis() as u64,
            "last": last.as_ref().map(outcome_json),
        }),
        Status::Busy(r) => running_json("busy", r),
        Status::Ended(reason) => ended_json(reason),
    }
}

pub(crate) fn outcome_json(o: &Outcome) -> serde_json::Value {
    let result = match &o.result {
        OpResult::Act {
            passed,
            toon,
            step,
            tree,
        } => serde_json::json!({ "passed": passed, "toon": toon, "step": step, "tree": tree }),
        OpResult::Tree(text) => serde_json::json!({ "tree": text }),
        OpResult::Probe { toon, json } => serde_json::json!({ "toon": toon, "report": json }),
        OpResult::Screenshot { png } => serde_json::json!({
            "png_base64": golem_driver::ime::base64_encode(png),
        }),
        OpResult::Opened {
            device,
            udid,
            bundle,
            flow,
        } => serde_json::json!({
            "opened": true,
            "device": device,
            "udid": udid,
            "bundle": bundle,
            "flow": flow,
        }),
        OpResult::Draft(text) => serde_json::json!({ "draft": text }),
        OpResult::Edited(text) => serde_json::json!({ "edited": text }),
        OpResult::Exported {
            path,
            steps,
            counts,
            unverified,
        } => serde_json::json!({
            "exported": path.display().to_string(),
            "steps": steps,
            "counts": counts,
            "unverified": unverified,
        }),
        OpResult::Failed(message) => serde_json::json!({ "error": message }),
        OpResult::Cancelled => serde_json::json!({ "cancelled": true }),
    };
    serde_json::json!({
        "status": "done",
        "op_id": o.op_id,
        "op": o.op,
        "result": result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    struct Daemon {
        server: crate::ipc::OrchestratorServer,
        socket: std::path::PathBuf,
        dir: tempfile::TempDir,
    }

    async fn daemon() -> Daemon {
        let dir = tempfile::Builder::new()
            .prefix("gsess")
            .tempdir_in("/tmp")
            .expect("tempdir");
        let socket = dir.path().join("d.sock");
        let server = crate::ipc::start_server(&socket, &crate::ipc::Identity::current())
            .await
            .expect("daemon");
        Daemon {
            server,
            socket,
            dir,
        }
    }

    impl Daemon {
        /// A project whose `golem.toml` does not parse: an open there
        /// fails before it looks for a device.
        fn broken_project(&self) -> std::path::PathBuf {
            let root = self.dir.path().join("broken");
            std::fs::create_dir_all(&root).expect("dir");
            std::fs::write(root.join("golem.toml"), "[[apps]\n").expect("golem.toml");
            root
        }

        /// One message on a connection of its own, as each `golem session`
        /// command sends.
        async fn call(&self, kind: &str, mut msg: serde_json::Value) -> serde_json::Value {
            let mut stream = crate::ipc::attach(
                &self.socket,
                &crate::ipc::Identity::current(),
                Duration::from_secs(5),
            )
            .await
            .expect("attach");
            msg["type"] = serde_json::json!(kind);
            msg["id"] = serde_json::json!(1);
            stream
                .write_all(format!("{msg}\n").as_bytes())
                .await
                .expect("send");
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .expect("reply");
            serde_json::from_str(&line).expect("json reply")
        }

        async fn open(&self, name: &str) -> serde_json::Value {
            self.call(
                "session_open",
                serde_json::json!({
                    "session": name,
                    "stub": true,
                    "project_root": self.dir.path().display().to_string(),
                }),
            )
            .await
        }
    }

    #[tokio::test]
    async fn a_named_session_outlives_each_connection() {
        let d = daemon().await;
        assert_eq!(d.open("a").await["result"]["opened"], true);
        let read = d
            .call(
                "session_act",
                serde_json::json!({ "session": "a", "step": r#"{ action = "read", on_text = "Submit", save_to = "label" }"# }),
            )
            .await;
        assert_eq!(read["result"]["passed"], true, "{read}");
        let assert = d
            .call(
                "session_act",
                serde_json::json!({ "session": "a", "step": r#"{ action = "assert_visible", on_text = "${label}" }"# }),
            )
            .await;
        assert_eq!(
            assert["result"]["passed"], true,
            "a var SHALL carry over between connections: {assert}"
        );
        let closed = d
            .call("session_close", serde_json::json!({ "session": "a" }))
            .await;
        assert_eq!(closed["status"], "closed", "{closed}");
        let gone = d
            .call("session_tree", serde_json::json!({ "session": "a" }))
            .await;
        assert!(
            gone["message"]
                .as_str()
                .is_some_and(|m| m.contains("no session named \"a\"")),
            "{gone}"
        );
    }

    #[tokio::test]
    async fn every_session_message_works_by_name() {
        let d = daemon().await;
        d.open("default").await;
        let named = |extra: serde_json::Value| {
            let mut msg = serde_json::json!({ "session": "default" });
            if let (Some(m), serde_json::Value::Object(e)) = (msg.as_object_mut(), extra) {
                m.extend(e);
            }
            msg
        };
        let tree = d.call("session_tree", named(serde_json::json!({}))).await;
        assert!(
            tree["result"]["tree"]
                .as_str()
                .is_some_and(|t| t.starts_with("tree visible")),
            "{tree}"
        );
        let probe = d
            .call(
                "session_probe",
                named(serde_json::json!({ "selector": r#"{ on_text = "Submit" }"# })),
            )
            .await;
        assert!(probe["result"]["toon"].is_string(), "{probe}");
        let shot = d
            .call("session_screenshot", named(serde_json::json!({})))
            .await;
        assert!(shot["result"]["png_base64"].is_string(), "{shot}");
        let png = d.dir.path().join("shot.png");
        let saved = d
            .call(
                "session_screenshot",
                named(serde_json::json!({ "path": png.display().to_string() })),
            )
            .await;
        assert_eq!(saved["result"]["bytes"], 4, "{saved}");
        assert_eq!(std::fs::read(&png).expect("png"), [0x89, 0x50, 0x4E, 0x47]);
        let status = d.call("session_status", named(serde_json::json!({}))).await;
        assert_eq!(status["status"], "idle", "{status}");
        let logs = d.call("session_logs", named(serde_json::json!({}))).await;
        assert!(logs["logs"].is_string(), "{logs}");
        let draft = d
            .call("session_draft_show", named(serde_json::json!({})))
            .await;
        assert!(draft["result"]["draft"].is_string(), "{draft}");
        let edit = d
            .call(
                "session_edit",
                named(serde_json::json!({ "edit": "flow_set", "name": "Renamed" })),
            )
            .await;
        assert!(
            edit["result"]["edited"].is_string(),
            "a draft edit's own `name` SHALL not pick the session: {edit}"
        );
    }

    #[tokio::test]
    async fn a_name_is_open_once_and_the_list_shows_each() {
        let d = daemon().await;
        d.open("a").await;
        let again = d.open("a").await;
        assert!(
            again["message"]
                .as_str()
                .is_some_and(|m| m.contains("already open")),
            "{again}"
        );
        assert_eq!(d.open("b").await["result"]["opened"], true);
        let list = d.call("session_list", serde_json::json!({})).await;
        let sessions = list["sessions"].as_array().expect("sessions");
        let names: Vec<&str> = sessions.iter().filter_map(|s| s["name"].as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(sessions[0]["device"], "android/Stub Device");
        assert_eq!(sessions[0]["status"], "idle");
    }

    #[tokio::test]
    async fn an_open_that_fails_after_the_client_stops_waiting_frees_its_name() {
        let d = daemon().await;
        let open = || {
            d.call(
                "session_open",
                serde_json::json!({
                    "session": "x",
                    "wait_ms": 0,
                    "project_root": d.broken_project().display().to_string(),
                }),
            )
        };
        let first = open().await;
        assert_eq!(first["status"], "pending", "{first}");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let list = d.call("session_list", serde_json::json!({})).await;
            if list["sessions"].as_array().is_some_and(|s| s.is_empty()) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the failed open SHALL leave the list: {list}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let again = open().await;
        assert!(
            !again["message"]
                .as_str()
                .is_some_and(|m| m.contains("already open")),
            "{again}"
        );
    }

    #[test]
    fn a_relative_flow_is_in_the_project_and_an_absolute_one_stays() {
        let root = std::path::Path::new("/work/app");
        let open = |flow: &str| {
            parse_flow_open(&serde_json::json!({ "flow": flow }), root)
                .expect("parse")
                .expect("flow")
                .path
        };
        assert_eq!(
            open("e2e/tap.test.toml"),
            std::path::Path::new("/work/app/e2e/tap.test.toml")
        );
        assert_eq!(
            open("/elsewhere/x.test.toml"),
            std::path::Path::new("/elsewhere/x.test.toml")
        );
    }

    #[tokio::test]
    async fn a_failed_open_answers_with_its_error_however_it_races_the_cleanup() {
        let d = daemon().await;
        for _ in 0..20 {
            let reply = d
                .call(
                    "session_open",
                    serde_json::json!({
                        "wait_ms": 5000,
                        "project_root": d.broken_project().display().to_string(),
                    }),
                )
                .await;
            let why = reply["result"]["error"]
                .as_str()
                .or(reply["reason"].as_str())
                .unwrap_or_default();
            let error = why.strip_prefix("the open failed: ").unwrap_or(why);
            assert!(
                !error.is_empty() && error != "the open failed",
                "the reply SHALL carry the open's error: {reply}"
            );
        }
    }

    #[tokio::test]
    async fn the_daemon_is_not_idle_while_a_named_session_is_open() {
        let d = daemon().await;
        d.open("a").await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            d.server.idle_for(),
            Duration::ZERO,
            "an open named session SHALL keep the daemon up with no client connected"
        );
        d.call("session_close", serde_json::json!({ "session": "a" }))
            .await;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while d.server.idle_for() == Duration::ZERO {
            assert!(
                std::time::Instant::now() < deadline,
                "the daemon SHALL go idle once the session closes"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
