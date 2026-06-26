//! Accessibility auditing over the **visible** UI tree.
//!
//! Pure functions — no I/O, no driver calls, fully testable. The block-end
//! executor hook ([`crate::executor`]) fetches (or reuses) the settled tree
//! and calls [`audit_hierarchy`]; the findings become `A11yIssue`s on the
//! flow's `a11y_audits`.
//!
//! Core invariant (see `docs/architecture.md`): the audit judges only the
//! **visible** tree — a node is a subject of a check iff its
//! `effective_bounds()` intersect the viewport. We walk the *real* tree (so
//! sibling/parent structure survives, unlike `filter_viewport`'s flattened
//! output) and gate each check on that visibility predicate.
//!
//! Size thresholds are in **dp/pt**. `Element.bounds` are device px on
//! Android and points on iOS, so the executor passes a `density` (px-per-dp:
//! Android `DeviceInfo.screen_scale`, iOS `1.0`) and we normalise before
//! thresholding — a raw-px comparison would misfire per-platform.

use golem_element::{Bounds, Element, Viewport};
use golem_events::{A11yIssue, Rect, Severity};

/// Strictness of the accessibility audit. Resolved from the flow option /
/// `--a11y` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum A11yLevel {
    /// No auditing.
    Off,
    /// Tree checks only, no screenshots (fast — for CI).
    Critical,
    /// Default. Tree checks + opportunistic contrast (only if a screenshot
    /// was already captured this block — never forces one).
    Relaxed,
    /// Tree checks + forces a screenshot per block for contrast/text-size,
    /// AAA warn bands.
    Strict,
}

impl A11yLevel {
    /// Parse a level string; `None` for unknown values (caller decides whether
    /// to error or default).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "critical" => Some(Self::Critical),
            "relaxed" => Some(Self::Relaxed),
            "strict" => Some(Self::Strict),
            _ => None,
        }
    }

    /// Whether any auditing runs at this level.
    pub fn is_enabled(self) -> bool {
        self != Self::Off
    }

    /// Whether screenshot-based checks (contrast/text-size) are eligible.
    /// `critical` is tree-only; `relaxed`/`strict` may run them.
    pub fn screenshot_checks(self) -> bool {
        matches!(self, Self::Relaxed | Self::Strict)
    }

    /// Whether the level forces a screenshot capture every block (`strict`)
    /// vs only reusing one already captured (`relaxed`).
    pub fn forces_screenshot(self) -> bool {
        self == Self::Strict
    }
}

/// Resolved thresholds for one audit, derived from [`A11yLevel`] plus the
/// device density (px per dp).
#[derive(Debug, Clone, Copy)]
pub struct A11yConfig {
    pub level: A11yLevel,
    /// Touch targets with a min dimension strictly below this (in dp) are an
    /// Error.
    pub touch_target_error_dp: f64,
    /// `[error, warn)` in dp is a Warning; `None` ⇒ no warn band (strict).
    pub touch_target_warn_dp: Option<f64>,
    /// Px-per-dp: Android `screen_scale`, iOS `1.0`. `dp = px / density`.
    pub density: f64,
}

impl A11yConfig {
    /// Build the config for a level + device density (px per dp; pass `1.0`
    /// for iOS where bounds are already points).
    pub fn new(level: A11yLevel, density: f64) -> Self {
        let density = if density > 0.0 { density } else { 1.0 };
        let (touch_target_error_dp, touch_target_warn_dp) = match level {
            // Conservative: WCAG-AA 24dp error floor, warn band above. Strict
            // uses the AAA 44dp error floor (no separate warn band).
            A11yLevel::Critical => (24.0, Some(32.0)),
            A11yLevel::Relaxed => (24.0, Some(44.0)),
            A11yLevel::Strict => (44.0, None),
            A11yLevel::Off => (0.0, None),
        };
        Self {
            level,
            touch_target_error_dp,
            touch_target_warn_dp,
            density,
        }
    }

    fn px_to_dp(&self, px: i32) -> f64 {
        px as f64 / self.density
    }
}

