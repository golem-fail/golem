use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// A discovered flow file with its metadata.
pub struct DiscoveredFlow {
    pub path: PathBuf,
    pub name: String,
    pub tags: Vec<String>,
    /// Skip this flow in the tag-less discovery sweep (subflows). See
    /// `FlowMeta::explicit_only`.
    pub explicit_only: bool,
}

/// A single tag filter clause (e.g., "smoke|regression" parsed into alternatives).
pub struct TagFilter {
    /// OR-combined alternatives within this clause.
    pub alternatives: Vec<String>,
}

impl TagFilter {
    /// Parse a tag filter string. Pipe (`|`) separates OR alternatives.
    pub fn parse(tag_str: &str) -> Self {
        TagFilter {
            alternatives: tag_str
                .split('|')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }

    /// Check if a flow's tags satisfy this filter (any alternative matches).
    pub fn matches(&self, flow_tags: &[String]) -> bool {
        self.alternatives.iter().any(|alt| flow_tags.contains(alt))
    }
}

/// Directory names to exclude from discovery.
const EXCLUDED_DIRS: &[&str] = &[".golem", "__fixtures__", "__mixins__", "target"];

/// Discover all `*.test.toml` flow files under the project root, optionally
/// filtering by tags. `tag_filters` are AND-combined (all must match).
pub fn discover_flows(root: &Path, tag_filters: &[TagFilter]) -> Result<Vec<DiscoveredFlow>> {
    let mut flows = Vec::new();
    walk_directory(root, &mut flows)?;

    // Apply tag filters: untagged flows are excluded by any filter
    if !tag_filters.is_empty() {
        flows.retain(|flow| tag_filters.iter().all(|filter| filter.matches(&flow.tags)));
    } else {
        // No tag filter: this is the bulk sweep. Skip explicit-only flows
        // (subflows) — they run only via a matching --tag or a direct path.
        flows.retain(|flow| !flow.explicit_only);
    }

    // Sort by path for deterministic ordering
    flows.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(flows)
}

fn walk_directory(dir: &Path, flows: &mut Vec<DiscoveredFlow>) -> Result<()> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read directory: {}", dir.display()))?;

    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read entry in: {}", dir.display()))?;
        let path = entry.path();

        if path.is_dir() {
            let dir_name = match entry.file_name().to_str() {
                Some(name) => name.to_string(),
                None => continue,
            };

            // Skip hidden directories and excluded directories
            if dir_name.starts_with('.') || EXCLUDED_DIRS.contains(&dir_name.as_str()) {
                continue;
            }

            walk_directory(&path, flows)?;
        } else if let Some(file_name) = entry.file_name().to_str() {
            if file_name.ends_with(".test.toml") {
                match parse_flow_file(&path) {
                    Ok(flow) => flows.push(flow),
                    Err(e) => {
                        return Err(e)
                            .with_context(|| format!("failed to parse flow: {}", path.display()));
                    }
                }
            }
        }
    }

    Ok(())
}

