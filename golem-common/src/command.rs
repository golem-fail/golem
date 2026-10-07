//! Shared subprocess seam.
//!
//! Every command execution in the workspace funnels through a process-global
//! [`CommandRunner`] so orchestration logic — boot-and-wait, install retry,
//! reboot timeouts, companion recovery — is hermetically testable. Production
//! uses [`SystemCommandRunner`], which spawns real processes; tests install a
//! [`FakeCommandRunner`] with canned responses keyed on `program + args`.
//!
//! This is deliberately a process-global (cf. the debug flag in `lib.rs`)
//! rather than a parameter threaded through every call site — it is the single
//! "shared seam" the roadmap calls for, avoiding piecemeal injection across
//! four crates.
//!
//! Isolation model: the runner is process-global, so a test that installs a
//! fake must not run *concurrently in the same process* with another that also
//! installs one — they would stomp the global. The workspace runs tests under
//! `cargo nextest`, which executes each test in its own process, so this never
//! happens (run override tests under nextest, not plain `cargo test`).
//! [`set_test_runner`] returns a guard that restores the *previous* runner on
//! drop, which keeps nested/sequential overrides within a single test correct.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::process::Output;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use async_trait::async_trait;

/// Extra knobs for a one-shot command. Defaults to no env, no stdin, inherit cwd.
#[derive(Default, Clone)]
pub struct CommandOpts {
    /// Environment variables to set for the child.
    pub env: Vec<(String, String)>,
    /// Bytes to write to the child's stdin (implies a piped stdin).
    pub stdin: Option<Vec<u8>>,
    /// Working directory for the child.
    pub current_dir: Option<String>,
    /// When set on a detached spawn, redirect the child's stdout+stderr to
    /// this file (appended) instead of discarding them. Used to persist the
    /// iOS companion's `xcodebuild test` output so a mid-flow XCUITest-host
    /// exit leaves a diagnosable teardown reason behind. Ignored by `output`
    /// (which already captures stdout/stderr into the returned `Output`).
    pub log_file: Option<String>,
    /// The whole inherited environment to start from, replacing this
    /// process's own; `env` is applied on top. `None` inherits.
    pub base_env: Option<Vec<(String, String)>>,
}

/// Kills a child's whole process group if dropped before the child exits.
///
/// A cancelled run aborts the task waiting on a child. Killing only the
/// child would leave what it started (an install script's gradle or
/// xcodebuild, a `bash` step's pipeline) running and racing the next run.
/// The child must lead its own group (`process_group(0)`). After a normal
/// exit the killer is disarmed: a step may start a background process on
/// purpose.
pub struct GroupKiller {
    pgid: Option<i32>,
}

impl GroupKiller {
    /// Arm for `child`, which leads its own process group.
    pub fn new(child: &tokio::process::Child) -> Self {
        GroupKiller {
            pgid: child.id().and_then(|id| i32::try_from(id).ok()),
        }
    }

    /// The child exited: leave its group alone.
    pub fn disarm(&mut self) {
        self.pgid = None;
    }

    /// Kill the group now.
    pub fn kill(&mut self) {
        if let Some(pgid) = self.pgid.take() {
            // SAFETY: `killpg` only sends a signal. The leader is not yet
            // reaped (armed means it has not been waited for), so the
            // group id cannot have been reused.
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
        }
    }
}

impl Drop for GroupKiller {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The environment and working directory of the client a daemon runs work
/// for. A long-lived daemon's own environment is that of whichever client
/// started it, so the processes it starts for a client's flows (install
/// scripts, `bash`, `run`) take that client's instead.
#[derive(Debug, Clone, Default)]
pub struct ChildEnv {
    pub cwd: Option<std::path::PathBuf>,
    pub vars: Vec<(String, String)>,
}

impl ChildEnv {
    /// Give `cmd` this environment, replacing the inherited one, and this
    /// working directory.
    pub fn apply(&self, cmd: &mut tokio::process::Command) {
        cmd.env_clear();
        cmd.envs(self.vars.iter().map(|(k, v)| (k, v)));
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
    }
}

impl CommandOpts {
    /// Options carrying only environment variables.
    pub fn with_env(env: Vec<(String, String)>) -> Self {
        Self {
            env,
            ..Self::default()
        }
    }