/// Maximum sibling count before the O(n²) overlap check is skipped (a list
/// with hundreds of siblings is not a meaningful overlap candidate).
const MAX_OVERLAP_SIBLINGS: usize = 200;

/// Run all tree-based accessibility checks on the visible portion of
/// `root`, within `viewport`. Returns findings in deterministic
/// (depth-first) order.
pub fn audit_hierarchy(root: &Element, viewport: &Viewport, config: &A11yConfig) -> Vec<A11yIssue> {
    let mut issues = Vec::new();
    walk(root, viewport, config, &mut issues);
    issues
}

fn walk(node: &Element, viewport: &Viewport, config: &A11yConfig, out: &mut Vec<A11yIssue>) {
    // Per-node checks apply only to visible, *actionable* elements (the
    // innermost tap target — see `is_actionable`).
    if is_visible(node, viewport) && is_actionable(node) {
        check_touch_target(node, config, out);
        check_missing_label(node, out);
    }

    // Sibling-group checks over this node's visible actionable children.
    let actionable_siblings: Vec<&Element> = node
        .children
        .iter()
        .filter(|c| is_visible(c, viewport) && is_actionable(c))
        .collect();
    check_duplicate_labels(&actionable_siblings, out);
    check_overlapping(&actionable_siblings, out);

    for child in &node.children {
        walk(child, viewport, config, out);
    }
}

/// A node is a check subject iff its effective (visible-clipped) bounds
/// intersect the viewport — the visible-tree predicate.
fn is_visible(node: &Element, viewport: &Viewport) -> bool {
    viewport.contains(node.effective_bounds())
}

/// The element a tap actually lands on: a clickable element with no clickable
/// descendant. Native platforms mark structural containers (iOS `window` /
/// `other` / `web_view`, Android layout groups) as clickable/hittable; the
/// real control is the innermost clickable node, so we audit only that and
/// skip the wrapping containers (which would otherwise flood `missing_label`).
fn is_actionable(node: &Element) -> bool {
    node.clickable && !has_clickable_descendant(node)
}

fn has_clickable_descendant(node: &Element) -> bool {
    node.children
        .iter()
        .any(|c| c.clickable || has_clickable_descendant(c))
}

/// Whether the element exposes an accessible name anywhere in its subtree —
/// a control is "labelled" if it or a descendant (e.g. a child `StaticText`)
/// carries visible text or an accessibility label.
fn subtree_has_name(node: &Element) -> bool {
    label_of(node).is_some() || node.children.iter().any(subtree_has_name)
}

fn label_of(node: &Element) -> Option<String> {
    node.text
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| node.accessibility_label.clone().filter(|s| !s.is_empty()))
}

fn rect_of(b: &Bounds) -> Rect {
    Rect {
        x: b.x,
        y: b.y,
        width: b.width,
        height: b.height,
    }
}

fn issue(
    check_id: &str,
    severity: Severity,
    message: String,
    node: &Element,
    bounds: &Bounds,
) -> A11yIssue {
    A11yIssue {
        check_id: check_id.into(),
        severity,
        message,
        element_type: node.element_type.clone(),
        element_label: label_of(node),
        element_bounds: Some(rect_of(bounds)),
    }
}

/// `touch_target_too_small` — clickable with a min dimension below the dp
/// threshold. Zero-size targets are left to `check_zero_size` (avoids a
/// duplicate finding on the same node).
fn check_touch_target(node: &Element, config: &A11yConfig, out: &mut Vec<A11yIssue>) {
    if !node.clickable {
        return;
    }
    let b = node.effective_bounds();
    if b.width <= 0 || b.height <= 0 {
        // A non-positive dimension means the element isn't a visible target
        // (and won't have passed the viewport predicate anyway) — nothing to
        // threshold.
        return;
    }
    let min_dp = config.px_to_dp(b.width.min(b.height));
    let label = label_of(node).unwrap_or_else(|| node.element_type.clone());
    if min_dp < config.touch_target_error_dp {
        out.push(issue(
            "touch_target_too_small",
            Severity::Error,
            format!(
                "{} \"{}\" {:.0}dp below {:.0}dp minimum",
                node.element_type, label, min_dp, config.touch_target_error_dp
            ),
            node,
            b,
        ));
    } else if let Some(warn_dp) = config.touch_target_warn_dp {
        if min_dp < warn_dp {
            out.push(issue(
                "touch_target_too_small",
                Severity::Warning,
                format!(
                    "{} \"{}\" {:.0}dp below recommended {:.0}dp",
                    node.element_type, label, min_dp, warn_dp
                ),
                node,
                b,
            ));
        }
    }
}

