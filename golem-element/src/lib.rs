//! The element tree: golem's shared model of a device UI screen.
//!
//! A companion (iOS/Android agent) walks the on-device accessibility tree (or,
//! for webviews, the merged DOM) and serializes it into [`Element`] nodes,
//! which the host deserializes over the wire. From there, [`selector`] resolves
//! `.test.toml` element selectors against the tree, [`glob`] backs the
//! text/label glob matching those selectors use, and [`filter_viewport`]
//! reduces a tree to only what's currently on screen — the visible tree that,
//! per golem's core invariant, is the only thing a test may judge against.
//! [`Element::compute_native_hit_points`] and [`Element::tap_point`] add
//! occlusion-aware tap routing on top of the raw geometry so a tap lands on
//! what a human would actually hit.

/// Glob-style (`*`/`?`) pattern matching used by selector `text`/
/// `accessibility_label` matching.
pub mod glob;
/// Resolving `.test.toml` element selectors — including relational anchors
/// (`below`, `contains`, ...) and observable traits — against an [`Element`] tree.
pub mod selector;

use serde::{Deserialize, Serialize};

/// A single node in a device UI tree, as reported by a companion.
///
/// This is golem's wire format and in-memory model for both native
/// accessibility trees (iOS/Android) and webview DOM subtrees (merged in by
/// the companion so a page's content appears as ordinary `Element` children).
/// Fields are deliberately permissive (most are `#[serde(default)]`) since
/// companions send sparse payloads and native/webview sources don't populate
/// every field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub element_type: String,
    pub text: Option<String>,
    pub accessibility_label: Option<String>,
    pub placeholder: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub checked: bool,
    #[serde(default)]
    pub clickable: bool,
    #[serde(default)]
    pub focused: bool,
    pub bounds: Bounds,
    /// Visible bounds clipped to ancestor containers. Falls back to bounds.
    #[serde(default)]
    pub visible_bounds: Option<Bounds>,
    /// Occlusion hit-test samples within the visible bounds (webview only).
    /// Each point records whether a tap there would actually reach this
    /// element (vs an element painted on top). Canonical order: centre, then
    /// arms, then corners. Empty when not computed (native / non-targetable).
    #[serde(default)]
    pub hit_points: Vec<HitPoint>,
    /// Sibling paint order (Android `getDrawingOrder`): higher = painted
    /// later (on top), capturing elevation/z that raw tree order misses. `None`
    /// on iOS/webview (no per-node signal) → callers fall back to tree order.
    /// Used by the host-side native occlusion hit-test.
    #[serde(default)]
    pub drawing_order: Option<i32>,
    #[serde(default)]
    pub children: Vec<Element>,
}

/// A sampled point (device coords) and whether a tap there reaches the element.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct HitPoint {
    pub x: i32,
    pub y: i32,
    pub hit: bool,
}

/// An axis-aligned rectangle in device coordinates: top-left origin `(x, y)`
/// plus `width`/`height`. Used throughout for both an element's raw `bounds`
/// and its ancestor-clipped `visible_bounds`, and for the [`Viewport`] a tree
/// is filtered against.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Bounds {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub fn center_x(&self) -> i32 {
        self.x + self.width / 2
    }
    pub fn center_y(&self) -> i32 {
        self.y + self.height / 2
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }
    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    /// Intersect this bounds with another, returning the overlapping region.
    /// Returns a zero-area Bounds if there is no overlap.
    pub fn intersect(&self, other: &Bounds) -> Bounds {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right > left && bottom > top {
            Bounds::new(left, top, right - left, bottom - top)
        } else {
            Bounds::new(left, top, 0, 0)
        }
    }

    /// Area of this bounds in square pixels.
    pub fn area(&self) -> i64 {
        self.width as i64 * self.height as i64
    }

    /// True when `(px, py)` falls within these bounds (half-open: left/top
    /// inclusive, right/bottom exclusive).
    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// True when these bounds overlap `other` with non-zero area. Edge-only
    /// contact (touching borders, no interior overlap) is `false`. Distinct
    /// from [`Bounds::intersect`], which returns the overlapping region.
    pub fn intersects(&self, other: &Bounds) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}

impl Element {
    /// Return visible bounds if available, otherwise fall back to full bounds.
    pub fn effective_bounds(&self) -> &Bounds {
        self.visible_bounds.as_ref().unwrap_or(&self.bounds)
    }

    /// The point to tap: the first occlusion-clear sample point (canonical
    /// order: centre → arms → corners), so a tap routes around an occluder
    /// (e.g. a sticky header covering the element's centre). Falls back to the
    /// visible-bounds centre when no hit-test data exists (native / non-target)
    /// or nothing sampled clear (still attempt — hittability is a heuristic).
    pub fn tap_point(&self) -> (i32, i32) {
        if let Some(p) = self.hit_points.iter().find(|p| p.hit) {
            return (p.x, p.y);
        }
        let b = self.effective_bounds();
        (b.center_x(), b.center_y())
    }

    /// Fraction (0.0–1.0) of sampled points that are occlusion-clear, or `None`
    /// when no hit-test was done. 0.0 = fully occluded; <1.0 = partially.
    pub fn hittable_fraction(&self) -> Option<f32> {
        if self.hit_points.is_empty() {
            return None;
        }
        let clear = self.hit_points.iter().filter(|p| p.hit).count();
        Some(clear as f32 / self.hit_points.len() as f32)
    }