    /// Options carrying only stdin bytes.
    pub fn with_stdin(stdin: Vec<u8>) -> Self {
        Self {
            stdin: Some(stdin),
            ..Self::default()
        }
    }

    /// Options carrying env vars plus a detached-spawn log file.
    pub fn with_env_and_log(env: Vec<(String, String)>, log_file: String) -> Self {
        Self {
            env,
            log_file: Some(log_file),
            ..Self::default()
        }
    }
}

/// Abstraction over subprocess execution. The real implementation spawns
/// processes; the fake returns canned [`Output`]/errors.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Run `program` with `args`, wait for it to finish, and capture its
    /// [`Output`] (status + stdout + stderr).
    async fn output(
        &self,
        program: &str,
        args: &[String],
        opts: &CommandOpts,
    ) -> std::io::Result<Output>;

    /// Spawn `program` detached (stdout/stderr discarded) and return as soon
    /// as it has launched — for long-running processes (emulator, companion)
    /// whose readiness is polled separately via [`CommandRunner::output`].
    async fn spawn_detached(
        &self,
        program: &str,
        args: &[String],
        opts: &CommandOpts,
    ) -> std::io::Result<()>;

    /// Spawn `program` and hand back a *live* child: stderr arrives line by
    /// line while it runs, and the caller owns the timeout and the kill.
    ///
    /// [`CommandRunner::output`] can't model this — it waits for exit before
    /// returning anything, so an install script's build progress would only
    /// appear once the build had finished.
    async fn spawn_streaming(
        &self,
        program: &str,
        args: &[String],
        opts: &CommandOpts,
    ) -> std::io::Result<StreamingProcess>;
}

/// A spawned child whose stderr streams while it runs.
///
/// `stderr` is closed at EOF; `child` is waited on (and, on timeout, killed)
/// by the caller. Split this way because the real implementation reads stderr
/// in its own task *concurrently* with the wait — a single `&mut` handle
/// offering both could not express that.
pub struct StreamingProcess {
    /// Stderr lines in order, ending when the stream closes.
    pub stderr: tokio::sync::mpsc::UnboundedReceiver<String>,
    /// The running child.
    pub child: Box<dyn StreamingChild>,
}

/// The wait/kill half of a [`StreamingProcess`].
#[async_trait]
pub trait StreamingChild: Send {
    /// Wait for exit, yielding the exit code (`None` when signalled).
    ///
    /// An implementation MAY never resolve — that is what lets a fake exercise
    /// a caller's timeout-and-kill path without a real process to hang.
    async fn wait(&mut self) -> std::io::Result<ExitOutcome>;

    /// Kill the child. Best-effort: it may already be gone.
    async fn kill(&mut self);
}

/// How a streamed child finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitOutcome {
    /// Whether it exited zero.
    pub success: bool,
    /// Exit code, or `None` when it was signalled.
    pub code: Option<i32>,
}

// ---------------------------------------------------------------------------
// Real implementation
// ---------------------------------------------------------------------------

/// The production runner: spawns real subprocesses via `tokio::process`.
pub struct SystemCommandRunner;

fn apply_opts(cmd: &mut tokio::process::Command, opts: &CommandOpts) {
    if let Some(base) = &opts.base_env {
        cmd.env_clear();
        cmd.envs(base.iter().map(|(k, v)| (k, v)));
    }
    for (k, v) in &opts.env {
        cmd.env(k, v);
    }
    if let Some(dir) = &opts.current_dir {
        cmd.current_dir(dir);
    }
}

