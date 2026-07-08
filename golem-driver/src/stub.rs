//! Stub platform driver for in-process integration tests.
//!
//! Unit tests cover individual modules well, but the end-to-end
//! *composition* — CLI args → config → orchestrator server →
//! `submit_and_wait` → event stream → renderer → result files → exit
//! code — has no automated coverage, and real bugs have shipped through
//! that gap (a `--output toon` that silently produced human output; a
//! daemon path that skipped writing top-level results files). Those are
//! pure wiring bugs: every individual unit was fine.
//!
//! `StubDriver` closes that gap. It implements [`PlatformDriver`] with
//! deterministic, device-free behaviour so a real suite can be driven
//! entirely in-process. It is NOT a fidelity model of a device — element
//! targeting, visibility filtering, and gesture mechanics have their own
//! unit + real-device coverage. The stub's job is to serve a tree that a
//! trivial fixture flow can resolve and assert against, and to fail
//! *deterministically* on chosen runs so the composition layer (output
//! format selection, per-run `output_dir` layout, flake aggregation
//! across `--repeat`, exit codes, IPC contract) can be exercised.
//!
//! Failure is modelled by *what the tree contains*, not a magic hook: on
//! a run listed in [`StubScript::fail_on_runs`] the stub serves a tree
//! WITHOUT the fixture target, so the flow's assertion fails through the
//! real assert path — same code that judges a real device.

use crate::{common, GestureFinger, PlatformDriver, ScreenshotResult};
use async_trait::async_trait;
use golem_element::{Bounds, Element};
use serde::Deserialize;
use std::sync::Mutex;

/// Bundle/package id the stub fixture flows launch. Arbitrary — the stub
/// never touches a real install — but kept stable so fixtures can name it.
pub const STUB_BUNDLE_ID: &str = "fail.golem.stub";

/// Visible text on the fixture's primary target element. A fixture flow
/// asserts this is present; it is absent from the fail tree.
pub const STUB_TARGET_TEXT: &str = "Submit";

/// Script controlling [`StubDriver`] behaviour across a `--repeat` run set.
///
/// Deserialised from a small TOML file passed via the hidden `--stub`
/// flag. Flat, no wrapper table:
/// ```toml
/// fail_on_runs = [2]
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StubScript {
    /// 1-based run indices that SHALL fail. On these runs the stub serves
    /// a tree without the fixture target, so the flow assertion fails
    /// through the real assert path. Empty (default) = every run passes.
    pub fail_on_runs: Vec<u32>,
}

impl StubScript {
    /// Parse a stub script from TOML text.
    pub fn from_toml_str(s: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(s)?)
    }

    /// Whether the given 1-based run index is scripted to fail.
    fn fails_run(&self, run_index: u32) -> bool {
        self.fail_on_runs.contains(&run_index)
    }
}

