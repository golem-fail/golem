#![allow(clippy::disallowed_macros)] // a command renderer: stdout is its output
use anyhow::{bail, Result};
use golem_driver::android::AndroidDriver;
use golem_driver::ios::IosDriver;
use golem_driver::PlatformDriver;
use golem_element::selector::element_has_trait;
use golem_element::{filter_viewport, Element, Viewport};

use crate::cli::TreeArgs;

/// Run the `golem tree` command: fetch and display the UI hierarchy.
pub async fn run(args: &TreeArgs) -> Result<()> {
    let platform = match args.platform.as_deref() {
        None => None,
        Some("ios") => Some(golem_devices::Platform::Ios),
        Some("android") => Some(golem_devices::Platform::Android),
        Some(p) => bail!("unknown platform: {p}. Use 'ios' or 'android'."),
    };
    let query = golem_orchestrator::target::TargetQuery {
        platform,
        device: args.device.clone(),
        bundle: args.bundle.clone(),
        app: args.app.clone(),
    };
    let cwd = std::env::current_dir()?;
    let (project, _) = golem_orchestrator::project::ProjectConfig::load_from(&cwd)?;
    let target = match golem_orchestrator::target::resolve(&query, &project.apps).await {
        Ok(target) => target,
        Err(e) => {
            if e.to_string().contains("start a simulator or emulator") {
                // Auto-invoke doctor: explain *why* nothing was found (missing
                // CLI, no booted device, absent companion) rather than a bare
                // error.
                crate::doctor::hint_no_device().await;
            }
            return Err(e);
        }
    };

    let port = target.port;
    let platform = target.device.platform.to_string();
    let name = &target.device.name;
    let device_id = &target.device.udid;
    let bundle = target.bundle.as_str();
    let driver: Box<dyn PlatformDriver> = match target.device.platform {
        golem_devices::Platform::Android => Box::new(AndroidDriver::new(
            device_id.clone(),
            bundle.to_string(),
            port,
            target.device.physical,
        )),
        golem_devices::Platform::Ios => Box::new(IosDriver::new(
            device_id.clone(),
            bundle.to_string(),
            port,
            target.device.physical,
        )),
    };

    // First call triggers async CDP setup for Android WebViews.
    // Second call (after a brief wait) gets the CDP-enriched tree.
    let (root, meta) = match driver.get_hierarchy().await {
        Ok(r) => r,
        Err(e) => bail!("{name} ({platform}, port {port}): failed to fetch hierarchy: {e}"),
    };

    // If the tree contains a WebView, wait for background inspector setup
    // (CDP on Android, WebKit Inspector on iOS) and fetch again with enrichment.
    let has_webview = has_webview_element(&root);
    let root = if has_webview {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        driver.get_hierarchy().await.map(|(r, _)| r).unwrap_or(root)
    } else {
        root
    };

    println!("── {name} ({platform}, port {port}) ──");

    if args.verbose {
        println!("  device_id: {device_id}");
        println!(
            "  bundle: {}",
            if bundle.is_empty() { "(none)" } else { bundle }
        );
        if meta.keyboard_height > 0 {
            println!("  keyboard: open ({}px)", meta.keyboard_height);
        } else {
            println!("  keyboard: closed");
        }
        if meta.safe_area_top > 0 || meta.safe_area_bottom > 0 {
            println!(
                "  safe_area: top={} bottom={}",
                meta.safe_area_top, meta.safe_area_bottom
            );
        }
        if !meta.cutouts.is_empty() {
            let rects: Vec<String> = meta
                .cutouts
                .iter()
                .map(|c| format!("Rect({},{} {}x{})", c.x, c.y, c.width, c.height))
                .collect();
            println!("  cutouts: {}", rects.join(", "));
        }
        if !meta.rounded_corners.is_empty() {
            let corners: Vec<String> = meta
                .rounded_corners
                .iter()
                .map(|c| {
                    let pos = match c.position {
                        golem_driver::common::CornerPosition::TopLeft => "TL",
                        golem_driver::common::CornerPosition::TopRight => "TR",
                        golem_driver::common::CornerPosition::BottomRight => "BR",
                        golem_driver::common::CornerPosition::BottomLeft => "BL",
                    };
                    format!("{}={}", pos, c.radius)
                })
                .collect();
            println!("  corners: {}", corners.join(" "));
        }
        if platform == "android" {
            let has_webview = has_webview_element(&root);
            if has_webview {
                println!("  webview: detected, CDP enrichment active");
            } else {
                println!("  webview: not detected");
            }
        }
    }

    let display = if args.full {
        root
    } else {
        let mut vp = Viewport::from_root(&root);
        if meta.keyboard_height > 0 {
            vp.height -= meta.keyboard_height;
        }
        filter_viewport(&root, &vp)
    };

    if args.json || args.output == crate::cli::TreeOutput::Json {
        if let Ok(json) = serde_json::to_string_pretty(&display) {
            println!("{json}");
        }
    } else if args.verbose {
        print_tree_debug(&display, 0);
    } else {
        let header = golem_element::toon::TreeHeader {
            full: args.full,
            keyboard_height: meta.keyboard_height,
        };
        print!("{}", golem_element::toon::encode_tree(&display, &header));
    }
    println!();

    Ok(())
}

