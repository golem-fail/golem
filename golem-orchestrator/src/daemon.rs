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

/// Connect to the daemon on `socket`, starting one if none answers.
pub async fn connect_or_start(socket: &Path, starter: &dyn DaemonStarter) -> Result<UnixStream> {
    if let Ok(stream) = ipc::try_connect(socket).await {
        return Ok(stream);
    }
    let _lock = lock(socket).await?;
    // Another client may have started a daemon while this one waited.
    if let Ok(stream) = ipc::try_connect(socket).await {
        return Ok(stream);
    }
    // Under the lock, a socket file nothing answers on is stale: the
    // daemon unlinks its socket before it releases the lock to exit.
    let _ = std::fs::remove_file(socket);
    starter.start(socket)?;
    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    loop {
        match ipc::try_connect(socket).await {
            Ok(stream) => return Ok(stream),
            Err(e) if tokio::time::Instant::now() >= deadline => {
                return Err(golem_events::coded(
                    golem_events::FailureCode::HostOrchestratorIpc,
                    e.context(format!(
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

/// Run the daemon on `socket` until it has had no client for `idle_grace`.
///
/// On the way out it stops accepting under the start lock, serves the
/// clients still connected, then shuts down the devices golem booted
/// unless a submit asked to keep them.
pub async fn run(socket: &Path, idle_grace: Duration) -> Result<()> {
    let server = ipc::start_server(socket).await?;
    eprintln!(
        "  [orchestrator] daemon {} (pid {}) listening on {}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        socket.display()
    );
    let tick = (idle_grace / 4).clamp(Duration::from_millis(10), Duration::from_secs(1));
    loop {
        tokio::time::sleep(tick).await;
        if server.idle_for() < idle_grace {
            continue;
        }
        let lock = lock(socket).await?;
        // A client may have connected while this one waited for the lock.
        if server.idle_for() < idle_grace {
            drop(lock);
            continue;
        }
        server.stop_accepting();
        server.wait_for_clients().await;
        for warning in server
            .resource_mgr()
            .shutdown_golem_booted(server.keep_devices())
            .await
        {
            eprintln!("  [devices] {warning}");
        }
        eprintln!(
            "  [orchestrator] idle for {}s, exiting",
            idle_grace.as_secs()
        );
        drop(lock);
        return Ok(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

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
                tokio::spawn(async move { connect_or_start(&socket, starter.as_ref()).await })
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
        connect_or_start(&socket, &starter)
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
        let held = connect_or_start(&socket, &starter).await.expect("connect");
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
        drop(connect_or_start(&socket, &starter).await.expect("first"));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while socket.exists() {
            assert!(tokio::time::Instant::now() < deadline, "SHALL go idle");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(connect_or_start(&socket, &starter).await.expect("second"));
        assert_eq!(starts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_lock_and_log_sit_next_to_the_socket() {
        let socket = Path::new("/tmp/x/golem.sock");
        assert_eq!(lock_path(socket), Path::new("/tmp/x/golem.lock"));
        assert_eq!(log_path(socket), Path::new("/tmp/x/golem.log"));
    }
}
