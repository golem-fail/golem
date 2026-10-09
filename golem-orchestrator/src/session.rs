//! An interactive session: one device and one driver that stay alive
//! across many operations, for an LLM or a person working step by step.
//!
//! A session holds a lease on its device, so no suite run takes the device
//! while the session is open. It keeps what separate one-step commands
//! would lose between calls: the driver (with its WebView inspector connection and IME state), the
//! variables, the step counter and the seeded RNG.
//!
//! A session runs one operation at a time. While one runs, [`Session::begin`]
//! refuses another with [`Begin::Busy`]; [`Session::wait`],
//! [`Session::status`] and [`Session::cancel`] still work. The outcome of
//! the last operation stays readable after it ends.
//!
//! Who owns a session is up to the caller: the daemon ties one to the
//! connection that opened it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use golem_devices::resource_manager::{DeviceLease, ResourceManager};
use golem_devices::DeviceInfo;
use golem_driver::PlatformDriver;
use golem_element::toon::{encode_tree, TreeHeader};
use tokio::time::Instant;

use crate::target::{self, TargetQuery};

/// How long a session waits with no operation before it ends.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// What to open a session on.
#[derive(Debug, Clone)]
pub struct OpenRequest {
    pub query: TargetQuery,
    pub project_root: PathBuf,
    pub child_env: Option<golem_common::command::ChildEnv>,
    /// Ends the session after this long with no operation.
    pub idle_timeout: Duration,
    /// Run this flow first, and open the session where it stops.
    pub flow: Option<FlowOpen>,
    /// Boot a shut-down device when no booted device fits.
    pub boot: bool,
    /// Open on the device-free stub driver. Debug builds only, for the
    /// tests of session clients.
    pub stub: bool,
}

/// The daemon-wide cap on the devices that sessions hold.
///
/// Several MCP servers can share one daemon and one host, for example
/// subagents or parallel worktrees. Each session takes a slot before it
/// picks a device, and holds it until the session ends. An open past the
/// cap waits: an LLM handles `pending`, while a refusal makes it retry or
/// give up.
pub struct Slots {
    permits: Arc<tokio::sync::Semaphore>,
    max: usize,
    /// What each held slot is for, by slot id: the device, once picked.
    holders: Arc<Mutex<Vec<(u64, String)>>>,
    next: AtomicU64,
}

/// One held slot, released on drop.
pub struct Slot {
    _permit: tokio::sync::OwnedSemaphorePermit,
    id: u64,
    holders: Arc<Mutex<Vec<(u64, String)>>>,
}

/// The default cap, when `GOLEM_SESSION_MAX_DEVICES` is not set.
pub const DEFAULT_MAX_SESSION_DEVICES: usize = 3;

impl Slots {
    pub fn new(max: usize) -> Slots {
        let max = max.max(1);
        Slots {
            permits: Arc::new(tokio::sync::Semaphore::new(max)),
            max,
            holders: Arc::default(),
            next: AtomicU64::new(1),
        }
    }

    /// The cap from `GOLEM_SESSION_MAX_DEVICES`, else the default.
    pub fn from_env() -> Slots {
        Slots::new(
            std::env::var("GOLEM_SESSION_MAX_DEVICES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_SESSION_DEVICES),
        )
    }

    /// Take a slot, waiting while every slot is held. `waiting` gets the
    /// phase to show, again whenever the holders change.
    pub async fn take(&self, waiting: &(dyn Fn(String) + Send + Sync)) -> Slot {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let permit = loop {
            if let Ok(p) = self.permits.clone().try_acquire_owned() {
                break p;
            }
            waiting(self.waiting_phase());
            tokio::select! {
                p = self.permits.clone().acquire_owned() => {
                    break p.unwrap_or_else(|_| unreachable!("the slot semaphore is never closed"));
                }
                () = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        };
        self.holders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((id, "a session that is opening".into()));
        Slot {
            _permit: permit,
            id,
            holders: self.holders.clone(),
        }
    }

    fn waiting_phase(&self) -> String {
        let holders = self.holders.lock().unwrap_or_else(|e| e.into_inner());
        let names: Vec<&str> = holders.iter().map(|(_, h)| h.as_str()).collect();
        format!(
            "waiting for a device: {} of {} held by sessions ({}); GOLEM_SESSION_MAX_DEVICES sets the cap",
            names.len(),
            self.max,
            names.join(", ")
        )
    }
}

impl Slot {
    /// Name what the slot holds, for the phase of an open that waits.
    fn hold(&self, what: String) {
        if let Some(h) = self
            .holders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter_mut()
            .find(|(id, _)| *id == self.id)
        {
            h.1 = what;
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.holders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(id, _)| *id != self.id);
    }
}

/// A flow to run before the session takes over its device.
#[derive(Debug, Clone)]
pub struct FlowOpen {
    pub path: PathBuf,
    /// Stop before this step; without it, the session opens where the flow
    /// ends.
    pub stop_at: Option<golem_runner::context::StopAt>,
    /// Keep the session open at a failed step. Without it, a failure ends
    /// the flow as `golem run` would, teardown included, and no session
    /// opens.
    pub break_on_failure: bool,
    /// Skip the flow's `[[teardown]]` on every way the session ends.
    pub no_teardown: bool,
    pub vars: Vec<(String, String)>,
    /// Run the flow on the device-free stub driver. Debug builds only, for
    /// the tests.
    pub stub: bool,
}

/// One operation.
#[derive(Debug, Clone)]
pub enum Op {
    /// Run a step, given in the canonical notation; with `tree`, also read
    /// the visible tree after it. A step that passes goes into the draft,
    /// with `comment` above it.
    Act {
        step: String,
        tree: bool,
        comment: Option<String>,
    },
    /// Read the tree: the visible one, or the full one as a hint; as TOON,
    /// or with `json` as the element tree.
    Tree { full: bool, json: bool },
    /// Report what a selector matches, polling up to `timeout_ms`.
    Probe { selector: String, timeout_ms: u64 },
    /// Capture the screen as PNG.
    Screenshot,
    /// The flow draft as TOML.
    DraftShow,
    /// The draft's steps, each with its address and status.
    DraftSteps(crate::draft::StepsQuery),
    /// Run the draft on the device: from the start with `restart`, else
    /// from the cursor; stop before `stop_at` (`block` or `block:step`).
    DraftRun {
        restart: bool,
        stop_at: Option<String>,
    },
    /// Change the draft without touching the device.
    Edit(DraftEdit),
    /// Check the draft and write it to `path`.
    Export { path: PathBuf, overwrite: bool },
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Op::Act { .. } => "act",
            Op::Tree { .. } => "tree",
            Op::Probe { .. } => "probe",
            Op::Screenshot => "screenshot",
            Op::DraftShow => "draft_show",
            Op::DraftSteps(_) => "draft_steps",
            Op::DraftRun { .. } => "draft_run",
            Op::Edit(e) => e.name(),
            Op::Export { .. } => "export_flow",
        }
    }
}

/// A change to the flow draft that does not touch the device.
#[derive(Debug, Clone)]
pub enum DraftEdit {
    FlowSet(crate::draft::FlowSet),
    AppSet(serde_json::Map<String, serde_json::Value>),
    BlockBegin {
        name: String,
        next: Option<String>,
    },
    BlockLink {
        block: String,
        next: Option<String>,
        branches: Vec<serde_json::Value>,
    },
    TeardownAdd {
        step: String,
        comment: Option<String>,
    },
    DataAdd(serde_json::Map<String, serde_json::Value>),
    CommentAdd(String),
    /// A step that does not run, for a path the live session does not take.
    RecordOnly {
        step: String,
        comment: Option<String>,
    },
    /// Replace the step at `at` (`block:step`), its comment, or both.
    StepEdit {
        at: String,
        step: Option<String>,
        comment: Option<String>,
    },
    StepDelete {
        at: String,
    },
    /// Move the step at `from` so that it becomes step `to`.
    StepMove {
        from: String,
        to: String,
    },
    BlockRename {
        name: String,
        to: String,
    },
    BlockDelete {
        name: String,
    },
}

impl DraftEdit {
    fn name(&self) -> &'static str {
        match self {
            DraftEdit::FlowSet(_) => "flow_set",
            DraftEdit::AppSet(_) => "apps_set",
            DraftEdit::BlockBegin { .. } => "block_begin",
            DraftEdit::BlockLink { .. } => "block_link",
            DraftEdit::TeardownAdd { .. } => "teardown_add",
            DraftEdit::DataAdd(_) => "data_add",
            DraftEdit::CommentAdd(_) => "comment_add",
            DraftEdit::RecordOnly { .. } => "record_only",
            DraftEdit::StepEdit { .. } => "step_edit",
            DraftEdit::StepDelete { .. } => "step_delete",
            DraftEdit::StepMove { .. } => "step_move",
            DraftEdit::BlockRename { .. } => "block_rename",
            DraftEdit::BlockDelete { .. } => "block_delete",
        }
    }

    /// Apply the edit. An edit to a step returns the listing around it.
    fn apply(&self, draft: &mut crate::draft::Draft) -> Result<Option<String>> {
        let near =
            |draft: &crate::draft::Draft, (b, i): (usize, usize)| Some(draft.listing_near(b, i));
        let line = |step: &str| golem_parser::inline::parse_step_inline(step).map(|p| p.line);
        match self {
            DraftEdit::StepEdit { at, step, comment } => {
                let step = step.as_deref().map(line).transpose()?;
                let place = draft.step_edit(at, step.as_deref(), comment.as_deref())?;
                return Ok(near(draft, place));
            }
            DraftEdit::StepDelete { at } => {
                let place = draft.step_delete(at)?;
                return Ok(near(draft, place));
            }
            DraftEdit::StepMove { from, to } => {
                let place = draft.step_move(from, to)?;
                return Ok(near(draft, place));
            }
            DraftEdit::BlockRename { name, to } => {
                let b = draft.block_rename(name, to)?;
                return Ok(near(draft, (b, 0)));
            }
            DraftEdit::BlockDelete { name } => {
                draft.block_delete(name)?;
                return Ok(None);
            }
            _ => {}
        }
        self.apply_add(draft).map(|()| None)
    }

    fn apply_add(&self, draft: &mut crate::draft::Draft) -> Result<()> {
        // A step that will not run is still checked as one.
        let line = |step: &str| golem_parser::inline::parse_step_inline(step).map(|p| p.line);
        match self {
            DraftEdit::FlowSet(set) => draft.flow_set(set),
            DraftEdit::AppSet(app) => draft.app_set(app),
            DraftEdit::BlockBegin { name, next } => draft.block_begin(name, next.as_deref()),
            DraftEdit::BlockLink {
                block,
                next,
                branches,
            } => draft.block_link(block, next.as_deref(), branches),
            DraftEdit::TeardownAdd { step, comment } => {
                draft.teardown_add(&line(step)?, comment.as_deref())
            }
            DraftEdit::DataAdd(row) => draft.data_add(row),
            DraftEdit::CommentAdd(text) => draft.comment_add(text),
            DraftEdit::RecordOnly { step, comment } => {
                draft.record_unverified(&line(step)?, comment.as_deref())
            }
            DraftEdit::StepEdit { .. }
            | DraftEdit::StepDelete { .. }
            | DraftEdit::StepMove { .. }
            | DraftEdit::BlockRename { .. }
            | DraftEdit::BlockDelete { .. } => Ok(()),
        }
    }
}

