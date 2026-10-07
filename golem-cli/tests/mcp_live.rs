//! `golem mcp` against a live device: a scripted MCP client, not an LLM,
//! drives the real binary over stdio against `test-app`.
//!
//! Every test is `#[ignore]`: it needs a booted device with the test app
//! and its companion installed (run any e2e flow once first). Run them one
//! platform at a time:
//!
//! ```text
//! GOLEM_E2E_PLATFORM=android cargo nextest run -p golem-cli --test mcp_live --run-ignored only
//! ```
//!
//! `GOLEM_E2E_DEVICE` picks the device when more than one is booted. The
//! SIGKILL test also runs `golem run` on the other platform, so it needs a
//! device of each.
//!
//! Each test starts its own daemon on a private socket, so a daemon from
//! another build is never drained.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};

const GOLEM: &str = env!("CARGO_BIN_EXE_golem");

struct Live {
    platform: String,
    device: Option<String>,
    root: PathBuf,
    /// Holds the private daemon socket.
    sock_dir: tempfile::TempDir,
}

impl Live {
    fn new() -> Live {
        let platform = std::env::var("GOLEM_E2E_PLATFORM")
            .expect("set GOLEM_E2E_PLATFORM=android or ios to run the live MCP tests");
        assert!(
            platform == "android" || platform == "ios",
            "GOLEM_E2E_PLATFORM must be android or ios, not {platform}"
        );
        Live {
            platform,
            device: std::env::var("GOLEM_E2E_DEVICE").ok(),
            root: Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("workspace root")
                .to_path_buf(),
            // A unix socket path has a ~100 byte limit: keep it short.
            sock_dir: tempfile::Builder::new()
                .prefix("gml")
                .tempdir_in("/tmp")
                .expect("socket dir"),
        }
    }

    fn socket(&self) -> PathBuf {
        self.sock_dir.path().join("d.sock")
    }

    fn other_platform(&self) -> &'static str {
        if self.platform == "android" {
            "ios"
        } else {
            "android"
        }
    }

    /// A scratch directory inside the project, so a flow exported there
    /// finds `golem.toml`.
    fn scratch(&self) -> tempfile::TempDir {
        let dir = self.root.join("target/mcp-live");
        std::fs::create_dir_all(&dir).expect("scratch dir");
        tempfile::tempdir_in(dir).expect("scratch")
    }

    async fn mcp(&self, extra: &[&str]) -> Mcp {
        let mut child = tokio::process::Command::new(GOLEM)
            .arg("mcp")
            .args(["--project", &self.root.display().to_string()])
            .args(extra)
            .env("GOLEM_SOCKET", self.socket())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn golem mcp");
        let stdout = child.stdout.take().expect("stdout");
        let stdin = child.stdin.take().expect("stdin");
        let client = ().serve((stdout, stdin)).await.expect("MCP handshake");
        Mcp { client, child }
    }

    /// `session_open` arguments for this platform and device.
    fn on_device(&self, mut args: serde_json::Value) -> serde_json::Value {
        args["platform"] = serde_json::json!(self.platform);
        if let Some(d) = &self.device {
            args["device"] = serde_json::json!(d);
        }
        args
    }

    /// `session_open` on this platform and device, waiting out `pending`.
    async fn open(&self, mcp: &Mcp, args: serde_json::Value) -> String {
        let mut t = mcp.ok("session_open", self.on_device(args)).await;
        while t.starts_with("pending") {
            t = mcp.ok("wait", serde_json::json!({ "timeout_s": 30 })).await;
        }
        assert!(t.starts_with("session open"), "{t}");
        t
    }

    fn golem_run(&self, flow: &Path, platform: &str) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(GOLEM);
        cmd.arg("run")
            .arg(flow)
            .args(["--no-build", "--platform", platform])
            .current_dir(&self.root)
            .env("GOLEM_SOCKET", self.socket())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    }
}

struct Mcp {
    client: RunningService<RoleClient, ()>,
    child: tokio::process::Child,
}

impl Mcp {
    /// The tool's text, and whether it is an error result.
    async fn call(&self, tool: &'static str, args: serde_json::Value) -> (String, bool) {
        let mut params = CallToolRequestParams::new(tool);
        if let serde_json::Value::Object(map) = args {
            params = params.with_arguments(map);
        }
        let result = self
            .client
            .call_tool(params)
            .await
            .unwrap_or_else(|e| panic!("{tool}: {e:?}"));
        let v = serde_json::to_value(&result).expect("json");
        let text = v["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        eprintln!("--- {tool}\n{}", text.trim_end());
        (text, v["isError"].as_bool().unwrap_or(false))
    }

    async fn ok(&self, tool: &'static str, args: serde_json::Value) -> String {
        let (t, err) = self.call(tool, args).await;
        assert!(!err, "{tool} failed: {t}");
        t
    }
}

/// The UDID or serial in `session open · <name> (<id>) · app …`.
fn device_id(opened: &str) -> String {
    opened
        .split_once(" (")
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(id, _)| id.to_string())
        .unwrap_or_else(|| panic!("no device id in {opened}"))
}

