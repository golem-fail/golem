use anyhow::{bail, Result};
use golem_driver::android::AndroidDriver;
use golem_driver::ios::IosDriver;
use golem_driver::PlatformDriver;
use golem_element::selector::element_has_trait;
use golem_element::{filter_viewport, Element, Viewport};

/// Traits surfaced in tree output. Order = render order: content type →
/// text → shape → size. `text` is an alias of `has_text` and is omitted to
/// avoid duplicate rendering.
const RENDERED_TRAITS: &[&str] = &[
    "button", "input", "toggle",
    "has_text", "no_text", "short_text", "long_text",
    "square", "wide", "tall",
    "small", "large",
];

use crate::cli::TreeArgs;

/// Run the `golem tree` command: fetch and display the UI hierarchy.
pub async fn run(args: &TreeArgs) -> Result<()> {
    let platform_filter = args.platform.as_deref().map(|p| match p {
        "ios" => "ios",
        "android" => "android",
        _ => {
            eprintln!("Unknown platform: {p}. Use 'ios' or 'android'.");
            std::process::exit(1);
        }
    });

    // Scan for running companions first
    let mut companions = crate::suite::scan_companions_public().await;

    // Filter by platform
    if let Some(pf) = platform_filter {
        companions.retain(|(_, h)| h.platform == pf);
    }

    // Filter by device name/UDID
    if let Some(ref filter) = args.device {
        let f = filter.to_lowercase();
        companions.retain(|(_, h)| {
            h.device_name.to_lowercase().contains(&f)
                || h.device_id.to_lowercase().contains(&f)
        });
    }

    // If no companions found, discover devices and start them
    if companions.is_empty() {
        eprintln!("  No running companions found. Starting...");
        let started = crate::suite::start_companions_public(platform_filter).await?;
        companions = started;

        if let Some(pf) = platform_filter {
            companions.retain(|(_, h)| h.platform == pf);
        }
        if let Some(ref filter) = args.device {
            let f = filter.to_lowercase();
            companions.retain(|(_, h)| {
                h.device_name.to_lowercase().contains(&f)
                    || h.device_id.to_lowercase().contains(&f)
            });
        }
    }

    if companions.is_empty() {
        bail!("No devices found. Start a simulator or emulator first.");
    }

    for (port, health) in &companions {
        let platform = &health.platform;
        let name = &health.device_name;
        let bundle = args.bundle.as_deref().unwrap_or("fail.golem.test");

        // Create the appropriate driver — same code path as test execution,
        // including CDP enrichment for Android WebViews.
        let device_id = find_device_id(platform, name).await;
        // `golem tree` only reads the accessibility hierarchy — it
        // never calls actions that branch on the `physical` flag, so
        // passing `false` here is correct regardless of the target's
        // actual kind. If a future tree feature needs phys/sim info,
        // plumb `DeviceInfo` through `find_device_id` instead.
        let driver: Box<dyn PlatformDriver> = match platform.as_str() {
            "android" => Box::new(AndroidDriver::new(
                device_id.clone(),
                bundle.to_string(),
                *port,
                false,
            )),
            _ => Box::new(IosDriver::new(
                device_id.clone(),
                bundle.to_string(),
                *port,
                false,
            )),
        };

        // First call triggers async CDP setup for Android WebViews.
        // Second call (after a brief wait) gets the CDP-enriched tree.
        let (root, meta) = match driver.get_hierarchy().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  {name} ({platform}, port {port}): failed to fetch hierarchy: {e}");
                continue;
            }
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
            println!("  bundle: {bundle}");
            if meta.keyboard_height > 0 {
                println!("  keyboard: open ({}px)", meta.keyboard_height);
            } else {
                println!("  keyboard: closed");
            }
            if meta.safe_area_top > 0 || meta.safe_area_bottom > 0 {
                println!("  safe_area: top={} bottom={}", meta.safe_area_top, meta.safe_area_bottom);
            }
            if !meta.cutouts.is_empty() {
                let rects: Vec<String> = meta.cutouts.iter()
                    .map(|c| format!("Rect({},{} {}x{})", c.x, c.y, c.width, c.height))
                    .collect();
                println!("  cutouts: {}", rects.join(", "));
            }
            if !meta.rounded_corners.is_empty() {
                let corners: Vec<String> = meta.rounded_corners.iter()
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

        if args.json {
            if let Ok(json) = serde_json::to_string_pretty(&display) {
                println!("{json}");
            }
        } else if args.full || args.verbose {
            if args.verbose {
                print_tree_debug(&display, 0);
            } else {
                print_tree(&display, 0);
            }
        } else {
            print_selectable_list(&display);
        }
        println!();
    }

    Ok(())
}