    /// Whether the element's centre (the first sample point) is occlusion-clear.
    /// `None` when no hit-test data exists.
    pub fn center_hittable(&self) -> Option<bool> {
        self.hit_points.first().map(|p| p.hit)
    }

    /// Recursively count all elements in the tree, including the root.
    /// Used by the post-launch settle gate to detect when the UI has
    /// finished rendering the first interactive screen.
    pub fn node_count(&self) -> usize {
        1 + self.children.iter().map(Element::node_count).sum::<usize>()
    }

    /// True when this element is a WebView host — iOS `web_view` or Android
    /// `WebView` (the simplified `android.webkit.WebView` class). Its children
    /// are the merged DOM (CDP on Android / WebKit Inspector on iOS).
    pub fn is_webview(&self) -> bool {
        self.element_type == "web_view" || self.element_type.contains("WebView")
    }

    /// If the tree contains a WebView host, return the node count of its
    /// subtree (the WebView node + the merged DOM beneath it); `None` when
    /// there is no WebView (a native screen — webview-readiness is N/A).
    ///
    /// Used by the post-launch settle gate: a WebView present with only a
    /// tiny subtree means the page hasn't hydrated yet (the native a11y tree
    /// settles long before the webview DOM renders), so the gate must keep
    /// waiting rather than let the first action run against an empty page.
    pub fn webview_subtree_count(&self) -> Option<usize> {
        if self.is_webview() {
            return Some(self.node_count());
        }
        self.children
            .iter()
            .find_map(Element::webview_subtree_count)
    }

    /// Canonical occlusion sample points within `b` — centre → arms → corners,
    /// matching the webview sampler: centre always; vertical/horizontal arms
    /// once the dimension exceeds a fingertip; corners only when both exceed a
    /// larger bound. Yields 1, 3, 5, or 9 points.
    fn occlusion_sample_points(b: &Bounds) -> Vec<(i32, i32)> {
        const FINGER: i32 = 44;
        const LARGE: i32 = 88;
        let (cx, cy) = (b.center_x(), b.center_y());
        let mut pts = vec![(cx, cy)];
        if b.height > FINGER {
            pts.push((cx, b.y + b.height / 4));
            pts.push((cx, b.y + b.height * 3 / 4));
        }
        if b.width > FINGER {
            pts.push((b.x + b.width / 4, cy));
            pts.push((b.x + b.width * 3 / 4, cy));
        }
        if b.width > LARGE && b.height > LARGE {
            pts.push((b.x + b.width / 4, b.y + b.height / 4));
            pts.push((b.x + b.width * 3 / 4, b.y + b.height / 4));
            pts.push((b.x + b.width / 4, b.y + b.height * 3 / 4));
            pts.push((b.x + b.width * 3 / 4, b.y + b.height * 3 / 4));
        }
        pts
    }

    /// Populate `hit_points` for native targets (clickable, or text/label-
    /// bearing — anything a selector may resolve to) via a host-side
    /// geometric hit-test — the native analogue of the webview
    /// `elementFromPoint` sampling, so `tap_point()`/occlusion routing work
    /// uniformly across native and webview. A sample point is "hit" when the
    /// target (or a descendant) is the topmost element painted there, and
    /// "occluded" when an unrelated, later-painted element covers it.
    ///
    /// Paint order is sibling `drawing_order` (Android `getDrawingOrder`)
    /// or tree order (iOS / no signal). It is therefore a HEURISTIC —
    /// cross-hierarchy elevation and iOS `zPosition` aren't captured — so a
    /// reported occlusion means "may be occluded", never authoritative. The
    /// tap still routes to a clear point but never blocks on this.
    ///
    /// No-op for nodes already carrying `hit_points` (webview, from the DOM
    /// hit-test). Bounded: O(targets × samples × nodes).
    pub fn compute_native_hit_points(&mut self) {
        let mut meta: Vec<HitMeta> = Vec::new();
        collect_hit_meta(self, &mut meta);
        // Fast out: nothing to do unless some native tap target lacks hit_points.
        if meta.iter().all(|m| !m.target || m.has_hit_points) {
            return;
        }
        let mut paint_rank = vec![0usize; meta.len()];
        let mut next_rank = 0usize;
        assign_paint_rank(0, &meta, &mut paint_rank, &mut next_rank);

        let mut computed: Vec<Option<Vec<HitPoint>>> = vec![None; meta.len()];
        for i in 0..meta.len() {
            if !meta[i].target || meta[i].has_hit_points {
                continue;
            }
            let b = meta[i].bounds;
            if b.width <= 0 || b.height <= 0 {
                continue;
            }
            let hp = Element::occlusion_sample_points(&b)
                .into_iter()
                .map(|(x, y)| {
                    let occluded = meta.iter().enumerate().any(|(j, mj)| {
                        j != i
                            && paint_rank[j] > paint_rank[i]
                            // not a descendant of the target (pre-order range)
                            && !(j > i && j < meta[i].subtree_end)
                            // not a coincident/enclosing wrapper: a real occluder
                            // partially covers the target. A later node with the
                            // SAME-or-larger bounds (e.g. Compose's separate
                            // clickable vs label nodes at identical bounds, or an
                            // ancestor-shaped sibling) isn't occluding — skip it.
                            && !encloses(&mj.bounds, &b)
                            && mj.bounds.contains_point(x, y)
                    });
                    HitPoint {
                        x,
                        y,
                        hit: !occluded,
                    }
                })
                .collect();
            computed[i] = Some(hp);
        }

        let mut idx = 0usize;
        write_hit_points(self, &mut idx, &computed);
    }
}