/// Whether `outer` holds every line of `inner`, in order.
fn keeps_lines_in_order(outer: &str, inner: &str) -> bool {
    let mut lines = outer.lines();
    inner.lines().all(|want| lines.any(|l| l == want))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a live device: GOLEM_E2E_PLATFORM=android|ios"]
async fn live_mcp_authors_a_flow_that_golem_run_passes() {
    let live = Live::new();
    let scratch = live.scratch();
    let out = scratch.path().join("authored.test.toml");
    let mcp = live.mcp(&[]).await;

    live.open(&mcp, serde_json::json!({ "app": "app" })).await;
    mcp.ok(
        "act",
        serde_json::json!({ "step": r#"{ action = "launch", app = "app", restart = true }"#, "comment": "Start clean" }),
    )
    .await;
    let probe = mcp
        .ok(
            "probe",
            serde_json::json!({ "selector": r#"{ on_text = "+" }"# }),
        )
        .await;
    assert!(probe.contains("1 visible match"), "{probe}");
    mcp.ok(
        "act",
        serde_json::json!({ "step": r#"{ action = "tap", on_text = "+" }"#, "comment": "Count one" }),
    )
    .await;
    mcp.ok(
        "act",
        serde_json::json!({ "step": r#"{ action = "assert_visible", on_text = "1", on_below = "Counter" }"# }),
    )
    .await;
    let (miss, err) = mcp
        .call(
            "act",
            serde_json::json!({ "step": r#"{ action = "tap", on_text = "No such button", timeout = 1000 }"# }),
        )
        .await;
    assert!(!err, "a failed step is a result, not a tool error: {miss}");

    let draft = mcp.ok("draft_show", serde_json::json!({})).await;
    assert!(draft.contains("# Count one"), "{draft}");
    assert!(
        draft.contains(r#"{ action = "tap", on_text = "+" },"#),
        "{draft}"
    );
    assert!(
        !draft.contains("No such button"),
        "a failed step SHALL stay out of the draft: {draft}"
    );

    let exported = mcp
        .ok(
            "export_flow",
            serde_json::json!({ "path": out.display().to_string() }),
        )
        .await;
    assert!(exported.starts_with("exported "), "{exported}");
    mcp.ok("session_close", serde_json::json!({})).await;
    drop(mcp);

    let run = live
        .golem_run(&out, &live.platform)
        .output()
        .await
        .expect("golem run");
    assert!(
        run.status.success(),
        "the exported flow SHALL pass under golem run:\n{}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a live device: GOLEM_E2E_PLATFORM=android|ios"]
async fn live_mcp_export_of_a_flow_session_keeps_its_comments_and_format() {
    let live = Live::new();
    let scratch = live.scratch();
    let original = std::fs::read_to_string(live.root.join("e2e/tap.test.toml")).expect("tap flow");
    let copy = scratch.path().join("tap.test.toml");
    std::fs::write(&copy, &original).expect("copy");
    let mcp = live.mcp(&[]).await;

    let opened = live
        .open(
            &mcp,
            serde_json::json!({ "flow": copy.display().to_string(), "stop_at": "tap_interactions" }),
        )
        .await;
    assert!(opened.contains("before tap_interactions:1"), "{opened}");
    mcp.ok(
        "act",
        serde_json::json!({ "step": r#"{ action = "assert_visible", on_text = "Submit" }"#, "comment": "The form shows" }),
    )
    .await;
    mcp.ok(
        "export_flow",
        serde_json::json!({ "path": copy.display().to_string() }),
    )
    .await;
    mcp.ok("session_close", serde_json::json!({ "teardown": false }))
        .await;

    let exported = std::fs::read_to_string(&copy).expect("exported");
    assert!(
        keeps_lines_in_order(&exported, &original),
        "every line of the flow SHALL survive the export unchanged:\n{exported}"
    );
    let added: Vec<&str> = {
        let mut keep = original.lines().peekable();
        exported
            .lines()
            .filter(|l| {
                if keep.peek() == Some(l) {
                    keep.next();
                    false
                } else {
                    true
                }
            })
            .collect()
    };
    assert_eq!(
        added.iter().map(|l| l.trim()).collect::<Vec<_>>(),
        vec![
            "# The form shows",
            r#"{ action = "assert_visible", on_text = "Submit" },"#
        ],
        "the export SHALL add only the new step and its comment:\n{exported}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a live device: GOLEM_E2E_PLATFORM=android|ios"]
async fn live_mcp_app_logs_show_the_launch_line_and_a_crash() {
    let live = Live::new();
    let mcp = live.mcp(&[]).await;

    let opened = live.open(&mcp, serde_json::json!({ "app": "app" })).await;
    let device = device_id(&opened);
    mcp.ok(
        "act",
        serde_json::json!({ "step": r#"{ action = "launch", app = "app", restart = true }"# }),
    )
    .await;
    let logs = mcp
        .ok(
            "app_logs",
            serde_json::json!({ "filter": "golem test app started" }),
        )
        .await;
    assert!(logs.contains("golem test app started"), "{logs}");

    crash_test_app(&live.platform, &device).await;
    // SpringBoard's exit line can reach the log store seconds late.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let logs = mcp
            .ok("app_logs", serde_json::json!({ "since": 60, "limit": 5 }))
            .await;
        let seen =
            logs.contains("crash[") && (live.platform == "android" || logs.contains("SIGABRT"));
        if seen {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no crash in the app log after 20 s:\n{logs}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    mcp.ok("session_close", serde_json::json!({})).await;
}

async fn crash_test_app(platform: &str, device: &str) {
    let ok = if platform == "android" {
        tokio::process::Command::new("adb")
            .args(["-s", device, "shell", "am", "crash", "fail.golem.test"])
            .status()
            .await
            .expect("adb")
            .success()
    } else {
        let list = tokio::process::Command::new("xcrun")
            .args(["simctl", "spawn", device, "launchctl", "list"])
            .output()
            .await
            .expect("simctl");
        let list = String::from_utf8_lossy(&list.stdout);
        let pid = list
            .lines()
            .find(|l| l.contains("UIKitApplication:fail.golem.test["))
            .and_then(|l| l.split_whitespace().next())
            .filter(|p| p.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or_else(|| panic!("the test app is not running:\n{list}"))
            .to_string();
        tokio::process::Command::new("kill")
            .args(["-ABRT", &pid])
            .status()
            .await
            .expect("kill")
            .success()
    };
    assert!(ok, "could not crash the test app");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a live device: GOLEM_E2E_PLATFORM=android|ios"]
async fn live_mcp_a_long_open_answers_pending_then_wait_gives_the_result() {
    let live = Live::new();
    let mcp = live.mcp(&["--soft-timeout", "5"]).await;
    let first = mcp
        .ok(
            "session_open",
            live.on_device(serde_json::json!({ "flow": "e2e/tap.test.toml" })),
        )
        .await;
    assert!(
        first.starts_with("pending · op 1 session_open"),
        "a flow longer than the soft timeout SHALL answer pending: {first}"
    );
    let (busy, err) = mcp
        .call(
            "act",
            serde_json::json!({ "step": r#"{ action = "tap", on_text = "+" }"# }),
        )
        .await;
    assert!(
        err && busy.starts_with("busy · op 1 session_open"),
        "{busy}"
    );
    let mut t = mcp
        .ok("wait", serde_json::json!({ "timeout_s": 120 }))
        .await;
    while t.starts_with("pending") {
        t = mcp
            .ok("wait", serde_json::json!({ "timeout_s": 120 }))
            .await;
    }
    assert!(t.starts_with("session open"), "{t}");
    let status = mcp.ok("status", serde_json::json!({})).await;
    assert!(
        status.contains("last: op 1 session_open"),
        "status SHALL show the finished open: {status}"
    );
    mcp.ok("session_close", serde_json::json!({ "teardown": false }))
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a live device of each platform: GOLEM_E2E_PLATFORM=android|ios"]
async fn live_mcp_a_killed_client_frees_its_device_and_a_run_passes() {
    let live = Live::new();
    let mut mcp = live.mcp(&[]).await;
    live.open(&mcp, serde_json::json!({ "app": "app" })).await;

    let run = live
        .golem_run(&live.root.join("e2e/tap.test.toml"), live.other_platform())
        .spawn()
        .expect("golem run");
    tokio::time::sleep(Duration::from_secs(3)).await;
    mcp.child.start_kill().expect("SIGKILL golem mcp");
    let _ = mcp.child.wait().await;

    let run = run.wait_with_output().await.expect("golem run");
    assert!(
        run.status.success(),
        "a run on another device SHALL pass when an MCP client dies:\n{}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    // A device still leased to the dead session would leave this open
    // waiting, and the answer would be pending.
    let next = live.mcp(&["--soft-timeout", "30"]).await;
    let opened = next
        .ok(
            "session_open",
            live.on_device(serde_json::json!({ "app": "app" })),
        )
        .await;
    assert!(
        opened.starts_with("session open"),
        "the killed client's device SHALL be free: {opened}"
    );
    next.ok("session_close", serde_json::json!({})).await;
}

#[test]
fn a_device_id_comes_from_the_open_line() {
    assert_eq!(
        device_id("session open · iPhone 17 (B910-54EB) · app fail.golem.test\n"),
        "B910-54EB"
    );
}

#[test]
fn line_order_check_allows_insertions_only() {
    assert!(keeps_lines_in_order("a\nx\nb\nc", "a\nb\nc"));
    assert!(!keeps_lines_in_order("a\nc\nb", "a\nb\nc"));
}
