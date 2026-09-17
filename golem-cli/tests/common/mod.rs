//! Shared harness for in-process integration tests.
//!
//! These tests exercise the end-to-end *composition* that unit tests can't:
//! CLI args → `SuiteConfig` → in-process orchestrator server → client
//! `submit_and_wait` → event stream → renderer → result files → exit code.
//! They run entirely in-process against [`golem_driver::stub::StubDriver`]
//! (no device, no companion, no install) via the hidden `--stub` flag, so
//! they're fast and deterministic.
//!
//! Each run points `$HOME` and the working directory at a fresh temp
//! project: `$HOME` isolates the orchestrator's unix socket
//! (`$HOME/.golem/golem.sock`) so parallel test processes don't collide,
//! and the cwd sets flow discovery + project root. Because both are
//! process-global, a run holds [`ENV_LOCK`] for the whole redirect+run
//! window — this keeps the tests correct even under plain `cargo test`
//! (threads in one process); under `cargo nextest` (process-per-test) the
//! lock is uncontended.
//!
//! Unix only (fd redirection + unix socket) — matches golem's supported
//! dev platforms.
//!
//! Runtime note: each run is ~2-3s wall (nextest flags it SLOW) but nearly
//! all of that is *waiting*, not CPU — debug-binary/runtime startup, the
//! post-launch settle gate, and the ~1s self-loopback client drain. That's
//! the price of exercising the real CLI→server→renderer→file→exit pipeline
//! in one shot; there's no faster way to cover the composition these tests
//! exist for, and they run concurrently under nextest.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::Mutex;

use clap::Parser;

/// Serialises the cwd/`$HOME`/fd-capture window across tests so concurrent
/// runs in the same process (plain `cargo test`) don't clobber each other's
/// global state.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Outcome of an in-process CLI run.
pub struct RunResult {
    /// Process exit code `run_cli` returned (0 = ok, 1 = a flow failed).
    pub code: i32,
    /// Everything written to fd 1 during the run.
    pub stdout: String,
    /// Everything written to fd 2 during the run.
    pub stderr: String,
    /// The temp project root (kept alive by `_tmp`); results land under
    /// `dir/.golem/results` unless `--output-dir` overrides it.
    pub dir: PathBuf,
    _tmp: tempfile::TempDir,
}

/// A minimal `golem.toml` naming the stub app. The stub bypasses install,
/// so no install script is needed.
fn golem_toml() -> String {
    format!(
        "[[apps]]\nname = \"app\"\nbundle = \"{}\"\n",
        golem_driver::stub::STUB_BUNDLE_ID
    )
}

/// A one-block fixture flow that asserts the stub's target element is
/// visible. It passes when the stub serves the pass tree and fails (through
/// the real assert path) when the stub serves the target-less fail tree.
pub fn fixture_flow() -> String {
    format!(
        r#"[flow]
name = "Stub fixture"

[flow.options]
# Single-axis device matrix (one os) → exactly one FlowRun per repeat, so
# no coverage strategy is needed and `--repeat` fans cleanly to N runs.
# Short step timeout so a scripted-fail run (target-less tree) fails its
# assert in a few hundred ms instead of polling the 10s default.
step_timeout = 400
# The stub tree isn't an accessibility fixture — turn the audit off so it
# doesn't fail runs independently of the scripted pass/fail logic.
a11y = "off"
# Recording/perf add device-shaped overhead with no value against a stub.
perf = false

[[flow.apps]]
name = "app"
bundle = "{bundle}"
[[flow.apps.devices]]
os = ["android:latest"]
type = "phone"

[[block]]
name = "check"
steps = [
  {{ action = "assert_visible", on_text = "{target}", timeout = 400 }},
]
"#,
        bundle = golem_driver::stub::STUB_BUNDLE_ID,
        target = golem_driver::stub::STUB_TARGET_TEXT,
    )
}

