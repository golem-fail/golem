//! An app's lines from the device log: Android logcat, the iOS simulator's
//! unified log. Parsing, crash detection and the TOON rendering live here so
//! they test without a device.

use serde::Serialize;

/// The most lines `app_logs` shows when the caller sets no limit.
pub const DEFAULT_LIMIT: usize = 200;

/// Longer messages are cut: Chromium and stdout lines can run to kilobytes.
const MAX_MESSAGE: usize = 400;

/// One device log line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogLine {
    /// Seconds since the Unix epoch.
    pub time: f64,
    /// `V`, `D`, `I`, `W`, `E` or `F`.
    pub level: char,
    pub tag: String,
    pub pid: u32,
    pub message: String,
    /// A fatal error or an uncaught exception.
    pub crash: bool,
}

/// The app's uid from `pm list packages -U <bundle>`. The command matches
/// by substring, so `fail.golem.test` also lists `fail.golem.testb`.
pub fn package_uid(listing: &str, bundle: &str) -> Option<u32> {
    listing.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("package:")?;
        let (name, uid) = rest.split_once(" uid:")?;
        if name != bundle {
            return None;
        }
        uid.split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    })
}

/// Lines of `logcat -v epoch,uid` that the app logged or that name it. The
/// app's uid holds across restarts, where a pid filter loses the lines
/// before a crash. A line from another process that names the bundle, such
/// as ActivityManager restarting it, is kept too.
pub fn parse_logcat(text: &str, uid: u32, bundle: &str) -> Vec<LogLine> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let time: f64 = fields.next()?.parse().ok()?;
            let line_uid = logcat_uid(fields.next()?);
            let pid: u32 = fields.next()?.parse().ok()?;
            let _tid = fields.next()?;
            let level = fields.next()?.chars().next()?;
            let rest = line.split_once(&format!(" {level} "))?.1;
            let (tag, message) = rest.split_once(": ").unwrap_or((rest, ""));
            let (tag, message) = (tag.trim(), message.trim_end());
            if line_uid != Some(uid) && !names(message, bundle) && !names(tag, bundle) {
                return None;
            }
            Some(LogLine {
                time,
                level,
                tag: tag.to_string(),
                pid,
                crash: logcat_crash(level, tag, message),
                message: message.to_string(),
            })
        })
        .collect()
}

/// A uid as `-v uid` prints it: a number, or `u<user>_a<app>` on older
/// Android builds. System uids print as names such as `radio`.
fn logcat_uid(field: &str) -> Option<u32> {
    if let Ok(uid) = field.parse() {
        return Some(uid);
    }
    let (user, app) = field.strip_prefix('u')?.split_once("_a")?;
    Some(user.parse::<u32>().ok()? * 100_000 + 10_000 + app.parse::<u32>().ok()?)
}

/// Whether `text` names `bundle` as a whole word, not as the start of a
/// longer package name.
fn names(text: &str, bundle: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    text.match_indices(bundle).any(|(at, _)| {
        let mut after = text[at + bundle.len()..].chars();
        let ends = match after.next() {
            None => true,
            Some('.') => !after.next().is_some_and(char::is_alphanumeric),
            Some(c) => !word(c),
        };
        ends && !text[..at].chars().next_back().is_some_and(word)
    })
}

fn logcat_crash(level: char, tag: &str, message: &str) -> bool {
    level == 'F' || (level == 'E' && tag == "AndroidRuntime") || message.starts_with("ANR in ")
}

