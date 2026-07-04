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
    /// Default. All tree checks (incl. the certain box-height text-size
    /// check); no screenshot, so no contrast.
    Relaxed,
    /// Tree checks + forces a per-block screenshot for the contrast check,
    /// with AAA warn bands.
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

    /// Whether the level forces a per-block screenshot for the contrast check.
    /// Only `strict` does; the others are screenshot-free.
    pub fn forces_screenshot(self) -> bool {
        self == Self::Strict
    }

    /// Canonical lowercase name (round-trips with [`parse`](Self::parse)).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Critical => "critical",
            Self::Relaxed => "relaxed",
            Self::Strict => "strict",
        }
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
    /// Text below this dp height is `text_too_small` (Warning).
    pub min_text_size_dp: f64,
    /// When set, contrast between AA and AAA is a Warning (strict only).
    pub contrast_warn_aaa: bool,
    /// A clickable element whose area is at least this fraction of the viewport
    /// is treated as a structural container (root tap handler, backdrop, scroll
    /// surface), not a perceived control — exempt from the interactive checks.
    /// A real tappable control is virtually never this large.
    pub max_control_area_fraction: f64,
    /// `occluded_element` fires when a control's reachable fraction of tap
    /// points is below this. `strict` is stricter (flags a quarter-covered
    /// control); `relaxed`/`critical` only flag a majority-covered one. Fully
    /// occluded (0) is already pruned from the visible tree.
    pub occluded_max_hittable: f32,
    /// Default confidence floor for *this level* — findings below it are
    /// dropped. `relaxed`/`critical` only ever produce deterministic (1.0)
    /// findings, so their default is `1.0` (no heuristic noise out of the box);
    /// `strict` runs the heuristic pixel checks, so it defaults lower to keep
    /// the credible ones while hiding borderline guesses. An explicit
    /// `a11y_min_confidence` always overrides this.
    pub min_confidence: f32,
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
            // Critical is the lean "worst-offenders" gate → a lower floor so
            // it flags only egregiously small text; relaxed 10, strict 12.
            min_text_size_dp: match level {
                A11yLevel::Strict => 12.0,
                A11yLevel::Critical => 8.0,
                _ => 10.0,
            },
            contrast_warn_aaa: level == A11yLevel::Strict,
            max_control_area_fraction: 0.5,
            // strict flags a control >25% covered; relaxed/critical only when
            // the majority is covered (fewer, higher-signal warnings).
            occluded_max_hittable: match level {
                A11yLevel::Strict => 0.75,
                _ => 0.5,
            },
            // Heuristic findings only exist at strict (the screenshot pass);
            // the others are all deterministic (1.0), so default 1.0 there.
            min_confidence: if level == A11yLevel::Strict { 0.5 } else { 1.0 },
        }
    }

    fn px_to_dp(&self, px: i32) -> f64 {
        px as f64 / self.density
    }
}

/// Maximum sibling count before the O(n²) overlap check is skipped (a list
/// with hundreds of siblings is not a meaningful overlap candidate).
const MAX_OVERLAP_SIBLINGS: usize = 200;

/// Pixel text-size floor (dp): a measured glyph height below this isn't
/// readable text — it's a thin rule / divider / border or a stray sub-pixel
/// band. The pixel pass skips it (FN-safe) rather than emit confident noise.
const MIN_PLAUSIBLE_TEXT_DP: f64 = 5.0;

/// WCAG "large-scale text" — judged at the laxer contrast ratios (3:1 AA,
/// 4.5:1 AAA) — is ≥18pt, which at 1pt≈1.333px is ≈24dp at normal weight.
/// (The 14pt-bold alternative needs weight detection we don't have.)
const WCAG_LARGE_TEXT_DP: f64 = 24.0;

/// Run all tree-based accessibility checks on the visible portion of
/// `root`, within `viewport`. Returns findings in deterministic
/// (depth-first) order.
pub fn audit_hierarchy(root: &Element, viewport: &Viewport, config: &A11yConfig) -> Vec<A11yIssue> {
    let mut issues = Vec::new();
    walk(root, viewport, config, false, &mut issues);
    issues
}

/// A control that is present but turned off: a clickable element with
/// `enabled == false`. WCAG exempts inactive components from contrast/size
/// requirements, so we skip such a control and its subtree. (Only clickable
/// elements — `enabled` defaults false on plenty of non-interactive nodes.)
fn is_disabled_control(node: &Element) -> bool {
    node.clickable && !node.enabled
}

fn walk(
    node: &Element,
    viewport: &Viewport,
    config: &A11yConfig,
    disabled_ancestor: bool,
    out: &mut Vec<A11yIssue>,
) {
    let disabled = disabled_ancestor || is_disabled_control(node);

    // Per-node checks, skipped inside a disabled control. Touch-target/
    // missing-label apply to the actionable control; text-size to any visible
    // text element.
    if !disabled && is_visible(node, viewport) {
        if is_actionable(node, viewport, config) {
            check_touch_target(node, config, out);
            check_missing_label(node, out);
            check_occluded(node, config, out);
        }
        check_text_size_box(node, config, out);
    }

    // Sibling-group checks over this node's visible, enabled, actionable
    // children (skipped entirely inside a disabled subtree).
    if !disabled {
        let actionable_siblings: Vec<&Element> = node
            .children
            .iter()
            .filter(|c| {
                is_visible(c, viewport)
                    && is_actionable(c, viewport, config)
                    && !is_disabled_control(c)
            })
            .collect();
        check_duplicate_labels(&actionable_siblings, out);
        check_overlapping(&actionable_siblings, out);
    }

    for child in &node.children {
        walk(child, viewport, config, disabled, out);
    }
}

/// A node is a check subject iff its effective (visible-clipped) bounds
/// intersect the viewport AND it isn't fully occluded — the visible-tree
/// predicate. IntersectionObserver (webview) / clip bounds catch ancestor
/// clipping, but an element painted *under* an opaque overlay (e.g. text
/// behind a sticky header) still intersects the viewport; the occlusion
/// hit-test catches that.
fn is_visible(node: &Element, viewport: &Viewport) -> bool {
    viewport.contains(node.effective_bounds()) && !is_fully_occluded(node)
}

/// Whether a hit-test ran and found *no* clear sample point — the element is
/// entirely covered by something painted on top. Authoritative in webviews
/// (`elementFromPoint`); for native it's best-effort, but a zero clear-fraction
/// is strong enough evidence to not judge a control the user can't see. Nodes
/// with no hit-test (`hit_points` empty) are treated as visible.
fn is_fully_occluded(node: &Element) -> bool {
    node.hittable_fraction() == Some(0.0)
}

/// The element a tap actually lands on: a clickable element with no clickable
/// descendant. Native platforms mark structural containers (iOS `window` /
/// `other` / `web_view`, Android layout groups) as clickable/hittable; the
/// real control is the innermost clickable node, so we audit only that and
/// skip the wrapping containers (which would otherwise flood `missing_label`).
/// An oversized clickable (≥ `max_control_area_fraction` of the viewport) is
/// also skipped — a control covering most of the screen is a backdrop/root tap
/// surface, not a perceived control, and judging it as one (missing_label,
/// touch target, overlap) is a false positive.
fn is_actionable(node: &Element, viewport: &Viewport, config: &A11yConfig) -> bool {
    node.clickable
        && !has_clickable_descendant(node)
        && !is_oversized_container(node, viewport, config)
}

/// Whether the element's visible area is at least `max_control_area_fraction`
/// of the viewport area — too large to be a discrete interactive control.
fn is_oversized_container(node: &Element, viewport: &Viewport, config: &A11yConfig) -> bool {
    let vp_area = viewport.width as f64 * viewport.height as f64;
    if vp_area <= 0.0 {
        return false;
    }
    let b = node.effective_bounds();
    let area = (b.width.max(0) as f64) * (b.height.max(0) as f64);
    area >= config.max_control_area_fraction * vp_area
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

/// Short human identifier for an element: `button "Save"` when it has a label,
/// else just its type. Used in finding messages that reference more than one
/// element (e.g. overlap pairs) so the reader can tell them apart.
fn describe(node: &Element) -> String {
    match label_of(node) {
        Some(l) => format!("{} \"{}\"", node.element_type, l),
        None => node.element_type.clone(),
    }
}

fn label_of(node: &Element) -> Option<String> {
    node.text
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| node.accessibility_label.clone().filter(|s| !s.is_empty()))
}

/// Whether the element is partially off-screen / clipped — its visible bounds
/// are smaller than its full bounds (beyond a 1px rounding tolerance). Size and
/// contrast checks are unreliable on a clipped element (a sliver reads as tiny,
/// the wrong colours get sampled), so those checks skip it; only the
/// size-independent checks (`missing_label`, `duplicate_labels`) stay reliable.
fn is_clipped(node: &Element) -> bool {
    match &node.visible_bounds {
        Some(vb) => {
            vb.width.saturating_add(1) < node.bounds.width
                || vb.height.saturating_add(1) < node.bounds.height
        }
        None => false,
    }
}

fn rect_of(b: &Bounds) -> Rect {
    Rect {
        x: b.x,
        y: b.y,
        width: b.width,
        height: b.height,
    }
}

/// Build a deterministic (confidence 1.0) finding. `detail` is the compact
/// on-image token (e.g. `32dp`) or `None` for checks with no single scalar.
/// Heuristic checks (contrast) construct `A11yIssue` directly with a scored
/// confidence.
fn issue(
    check_id: &str,
    severity: Severity,
    message: String,
    node: &Element,
    bounds: &Bounds,
    detail: Option<String>,
) -> A11yIssue {
    A11yIssue {
        check_id: check_id.into(),
        severity,
        message,
        element_type: node.element_type.clone(),
        element_label: label_of(node),
        element_bounds: Some(rect_of(bounds)),
        related_bounds: Vec::new(),
        measure_bounds: None,
        occlusion: Vec::new(),
        confidence: 1.0,
        detail,
    }
}

/// `touch_target_too_small` — clickable with a min dimension below the dp
/// threshold. Zero-size / non-positive-dimension targets are skipped — they
/// aren't visible tap targets (the visible-tree filter already drops them), so
/// there's nothing to threshold.
fn check_touch_target(node: &Element, config: &A11yConfig, out: &mut Vec<A11yIssue>) {
    if !node.clickable {
        return;
    }
    // A clipped element's visible bounds are a sliver — its real tap target is
    // bigger, so the measured size would be a misleading false "too small".
    if is_clipped(node) {
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
            Some(format!("{min_dp:.0}dp")),
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
                Some(format!("{min_dp:.0}dp")),
            ));
        }
    }
}

/// `text_too_small` — a text element whose *box* is already shorter than the
/// minimum dp height. Since glyph height ≤ box height, this is certain (no
/// screenshot, no false positives); padding only makes it conservative, so it
/// misses small text inside a tall padded box (an acceptable false negative —
/// the pixel-based glyph-height refinement for that case is a follow-up).
///
/// Measures the element's **full** box height, not its clipped visible height:
/// a normal-size row half-scrolled past a scroll container's edge has a tiny
/// *visible* sliver but normal text — measuring the clip would be a false
/// positive. Visibility is still gated upstream (the element must intersect the
/// viewport to be a subject at all).
fn check_text_size_box(node: &Element, config: &A11yConfig, out: &mut Vec<A11yIssue>) {
    if node.text.as_deref().is_none_or(|t| t.is_empty()) {
        return;
    }
    if node.bounds.height <= 0 {
        return;
    }
    // Round the measured height UP to a whole dp — give borderline text the
    // benefit of the doubt (anything over 11.0 reads as 12, not flagged at a 12
    // floor). Conservative, matching how imprecise on-device measurement is;
    // ceil only ever removes a flag, never adds one, so this stays FP-free.
    let height_dp = (node.bounds.height as f64 / config.density).ceil();
    if height_dp < config.min_text_size_dp {
        out.push(issue(
            "text_too_small",
            Severity::Warning,
            // "text" rather than the raw element type — webview text nodes are
            // DOM tags (p/span/div) that leak implementation detail.
            format!(
                "text is {:.0}dp tall — below {:.0}dp minimum",
                height_dp, config.min_text_size_dp
            ),
            node,
            node.effective_bounds(),
            Some(format!("{height_dp:.0}dp")),
        ));
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
            None,
        ));
    }
}

