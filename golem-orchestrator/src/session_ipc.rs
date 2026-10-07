//! The `session_*` messages on a daemon connection.
//!
//! A connection holds at most one session, and the session ends with the
//! connection. Each message carries an `id` that its reply echoes, and
//! each runs in its own task: `wait`, `status` and `cancel` must answer
//! while an operation runs, and a client may send several calls at once.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use crate::session::{Begin, Op, OpResult, OpenRequest, Outcome, Running, Session, Status, Waited};

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

/// Answer one `session_*` message in its own task.
pub(crate) fn spawn(
    msg: serde_json::Value,
    conn: ConnSession,
    resource_mgr: Arc<golem_devices::resource_manager::ResourceManager>,
    install_cache: golem_runner::installer::InstallCache,
    sessions: Arc<std::sync::atomic::AtomicU64>,
    writer: Writer,
) {
    tokio::spawn(async move {
        let id = msg["id"].clone();
        let mut reply = handle(
            &msg,
            &conn,
            &resource_mgr,
            &install_cache,
            &sessions,
            &writer,
        )
        .await;
        reply["type"] = serde_json::json!("session_reply");
        reply["id"] = id;
        let mut w = writer.lock().await;
        let _ = w.write_all(format!("{reply}\n").as_bytes()).await;
    });
}

async fn handle(
    msg: &serde_json::Value,
    conn: &ConnSession,
    resource_mgr: &Arc<golem_devices::resource_manager::ResourceManager>,
    install_cache: &golem_runner::installer::InstallCache,
    sessions: &Arc<std::sync::atomic::AtomicU64>,
    writer: &Writer,
) -> serde_json::Value {
    let kind = msg["type"].as_str().unwrap_or_default();
    if kind == "session_open" {
        return open(msg, conn, resource_mgr, install_cache, sessions, writer).await;
    }
    let Some(session) = conn.slot.lock().await.clone() else {
        return error("no session is open on this connection; call session_open first");
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
            Begin::Started(_) => waited_json(session.wait(wait).await),
            Begin::Busy(running) => running_json("busy", &running),
            Begin::Ended(reason) => ended_json(&reason),
        };
    }
    match kind {
        "session_wait" => waited_json(session.wait(wait).await),
        "session_status" => status_json(&session.status()),
        "session_cancel" => {
            let cancelled = session.cancel().await;
            serde_json::json!({ "status": "ok", "cancelled": cancelled })
        }
        "session_close" => {
            let teardown = msg["teardown"].as_bool().unwrap_or(true);
            let notes = session.close("closed", teardown).await;
            *conn.slot.lock().await = None;
            serde_json::json!({ "status": "closed", "teardown": notes })
        }
        other => error(&format!("unknown session message: {other}")),
    }
}

async fn open(
    msg: &serde_json::Value,
    conn: &ConnSession,
    resource_mgr: &Arc<golem_devices::resource_manager::ResourceManager>,
    install_cache: &golem_runner::installer::InstallCache,
    sessions: &Arc<std::sync::atomic::AtomicU64>,
    writer: &Writer,
) -> serde_json::Value {
    let mut slot = conn.slot.lock().await;
    if slot.is_some() {
        return error("a session is already open on this connection; close it first");
    }
    let req = match crate::interactive::parse_do_request(&serde_json::json!({
        "step": "",
        "query": msg["query"],
        "project_root": msg["project_root"],
        "client_env": msg["client_env"],
        "client_cwd": msg["client_cwd"],
    })) {
        Ok(r) => r,
        Err(e) => return error(&format!("{e:#}")),
    };
    let flow = match parse_flow_open(msg) {
        Ok(f) => f,
        Err(e) => return error(&format!("{e:#}")),
    };
    let idle_timeout = msg["idle_timeout_s"]
        .as_u64()
        .map_or(crate::session::DEFAULT_IDLE_TIMEOUT, Duration::from_secs);
    let wait = msg["wait_ms"]
        .as_u64()
        .map_or(DEFAULT_WAIT, Duration::from_millis);
    #[cfg(debug_assertions)]
    let stub = msg["stub"].as_bool() == Some(true) && flow.is_none();
    #[cfg(not(debug_assertions))]
    let stub = false;
    let session = Arc::new(if stub {
        #[cfg(debug_assertions)]
        {
            stub_session(req.project_root, idle_timeout)
        }
        #[cfg(not(debug_assertions))]
        unreachable!("stub sessions exist in debug builds only")
    } else {
        Session::start(
            OpenRequest {
                query: req.query,
                project_root: req.project_root,
                child_env: req.child_env,
                idle_timeout,
                flow,
            },
            resource_mgr.clone(),
            install_cache.clone(),
        )
    });
    *slot = Some(session.clone());
    drop(slot);
    sessions.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    watch_idle(
        session.clone(),
        conn.clone(),
        sessions.clone(),
        writer.clone(),
    );
    if stub {
        return opened_json(idle_timeout);
    }
    let waited = session.wait(wait).await;
    // An open that failed leaves nothing to keep.
    if let Waited::Done(Outcome {
        result: OpResult::Failed(_),
        ..
    }) = &waited
    {
        session.close("the open failed", false).await;
        let mut slot = conn.slot.lock().await;
        if slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &session)) {
            *slot = None;
        }
    }
    waited_json(waited)
}