#[async_trait]
impl CommandRunner for SystemCommandRunner {
    async fn output(
        &self,
        program: &str,
        args: &[String],
        opts: &CommandOpts,
    ) -> std::io::Result<Output> {
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args);
        apply_opts(&mut cmd, opts);

        let Some(stdin_bytes) = &opts.stdin else {
            return cmd.output().await;
        };

        // Piped stdin: feed the bytes, then collect output.
        use std::process::Stdio;
        use tokio::io::AsyncWriteExt;
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        if let Some(mut sink) = child.stdin.take() {
            sink.write_all(stdin_bytes).await?;
            sink.shutdown().await?;
        }
        child.wait_with_output().await
    }

    async fn spawn_detached(
        &self,
        program: &str,
        args: &[String],
        opts: &CommandOpts,
    ) -> std::io::Result<()> {
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args);
        match &opts.log_file {
            Some(path) => {
                // Append so a companion restart to the same path preserves the
                // prior instance's death output rather than truncating it.
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?;
                let err = file.try_clone()?;
                cmd.stdout(std::process::Stdio::from(file))
                    .stderr(std::process::Stdio::from(err));
            }
            None => {
                cmd.stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
            }
        }
        cmd.stdin(std::process::Stdio::null());
        apply_opts(&mut cmd, opts);
        cmd.spawn()?;
        Ok(())
    }

    async fn spawn_streaming(
        &self,
        program: &str,
        args: &[String],
        opts: &CommandOpts,
    ) -> std::io::Result<StreamingProcess> {
        use std::process::Stdio;
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args);
        apply_opts(&mut cmd, opts);
        // stdout discarded, stderr piped: install scripts report progress on
        // stderr, and a script that chats on stdout must not fill a pipe
        // nobody drains. stdin is empty, never golem's own: when golem runs
        // as a background job, a child that reads the terminal (xcodebuild
        // does) is stopped with SIGTTIN.
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        let killer = GroupKiller::new(&child);

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("no stderr pipe on a piped spawn"))?;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                // Receiver gone = caller stopped caring; stop reading.
                if tx.send(line).is_err() {
                    break;
                }
            }
        });

        Ok(StreamingProcess {
            stderr: rx,
            child: Box::new(SystemStreamingChild { child, killer }),
        })
    }
}

struct SystemStreamingChild {
    child: tokio::process::Child,
    killer: GroupKiller,
}

#[async_trait]
impl StreamingChild for SystemStreamingChild {
    async fn wait(&mut self) -> std::io::Result<ExitOutcome> {
        let status = self.child.wait().await?;
        self.killer.disarm();
        Ok(ExitOutcome {
            success: status.success(),
            code: status.code(),
        })
    }

    async fn kill(&mut self) {
        self.killer.kill();
        let _ = self.child.kill().await;
    }
}

// ---------------------------------------------------------------------------
// Process-global runner + test override
// ---------------------------------------------------------------------------

static OVERRIDE: RwLock<Option<Arc<dyn CommandRunner>>> = RwLock::new(None);

fn default_runner() -> &'static Arc<dyn CommandRunner> {
    static DEFAULT: OnceLock<Arc<dyn CommandRunner>> = OnceLock::new();
    DEFAULT.get_or_init(|| Arc::new(SystemCommandRunner))
}

/// The currently active runner (test override if set, else the real one).
pub fn runner() -> Arc<dyn CommandRunner> {
    let guard = OVERRIDE.read().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(r) => Arc::clone(r),
        None => Arc::clone(default_runner()),
    }
}

/// Restores the previous runner when dropped. Held by tests for the duration
/// of the code under test.
#[must_use = "dropping the guard immediately restores the previous runner"]
pub struct TestRunnerGuard {
    prev: Option<Arc<dyn CommandRunner>>,
}

impl Drop for TestRunnerGuard {
    fn drop(&mut self) {
        *OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = self.prev.take();
    }
}