/// Per-node scratch for the native hit-test, in pre-order.
struct HitMeta {
    bounds: Bounds,
    drawing_order: Option<i32>,
    target: bool,
    has_hit_points: bool,
    /// Exclusive pre-order index just past this node's subtree.
    subtree_end: usize,
    /// Pre-order indices of direct children (natural order).
    children: Vec<usize>,
}

/// True when `outer` fully encloses `inner` (used to skip wrapper/coincident
/// "occluders" — a real occluder only partially covers the target).
fn encloses(outer: &Bounds, inner: &Bounds) -> bool {
    outer.x <= inner.x
        && outer.y <= inner.y
        && outer.right() >= inner.right()
        && outer.bottom() >= inner.bottom()
}

fn collect_hit_meta(el: &Element, out: &mut Vec<HitMeta>) -> usize {
    let my = out.len();
    out.push(HitMeta {
        bounds: *el.effective_bounds(),
        drawing_order: el.drawing_order,
        // A target is anything golem might tap or assert on — clickable, or
        // carrying text / an accessibility label. Mirrors the webview
        // "selectable" set, so occlusion routing applies to whatever a
        // selector resolves to (a label may sit on a node distinct from the
        // clickable wrapper, e.g. a Compose Button's merged semantics node).
        target: el.clickable || el.text.is_some() || el.accessibility_label.is_some(),
        has_hit_points: !el.hit_points.is_empty(),
        subtree_end: 0,
        children: Vec::new(),
    });
    let mut kids = Vec::new();
    for child in &el.children {
        kids.push(collect_hit_meta(child, out));
    }
    out[my].subtree_end = out.len();
    out[my].children = kids;
    my
}

/// Assign paint rank: a node paints before its children, and siblings paint in
/// ascending `drawing_order` (stable → natural order when the signal is absent).
fn assign_paint_rank(i: usize, meta: &[HitMeta], paint_rank: &mut [usize], next: &mut usize) {
    paint_rank[i] = *next;
    *next += 1;
    let mut kids = meta[i].children.clone();
    kids.sort_by_key(|&c| meta[c].drawing_order.unwrap_or(0));
    for c in kids {
        assign_paint_rank(c, meta, paint_rank, next);
    }
}

fn write_hit_points(el: &mut Element, idx: &mut usize, computed: &[Option<Vec<HitPoint>>]) {
    let my = *idx;
    *idx += 1;
    if let Some(hp) = &computed[my] {
        el.hit_points = hp.clone();
    }
    for child in &mut el.children {
        write_hit_points(child, idx, computed);
    }
}

/// A selector match: the matched [`Element`] plus the device-coordinate point
/// golem will actually tap, per [`Element::tap_point`] (occlusion-aware, with
/// a plain centre-of-bounds fallback). This is the return type of
/// [`selector::find_elements`] and what the runner acts on.
#[derive(Debug, Clone)]
pub struct FindResult {
    pub element: Element,
    pub tap_x: i32,
    pub tap_y: i32,
}

/// Screen viewport for filtering visible elements.
/// Origin (x, y) handles windows not at (0,0) like alert dialogs.
/// Width and height are dimensions, not absolute coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Viewport {
    /// A viewport at the origin `(0, 0)` with the given dimensions — the
    /// common case for a full-screen root with no offset window.
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// Detect viewport from the root element's bounds.
    pub fn from_root(root: &Element) -> Self {
        Self {
            x: root.bounds.x,
            y: root.bounds.y,
            width: root.bounds.width,
            height: root.bounds.height,
        }
    }

    /// Check if an element's bounds intersect this viewport (partially or fully visible).
    pub fn contains(&self, bounds: &Bounds) -> bool {
        let right = self.x + self.width;
        let bottom = self.y + self.height;
        bounds.x + bounds.width > self.x
            && bounds.x < right
            && bounds.y + bounds.height > self.y
            && bounds.y < bottom
    }
}

/// Collect all elements whose bounds intersect the viewport into a flat list.
///
/// Uses absolute positions only — an element at y=389 is visible regardless
/// of where its parent is in the tree. This correctly handles fixed-position
/// overlays, scrolled content, and dynamic DOM insertions.
pub fn filter_viewport(root: &Element, viewport: &Viewport) -> Element {
    let mut visible = Vec::new();
    // Collect visible descendants (skip the root — it's the container).
    for child in &root.children {
        collect_visible(child, viewport, &mut visible);
    }
    // Return the root with visible elements as flat children.
    Element {
        element_type: root.element_type.clone(),
        text: root.text.clone(),
        accessibility_label: root.accessibility_label.clone(),
        placeholder: root.placeholder.clone(),
        enabled: root.enabled,
        checked: root.checked,
        clickable: root.clickable,
        focused: root.focused,
        bounds: root.bounds,
        visible_bounds: root.visible_bounds,
        hit_points: vec![],
        drawing_order: None,
        children: visible,
    }
}

fn collect_visible(element: &Element, viewport: &Viewport, out: &mut Vec<Element>) {
    if viewport.contains(element.effective_bounds()) {
        let mut leaf = element.clone();
        leaf.children = Vec::new();
        out.push(leaf);
    }
    for child in &element.children {
        collect_visible(child, viewport, out);
    }
}

