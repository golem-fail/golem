use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// A discovered flow file with its metadata.
pub struct DiscoveredFlow {
    pub path: PathBuf,
    pub name: String,
    pub tags: Vec<String>,
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
        self.alternatives
            .iter()
            .any(|alt| flow_tags.contains(alt))
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
            format!(
                "[flow]\nname = \"{name}\"\n",
                name = name
            )
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
        write_flow(
            tmp.path(),
            "auth/login.test.toml",
            &flow_toml("login", &[]),
        );
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
}