/// A flow whose only step drives the browser, against a `data:` URL so no
/// server is needed. Used to prove the CLI actually reaches the browser
/// dispatch — a path no unit test covers, because a unit test compiles the
/// browser feature in itself and so cannot notice the binary shipping without
/// it.
pub fn browser_flow() -> String {
    format!(
        r#"[flow]
name = "Stub browser"

[flow.options]
step_timeout = 5000
a11y = "off"
perf = false

[[flow.apps]]
name = "app"
bundle = "{bundle}"
[[flow.apps.devices]]
os = ["android:latest"]
type = "phone"

[[block]]
name = "web"
steps = [
  {{ action = "browse_navigate", url = "data:text/html,<h1>Greetings</h1>" }},
  {{ action = "browse_assert_text", selector = "h1", text = "Greetings" }},
]
"#,
        bundle = golem_driver::stub::STUB_BUNDLE_ID,
    )
}

/// A flow with two device axes, so the plan fans it out into one FlowRun per
/// axis. Coverage strategy then decides how many of those actually run —
/// which is the thing under test, so this flow must NOT be run with a
/// `--platform` override (see [`StubOpts::no_platform_override`]).
pub fn coverage_flow() -> String {
    format!(
        r#"[flow]
name = "Stub coverage"

[flow.options]
step_timeout = 400
a11y = "off"
perf = false

[[flow.apps]]
name = "app"
bundle = "{bundle}"
[[flow.apps.devices]]
os = ["android:latest", "ios:latest"]
type = "phone"

[[block]]
name = "check"
steps = [
  {{ action = "assert_visible", on_text = "{target}", timeout = 400 }},
]
"#,
        bundle = golem_driver::stub::STUB_BUNDLE_ID,
        target = golem_driver::stub::STUB_TARGET_TEXT,
    )
}

/// A flow whose one step sleeps, so two FlowRuns overlap measurably when
/// they run concurrently and are provably disjoint when they don't. The
/// pass/fail fixture completes inside a millisecond, which can't tell the
/// two apart.
pub fn slow_flow() -> String {
    format!(
        r#"[flow]
name = "Stub slow"

[flow.options]
step_timeout = 5000
a11y = "off"
perf = false

[[flow.apps]]
name = "app"
bundle = "{bundle}"
[[flow.apps.devices]]
os = ["android:latest"]
type = "phone"

[[block]]
name = "dawdle"
steps = [
  {{ action = "bash", run = "sleep 0.5" }},
]
"#,
        bundle = golem_driver::stub::STUB_BUNDLE_ID,
    )
}

/// Build a temp project (golem.toml + the fixture flow + a stub script),
/// then run `golem run <flow> --stub <script> --platform android <extra>`
/// in-process against the stub driver, capturing fd-level stdout/stderr.
///
/// `stub_script_toml` is the `--stub` file body (e.g. `"fail_on_runs = [2]"`;
/// `""` = every run passes).
pub fn run_stub(stub_script_toml: &str, extra_args: &[&str]) -> RunResult {
    run_stub_opts(stub_script_toml, extra_args, StubOpts::default())
}

/// Knobs the default `run_stub` fixes. Kept out of `run_stub`'s signature so
/// the existing call sites stay readable — only the composition tests need
/// to vary these.
#[derive(Default, Clone, Copy)]
pub struct StubOpts {
    /// Connect to a real orchestrator daemon started in-process first, rather
    /// than letting the CLI spin up its own server and self-connect. Both are
    /// production paths; this is what tells them apart.
    pub daemon: bool,
    /// Drop the `--platform android` override, so a flow's own device axes
    /// survive into the plan (what coverage fan-out is about).
    pub no_platform_override: bool,
}