/// Deterministic, device-free [`PlatformDriver`]. Constructed per FlowRun
/// with the run's 1-based index so scripted per-run failure is possible
/// without cross-run shared state.
pub struct StubDriver {
    /// 1-based index of this FlowRun within a `--repeat` set (1 when not
    /// repeating). Drives `StubScript::fail_on_runs`.
    run_index: u32,
    script: StubScript,
    /// Record of all method calls (method_name, args), like the mock
    /// driver — lets integration tests assert the step→driver-call mapping.
    calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl StubDriver {
    /// Build a stub for the given 1-based run index and script.
    pub fn new(run_index: u32, script: StubScript) -> Self {
        Self {
            run_index,
            script,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// All recorded calls, in order.
    pub fn get_calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().expect("lock poisoned").clone()
    }

    fn record(&self, method: &str, args: Vec<String>) {
        self.calls
            .lock()
            .expect("lock poisoned")
            .push((method.to_string(), args));
    }

    /// Whether this run is scripted to fail (serves the target-less tree).
    fn should_fail(&self) -> bool {
        self.script.fails_run(self.run_index)
    }

    /// The tree this run serves: the pass tree normally, the fail tree
    /// (missing the fixture target) on a scripted-failure run.
    fn tree(&self) -> Element {
        if self.should_fail() {
            fail_tree()
        } else {
            pass_tree()
        }
    }
}

/// Fixture viewport — a generic phone-ish size. The stub reports this as
/// both `bounds` and `visible_bounds` on the root so `filter_viewport`
/// keeps everything (nothing is scrolled off).
const VIEWPORT: (i32, i32) = (375, 812);

/// Build a fully-visible element. `visible_bounds` mirrors `bounds` so the
/// viewport filter and occlusion logic treat it as on-screen and tappable.
fn stub_el(
    element_type: &str,
    text: Option<&str>,
    placeholder: Option<&str>,
    bounds: Bounds,
) -> Element {
    Element {
        element_type: element_type.to_string(),
        text: text.map(str::to_string),
        accessibility_label: text.map(str::to_string),
        placeholder: placeholder.map(str::to_string),
        enabled: true,
        checked: false,
        clickable: text.is_some(),
        focused: false,
        bounds,
        visible_bounds: Some(bounds),
        hit_points: Vec::new(),
        drawing_order: None,
        children: Vec::new(),
    }
}

/// Root padded with `filler` benign labelled rows below the interactive
/// elements. The padding pushes the node count above the runner's
/// post-launch settle threshold (`AWAIT_FIRST_FRAME_MIN_NODES`) so
/// `await_first_frame` settles in a couple of polls instead of waiting out
/// its multi-second deadline — keeping stub runs fast.
fn padded_root() -> Element {
    const FILLER: usize = 24;
    let mut root = stub_el(
        "View",
        None,
        None,
        Bounds::new(0, 0, VIEWPORT.0, VIEWPORT.1),
    );
    root.children.push(stub_el(
        "TextField",
        None,
        Some("email"),
        Bounds::new(20, 100, 335, 44),
    ));
    for i in 0..FILLER {
        let y = 220 + (i as i32) * 20;
        root.children.push(stub_el(
            "Text",
            Some(&format!("row {i}")),
            None,
            Bounds::new(20, y, 335, 18),
        ));
    }
    root
}

/// Tree served on passing runs: the padded base plus the Submit button, all
/// visible and within the viewport, so a fixture flow can assert/tap it.
fn pass_tree() -> Element {
    let mut root = padded_root();
    root.children.push(stub_el(
        "Button",
        Some(STUB_TARGET_TEXT),
        None,
        Bounds::new(20, 160, 335, 48),
    ));
    root
}

/// Tree served on scripted-failure runs: the same padded base WITHOUT the
/// Submit button, so a fixture flow that asserts the target present fails
/// through the real assert path (still above the settle threshold).
fn fail_tree() -> Element {
    padded_root()
}

#[async_trait]
impl PlatformDriver for StubDriver {
    async fn get_hierarchy(&self) -> anyhow::Result<(Element, common::HierarchyMeta)> {
        self.record("get_hierarchy", vec![]);
        let tree = self.tree();
        let meta = common::HierarchyMeta {
            node_count: tree.node_count() as u32,
            ..common::HierarchyMeta::default()
        };
        Ok((tree, meta))
    }

    async fn tap(&self, x: i32, y: i32) -> anyhow::Result<()> {
        self.record("tap", vec![x.to_string(), y.to_string()]);
        Ok(())
    }

    async fn long_press(&self, x: i32, y: i32, duration_ms: u64) -> anyhow::Result<()> {
        self.record(
            "long_press",
            vec![x.to_string(), y.to_string(), duration_ms.to_string()],
        );
        Ok(())
    }

    async fn type_text(&self, text: &str) -> anyhow::Result<Option<bool>> {
        self.record("type_text", vec![text.to_string()]);
        Ok(None)
    }

    async fn backspace(&self, count: u32) -> anyhow::Result<Option<bool>> {
        self.record("backspace", vec![count.to_string()]);
        Ok(None)
    }

    async fn swipe_coords(
        &self,
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
    ) -> anyhow::Result<()> {
        self.record(
            "swipe_coords",
            vec![
                from_x.to_string(),
                from_y.to_string(),
                to_x.to_string(),
                to_y.to_string(),
            ],
        );
        Ok(())
    }

    async fn pinch(&self, x: i32, y: i32, scale: f64, velocity: f64) -> anyhow::Result<()> {
        self.record(
            "pinch",
            vec![
                x.to_string(),
                y.to_string(),
                format!("{scale}"),
                format!("{velocity}"),
            ],
        );
        Ok(())
    }

    async fn gesture(&self, fingers: Vec<GestureFinger>) -> anyhow::Result<()> {
        let args: Vec<String> = fingers
            .iter()
            .map(|f| format!("{}pts@{}ms", f.points.len(), f.duration_ms))
            .collect();
        self.record("gesture", args);
        Ok(())
    }

    async fn screenshot(&self) -> anyhow::Result<ScreenshotResult> {
        self.record("screenshot", vec![]);
        Ok(ScreenshotResult {
            path: "stub_screenshot.png".to_string(),
            data: vec![0x89, 0x50, 0x4E, 0x47], // PNG magic bytes
        })
    }

    async fn hide_keyboard(&self) -> anyhow::Result<()> {
        self.record("hide_keyboard", vec![]);
        Ok(())
    }

    async fn launch_app(&self, bundle_id: &str) -> anyhow::Result<Option<String>> {
        self.record("launch_app", vec![bundle_id.to_string()]);
        Ok(None)
    }

    async fn stop_app(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.record("stop_app", vec![bundle_id.to_string()]);
        Ok(())
    }

    async fn clear_app_data(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.record("clear_app_data", vec![bundle_id.to_string()]);
        Ok(())
    }

    async fn press_button(&self, button: &str) -> anyhow::Result<()> {
        self.record("press_button", vec![button.to_string()]);
        Ok(())
    }

    async fn set_dark_mode(&self, enabled: bool) -> anyhow::Result<()> {
        self.record("set_dark_mode", vec![enabled.to_string()]);
        Ok(())
    }

    async fn set_location(&self, lat: f64, lon: f64) -> anyhow::Result<()> {
        self.record("set_location", vec![lat.to_string(), lon.to_string()]);
        Ok(())
    }

    async fn open_url(&self, url: &str) -> anyhow::Result<()> {
        self.record("open_url", vec![url.to_string()]);
        Ok(())
    }

    async fn push_notification(
        &self,
        title: &str,
        body: &str,
        payload: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut args = vec![title.to_string(), body.to_string()];
        if let Some(p) = payload {
            args.push(p.to_string());
        }
        self.record("push_notification", args);
        Ok(())
    }

    async fn add_media(&self, path: &str) -> anyhow::Result<()> {
        self.record("add_media", vec![path.to_string()]);
        Ok(())
    }

    async fn grant_permission(&self, bundle_id: &str, permission: &str) -> anyhow::Result<()> {
        self.record(
            "grant_permission",
            vec![bundle_id.to_string(), permission.to_string()],
        );
        Ok(())
    }

    async fn revoke_permission(&self, bundle_id: &str, permission: &str) -> anyhow::Result<()> {
        self.record(
            "revoke_permission",
            vec![bundle_id.to_string(), permission.to_string()],
        );
        Ok(())
    }

    async fn start_recording(&self, name: &str) -> anyhow::Result<()> {
        self.record("start_recording", vec![name.to_string()]);
        Ok(())
    }

    async fn stop_recording(&self) -> anyhow::Result<String> {
        self.record("stop_recording", vec![]);
        Ok("stub_recording.mp4".to_string())
    }

    async fn remove_port_forwards(&self) -> anyhow::Result<()> {
        self.record("remove_port_forwards", vec![]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_target(tree: &Element) -> bool {
        if tree.text.as_deref() == Some(STUB_TARGET_TEXT) {
            return true;
        }
        tree.children.iter().any(has_target)
    }

    #[tokio::test]
    async fn passing_run_serves_tree_with_target() {
        let driver = StubDriver::new(1, StubScript::default());
        let (tree, meta) = driver.get_hierarchy().await.expect("get_hierarchy");
        assert!(
            has_target(&tree),
            "passing run SHALL serve the fixture target"
        );
        assert_eq!(meta.node_count, tree.node_count() as u32);
    }

    // Both trees SHALL stay above the runner's post-launch settle threshold
    // so `await_first_frame` settles fast instead of polling out its
    // multi-second deadline (which would make every stub run slow).
    #[tokio::test]
    async fn both_trees_exceed_settle_threshold() {
        use crate::AWAIT_FIRST_FRAME_MIN_NODES as MIN;
        let pass = StubDriver::new(1, StubScript::default());
        let (pt, _) = pass.get_hierarchy().await.expect("get_hierarchy");
        let fail = StubDriver::new(
            1,
            StubScript {
                fail_on_runs: vec![1],
            },
        );
        let (ft, _) = fail.get_hierarchy().await.expect("get_hierarchy");
        assert!(
            pt.node_count() > MIN,
            "pass tree {} SHALL exceed {MIN}",
            pt.node_count()
        );
        assert!(
            ft.node_count() > MIN,
            "fail tree {} SHALL exceed {MIN}",
            ft.node_count()
        );
    }

    #[tokio::test]
    async fn scripted_fail_run_serves_tree_without_target() {
        let script = StubScript {
            fail_on_runs: vec![2],
        };
        // Run 2 is scripted to fail → target absent.
        let failing = StubDriver::new(2, script.clone());
        let (fail_tree, _) = failing.get_hierarchy().await.expect("get_hierarchy");
        assert!(
            !has_target(&fail_tree),
            "scripted-fail run SHALL omit the target so the assert fails"
        );

        // Runs 1 and 3 pass → target present (flake shape: pass/fail/pass).
        for run in [1, 3] {
            let ok = StubDriver::new(run, script.clone());
            let (tree, _) = ok.get_hierarchy().await.expect("get_hierarchy");
            assert!(
                has_target(&tree),
                "run {run} SHALL pass with the target present"
            );
        }
    }

    #[tokio::test]
    async fn records_calls_in_order() {
        let driver = StubDriver::new(1, StubScript::default());
        driver.launch_app(STUB_BUNDLE_ID).await.expect("launch");
        driver.tap(10, 20).await.expect("tap");
        driver.type_text("hello").await.expect("type");
        let calls = driver.get_calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].0, "launch_app");
        assert_eq!(calls[0].1, vec![STUB_BUNDLE_ID]);
        assert_eq!(
            calls[1],
            ("tap".to_string(), vec!["10".to_string(), "20".to_string()])
        );
        assert_eq!(
            calls[2],
            ("type_text".to_string(), vec!["hello".to_string()])
        );
    }

    #[test]
    fn script_parses_from_toml() {
        let s = StubScript::from_toml_str("fail_on_runs = [2, 4]").expect("parse");
        assert_eq!(s.fail_on_runs, vec![2, 4]);
    }

    #[test]
    fn empty_script_parses_to_no_failures() {
        let s = StubScript::from_toml_str("").expect("parse empty");
        assert!(s.fail_on_runs.is_empty());
    }

    #[test]
    fn unknown_field_is_rejected() {
        // deny_unknown_fields guards against silently-ignored typos in a
        // hand-written stub script.
        assert!(StubScript::from_toml_str("fail_on_run = [2]").is_err());
    }
}