/// `occluded_element` — an actionable control whose tap target is partly
/// painted over by something on top (from the occlusion hit-test's `hit_points`
/// ground truth). Fully-covered controls are already dropped from the visible
/// tree, so this catches the *partial* case a bounds-only check can't see: the
/// control looks tappable but a chunk of it is unreachable. Deterministic
/// (confidence 1.0). Skips elements with no hit-test data, and clipped elements
/// (a scroll-clipped control's off-clip points read as unreachable — that's
/// clipping, not occlusion, and is handled elsewhere).
fn check_occluded(node: &Element, config: &A11yConfig, out: &mut Vec<A11yIssue>) {
    if is_clipped(node) {
        return;
    }
    let Some(frac) = node.hittable_fraction() else {
        return;
    };
    if frac > 0.0 && frac < config.occluded_max_hittable {
        let b = node.effective_bounds();
        let pct = (frac * 100.0).round() as i32;
        let mut iss = issue(
            "occluded_element",
            Severity::Warning,
            format!("{} is only {pct}% reachable — tap target partly covered", describe(node)),
            node,
            b,
            Some(format!("{pct}%")),
        );
        // Carry every sampled cell with its reachability (each hit point → its
        // 3×3 zone). The annotator draws only these — covered solid, reachable a
        // faint outline — so untested zones make no claim; JSON gets the map too.
        iss.occlusion = node
            .hit_points
            .iter()
            .map(|p| golem_events::OcclusionCell {
                bounds: occlusion_cell_rect(b, p.x, p.y),
                reachable: p.hit,
            })
            .collect();
        out.push(iss);
    }
}

/// The 3×3 sub-cell of `b` containing point `(px, py)`. Hit points sample at the
/// ¼/½/¾ lines, so each maps to a distinct col/row (0/1/2) — the returned rect
/// marks which zone of the control that covered point falls in.
fn occlusion_cell_rect(b: &Bounds, px: i32, py: i32) -> Rect {
    let w = b.width.max(1);
    let h = b.height.max(1);
    let col = (((px - b.x) * 3) / w).clamp(0, 2);
    let row = (((py - b.y) * 3) / h).clamp(0, 2);
    Rect {
        x: b.x + col * (w / 3),
        y: b.y + row * (h / 3),
        width: (w / 3).max(1),
        height: (h / 3).max(1),
    }
}