/// What an operation produced.
#[derive(Debug, Clone)]
pub enum OpResult {
    Act {
        passed: bool,
        /// The step's TOON line.
        toon: String,
        /// The step as `results.json` records it.
        step: serde_json::Value,
        tree: Option<String>,
    },
    Tree(String),
    Probe {
        toon: String,
        json: serde_json::Value,
    },
    Screenshot {
        png: Vec<u8>,
    },
    Draft(String),
    /// A draft edit applied; where the next step goes.
    Edited(String),
    Exported {
        path: PathBuf,
        steps: usize,
        /// The count of steps with each status, as text.
        counts: String,
        unverified: Vec<String>,
    },
    /// The session holds its device now.
    Opened {
        device: String,
        udid: String,
        bundle: String,
        /// For a flow open: the flow run's TOON report, and where it stopped.
        flow: Option<String>,
    },
    /// The operation could not run, or the driver failed under it.
    Failed(String),
    Cancelled,
}

/// A finished operation.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub op_id: u64,
    pub op: &'static str,
    pub result: OpResult,
}

/// The operation in progress.
#[derive(Debug, Clone)]
pub struct Running {
    pub op_id: u64,
    pub op: &'static str,
    /// What it is doing now, for example `act tap:on_text="OK"`.
    pub phase: String,
    pub started: Instant,
}

/// What a session is doing.
#[derive(Debug, Clone)]
pub enum Status {
    Idle {
        since: Instant,
        last: Option<Outcome>,
    },
    Busy(Running),
    /// The session ended; operations are refused.
    Ended(String),
}

/// The answer to [`Session::begin`].
#[derive(Debug)]
pub enum Begin {
    Started(u64),
    Busy(Running),
    Ended(String),
}

/// The answer to [`Session::wait`].
#[derive(Debug, Clone)]
pub enum Waited {
    Done(Outcome),
    /// Still running when the wait ran out.
    Pending(Running),
    /// Nothing ran: the session is idle with no outcome to report.
    Nothing,
    Ended(String),
}

/// A step log entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub op_id: u64,
    pub op: &'static str,
    pub summary: String,
    pub at: std::time::SystemTime,
}

/// The state an operation works on. One operation holds it at a time.
struct Work {
    device: DeviceInfo,
    driver: Arc<dyn PlatformDriver>,
    leases: Vec<DeviceLease>,
    /// The flow the session opened from: its teardown runs on close.
    flow: Option<golem_parser::FlowFile>,
    /// The `.test.toml` being written from the steps that pass.
    draft: crate::draft::Draft,
    base_timeout_ms: u64,
    teardown_on_close: bool,
    project_root: PathBuf,
    capture: golem_runner::capture::CaptureConfig,
    apps: Vec<golem_parser::AppConfig>,
    child_env: Option<golem_common::command::ChildEnv>,
    vars: golem_vars::VariableStore,
    step_count: u64,
    rng: golem_vars::seed::FakeRng,
    browser: Arc<tokio::sync::Mutex<golem_runner::browser::BrowserSlot>>,
    log: Vec<LogEntry>,
    /// The session's place under the device cap.
    slot: Option<Slot>,
    /// Restarts the companion when it dies. `None` on the stub driver.
    recovery: Option<Arc<dyn golem_runner::recovery::CompanionRecovery>>,
}

/// Restart a dead companion on the session's device, and point the
/// session's driver at the new one. The suite's `CompanionRecoveryImpl`
/// does the same with the suite's registration server; a session has
/// none, so it starts the companion as `golem tree` does.
struct SessionRecovery {
    device: DeviceInfo,
    driver: Arc<dyn PlatformDriver>,
    /// The app to re-target on iOS, where a new companion drives no app
    /// until one is launched. Empty when the session names none.
    bundle: String,
}

#[async_trait::async_trait]
impl golem_runner::recovery::CompanionRecovery for SessionRecovery {
    async fn restart_and_reconnect(&self) -> Result<()> {
        let (port, _) = crate::suite::start_companion_for_device(&self.device).await?;
        self.driver.reconnect(port);
        // `launch_app` on the iOS companion activates a running app without
        // restarting it. Android's tree covers the whole screen, and its
        // launch could restart the app, so it is left alone.
        if self.device.platform == golem_devices::Platform::Ios && !self.bundle.is_empty() {
            let _ = self.driver.launch_app(&self.bundle).await;
        }
        Ok(())
    }
}

/// An open session.
pub struct Session {
    idle_timeout: Duration,
    status: Arc<Mutex<Status>>,
    changed: Arc<tokio::sync::Notify>,
    /// `None` until the session holds a device.
    work: Arc<tokio::sync::Mutex<Option<Work>>>,
    /// Set with `work`, apart from it: an operation holds `work` while it
    /// runs, and a stuck operation is when the app's log matters most.
    logs: Arc<Mutex<Option<LogSource>>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    next_op: AtomicU64,
}

/// What `app_logs` reads the device log with, and what `device` reports.
#[derive(Clone)]
struct LogSource {
    driver: Arc<dyn PlatformDriver>,
    apps: Vec<golem_parser::AppConfig>,
    opened: SystemTime,
    device: String,
}

impl LogSource {
    fn of(work: &Work, opened: SystemTime) -> LogSource {
        LogSource {
            driver: work.driver.clone(),
            apps: work.apps.clone(),
            opened,
            device: format!("{}/{}", work.device.platform, work.device.name),
        }
    }
}

/// What `app_logs` shows.
#[derive(Debug, Clone, Default)]
pub struct LogsRequest {
    /// Seconds back from now; by default, from when the session opened.
    pub since_secs: Option<u64>,
    /// Only lines whose tag or message contains this, in any case.
    pub filter: Option<String>,
    /// The most lines to show besides crash lines.
    pub limit: Option<usize>,
    /// The app's name or bundle; by default, the session's target app.
    pub app: Option<String>,
}

impl Session {
    /// Start opening a session. The open is the session's first operation,
    /// so `wait`, `status` and `cancel` work while it runs: a flow open can
    /// build, install and boot for minutes.
    pub fn start(
        req: OpenRequest,
        resource_mgr: Arc<ResourceManager>,
        install_cache: golem_runner::installer::InstallCache,
        slots: Arc<Slots>,
    ) -> Session {
        let session = Session::empty(req.idle_timeout);
        let phase = session.status.clone();
        let logs = session.logs.clone();
        let opened = SystemTime::now();
        session.run(
            "session_open",
            match &req.flow {
                Some(f) => format!("opening from {}", f.path.display()),
                None => "opening".to_string(),
            },
            move |slot| {
                Box::pin(async move {
                    open(
                        req,
                        resource_mgr,
                        install_cache,
                        &slots,
                        slot,
                        phase,
                        (logs, opened),
                    )
                    .await
                })
            },
        );
        session
    }

    /// A session on an already-resolved device and driver.
    pub fn from_parts(parts: Parts, idle_timeout: Duration) -> Session {
        let session = Session::empty(idle_timeout);
        let bundle = parts.bundle.clone();
        let work = Work::from_parts(parts);
        let mut source = LogSource::of(&work, SystemTime::now());
        if source.apps.is_empty() && !bundle.is_empty() {
            source.apps.push(bundle_app(&bundle));
        }
        *session.logs.lock().unwrap_or_else(|e| e.into_inner()) = Some(source);
        *session
            .work
            .try_lock()
            .unwrap_or_else(|_| unreachable!("a new session's work is unlocked")) = Some(work);
        session
    }

    fn empty(idle_timeout: Duration) -> Session {
        Session {
            idle_timeout,
            status: Arc::new(Mutex::new(Status::Idle {
                since: Instant::now(),
                last: None,
            })),
            changed: Arc::new(tokio::sync::Notify::new()),
            work: Arc::new(tokio::sync::Mutex::new(None)),
            logs: Arc::new(Mutex::new(None)),
            task: Mutex::new(None),
            next_op: AtomicU64::new(1),
        }
    }

    pub fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    /// Start `op` unless another is running or the session has ended.
    pub fn begin(&self, op: Op) -> Begin {
        let name = op.name();
        let phase = phase_of(&op);
        let status = self.status.clone();
        self.run_checked(name, phase, move |slot| {
            Box::pin(async move {
                let mut slot = slot;
                let Some(work) = slot.as_mut() else {
                    return OpResult::Failed("the session holds no device".into());
                };
                let result = run_op(work, &op, &status).await;
                work.log.push(LogEntry {
                    op_id: 0,
                    op: op.name(),
                    summary: summary_of(&op, &result),
                    at: std::time::SystemTime::now(),
                });
                result
            })
        })
    }

