//! Step-level, commit-aware companion recovery.
//!
//! The iOS companion is a long-running `xcodebuild test` server the host talks
//! to over HTTP; the simulator host sometimes force-quits it mid-flow. That
//! surfaces to the driver as a connection failure. Rather than let a step time
//! out as a flow fault (`EF408` — nothing the test author controls), the
//! executor restarts the companion, [reconnects](golem_driver::PlatformDriver::reconnect)
//! the driver in place, and retries — but only when it is *safe* to do so.
//!
//! Safety hinges on whether the step's last non-idempotent mutation
//! (tap / type / swipe / …) actually landed. [`WitnessDriver`] wraps the real
//! driver and records that per step; [`decide`] maps the recorded
//! [`MutationState`] to the action the step loop takes ([`RecoveryDecision`]).
//! [`CompanionRecovery`] is the restart hook the CLI implements.

use async_trait::async_trait;
use golem_driver::{common, GestureFinger, PlatformDriver, ScreenshotResult};
use golem_element::Element;
use golem_events::FailureCode;
use std::sync::Mutex;

/// How many consecutive restarts that yield *zero* additional flow progress
/// are allowed before the flow is abandoned with `DeviceCompanionUnrecoverable`
/// (D506). A companion that can't stay up for even one step across this many
/// restarts is fatal — never a flow/app fault.
pub const MAX_COMPANION_RESTARTS: u32 = 3;

/// What the last non-idempotent mutation of the current step did, as observed
/// at the transport layer. Reset to [`None`](MutationState::None) at the start
/// of each step and overwritten by each mutation's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationState {
    /// No mutation ran yet this step (reads only), OR the companion died on a
    /// read before any mutation. A restart+retry re-runs the whole step safely.
    None,
    /// The last mutation returned `Ok` — it committed. If the companion then
    /// dies, re-running the step would double the action, so the step is
    /// deemed done instead.
    Committed,
    /// The last mutation failed with a clean connect-refused (D505): the
    /// request never left the host, so it did NOT land. Safe to restart+retry.
    NotCommitted,
    /// The last mutation failed with a mid-exchange drop (D507): the request
    /// may or may not have been applied before the socket died. Unknowable —
    /// neither retry nor deem-done is safe.
    Ambiguous,
}

/// Which mutation of a step, if any, commits its *semantic* effect — the only
/// one whose success may [`DeemDone`](RecoveryDecision::DeemDone) the step when
/// the companion then dies. A step often issues several driver mutations (a
/// `type` taps to focus *before* `type_text`; any `auto_scroll` step swipes to
/// navigate *before* its real action). Only the decisive one landing means the
/// step is done; a committed navigation mutation followed by a death leaves the
/// outcome unknown, so the step must retry, never silently pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalMutation {
    /// The action's sole mutation *is* its commit (no focus-tap, no
    /// auto-scroll): any committed mutation deems the step done.
    Any,
    /// Only this driver method commits the step; earlier mutations
    /// (a `type` focus-tap, auto-scroll swipes) are navigation and must not
    /// deem-done on their own.
    Only(&'static str),
    /// No mutation commits the step — a *read* decides it (`assert_*`, `read`,
    /// `scroll`-to-find). A committed navigation mutation must never deem-done;
    /// a death instead retries the whole step.
    Never,
}

/// The decisive mutation for an action, used to configure the [`WitnessDriver`]
/// per step. `Any` is the safe default for single-mutation actions whose only
/// mutation is their commit (`swipe`, `pinch`, `gesture`, `rotate`, `press`, …).
pub fn terminal_mutation(action: &str) -> TerminalMutation {
    match action {
        // Composite: a focus-tap precedes the real commit (`type_text`).
        "type" => TerminalMutation::Only("type_text"),
        // Auto-scroll-capable interactions: navigation swipes (via `gesture`)
        // may precede the terminal tap/press, so pin the commit to the action's
        // own method — a committed nav swipe must not deem-done.
        "tap" | "double_tap" => TerminalMutation::Only("tap"),
        "long_press" => TerminalMutation::Only("long_press"),
        // Read-decided: the effect is "an element became (in)visible / was
        // found". Auto-scroll swipes mutate but never commit these.
        "scroll" | "assert_visible" | "assert_not_visible" | "read" => TerminalMutation::Never,
        _ => TerminalMutation::Any,
    }
}

