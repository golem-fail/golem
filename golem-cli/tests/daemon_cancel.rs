//! A client that disconnects mid-run cancels its suite in the daemon: the
//! flow is aborted, the processes it started are killed, and the daemon
//! keeps serving other clients.

#![cfg(unix)]

mod common;

use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn long_flow() -> String {
    common::slow_flow().replace(
        r#"run = "sleep 0.5""#,
        r#"run = "sleep 30 & echo $! > step.pid; wait""#,
    )
}

/// Running, as opposed to gone or a zombie: a killed child stays a zombie
/// until its parent reaps it, and signal 0 still finds a zombie.
fn alive(pid: i32) -> bool {
    std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .map(|o| {
            let stat = String::from_utf8_lossy(&o.stdout);
            let stat = stat.trim();
            !stat.is_empty() && !stat.starts_with('Z')
        })
        .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_disconnected_client_cancels_its_run() {
    let dir = tempfile::Builder::new()
        .prefix("gdc")
        .tempdir_in("/tmp")
        .expect("tempdir");
    let root = dir.path();
    std::fs::write(root.join("golem.toml"), common::golem_toml()).expect("golem.toml");
    std::fs::write(root.join("long.test.toml"), long_flow()).expect("flow");
    let socket = root.join("d.sock");
    let _server = golem_orchestrator::ipc::start_server(&socket)
        .await
        .expect("server");

    let submit = |flow: &str| {
        serde_json::json!({
            "type": "submit",
            "flow_paths": [root.join(flow).display().to_string()],
            "config": {
                "platform": "android",
                "project_root": root.display().to_string(),
                "output_dir": root.join("out").display().to_string(),
                "no_results": true,
                "stub_fail_on_runs": [],
                "client_cwd": root.display().to_string(),
                "client_env": [["PATH", std::env::var("PATH").unwrap_or_default()]],
            },
        })
    };

    let mut stream = tokio::net::UnixStream::connect(&socket)
        .await
        .expect("connect");
    stream
        .write_all(format!("{}\n", submit("long.test.toml")).as_bytes())
        .await
        .expect("submit");
    let pid_file = root.join("step.pid");
    let deadline = Instant::now() + Duration::from_secs(10);
    let pid = loop {
        if let Some(pid) = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<i32>().ok())
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "the bash step SHALL start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(alive(pid));

    drop(stream);

    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(pid) {
        assert!(
            Instant::now() < deadline,
            "the step's process SHALL be killed once its client is gone"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // The daemon still serves: a fresh client's run completes.
    std::fs::write(root.join("quick.test.toml"), common::slow_flow()).expect("flow");
    let stream = tokio::net::UnixStream::connect(&socket)
        .await
        .expect("connect");
    let (read, mut write) = stream.into_split();
    write
        .write_all(format!("{}\n", submit("quick.test.toml")).as_bytes())
        .await
        .expect("submit");
    let mut lines = BufReader::new(read).lines();
    let done = tokio::time::timeout(Duration::from_secs(20), async {
        while let Ok(Some(line)) = lines.next_line().await {
            if line.contains(r#""type":"done""#) {
                return line;
            }
        }
        String::new()
    })
    .await
    .expect("the second run SHALL finish");
    assert!(done.contains(r#""success":true"#), "{done}");
}