impl FindResult {
    pub fn new(element: Element) -> Self {
        // Occlusion-aware: tap the first clear sample point (routes around a
        // sticky/overlapping element), falling back to the visible centre.
        let (tap_x, tap_y) = element.tap_point();
        Self {
            element,
            tap_x,
            tap_y,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bounds(x: i32, y: i32, width: i32, height: i32) -> Bounds {
        Bounds::new(x, y, width, height)
    }

    fn make_element(element_type: &str, bounds: Bounds) -> Element {
        Element {
            element_type: element_type.to_string(),
            text: None,
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds,
            visible_bounds: None,
            hit_points: vec![],
            drawing_order: None,
            children: Vec::new(),
        }
    }

    // ── Bounds::intersects / is_offscreen (a11y geometry) ───────────

    #[test]
    fn intersects_overlapping_partial() {
        assert!(make_bounds(0, 0, 100, 100).intersects(&make_bounds(50, 50, 100, 100)));
    }

    #[test]
    fn intersects_fully_contained() {
        assert!(make_bounds(0, 0, 100, 100).intersects(&make_bounds(10, 10, 20, 20)));
        assert!(make_bounds(10, 10, 20, 20).intersects(&make_bounds(0, 0, 100, 100)));
    }

    #[test]
    fn intersects_edge_contact_is_false() {
        // Right edge of A (x=100) touches left edge of B (x=100): no interior overlap.
        assert!(!make_bounds(0, 0, 100, 100).intersects(&make_bounds(100, 0, 100, 100)));
    }

    #[test]
    fn intersects_disjoint_is_false() {
        assert!(!make_bounds(0, 0, 50, 50).intersects(&make_bounds(200, 200, 50, 50)));
    }

    // ── native occlusion hit-test ───────────────────────────────────

    /// root container (non-target) holding a target button `B` and an
    /// `overlay`. `B` spans (0,0,200,100); `overlay` (added per `overlay_first`)
    /// covers x<150 — i.e. B's centre and left, leaving the right edge clear.
    fn occlusion_tree(overlay_first: bool, b_order: Option<i32>, o_order: Option<i32>) -> Element {
        let mut root = make_element("Root", make_bounds(0, 0, 400, 400));
        root.clickable = false; // container, not a tap target
        let mut b = make_element("Button", make_bounds(0, 0, 200, 100));
        b.drawing_order = b_order;
        let mut overlay = make_element("Overlay", make_bounds(0, 0, 150, 200));
        overlay.drawing_order = o_order;
        if overlay_first {
            root.children = vec![overlay, b];
        } else {
            root.children = vec![b, overlay];
        }
        root
    }

    fn find_button(root: &Element) -> &Element {
        root.children
            .iter()
            .find(|c| c.element_type == "Button")
            .expect("button")
    }

    #[test]
    fn native_hit_test_routes_around_a_later_painted_overlay() {
        // Overlay is a later sibling → paints on top → occludes B's centre.
        let mut tree = occlusion_tree(false, None, None);
        tree.compute_native_hit_points();
        let b = find_button(&tree);
        assert_eq!(
            b.center_hittable(),
            Some(false),
            "centre is under the overlay"
        );
        assert!(
            b.hittable_fraction().is_some_and(|f| f > 0.0 && f < 1.0),
            "partially occluded: some points clear, some not"
        );
        // Routes to a clear point on the uncovered right edge (x >= 150).
        let (tx, _ty) = b.tap_point();
        assert!(
            tx >= 150,
            "tap SHALL route to the clear right edge, got x={tx}"
        );
    }

    #[test]
    fn native_hit_test_drawing_order_overrides_tree_order() {
        // Same geometry, but B has a HIGHER drawing_order than the overlay →
        // B paints on top → NOT occluded, even though the overlay is a later
        // sibling in tree order.
        let mut tree = occlusion_tree(false, Some(1), Some(0));
        tree.compute_native_hit_points();
        let b = find_button(&tree);
        assert_eq!(
            b.center_hittable(),
            Some(true),
            "higher drawing_order SHALL paint B above the overlay (no occlusion)"
        );
    }

    #[test]
    fn native_hit_test_unoccluded_target_is_all_clear() {
        let mut root = make_element("Root", make_bounds(0, 0, 400, 400));
        root.clickable = false;
        root.children = vec![make_element("Button", make_bounds(10, 10, 120, 120))];
        root.compute_native_hit_points();
        let b = find_button(&root);
        assert_eq!(
            b.hittable_fraction(),
            Some(1.0),
            "lone target SHALL be fully clear"
        );
    }

    #[test]
    fn native_hit_test_preserves_existing_webview_hit_points() {
        let mut root = make_element("Root", make_bounds(0, 0, 400, 400));
        root.clickable = false;
        let mut wv = make_element("div", make_bounds(0, 0, 50, 50));
        wv.hit_points = vec![HitPoint {
            x: 1,
            y: 2,
            hit: true,
        }];
        root.children = vec![wv];
        root.compute_native_hit_points();
        assert_eq!(
            root.children[0].hit_points,
            vec![HitPoint {
                x: 1,
                y: 2,
                hit: true
            }],
            "nodes that already carry hit_points (webview) SHALL not be recomputed"
        );
    }

    #[test]
    fn tap_point_falls_back_to_center_without_hit_points() {
        let e = make_element("Button", make_bounds(0, 0, 100, 40));
        assert_eq!(
            e.tap_point(),
            (50, 20),
            "no hit_points → visible-bounds centre"
        );
        assert_eq!(e.hittable_fraction(), None);
        assert_eq!(e.center_hittable(), None);
    }

    #[test]
    fn tap_point_uses_first_clear_sample() {
        let mut e = make_element("Button", make_bounds(0, 0, 100, 40));
        // Centre occluded, an arm clear → tap the clear arm, not the centre.
        e.hit_points = vec![
            HitPoint {
                x: 50,
                y: 20,
                hit: false,
            }, // centre (canonical first)
            HitPoint {
                x: 50,
                y: 10,
                hit: false,
            }, // top
            HitPoint {
                x: 50,
                y: 30,
                hit: true,
            }, // bottom — first clear
        ];
        assert_eq!(
            e.tap_point(),
            (50, 30),
            "SHALL route to the first clear sample"
        );
        assert_eq!(e.center_hittable(), Some(false));
        assert!(
            (e.hittable_fraction()
                .expect("hittable_fraction() SHALL succeed")
                - 1.0 / 3.0)
                .abs()
                < 1e-6
        );
    }

    #[test]
    fn tap_point_clear_center_wins() {
        let mut e = make_element("Button", make_bounds(0, 0, 100, 40));
        e.hit_points = vec![
            HitPoint {
                x: 50,
                y: 20,
                hit: true,
            },
            HitPoint {
                x: 50,
                y: 30,
                hit: true,
            },
        ];
        assert_eq!(
            e.tap_point(),
            (50, 20),
            "clear centre is preferred (canonical first)"
        );
        assert_eq!(e.center_hittable(), Some(true));
        assert_eq!(e.hittable_fraction(), Some(1.0));
    }

    #[test]
    fn tap_point_fully_occluded_falls_back_to_center() {
        let mut e = make_element("Button", make_bounds(0, 0, 100, 40));
        e.hit_points = vec![HitPoint {
            x: 50,
            y: 20,
            hit: false,
        }];
        // No clear point → still attempt at centre (heuristic, never blocks).
        assert_eq!(e.tap_point(), (50, 20));
        assert_eq!(e.hittable_fraction(), Some(0.0));
    }

    #[test]
    fn bounds_center_x_and_center_y() {
        let b = make_bounds(10, 20, 100, 50);
        assert_eq!(b.center_x(), 60);
        assert_eq!(b.center_y(), 45);
    }

    #[test]
    fn bounds_bottom_and_right() {
        let b = make_bounds(10, 20, 100, 50);
        assert_eq!(b.bottom(), 70);
        assert_eq!(b.right(), 110);
    }

    #[test]
    fn find_result_computes_tap_coordinates() {
        let elem = make_element("Button", make_bounds(0, 0, 200, 80));
        let result = FindResult::new(elem);
        assert_eq!(result.tap_x, 100);
        assert_eq!(result.tap_y, 40);
    }

    #[test]
    fn element_serialization_round_trip() {
        let elem = Element {
            element_type: "TextField".to_string(),
            text: Some("hello".to_string()),
            accessibility_label: Some("input-1".to_string()),
            placeholder: Some("Enter name".to_string()),
            enabled: true,
            checked: false,
            clickable: true,
            focused: true,
            bounds: make_bounds(5, 10, 300, 44),
            visible_bounds: None,
            hit_points: vec![],
            drawing_order: None,
            children: Vec::new(),
        };

        let json = serde_json::to_string(&elem).expect("serialization failed");
        let deserialized: Element = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.element_type, "TextField");
        assert_eq!(deserialized.text.as_deref(), Some("hello"));
        assert_eq!(deserialized.accessibility_label.as_deref(), Some("input-1"));
        assert_eq!(deserialized.placeholder.as_deref(), Some("Enter name"));
        assert!(deserialized.enabled);
        assert!(!deserialized.checked);
        assert!(deserialized.clickable);
        assert!(deserialized.focused);
        assert_eq!(deserialized.bounds, make_bounds(5, 10, 300, 44));
        assert!(deserialized.children.is_empty());
    }

    #[test]
    fn element_with_children_serializes_and_deserializes() {
        let child = make_element("Label", make_bounds(10, 10, 80, 20));
        let mut parent = make_element("View", make_bounds(0, 0, 100, 100));
        parent.children.push(child);

        let json = serde_json::to_string(&parent).expect("serialization failed");
        let deserialized: Element = serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.children.len(), 1);
        assert_eq!(deserialized.children[0].element_type, "Label");
        assert_eq!(deserialized.children[0].bounds, make_bounds(10, 10, 80, 20));
    }