/// The step loop's action for a companion-death step failure, from the
/// witnessed [`MutationState`] and the restart budget. Pure so it is exhaustively
/// unit-testable without a driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryDecision {
    /// Committed: mark the step succeeded and advance (restart first so the
    /// next step meets a live companion).
    DeemDone,
    /// Ambiguous: fail the step with D507 (restart first so `if_fail`/teardown
    /// run against a live companion). Does NOT count toward the give-up.
    FailAmbiguous,
    /// None/NotCommitted with budget left: restart+reconnect, then retry.
    Retry,
    /// None/NotCommitted with the restart budget spent: abandon with D506.
    GiveUp,
}

/// Map a witnessed mutation state + restarts-already-used to the step loop's
/// action. `restarts_used` is the count of consecutive zero-progress restarts
/// so far; once it reaches `max_restarts` the recoverable path gives up.
pub fn decide(state: MutationState, restarts_used: u32, max_restarts: u32) -> RecoveryDecision {
    match state {
        MutationState::Committed => RecoveryDecision::DeemDone,
        MutationState::Ambiguous => RecoveryDecision::FailAmbiguous,
        MutationState::None | MutationState::NotCommitted => {
            if restarts_used >= max_restarts {
                RecoveryDecision::GiveUp
            } else {
                RecoveryDecision::Retry
            }
        }
    }
}

/// Whether a failure code is a companion death the recovery machine handles:
/// connect-refused (D505), mid-exchange drop (D507), or wedge (D503).
pub fn is_companion_death(code: Option<FailureCode>) -> bool {
    matches!(
        code,
        Some(
            FailureCode::DeviceCompanionUnreachable
                | FailureCode::DeviceCompanionDropped
                | FailureCode::DeviceCompanionWedged
        )
    )
}

/// Whether an error carries a companion-death code. Used by the polling
/// resolvers to break out *immediately* on a dead companion instead of
/// tolerating the error until the step deadline — a swallowed D505 would
/// surface as `EF408` (a flow fault), and the step loop's recovery only fires
/// on a companion-death code. Propagating fast is both the correct signal
/// (a dead socket won't answer a re-poll) and what lets recovery kick in.
pub fn is_companion_death_err(e: &anyhow::Error) -> bool {
    is_companion_death(golem_events::extract_code(e))
}

/// Restart-and-reconnect hook the executor calls when it detects a recoverable
/// companion death. Implemented by the CLI (relaunch the companion, then
/// [`PlatformDriver::reconnect`] the shared driver to its new port); `None` in
/// tests/stub keeps the current no-recovery behavior.
#[async_trait]
pub trait CompanionRecovery: Send + Sync {
    /// Relaunch the companion and repoint the driver at it. Returns once the
    /// companion is healthy again, or an error if it could not be brought back.
    async fn restart_and_reconnect(&self) -> anyhow::Result<()>;
}

/// A [`PlatformDriver`] decorator that records, per step, the commit state of
/// the last non-idempotent mutation. Reads and lifecycle calls pass through
/// untouched; the eight mutation methods (tap / long_press / swipe_coords /
/// pinch / gesture / type_text / backspace / press_button) update the state
/// from their result. Interior-mutable (the trait is `&self`).
pub struct WitnessDriver<'a> {
    inner: &'a dyn PlatformDriver,
    state: Mutex<MutationState>,
    terminal: Mutex<TerminalMutation>,
}

impl<'a> WitnessDriver<'a> {
    pub fn new(inner: &'a dyn PlatformDriver) -> Self {
        Self {
            inner,
            state: Mutex::new(MutationState::None),
            // Default: every mutation is decisive. The step loop narrows this
            // per step via `set_terminal` for composite / read-decided actions.
            terminal: Mutex::new(TerminalMutation::Any),
        }
    }

    /// Reset the commit state to `None` — called at the start of each step.
    /// Leaves the terminal designation untouched so a within-step retry keeps
    /// the same step's decisive-mutation rule.
    pub fn reset(&self) {
        *self.state.lock().expect("witness state poisoned") = MutationState::None;
    }