/// Entries of `log show --style ndjson`. Lines that are not log events, such
/// as the closing `{"count":…}` summary, are skipped.
pub fn parse_os_log(ndjson: &str) -> Vec<LogLine> {
    ndjson
        .lines()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            if v["eventType"] != "logEvent" {
                return None;
            }
            let time = chrono::DateTime::parse_from_str(
                v["timestamp"].as_str()?,
                "%Y-%m-%d %H:%M:%S%.f%z",
            )
            .ok()?;
            let level = match v["messageType"].as_str().unwrap_or_default() {
                "Debug" => 'D',
                "Error" => 'E',
                "Fault" => 'F',
                _ => 'I',
            };
            let subsystem = v["subsystem"].as_str().unwrap_or_default();
            let category = v["category"].as_str().unwrap_or_default();
            let tag = match (subsystem, category) {
                ("", "") => "-".to_string(),
                (s, "") | ("", s) => s.to_string(),
                (s, c) => format!("{s}/{c}"),
            };
            let message = v["eventMessage"].as_str().unwrap_or_default().to_string();
            Some(LogLine {
                time: time.timestamp_micros() as f64 / 1e6,
                level,
                tag,
                pid: v["processID"].as_u64().unwrap_or(0) as u32,
                crash: os_log_crash(&message),
                message,
            })
        })
        .collect()
}

/// A Fault is common and not a crash. These are what the Swift runtime and
/// an uncaught Objective-C exception log as the app dies, and SpringBoard's
/// line when a signal kills it. SIGKILL is left out: it is how the system
/// stops an app, not how an app crashes.
fn os_log_crash(message: &str) -> bool {
    message.contains("Fatal error:")
        || message.contains("Terminating app due to uncaught exception")
        || message.starts_with("*** Terminating app")
        || (message.starts_with("Process exited: ")
            && message.contains("domain:signal")
            && !message.contains("SIGKILL"))
}

/// The lines to show, after `filter` and `limit`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Selection {
    /// The head of each crash, newest crashes up to the limit, however old
    /// the other lines shown: finding the crash is the point.
    pub crashes: Vec<LogLine>,
    /// Crash lines left out.
    pub crashes_cut: usize,
    /// The newest other lines, up to the limit, oldest first.
    pub lines: Vec<LogLine>,
    /// The lines that matched the filter.
    pub matched: usize,
}

/// Keep the lines whose tag or message contains `filter` (any case), then
/// the newest `limit` crash lines and the newest `limit` other lines. A
/// native tombstone writes hundreds of crash lines, and their tail is the
/// least useful part, so only the first [`CRASH_HEAD`] lines of each crash
/// count.
pub fn select(lines: Vec<LogLine>, filter: Option<&str>, limit: usize) -> Selection {
    let needle = filter.map(str::to_lowercase);
    let matching: Vec<LogLine> = lines
        .into_iter()
        .filter(|l| {
            needle.as_deref().is_none_or(|n| {
                l.message.to_lowercase().contains(n) || l.tag.to_lowercase().contains(n)
            })
        })
        .collect();
    let matched = matching.len();
    let (crashes, rest): (Vec<_>, Vec<_>) = matching.into_iter().partition(|l| l.crash);
    let newest = |lines: Vec<LogLine>| {
        let skip = lines.len().saturating_sub(limit);
        (skip, lines.into_iter().skip(skip).map(cut).collect())
    };
    let total = crashes.len();
    let crashes: Vec<LogLine> = crash_heads(crashes, limit).into_iter().map(cut).collect();
    let (_, lines) = newest(rest);
    Selection {
        crashes_cut: total - crashes.len(),
        crashes,
        lines,
        matched,
    }
}

/// The lines of one crash kept: the signal or exception, the abort message
/// and the top frames.
pub const CRASH_HEAD: usize = 12;

/// The first [`CRASH_HEAD`] lines of each crash, where one crash is a row of
/// crash lines from the same process and tag. The newest crashes come first
/// within `limit`, and each keeps its first lines when the limit cuts it.
fn crash_heads(crashes: Vec<LogLine>, limit: usize) -> Vec<LogLine> {
    let mut runs: Vec<Vec<LogLine>> = Vec::new();
    for line in crashes {
        match runs.last_mut() {
            Some(run) if run[0].pid == line.pid && run[0].tag == line.tag => run.push(line),
            _ => runs.push(vec![line]),
        }
    }
    let mut room = limit;
    let mut kept: Vec<Vec<LogLine>> = Vec::new();
    for mut run in runs.into_iter().rev() {
        if room == 0 {
            break;
        }
        run.truncate(CRASH_HEAD.min(room));
        room -= run.len();
        kept.push(run);
    }
    kept.into_iter().rev().flatten().collect()
}