/// `missing_label` — an actionable control with no accessible name anywhere
/// in its subtree. A screen reader has nothing to announce.
fn check_missing_label(node: &Element, out: &mut Vec<A11yIssue>) {
    if !subtree_has_name(node) {
        out.push(issue(
            "missing_label",
            Severity::Error,
            format!("{} has no text or accessibility label", node.element_type),
            node,
            node.effective_bounds(),
        ));
    }
}

/// `duplicate_labels` — sibling clickables sharing identical non-empty visible
/// text. A screen reader reads them identically; users can't tell them apart.
fn check_duplicate_labels(siblings: &[&Element], out: &mut Vec<A11yIssue>) {
    let mut seen: Vec<(&str, bool)> = Vec::new(); // (text, already-reported)
    for s in siblings {
        let Some(text) = s.text.as_deref().filter(|t| !t.is_empty()) else {
            continue;
        };
        if let Some(entry) = seen.iter_mut().find(|(t, _)| *t == text) {
            if !entry.1 {
                entry.1 = true;
                out.push(issue(
                    "duplicate_labels",
                    Severity::Warning,
                    format!("multiple sibling {}s labelled \"{}\"", s.element_type, text),
                    s,
                    s.effective_bounds(),
                ));
            }
        } else {
            seen.push((text, false));
        }
    }
}

/// `overlapping_interactive` — sibling clickables whose bounds overlap, so a
/// tap in the overlap region is ambiguous. Coincident/enclosed pairs (a
/// wrapper plus its content) are excluded. Skipped for very large sibling
/// groups (a long list is not a meaningful overlap candidate).
fn check_overlapping(siblings: &[&Element], out: &mut Vec<A11yIssue>) {
    if siblings.len() > MAX_OVERLAP_SIBLINGS {
        return;
    }
    for i in 0..siblings.len() {
        for j in (i + 1)..siblings.len() {
            let a = siblings[i].effective_bounds();
            let b = siblings[j].effective_bounds();
            if a.intersects(b) && !coincident(a, b) && !encloses(a, b) && !encloses(b, a) {
                out.push(issue(
                    "overlapping_interactive",
                    Severity::Warning,
                    format!(
                        "{} overlaps sibling {}",
                        siblings[i].element_type, siblings[j].element_type
                    ),
                    siblings[i],
                    a,
                ));
            }
        }
    }
}

/// Two bounds are coincident when they occupy the same rect — the signature
/// of a wrapper/content node split that is one visual target.
fn coincident(a: &Bounds, b: &Bounds) -> bool {
    a.x == b.x && a.y == b.y && a.width == b.width && a.height == b.height
}