fn parse_flow_file(path: &Path) -> Result<DiscoveredFlow> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read file: {}", path.display()))?;
    let flow_file = golem_parser::parse_flow(&content)?;

    Ok(DiscoveredFlow {
        path: path.to_path_buf(),
        name: flow_file.flow.name,
        tags: flow_file.flow.tags,
        explicit_only: flow_file.flow.explicit_only,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: create a flow TOML with given name and tags.
    fn flow_toml(name: &str, tags: &[&str]) -> String {
        if tags.is_empty() {
            format!("[flow]\nname = \"{name}\"\n", name = name)
        } else {
            let tags_str: Vec<String> = tags.iter().map(|t| format!("\"{t}\"")).collect();
            format!(
                "[flow]\nname = \"{name}\"\ntags = [{tags}]\n",
                name = name,
                tags = tags_str.join(", ")
            )
        }
    }

    /// Helper: write a file at a relative path under a temp dir, creating
    /// intermediate directories as needed.
    fn write_flow(dir: &Path, rel_path: &str, content: &str) {
        let full = dir.join(rel_path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(&full, content).expect("write file");
    }

    // ---------------------------------------------------------------
    // 1. Discover single flow file
    // ---------------------------------------------------------------
    #[test]
    fn discover_single_flow_file() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "login.test.toml", &flow_toml("login", &[]));

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "login");
        assert!(flows[0].path.ends_with("login.test.toml"));
    }

    // ---------------------------------------------------------------
    // 2. Discover flows recursively in subdirectories
    // ---------------------------------------------------------------
    #[test]
    fn discover_flows_recursively() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "top.test.toml", &flow_toml("top", &[]));
        write_flow(tmp.path(), "auth/login.test.toml", &flow_toml("login", &[]));
        write_flow(
            tmp.path(),
            "auth/deep/nested.test.toml",
            &flow_toml("nested", &[]),
        );

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(flows.len(), 3);
    }

    // ---------------------------------------------------------------
    // 3. Skip non-test.toml files
    // ---------------------------------------------------------------
    #[test]
    fn skip_non_test_toml_files() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "login.test.toml", &flow_toml("login", &[]));
        write_flow(tmp.path(), "config.toml", "[flow]\nname = \"config\"\n");
        write_flow(tmp.path(), "readme.md", "# README");
        write_flow(tmp.path(), "notes.test.txt", "not toml");

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "login");
    }

    // ---------------------------------------------------------------
    // 4. Skip .golem directory
    // ---------------------------------------------------------------
    #[test]
    fn skip_golem_directory() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "real.test.toml", &flow_toml("real", &[]));
        write_flow(
            tmp.path(),
            ".golem/internal.test.toml",
            &flow_toml("internal", &[]),
        );

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "real");
    }

    // ---------------------------------------------------------------
    // 5. Skip __fixtures__ directory
    // ---------------------------------------------------------------
    #[test]
    fn skip_fixtures_directory() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "real.test.toml", &flow_toml("real", &[]));
        write_flow(
            tmp.path(),
            "__fixtures__/fixture.test.toml",
            &flow_toml("fixture", &[]),
        );

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "real");
    }

    // ---------------------------------------------------------------
    // 6. Skip __mixins__ directory
    // ---------------------------------------------------------------
    #[test]
    fn skip_mixins_directory() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "real.test.toml", &flow_toml("real", &[]));
        write_flow(
            tmp.path(),
            "__mixins__/mixin.test.toml",
            &flow_toml("mixin", &[]),
        );

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "real");
    }

    // ---------------------------------------------------------------
    // 7. Skip hidden directories
    // ---------------------------------------------------------------
    #[test]
    fn skip_hidden_directories() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "real.test.toml", &flow_toml("real", &[]));
        write_flow(
            tmp.path(),
            ".hidden/secret.test.toml",
            &flow_toml("secret", &[]),
        );
        write_flow(
            tmp.path(),
            ".another/deep/nested.test.toml",
            &flow_toml("nested", &[]),
        );

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "real");
    }

    // ---------------------------------------------------------------
    // 8. Filter by single tag
    // ---------------------------------------------------------------
    #[test]
    fn filter_by_single_tag() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(
            tmp.path(),
            "smoke.test.toml",
            &flow_toml("smoke test", &["smoke", "auth"]),
        );
        write_flow(
            tmp.path(),
            "regression.test.toml",
            &flow_toml("regression test", &["regression"]),
        );

        let filters = [TagFilter::parse("smoke")];
        let flows = discover_flows(tmp.path(), &filters).expect("discover");
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "smoke test");
    }

    // ---------------------------------------------------------------
    // 9. Filter by multiple tags (AND)
    // ---------------------------------------------------------------
    #[test]
    fn filter_by_multiple_tags_and() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(
            tmp.path(),
            "both.test.toml",
            &flow_toml("both", &["smoke", "critical"]),
        );
        write_flow(
            tmp.path(),
            "smoke_only.test.toml",
            &flow_toml("smoke only", &["smoke"]),
        );
        write_flow(
            tmp.path(),
            "critical_only.test.toml",
            &flow_toml("critical only", &["critical"]),
        );

        let filters = [TagFilter::parse("smoke"), TagFilter::parse("critical")];
        let flows = discover_flows(tmp.path(), &filters).expect("discover");
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "both");
    }

    // ---------------------------------------------------------------
    // 10. OR filter within single tag clause
    // ---------------------------------------------------------------
    #[test]
    fn or_filter_within_single_clause() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(
            tmp.path(),
            "smoke.test.toml",
            &flow_toml("smoke", &["smoke"]),
        );
        write_flow(
            tmp.path(),
            "regression.test.toml",
            &flow_toml("regression", &["regression"]),
        );
        write_flow(
            tmp.path(),
            "perf.test.toml",
            &flow_toml("perf", &["performance"]),
        );

        let filters = [TagFilter::parse("smoke|regression")];
        let flows = discover_flows(tmp.path(), &filters).expect("discover");
        assert_eq!(flows.len(), 2);
        let names: Vec<&str> = flows.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"smoke"));
        assert!(names.contains(&"regression"));
    }

    // ---------------------------------------------------------------
    // 11. Composed AND + OR filter
    // ---------------------------------------------------------------
    #[test]
    fn composed_and_or_filter() {
        let tmp = TempDir::new().expect("tempdir");
        // (smoke OR regression) AND auth
        write_flow(
            tmp.path(),
            "a.test.toml",
            &flow_toml("smoke+auth", &["smoke", "auth"]),
        );
        write_flow(
            tmp.path(),
            "b.test.toml",
            &flow_toml("regression+auth", &["regression", "auth"]),
        );
        write_flow(
            tmp.path(),
            "c.test.toml",
            &flow_toml("smoke no auth", &["smoke"]),
        );
        write_flow(
            tmp.path(),
            "d.test.toml",
            &flow_toml("auth only", &["auth"]),
        );

        let filters = [
            TagFilter::parse("smoke|regression"),
            TagFilter::parse("auth"),
        ];
        let flows = discover_flows(tmp.path(), &filters).expect("discover");
        assert_eq!(flows.len(), 2);
        let names: Vec<&str> = flows.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"smoke+auth"));
        assert!(names.contains(&"regression+auth"));
    }

    // ---------------------------------------------------------------
    // 12. Untagged flows included when no filter
    // ---------------------------------------------------------------
    #[test]
    fn untagged_flows_included_when_no_filter() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(
            tmp.path(),
            "tagged.test.toml",
            &flow_toml("tagged", &["smoke"]),
        );
        write_flow(
            tmp.path(),
            "untagged.test.toml",
            &flow_toml("untagged", &[]),
        );

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(flows.len(), 2);
    }

    // ---------------------------------------------------------------
    // 13. Untagged flows excluded by any tag filter
    // ---------------------------------------------------------------
    #[test]
    fn untagged_flows_excluded_by_tag_filter() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(
            tmp.path(),
            "tagged.test.toml",
            &flow_toml("tagged", &["smoke"]),
        );
        write_flow(
            tmp.path(),
            "untagged.test.toml",
            &flow_toml("untagged", &[]),
        );

        let filters = [TagFilter::parse("smoke")];
        let flows = discover_flows(tmp.path(), &filters).expect("discover");
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "tagged");
    }

    // ---------------------------------------------------------------
    // 14. Empty directory returns empty list
    // ---------------------------------------------------------------
    #[test]
    fn empty_directory_returns_empty_list() {
        let tmp = TempDir::new().expect("tempdir");
        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert!(flows.is_empty());
    }

    // ---------------------------------------------------------------
    // 15. Skip target directory
    // ---------------------------------------------------------------
    #[test]
    fn skip_target_directory() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "real.test.toml", &flow_toml("real", &[]));
        write_flow(
            tmp.path(),
            "target/built.test.toml",
            &flow_toml("built", &[]),
        );

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(
            flows.len(),
            1,
            "target dir SHALL be excluded from discovery"
        );
        assert_eq!(flows[0].name, "real");
    }

    // ---------------------------------------------------------------
    // 16. Discovered flows are sorted by path for determinism
    // ---------------------------------------------------------------
    #[test]
    fn flows_sorted_by_path() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "zebra.test.toml", &flow_toml("zebra", &[]));
        write_flow(tmp.path(), "alpha.test.toml", &flow_toml("alpha", &[]));
        write_flow(tmp.path(), "mid/beta.test.toml", &flow_toml("beta", &[]));

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        let paths: Vec<PathBuf> = flows.iter().map(|f| f.path.clone()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "flows SHALL be sorted by path");
    }

    // ---------------------------------------------------------------
    // 17. Discovery preserves flow tags from parsed file
    // ---------------------------------------------------------------
    #[test]
    fn discovery_preserves_tags() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(
            tmp.path(),
            "tagged.test.toml",
            &flow_toml("tagged", &["smoke", "auth"]),
        );

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(flows.len(), 1);
        assert_eq!(
            flows[0].tags,
            vec!["smoke".to_string(), "auth".to_string()],
            "discovered flow SHALL carry parsed tags"
        );
    }

    /// Helper: a flow marked `explicit_only`, optionally tagged.
    fn explicit_only_toml(name: &str, tags: &[&str]) -> String {
        let mut toml = flow_toml(name, tags);
        toml.push_str("explicit_only = true\n");
        toml
    }

    // ---------------------------------------------------------------
    // 18. explicit_only flows are skipped by the tag-less sweep
    // ---------------------------------------------------------------
    #[test]
    fn explicit_only_excluded_from_untagged_scan() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "real.test.toml", &flow_toml("real", &[]));
        write_flow(
            tmp.path(),
            "subflows/login.test.toml",
            &explicit_only_toml("login subflow", &[]),
        );

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(flows.len(), 1, "explicit_only flow SHALL be skipped by the bare sweep");
        assert_eq!(flows[0].name, "real");
    }

    // ---------------------------------------------------------------
    // 19. A matching --tag opts an explicit_only flow back in
    // ---------------------------------------------------------------
    #[test]
    fn explicit_only_included_when_tag_matches() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(
            tmp.path(),
            "subflows/login.test.toml",
            &explicit_only_toml("login subflow", &["login"]),
        );

        let filters = [TagFilter::parse("login")];
        let flows = discover_flows(tmp.path(), &filters).expect("discover");
        assert_eq!(flows.len(), 1, "matching --tag SHALL include an explicit_only flow");
        assert_eq!(flows[0].name, "login subflow");
    }

    // ---------------------------------------------------------------
    // 20. A non-matching --tag still excludes an explicit_only flow
    // ---------------------------------------------------------------
    #[test]
    fn explicit_only_excluded_when_tag_absent() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(
            tmp.path(),
            "subflows/login.test.toml",
            &explicit_only_toml("login subflow", &["login"]),
        );

        let filters = [TagFilter::parse("smoke")];
        let flows = discover_flows(tmp.path(), &filters).expect("discover");
        assert!(flows.is_empty(), "non-matching --tag SHALL NOT include the flow");
    }

    // ---------------------------------------------------------------
    // 21. Flows without the field default to discoverable (backward compat)
    // ---------------------------------------------------------------
    #[test]
    fn explicit_only_defaults_false() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "plain.test.toml", &flow_toml("plain", &[]));

        let flows = discover_flows(tmp.path(), &[]).expect("discover");
        assert_eq!(flows.len(), 1);
        assert!(!flows[0].explicit_only, "absent field SHALL default to false");
    }

    // ---------------------------------------------------------------
    // 18. Non-existent root directory yields an error
    // ---------------------------------------------------------------
    #[test]
    fn nonexistent_root_errors() {
        let tmp = TempDir::new().expect("tempdir");
        let missing = tmp.path().join("does_not_exist");

        let result = discover_flows(&missing, &[]);
        assert!(result.is_err(), "missing root SHALL produce an error");
        let msg = format!("{:#}", result.err().expect("error present"));
        assert!(
            msg.contains("failed to read directory"),
            "error SHALL carry read-directory context, got: {msg}"
        );
    }

    // ---------------------------------------------------------------
    // 19. Malformed flow TOML yields a parse error with path context
    // ---------------------------------------------------------------
    #[test]
    fn malformed_flow_file_errors() {
        let tmp = TempDir::new().expect("tempdir");
        // Missing required [flow].name field.
        write_flow(tmp.path(), "bad.test.toml", "[flow]\n");

        let result = discover_flows(tmp.path(), &[]);
        assert!(result.is_err(), "unparseable flow SHALL produce an error");
        let msg = format!("{:#}", result.err().expect("error present"));
        assert!(
            msg.contains("failed to parse flow") && msg.contains("bad.test.toml"),
            "error SHALL carry the offending file path context, got: {msg}"
        );
    }

    // ---------------------------------------------------------------
    // 20. Syntactically invalid TOML yields an error
    // ---------------------------------------------------------------
    #[test]
    fn invalid_toml_syntax_errors() {
        let tmp = TempDir::new().expect("tempdir");
        write_flow(tmp.path(), "broken.test.toml", "this is not = = toml");

        let result = discover_flows(tmp.path(), &[]);
        assert!(result.is_err(), "invalid TOML SHALL produce an error");
        let msg = format!("{:#}", result.err().expect("error present"));
        // 1. Syntactic-TOML failures take the same parse branch and SHALL be
        //    wrapped with both the parse context and the offending file path.
        assert!(
            msg.contains("failed to parse flow") && msg.contains("broken.test.toml"),
            "syntax-error SHALL carry parse + file-path context, got: {msg}"
        );
    }

    // ---------------------------------------------------------------
    // 21. TagFilter::parse trims whitespace around alternatives
    // ---------------------------------------------------------------
    #[test]
    fn tag_filter_parse_trims_whitespace() {
        let filter = TagFilter::parse("  smoke  |  regression ");
        assert_eq!(
            filter.alternatives,
            vec!["smoke".to_string(), "regression".to_string()],
            "parse SHALL trim whitespace around each alternative"
        );
    }

    // ---------------------------------------------------------------
    // 22. TagFilter::parse drops empty alternatives
    // ---------------------------------------------------------------
    #[test]
    fn tag_filter_parse_drops_empty_alternatives() {
        let filter = TagFilter::parse("smoke||  |regression");
        assert_eq!(
            filter.alternatives,
            vec!["smoke".to_string(), "regression".to_string()],
            "parse SHALL drop empty/whitespace-only alternatives"
        );
    }

    // ---------------------------------------------------------------
    // 23. TagFilter::parse on empty/blank string yields no alternatives
    // ---------------------------------------------------------------
    #[test]
    fn tag_filter_parse_empty_string() {
        assert!(
            TagFilter::parse("").alternatives.is_empty(),
            "empty string SHALL parse to no alternatives"
        );
        assert!(
            TagFilter::parse("   ").alternatives.is_empty(),
            "blank string SHALL parse to no alternatives"
        );
        assert!(
            TagFilter::parse("|").alternatives.is_empty(),
            "lone pipe SHALL parse to no alternatives"
        );
    }

    // ---------------------------------------------------------------
    // 24. TagFilter::matches with empty alternatives never matches
    // ---------------------------------------------------------------
    #[test]
    fn tag_filter_empty_never_matches() {
        let filter = TagFilter::parse("");
        assert!(
            !filter.matches(&["smoke".to_string()]),
            "empty filter SHALL NOT match any tags"
        );
        assert!(
            !filter.matches(&[]),
            "empty filter SHALL NOT match empty tags"
        );
    }

    // ---------------------------------------------------------------
    // 25. TagFilter::matches against empty flow tags
    // ---------------------------------------------------------------
    #[test]
    fn tag_filter_matches_empty_flow_tags() {
        let filter = TagFilter::parse("smoke");
        assert!(
            !filter.matches(&[]),
            "non-empty filter SHALL NOT match a flow with no tags"
        );
    }

    // ---------------------------------------------------------------
    // 26. TagFilter::matches requires exact tag (no substring/prefix)
    // ---------------------------------------------------------------
    #[test]
    fn tag_filter_requires_exact_match() {
        let filter = TagFilter::parse("smoke");
        assert!(
            !filter.matches(&["smoketest".to_string()]),
            "filter SHALL match tags exactly, not by substring"
        );
        assert!(
            filter.matches(&["smoke".to_string()]),
            "filter SHALL match an exact tag"
        );
    }
}
