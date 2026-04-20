use std::collections::HashMap;
use golem_events::{DeviceId, Event, EventKind, SubstepEvent, ScrollAttemptResult};
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

/// Get or assign a circled number for a device.
fn device_prefix(
    device_id: &DeviceId,
    device_map: &mut HashMap<String, usize>,
    multi_device: bool,
    use_color: bool,
) -> String {
    if !multi_device {
        return String::new();
    }
    let next_idx = device_map.len();
    let idx = *device_map.entry(device_id.0.clone()).or_insert(next_idx);
    let num = CIRCLED_NUMBERS.get(idx).unwrap_or(&"?");
    if use_color {
        let color = DEVICE_COLORS.get(idx % DEVICE_COLORS.len()).unwrap_or(&"");
        format!("{DIM}{color}{num}{RESET} ")
    } else {
        format!("{num} ")
    }
}

/// Stream events to stderr in human-readable format.
pub async fn stream_human(
    mut rx: broadcast::Receiver<Event>,
    verbose: bool,
    multi_device: bool,
) {
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());

    // Device ID → index mapping for circled numbers
    let mut device_map: HashMap<String, usize> = HashMap::new();
    // Track current block per device
    let mut current_blocks: HashMap<String, (String, u32)> = HashMap::new();

    while let Ok(event) = rx.recv().await {
        let dp = device_prefix(&event.device_id, &mut device_map, multi_device, use_color);

        match &event.kind {
            EventKind::FlowStarted { flow_name } => {
                if use_color {
                    eprintln!("{dp}{BLUE}{SYM_FLOW} {flow_name}{RESET}");
                } else {
                    eprintln!("{dp}{SYM_FLOW} {flow_name}");
                }
                // Print device legend on first flow start in multi-device mode
                if multi_device && device_map.len() <= 2 {
                    let idx = device_map.get(&event.device_id.0).copied().unwrap_or(0);
                    let num = CIRCLED_NUMBERS.get(idx).unwrap_or(&"?");
                    if use_color {
                        let color = DEVICE_COLORS.get(idx % DEVICE_COLORS.len()).unwrap_or(&"");
                        eprintln!("  {DIM}{color}{num} {}{RESET}", event.device_id);
                    } else {
                        eprintln!("  {num} {}", event.device_id);
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
                    eprintln!("  {dp}{CYAN}\u{2500}\u{2500} {block_name}{iter_suffix} \u{2500}\u{2500}{RESET}");
                } else {
                    eprintln!("  {dp}\u{2500}\u{2500} {block_name}{iter_suffix} \u{2500}\u{2500}");
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
                    eprintln!("  {dp}{DIM}[{global_step_index}][{block_tag}][{step_index_in_block}]{RESET} {BOLD}{action}{RESET}{target_str}");
                } else {
                    eprintln!("  {dp}[{global_step_index}][{block_tag}][{step_index_in_block}] {action}{target_str}");
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
                            eprintln!("  {dp}    {GREEN}{SYM_SUCCESS}{RESET}  {DIM}[{duration_ms}ms]{RESET}{stats_str}");
                        }
                        golem_events::StepOutcome::Failed(msg) => {
                            eprintln!("  {dp}    {BRIGHT_RED}{SYM_FAILED} FAIL  [{duration_ms}ms]{RESET}");
                            eprintln!("  {dp}    {BRIGHT_RED}{msg}{RESET}");
                        }
                        golem_events::StepOutcome::Warning(msg) => {
                            eprintln!("  {dp}    {BRIGHT_YELLOW}{SYM_WARNING}{RESET}  {DIM}[{duration_ms}ms]{RESET}");
                            eprintln!("  {dp}    {YELLOW}{msg}{RESET}");
                        }
                        golem_events::StepOutcome::Skipped | golem_events::StepOutcome::Ignored => {
                            eprintln!("  {dp}    {DIM}{SYM_SKIPPED}  [{duration_ms}ms]{RESET}");
                        }
                    }
                } else {
                    match outcome {
                        golem_events::StepOutcome::Success => {
                            eprintln!("  {dp}    {SYM_SUCCESS}  [{duration_ms}ms]{stats_str}");
                        }
                        golem_events::StepOutcome::Failed(msg) => {
                            eprintln!("  {dp}    {SYM_FAILED} FAIL  [{duration_ms}ms]");
                            eprintln!("  {dp}    {msg}");
                        }
                        golem_events::StepOutcome::Warning(msg) => {
                            eprintln!("  {dp}    {SYM_WARNING}  [{duration_ms}ms]");
                            eprintln!("  {dp}    {msg}");
                        }
                        golem_events::StepOutcome::Skipped | golem_events::StepOutcome::Ignored => {
                            eprintln!("  {dp}    {SYM_SKIPPED}  [{duration_ms}ms]");
                        }
                    }
                }
            }
            EventKind::Substep(sub) if verbose => {
                print_substep(&dp, sub, use_color);
            }
            EventKind::FlowFinished { flow_name, success, duration_ms } => {
                let secs = *duration_ms as f64 / 1000.0;
                eprintln!();
                if use_color {
                    if *success {
                        eprintln!("  {dp}{GREEN}{SYM_SUCCESS} PASSED{RESET}  {flow_name}  {DIM}[{secs:.1}s]{RESET}");
                    } else {
                        eprintln!("  {dp}{BRIGHT_RED}{SYM_FAILED} FAILED{RESET}  {flow_name}  {DIM}[{secs:.1}s]{RESET}");
                    }
                } else {
                    let sym = if *success { SYM_SUCCESS } else { SYM_FAILED };
                    let label = if *success { "PASSED" } else { "FAILED" };
                    eprintln!("  {dp}{sym} {label}  {flow_name}  [{secs:.1}s]");
                }
            }
            EventKind::SuiteFinished { duration_ms, passed, failed } => {
                let secs = *duration_ms as f64 / 1000.0;
                eprintln!();
                if use_color {
                    eprintln!("{DIM}──────────────────────────────────────{RESET}");
                } else {
                    eprintln!("──────────────────────────────────────");
                }
                eprintln!("Suite: {passed} passed, {failed} failed  [{secs:.1}s]");
            }
            _ => {}
        }
    }
}