/// `run_stub` with the knobs exposed. See [`StubOpts`].
pub fn run_stub_opts(stub_script_toml: &str, extra_args: &[&str], opts: StubOpts) -> RunResult {
    let _guard = ENV_LOCK.lock().expect("env lock");

    let tmp = tempfile::TempDir::new().expect("temp dir");
    let root = tmp.path().to_path_buf();
    std::fs::write(root.join("golem.toml"), golem_toml()).expect("write golem.toml");
    std::fs::write(root.join("fixture.test.toml"), fixture_flow()).expect("write flow");
    std::fs::write(root.join("browser.test.toml"), browser_flow()).expect("write browser flow");
    std::fs::write(root.join("slow.test.toml"), slow_flow()).expect("write slow flow");
    std::fs::write(root.join("coverage.test.toml"), coverage_flow()).expect("write coverage flow");
    std::fs::write(root.join("stub.toml"), stub_script_toml).expect("write stub script");

    // Point cwd + $HOME at the temp project. Saved and restored around the
    // run so the test process is left as it was found.
    let prev_cwd = std::env::current_dir().ok();
    let prev_home = std::env::var_os("HOME");
    std::env::set_current_dir(&root).expect("set cwd");
    std::env::set_var("HOME", &root);

    let flow = extra_args
        .iter()
        .position(|a| *a == "--flow")
        .and_then(|i| extra_args.get(i + 1))
        .copied()
        .unwrap_or("fixture.test.toml");
    let extra_args: Vec<&str> = {
        let mut kept = Vec::new();
        let mut skip = 0;
        for (i, arg) in extra_args.iter().enumerate() {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            if *arg == "--flow" {
                skip = 1;
                continue;
            }
            let _ = i;
            kept.push(*arg);
        }
        kept
    };

    let mut argv: Vec<String> = vec![
        "golem".into(),
        "run".into(),
        flow.into(),
        "--stub".into(),
        "stub.toml".into(),
    ];
    if !opts.no_platform_override {
        argv.push("--platform".into());
        argv.push("android".into());
    }
    argv.extend(extra_args.iter().map(|s| s.to_string()));
    let cli = golem_cli::cli::Cli::parse_from(&argv);

    // Capture stdout/stderr at the fd level: the streamed human renderer
    // writes from spawned tasks, so a process-fd redirect catches it where
    // threading a writer wouldn't. Redirect to temp files (not pipes) to
    // avoid any buffer-full deadlock.
    let out_cap = FdCapture::start(libc::STDOUT_FILENO, root.join("cap_out"));
    let err_cap = FdCapture::start(libc::STDERR_FILENO, root.join("cap_err"));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    // With `daemon`, a server is listening before the CLI runs, so the client
    // connects to it instead of spinning up its own — the same split a user
    // gets from `golem serve` in another terminal. The server owns the socket
    // for the run and is dropped (socket cleaned up) before HOME is restored.
    let code = rt.block_on(async {
        let server = if opts.daemon {
            Some(
                golem_cli::orchestrator::start_server()
                    .await
                    .expect("daemon SHALL start"),
            )
        } else {
            None
        };
        let code = golem_cli::run_cli(cli)
            .await
            .expect("run_cli SHALL not error");
        drop(server);
        code
    });

    let stdout = out_cap.finish();
    let stderr = err_cap.finish();

    // Restore process globals.
    if let Some(cwd) = prev_cwd {
        let _ = std::env::set_current_dir(cwd);
    }
    match prev_home {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }

    RunResult {
        code,
        stdout,
        stderr,
        dir: root,
        _tmp: tmp,
    }
}

/// Read + parse a `results.json` under the run's results directory.
/// `subdir` is e.g. `"run_1"` for `--repeat`, or `""` for the flat layout.
pub fn read_results_json(res: &RunResult, subdir: &str) -> serde_json::Value {
    let mut path = res.dir.join(".golem").join("results");
    if !subdir.is_empty() {
        path = path.join(subdir);
    }
    path = path.join("results.json");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("results.json SHALL be valid JSON")
}

/// Redirects one fd to a file for the lifetime of a run, then restores it.
struct FdCapture {
    target_fd: i32,
    saved_fd: i32,
    path: PathBuf,
}

impl FdCapture {
    fn start(target_fd: i32, path: PathBuf) -> Self {
        let file = std::fs::File::create(&path).expect("create capture file");
        // SAFETY: dup/dup2 on valid fds; `file` stays alive until dup2 has
        // installed its fd as `target_fd`.
        let saved_fd = unsafe { libc::dup(target_fd) };
        assert!(saved_fd >= 0, "dup({target_fd}) failed");
        let rc = unsafe { libc::dup2(file.as_raw_fd(), target_fd) };
        assert!(rc >= 0, "dup2 onto {target_fd} failed");
        Self {
            target_fd,
            saved_fd,
            path,
        }
    }

    fn finish(self) -> String {
        // Flush Rust's own buffers before restoring the fd so buffered
        // stdout writes land in the capture file, not post-restore.
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        // SAFETY: restore the original fd and drop our saved copy.
        unsafe {
            libc::dup2(self.saved_fd, self.target_fd);
            libc::close(self.saved_fd);
        }
        let mut s = String::new();
        if let Ok(mut f) = std::fs::File::open(&self.path) {
            let _ = f.read_to_string(&mut s);
        }
        s
    }
}
