//! An interactive session: one device and one driver that stay alive
//! across many operations, for an LLM or a person working step by step.
//!
//! A session holds a lease on its device, so no suite run takes the device
//! while the session is open. It keeps what `golem do` loses between calls:
//! the driver (with its WebView inspector connection and IME state), the
//! variables, the step counter and the seeded RNG.
//!
//! A session runs one operation at a time. While one runs, [`Session::begin`]
//! refuses another with [`Begin::Busy`]; [`Session::wait`],
//! [`Session::status`] and [`Session::cancel`] still work. The outcome of
//! the last operation stays readable after it ends.
//!
//! Who owns a session is up to the caller: the daemon ties one to the
//! connection that opened it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
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
}

/// One operation.
#[derive(Debug, Clone)]
pub enum Op {
    /// Run a step, given in the canonical notation; with `tree`, also read
    /// the visible tree after it.
    Act { step: String, tree: bool },
    /// Read the tree: the visible one, or the full one as a hint.
    Tree { full: bool },
    /// Report what a selector matches, polling up to `timeout_ms`.
    Probe { selector: String, timeout_ms: u64 },
    /// Capture the screen as PNG.
    Screenshot,
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Op::Act { .. } => "act",
            Op::Tree { .. } => "tree",
            Op::Probe { .. } => "probe",
            Op::Screenshot => "screenshot",
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
    lease: Option<DeviceLease>,
    project_root: PathBuf,
    capture: golem_runner::capture::CaptureConfig,
    apps: Vec<golem_parser::AppConfig>,
    child_env: Option<golem_common::command::ChildEnv>,
    vars: golem_vars::VariableStore,
    step_count: u64,
    rng: golem_vars::seed::FakeRng,
    browser: Arc<tokio::sync::Mutex<golem_runner::browser::BrowserSlot>>,
    log: Vec<LogEntry>,
}

/// An open session.
pub struct Session {
    device: DeviceInfo,
    bundle: String,
    idle_timeout: Duration,
    status: Arc<Mutex<Status>>,
    changed: Arc<tokio::sync::Notify>,
    work: Arc<tokio::sync::Mutex<Work>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    next_op: AtomicU64,
}

impl Session {
    /// Select the device and app, lease the device, then connect to its
    /// companion: the lease comes before any companion work, as for
    /// `golem do`.
    pub async fn open(req: OpenRequest, resource_mgr: &Arc<ResourceManager>) -> Result<Session> {
        let (project, _) = crate::project::ProjectConfig::load_from(&req.project_root)?;
        let selection = target::select(&req.query, &project.apps).await?;
        let lease = crate::interactive::lease(resource_mgr, &selection.device)?;
        let target = target::connect(selection).await?;
        let driver: Arc<dyn PlatformDriver> = Arc::from(target.driver());
        Ok(Self::from_parts(
            Parts {
                device: target.device,
                bundle: target.bundle,
                driver,
                lease: Some(lease),
                project_root: req.project_root,
                apps: crate::interactive::app_configs(&project.apps),
                child_env: req.child_env,
            },
            req.idle_timeout,
        ))
    }

    /// A session on an already-resolved device and driver.
    pub fn from_parts(parts: Parts, idle_timeout: Duration) -> Session {
        let capture = golem_runner::capture::CaptureConfig {
            screenshot_on_failure: false,
            output_dir: parts.project_root.join(".golem/results"),
            ..Default::default()
        };
        let mut vars = golem_vars::VariableStore::new();
        vars.push_scope(golem_vars::Scope::new(golem_vars::ScopeLevel::Flow));
        Session {
            device: parts.device.clone(),
            bundle: parts.bundle,
            idle_timeout,
            status: Arc::new(Mutex::new(Status::Idle {
                since: Instant::now(),
                last: None,
            })),
            changed: Arc::new(tokio::sync::Notify::new()),
            work: Arc::new(tokio::sync::Mutex::new(Work {
                device: parts.device,
                driver: parts.driver,
                lease: parts.lease,
                project_root: parts.project_root,
                capture,
                apps: parts.apps,
                child_env: parts.child_env,
                vars,
                step_count: 0,
                rng: golem_vars::seed::FakeRng::from_optional_seed(None),
                browser: Default::default(),
                log: Vec::new(),
            })),
            task: Mutex::new(None),
            next_op: AtomicU64::new(1),
        }
    }

    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    pub fn bundle(&self) -> &str {
        &self.bundle
    }