/// The flow part of a `session_open`: `flow`, `stop_at`,
/// `break_on_failure`, `teardown` and `vars`.
fn parse_flow_open(msg: &serde_json::Value) -> anyhow::Result<Option<crate::session::FlowOpen>> {
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
        path: std::path::PathBuf::from(path),
        stop_at,
        break_on_failure: msg["break_on_failure"].as_bool().unwrap_or(false),
        no_teardown: msg["teardown"].as_bool() == Some(false),
        vars,
        stub: cfg!(debug_assertions) && msg["stub"].as_bool() == Some(true),
    }))
}

fn opened_json(idle_timeout: Duration) -> serde_json::Value {
    serde_json::json!({
        "status": "done",
        "op_id": 0,
        "op": "session_open",
        "result": {
            "opened": true,
            "device": "android/Stub Device",
            "udid": "stub-session",
            "bundle": golem_driver::stub::STUB_BUNDLE_ID,
            "idle_timeout_s": idle_timeout.as_secs(),
        },
    })
}

/// A session on the device-free stub driver, for the integration tests of
/// session clients. Debug builds only, like the `--stub` run path.
#[cfg(debug_assertions)]
fn stub_session(project_root: std::path::PathBuf, idle_timeout: Duration) -> Session {
    let device = golem_devices::DeviceInfo {
        name: "Stub Device".into(),
        udid: "stub-session".into(),
        platform: golem_devices::Platform::Android,
        device_type: golem_devices::DeviceType::Phone,
        os_major: 0,
        os_version: "0".into(),
        state: golem_devices::DeviceState::Booted,
        physical: false,
        playstore: false,
        screen_width: None,
        screen_height: None,
        screen_scale: None,
        last_booted: None,
        runtime_id: None,
        device_type_id: None,
    };
    Session::from_parts(
        crate::session::Parts {
            device,
            bundle: golem_driver::stub::STUB_BUNDLE_ID.into(),
            driver: Arc::new(golem_driver::stub::StubDriver::new(1, Default::default())),
            lease: None,
            project_root,
            apps: Vec::new(),
            child_env: None,
        },
        idle_timeout,
    )
}

/// End the session when its idle timeout passes, and tell the client why.
/// Also drops the session count when the session ends for any reason.
fn watch_idle(
    session: Arc<Session>,
    conn: ConnSession,
    sessions: Arc<std::sync::atomic::AtomicU64>,
    writer: Writer,
) {
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
                let mut slot = conn.slot.lock().await;
                if slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &session)) {
                    *slot = None;
                }
                drop(slot);
                let notice = serde_json::json!({ "type": "session_ended", "reason": reason });
                let mut w = writer.lock().await;
                let _ = w.write_all(format!("{notice}\n").as_bytes()).await;
                break;
            }
        }
        sessions.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    });
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
        OpResult::Exported {
            path,
            steps,
            unverified,
        } => serde_json::json!({
            "exported": path.display().to_string(),
            "steps": steps,
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