    #[test]
    fn bounds_with_zero_dimensions() {
        let b = make_bounds(50, 50, 0, 0);
        assert_eq!(b.center_x(), 50);
        assert_eq!(b.center_y(), 50);
        assert_eq!(b.bottom(), 50);
        assert_eq!(b.right(), 50);
    }

    // ── Viewport filtering tests ──────────────────────────────────────

    #[test]
    fn viewport_contains_fully_visible() {
        let vp = Viewport::new(375, 812);
        assert!(vp.contains(&Bounds::new(10, 10, 100, 44)));
    }

    #[test]
    fn viewport_contains_partially_visible() {
        let vp = Viewport::new(375, 812);
        // Element straddles bottom edge
        assert!(vp.contains(&Bounds::new(10, 790, 100, 44)));
    }

    #[test]
    fn viewport_excludes_fully_below() {
        let vp = Viewport::new(375, 812);
        assert!(!vp.contains(&Bounds::new(10, 900, 100, 44)));
    }

    #[test]
    fn viewport_excludes_fully_above() {
        let vp = Viewport::new(375, 812);
        assert!(!vp.contains(&Bounds::new(10, -100, 100, 44)));
    }

    #[test]
    fn filter_viewport_keeps_visible_removes_offscreen() {
        let vp = Viewport::new(375, 812);
        let mut root = make_element("View", make_bounds(0, 0, 375, 2000));
        root.children
            .push(make_element("Button", make_bounds(10, 100, 100, 44))); // visible
        root.children
            .push(make_element("Button", make_bounds(10, 900, 100, 44))); // offscreen
        root.children
            .push(make_element("Button", make_bounds(10, 400, 100, 44))); // visible

        let filtered = filter_viewport(&root, &vp);
        assert_eq!(
            filtered.children.len(),
            2,
            "SHALL keep 2 visible, remove 1 offscreen"
        );
    }

