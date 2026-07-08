use std::time::Duration;

use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_element::Element;
use tokio::time::Instant;

use super::bounded::get_hierarchy_bounded;

/// Maximum time to wait for the UI hierarchy to stabilize (1.5 seconds).
const SETTLE_TIMEOUT: Duration = Duration::from_millis(1500);

/// Interval between settle comparison checks (250ms).
const SETTLE_INTERVAL: Duration = Duration::from_millis(250);

/// Extended settle budget, used for the one settle immediately after a
/// `/type`/`/backspace` whose mutation the companion couldn't confirm
/// (slow IME). Gives the field up to 3s to update without burning extra
/// a11y calls on the happy path. See `ExecutionContext::extend_next_settle`.
const SETTLE_TIMEOUT_EXTENDED: Duration = Duration::from_millis(3000);

/// Poll interval paired with [`SETTLE_TIMEOUT_EXTENDED`] (500ms).
const SETTLE_INTERVAL_EXTENDED: Duration = Duration::from_millis(500);

/// Build a bounds-only fingerprint of the hierarchy for settle detection.
///
/// Ignores text and accessibility_label so that cursor blinks, live counters,
/// and other content changes don't prevent settling. Only structural and
/// spatial changes (animations, scroll momentum, layout shifts) count.
pub(crate) fn bounds_fingerprint(element: &Element) -> String {
    let mut buf = String::with_capacity(256);
    build_bounds_fingerprint(element, &mut buf);
    buf
}

fn build_bounds_fingerprint(element: &Element, buf: &mut String) {
    buf.push_str(&element.element_type);
    let b = &element.bounds;
    buf.push_str(&format!("@{},{},{}x{}", b.x, b.y, b.width, b.height));
    buf.push('[');
    for child in &element.children {
        build_bounds_fingerprint(child, buf);
        buf.push(',');
    }
    buf.push(']');
}

/// Wait for the UI hierarchy to stabilize before acting on it.
///
/// Compares consecutive hierarchy snapshots using a bounds-only fingerprint.
/// Returns the settled hierarchy when two consecutive snapshots match, or
/// the latest snapshot if the settle timeout is exceeded (never fails).
///
/// When the UI is already stable, this completes in a single extra hierarchy
/// fetch (~250ms). During animations it waits up to `SETTLE_TIMEOUT` (1.5s).
/// Maximum time to wait for WebView enrichment after settle.
/// Only applies when the tree contains a web_view with no children,
/// indicating WebKit Inspector hasn't connected yet.
///
/// Tighter than the inspector handshake budget (15s+30s) on purpose:
/// if enrichment hasn't arrived in 2.5s of post-action settle, the
/// previous action's effects are already observable in the
/// XCUITest accessibility tree — waiting longer just burns the
/// step's budget without adding information. The wedge case (handshake
/// genuinely never completes) shouldn't hold up every step's settle
/// for 10s.
const ENRICHMENT_TIMEOUT: Duration = Duration::from_millis(2500);

/// Check if the tree contains a web_view element with no children
/// (unenriched — WebKit Inspector hasn't connected yet).
pub(crate) fn has_empty_webview(element: &Element) -> bool {
    if element.element_type == "web_view" && element.children.is_empty() {
        return true;
    }
    element.children.iter().any(has_empty_webview)
}

/// Wait for the UI hierarchy to stabilize using the normal budget.
pub(crate) async fn wait_for_settle(
    driver: &dyn PlatformDriver,
) -> Result<(
    Element,
    golem_driver::common::HierarchyMeta,
    golem_events::TreeStats,
)> {
    wait_for_settle_with(driver, SETTLE_TIMEOUT, SETTLE_INTERVAL).await
}

/// Extended-budget settle for the one step following an un-verified
/// `/type`/`/backspace` mutation (see `ExecutionContext::extend_next_settle`).
pub(crate) async fn wait_for_settle_extended(
    driver: &dyn PlatformDriver,
) -> Result<(
    Element,
    golem_driver::common::HierarchyMeta,
    golem_events::TreeStats,
)> {
    wait_for_settle_with(driver, SETTLE_TIMEOUT_EXTENDED, SETTLE_INTERVAL_EXTENDED).await
}

async fn wait_for_settle_with(
    driver: &dyn PlatformDriver,
    settle_timeout: Duration,
    settle_interval: Duration,
) -> Result<(
    Element,
    golem_driver::common::HierarchyMeta,
    golem_events::TreeStats,
)> {
    let deadline = Instant::now() + settle_timeout;
    let mut stats = golem_events::TreeStats::default();

    let (root, meta) = get_hierarchy_bounded(driver).await?;
    stats.record(meta.node_count);
    crate::record_tree_fetch(meta.node_count);
    let mut prev_fp = bounds_fingerprint(&root);
    let mut prev_root = root;
    let mut prev_meta = meta;

    loop {
        if Instant::now() >= deadline {
            // Tree settled but check for unenriched WebView — keep polling
            // until enrichment arrives or enrichment timeout.
            if has_empty_webview(&prev_root) {
                let enrich_deadline = Instant::now() + ENRICHMENT_TIMEOUT;
                while Instant::now() < enrich_deadline {
                    tokio::time::sleep(settle_interval).await;
                    let (root, meta) = match get_hierarchy_bounded(driver).await {
                        Ok(r) => r,
                        Err(_) => break,
                    };
                    stats.record(meta.node_count);
                    crate::record_tree_fetch(meta.node_count);
                    if !has_empty_webview(&root) {
                        // Enrichment arrived — re-settle with enriched tree
                        prev_root = root;
                        prev_meta = meta;
                        // Quick settle check on enriched tree
                        tokio::time::sleep(settle_interval).await;
                        if let Ok((r2, m2)) = get_hierarchy_bounded(driver).await {
                            stats.record(m2.node_count);
                            crate::record_tree_fetch(m2.node_count);
                            return Ok((r2, m2, stats));
                        }
                        return Ok((prev_root, prev_meta, stats));
                    }
                }
            }
            return Ok((prev_root, prev_meta, stats));
        }

        tokio::time::sleep(settle_interval).await;

        let (root, meta) = match get_hierarchy_bounded(driver).await {
            Ok(r) => r,
            Err(_) => return Ok((prev_root, prev_meta, stats)),
        };
        stats.record(meta.node_count);
        crate::record_tree_fetch(meta.node_count);
        let fp = bounds_fingerprint(&root);

        if fp == prev_fp {
            // Settled — but if web_view is empty, keep polling for enrichment
            if has_empty_webview(&root) {
                prev_root = root;
                prev_meta = meta;
                prev_fp = fp;
                continue; // don't return yet, wait for enrichment
            }
            return Ok((root, meta, stats));
        }

        prev_fp = fp;
        prev_root = root;
        prev_meta = meta;
    }
}
