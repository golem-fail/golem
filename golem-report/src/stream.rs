use std::collections::HashMap;
use std::time::SystemTime;

use chrono::{DateTime, Local};
use golem_events::{Event, EventKind, SubstepEvent, ScrollAttemptResult};
use tokio::sync::broadcast;

const SYM_SUCCESS: &str = "\u{2713}";  // ✓
const SYM_FAILED: &str = "\u{2717}";   // ✗
const SYM_WARNING: &str = "\u{26A0}";  // ⚠
const SYM_SKIPPED: &str = "\u{2212}";  // −
const SYM_FLOW: &str = "\u{25B6}";     // ▶
const SYM_BULLET: &str = "\u{2219}";   // ∙

// ANSI color codes — muted palette, bright reserved for errors
const DIM: &str = "\x1b[2m";           // dim/faint — indices, timing, structural
const RESET: &str = "\x1b[0m";
const GREEN: &str = "\x1b[32m";        // success (not bright)
const YELLOW: &str = "\x1b[33m";       // warning (not bright)
const BRIGHT_RED: &str = "\x1b[1;31m"; // FAIL — bright, stands out
const BRIGHT_YELLOW: &str = "\x1b[1;33m"; // warning symbol — bright
const CYAN: &str = "\x1b[36m";         // block headers
const BLUE: &str = "\x1b[34m";         // flow name
const BOLD: &str = "\x1b[1m";         // action names — bold pops against dim indices

/// Circled number symbols ① through ㊿ for device identification.
const CIRCLED_NUMBERS: &[&str] = &[
    "①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩",
    "⑪", "⑫", "⑬", "⑭", "⑮", "⑯", "⑰", "⑱", "⑲", "⑳",
    "㉑", "㉒", "㉓", "㉔", "㉕", "㉖", "㉗", "㉘", "㉙", "㉚",
    "㉛", "㉜", "㉝", "㉞", "㉟", "㊱", "㊲", "㊳", "㊴", "㊵",
    "㊶", "㊷", "㊸", "㊹", "㊺", "㊻", "㊼", "㊽", "㊾", "㊿",
];

/// Dim ANSI colors for device prefixes — subtle, won't clash with status colors.
const DEVICE_COLORS: &[&str] = &[
    "\x1b[36m",  // cyan
    "\x1b[35m",  // magenta
    "\x1b[33m",  // yellow (dim)
    "\x1b[34m",  // blue
    "\x1b[32m",  // green (dim)
    "\x1b[91m",  // bright red (for device ID only)
];

/// Render an event's wall-clock time as `HH:MM:SS.mmm ` (trailing space),
/// dimmed when the terminal supports colour. One prefix per rendered line
/// so live output has a consistent time column left of the device circle.
fn format_timestamp(wall_time: SystemTime, use_color: bool) -> String {
    let dt: DateTime<Local> = wall_time.into();
    let stamp = dt.format("%H:%M:%S%.3f");
    if use_color {
        format!("{DIM}{stamp}{RESET} ")
    } else {
        format!("{stamp} ")
    }
}

/// Get or assign a circled number for a device.
fn format_circle(idx: usize, multi_device: bool, use_color: bool) -> String {
    if !multi_device {
        return String::new();
    }
    let num = CIRCLED_NUMBERS.get(idx).unwrap_or(&"?");
    if use_color {
        let color = DEVICE_COLORS.get(idx % DEVICE_COLORS.len()).unwrap_or(&"");
        format!("{DIM}{color}{num}{RESET} ")
    } else {
        format!("{num} ")
    }
}

