//! `golem a11y-extract <png>` — read the audit embedded in an annotated a11y
//! screenshot and print it in human form, plus the command to replay that exact
//! run. The PNG carries everything (see `golem_runner::accessibility`'s iTXt
//! metadata); we refuse any image not stamped `Software = Golem`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::cli::A11yExtractArgs;

/// A rectangle in screenshot pixels (the embedded bounds coordinate space).
#[derive(Deserialize)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Deserialize, Default)]
struct Dim {
    #[serde(default)]
    w: u32,
    #[serde(default)]
    h: u32,
}

#[derive(Deserialize)]
struct Issue {
    #[serde(default)]
    marker: usize,
    #[serde(default)]
    check: String,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    confidence: f32,
    #[serde(default)]
    bounds: Option<Rect>,
    #[serde(default)]
    related: Vec<Rect>,
}

/// The embedded `Golem-Audit` record. Fields default so a PNG from an older
/// build (missing e.g. `platform`) still extracts cleanly.
#[derive(Deserialize)]
struct Audit {
    #[serde(default)]
    device: String,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    flow: String,
    #[serde(default)]
    block: String,
    #[serde(default)]
    iteration: u32,
    #[serde(default)]
    seed: u64,
    #[serde(default)]
    a11y_level: String,
    #[serde(default)]
    image: Dim,
    #[serde(default)]
    viewport: Dim,
    #[serde(default)]
    errors: usize,
    #[serde(default)]
    warnings: usize,
    #[serde(default)]
    issues: Vec<Issue>,
}

pub fn run(args: &A11yExtractArgs) -> Result<()> {
    let bytes = std::fs::read(&args.png)
        .with_context(|| format!("reading {}", args.png.display()))?;

    // Refuse anything not stamped by golem (the "if software wasn't golem,
    // error" rule). The reader validates `Software = Golem` first.
    let audit_json = golem_runner::accessibility::read_embedded_audit(&bytes)
        .map_err(|e| anyhow::anyhow!("{}: {e}", args.png.display()))?;

    // `--json`: hand back the raw record verbatim for tooling.
    if args.json {
        println!("{audit_json}");
        return Ok(());
    }

    let audit: Audit = serde_json::from_str(&audit_json)
        .context("the embedded Golem-Audit JSON is malformed")?;

    println!("{}", render(&audit));
    println!();
    println!("Replay:");
    println!("  {}", replay_command(&audit));
    Ok(())
}

/// The human report: context header, counts, then one line per finding.
fn render(audit: &Audit) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "golem a11y · \"{}\" / {}\n",
        audit.flow, audit.block
    ));
    let platform = audit.platform.as_deref().unwrap_or("unknown");
    out.push_str(&format!(
        "  device {} ({platform}) · level {} · seed {} · iteration {}\n",
        audit.device, audit.a11y_level, audit.seed, audit.iteration
    ));
    out.push_str(&format!(
        "  image {}×{}px · viewport {}×{}\n\n",
        audit.image.w, audit.image.h, audit.viewport.w, audit.viewport.h
    ));
    out.push_str(&format!(
        "  {} error(s), {} warning(s)\n",
        audit.errors, audit.warnings
    ));

    if audit.issues.is_empty() {
        out.push_str("  (no findings)\n");
        return out;
    }

    // Right-align the marker to the widest number so the list stays in columns.
    let marker_w = audit
        .issues
        .iter()
        .map(|i| i.marker)
        .max()
        .unwrap_or(0)
        .to_string()
        .len();

    for iss in &audit.issues {
        let tag = if iss.severity == "error" {
            "[ERR]"
        } else {
            "[WRN]"
        };
        let mut line = format!(
            "  {:>marker_w$} {tag} {:<24} {}",
            iss.marker, iss.check, iss.message
        );
        if let Some(d) = &iss.detail {
            line.push_str(&format!("  {d}"));
        }
        // Confidence is shown only when below 1.0 — a heuristic finding should
        // never read as a certain one (matches the report surfaces).
        if iss.confidence < 1.0 {
            line.push_str(&format!("  (confidence {:.2})", iss.confidence));
        }
        if let Some(b) = &iss.bounds {
            line.push_str(&format!("  @ {},{} {}×{}px", b.x, b.y, b.w, b.h));
        }
        if !iss.related.is_empty() {
            line.push_str(&format!("  (+{} related)", iss.related.len()));
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// The `golem run …` line to reproduce this run. `--seed` and `--a11y` are
/// exact; `--platform` is exact when the PNG recorded it. The flow *file* isn't
/// stored, so we glob the project for a `*.test.toml` whose `[flow].name`
/// matches — assuming `a11y-extract` runs inside the project (flow names are
/// unique by convention). Falls back to a named placeholder.
fn replay_command(audit: &Audit) -> String {
    let flow_arg = find_flow_file(&audit.flow)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| format!("<flow file for \"{}\">", audit.flow));

    let mut cmd = format!("golem run {flow_arg}");
    if let Some(p) = &audit.platform {
        if p != "unknown" {
            cmd.push_str(&format!(" --platform {p}"));
        }
    }
    cmd.push_str(&format!(" --seed {} --a11y {}", audit.seed, audit.a11y_level));
    cmd
}

/// Walk the working tree for a `*.test.toml` whose `[flow].name` equals `name`.
/// Returns the first match (flow names are unique by convention). Skips the
/// usual noise dirs so a big repo doesn't take forever.
fn find_flow_file(name: &str) -> Option<PathBuf> {
    let mut found = None;
    walk(Path::new("."), &mut |path| {
        if found.is_some() {
            return;
        }
        let is_flow = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".test.toml"));
        if !is_flow {
            return;
        }
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(flow) = golem_parser::parse_flow(&text) {
                if flow.flow.name == name {
                    found = Some(path.to_path_buf());
                }
            }
        }
    });
    found
}