    #[test]
    fn filter_viewport_from_root_uses_root_bounds() {
        let root = make_element("Window", make_bounds(0, 0, 390, 844));
        let vp = Viewport::from_root(&root);
        assert_eq!(vp.width, 390);
        assert_eq!(vp.height, 844);
    }

    // ── Bounds::intersect tests ──────────────────────────────────────

    #[test]
    fn bounds_intersect_overlapping() {
        let a = make_bounds(0, 0, 100, 100);
        let b = make_bounds(50, 50, 100, 100);
        let i = a.intersect(&b);
        assert_eq!(i, make_bounds(50, 50, 50, 50));
    }

    #[test]
    fn bounds_intersect_no_overlap() {
        let a = make_bounds(0, 0, 50, 50);
        let b = make_bounds(100, 100, 50, 50);
        let i = a.intersect(&b);
        assert_eq!(i.area(), 0, "SHALL have zero area when no overlap");
    }

    #[test]
    fn bounds_intersect_contained() {
        let outer = make_bounds(0, 0, 200, 200);
        let inner = make_bounds(10, 10, 50, 50);
        let i = outer.intersect(&inner);
        assert_eq!(i, make_bounds(10, 10, 50, 50));
    }

    #[test]
    fn bounds_area() {
        assert_eq!(make_bounds(0, 0, 100, 50).area(), 5000);
        assert_eq!(make_bounds(0, 0, 0, 50).area(), 0);
    }

    // ── effective_bounds tests ────────────────────────────────────────

    #[test]
    fn effective_bounds_falls_back_to_bounds() {
        let elem = make_element("Button", make_bounds(10, 20, 100, 40));
        assert_eq!(elem.effective_bounds(), &make_bounds(10, 20, 100, 40));
    }

    #[test]
    fn effective_bounds_returns_visible_bounds_when_set() {
        let mut elem = make_element("Button", make_bounds(10, 20, 100, 40));
        elem.visible_bounds = Some(make_bounds(10, 20, 50, 40));
        assert_eq!(elem.effective_bounds(), &make_bounds(10, 20, 50, 40));
    }

    // ── filter_viewport uses effective_bounds ─────────────────────────

    #[test]
    fn filter_viewport_uses_effective_bounds() {
        let vp = Viewport::new(375, 812);
        let mut root = make_element("View", make_bounds(0, 0, 375, 2000));
        // Element with bounds off-screen but visible_bounds on-screen
        let mut elem = make_element("Button", make_bounds(10, 900, 100, 44));
        elem.visible_bounds = Some(make_bounds(10, 100, 100, 44));
        root.children.push(elem);
        // Element with bounds on-screen but visible_bounds off-screen (fully clipped)
        let mut elem2 = make_element("Button", make_bounds(10, 100, 100, 44));
        elem2.visible_bounds = Some(make_bounds(10, 900, 100, 44));
        root.children.push(elem2);

        let filtered = filter_viewport(&root, &vp);
        assert_eq!(
            filtered.children.len(),
            1,
            "SHALL use effective_bounds (visible_bounds) for viewport filtering"
        );
    }

    // ── node_count tests ──────────────────────────────────────────────

    // 1. A lone element with no children counts as 1 (the root itself).
    #[test]
    fn node_count_single_element_is_one() {
        let elem = make_element("View", make_bounds(0, 0, 10, 10));
        assert_eq!(elem.node_count(), 1, "lone element SHALL count as 1");
    }

    // 2. node_count recurses through nested children and sums the whole tree.
    #[test]
    fn node_count_counts_nested_tree() {
        let mut root = make_element("View", make_bounds(0, 0, 100, 100));
        let mut mid = make_element("Group", make_bounds(0, 0, 50, 50));
        mid.children
            .push(make_element("Leaf", make_bounds(0, 0, 10, 10)));
        mid.children
            .push(make_element("Leaf", make_bounds(0, 0, 10, 10)));
        root.children.push(mid);
        root.children
            .push(make_element("Sibling", make_bounds(0, 0, 10, 10)));
        // root(1) + mid(1) + 2 leaves(2) + sibling(1) = 5
        assert_eq!(root.node_count(), 5, "node_count SHALL sum the whole tree");
    }

