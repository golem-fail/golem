use std::collections::HashMap;
use std::time::SystemTime;

use chrono::{DateTime, Local};
use golem_events::{Event, EventKind, SubstepEvent, ScrollAttemptResult};
use tokio::sync::broadcast;

const SYM_FAILED: &str = "\u{2717}";   // ✗ (install failure marker)
const SYM_BULLET: &str = "\u{2219}";   // ∙
const SEPARATOR: &str = "────────────────────────────────────────";

// ANSI color codes — muted palette, bright reserved for errors
const DIM: &str = "\x1b[2m";           // dim/faint — indices, timing, structural
const RESET: &str = "\x1b[0m";
const YELLOW: &str = "\x1b[33m";       // warning message body
const CYAN: &str = "\x1b[36m";         // block headers, [plan] tag
const MAGENTA: &str = "\x1b[35m";      // [devices] tag
const BLUE: &str = "\x1b[34m";         // [companion] tag
const BOLD_BLUE: &str = "\x1b[1;34m";  // flow name leaf, step local index
const BOLD_GREEN: &str = "\x1b[1;32m"; // PASS / Starting / Summary
const BOLD_RED: &str = "\x1b[1;31m";   // FAIL
const BOLD_YELLOW: &str = "\x1b[1;33m"; // SKIP / WARN, [install ...] tag
const BOLD_MAGENTA: &str = "\x1b[1;35m"; // bundle ID identity
const BOLD: &str = "\x1b[1m";          // action names, device name

// Threshold (ms) above which a successful step is annotated SLOW.
const SLOW_THRESHOLD_MS: u64 = 5_000;

/// Padding that visually replaces the timestamp column on continuation
/// lines (`HH:MM:SS.mmm` is 12 chars; format_timestamp adds a trailing
/// space; the renderer adds 2 more before `{dp}`). Total: 15 visible chars
/// before `{dp}`. The `│` gutter consumes 1, leaving 14 spaces here.
const TS_CONTINUATION_PAD: &str = "              "; // 14 spaces

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

/// 8-char right-aligned duration column, dim when colour is on.
/// `[   1.500s]` — fixed width so timing aligns vertically across lines,
/// nextest-style. Always renders seconds, three decimals.
fn fmt_dur(ms: u64, use_color: bool) -> String {
    let secs = ms as f64 / 1000.0;
    if use_color {
        format!("{DIM}[{secs:>8.3}s]{RESET}")
    } else {
        format!("[{secs:>8.3}s]")
    }
}

/// 6-char left-aligned bold status keyword. The keyword + color carries the
/// status — no leading symbol needed. PASS/FAIL/SKIP/WARN render in the same
/// fixed column so the eye can scan the left margin for failures.
fn keyword(label: &str, color: &str, use_color: bool) -> String {
    if use_color {
        format!("{color}{label:<6}{RESET}")
    } else {
        format!("{label:<6}")
    }
}

/// Render a step path as `{global}::{block}({iter})::{local}` — global
/// right-padded to 5 chars dim, `::` dim, block cyan, iteration dim parens
/// (omitted when 0), local index bold. Empty block degrades to
/// `{global}::{local}` (rare — pre-block events).
fn fmt_step_path(
    global: u64,
    block: &(String, u32),
    local: usize,
    use_color: bool,
) -> String {
    let (block_name, iteration) = block;
    let iter_part = if *iteration > 0 {
        if use_color {
            format!("{DIM}({iteration}){RESET}")
        } else {
            format!("({iteration})")
        }
    } else {
        String::new()
    };
    if block_name.is_empty() {
        if use_color {
            format!("{DIM}{global:>5}::{RESET}{BOLD_BLUE}{local}{RESET}")
        } else {
            format!("{global:>5}::{local}")
        }
    } else if use_color {
        format!(
            "{DIM}{global:>5}::{RESET}{CYAN}{block_name}{RESET}{iter_part}{DIM}::{RESET}{BOLD_BLUE}{local}{RESET}"
        )
    } else {
        format!("{global:>5}::{block_name}{iter_part}::{local}")
    }
}

/// Render a subsystem tag like `[plan]` / `[devices]` / `[install …]` in a
/// fixed colour. Lets the eye scan the left margin to tell apart setup-phase
/// concerns. No-color path returns the tag verbatim.
fn tag(label: &str, color: &str, use_color: bool) -> String {
    if use_color {
        format!("{color}{label}{RESET}")
    } else {
        label.to_string()
    }
}

