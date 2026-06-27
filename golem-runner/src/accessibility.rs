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
            min_text_size_dp: if level == A11yLevel::Strict { 12.0 } else { 10.0 },
            contrast_warn_aaa: level == A11yLevel::Strict,
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
        if is_actionable(node) {
            check_touch_target(node, config, out);
            check_missing_label(node, out);
        }
        check_text_size_box(node, config, out);
    }

    // Sibling-group checks over this node's visible, enabled, actionable
    // children (skipped entirely inside a disabled subtree).
    if !disabled {
        let actionable_siblings: Vec<&Element> = node
            .children
            .iter()
            .filter(|c| is_visible(c, viewport) && is_actionable(c) && !is_disabled_control(c))
            .collect();
        check_duplicate_labels(&actionable_siblings, out);
        check_overlapping(&actionable_siblings, out);
    }

    for child in &node.children {
        walk(child, viewport, config, disabled, out);
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

/// Build a deterministic (confidence 1.0) finding. Heuristic checks
/// (contrast) construct `A11yIssue` directly with a scored confidence.
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
        confidence: 1.0,
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

/// `text_too_small` — a text element whose *box* is already shorter than the
/// minimum dp height. Since glyph height ≤ box height, this is certain (no
/// screenshot, no false positives); padding only makes it conservative, so it
/// misses small text inside a tall padded box (an acceptable false negative —
/// the pixel-based glyph-height refinement for that case is a follow-up).
fn check_text_size_box(node: &Element, config: &A11yConfig, out: &mut Vec<A11yIssue>) {
    if node.text.as_deref().is_none_or(|t| t.is_empty()) {
        return;
    }
    let b = node.effective_bounds();
    if b.height <= 0 {
        return;
    }
    let height_dp = b.height as f64 / config.density;
    if height_dp < config.min_text_size_dp {
        out.push(issue(
            "text_too_small",
            Severity::Warning,
            format!(
                "{} is {:.0}dp tall — text below {:.0}dp minimum",
                node.element_type, height_dp, config.min_text_size_dp
            ),
            node,
            b,
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

/// `low_contrast` over the screenshot, with a per-finding confidence. The
/// points→pixels factor is derived from the screenshot vs the viewport (no
/// reliance on a device-reported backing scale). Text-size is handled by the
/// certain box-height check, not here.
pub fn check_contrast(
    screenshot_png: &[u8],
    root: &Element,
    viewport: &Viewport,
    config: &A11yConfig,
) -> Vec<A11yIssue> {
    let mut issues = Vec::new();
    let Ok(img) = image::load_from_memory(screenshot_png) else {
        return issues;
    };
    let img = img.to_rgb8();
    // Derive points→pixels from the screenshot itself (width ÷ viewport
    // width): 3.0 on iOS Retina, 1.0 on Android — robust without relying on
    // a device-reported backing scale (which iOS leaves unset).
    let sf = screenshot_scale(img.width(), viewport.width);

    let mut texts = Vec::new();
    visible_text_elements(root, viewport, false, &mut texts);

    for el in texts {
        let b = el.effective_bounds();
        let (pixels, w) = crop_pixels(
            &img,
            (b.x as f64 * sf).round() as i32,
            (b.y as f64 * sf).round() as i32,
            (b.width as f64 * sf).round() as i32,
            (b.height as f64 * sf).round() as i32,
        );
        let Some((bg, fg, band_px)) = extract_text_colors(&pixels, w) else {
            continue; // no readable text / too complex — undetermined
        };
        let ratio = wcag_contrast_ratio(bg, fg);
        // Glyph-band height in dp drives the large-text threshold (≥16dp →
        // 3:1). band_px is the contrast check's own measurement, used only
        // here — NOT for text_too_small (that's the certain box-height check).
        let large = (band_px as f64 / sf) >= 16.0;
        // Cleaner region (text + bg = 2 clusters) → higher measurement
        // confidence than a busy 3-cluster region.
        let cleanliness = if significant_cluster_count(&pixels) <= 2 {
            1.0
        } else {
            0.6
        };

        let (severity, threshold, standard) = if ratio < if large { 3.0 } else { 4.5 } {
            (Severity::Error, if large { 3.0 } else { 4.5 }, "AA")
        } else if config.contrast_warn_aaa && ratio < if large { 4.5 } else { 7.0 } {
            (Severity::Warning, if large { 4.5 } else { 7.0 }, "AAA")
        } else {
            continue; // meets the applicable threshold
        };
        // Confidence: how far below threshold (a clear fail is more certain
        // than a borderline one), scaled by region cleanliness.
        let margin = ((threshold - ratio) / threshold).clamp(0.0, 1.0) as f32;
        let confidence = (cleanliness * (0.6 + 0.4 * margin)).clamp(0.0, 1.0);
        issues.push(A11yIssue {
            check_id: "low_contrast".into(),
            severity,
            message: format!(
                "{} text contrast {:.1}:1 below {:.1}:1 (WCAG {standard})",
                el.element_type, ratio, threshold
            ),
            element_type: el.element_type.clone(),
            element_label: label_of(el),
            element_bounds: Some(rect_of(b)),
            confidence,
        });
    }
    issues
}

/// Draw numbered-by-order rectangles for each issue onto the screenshot
/// (orange for warnings first, red for errors on top so red stays visible
/// where they overlap) and re-encode as PNG. `scale_factor` maps bounds to
/// screenshot pixels.
pub fn annotate_screenshot(
    screenshot_png: &[u8],
    issues: &[A11yIssue],
    viewport_width: i32,
) -> anyhow::Result<Vec<u8>> {
    use image::Rgba;
    use imageproc::drawing::draw_hollow_rect_mut;
    use imageproc::rect::Rect as IpRect;

    let mut img = image::load_from_memory(screenshot_png)?.to_rgba8();
    let sf = screenshot_scale(img.width(), viewport_width);
    let red = Rgba([220u8, 40, 40, 255]);
    let orange = Rgba([240u8, 150, 30, 255]);

    // Warnings first, errors last → red drawn on top.
    for pass in [Severity::Warning, Severity::Error] {
        let color = if pass == Severity::Error { red } else { orange };
        for issue in issues.iter().filter(|i| i.severity == pass) {
            let Some(b) = issue.element_bounds else {
                continue;
            };
            let x = (b.x as f64 * sf).round() as i32;
            let y = (b.y as f64 * sf).round() as i32;
            let w = (b.width as f64 * sf).round().max(1.0) as u32;
            let h = (b.height as f64 * sf).round().max(1.0) as u32;
            // 2px stroke for visibility.
            for inset in 0..2 {
                let rx = x + inset;
                let ry = y + inset;
                let rw = w.saturating_sub((inset * 2) as u32).max(1);
                let rh = h.saturating_sub((inset * 2) as u32).max(1);
                draw_hollow_rect_mut(&mut img, IpRect::at(rx, ry).of_size(rw, rh), color);
            }
        }
    }

    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
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
    fn check_contrast_flags_low_contrast_text() {
        // White canvas, a light-grey text band (rows 45..55) → low ratio.
        let mut img = image::RgbImage::from_pixel(100, 100, image::Rgb([255, 255, 255]));
        for y in 45..55u32 {
            for x in 10..90u32 {
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
            confidence: 1.0,
        };
        let out = annotate_screenshot(&png, &[issue], 60).expect("annotate ok");
        let decoded = image::load_from_memory(&out).expect("valid png");
        assert_eq!((decoded.width(), decoded.height()), (60, 40));
    }
}
