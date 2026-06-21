use std::collections::HashMap;
use std::io::Write;
use std::time::SystemTime;

use chrono::{DateTime, Local};
use golem_events::{Event, EventKind, ScrollAttemptResult, SubstepEvent};
use tokio::sync::broadcast;

const SYM_FAILED: &str = "\u{2717}"; // ✗ (install failure marker)
const SYM_BULLET: &str = "\u{2219}"; // ∙
const SEPARATOR: &str = "────────────────────────────────────────";

// ANSI color codes — muted palette, bright reserved for errors
const DIM: &str = "\x1b[2m"; // dim/faint — indices, timing, structural
const RESET: &str = "\x1b[0m";
const YELLOW: &str = "\x1b[33m"; // warning message body
const CYAN: &str = "\x1b[36m"; // block headers, [plan] tag
const MAGENTA: &str = "\x1b[35m"; // [devices] tag
const BLUE: &str = "\x1b[34m"; // [companion] tag
const BOLD_BLUE: &str = "\x1b[1;34m"; // flow name leaf, step local index
const BOLD_GREEN: &str = "\x1b[1;32m"; // PASS / Starting / Summary
const BOLD_RED: &str = "\x1b[1;31m"; // FAIL
const BOLD_YELLOW: &str = "\x1b[1;33m"; // SKIP / WARN, [install ...] tag
const BOLD_MAGENTA: &str = "\x1b[1;35m"; // bundle ID identity
const BOLD: &str = "\x1b[1m"; // action names, device name

// Threshold (ms) above which a successful step is annotated SLOW.
const SLOW_THRESHOLD_MS: u64 = 5_000;

/// Padding that visually replaces the timestamp column on continuation
/// lines (`HH:MM:SS.mmm` is 12 chars; format_timestamp adds a trailing
/// space; the renderer adds 2 more before `{dp}`). Total: 15 visible chars
/// before `{dp}`. The `│` gutter consumes 1, leaving 14 spaces here.
const TS_CONTINUATION_PAD: &str = "              "; // 14 spaces

/// Number of decimal digits needed to represent `total` (≥1 for 0).
/// Determines the left-padding width for flow-run prefixes; the
/// renderer learns `total` from `SuitePlanned.flow_runs.len()` so
/// the column width is fixed at suite start and doesn't reflow
/// across short / long runs.
fn flow_index_width(total: usize) -> usize {
    let mut n = total.max(1);
    let mut digits = 0;
    while n > 0 {
        n /= 10;
        digits += 1;
    }
    digits
}

