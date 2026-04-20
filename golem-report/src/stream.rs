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

/// Stream events to stderr in human-readable format.
pub async fn stream_human(
    mut rx: broadcast::Receiver<Event>,
    verbose: bool,
    multi_device: bool,
) {
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stderr());

    // Track current block for step prefix
    let mut current_block = String::new();
    let mut current_iteration: u32 = 0;

    while let Ok(event) = rx.recv().await {
        let dev_prefix = if multi_device {
            format!(" [{}]", event.device_id)
        } else {
            String::new()
        };

        match &event.kind {
            EventKind::FlowStarted { flow_name } => {
                if use_color {
                    eprintln!("{BLUE}{SYM_FLOW} {flow_name}{dev_prefix}{RESET}");
                } else {
                    eprintln!("{SYM_FLOW} {flow_name}{dev_prefix}");
                }
            }
            EventKind::BlockStarted { block_name, iteration, .. } => {
                current_block = block_name.clone();
                current_iteration = *iteration;
                let iter_suffix = if *iteration > 0 {
                    format!(" (iteration {iteration})")
                } else {
                    String::new()
                };
                if use_color {
                    eprintln!("  {CYAN}\u{2500}\u{2500} {block_name}{iter_suffix} \u{2500}\u{2500}{RESET}");
                } else {
                    eprintln!("  \u{2500}\u{2500} {block_name}{iter_suffix} \u{2500}\u{2500}");
                }
            }
            EventKind::StepStarted { global_step_index, step_index_in_block, action, selector_label, .. } => {
                let target_str = if selector_label.is_empty() {
                    String::new()
                } else {
                    format!(" {selector_label}")
                };
                let block_tag = if current_iteration > 0 {
                    format!("{current_block}:{current_iteration}")
                } else {
                    current_block.clone()
                };
                if use_color {
                    eprintln!("  {DIM}[{global_step_index}][{block_tag}][{step_index_in_block}]{RESET}{dev_prefix} {BOLD}{action}{RESET}{target_str}");
                } else {
                    eprintln!("  [{global_step_index}][{block_tag}][{step_index_in_block}]{dev_prefix} {action}{target_str}");
                }
            }
            EventKind::StepFinished { outcome, duration_ms, .. } => {
                if use_color {
                    match outcome {
                        golem_events::StepOutcome::Success => {
                            eprintln!("      {GREEN}{SYM_SUCCESS}{RESET}  {DIM}[{duration_ms}ms]{RESET}");
                        }
                        golem_events::StepOutcome::Failed(msg) => {
                            eprintln!("      {BRIGHT_RED}{SYM_FAILED} FAIL  [{duration_ms}ms]{RESET}");
                            eprintln!("      {BRIGHT_RED}{msg}{RESET}");
                        }
                        golem_events::StepOutcome::Warning(msg) => {
                            eprintln!("      {BRIGHT_YELLOW}{SYM_WARNING}{RESET}  {DIM}[{duration_ms}ms]{RESET}");
                            eprintln!("      {YELLOW}{msg}{RESET}");
                        }
                        golem_events::StepOutcome::Skipped | golem_events::StepOutcome::Ignored => {
                            eprintln!("      {DIM}{SYM_SKIPPED}  [{duration_ms}ms]{RESET}");
                        }
                    }
                } else {
                    match outcome {
                        golem_events::StepOutcome::Success => {
                            eprintln!("      {SYM_SUCCESS}  [{duration_ms}ms]");
                        }
                        golem_events::StepOutcome::Failed(msg) => {
                            eprintln!("      {SYM_FAILED} FAIL  [{duration_ms}ms]");
                            eprintln!("      {msg}");
                        }
                        golem_events::StepOutcome::Warning(msg) => {
                            eprintln!("      {SYM_WARNING}  [{duration_ms}ms]");
                            eprintln!("      {msg}");
                        }
                        golem_events::StepOutcome::Skipped | golem_events::StepOutcome::Ignored => {
                            eprintln!("      {SYM_SKIPPED}  [{duration_ms}ms]");
                        }
                    }
                }
            }
            EventKind::Substep(sub) if verbose => {
                print_substep(&dev_prefix, sub, use_color);
            }
            EventKind::FlowFinished { flow_name, success, duration_ms } => {
                let secs = *duration_ms as f64 / 1000.0;
                eprintln!();
                if use_color {
                    if *success {
                        eprintln!("  {GREEN}{SYM_SUCCESS} PASSED{RESET}  {flow_name}  {DIM}[{secs:.1}s]{RESET}");
                    } else {
                        eprintln!("  {BRIGHT_RED}{SYM_FAILED} FAILED{RESET}  {flow_name}  {DIM}[{secs:.1}s]{RESET}");
                    }
                } else {
                    let sym = if *success { SYM_SUCCESS } else { SYM_FAILED };
                    let label = if *success { "PASSED" } else { "FAILED" };
                    eprintln!("  {sym} {label}  {flow_name}  [{secs:.1}s]");
                }
            }
            _ => {}
        }
    }
}

