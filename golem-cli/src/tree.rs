use anyhow::{bail, Result};
use golem_devices::Platform;
use golem_driver::common::{parse_hierarchy, CompanionClient};
use golem_element::{filter_viewport, Element, Viewport};

use crate::cli::TreeArgs;

/// Run the `golem tree` command: fetch and display the UI hierarchy.
pub async fn run(args: &TreeArgs) -> Result<()> {
    let platform_filter = args.platform.as_deref().map(|p| match p {
        "ios" => Platform::Ios,
        "android" => Platform::Android,
        _ => {
            eprintln!("Unknown platform: {p}. Use 'ios' or 'android'.");
            std::process::exit(1);
        }
    });

    // Scan for running companions
    let companions = crate::suite::scan_companions_public().await;

    if companions.is_empty() {
        bail!("No running companions found. Start a test or launch a companion first.");
    }

    // Filter by platform
    let companions: Vec<_> = companions
        .into_iter()
        .filter(|(_, health)| {
            if let Some(filter) = platform_filter {
                let platform_str = match filter {
                    Platform::Ios => "ios",
                    Platform::Android => "android",
                };
                health.platform == platform_str
            } else {
                true
            }
        })
        .collect();

    // Filter by device name/UDID
    let companions: Vec<_> = if let Some(ref filter) = args.device {
        let f = filter.to_lowercase();
        companions
            .into_iter()
            .filter(|(_, health)| {
                health.device_name.to_lowercase().contains(&f)
                    || health.device_id.to_lowercase().contains(&f)
            })
            .collect()
    } else {
        companions
    };

    if companions.is_empty() {
        bail!("No matching companions found.");
    }

    for (port, health) in &companions {
        let platform = &health.platform;
        let name = &health.device_name;

        let mut client = CompanionClient::new(*port);
        // iOS needs bundle_id to target the correct app's hierarchy
        if platform == "ios" {
            if let Some(ref bundle) = args.bundle {
                client.default_query = format!("bundle_id={bundle}");
            }
        }

        let text = match client.get_text("/hierarchy").await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("  {name} ({platform}, port {port}): failed to fetch hierarchy: {e}");
                continue;
            }
        };

        let root = match parse_hierarchy(&text) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  {name} ({platform}, port {port}): failed to parse hierarchy: {e}");
                continue;
            }
        };

        println!("── {name} ({platform}, port {port}) ──");

        if args.json {
            let display = if args.full {
                root
            } else {
                let vp = Viewport::from_root(&root);
                filter_viewport(&root, &vp)
            };
            if let Ok(json) = serde_json::to_string_pretty(&display) {
                println!("{json}");
            }
        } else {
            let display = if args.full {
                root
            } else {
                let vp = Viewport::from_root(&root);
                filter_viewport(&root, &vp)
            };
            print_tree(&display, 0);
        }
        println!();
    }

    Ok(())
}

fn print_tree(element: &Element, depth: usize) {
    let indent = "  ".repeat(depth);
    let text = element.text.as_deref().unwrap_or("");
    let id = element
        .accessibility_label
        .as_deref()
        .map(|s| format!(" id={s}"))
        .unwrap_or_default();
    let et = &element.element_type;
    let b = &element.bounds;

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

    // Show elements that have text, id, or are structural containers
    if !text.is_empty() || !id.is_empty() {
        println!(
            "{indent}{et} \"{text}\"{id} ({},{} {}x{}){state}",
            b.x, b.y, b.width, b.height
        );
    } else if !element.children.is_empty() {
        println!(
            "{indent}{et} ({},{} {}x{})",
            b.x, b.y, b.width, b.height
        );
    }

    for child in &element.children {
        print_tree(child, depth + 1);
    }
}
