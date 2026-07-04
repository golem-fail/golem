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

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use golem_events::emitter::DeviceEmitter;
use golem_events::EventKind;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::fingerprint::Fingerprint;

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
        self.outcomes
            .lock()
            .await
            .insert(self.key.clone(), BuildOutcome::Succeeded);
    }
    pub async fn record_failure(self, err: String) {
        self.outcomes
            .lock()
            .await
            .insert(self.key.clone(), BuildOutcome::Failed(err));
    }
}

/// Persisted install record. One per `(udid, bundle)` in the on-disk
/// cache. All three "integrity gates" come from this entry: the cache
/// `device_install_time` is compared against the device's current install
/// time to detect external reinstalls; the `fingerprint` is compared
/// against the current source fingerprint to detect source changes;
/// presence on the device is checked separately at gate time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedInstall {
    /// Source fingerprint at the time of the recorded install.
    pub fingerprint: Fingerprint,
    /// Device-reported install time at install. Compared against
    /// the current device value to detect external reinstalls.
    /// `None` when the platform doesn't expose it (e.g. iOS phys today).
    pub device_install_time: Option<DateTime<Utc>>,
    /// Best-effort device-reported version at install. For debugging /
    /// future CI scenarios; not used in cache decisions today.
    pub installed_version: Option<String>,
    /// Wall-clock time golem completed the install. For human inspection.
    pub installed_at: DateTime<Utc>,
}

/// On-disk cache file shape. Keyed by `<udid>:<bundle>`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CacheFile {
    /// Schema version. Bump on incompatible field changes; readers ignore
    /// unknown versions (treat as empty).
    version: u32,
    entries: HashMap<String, PersistedInstall>,
}

const CACHE_FILE_VERSION: u32 = 1;