    fn run<F>(&self, name: &'static str, phase: String, body: F) -> u64
    where
        F: FnOnce(tokio::sync::OwnedMutexGuard<Option<Work>>) -> OpFuture + Send + 'static,
    {
        match self.run_checked(name, phase, body) {
            Begin::Started(id) => id,
            _ => unreachable!("a new session is idle"),
        }
    }

    fn run_checked<F>(&self, name: &'static str, phase: String, body: F) -> Begin
    where
        F: FnOnce(tokio::sync::OwnedMutexGuard<Option<Work>>) -> OpFuture + Send + 'static,
    {
        let op_id = {
            let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
            match &*status {
                Status::Busy(running) => return Begin::Busy(running.clone()),
                Status::Ended(reason) => return Begin::Ended(reason.clone()),
                Status::Idle { .. } => {}
            }
            let op_id = self.next_op.fetch_add(1, Ordering::Relaxed);
            *status = Status::Busy(Running {
                op_id,
                op: name,
                phase,
                started: Instant::now(),
            });
            op_id
        };
        let status = self.status.clone();
        let changed = self.changed.clone();
        let work = self.work.clone();
        let task = tokio::spawn(async move {
            let slot = work.lock_owned().await;
            let result = body(slot).await;
            finish(
                &status,
                &changed,
                Outcome {
                    op_id,
                    op: name,
                    result,
                },
            );
        });
        *self.task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
        Begin::Started(op_id)
    }

    /// Wait up to `timeout` for the running operation, or report the last
    /// outcome when none is running.
    pub async fn wait(&self, timeout: Duration) -> Waited {
        let deadline = Instant::now() + timeout;
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match self.status() {
                Status::Idle { last: Some(o), .. } => return Waited::Done(o),
                Status::Idle { last: None, .. } => return Waited::Nothing,
                Status::Ended(reason) => return Waited::Ended(reason),
                Status::Busy(running) => {
                    if Instant::now() >= deadline {
                        return Waited::Pending(running);
                    }
                    // On the deadline, loop to answer with the current phase,
                    // which may have changed without a notify.
                    tokio::select! {
                        () = notified => {}
                        () = tokio::time::sleep_until(deadline) => {}
                    }
                }
            }
        }
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Stop the running operation, if any, and record it as cancelled.
    /// Returns once the operation has stopped, so its driver calls are
    /// over before anything else uses the device.
    pub async fn cancel(&self) -> bool {
        let Status::Busy(running) = self.status() else {
            return false;
        };
        let task = self.task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        finish(
            &self.status,
            &self.changed,
            Outcome {
                op_id: running.op_id,
                op: running.op,
                result: OpResult::Cancelled,
            },
        );
        true
    }

    /// End the session: stop any operation, run the flow's teardown when
    /// `run_teardown` (an explicit close) and the session opened from a flow,
    /// close its browser, then release the device. The device is not shut
    /// down: the daemon does that when it exits.
    pub async fn close(&self, reason: &str, run_teardown: bool) -> Option<String> {
        self.cancel().await;
        {
            let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(&*status, Status::Ended(_)) {
                return None;
            }
            *status = Status::Ended(reason.to_string());
        }
        *self.logs.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.changed.notify_waiters();
        let mut slot = self.work.lock().await;
        let mut notes = None;
        if let Some(work) = slot.as_mut() {
            notes = finish_work(work, run_teardown).await;
            golem_driver::ime::restore(std::slice::from_ref(&work.device.udid)).await;
        }
        *slot = None;
        notes
    }

    /// How long the session has had no operation running. Zero while one
    /// runs, so a long operation never counts toward the idle timeout.
    pub fn idle_for(&self) -> Duration {
        match self.status() {
            Status::Idle { since, .. } => since.elapsed(),
            _ => Duration::ZERO,
        }
    }

    /// Whether the idle timeout has passed.
    pub fn expired(&self) -> bool {
        self.idle_for() >= self.idle_timeout
    }

    /// The device the session holds, as `platform/name`; `None` while it
    /// opens and after it ends.
    pub fn device(&self) -> Option<String> {
        self.logs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|s| s.device.clone())
    }

    /// The app's device log lines as TOON, crash lines first. Runs beside
    /// any operation: it reads the device log from the host, not through
    /// the companion.
    pub async fn app_logs(&self, req: &LogsRequest) -> Result<String> {
        let source = self.logs.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(source) = source else {
            if let Status::Ended(reason) = self.status() {
                anyhow::bail!("the session ended: {reason}");
            }
            anyhow::bail!("the session holds no device yet");
        };
        let bundle = match &req.app {
            None => source.apps.first().and_then(|a| a.bundle.clone()),
            Some(app) => Some(
                source
                    .apps
                    .iter()
                    .find(|a| &a.name == app)
                    .and_then(|a| a.bundle.clone())
                    .unwrap_or_else(|| app.clone()),
            ),
        }
        .filter(|b| !b.is_empty())
        .ok_or_else(|| anyhow::anyhow!("the session targets no app; pass `app`"))?;
        let since = match req.since_secs {
            Some(secs) => SystemTime::now()
                .checked_sub(Duration::from_secs(secs))
                .unwrap_or(SystemTime::UNIX_EPOCH),
            None => source.opened,
        };
        let lines = source.driver.app_logs(&bundle, since).await?;
        let selection = golem_driver::logs::select(
            lines,
            req.filter.as_deref(),
            req.limit.unwrap_or(golem_driver::logs::DEFAULT_LIMIT),
        );
        Ok(golem_driver::logs::render(
            &bundle,
            &selection,
            &chrono::Local,
        ))
    }

    /// The operations so far, oldest first.
    pub async fn log(&self) -> Vec<LogEntry> {
        self.work
            .lock()
            .await
            .as_ref()
            .map(|w| w.log.clone())
            .unwrap_or_default()
    }
}

type OpFuture = std::pin::Pin<Box<dyn std::future::Future<Output = OpResult> + Send>>;

/// What [`Session::from_parts`] takes.
pub struct Parts {
    pub device: DeviceInfo,
    pub bundle: String,
    pub driver: Arc<dyn PlatformDriver>,
    pub lease: Option<DeviceLease>,
    pub project_root: PathBuf,
    pub apps: Vec<golem_parser::AppConfig>,
    pub child_env: Option<golem_common::command::ChildEnv>,
}

/// The parts of a session on the device-free stub driver.
fn stub_parts(project_root: PathBuf) -> Parts {
    let device = DeviceInfo {
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
    Parts {
        device,
        bundle: golem_driver::stub::STUB_BUNDLE_ID.into(),
        driver: Arc::new(golem_driver::stub::StubDriver::new(1, Default::default())),
        lease: None,
        project_root,
        apps: Vec::new(),
        child_env: None,
    }
}

impl Work {
    fn from_parts(parts: Parts) -> Work {
        let mut vars = golem_vars::VariableStore::new();
        vars.push_scope(golem_vars::Scope::new(golem_vars::ScopeLevel::Flow));
        let draft = new_draft(&parts.device, &parts.apps);
        Work {
            capture: capture_for(&parts.project_root),
            device: parts.device,
            driver: parts.driver,
            leases: parts.lease.into_iter().collect(),
            flow: None,
            draft,
            base_timeout_ms: golem_runner::policy::DEFAULT_BASE_TIMEOUT_MS,
            teardown_on_close: false,
            project_root: parts.project_root,
            apps: parts.apps,
            child_env: parts.child_env,
            vars,
            step_count: 0,
            rng: golem_vars::seed::FakeRng::from_optional_seed(None),
            browser: Default::default(),
            log: Vec::new(),
            slot: None,
            recovery: None,
        }
    }
}

/// A new draft for a session on `device`, with the session's first app.
fn new_draft(device: &DeviceInfo, apps: &[golem_parser::AppConfig]) -> crate::draft::Draft {
    let os = format!("{}:latest", device.platform);
    let app = apps
        .first()
        .and_then(|a| a.bundle.as_deref().map(|b| (a.name.as_str(), b)));
    crate::draft::Draft::new("New flow", app.map(|(n, b)| (n, b, os.as_str())))
}

fn capture_for(project_root: &std::path::Path) -> golem_runner::capture::CaptureConfig {
    golem_runner::capture::CaptureConfig {
        screenshot_on_failure: false,
        output_dir: project_root.join(".golem/results"),
        ..Default::default()
    }
}

/// Run the teardown when asked and there is one, then close the browser.
/// Returns the teardown's notes.
async fn finish_work(work: &mut Work, run_teardown: bool) -> Option<String> {
    let Work {
        device,
        driver,
        project_root,
        capture,
        child_env,
        vars,
        step_count,
        browser,
        flow,
        base_timeout_ms,
        teardown_on_close,
        ..
    } = work;
    let empty = golem_parser::FlowFile {
        flow: golem_parser::FlowMeta {
            name: "session".into(),
            start: None,
            seed: None,
            tags: Vec::new(),
            explicit_only: false,
            vars: Default::default(),
            apps: Vec::new(),
            options: None,
        },
        block: Vec::new(),
        data: Vec::new(),
        teardown: Vec::new(),
    };
    let flow = flow.as_ref().unwrap_or(&empty);
    let ctx = golem_runner::context::ExecutionContext {
        device: Some(device),
        child_env: child_env.as_ref(),
        global_step_index: *step_count,
        browser: browser.clone(),
        ..golem_runner::context::ExecutionContext::new(
            project_root,
            project_root,
            capture,
            "session",
        )
    };
    let mut result = Ok(golem_runner::executor::FlowResult {
        success: true,
        warnings: Vec::new(),
        failed_step: None,
        failed_block: None,
        failed_action: None,
        failed_reason: None,
        failed_code: None,
        barrier_aborted: false,
        perf_snapshots: Vec::new(),
        recordings: Vec::new(),
        a11y_audits: Vec::new(),
    });
    let teardown = run_teardown && *teardown_on_close && !flow.teardown.is_empty();
    golem_runner::executor::finish_flow(
        flow,
        driver.as_ref(),
        vars,
        *base_timeout_ms,
        &ctx,
        teardown,
        &mut result,
    )
    .await;
    let warnings = result.map(|r| r.warnings).unwrap_or_default();
    match (teardown, warnings.is_empty()) {
        (false, _) => None,
        (true, true) => Some("teardown ran".into()),
        (true, false) => Some(format!("teardown ran: {}", warnings.join("; "))),
    }
}

/// The session open: select, lease, connect; or for a flow, run it as a
/// one-flow suite that hands its device over.
async fn open(
    req: OpenRequest,
    resource_mgr: Arc<ResourceManager>,
    install_cache: golem_runner::installer::InstallCache,
    slots: &Slots,
    mut slot: tokio::sync::OwnedMutexGuard<Option<Work>>,
    status: Arc<Mutex<Status>>,
    (logs, opened): (Arc<Mutex<Option<LogSource>>>, SystemTime),
) -> OpResult {
    let phase = status.clone();
    let held = slots
        .take(&move |text| {
            if let Ok(mut s) = phase.lock() {
                if let Status::Busy(r) = &mut *s {
                    r.phase = text;
                }
            }
        })
        .await;
    let result = if cfg!(debug_assertions) && req.stub {
        Ok((Work::from_parts(stub_parts(req.project_root.clone())), None))
    } else {
        match req.flow.clone() {
            None => open_device(&req, &resource_mgr, &status).await,
            Some(f) => open_flow(&req, f, resource_mgr, install_cache, &status).await,
        }
    };
    match result {
        Ok((mut work, flow_report)) => {
            let bundle = if req.stub {
                golem_driver::stub::STUB_BUNDLE_ID.to_string()
            } else {
                work.apps
                    .first()
                    .and_then(|a| a.bundle.clone())
                    .unwrap_or_default()
            };
            let out = OpResult::Opened {
                device: format!("{}/{}", work.device.platform, work.device.name),
                udid: work.device.udid.clone(),
                bundle: bundle.clone(),
                flow: flow_report,
            };
            held.hold(format!("{} ({})", work.device.name, work.device.udid));
            work.slot = Some(held);
            if !req.stub {
                work.recovery = Some(Arc::new(SessionRecovery {
                    device: work.device.clone(),
                    driver: work.driver.clone(),
                    bundle: bundle.clone(),
                }));
            }
            let mut source = LogSource::of(&work, opened);
            if source.apps.is_empty() && !bundle.is_empty() {
                source.apps.push(bundle_app(&bundle));
            }
            *logs.lock().unwrap_or_else(|e| e.into_inner()) = Some(source);
            *slot = Some(work);
            out
        }
        Err(e) => OpResult::Failed(format!("{e:#}")),
    }
}

/// Show the boot of `device` as the open's phase.
fn boot_phase(status: &Arc<Mutex<Status>>) -> impl Fn(&golem_devices::DeviceInfo) + Send + Sync {
    let status = status.clone();
    move |device| {
        if let Ok(mut s) = status.lock() {
            if let Status::Busy(r) = &mut *s {
                r.phase = format!("booting {} ({})", device.name, device.udid);
            }
        }
    }
}

async fn open_device(
    req: &OpenRequest,
    resource_mgr: &Arc<ResourceManager>,
    status: &Arc<Mutex<Status>>,
) -> Result<(Work, Option<String>)> {
    let (project, _) = crate::project::ProjectConfig::load_from(&req.project_root)?;
    let selection = target::select_for_session(
        &req.query,
        &project.apps,
        resource_mgr,
        req.boot,
        &boot_phase(status),
    )
    .await?;
    let lease = crate::interactive::lease(resource_mgr, &selection.device)?;
    let target = target::connect(selection).await?;
    let mut apps = crate::interactive::app_configs(&project.apps);
    // The app the session targets first, so its bundle is the one reported.
    if let Some(i) = apps
        .iter()
        .position(|a| a.bundle.as_deref() == Some(target.bundle.as_str()))
    {
        apps.swap(0, i);
    } else if !target.bundle.is_empty() {
        apps.insert(0, bundle_app(&target.bundle));
    }
    let driver: Arc<dyn PlatformDriver> = Arc::from(target.driver());
    Ok((
        Work::from_parts(Parts {
            device: target.device,
            bundle: target.bundle,
            driver,
            lease: Some(lease),
            project_root: req.project_root.clone(),
            apps,
            child_env: req.child_env.clone(),
        }),
        None,
    ))
}

fn bundle_app(bundle: &str) -> golem_parser::AppConfig {
    golem_parser::AppConfig {
        name: bundle.into(),
        bundle: Some(bundle.into()),
        devices: Vec::new(),
        install_script: None,
        install_timeout_ms: None,
        install_env: None,
        profile: None,
        permissions: Default::default(),
    }
}

async fn open_flow(
    req: &OpenRequest,
    f: FlowOpen,
    resource_mgr: Arc<ResourceManager>,
    install_cache: golem_runner::installer::InstallCache,
    status: &Arc<Mutex<Status>>,
) -> Result<(Work, Option<String>)> {
    let path = if f.path.is_absolute() {
        f.path.clone()
    } else {
        req.child_env
            .as_ref()
            .and_then(|e| e.cwd.clone())
            .unwrap_or_else(|| req.project_root.clone())
            .join(&f.path)
    };
    let (project, _) = crate::project::ProjectConfig::load_from(&req.project_root)?;
    let stub = cfg!(debug_assertions) && f.stub;
    let slot = check_flow(
        &path,
        &project.apps,
        &req.project_root,
        if stub {
            Some(
                req.query
                    .platform()
                    .unwrap_or(golem_devices::Platform::Android),
            )
        } else {
            req.query.platform()
        },
        f.stop_at.as_ref(),
    )
    .await?;
    let (platform, pin_udid) = if stub {
        (
            req.query
                .platform()
                .unwrap_or(golem_devices::Platform::Android),
            None,
        )
    } else {
        let selection = target::select_for_session(
            &flow_query(&req.query, &slot),
            &project.apps,
            &resource_mgr,
            req.boot,
            &boot_phase(status),
        )
        .await?;
        (selection.device.platform, Some(selection.device.udid))
    };

    let slot: Arc<std::sync::Mutex<Option<crate::suite::Handoff>>> = Arc::default();
    let config = crate::suite::SuiteConfig {
        platform: Some(platform),
        stub_fail_on_runs: stub.then(Vec::new),
        vars: f.vars.clone(),
        output_dir: req.project_root.join(".golem/results"),
        project_root: req.project_root.clone(),
        project_apps: project.apps.clone(),
        device_settings: project.device_settings.clone(),
        no_teardown: f.no_teardown,
        record: false,
        no_record: true,
        trace: false,
        no_results: true,
        stream_human: false,
        child_env: req.child_env.clone().map(Arc::new),
        handoff: Some(crate::suite::HandoffRequest {
            slot: slot.clone(),
            stop_at: f.stop_at.clone(),
            break_on_failure: f.break_on_failure,
            pin_udid,
        }),
        ..crate::suite::SuiteConfig::default()
    };
    let (events, subs) = golem_events::channel::event_channel();
    let mut runner =
        crate::suite::SuiteRunner::with_resource_manager(config, resource_mgr, install_cache);
    runner.event_forwarder = Some(events);
    let passed: Passed = Arc::default();
    let mut phase = tokio::spawn(follow_phase(
        subs.subscribe(),
        status.clone(),
        passed.clone(),
    ));
    drop(subs);
    let report = runner.run_suite(std::slice::from_ref(&path)).await;
    drop(runner);
    // The last step events can still be in the channel: give the follower
    // a moment to read them before it stops.
    let _ = tokio::time::timeout(Duration::from_millis(500), &mut phase).await;
    phase.abort();
    let report = report?;
    // The legend lines (`# …`) explain TOON to a first-time reader; a
    // session client reads many of these.
    let toon = golem_report::output::render(&report, &golem_report::output::OutputFormat::Toon)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with("# "))
        .map(|l| format!("{l}\n"))
        .collect::<String>();
    let handoff = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some(h) = handoff else {
        anyhow::bail!("the flow did not reach the session, so no session opened:\n{toon}");
    };
    let mut summary = toon;
    match (&h.failure, &h.stopped_at) {
        (Some(failure), _) => {
            summary.push_str(&format!("session open at the failed step: {failure}\n"))
        }
        (None, Some(stop)) => summary.push_str(&format!("session open before {stop}\n")),
        (None, None) => summary.push_str("session open at the end of the flow\n"),
    }
    let apps = h.flow.flow.apps.clone();
    let mut work = Work::from_parts(Parts {
        device: h.device,
        bundle: String::new(),
        driver: h.driver,
        lease: None,
        project_root: req.project_root.clone(),
        apps,
        child_env: req.child_env.clone(),
    });
    work.leases = h.leases;
    work.vars = h.vars;
    work.rng = h.rng;
    work.step_count = h.step_count;
    work.browser = h.browser;
    work.base_timeout_ms = h.base_timeout_ms;
    work.teardown_on_close = !f.no_teardown;
    work.flow = Some(h.flow);
    work.draft = crate::draft::Draft::from_file(&path)?;
    let passed = std::mem::take(&mut *passed.lock().unwrap_or_else(|e| e.into_inner()));
    if let Some(flow) = &work.flow {
        work.draft.mark_ran(flow, &passed)?;
    }
    let placed = match (h.stopped_at.as_ref().or(h.failed_at.as_ref()), &work.flow) {
        (Some(at), Some(flow)) => work.draft.place_cursor(flow, &at.block, at.step),
        _ => false,
    };
    // Blocks a mixin added are not in the file; record at the end then.
    if !placed {
        work.draft.insert_at_end();
    }
    Ok((work, Some(summary)))
}

/// The session's query, with the flow's device constraint filling each
/// field the session left open.
fn flow_query(query: &TargetQuery, slot: &crate::plan::DeviceSlot) -> TargetQuery {
    let mut q = query.clone();
    if q.os.is_none() {
        q.os = slot
            .platform
            .map(|p| target::OsQuery::of(p, slot.os_version.clone()));
    }
    q.device_type = q.device_type.or(slot.device_type);
    if q.device.is_none() {
        q.device = slot.name.clone();
    }
    q
}

/// Refuse a flow that would not run as one session on one device, and a
/// `stop_at` it does not have, before any device work. Returns the flow's
/// one device slot.
async fn check_flow(
    path: &std::path::Path,
    apps: &[golem_parser::ProjectAppConfig],
    project_root: &std::path::Path,
    platform: Option<golem_devices::Platform>,
    stop_at: Option<&golem_runner::context::StopAt>,
) -> Result<crate::plan::DeviceSlot> {
    let planned = crate::plan::plan(
        &[path.to_path_buf()],
        apps,
        project_root,
        platform,
        None,
        1,
        None,
        false,
    )
    .await?;
    if let Some(failure) = planned.parse_failures.first() {
        anyhow::bail!("{}: {}", failure.path.display(), failure.error);
    }
    let slot = match planned.flow_runs.as_slice() {
        [run] if run.slots.len() == 1 => run.slots[0].clone(),
        [run] => anyhow::bail!(
            "the flow drives {} devices at once; a session holds one",
            run.slots.len()
        ),
        runs if platform.is_none() => anyhow::bail!(
            "the flow expands to {} runs; set os (for example os = \"ios\") so that it runs once on one device",
            runs.len()
        ),
        runs => anyhow::bail!(
            "the flow expands to {} runs on {}; a session runs it once on one device",
            runs.len(),
            platform.map_or_else(String::new, |p| p.to_string())
        ),
    };
    let Some(stop) = stop_at else { return Ok(slot) };
    let flow = &planned.flows[0].flow;
    let Some(block) = flow
        .block
        .iter()
        .find(|b| b.name.as_deref() == Some(stop.block.as_str()))
    else {
        let names: Vec<&str> = flow
            .block
            .iter()
            .filter_map(|b| b.name.as_deref())
            .collect();
        anyhow::bail!(
            "stop_at {stop}: no block named {:?}; blocks: {}",
            stop.block,
            names.join(", ")
        );
    };
    if block.for_each.is_some() {
        anyhow::bail!("stop_at {stop}: {:?} is a for_each block, whose row values would not reach the session", stop.block);
    }
    if stop.step > block.steps.len() {
        anyhow::bail!(
            "stop_at {stop}: block {:?} has {} step(s)",
            stop.block,
            block.steps.len()
        );
    }
    Ok(slot)
}

/// The steps a flow run passed, as (block label, 0-based step).
type Passed = Arc<std::sync::Mutex<Vec<(String, usize)>>>;

/// Keep the open's phase current from the flow run's events, and collect
/// the steps that pass.
async fn follow_phase(
    mut events: tokio::sync::broadcast::Receiver<golem_events::Event>,
    status: Arc<Mutex<Status>>,
    passed: Passed,
) {
    let mut running: Option<(String, usize)> = None;
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };
        if let golem_events::EventKind::StepFinished { outcome, .. } = &event.kind {
            let ok = matches!(
                outcome,
                golem_events::StepOutcome::Success | golem_events::StepOutcome::Warning { .. }
            );
            if let (true, Some(step)) = (ok, running.take()) {
                passed.lock().unwrap_or_else(|e| e.into_inner()).push(step);
            }
            continue;
        }
        if let golem_events::EventKind::StepStarted {
            block_name,
            step_index_in_block,
            ..
        } = &event.kind
        {
            running = Some((block_name.clone(), *step_index_in_block));
        }
        let phase = match &event.kind {
            golem_events::EventKind::InstallStarted { app_name, .. } => {
                format!("installing {app_name}")
            }
            golem_events::EventKind::StepStarted {
                block_name,
                step_index_in_block,
                action,
                ..
            } => format!(
                "running flow, {block_name}:{} {action}",
                step_index_in_block + 1
            ),
            _ => continue,
        };
        if let Ok(mut s) = status.lock() {
            if let Status::Busy(r) = &mut *s {
                r.phase = phase;
            }
        }
    }
}