    pub fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    /// Start `op` unless another is running or the session has ended.
    pub fn begin(&self, op: Op) -> Begin {
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
                op: op.name(),
                phase: phase_of(&op),
                started: Instant::now(),
            });
            op_id
        };
        let status = self.status.clone();
        let changed = self.changed.clone();
        let work = self.work.clone();
        let task = tokio::spawn(async move {
            let mut work = work.lock_owned().await;
            let name = op.name();
            let result = run_op(&mut work, &op).await;
            work.log.push(LogEntry {
                op_id,
                op: name,
                summary: summary_of(&op, &result),
                at: std::time::SystemTime::now(),
            });
            drop(work);
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
                    tokio::select! {
                        () = notified => {}
                        () = tokio::time::sleep_until(deadline) => return Waited::Pending(running),
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

    /// End the session: stop any operation, then release the device. The
    /// device is not shut down: the daemon does that when it exits.
    pub async fn close(&self, reason: &str) {
        self.cancel().await;
        {
            let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(&*status, Status::Ended(_)) {
                return;
            }
            *status = Status::Ended(reason.to_string());
        }
        self.changed.notify_waiters();
        let mut work = self.work.lock().await;
        golem_driver::ime::restore(std::slice::from_ref(&work.device.udid)).await;
        work.lease = None;
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

    /// The operations so far, oldest first.
    pub async fn log(&self) -> Vec<LogEntry> {
        self.work.lock().await.log.clone()
    }
}

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
        Op::Tree { full } => format!("tree{}", if *full { " (full)" } else { "" }),
        Op::Probe { selector, .. } => format!("probe {selector}"),
        Op::Screenshot => "screenshot".to_string(),
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

async fn run_op(work: &mut Work, op: &Op) -> OpResult {
    match op {
        Op::Act { step, tree } => act(work, step, *tree).await,
        Op::Tree { full } => match read_tree(work.driver.as_ref(), *full).await {
            Ok(text) => OpResult::Tree(text),
            Err(e) => OpResult::Failed(format!("{e:#}")),
        },
        Op::Probe {
            selector,
            timeout_ms,
        } => {
            let parsed = match golem_parser::inline::parse_selector_inline(selector) {
                Ok(p) => p,
                Err(e) => return OpResult::Failed(format!("{e:#}")),
            };
            match golem_runner::probe::probe(
                work.driver.as_ref(),
                &parsed.step,
                &parsed.line,
                *timeout_ms,
            )
            .await
            {
                Ok(report) => OpResult::Probe {
                    toon: golem_runner::probe::render_toon(&report),
                    json: golem_runner::probe::render_json(&report),
                },
                Err(e) => OpResult::Failed(format!("{e:#}")),
            }
        }
        Op::Screenshot => match work.driver.screenshot().await {
            Ok(shot) => OpResult::Screenshot { png: shot.data },
            Err(e) => OpResult::Failed(format!("{e:#}")),
        },
    }
}

async fn act(work: &mut Work, step: &str, tree: bool) -> OpResult {
    let step = match golem_parser::inline::parse_step_inline(step) {
        Ok(parsed) => parsed.step,
        Err(e) => return OpResult::Failed(format!("{e:#}")),
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
        golem_runner::policy::DEFAULT_BASE_TIMEOUT_MS,
    )
    .await;
    *step_count = ctx.global_step_index;
    *rng = ctx.rng.into_inner().unwrap_or_else(|e| e.into_inner());
    let tree = if tree {
        read_tree(driver.as_ref(), false).await.ok()
    } else {
        None
    };
    OpResult::Act {
        passed: !matches!(report.outcome, golem_report::StepOutcome::Failed { .. }),
        toon: golem_report::toon::format_step_toon(&report)
            .trim_start()
            .to_string(),
        step: golem_report::json::step_json(&report),
        tree,
    }
}

async fn read_tree(driver: &dyn PlatformDriver, full: bool) -> Result<String> {
    if !full {
        return crate::interactive::visible_tree(driver).await;
    }
    let (root, meta) = driver.get_hierarchy().await?;
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

    fn act(step: &str) -> Op {
        Op::Act {
            step: step.into(),
            tree: false,
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
        s.close("closed").await;
        assert!(matches!(s.status(), Status::Ended(_)));
    }

    #[tokio::test]
    async fn tree_probe_and_screenshot_run_as_operations() {
        let s = session_on(
            Arc::new(MockPlatformDriver::new(screen())),
            None,
            DEFAULT_IDLE_TIMEOUT,
        );
        match run(&s, Op::Tree { full: false }).await.result {
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
        let Begin::Started(first) = s.begin(Op::Tree { full: false }) else {
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
        let _ = s.begin(Op::Tree { full: false });
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
        let _ = s.begin(Op::Tree { full: false });
        assert!(
            rm.try_allocate(&device(), 0).is_err(),
            "the session SHALL hold the device"
        );
        s.close("client disconnected").await;
        assert_eq!(rm.active_count(), 0, "closing SHALL release the device");
        match s.begin(Op::Screenshot) {
            Begin::Ended(reason) => assert_eq!(reason, "client disconnected"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(s.wait(Duration::ZERO).await, Waited::Ended(_)));
    }
}