/// `duplicate_labels` — sibling clickables sharing identical non-empty visible
/// text. A screen reader reads them identically; users can't tell them apart.
/// One finding per group: the first occurrence is the primary, every other
/// member rides in `related_bounds` so the annotator can rect+connect them all.
fn check_duplicate_labels(siblings: &[&Element], out: &mut Vec<A11yIssue>) {
    // Preserve first-seen order of distinct labels, accumulating each label's
    // member elements.
    let mut groups: Vec<(&str, Vec<&Element>)> = Vec::new();
    for s in siblings {
        let Some(text) = s.text.as_deref().filter(|t| !t.is_empty()) else {
            continue;
        };
        if let Some(entry) = groups.iter_mut().find(|(t, _)| *t == text) {
            entry.1.push(s);
        } else {
            groups.push((text, vec![s]));
        }
    }
    for (text, members) in groups {
        if members.len() < 2 {
            continue;
        }
        let primary = members[0];
        let mut iss = issue(
            "duplicate_labels",
            Severity::Warning,
            format!(
                "{} sibling {}s labelled \"{}\"",
                members.len(),
                primary.element_type,
                text
            ),
            primary,
            primary.effective_bounds(),
            None,
        );
        iss.related_bounds = members[1..]
            .iter()
            .map(|m| rect_of(m.effective_bounds()))
            .collect();
        out.push(iss);
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
                let mut iss = issue(
                    "overlapping_interactive",
                    Severity::Warning,
                    // Name both elements (label where present) so it's clear
                    // *which* controls overlap, not just "button overlaps button".
                    format!("{} overlaps {}", describe(siblings[i]), describe(siblings[j])),
                    siblings[i],
                    a,
                    None,
                );
                iss.related_bounds = vec![rect_of(b)];
                out.push(iss);
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

// ── Screenshot-based check: contrast ───────────────────────────────
//
// Heuristic pixel analysis. Best-effort: skips elements whose background is
// too complex to read reliably (gradients/photos) rather than emitting a
// false positive, and carries a confidence score so noisy findings can be
// filtered via `a11y_min_confidence`.

type Rgb = [u8; 3];

/// WCAG relative luminance of an sRGB colour (0.0–1.0).
fn relative_luminance(c: Rgb) -> f64 {
    fn lin(v: u8) -> f64 {
        let s = v as f64 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG contrast ratio between two colours (1.0–21.0), order-independent.
fn wcag_contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn color_distance(a: Rgb, b: Rgb) -> i32 {
    let dr = a[0] as i32 - b[0] as i32;
    let dg = a[1] as i32 - b[1] as i32;
    let db = a[2] as i32 - b[2] as i32;
    dr * dr + dg * dg + db * db
}

const CLUSTER_DIST_SQ: i32 = 48 * 48; // tolerance to absorb anti-aliasing

/// Most common colour in a pixel set (quantised to 16-levels, bucket mean).
fn dominant_color(pixels: &[Rgb]) -> Option<Rgb> {
    if pixels.is_empty() {
        return None;
    }
    let mut buckets: std::collections::HashMap<Rgb, (usize, [u64; 3])> =
        std::collections::HashMap::new();
    for &p in pixels {
        let key = [p[0] & 0xF0, p[1] & 0xF0, p[2] & 0xF0];
        let e = buckets.entry(key).or_insert((0, [0; 3]));
        e.0 += 1;
        e.1[0] += p[0] as u64;
        e.1[1] += p[1] as u64;
        e.1[2] += p[2] as u64;
    }
    buckets.values().max_by_key(|(n, _)| *n).map(|(n, sum)| {
        [
            (sum[0] / *n as u64) as u8,
            (sum[1] / *n as u64) as u8,
            (sum[2] / *n as u64) as u8,
        ]
    })
}

/// How many quantised colour clusters each hold ≥5% of the pixels. Used to
/// detect gradients/photos (>3) where contrast can't be read reliably.
fn significant_cluster_count(pixels: &[Rgb]) -> usize {
    if pixels.is_empty() {
        return 0;
    }
    let mut buckets: std::collections::HashMap<Rgb, usize> = std::collections::HashMap::new();
    for &p in pixels {
        *buckets
            .entry([p[0] & 0xF0, p[1] & 0xF0, p[2] & 0xF0])
            .or_insert(0) += 1;
    }
    let total = pixels.len();
    buckets.values().filter(|n| **n * 20 >= total).count()
}

/// The tallest contiguous band of rows carrying glyph ink (pixels far from
/// `bg`), as an inclusive `(start, end)` row range. `None` when no ink rows —
/// strips surrounding padding so the height reflects the glyphs, not the box.
fn text_band(pixels: &[Rgb], width: usize, bg: Rgb) -> Option<(usize, usize)> {
    if width == 0 || pixels.is_empty() {
        return None;
    }
    let height = pixels.len() / width;
    let mut best: Option<(usize, usize)> = None;
    let mut run_start: Option<usize> = None;
    for row in 0..height {
        let ink = (0..width)
            .filter(|&x| color_distance(pixels[row * width + x], bg) > CLUSTER_DIST_SQ)
            .count();
        // A text row has moderate ink — glyph strokes with inter-letter gaps.
        // A near-full-width ink row is a border/divider/underline, not text,
        // and must be excluded or its colour pollutes the foreground estimate.
        let is_text_row = ink * 20 >= width && ink * 10 < width * 9;
        if is_text_row {
            run_start.get_or_insert(row);
        } else if let Some(s) = run_start.take() {
            let len = row - s;
            if best.is_none_or(|(bs, be)| len > be - bs + 1) {
                best = Some((s, row - 1));
            }
        }
    }
    if let Some(s) = run_start {
        let len = height - s;
        if best.is_none_or(|(bs, be)| len > be - bs + 1) {
            best = Some((s, height - 1));
        }
    }
    best
}

/// Background + foreground (glyph ink) for a text region. `bg` is the box's
/// dominant colour; `fg` is the ink colour *within the detected text band*
/// that sits FARTHEST from `bg` (the glyph core) — not the most populous ink,
/// which would be the anti-aliasing halo sitting between ink and bg and would
/// understate the true contrast. Isolating the band excludes a mostly-
/// background box; the farthest-cluster pick excludes halos. Returns
/// `(bg, fg, band_height_px)`; `None` when there's no readable text or the
/// region is too complex (gradient/photo).
fn extract_text_colors(pixels: &[Rgb], width: usize) -> Option<(Rgb, Rgb, u32)> {
    let bg = dominant_color(pixels)?;
    if significant_cluster_count(pixels) > 3 {
        return None; // gradient / photo — undetermined
    }
    let (r0, r1) = text_band(pixels, width, bg)?;
    let ink: Vec<Rgb> = (r0..=r1)
        .flat_map(|row| (0..width).map(move |x| (row, x)))
        .map(|(row, x)| pixels[row * width + x])
        .filter(|p| color_distance(*p, bg) > CLUSTER_DIST_SQ)
        .collect();
    let fg = farthest_ink_color(&ink, bg)?;
    Some((bg, fg, (r1 - r0 + 1) as u32))
}

/// The ink cluster farthest from `bg` among clusters holding ≥10% of the ink
/// (so a few stray/noise pixels can't define the foreground). This recovers
/// the glyph core (black/blue) rather than the anti-aliasing gradient.
fn farthest_ink_color(ink: &[Rgb], bg: Rgb) -> Option<Rgb> {
    if ink.is_empty() {
        return None;
    }
    let mut buckets: std::collections::HashMap<Rgb, (usize, [u64; 3])> =
        std::collections::HashMap::new();
    for &p in ink {
        let e = buckets
            .entry([p[0] & 0xF0, p[1] & 0xF0, p[2] & 0xF0])
            .or_insert((0, [0; 3]));
        e.0 += 1;
        e.1[0] += p[0] as u64;
        e.1[1] += p[1] as u64;
        e.1[2] += p[2] as u64;
    }
    let total = ink.len();
    buckets
        .values()
        .filter(|(n, _)| *n * 10 >= total)
        .map(|(n, sum)| {
            [
                (sum[0] / *n as u64) as u8,
                (sum[1] / *n as u64) as u8,
                (sum[2] / *n as u64) as u8,
            ]
        })
        .max_by_key(|c| color_distance(*c, bg))
}

/// Crop a region of an RGB image into a flat pixel vector, clamped to bounds.
fn crop_pixels(img: &image::RgbImage, x: i32, y: i32, w: i32, h: i32) -> (Vec<Rgb>, usize) {
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let x0 = x.clamp(0, iw);
    let y0 = y.clamp(0, ih);
    let x1 = (x + w).clamp(0, iw);
    let y1 = (y + h).clamp(0, ih);
    let cw = (x1 - x0).max(0) as usize;
    let mut out = Vec::new();
    for yy in y0..y1 {
        for xx in x0..x1 {
            let p = img.get_pixel(xx as u32, yy as u32).0;
            out.push([p[0], p[1], p[2]]);
        }
    }
    (out, cw)
}

/// Points→pixels factor derived from the screenshot vs the tree's coordinate
/// space (viewport width). iOS Retina → ~3.0, Android → 1.0; no dependence on
/// a device-reported backing scale.
fn screenshot_scale(img_width: u32, viewport_width: i32) -> f64 {
    if viewport_width > 0 {
        (img_width as f64 / viewport_width as f64).max(0.01)
    } else {
        1.0
    }
}

/// Whether any descendant carries non-empty text.
fn has_text_descendant(node: &Element) -> bool {
    node.children.iter().any(|c| {
        c.text.as_deref().is_some_and(|t| !t.is_empty()) || has_text_descendant(c)
    })
}

/// Collect visible *innermost* text elements (non-empty `text`, no
/// text-bearing descendant) for contrast, skipping anything inside a disabled
/// control (WCAG exempts inactive components). Restricting to the innermost
/// text node skips wrapping containers (a `div`/`ul` whose box spans a whole
/// row and mixes content), whose crop would give an unreliable ratio — the
/// real glyph subject is the leaf label/run.
fn visible_text_elements<'a>(
    node: &'a Element,
    viewport: &Viewport,
    disabled_ancestor: bool,
    out: &mut Vec<&'a Element>,
) {
    let disabled = disabled_ancestor || is_disabled_control(node);
    if !disabled
        && is_visible(node, viewport)
        && node.text.as_deref().is_some_and(|t| !t.is_empty())
        && !has_text_descendant(node)
    {
        out.push(node);
    }
    for c in &node.children {
        visible_text_elements(c, viewport, disabled, out);
    }
}

/// Estimate the rendered font size (em) of a text region, in screenshot
/// pixels, from the tallest text line's ink-row profile. Returns `None` when
/// there's no readable text line.
///
/// The ink band alone is an unreliable proxy — a word with no ascenders or
/// descenders ("save") inks only its x-height (~0.5em), so the band
/// *under*-states the em and would over-flag normal text. We instead take the
/// **maximum** of three interpretations of the same line, which biases the
/// estimate *upward* (toward not flagging — the deliberate false-negative-over-
/// false-positive stance):
/// - the full ink extent (top of caps/ascenders → bottom of descenders) ≈ em
///   when the line has both;
/// - the dense (x-height) band ÷ 0.5 — recovers em for x-height-only words;
/// - the ascent (ink top → baseline) ÷ 0.75 — recovers em from cap/ascender
///   height.
/// Only text that's small under *every* interpretation reads as small.
///
/// Returns `(em_px, line_top_row, line_bot_row, runs)` — the size estimate, the
/// measured line's row range (relative to the crop) so the annotation can put
/// its dimension on the actual line rather than the whole (padded/multi-line)
/// box, and `runs` = the number of horizontal ink runs in the line's peak row.
/// A single run is a solid line (divider/underline) or one glyph; multiple runs
/// are characteristic of real multi-stroke text — the caller uses it to temper
/// confidence.
fn estimate_text_em_px(pixels: &[Rgb], width: usize, bg: Rgb) -> Option<(f64, usize, usize, usize)> {
    let (r0, r1) = text_band(pixels, width, bg)?;
    let is_ink = |row: usize, x: usize| color_distance(pixels[row * width + x], bg) > CLUSTER_DIST_SQ;
    let row_ink = |row: usize| (0..width).filter(|&x| is_ink(row, x)).count();
    let max_ink = (r0..=r1).map(row_ink).max().unwrap_or(0);
    if max_ink == 0 {
        return None;
    }
    // Dense rows (≥50% of the line's peak ink) are the x-height body; sparse
    // rows above/below are caps/ascenders/descenders.
    let dense: Vec<usize> = (r0..=r1).filter(|&row| row_ink(row) * 2 >= max_ink).collect();
    let dtop = *dense.first()?;
    let dbot = *dense.last()?;
    let dense_band = (dbot - dtop + 1) as f64;
    let full = (r1 - r0 + 1) as f64;
    let ascent = (dbot - r0 + 1) as f64; // ink top (cap/ascender) → baseline

    // Count ink runs across the peak-ink row: a horizontal line is one run;
    // multi-stroke text has several (letters/strokes with gaps).
    let peak = (dtop..=dbot).max_by_key(|&r| row_ink(r)).unwrap_or(dtop);
    let mut runs = 0usize;
    let mut prev = false;
    for x in 0..width {
        let ink = is_ink(peak, x);
        if ink && !prev {
            runs += 1;
        }
        prev = ink;
    }

    Some((full.max(dense_band / 0.5).max(ascent / 0.75), r0, r1, runs))
}

/// Screenshot checks: `low_contrast`, plus a pixel `text_too_small` refinement
/// that catches small glyphs inside a *tall* box (padding / multi-line) the
/// certain box-height check can't see. Each carries a per-finding confidence.
/// The points→pixels factor is derived from the screenshot vs the viewport (no
/// reliance on a device-reported backing scale).
/// The WCAG verdict for a measured contrast `ratio` at the given text-size
/// class. Large text is judged at 3:1 (AA) / 4.5:1 (AAA), normal at 4.5:1 /
/// 7:1. Returns `(severity, failed threshold, standard label)`, or `None` when
/// the ratio meets the applicable bar. Pure, so the caller can also ask "what
/// would the verdict be if the size class flipped?" for confidence scoring.
fn contrast_verdict(ratio: f64, large: bool, warn_aaa: bool) -> Option<(Severity, f64, &'static str)> {
    let (aa, aaa) = if large { (3.0, 4.5) } else { (4.5, 7.0) };
    if ratio < aa {
        Some((Severity::Error, aa, "AA"))
    } else if warn_aaa && ratio < aaa {
        Some((Severity::Warning, aaa, "AAA"))
    } else {
        None
    }
}

pub fn check_contrast(
    screenshot_png: &[u8],
    root: &Element,
    viewport: &Viewport,
    config: &A11yConfig,
) -> Vec<A11yIssue> {
    match image::load_from_memory(screenshot_png) {
        Ok(img) => check_contrast_img(&img, root, viewport, config),
        Err(_) => Vec::new(),
    }
}

/// Contrast + pixel text-size checks on an already-decoded image — lets the
/// block-end audit decode the screenshot once and share it with the annotator
/// instead of decoding twice.
pub(crate) fn check_contrast_img(
    img: &image::DynamicImage,
    root: &Element,
    viewport: &Viewport,
    config: &A11yConfig,
) -> Vec<A11yIssue> {
    let mut issues = Vec::new();
    let img = img.to_rgb8();
    // Derive points→pixels from the screenshot itself (width ÷ viewport
    // width): 3.0 on iOS Retina, 1.0 on Android — robust without relying on
    // a device-reported backing scale (which iOS leaves unset).
    let sf = screenshot_scale(img.width(), viewport.width);

    let mut texts = Vec::new();
    visible_text_elements(root, viewport, false, &mut texts);

    for el in texts {
        // A clipped element shows only a sliver — we'd sample the wrong colours
        // and a fraction of the glyphs, so both contrast and the pixel text-size
        // read would be unreliable. Skip (the size-independent tree checks —
        // missing_label / duplicate_labels — still cover it).
        if is_clipped(el) {
            continue;
        }
        let b = el.effective_bounds();
        let x0 = (b.x as f64 * sf).round() as i32;
        let y0 = (b.y as f64 * sf).round() as i32;
        let w_px = (b.width as f64 * sf).round() as i32;
        let h_px = (b.height as f64 * sf).round() as i32;
        // Analyse only the centre horizontal strip. Rounded corners (and any
        // border/edge bleed) expose the surrounding background at the left/right
        // extremes, which would pollute the bg/ink detection — on a small pill
        // it can flip the contrast to fill-vs-page. We only measure text
        // *height*, so trimming width is free. Bound the trim — at most height/2
        // (enough for a full pill radius) and never more than a quarter per side
        // (keeps the centre half) — so we don't crop the text off a narrow box.
        let side = (h_px / 2).min(w_px / 4).max(0);
        let (pixels, w) = crop_pixels(&img, x0 + side, y0, (w_px - 2 * side).max(1), h_px);
        let Some((bg, fg, band_px)) = extract_text_colors(&pixels, w) else {
            continue; // no readable text / too complex — undetermined
        };
        let ratio = wcag_contrast_ratio(bg, fg);
        // Estimate the font em once (cap-line/x-height of the tallest line,
        // biased large) and reuse it for BOTH the large-text contrast split and
        // the small-text check below — one sound size read, not two proxies.
        let em = estimate_text_em_px(&pixels, w, bg);
        let em_dp = em.map(|(em_px, ..)| em_px / sf / config.density);
        // Large text is judged at the laxer 3:1. The em is biased large, so
        // comparing it to the WCAG 18pt size leans toward "large" → the laxer
        // threshold → FN-over-FP on contrast. Fall back to the cruder cap/
        // x-height band only when the em read fails.
        let large = match em_dp {
            Some(d) => d >= WCAG_LARGE_TEXT_DP,
            None => (band_px as f64 / sf) / config.density >= 16.0,
        };
        // Cleaner region (text + bg = 2 clusters) → higher measurement
        // confidence than a busy 3-cluster region.
        let cleanliness = if significant_cluster_count(&pixels) <= 2 {
            1.0
        } else {
            0.6
        };

        // Pixel text-size: catches small glyphs in a tall box (padding /
        // multi-line) the certain box-height check missed. Only where box-height
        // was blind (box ≥ floor) — it owns the short-box case at full
        // confidence, so this never double-flags. The em estimate is biased
        // large, so normal text stays clear. Heuristic ⇒ confidence < 1.
        let box_dp = el.bounds.height as f64 / config.density;
        if box_dp >= config.min_text_size_dp {
            if let Some((em_px, line_r0, line_r1, runs)) = em {
                let em_raw = em_px / sf / config.density;
                // Round UP for the verdict + display (benefit of the doubt at the
                // boundary); the raw value gates the sub-5dp artifact floor so a
                // thin 4.2dp line can't ceil its way past it.
                let em_dp = em_raw.ceil();
                // Floor: below ~5dp it's a thin rule/border/sub-pixel band, not
                // readable text → skip (FN-safe). Above the floor and below the
                // level's minimum → a genuine small-text candidate.
                if em_raw >= MIN_PLAUSIBLE_TEXT_DP && em_dp < config.min_text_size_dp {
                    let margin = ((config.min_text_size_dp - em_dp) / config.min_text_size_dp)
                        .clamp(0.0, 1.0) as f32;
                    // Temper confidence by how trustworthy the size read is:
                    // - few characters ⇒ less chance of a full-height glyph to
                    //   anchor the estimate (a lone "+"/"-" reads short);
                    // - a single ink run ⇒ a solid line or one glyph, not the
                    //   multi-stroke pattern of real text.
                    let chars = el.text.as_deref().map(|t| t.chars().count()).unwrap_or(0);
                    let char_factor = (chars as f32 / 4.0).clamp(0.35, 1.0);
                    let run_factor = if runs >= 2 { 1.0 } else { 0.5 };
                    let confidence = (cleanliness * (0.55 + 0.35 * margin) * char_factor * run_factor)
                        .clamp(0.0, 0.95);
                    // Rect marks the whole element (full height); the dimension
                    // anchors to the measured text line via `measure_bounds`, so
                    // a padded / multi-line box shows the measurement on the
                    // actual glyphs, not the whole box. `b` is the crop origin;
                    // the rows are relative to it.
                    let line = Rect {
                        x: b.x,
                        y: b.y + (line_r0 as f64 / sf).round() as i32,
                        width: b.width,
                        height: (((line_r1 - line_r0 + 1) as f64) / sf).round().max(1.0) as i32,
                    };
                    issues.push(A11yIssue {
                        check_id: "text_too_small".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "text glyphs ~{:.0}dp — below {:.0}dp minimum",
                            em_dp, config.min_text_size_dp
                        ),
                        element_type: el.element_type.clone(),
                        element_label: label_of(el),
                        element_bounds: Some(rect_of(b)),
                        related_bounds: Vec::new(),
                        measure_bounds: Some(line),
                        occlusion: Vec::new(),
                        confidence,
                        detail: Some(format!("{em_dp:.0}dp")),
                    });
                }
            }
        }

        let Some((severity, threshold, standard)) =
            contrast_verdict(ratio, large, config.contrast_warn_aaa)
        else {
            continue; // meets the applicable threshold
        };
        // Confidence: how far below threshold (a clear fail is more certain
        // than a borderline one), scaled by region cleanliness.
        let margin = ((threshold - ratio) / threshold).clamp(0.0, 1.0) as f32;
        // Size-classification confidence: only matters when flipping the
        // large/normal call would actually change the verdict (the ratio sits in
        // the size-sensitive band). When it does, temper by how near the em is to
        // the large/normal boundary — at the boundary the threshold itself is a
        // coin-flip; ≥6dp away the classification is effectively certain.
        let size_factor = if contrast_verdict(ratio, !large, config.contrast_warn_aaa)
            .map(|(s, ..)| s)
            == Some(severity)
        {
            1.0 // verdict is the same either way — size doesn't decide it
        } else {
            match em_dp {
                Some(d) => {
                    (0.6 + 0.4 * ((d - WCAG_LARGE_TEXT_DP).abs() / 6.0)).clamp(0.6, 1.0) as f32
                }
                None => 0.75, // size unknown but the verdict hinges on it
            }
        };
        // Placeholder de-rating: low-contrast placeholder text is an
        // intentional, industry-standard pattern — measured-ratio confidence is
        // unchanged, but our confidence that it's an actionable defect drops.
        // Only when the element is *showing* its placeholder (text == placeholder),
        // not when a real value has been typed in. `placeholder` is populated for
        // native inputs; webview doesn't expose it yet (so webview placeholders
        // aren't de-rated — see roadmap).
        let showing_placeholder =
            el.placeholder.as_deref().is_some_and(|ph| el.text.as_deref() == Some(ph));
        let placeholder_factor = if showing_placeholder { 0.6 } else { 1.0 };
        let confidence = (cleanliness * (0.6 + 0.4 * margin) * placeholder_factor * size_factor)
            .clamp(0.0, 1.0);
        issues.push(A11yIssue {
            check_id: "low_contrast".into(),
            severity,
            message: format!(
                "text contrast {:.1}:1 below {:.1}:1 (WCAG {standard})",
                ratio, threshold
            ),
            element_type: el.element_type.clone(),
            element_label: label_of(el),
            element_bounds: Some(rect_of(b)),
            related_bounds: Vec::new(),
            measure_bounds: None,
            occlusion: Vec::new(),
            confidence,
            detail: Some(format!("{ratio:.1}:1")),
        });
    }
    issues
}

/// Run + device context embedded in the annotated PNG's metadata so the image
/// is a self-describing artifact (shareable standalone — extractable by any PNG
/// tool, and enough to replay via `seed`).
pub struct A11yMeta {
    pub app: String,
    pub device: String,
    /// Platform string (`ios`/`android`) — lets `a11y-extract` emit an exact
    /// `--platform` in the replay command.
    pub platform: String,
    pub flow: String,
    pub block: String,
    pub iteration: u32,
    pub seed: u64,
    pub level: String,
}

/// Annotate the screenshot with one numbered finding per issue and re-encode
/// as PNG. `viewport` maps element bounds (points/dp) to screenshot px; `meta`
/// is embedded as iTXt metadata.
///
/// Visual channels keep findings legible when several land on one element:
/// - **rect** per element (orange=warning drawn first, red=error on top).
/// - **marker** = the issue's 1-based index (canonical order shared with every
///   report surface). Single-element findings put it at the top-left corner
///   (colliding corners cascade right); grouped findings put one marker at the
///   group centroid.
/// - **size checks** (`touch_target_too_small`, `text_too_small`) draw an
///   industrial dimension line on the limiting axis with the measurement —
///   off the corner, so it never collides with the contrast token.
/// - **`low_contrast`** draws its ratio token semi-translucent bottom-left.
/// - **grouped findings** (`duplicate_labels`, `overlapping_interactive`) draw
///   a rect on every member; duplicates also connect members with dashed lines.
///
/// The PNG carries three iTXt chunks: `Software` (`Golem`), a human
/// `Golem-Summary` one-liner, and `Golem-Audit` — a JSON record of the context
/// plus every finding (with screenshot-pixel `bounds` for hover tooling).
pub fn annotate_screenshot(
    screenshot_png: &[u8],
    issues: &[A11yIssue],
    viewport: &Viewport,
    meta: &A11yMeta,
) -> anyhow::Result<Vec<u8>> {
    let img = image::load_from_memory(screenshot_png)?;
    annotate_image(&img, issues, viewport, meta)
}

/// Annotate an already-decoded image — lets the block-end audit decode the
/// screenshot once and share it between the contrast check and the annotator.
pub(crate) fn annotate_image(
    img: &image::DynamicImage,
    issues: &[A11yIssue],
    viewport: &Viewport,
    meta: &A11yMeta,
) -> anyhow::Result<Vec<u8>> {
    use image::Rgba;

    let mut img = img.to_rgba8();
    let sf = screenshot_scale(img.width(), viewport.width);
    let red = Rgba([220u8, 40, 40, 255]);
    let orange = Rgba([240u8, 150, 30, 255]);
    let white = Rgba([255u8, 255, 255, 255]);
    // Marker glyph scale, sized off the screenshot so numbers stay legible on
    // both phone (~1170px) and tablet (~2048px) captures.
    let gscale = (img.width() / 350).clamp(2, 5);
    let img_w = img.width() as i32;
    let img_h = img.height() as i32;

    let scale_rect = |r: &Rect| -> (i32, i32, i32, i32) {
        (
            (r.x as f64 * sf).round() as i32,
            (r.y as f64 * sf).round() as i32,
            (r.width as f64 * sf).round().max(1.0) as i32,
            (r.height as f64 * sf).round().max(1.0) as i32,
        )
    };

    // Top-left corner markers cascade right when they collide; shared across
    // both passes so numbering layout is stable regardless of severity order.
    let mut corner_shift: std::collections::HashMap<(i32, i32), i32> =
        std::collections::HashMap::new();

    // Warnings first, errors last → red drawn on top.
    for pass in [Severity::Warning, Severity::Error] {
        let color = if pass == Severity::Error { red } else { orange };
        for (idx, issue) in issues.iter().enumerate() {
            if issue.severity != pass {
                continue;
            }
            let Some(pb) = issue.element_bounds else {
                continue;
            };
            let n = idx + 1;
            let primary = scale_rect(&pb);
            let members: Vec<(i32, i32, i32, i32)> = std::iter::once(primary)
                .chain(issue.related_bounds.iter().map(scale_rect))
                .collect();

            for m in &members {
                draw_rect(&mut img, *m, color);
            }

            let chip_h = (crate::glyph::text_height(gscale) + 2 * gscale) as i32;
            let chip_w = marker_chip_width(n, gscale) as i32;
            if members.len() > 1 && issue.check_id == "duplicate_labels" {
                // Connect group members with dashed lines (nearest corners) and
                // stamp the marker on EACH segment — for a triplicate that's the
                // same number on both links, making the grouping unmistakable.
                for w in members.windows(2) {
                    let (p, q) = nearest_corners(w[0], w[1]);
                    draw_dashed_line(&mut img, p, q, color, gscale);
                    let mx = ((p.0 + q.0) / 2.0) as i32;
                    let my = ((p.1 + q.1) / 2.0) as i32;
                    draw_marker(&mut img, mx - chip_w / 2, my - chip_h / 2, gscale, color, white, n);
                }
            } else if members.len() > 1 {
                // Other grouped findings (overlapping): both rects already
                // intersect, so a connector would be degenerate — one centred
                // marker over the group.
                let (cx, cy) = centroid(&members);
                draw_marker(&mut img, cx - chip_w / 2, cy - chip_h / 2, gscale, color, white, n);
            } else {
                let (x, y, _, _) = primary;
                let shift = corner_shift.entry((x, y)).or_insert(0);
                let chip_x = (x + *shift).min((img_w - chip_w).max(0));
                *shift += chip_w + gscale as i32;
                draw_marker(&mut img, chip_x, y, gscale, color, white, n);
            }

            // Measurement channel.
            let label = issue.detail.clone().unwrap_or_default();
            match issue.check_id.as_str() {
                "text_too_small" => {
                    // Dimension anchors to the measured text line when the pixel
                    // pass set one (padded / multi-line); the box-height pass
                    // leaves it None → spans the whole box. Left side — text is
                    // usually left-aligned (touch-target uses the right).
                    let (x, y, w, h) = issue
                        .measure_bounds
                        .as_ref()
                        .map(|r| scale_rect(r))
                        .unwrap_or(primary);
                    draw_v_dimension(&mut img, x, y, w, h, color, &label, gscale, img_w, false);
                }
                "touch_target_too_small" => {
                    let (x, y, w, h) = primary;
                    // Dimension the limiting (smaller) axis — that's what failed.
                    // Height-limited → vertical on the RIGHT; width-limited → below.
                    if h <= w {
                        draw_v_dimension(&mut img, x, y, w, h, color, &label, gscale, img_w, true);
                    } else {
                        draw_h_dimension(&mut img, x, y, w, h, color, &label, gscale, img_h);
                    }
                }
                "low_contrast" => {
                    if let Some(d) = &issue.detail {
                        let (x, y, _, h) = primary;
                        let ds = gscale.saturating_sub(1).max(2);
                        let dh = crate::glyph::text_height(ds) as i32;
                        let faint = Rgba([color.0[0], color.0[1], color.0[2], 180]);
                        // +2px clears the rect's 2px border so the token isn't
                        // flush against it.
                        crate::glyph::draw_str(
                            &mut img,
                            x + gscale as i32 + 2,
                            y + h - dh - gscale as i32,
                            ds,
                            faint,
                            d,
                        );
                    }
                }
                "occluded_element" => {
                    // A 3×3 mini-map at the bottom-right showing the *pattern* of
                    // occlusion — which sampled zones are covered (solid) vs
                    // reachable (faint outline); untested zones stay blank.
                    let cells: Vec<((i32, i32, i32, i32), bool)> = issue
                        .occlusion
                        .iter()
                        .map(|c| (scale_rect(&c.bounds), c.reachable))
                        .collect();
                    draw_occlusion_minimap(&mut img, primary, &cells, gscale, color);
                }
                "missing_label" => {
                    // No measurement to show — mark it with a "?" in the
                    // bottom-right (its "what is this control?" indicator), so a
                    // label-less control reads as deliberately flagged, not bare.
                    let (x, y, w, h) = primary;
                    let q = "?";
                    let qw = crate::glyph::text_width(q, gscale) as i32;
                    let qh = crate::glyph::text_height(gscale) as i32;
                    let m = gscale as i32 + 2;
                    crate::glyph::draw_str(
                        &mut img,
                        x + w - qw - m,
                        y + h - qh - m,
                        gscale,
                        color,
                        q,
                    );
                }
                _ => {}
            }
        }
    }

    // Encode via the `png` crate (not `image`) so we can attach iTXt metadata.
    let (iw, ih) = (img.width(), img.height());
    let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
    let warnings = issues.len() - errors;
    let (summary, audit_json) =
        build_metadata(issues, meta, sf, iw, ih, viewport, errors, warnings);

    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, iw, ih);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        // iTXt (UTF-8) — element text may be non-Latin. Keyword failures are
        // non-fatal (the image still encodes), so ignore them.
        let _ = enc.add_itxt_chunk("Software".to_string(), "Golem".to_string());
        let _ = enc.add_itxt_chunk("Golem-Summary".to_string(), summary);
        let _ = enc.add_itxt_chunk("Golem-Audit".to_string(), audit_json);
        let mut writer = enc.write_header()?;
        writer.write_image_data(img.as_raw())?;
    }
    Ok(out)
}

