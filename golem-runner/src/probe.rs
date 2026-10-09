//! `probe`: what a selector matches on the current screen, without acting.
//!
//! It never fails a step and is never recorded into a flow. It reports
//! every visible match in the order `resolve_element` picks from, which
//! match a step would act on, what each relational anchor resolved to,
//! and on a miss how many candidates each clause left. Matches in the full
//! tree that are off-screen are a hint only, as the visibility model
//! requires: a step cannot act on them without scrolling.

use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use golem_driver::PlatformDriver;
use golem_element::selector::{
    find_elements, resolve_anchor, resolve_visible_anchor, AnchorSelector, Selector,
};
use golem_element::toon::{format_line, selectable};
use golem_element::{Element, FindResult, Viewport};

use crate::resolution::build_selector;
use crate::resolution::visible_matches;

/// One element a selector matched.
#[derive(Debug, Clone)]
pub struct ProbeMatch {
    /// The element's `[n]` in the visible TOON tree; `None` when it gets no
    /// line there (a layout container matched by traits, an off-screen
    /// element).
    pub index: Option<usize>,
    pub element: Element,
}

/// What one relational anchor resolved to.
#[derive(Debug, Clone)]
pub struct AnchorReport {
    /// `below`, `above`, `right_of`, `left_of`, `contains` or `inside`.
    pub clause: &'static str,
    pub anchor: String,
    pub resolved: AnchorState,
}

#[derive(Debug, Clone)]
pub enum AnchorState {
    Visible(ProbeMatch),
    /// In the full tree but not on screen: the relation cannot hold until
    /// a scroll brings it into view.
    OffScreen(Element),
    Missing,
}

/// The result of a probe.
#[derive(Debug, Clone)]
pub struct ProbeReport {
    /// The selector as written, in the canonical notation.
    pub selector: String,
    /// Visible matches, in the order a step picks from: it acts on the first.
    pub visible: Vec<ProbeMatch>,
    pub anchors: Vec<AnchorReport>,
    /// On a miss, each clause and the matches left once it applies, in
    /// order. Empty when something matched.
    pub clauses: Vec<(String, usize)>,
    /// Full-tree matches that are not visible: a hint only.
    pub offscreen: Vec<ProbeMatch>,
    /// The direction `auto_scroll` would scroll for the first off-screen
    /// match.
    pub scroll: Option<&'static str>,
}

/// Probe `selector` against one snapshot of the screen.
pub fn probe_tree(
    root: &Element,
    keyboard_height: i32,
    selector: &Selector,
    written: &str,
) -> Result<ProbeReport> {
    if selector.is_unconstrained() {
        bail!("the selector has no element criterion; add one, e.g. {{ on_text = \"Sign in\" }}");
    }
    let (viewport, visible_root, results) = visible_matches(root, keyboard_height, selector);
    let indexed = selectable(&visible_root);
    let index_of = |e: &Element| {
        indexed
            .iter()
            .position(|s| same_element(s, e))
            .map(|i| i + 1)
    };
    let visible: Vec<ProbeMatch> = results
        .iter()
        .map(|r| ProbeMatch {
            index: index_of(&r.element),
            element: r.element.clone(),
        })
        .collect();

    let anchors = anchors_of(selector)
        .into_iter()
        .map(|(clause, anchor)| AnchorReport {
            clause,
            anchor: describe_anchor(anchor),
            resolved: match resolve_visible_anchor(&visible_root, anchor) {
                Some(found) => AnchorState::Visible(ProbeMatch {
                    index: index_of(&found.element),
                    element: found.element,
                }),
                None => match resolve_anchor(root, anchor) {
                    Some(found) => AnchorState::OffScreen(found.element),
                    None => AnchorState::Missing,
                },
            },
        })
        .collect();

    let clauses = if visible.is_empty() {
        clause_counts(&visible_root, selector)
    } else {
        Vec::new()
    };

    let offscreen: Vec<ProbeMatch> = find_elements(root, selector)
        .into_iter()
        .filter(|r: &FindResult| !results.iter().any(|v| same_element(&v.element, &r.element)))
        .map(|r| ProbeMatch {
            index: None,
            element: r.element,
        })
        .collect();
    let scroll = offscreen
        .first()
        .map(|m| scroll_direction(&m.element, &viewport));

    Ok(ProbeReport {
        selector: written.to_string(),
        visible,
        anchors,
        clauses,
        offscreen,
        scroll,
    })
}