/// Find the device serial/UDID for a platform and device name.
/// For Android, queries `adb devices`. For iOS, queries `xcrun simctl`.
async fn find_device_id(platform: &str, device_name: &str) -> String {
    match platform {
        "android" => {
            // Get first connected Android device serial
            if let Ok(output) = tokio::process::Command::new("adb")
                .args(["devices"])
                .output()
                .await
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().skip(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 && parts[1] == "device" {
                        return parts[0].to_string();
                    }
                }
            }
            "emulator-5554".to_string() // fallback
        }
        "ios" => {
            // Get UDID by matching device name
            if let Ok(devices) = golem_devices::ios::discover_ios_devices().await {
                if let Some(d) = devices.iter().find(|d| d.name == device_name && d.state == golem_devices::DeviceState::Booted) {
                    return d.udid.clone();
                }
                // Fallback: first booted device
                if let Some(d) = devices.iter().find(|d| d.state == golem_devices::DeviceState::Booted) {
                    return d.udid.clone();
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

fn has_webview_element(root: &Element) -> bool {
    if root.element_type.to_lowercase().contains("webview")
        || root.element_type.to_lowercase().contains("web_view")
    {
        return true;
    }
    root.children.iter().any(has_webview_element)
}

fn print_tree(element: &Element, depth: usize) {
    print_tree_inner(element, depth, false);
}

fn print_tree_debug(element: &Element, depth: usize) {
    print_tree_inner(element, depth, true);
}

fn print_tree_inner(element: &Element, depth: usize, debug: bool) {
    let indent = "  ".repeat(depth);
    let text = element.text.as_deref().unwrap_or("");
    let label = element
        .accessibility_label
        .as_deref()
        .filter(|s| !s.is_empty() && Some(*s) != element.text.as_deref())
        .map(|s| format!(" label={s}"))
        .unwrap_or_default();
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
    let traits_part = if traits.is_empty() { String::new() } else { format!("  {traits}") };

    if !text.is_empty() || !label.is_empty() {
        println!(
            "{indent}{et} \"{text}\"{label} ({},{} {}x{}){bounds_extra}{traits_part}{state}",
            b.x, b.y, b.width, b.height
        );
    } else {
        println!(
            "{indent}{et} ({},{} {}x{}){bounds_extra}{traits_part}{state}",
            b.x, b.y, b.width, b.height
        );
    }

    for child in &element.children {
        print_tree_inner(child, depth + 1, debug);
    }
}

/// True if this element is worth surfacing in the default `golem tree`
/// output. Filters out pure layout containers that have no selector
/// affordance.
fn is_selectable(e: &Element) -> bool {
    let has_text = e.text.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
    let has_label = e.accessibility_label.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
    has_text
        || has_label
        || e.clickable
        || element_has_trait(e, "button")
        || element_has_trait(e, "input")
        || element_has_trait(e, "toggle")
}

fn collect_selectable<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
    if is_selectable(e) {
        out.push(e);
    }
    for child in &e.children {
        collect_selectable(child, out);
    }
}

/// Render trait list as `·a·b·c·`, or empty string if none match.
fn format_traits(e: &Element) -> String {
    let matched: Vec<&str> = RENDERED_TRAITS
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

fn print_selectable_list(root: &Element) {
    let mut nodes: Vec<&Element> = Vec::new();
    collect_selectable(root, &mut nodes);

    if nodes.is_empty() {
        println!("(no selectable elements — try --full)");
        return;
    }

    for (i, e) in nodes.iter().enumerate() {
        let idx = i + 1;
        let b = e.effective_bounds();
        let bounds = format!("({},{} {}x{})", b.x, b.y, b.width, b.height);

        let text = e.text.as_deref().unwrap_or("");
        let text_part = if text.is_empty() {
            String::new()
        } else {
            format!(" \"{text}\"")
        };

        let label_part = e
            .accessibility_label
            .as_deref()
            .filter(|s| !s.is_empty() && Some(*s) != e.text.as_deref())
            .map(|s| format!(" label={s}"))
            .unwrap_or_default();

        let traits = format_traits(e);
        let traits_part = if traits.is_empty() { String::new() } else { format!("  {traits}") };

        let mut state_parts = Vec::new();
        if !e.enabled { state_parts.push("disabled"); }
        if e.checked { state_parts.push("checked"); }
        if e.focused { state_parts.push("focused"); }
        let state = if state_parts.is_empty() {
            String::new()
        } else {
            format!(" [{}]", state_parts.join(", "))
        };

        println!("[{idx}] {bounds}{text_part}{label_part}{traits_part}{state}");
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
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: false,
            focused: false,
            bounds: Bounds::new(0, 0, 100, 40),
            visible_bounds: None,
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

    // ── is_selectable ─────────────────────────────────────────────────

    // 5. Plain layout container with no text/label/click/trait is not selectable.
    #[test]
    fn plain_container_not_selectable() {
        let e = elem("View");
        assert!(
            !is_selectable(&e),
            "layout container with no affordance SHALL NOT be selectable"
        );
    }

    // 6. Non-empty text makes an element selectable.
    #[test]
    fn element_with_text_is_selectable() {
        let e = elem_with_text("Label", "Hello");
        assert!(is_selectable(&e), "element with text SHALL be selectable");
    }

    // 7. Empty-string text does NOT make an element selectable.
    #[test]
    fn element_with_empty_text_not_selectable() {
        let mut e = elem("Label");
        e.text = Some(String::new());
        assert!(
            !is_selectable(&e),
            "element with empty text SHALL NOT be selectable"
        );
    }

    // 8. Non-empty accessibility_label makes an element selectable.
    #[test]
    fn element_with_label_is_selectable() {
        let mut e = elem("View");
        e.accessibility_label = Some("Submit".to_string());
        assert!(
            is_selectable(&e),
            "element with accessibility label SHALL be selectable"
        );
    }

    // 9. Empty accessibility_label does NOT make an element selectable.
    #[test]
    fn element_with_empty_label_not_selectable() {
        let mut e = elem("View");
        e.accessibility_label = Some(String::new());
        assert!(
            !is_selectable(&e),
            "element with empty label SHALL NOT be selectable"
        );
    }

    // 10. clickable flag alone makes an element selectable.
    #[test]
    fn clickable_element_is_selectable() {
        let mut e = elem("View");
        e.clickable = true;
        assert!(
            is_selectable(&e),
            "clickable element SHALL be selectable"
        );
    }

    // 11. A button-type element (via the "button" trait) is selectable even
    //     with no text/label/click.
    #[test]
    fn button_trait_is_selectable() {
        let e = elem("Button");
        assert!(
            is_selectable(&e),
            "button-trait element SHALL be selectable"
        );
    }

    // 12. An input-type element (via the "input" trait) is selectable.
    #[test]
    fn input_trait_is_selectable() {
        let e = elem("text_field");
        assert!(
            is_selectable(&e),
            "input-trait element SHALL be selectable"
        );
    }

    // 13. A toggle-type element (via the "toggle" trait) is selectable.
    #[test]
    fn toggle_trait_is_selectable() {
        let e = elem("switch");
        assert!(
            is_selectable(&e),
            "toggle-trait element SHALL be selectable"
        );
    }

    // ── collect_selectable ────────────────────────────────────────────

    // 14. Collects selectable nodes in pre-order (parent before children),
    //     skipping non-selectable containers but still descending into them.
    #[test]
    fn collect_selectable_preorder_skips_containers() {
        let mut root = elem("View"); // not selectable
        let mut wrapper = elem("Group"); // not selectable
        wrapper.children.push(elem_with_text("Label", "Deep"));
        root.children.push(elem_with_text("Button", "Top"));
        root.children.push(wrapper);

        let mut out: Vec<&Element> = Vec::new();
        collect_selectable(&root, &mut out);

        assert_eq!(out.len(), 2, "two selectable descendants SHALL be collected");
        assert_eq!(
            out[0].text.as_deref(),
            Some("Top"),
            "pre-order SHALL visit the earlier sibling first"
        );
        assert_eq!(
            out[1].text.as_deref(),
            Some("Deep"),
            "recursion SHALL descend into non-selectable containers"
        );
    }

    // 15. A selectable root includes itself before its children.
    #[test]
    fn collect_selectable_includes_selectable_root() {
        let mut root = elem_with_text("Button", "Root");
        root.children.push(elem_with_text("Label", "Child"));

        let mut out: Vec<&Element> = Vec::new();
        collect_selectable(&root, &mut out);

        assert_eq!(out.len(), 2, "selectable root and child SHALL both be collected");
        assert_eq!(
            out[0].text.as_deref(),
            Some("Root"),
            "selectable root SHALL be collected before its children"
        );
    }

    // 16. A tree of only non-selectable containers yields an empty list.
    #[test]
    fn collect_selectable_empty_for_pure_containers() {
        let mut root = elem("View");
        root.children.push(elem("Group"));
        root.children.push(elem("Stack"));

        let mut out: Vec<&Element> = Vec::new();
        collect_selectable(&root, &mut out);

        assert!(out.is_empty(), "pure-container tree SHALL collect nothing");
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

    // 20. The print/render helpers SHALL not panic on a representative tree,
    //     including the empty-selectable branch and verbose/debug bounds-extra.
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

        print_tree(&root, 0);
        print_tree_debug(&root, 0);
        print_selectable_list(&root);

        // Empty-selectable branch.
        let empty = elem("View");
        print_selectable_list(&empty);
    }
}