fn finish(status: &Mutex<Status>, changed: &tokio::sync::Notify, outcome: Outcome) {
    {
        let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
        // A cancel or a close got here first.
        if !matches!(&*status, Status::Busy(r) if r.op_id == outcome.op_id) {
            return;
        }
        *status = Status::Idle {
            since: Instant::now(),
            last: Some(outcome),
        };
    }
    changed.notify_waiters();
}

fn phase_of(op: &Op) -> String {
    match op {
        Op::Act { step, .. } => match golem_parser::inline::parse_step_inline(step) {
            Ok(parsed) => format!("act {}", parsed.line),
            Err(_) => "act".to_string(),
        },
        Op::Tree { full, .. } => format!("tree{}", if *full { " (full)" } else { "" }),
        Op::Probe { selector, .. } => format!("probe {selector}"),
        Op::Screenshot => "screenshot".to_string(),
        Op::DraftShow => "draft_show".to_string(),
        Op::DraftSteps(_) => "draft_steps".to_string(),
        Op::DraftRun { restart, .. } => {
            format!(
                "draft_run{}",
                if *restart {
                    " from the start"
                } else {
                    " from the cursor"
                }
            )
        }
        Op::Edit(e) => e.name().to_string(),
        Op::Export { path, .. } => format!("export_flow {}", path.display()),
    }
}

