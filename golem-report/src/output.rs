//! Output orchestration for writing results to multiple formats.
//!
//! `--output` controls stdout format: `human`, `json`, `junit`, `toon`.
//! Results are always written to `--output-dir` (default `.golem/results/`)
//! unless `--no-results` is set. JSON and toon are always written; JUnit
//! only when `--output junit` is specified.
//!
//! # CLI usage
//!
//! ```text
//! golem run test.toml                          # human to stderr, json+toon to disk
//! golem run test.toml --output json            # json to stdout + disk
//! golem run test.toml --output junit           # junit to stdout + disk (adds results.xml)
//! golem run test.toml --no-results             # stdout only, no files
//! golem run test.toml --output-dir /tmp/out    # custom results directory
//! ```

use crate::SuiteReport;
use anyhow::Result;
use std::path::Path;

/// Supported output formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable terminal output with Unicode symbols.
    Human,
    /// Structured JSON output.
    Json,
    /// JUnit XML output for CI integration.
    Junit,
    /// Token-Optimized Output Notation for LLM consumption.
    Toon,
}

/// Parse a `--output` CLI string into a stdout format.
pub fn parse_output_format(spec: &str) -> Result<OutputFormat> {
    parse_format(spec)
}

/// Parse a format name string into an [`OutputFormat`].
fn parse_format(s: &str) -> Result<OutputFormat> {
    match s {
        "human" => Ok(OutputFormat::Human),
        "json" => Ok(OutputFormat::Json),
        "junit" => Ok(OutputFormat::Junit),
        "toon" => Ok(OutputFormat::Toon),
        _ => anyhow::bail!("Unknown output format: {s}"),
    }
}

/// Write results files to the output directory.
///
/// Always writes `results.json` and `results.toon`. Writes `results.xml`
/// only when `include_junit` is true (i.e. user specified `--output junit`).
pub fn write_results_to_dir(
    report: &SuiteReport,
    output_dir: &Path,
    include_junit: bool,
) -> Result<Vec<String>> {
    std::fs::create_dir_all(output_dir)?;
    let mut written = Vec::new();

    // Always write JSON
    let json = render(report, &OutputFormat::Json)?;
    let json_path = output_dir.join("results.json");
    std::fs::write(&json_path, &json)?;
    written.push(json_path.display().to_string());

    // Always write toon
    let toon = render(report, &OutputFormat::Toon)?;
    let toon_path = output_dir.join("results.toon");
    std::fs::write(&toon_path, &toon)?;
    written.push(toon_path.display().to_string());

    // JUnit only when requested
    if include_junit {
        let junit = render(report, &OutputFormat::Junit)?;
        let junit_path = output_dir.join("results.xml");
        std::fs::write(&junit_path, &junit)?;
        written.push(junit_path.display().to_string());
    }

    Ok(written)
}