/// Install `r` as the process-global runner until the returned guard drops.
pub fn set_test_runner(r: Arc<dyn CommandRunner>) -> TestRunnerGuard {
    let mut w = OVERRIDE.write().unwrap_or_else(|e| e.into_inner());
    let prev = w.take();
    *w = Some(r);
    TestRunnerGuard { prev }
}

// ---------------------------------------------------------------------------
// Convenience free functions (call the active runner)
// ---------------------------------------------------------------------------

/// Run a command with default options via the active runner.
pub async fn output(program: &str, args: &[String]) -> std::io::Result<Output> {
    runner()
        .output(program, args, &CommandOpts::default())
        .await
}

/// Run a command with explicit options via the active runner.
pub async fn output_with(
    program: &str,
    args: &[String],
    opts: &CommandOpts,
) -> std::io::Result<Output> {
    runner().output(program, args, opts).await
}

/// Run a command whose args are string-ish (`&str`, `String`, …) with default
/// options. Convenience for call sites that build args from literals.
pub async fn output_argv<S: AsRef<str>>(program: &str, args: &[S]) -> std::io::Result<Output> {
    let owned: Vec<String> = args.iter().map(|a| a.as_ref().to_string()).collect();
    output(program, &owned).await
}

/// Spawn a detached process with default options via the active runner.
pub async fn spawn_detached(program: &str, args: &[String]) -> std::io::Result<()> {
    runner()
        .spawn_detached(program, args, &CommandOpts::default())
        .await
}

/// Spawn a detached process with explicit options via the active runner.
pub async fn spawn_detached_with(
    program: &str,
    args: &[String],
    opts: &CommandOpts,
) -> std::io::Result<()> {
    runner().spawn_detached(program, args, opts).await
}

// ---------------------------------------------------------------------------
// Fake implementation (for hermetic tests)
// ---------------------------------------------------------------------------

/// A canned response for one invocation, matched on `program + args`.
pub enum Canned {
    /// Return this exact output.
    Output(Output),
    /// Fail with a fresh `io::Error` of this kind + message.
    Err(std::io::ErrorKind, String),
}