    // ── webview detection tests ───────────────────────────────────────

    // No WebView anywhere → None (native screen, readiness N/A).
    #[test]
    fn webview_subtree_count_none_when_no_webview() {
        let mut root = make_element("View", make_bounds(0, 0, 100, 100));
        root.children
            .push(make_element("Button", make_bounds(0, 0, 10, 10)));
        assert_eq!(
            root.webview_subtree_count(),
            None,
            "a native tree SHALL report no webview subtree"
        );
    }

    // iOS `web_view` and Android `WebView` are both recognised; the count is
    // the webview node plus its merged DOM descendants.
    #[test]
    fn webview_subtree_count_counts_dom_subtree() {
        for ty in ["web_view", "WebView"] {
            let mut root = make_element("View", make_bounds(0, 0, 100, 100));
            let mut wv = make_element(ty, make_bounds(0, 0, 100, 100));
            let mut dom = make_element("body", make_bounds(0, 0, 100, 100));
            dom.children
                .push(make_element("div", make_bounds(0, 0, 10, 10)));
            wv.children.push(dom);
            root.children.push(wv);
            // wv(1) + body(1) + div(1) = 3
            assert_eq!(
                root.webview_subtree_count(),
                Some(3),
                "{ty} SHALL be detected and its DOM subtree counted"
            );
        }
    }

    // An unhydrated WebView (no DOM children yet) counts as just itself — the
    // signal the settle gate uses to keep waiting.
    #[test]
    fn webview_subtree_count_is_one_when_dom_empty() {
        let mut root = make_element("View", make_bounds(0, 0, 100, 100));
        root.children
            .push(make_element("web_view", make_bounds(0, 0, 100, 100)));
        assert_eq!(
            root.webview_subtree_count(),
            Some(1),
            "an empty WebView SHALL count as 1 (not hydrated)"
        );
    }

    // ── Viewport tests ────────────────────────────────────────────────

    // 3. Viewport::new sets the origin to (0, 0) and keeps the dimensions.
    #[test]
    fn viewport_new_has_zero_origin() {
        let vp = Viewport::new(375, 812);
        assert_eq!(vp.x, 0, "Viewport::new SHALL place origin x at 0");
        assert_eq!(vp.y, 0, "Viewport::new SHALL place origin y at 0");
        assert_eq!(vp.width, 375);
        assert_eq!(vp.height, 812);
    }

    // 4. from_root carries a non-zero origin (e.g. an alert dialog window).
    #[test]
    fn viewport_from_root_preserves_nonzero_origin() {
        let root = make_element("Alert", make_bounds(40, 200, 300, 400));
        let vp = Viewport::from_root(&root);
        assert_eq!(vp.x, 40, "from_root SHALL preserve origin x");
        assert_eq!(vp.y, 200, "from_root SHALL preserve origin y");
        assert_eq!(vp.width, 300);
        assert_eq!(vp.height, 400);
    }

    // 5. contains respects a non-zero viewport origin: an element to the left
    //    of the origin is excluded even though its absolute x is positive.
    #[test]
    fn viewport_contains_respects_nonzero_origin() {
        let root = make_element("Alert", make_bounds(100, 100, 200, 200));
        let vp = Viewport::from_root(&root);
        // Fully inside the shifted window.
        assert!(
            vp.contains(&Bounds::new(120, 120, 50, 50)),
            "element inside shifted window SHALL be contained"
        );
        // Left of the window's origin (x+width=90 <= 100) — excluded.
        assert!(
            !vp.contains(&Bounds::new(40, 120, 50, 50)),
            "element left of shifted origin SHALL be excluded"
        );
    }

    // 6. contains treats edge-touching as non-overlap (strict inequality):
    //    an element whose right edge equals the origin x is excluded.
    #[test]
    fn viewport_contains_edge_touch_is_excluded() {
        let vp = Viewport::new(375, 812);
        // right edge x+width = 0 == viewport.x = 0 -> not > x -> excluded.
        assert!(
            !vp.contains(&Bounds::new(-100, 10, 100, 44)),
            "element whose right edge touches origin SHALL be excluded"
        );
        // bottom edge y+height = 0 == viewport.y = 0 -> excluded.
        assert!(
            !vp.contains(&Bounds::new(10, -44, 100, 44)),
            "element whose bottom edge touches origin SHALL be excluded"
        );
    }

    // ── Bounds::intersect edge cases ──────────────────────────────────

    // 7. Edge-touching bounds (right of A == left of B) have zero area.
    #[test]
    fn bounds_intersect_touching_edges_zero_area() {
        let a = make_bounds(0, 0, 50, 50);
        let b = make_bounds(50, 0, 50, 50);
        let i = a.intersect(&b);
        assert_eq!(
            i.area(),
            0,
            "edge-touching bounds SHALL produce zero-area intersection"
        );
    }

    // 8. Identical bounds intersect to themselves.
    #[test]
    fn bounds_intersect_identical() {
        let a = make_bounds(10, 20, 100, 40);
        assert_eq!(
            a.intersect(&a),
            make_bounds(10, 20, 100, 40),
            "self-intersection SHALL equal the bounds"
        );
    }