fn cut(mut line: LogLine) -> LogLine {
    if let Some((at, _)) = line.message.char_indices().nth(MAX_MESSAGE) {
        line.message.truncate(at);
        line.message.push('…');
    }
    line
}

/// The selection as TOON-style text, crash lines first, with clock times in
/// `tz`.
pub fn render<Tz: chrono::TimeZone>(bundle: &str, sel: &Selection, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let clock = |t: f64| {
        chrono::DateTime::from_timestamp_micros((t * 1e6) as i64)
            .map(|d| d.with_timezone(tz).format("%H:%M:%S%.3f").to_string())
            .unwrap_or_default()
    };
    let row = |l: &LogLine| {
        format!(
            "  {} {} {}: {}",
            clock(l.time),
            l.level,
            l.tag,
            l.message.trim_end().replace('\n', "\n      ")
        )
    };
    let shown = sel.crashes.len() + sel.lines.len();
    let mut out = format!("app_logs {bundle} · {shown} of {} lines", sel.matched);
    if !sel.crashes.is_empty() {
        out.push_str(&format!("\ncrash[{}]", sel.crashes.len()));
        if sel.crashes_cut > 0 {
            out.push_str(&format!(" (+{} cut)", sel.crashes_cut));
        }
        out.push(':');
        for l in &sel.crashes {
            out.push('\n');
            out.push_str(&row(l));
        }
    }
    out.push_str(&format!("\nlines[{}]:", sel.lines.len()));
    for l in &sel.lines {
        out.push('\n');
        out.push_str(&row(l));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGCAT: &str = include_str!("../tests/fixtures/logs/logcat-crash.txt");
    const OS_LOG: &str = include_str!("../tests/fixtures/logs/os-log.ndjson");
    const BUNDLE: &str = "fail.golem.test";

    fn line(message: &str, crash: bool) -> LogLine {
        LogLine {
            time: 1.0,
            level: 'I',
            tag: "app".into(),
            pid: 1,
            message: message.into(),
            crash,
        }
    }

    #[test]
    fn package_uid_matches_the_bundle_exactly() {
        let listing = "package:fail.golem.testn uid:10231\n\
                       package:fail.golem.test uid:10233\n\
                       package:fail.golem.testb uid:10217\n";
        assert_eq!(package_uid(listing, BUNDLE), Some(10233));
        assert_eq!(package_uid(listing, "fail.golem.testb"), Some(10217));
        assert_eq!(package_uid(listing, "fail.golem"), None);
        assert_eq!(
            package_uid("package:a.b uid:10001,1077\n", "a.b"),
            Some(10001)
        );
    }

    #[test]
    fn logcat_uid_reads_numbers_and_app_names() {
        assert_eq!(logcat_uid("10233"), Some(10233));
        assert_eq!(logcat_uid("u0_a233"), Some(10233));
        assert_eq!(logcat_uid("u10_a5"), Some(1_010_005));
        assert_eq!(logcat_uid("radio"), None);
    }

    #[test]
    fn names_needs_the_whole_bundle() {
        assert!(names(
            "Scheduling restart of crashed service fail.golem.test/org.Svc",
            BUNDLE
        ));
        assert!(names("Process fail.golem.test (pid 1) has died", BUNDLE));
        assert!(names("ANR in fail.golem.test", BUNDLE));
        assert!(names("in fail.golem.test.", BUNDLE));
        assert!(!names("Start proc fail.golem.testb", BUNDLE));
        assert!(!names("Start proc fail.golem.test.child", BUNDLE));
        assert!(!names("Start proc xfail.golem.test", BUNDLE));
    }

    #[test]
    fn logcat_keeps_the_apps_lines_and_lines_that_name_it() {
        let lines = parse_logcat(LOGCAT, 10233, BUNDLE);
        assert!(!lines.is_empty());
        assert!(lines.iter().all(|l| l.pid == 28018
            || l.pid == 28165
            || l.pid == 28170
            || names(&l.message, BUNDLE)));
        assert!(
            lines.iter().any(|l| l.tag == "ActivityManager"
                && l.message
                    .starts_with("Scheduling restart of crashed service fail.golem.test/")),
            "a system line that names the app SHALL be kept: {lines:#?}"
        );
        assert!(
            !lines
                .iter()
                .any(|l| l.tag == "bluetooth" || l.message.contains("sandboxed_process0:org")),
            "other processes' lines SHALL be dropped"
        );
    }

    #[test]
    fn logcat_marks_fatal_lines_as_crashes() {
        let lines = parse_logcat(LOGCAT, 10233, BUNDLE);
        let signal = lines
            .iter()
            .find(|l| l.message.starts_with("Fatal signal 5 (SIGTRAP)"))
            .expect("the fatal signal");
        assert_eq!(
            (signal.level, signal.tag.as_str(), signal.crash),
            ('F', "libc", true)
        );
        assert_eq!(signal.time, 1791357627.367);
        assert!(lines.iter().filter(|l| !l.crash).all(|l| l.level != 'F'));
        let other = parse_logcat(LOGCAT, 99086, BUNDLE);
        let fatal = other
            .iter()
            .find(|l| l.message == "FATAL EXCEPTION: main")
            .expect("the uncaught exception");
        assert!(fatal.crash, "{fatal:?}");
        assert_eq!(
            other
                .iter()
                .find(|l| l.tag == "DEBUG")
                .map(|l| l.tag.as_str()),
            Some("DEBUG"),
            "a padded tag SHALL be trimmed"
        );
    }

    #[test]
    fn os_log_reads_events_and_skips_the_summary() {
        let lines = parse_os_log(OS_LOG);
        assert_eq!(lines.len(), 7, "{lines:#?}");
        assert_eq!(lines[1].message, "Initializing connection");
        assert_eq!(lines[1].pid, 72271);
        assert_eq!(lines[1].level, 'I');
        assert_eq!(lines[3].level, 'E');
        assert_eq!(lines[4].level, 'F');
        assert!(
            !lines[4].crash,
            "a Fault SHALL not count as a crash on its own"
        );
        let t =
            chrono::DateTime::parse_from_rfc3339("2026-10-07T16:16:51.703261+09:00").expect("time");
        assert_eq!(lines[1].time, t.timestamp_micros() as f64 / 1e6);
    }

    #[test]
    fn os_log_marks_a_signal_exit_as_a_crash_but_not_a_stop() {
        let lines = parse_os_log(OS_LOG);
        let (abort, stop) = (&lines[5], &lines[6]);
        assert!(abort.message.contains("code:SIGABRT(6)"), "{abort:?}");
        assert!(abort.crash, "{abort:?}");
        assert_eq!(abort.tag, "com.apple.SpringBoard/Workspace");
        assert!(stop.message.contains("force-quit"), "{stop:?}");
        assert!(
            !stop.crash,
            "a normal stop SHALL not count as a crash: {stop:?}"
        );
        assert!(!os_log_crash(
            "Process exited: <app<a.b>:1> -> <RBSProcessExitStatus| domain:signal(2) code:SIGKILL(9)>>"
        ));
    }

    #[test]
    fn os_log_marks_swift_and_objc_crashes() {
        assert!(os_log_crash(
            "App/main.swift:10: Fatal error: Index out of range"
        ));
        assert!(os_log_crash(
            "*** Terminating app due to uncaught exception 'NSInvalidArgumentException'"
        ));
        assert!(!os_log_crash("Failed to initialize the tracker"));
    }

    #[test]
    fn select_keeps_every_crash_and_the_newest_lines() {
        let mut lines: Vec<LogLine> = (0..10).map(|i| line(&format!("line {i}"), false)).collect();
        lines.insert(1, line("FATAL EXCEPTION: main", true));
        let sel = select(lines, None, 3);
        assert_eq!(sel.crashes.len(), 1, "an old crash SHALL survive the limit");
        let shown: Vec<&str> = sel.lines.iter().map(|l| l.message.as_str()).collect();
        assert_eq!(shown, ["line 7", "line 8", "line 9"]);
        assert_eq!(sel.matched, 11);
    }

    #[test]
    fn select_limits_crash_lines_by_the_newest_crashes() {
        let crash = |pid: u32, n: usize| -> Vec<LogLine> {
            (0..n)
                .map(|i| {
                    let mut l = line(&format!("crash {pid} line {i}"), true);
                    l.pid = pid;
                    l
                })
                .collect()
        };
        let lines = [crash(1, 3), crash(2, 3), crash(3, 3)].concat();
        let sel = select(lines, None, 5);
        let shown: Vec<&str> = sel.crashes.iter().map(|l| l.message.as_str()).collect();
        assert_eq!(
            shown,
            [
                "crash 2 line 0",
                "crash 2 line 1",
                "crash 3 line 0",
                "crash 3 line 1",
                "crash 3 line 2"
            ],
            "an older crash cut by the limit SHALL keep its first lines"
        );
        assert_eq!(sel.crashes_cut, 4);
        assert!(render(BUNDLE, &sel, &chrono::Utc).contains("\ncrash[5] (+4 cut):\n"));
    }

    #[test]
    fn select_keeps_the_head_of_each_crash() {
        let mut lines: Vec<LogLine> = (0..300).map(|i| line(&format!("#{i} pc"), true)).collect();
        let mut signal = line("Fatal signal 6 (SIGABRT)", true);
        signal.tag = "libc".into();
        lines.insert(0, signal);
        let sel = select(lines, None, 200);
        assert_eq!(sel.crashes.len(), 1 + CRASH_HEAD);
        assert_eq!(sel.crashes[0].message, "Fatal signal 6 (SIGABRT)");
        assert_eq!(sel.crashes[1].message, "#0 pc");
        assert_eq!(sel.crashes_cut, 300 - CRASH_HEAD);
    }

    #[test]
    fn select_filters_before_the_limit_and_ignores_case() {
        let lines = vec![
            line("golem MARKER 1", false),
            line("noise", false),
            line("noise", false),
        ];
        let sel = select(lines, Some("marker"), 1);
        assert_eq!(sel.lines.len(), 1);
        assert_eq!(sel.lines[0].message, "golem MARKER 1");
        assert_eq!(sel.matched, 1);
    }

    #[test]
    fn select_cuts_long_messages() {
        let sel = select(vec![line(&"x".repeat(5000), false)], None, 5);
        assert_eq!(sel.lines[0].message.chars().count(), MAX_MESSAGE + 1);
        assert!(sel.lines[0].message.ends_with('…'));
    }

    #[test]
    fn render_puts_crash_lines_first() {
        let mut crash = line("FATAL EXCEPTION: main", true);
        crash.level = 'E';
        crash.tag = "AndroidRuntime".into();
        let sel = select(vec![crash, line("hello", false)], None, 5);
        assert_eq!(
            render(BUNDLE, &sel, &chrono::Utc),
            "app_logs fail.golem.test · 2 of 2 lines\n\
             crash[1]:\n  00:00:01.000 E AndroidRuntime: FATAL EXCEPTION: main\n\
             lines[1]:\n  00:00:01.000 I app: hello"
        );
    }

    #[test]
    fn render_indents_the_lines_of_a_long_message() {
        let sel = select(
            vec![line("*** Terminating app\nFirst throw call stack:\n", true)],
            None,
            5,
        );
        assert_eq!(
            render(BUNDLE, &sel, &chrono::Utc),
            "app_logs fail.golem.test · 1 of 1 lines\n\
             crash[1]:\n  00:00:01.000 I app: *** Terminating app\n      First throw call stack:\n\
             lines[0]:"
        );
    }

    #[test]
    fn render_without_crashes_has_no_crash_section() {
        let sel = select(Vec::new(), None, 5);
        assert_eq!(
            render(BUNDLE, &sel, &chrono::Utc),
            "app_logs fail.golem.test · 0 of 0 lines\nlines[0]:"
        );
    }
}