/// Depth-first file walk skipping hidden dirs and the usual build/dep noise.
fn walk(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name.starts_with('.') || matches!(name.as_ref(), "target" | "node_modules") {
                continue;
            }
            walk(&path, visit);
        } else {
            visit(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit(flow: &str, platform: Option<&str>) -> Audit {
        Audit {
            device: "iPhone 17".into(),
            platform: platform.map(str::to_string),
            flow: flow.into(),
            block: "blk".into(),
            iteration: 0,
            seed: 12345,
            a11y_level: "strict".into(),
            image: Dim { w: 1206, h: 2622 },
            viewport: Dim { w: 402, h: 874 },
            errors: 1,
            warnings: 1,
            issues: vec![
                Issue {
                    marker: 1,
                    check: "missing_label".into(),
                    severity: "error".into(),
                    message: "button — no accessible name".into(),
                    detail: None,
                    confidence: 1.0,
                    bounds: Some(Rect { x: 96, y: 1863, w: 44, h: 44 }),
                    related: vec![],
                },
                Issue {
                    marker: 2,
                    check: "low_contrast".into(),
                    severity: "warning".into(),
                    message: "text 2.1:1 — below 4.5:1 (AA)".into(),
                    detail: Some("2.1:1".into()),
                    confidence: 0.48,
                    bounds: Some(Rect { x: 10, y: 20, w: 100, h: 30 }),
                    related: vec![],
                },
            ],
        }
    }

    // Replay command: platform present → exact; seed + level always exact.
    #[test]
    fn replay_command_includes_platform_seed_level() {
        let cmd = replay_command(&audit("No Such Flow Name 123", Some("ios")));
        assert!(cmd.contains("--platform ios"), "platform SHALL be exact: {cmd}");
        assert!(cmd.contains("--seed 12345"), "seed SHALL be exact: {cmd}");
        assert!(cmd.contains("--a11y strict"), "level SHALL be exact: {cmd}");
        // Unknown flow → placeholder, not a real path.
        assert!(
            cmd.contains("<flow file for \"No Such Flow Name 123\">"),
            "unresolved flow SHALL be a placeholder: {cmd}"
        );
    }

    // Absent platform (older PNG) → no --platform flag emitted.
    #[test]
    fn replay_command_omits_platform_when_unknown() {
        let cmd = replay_command(&audit("X", None));
        assert!(!cmd.contains("--platform"), "absent platform SHALL be omitted: {cmd}");
        let cmd2 = replay_command(&audit("X", Some("unknown")));
        assert!(!cmd2.contains("--platform"), "\"unknown\" platform SHALL be omitted: {cmd2}");
    }

    // Human render: header context, both findings, confidence shown only <1.0.
    #[test]
    fn render_lists_findings_and_confidence_only_below_one() {
        let out = render(&audit("My Flow", Some("ios")));
        assert!(out.contains("\"My Flow\" / blk"));
        assert!(out.contains("device iPhone 17 (ios)"));
        assert!(out.contains("seed 12345"));
        assert!(out.contains("1 error(s), 1 warning(s)"));
        assert!(out.contains("[ERR] missing_label"));
        assert!(out.contains("[WRN] low_contrast"));
        // 0.48 finding shows confidence; the 1.0 finding does not.
        assert!(out.contains("(confidence 0.48)"), "low-conf SHALL show: {out}");
        let err_line = out.lines().find(|l| l.contains("missing_label")).unwrap_or("");
        assert!(
            !err_line.contains("confidence"),
            "a 1.0 finding SHALL NOT show confidence: {err_line}"
        );
    }
}