fn entry_key(udid: &str, bundle: &str) -> String {
    format!("{udid}:{bundle}")
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
    /// In-memory snapshot of the on-disk persistent cache. Loaded from
    /// `persistent_path` at suite start, written back after each
    /// successful install. `None` until [`InstallCache::load_persistent`]
    /// is called — calls before that get/set treat the cache as empty.
    persistent: Arc<Mutex<HashMap<String, PersistedInstall>>>,
    /// Path to the JSON cache file. `None` disables persistence (loads /
    /// saves are no-ops).
    persistent_path: Arc<Mutex<Option<PathBuf>>>,
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

    /// Configure the on-disk persistent cache and load its contents.
    /// Subsequent [`Self::get_persistent`] / [`Self::set_persistent`] calls
    /// read from / write to `path`. Soft-fails on parse / IO errors —
    /// returns `Ok(())` with an empty cache and emits a warning to stderr,
    /// rather than blocking the suite from running. A corrupt cache should
    /// degrade to "every (udid, bundle) misses" not crash the suite.
    pub async fn load_persistent(&self, path: PathBuf) -> Result<()> {
        let entries = match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<CacheFile>(&s) {
                Ok(file) if file.version == CACHE_FILE_VERSION => file.entries,
                Ok(file) => {
                    eprintln!(
                        "  [install] cache file {} has unknown version {} — treating as empty",
                        path.display(),
                        file.version
                    );
                    HashMap::new()
                }
                Err(e) => {
                    eprintln!(
                        "  [install] cache file {} unreadable ({e}) — treating as empty",
                        path.display()
                    );
                    HashMap::new()
                }
            },
            // Missing file is normal for a fresh project — quiet path.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                eprintln!(
                    "  [install] cache file {} unreadable ({e}) — treating as empty",
                    path.display()
                );
                HashMap::new()
            }
        };
        *self.persistent.lock().await = entries;
        *self.persistent_path.lock().await = Some(path);
        Ok(())
    }

    /// Get the persisted install entry for `(udid, bundle)`, if any.
    pub async fn get_persistent(&self, udid: &str, bundle: &str) -> Option<PersistedInstall> {
        self.persistent
            .lock()
            .await
            .get(&entry_key(udid, bundle))
            .cloned()
    }

    /// Insert / update the persisted entry for `(udid, bundle)` and write
    /// the whole cache file atomically (tmp + rename). When persistence
    /// is disabled (no `load_persistent` call), this is a no-op.
    pub async fn set_persistent(
        &self,
        udid: &str,
        bundle: &str,
        entry: PersistedInstall,
    ) -> Result<()> {
        let path_opt = self.persistent_path.lock().await.clone();
        let Some(path) = path_opt else {
            return Ok(());
        };
        {
            let mut map = self.persistent.lock().await;
            map.insert(entry_key(udid, bundle), entry);
        }
        self.flush_persistent(&path).await
    }

    /// Remove a persisted entry for `(udid, bundle)`. Used when external
    /// integrity checks fail and we want the cache to forget. Writes
    /// through to disk.
    pub async fn forget_persistent(&self, udid: &str, bundle: &str) -> Result<()> {
        let path_opt = self.persistent_path.lock().await.clone();
        let Some(path) = path_opt else {
            return Ok(());
        };
        let removed = {
            let mut map = self.persistent.lock().await;
            map.remove(&entry_key(udid, bundle)).is_some()
        };
        if removed {
            self.flush_persistent(&path).await?;
        }
        Ok(())
    }

    async fn flush_persistent(&self, path: &Path) -> Result<()> {
        let snapshot = self.persistent.lock().await.clone();
        let file = CacheFile {
            version: CACHE_FILE_VERSION,
            entries: snapshot,
        };
        let json = serde_json::to_string_pretty(&file).context("serialise install cache")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create cache dir {}", parent.display()))?;
        }
        // Atomic write: tmp + rename so a crash mid-write can't corrupt
        // the cache file. The rename is atomic on the same filesystem.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).with_context(|| format!("write cache tmp {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("rename cache tmp -> {}", path.display()))?;
        Ok(())
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
            let err = format!(
                "failed to spawn install script {}: {e}",
                script_path.display()
            );
            if let Some(em) = emitter {
                em.emit(EventKind::InstallFinished {
                    app_name: app_name.to_string(),
                    bundle_id: bundle_id.to_string(),
                    success: false,
                    duration_ms: start.elapsed().as_millis() as u64,
                    exit_code: None,
                    error: Some(err.clone()),
                    code: Some(golem_events::FailureCode::AppInstallScriptNotFound),
                    target: target.to_string(),
                    os_major,
                });
            }
            return Err(golem_events::coded(
                golem_events::FailureCode::AppInstallScriptNotFound,
                anyhow!(err),
            ));
        }
    };

    // Stream stderr line-by-line via events; also keep a tail for error context.
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("no stderr pipe"))?;
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

    let wait_result = tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait()).await;

    let (success, exit_code, error_msg, fail_code) = match wait_result {
        Ok(Ok(status)) => {
            let tail = stderr_task.await.unwrap_or_default();
            if status.success() {
                (
                    true,
                    status.code(),
                    None,
                    golem_events::FailureCode::AppInstallFailed,
                )
            } else {
                let tail_str = tail.join("\n");
                let code = status.code();
                let msg = format!(
                    "install script exited {} for {app_name} on {device_udid}:\n{tail_str}",
                    code.map(|c| c.to_string())
                        .unwrap_or_else(|| "signal".into()),
                );
                (
                    false,
                    code,
                    Some(msg),
                    golem_events::FailureCode::AppInstallFailed,
                )
            }
        }
        Ok(Err(e)) => {
            let _ = stderr_task.await;
            (
                false,
                None,
                Some(format!("install script wait failed: {e}")),
                golem_events::FailureCode::AppInstallFailed,
            )
        }
        Err(_elapsed) => {
            let _ = child.kill().await;
            let _ = stderr_task.await;
            (
                false,
                None,
                Some(format!(
                    "install script timed out after {}ms for {app_name} on {device_udid}",
                    timeout_ms
                )),
                golem_events::FailureCode::AppInstallTimeout,
            )
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
            code: if success { None } else { Some(fail_code) },
            target: target.to_string(),
            os_major,
        });
    }

    if success {
        Ok(())
    } else {
        Err(golem_events::coded(
            fail_code,
            anyhow!(error_msg.unwrap_or_else(|| "install failed".into())),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_script(dir: &Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("install.sh");
        std::fs::write(&path, body).expect("write() SHALL succeed");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).expect("metadata() SHALL succeed").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).expect("set_permissions() SHALL succeed");
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
        for h in handles {
            h.await.expect("async operation SHALL succeed");
        }
        assert_eq!(
            max_seen.load(Ordering::SeqCst),
            1,
            "same (root, script) install SHALL be serialised (at most 1 in-flight)"
        );
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
        let a = cache
            .project_lock(root, Path::new("/tmp/monorepo/app-a.sh"))
            .await;
        let b = cache
            .project_lock(root, Path::new("/tmp/monorepo/app-b.sh"))
            .await;
        let _ga = a.lock().await;
        // Different script within same root SHALL NOT block (monorepo case).
        let _gb = tokio::time::timeout(Duration::from_millis(100), b.lock())
            .await
            .expect("locks for different scripts SHALL be independent");
    }

    #[tokio::test]
    async fn persistent_load_missing_file_is_empty_ok() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let path = tmp.path().join("install-cache.json");
        let cache = InstallCache::new();
        cache.load_persistent(path).await.expect("async operation SHALL succeed");
        assert!(cache.get_persistent("u-1", "com.x").await.is_none());
    }

    #[tokio::test]
    async fn persistent_set_then_get_in_same_session() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let path = tmp.path().join("install-cache.json");
        let cache = InstallCache::new();
        cache.load_persistent(path.clone()).await.expect("async operation SHALL succeed");
        let entry = PersistedInstall {
            fingerprint: Fingerprint::Git {
                rev: "abc".into(),
                porcelain: "def".into(),
            },
            device_install_time: None,
            installed_version: Some("0.1.0".into()),
            installed_at: chrono::Utc::now(),
        };
        cache
            .set_persistent("u-1", "com.x", entry.clone())
            .await
            .expect("async operation SHALL succeed");
        let got = cache.get_persistent("u-1", "com.x").await.expect("async operation SHALL succeed");
        assert_eq!(got, entry);
        // The file SHALL exist after a set.
        assert!(path.exists(), "set_persistent SHALL write the file");
    }

    #[tokio::test]
    async fn persistent_round_trip_across_caches() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let path = tmp.path().join("install-cache.json");

        let entry = PersistedInstall {
            fingerprint: Fingerprint::Content { hash: "abc".into() },
            device_install_time: None,
            installed_version: None,
            installed_at: chrono::Utc::now(),
        };

        let cache_a = InstallCache::new();
        cache_a.load_persistent(path.clone()).await.expect("async operation SHALL succeed");
        cache_a
            .set_persistent("u-9", "com.y", entry.clone())
            .await
            .expect("async operation SHALL succeed");

        let cache_b = InstallCache::new();
        cache_b.load_persistent(path).await.expect("async operation SHALL succeed");
        let got = cache_b.get_persistent("u-9", "com.y").await.expect("async operation SHALL succeed");
        assert_eq!(
            got, entry,
            "fresh cache SHALL load entries written by another"
        );
    }

    #[tokio::test]
    async fn persistent_corrupt_file_treated_as_empty() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let path = tmp.path().join("install-cache.json");
        std::fs::write(&path, "{not json").expect("write() SHALL succeed");
        let cache = InstallCache::new();
        cache.load_persistent(path).await.expect("async operation SHALL succeed");
        assert!(
            cache.get_persistent("u-1", "com.x").await.is_none(),
            "corrupt cache SHALL not block startup"
        );
    }

    #[tokio::test]
    async fn persistent_unknown_version_treated_as_empty() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let path = tmp.path().join("install-cache.json");
        std::fs::write(&path, r#"{"version": 99, "entries": {}}"#).expect("write() SHALL succeed");
        let cache = InstallCache::new();
        cache.load_persistent(path).await.expect("async operation SHALL succeed");
        assert!(cache.get_persistent("u-1", "com.x").await.is_none());
    }

    #[tokio::test]
    async fn persistent_no_load_means_set_is_noop() {
        let cache = InstallCache::new();
        let entry = PersistedInstall {
            fingerprint: Fingerprint::None,
            device_install_time: None,
            installed_version: None,
            installed_at: chrono::Utc::now(),
        };
        // Should not error, just nothing happens.
        cache.set_persistent("u-1", "com.x", entry).await.expect("async operation SHALL succeed");
        // get returns None because the in-memory map remains empty.
        assert!(cache.get_persistent("u-1", "com.x").await.is_none());
    }

    #[tokio::test]
    async fn persistent_forget_removes_entry() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let path = tmp.path().join("install-cache.json");
        let cache = InstallCache::new();
        cache.load_persistent(path).await.expect("async operation SHALL succeed");
        let entry = PersistedInstall {
            fingerprint: Fingerprint::None,
            device_install_time: None,
            installed_version: None,
            installed_at: chrono::Utc::now(),
        };
        cache.set_persistent("u-1", "com.x", entry).await.expect("async operation SHALL succeed");
        cache.forget_persistent("u-1", "com.x").await.expect("async operation SHALL succeed");
        assert!(cache.get_persistent("u-1", "com.x").await.is_none());
    }

    #[tokio::test]
    async fn install_cache_basic() {
        let cache = InstallCache::new();
        let key = ("udid-1".to_string(), "com.x".to_string());
        assert!(cache.get(&key).await.is_none());
        cache.set(key.clone(), InstallOutcome::Succeeded).await;
        assert!(matches!(
            cache.get(&key).await,
            Some(InstallOutcome::Succeeded)
        ));
    }

    #[tokio::test]
    async fn script_exit_0_succeeds() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let script = write_script(tmp.path(), "#!/bin/sh\necho running >&2\nexit 0\n");
        let result = run_install_script(
            &script,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "app",
            5_000,
            "test target",
            0,
            false,
            None,
        )
        .await;
        assert!(result.is_ok(), "exit 0 SHALL be ok: {:?}", result);
    }

    #[tokio::test]
    async fn script_exit_nonzero_fails_with_stderr() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let script = write_script(
            tmp.path(),
            "#!/bin/sh\necho 'build failed: missing signing' >&2\nexit 1\n",
        );
        let result = run_install_script(
            &script,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "app",
            5_000,
            "test target",
            0,
            false,
            None,
        )
        .await;
        assert!(result.is_err());
        let err = format!("{}", result.expect_err("operation SHALL fail"));
        assert!(
            err.contains("exited 1"),
            "error SHALL include exit code: {err}"
        );
        assert!(
            err.contains("missing signing"),
            "error SHALL include stderr tail: {err}"
        );
    }

    #[tokio::test]
    async fn script_timeout_kills_process() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let script = write_script(tmp.path(), "#!/bin/sh\nsleep 10\n");
        let result = run_install_script(
            &script,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "app",
            200,
            "test target",
            0,
            false,
            None,
        )
        .await;
        assert!(result.is_err());
        assert!(format!("{}", result.expect_err("operation SHALL fail")).contains("timed out"));
    }

    #[tokio::test]
    async fn script_receives_args_in_correct_order() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let out_file = tmp.path().join("args.txt");
        let script_body = format!(
            "#!/bin/sh\necho \"$1 $2 $3 $4\" > {}\nexit 0\n",
            out_file.display()
        );
        let script = write_script(tmp.path(), &script_body);
        let result = run_install_script(
            &script,
            tmp.path(),
            "android",
            "emulator-5554",
            "com.example.app",
            "app",
            5_000,
            "test target",
            0,
            false,
            None,
        )
        .await;
        assert!(result.is_ok());
        let args = std::fs::read_to_string(&out_file).expect("read_to_string() SHALL succeed");
        // $4 unset (install_only=false) SHALL produce empty trailing slot.
        assert_eq!(args.trim(), "android emulator-5554 com.example.app");
    }

    #[tokio::test]
    async fn script_runs_in_working_dir() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let marker = tmp.path().join("marker.txt");
        std::fs::write(&marker, "hello").expect("write() SHALL succeed");
        let script = write_script(
            tmp.path(),
            "#!/bin/sh\ntest -f ./marker.txt || { echo missing >&2; exit 1; }\n",
        );
        let result = run_install_script(
            &script,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "app",
            5_000,
            "test target",
            0,
            false,
            None,
        )
        .await;
        assert!(
            result.is_ok(),
            "SHALL run in provided working_dir: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn script_install_only_passes_fourth_arg() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let out_file = tmp.path().join("args.txt");
        let script_body = format!(
            "#!/bin/sh\necho \"$1|$2|$3|$4\" > {}\nexit 0\n",
            out_file.display()
        );
        let script = write_script(tmp.path(), &script_body);
        let result = run_install_script(
            &script,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "app",
            5_000,
            "test target",
            0,
            true,
            None,
        )
        .await;
        assert!(result.is_ok());
        let args = std::fs::read_to_string(&out_file).expect("read_to_string() SHALL succeed");
        assert_eq!(
            args.trim(),
            "ios|udid-1|com.x|install-only",
            "install_only=true SHALL pass \"install-only\" as $4"
        );
    }

    #[tokio::test]
    async fn script_full_build_omits_fourth_arg() {
        let tmp = tempdir().expect("tempdir() SHALL succeed");
        let out_file = tmp.path().join("args.txt");
        // Use -z to check $4 is empty/unset.
        let script_body = format!(
            "#!/bin/sh\nif [ -z \"$4\" ]; then echo NO4 > {}; else echo \"got:$4\" > {}; fi\nexit 0\n",
            out_file.display(), out_file.display()
        );
        let script = write_script(tmp.path(), &script_body);
        let result = run_install_script(
            &script,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "app",
            5_000,
            "test target",
            0,
            false,
            None,
        )
        .await;
        assert!(result.is_ok());
        let marker = std::fs::read_to_string(&out_file).expect("read_to_string() SHALL succeed");
        assert_eq!(
            marker.trim(),
            "NO4",
            "install_only=false SHALL omit the 4th arg entirely"
        );
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
        for h in handles {
            h.await.expect("async operation SHALL succeed");
        }

        assert_eq!(
            builder_count.load(Ordering::SeqCst),
            1,
            "exactly one Builder SHALL be elected"
        );
        assert_eq!(
            installed_count.load(Ordering::SeqCst),
            4,
            "other 4 callers SHALL see Installed"
        );
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
    async fn spawn_failure_yields_script_not_found_code() {
        // 1. A nonexistent script SHALL fail to spawn and surface
        //    AppInstallScriptNotFound (not the generic install-failed code).
        let tmp = tempdir().expect("tempdir");
        let missing = tmp.path().join("does-not-exist.sh");
        let result = run_install_script(
            &missing,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "app",
            5_000,
            "test target",
            0,
            false,
            None,
        )
        .await;
        let err = result.expect_err("spawning a missing script SHALL fail");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::AppInstallScriptNotFound),
            "spawn failure SHALL be coded AppInstallScriptNotFound: {err:#}"
        );
        assert!(
            format!("{err:#}").contains("failed to spawn install script"),
            "error SHALL mention the spawn failure: {err:#}"
        );
    }

    #[tokio::test]
    async fn nonzero_exit_is_coded_install_failed() {
        // 2. A nonzero script exit SHALL be coded AppInstallFailed.
        let tmp = tempdir().expect("tempdir");
        let script = write_script(tmp.path(), "#!/bin/sh\nexit 3\n");
        let result = run_install_script(
            &script,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "app",
            5_000,
            "test target",
            0,
            false,
            None,
        )
        .await;
        let err = result.expect_err("nonzero exit SHALL fail");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::AppInstallFailed),
            "nonzero exit SHALL be coded AppInstallFailed: {err:#}"
        );
    }

    #[tokio::test]
    async fn timeout_is_coded_install_timeout() {
        // 3. A timed-out script SHALL be coded AppInstallTimeout (distinct
        //    from the plain exit-failure code).
        let tmp = tempdir().expect("tempdir");
        let script = write_script(tmp.path(), "#!/bin/sh\nsleep 10\n");
        let result = run_install_script(
            &script,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "app",
            200,
            "test target",
            0,
            false,
            None,
        )
        .await;
        let err = result.expect_err("timeout SHALL fail");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::AppInstallTimeout),
            "timeout SHALL be coded AppInstallTimeout: {err:#}"
        );
    }

    #[tokio::test]
    async fn success_emits_started_and_finished_events() {
        // 4. On exit 0, an emitter SHALL receive InstallStarted (with the
        //    target/os_major fields plumbed) followed by a successful
        //    InstallFinished.
        use golem_events::channel::event_channel;
        use golem_events::DeviceId;
        let (sender, subs) = event_channel();
        let mut rx = subs.subscribe();
        let emitter = DeviceEmitter::new(sender, DeviceId("ios/sim".into()));

        let tmp = tempdir().expect("tempdir");
        let script = write_script(tmp.path(), "#!/bin/sh\nexit 0\n");
        let result = run_install_script(
            &script,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "app",
            5_000,
            "iPhone 16e (ios/v18/phone)",
            18,
            false,
            Some(&emitter),
        )
        .await;
        assert!(result.is_ok(), "exit 0 SHALL be ok: {result:?}");

        let first = rx.recv().await.expect("SHALL receive InstallStarted");
        match first.kind {
            EventKind::InstallStarted {
                app_name,
                bundle_id,
                target,
                os_major,
                ..
            } => {
                assert_eq!(app_name, "app", "InstallStarted SHALL carry app_name");
                assert_eq!(bundle_id, "com.x", "InstallStarted SHALL carry bundle_id");
                assert_eq!(
                    target, "iPhone 16e (ios/v18/phone)",
                    "InstallStarted SHALL carry the target string verbatim"
                );
                assert_eq!(os_major, 18, "InstallStarted SHALL carry os_major");
            }
            other => panic!("first event SHALL be InstallStarted, got {other:?}"),
        }

        // The next event(s) may include InstallOutput; find InstallFinished.
        loop {
            let ev = rx.recv().await.expect("SHALL receive InstallFinished");
            if let EventKind::InstallFinished {
                success,
                exit_code,
                error,
                code,
                os_major,
                ..
            } = ev.kind
            {
                assert!(success, "exit 0 SHALL emit success=true");
                assert_eq!(exit_code, Some(0), "success SHALL report exit_code 0");
                assert!(error.is_none(), "success SHALL carry no error");
                assert!(code.is_none(), "success SHALL carry no failure code");
                assert_eq!(os_major, 18, "InstallFinished SHALL echo os_major");
                break;
            }
        }
    }

    #[tokio::test]
    async fn stderr_lines_are_streamed_as_install_output_events() {
        // 5. Each stderr line SHALL be emitted as an InstallOutput event,
        //    tagged with the app name, in order.
        use golem_events::channel::event_channel;
        use golem_events::DeviceId;
        let (sender, subs) = event_channel();
        let mut rx = subs.subscribe();
        let emitter = DeviceEmitter::new(sender, DeviceId("ios/sim".into()));

        let tmp = tempdir().expect("tempdir");
        let script = write_script(
            tmp.path(),
            "#!/bin/sh\necho line-one >&2\necho line-two >&2\nexit 0\n",
        );
        let result = run_install_script(
            &script,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "myapp",
            5_000,
            "test target",
            0,
            false,
            Some(&emitter),
        )
        .await;
        assert!(result.is_ok(), "exit 0 SHALL be ok: {result:?}");

        let mut output_lines = Vec::new();
        // Drain all events; collect the InstallOutput lines.
        while let Ok(ev) = rx.try_recv() {
            if let EventKind::InstallOutput { app_name, line } = ev.kind {
                assert_eq!(app_name, "myapp", "InstallOutput SHALL carry the app name");
                output_lines.push(line);
            }
        }
        assert_eq!(
            output_lines,
            vec!["line-one".to_string(), "line-two".to_string()],
            "every stderr line SHALL be streamed as InstallOutput in order"
        );
    }

    #[tokio::test]
    async fn failure_emits_finished_with_code_and_error() {
        // 6. On nonzero exit, the emitted InstallFinished SHALL carry
        //    success=false, the exit code, an error message, and the
        //    AppInstallFailed failure code.
        use golem_events::channel::event_channel;
        use golem_events::DeviceId;
        let (sender, subs) = event_channel();
        let mut rx = subs.subscribe();
        let emitter = DeviceEmitter::new(sender, DeviceId("ios/sim".into()));

        let tmp = tempdir().expect("tempdir");
        let script = write_script(tmp.path(), "#!/bin/sh\necho boom >&2\nexit 7\n");
        let result = run_install_script(
            &script,
            tmp.path(),
            "ios",
            "udid-1",
            "com.x",
            "app",
            5_000,
            "test target",
            0,
            false,
            Some(&emitter),
        )
        .await;
        assert!(result.is_err(), "exit 7 SHALL fail");

        loop {
            let ev = rx.recv().await.expect("SHALL receive InstallFinished");
            if let EventKind::InstallFinished {
                success,
                exit_code,
                error,
                code,
                ..
            } = ev.kind
            {
                assert!(!success, "nonzero exit SHALL emit success=false");
                assert_eq!(
                    exit_code,
                    Some(7),
                    "InstallFinished SHALL report the exit code"
                );
                assert!(error.is_some(), "failure SHALL carry an error message");
                assert_eq!(
                    code,
                    Some(golem_events::FailureCode::AppInstallFailed),
                    "failure SHALL carry AppInstallFailed code"
                );
                break;
            }
        }
    }

    #[tokio::test]
    async fn forget_persistent_disabled_is_noop_ok() {
        // 7. With persistence never loaded, forget_persistent SHALL be a
        //    no-op that returns Ok (no path configured).
        let cache = InstallCache::new();
        cache
            .forget_persistent("u-1", "com.x")
            .await
            .expect("forget without a configured path SHALL be a no-op Ok");
    }

    #[tokio::test]
    async fn forget_persistent_missing_entry_is_ok() {
        // 8. Forgetting an entry that was never set SHALL be Ok and not
        //    create/alter the cache file (nothing removed -> no flush).
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("install-cache.json");
        let cache = InstallCache::new();
        cache.load_persistent(path.clone()).await.expect("load");
        cache
            .forget_persistent("u-1", "com.absent")
            .await
            .expect("forgetting an absent entry SHALL be Ok");
        assert!(
            !path.exists(),
            "forgetting an absent entry SHALL NOT write the cache file"
        );
    }

    #[tokio::test]
    async fn set_persistent_creates_missing_parent_dirs() {
        // 9. flush_persistent SHALL create missing parent directories so a
        //    set into a not-yet-existing cache dir succeeds.
        let tmp = tempdir().expect("tempdir");
        let path = tmp
            .path()
            .join("nested")
            .join("dir")
            .join("install-cache.json");
        let cache = InstallCache::new();
        cache.load_persistent(path.clone()).await.expect("load");
        let entry = PersistedInstall {
            fingerprint: Fingerprint::None,
            device_install_time: None,
            installed_version: None,
            installed_at: chrono::Utc::now(),
        };
        cache
            .set_persistent("u-1", "com.x", entry)
            .await
            .expect("set into a missing parent dir SHALL create it and succeed");
        assert!(
            path.exists(),
            "set_persistent SHALL create parent dirs and write the file"
        );
    }

    #[tokio::test]
    async fn acquire_build_fast_path_after_resolution() {
        // 10. Once an outcome is recorded, a fresh acquire SHALL take the
        //     fast path and return Installed without becoming a Builder.
        let cache = InstallCache::new();
        match cache.acquire_build("ios", "com.fast").await {
            BuildRole::Build(slot) => slot.record_success().await,
            _ => panic!("first caller SHALL be Builder"),
        }
        match cache.acquire_build("ios", "com.fast").await {
            BuildRole::Installed(BuildOutcome::Succeeded) => {}
            _ => panic!("resolved key SHALL return Installed(Succeeded) on fast path"),
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
