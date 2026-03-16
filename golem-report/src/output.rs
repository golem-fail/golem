//! Output orchestration for writing results to multiple formats simultaneously.
//!
//! Supports writing to stdout and/or files in any combination of formats.
//!
//! # CLI usage
//!
//! ```text
//! golem run login.test.toml --output human                    # default: human to stdout
//! golem run login.test.toml --output json:report.json         # JSON to file
//! golem run login.test.toml --output junit:results.xml        # JUnit to file
//! golem run login.test.toml --output human --output json:report.json  # both
//! golem run login.test.toml --output toon                     # TOON to stdout
//! ```

use crate::SuiteReport;
use anyhow::Result;
use std::path::PathBuf;

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

/// An output destination: a format plus an optional file path.
///
/// When `file_path` is `None`, output goes to stdout.
#[derive(Debug, Clone)]
pub struct OutputTarget {
    /// The format to render.
    pub format: OutputFormat,
    /// Where to write. `None` means stdout.
    pub file_path: Option<PathBuf>,
}

impl OutputTarget {
    /// Parse from a CLI string like `"json:report.json"` or `"human"`.
    ///
    /// The format portion appears before an optional `:` delimiter.
    /// Everything after the first `:` is treated as the file path.
    pub fn parse(spec: &str) -> Result<Self> {
        if let Some((format_str, path)) = spec.split_once(':') {
            let format = parse_format(format_str)?;
            Ok(Self {
                format,
                file_path: Some(PathBuf::from(path)),
            })
        } else {
            let format = parse_format(spec)?;
            Ok(Self {
                format,
                file_path: None,
            })
        }
    }
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

/// Write results to all specified output targets.
///
/// For file targets, the content is written to disk (creating parent
/// directories as needed). For stdout targets, the rendered content is
/// returned so the caller can print it.
///
/// Returns a tuple of:
/// - `Vec<String>`: paths of files that were written
/// - `Vec<String>`: rendered content for stdout targets
pub fn write_outputs(
    report: &SuiteReport,
    targets: &[OutputTarget],
) -> Result<(Vec<String>, Vec<String>)> {
    let mut written_files = Vec::new();
    let mut stdout_contents = Vec::new();

    for target in targets {
        let content = render(report, &target.format)?;

        match &target.file_path {
            Some(path) => {
                // Create parent directories if needed
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::write(path, &content)?;
                written_files.push(path.display().to_string());
            }
            None => {
                stdout_contents.push(content);
            }
        }
    }

    Ok((written_files, stdout_contents))
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

/// Return the default output targets: human format to stdout.
pub fn default_outputs() -> Vec<OutputTarget> {
    vec![OutputTarget {
        format: OutputFormat::Human,
        file_path: None,
    }]
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FlowReport, StepOutcome, StepReport};
    use std::fs;

    // Helpers --------------------------------------------------------

    fn success_step(action: &str, target: &str, ms: u64) -> StepReport {
        StepReport {
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Success,
            duration_ms: ms,
        }
    }

    fn failed_step(action: &str, target: &str, ms: u64, msg: &str) -> StepReport {
        StepReport {
            action: action.to_string(),
            target: target.to_string(),
            outcome: StepOutcome::Failed(msg.to_string()),
            duration_ms: ms,
        }
    }

    fn sample_suite() -> SuiteReport {
        SuiteReport {
            flows: vec![
                FlowReport {
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
                },
                FlowReport {
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
                },
            ],
            total_duration_ms: 45300,
        }
    }

    // 1. Parse "human" -> Human format, no file -----------------------

    #[test]
    fn parse_human_no_file() {
        let target = OutputTarget::parse("human").expect("should parse");
        assert_eq!(target.format, OutputFormat::Human);
        assert!(target.file_path.is_none());
    }

    // 2. Parse "json:report.json" -> Json format, with file path ------

    #[test]
    fn parse_json_with_file() {
        let target = OutputTarget::parse("json:report.json").expect("should parse");
        assert_eq!(target.format, OutputFormat::Json);
        assert_eq!(
            target.file_path.as_deref(),
            Some(std::path::Path::new("report.json"))
        );
    }

    // 3. Parse "junit:results/test.xml" -> JUnit with nested path -----

    #[test]
    fn parse_junit_with_nested_path() {
        let target = OutputTarget::parse("junit:results/test.xml").expect("should parse");
        assert_eq!(target.format, OutputFormat::Junit);
        assert_eq!(
            target.file_path.as_deref(),
            Some(std::path::Path::new("results/test.xml"))
        );
    }

    // 4. Parse "toon" -> Toon format, no file -------------------------

    #[test]
    fn parse_toon_no_file() {
        let target = OutputTarget::parse("toon").expect("should parse");
        assert_eq!(target.format, OutputFormat::Toon);
        assert!(target.file_path.is_none());
    }

    // 5. Parse unknown format -> error --------------------------------

    #[test]
    fn parse_unknown_format_is_error() {
        let result = OutputTarget::parse("csv");
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Unknown output format"),
            "error message should mention unknown format: {err_msg}"
        );
    }

    // 6. render produces output for each format -----------------------