/// Why an annotated PNG can't be read as a Golem a11y audit.
#[derive(Debug)]
pub enum AuditReadError {
    /// The bytes aren't a decodable PNG.
    NotPng(String),
    /// No `Software = Golem` chunk — not produced by golem (or stripped by a
    /// re-encode). We refuse to interpret a foreign image's metadata.
    NotGolem,
    /// A golem PNG with no `Golem-Audit` chunk (e.g. an older build, or the
    /// chunk was stripped).
    MissingAudit,
}

impl std::fmt::Display for AuditReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotPng(e) => write!(f, "not a readable PNG: {e}"),
            Self::NotGolem => write!(
                f,
                "no Golem metadata (Software != \"Golem\") — not a golem annotated screenshot"
            ),
            Self::MissingAudit => write!(f, "Golem PNG has no Golem-Audit chunk"),
        }
    }
}

impl std::error::Error for AuditReadError {}

/// Read the embedded `Golem-Audit` JSON out of an annotated PNG. Validates the
/// `Software = Golem` iTXt chunk first, so a non-golem image is rejected rather
/// than mis-parsed. Returns the raw JSON string (the caller deserializes it).
pub fn read_embedded_audit(png_bytes: &[u8]) -> Result<String, AuditReadError> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let reader = decoder
        .read_info()
        .map_err(|e| AuditReadError::NotPng(e.to_string()))?;
    let info = reader.info();

    let text_of = |keyword: &str| {
        info.utf8_text
            .iter()
            .find(|c| c.keyword == keyword)
            .and_then(|c| {
                let mut c = c.clone();
                c.decompress_text().ok()?;
                c.get_text().ok()
            })
    };

    if text_of("Software").as_deref() != Some("Golem") {
        return Err(AuditReadError::NotGolem);
    }
    text_of("Golem-Audit").ok_or(AuditReadError::MissingAudit)
}