fn print_substep(dp: &str, sub: &SubstepEvent, use_color: bool) {
    let b = SYM_BULLET;
    let (d, r) = if use_color { (DIM, RESET) } else { ("", "") };

    match sub {
        SubstepEvent::ElementResolved { selector, bounds, tap_point } => {
            eprintln!("  {dp}    {d}{b} element_resolved \"{selector}\" bounds=({},{},{},{}) tap=({},{}){r}",
                bounds.x, bounds.y, bounds.width, bounds.height, tap_point.x, tap_point.y);
        }
        SubstepEvent::ElementNotFound { selector, timeout_ms } => {
            eprintln!("  {dp}    {d}{b} element_not_found \"{selector}\" after {timeout_ms}ms{r}");
        }
        SubstepEvent::Tap { point, .. } => {
            eprintln!("  {dp}    {d}{b} tap ({},{}){r}", point.x, point.y);
        }
        SubstepEvent::DoubleTap { point, .. } => {
            eprintln!("  {dp}    {d}{b} double_tap ({},{}){r}", point.x, point.y);
        }
        SubstepEvent::LongPress { point, duration_ms, .. } => {
            eprintln!("  {dp}    {d}{b} long_press ({},{}) {}ms{r}", point.x, point.y, duration_ms);
        }
        SubstepEvent::TextInput { text, .. } => {
            eprintln!("  {dp}    {d}{b} text_input \"{text}\"{r}");
        }
        SubstepEvent::Backspace { count } => {
            eprintln!("  {dp}    {d}{b} backspace ×{count}{r}");
        }
        SubstepEvent::Swipe { from, to, .. } => {
            eprintln!("  {dp}    {d}{b} swipe ({},{})→({},{}){r}", from.x, from.y, to.x, to.y);
        }
        SubstepEvent::ScrollStarted { selector, direction } => {
            eprintln!("  {dp}    {d}{b} scroll_started \"{selector}\" direction={direction}{r}");
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
            eprintln!("  {dp}      {d}[scroll] {dir_arrow} strategy {} ({},{})→({},{}) → {result_str} {stats}{r}",
                strategy_index + 1, from.x, from.y, to.x, to.y);
        }
        SubstepEvent::ScrollFound { selector, position, total_attempts } => {
            eprintln!("  {dp}    {d}{b} scroll_found \"{selector}\" at ({},{}) after {total_attempts} attempts{r}",
                position.x, position.y);
        }
        SubstepEvent::ScrollDirectionReversed { to_direction, reason } => {
            eprintln!("  {dp}    {d}{b} scroll_reversed →{to_direction} {reason}{r}");
        }
        SubstepEvent::ScrollStrategySwitch { to_index, reason } => {
            eprintln!("  {dp}    {d}{b} scroll_strategy_switch →{} {reason}{r}", to_index + 1);
        }
        SubstepEvent::AppLaunch { bundle, duration_ms } => {
            eprintln!("  {dp}    {d}{b} app_launch bundle={bundle} {duration_ms}ms{r}");
        }
        SubstepEvent::AppStop { bundle } => {
            eprintln!("  {dp}    {d}{b} app_stop bundle={bundle}{r}");
        }
        SubstepEvent::RetryAttempt { attempt, max, delay_ms, error } => {
            eprintln!("  {dp}    {d}{b} retry {attempt}/{max} delay={delay_ms}ms: {error}{r}");
        }
        SubstepEvent::HttpRequest { method, url, status, duration_ms } => {
            let s = status.map(|s| s.to_string()).unwrap_or_else(|| "?".to_string());
            eprintln!("  {dp}    {d}{b} http {method} {url} → {s} [{duration_ms}ms]{r}");
        }
        SubstepEvent::BashCommand { command, exit_code, duration_ms } => {
            let code = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
            eprintln!("  {dp}    {d}{b} bash \"{command}\" exit={code} [{duration_ms}ms]{r}");
        }
        SubstepEvent::Screenshot { path } => {
            eprintln!("  {dp}    {d}{b} screenshot {path}{r}");
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
