//! The detached orchestrator daemon: how a client finds or starts it, and
//! when it exits.
//!
//! No client becomes the daemon. A client that finds no daemon takes the
//! start lock, checks again, and starts `golem daemon` as a detached
//! process. The daemon exits after a grace period with no client
//! connected, under the same lock, so a client never meets two daemons or
//! a half-stopped one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::UnixStream;

use crate::ipc;

/// How long the daemon stays up with no client connected.
pub const DEFAULT_IDLE_GRACE: Duration = Duration::from_secs(45);

/// How long a client waits for a daemon it started to accept connections.
const START_TIMEOUT: Duration = Duration::from_secs(15);

/// The lock that serialises starting and stopping the daemon on `socket`.
pub fn lock_path(socket: &Path) -> PathBuf {
    socket.with_extension("lock")
}

/// Where the daemon on `socket` writes its stdout and stderr.
pub fn log_path(socket: &Path) -> PathBuf {
    socket.with_extension("log")
}

/// An exclusive hold on the start lock, released on drop.
pub struct StartLock {
    _file: std::fs::File,
}

/// Take the start lock for `socket`, waiting while another process holds it.
pub async fn lock(socket: &Path) -> Result<StartLock> {
    let path = lock_path(socket);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    tokio::task::spawn_blocking(move || file.lock().map(|()| file))
        .await
        .context("the lock task failed")?
        .with_context(|| format!("failed to lock {}", path.display()))
        .map(|file| StartLock { _file: file })
}

/// How a client brings up a daemon on a socket.
pub trait DaemonStarter: Send + Sync {
    /// Start a daemon that listens on `socket`. Returns once the daemon is
    /// launched; [`connect_or_start`] waits for it to accept connections.
    fn start(&self, socket: &Path) -> Result<()>;
}

/// Starts `<this executable> daemon` as a detached process: its own
/// session (a closing terminal does not hang it up), stdin from
/// `/dev/null`, stdout and stderr appended to the log, and `/` as its
/// working directory, so nothing in it depends on where the first client
/// ran.
pub struct ExeStarter;

impl DaemonStarter for ExeStarter {
    fn start(&self, socket: &Path) -> Result<()> {
        use std::os::unix::process::CommandExt;
        let exe = std::env::current_exe().context("cannot find the golem executable")?;
        let log_file = log_path(socket);
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
            .with_context(|| format!("failed to open {}", log_file.display()))?;
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("daemon")
            .env("GOLEM_SOCKET", socket)
            .current_dir("/")
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        // SAFETY: `setsid` is async-signal-safe and touches no memory the
        // parent shares.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        cmd.spawn()
            .with_context(|| format!("failed to start {} daemon", exe.display()))?;
        Ok(())
    }
}

/// Runs the daemon as a task in this process. For tests, and for
/// `GOLEM_DAEMON_IN_PROCESS=1`: the daemon then ends with the process.
pub struct InProcessStarter {
    pub idle_grace: Duration,
}

impl DaemonStarter for InProcessStarter {
    fn start(&self, socket: &Path) -> Result<()> {
        let socket = socket.to_path_buf();
        let idle_grace = self.idle_grace;
        tokio::spawn(async move {
            if let Err(e) = run(&socket, idle_grace).await {
                eprintln!("  [orchestrator] in-process daemon failed: {e:#}");
            }
        });
        Ok(())
    }
}

/// How a client connects.
#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub identity: ipc::Identity,
    /// How long to wait for a draining daemon to exit.
    pub wait: Duration,
    /// How long a daemon has to answer `hello`.
    pub answer_timeout: Duration,
}

/// How long a client waits for a draining daemon by default.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(300);