fn print_substep(dev_prefix: &str, sub: &SubstepEvent, use_color: bool) {
    let b = SYM_BULLET;
    let (d, r) = if use_color { (DIM, RESET) } else { ("", "") };

    match sub {
        SubstepEvent::ElementResolved { selector, bounds, tap_point } => {
            eprintln!("      {d}{b}{dev_prefix} element_resolved \"{selector}\" bounds=({},{},{},{}) tap=({},{}){r}",
                bounds.x, bounds.y, bounds.width, bounds.height, tap_point.x, tap_point.y);
        }
        SubstepEvent::ElementNotFound { selector, timeout_ms } => {
            if use_color {
                eprintln!("      {BRIGHT_RED}{b}{dev_prefix} element_not_found \"{selector}\" after {timeout_ms}ms{RESET}");
            } else {
                eprintln!("      {b}{dev_prefix} element_not_found \"{selector}\" after {timeout_ms}ms");
            }
        }
        SubstepEvent::Tap { point, .. } => {
            eprintln!("      {d}{b}{dev_prefix} tap ({},{}){r}", point.x, point.y);
        }
        SubstepEvent::DoubleTap { point, .. } => {
            eprintln!("      {d}{b}{dev_prefix} double_tap ({},{}){r}", point.x, point.y);
        }
        SubstepEvent::TextInput { text, .. } => {
            eprintln!("      {d}{b}{dev_prefix} text_input \"{text}\"{r}");
        }
        SubstepEvent::Swipe { from, to, .. } => {
            eprintln!("      {d}{b}{dev_prefix} swipe ({},{})→({},{}){r}", from.x, from.y, to.x, to.y);
        }
        SubstepEvent::ScrollStarted { selector, direction } => {
            eprintln!("      {d}{b}{dev_prefix} scroll_started \"{selector}\" direction={direction}{r}");
        }
        SubstepEvent::ScrollAttempt { attempt, direction, strategy_index, from, to, result } => {
            let result_str = match result {
                ScrollAttemptResult::PageScrolled => "page_scrolled",
                ScrollAttemptResult::InnerScrollableDetected => "inner_scrollable",
                ScrollAttemptResult::Stall { count, max } => {
                    eprintln!("      {d}{b}{dev_prefix} scroll_attempt #{attempt} strategy={} {direction} ({},{})→({},{}) stall {count}/{max}{r}",
                        strategy_index + 1, from.x, from.y, to.x, to.y);
                    return;
                }
                ScrollAttemptResult::BoundaryReached => "boundary",
            };
            eprintln!("      {d}{b}{dev_prefix} scroll_attempt #{attempt} strategy={} {direction} ({},{})→({},{}) {result_str}{r}",
                strategy_index + 1, from.x, from.y, to.x, to.y);
        }
        SubstepEvent::ScrollFound { selector, position, total_attempts } => {
            eprintln!("      {d}{b}{dev_prefix} scroll_found \"{selector}\" at ({},{}) after {total_attempts} attempts{r}",
                position.x, position.y);
        }
        SubstepEvent::ScrollDirectionReversed { to_direction, reason } => {
            eprintln!("      {d}{b}{dev_prefix} scroll_reversed →{to_direction} {reason}{r}");
        }
        SubstepEvent::ScrollStrategySwitch { to_index, reason } => {
            eprintln!("      {d}{b}{dev_prefix} scroll_strategy_switch →{} {reason}{r}", to_index + 1);
        }
        SubstepEvent::AppLaunch { bundle, duration_ms } => {
            eprintln!("      {d}{b}{dev_prefix} app_launch bundle={bundle} {duration_ms}ms{r}");
        }
        SubstepEvent::AppStop { bundle } => {
            eprintln!("      {d}{b}{dev_prefix} app_stop bundle={bundle}{r}");
        }
        SubstepEvent::RetryAttempt { attempt, max, delay_ms, error } => {
            if use_color {
                eprintln!("      {YELLOW}{b}{dev_prefix} retry {attempt}/{max} delay={delay_ms}ms: {error}{RESET}");
            } else {
                eprintln!("      {b}{dev_prefix} retry {attempt}/{max} delay={delay_ms}ms: {error}");
            }
        }
        SubstepEvent::HttpRequest { method, url, status, duration_ms } => {
            let status_str = status.map(|s| s.to_string()).unwrap_or_else(|| "?".to_string());
            eprintln!("      {d}{b}{dev_prefix} http {method} {url} → {status_str} [{duration_ms}ms]{r}");
        }
        SubstepEvent::BashCommand { command, exit_code, duration_ms } => {
            let code = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
            eprintln!("      {d}{b}{dev_prefix} bash \"{command}\" exit={code} [{duration_ms}ms]{r}");
        }
        SubstepEvent::Screenshot { path } => {
            eprintln!("      {d}{b}{dev_prefix} screenshot {path}{r}");
        }
        _ => {}
    }
}