/// Build an [`Output`] with the given exit code, stdout, and stderr.
fn make_output(code: i32, stdout: &str, stderr: &str) -> Output {
    // On Unix a wait-status's exit code lives in the high byte, so `code << 8`
    // yields an `ExitStatus` whose `.code()` is `Some(code)`.
    use std::os::unix::process::ExitStatusExt;
    Output {
        status: std::process::ExitStatus::from_raw(code << 8),
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

impl Canned {
    /// Exit 0 with the given stdout and empty stderr.
    pub fn ok_stdout(stdout: impl AsRef<str>) -> Self {
        Canned::Output(make_output(0, stdout.as_ref(), ""))
    }

    /// Exit 0 with the given stdout and stderr.
    pub fn ok(stdout: impl AsRef<str>, stderr: impl AsRef<str>) -> Self {
        Canned::Output(make_output(0, stdout.as_ref(), stderr.as_ref()))
    }

    /// A non-zero exit (command ran but failed) with the given streams.
    pub fn exit(code: i32, stdout: impl AsRef<str>, stderr: impl AsRef<str>) -> Self {
        Canned::Output(make_output(code, stdout.as_ref(), stderr.as_ref()))
    }

    /// The process could not be run at all (e.g. binary not found).
    pub fn io_error(kind: std::io::ErrorKind, msg: impl Into<String>) -> Self {
        Canned::Err(kind, msg.into())
    }

    fn realize(&self) -> std::io::Result<Output> {
        match self {
            Canned::Output(o) => Ok(Output {
                status: o.status,
                stdout: o.stdout.clone(),
                stderr: o.stderr.clone(),
            }),
            Canned::Err(kind, msg) => Err(std::io::Error::new(*kind, msg.clone())),
        }
    }
}

/// A test double for [`CommandRunner`]. Responses are keyed on the exact
/// `[program, args…]` invocation; queued responses for a key are consumed
/// FIFO and the last one repeats once the queue drains (so a poll that flips
/// `0` → `1` is scripted as three entries). Every invocation is recorded for
/// assertions. An un-scripted command yields an `io::Error` so gaps surface
/// loudly rather than passing silently.
pub struct FakeCommandRunner {
    responses: Mutex<HashMap<Vec<String>, VecDeque<Canned>>>,
    calls: Mutex<Vec<Vec<String>>>,
    streams: Mutex<HashMap<Vec<String>, VecDeque<CannedStream>>>,
    killed: Arc<Mutex<bool>>,
    stream_opts: Mutex<Vec<CommandOpts>>,
}

/// A scripted streamed child: what it prints, and how (or whether) it ends.
#[derive(Clone)]
struct CannedStream {
    lines: Vec<String>,
    /// `None` = never exits, so the caller's timeout fires.
    exit: Option<i32>,
}

impl Default for FakeCommandRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeCommandRunner {
    /// A fake with no scripted responses.
    pub fn new() -> Self {
        Self {
            responses: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
            streams: Mutex::new(HashMap::new()),
            killed: Arc::new(Mutex::new(false)),
            stream_opts: Mutex::new(Vec::new()),
        }
    }

    /// Queue a response for an exact `[program, args…]` invocation. Call
    /// repeatedly with the same key to script a sequence.
    pub fn expect(&self, cmd: &[&str], resp: Canned) -> &Self {
        let key: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();
        self.responses
            .lock()
            .expect("responses lock poisoned")
            .entry(key)
            .or_default()
            .push_back(resp);
        self
    }

    /// Every `[program, args…]` invocation seen so far, in order.
    pub fn recorded(&self) -> Vec<Vec<String>> {
        self.calls.lock().expect("calls lock poisoned").clone()
    }

    /// How many commands have been invoked.
    pub fn call_count(&self) -> usize {
        self.calls.lock().expect("calls lock poisoned").len()
    }

    /// Queue a streamed response: the stderr lines the child emits, then how
    /// it finishes. `exit` of `None` models a child that NEVER exits, so a
    /// caller's timeout-and-kill path runs for real against the fake.
    pub fn expect_stream(&self, cmd: &[&str], lines: &[&str], exit: Option<i32>) -> &Self {
        let key: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();
        self.streams
            .lock()
            .expect("streams lock poisoned")
            .entry(key)
            .or_default()
            .push_back(CannedStream {
                lines: lines.iter().map(|s| s.to_string()).collect(),
                exit,
            });
        self
    }

    /// Whether the caller killed the streamed child (after its own timeout).
    pub fn stream_was_killed(&self) -> bool {
        *self.killed.lock().expect("killed lock poisoned")
    }

    /// The [`CommandOpts`] each streamed spawn was given, in order — so a test
    /// can assert on the env and working directory a caller composed.
    pub fn recorded_stream_opts(&self) -> Vec<CommandOpts> {
        self.stream_opts
            .lock()
            .expect("stream_opts lock poisoned")
            .clone()
    }

    fn record_and_lookup(&self, program: &str, args: &[String]) -> std::io::Result<Output> {
        let mut key = Vec::with_capacity(args.len() + 1);
        key.push(program.to_string());
        key.extend_from_slice(args);
        self.calls
            .lock()
            .expect("calls lock poisoned")
            .push(key.clone());

        let mut map = self.responses.lock().expect("responses lock poisoned");
        match map.get_mut(&key) {
            Some(queue) if queue.len() > 1 => queue
                .pop_front()
                .expect("queue non-empty by guard")
                .realize(),
            // Last entry repeats — leave it in place.
            Some(queue) => queue
                .front()
                .expect("queue non-empty by presence")
                .realize(),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("FakeCommandRunner: no canned response for {key:?}"),
            )),
        }
    }
}

#[async_trait]
impl CommandRunner for FakeCommandRunner {
    async fn output(
        &self,
        program: &str,
        args: &[String],
        _opts: &CommandOpts,
    ) -> std::io::Result<Output> {
        self.record_and_lookup(program, args)
    }