impl ClientOptions {
    /// This process, waiting `GOLEM_DAEMON_WAIT` seconds (default 300) for
    /// a draining daemon.
    pub fn current() -> Self {
        let wait = std::env::var("GOLEM_DAEMON_WAIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .map_or(DEFAULT_WAIT, Duration::from_secs);
        ClientOptions {
            identity: ipc::Identity::current(),
            wait,
            answer_timeout: ipc::HELLO_TIMEOUT,
        }
    }
}

/// Connect to the daemon on `socket`, starting one if none answers.
///
/// A daemon of the same golem is used as it is. An older one, or another
/// build of this version, is asked to drain: it finishes its runs and
/// exits, and this client then starts one of its own. A newer one is an
/// error. While a daemon drains, the client waits up to `opts.wait`.
pub async fn connect_or_start(
    socket: &Path,
    starter: &dyn DaemonStarter,
    opts: &ClientOptions,
) -> Result<UnixStream> {
    let started = tokio::time::Instant::now();
    let mut announced = false;
    loop {
        match ipc::hello(socket, &opts.identity, opts.answer_timeout).await {
            Ok(ipc::Hello::Ready(stream)) => return Ok(stream),
            Ok(ipc::Hello::Newer { daemon }) => {
                return Err(newer_daemon(socket, &opts.identity, &daemon))
            }
            Ok(ipc::Hello::Stale { stream, daemon }) => {
                if daemon.version == opts.identity.version {
                    eprintln!(
                        "  [orchestrator] replacing the golem daemon from an earlier build of {}",
                        daemon.version
                    );
                } else {
                    eprintln!(
                        "  [orchestrator] replacing golem daemon {} ({}) with {} ({})",
                        daemon.version, daemon.binary, opts.identity.version, opts.identity.binary
                    );
                }
                ipc::drain(stream).await?;
            }
            Ok(ipc::Hello::Draining {
                daemon,
                runs,
                sessions,
            }) => {
                if started.elapsed() >= opts.wait {
                    return Err(golem_events::coded(
                        golem_events::FailureCode::HostOrchestratorIpc,
                        anyhow::anyhow!(
                            "the golem daemon {} is still finishing {runs} run(s) and {sessions} session(s) after {}s; \
                             set GOLEM_DAEMON_WAIT to wait longer, or GOLEM_SOCKET to use another daemon",
                            daemon.version,
                            opts.wait.as_secs()
                        ),
                    ));
                }
                if !announced {
                    if runs == 0 && sessions == 0 {
                        eprintln!(
                            "  [orchestrator] waiting for golem daemon {} to exit...",
                            daemon.version
                        );
                    } else {
                        eprintln!(
                            "  [orchestrator] waiting for golem daemon {} to finish {runs} run(s) and {sessions} session(s) before it exits...",
                            daemon.version
                        );
                    }
                    announced = true;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(ipc::HelloFailure::Unresponsive(e)) => {
                if started.elapsed() >= opts.wait {
                    return Err(golem_events::coded(
                        golem_events::FailureCode::HostOrchestratorIpc,
                        e.context(format!(
                            "the golem daemon on {} is not answering after {}s; see {}",
                            socket.display(),
                            opts.wait.as_secs(),
                            log_path(socket).display()
                        )),
                    ));
                }
                if !announced {
                    eprintln!("  [orchestrator] waiting for the golem daemon to answer...");
                    announced = true;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(ipc::HelloFailure::Absent) => {
                if let Some(stream) = start_under_lock(socket, starter, opts).await? {
                    return Ok(stream);
                }
            }
        }
    }
}

/// With nothing answering, take the lock and start a daemon. `None` when,
/// by the time the lock is held, a daemon answers after all: the caller
/// goes round again.
async fn start_under_lock(
    socket: &Path,
    starter: &dyn DaemonStarter,
    opts: &ClientOptions,
) -> Result<Option<UnixStream>> {
    let _lock = lock(socket).await?;
    // Another client may have started a daemon while this one waited, or a
    // slow one may be there after all.
    if !matches!(
        ipc::hello(socket, &opts.identity, opts.answer_timeout).await,
        Err(ipc::HelloFailure::Absent)
    ) {
        return Ok(None);
    }
    // Under the lock, a socket file nothing accepts on is stale: the daemon
    // unlinks its socket before it releases the lock to exit.
    let _ = std::fs::remove_file(socket);
    starter.start(socket)?;
    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    loop {
        match ipc::hello(socket, &opts.identity, opts.answer_timeout).await {
            Ok(ipc::Hello::Ready(stream)) => return Ok(Some(stream)),
            Ok(ipc::Hello::Newer { daemon } | ipc::Hello::Stale { daemon, .. })
            | Ok(ipc::Hello::Draining { daemon, .. }) => {
                anyhow::bail!(
                    "started a golem daemon but {} ({}) answered",
                    daemon.version,
                    daemon.binary
                );
            }
            Err(e) if tokio::time::Instant::now() >= deadline => {
                let cause = match e {
                    ipc::HelloFailure::Absent => anyhow::anyhow!("nothing listens on the socket"),
                    ipc::HelloFailure::Unresponsive(e) => e,
                };
                return Err(golem_events::coded(
                    golem_events::FailureCode::HostOrchestratorIpc,
                    cause.context(format!(
                        "the golem daemon did not start within {}s; see {}",
                        START_TIMEOUT.as_secs(),
                        log_path(socket).display()
                    )),
                ));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

fn newer_daemon(socket: &Path, me: &ipc::Identity, daemon: &ipc::Identity) -> anyhow::Error {
    golem_events::coded(
        golem_events::FailureCode::HostOrchestratorIpc,
        anyhow::anyhow!(
            "this golem {} ({}) is older than the running daemon {} ({}) on {}; \
             upgrade this golem, or set GOLEM_SOCKET to use a separate daemon",
            me.version,
            me.binary,
            daemon.version,
            daemon.binary,
            socket.display()
        ),
    )
}

/// Run the daemon on `socket` until it has had no client for `idle_grace`.
///
/// On the way out it stops accepting under the start lock, serves the
/// clients still connected, then shuts down the devices golem booted
/// unless a submit asked to keep them.
pub async fn run(socket: &Path, idle_grace: Duration) -> Result<()> {
    run_as(socket, idle_grace, &ipc::Identity::current()).await
}

/// [`run`] as `identity`, which it reports to clients.
pub async fn run_as(socket: &Path, idle_grace: Duration, identity: &ipc::Identity) -> Result<()> {
    let server = ipc::start_server(socket, identity).await?;
    eprintln!(
        "  [orchestrator] daemon {} (pid {}) listening on {}",
        identity.version,
        std::process::id(),
        socket.display()
    );
    let tick = (idle_grace / 4).clamp(Duration::from_millis(10), Duration::from_secs(1));
    loop {
        tokio::time::sleep(tick).await;
        let done = |s: &ipc::OrchestratorServer| s.drained() || s.idle_for() >= idle_grace;
        if !done(&server) {
            continue;
        }
        let lock = lock(socket).await?;
        // A client may have connected while this one waited for the lock.
        if !done(&server) {
            drop(lock);
            continue;
        }
        server.stop_accepting();
        server.wait_for_clients().await;
        golem_driver::ime::restore_all().await;
        for warning in server
            .resource_mgr()
            .shutdown_golem_booted(server.keep_devices())
            .await
        {
            eprintln!("  [devices] {warning}");
        }
        if server.drained() {
            eprintln!("  [orchestrator] drained, exiting");
        } else {
            eprintln!(
                "  [orchestrator] idle for {}s, exiting",
                idle_grace.as_secs()
            );
        }
        drop(lock);
        return Ok(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    fn opts() -> ClientOptions {
        ClientOptions {
            identity: ipc::Identity::current(),
            wait: Duration::from_secs(5),
            answer_timeout: Duration::from_millis(200),
        }
    }

    fn identity(version: &str, build: &str) -> ipc::Identity {
        ipc::Identity {
            version: version.into(),
            build: build.into(),
            binary: format!("/opt/golem-{version}/golem"),
        }
    }

    /// Run a daemon as `identity` and wait until it answers.
    async fn daemon_as(
        socket: &Path,
        identity: ipc::Identity,
    ) -> tokio::task::JoinHandle<Result<()>> {
        let task = tokio::spawn({
            let socket = socket.to_path_buf();
            async move { run_as(&socket, Duration::from_secs(30), &identity).await }
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while ipc::try_connect(socket).await.is_err() {
            assert!(tokio::time::Instant::now() < deadline, "daemon SHALL start");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        task
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_newer_client_drains_an_older_daemon_and_starts_its_own() {
        let dir = short_tempdir();
        let socket = socket_in(&dir);
        let old = daemon_as(&socket, identity("0.1.0", "a")).await;
        let starts = Arc::new(AtomicU32::new(0));
        let starter = CountingStarter {
            starts: starts.clone(),
            idle_grace: Duration::from_secs(30),
        };

        let stream = connect_or_start(&socket, &starter, &opts())
            .await
            .expect("SHALL connect to a new daemon");
        drop(stream);
        tokio::time::timeout(Duration::from_secs(5), old)
            .await
            .expect("the old daemon SHALL exit")
            .expect("task")
            .expect("clean exit");
        assert_eq!(
            starts.load(Ordering::SeqCst),
            1,
            "the client SHALL start its own daemon"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn another_build_of_the_same_version_is_replaced_too() {
        let dir = short_tempdir();
        let socket = socket_in(&dir);
        let me = ipc::Identity::current();
        let old = daemon_as(&socket, identity(&me.version, "an-earlier-build")).await;
        let starts = Arc::new(AtomicU32::new(0));
        let starter = CountingStarter {
            starts: starts.clone(),
            idle_grace: Duration::from_secs(30),
        };
        drop(
            connect_or_start(&socket, &starter, &opts())
                .await
                .expect("connect"),
        );
        tokio::time::timeout(Duration::from_secs(5), old)
            .await
            .expect("the stale daemon SHALL exit")
            .expect("task")
            .expect("clean exit");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_older_client_fails_at_once_naming_both_versions() {
        let dir = short_tempdir();
        let socket = socket_in(&dir);
        let _newer = daemon_as(&socket, identity("999.0.0", "z")).await;
        let started = std::time::Instant::now();
        let err = connect_or_start(
            &socket,
            &InProcessStarter {
                idle_grace: Duration::from_secs(1),
            },
            &opts(),
        )
        .await
        .expect_err("SHALL refuse")
        .to_string();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "SHALL fail at once"
        );
        let me = ipc::Identity::current();
        for part in [
            me.version.as_str(),
            me.binary.as_str(),
            "999.0.0",
            "/opt/golem-999.0.0/golem",
            "GOLEM_SOCKET",
        ] {
            assert!(err.contains(part), "the error SHALL name {part}: {err}");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_client_waits_for_a_draining_daemon_then_gives_up() {
        let dir = short_tempdir();
        let socket = socket_in(&dir);
        // A daemon serving a run cannot finish draining while it lasts.
        let server = ipc::start_server(&socket, &identity("0.1.0", "a"))
            .await
            .expect("server");
        let _run = server.begin_run();
        let short = ClientOptions {
            identity: ipc::Identity::current(),
            wait: Duration::from_millis(600),
            answer_timeout: Duration::from_millis(200),
        };
        let started = std::time::Instant::now();
        let err = connect_or_start(
            &socket,
            &InProcessStarter {
                idle_grace: Duration::from_secs(1),
            },
            &short,
        )
        .await
        .expect_err("SHALL time out")
        .to_string();
        assert!(
            started.elapsed() >= Duration::from_millis(600),
            "SHALL have waited"
        );
        assert!(err.contains("still finishing 1 run(s)"), "{err}");
        assert!(err.contains("GOLEM_DAEMON_WAIT"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_golem_from_before_the_handshake_is_waited_for_not_replaced() {
        let dir = short_tempdir();
        let socket = socket_in(&dir);
        // Answers every message the way a pre-handshake golem answers
        // `hello`.
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut lines = tokio::io::BufReader::new(r).lines();
                    while let Ok(Some(_)) = lines.next_line().await {
                        let _ = w
                            .write_all(b"{\"type\":\"error\",\"message\":\"unknown message type: hello\"}\n")
                            .await;
                    }
                });
            }
        });
        let starts = Arc::new(AtomicU32::new(0));
        let starter = CountingStarter {
            starts: starts.clone(),
            idle_grace: Duration::from_secs(1),
        };
        let short = ClientOptions {
            identity: ipc::Identity::current(),
            wait: Duration::from_millis(300),
            answer_timeout: Duration::from_millis(200),
        };
        let err = connect_or_start(&socket, &starter, &short)
            .await
            .expect_err("SHALL wait, then give up")
            .to_string();
        assert!(err.contains("still finishing"), "{err}");
        assert_eq!(
            starts.load(Ordering::SeqCst),
            0,
            "SHALL NOT start a second daemon"
        );
        assert!(socket.exists(), "SHALL NOT remove the live socket");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_daemon_slow_to_answer_is_waited_for_not_replaced() {
        let dir = short_tempdir();
        let socket = socket_in(&dir);
        // Accepts connections and never answers, like a daemon under load.
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });
        let starts = Arc::new(AtomicU32::new(0));
        let starter = CountingStarter {
            starts: starts.clone(),
            idle_grace: Duration::from_secs(1),
        };
        let short = ClientOptions {
            identity: ipc::Identity::current(),
            wait: Duration::from_millis(100),
            answer_timeout: Duration::from_millis(200),
        };
        let err = connect_or_start(&socket, &starter, &short)
            .await
            .expect_err("SHALL give up")
            .to_string();
        assert!(err.contains("not answering"), "{err}");
        assert_eq!(
            starts.load(Ordering::SeqCst),
            0,
            "SHALL NOT start a second daemon"
        );
        assert!(
            socket.exists(),
            "SHALL NOT remove a socket something accepts on"
        );
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(ipc::version_older("0.9.0", "0.15.0"));
        assert!(ipc::version_older("0.15.0", "1.0.0"));
        assert!(!ipc::version_older("0.15.0", "0.15.0"));
        assert!(!ipc::version_older("0.15.1", "0.15.0"));
    }

    /// A short socket path: macOS caps a unix socket path near 104 bytes,
    /// which a nested temp dir can exceed.
    fn socket_in(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("d.sock")
    }

    fn short_tempdir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("gd")
            .tempdir_in("/tmp")
            .expect("tempdir")
    }

    /// Counts its starts, and runs each daemon in-process.
    struct CountingStarter {
        starts: Arc<AtomicU32>,
        idle_grace: Duration,
    }

    impl DaemonStarter for CountingStarter {
        fn start(&self, socket: &Path) -> Result<()> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            InProcessStarter {
                idle_grace: self.idle_grace,
            }
            .start(socket)
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_clients_start_exactly_one_daemon() {
        let dir = short_tempdir();
        let socket = socket_in(&dir);
        let starts = Arc::new(AtomicU32::new(0));
        let starter = Arc::new(CountingStarter {
            starts: starts.clone(),
            idle_grace: Duration::from_secs(30),
        });

        let clients: Vec<_> = (0..10)
            .map(|_| {
                let socket = socket.clone();
                let starter = starter.clone();
                tokio::spawn(
                    async move { connect_or_start(&socket, starter.as_ref(), &opts()).await },
                )
            })
            .collect();
        for client in clients {
            client
                .await
                .expect("client task")
                .expect("every client SHALL connect");
        }
        assert_eq!(
            starts.load(Ordering::SeqCst),
            1,
            "exactly one daemon SHALL start"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stale_socket_file_is_replaced() {
        let dir = short_tempdir();
        let socket = socket_in(&dir);
        // A socket file with no listener, as a killed daemon leaves behind.
        drop(std::os::unix::net::UnixListener::bind(&socket).expect("bind"));
        assert!(socket.exists());

        let starts = Arc::new(AtomicU32::new(0));
        let starter = CountingStarter {
            starts: starts.clone(),
            idle_grace: Duration::from_secs(30),
        };
        connect_or_start(&socket, &starter, &opts())
            .await
            .expect("SHALL connect to a new daemon");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_daemon_exits_when_idle_and_unlinks_its_socket() {
        let dir = short_tempdir();
        let socket = socket_in(&dir);
        let daemon = tokio::spawn({
            let socket = socket.clone();
            async move { run(&socket, Duration::from_millis(200)).await }
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while ipc::try_connect(&socket).await.is_err() {
            assert!(tokio::time::Instant::now() < deadline, "daemon SHALL start");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        tokio::time::timeout(Duration::from_secs(3), daemon)
            .await
            .expect("the daemon SHALL exit once idle")
            .expect("daemon task")
            .expect("the daemon SHALL exit cleanly");
        assert!(!socket.exists(), "the socket SHALL be gone");
        assert!(ipc::try_connect(&socket).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_connected_client_keeps_the_daemon_up() {
        let dir = short_tempdir();
        let socket = socket_in(&dir);
        let starter = InProcessStarter {
            idle_grace: Duration::from_millis(200),
        };
        let held = connect_or_start(&socket, &starter, &opts())
            .await
            .expect("connect");
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(
            ipc::try_connect(&socket).await.is_ok(),
            "the daemon SHALL stay up while a client is connected"
        );
        drop(held);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_client_after_idle_exit_starts_a_new_daemon() {
        let dir = short_tempdir();
        let socket = socket_in(&dir);
        let starts = Arc::new(AtomicU32::new(0));
        let starter = CountingStarter {
            starts: starts.clone(),
            idle_grace: Duration::from_millis(100),
        };
        drop(
            connect_or_start(&socket, &starter, &opts())
                .await
                .expect("first"),
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while socket.exists() {
            assert!(tokio::time::Instant::now() < deadline, "SHALL go idle");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(
            connect_or_start(&socket, &starter, &opts())
                .await
                .expect("second"),
        );
        assert_eq!(starts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_lock_and_log_sit_next_to_the_socket() {
        let socket = Path::new("/tmp/x/golem.sock");
        assert_eq!(lock_path(socket), Path::new("/tmp/x/golem.lock"));
        assert_eq!(log_path(socket), Path::new("/tmp/x/golem.log"));
    }
}
