//! Pure post-hoc rendering helpers for `stream_human`: failure-block replay
//! (error line + buffered substep context) and substep/tree-stat formatting.
//! Split out of `stream.rs` — no behaviour change, just relocation.

use super::*;

/// Print the post-FAIL/WARN error message + buffered substep replay tied
/// to the just-finished step. The error continuation gets a `│` gutter
/// (still part of the FAIL line); replayed substeps get `├` (or `╰` on the
/// last) to visually rope them under the failure. Substeps print with their
/// own original wall_time — the user sees timestamps "go back" then forward
/// again, which is the cue that this is post-mortem context.
pub(crate) fn print_failure_block(
    code: &str,
    msg: &str,
    msg_color: &str,
    dp: &str,
    pending: &[(SystemTime, SubstepEvent)],
    use_color: bool,
) {
    let mut w = std::io::stderr().lock();
    print_failure_block_to(
        &mut w,
        &FailureBlock {
            code,
            msg,
            msg_color,
            dp,
            pending,
            use_color,
        },
    );
}

/// Bundled failure-line + buffered-substep-replay context for
/// `print_failure_block_to`.
#[derive(Clone, Copy)]
pub(crate) struct FailureBlock<'a> {
    pub(crate) code: &'a str,
    pub(crate) msg: &'a str,
    pub(crate) msg_color: &'a str,
    pub(crate) dp: &'a str,
    pub(crate) pending: &'a [(SystemTime, SubstepEvent)],
    pub(crate) use_color: bool,
}

pub(crate) fn print_failure_block_to(w: &mut dyn Write, block: &FailureBlock) {
    let FailureBlock {
        code,
        msg,
        msg_color,
        dp,
        pending,
        use_color,
    } = *block;
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

pub(crate) fn print_substep(ts: &str, dp: &str, sub: &SubstepEvent, use_color: bool) {
    let mut w = std::io::stderr().lock();
    print_substep_to(&mut w, ts, dp, sub, use_color);
}

pub(crate) fn print_substep_to(
    w: &mut dyn Write,
    ts: &str,
    dp: &str,
    sub: &SubstepEvent,
    use_color: bool,
) {
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
pub(crate) fn format_tree_stats(stats: &golem_events::TreeStats) -> String {
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