/// Render a report in the specified format.
pub fn render(report: &SuiteReport, format: &OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Human => Ok(crate::human::format_suite(report)),
        OutputFormat::Json => {
            crate::json::format_suite_json(report).map_err(|e| anyhow::anyhow!(e))
        }
        OutputFormat::Junit => Ok(crate::junit::format_suite_junit(report)),
        OutputFormat::Toon => Ok(crate::toon::format_suite_toon(report)),
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FlowReport, StepOutcome, StepReport};

    // Helpers --------------------------------------------------------

    fn success_step(action: &str, target: &str, ms: u64) -> StepReport {
        StepReport {
            global_step_index: 0,
            block_name: String::new(),
            block_iteration: 0,
            step_index_in_block: 0,
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Success,
            duration_ms: ms,
            retry_count: 0,
            screenshot_path: None,
            substeps: vec![],
            tree_stats: golem_events::TreeStats::default(),
            started_at: None,
            finished_at: None,
        }
    }

    fn failed_step(action: &str, target: &str, ms: u64, msg: &str) -> StepReport {
        StepReport {
            global_step_index: 0,
            block_name: String::new(),
            block_iteration: 0,
            step_index_in_block: 0,
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Failed {
                message: msg.to_string(),
                code: golem_events::FailureCode::Uncoded,
            },
            duration_ms: ms,
            retry_count: 0,
            screenshot_path: None,
            substeps: vec![],
            tree_stats: golem_events::TreeStats::default(),
            started_at: None,
            finished_at: None,
        }
    }

    fn sample_suite() -> SuiteReport {
        SuiteReport {
            flows: vec![
                FlowReport {
                    first_failure_code: None,
                    a11y_audits: vec![],
                    flow_name: "login_flow".to_string(),
                    success: true,
                    step_results: vec![
                        success_step("launch", "", 100),
                        success_step("tap", "OK", 50),
                    ],
                    warnings: vec![],
                    duration_ms: 150,
                    seed: None,
                    screenshot_path: None,
                    device_name: None,
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                    covered_axes: Vec::new(),
                    recordings: Vec::new(),
                    repeat: None,
                    started_at: None,
                    finished_at: None,
                },
                FlowReport {
                    first_failure_code: None,
                    a11y_audits: vec![],
                    flow_name: "signup_flow".to_string(),
                    success: false,
                    step_results: vec![
                        success_step("launch", "", 80),
                        failed_step("assert_visible", "Welcome", 5000, "not found"),
                    ],
                    warnings: vec![],
                    duration_ms: 5080,
                    seed: None,
                    screenshot_path: None,
                    device_name: None,
                    os_major: None,
                    perf_snapshots: vec![],
                    skipped_reason: None,
                    covered_axes: Vec::new(),
                    recordings: Vec::new(),
                    repeat: None,
                    started_at: None,
                    finished_at: None,
                },
            ],
            installs: Vec::new(),
            total_duration_ms: 45300,
            started_at: None,
            finished_at: None,
        }
    }

    // 1. parse_output_format valid formats
    #[test]
    fn parse_valid_formats() {
        assert_eq!(parse_output_format("human").unwrap(), OutputFormat::Human);
        assert_eq!(parse_output_format("json").unwrap(), OutputFormat::Json);
        assert_eq!(parse_output_format("junit").unwrap(), OutputFormat::Junit);
        assert_eq!(parse_output_format("toon").unwrap(), OutputFormat::Toon);
    }

    // 2. parse_output_format rejects unknown
    #[test]
    fn parse_unknown_format_is_error() {
        let result = parse_output_format("csv");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown output format"));
    }

    // 3. render produces output for each format
    #[test]
    fn render_produces_output_for_each_format() {
        let suite = sample_suite();

        for fmt in &[
            OutputFormat::Human,
            OutputFormat::Json,
            OutputFormat::Junit,
            OutputFormat::Toon,
        ] {
            let out = render(&suite, fmt).expect("render");
            assert!(!out.is_empty());
            assert!(out.contains("login_flow"));
        }
    }

    // 4. write_results_to_dir creates json + toon
    #[test]
    fn write_results_to_dir_creates_json_and_toon() {
        let suite = sample_suite();
        let dir = tempfile::tempdir().expect("tempdir");

        let written = write_results_to_dir(&suite, dir.path(), false).expect("write");
        assert_eq!(written.len(), 2);

        let json = std::fs::read_to_string(dir.path().join("results.json")).expect("read json");
        assert!(json.contains("login_flow"));
        let toon = std::fs::read_to_string(dir.path().join("results.toon")).expect("read toon");
        assert!(toon.contains("login_flow"));
        assert!(
            !dir.path().join("results.xml").exists(),
            "no junit without flag"
        );
    }

    // 5. write_results_to_dir includes junit when requested
    #[test]
    fn write_results_to_dir_includes_junit() {
        let suite = sample_suite();
        let dir = tempfile::tempdir().expect("tempdir");

        let written = write_results_to_dir(&suite, dir.path(), true).expect("write");
        assert_eq!(written.len(), 3);
        assert!(dir.path().join("results.xml").exists());
    }

    // 6. write_results_to_dir returns paths in json, toon, junit order pointing at expected filenames
    #[test]
    fn write_results_returns_ordered_paths() {
        let suite = sample_suite();
        let dir = tempfile::tempdir().expect("tempdir");

        let written = write_results_to_dir(&suite, dir.path(), true).expect("write");
        assert_eq!(
            written.len(),
            3,
            "json+toon+junit SHALL produce three paths"
        );
        assert!(
            written[0].ends_with("results.json"),
            "first path SHALL be results.json"
        );
        assert!(
            written[1].ends_with("results.toon"),
            "second path SHALL be results.toon"
        );
        assert!(
            written[2].ends_with("results.xml"),
            "junit path SHALL be appended last"
        );
    }

    // 7. write_results_to_dir creates nested directories that do not yet exist
    #[test]
    fn write_results_creates_nested_dirs() {
        let suite = sample_suite();
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a").join("b").join("c");
        assert!(!nested.exists(), "nested dir SHALL NOT pre-exist");

        let written = write_results_to_dir(&suite, &nested, false).expect("write");
        assert_eq!(written.len(), 2);
        assert!(
            nested.join("results.json").exists(),
            "json SHALL be written into created nested dir"
        );
        assert!(
            nested.join("results.toon").exists(),
            "toon SHALL be written into created nested dir"
        );
    }

    // 8. write_results_to_dir into a pre-existing directory does not error
    #[test]
    fn write_results_into_existing_dir_succeeds() {
        let suite = sample_suite();
        let dir = tempfile::tempdir().expect("tempdir");

        // First write creates the files; second write into the same existing dir must still succeed (overwrite).
        write_results_to_dir(&suite, dir.path(), false).expect("first write");
        let written = write_results_to_dir(&suite, dir.path(), false).expect("second write");
        assert_eq!(
            written.len(),
            2,
            "repeated write into existing dir SHALL succeed"
        );
    }

    // 9. parse_output_format rejects empty string
    #[test]
    fn parse_empty_string_is_error() {
        let result = parse_output_format("");
        assert!(result.is_err(), "empty format spec SHALL be an error");
        assert!(
            result
                .expect_err("empty is err")
                .to_string()
                .contains("Unknown output format"),
            "empty spec SHALL report Unknown output format"
        );
    }

    // 10. parse_output_format is case-sensitive (uppercase is not accepted)
    #[test]
    fn parse_is_case_sensitive() {
        assert!(
            parse_output_format("JSON").is_err(),
            "uppercase JSON SHALL NOT parse"
        );
        assert!(
            parse_output_format("Human").is_err(),
            "capitalized Human SHALL NOT parse"
        );
    }

    // 11. render of an empty suite (no flows) succeeds for every format
    #[test]
    fn render_empty_suite_succeeds() {
        let empty = SuiteReport {
            flows: vec![],
            installs: Vec::new(),
            total_duration_ms: 0,
            started_at: None,
            finished_at: None,
        };

        for fmt in &[
            OutputFormat::Human,
            OutputFormat::Json,
            OutputFormat::Junit,
            OutputFormat::Toon,
        ] {
            let out = render(&empty, fmt)
                .unwrap_or_else(|e| panic!("render of empty suite SHALL succeed: {e}"));
            assert!(
                !out.is_empty(),
                "render of empty suite SHALL produce non-empty output"
            );
        }
    }

    // 12. json render of a failed flow carries the failure message
    #[test]
    fn render_json_includes_failure_message() {
        let suite = sample_suite();
        let json = render(&suite, &OutputFormat::Json).expect("render json");
        assert!(
            json.contains("signup_flow"),
            "json SHALL include the failed flow name"
        );
        assert!(
            json.contains("not found"),
            "json SHALL include the step failure message"
        );
    }
}
