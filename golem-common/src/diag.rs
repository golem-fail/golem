//! Host diagnostics: lines about the host, the devices and the drivers that
//! belong to the run that caused them.
//!
//! The daemon runs every suite, so an `eprintln!` from run-path code lands
//! in the daemon log, not in the terminal of the run. The daemon runs each
//! suite inside a [`scope`] that carries the run's sink and its `--debug`
//! flag. [`info`], [`warn`] and [`debug`] go to that sink. Outside a scope
//! (the CLI's own work, tests) they go to stderr as before.
//!
//! A scope is task-local, so a task spawned with `tokio::spawn` leaves it.
//! Run-path code spawns with [`spawn`], which carries the scope over.

use std::future::Future;
use std::sync::Arc;

/// How much a diagnostic matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    /// Shown only with `--debug`.
    Debug,
}

/// Takes one diagnostic line of a run.
pub type Sink = Arc<dyn Fn(Level, &str) + Send + Sync>;

/// Where a run's diagnostics go, and whether the run asked for `--debug`.
#[derive(Clone)]
pub struct Scope {
    pub debug: bool,
    pub sink: Sink,
}

tokio::task_local! {
    static SCOPE: Scope;
}

/// Run `f` with `scope` as its diagnostics scope.
pub async fn scope<F: Future>(scope: Scope, f: F) -> F::Output {
    SCOPE.scope(scope, f).await
}

/// The scope of the current task, if it has one.
pub fn current() -> Option<Scope> {
    SCOPE.try_with(Scope::clone).ok()
}

/// `tokio::spawn`, keeping the current diagnostics scope in the new task.
pub fn spawn<F>(f: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match current() {
        Some(s) => tokio::spawn(SCOPE.scope(s, f)),
        None => tokio::spawn(f),
    }
}

/// The current scope's `--debug`, if the task has a scope.
pub(crate) fn scoped_debug() -> Option<bool> {
    SCOPE.try_with(|s| s.debug).ok()
}

/// Report `message` at `level`: to the run's sink in a scope, else to
/// stderr. A [`Level::Debug`] line is dropped unless `--debug` is on.
pub fn emit(level: Level, message: impl AsRef<str>) {
    if level == Level::Debug && !crate::is_debug() {
        return;
    }
    let message = message.as_ref();
    match SCOPE.try_with(|s| s.sink.clone()) {
        Ok(sink) => sink(level, message),
        Err(_) => eprintln!("  {message}"),
    }
}

pub fn info(message: impl AsRef<str>) {
    emit(Level::Info, message);
}

pub fn warn(message: impl AsRef<str>) {
    emit(Level::Warn, message);
}

pub fn debug(message: impl AsRef<str>) {
    emit(Level::Debug, message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    type Seen = Arc<Mutex<Vec<(Level, String)>>>;

    fn recording(debug: bool) -> (Scope, Seen) {
        let seen: Seen = Arc::default();
        let into = seen.clone();
        let scope = Scope {
            debug,
            sink: Arc::new(move |level, message| {
                into.lock()
                    .expect("seen")
                    .push((level, message.to_string()));
            }),
        };
        (scope, seen)
    }

    #[tokio::test]
    async fn a_scope_takes_its_runs_lines_and_its_spawned_tasks_lines() {
        let (s, seen) = recording(false);
        scope(s, async {
            info("[devices] booting iPhone 17");
            spawn(async { warn("[ime] restore failed") })
                .await
                .expect("join");
        })
        .await;
        assert_eq!(
            *seen.lock().expect("seen"),
            vec![
                (Level::Info, "[devices] booting iPhone 17".to_string()),
                (Level::Warn, "[ime] restore failed".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn debug_lines_follow_the_runs_own_flag() {
        let (quiet, quiet_seen) = recording(false);
        scope(quiet, async { debug("[webkit] setup failed") }).await;
        assert!(quiet_seen.lock().expect("seen").is_empty());

        let (loud, loud_seen) = recording(true);
        scope(loud, async {
            assert!(crate::is_debug(), "is_debug SHALL read the run's flag");
            debug("[webkit] setup failed");
        })
        .await;
        assert_eq!(loud_seen.lock().expect("seen").len(), 1);
    }

    #[tokio::test]
    async fn two_runs_keep_their_lines_apart() {
        let (a, a_seen) = recording(false);
        let (b, b_seen) = recording(false);
        let one = tokio::spawn(scope(a, async { info("from a") }));
        let two = tokio::spawn(scope(b, async { info("from b") }));
        one.await.expect("a");
        two.await.expect("b");
        assert_eq!(a_seen.lock().expect("seen")[0].1, "from a");
        assert_eq!(b_seen.lock().expect("seen")[0].1, "from b");
        assert_eq!(a_seen.lock().expect("seen").len(), 1);
    }
}
