//! The flow's browser, and the one place that knows whether this build has one.
//!
//! Browser support is a cargo feature, so every `browse_*` step has two
//! possible fates: run, or explain that this binary can't run it. Keeping both
//! behind one type means the executor, policy and dispatch paths are written
//! once, with no `#[cfg]` threaded through them.

use std::path::Path;

use anyhow::Result;
use golem_parser::Step;
use golem_vars::VariableStore;

/// Holds a flow's browser once something asks for one.
///
/// Lazy by design: a slot costs nothing, so every flow can carry one and only
/// the flows that reach a `browse_*` step ever start Chrome.
pub struct BrowserSlot {
    #[cfg(feature = "browser")]
    pool: Option<golem_browser::BrowserPool>,
    #[cfg(feature = "browser")]
    headless: bool,
}

impl Default for BrowserSlot {
    /// Headless. Spelled out rather than derived because `bool::default()` is
    /// `false`, which would quietly make every unconfigured flow headed.
    fn default() -> Self {
        Self::new(true)
    }
}

impl BrowserSlot {
    /// A slot whose browser, if one is ever launched, runs headless or headed.
    /// Resolved by the caller from CLI flag, then flow option, then the default.
    pub fn new(headless: bool) -> Self {
        #[cfg(feature = "browser")]
        {
            Self {
                pool: None,
                headless,
            }
        }
        #[cfg(not(feature = "browser"))]
        {
            let _ = headless;
            Self {}
        }
    }

    /// Whether a browser is actually running — the flow-end teardown uses this
    /// to stay silent about flows that never touched one.
    pub fn is_active(&self) -> bool {
        #[cfg(feature = "browser")]
        {
            self.pool.as_ref().is_some_and(|p| p.is_running())
        }
        #[cfg(not(feature = "browser"))]
        {
            false
        }
    }

    #[cfg(feature = "browser")]
    pub async fn run_step(
        &mut self,
        step: &Step,
        vars: &mut VariableStore,
        flow_dir: &Path,
        project_root: &Path,
    ) -> Result<()> {
        let pool = self.pool.get_or_insert_with(|| {
            golem_browser::BrowserPool::new(golem_browser::PoolConfig {
                headless: self.headless,
            })
        });
        golem_browser::execute_browser_action(
            pool,
            step,
            vars,
            golem_browser::ScriptPaths {
                flow_dir,
                project_root,
            },
        )
        .await
    }

    /// Without the feature there is no browser to run the step on. This is the
    /// same H501 the suite preflight raises — a flow reaching here means the
    /// step was authored fine and this build simply can't serve it, which is
    /// the operator's problem, not the test author's.
    #[cfg(not(feature = "browser"))]
    pub async fn run_step(
        &mut self,
        step: &Step,
        _vars: &mut VariableStore,
        _flow_dir: &Path,
        _project_root: &Path,
    ) -> Result<()> {
        Err(golem_events::coded(
            golem_events::FailureCode::HostBrowserUnsupported,
            anyhow::anyhow!(
                "`{}` needs browser support, but this golem was built without it — \
                 rebuild without `--no-default-features`, or with `--features browser`",
                step.action
            ),
        ))
    }

    /// Release the browser. Safe to call when nothing was ever launched.
    pub async fn close(&mut self) -> Result<()> {
        #[cfg(feature = "browser")]
        {
            if let Some(pool) = self.pool.as_mut() {
                return pool.close().await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(toml_src: &str) -> Step {
        toml::from_str(toml_src).expect("fixture SHALL parse")
    }

    // 1. An unlaunched slot reports no browser, so flow-end teardown stays
    //    quiet about flows that never touched one.
    #[test]
    fn a_fresh_slot_has_no_browser() {
        assert!(!BrowserSlot::default().is_active());
        assert!(!BrowserSlot::new(false).is_active());
    }

    // 2. Closing a slot that never launched is a no-op — teardown runs on
    //    every flow, browser or not.
    #[tokio::test]
    async fn closing_an_unused_slot_succeeds() {
        BrowserSlot::default()
            .close()
            .await
            .expect("closing an unused slot SHALL succeed");
    }

    // 3. Without the feature, a browser step is a HOST failure naming the
    //    build — never an unknown action, which would blame the test author
    //    for a decision made at compile time.
    #[cfg(not(feature = "browser"))]
    #[tokio::test]
    async fn without_the_feature_a_browser_step_reports_h501() {
        let mut vars = VariableStore::new();
        let e = BrowserSlot::default()
            .run_step(
                &step(r#"action = "browse_navigate""#),
                &mut vars,
                Path::new("."),
                Path::new("."),
            )
            .await
            .expect_err("a browser step SHALL fail without browser support");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(golem_events::FailureCode::HostBrowserUnsupported)
        );
        assert!(format!("{e:#}").contains("browse_navigate"));
    }

    // 4. With the feature, the step reaches the browser crate's own dispatch:
    //    a malformed one comes back as its param error, and nothing launches.
    #[cfg(feature = "browser")]
    #[tokio::test]
    async fn with_the_feature_steps_reach_the_browser_dispatch() {
        let mut vars = VariableStore::new();
        let mut slot = BrowserSlot::default();
        let e = slot
            .run_step(
                &step(r#"action = "browse_navigate""#),
                &mut vars,
                Path::new("."),
                Path::new("."),
            )
            .await
            .expect_err("navigate without a url SHALL fail");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(golem_events::FailureCode::ParseMissingParam)
        );
        assert!(
            !slot.is_active(),
            "a param error SHALL NOT launch a browser"
        );
    }
}