/// Build the `(Golem-Summary, Golem-Audit)` metadata for the annotated PNG: a
/// human one-liner and a JSON record (context + every finding, with bounds in
/// screenshot pixels so hover tooling can overlay directly on the image).
#[allow(clippy::too_many_arguments)]
fn build_metadata(
    issues: &[A11yIssue],
    meta: &A11yMeta,
    sf: f64,
    img_w: u32,
    img_h: u32,
    viewport: &Viewport,
    errors: usize,
    warnings: usize,
) -> (String, String) {
    let px = |r: &Rect| {
        serde_json::json!({
            "x": (r.x as f64 * sf).round() as i32,
            "y": (r.y as f64 * sf).round() as i32,
            "w": (r.width as f64 * sf).round() as i32,
            "h": (r.height as f64 * sf).round() as i32,
        })
    };
    let issues_json: Vec<serde_json::Value> = issues
        .iter()
        .enumerate()
        .map(|(i, iss)| {
            serde_json::json!({
                "marker": i + 1,
                "check": iss.check_id,
                "severity": if iss.severity == Severity::Error { "error" } else { "warning" },
                "message": iss.message,
                "detail": iss.detail,
                "confidence": iss.confidence,
                "bounds": iss.element_bounds.as_ref().map(&px),
                "related": iss.related_bounds.iter().map(&px).collect::<Vec<_>>(),
            })
        })
        .collect();

    let summary = format!(
        "golem a11y · flow \"{}\" block \"{}\" · {} · {} error(s), {} warning(s) · seed {} · level {}",
        meta.flow, meta.block, meta.device, errors, warnings, meta.seed, meta.level
    );
    let audit = serde_json::json!({
        "software": "Golem",
        "app": meta.app,
        "device": meta.device,
        "platform": meta.platform,
        "flow": meta.flow,
        "block": meta.block,
        "iteration": meta.iteration,
        "seed": meta.seed,
        "a11y_level": meta.level,
        "image": { "w": img_w, "h": img_h },
        "viewport": { "w": viewport.width, "h": viewport.height },
        "errors": errors,
        "warnings": warnings,
        "issues": issues_json,
    });
    (summary, audit.to_string())
}

/// Pixel width of the marker chip for finding number `n` at glyph `scale` —
/// the rendered digits plus `scale` padding on each side. Mirrors the chip
/// geometry in [`draw_marker`] so cascade layout and drawing agree.
fn marker_chip_width(n: usize, scale: u32) -> u32 {
    crate::glyph::text_width(&n.to_string(), scale) + 2 * scale.max(1)
}

/// Draw a numbered marker chip at the top-left corner of a finding rectangle:
/// a solid `chip` background (severity colour) with the number rendered in
/// `text` colour on top, so it stays readable over any screenshot content.
fn draw_marker(
    img: &mut image::RgbaImage,
    x: i32,
    y: i32,
    scale: u32,
    chip: image::Rgba<u8>,
    text: image::Rgba<u8>,
    n: usize,
) {
    use imageproc::drawing::draw_filled_rect_mut;
    use imageproc::rect::Rect as IpRect;

    let s = n.to_string();
    let pad = scale.max(1) as i32;
    let tw = crate::glyph::text_width(&s, scale);
    let th = crate::glyph::text_height(scale);
    let chip_w = tw + 2 * pad as u32;
    let chip_h = th + 2 * pad as u32;
    draw_filled_rect_mut(img, IpRect::at(x, y).of_size(chip_w, chip_h), chip);
    crate::glyph::draw_str(img, x + pad, y + pad, scale, text, &s);
}

/// 2px hollow rectangle in `color` for a scaled `(x, y, w, h)`.
fn draw_rect(img: &mut image::RgbaImage, rect: (i32, i32, i32, i32), color: image::Rgba<u8>) {
    use imageproc::drawing::draw_hollow_rect_mut;
    use imageproc::rect::Rect as IpRect;
    let (x, y, w, h) = rect;
    for inset in 0..2 {
        let rw = (w - inset * 2).max(1) as u32;
        let rh = (h - inset * 2).max(1) as u32;
        draw_hollow_rect_mut(img, IpRect::at(x + inset, y + inset).of_size(rw, rh), color);
    }
}

/// Centre of a scaled rect.
fn rect_center((x, y, w, h): (i32, i32, i32, i32)) -> (f32, f32) {
    (x as f32 + w as f32 / 2.0, y as f32 + h as f32 / 2.0)
}

/// Draw a small 3×3 occupancy map at the bottom-right of `elem`, filling the
/// tested cells: **covered** ones solid, **reachable** ones a faint (opaque,
/// pre-blended pale) outline; untested zones are left blank so the map makes no
/// claim about them. `cells` are `((x,y,w,h), reachable)` sub-rects already
/// scaled to image px, positioned on the control's 3×3 lattice by their centre.
/// No-op when the element is too small to place the grid.
fn draw_occlusion_minimap(
    img: &mut image::RgbaImage,
    elem: (i32, i32, i32, i32),
    cells: &[((i32, i32, i32, i32), bool)],
    gscale: u32,
    color: image::Rgba<u8>,
) {
    use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut};
    use imageproc::rect::Rect as IpRect;
    let (ex, ey, ew, eh) = elem;
    if ew <= 0 || eh <= 0 {
        return;
    }
    let cell = (gscale as i32 * 2).max(4);
    let grid = cell * 3;
    let m = gscale as i32 + 2;
    let gx = ex + ew - grid - m;
    let gy = ey + eh - grid - m;
    // Needs room inside the control; skip if it would spill past the top-left.
    if gx < ex || gy < ey {
        return;
    }
    // Reachable outline: the finding colour blended 50% toward white → an
    // *opaque* pale stroke. (A real alpha here would leave semi-transparent
    // pixels that composite dark over a dark viewer backdrop.)
    let pale = image::Rgba([
        ((color.0[0] as u16 + 255) / 2) as u8,
        ((color.0[1] as u16 + 255) / 2) as u8,
        ((color.0[2] as u16 + 255) / 2) as u8,
        255,
    ]);
    for &((cx, cy, cw, ch), reachable) in cells {
        let col = (((cx + cw / 2 - ex) * 3) / ew).clamp(0, 2);
        let row = (((cy + ch / 2 - ey) * 3) / eh).clamp(0, 2);
        let r = IpRect::at(gx + col * cell, gy + row * cell).of_size(cell as u32, cell as u32);
        if reachable {
            draw_hollow_rect_mut(img, r, pale);
        } else {
            draw_filled_rect_mut(img, r, color);
        }
    }
}

/// Centroid of all member-rect centres (marker anchor for grouped findings).
fn centroid(members: &[(i32, i32, i32, i32)]) -> (i32, i32) {
    let n = members.len().max(1) as f32;
    let (sx, sy) = members
        .iter()
        .map(|m| rect_center(*m))
        .fold((0.0, 0.0), |(ax, ay), (cx, cy)| (ax + cx, ay + cy));
    ((sx / n).round() as i32, (sy / n).round() as i32)
}