    // 9. Area handles a tall, narrow region beyond i32 overflow range,
    //    confirming the i64 widening cast.
    #[test]
    fn bounds_area_widens_to_i64() {
        let big = make_bounds(0, 0, 100_000, 100_000);
        assert_eq!(
            big.area(),
            10_000_000_000_i64,
            "area SHALL widen to i64 and not overflow"
        );
    }

    // ── FindResult uses effective_bounds ──────────────────────────────

    // 10. FindResult::new computes tap coordinates from visible_bounds when set.
    #[test]
    fn find_result_uses_visible_bounds_for_tap() {
        let mut elem = make_element("Button", make_bounds(0, 0, 200, 80));
        elem.visible_bounds = Some(make_bounds(0, 0, 50, 40));
        let result = FindResult::new(elem);
        assert_eq!(
            result.tap_x, 25,
            "tap_x SHALL come from visible_bounds center"
        );
        assert_eq!(
            result.tap_y, 20,
            "tap_y SHALL come from visible_bounds center"
        );
    }

    // ── filter_viewport structural behavior ──────────────────────────

    // 11. filter_viewport flattens deeply nested visible descendants into a
    //     single flat child list, and emptied leaves carry no children.
    #[test]
    fn filter_viewport_flattens_deep_tree() {
        let vp = Viewport::new(375, 812);
        let mut root = make_element("View", make_bounds(0, 0, 375, 812));
        let mut group = make_element("Group", make_bounds(10, 100, 200, 200));
        let nested = make_element("Label", make_bounds(20, 110, 80, 20));
        group.children.push(nested);
        root.children.push(group);

        let filtered = filter_viewport(&root, &vp);
        // Group + nested label both visible -> flattened to 2 top-level children.
        assert_eq!(
            filtered.children.len(),
            2,
            "SHALL flatten visible descendants into a single list"
        );
        for child in &filtered.children {
            assert!(
                child.children.is_empty(),
                "flattened leaves SHALL carry no children"
            );
        }
    }

    // 12. filter_viewport preserves the root's own metadata and bounds.
    #[test]
    fn filter_viewport_preserves_root_metadata() {
        let vp = Viewport::new(375, 812);
        let mut root = make_element("Window", make_bounds(0, 0, 375, 812));
        root.text = Some("title".to_string());
        root.accessibility_label = Some("root-window".to_string());
        root.placeholder = Some("ph".to_string());
        root.enabled = true;
        root.checked = true;
        root.clickable = false;
        root.focused = true;
        root.visible_bounds = Some(make_bounds(0, 0, 375, 812));

        let filtered = filter_viewport(&root, &vp);
        assert_eq!(filtered.element_type, "Window");
        assert_eq!(filtered.text.as_deref(), Some("title"));
        assert_eq!(filtered.accessibility_label.as_deref(), Some("root-window"));
        assert_eq!(filtered.placeholder.as_deref(), Some("ph"));
        assert!(filtered.enabled);
        assert!(filtered.checked);
        assert!(!filtered.clickable);
        assert!(filtered.focused);
        assert_eq!(filtered.bounds, make_bounds(0, 0, 375, 812));
        assert_eq!(filtered.visible_bounds, Some(make_bounds(0, 0, 375, 812)));
    }

    // 13. The root element itself is never emitted as a child even if it
    //     would be visible — only descendants are collected.
    #[test]
    fn filter_viewport_excludes_root_from_children() {
        let vp = Viewport::new(375, 812);
        let root = make_element("View", make_bounds(0, 0, 100, 100));
        let filtered = filter_viewport(&root, &vp);
        assert!(
            filtered.children.is_empty(),
            "root SHALL not appear in its own flattened children"
        );
    }

    // ── serde defaults ────────────────────────────────────────────────

    // 14. A minimal companion wire payload — only the required
    //     element_type and bounds, every optional field omitted — SHALL
    //     still parse, with each omitted field falling back to its default.
    //     This guards the wire contract: companions send sparse payloads.
    #[test]
    fn element_deserializes_with_defaults_for_missing_fields() {
        let json = r#"{
            "element_type": "View",
            "bounds": {"x": 0, "y": 0, "width": 10, "height": 10}
        }"#;
        let elem: Element = serde_json::from_str(json).expect("minimal payload SHALL parse");
        // 14a. Required fields carry their wire values.
        assert_eq!(
            elem.element_type, "View",
            "element_type SHALL parse from wire"
        );
        assert_eq!(
            elem.bounds,
            Bounds::new(0, 0, 10, 10),
            "bounds SHALL parse from wire"
        );
        // 14b. Omitted Option fields default to None (serde Option handling).
        assert!(elem.text.is_none(), "omitted text SHALL default to None");
        assert!(
            elem.accessibility_label.is_none(),
            "omitted accessibility_label SHALL default to None"
        );
        assert!(
            elem.placeholder.is_none(),
            "omitted placeholder SHALL default to None"
        );
        assert!(
            elem.visible_bounds.is_none(),
            "omitted visible_bounds SHALL default to None"
        );
        // 14c. Omitted bool fields default to false.
        assert!(!elem.enabled, "omitted enabled SHALL default to false");
        assert!(!elem.checked, "omitted checked SHALL default to false");
        assert!(!elem.clickable, "omitted clickable SHALL default to false");
        assert!(!elem.focused, "omitted focused SHALL default to false");
        // 14d. Omitted children default to an empty Vec.
        assert!(
            elem.children.is_empty(),
            "omitted children SHALL default to empty"
        );
    }
}