/// `outer` fully encloses `inner` (inner sits entirely within outer). Used to
/// exclude ancestor-wrapper pairs from overlap reporting.
fn encloses(outer: &Bounds, inner: &Bounds) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Viewport {
        Viewport::new(1000, 2000)
    }

    fn relaxed() -> A11yConfig {
        // density 1.0 ⇒ px == dp (iOS-like), so test bounds read directly in dp.
        A11yConfig::new(A11yLevel::Relaxed, 1.0)
    }

    fn el(ty: &str, x: i32, y: i32, w: i32, h: i32) -> Element {
        Element {
            element_type: ty.into(),
            text: None,
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds: Bounds::new(x, y, w, h),
            visible_bounds: None,
            hit_points: vec![],
            drawing_order: None,
            children: vec![],
        }
    }

    fn with_text(mut e: Element, t: &str) -> Element {
        e.text = Some(t.into());
        e
    }

    fn root_with(children: Vec<Element>) -> Element {
        let mut r = el("Root", 0, 0, 1000, 2000);
        r.clickable = false;
        r.children = children;
        r
    }

    fn ids(issues: &[A11yIssue]) -> Vec<&str> {
        issues.iter().map(|i| i.check_id.as_str()).collect()
    }

    // ── A11yLevel parsing ───────────────────────────────────────────
    #[test]
    fn level_parse_known_and_unknown() {
        assert_eq!(A11yLevel::parse("off"), Some(A11yLevel::Off));
        assert_eq!(A11yLevel::parse("relaxed"), Some(A11yLevel::Relaxed));
        assert_eq!(A11yLevel::parse("strict"), Some(A11yLevel::Strict));
        assert_eq!(A11yLevel::parse("banana"), None);
    }

    #[test]
    fn level_screenshot_semantics() {
        assert!(!A11yLevel::Critical.screenshot_checks());
        assert!(A11yLevel::Relaxed.screenshot_checks());
        assert!(A11yLevel::Strict.screenshot_checks());
        assert!(!A11yLevel::Relaxed.forces_screenshot());
        assert!(A11yLevel::Strict.forces_screenshot());
    }

    // ── touch target ────────────────────────────────────────────────
    #[test]
    fn touch_target_below_error_threshold() {
        let issues = audit_hierarchy(&root_with(vec![with_text(el("Button", 0, 0, 20, 20), "x")]), &vp(), &relaxed());
        assert_eq!(ids(&issues), ["touch_target_too_small"]);
        assert_eq!(issues[0].severity, Severity::Error);
    }

    #[test]
    fn touch_target_in_warn_band() {
        // 30dp: above 24 error floor, below 44 warn ceiling → Warning.
        let issues = audit_hierarchy(&root_with(vec![with_text(el("Button", 0, 0, 30, 30), "x")]), &vp(), &relaxed());
        assert_eq!(ids(&issues), ["touch_target_too_small"]);
        assert_eq!(issues[0].severity, Severity::Warning);
    }

    #[test]
    fn touch_target_above_warn_ceiling_clean() {
        let issues = audit_hierarchy(&root_with(vec![with_text(el("Button", 0, 0, 50, 50), "x")]), &vp(), &relaxed());
        assert!(issues.is_empty());
    }

    #[test]
    fn touch_target_width_only_small() {
        // min(20,60)=20 < 24 → Error.
        let issues = audit_hierarchy(&root_with(vec![with_text(el("Button", 0, 0, 20, 60), "x")]), &vp(), &relaxed());
        assert_eq!(issues[0].severity, Severity::Error);
    }

    #[test]
    fn touch_target_non_clickable_ignored() {
        let mut e = el("Image", 0, 0, 10, 10);
        e.clickable = false;
        let issues = audit_hierarchy(&root_with(vec![e]), &vp(), &relaxed());
        assert!(issues.is_empty());
    }

    #[test]
    fn touch_target_dp_normalisation_android_vs_ios() {
        // 60px button. On iOS (density 1.0) → 60dp, clean. On Android at
        // density 3.0 → 20dp → Error. Same px, opposite verdict — the whole
        // point of dp-normalisation.
        let ios = audit_hierarchy(
            &root_with(vec![with_text(el("Button", 0, 0, 60, 60), "x")]),
            &vp(),
            &A11yConfig::new(A11yLevel::Relaxed, 1.0),
        );
        assert!(ios.is_empty(), "60dp on iOS is fine");
        let android = audit_hierarchy(
            &root_with(vec![with_text(el("Button", 0, 0, 60, 60), "x")]),
            &vp(),
            &A11yConfig::new(A11yLevel::Relaxed, 3.0),
        );
        assert_eq!(android[0].check_id, "touch_target_too_small");
        assert_eq!(android[0].severity, Severity::Error, "20dp on Android is an error");
    }

    #[test]
    fn touch_target_strict_uses_44_error() {
        let issues = audit_hierarchy(
            &root_with(vec![with_text(el("Button", 0, 0, 40, 40), "x")]),
            &vp(),
            &A11yConfig::new(A11yLevel::Strict, 1.0),
        );
        assert_eq!(issues[0].severity, Severity::Error, "40dp < 44 strict floor");
    }

    // ── missing label ───────────────────────────────────────────────
    #[test]
    fn missing_label_no_text_no_label() {
        let issues = audit_hierarchy(&root_with(vec![el("Image", 0, 0, 50, 50)]), &vp(), &relaxed());
        assert_eq!(ids(&issues), ["missing_label"]);
        assert_eq!(issues[0].severity, Severity::Error);
    }

    #[test]
    fn missing_label_has_text_ok() {
        let e = with_text(el("Button", 0, 0, 50, 50), "Submit");
        let issues = audit_hierarchy(&root_with(vec![e]), &vp(), &relaxed());
        assert!(issues.is_empty());
    }

    #[test]
    fn missing_label_has_accessibility_label_ok() {
        let mut e = el("Button", 0, 0, 50, 50);
        e.accessibility_label = Some("close".into());
        let issues = audit_hierarchy(&root_with(vec![e]), &vp(), &relaxed());
        assert!(issues.is_empty());
    }

    #[test]
    fn missing_label_empty_strings_flagged() {
        let mut e = el("Button", 0, 0, 50, 50);
        e.text = Some(String::new());
        e.accessibility_label = Some(String::new());
        let issues = audit_hierarchy(&root_with(vec![e]), &vp(), &relaxed());
        assert_eq!(ids(&issues), ["missing_label"]);
    }

    // ── duplicate labels ─────────────────────────────────────────────
    #[test]
    fn duplicate_labels_same_parent_one_warning() {
        let issues = audit_hierarchy(
            &root_with(vec![
                with_text(el("Button", 0, 0, 50, 50), "Add"),
                with_text(el("Button", 0, 100, 50, 50), "Add"),
                with_text(el("Button", 0, 200, 50, 50), "Add"),
            ]),
            &vp(),
            &relaxed(),
        );
        let dups: Vec<_> = issues.iter().filter(|i| i.check_id == "duplicate_labels").collect();
        assert_eq!(dups.len(), 1, "three-way dup → one warning");
        assert_eq!(dups[0].severity, Severity::Warning);
    }

    #[test]
    fn duplicate_labels_distinct_text_ok() {
        let issues = audit_hierarchy(
            &root_with(vec![
                with_text(el("Button", 0, 0, 50, 50), "Add"),
                with_text(el("Button", 0, 100, 50, 50), "Remove"),
            ]),
            &vp(),
            &relaxed(),
        );
        assert!(issues.iter().all(|i| i.check_id != "duplicate_labels"));
    }

    #[test]
    fn duplicate_labels_different_parents_ok() {
        // Same text under different parents → not siblings → no dup.
        let p1 = root_with(vec![with_text(el("Button", 0, 0, 50, 50), "Go")]);
        let mut p2 = el("Group", 0, 300, 200, 200);
        p2.clickable = false;
        p2.children = vec![with_text(el("Button", 0, 300, 50, 50), "Go")];
        let mut root = root_with(vec![]);
        root.children = vec![p1, p2];
        let issues = audit_hierarchy(&root, &vp(), &relaxed());
        assert!(issues.iter().all(|i| i.check_id != "duplicate_labels"));
    }

    // ── innermost-actionable gating (native container noise) ────────
    #[test]
    fn structural_clickable_container_not_flagged() {
        // iOS-style: clickable `window` ⊃ clickable `other` ⊃ clickable
        // labelled `button`. Only the innermost button is a real target;
        // the containers must NOT trip missing_label.
        let mut window = el("window", 0, 0, 400, 800);
        let mut other = el("other", 0, 0, 400, 800);
        other.children = vec![with_text(el("button", 0, 0, 100, 60), "Submit")];
        window.children = vec![other];
        let issues = audit_hierarchy(&root_with(vec![window]), &vp(), &relaxed());
        assert!(
            issues.is_empty(),
            "labelled innermost button is clean; containers must not flag: {issues:?}"
        );
    }

    #[test]
    fn innermost_unlabelled_control_is_flagged() {
        // Same nesting but the button has no name → exactly one missing_label
        // (on the button, not the containers).
        let mut window = el("window", 0, 0, 400, 800);
        let mut other = el("other", 0, 0, 400, 800);
        other.children = vec![el("button", 0, 0, 100, 60)]; // no text/label
        window.children = vec![other];
        let issues = audit_hierarchy(&root_with(vec![window]), &vp(), &relaxed());
        let missing: Vec<_> = issues.iter().filter(|i| i.check_id == "missing_label").collect();
        assert_eq!(missing.len(), 1, "only the innermost control flags: {issues:?}");
        assert_eq!(missing[0].element_type, "button");
    }

    #[test]
    fn label_from_descendant_text_satisfies() {
        // A clickable button whose name comes from a child StaticText (common
        // on iOS) is NOT missing a label.
        let mut button = el("button", 0, 0, 100, 60);
        let mut text = el("StaticText", 0, 0, 100, 60);
        text.clickable = false;
        text.text = Some("Buy".into());
        button.children = vec![text];
        let issues = audit_hierarchy(&root_with(vec![button]), &vp(), &relaxed());
        assert!(
            issues.iter().all(|i| i.check_id != "missing_label"),
            "descendant text supplies the accessible name: {issues:?}"
        );
    }

    // ── overlapping ──────────────────────────────────────────────────
    #[test]
    fn overlapping_siblings_flagged() {
        let issues = audit_hierarchy(
            &root_with(vec![
                with_text(el("Button", 0, 0, 100, 100), "A"),
                with_text(el("Button", 50, 50, 100, 100), "B"),
            ]),
            &vp(),
            &relaxed(),
        );
        assert!(issues.iter().any(|i| i.check_id == "overlapping_interactive"));
    }

    #[test]
    fn overlapping_disjoint_ok() {
        let issues = audit_hierarchy(
            &root_with(vec![
                with_text(el("Button", 0, 0, 50, 50), "A"),
                with_text(el("Button", 200, 200, 50, 50), "B"),
            ]),
            &vp(),
            &relaxed(),
        );
        assert!(issues.iter().all(|i| i.check_id != "overlapping_interactive"));
    }

    #[test]
    fn overlapping_coincident_excluded() {
        let issues = audit_hierarchy(
            &root_with(vec![
                with_text(el("Button", 0, 0, 100, 100), "A"),
                with_text(el("Overlay", 0, 0, 100, 100), "B"),
            ]),
            &vp(),
            &relaxed(),
        );
        assert!(issues.iter().all(|i| i.check_id != "overlapping_interactive"));
    }

    // ── visibility gating ────────────────────────────────────────────
    #[test]
    fn offscreen_node_not_judged() {
        // A tiny button entirely off-screen (below the viewport) must NOT be
        // flagged — the audit judges only the visible tree.
        let issues = audit_hierarchy(&root_with(vec![el("Button", 0, 5000, 10, 10)]), &vp(), &relaxed());
        assert!(issues.is_empty(), "off-screen node is not a subject");
    }

    // ── combined / edges ─────────────────────────────────────────────
    #[test]
    fn clean_tree_no_issues() {
        let issues = audit_hierarchy(
            &root_with(vec![
                with_text(el("Button", 0, 0, 80, 80), "Submit"),
                with_text(el("Button", 0, 100, 80, 80), "Cancel"),
            ]),
            &vp(),
            &relaxed(),
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn empty_tree_no_crash() {
        let issues = audit_hierarchy(&root_with(vec![]), &vp(), &relaxed());
        assert!(issues.is_empty());
    }

    #[test]
    fn multiple_issues_one_node() {
        // Tiny + no label → both touch_target Error and missing_label Error.
        let issues = audit_hierarchy(&root_with(vec![el("Image", 0, 0, 10, 10)]), &vp(), &relaxed());
        let mut got = ids(&issues);
        got.sort_unstable();
        assert_eq!(got, ["missing_label", "touch_target_too_small"]);
    }
}