/// Stream events to stderr in human-readable format.
///
/// `debug` enables per-line install script output. When false, install
/// output is silent on success and shows only the tail on failure (from
/// the error payload). The installer still captures stderr internally
/// for the failure tail.
pub async fn stream_human(
    mut rx: broadcast::Receiver<Event>,
    verbose: bool,
    multi_device: bool,
    debug: bool,
) {
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());

    // Circle-slot allocation: each FlowStarted grabs a fresh slot so two
    // sequential flows on the same device get distinct circles. Events
    // between two FlowStarteds on the same device inherit that device's
    // currently-assigned slot. Pre-FlowStarted events (e.g. install) get
    // allocated on-demand per device_id.
    let mut current_slot: HashMap<String, usize> = HashMap::new();
    let mut next_slot: usize = 0;
    // Legacy device_map retained for legend printing on FlowStarted.
    let mut device_map: HashMap<String, usize> = HashMap::new();
    // Track current block per device
    let mut current_blocks: HashMap<String, (String, u32)> = HashMap::new();

    while let Ok(event) = rx.recv().await {
        let ts = format_timestamp(event.wall_time, use_color);
        let is_flow_started = matches!(event.kind, EventKind::FlowStarted { .. });
        let dp = if !multi_device || event.device_id.0 == "suite" {
            String::new()
        } else if is_flow_started {
            // FlowStarted always claims a fresh slot for this device —
            // so sequential flows on the same device get distinct circles.
            let idx = next_slot;
            next_slot += 1;
            current_slot.insert(event.device_id.0.clone(), idx);
            device_map.insert(event.device_id.0.clone(), idx);
            format_circle(idx, multi_device, use_color)
        } else if let Some(&slot) = current_slot.get(&event.device_id.0) {
            // Non-flow event on a device that has already entered a flow —
            // inherit that flow's circle (step events, install output
            // during a flow, etc.).
            format_circle(slot, multi_device, use_color)
        } else {
            // Pre-flow events (installs, etc.) get no circle so they don't
            // consume numbers that should map to flow runs.
            String::new()
        };

        match &event.kind {
            EventKind::SuitePlanned { flow_runs, install_entries, device_availability } => {
                // Only render under --verbose; diagnostic view of plan output.
                if verbose {
                    eprintln!("{ts}  [plan] {} flow run(s):", flow_runs.len());
                    for line in flow_runs {
                        eprintln!("{ts}  [plan]   {line}");
                    }
                    if !install_entries.is_empty() {
                        eprintln!(
                            "{ts}  [plan] install matrix ({} entr{}):",
                            install_entries.len(),
                            if install_entries.len() == 1 { "y" } else { "ies" }
                        );
                        for line in install_entries {
                            eprintln!("{ts}  [plan]   {line}");
                        }
                    }
                    if !device_availability.is_empty() {
                        eprintln!(
                            "{ts}  [devices] {} slot requirement(s):",
                            device_availability.len()
                        );
                        for line in device_availability {
                            eprintln!("{ts}  [devices]   · {line}");
                        }
                    }
                }
            }
            EventKind::FlowStarted { flow_name, .. } => {
                if use_color {
                    eprintln!("{ts}{dp}{BLUE}{SYM_FLOW} {flow_name}{RESET}");
                } else {
                    eprintln!("{ts}{dp}{SYM_FLOW} {flow_name}");
                }
                // Print device legend on first flow start in multi-device mode
                if multi_device && device_map.len() <= 2 {
                    let idx = device_map.get(&event.device_id.0).copied().unwrap_or(0);
                    let num = CIRCLED_NUMBERS.get(idx).unwrap_or(&"?");
                    if use_color {
                        let color = DEVICE_COLORS.get(idx % DEVICE_COLORS.len()).unwrap_or(&"");
                        eprintln!("{ts}  {DIM}{color}{num} {}{RESET}", event.device_id);
                    } else {
                        eprintln!("{ts}  {num} {}", event.device_id);
                    }
                }
            }
            EventKind::BlockStarted { block_name, iteration, .. } => {
                current_blocks.insert(event.device_id.0.clone(), (block_name.clone(), *iteration));
                let iter_suffix = if *iteration > 0 {
                    format!(" (iteration {iteration})")
                } else {
                    String::new()
                };
                if use_color {
                    eprintln!("{ts}  {dp}{CYAN}\u{2500}\u{2500} {block_name}{iter_suffix} \u{2500}\u{2500}{RESET}");
                } else {
                    eprintln!("{ts}  {dp}\u{2500}\u{2500} {block_name}{iter_suffix} \u{2500}\u{2500}");
                }
            }
            EventKind::StepStarted { global_step_index, step_index_in_block, action, selector_label, .. } => {
                let target_str = if selector_label.is_empty() {
                    String::new()
                } else {
                    format!(" {selector_label}")
                };
                let (block_name, iteration) = current_blocks
                    .get(&event.device_id.0)
                    .cloned()
                    .unwrap_or_default();
                let block_tag = if iteration > 0 {
                    format!("{block_name}:{iteration}")
                } else {
                    block_name
                };
                if use_color {
                    eprintln!("{ts}  {dp}{DIM}[{global_step_index}][{block_tag}][{step_index_in_block}]{RESET} {BOLD}{action}{RESET}{target_str}");
                } else {
                    eprintln!("{ts}  {dp}[{global_step_index}][{block_tag}][{step_index_in_block}] {action}{target_str}");
                }
            }
            EventKind::StepFinished { outcome, duration_ms, tree_stats, .. } => {
                let stats_text = if verbose && tree_stats.fetches > 0 {
                    format_tree_stats(tree_stats)
                } else {
                    String::new()
                };
                let stats_str = if !stats_text.is_empty() && use_color {
                    format!(" \x1b[2;90m{stats_text}{RESET}")
                } else if !stats_text.is_empty() {
                    format!(" {stats_text}")
                } else {
                    String::new()
                };
                if use_color {
                    match outcome {
                        golem_events::StepOutcome::Success => {
                            eprintln!("{ts}  {dp}    {GREEN}{SYM_SUCCESS}{RESET}  {DIM}[{duration_ms}ms]{RESET}{stats_str}");
                        }
                        golem_events::StepOutcome::Failed(msg) => {
                            eprintln!("{ts}  {dp}    {BRIGHT_RED}{SYM_FAILED} FAIL  [{duration_ms}ms]{RESET}");
                            eprintln!("{ts}  {dp}    {BRIGHT_RED}{msg}{RESET}");
                        }
                        golem_events::StepOutcome::Warning(msg) => {
                            eprintln!("{ts}  {dp}    {BRIGHT_YELLOW}{SYM_WARNING}{RESET}  {DIM}[{duration_ms}ms]{RESET}");
                            eprintln!("{ts}  {dp}    {YELLOW}{msg}{RESET}");
                        }
                        golem_events::StepOutcome::Skipped | golem_events::StepOutcome::Ignored => {
                            eprintln!("{ts}  {dp}    {DIM}{SYM_SKIPPED}  [{duration_ms}ms]{RESET}");
                        }
                    }
                } else {
                    match outcome {
                        golem_events::StepOutcome::Success => {
                            eprintln!("{ts}  {dp}    {SYM_SUCCESS}  [{duration_ms}ms]{stats_str}");
                        }
                        golem_events::StepOutcome::Failed(msg) => {
                            eprintln!("{ts}  {dp}    {SYM_FAILED} FAIL  [{duration_ms}ms]");
                            eprintln!("{ts}  {dp}    {msg}");
                        }
                        golem_events::StepOutcome::Warning(msg) => {
                            eprintln!("{ts}  {dp}    {SYM_WARNING}  [{duration_ms}ms]");
                            eprintln!("{ts}  {dp}    {msg}");
                        }
                        golem_events::StepOutcome::Skipped | golem_events::StepOutcome::Ignored => {
                            eprintln!("{ts}  {dp}    {SYM_SKIPPED}  [{duration_ms}ms]");
                        }
                    }
                }
            }
            EventKind::Substep(sub) if verbose => {
                print_substep(&ts, &dp, sub, use_color);
            }
            EventKind::FlowFinished { flow_name, success, duration_ms, seed, .. } => {
                let secs = *duration_ms as f64 / 1000.0;
                eprintln!();
                if use_color {
                    if *success {
                        eprintln!("{ts}  {dp}{GREEN}{SYM_SUCCESS} PASSED{RESET}  {flow_name}  {DIM}[{secs:.1}s]  seed:{seed}{RESET}");
                    } else {
                        eprintln!("{ts}  {dp}{BRIGHT_RED}{SYM_FAILED} FAILED{RESET}  {flow_name}  {DIM}[{secs:.1}s]  seed:{seed}{RESET}");
                    }
                } else {
                    let sym = if *success { SYM_SUCCESS } else { SYM_FAILED };
                    let label = if *success { "PASSED" } else { "FAILED" };
                    eprintln!("{ts}  {dp}{sym} {label}  {flow_name}  [{secs:.1}s]  seed:{seed}");
                }
            }
            EventKind::SuiteFinished { duration_ms, passed, failed, skipped } => {
                let secs = *duration_ms as f64 / 1000.0;
                eprintln!();
                if use_color {
                    eprintln!("{ts}{DIM}──────────────────────────────────────{RESET}");
                } else {
                    eprintln!("{ts}──────────────────────────────────────");
                }
                let skip_suffix = if *skipped > 0 {
                    format!(", {skipped} skipped")
                } else {
                    String::new()
                };
                eprintln!("{ts}Suite: {passed} passed, {failed} failed{skip_suffix}  [{secs:.1}s]");
            }
            EventKind::InstallStarted { app_name, bundle_id, target, .. } => {
                // Script may build + install, or just install (install-only mode).
                // We can't tell which from events; "building and installing" covers both.
                if use_color {
                    eprintln!("{ts}  {dp}{DIM}[install {app_name}] building and installing {bundle_id} on {target}...{RESET}");
                } else {
                    eprintln!("{ts}  {dp}[install {app_name}] building and installing {bundle_id} on {target}...");
                }
            }
            EventKind::InstallOutput { app_name, line } => {
                // Only stream per-line script stderr under --debug.
                // Otherwise the install is silent between "building..." and
                // the final success/failure line.
                if debug {
                    if use_color {
                        eprintln!("{ts}  {dp}{DIM}[install {app_name}]{RESET} {line}");
                    } else {
                        eprintln!("{ts}  {dp}[install {app_name}] {line}");
                    }
                }
            }
            EventKind::InstallFinished { app_name, bundle_id, success, duration_ms, error, target, .. } => {
                let secs = *duration_ms as f64 / 1000.0;
                if *success {
                    if use_color {
                        eprintln!("{ts}  {dp}{GREEN}{SYM_SUCCESS}{RESET} {DIM}[install {app_name}] installed {bundle_id} on {target}  [{secs:.1}s]{RESET}");
                    } else {
                        eprintln!("{ts}  {dp}{SYM_SUCCESS} [install {app_name}] installed {bundle_id} on {target}  [{secs:.1}s]");
                    }
                } else {
                    let err = error.as_deref().unwrap_or("install failed");
                    if use_color {
                        eprintln!("{ts}  {dp}{BRIGHT_RED}{SYM_FAILED}{RESET} [install {app_name}] on {target} — {err}");
                    } else {
                        eprintln!("{ts}  {dp}{SYM_FAILED} [install {app_name}] on {target} — {err}");
                    }
                }
            }
            EventKind::FlowSkipped { flow_name, reason } => {
                if use_color {
                    eprintln!("{ts}  {dp}{YELLOW}{SYM_WARNING} SKIPPED{RESET}  {flow_name}  {DIM}{reason}{RESET}");
                } else {
                    eprintln!("{ts}  {dp}{SYM_WARNING} SKIPPED  {flow_name}  {reason}");
                }
            }
            EventKind::FlowParseFailed { path, error } => {
                if use_color {
                    eprintln!("{ts}  {BRIGHT_RED}Parse error{RESET} ({path}): {error}");
                } else {
                    eprintln!("{ts}  Parse error ({path}): {error}");
                }
            }
            EventKind::DeviceAutoBoot { device_name, slot_shape } => {
                if use_color {
                    eprintln!("{ts}  {DIM}[devices] no booted match — booting {device_name} to satisfy {slot_shape}...{RESET}");
                } else {
                    eprintln!("{ts}  [devices] no booted match — booting {device_name} to satisfy {slot_shape}...");
                }
            }
            EventKind::SlotSetupFailed { slot_label, reason } => {
                if use_color {
                    eprintln!("{ts}  {BRIGHT_RED}[slot] setup failed for {slot_label}:{RESET} {reason}");
                } else {
                    eprintln!("{ts}  [slot] setup failed for {slot_label}: {reason}");
                }
            }
            EventKind::ResourcesWaiting { platform } => {
                if use_color {
                    eprintln!("{ts}  {dp}{DIM}[resources] waiting for {platform}...{RESET}");
                } else {
                    eprintln!("{ts}  {dp}[resources] waiting for {platform}...");
                }
            }
            EventKind::CompanionStarting { platform, device_name } => {
                if use_color {
                    eprintln!("{ts}  {dp}{DIM}[companion] starting on {device_name} ({platform})...{RESET}");
                } else {
                    eprintln!("{ts}  {dp}[companion] starting on {device_name} ({platform})...");
                }
            }
            EventKind::CompanionReady { platform, version, device_name, os_version } => {
                if use_color {
                    eprintln!("{ts}  {dp}{DIM}[companion] ready — {platform} v{version} on {device_name} ({os_version}){RESET}");
                } else {
                    eprintln!("{ts}  {dp}[companion] ready — {platform} v{version} on {device_name} ({os_version})");
                }
            }
            _ => {}
        }
    }
}