/// The closest pair of corners between two scaled rects — endpoints for a
/// connector that visually links the two without crossing them.
fn nearest_corners(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> ((f32, f32), (f32, f32)) {
    let corners = |(x, y, w, h): (i32, i32, i32, i32)| {
        [
            (x as f32, y as f32),
            ((x + w) as f32, y as f32),
            (x as f32, (y + h) as f32),
            ((x + w) as f32, (y + h) as f32),
        ]
    };
    let (ca, cb) = (corners(a), corners(b));
    let mut best = (ca[0], cb[0]);
    let mut best_d = f32::MAX;
    for &p in &ca {
        for &q in &cb {
            let d = (p.0 - q.0).powi(2) + (p.1 - q.1).powi(2);
            if d < best_d {
                best_d = d;
                best = (p, q);
            }
        }
    }
    best
}

/// A 2px solid line (drawn twice, offset 1px perpendicular for weight).
fn draw_solid_line(img: &mut image::RgbaImage, p0: (f32, f32), p1: (f32, f32), color: image::Rgba<u8>) {
    use imageproc::drawing::draw_line_segment_mut;
    draw_line_segment_mut(img, p0, p1, color);
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let (px, py) = (-dy / len, dx / len); // unit perpendicular
    draw_line_segment_mut(img, (p0.0 + px, p0.1 + py), (p1.0 + px, p1.1 + py), color);
}

/// A dashed line in `color`; dash/gap scale with the glyph size.
fn draw_dashed_line(
    img: &mut image::RgbaImage,
    p0: (f32, f32),
    p1: (f32, f32),
    color: image::Rgba<u8>,
    scale: u32,
) {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let dash = (3 * scale) as f32;
    let period = dash + (2 * scale) as f32;
    let mut t = 0.0;
    while t < len {
        let e = (t + dash).min(len);
        draw_solid_line(
            img,
            (p0.0 + ux * t, p0.1 + uy * t),
            (p0.0 + ux * e, p0.1 + uy * e),
            color,
        );
        t += period;
    }
}

/// A small inward-pointing arrowhead at `tip`, opening along unit `dir`.
fn draw_arrowhead(
    img: &mut image::RgbaImage,
    tip: (f32, f32),
    dir: (f32, f32),
    color: image::Rgba<u8>,
    size: f32,
) {
    let (dx, dy) = dir;
    let (px, py) = (-dy, dx); // perpendicular
    let back = (tip.0 + dx * size, tip.1 + dy * size);
    draw_solid_line(img, tip, (back.0 + px * size * 0.6, back.1 + py * size * 0.6), color);
    draw_solid_line(img, tip, (back.0 - px * size * 0.6, back.1 - py * size * 0.6), color);
}

/// Vertical (height) dimension annotation in industrial style: extension lines
/// at top+bottom, an arrowed dimension line spanning the height, and the
/// measurement beside it. `right` chooses the side — `touch_target` uses the
/// left edge, `text_too_small` the right, so the two are distinguishable and
/// never overlap when both apply to one element. Falls back to the inner side
/// when there's no room (element flush to a screen edge).
#[allow(clippy::too_many_arguments)]
fn draw_v_dimension(
    img: &mut image::RgbaImage,
    rect_x: i32,
    rect_y: i32,
    rect_w: i32,
    rect_h: i32,
    color: image::Rgba<u8>,
    label: &str,
    scale: u32,
    img_w: i32,
    right: bool,
) {
    let ds = scale.saturating_sub(1).max(2);
    let label_w = crate::glyph::text_width(label, ds) as i32;
    let lh = crate::glyph::text_height(ds) as i32;
    let off = (4 * scale) as i32;
    let gap = (2 * scale) as i32;
    let (top, bot) = (rect_y as f32, (rect_y + rect_h) as f32);
    let asz = (2 * scale) as f32;

    // Edge the dimension hangs off, and whether the preferred (outward) side
    // has room for the line + label.
    let (edge, outward) = if right {
        let e = rect_x + rect_w;
        (e, e + off + gap + label_w <= img_w)
    } else {
        (rect_x, rect_x - off - gap - label_w >= 0)
    };
    // Dimension-line x: outward from the edge, or inward on overflow.
    let lx = match (right, outward) {
        (true, true) => edge + off,
        (true, false) => edge - off,
        (false, true) => edge - off,
        (false, false) => edge + off,
    };
    let lxf = lx as f32;
    draw_solid_line(img, (edge as f32, top), (lxf, top), color);
    draw_solid_line(img, (edge as f32, bot), (lxf, bot), color);
    draw_solid_line(img, (lxf, top), (lxf, bot), color);
    draw_arrowhead(img, (lxf, top), (0.0, 1.0), color, asz);
    draw_arrowhead(img, (lxf, bot), (0.0, -1.0), color, asz);
    // Label vertically centred, on the far side of the dimension line.
    let ly = rect_y + rect_h / 2 - lh / 2;
    let label_left = (right && !outward) || (!right && outward);
    let label_x = if label_left { lx - gap - label_w } else { lx + gap };
    crate::glyph::draw_str(img, label_x, ly, ds, color, label);
}

/// Horizontal (width) dimension annotation, below the rect (inside on
/// overflow). Mirrors [`draw_v_dimension`] for the width axis.
fn draw_h_dimension(
    img: &mut image::RgbaImage,
    rect_x: i32,
    rect_y: i32,
    rect_w: i32,
    rect_h: i32,
    color: image::Rgba<u8>,
    label: &str,
    scale: u32,
    img_h: i32,
) {
    let ds = scale.saturating_sub(1).max(2);
    let lh = crate::glyph::text_height(ds) as i32;
    let off = (4 * scale) as i32;
    let gap = (2 * scale) as i32;
    let bottom = rect_y + rect_h;
    let outside = bottom + off + lh + gap <= img_h;
    let by = if outside { bottom + off } else { bottom - off };
    let byf = by as f32;
    let (left, right) = (rect_x as f32, (rect_x + rect_w) as f32);
    draw_solid_line(img, (left, bottom as f32), (left, byf), color);
    draw_solid_line(img, (right, bottom as f32), (right, byf), color);
    draw_solid_line(img, (left, byf), (right, byf), color);
    let asz = (2 * scale) as f32;
    draw_arrowhead(img, (left, byf), (1.0, 0.0), color, asz);
    draw_arrowhead(img, (right, byf), (-1.0, 0.0), color, asz);
    let label_w = crate::glyph::text_width(label, ds) as i32;
    let label_x = rect_x + rect_w / 2 - label_w / 2;
    let ly = if outside { by + gap } else { by - gap - lh };
    crate::glyph::draw_str(img, label_x, ly, ds, color, label);
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
        // Only strict forces a screenshot (for contrast); others are free.
        assert!(!A11yLevel::Critical.forces_screenshot());
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

    #[test]
    fn touch_target_skips_clipped_element() {
        // A 20dp-wide button (would be a touch-target error) clipped to a 10dp
        // sliver: its real target is bigger, so the sliver size is a misleading
        // "too small" → skip touch_target. (missing_label still applies — it's
        // size-independent and reliable when clipped.)
        let mut clipped = el("button", 0, 0, 20, 60);
        clipped.visible_bounds = Some(Bounds::new(0, 0, 20, 10));
        let issues = audit_hierarchy(&root_with(vec![clipped]), &vp(), &relaxed());
        assert!(
            issues.iter().all(|i| i.check_id != "touch_target_too_small"),
            "clipped element skipped for touch_target: {issues:?}"
        );
        // Same size, not clipped → flagged (20dp < 24dp error).
        let issues = audit_hierarchy(&root_with(vec![el("button", 0, 0, 20, 60)]), &vp(), &relaxed());
        assert!(
            issues.iter().any(|i| i.check_id == "touch_target_too_small"),
            "unclipped small target flagged: {issues:?}"
        );
    }

    #[test]
    fn fully_occluded_element_is_skipped() {
        // Clickable, unlabeled, but its only hit-test point is covered → behind
        // an overlay → not judged (no missing_label).
        let mut covered = el("button", 0, 0, 100, 50);
        covered.hit_points = vec![golem_element::HitPoint { x: 50, y: 25, hit: false }];
        let issues = audit_hierarchy(&root_with(vec![covered]), &vp(), &relaxed());
        assert!(
            issues.iter().all(|i| i.check_id != "missing_label"),
            "occluded control must not be judged: {issues:?}"
        );
        // Identical element, hit-test clear → flagged as normal.
        let mut clear = el("button", 0, 0, 100, 50);
        clear.hit_points = vec![golem_element::HitPoint { x: 50, y: 25, hit: true }];
        let issues = audit_hierarchy(&root_with(vec![clear]), &vp(), &relaxed());
        assert!(
            issues.iter().any(|i| i.check_id == "missing_label"),
            "visible unlabeled control should flag: {issues:?}"
        );
    }

    #[test]
    fn occluded_element_flags_partial_coverage() {
        let pts = |hits: &[bool]| -> Vec<golem_element::HitPoint> {
            hits.iter()
                .enumerate()
                .map(|(i, &h)| golem_element::HitPoint { x: i as i32 * 10, y: 25, hit: h })
                .collect()
        };
        // 1 of 4 reachable (25%) → below the 0.5 floor → flagged (label present
        // so missing_label doesn't muddy it).
        let mut covered = with_text(el("button", 0, 0, 100, 50), "Save");
        covered.hit_points = pts(&[true, false, false, false]);
        let issues = audit_hierarchy(&root_with(vec![covered]), &vp(), &relaxed());
        assert!(
            issues.iter().any(|i| i.check_id == "occluded_element"),
            "partly-covered control flagged: {issues:?}"
        );

        // 3 of 4 reachable (75%) → at/above the floor → not flagged.
        let mut mostly = with_text(el("button", 0, 0, 100, 50), "Save");
        mostly.hit_points = pts(&[true, true, true, false]);
        let issues = audit_hierarchy(&root_with(vec![mostly]), &vp(), &relaxed());
        assert!(
            issues.iter().all(|i| i.check_id != "occluded_element"),
            "mostly-reachable control not flagged: {issues:?}"
        );

        // No hit-test data → not flagged (unknown, FN-safe).
        let plain = with_text(el("button", 0, 0, 100, 50), "Save");
        let issues = audit_hierarchy(&root_with(vec![plain]), &vp(), &relaxed());
        assert!(
            issues.iter().all(|i| i.check_id != "occluded_element"),
            "no hit_points → not flagged: {issues:?}"
        );

        // Level-dependent floor: exactly half reachable (2 of 4) is below
        // strict's 0.75 but not relaxed's 0.5 — strict flags, relaxed doesn't.
        let half = || {
            let mut e = with_text(el("button", 0, 0, 100, 50), "Save");
            e.hit_points = pts(&[true, true, false, false]);
            root_with(vec![e])
        };
        assert!(
            audit_hierarchy(&half(), &vp(), &strict())
                .iter()
                .any(|i| i.check_id == "occluded_element"),
            "50%-covered control flagged at strict"
        );
        assert!(
            audit_hierarchy(&half(), &vp(), &relaxed())
                .iter()
                .all(|i| i.check_id != "occluded_element"),
            "50%-covered control NOT flagged at relaxed"
        );
    }

    #[test]
    fn oversized_clickable_exempt_but_small_one_flagged() {
        // A clickable spanning the whole viewport with no label is a backdrop/
        // root container, not a control → exempt from missing_label.
        let big = el("other", 0, 0, 1000, 2000); // == full vp() ⇒ oversized
        let issues = audit_hierarchy(&root_with(vec![big]), &vp(), &relaxed());
        assert!(
            issues.iter().all(|i| i.check_id != "missing_label"),
            "full-viewport container must be exempt: {issues:?}"
        );
        // A normal-sized unlabeled clickable is still flagged — the exemption
        // is size-gated, not a blanket pass.
        let small = el("button", 0, 0, 100, 50);
        let issues = audit_hierarchy(&root_with(vec![small]), &vp(), &relaxed());
        assert!(
            issues.iter().any(|i| i.check_id == "missing_label"),
            "small unlabeled control should still flag: {issues:?}"
        );
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
        // One marker for the group; the other two members ride in related_bounds
        // so the annotator can rect + connect all three.
        assert_eq!(dups[0].related_bounds.len(), 2, "triplicate → primary + 2 related");
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

    // ── disabled-control exemption (WCAG inactive-component) ────────
    #[test]
    fn disabled_control_exempt_from_tree_checks() {
        // Tiny disabled button — would trip touch_target if enabled.
        let mut b = with_text(el("button", 0, 0, 10, 10), "Off");
        b.enabled = false;
        let issues = audit_hierarchy(&root_with(vec![b]), &vp(), &relaxed());
        assert!(issues.is_empty(), "disabled control is exempt: {issues:?}");
    }

    #[test]
    fn enabled_control_still_checked() {
        // Same button, enabled → touch_target fires (sanity that the exemption
        // is gated on enabled, not blanket).
        let issues = audit_hierarchy(
            &root_with(vec![with_text(el("button", 0, 0, 10, 10), "On")]),
            &vp(),
            &relaxed(),
        );
        assert!(issues.iter().any(|i| i.check_id == "touch_target_too_small"));
    }

    #[test]
    fn disabled_control_exempt_from_contrast() {
        // Grey-on-white text that would flag, but the control is disabled.
        let mut img = image::RgbImage::from_pixel(100, 100, image::Rgb([255, 255, 255]));
        for y in 45..55u32 {
            for x in 10..90u32 {
                img.put_pixel(x, y, image::Rgb([200, 200, 200]));
            }
        }
        let png = png_of(img);
        let mut b = with_text(el("button", 0, 0, 100, 100), "Disabled");
        b.enabled = false;
        let issues = check_contrast(&png, &root_with(vec![b]), &Viewport::new(100, 100), &relaxed());
        assert!(
            issues.iter().all(|i| i.check_id != "low_contrast"),
            "disabled control exempt from contrast: {issues:?}"
        );
    }

    // ── text_too_small (certain box-height check, no screenshot) ────
    #[test]
    fn text_too_small_by_box_height() {
        // 8dp-tall text box (density 1.0) is certainly below the 10dp relaxed
        // minimum — flagged with full confidence, no screenshot.
        let mut t = with_text(el("StaticText", 0, 0, 100, 8), "hi");
        t.clickable = false;
        let issues = audit_hierarchy(&root_with(vec![t]), &vp(), &relaxed());
        let f = issues
            .iter()
            .find(|i| i.check_id == "text_too_small")
            .expect("8dp box flagged");
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.confidence, 1.0, "box-height check is deterministic");
    }

    #[test]
    fn text_normal_box_height_not_flagged() {
        let mut t = with_text(el("StaticText", 0, 0, 100, 20), "hi");
        t.clickable = false;
        let issues = audit_hierarchy(&root_with(vec![t]), &vp(), &relaxed());
        assert!(issues.iter().all(|i| i.check_id != "text_too_small"));
    }

    #[test]
    fn critical_text_threshold_is_lower_than_relaxed() {
        // A 9dp text box: flagged at relaxed (10dp floor) but clean at critical
        // (8dp floor) — critical catches only egregiously small text.
        let tree = || {
            let mut t = with_text(el("StaticText", 0, 0, 100, 9), "hi");
            t.clickable = false;
            root_with(vec![t])
        };
        let relaxed = audit_hierarchy(&tree(), &vp(), &A11yConfig::new(A11yLevel::Relaxed, 1.0));
        assert!(
            relaxed.iter().any(|i| i.check_id == "text_too_small"),
            "9dp flagged at relaxed"
        );
        let critical = audit_hierarchy(&tree(), &vp(), &A11yConfig::new(A11yLevel::Critical, 1.0));
        assert!(
            critical.iter().all(|i| i.check_id != "text_too_small"),
            "9dp clean at critical's 8dp floor"
        );
    }

    #[test]
    fn text_too_small_ignores_scroll_clip() {
        // A normal 40dp row clipped to a 5dp sliver by a scroll viewport must
        // NOT flag — the glyph size is the full box, not the visible clip.
        let mut t = with_text(el("div", 0, 0, 200, 40), "Row");
        t.clickable = false;
        t.visible_bounds = Some(Bounds::new(0, 0, 200, 5));
        let issues = audit_hierarchy(&root_with(vec![t]), &vp(), &relaxed());
        assert!(
            issues.iter().all(|i| i.check_id != "text_too_small"),
            "clipped row should be measured by full height: {issues:?}"
        );
    }

    // ── screenshot checks: WCAG contrast ────────────────────────────
    #[test]
    fn contrast_black_on_white_is_21() {
        let r = wcag_contrast_ratio([0, 0, 0], [255, 255, 255]);
        assert!((r - 21.0).abs() < 0.1, "black/white ≈ 21:1, got {r}");
    }

    #[test]
    fn contrast_identical_is_1() {
        assert!((wcag_contrast_ratio([120, 120, 120], [120, 120, 120]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn contrast_is_symmetric() {
        let a = [10, 80, 200];
        let b = [240, 240, 240];
        assert!((wcag_contrast_ratio(a, b) - wcag_contrast_ratio(b, a)).abs() < 1e-9);
    }

    // ── colour extraction (band-isolated) ───────────────────────────
    /// 10×10: white box with a black glyph band in rows 4..7. Ink spans
    /// cols 1..9 (80% width) — like text (inter-letter gaps), not a
    /// full-width border that the band detector excludes.
    fn text_box() -> (Vec<Rgb>, usize) {
        let w = 10usize;
        let mut px = vec![[255u8, 255, 255]; w * 10];
        for row in 4..7 {
            for x in 1..(w - 1) {
                px[row * w + x] = [0, 0, 0];
            }
        }
        (px, w)
    }

    #[test]
    fn extract_text_colors_isolates_band_ink() {
        let (px, w) = text_box();
        let (bg, fg, h) = extract_text_colors(&px, w).expect("text present");
        assert!(bg[0] > 200, "bg ~white");
        assert!(fg[0] < 60, "fg ~black (glyph ink from band)");
        assert_eq!(h, 3, "band height = 3 rows");
        assert!(wcag_contrast_ratio(bg, fg) > 15.0, "black-on-white is high contrast");
    }

    #[test]
    fn extract_text_colors_sparse_text_still_finds_ink() {
        // Mostly-white box, a single thin ink row (row 5, ~80% width) — the
        // whole-box histogram would miss it; band isolation must still pick
        // black fg.
        let w = 20usize;
        let mut px = vec![[255u8, 255, 255]; w * 40];
        for x in 2..(w - 2) {
            px[5 * w + x] = [0, 0, 0];
        }
        let (bg, fg, _) = extract_text_colors(&px, w).expect("sparse text found");
        assert!(bg[0] > 200 && fg[0] < 60, "bg white, fg black");
    }

    #[test]
    fn extract_text_colors_solid_is_none() {
        let px = vec![[128u8, 128, 128]; 100];
        assert!(extract_text_colors(&px, 10).is_none(), "no ink → None");
    }

    #[test]
    fn extract_text_colors_complex_is_none() {
        // Four significant clusters → gradient/photo → undetermined.
        let mut px = Vec::new();
        for c in [[10u8, 10, 10], [90, 90, 90], [170, 170, 170], [250, 250, 250]] {
            px.extend(vec![c; 25]);
        }
        assert!(extract_text_colors(&px, 10).is_none(), ">3 clusters → None");
    }

    #[test]
    fn text_band_strips_padding() {
        let (px, w) = text_box();
        assert_eq!(text_band(&px, w, [255, 255, 255]), Some((4, 6)));
    }

    #[test]
    fn text_band_all_background_is_none() {
        let px = vec![[255u8, 255, 255]; 100];
        assert!(text_band(&px, 10, [255, 255, 255]).is_none());
    }

    // ── check_contrast / annotate (integration) ─────────────────────
    fn png_of(img: image::RgbImage) -> Vec<u8> {
        let mut c = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut c, image::ImageFormat::Png)
            .expect("encode png");
        c.into_inner()
    }

    #[test]
    fn contrast_verdict_large_vs_normal_split() {
        use Severity::{Error, Warning};
        // Normal text: <4.5 AA error, [4.5,7) AAA warn, ≥7 passes.
        assert!(matches!(contrast_verdict(3.0, false, true), Some((Error, _, "AA"))));
        assert!(matches!(contrast_verdict(5.0, false, true), Some((Warning, _, "AAA"))));
        assert!(contrast_verdict(7.5, false, true).is_none());
        // Large text is laxer: 3:1 AA, 4.5:1 AAA.
        assert!(matches!(contrast_verdict(2.5, true, true), Some((Error, _, "AA"))));
        assert!(matches!(contrast_verdict(4.0, true, true), Some((Warning, _, "AAA"))));
        assert!(contrast_verdict(5.0, true, true).is_none());
        // 3.5:1 is size-sensitive — AA error if normal, only AAA warn if large.
        assert_eq!(contrast_verdict(3.5, false, true).map(|(s, ..)| s), Some(Error));
        assert_eq!(contrast_verdict(3.5, true, true).map(|(s, ..)| s), Some(Warning));
        // With AAA warnings off, only AA failures flag.
        assert!(contrast_verdict(5.0, false, false).is_none());
    }

    #[test]
    fn check_contrast_flags_low_contrast_text() {
        // White canvas, a light-grey text band (rows 45..55). Kept centred (cols
        // 35..65) so it survives the centre-strip crop the way real text does —
        // a full-width solid band would read as a border, not text.
        let mut img = image::RgbImage::from_pixel(100, 100, image::Rgb([255, 255, 255]));
        for y in 45..55u32 {
            for x in 35..65u32 {
                img.put_pixel(x, y, image::Rgb([200, 200, 200]));
            }
        }
        let png = png_of(img);
        let text = with_text(el("StaticText", 0, 0, 100, 100), "hello");
        let root = root_with(vec![text]);
        // Viewport width matches the image width ⇒ scale 1.0.
        let issues = check_contrast(&png, &root, &Viewport::new(100, 100), &relaxed());
        assert!(
            issues.iter().any(|i| i.check_id == "low_contrast" && i.severity == Severity::Error),
            "grey-on-white text is below AA: {issues:?}"
        );
    }

    #[test]
    fn check_contrast_passes_high_contrast() {
        // Black text band on white → 21:1, no contrast issue.
        let mut img = image::RgbImage::from_pixel(100, 100, image::Rgb([255, 255, 255]));
        for y in 45..55u32 {
            for x in 10..90u32 {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        let png = png_of(img);
        let text = with_text(el("StaticText", 0, 0, 100, 100), "hello");
        let root = root_with(vec![text]);
        let issues = check_contrast(&png, &root, &Viewport::new(100, 100), &relaxed());
        assert!(issues.iter().all(|i| i.check_id != "low_contrast"));
    }

    fn strict() -> A11yConfig {
        A11yConfig::new(A11yLevel::Strict, 1.0)
    }

    #[test]
    fn low_contrast_placeholder_is_derated() {
        // Grey low-contrast text band (centred so it survives the side crop).
        let mut img = image::RgbImage::from_pixel(100, 100, image::Rgb([255, 255, 255]));
        for y in 45..55u32 {
            for x in 35..65u32 {
                img.put_pixel(x, y, image::Rgb([170, 170, 170]));
            }
        }
        let png = png_of(img);
        let conf_for = |text: &str, placeholder: &str| {
            let mut e = with_text(el("input", 0, 0, 100, 100), text);
            e.clickable = false;
            e.placeholder = Some(placeholder.into());
            check_contrast(&png, &root_with(vec![e]), &Viewport::new(100, 100), &strict())
                .iter()
                .find(|i| i.check_id == "low_contrast")
                .map(|i| i.confidence)
        };
        // Showing the placeholder (text == placeholder) → de-rated.
        let ph = conf_for("Search", "Search").expect("placeholder contrast finding");
        // A typed value (text != placeholder) → full confidence (caveat honored).
        let val = conf_for("Search", "Find…").expect("value contrast finding");
        assert!(ph < val, "placeholder de-rated: {ph} should be < value {val}");
    }

    fn text_in_tall_box(ink_rows: std::ops::Range<u32>) -> Vec<u8> {
        // 100×100 white with a black glyph band over `ink_rows` (moderate-width
        // ink so it reads as a text row, not a border). The element box is the
        // full 100dp → box-height is blind, exercising the pixel pass.
        let mut img = image::RgbImage::from_pixel(100, 100, image::Rgb([255, 255, 255]));
        for y in ink_rows {
            for x in 20..60u32 {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        png_of(img)
    }

    fn tall_text_el() -> Element {
        let mut t = with_text(el("p", 0, 0, 100, 100), "x");
        t.clickable = false;
        t
    }

    #[test]
    fn pixel_text_size_flags_small_glyphs_in_tall_box() {
        // ~3px glyph band in a 100dp box → the pixel pass flags it (box-height
        // is blind). Black-on-white ⇒ no contrast finding, so it's isolated.
        let png = text_in_tall_box(48..51);
        let issues = check_contrast(
            &png,
            &root_with(vec![tall_text_el()]),
            &Viewport::new(100, 100),
            &strict(),
        );
        let f = issues
            .iter()
            .find(|i| i.check_id == "text_too_small")
            .expect("small glyphs in a tall box flagged");
        assert_eq!(f.severity, Severity::Warning);
        // Heuristic ⇒ < 1; and a single-char ("x") single-run band is tempered
        // hard by the char-count and run-count factors.
        assert!(f.confidence < 0.4, "single char/run ⇒ low confidence, got {}", f.confidence);
    }

    #[test]
    fn pixel_text_size_floor_skips_thin_band() {
        // A 2px ink band estimates ~4dp — below the 5dp plausibility floor, so
        // it's treated as a thin rule / sub-pixel artifact, not text → no flag.
        let png = text_in_tall_box(48..50);
        let issues = check_contrast(
            &png,
            &root_with(vec![tall_text_el()]),
            &Viewport::new(100, 100),
            &strict(),
        );
        assert!(
            issues.iter().all(|i| i.check_id != "text_too_small"),
            "sub-{MIN_PLAUSIBLE_TEXT_DP}dp band skipped by the floor: {issues:?}"
        );
    }

    #[test]
    fn pixel_text_size_passes_normal_glyphs_in_tall_box() {
        // A ~16px band → em estimate stays ≥ floor (biased large), no flag.
        let png = text_in_tall_box(40..56);
        let issues = check_contrast(
            &png,
            &root_with(vec![tall_text_el()]),
            &Viewport::new(100, 100),
            &strict(),
        );
        assert!(
            issues.iter().all(|i| i.check_id != "text_too_small"),
            "normal-size glyphs must not flag: {issues:?}"
        );
    }

    #[test]
    fn pixel_text_size_skips_short_box() {
        // A short box is the box-height check's job (full confidence) — the
        // pixel pass must not also fire and double-flag.
        let png = text_in_tall_box(48..51);
        let mut t = with_text(el("p", 0, 0, 100, 8), "x");
        t.clickable = false;
        let issues =
            check_contrast(&png, &root_with(vec![t]), &Viewport::new(100, 100), &strict());
        assert!(
            issues.iter().all(|i| i.check_id != "text_too_small"),
            "short box left to the box-height check: {issues:?}"
        );
    }

    #[test]
    fn annotate_produces_valid_png_same_size() {
        let png = png_of(image::RgbImage::from_pixel(60, 40, image::Rgb([255, 255, 255])));
        let issue = A11yIssue {
            check_id: "touch_target_too_small".into(),
            severity: Severity::Warning,
            message: "m".into(),
            element_type: "button".into(),
            element_label: None,
            element_bounds: Some(Rect { x: 5, y: 5, width: 20, height: 20 }),
            related_bounds: Vec::new(),
            measure_bounds: None,
            occlusion: Vec::new(),
            confidence: 1.0,
            detail: None,
        };
        let out =
            annotate_screenshot(&png, &[issue], &Viewport::new(60, 40), &test_meta()).expect("ok");
        let decoded = image::load_from_memory(&out).expect("valid png");
        assert_eq!((decoded.width(), decoded.height()), (60, 40));
        // Self-describing metadata travels in the PNG (uncompressed iTXt → the
        // keyword + JSON appear verbatim in the bytes).
        let has = |k: &[u8]| out.windows(k.len()).any(|w| w == k);
        assert!(has(b"Golem-Audit"), "audit chunk present");
        assert!(has(b"\"seed\":42"), "seed embedded for replay");
        assert!(has(b"touch_target_too_small"), "finding embedded");
    }

    fn test_meta() -> A11yMeta {
        A11yMeta {
            app: "com.example".into(),
            device: "Test Device".into(),
            platform: "ios".into(),
            flow: "f".into(),
            block: "b".into(),
            iteration: 0,
            seed: 42,
            level: "strict".into(),
        }
    }

    fn encode_png_with_chunks(chunks: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 1, 1);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            for (k, v) in chunks {
                let _ = enc.add_itxt_chunk(k.to_string(), v.to_string());
            }
            let mut w = enc.write_header().expect("header");
            w.write_image_data(&[0, 0, 0, 0]).expect("data");
        }
        out
    }

    #[test]
    fn read_embedded_audit_roundtrips_and_rejects_foreign() {
        // A golem PNG yields its Golem-Audit JSON verbatim.
        let golem = encode_png_with_chunks(&[
            ("Software", "Golem"),
            ("Golem-Audit", r#"{"seed":42}"#),
        ]);
        assert_eq!(
            read_embedded_audit(&golem).expect("golem audit reads"),
            r#"{"seed":42}"#
        );
        // A foreign image (Software != Golem) is refused, not mis-parsed.
        let foreign = encode_png_with_chunks(&[("Software", "Acme Snipper")]);
        assert!(
            matches!(read_embedded_audit(&foreign), Err(AuditReadError::NotGolem)),
            "non-golem Software SHALL be rejected"
        );
        // A PNG with no metadata at all is also not-golem.
        let bare = encode_png_with_chunks(&[]);
        assert!(matches!(
            read_embedded_audit(&bare),
            Err(AuditReadError::NotGolem)
        ));
        // A golem PNG missing the audit chunk is a distinct error.
        let no_audit = encode_png_with_chunks(&[("Software", "Golem")]);
        assert!(matches!(
            read_embedded_audit(&no_audit),
            Err(AuditReadError::MissingAudit)
        ));
        // Non-PNG bytes fail cleanly.
        assert!(matches!(
            read_embedded_audit(b"not a png"),
            Err(AuditReadError::NotPng(_))
        ));
    }

    #[test]
    fn marker_chip_width_matches_glyph_geometry() {
        // "1" at scale 2: glyph 5*2 wide + 2*2 padding = 14.
        assert_eq!(marker_chip_width(1, 2), crate::glyph::text_width("1", 2) + 4);
        // Two-digit marker is wider.
        assert!(marker_chip_width(11, 2) > marker_chip_width(1, 2));
    }

    fn issue_at(bounds: Rect, sev: Severity) -> A11yIssue {
        A11yIssue {
            check_id: "touch_target_too_small".into(),
            severity: sev,
            message: "m".into(),
            element_type: "button".into(),
            element_label: None,
            element_bounds: Some(bounds),
            related_bounds: Vec::new(),
            measure_bounds: None,
            occlusion: Vec::new(),
            confidence: 1.0,
            detail: None,
        }
    }

    #[test]
    fn annotate_cascades_colliding_markers() {
        // Two findings on the SAME bounds must not stack: the second chip
        // cascades right, painting NEW pixels. Stacking would overdraw the
        // same pixels and leave the coloured-pixel count ~unchanged.
        let bounds = Rect { x: 10, y: 10, width: 30, height: 30 };
        let png = png_of(image::RgbImage::from_pixel(400, 80, image::Rgb([255, 255, 255])));

        let count_colored = |bytes: &[u8]| -> usize {
            let img = image::load_from_memory(bytes).expect("decode png").to_rgba8();
            img.pixels().filter(|p| p.0 != [255, 255, 255, 255]).count()
        };

        let vp = Viewport::new(400, 80);
        let one = annotate_screenshot(&png, &[issue_at(bounds, Severity::Error)], &vp, &test_meta())
            .expect("annotate one");
        let two = annotate_screenshot(
            &png,
            &[
                issue_at(bounds, Severity::Error),
                issue_at(bounds, Severity::Error),
            ],
            &vp,
            &test_meta(),
        )
        .expect("annotate two");
        assert!(
            count_colored(&two) > count_colored(&one),
            "second colliding marker should cascade into new pixels"
        );
    }

    #[test]
    fn occlusion_cell_rect_maps_points_to_3x3() {
        // Points sample the ¼/½/¾ lines → distinct col/row (0/1/2). Cell width
        // is a third of the element; origin at the cell's top-left.
        let b = Bounds::new(0, 0, 90, 90);
        let tuple = |r: Rect| (r.x, r.y, r.width, r.height);
        assert_eq!(tuple(occlusion_cell_rect(&b, 15, 15)), (0, 0, 30, 30), "¼,¼ → top-left");
        assert_eq!(tuple(occlusion_cell_rect(&b, 45, 45)), (30, 30, 30, 30), "½,½ → centre");
        assert_eq!(tuple(occlusion_cell_rect(&b, 75, 15)), (60, 0, 30, 30), "¾,¼ → top-right");
    }

    fn colored_count(bytes: &[u8]) -> usize {
        let img = image::load_from_memory(bytes).expect("decode png").to_rgba8();
        img.pixels().filter(|p| p.0 != [255, 255, 255, 255]).count()
    }

    #[test]
    fn annotate_draws_occlusion_minimap() {
        // occluded_element with a mix of covered + reachable sample cells must
        // render a valid same-size PNG with a drawn mini-map (colored pixels),
        // exercising draw_occlusion_minimap without panicking.
        let png = png_of(image::RgbImage::from_pixel(400, 400, image::Rgb([255, 255, 255])));
        let b = Rect { x: 40, y: 40, width: 240, height: 240 };
        let cell = |cx: i32, cy: i32, reachable: bool| golem_events::OcclusionCell {
            bounds: occlusion_cell_rect(&Bounds::new(b.x, b.y, b.width, b.height), cx, cy),
            reachable,
        };
        let mut iss = issue_at(b, Severity::Warning);
        iss.check_id = "occluded_element".into();
        iss.occlusion = vec![
            cell(b.x + 30, b.y + 30, false), // top-left covered
            cell(b.x + 120, b.y + 120, false), // centre covered
            cell(b.x + 120, b.y + 210, true), // bottom reachable
        ];
        let out = annotate_screenshot(&png, &[iss], &Viewport::new(400, 400), &test_meta())
            .expect("annotate occluded");
        let decoded = image::load_from_memory(&out).expect("valid png");
        assert_eq!((decoded.width(), decoded.height()), (400, 400));
        assert!(colored_count(&out) > 0, "mini-map should draw colored pixels");
    }

    #[test]
    fn annotate_draws_dimension_line_and_connector() {
        // text_too_small with a measure_bounds line exercises the dimension-line
        // + arrowhead helpers; a duplicate_labels finding with related_bounds
        // exercises the dashed connector. Both must render a valid PNG.
        let png = png_of(image::RgbImage::from_pixel(400, 200, image::Rgb([255, 255, 255])));
        let vp = Viewport::new(400, 200);

        let mut txt = issue_at(Rect { x: 20, y: 20, width: 80, height: 40 }, Severity::Warning);
        txt.check_id = "text_too_small".into();
        txt.measure_bounds = Some(Rect { x: 20, y: 30, width: 80, height: 12 });
        txt.detail = Some("11dp".into());
        let out = annotate_screenshot(&png, &[txt], &vp, &test_meta()).expect("annotate dim");
        assert!(colored_count(&out) > 0, "dimension line should draw");

        let mut dup = issue_at(Rect { x: 20, y: 20, width: 60, height: 40 }, Severity::Warning);
        dup.check_id = "duplicate_labels".into();
        dup.related_bounds = vec![Rect { x: 200, y: 120, width: 60, height: 40 }];
        let out = annotate_screenshot(&png, &[dup], &vp, &test_meta()).expect("annotate dup");
        let decoded = image::load_from_memory(&out).expect("valid png");
        assert_eq!((decoded.width(), decoded.height()), (400, 200));
        assert!(colored_count(&out) > 0, "connector + rects should draw");
    }
}