    /// Set which mutation commits the current step (see [`TerminalMutation`]).
    /// Called once per step (and preserved across the step's recovery retries).
    pub fn set_terminal(&self, terminal: TerminalMutation) {
        *self.terminal.lock().expect("witness terminal poisoned") = terminal;
    }

    /// Current commit state without clearing it.
    pub fn state(&self) -> MutationState {
        *self.state.lock().expect("witness state poisoned")
    }

    /// Read the commit state and reset it to `None`.
    pub fn take_state(&self) -> MutationState {
        let mut g = self.state.lock().expect("witness state poisoned");
        std::mem::replace(&mut *g, MutationState::None)
    }

    /// Update the commit state from a mutation's result. A successful mutation
    /// records `Committed` **only if it is the step's terminal mutation** (per
    /// [`TerminalMutation`]) — a committed navigation mutation (a focus-tap, an
    /// auto-scroll swipe) leaves the state alone so a later death retries rather
    /// than falsely deem-done. Failures classify by code regardless of which
    /// mutation they hit: D505 → NotCommitted (never left the host); D507 /
    /// D503 → Ambiguous (mid-exchange drop / timeout — may or may not have
    /// landed); any other error is left alone (a non-death failure must not
    /// reclassify an earlier real mutation).
    fn note_mutation<T>(&self, method: &'static str, result: &anyhow::Result<T>) {
        let next = match result {
            Ok(_) => {
                let terminal = *self.terminal.lock().expect("witness terminal poisoned");
                let is_terminal = match terminal {
                    TerminalMutation::Any => true,
                    TerminalMutation::Only(m) => m == method,
                    TerminalMutation::Never => false,
                };
                if is_terminal {
                    Some(MutationState::Committed)
                } else {
                    // Navigation mutation landed but the decisive one hasn't
                    // run: leave the state as-is (a step starts at `None`, so a
                    // subsequent death still retries).
                    None
                }
            }
            Err(e) => match golem_events::extract_code(e) {
                Some(FailureCode::DeviceCompanionUnreachable) => Some(MutationState::NotCommitted),
                Some(FailureCode::DeviceCompanionDropped) => Some(MutationState::Ambiguous),
                Some(FailureCode::DeviceCompanionWedged) => Some(MutationState::Ambiguous),
                _ => None,
            },
        };
        if let Some(s) = next {
            *self.state.lock().expect("witness state poisoned") = s;
        }
    }
}