fn summary_of(op: &Op, result: &OpResult) -> String {
    match result {
        OpResult::Act { toon, .. } => toon.clone(),
        OpResult::Failed(e) => format!("{} failed: {e}", op.name()),
        OpResult::Cancelled => format!("{} cancelled", op.name()),
        _ => op.name().to_string(),
    }
}

async fn run_op(work: &mut Work, op: &Op, status: &Arc<Mutex<Status>>) -> OpResult {
    match op {
        Op::DraftRun { restart, stop_at } => draft_run(work, *restart, stop_at.as_deref(), status)
            .await
            .unwrap_or_else(|e| OpResult::Failed(format!("{e:#}"))),
        Op::Act {
            step,
            tree,
            comment,
        } => act(work, step, *tree, comment.as_deref()).await,
        Op::DraftShow => OpResult::Draft(work.draft.text()),
        Op::DraftSteps(q) => match work.draft.steps(q) {
            Ok(text) => OpResult::Draft(text),
            Err(e) => OpResult::Failed(format!("{e:#}")),
        },
        Op::Edit(edit) => match edit.apply(&mut work.draft) {
            Ok(None) => OpResult::Edited(format!(
                "draft updated · next step goes at {}",
                work.draft.describe_insertion()
            )),
            Ok(Some(listing)) => OpResult::Edited(format!("draft updated\n{listing}")),
            Err(e) => OpResult::Failed(format!("{e:#}")),
        },
        Op::Export { path, overwrite } => {
            let path = if path.is_absolute() {
                path.clone()
            } else {
                work.child_env
                    .as_ref()
                    .and_then(|e| e.cwd.clone())
                    .unwrap_or_else(|| work.project_root.clone())
                    .join(path)
            };
            match work.draft.export(&path, *overwrite) {
                Ok(done) => OpResult::Exported {
                    path: done.path,
                    steps: done.steps,
                    counts: done.counts.to_string(),
                    unverified: done.unverified,
                },
                Err(e) => OpResult::Failed(format!("{e:#}")),
            }
        }
        Op::Tree { .. } | Op::Probe { .. } | Op::Screenshot => {
            let first = read(work, op).await;
            let result = match (first, &work.recovery) {
                (Err(e), Some(recovery)) if golem_runner::recovery::is_companion_death_err(&e) => {
                    if let Err(restart) = recovery.restart_and_reconnect().await {
                        return OpResult::Failed(format!(
                            "{e:#}; restarting the companion failed: {restart:#}"
                        ));
                    }
                    read(work, op).await.map(|done| match done {
                        OpResult::Tree(text) => OpResult::Tree(format!("{RESTARTED}\n{text}")),
                        OpResult::Probe { toon, json } => OpResult::Probe {
                            toon: format!("{RESTARTED}\n{toon}"),
                            json,
                        },
                        other => other,
                    })
                }
                (result, _) => result,
            };
            result.unwrap_or_else(|e| OpResult::Failed(format!("{e:#}")))
        }
    }
}

/// The first line of a read that restarted a dead companion.
const RESTARTED: &str = "companion restarted: it had stopped answering";

/// A read-only operation: the tree, a probe or a screenshot.
async fn read(work: &Work, op: &Op) -> Result<OpResult> {
    match op {
        Op::Tree { full, json } => Ok(OpResult::Tree(
            read_tree(work.driver.as_ref(), *full, *json).await?,
        )),
        Op::Probe {
            selector,
            timeout_ms,
        } => {
            let parsed = golem_parser::inline::parse_selector_inline(selector)?;
            let report = golem_runner::probe::probe(
                work.driver.as_ref(),
                &parsed.step,
                &parsed.line,
                *timeout_ms,
            )
            .await?;
            Ok(OpResult::Probe {
                toon: golem_runner::probe::render_toon(&report),
                json: golem_runner::probe::render_json(&report),
            })
        }
        Op::Screenshot => Ok(OpResult::Screenshot {
            png: work.driver.screenshot().await?.data,
        }),
        _ => unreachable!("read() takes only read-only operations"),
    }
}

async fn act(work: &mut Work, step: &str, tree: bool, comment: Option<&str>) -> OpResult {
    let (step, line) = match golem_parser::inline::parse_step_inline(step) {
        Ok(parsed) => (parsed.step, parsed.line),
        Err(e) => return OpResult::Failed(format!("{e:#}")),
    };
    let timeout_ms = golem_runner::policy::effective_timeout(&step, work.base_timeout_ms);
    // Read before the step runs: after a tap the element may be gone.
    let text_alternative = if step.on_accessibility_label.is_some() {
        match work.driver.get_hierarchy().await {
            Ok((root, meta)) => {
                golem_runner::probe::text_alternative(&root, meta.keyboard_height, &step)
            }
            Err(_) => None,
        }
    } else {
        None
    };
    let Work {
        device,
        driver,
        project_root,
        capture,
        apps,
        child_env,
        vars,
        step_count,
        rng,
        browser,
        recovery,
        base_timeout_ms,
        ..
    } = work;
    // The context lives for one step; what must outlast it (step counter,
    // RNG, browser) moves in and back out.
    let mut ctx = golem_runner::context::ExecutionContext {
        device: Some(device),
        child_env: child_env.as_ref(),
        global_step_index: *step_count,
        rng: std::sync::Mutex::new(std::mem::replace(
            rng,
            golem_vars::seed::FakeRng::from_optional_seed(None),
        )),
        browser: browser.clone(),
        recovery: recovery.as_deref(),
        ..golem_runner::context::ExecutionContext::new(
            project_root,
            project_root,
            capture,
            "session",
        )
    };
    let report = golem_runner::single_step::execute_single_step(
        &step,
        driver.as_ref(),
        vars,
        &mut ctx,
        apps,
        *base_timeout_ms,
    )
    .await;
    *step_count = ctx.global_step_index;
    *rng = ctx.rng.into_inner().unwrap_or_else(|e| e.into_inner());
    let tree = if tree {
        read_tree(driver.as_ref(), false, false).await.ok()
    } else {
        None
    };
    let passed = !matches!(report.outcome, golem_report::StepOutcome::Failed { .. });
    // Only a step that passed belongs in the flow; it goes in as written,
    // so a `${var}` stays a reference.
    if passed {
        if let Err(e) = work.draft.record(&line, comment) {
            return OpResult::Failed(format!("the step passed, but the draft refused it: {e:#}"));
        }
    }
    let mut toon = golem_report::toon::format_step_toon(&report)
        .trim_start()
        .to_string();
    let notes = [
        passed
            .then(|| slow_warning(report.duration_ms, timeout_ms))
            .flatten(),
        passed
            .then(|| text_alternative.map(|text| label_hint(&text)))
            .flatten(),
        fix_line(&report.outcome),
    ];
    for note in notes.into_iter().flatten() {
        if !toon.ends_with('\n') {
            toon.push('\n');
        }
        toon.push_str(&note);
        toon.push('\n');
    }
    OpResult::Act {
        passed,
        toon,
        step: golem_report::json::step_json(&report),
        tree,
    }
}