fn print_substep(ts: &str, dp: &str, sub: &SubstepEvent, use_color: bool) {
    let b = SYM_BULLET;
    let (d, r) = if use_color { (DIM, RESET) } else { ("", "") };

    match sub {
        SubstepEvent::ElementResolved { selector, bounds, tap_point } => {
            eprintln!("{ts}  {dp}    {d}{b} element_resolved \"{selector}\" bounds=({},{},{},{}) tap=({},{}){r}",
                bounds.x, bounds.y, bounds.width, bounds.height, tap_point.x, tap_point.y);
        }
        SubstepEvent::ElementNotFound { selector, timeout_ms } => {
            eprintln!("{ts}  {dp}    {d}{b} element_not_found \"{selector}\" after {timeout_ms}ms{r}");
        }
        SubstepEvent::Tap { point, .. } => {
            eprintln!("{ts}  {dp}    {d}{b} tap ({},{}){r}", point.x, point.y);
        }
        SubstepEvent::DoubleTap { point, .. } => {
            eprintln!("{ts}  {dp}    {d}{b} double_tap ({},{}){r}", point.x, point.y);
        }
        SubstepEvent::LongPress { point, duration_ms, .. } => {
            eprintln!("{ts}  {dp}    {d}{b} long_press ({},{}) {}ms{r}", point.x, point.y, duration_ms);
        }
        SubstepEvent::TextInput { text, .. } => {
            eprintln!("{ts}  {dp}    {d}{b} text_input \"{text}\"{r}");
        }
        SubstepEvent::Backspace { count } => {
            eprintln!("{ts}  {dp}    {d}{b} backspace ×{count}{r}");
        }
        SubstepEvent::Swipe { from, to, .. } => {
            eprintln!("{ts}  {dp}    {d}{b} swipe ({},{})→({},{}){r}", from.x, from.y, to.x, to.y);
        }
        SubstepEvent::ScrollStarted { selector, direction } => {
            eprintln!("{ts}  {dp}    {d}{b} scroll_started \"{selector}\" direction={direction}{r}");
        }
        SubstepEvent::ScrollAttempt { attempt: _, direction, strategy_index, from, to, result, tree_stats } => {
            let dir_arrow = match direction.as_str() {
                "Down" => "↓", "Up" => "↑", "Left" => "←", "Right" => "→", _ => "?",
            };
            let result_str = match result {
                ScrollAttemptResult::PageScrolled => "page scrolled".to_string(),
                ScrollAttemptResult::InnerScrollableDetected => format!("inner scrollable → strategy {}", strategy_index + 2),
                ScrollAttemptResult::Stall { count, max } => format!("stall {count}/{max}"),
                ScrollAttemptResult::BoundaryReached => "boundary reached".to_string(),
            };
            let stats = format_tree_stats(tree_stats);
            eprintln!("{ts}  {dp}      {d}[scroll] {dir_arrow} strategy {} ({},{})→({},{}) → {result_str} {stats}{r}",
                strategy_index + 1, from.x, from.y, to.x, to.y);
        }
        SubstepEvent::ScrollFound { selector, position, total_attempts } => {
            eprintln!("{ts}  {dp}    {d}{b} scroll_found \"{selector}\" at ({},{}) after {total_attempts} attempts{r}",
                position.x, position.y);
        }
        SubstepEvent::ScrollDirectionReversed { to_direction, reason } => {
            eprintln!("{ts}  {dp}    {d}{b} scroll_reversed →{to_direction} {reason}{r}");
        }
        SubstepEvent::ScrollStrategySwitch { to_index, reason } => {
            eprintln!("{ts}  {dp}    {d}{b} scroll_strategy_switch →{} {reason}{r}", to_index + 1);
        }
        SubstepEvent::AppLaunch { bundle, duration_ms } => {
            eprintln!("{ts}  {dp}    {d}{b} app_launch bundle={bundle} {duration_ms}ms{r}");
        }
        SubstepEvent::AppStop { bundle } => {
            eprintln!("{ts}  {dp}    {d}{b} app_stop bundle={bundle}{r}");
        }
        SubstepEvent::RetryAttempt { attempt, max, delay_ms, error } => {
            eprintln!("{ts}  {dp}    {d}{b} retry {attempt}/{max} delay={delay_ms}ms: {error}{r}");
        }
        SubstepEvent::HttpRequest { method, url, status, duration_ms } => {
            let s = status.map(|s| s.to_string()).unwrap_or_else(|| "?".to_string());
            eprintln!("{ts}  {dp}    {d}{b} http {method} {url} → {s} [{duration_ms}ms]{r}");
        }
        SubstepEvent::BashCommand { command, exit_code, duration_ms } => {
            let code = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
            eprintln!("{ts}  {dp}    {d}{b} bash \"{command}\" exit={code} [{duration_ms}ms]{r}");
        }
        SubstepEvent::Screenshot { path } => {
            eprintln!("{ts}  {dp}    {d}{b} screenshot {path}{r}");
        }
        _ => {}
    }
}

/// Format tree stats as dim summary: `{3 trees, 181 nodes}` or `{3 trees, 181~190 nodes}`.
fn format_tree_stats(stats: &golem_events::TreeStats) -> String {
    if stats.fetches == 0 {
        return String::new();
    }
    let nodes = if stats.min_nodes == stats.max_nodes {
        format!("{}", stats.max_nodes)
    } else {
        format!("{}~{}", stats.min_nodes, stats.max_nodes)
    };
    format!("{{{} trees, {} nodes}}", stats.fetches, nodes)
}