    #[test]
    fn render_produces_output_for_each_format() {
        let suite = sample_suite();

        let human = render(&suite, &OutputFormat::Human).expect("human render");
        assert!(!human.is_empty(), "human output should not be empty");
        assert!(human.contains("login_flow"), "human should contain flow name");

        let json = render(&suite, &OutputFormat::Json).expect("json render");
        assert!(!json.is_empty(), "json output should not be empty");
        assert!(json.contains("login_flow"), "json should contain flow name");

        let junit = render(&suite, &OutputFormat::Junit).expect("junit render");
        assert!(!junit.is_empty(), "junit output should not be empty");
        assert!(junit.contains("login_flow"), "junit should contain flow name");

        let toon = render(&suite, &OutputFormat::Toon).expect("toon render");
        assert!(!toon.is_empty(), "toon output should not be empty");
        assert!(toon.contains("login_flow"), "toon should contain flow name");
    }

    // 7. write_outputs creates files on disk (temp dir) ---------------

    #[test]
    fn write_outputs_creates_file() {
        let suite = sample_suite();
        let dir = tempfile::tempdir().expect("should create temp dir");
        let json_path = dir.path().join("report.json");

        let targets = vec![OutputTarget {
            format: OutputFormat::Json,
            file_path: Some(json_path.clone()),
        }];

        let (written, stdout) = write_outputs(&suite, &targets).expect("should write");
        assert_eq!(written.len(), 1);
        assert!(written[0].contains("report.json"));
        assert!(stdout.is_empty(), "no stdout targets");

        // Verify the file exists and contains valid JSON
        let content = fs::read_to_string(&json_path).expect("should read file");
        assert!(content.contains("login_flow"));
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("should be valid JSON");
        assert!(parsed.is_object());
    }

    // 8. write_outputs creates parent directories ---------------------

    #[test]
    fn write_outputs_creates_parent_directories() {
        let suite = sample_suite();
        let dir = tempfile::tempdir().expect("should create temp dir");
        let nested_path = dir.path().join("deep").join("nested").join("results.xml");

        let targets = vec![OutputTarget {
            format: OutputFormat::Junit,
            file_path: Some(nested_path.clone()),
        }];

        let (written, _) = write_outputs(&suite, &targets).expect("should write");
        assert_eq!(written.len(), 1);
        assert!(nested_path.exists(), "file should exist at nested path");

        let content = fs::read_to_string(&nested_path).expect("should read file");
        assert!(content.contains("<?xml"), "should be valid XML");
    }

    // 9. default_outputs returns human to stdout ----------------------

    #[test]
    fn default_outputs_returns_human_to_stdout() {
        let defaults = default_outputs();
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].format, OutputFormat::Human);
        assert!(defaults[0].file_path.is_none());
    }

    // 10. Multiple outputs all written --------------------------------

    #[test]
    fn multiple_outputs_all_written() {
        let suite = sample_suite();
        let dir = tempfile::tempdir().expect("should create temp dir");
        let json_path = dir.path().join("report.json");
        let junit_path = dir.path().join("results.xml");

        let targets = vec![
            OutputTarget {
                format: OutputFormat::Human,
                file_path: None, // stdout
            },
            OutputTarget {
                format: OutputFormat::Json,
                file_path: Some(json_path.clone()),
            },
            OutputTarget {
                format: OutputFormat::Junit,
                file_path: Some(junit_path.clone()),
            },
        ];

        let (written, stdout) = write_outputs(&suite, &targets).expect("should write");

        // Two files written
        assert_eq!(written.len(), 2, "should write two files");
        assert!(written.iter().any(|p| p.contains("report.json")));
        assert!(written.iter().any(|p| p.contains("results.xml")));

        // One stdout target
        assert_eq!(stdout.len(), 1, "should have one stdout output");
        assert!(
            stdout[0].contains("login_flow"),
            "stdout should contain human output"
        );

        // Files on disk contain correct content
        let json_content = fs::read_to_string(&json_path).expect("read json");
        assert!(json_content.contains("\"login_flow\""));

        let junit_content = fs::read_to_string(&junit_path).expect("read junit");
        assert!(junit_content.contains("<testsuite"));
    }

    // 11. Parse with colon in file path preserves full path -----------

    #[test]
    fn parse_colon_in_path_takes_first_colon_only() {
        // On some systems paths might not have colons, but we should handle
        // the split correctly: only split on first colon
        let target = OutputTarget::parse("json:C:/reports/out.json").expect("should parse");
        assert_eq!(target.format, OutputFormat::Json);
        assert_eq!(
            target.file_path.as_deref(),
            Some(std::path::Path::new("C:/reports/out.json"))
        );
    }

    // 12. Stdout targets with no file targets returns empty files vec --

    #[test]
    fn stdout_only_returns_no_written_files() {
        let suite = sample_suite();
        let targets = vec![
            OutputTarget {
                format: OutputFormat::Human,
                file_path: None,
            },
            OutputTarget {
                format: OutputFormat::Toon,
                file_path: None,
            },
        ];

        let (written, stdout) = write_outputs(&suite, &targets).expect("should write");
        assert!(written.is_empty(), "no files should be written");
        assert_eq!(stdout.len(), 2, "should have two stdout outputs");
    }
}