/// Run the draft on the session's device, as `golem run` runs a flow but
/// without its setup or teardown. The steps that pass become passed, and
/// the cursor goes where the run stops or fails.
async fn draft_run(
    work: &mut Work,
    restart: bool,
    stop_at: Option<&str>,
    status: &Arc<Mutex<Status>>,
) -> Result<OpResult> {
    let mut flow = golem_parser::parse_flow(&work.draft.text())
        .context("the draft does not parse as a flow")?;
    let errors = golem_parser::validation::validate_flow(&flow);
    if !errors.is_empty() {
        let detail: Vec<String> = errors.into_iter().map(|e| e.message).collect();
        anyhow::bail!("the draft does not validate: {}", detail.join("; "));
    }
    let flow_dir = work
        .draft
        .source_dir()
        .map_or_else(|| work.project_root.clone(), Path::to_path_buf);
    for block in &mut flow.block {
        block.steps =
            golem_parser::mixin::expand_mixins(&block.steps, &flow_dir, &work.project_root)?;
    }
    // The draft's apps can lack what golem.toml adds (a bundle, a profile):
    // the run drives the apps the session resolved, as `act` does.
    if !work.apps.is_empty() {
        flow.flow.apps = work.apps.clone();
    }
    // A place in the draft as the run numbers it, through `load_mixin`.
    let run_place = |draft: &crate::draft::Draft,
                     block: &str,
                     index: usize|
     -> Result<golem_runner::context::StopAt> {
        let ran = flow
            .block
            .iter()
            .find(|b| b.name.as_deref() == Some(block))
            .with_context(|| format!("the draft has no block named {block:?}"))?;
        let b = draft
            .block_position(block)
            .with_context(|| format!("the draft has no block named {block:?}"))?;
        let step = draft.run_step(b, index, ran.steps.len()).with_context(|| {
            format!("block {block:?} has more than one load_mixin, so a place in it is unclear; run from the start")
        })?;
        Ok(golem_runner::context::StopAt {
            block: block.to_string(),
            step: step + 1,
        })
    };
    let stop = match stop_at {
        Some(s) => {
            let at = golem_runner::context::StopAt::parse(s)?;
            Some(run_place(&work.draft, &at.block, at.step - 1)?)
        }
        None => None,
    };
    let start = if restart {
        None
    } else {
        let (block, index) = work
            .draft
            .cursor()
            .context("the draft has no steps to run from")?;
        Some(run_place(&work.draft, &block, index)?)
    };
    let start_block = match &start {
        Some(s) => Some(s.block.clone()),
        None => flow.flow.start.clone(),
    };
    let base_timeout = flow
        .flow
        .options
        .as_ref()
        .and_then(|o| o.step_timeout)
        .unwrap_or(work.base_timeout_ms);

    let (events, subs) = golem_events::channel::event_channel();
    let passed: Passed = Arc::default();
    let mut follow = tokio::spawn(follow_phase(
        subs.subscribe(),
        status.clone(),
        passed.clone(),
    ));
    drop(subs);
    let emitter = golem_events::emitter::DeviceEmitter::new(
        events,
        golem_events::DeviceId(work.device.name.clone()),
    );
    let Work {
        device,
        driver,
        project_root,
        capture,
        child_env,
        vars,
        step_count,
        rng,
        browser,
        recovery,
        ..
    } = work;
    let flow_name = flow.flow.name.clone();
    let mut ctx = golem_runner::context::ExecutionContext {
        device: Some(device),
        child_env: child_env.as_ref(),
        global_step_index: *step_count,
        rng: std::sync::Mutex::new(std::mem::replace(
            rng,
            golem_vars::seed::FakeRng::from_optional_seed(None),
        )),
        browser: browser.clone(),
        recovery: recovery.as_deref(),
        emitter: Some(&emitter),
        stop_at: stop,
        start_at: start,
        ..golem_runner::context::ExecutionContext::new(&flow_dir, project_root, capture, &flow_name)
    };
    let result = golem_runner::executor::execute_flow(
        &flow,
        driver.as_ref(),
        vars,
        start_block.as_deref(),
        base_timeout,
        &mut ctx,
        None,
    )
    .await;
    *step_count = ctx.global_step_index;
    let stopped = ctx.stopped_at.take();
    *rng = ctx.rng.into_inner().unwrap_or_else(|e| e.into_inner());
    drop(emitter);
    let _ = tokio::time::timeout(Duration::from_millis(500), &mut follow).await;
    follow.abort();

    let passed = std::mem::take(&mut *passed.lock().unwrap_or_else(|e| e.into_inner()));
    work.draft.mark_ran(&flow, &passed)?;
    let (outcome, at) = match &result {
        Ok(r) if !r.success => (
            format!(
                "failed at {}:{} {}: {}",
                r.failed_block.as_deref().unwrap_or("?"),
                r.failed_step.map_or(0, |i| i + 1),
                r.failed_action.as_deref().unwrap_or("?"),
                r.failed_reason.as_deref().unwrap_or_default()
            ),
            r.failed_block
                .clone()
                .map(|b| (b, r.failed_step.map_or(1, |i| i + 1))),
        ),
        Ok(_) => match &stopped {
            Some(s) => (
                format!("stopped before {s}"),
                Some((s.block.clone(), s.step)),
            ),
            None => ("ran to the end".to_string(), None),
        },
        Err(e) => (format!("failed: {e:#}"), None),
    };
    let placed = at.is_some_and(|(block, step)| work.draft.place_cursor(&flow, &block, step));
    if !placed {
        work.draft.insert_at_end();
    }
    let listing = work.draft.steps(&crate::draft::StepsQuery {
        context: Some(3),
        ..crate::draft::StepsQuery::default()
    })?;
    Ok(OpResult::Edited(format!(
        "draft_run · {} step{} passed · {outcome}\n{listing}",
        passed.len(),
        if passed.len() == 1 { "" } else { "s" }
    )))
}

/// A warning for a step that passed in half its timeout or more: on a
/// slower device or a busy host the same step can time out in `golem run`.
/// It suggests about twice the time taken, rounded up to a second.
/// A step that selected by `on_accessibility_label` where the visible
/// text selects the same element.
fn label_hint(text: &str) -> String {
    format!(
        "hint: on_text = {text:?} selects the same element; prefer it unless the test checks the accessibility label"
    )
}

/// The usual fix for a failed or warned step's code.
fn fix_line(outcome: &golem_report::StepOutcome) -> Option<String> {
    let code = match outcome {
        golem_report::StepOutcome::Failed { code, .. }
        | golem_report::StepOutcome::Warning { code, .. } => *code,
        _ => return None,
    };
    (code != golem_events::FailureCode::Uncoded).then(|| format!("fix: {}", code.fix()))
}

fn slow_warning(duration_ms: u64, timeout_ms: u64) -> Option<String> {
    if timeout_ms == 0 || duration_ms.saturating_mul(2) < timeout_ms {
        return None;
    }
    let suggest = (duration_ms.saturating_mul(2).max(timeout_ms + 1)).div_ceil(1000) * 1000;
    let secs = |ms: u64| {
        if ms.is_multiple_of(1000) {
            format!("{}s", ms / 1000)
        } else {
            format!("{:.1}s", ms as f64 / 1000.0)
        }
    };
    Some(format!(
        "warning: took {} of its {} timeout · consider timeout = {suggest}",
        secs(duration_ms),
        secs(timeout_ms)
    ))
}