/// Probe the step's selector on `driver`'s screen. With `timeout_ms` above
/// 0, poll until something visible matches or the time runs out, as a step
/// would.
pub async fn probe(
    driver: &dyn PlatformDriver,
    step: &golem_parser::Step,
    written: &str,
    timeout_ms: u64,
) -> Result<ProbeReport> {
    let selector = build_selector(step);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let (root, meta) = driver.get_hierarchy().await?;
        let report = probe_tree(&root, meta.keyboard_height, &selector, written)?;
        if !report.visible.is_empty() || Instant::now() >= deadline {
            return Ok(report);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The visible text of the element that a step selects by
/// `on_accessibility_label` alone, when `on_text` with that text selects
/// the same element and nothing else: the selector a user would read.
/// `None` for any other step.
pub fn text_alternative(
    root: &Element,
    keyboard_height: i32,
    step: &golem_parser::Step,
) -> Option<String> {
    if step.on_accessibility_label.is_none() || step.on_text.is_some() || step.on.is_some() {
        return None;
    }
    let (_, _, picked) = visible_matches(root, keyboard_height, &build_selector(step));
    let element = &picked.first()?.element;
    let text = element.text.as_deref()?.trim();
    // A `*` or `?` would make the text a glob that matches more.
    if text.is_empty() || text.contains(['*', '?']) {
        return None;
    }
    let by_text = golem_parser::Step {
        on_accessibility_label: None,
        on_text: Some(text.to_string()),
        ..step.clone()
    };
    let (_, _, found) = visible_matches(root, keyboard_height, &build_selector(&by_text));
    match found.as_slice() {
        [one] if one.element.bounds == element.bounds => Some(text.to_string()),
        _ => None,
    }
}

/// The report as TOON text.
pub fn render_toon(report: &ProbeReport) -> String {
    let mut out = format!("probe {} · ", report.selector);
    match report.visible.len() {
        0 => out.push_str("no visible match\n"),
        n => {
            out.push_str(&format!(
                "{n} visible match{} · act picks {}\n",
                if n == 1 { "" } else { "es" },
                index_label(report.visible[0].index)
            ));
        }
    }
    for m in &report.visible {
        out.push_str(&format!(" {}\n", match_line(m)));
    }
    for a in &report.anchors {
        let resolved = match &a.resolved {
            AnchorState::Visible(m) => match_line(m),
            AnchorState::OffScreen(e) => format!("off-screen (hint only) {}", bounds_text(e)),
            AnchorState::Missing => "not found".to_string(),
        };
        out.push_str(&format!("anchor {} {} → {resolved}\n", a.clause, a.anchor));
    }
    for (clause, count) in &report.clauses {
        out.push_str(&format!(" {clause} → {count}\n"));
    }
    if report.visible.len() > 1 {
        out.push_str("warn: more than one element matches, and act picks the first. Add index, or a tighter selector\n");
    }
    if !report.offscreen.is_empty() {
        out.push_str(&format!(
            "hint: {} off-screen match{} (full tree, not what the user sees){}\n",
            report.offscreen.len(),
            if report.offscreen.len() == 1 {
                ""
            } else {
                "es"
            },
            report
                .scroll
                .map(|d| format!("; auto_scroll would scroll {d}"))
                .unwrap_or_default()
        ));
        for m in &report.offscreen {
            out.push_str(&format!(" {}\n", match_line(m)));
        }
    }
    out
}

/// The report as JSON.
pub fn render_json(report: &ProbeReport) -> serde_json::Value {
    let m = |m: &ProbeMatch| {
        let b = m.element.effective_bounds();
        serde_json::json!({
            "index": m.index,
            "type": m.element.element_type,
            "text": m.element.text,
            "label": m.element.accessibility_label,
            "id": m.element.accessibility_id,
            "bounds": { "x": b.x, "y": b.y, "width": b.width, "height": b.height },
            "center": { "x": b.x + b.width / 2, "y": b.y + b.height / 2 },
            "enabled": m.element.enabled,
            "checked": m.element.checked,
        })
    };
    serde_json::json!({
        "selector": report.selector,
        "matches": report.visible.len(),
        "picks": report.visible.first().map(m),
        "visible": report.visible.iter().map(m).collect::<Vec<_>>(),
        "anchors": report.anchors.iter().map(|a| serde_json::json!({
            "clause": a.clause,
            "anchor": a.anchor,
            "state": match &a.resolved {
                AnchorState::Visible(_) => "visible",
                AnchorState::OffScreen(_) => "off_screen",
                AnchorState::Missing => "missing",
            },
            "element": match &a.resolved {
                AnchorState::Visible(found) => m(found),
                _ => serde_json::Value::Null,
            },
        })).collect::<Vec<_>>(),
        "clauses": report.clauses.iter().map(|(c, n)| serde_json::json!({ "clause": c, "matches": n })).collect::<Vec<_>>(),
        "offscreen_hint": report.offscreen.iter().map(m).collect::<Vec<_>>(),
        "scroll": report.scroll,
    })
}

fn match_line(m: &ProbeMatch) -> String {
    let line = format_line(m.index.unwrap_or(0), &m.element);
    match m.index {
        Some(_) => line,
        None => line.replacen("[0]", "[-]", 1),
    }
}

fn index_label(index: Option<usize>) -> String {
    index.map_or_else(|| "[-]".to_string(), |i| format!("[{i}]"))
}

fn bounds_text(e: &Element) -> String {
    let b = e.bounds;
    format!("{},{} {}x{}", b.x, b.y, b.width, b.height)
}

/// The same node, compared by what identifies it on screen: the matcher
/// returns clones.
fn same_element(a: &Element, b: &Element) -> bool {
    a.element_type == b.element_type
        && a.bounds == b.bounds
        && a.text == b.text
        && a.accessibility_label == b.accessibility_label
        && a.accessibility_id == b.accessibility_id
}

fn anchors_of(s: &Selector) -> Vec<(&'static str, &AnchorSelector)> {
    [
        ("below", s.below.as_ref()),
        ("above", s.above.as_ref()),
        ("right_of", s.right_of.as_ref()),
        ("left_of", s.left_of.as_ref()),
        ("contains", s.contains.as_ref()),
        ("inside", s.inside.as_ref()),
    ]
    .into_iter()
    .filter_map(|(clause, a)| a.map(|a| (clause, a)))
    .collect()
}

fn describe_anchor(a: &AnchorSelector) -> String {
    match a {
        AnchorSelector::Text(t) => format!("\"{t}\""),
        AnchorSelector::Full(s) => describe_selector(s),
    }
}

fn describe_selector(s: &Selector) -> String {
    let mut parts = Vec::new();
    if let Some(t) = &s.text {
        parts.push(format!("text = \"{t}\""));
    }
    if let Some(l) = &s.accessibility_label {
        parts.push(format!("accessibility_label = \"{l}\""));
    }
    if !s.traits.is_empty() {
        parts.push(format!("traits = {:?}", s.traits));
    }
    if parts.is_empty() {
        "{ … }".to_string()
    } else {
        format!("{{ {} }}", parts.join(", "))
    }
}

/// Each clause the selector sets, with the matches left once it and the
/// clauses before it apply.
fn clause_counts(visible_root: &Element, s: &Selector) -> Vec<(String, usize)> {
    let mut cumulative = Selector::default();
    let mut out = Vec::new();
    let mut step = |label: String, apply: &dyn Fn(&mut Selector)| {
        apply(&mut cumulative);
        out.push((label, find_elements(visible_root, &cumulative).len()));
    };
    if let Some(t) = &s.text {
        step(format!("text \"{t}\""), &|c| c.text = Some(t.clone()));
    }
    if let Some(l) = &s.accessibility_label {
        step(format!("accessibility_label \"{l}\""), &|c| {
            c.accessibility_label = Some(l.clone())
        });
    }
    if let Some(v) = s.enabled {
        step(format!("enabled {v}"), &|c| c.enabled = Some(v));
    }
    if let Some(v) = s.checked {
        step(format!("checked {v}"), &|c| c.checked = Some(v));
    }
    if let Some(v) = s.clickable {
        step(format!("clickable {v}"), &|c| c.clickable = Some(v));
    }
    if !s.traits.is_empty() {
        step(format!("traits {:?}", s.traits), &|c| {
            c.traits = s.traits.clone()
        });
    }
    for (clause, anchor) in anchors_of(s) {
        let anchor = anchor.clone();
        let min = s.contains_min_matches;
        step(
            format!("{clause} {}", describe_anchor(&anchor)),
            &move |c| {
                let a = Some(anchor.clone());
                match clause {
                    "below" => c.below = a,
                    "above" => c.above = a,
                    "right_of" => c.right_of = a,
                    "left_of" => c.left_of = a,
                    "contains" => {
                        c.contains = a;
                        c.contains_min_matches = min;
                    }
                    _ => c.inside = a,
                }
            },
        );
    }
    if let Some(i) = s.index {
        step(format!("index {i}"), &|c| c.index = Some(i));
    }
    out
}

/// `down` when the element sits below the middle of the viewport, as
/// `auto_scroll` decides with no `within` container.
fn scroll_direction(e: &Element, viewport: &Viewport) -> &'static str {
    if e.bounds.center_y() > viewport.y + viewport.height / 2 {
        "down"
    } else {
        "up"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use golem_element::Bounds;

    fn screen() -> Element {
        let mut root = make_element("View", Bounds::new(0, 0, 400, 800));
        root.children.push(make_element_with_text(
            "Label",
            "Password",
            Bounds::new(20, 100, 360, 40),
        ));
        root.children.push(make_element_with_text(
            "Button",
            "Sign in",
            Bounds::new(20, 200, 360, 60),
        ));
        root.children.push(make_element_with_text(
            "Button",
            "Sign in with Google",
            Bounds::new(20, 300, 360, 60),
        ));
        root.children.push(make_element_with_text(
            "Button",
            "Sign up",
            Bounds::new(20, 1200, 360, 60),
        ));
        root
    }

    fn sel(text: &str) -> Selector {
        Selector {
            text: Some(text.into()),
            ..Selector::default()
        }
    }

    #[test]
    fn one_match_is_what_act_picks() {
        let r =
            probe_tree(&screen(), 0, &sel("Sign in"), r#"{ on_text = "Sign in" }"#).expect("probe");
        assert_eq!(r.visible.len(), 1);
        let text = render_toon(&r);
        assert!(
            text.starts_with("probe { on_text = \"Sign in\" } · 1 visible match · act picks [3]\n"),
            "{text}"
        );
        assert!(!text.contains("warn:"), "{text}");
    }

    #[test]
    fn several_matches_name_the_pick_and_warn() {
        let r = probe_tree(
            &screen(),
            0,
            &sel("Sign in*"),
            r#"{ on_text = "Sign in*" }"#,
        )
        .expect("probe");
        let text = render_toon(&r);
        assert!(text.contains("2 visible matches · act picks [3]"), "{text}");
        assert!(
            text.contains(" [4] Button \"Sign in with Google\""),
            "{text}"
        );
        assert!(
            text.contains("warn: more than one element matches"),
            "{text}"
        );
    }

    #[test]
    fn a_miss_counts_the_matches_each_clause_leaves() {
        let s = Selector {
            text: Some("Sign in*".into()),
            above: Some(AnchorSelector::Text("Password".into())),
            ..Selector::default()
        };
        let r = probe_tree(&screen(), 0, &s, "x").expect("probe");
        assert!(r.visible.is_empty());
        assert_eq!(
            r.clauses,
            vec![
                ("text \"Sign in*\"".to_string(), 2),
                ("above \"Password\"".to_string(), 0)
            ]
        );
        assert!(render_toon(&r).contains(" above \"Password\" → 0\n"));
    }

    #[test]
    fn a_chained_anchor_shows_what_it_resolved_to() {
        let s = Selector {
            text: Some("Sign in*".into()),
            below: Some(AnchorSelector::Text("Password".into())),
            ..Selector::default()
        };
        let r = probe_tree(&screen(), 0, &s, "x").expect("probe");
        let text = render_toon(&r);
        assert!(
            text.contains("anchor below \"Password\" → [2] Label \"Password\" 20,100"),
            "{text}"
        );
    }

    #[test]
    fn an_off_screen_match_is_a_hint_with_the_scroll_direction() {
        let r = probe_tree(&screen(), 0, &sel("Sign up"), "x").expect("probe");
        assert!(
            r.visible.is_empty(),
            "an off-screen element SHALL NOT count as a match"
        );
        assert_eq!(r.scroll, Some("down"));
        let text = render_toon(&r);
        assert!(text.contains("no visible match"), "{text}");
        assert!(text.contains("hint: 1 off-screen match (full tree, not what the user sees); auto_scroll would scroll down"), "{text}");
        assert!(text.contains(" [-] Button \"Sign up\" 20,1200"), "{text}");
    }

    #[test]
    fn an_element_under_the_keyboard_is_not_visible() {
        let r = probe_tree(&screen(), 550, &sel("Sign in with Google"), "x").expect("probe");
        assert!(r.visible.is_empty());
        assert_eq!(r.offscreen.len(), 1);
    }

    #[test]
    fn an_empty_selector_is_refused() {
        assert!(probe_tree(&screen(), 0, &Selector::default(), "{}").is_err());
    }

    #[test]
    fn json_names_the_pick() {
        let r = probe_tree(&screen(), 0, &sel("Sign in*"), "x").expect("probe");
        let v = render_json(&r);
        assert_eq!(v["matches"], 2);
        assert_eq!(v["picks"]["text"], "Sign in");
        assert_eq!(v["picks"]["index"], 3);
        assert_eq!(v["picks"]["center"]["y"], 230);
    }

    /// `probe` and `resolve_element` SHALL pick the same element for the
    /// same selector and the same screen.
    #[tokio::test]
    async fn probe_and_resolve_element_pick_the_same_element() {
        for text in ["Sign in*", "Sign in", "*Google"] {
            let driver = golem_driver::MockPlatformDriver::new(screen());
            let step = golem_parser::Step {
                on_text: Some(text.into()),
                timeout: Some(500),
                ..make_step("tap")
            };
            let (resolved, _) = crate::resolution::resolve_element(&step, &driver, None)
                .await
                .expect("resolve");
            let report = probe(&driver, &step, "x", 0).await.expect("probe");
            assert!(
                same_element(&report.visible[0].element, &resolved),
                "{text}: probe picked {:?}, resolve_element {:?}",
                report.visible[0].element.text,
                resolved.text
            );
        }
    }

    fn by_label(label: &str) -> golem_parser::Step {
        golem_parser::Step {
            action: "tap".into(),
            on_accessibility_label: Some(label.into()),
            ..golem_parser::Step::default()
        }
    }

    fn labelled(text: &str, label: &str, y: i32) -> Element {
        let mut e = make_element_with_text("Button", text, Bounds::new(20, y, 360, 60));
        e.accessibility_label = Some(label.into());
        e
    }

    #[test]
    fn a_label_selector_gets_the_text_that_selects_the_same_element() {
        let mut root = make_element("View", Bounds::new(0, 0, 400, 800));
        root.children.push(labelled("Sign in", "login", 100));
        assert_eq!(
            text_alternative(&root, 0, &by_label("login")).as_deref(),
            Some("Sign in")
        );
    }

    #[test]
    fn no_text_alternative_when_the_text_is_shared_or_missing() {
        let mut root = make_element("View", Bounds::new(0, 0, 400, 800));
        root.children.push(labelled("OK", "first", 100));
        root.children.push(labelled("OK", "second", 200));
        root.children.push(labelled("", "icon", 300));
        assert_eq!(text_alternative(&root, 0, &by_label("second")), None);
        assert_eq!(text_alternative(&root, 0, &by_label("icon")), None);
        let mut both = by_label("first");
        both.on_text = Some("OK".into());
        assert_eq!(text_alternative(&root, 0, &both), None);
    }
}