#[async_trait]
impl PlatformDriver for WitnessDriver<'_> {
    // ── reads: pure delegation ──
    async fn get_hierarchy(&self) -> anyhow::Result<(Element, common::HierarchyMeta)> {
        self.inner.get_hierarchy().await
    }

    async fn screenshot(&self) -> anyhow::Result<ScreenshotResult> {
        self.inner.screenshot().await
    }

    // ── mutations (commit points): record state from the result ──
    async fn tap(&self, x: i32, y: i32) -> anyhow::Result<()> {
        let r = self.inner.tap(x, y).await;
        self.note_mutation("tap", &r);
        r
    }

    async fn long_press(&self, x: i32, y: i32, duration_ms: u64) -> anyhow::Result<()> {
        let r = self.inner.long_press(x, y, duration_ms).await;
        self.note_mutation("long_press", &r);
        r
    }

    async fn type_text(&self, text: &str) -> anyhow::Result<Option<bool>> {
        let r = self.inner.type_text(text).await;
        self.note_mutation("type_text", &r);
        r
    }

    async fn backspace(&self, count: u32) -> anyhow::Result<Option<bool>> {
        let r = self.inner.backspace(count).await;
        self.note_mutation("backspace", &r);
        r
    }

    async fn swipe_coords(
        &self,
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
    ) -> anyhow::Result<()> {
        let r = self.inner.swipe_coords(from_x, from_y, to_x, to_y).await;
        self.note_mutation("swipe_coords", &r);
        r
    }

    async fn pinch(&self, x: i32, y: i32, scale: f64, velocity: f64) -> anyhow::Result<()> {
        let r = self.inner.pinch(x, y, scale, velocity).await;
        self.note_mutation("pinch", &r);
        r
    }

    async fn gesture(&self, fingers: Vec<GestureFinger>) -> anyhow::Result<()> {
        let r = self.inner.gesture(fingers).await;
        self.note_mutation("gesture", &r);
        r
    }

    async fn press_button(&self, button: &str) -> anyhow::Result<()> {
        let r = self.inner.press_button(button).await;
        self.note_mutation("press_button", &r);
        r
    }

    // ── lifecycle / other: pure delegation, no commit tracking ──
    async fn hide_keyboard(&self) -> anyhow::Result<()> {
        self.inner.hide_keyboard().await
    }

    async fn launch_app(&self, bundle_id: &str) -> anyhow::Result<Option<String>> {
        self.inner.launch_app(bundle_id).await
    }

    async fn stop_app(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.inner.stop_app(bundle_id).await
    }

    async fn clear_app_data(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.inner.clear_app_data(bundle_id).await
    }

    async fn set_dark_mode(&self, enabled: bool) -> anyhow::Result<()> {
        self.inner.set_dark_mode(enabled).await
    }

    async fn set_location(&self, lat: f64, lon: f64) -> anyhow::Result<()> {
        self.inner.set_location(lat, lon).await
    }

    async fn open_url(&self, url: &str) -> anyhow::Result<()> {
        self.inner.open_url(url).await
    }

    async fn push_notification(
        &self,
        title: &str,
        body: &str,
        payload: Option<&str>,
    ) -> anyhow::Result<()> {
        self.inner.push_notification(title, body, payload).await
    }

    async fn add_media(&self, path: &str) -> anyhow::Result<()> {
        self.inner.add_media(path).await
    }

    async fn grant_permission(&self, bundle_id: &str, permission: &str) -> anyhow::Result<()> {
        self.inner.grant_permission(bundle_id, permission).await
    }

    async fn revoke_permission(&self, bundle_id: &str, permission: &str) -> anyhow::Result<()> {
        self.inner.revoke_permission(bundle_id, permission).await
    }

    async fn start_recording(&self, name: &str) -> anyhow::Result<()> {
        self.inner.start_recording(name).await
    }

    async fn stop_recording(&self) -> anyhow::Result<String> {
        self.inner.stop_recording().await
    }

    fn last_recording_end(&self) -> Option<std::time::Instant> {
        self.inner.last_recording_end()
    }

    async fn remove_port_forwards(&self) -> anyhow::Result<()> {
        self.inner.remove_port_forwards().await
    }

    async fn poke_for_system_alert(&self) -> anyhow::Result<()> {
        self.inner.poke_for_system_alert().await
    }

    async fn accept_system_alert(&self) -> anyhow::Result<bool> {
        self.inner.accept_system_alert().await
    }

    async fn prepare_type(&self, text: &str) -> anyhow::Result<()> {
        self.inner.prepare_type(text).await
    }

    fn set_request_timeout(&self, timeout: std::time::Duration) {
        self.inner.set_request_timeout(timeout);
    }

    fn reconnect(&self, port: u16) {
        self.inner.reconnect(port);
    }

    async fn await_first_frame(&self) -> anyhow::Result<Option<String>> {
        self.inner.await_first_frame().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::{Bounds, Element};

    fn tree() -> Element {
        Element {
            element_type: "View".to_string(),
            text: None,
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds: Bounds::new(0, 0, 100, 100),
            visible_bounds: None,
            hit_points: vec![],
            drawing_order: None,
            children: vec![],
        }
    }

    // ── decide(): the full decision table ──

    #[test]
    fn decide_committed_deems_done() {
        assert_eq!(
            decide(MutationState::Committed, 0, MAX_COMPANION_RESTARTS),
            RecoveryDecision::DeemDone,
            "a committed mutation SHALL deem the step done regardless of budget"
        );
        // Budget-independent.
        assert_eq!(
            decide(
                MutationState::Committed,
                MAX_COMPANION_RESTARTS,
                MAX_COMPANION_RESTARTS
            ),
            RecoveryDecision::DeemDone
        );
    }

    #[test]
    fn decide_ambiguous_fails() {
        assert_eq!(
            decide(MutationState::Ambiguous, 0, MAX_COMPANION_RESTARTS),
            RecoveryDecision::FailAmbiguous,
            "an ambiguous mutation SHALL fail the step (D507), never retry or deem-done"
        );
    }

    #[test]
    fn decide_recoverable_retries_until_budget_then_gives_up() {
        for state in [MutationState::None, MutationState::NotCommitted] {
            assert_eq!(decide(state, 0, 3), RecoveryDecision::Retry);
            assert_eq!(decide(state, 2, 3), RecoveryDecision::Retry);
            // At the budget, the next restart is refused: give up with D506.
            assert_eq!(
                decide(state, 3, 3),
                RecoveryDecision::GiveUp,
                "{state:?} SHALL give up once restarts_used reaches max"
            );
            assert_eq!(decide(state, 4, 3), RecoveryDecision::GiveUp);
        }
    }

    // ── is_companion_death(): the three death codes and nothing else ──

    #[test]
    fn companion_death_codes() {
        assert!(is_companion_death(Some(
            FailureCode::DeviceCompanionUnreachable
        )));
        assert!(is_companion_death(Some(
            FailureCode::DeviceCompanionDropped
        )));
        assert!(is_companion_death(Some(FailureCode::DeviceCompanionWedged)));
        assert!(!is_companion_death(Some(FailureCode::FlowElementNotFound)));
        assert!(!is_companion_death(Some(
            FailureCode::DeviceCompanionUnrecoverable
        )));
        assert!(!is_companion_death(None));
    }

    // ── WitnessDriver: mutation outcomes map to commit states ──

    #[tokio::test]
    async fn witness_records_committed_on_ok_mutation() {
        let mock = MockPlatformDriver::new(tree());
        let w = WitnessDriver::new(&mock);
        w.tap(1, 2).await.expect("tap SHALL be Ok");
        assert_eq!(
            w.state(),
            MutationState::Committed,
            "an Ok mutation SHALL record Committed"
        );
    }

    #[tokio::test]
    async fn witness_records_not_committed_on_d505() {
        let mock = MockPlatformDriver::new(tree());
        mock.set_error_coded("tap", FailureCode::DeviceCompanionUnreachable, "refused");
        let w = WitnessDriver::new(&mock);
        w.tap(1, 2).await.expect_err("tap SHALL fail D505");
        assert_eq!(
            w.state(),
            MutationState::NotCommitted,
            "a D505 mutation SHALL record NotCommitted"
        );
    }

    #[tokio::test]
    async fn witness_records_ambiguous_on_d507() {
        let mock = MockPlatformDriver::new(tree());
        mock.set_error_coded("type_text", FailureCode::DeviceCompanionDropped, "dropped");
        let w = WitnessDriver::new(&mock);
        w.type_text("hi")
            .await
            .expect_err("type_text SHALL fail D507");
        assert_eq!(
            w.state(),
            MutationState::Ambiguous,
            "a D507 mutation SHALL record Ambiguous"
        );
    }

    #[tokio::test]
    async fn witness_records_ambiguous_on_d503() {
        // A mutation that TIMES OUT (D503, companion wedged) may or may not have
        // landed — the ambiguous case, exactly like a mid-exchange drop (D507).
        let mock = MockPlatformDriver::new(tree());
        mock.set_error_coded("tap", FailureCode::DeviceCompanionWedged, "wedged");
        let w = WitnessDriver::new(&mock);
        w.tap(1, 2).await.expect_err("tap SHALL fail D503");
        assert_eq!(
            w.state(),
            MutationState::Ambiguous,
            "a D503 (timeout) mutation SHALL record Ambiguous, not leave the state at None"
        );
    }

    // ── terminal-mutation gating: only the decisive mutation deem-dones ──

    #[tokio::test]
    async fn witness_terminal_only_gates_deem_done() {
        // A `type` step: `type_text` is the commit; the focus-`tap` before it is
        // navigation. Pin the terminal to `type_text`.
        let mock = MockPlatformDriver::new(tree());
        let w = WitnessDriver::new(&mock);
        w.set_terminal(TerminalMutation::Only("type_text"));

        // The focus-tap commits, but it is NOT terminal → state stays None, so a
        // death on the next read would retry (not falsely deem the step done).
        w.tap(1, 2).await.expect("focus tap Ok");
        assert_eq!(
            w.state(),
            MutationState::None,
            "a committed non-terminal mutation (focus-tap) SHALL NOT deem-done"
        );

        // The terminal mutation commits → Committed (deem-done eligible), so a
        // later death correctly deems the step done (re-typing would duplicate).
        w.type_text("hi").await.expect("type_text Ok");
        assert_eq!(
            w.state(),
            MutationState::Committed,
            "a committed terminal mutation (type_text) SHALL deem-done"
        );
    }

    #[tokio::test]
    async fn witness_terminal_never_never_deems_done() {
        // A read-decided step (scroll / assert with auto_scroll): navigation
        // swipes commit but never make the step "done".
        let mock = MockPlatformDriver::new(tree());
        let w = WitnessDriver::new(&mock);
        w.set_terminal(TerminalMutation::Never);
        w.swipe_coords(0, 0, 0, 100).await.expect("nav swipe Ok");
        assert_eq!(
            w.state(),
            MutationState::None,
            "with TerminalMutation::Never, no committed mutation SHALL deem-done"
        );
    }

    #[test]
    fn terminal_mutation_maps_actions() {
        assert_eq!(
            terminal_mutation("type"),
            TerminalMutation::Only("type_text")
        );
        assert_eq!(terminal_mutation("tap"), TerminalMutation::Only("tap"));
        assert_eq!(
            terminal_mutation("double_tap"),
            TerminalMutation::Only("tap")
        );
        assert_eq!(
            terminal_mutation("long_press"),
            TerminalMutation::Only("long_press")
        );
        assert_eq!(terminal_mutation("scroll"), TerminalMutation::Never);
        assert_eq!(terminal_mutation("assert_visible"), TerminalMutation::Never);
        // Single-mutation actions with no navigation default to Any.
        assert_eq!(terminal_mutation("swipe"), TerminalMutation::Any);
        assert_eq!(terminal_mutation("pinch"), TerminalMutation::Any);
    }

    #[tokio::test]
    async fn witness_reads_do_not_touch_state() {
        let mock = MockPlatformDriver::new(tree());
        let w = WitnessDriver::new(&mock);
        w.get_hierarchy().await.expect("read SHALL be Ok");
        w.screenshot().await.expect("read SHALL be Ok");
        assert_eq!(
            w.state(),
            MutationState::None,
            "reads SHALL leave the commit state untouched"
        );
    }

    #[tokio::test]
    async fn witness_last_mutation_wins_and_take_resets() {
        let mock = MockPlatformDriver::new(tree());
        let w = WitnessDriver::new(&mock);
        // A focus-tap commits...
        w.tap(1, 2).await.expect("tap Ok");
        assert_eq!(w.state(), MutationState::Committed);
        // ...then the real commit point (type_text) drops mid-exchange: it wins.
        mock.set_error_coded("type_text", FailureCode::DeviceCompanionDropped, "dropped");
        w.type_text("x").await.expect_err("type_text SHALL fail");
        assert_eq!(
            w.state(),
            MutationState::Ambiguous,
            "the last mutation's outcome SHALL overwrite an earlier one"
        );
        assert_eq!(w.take_state(), MutationState::Ambiguous);
        assert_eq!(
            w.state(),
            MutationState::None,
            "take_state SHALL reset the witness"
        );
    }

    #[tokio::test]
    async fn witness_non_death_error_does_not_reclassify() {
        let mock = MockPlatformDriver::new(tree());
        let w = WitnessDriver::new(&mock);
        w.tap(0, 0).await.expect("tap Ok"); // Committed
                                            // A subsequent non-death mutation error must NOT wipe the commit.
        mock.set_error("long_press", "some unrelated failure");
        w.long_press(0, 0, 10)
            .await
            .expect_err("long_press SHALL fail");
        assert_eq!(
            w.state(),
            MutationState::Committed,
            "a non-death mutation error SHALL leave the prior commit state intact"
        );
    }

    // reconnect + set_request_timeout SHALL delegate through the witness (a
    // no-op default would silently break the real driver).
    #[test]
    fn witness_delegates_reconnect() {
        let mock = MockPlatformDriver::new(tree());
        let w = WitnessDriver::new(&mock);
        w.reconnect(9191);
        assert_eq!(
            mock.reconnect_port(),
            Some(9191),
            "witness reconnect SHALL forward to the inner driver"
        );
    }
}
