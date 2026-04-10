use anyhow::{bail, Result};
use golem_driver::android::AndroidDriver;
use golem_driver::ios::IosDriver;
use golem_driver::PlatformDriver;
use golem_element::{filter_viewport, Element, Viewport};

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

    // Scan for running companions
    let companions = crate::suite::scan_companions_public().await;

    if companions.is_empty() {
        bail!("No running companions found. Start a test or launch a companion first.");
    }

    // Filter by platform
    let companions: Vec<_> = companions
        .into_iter()
        .filter(|(_, health)| {
            platform_filter.is_none_or(|f| health.platform == f)
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
        let bundle = args.bundle.as_deref().unwrap_or("fail.golem.test");

        // Create the appropriate driver — same code path as test execution,
        // including CDP enrichment for Android WebViews.
        let device_id = find_device_id(platform, name).await;
        let driver: Box<dyn PlatformDriver> = match platform.as_str() {
            "android" => Box::new(AndroidDriver::new(
                device_id.clone(),
                bundle.to_string(),
                *port,
            )),
            _ => Box::new(IosDriver::new(
                device_id.clone(),
                bundle.to_string(),
                *port,
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

        // If Android, wait for CDP setup and fetch again with enrichment.
        let root = if platform == "android" {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            driver.get_hierarchy().await.map(|(r, _)| r).unwrap_or(root)
        } else {
            root
        };

        println!("── {name} ({platform}, port {port}) ──");

        if args.debug {
            println!("  device_id: {device_id}");
            println!("  bundle: {bundle}");
            if meta.keyboard_height > 0 {
                println!("  keyboard: open ({}px)", meta.keyboard_height);
            } else {
                println!("  keyboard: closed");
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
        } else {
            print_tree(&display, 0);
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
    root.children.iter().any(|c| has_webview_element(c))
}

fn print_tree(element: &Element, depth: usize) {
    let indent = "  ".repeat(depth);
    let text = element.text.as_deref().unwrap_or("");
    let label = element
        .accessibility_label
        .as_deref()
        .filter(|s| !s.is_empty() && Some(*s) != element.text.as_deref())
        .map(|s| format!(" label={s}"))
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

    if !text.is_empty() || !label.is_empty() {
        println!(
            "{indent}{et} \"{text}\"{label} ({},{} {}x{}){state}",
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