fn has_webview_element(root: &Element) -> bool {
    if root.element_type.to_lowercase().contains("webview")
        || root.element_type.to_lowercase().contains("web_view")
    {
        return true;
    }
    root.children.iter().any(has_webview_element)
}

fn print_tree_debug(element: &Element, depth: usize) {
    print_tree_inner(element, depth, true);
}

fn print_tree_inner(element: &Element, depth: usize, debug: bool) {
    println!("{}", format_tree_line(element, depth, debug));
    for child in &element.children {
        print_tree_inner(child, depth + 1, debug);
    }
}

/// ` label=… id=…` for the parts that add something beyond the visible text.
fn label_and_id_part(e: &Element) -> String {
    let text = e.text.as_deref();
    let mut out = String::new();
    for (key, value) in [
        ("label", e.accessibility_label.as_deref()),
        ("id", e.accessibility_id.as_deref()),
    ] {
        if let Some(v) = value.filter(|v| !v.is_empty() && Some(*v) != text) {
            out.push_str(&format!(" {key}={v}"));
        }
    }
    out
}

/// Render a single element's tree line (no trailing newline, no children).
fn format_tree_line(element: &Element, depth: usize, debug: bool) -> String {
    let indent = "  ".repeat(depth);
    let text = element.text.as_deref().unwrap_or("");
    let label = label_and_id_part(element);
    let et = &element.element_type;
    let b = element.effective_bounds();

    let mut state_parts = Vec::new();
    if !element.enabled {
        state_parts.push("disabled");
    }
    if element.checked {
        state_parts.push("checked");
    }
    if element.focused {
        state_parts.push("focused");
    }
    let state = if state_parts.is_empty() {
        String::new()
    } else {
        format!(" [{}]", state_parts.join(", "))
    };

    // In debug mode, show both bounds when they differ
    let bounds_extra = if debug {
        if let Some(ref vb) = element.visible_bounds {
            if *vb != element.bounds {
                let fb = &element.bounds;
                format!(" (full: {},{} {}x{})", fb.x, fb.y, fb.width, fb.height)
            } else {
                String::new()
            }
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let traits = format_traits(element);
    let traits_part = if traits.is_empty() {
        String::new()
    } else {
        format!("  {traits}")
    };

    if !text.is_empty() || !label.is_empty() {
        format!(
            "{indent}{et} \"{text}\"{label} ({},{} {}x{}){bounds_extra}{traits_part}{state}",
            b.x, b.y, b.width, b.height
        )
    } else {
        format!(
            "{indent}{et} ({},{} {}x{}){bounds_extra}{traits_part}{state}",
            b.x, b.y, b.width, b.height
        )
    }
}

/// Render trait list as `·a·b·c·`, or empty string if none match.
fn format_traits(e: &Element) -> String {
    let matched: Vec<&str> = golem_element::toon::RENDERED_TRAITS
        .iter()
        .copied()
        .filter(|t| element_has_trait(e, t))
        .collect();
    if matched.is_empty() {
        String::new()
    } else {
        format!("·{}·", matched.join("·"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_element::Bounds;

    // ── Test helpers ──────────────────────────────────────────────────

    fn elem(element_type: &str) -> Element {
        Element {
            element_type: element_type.to_string(),
            text: None,
            accessibility_label: None,
            accessibility_id: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: false,
            focused: false,
            bounds: Bounds::new(0, 0, 100, 40),
            visible_bounds: None,
            hit_points: vec![],
            drawing_order: None,
            children: Vec::new(),
        }
    }

    fn elem_with_text(element_type: &str, text: &str) -> Element {
        let mut e = elem(element_type);
        e.text = Some(text.to_string());
        e
    }

    // ── has_webview_element ───────────────────────────────────────────

    // 1. Root element_type containing "webview" (case-insensitive) matches.
    #[test]
    fn webview_detected_on_root_case_insensitive() {
        let root = elem("WebView");
        assert!(
            has_webview_element(&root),
            "root type containing 'webview' SHALL be detected"
        );
    }

    // 2. The "web_view" underscore spelling also matches.
    #[test]
    fn webview_detected_underscore_spelling() {
        let root = elem("ANDROID_WEB_VIEW");
        assert!(
            has_webview_element(&root),
            "root type containing 'web_view' SHALL be detected"
        );
    }

    // 3. Tree with no webview anywhere returns false.
    #[test]
    fn webview_absent_returns_false() {
        let mut root = elem("View");
        root.children.push(elem("Button"));
        root.children.push(elem_with_text("Text", "hello"));
        assert!(
            !has_webview_element(&root),
            "tree without any webview SHALL return false"
        );
    }

    // 4. Webview nested deep in descendants is found via recursion.
    #[test]
    fn webview_detected_in_nested_descendant() {
        let mut root = elem("View");
        let mut mid = elem("Group");
        mid.children.push(elem("WebView"));
        root.children.push(mid);
        assert!(
            has_webview_element(&root),
            "webview nested in descendants SHALL be detected"
        );
    }

    // ── format_traits ─────────────────────────────────────────────────

    // 17. A text-less element with zero-area bounds matches only `no_text`
    //     (every element matches exactly one of has_text/no_text, so the
    //     output is never empty in practice).
    #[test]
    fn format_traits_no_text_only_for_empty_element() {
        let mut e = elem("View");
        e.bounds = Bounds::new(0, 0, 0, 0);
        assert_eq!(
            format_traits(&e),
            "·no_text·",
            "text-less zero-area element SHALL render only the no_text trait"
        );
    }

    // 18. Matched traits are wrapped and joined with `·` delimiters, in
    //     RENDERED_TRAITS order (content type → text → shape → size).
    #[test]
    fn format_traits_orders_and_wraps_with_dots() {
        // Button + has_text("Hi" => short_text) + wide (100 > 2*40) + size.
        let mut e = elem_with_text("button", "Hi");
        e.bounds = Bounds::new(0, 0, 100, 40);
        let out = format_traits(&e);
        // Expected order: button, has_text, short_text, wide, small (area 4000? no >2500)
        // area = 100*40 = 4000 -> not small (<2500), not large (>100k).
        assert_eq!(
            out, "·button·has_text·short_text·wide·",
            "traits SHALL render in RENDERED_TRAITS order wrapped in dots"
        );
    }

    // 19. The "text" alias is intentionally excluded from rendered output even
    //     though it matches the same condition as has_text.
    #[test]
    fn format_traits_excludes_text_alias() {
        // "Hello" is 5 chars => has_text and short_text both match. The "text"
        // alias matches the same condition as has_text but is NOT in
        // RENDERED_TRAITS, so it never duplicates has_text in the output.
        let mut e = elem_with_text("Label", "Hello");
        e.bounds = Bounds::new(0, 0, 0, 0); // suppress shape/size traits
        let out = format_traits(&e);
        assert_eq!(
            out, "·has_text·short_text·",
            "has_text SHALL render but its 'text' alias SHALL NOT duplicate it"
        );
    }

    // ── print smoke (no panic) ────────────────────────────────────────

    // 20. The debug tree printer SHALL not panic on a representative tree,
    //     including the verbose bounds-extra.
    #[test]
    fn print_helpers_do_not_panic() {
        let mut root = elem("View");
        let mut titled = elem_with_text("button", "Go");
        titled.accessibility_label = Some("Go button".to_string());
        titled.enabled = false;
        titled.checked = true;
        titled.focused = true;
        titled.visible_bounds = Some(Bounds::new(5, 5, 10, 10)); // differs from bounds
        root.children.push(titled);

        print_tree_debug(&root, 0);
    }

    // ── format_tree_line ──────────────────────────────────────────────

    // 24. A text+label element renders the quoted text, label, bounds and traits;
    //     indentation is two spaces per depth level.
    #[test]
    fn format_tree_line_with_text_and_label() {
        let mut e = elem_with_text("button", "Go");
        e.accessibility_label = Some("Go button".to_string());
        e.bounds = Bounds::new(1, 2, 100, 40);
        let line = format_tree_line(&e, 2, false);
        assert_eq!(
            line,
            "    button \"Go\" label=Go button (1,2 100x40)  ·button·has_text·short_text·wide·",
            "tree line SHALL render indent, type, text, label, bounds and traits"
        );
    }

    // 24a. An accessibility_id renders as `id=` after the label.
    #[test]
    fn format_tree_line_with_label_and_id() {
        let mut e = elem_with_text("text", "Tagged");
        e.accessibility_label = Some("Tagged".to_string());
        e.accessibility_id = Some("tagged-text".to_string());
        e.bounds = Bounds::new(1, 2, 100, 40);
        let line = format_tree_line(&e, 0, false);
        assert!(
            line.starts_with("text \"Tagged\" id=tagged-text (1,2 100x40)"),
            "tree line SHALL show the id and skip a label equal to the text: {line}"
        );
    }

    // 25. A text-less element omits the quoted-text segment and renders state.
    #[test]
    fn format_tree_line_textless_with_state() {
        let mut e = elem("View");
        e.bounds = Bounds::new(0, 0, 0, 0); // suppress shape/size traits
        e.enabled = false;
        e.checked = true;
        let line = format_tree_line(&e, 0, false);
        assert_eq!(
            line, "View (0,0 0x0)  ·no_text· [disabled, checked]",
            "text-less element SHALL omit quoted text and render state suffix"
        );
    }

    // 26. Debug mode appends `(full: ...)` only when visible_bounds differ from
    //     bounds; non-debug never shows the extra even when they differ.
    #[test]
    fn format_tree_line_debug_bounds_extra() {
        let mut e = elem("View");
        e.bounds = Bounds::new(0, 0, 100, 40);
        e.visible_bounds = Some(Bounds::new(5, 5, 10, 10));
        let debug = format_tree_line(&e, 0, true);
        assert!(
            debug.contains("(full: 0,0 100x40)"),
            "debug mode SHALL show full bounds when visible_bounds differ"
        );
        let plain = format_tree_line(&e, 0, false);
        assert!(
            !plain.contains("full:"),
            "non-debug mode SHALL NOT show full bounds extra"
        );
    }
}
