//! App install via user-provided bash script.
//!
//! See `docs/roadmap.md` for the design. Summary:
//!
//! - User writes a bash script that builds and installs their app on a target device.
//! - Golem invokes the script before each flow's `launch_app` in the Reset lifecycle.
//! - Script is invoked once per `(device_udid, bundle_id)` across the whole suite,
//!   tracked via a shared `InstallCache`.
//! - Exit 0 = success. Nonzero = flow failure. Stderr is streamed via event system.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use golem_events::emitter::DeviceEmitter;
use golem_events::EventKind;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

/// Default install script timeout if none is configured.
pub const DEFAULT_INSTALL_TIMEOUT_MS: u64 = 600_000; // 10 min

/// Cache key: `(device_udid, bundle_id)`.
pub type InstallKey = (String, String);

/// Outcome of a prior install attempt (cached across all flows in a suite).
#[derive(Debug, Clone)]
pub enum InstallOutcome {
    /// Script exited 0, OR no script configured but launch worked (back-fill).
    Succeeded,
    /// Script exited nonzero. Stderr captured.
    FailedScript(String),
    /// Launch failed AND no install_script was configured for this app.
    FailedNoScript,
}

/// Build-phase cache key: `(platform, bundle_id)`. Independent of device —
/// the first device for a given (platform, bundle) drives the build; later
/// devices skip straight to install-only.
pub type BuildKey = (String, String);

/// Outcome of the one-time build performed by the first device for a
/// given `(platform, bundle)` pair.
#[derive(Debug, Clone)]
pub enum BuildOutcome {
    Succeeded,
    /// Full build failed. Every waiter for the same key short-circuits
    /// to a skip with this error attached.
    Failed(String),
}

/// Role handed out by [`InstallCache::acquire_build`]. The first caller
/// for a `(platform, bundle)` becomes the Builder and runs the full
/// script; subsequent callers wait for the outcome and then install-only.
pub enum BuildRole {
    /// Caller SHALL run the full install script (no `install-only` arg)
    /// and record the outcome via [`BuildSlot::record_success`] or
    /// [`BuildSlot::record_failure`] before dropping the slot.
    Build(BuildSlot),
    /// Build already finished. Caller uses the outcome: on `Succeeded`,
    /// invoke the script with `install-only`; on `Failed`, skip.
    Installed(BuildOutcome),
}