/// Dim ANSI colors for device prefixes — subtle, won't clash with status colors.
const DEVICE_COLORS: &[&str] = &[
    "\x1b[36m", // cyan
    "\x1b[35m", // magenta
    "\x1b[33m", // yellow (dim)
    "\x1b[34m", // blue
    "\x1b[32m", // green (dim)
    "\x1b[91m", // bright red (for device ID only)
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

/// Format a flow-run prefix. `idx` is 1-based for display; `width`
/// is decimal digits (e.g. width=3 → "  1", "012", "195"). Returns
/// an empty string in single-device mode so non-multi-device output
/// is unchanged. Width comes from `SuitePlanned.flow_runs.len()` so
/// the column is fixed for the run.
fn format_flow_prefix(idx: usize, width: usize, multi_device: bool, use_color: bool) -> String {
    if !multi_device {
        return String::new();
    }
    let display = idx + 1; // 1-based — easier for users to map to logs
    let num = format!("{display:>width$}");
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

/// 4-char left-aligned bold status keyword. Sized to fit the widest
/// labels (`PASS`/`FAIL`/`WARN`/`SKIP`); step-level `ok`/`NG` pad to
/// the same column so the eye can scan the left margin for failures.
fn keyword(label: &str, color: &str, use_color: bool) -> String {
    if use_color {
        format!("{color}{label:<4}{RESET}")
    } else {
        format!("{label:<4}")
    }
}

/// Render a step path as `{global}::{block}({iter})::{local}` — global
/// right-padded to 5 chars dim, `::` dim, block cyan, iteration dim parens
/// (omitted when 0), local index bold. Empty block degrades to
/// `{global}::{local}` (rare — pre-block events).
fn fmt_step_path(global: u64, block: &(String, u32), local: usize, use_color: bool) -> String {
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

    // Flow-prefix slot allocation: each FlowStarted grabs a fresh slot
    // so two sequential flows on the same device get distinct prefixes.
    // Events between two FlowStarteds on the same device inherit that
    // device's currently-assigned slot. Pre-FlowStarted events (e.g.
    // install) get allocated on-demand per device_id. Width is set on
    // SuitePlanned from `flow_runs.len()` so the decimal column is
    // fixed for the run.
    let mut current_slot: HashMap<String, usize> = HashMap::new();
    let mut next_slot: usize = 0;
    let mut prefix_width: usize = 1;
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
    // First failed/warned step's code per device, surfaced on the FlowFinished
    // FAIL line. Set once per flow; cleared when the flow finishes.
    let mut first_fail_code: HashMap<String, golem_events::FailureCode> = HashMap::new();

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
            format_flow_prefix(idx, prefix_width, multi_device, use_color)
        } else if let Some(&slot) = current_slot.get(&event.device_id.0) {
            // Non-flow event on a device that has already entered a flow —
            // inherit that flow's circle (step events, install output
            // during a flow, etc.).
            format_flow_prefix(slot, prefix_width, multi_device, use_color)
        } else {
            // Pre-flow events (installs, etc.) get no circle so they don't
            // consume numbers that should map to flow runs.
            String::new()
        };

        match &event.kind {
            EventKind::SuiteLint { warnings } => {
                let tag_str = tag("[lint]", YELLOW, use_color);
                for w in warnings {
                    eprintln!("{ts}{tag_str} {w}");
                }
            }
            EventKind::SuitePlanned {
                flow_runs,
                install_entries,
                device_availability,
            } => {
                // Set the decimal width for flow-prefix lines based on
                // the total FlowRun count for this suite. Stays fixed
                // for the whole run so the prefix column doesn't shift.
                prefix_width = flow_index_width(flow_runs.len());
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
                let dev_word = if n_devs == 1 {
                    "device slot"
                } else {
                    "device slots"
                };
                eprintln!("{ts}{kw} {n_flows} {flow_word} across {n_devs} {dev_word}");

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
                            if install_entries.len() == 1 {
                                "y"
                            } else {
                                "ies"
                            }
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
            EventKind::FlowStarted {
                flow_name, repeat, ..
            } => {
                // Reset any stale code from a prior flow on this device so it
                // can't leak onto this flow's FAIL line.
                first_fail_code.remove(&event.device_id.0);
                let name = fmt_flow_name(flow_name, use_color);
                let repeat_suffix = repeat
                    .as_ref()
                    .map(|r| format!("  (run {}/{})", r.index + 1, r.total))
                    .unwrap_or_default();
                if use_color {
                    eprintln!(
                        "{ts}{dp}{BOLD_GREEN}\u{25B6}{RESET} {name}{repeat_suffix}  {DIM}device={}{RESET}",
                        event.device_id
                    );
                } else {
                    eprintln!(
                        "{ts}{dp}\u{25B6} {name}{repeat_suffix}  device={}",
                        event.device_id
                    );
                }
            }
            EventKind::BlockStarted {
                block_name,
                iteration,
                ..
            } => {
                current_blocks.insert(event.device_id.0.clone(), (block_name.clone(), *iteration));
                let iter_suffix = if *iteration > 0 {
                    format!(" (iteration {iteration})")
                } else {
                    String::new()
                };
                if use_color {
                    eprintln!("{ts}  {dp}{CYAN}\u{2500}\u{2500} {block_name}{iter_suffix} \u{2500}\u{2500}{RESET}");
                } else {
                    eprintln!(
                        "{ts}  {dp}\u{2500}\u{2500} {block_name}{iter_suffix} \u{2500}\u{2500}"
                    );
                }
            }
            EventKind::BlockFinished {
                recording_path: Some(path),
                ..
            } => {
                // Surface the recording path inline under the block.
                // OSC 8 hyperlink when the terminal supports it; plain
                // text otherwise. Falls back gracefully if path can't
                // be canonicalised (e.g. file was moved between
                // capture and stream).
                if use_color {
                    let abs = std::fs::canonicalize(path)
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| path.clone());
                    let uri = format!("file://{abs}");
                    eprintln!(
                        "{ts}     {dp}{DIM}\u{2570}\u{2500} rec: \x1b]8;;{uri}\x1b\\{path}\x1b]8;;\x1b\\{RESET}"
                    );
                } else {
                    eprintln!("{ts}     {dp}\u{2570}\u{2500} rec: {path}");
                }
            }
            EventKind::BlockFinished { .. } => {}
            EventKind::StepStarted {
                global_step_index,
                step_index_in_block,
                action,
                selector_label,
                ..
            } => {
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
                    &current_blocks
                        .get(&event.device_id.0)
                        .cloned()
                        .unwrap_or_default(),
                    *step_index_in_block,
                    use_color,
                );
                if use_color {
                    eprintln!("{ts}  {dp}{path} {BOLD}{action}{RESET}{target_str}");
                } else {
                    eprintln!("{ts}  {dp}{path} {action}{target_str}");
                }
            }
            EventKind::StepFinished {
                outcome,
                duration_ms,
                tree_stats,
                retry_count,
                global_step_index,
                ..
            } => {
                let (action, selector, local_idx) =
                    current_steps.remove(&event.device_id.0).unwrap_or_default();
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
                        // Step-level uses `ok`/`NG` so `grep " PASS "`
                        // and `grep " FAIL "` reliably match flow-level
                        // only. `NG` stays uppercase so errors still
                        // visually stand out at a glance.
                        let kw = keyword("ok", BOLD_GREEN, use_color);
                        eprintln!("{ts}  {dp}{kw} {dur}  {action_target}{tag_str}{block_suffix}{stats_str}");
                        pending_substeps.remove(&event.device_id.0);
                    }
                    golem_events::StepOutcome::Failed { message, code } => {
                        let tag_str = if tags.is_empty() {
                            String::new()
                        } else {
                            format!("  {}", tags.join(" "))
                        };
                        let kw = keyword("NG", BOLD_RED, use_color);
                        eprintln!("{ts}  {dp}{kw} {dur}  {action_target}{tag_str}{block_suffix}");
                        let pending = pending_substeps
                            .remove(&event.device_id.0)
                            .unwrap_or_default();
                        first_fail_code
                            .entry(event.device_id.0.clone())
                            .or_insert(*code);
                        let rendered = code.render(golem_events::Severity::Error);
                        print_failure_block(&rendered, message, BOLD_RED, &dp, &pending, use_color);
                        // NB: only Failed populates first_fail_code — a warning
                        // doesn't fail the flow, so it must not own the FAIL line.
                    }
                    golem_events::StepOutcome::Warning { message, code } => {
                        let tag_str = if tags.is_empty() {
                            String::new()
                        } else {
                            format!("  {}", tags.join(" "))
                        };
                        let kw = keyword("WARN", BOLD_YELLOW, use_color);
                        eprintln!("{ts}  {dp}{kw} {dur}  {action_target}{tag_str}{block_suffix}");
                        let pending = pending_substeps
                            .remove(&event.device_id.0)
                            .unwrap_or_default();
                        let rendered = code.render(golem_events::Severity::Warning);
                        print_failure_block(&rendered, message, YELLOW, &dp, &pending, use_color);
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
            EventKind::FlowFinished {
                flow_name,
                success,
                duration_ms,
                seed,
                code,
                ..
            } => {
                eprintln!();
                let dur = fmt_dur(*duration_ms, use_color);
                let name = fmt_flow_name(flow_name, use_color);
                let kw = if *success {
                    keyword("PASS", BOLD_GREEN, use_color)
                } else {
                    keyword("FAIL", BOLD_RED, use_color)
                };
                // Surface the first failing step's code between flow name and
                // seed (only on failure; severity is Error at the flow level).
                // Prefer the first failing step's code; fall back to the
                // flow-level abort code (e.g. EF504 max_runtime / EF508 max_steps),
                // which has no owning step to carry it.
                let failed_code = first_fail_code.remove(&event.device_id.0).or(*code);
                let code_str = match failed_code {
                    Some(c) if !*success => {
                        let r = c.render(golem_events::Severity::Error);
                        if use_color {
                            format!("{BOLD_RED}{r}{RESET}  ")
                        } else {
                            format!("{r}  ")
                        }
                    }
                    _ => String::new(),
                };
                if use_color {
                    eprintln!("{ts}  {dp}{kw} {dur}  {name}  {code_str}{DIM}seed:{seed}{RESET}");
                } else {
                    eprintln!("{ts}  {dp}{kw} {dur}  {name}  {code_str}seed:{seed}");
                }
            }
            EventKind::SuiteFinished {
                duration_ms,
                passed,
                failed,
                skipped,
            } => {
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
                eprintln!("{ts}{kw} {dur}  {passed} passed, {failed} failed{skip_suffix}");
            }
            EventKind::InstallStarted {
                app_name,
                bundle_id,
                target,
                ..
            } => {
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
            EventKind::InstallFinished {
                app_name,
                bundle_id,
                success,
                duration_ms,
                error,
                target,
                ..
            } => {
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
                        eprintln!(
                            "{ts}  {dp}{t} {BOLD_RED}{SYM_FAILED} {err}{RESET} on {target}  {dur}"
                        );
                    } else {
                        eprintln!("{ts}  {dp}{t} {SYM_FAILED} {err} on {target}  {dur}");
                    }
                }
            }
            EventKind::InstallSkipped {
                app_name,
                bundle_id,
                target,
                reason,
            } => {
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
            EventKind::InstallCacheMiss {
                app_name,
                bundle_id: _,
                target,
                reason,
            } => {
                let t = tag(&format!("[install {app_name}]"), BOLD_YELLOW, use_color);
                if use_color {
                    eprintln!("{ts}  {dp}{t} {DIM}cache miss on {target} — {reason}{RESET}");
                } else {
                    eprintln!("{ts}  {dp}{t} cache miss on {target} — {reason}");
                }
            }
            EventKind::FlowSkipped { flow_name, reason } => {
                // Deliberate skip / informational notice — never a failure.
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
            EventKind::FlowCouldNotRun {
                flow_name,
                reason,
                code,
            } => {
                // The flow never ran — render FAIL with its code, matching the
                // report files and exit code.
                let kw = keyword("FAIL", BOLD_RED, use_color);
                let name = fmt_flow_name(flow_name, use_color);
                let rendered = code.render(golem_events::Severity::Error);
                let dur_blank = if use_color {
                    format!("{DIM}[       --]{RESET}")
                } else {
                    "[       --]".to_string()
                };
                if use_color {
                    eprintln!("{ts}  {dp}{kw} {dur_blank}  {name}  {BOLD_RED}{rendered}{RESET}  {DIM}{reason}{RESET}");
                } else {
                    eprintln!("{ts}  {dp}{kw} {dur_blank}  {name}  {rendered}  {reason}");
                }
            }
            EventKind::FlowParseFailed { path, error } => {
                let code =
                    golem_events::FailureCode::ParseFlowFile.render(golem_events::Severity::Error);
                if use_color {
                    eprintln!("{ts}  {BOLD_RED}Parse error{RESET} {code} ({path}): {error}");
                } else {
                    eprintln!("{ts}  Parse error {code} ({path}): {error}");
                }
            }
            EventKind::DeviceAutoBoot {
                device_name,
                slot_shape,
            } => {
                let t = tag("[devices]", MAGENTA, use_color);
                let dn = if use_color {
                    format!("{BOLD}{device_name}{RESET}")
                } else {
                    device_name.clone()
                };
                if use_color {
                    eprintln!("{ts}  {t} {DIM}no booted match — booting{RESET} {dn} {DIM}to satisfy {slot_shape}...{RESET}");
                } else {
                    eprintln!(
                        "{ts}  {t} no booted match — booting {dn} to satisfy {slot_shape}..."
                    );
                }
            }
            EventKind::DeviceAutoBootFinished {
                device_name,
                slot_shape,
                duration_ms,
            } => {
                let dur = fmt_dur(*duration_ms, use_color);
                let t = tag("[devices]", MAGENTA, use_color);
                let dn = if use_color {
                    format!("{BOLD}{device_name}{RESET}")
                } else {
                    device_name.clone()
                };
                if use_color {
                    eprintln!("{ts}  {t} booted {dn} {DIM}for {slot_shape}{RESET}  {dur}");
                } else {
                    eprintln!("{ts}  {t} booted {dn} for {slot_shape}  {dur}");
                }
            }
            EventKind::SlotSetupFailed { slot_label, reason } => {
                if use_color {
                    eprintln!(
                        "{ts}  {BOLD_RED}[slot] setup failed for {slot_label}:{RESET} {reason}"
                    );
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
            EventKind::CompanionStarting {
                platform,
                device_name,
            } => {
                let t = tag("[companion]", BLUE, use_color);
                let dn = if use_color {
                    format!("{BOLD}{device_name}{RESET}")
                } else {
                    device_name.clone()
                };
                if use_color {
                    eprintln!(
                        "{ts}  {dp}{t} {DIM}starting on{RESET} {dn} {DIM}({platform})...{RESET}"
                    );
                } else {
                    eprintln!("{ts}  {dp}{t} starting on {dn} ({platform})...");
                }
            }
            EventKind::CompanionReady {
                platform,
                version,
                device_name,
                os_version,
            } => {
                let t = tag("[companion]", BLUE, use_color);
                let dn = if use_color {
                    format!("{BOLD}{device_name}{RESET}")
                } else {
                    device_name.clone()
                };
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
    code: &str,
    msg: &str,
    msg_color: &str,
    dp: &str,
    pending: &[(SystemTime, SubstepEvent)],
    use_color: bool,
) {
    let mut w = std::io::stderr().lock();
    print_failure_block_to(&mut w, code, msg, msg_color, dp, pending, use_color);
}

#[allow(clippy::too_many_arguments)]
fn print_failure_block_to(
    w: &mut dyn Write,
    code: &str,
    msg: &str,
    msg_color: &str,
    dp: &str,
    pending: &[(SystemTime, SubstepEvent)],
    use_color: bool,
) {
    // Continuation gutter for the error message — replaces the timestamp
    // column so the line reads as "still the FAIL above talking". When
    // there are no substeps below, close the rope with `╰`; otherwise `│`
    // continues into the substep replay.
    let pipe_glyph = if pending.is_empty() {
        "\u{2570}"
    } else {
        "│"
    }; // ╰ or │
    let gutter_pipe = if use_color {
        format!("{DIM}{pipe_glyph}{RESET}")
    } else {
        pipe_glyph.to_string()
    };
    if use_color {
        let _ = writeln!(
            w,
            "{gutter_pipe}{TS_CONTINUATION_PAD}{dp}       {msg_color}{code} {msg}{RESET}"
        );
    } else {
        let _ = writeln!(
            w,
            "{gutter_pipe}{TS_CONTINUATION_PAD}{dp}       {code} {msg}"
        );
    }
    // Replayed substeps with ├/╰ rope.
    let total = pending.len();
    for (i, (st, sub)) in pending.iter().enumerate() {
        let last = i + 1 == total;
        let g = if last { "\u{2570}" } else { "\u{251C}" }; // ╰ or ├
        let gutter = if use_color {
            format!("{DIM}{g}{RESET} ")
        } else {
            format!("{g} ")
        };
        let sub_ts = format_timestamp(*st, use_color);
        let prefixed_ts = format!("{gutter}{sub_ts}");
        print_substep_to(w, &prefixed_ts, dp, sub, use_color);
    }
}

fn print_substep(ts: &str, dp: &str, sub: &SubstepEvent, use_color: bool) {
    let mut w = std::io::stderr().lock();
    print_substep_to(&mut w, ts, dp, sub, use_color);
}

fn print_substep_to(w: &mut dyn Write, ts: &str, dp: &str, sub: &SubstepEvent, use_color: bool) {
    let b = SYM_BULLET;
    let (d, r) = if use_color { (DIM, RESET) } else { ("", "") };

    match sub {
        SubstepEvent::ElementResolved {
            selector,
            bounds,
            tap_point,
        } => {
            let _ = writeln!(w, "{ts}  {dp}    {d}{b} element_resolved \"{selector}\" bounds=({},{},{},{}) tap=({},{}){r}",
                bounds.x, bounds.y, bounds.width, bounds.height, tap_point.x, tap_point.y);
        }
        SubstepEvent::ElementNotFound {
            selector,
            timeout_ms,
        } => {
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} element_not_found \"{selector}\" after {timeout_ms}ms{r}"
            );
        }
        SubstepEvent::Tap { point, .. } => {
            let _ = writeln!(w, "{ts}  {dp}    {d}{b} tap ({},{}){r}", point.x, point.y);
        }
        SubstepEvent::DoubleTap { point, .. } => {
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} double_tap ({},{}){r}",
                point.x, point.y
            );
        }
        SubstepEvent::LongPress {
            point, duration_ms, ..
        } => {
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} long_press ({},{}) {}ms{r}",
                point.x, point.y, duration_ms
            );
        }
        SubstepEvent::TextInput { text, .. } => {
            let _ = writeln!(w, "{ts}  {dp}    {d}{b} text_input \"{text}\"{r}");
        }
        SubstepEvent::Backspace { count } => {
            let _ = writeln!(w, "{ts}  {dp}    {d}{b} backspace ×{count}{r}");
        }
        SubstepEvent::Swipe { from, to, .. } => {
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} swipe ({},{})→({},{}){r}",
                from.x, from.y, to.x, to.y
            );
        }
        SubstepEvent::ScrollStarted {
            selector,
            direction,
        } => {
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} scroll_started \"{selector}\" direction={direction}{r}"
            );
        }
        SubstepEvent::ScrollAttempt {
            attempt: _,
            direction,
            strategy_index,
            container,
            from,
            to,
            result,
            tree_stats,
        } => {
            let dir_arrow = match direction.as_str() {
                "Down" => "↓",
                "Up" => "↑",
                "Left" => "←",
                "Right" => "→",
                _ => "?",
            };
            let result_str = match result {
                ScrollAttemptResult::PageScrolled => "page scrolled".to_string(),
                ScrollAttemptResult::InnerScrollableDetected => format!(
                    "inner scrollable consumed swipe → switching to preset {}",
                    strategy_index + 2
                ),
                ScrollAttemptResult::ContainerAdvanced => "container advanced".to_string(),
                ScrollAttemptResult::Stall { count, max } => format!("stall {count}/{max}"),
                ScrollAttemptResult::BoundaryReached => "boundary reached".to_string(),
            };
            let stats = format_tree_stats(tree_stats);
            // Container scrolls use fixed geometry, not presets — naming a
            // preset would be meaningless (it's always preset 1).
            let label = if *container {
                "container".to_string()
            } else {
                format!("preset {}", strategy_index + 1)
            };
            let _ = writeln!(w, "{ts}  {dp}      {d}[scroll] {dir_arrow} {label} ({},{})→({},{}) → {result_str} {stats}{r}",
                from.x, from.y, to.x, to.y);
        }
        SubstepEvent::ScrollFound {
            selector,
            position,
            total_attempts,
        } => {
            let _ = writeln!(w, "{ts}  {dp}    {d}{b} scroll_found \"{selector}\" at ({},{}) after {total_attempts} attempts{r}",
                position.x, position.y);
        }
        SubstepEvent::ScrollDirectionReversed {
            to_direction,
            reason,
        } => {
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} scroll_reversed →{to_direction} {reason}{r}"
            );
        }
        SubstepEvent::ScrollStrategySwitch { to_index, reason } => {
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} scroll_preset_switch →{} {reason}{r}",
                to_index + 1
            );
        }
        SubstepEvent::AppLaunch {
            bundle,
            duration_ms,
        } => {
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} app_launch bundle={bundle} {duration_ms}ms{r}"
            );
        }
        SubstepEvent::PostSettle {
            action,
            duration_ms,
            stable,
        } => {
            let note = if *stable {
                "stable"
            } else {
                "timeout, still animating"
            };
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} post_settle {action} {duration_ms}ms ({note}){r}"
            );
        }
        SubstepEvent::AppStop { bundle } => {
            let _ = writeln!(w, "{ts}  {dp}    {d}{b} app_stop bundle={bundle}{r}");
        }
        SubstepEvent::DriverWarning { message } => {
            // Render in yellow + bold to stand out next to neutral
            // substep lines — the operator should notice before the
            // next step fails.
            let warn_tag = tag("[warning]", YELLOW, use_color);
            let _ = writeln!(w, "{ts}  {dp}    {warn_tag} {message}");
        }
        SubstepEvent::RetryAttempt {
            attempt,
            max,
            delay_ms,
            error,
        } => {
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} retry {attempt}/{max} delay={delay_ms}ms: {error}{r}"
            );
        }
        SubstepEvent::HttpRequest {
            method,
            url,
            status,
            duration_ms,
        } => {
            let s = status
                .map(|s| s.to_string())
                .unwrap_or_else(|| "?".to_string());
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} http {method} {url} → {s} [{duration_ms}ms]{r}"
            );
        }
        SubstepEvent::BashCommand {
            command,
            exit_code,
            duration_ms,
        } => {
            let code = exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".to_string());
            let _ = writeln!(
                w,
                "{ts}  {dp}    {d}{b} bash \"{command}\" exit={code} [{duration_ms}ms]{r}"
            );
        }
        SubstepEvent::Screenshot { path } => {
            let _ = writeln!(w, "{ts}  {dp}    {d}{b} screenshot {path}{r}");
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

#[cfg(test)]
mod tests {
    use super::*;

    // 1. flow_index_width: 0 total still needs one digit (max(1)).
    #[test]
    fn flow_index_width_zero_yields_one() {
        assert_eq!(flow_index_width(0), 1, "0 total SHALL still need 1 digit");
    }

    // 2. flow_index_width: single-digit totals are width 1.
    #[test]
    fn flow_index_width_single_digit() {
        assert_eq!(flow_index_width(1), 1, "1 SHALL be width 1");
        assert_eq!(flow_index_width(9), 1, "9 SHALL be width 1");
    }

    // 3. flow_index_width: decimal boundaries (10, 100) bump the width.
    #[test]
    fn flow_index_width_decimal_boundaries() {
        assert_eq!(flow_index_width(10), 2, "10 SHALL be width 2");
        assert_eq!(flow_index_width(99), 2, "99 SHALL be width 2");
        assert_eq!(flow_index_width(100), 3, "100 SHALL be width 3");
        assert_eq!(flow_index_width(195), 3, "195 SHALL be width 3");
    }

    // 4. format_flow_prefix: single-device mode yields empty string regardless
    //    of other args.
    #[test]
    fn format_flow_prefix_single_device_is_empty() {
        assert_eq!(
            format_flow_prefix(3, 3, false, true),
            "",
            "single-device mode SHALL produce no prefix"
        );
        assert_eq!(
            format_flow_prefix(0, 1, false, false),
            "",
            "single-device mode SHALL produce no prefix without color"
        );
    }

    // 5. format_flow_prefix: no-color multi-device renders 1-based, right-padded.
    #[test]
    fn format_flow_prefix_no_color_one_based_padded() {
        // idx 0 → display 1, width 3 → "  1 " (trailing space).
        assert_eq!(
            format_flow_prefix(0, 3, true, false),
            "  1 ",
            "idx 0 SHALL display as 1, right-padded to width 3 plus trailing space"
        );
        // idx 194 → display 195, width 3 → no extra padding.
        assert_eq!(
            format_flow_prefix(194, 3, true, false),
            "195 ",
            "idx 194 SHALL display as 195"
        );
    }

    // 6. format_flow_prefix: color path wraps with DIM + a device color + RESET.
    #[test]
    fn format_flow_prefix_color_wraps_with_device_color() {
        // idx 0 → display 1, width 1, color index 0 → cyan.
        // Expected bytes pinned independently of the named constants:
        // DIM=\x1b[2m, DEVICE_COLORS[0] (cyan)=\x1b[36m, RESET=\x1b[0m.
        let out = format_flow_prefix(0, 1, true, true);
        assert_eq!(
            out, "\x1b[2m\x1b[36m1\x1b[0m ",
            "color prefix SHALL wrap padded number in DIM+device color+RESET"
        );
    }

    // 7. format_flow_prefix: color index wraps around DEVICE_COLORS by modulo.
    #[test]
    fn format_flow_prefix_color_index_wraps_modulo() {
        // idx == len(6) → display 7; color index 6 % 6 == 0 → cyan (first
        // entry), proving the modulo wrap. Bytes pinned independently:
        // DIM=\x1b[2m, cyan=\x1b[36m, RESET=\x1b[0m.
        let out = format_flow_prefix(DEVICE_COLORS.len(), 1, true, true);
        assert_eq!(
            out, "\x1b[2m\x1b[36m7\x1b[0m ",
            "color index SHALL wrap around DEVICE_COLORS via modulo to the first color"
        );
    }

    // 8. fmt_dur: no-color path renders fixed 8-wide seconds with 3 decimals.
    #[test]
    fn fmt_dur_no_color_fixed_width() {
        assert_eq!(
            fmt_dur(1500, false),
            "[   1.500s]",
            "1500ms SHALL render as right-aligned 1.500s in 8 cols"
        );
        assert_eq!(
            fmt_dur(0, false),
            "[   0.000s]",
            "0ms SHALL render as 0.000s"
        );
    }

    // 9. fmt_dur: color path wraps the bracketed duration in DIM/RESET.
    #[test]
    fn fmt_dur_color_wraps_dim() {
        assert_eq!(
            fmt_dur(1500, true),
            format!("{DIM}[   1.500s]{RESET}"),
            "color duration SHALL be wrapped in DIM and RESET"
        );
    }

    // 10. keyword: no-color path left-pads label to 4 chars.
    #[test]
    fn keyword_no_color_left_pads_to_four() {
        assert_eq!(
            keyword("ok", BOLD_GREEN, false),
            "ok  ",
            "short label SHALL left-pad to 4"
        );
        assert_eq!(
            keyword("PASS", BOLD_GREEN, false),
            "PASS",
            "4-char label SHALL not over-pad"
        );
    }

    // 11. keyword: color path applies color then 4-wide label then RESET.
    #[test]
    fn keyword_color_wraps_color_and_pad() {
        assert_eq!(
            keyword("NG", BOLD_RED, true),
            format!("{BOLD_RED}NG  {RESET}"),
            "color keyword SHALL wrap left-padded label in color+RESET"
        );
    }

    // 12. fmt_step_path: no-color, empty block degrades to {global}::{local}.
    #[test]
    fn fmt_step_path_no_color_empty_block() {
        let block = (String::new(), 0u32);
        assert_eq!(
            fmt_step_path(7, &block, 2, false),
            "    7::2",
            "empty block SHALL degrade to right-padded global::local"
        );
    }

    // 13. fmt_step_path: no-color, named block with iteration 0 omits parens.
    #[test]
    fn fmt_step_path_no_color_named_block_no_iter() {
        let block = ("loginblk".to_string(), 0u32);
        assert_eq!(
            fmt_step_path(3, &block, 5, false),
            "    3::loginblk::5",
            "iteration 0 SHALL omit the (n) part"
        );
    }

    // 14. fmt_step_path: no-color, named block with iteration > 0 includes parens.
    #[test]
    fn fmt_step_path_no_color_named_block_with_iter() {
        let block = ("retryblk".to_string(), 2u32);
        assert_eq!(
            fmt_step_path(12, &block, 1, false),
            "   12::retryblk(2)::1",
            "iteration > 0 SHALL render as (n) between block and ::local"
        );
    }

    // 15. fmt_step_path: color, named block with iteration wraps each segment.
    #[test]
    fn fmt_step_path_color_named_block_with_iter() {
        let block = ("blk".to_string(), 3u32);
        let out = fmt_step_path(1, &block, 4, true);
        let expected = format!(
            "{DIM}{:>5}::{RESET}{CYAN}blk{RESET}{DIM}(3){RESET}{DIM}::{RESET}{BOLD_BLUE}4{RESET}",
            1
        );
        assert_eq!(out, expected, "color path SHALL wrap each segment per spec");
    }

    // 16. fmt_step_path: color, empty block uses the degraded color form.
    #[test]
    fn fmt_step_path_color_empty_block() {
        let block = (String::new(), 0u32);
        let out = fmt_step_path(9, &block, 0, true);
        let expected = format!("{DIM}{:>5}::{RESET}{BOLD_BLUE}0{RESET}", 9);
        assert_eq!(
            out, expected,
            "color empty block SHALL use degraded global::local form"
        );
    }

    // 17. tag: no-color returns the label verbatim; color wraps in color+RESET.
    #[test]
    fn tag_color_and_no_color() {
        assert_eq!(
            tag("[plan]", CYAN, false),
            "[plan]",
            "no-color tag SHALL be verbatim"
        );
        assert_eq!(
            tag("[plan]", CYAN, true),
            format!("{CYAN}[plan]{RESET}"),
            "color tag SHALL wrap label in color+RESET"
        );
    }

    // 18. bold_numbers: no-color returns the input unchanged.
    #[test]
    fn bold_numbers_no_color_unchanged() {
        assert_eq!(
            bold_numbers("2 device(s)", false),
            "2 device(s)",
            "no-color SHALL leave the string untouched"
        );
    }

    // 19. bold_numbers: digit run after whitespace/paren/start is bolded;
    //     a run embedded in an identifier (preceded by a letter) is not.
    #[test]
    fn bold_numbers_bolds_only_leading_runs() {
        // Leading run at start, after space, and after '(' is bolded;
        // the "34" in "v34" is preceded by 'v' so it stays plain.
        let out = bold_numbers("2 booted (1 of v34)", true);
        let expected = format!("{BOLD}2{RESET} booted ({BOLD}1{RESET} of v34)");
        assert_eq!(
            out, expected,
            "only whitespace/paren/start-preceded digit runs SHALL bold"
        );
    }

    // 20. bold_numbers: a multi-digit leading run is bolded as a single unit.
    #[test]
    fn bold_numbers_multidigit_run_single_unit() {
        let out = bold_numbers("128 nodes", true);
        assert_eq!(
            out,
            format!("{BOLD}128{RESET} nodes"),
            "a contiguous digit run SHALL be wrapped once, not per digit"
        );
    }

    // 21. fmt_flow_name: no-color returns the name verbatim.
    #[test]
    fn fmt_flow_name_no_color_verbatim() {
        assert_eq!(
            fmt_flow_name("e2e/cross/tap", false),
            "e2e/cross/tap",
            "no-color flow name SHALL be verbatim"
        );
    }

    // 22. fmt_flow_name: color with a slash dims the dir (incl. slash) and
    //     bold-blues the leaf.
    #[test]
    fn fmt_flow_name_color_with_slash() {
        let out = fmt_flow_name("e2e/cross/tap", true);
        assert_eq!(
            out,
            format!("{DIM}e2e/cross/{RESET}{BOLD_BLUE}tap{RESET}"),
            "dir up to and including the last slash SHALL be dim, leaf bold-blue"
        );
    }

    // 23. fmt_flow_name: color with no slash bold-blues the whole name.
    #[test]
    fn fmt_flow_name_color_no_slash() {
        let out = fmt_flow_name("tap", true);
        assert_eq!(
            out,
            format!("{BOLD_BLUE}tap{RESET}"),
            "a slashless name SHALL be entirely bold-blue"
        );
    }

    // 24. format_tree_stats: zero fetches yields an empty string.
    #[test]
    fn format_tree_stats_zero_fetches_empty() {
        let stats = golem_events::TreeStats {
            fetches: 0,
            min_nodes: 0,
            max_nodes: 0,
        };
        assert_eq!(
            format_tree_stats(&stats),
            "",
            "zero fetches SHALL produce no summary"
        );
    }

    // 25. format_tree_stats: equal min/max renders a single node count.
    #[test]
    fn format_tree_stats_equal_min_max() {
        let stats = golem_events::TreeStats {
            fetches: 3,
            min_nodes: 181,
            max_nodes: 181,
        };
        assert_eq!(
            format_tree_stats(&stats),
            "{3 trees, 181 nodes}",
            "equal min/max SHALL render a single node count"
        );
    }

    // 26. format_tree_stats: differing min/max renders a min~max range.
    #[test]
    fn format_tree_stats_range() {
        let stats = golem_events::TreeStats {
            fetches: 3,
            min_nodes: 181,
            max_nodes: 190,
        };
        assert_eq!(
            format_tree_stats(&stats),
            "{3 trees, 181~190 nodes}",
            "differing min/max SHALL render a min~max range"
        );
    }

    // 27. print_substep_to: a no-color Tap substep renders the expected line
    //     into an injected sink (proves the sink seam works end to end).
    #[test]
    fn print_substep_to_renders_tap_no_color() {
        let sub = SubstepEvent::Tap {
            point: golem_events::Point { x: 12, y: 34 },
            element_bounds: None,
        };
        let mut buf: Vec<u8> = Vec::new();
        print_substep_to(&mut buf, "TS ", "", &sub, false);
        let out = String::from_utf8(buf).expect("substep output SHALL be valid UTF-8");
        assert_eq!(
            out, "TS       \u{2219} tap (12,34)\n",
            "no-color tap substep SHALL render bullet + coordinates with a trailing newline"
        );
    }

    // 28. print_failure_block_to: no-color, no pending substeps closes the rope
    //     with `╰` and emits a single error-continuation line into the sink.
    #[test]
    fn print_failure_block_to_no_color_no_substeps() {
        let mut buf: Vec<u8> = Vec::new();
        print_failure_block_to(
            &mut buf,
            "EP422",
            "type unsupported",
            BOLD_RED,
            "",
            &[],
            false,
        );
        let out = String::from_utf8(buf).expect("failure block output SHALL be valid UTF-8");
        // ╰ gutter + 14-space pad + 7 spaces + "EP422 type unsupported".
        let expected = format!("\u{2570}{TS_CONTINUATION_PAD}       EP422 type unsupported\n");
        assert_eq!(
            out, expected,
            "empty-pending failure block SHALL close the rope with ╰ and render the code + message"
        );
    }

    // 29. print_failure_block_to: a buffered substep replays under the error
    //     line; the gutter switches to `│` then the substep gets the closing `╰`.
    #[test]
    fn print_failure_block_to_replays_pending_substep() {
        let sub = SubstepEvent::ElementNotFound {
            selector: "Submit".to_string(),
            timeout_ms: 5000,
        };
        let pending = [(SystemTime::UNIX_EPOCH, sub)];
        let mut buf: Vec<u8> = Vec::new();
        print_failure_block_to(
            &mut buf,
            "EE301",
            "not found",
            BOLD_RED,
            "",
            &pending,
            false,
        );
        let out = String::from_utf8(buf).expect("failure block output SHALL be valid UTF-8");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines.len(),
            2,
            "one error line plus one replayed substep SHALL render"
        );
        assert!(
            lines[0].starts_with('\u{2502}'),
            "with pending substeps the error gutter SHALL be │, got: {:?}",
            lines[0]
        );
        assert!(
            lines[1].contains('\u{2570}'),
            "the last (only) replayed substep SHALL use the closing ╰ rope, got: {:?}",
            lines[1]
        );
        assert!(
            lines[1].contains("element_not_found \"Submit\" after 5000ms"),
            "the replayed substep SHALL render its element_not_found detail, got: {:?}",
            lines[1]
        );
    }
}
