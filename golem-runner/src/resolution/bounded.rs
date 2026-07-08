use std::time::Duration;

use golem_driver::PlatformDriver;
use golem_element::Element;

/// Per-call deadline for `driver.get_hierarchy()`. Normal fetches
/// return in 100-300ms; under sweep load with large hierarchies a
/// slow-tail fetch can reach 2-4s. A true wedge (e.g. UiAutomation
/// lost its accessibility handle after a focus-changing action)
/// hangs the .await indefinitely. The ceiling separates "slow but
/// recovering" from "wedged forever" — high enough to absorb slow
/// tails, low enough to fail fast on real wedges.
const HIERARCHY_FETCH_TIMEOUT: Duration = Duration::from_millis(6000);

/// Wrap `driver.get_hierarchy()` with a hard per-call timeout.
/// Returns the same shape as the underlying call; treats a timeout
/// as an error so callers fall through their `Err(_)` arms (which
/// already exist for transient network/companion failures).
pub(crate) async fn get_hierarchy_bounded(
    driver: &dyn PlatformDriver,
) -> anyhow::Result<(Element, golem_driver::common::HierarchyMeta)> {
    match tokio::time::timeout(HIERARCHY_FETCH_TIMEOUT, driver.get_hierarchy()).await {
        Ok(r) => r,
        Err(_) => crate::fail_code!(
            golem_events::FailureCode::DeviceCompanionWedged,
            "hierarchy fetch timed out after {}ms (companion likely wedged)",
            HIERARCHY_FETCH_TIMEOUT.as_millis()
        ),
    }
}

const SCREENSHOT_FETCH_TIMEOUT: Duration = Duration::from_millis(6000);
const SWIPE_TIMEOUT: Duration = Duration::from_millis(6000);

/// Scroll/auto_scroll swipe with dwell-before-lift to suppress fling
/// momentum. A regular `swipe` (finger down → move → lift) leaves the
/// velocity tracker with a non-zero release velocity, and Android adds
/// momentum scroll on top of the gesture — frequently overshooting the
/// target by 2-3× the swipe distance, so the resolver scrolls past the
/// element before the next hierarchy sample can see it in the viewport.
///
/// Implemented via the multi-touch gesture endpoint with a single-finger
/// 3-point path `(from, to, to)`: the interpolator splits the duration
/// evenly across the two segments, so the finger holds still at the
/// end for ~half the duration. The velocity tracker reads near-zero
/// at UP → no fling → page scrolls exactly the swipe distance.
pub(crate) async fn scroll_swipe_bounded(
    driver: &dyn PlatformDriver,
    fx: i32,
    fy: i32,
    tx: i32,
    ty: i32,
) -> anyhow::Result<()> {
    let finger = golem_driver::GestureFinger {
        points: vec![(fx, fy), (tx, ty), (tx, ty)],
        duration_ms: 600,
    };
    match tokio::time::timeout(SWIPE_TIMEOUT, driver.gesture(vec![finger])).await {
        Ok(r) => r,
        Err(_) => crate::fail_code!(
            golem_events::FailureCode::DeviceCompanionWedged,
            "scroll swipe timed out after {}ms (companion likely wedged)",
            SWIPE_TIMEOUT.as_millis()
        ),
    }
}

pub(crate) async fn screenshot_bounded(
    driver: &dyn PlatformDriver,
) -> anyhow::Result<golem_driver::ScreenshotResult> {
    match tokio::time::timeout(SCREENSHOT_FETCH_TIMEOUT, driver.screenshot()).await {
        Ok(r) => r,
        Err(_) => crate::fail_code!(
            golem_events::FailureCode::DeviceCompanionWedged,
            "screenshot timed out after {}ms (companion likely wedged)",
            SCREENSHOT_FETCH_TIMEOUT.as_millis()
        ),
    }
}