async fn read_tree(driver: &dyn PlatformDriver, full: bool, json: bool) -> Result<String> {
    if !full && !json {
        return crate::interactive::visible_tree(driver).await;
    }
    let (root, meta) = driver.get_hierarchy().await?;
    if json {
        let tree = if full {
            root
        } else {
            let mut viewport = golem_element::Viewport::from_root(&root);
            viewport.height -= meta.keyboard_height;
            golem_element::filter_viewport(&root, &viewport)
        };
        return Ok(serde_json::to_string_pretty(&tree)?);
    }
    Ok(encode_tree(
        &root,
        &TreeHeader {
            full: true,
            keyboard_height: meta.keyboard_height,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::{Bounds, Element};

    fn el(t: &str, text: Option<&str>, b: (i32, i32, i32, i32)) -> Element {
        Element {
            element_type: t.into(),
            text: text.map(str::to_string),
            accessibility_label: None,
            accessibility_id: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: text.is_some(),
            focused: false,
            bounds: Bounds::new(b.0, b.1, b.2, b.3),
            visible_bounds: None,
            hit_points: vec![],
            drawing_order: None,
            children: vec![],
        }
    }

    fn screen() -> Element {
        let mut root = el("View", None, (0, 0, 400, 800));
        root.children
            .push(el("Label", Some("Total"), (20, 100, 200, 40)));
        root.children
            .push(el("Label", Some("42"), (20, 160, 200, 40)));
        root.children
            .push(el("Button", Some("Pay 42"), (20, 300, 360, 60)));
        root
    }

    fn device() -> DeviceInfo {
        DeviceInfo {
            name: "Mock".into(),
            udid: "mock-1".into(),
            platform: golem_devices::Platform::Android,
            device_type: golem_devices::DeviceType::Phone,
            os_major: 16,
            os_version: "16".into(),
            state: golem_devices::DeviceState::Booted,
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        }
    }

    fn session_on(
        driver: Arc<dyn PlatformDriver>,
        lease: Option<DeviceLease>,
        idle: Duration,
    ) -> Session {
        Session::from_parts(
            Parts {
                device: device(),
                bundle: "fail.golem.test".into(),
                driver,
                lease,
                project_root: std::env::temp_dir(),
                apps: Vec::new(),
                child_env: None,
            },
            idle,
        )
    }

    #[tokio::test]
    async fn app_logs_shows_crashes_first_and_filters() {
        let s = session_on(
            Arc::new(golem_driver::stub::StubDriver::new(1, Default::default())),
            None,
            Duration::from_secs(60),
        );
        let all = s.app_logs(&LogsRequest::default()).await.expect("logs");
        assert!(
            all.starts_with("app_logs fail.golem.test · 3 of 3 lines"),
            "{all}"
        );
        let crash = all.find("crash[1]:").expect("crash section");
        assert!(
            crash < all.find("lines[2]:").expect("lines section"),
            "{all}"
        );
        let marker = s
            .app_logs(&LogsRequest {
                filter: Some("MARKER".into()),
                ..Default::default()
            })
            .await
            .expect("logs");
        assert!(marker.contains("1 of 1 lines"), "{marker}");
        assert!(marker.contains("golem-marker stub"), "{marker}");
        let other = s
            .app_logs(&LogsRequest {
                app: Some("other.app".into()),
                ..Default::default()
            })
            .await
            .expect("logs");
        assert!(other.starts_with("app_logs other.app"), "{other}");
    }

    #[tokio::test]
    async fn app_logs_runs_while_an_operation_is_busy() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let s = session_on(
            Arc::new(golem_driver::stub::StubDriver::new(1, Default::default())),
            None,
            Duration::from_secs(60),
        );
        let held = gate.clone();
        let Begin::Started(_) = s.run_checked("act", "stuck".into(), move |slot| {
            Box::pin(async move {
                held.notified().await;
                drop(slot);
                OpResult::Cancelled
            })
        }) else {
            panic!("SHALL start");
        };
        assert!(matches!(s.status(), Status::Busy(_)));
        let logs =
            tokio::time::timeout(Duration::from_secs(5), s.app_logs(&LogsRequest::default()))
                .await
                .expect("app_logs SHALL not wait for the busy operation")
                .expect("logs");
        assert!(logs.contains("FATAL EXCEPTION"), "{logs}");
        gate.notify_one();
    }

    #[tokio::test]
    async fn app_logs_refuses_after_close() {
        let s = session_on(
            Arc::new(golem_driver::stub::StubDriver::new(1, Default::default())),
            None,
            Duration::from_secs(60),
        );
        s.close("closed", false).await;
        let err = s
            .app_logs(&LogsRequest::default())
            .await
            .expect_err("closed");
        assert!(
            err.to_string().contains("the session ended: closed"),
            "{err}"
        );
    }

    async fn run(s: &Session, op: Op) -> Outcome {
        match s.begin(op) {
            Begin::Started(_) => {}
            other => panic!("SHALL start: {other:?}"),
        }
        match s.wait(Duration::from_secs(10)).await {
            Waited::Done(o) => o,
            other => panic!("SHALL finish: {other:?}"),
        }
    }

    /// Counts restarts. The mock driver's call after a scripted death
    /// succeeds, so a restart needs to change nothing.
    struct CountingRecovery(std::sync::atomic::AtomicU32);

    #[async_trait::async_trait]
    impl golem_runner::recovery::CompanionRecovery for CountingRecovery {
        async fn restart_and_reconnect(&self) -> Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A mock session whose companion dies on the first `method` call.
    async fn dying_session(
        method: &str,
    ) -> (Session, Arc<MockPlatformDriver>, Arc<CountingRecovery>) {
        let driver = Arc::new(MockPlatformDriver::new(screen()));
        driver.set_error_on_calls(
            method,
            golem_events::FailureCode::DeviceCompanionUnreachable,
            "connection refused",
            &[1],
        );
        let s = session_on(driver.clone(), None, DEFAULT_IDLE_TIMEOUT);
        let recovery = Arc::new(CountingRecovery(Default::default()));
        s.work.lock().await.as_mut().expect("work").recovery = Some(recovery.clone());
        (s, driver, recovery)
    }

    #[tokio::test]
    async fn an_act_whose_companion_dies_restarts_it_and_passes_on_the_retry() {
        let (s, driver, recovery) = dying_session("tap").await;
        let done = run(&s, act(r#"{ action = "tap", on_text = "Pay 42" }"#)).await;
        match &done.result {
            OpResult::Act { passed, toon, .. } => {
                assert!(passed, "{toon}");
                assert!(toon.contains("restart:companion 1/"), "{toon}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(recovery.0.load(Ordering::SeqCst), 1);
        assert_eq!(
            driver.get_calls().iter().filter(|c| c.0 == "tap").count(),
            2
        );
    }

    #[tokio::test]
    async fn a_tree_whose_companion_dies_restarts_it_and_says_so() {
        let (s, _, recovery) = dying_session("get_hierarchy").await;
        let done = run(
            &s,
            Op::Tree {
                full: false,
                json: false,
            },
        )
        .await;
        match &done.result {
            OpResult::Tree(text) => assert!(text.starts_with(RESTARTED), "{text}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(recovery.0.load(Ordering::SeqCst), 1);
    }

    fn act(step: &str) -> Op {
        Op::Act {
            step: step.into(),
            tree: false,
            comment: None,
        }
    }

    #[tokio::test]
    async fn a_value_one_act_reads_is_there_for_the_next() {
        let s = session_on(
            Arc::new(MockPlatformDriver::new(screen())),
            None,
            DEFAULT_IDLE_TIMEOUT,
        );
        let read = run(
            &s,
            act(r#"{ action = "read", on_below = "Total", save_to = "total" }"#),
        )
        .await;
        assert!(
            matches!(read.result, OpResult::Act { passed: true, .. }),
            "{read:?}"
        );
        let tap = run(
            &s,
            act(r#"{ action = "assert_visible", on_text = "Pay ${total}", timeout = 500 }"#),
        )
        .await;
        match &tap.result {
            OpResult::Act { passed, toon, .. } => {
                assert!(passed, "{toon}");
                assert!(toon.contains("Pay 42"), "{toon}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(tap.op_id, 2, "op ids count up");
        let log = s.log().await;
        assert_eq!(log.len(), 2);
        s.close("closed", false).await;
        assert!(matches!(s.status(), Status::Ended(_)));
    }

    #[tokio::test]
    async fn tree_probe_and_screenshot_run_as_operations() {
        let s = session_on(
            Arc::new(MockPlatformDriver::new(screen())),
            None,
            DEFAULT_IDLE_TIMEOUT,
        );
        match run(
            &s,
            Op::Tree {
                full: false,
                json: false,
            },
        )
        .await
        .result
        {
            OpResult::Tree(t) => assert!(t.contains("\"Pay 42\""), "{t}"),
            other => panic!("{other:?}"),
        }
        match run(
            &s,
            Op::Probe {
                selector: r#"{ on_text = "Pay*" }"#.into(),
                timeout_ms: 0,
            },
        )
        .await
        .result
        {
            OpResult::Probe { toon, .. } => assert!(toon.contains("1 visible match"), "{toon}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            run(&s, Op::Screenshot).await.result,
            OpResult::Screenshot { .. }
        ));
    }

    /// A driver whose hierarchy calls never return, for a busy session.
    fn stuck() -> Arc<MockPlatformDriver> {
        let d = MockPlatformDriver::new(screen());
        d.set_hierarchy_delay(Duration::from_secs(3600));
        Arc::new(d)
    }

    #[tokio::test(start_paused = true)]
    async fn a_second_call_while_busy_is_refused_and_cancel_stops_it() {
        let s = session_on(stuck(), None, DEFAULT_IDLE_TIMEOUT);
        let Begin::Started(first) = s.begin(Op::Tree {
            full: false,
            json: false,
        }) else {
            panic!("SHALL start");
        };
        match s.begin(Op::Screenshot) {
            Begin::Busy(r) => {
                assert_eq!(r.op_id, first);
                assert_eq!(r.op, "tree");
            }
            other => panic!("SHALL be busy: {other:?}"),
        }
        assert!(matches!(
            s.wait(Duration::from_secs(5)).await,
            Waited::Pending(_)
        ));
        assert!(s.cancel().await);
        match s.wait(Duration::from_secs(1)).await {
            Waited::Done(o) => assert!(matches!(o.result, OpResult::Cancelled)),
            other => panic!("{other:?}"),
        }
        assert!(
            matches!(s.begin(Op::Screenshot), Begin::Started(_)),
            "the session SHALL take work again"
        );
    }

    #[tokio::test]
    async fn the_last_outcome_is_readable_after_a_failure() {
        let s = session_on(
            Arc::new(MockPlatformDriver::new(screen())),
            None,
            DEFAULT_IDLE_TIMEOUT,
        );
        let failed = run(&s, act(r#"{ action = "fail", message = "boom" }"#)).await;
        match s.wait(Duration::ZERO).await {
            Waited::Done(o) => {
                assert_eq!(o.op_id, failed.op_id);
                assert!(matches!(o.result, OpResult::Act { passed: false, .. }));
            }
            other => panic!("{other:?}"),
        }
        match s.status() {
            Status::Idle { last: Some(o), .. } => assert_eq!(o.op_id, failed.op_id),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_idle_clock_counts_only_time_with_no_operation() {
        let s = session_on(stuck(), None, Duration::from_secs(60));
        tokio::time::advance(Duration::from_secs(30)).await;
        assert_eq!(s.idle_for(), Duration::from_secs(30));
        let _ = s.begin(Op::Tree {
            full: false,
            json: false,
        });
        tokio::time::advance(Duration::from_secs(600)).await;
        assert_eq!(
            s.idle_for(),
            Duration::ZERO,
            "a running operation SHALL NOT count"
        );
        assert!(!s.expired());
        s.cancel().await;
        tokio::time::advance(Duration::from_secs(59)).await;
        assert!(
            !s.expired(),
            "the clock SHALL restart when the operation ends"
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(s.expired());
    }

    #[tokio::test(start_paused = true)]
    async fn closing_a_busy_session_releases_the_device_after_the_operation_stops() {
        let rm = Arc::new(ResourceManager::new(
            golem_devices::concurrency::ConcurrencyConfig::default(),
        ));
        let lease = rm.try_lease(&device(), 0).expect("lease");
        let s = session_on(stuck(), Some(lease), DEFAULT_IDLE_TIMEOUT);
        let _ = s.begin(Op::Tree {
            full: false,
            json: false,
        });
        assert!(
            rm.try_allocate(&device(), 0).is_err(),
            "the session SHALL hold the device"
        );
        s.close("client disconnected", false).await;
        assert_eq!(rm.active_count(), 0, "closing SHALL release the device");
        match s.begin(Op::Screenshot) {
            Begin::Ended(reason) => assert_eq!(reason, "client disconnected"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(s.wait(Duration::ZERO).await, Waited::Ended(_)));
    }

    fn flow_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("teardown.marker");
        let flow = format!(
            r#"[flow]
name = "Session flow"
[flow.vars]
target = "Submit"
[flow.options]
a11y = "off"
perf = false

[[flow.apps]]
name = "app"
bundle = "{bundle}"
[[flow.apps.devices]]
os = ["android:latest"]
type = "phone"

[[block]]
name = "one"
steps = [ {{ action = "assert_visible", on_text = "${{target}}" }} ]

[[block]]
name = "two"
steps = [
  {{ action = "assert_visible", on_text = "Submit" }},
  {{ action = "fail", message = "the second step of two" }},
]

[[teardown]]
steps = [ {{ action = "bash", run = "touch {marker}" }} ]
"#,
            bundle = golem_driver::stub::STUB_BUNDLE_ID,
            marker = marker.display(),
        );
        std::fs::write(dir.path().join("s.test.toml"), flow).expect("flow");
        (dir, marker)
    }

    fn open_flow(dir: &std::path::Path, stop_at: Option<&str>, break_on_failure: bool) -> Session {
        Session::start(
            OpenRequest {
                query: TargetQuery::default(),
                project_root: dir.to_path_buf(),
                child_env: None,
                idle_timeout: DEFAULT_IDLE_TIMEOUT,
                flow: Some(FlowOpen {
                    path: dir.join("s.test.toml"),
                    stop_at: stop_at
                        .map(|s| golem_runner::context::StopAt::parse(s).expect("stop")),
                    break_on_failure,
                    no_teardown: false,
                    vars: Vec::new(),
                    stub: true,
                }),
                boot: false,
                stub: false,
            },
            Arc::new(ResourceManager::new(
                golem_devices::concurrency::ConcurrencyConfig::default(),
            )),
            golem_runner::installer::InstallCache::new(),
            Arc::new(Slots::new(DEFAULT_MAX_SESSION_DEVICES)),
        )
    }

    /// A stub session that takes its slot from `slots`.
    fn open_stub(slots: &Arc<Slots>) -> Session {
        Session::start(
            OpenRequest {
                query: TargetQuery::default(),
                project_root: std::env::temp_dir(),
                child_env: None,
                idle_timeout: DEFAULT_IDLE_TIMEOUT,
                flow: None,
                boot: false,
                stub: true,
            },
            Arc::new(ResourceManager::new(
                golem_devices::concurrency::ConcurrencyConfig::default(),
            )),
            golem_runner::installer::InstallCache::new(),
            slots.clone(),
        )
    }

    async fn is_open(s: &Session) -> bool {
        matches!(
            s.wait(Duration::from_secs(5)).await,
            Waited::Done(Outcome {
                result: OpResult::Opened { .. },
                ..
            })
        )
    }

    #[tokio::test]
    async fn an_open_past_the_device_cap_waits_and_names_the_holders() {
        let slots = Arc::new(Slots::new(3));
        let held = [open_stub(&slots), open_stub(&slots), open_stub(&slots)];
        for s in &held {
            assert!(is_open(s).await);
        }
        let fourth = open_stub(&slots);
        let Waited::Pending(running) = fourth.wait(Duration::from_millis(200)).await else {
            panic!("the fourth open SHALL wait");
        };
        assert!(
            running.phase.starts_with(
                "waiting for a device: 3 of 3 held by sessions (Stub Device (stub-session), "
            ),
            "{}",
            running.phase
        );

        held[1].close("closed", false).await;
        assert!(
            is_open(&fourth).await,
            "the waiting open SHALL go on once a session closes"
        );
    }

    #[tokio::test]
    async fn cancel_ends_a_wait_for_a_device_without_taking_a_slot() {
        let slots = Arc::new(Slots::new(1));
        let first = open_stub(&slots);
        assert!(is_open(&first).await);
        let second = open_stub(&slots);
        assert!(matches!(
            second.wait(Duration::from_millis(100)).await,
            Waited::Pending(_)
        ));
        assert!(second.cancel().await, "the waiting open SHALL be cancelled");

        first.close("closed", false).await;
        let third = open_stub(&slots);
        assert!(
            is_open(&third).await,
            "the cancelled open SHALL have left the slot free"
        );
    }

    async fn opened(s: &Session) -> String {
        match s.wait(Duration::from_secs(30)).await {
            Waited::Done(Outcome {
                result: OpResult::Opened { flow: Some(f), .. },
                ..
            }) => f,
            other => panic!("SHALL open: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_flow_open_stops_at_the_step_and_keeps_the_flow_vars() {
        let (dir, marker) = flow_dir();
        let s = open_flow(dir.path(), Some("two:2"), false);
        let summary = opened(&s).await;
        assert!(summary.contains("session open before two:2"), "{summary}");
        match run(&s, Op::DraftSteps(crate::draft::StepsQuery::default()))
            .await
            .result
        {
            OpResult::Draft(steps) => {
                for line in [
                    "one:1 ✓",
                    "two:1 ✓",
                    "▸ cursor\n  two:2 · { action = \"fail\"",
                ] {
                    assert!(
                        steps.contains(line),
                        "the steps the flow ran SHALL be passed: {steps}"
                    );
                }
            }
            other => panic!("{other:?}"),
        }
        let next = run(
            &s,
            act(r#"{ action = "assert_visible", on_text = "${target}" }"#),
        )
        .await;
        assert!(
            matches!(next.result, OpResult::Act { passed: true, .. }),
            "{next:?}"
        );
        assert!(!marker.exists(), "no teardown while the session is open");
        let notes = s.close("closed", true).await;
        assert_eq!(notes.as_deref(), Some("teardown ran"));
        assert!(marker.exists(), "an explicit close SHALL run the teardown");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn break_on_failure_opens_at_the_failed_step_and_a_disconnect_skips_teardown() {
        let (dir, marker) = flow_dir();
        let s = open_flow(dir.path(), None, true);
        let summary = opened(&s).await;
        assert!(
            summary.contains("session open at the failed step: two:2 fail"),
            "{summary}"
        );
        s.close("client disconnected", false).await;
        assert!(!marker.exists(), "a disconnect SHALL skip the teardown");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn without_break_on_failure_a_failed_flow_opens_nothing_and_tears_down() {
        let (dir, marker) = flow_dir();
        let s = open_flow(dir.path(), None, false);
        match s.wait(Duration::from_secs(30)).await {
            Waited::Done(Outcome {
                result: OpResult::Failed(e),
                ..
            }) => assert!(e.contains("no session opened"), "{e}"),
            other => panic!("SHALL fail: {other:?}"),
        }
        assert!(
            marker.exists(),
            "the flow SHALL end as golem run would, teardown included"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stop_at_the_flow_does_not_have_is_refused_before_device_work() {
        let (dir, _) = flow_dir();
        for (stop, want) in [
            ("three", "no block named \"three\""),
            ("two:5", "has 2 step(s)"),
        ] {
            let s = open_flow(dir.path(), Some(stop), false);
            match s.wait(Duration::from_secs(30)).await {
                Waited::Done(Outcome {
                    result: OpResult::Failed(e),
                    ..
                }) => assert!(e.contains(want), "{stop}: {e}"),
                other => panic!("{stop} SHALL fail: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_passing_step_goes_into_the_draft_and_a_failing_one_does_not() {
        let s = session_on(
            Arc::new(MockPlatformDriver::new(screen())),
            None,
            DEFAULT_IDLE_TIMEOUT,
        );
        let tap = run(
            &s,
            Op::Act {
                step: r#"action="tap",on_text="Pay*""#.into(),
                tree: false,
                comment: Some("Pay now".into()),
            },
        )
        .await;
        assert!(
            matches!(tap.result, OpResult::Act { passed: true, .. }),
            "{tap:?}"
        );
        let _ = run(&s, act(r#"{ action = "fail", message = "no" }"#)).await;
        match run(&s, Op::DraftShow).await.result {
            OpResult::Draft(text) => {
                assert!(
                    text.contains("  # Pay now\n  { action = \"tap\", on_text = \"Pay*\" },\n"),
                    "{text}"
                );
                assert!(
                    !text.contains("fail"),
                    "a failed step SHALL NOT be recorded: {text}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_step_that_takes_half_its_timeout_or_more_gets_a_warning() {
        assert_eq!(slow_warning(2_400, 5_000), None);
        assert_eq!(
            slow_warning(2_500, 5_000).as_deref(),
            Some("warning: took 2.5s of its 5s timeout · consider timeout = 6000")
        );
        assert_eq!(
            slow_warning(4_100, 5_000).as_deref(),
            Some("warning: took 4.1s of its 5s timeout · consider timeout = 9000")
        );
        assert_eq!(
            slow_warning(9_000, 10_000).as_deref(),
            Some("warning: took 9s of its 10s timeout · consider timeout = 18000")
        );
    }

    #[test]
    fn a_failed_step_shows_the_fix_for_its_code() {
        let failed = golem_report::StepOutcome::Failed {
            message: "no match".into(),
            code: golem_events::FailureCode::FlowElementNotFound,
        };
        assert_eq!(
            fix_line(&failed),
            Some(format!(
                "fix: {}",
                golem_events::FailureCode::FlowElementNotFound.fix()
            ))
        );
        let uncoded = golem_report::StepOutcome::Failed {
            message: "?".into(),
            code: golem_events::FailureCode::Uncoded,
        };
        assert_eq!(fix_line(&uncoded), None);
    }

    #[test]
    fn the_label_hint_names_the_text_selector() {
        assert_eq!(
            label_hint("Sign in"),
            "hint: on_text = \"Sign in\" selects the same element; prefer it unless the test checks the accessibility label"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn act_warns_when_a_step_passes_near_its_explicit_timeout() {
        let s = session_on(
            Arc::new(golem_driver::stub::StubDriver::new(1, Default::default())),
            None,
            Duration::from_secs(60),
        );
        let done = run(
            &s,
            act(r#"{ action = "bash", run = "sleep 0.6", timeout = 1000 }"#),
        )
        .await;
        match done.result {
            OpResult::Act {
                passed: true, toon, ..
            } => assert!(
                toon.contains("warning: took ") && toon.contains("of its 1s timeout"),
                "{toon}"
            ),
            other => panic!("{other:?}"),
        }
        let quick = run(&s, act(r#"{ action = "hide_keyboard" }"#)).await;
        match quick.result {
            OpResult::Act { toon, .. } => assert!(!toon.contains("warning"), "{toon}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn draft_run_resumes_from_the_cursor_and_restarts_from_the_start() {
        let (dir, _) = flow_dir();
        // As run = false: nothing ran, and the cursor is before one:1.
        let s = open_flow(dir.path(), Some("one"), false);
        let _ = opened(&s).await;
        let text = |o: Outcome| match o.result {
            OpResult::Edited(t) => t,
            other => panic!("{other:?}"),
        };
        let resumed = text(
            run(
                &s,
                Op::DraftRun {
                    restart: false,
                    stop_at: Some("two:2".into()),
                },
            )
            .await,
        );
        assert!(
            resumed.starts_with("draft_run · 2 steps passed · stopped before two:2\n"),
            "{resumed}"
        );
        assert!(resumed.contains("two:1 ✓"), "{resumed}");
        assert!(resumed.contains("▸ cursor\n  two:2 ·"), "{resumed}");

        let restarted = text(
            run(
                &s,
                Op::DraftRun {
                    restart: true,
                    stop_at: None,
                },
            )
            .await,
        );
        assert!(
            restarted.starts_with(
                "draft_run · 2 steps passed · failed at two:2 fail: the second step of two"
            ),
            "{restarted}"
        );
        assert!(restarted.contains("▸ cursor\n  two:2 ·"), "{restarted}");
        s.close("closed", false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn draft_run_clears_the_marker_of_a_step_that_passes() {
        let (dir, _) = flow_dir();
        let s = open_flow(dir.path(), Some("two:2"), false);
        let _ = opened(&s).await;
        let added = run(
            &s,
            act(r#"{ action = "assert_visible", on_text = "Submit" }"#),
        )
        .await;
        assert!(matches!(added.result, OpResult::Act { passed: true, .. }));
        match run(&s, Op::DraftShow).await.result {
            OpResult::Draft(t) => assert!(t.contains("# unverified"), "{t}"),
            other => panic!("{other:?}"),
        }
        let edited = run(&s, Op::Edit(DraftEdit::StepDelete { at: "two:3".into() })).await;
        assert!(matches!(edited.result, OpResult::Edited(_)), "{edited:?}");
        let _ = run(
            &s,
            Op::DraftRun {
                restart: true,
                stop_at: None,
            },
        )
        .await;
        match run(&s, Op::DraftShow).await.result {
            OpResult::Draft(t) => assert!(!t.contains("# unverified"), "{t}"),
            other => panic!("{other:?}"),
        }
        match run(&s, Op::DraftSteps(crate::draft::StepsQuery::default()))
            .await
            .result
        {
            OpResult::Draft(t) => assert!(
                t.contains("draft · 3 steps: 3 ✓ passed"),
                "a full run SHALL pass every step: {t}"
            ),
            other => panic!("{other:?}"),
        }
        s.close("closed", false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_export_adds_to_the_flow_file_at_the_stop_point() {
        let (dir, _) = flow_dir();
        let path = dir.path().join("s.test.toml");
        let before = std::fs::read_to_string(&path).expect("read");
        let s = open_flow(dir.path(), Some("two:2"), false);
        let _ = opened(&s).await;
        let added = run(
            &s,
            Op::Act {
                step: r#"{ action = "assert_visible", on_text = "${target}" }"#.into(),
                tree: false,
                comment: Some("Still here".into()),
            },
        )
        .await;
        assert!(
            matches!(added.result, OpResult::Act { passed: true, .. }),
            "{added:?}"
        );
        match run(
            &s,
            Op::Export {
                path: path.clone(),
                overwrite: false,
            },
        )
        .await
        .result
        {
            OpResult::Exported { steps, .. } => assert_eq!(steps, 4),
            other => panic!("{other:?}"),
        }
        let after = std::fs::read_to_string(&path).expect("read");
        let want = before.replace(
            "  {{ action = \"fail\"".replace("{{", "{").as_str(),
            "  # Still here\n  { action = \"assert_visible\", on_text = \"${target}\" },\n  # unverified\n  { action = \"fail\"",
        );
        assert_eq!(
            after, want,
            "the export SHALL add the step before the stop and change nothing else"
        );
        s.close("closed", false).await;
    }
}