/// Held by the winning builder. Dropping without calling
/// `record_success`/`record_failure` is safe — no outcome is recorded
/// and the next waiter becomes the new builder (retry on panic).
pub struct BuildSlot {
    outcomes: Arc<Mutex<HashMap<BuildKey, BuildOutcome>>>,
    key: BuildKey,
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

impl BuildSlot {
    pub async fn record_success(self) {
        self.outcomes.lock().await.insert(self.key.clone(), BuildOutcome::Succeeded);
    }
    pub async fn record_failure(self, err: String) {
        self.outcomes.lock().await.insert(self.key.clone(), BuildOutcome::Failed(err));
    }
}

/// Shared install cache. Safe to clone (`Arc` inside).
#[derive(Clone, Default)]
pub struct InstallCache {
    inner: Arc<Mutex<HashMap<InstallKey, InstallOutcome>>>,
    /// Per-(project-root, script-path) locks. Serialises concurrent install
    /// scripts that drive the same build tree (e.g. iOS + Android tauri
    /// builds both using `src-tauri/`): parallel `cargo tauri` parents fight
    /// over shared target-dir locks and WS IPC, causing ECONNREFUSED panics.
    ///
    /// Keying by script path too means unrelated apps under one monorepo
    /// project root don't serialise with each other — only runs that share
    /// both root AND script collide.
    #[allow(clippy::type_complexity)]
    project_locks: Arc<Mutex<HashMap<(PathBuf, PathBuf), Arc<Mutex<()>>>>>,
    /// Per-(platform, bundle) build outcomes. Populated by the winning
    /// builder; read by waiters.
    build_outcomes: Arc<Mutex<HashMap<BuildKey, BuildOutcome>>>,
    /// Per-(platform, bundle) mutex that serialises build-winner selection.
    /// Held by the Builder across the full script run; waiters queue on it
    /// and re-check `build_outcomes` once they acquire.
    #[allow(clippy::type_complexity)]
    build_locks: Arc<Mutex<HashMap<BuildKey, Arc<tokio::sync::Mutex<()>>>>>,
}

impl InstallCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self, key: &InstallKey) -> Option<InstallOutcome> {
        self.inner.lock().await.get(key).cloned()
    }

    pub async fn set(&self, key: InstallKey, outcome: InstallOutcome) {
        self.inner.lock().await.insert(key, outcome);
    }

    /// Get (or create) the serialisation lock for a given (project_root,
    /// script_path) pair. Callers hold the returned guard for the duration
    /// of the install script run to prevent concurrent invocations that
    /// share the same build tree.
    pub async fn project_lock(&self, root: &Path, script: &Path) -> Arc<Mutex<()>> {
        let mut map = self.project_locks.lock().await;
        map.entry((root.to_path_buf(), script.to_path_buf()))
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Determine this caller's role in the (platform, bundle) build.
    ///
    /// Fast-path: if the build has already finished, return
    /// `BuildRole::Installed(outcome)` immediately.
    ///
    /// Otherwise acquire the per-key build mutex and re-check under the
    /// lock. If still no outcome, hand the caller a `BuildSlot` (Builder
    /// role). If a winner finished while we queued, return `Installed`.
    pub async fn acquire_build(&self, platform: &str, bundle: &str) -> BuildRole {
        let key: BuildKey = (platform.to_string(), bundle.to_string());

        // Fast-path: already resolved, no need to touch the build-lock map.
        if let Some(outcome) = self.build_outcomes.lock().await.get(&key).cloned() {
            return BuildRole::Installed(outcome);
        }

        // Get or create the per-key build mutex.
        let lock = {
            let mut locks = self.build_locks.lock().await;
            locks
                .entry(key.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let guard = lock.lock_owned().await;

        // Re-check under the per-key lock — a winner may have finished
        // while we queued.
        if let Some(outcome) = self.build_outcomes.lock().await.get(&key).cloned() {
            return BuildRole::Installed(outcome);
        }

        BuildRole::Build(BuildSlot {
            outcomes: self.build_outcomes.clone(),
            key,
            _guard: guard,
        })
    }
}

/// Run an install script. Emits `InstallStarted`, `InstallOutput` (per stderr line),
/// and `InstallFinished` events via the provided emitter (when present).
///
/// Stdout is discarded. Stderr is streamed line-by-line via `InstallOutput` events.
///
/// Returns `Ok(())` on exit 0, `Err(...)` on nonzero exit, timeout, or spawn error.
/// The error's `Display` contains exit info + stderr tail (last ~100 lines).
#[allow(clippy::too_many_arguments)]
pub async fn run_install_script(
    script_path: &Path,
    working_dir: &Path,
    platform: &str,
    device_udid: &str,
    bundle_id: &str,
    app_name: &str,
    timeout_ms: u64,
    target: &str,
    os_major: u32,
    install_only: bool,
    emitter: Option<&DeviceEmitter>,
) -> Result<()> {
    let start = Instant::now();
    if let Some(e) = emitter {
        e.emit(EventKind::InstallStarted {
            app_name: app_name.to_string(),
            bundle_id: bundle_id.to_string(),
            script_path: script_path.display().to_string(),
            target: target.to_string(),
            os_major,
        });
    }

    let mut cmd = Command::new(script_path);
    cmd.arg(platform).arg(device_udid).arg(bundle_id);
    if install_only {
        // Scripts that know the protocol SHALL skip their build step when
        // `$4 == "install-only"` and install the already-built artifact.
        // Scripts that don't check the arg fall back to a full rebuild —
        // correct, just miss the optimisation.
        cmd.arg("install-only");
    }
    let spawn_result = cmd
        .current_dir(working_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match spawn_result {
        Ok(c) => c,
        Err(e) => {
            let err = format!("failed to spawn install script {}: {e}", script_path.display());
            if let Some(em) = emitter {
                em.emit(EventKind::InstallFinished {
                    app_name: app_name.to_string(),
                    bundle_id: bundle_id.to_string(),
                    success: false,
                    duration_ms: start.elapsed().as_millis() as u64,
                    exit_code: None,
                    error: Some(err.clone()),
                    target: target.to_string(),
                    os_major,
                });
            }
            return Err(anyhow!(err));
        }
    };

    // Stream stderr line-by-line via events; also keep a tail for error context.
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr pipe"))?;
    let emitter_for_task: Option<DeviceEmitter> = emitter.cloned();
    let app_name_for_task = app_name.to_string();
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut tail: Vec<String> = Vec::new();
        while let Ok(Some(line)) = reader.next_line().await {
            if let Some(ref em) = emitter_for_task {
                em.emit(EventKind::InstallOutput {
                    app_name: app_name_for_task.clone(),
                    line: line.clone(),
                });
            }
            tail.push(line);
            if tail.len() > 100 {
                tail.remove(0);
            }
        }
        tail
    });

    let wait_result = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        child.wait(),
    ).await;

    let (success, exit_code, error_msg) = match wait_result {
        Ok(Ok(status)) => {
            let tail = stderr_task.await.unwrap_or_default();
            if status.success() {
                (true, status.code(), None)
            } else {
                let tail_str = tail.join("\n");
                let code = status.code();
                let msg = format!(
                    "install script exited {} for {app_name} on {device_udid}:\n{tail_str}",
                    code.map(|c| c.to_string()).unwrap_or_else(|| "signal".into()),
                );
                (false, code, Some(msg))
            }
        }
        Ok(Err(e)) => {
            let _ = stderr_task.await;
            (false, None, Some(format!("install script wait failed: {e}")))
        }
        Err(_elapsed) => {
            let _ = child.kill().await;
            let _ = stderr_task.await;
            (false, None, Some(format!(
                "install script timed out after {}ms for {app_name} on {device_udid}",
                timeout_ms
            )))
        }
    };

    if let Some(em) = emitter {
        em.emit(EventKind::InstallFinished {
            app_name: app_name.to_string(),
            bundle_id: bundle_id.to_string(),
            success,
            duration_ms: start.elapsed().as_millis() as u64,
            exit_code,
            error: error_msg.clone(),
            target: target.to_string(),
            os_major,
        });
    }

    if success {
        Ok(())
    } else {
        Err(anyhow!(error_msg.unwrap_or_else(|| "install failed".into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_script(dir: &Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("install.sh");
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        path
    }

    #[tokio::test]
    async fn project_lock_serialises_same_root_and_script() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let cache = InstallCache::new();
        let root = Path::new("/tmp/p1");
        let script = Path::new("/tmp/p1/install.sh");
        let in_flight = Arc::new(AtomicU32::new(0));
        let max_seen = Arc::new(AtomicU32::new(0));

        let mut handles = Vec::new();
        for _ in 0..5 {
            let cache = cache.clone();
            let in_flight = in_flight.clone();
            let max_seen = max_seen.clone();
            handles.push(tokio::spawn(async move {
                let lock = cache.project_lock(root, script).await;
                let _g = lock.lock().await;
                let n = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                max_seen.fetch_max(n, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles { h.await.unwrap(); }
        assert_eq!(max_seen.load(Ordering::SeqCst), 1,
            "same (root, script) install SHALL be serialised (at most 1 in-flight)");
    }

    #[tokio::test]
    async fn project_lock_independent_per_root() {
        let cache = InstallCache::new();
        let s = Path::new("/tmp/shared.sh");
        let a = cache.project_lock(Path::new("/tmp/a"), s).await;
        let b = cache.project_lock(Path::new("/tmp/b"), s).await;
        let _ga = a.lock().await;
        // Different root SHALL not block.
        let _gb = tokio::time::timeout(Duration::from_millis(100), b.lock())
            .await
            .expect("different-root locks SHALL be independent");
    }

    #[tokio::test]
    async fn project_lock_independent_per_script_within_same_root() {
        let cache = InstallCache::new();
        let root = Path::new("/tmp/monorepo");
        let a = cache.project_lock(root, Path::new("/tmp/monorepo/app-a.sh")).await;
        let b = cache.project_lock(root, Path::new("/tmp/monorepo/app-b.sh")).await;
        let _ga = a.lock().await;
        // Different script within same root SHALL NOT block (monorepo case).
        let _gb = tokio::time::timeout(Duration::from_millis(100), b.lock())
            .await
            .expect("locks for different scripts SHALL be independent");
    }

    #[tokio::test]
    async fn install_cache_basic() {
        let cache = InstallCache::new();
        let key = ("udid-1".to_string(), "com.x".to_string());
        assert!(cache.get(&key).await.is_none());
        cache.set(key.clone(), InstallOutcome::Succeeded).await;
        assert!(matches!(cache.get(&key).await, Some(InstallOutcome::Succeeded)));
    }

    #[tokio::test]
    async fn script_exit_0_succeeds() {
        let tmp = tempdir().unwrap();
        let script = write_script(tmp.path(),
            "#!/bin/sh\necho running >&2\nexit 0\n");
        let result = run_install_script(
            &script, tmp.path(),
            "ios", "udid-1", "com.x", "app", 5_000, "test target", 0, false, None,
        ).await;
        assert!(result.is_ok(), "exit 0 SHALL be ok: {:?}", result);
    }

    #[tokio::test]
    async fn script_exit_nonzero_fails_with_stderr() {
        let tmp = tempdir().unwrap();
        let script = write_script(tmp.path(),
            "#!/bin/sh\necho 'build failed: missing signing' >&2\nexit 1\n");
        let result = run_install_script(
            &script, tmp.path(),
            "ios", "udid-1", "com.x", "app", 5_000, "test target", 0, false, None,
        ).await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("exited 1"), "error SHALL include exit code: {err}");
        assert!(err.contains("missing signing"), "error SHALL include stderr tail: {err}");
    }

    #[tokio::test]
    async fn script_timeout_kills_process() {
        let tmp = tempdir().unwrap();
        let script = write_script(tmp.path(),
            "#!/bin/sh\nsleep 10\n");
        let result = run_install_script(
            &script, tmp.path(),
            "ios", "udid-1", "com.x", "app", 200, "test target", 0, false, None,
        ).await;
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("timed out"));
    }

    #[tokio::test]
    async fn script_receives_args_in_correct_order() {
        let tmp = tempdir().unwrap();
        let out_file = tmp.path().join("args.txt");
        let script_body = format!(
            "#!/bin/sh\necho \"$1 $2 $3 $4\" > {}\nexit 0\n",
            out_file.display()
        );
        let script = write_script(tmp.path(), &script_body);
        let result = run_install_script(
            &script, tmp.path(),
            "android", "emulator-5554", "com.example.app", "app", 5_000, "test target", 0, false, None,
        ).await;
        assert!(result.is_ok());
        let args = std::fs::read_to_string(&out_file).unwrap();
        // $4 unset (install_only=false) SHALL produce empty trailing slot.
        assert_eq!(args.trim(), "android emulator-5554 com.example.app");
    }

    #[tokio::test]
    async fn script_runs_in_working_dir() {
        let tmp = tempdir().unwrap();
        let marker = tmp.path().join("marker.txt");
        std::fs::write(&marker, "hello").unwrap();
        let script = write_script(tmp.path(),
            "#!/bin/sh\ntest -f ./marker.txt || { echo missing >&2; exit 1; }\n");
        let result = run_install_script(
            &script, tmp.path(),
            "ios", "udid-1", "com.x", "app", 5_000, "test target", 0, false, None,
        ).await;
        assert!(result.is_ok(), "SHALL run in provided working_dir: {:?}", result);
    }

    #[tokio::test]
    async fn script_install_only_passes_fourth_arg() {
        let tmp = tempdir().unwrap();
        let out_file = tmp.path().join("args.txt");
        let script_body = format!(
            "#!/bin/sh\necho \"$1|$2|$3|$4\" > {}\nexit 0\n",
            out_file.display()
        );
        let script = write_script(tmp.path(), &script_body);
        let result = run_install_script(
            &script, tmp.path(),
            "ios", "udid-1", "com.x", "app", 5_000, "test target", 0, true, None,
        ).await;
        assert!(result.is_ok());
        let args = std::fs::read_to_string(&out_file).unwrap();
        assert_eq!(args.trim(), "ios|udid-1|com.x|install-only",
            "install_only=true SHALL pass \"install-only\" as $4");
    }

    #[tokio::test]
    async fn script_full_build_omits_fourth_arg() {
        let tmp = tempdir().unwrap();
        let out_file = tmp.path().join("args.txt");
        // Use -z to check $4 is empty/unset.
        let script_body = format!(
            "#!/bin/sh\nif [ -z \"$4\" ]; then echo NO4 > {}; else echo \"got:$4\" > {}; fi\nexit 0\n",
            out_file.display(), out_file.display()
        );
        let script = write_script(tmp.path(), &script_body);
        let result = run_install_script(
            &script, tmp.path(),
            "ios", "udid-1", "com.x", "app", 5_000, "test target", 0, false, None,
        ).await;
        assert!(result.is_ok());
        let marker = std::fs::read_to_string(&out_file).unwrap();
        assert_eq!(marker.trim(), "NO4",
            "install_only=false SHALL omit the 4th arg entirely");
    }

    #[tokio::test]
    async fn acquire_build_winner_selected_once_under_concurrency() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let cache = InstallCache::new();
        let builder_count = Arc::new(AtomicU32::new(0));
        let installed_count = Arc::new(AtomicU32::new(0));

        let mut handles = Vec::new();
        for _ in 0..5 {
            let cache = cache.clone();
            let bc = builder_count.clone();
            let ic = installed_count.clone();
            handles.push(tokio::spawn(async move {
                match cache.acquire_build("ios", "com.x").await {
                    BuildRole::Build(slot) => {
                        bc.fetch_add(1, Ordering::SeqCst);
                        // Simulate build work so waiters actually queue.
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        slot.record_success().await;
                    }
                    BuildRole::Installed(_) => {
                        ic.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }));
        }
        for h in handles { h.await.unwrap(); }

        assert_eq!(builder_count.load(Ordering::SeqCst), 1,
            "exactly one Builder SHALL be elected");
        assert_eq!(installed_count.load(Ordering::SeqCst), 4,
            "other 4 callers SHALL see Installed");
    }

    #[tokio::test]
    async fn acquire_build_failure_propagates_to_waiters() {
        let cache = InstallCache::new();

        // First caller is Builder; records failure.
        match cache.acquire_build("ios", "com.y").await {
            BuildRole::Build(slot) => slot.record_failure("build bust".into()).await,
            _ => panic!("first caller SHALL be Builder"),
        }

        // Second caller sees the failure.
        match cache.acquire_build("ios", "com.y").await {
            BuildRole::Installed(BuildOutcome::Failed(err)) => {
                assert_eq!(err, "build bust");
            }
            _ => panic!("second caller SHALL see Installed(Failed)"),
        }
    }

    #[tokio::test]
    async fn acquire_build_builder_drop_without_record_allows_retry() {
        let cache = InstallCache::new();

        // First caller becomes Builder but drops without recording
        // (simulates panic / early return).
        {
            match cache.acquire_build("ios", "com.z").await {
                BuildRole::Build(_slot) => { /* drop without recording */ }
                _ => panic!("first SHALL be Builder"),
            }
        }

        // Second caller SHALL also become Builder (no outcome stored).
        match cache.acquire_build("ios", "com.z").await {
            BuildRole::Build(slot) => slot.record_success().await,
            _ => panic!("second SHALL be Builder after drop without record"),
        }

        // Third sees the success.
        match cache.acquire_build("ios", "com.z").await {
            BuildRole::Installed(BuildOutcome::Succeeded) => {}
            _ => panic!("third SHALL see Installed(Succeeded)"),
        }
    }

    #[tokio::test]
    async fn acquire_build_keys_are_per_platform_and_bundle() {
        let cache = InstallCache::new();
        // iOS / com.a builds successfully.
        match cache.acquire_build("ios", "com.a").await {
            BuildRole::Build(slot) => slot.record_success().await,
            _ => panic!(),
        }
        // Android / com.a SHALL still be a fresh build (different platform).
        match cache.acquire_build("android", "com.a").await {
            BuildRole::Build(slot) => slot.record_success().await,
            _ => panic!("android/com.a SHALL be its own build"),
        }
        // iOS / com.b SHALL still be fresh (different bundle).
        match cache.acquire_build("ios", "com.b").await {
            BuildRole::Build(_) => {}
            _ => panic!("ios/com.b SHALL be its own build"),
        }
    }
}