/// Bold every digit run in a string. Used on `[devices]` availability lines
/// where the counts (`2 device(s)`, `1 booted`, …) are the actionable
/// information. Numbers embedded in identifiers (like `v34`) are excluded by
/// only bolding runs preceded by whitespace, `(`, or start-of-string.
fn bold_numbers(s: &str, use_color: bool) -> String {
    if !use_color {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 32);
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev_ok = i == 0 || matches!(chars[i - 1], ' ' | '(' | '\t');
        if c.is_ascii_digit() && prev_ok {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            out.push_str(BOLD);
            for &d in &chars[start..i] {
                out.push(d);
            }
            out.push_str(RESET);
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Render a flow path as `dir/name` with directory dim and leaf bold-blue.
/// Strips the `.test` suffix from the leaf for cleaner display when present.
fn fmt_flow_name(flow_name: &str, use_color: bool) -> String {
    if !use_color {
        return flow_name.to_string();
    }
    if let Some(slash) = flow_name.rfind('/') {
        let (dir, leaf) = (&flow_name[..=slash], &flow_name[slash + 1..]);
        format!("{DIM}{dir}{RESET}{BOLD_BLUE}{leaf}{RESET}")
    } else {
        format!("{BOLD_BLUE}{flow_name}{RESET}")
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
    // Track current block per device
    let mut current_blocks: HashMap<String, (String, u32)> = HashMap::new();
    // Track in-flight step (action + selector + local index) per device so
    // StepFinished can render the full row — collapses two-line render to
    // one line, vital for parallel interleaving readability.
    let mut current_steps: HashMap<String, (String, String, usize)> = HashMap::new();
    // Buffer substeps per device so a non-verbose failure can replay them
    // under the FAIL line as post-mortem context. Verbose streams them live
    // and never reads this map.
    let mut pending_substeps: HashMap<String, Vec<(SystemTime, SubstepEvent)>> = HashMap::new();

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
                // Top bookend + Starting header — non-verbose, single line.
                if use_color {
                    eprintln!("{ts}{DIM}{SEPARATOR}{RESET}");
                } else {
                    eprintln!("{ts}{SEPARATOR}");
                }
                let kw = keyword("Starting", BOLD_GREEN, use_color);
                let n_flows = flow_runs.len();
                let n_devs = device_availability.len();
                let flow_word = if n_flows == 1 { "flow" } else { "flows" };
                let dev_word = if n_devs == 1 { "device slot" } else { "device slots" };
                eprintln!(
                    "{ts}{kw} {n_flows} {flow_word} across {n_devs} {dev_word}"
                );

                // Verbose adds the diagnostic plan + install matrix dump.
                if verbose {
                    let plan_tag = tag("[plan]", CYAN, use_color);
                    let dev_tag = tag("[devices]", MAGENTA, use_color);
                    for line in flow_runs {
                        eprintln!("{ts}  {plan_tag} {line}");
                    }
                    if !install_entries.is_empty() {
                        eprintln!(
                            "{ts}  {plan_tag} install matrix ({} entr{})",
                            install_entries.len(),
                            if install_entries.len() == 1 { "y" } else { "ies" }
                        );
                        for line in install_entries {
                            eprintln!("{ts}  {plan_tag}   {line}");
                        }
                    }
                    if !device_availability.is_empty() {
                        for line in device_availability {
                            let line = bold_numbers(line, use_color);
                            eprintln!("{ts}  {dev_tag} {line}");
                        }
                    }
                }
            }
            EventKind::FlowStarted { flow_name, .. } => {
                let name = fmt_flow_name(flow_name, use_color);
                if use_color {
                    eprintln!(
                        "{ts}{dp}{BOLD_GREEN}\u{25B6}{RESET} {name}  {DIM}device={}{RESET}",
                        event.device_id
                    );
                } else {
                    eprintln!("{ts}{dp}\u{25B6} {name}  device={}", event.device_id);
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
                // Capture action+selector+local-index so StepFinished can render
                // the full row on one line. Two-line render fragments badly
                // under parallel device interleaving; we collapse to one line
                // per outcome.
                current_steps.insert(
                    event.device_id.0.clone(),
                    (action.clone(), selector_label.clone(), *step_index_in_block),
                );
                // Reset substep buffer for this device — only the about-to-run
                // step's substeps are interesting if it fails.
                pending_substeps.remove(&event.device_id.0);
                if !verbose {
                    continue;
                }
                // --verbose only: dim "starting" hint so substeps have context.
                let target_str = if selector_label.is_empty() {
                    String::new()
                } else {
                    format!(" {selector_label}")
                };
                let path = fmt_step_path(
                    *global_step_index,
                    &current_blocks.get(&event.device_id.0).cloned().unwrap_or_default(),
                    *step_index_in_block,
                    use_color,
                );
                if use_color {
                    eprintln!("{ts}  {dp}{path} {BOLD}{action}{RESET}{target_str}");
                } else {
                    eprintln!("{ts}  {dp}{path} {action}{target_str}");
                }
            }
            EventKind::StepFinished { outcome, duration_ms, tree_stats, retry_count, global_step_index, .. } => {
                let (action, selector, local_idx) = current_steps
                    .remove(&event.device_id.0)
                    .unwrap_or_default();
                let target_str = if selector.is_empty() {
                    String::new()
                } else {
                    format!(" {selector}")
                };
                let block = current_blocks
                    .get(&event.device_id.0)
                    .cloned()
                    .unwrap_or_default();
                let path = fmt_step_path(*global_step_index, &block, local_idx, use_color);
                let block_suffix = format!("  {path}");
                let dur = fmt_dur(*duration_ms, use_color);
                let action_target = if use_color {
                    format!("{BOLD}{action}{RESET}{target_str}")
                } else {
                    format!("{action}{target_str}")
                };
                let mut tags: Vec<String> = Vec::new();
                if *retry_count > 0 {
                    let s = format!("RETRY {retry_count}");
                    tags.push(if use_color {
                        format!("{BOLD_YELLOW}{s}{RESET}")
                    } else {
                        s
                    });
                }
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
                match outcome {
                    golem_events::StepOutcome::Success => {
                        if *duration_ms > SLOW_THRESHOLD_MS {
                            tags.push(if use_color {
                                format!("{BOLD_YELLOW}SLOW{RESET}")
                            } else {
                                "SLOW".to_string()
                            });
                        }
                        let tag_str = if tags.is_empty() {
                            String::new()
                        } else {
                            format!("  {}", tags.join(" "))
                        };
                        let kw = keyword("PASS", BOLD_GREEN, use_color);
                        eprintln!("{ts}  {dp}{kw} {dur}  {action_target}{tag_str}{block_suffix}{stats_str}");
                        pending_substeps.remove(&event.device_id.0);
                    }
                    golem_events::StepOutcome::Failed(msg) => {
                        let tag_str = if tags.is_empty() {
                            String::new()
                        } else {
                            format!("  {}", tags.join(" "))
                        };
                        let kw = keyword("FAIL", BOLD_RED, use_color);
                        eprintln!("{ts}  {dp}{kw} {dur}  {action_target}{tag_str}{block_suffix}");
                        let pending = pending_substeps.remove(&event.device_id.0).unwrap_or_default();
                        print_failure_block(msg, BOLD_RED, &dp, &pending, use_color);
                    }
                    golem_events::StepOutcome::Warning(msg) => {
                        let tag_str = if tags.is_empty() {
                            String::new()
                        } else {
                            format!("  {}", tags.join(" "))
                        };
                        let kw = keyword("WARN", BOLD_YELLOW, use_color);
                        eprintln!("{ts}  {dp}{kw} {dur}  {action_target}{tag_str}{block_suffix}");
                        let pending = pending_substeps.remove(&event.device_id.0).unwrap_or_default();
                        print_failure_block(msg, YELLOW, &dp, &pending, use_color);
                    }
                    golem_events::StepOutcome::Skipped | golem_events::StepOutcome::Ignored => {
                        let kw = keyword("SKIP", DIM, use_color);
                        eprintln!("{ts}  {dp}{kw} {dur}  {action_target}{block_suffix}");
                        pending_substeps.remove(&event.device_id.0);
                    }
                }
            }
            EventKind::Substep(sub) => {
                if verbose {
                    print_substep(&ts, &dp, sub, use_color);
                } else {
                    pending_substeps
                        .entry(event.device_id.0.clone())
                        .or_default()
                        .push((event.wall_time, sub.clone()));
                }
            }
            EventKind::FlowFinished { flow_name, success, duration_ms, seed, .. } => {
                eprintln!();
                let dur = fmt_dur(*duration_ms, use_color);
                let name = fmt_flow_name(flow_name, use_color);
                let kw = if *success {
                    keyword("PASS", BOLD_GREEN, use_color)
                } else {
                    keyword("FAIL", BOLD_RED, use_color)
                };
                if use_color {
                    eprintln!("{ts}  {dp}{kw} {dur}  {name}  {DIM}seed:{seed}{RESET}");
                } else {
                    eprintln!("{ts}  {dp}{kw} {dur}  {name}  seed:{seed}");
                }
            }
            EventKind::SuiteFinished { duration_ms, passed, failed, skipped } => {
                eprintln!();
                if use_color {
                    eprintln!("{ts}{DIM}{SEPARATOR}{RESET}");
                } else {
                    eprintln!("{ts}{SEPARATOR}");
                }
                let skip_suffix = if *skipped > 0 {
                    format!(", {skipped} skipped")
                } else {
                    String::new()
                };
                let kw = keyword("Summary", BOLD_GREEN, use_color);
                let dur = fmt_dur(*duration_ms, use_color);
                eprintln!(
                    "{ts}{kw} {dur}  {passed} passed, {failed} failed{skip_suffix}"
                );
            }
            EventKind::InstallStarted { app_name, bundle_id, target, .. } => {
                let t = tag(&format!("[install {app_name}]"), BOLD_YELLOW, use_color);
                let bid = if use_color {
                    format!("{BOLD_MAGENTA}{bundle_id}{RESET}")
                } else {
                    bundle_id.clone()
                };
                eprintln!("{ts}  {dp}{t} building and installing {bid} on {target}...");
            }
            EventKind::InstallOutput { app_name, line } => {
                if debug {
                    let t = tag(&format!("[install {app_name}]"), BOLD_YELLOW, use_color);
                    eprintln!("{ts}  {dp}{t} {line}");
                }
            }
            EventKind::InstallFinished { app_name, bundle_id, success, duration_ms, error, target, .. } => {
                let dur = fmt_dur(*duration_ms, use_color);
                let t = tag(&format!("[install {app_name}]"), BOLD_YELLOW, use_color);
                let bid = if use_color {
                    format!("{BOLD_MAGENTA}{bundle_id}{RESET}")
                } else {
                    bundle_id.clone()
                };
                if *success {
                    eprintln!("{ts}  {dp}{t} installed {bid} on {target}  {dur}");
                } else {
                    let err = error.as_deref().unwrap_or("install failed");
                    if use_color {
                        eprintln!("{ts}  {dp}{t} {BOLD_RED}{SYM_FAILED} {err}{RESET} on {target}  {dur}");
                    } else {
                        eprintln!("{ts}  {dp}{t} {SYM_FAILED} {err} on {target}  {dur}");
                    }
                }
            }
            EventKind::InstallSkipped { app_name, bundle_id, target, reason } => {
                let t = tag(&format!("[install {app_name}]"), BOLD_YELLOW, use_color);
                let bid = if use_color {
                    format!("{BOLD_MAGENTA}{bundle_id}{RESET}")
                } else {
                    bundle_id.clone()
                };
                if use_color {
                    eprintln!("{ts}  {dp}{t} {bid} on {target} {DIM}— skipped ({reason}){RESET}");
                } else {
                    eprintln!("{ts}  {dp}{t} {bid} on {target} — skipped ({reason})");
                }
            }
            EventKind::InstallCacheMiss { app_name, bundle_id: _, target, reason } => {
                let t = tag(&format!("[install {app_name}]"), BOLD_YELLOW, use_color);
                if use_color {
                    eprintln!("{ts}  {dp}{t} {DIM}cache miss on {target} — {reason}{RESET}");
                } else {
                    eprintln!("{ts}  {dp}{t} cache miss on {target} — {reason}");
                }
            }
            EventKind::FlowSkipped { flow_name, reason } => {
                let kw = keyword("SKIP", BOLD_YELLOW, use_color);
                let name = fmt_flow_name(flow_name, use_color);
                let dur_blank = if use_color {
                    format!("{DIM}[       --]{RESET}")
                } else {
                    "[       --]".to_string()
                };
                if use_color {
                    eprintln!("{ts}  {dp}{kw} {dur_blank}  {name}  {DIM}{reason}{RESET}");
                } else {
                    eprintln!("{ts}  {dp}{kw} {dur_blank}  {name}  {reason}");
                }
            }
            EventKind::FlowParseFailed { path, error } => {
                if use_color {
                    eprintln!("{ts}  {BOLD_RED}Parse error{RESET} ({path}): {error}");
                } else {
                    eprintln!("{ts}  Parse error ({path}): {error}");
                }
            }
            EventKind::DeviceAutoBoot { device_name, slot_shape } => {
                let t = tag("[devices]", MAGENTA, use_color);
                let dn = if use_color { format!("{BOLD}{device_name}{RESET}") } else { device_name.clone() };
                if use_color {
                    eprintln!("{ts}  {t} {DIM}no booted match — booting{RESET} {dn} {DIM}to satisfy {slot_shape}...{RESET}");
                } else {
                    eprintln!("{ts}  {t} no booted match — booting {dn} to satisfy {slot_shape}...");
                }
            }
            EventKind::DeviceAutoBootFinished { device_name, slot_shape, duration_ms } => {
                let dur = fmt_dur(*duration_ms, use_color);
                let t = tag("[devices]", MAGENTA, use_color);
                let dn = if use_color { format!("{BOLD}{device_name}{RESET}") } else { device_name.clone() };
                if use_color {
                    eprintln!("{ts}  {t} booted {dn} {DIM}for {slot_shape}{RESET}  {dur}");
                } else {
                    eprintln!("{ts}  {t} booted {dn} for {slot_shape}  {dur}");
                }
            }
            EventKind::SlotSetupFailed { slot_label, reason } => {
                if use_color {
                    eprintln!("{ts}  {BOLD_RED}[slot] setup failed for {slot_label}:{RESET} {reason}");
                } else {
                    eprintln!("{ts}  [slot] setup failed for {slot_label}: {reason}");
                }
            }
            EventKind::ResourcesWaiting { platform } => {
                let t = tag("[resources]", DIM, use_color);
                if use_color {
                    eprintln!("{ts}  {dp}{t} {DIM}waiting for {platform}...{RESET}");
                } else {
                    eprintln!("{ts}  {dp}{t} waiting for {platform}...");
                }
            }
            EventKind::CompanionStarting { platform, device_name } => {
                let t = tag("[companion]", BLUE, use_color);
                let dn = if use_color { format!("{BOLD}{device_name}{RESET}") } else { device_name.clone() };
                if use_color {
                    eprintln!("{ts}  {dp}{t} {DIM}starting on{RESET} {dn} {DIM}({platform})...{RESET}");
                } else {
                    eprintln!("{ts}  {dp}{t} starting on {dn} ({platform})...");
                }
            }
            EventKind::CompanionReady { platform, version, device_name, os_version } => {
                let t = tag("[companion]", BLUE, use_color);
                let dn = if use_color { format!("{BOLD}{device_name}{RESET}") } else { device_name.clone() };
                if use_color {
                    eprintln!("{ts}  {dp}{t} {DIM}ready —{RESET} {dn} {DIM}{platform} v{version} ({os_version}){RESET}");
                } else {
                    eprintln!("{ts}  {dp}{t} ready — {dn} {platform} v{version} ({os_version})");
                }
            }
            _ => {}
        }
    }
}

/// Print the post-FAIL/WARN error message + buffered substep replay tied
/// to the just-finished step. The error continuation gets a `│` gutter
/// (still part of the FAIL line); replayed substeps get `├` (or `╰` on the
/// last) to visually rope them under the failure. Substeps print with their
/// own original wall_time — the user sees timestamps "go back" then forward
/// again, which is the cue that this is post-mortem context.
fn print_failure_block(
    msg: &str,
    msg_color: &str,
    dp: &str,
    pending: &[(SystemTime, SubstepEvent)],
    use_color: bool,
) {
    // Continuation gutter for the error message — `│` replaces the
    // timestamp column so the line reads as "still the FAIL above talking".
    let gutter_pipe = if use_color { format!("{DIM}│{RESET}") } else { "│".to_string() };
    if use_color {
        eprintln!("{gutter_pipe}{TS_CONTINUATION_PAD}{dp}       {msg_color}{msg}{RESET}");
    } else {
        eprintln!("{gutter_pipe}{TS_CONTINUATION_PAD}{dp}       {msg}");
    }
    // Replayed substeps with ├/╰ rope.
    let total = pending.len();
    for (i, (st, sub)) in pending.iter().enumerate() {
        let last = i + 1 == total;
        let g = if last { "\u{2570}" } else { "\u{251C}" }; // ╰ or ├
        let gutter = if use_color { format!("{DIM}{g}{RESET} ") } else { format!("{g} ") };
        let sub_ts = format_timestamp(*st, use_color);
        let prefixed_ts = format!("{gutter}{sub_ts}");
        print_substep(&prefixed_ts, dp, sub, use_color);
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