    async fn spawn_detached(
        &self,
        program: &str,
        args: &[String],
        _opts: &CommandOpts,
    ) -> std::io::Result<()> {
        // Record the launch. Honour a scripted error for this key (to simulate
        // a spawn failure); otherwise a detached spawn just succeeds.
        let mut key = Vec::with_capacity(args.len() + 1);
        key.push(program.to_string());
        key.extend_from_slice(args);
        self.calls
            .lock()
            .expect("calls lock poisoned")
            .push(key.clone());

        let mut map = self.responses.lock().expect("responses lock poisoned");
        match map.get_mut(&key) {
            Some(queue) => match queue.front() {
                Some(Canned::Err(kind, msg)) => Err(std::io::Error::new(*kind, msg.clone())),
                _ => Ok(()),
            },
            None => Ok(()),
        }
    }
    async fn spawn_streaming(
        &self,
        program: &str,
        args: &[String],
        opts: &CommandOpts,
    ) -> std::io::Result<StreamingProcess> {
        self.stream_opts
            .lock()
            .expect("stream_opts lock poisoned")
            .push(opts.clone());
        let mut key = Vec::with_capacity(args.len() + 1);
        key.push(program.to_string());
        key.extend_from_slice(args);
        self.calls
            .lock()
            .expect("calls lock poisoned")
            .push(key.clone());

        let mut map = self.streams.lock().expect("streams lock poisoned");
        let canned = match map.get_mut(&key) {
            Some(queue) if queue.len() > 1 => queue.pop_front().expect("non-empty by guard"),
            Some(queue) => queue.front().expect("non-empty by presence").clone(),
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("FakeCommandRunner: no canned stream for {key:?}"),
                ))
            }
        };

        // Lines are delivered up front: the channel is buffered, so a caller
        // draining it after the wait sees exactly what a real child printed
        // before exiting.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        for line in canned.lines {
            let _ = tx.send(line);
        }
        drop(tx);

        Ok(StreamingProcess {
            stderr: rx,
            child: Box::new(FakeStreamingChild {
                exit: canned.exit,
                killed: Arc::clone(&self.killed),
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Running, as opposed to gone or a zombie.
    fn alive(pid: &str) -> bool {
        std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .map(|o| {
                let stat = String::from_utf8_lossy(&o.stdout);
                let stat = stat.trim();
                !stat.is_empty() && !stat.starts_with('Z')
            })
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn dropping_a_streamed_child_kills_what_it_started() {
        let dir = std::env::temp_dir().join(format!("golem-group-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let pid_file = dir.join("grandchild.pid");
        let script = format!("sleep 30 & echo $! > {}; wait", pid_file.display());
        let child = SystemCommandRunner
            .spawn_streaming("sh", &argv(&["-c", &script]), &CommandOpts::default())
            .await
            .expect("spawn");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let pid = loop {
            if let Some(pid) = std::fs::read_to_string(&pid_file)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
            {
                break pid;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the script SHALL start"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        assert!(alive(&pid));

        drop(child);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while alive(&pid) {
            assert!(
                std::time::Instant::now() < deadline,
                "the script's own child SHALL die with it"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_streamed_child_that_exits_leaves_its_background_children_running() {
        let dir = std::env::temp_dir().join(format!("golem-group-bg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let pid_file = dir.join("bg.pid");
        let script = format!(
            "sleep 30 >/dev/null 2>&1 & echo $! > {}",
            pid_file.display()
        );
        let mut child = SystemCommandRunner
            .spawn_streaming("sh", &argv(&["-c", &script]), &CommandOpts::default())
            .await
            .expect("spawn");
        assert!(child.child.wait().await.expect("wait").success);
        drop(child);
        let pid = std::fs::read_to_string(&pid_file)
            .expect("pid")
            .trim()
            .to_string();
        assert!(
            alive(&pid),
            "a background process started on purpose SHALL survive"
        );
        let _ = std::process::Command::new("kill").arg(&pid).status();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_base_env_replaces_this_process_environment() {
        // Set here and absent from the base: a daemon's own variable.
        std::env::set_var("GOLEM_BASE_ENV_PROBE", "leak");
        let opts = CommandOpts {
            base_env: Some(vec![
                ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
                ("K".into(), "v".into()),
            ]),
            env: vec![("E".into(), "e".into())],
            ..CommandOpts::default()
        };
        let out = SystemCommandRunner
            .output(
                "sh",
                &argv(&["-c", "echo \"${GOLEM_BASE_ENV_PROBE:-none}:$K:$E\""]),
                &opts,
            )
            .await
            .expect("sh");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "none:v:e");
    }

    const STDIN_PROBE_ENV: &str = "STDIN_PROBE_CHILD";
    const READ_STDIN: &str = "if read -r line; then echo \"got:$line\" >&2; else echo eof >&2; fi";

    /// The child half of `spawned_children_get_an_empty_stdin`. It does
    /// nothing unless that test re-runs this binary with data on stdin.
    /// Its `RESULT` lines go to stdout, which the parent test reads.
    #[allow(clippy::disallowed_macros)]
    #[tokio::test]
    async fn stdin_probe_child() {
        if std::env::var_os(STDIN_PROBE_ENV).is_none() {
            return;
        }
        let runner = SystemCommandRunner;
        let args = argv(&["-c", READ_STDIN]);

        let mut streaming = runner
            .spawn_streaming("sh", &args, &CommandOpts::default())
            .await
            .expect("spawn_streaming");
        let line = streaming.stderr.recv().await.unwrap_or_default();
        println!("RESULT streaming={line}");

        let log = std::env::temp_dir().join(format!("stdin-probe-{}.log", std::process::id()));
        let opts = CommandOpts {
            log_file: Some(log.to_string_lossy().into_owned()),
            ..CommandOpts::default()
        };
        runner
            .spawn_detached("sh", &args, &opts)
            .await
            .expect("spawn_detached");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut detached = String::new();
        while detached.trim().is_empty() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            detached = std::fs::read_to_string(&log).unwrap_or_default();
        }
        let _ = std::fs::remove_file(&log);
        println!("RESULT detached={}", detached.trim());
    }

    /// Re-runs this test binary with data on its stdin. The test runner's own
    /// stdin is usually already empty, so the children could only prove they
    /// don't inherit it from a parent that has something to read.
    #[test]
    fn spawned_children_get_an_empty_stdin() {
        use std::io::Write;
        let mut parent = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "command::tests::stdin_probe_child",
                "--exact",
                "--nocapture",
            ])
            .env(STDIN_PROBE_ENV, "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("re-run the test binary");
        parent
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(b"secret\nsecret\n")
            .expect("feed stdin");
        let out = parent.wait_with_output().expect("probe finishes");
        let stdout = String::from_utf8_lossy(&out.stdout);
        for path in ["streaming", "detached"] {
            assert!(
                stdout.contains(&format!("RESULT {path}=eof")),
                "a {path} child SHALL read EOF, not the parent's stdin:\n{stdout}"
            );
        }
    }

    fn stdout_of(o: &Output) -> String {
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    // The fake's own behaviour is tested by calling it directly (no global),
    // so these run parallel-safe under any test runner. Only the
    // set_test_runner/guard mechanism (below) touches the process-global.

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn fake_matches_on_program_and_args() {
        let fake = FakeCommandRunner::new();
        fake.expect(
            &["adb", "devices"],
            Canned::ok_stdout("emulator-5554\tdevice"),
        );

        let out = fake
            .output("adb", &argv(&["devices"]), &CommandOpts::default())
            .await
            .expect("scripted command runs");
        assert_eq!(stdout_of(&out), "emulator-5554\tdevice");
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(fake.recorded(), vec![argv(&["adb", "devices"])]);
    }

    #[tokio::test]
    async fn fake_queue_consumes_fifo_then_repeats_last() {
        let fake = FakeCommandRunner::new();
        let cmd = ["adb", "shell", "getprop", "sys.boot_completed"];
        fake.expect(&cmd, Canned::ok_stdout("0"));
        fake.expect(&cmd, Canned::ok_stdout("0"));
        fake.expect(&cmd, Canned::ok_stdout("1"));

        let args = argv(&cmd[1..]);
        let read = || async {
            stdout_of(
                &fake
                    .output("adb", &args, &CommandOpts::default())
                    .await
                    .expect("scripted"),
            )
        };
        assert_eq!(read().await, "0");
        assert_eq!(read().await, "0");
        assert_eq!(read().await, "1");
        // Drained: last response repeats.
        assert_eq!(read().await, "1");
    }

    #[tokio::test]
    async fn fake_unscripted_command_errors() {
        let fake = FakeCommandRunner::new();
        let err = fake
            .output("adb", &argv(&["reboot"]), &CommandOpts::default())
            .await
            .expect_err("unscripted command must error");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn fake_can_return_nonzero_and_io_error() {
        let fake = FakeCommandRunner::new();
        fake.expect(&["adb", "install", "bad"], Canned::exit(1, "", "failure"));
        fake.expect(
            &["missing-bin"],
            Canned::io_error(std::io::ErrorKind::NotFound, "no such binary"),
        );

        let out = fake
            .output("adb", &argv(&["install", "bad"]), &CommandOpts::default())
            .await
            .expect("command ran (non-zero exit is still Ok)");
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(String::from_utf8_lossy(&out.stderr), "failure");

        let err = fake
            .output("missing-bin", &[], &CommandOpts::default())
            .await
            .expect_err("io error surfaces");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn fake_spawn_detached_records_and_defaults_ok() {
        let fake = FakeCommandRunner::new();
        fake.spawn_detached(
            "emulator",
            &argv(&["-avd", "Pixel"]),
            &CommandOpts::default(),
        )
        .await
        .expect("detached spawn defaults to Ok");
        assert_eq!(fake.recorded(), vec![argv(&["emulator", "-avd", "Pixel"])]);
    }

    // The only test that exercises the process-global override. It is the sole
    // global-touching test in this crate, so it is safe under a threaded
    // runner; cross-crate override tests rely on nextest's process isolation.
    #[tokio::test]
    async fn guard_restores_previous_runner_on_drop() {
        let outer = Arc::new(FakeCommandRunner::new());
        outer.expect(&["echo", "outer"], Canned::ok_stdout("outer"));
        let outer_guard = set_test_runner(outer.clone());
        {
            let inner = Arc::new(FakeCommandRunner::new());
            inner.expect(&["echo", "inner"], Canned::ok_stdout("inner"));
            let _inner_guard = set_test_runner(inner);
            assert_eq!(
                stdout_of(&output("echo", &argv(&["inner"])).await.expect("inner")),
                "inner"
            );
        }
        // Inner guard dropped — outer runner is active again.
        assert_eq!(
            stdout_of(&output("echo", &argv(&["outer"])).await.expect("outer")),
            "outer"
        );
        drop(outer_guard);
    }
}

struct FakeStreamingChild {
    /// `None` = never exits.
    exit: Option<i32>,
    killed: Arc<Mutex<bool>>,
}

#[async_trait]
impl StreamingChild for FakeStreamingChild {
    async fn wait(&mut self) -> std::io::Result<ExitOutcome> {
        match self.exit {
            Some(code) => Ok(ExitOutcome {
                success: code == 0,
                code: Some(code),
            }),
            // Never resolves, so the caller's own timeout is what ends the
            // wait — the only way a fake can exercise a timeout-and-kill path.
            None => std::future::pending().await,
        }
    }

    async fn kill(&mut self) {
        *self.killed.lock().expect("killed lock poisoned") = true;
    }
}
