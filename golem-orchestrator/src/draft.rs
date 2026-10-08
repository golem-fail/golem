//! A session's flow draft: the `.test.toml` an LLM builds while it works.
//!
//! The draft is a `toml_edit` document, so a draft that starts from an
//! existing flow file keeps that file's comments, key order and whitespace
//! byte for byte; edits only add. Each step that passes in the session is
//! recorded at the insertion point, in the canonical one-line form.
//!
//! TOML allows a block to hold its steps either as `steps = [ … ]` or as
//! `[[block.steps]]` tables, but not both in one block. A recorded step
//! takes the form its block already uses; a new block uses the inline
//! array.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

/// Where the next recorded step goes: a block, and a position in its steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insertion {
    /// Index into the `[[block]]` array.
    pub block: usize,
    /// Index in the block's steps; `None` appends.
    pub at: Option<usize>,
}

/// A flow draft.
#[derive(Debug, Clone)]
pub struct Draft {
    doc: DocumentMut,
    /// The flow file the session opened from, and every file it exported
    /// to: writing over these needs no `overwrite`.
    own: Vec<PathBuf>,
    insertion: Option<Insertion>,
    /// Steps recorded without running them (`record_only`), for the export
    /// report.
    unverified: Vec<String>,
}

/// What an export wrote.
#[derive(Debug, Clone)]
pub struct Exported {
    pub path: PathBuf,
    pub steps: usize,
    pub unverified: Vec<String>,
}

impl Draft {
    /// An empty draft: `[flow]` with `name`, and one `[[flow.apps]]` entry
    /// for the app the session drives when there is one.
    pub fn new(name: &str, app: Option<(&str, &str, &str)>) -> Draft {
        let mut text = format!("[flow]\nname = {}\n", toml_string(name));
        if let Some((app_name, bundle, os)) = app {
            text.push_str(&format!(
                "\n[[flow.apps]]\nname = {}\nbundle = {}\n[[flow.apps.devices]]\nos = {}\n",
                toml_string(app_name),
                toml_string(bundle),
                toml_string(os)
            ));
        }
        Draft {
            doc: text.parse().unwrap_or_default(),
            own: Vec::new(),
            insertion: None,
            unverified: Vec::new(),
        }
    }

    /// A draft of an existing flow file, to add to.
    pub fn from_file(path: &Path) -> Result<Draft> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Ok(Draft {
            doc: text
                .parse()
                .with_context(|| format!("failed to parse {}", path.display()))?,
            own: vec![path.to_path_buf()],
            insertion: None,
            unverified: Vec::new(),
        })
    }

    /// The draft as TOML text.
    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    pub fn insertion(&self) -> Option<&Insertion> {
        self.insertion.as_ref()
    }

    /// Record the next steps before the 1-based `step` of block `name`, or
    /// at its end.
    pub fn insert_at(&mut self, name: &str, step: Option<usize>) -> Result<()> {
        let block = self
            .block_index(name)
            .with_context(|| format!("the draft has no block named {name:?}"))?;
        self.insertion = Some(Insertion {
            block,
            at: step.map(|s| s.saturating_sub(1)),
        });
        Ok(())
    }

    /// Record the next steps at the end of the last block.
    pub fn insert_at_end(&mut self) {
        self.insertion = self
            .blocks()
            .map(|b| b.len())
            .filter(|n| *n > 0)
            .map(|n| Insertion {
                block: n - 1,
                at: None,
            });
    }

    /// Record a step that passed: `line` in the canonical one-line form,
    /// with `comment` on its own line above it.
    pub fn record(&mut self, line: &str, comment: Option<&str>) -> Result<()> {
        let step: Value = line
            .parse()
            .with_context(|| format!("not a one-line step: {line}"))?;
        let Value::InlineTable(step) = step else {
            bail!("not a one-line step: {line}");
        };
        let insertion = match self.insertion.clone() {
            Some(i) => i,
            None => {
                self.new_block("main")?;
                self.insertion
                    .clone()
                    .context("a new block sets the insertion point")?
            }
        };
        let at = self.insert_step(&insertion, step, comment)?;
        self.insertion = Some(Insertion {
            block: insertion.block,
            at: insertion.at.map(|_| at + 1),
        });
        self.reparse()
    }

    /// Record a step that did not run, marked `# unverified`.
    pub fn record_unverified(&mut self, line: &str, comment: Option<&str>) -> Result<()> {
        let marked = match comment {
            Some(c) => format!("unverified: {c}"),
            None => "unverified".to_string(),
        };
        self.record(line, Some(&marked))?;
        self.unverified.push(line.to_string());
        Ok(())
    }

    /// Add a `[[block]]` named `name` at the end, in the inline-array form,
    /// and move the insertion point to it.
    pub fn new_block(&mut self, name: &str) -> Result<()> {
        if self.block_index(name).is_some() {
            bail!("the draft already has a block named {name:?}");
        }
        let mut table = Table::new();
        table.insert("name", toml_edit::value(name));
        let mut steps = toml_edit::Array::new();
        steps.set_trailing("\n");
        steps.set_trailing_comma(true);
        table.insert("steps", Item::Value(Value::Array(steps)));
        table.decor_mut().set_prefix("\n");
        let root = self.doc.as_table_mut();
        if !root.contains_key("block") {
            root.insert("block", Item::ArrayOfTables(ArrayOfTables::new()));
        }
        let blocks = root["block"]
            .as_array_of_tables_mut()
            .context("`block` is not an array of tables")?;
        blocks.push(table);
        let index = blocks.len() - 1;
        self.reparse()?;
        self.insertion = Some(Insertion {
            block: index,
            at: None,
        });
        Ok(())
    }

    /// Check the draft as `golem run` would, then write it to `path`.
    /// Refuses to replace a file the session did not open from unless
    /// `overwrite`.
    pub fn export(&mut self, path: &Path, overwrite: bool) -> Result<Exported> {
        let text = self.text();
        let flow = golem_parser::parse_flow(&text).context("the draft does not parse as a flow")?;
        let errors = golem_parser::validation::validate_flow(&flow);
        if !errors.is_empty() {
            let detail: Vec<String> = errors.into_iter().map(|e| e.message).collect();
            bail!("the draft does not validate: {}", detail.join("; "));
        }
        let same_as_source = self.own.iter().any(|s| same_file(s, path));
        if path.exists() && !same_as_source && !overwrite {
            bail!(
                "{} exists and the session did not open from it; pass overwrite = true to replace it",
                path.display()
            );
        }
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
        std::fs::write(path, &text)
            .with_context(|| format!("failed to write {}", path.display()))?;
        if !same_as_source {
            self.own.push(path.to_path_buf());
        }
        Ok(Exported {
            path: path.to_path_buf(),
            steps: flow.block.iter().map(|b| b.steps.len()).sum(),
            unverified: self.unverified.clone(),
        })
    }

    fn blocks(&self) -> Option<&ArrayOfTables> {
        self.doc.get("block").and_then(Item::as_array_of_tables)
    }

    fn block_index(&self, name: &str) -> Option<usize> {
        self.blocks()?
            .iter()
            .position(|b| b.get("name").and_then(Item::as_str) == Some(name))
    }

    /// Insert `step` into the block, in the form the block already uses.
    /// Returns the index it went to.
    fn insert_step(
        &mut self,
        ins: &Insertion,
        step: InlineTable,
        comment: Option<&str>,
    ) -> Result<usize> {
        let block = self
            .doc
            .get_mut("block")
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|b| b.get_mut(ins.block))
            .context("the insertion point names a block the draft no longer has")?;
        match block.get("steps") {
            None => {
                let mut steps = toml_edit::Array::new();
                steps.set_trailing("\n");
                steps.set_trailing_comma(true);
                block.insert("steps", Item::Value(Value::Array(steps)));
            }
            Some(Item::Value(Value::Array(_))) | Some(Item::ArrayOfTables(_)) => {}
            Some(_) => bail!("the block's `steps` is neither an array nor [[block.steps]] tables"),
        }
        if let Some(Item::Value(Value::Array(steps))) = block.get_mut("steps") {
            // A one-line `steps = [{ … }]` has no room for a comment line:
            // it becomes one step per line, the house form, and only its own
            // lines change.
            let single_line = !steps.is_empty()
                && !steps.iter().any(|v| {
                    v.decor()
                        .prefix()
                        .and_then(|p| p.as_str())
                        .is_some_and(|p| p.contains('\n'))
                });
            if single_line {
                for v in steps.iter_mut() {
                    v.decor_mut().set_prefix("\n  ");
                    v.decor_mut().set_suffix("");
                }
                steps.set_trailing("\n");
                steps.set_trailing_comma(true);
            }
            let at = ins.at.unwrap_or(steps.len()).min(steps.len());
            let indent = element_indent(steps);
            let mut value = Value::InlineTable(step);
            // Appending after the last element: a comment that ends its line
            // lives in the array's trailing text, so it moves to the new
            // element's prefix to stay on its line.
            let lead = if at == steps.len() && !steps.is_empty() {
                let trailing = steps.trailing().as_str().unwrap_or("\n").to_string();
                let kept = trailing.trim_end_matches([' ', '\t']);
                if kept.ends_with('\n') {
                    kept.to_string()
                } else {
                    format!("{kept}\n")
                }
            } else {
                "\n".to_string()
            };
            let prefix = match comment {
                Some(c) => format!("{lead}{indent}# {}\n{indent}", one_line(c)),
                None => format!("{lead}{indent}"),
            };
            if at == steps.len() {
                steps.set_trailing("\n");
            }
            value.decor_mut().set_prefix(prefix);
            value.decor_mut().set_suffix("");
            steps.insert_formatted(at, value);
            if steps.trailing().as_str().is_none_or(|t| !t.contains('\n')) {
                steps.set_trailing("\n");
            }
            steps.set_trailing_comma(true);
            return Ok(at);
        }
        // `[[block.steps]]`: tables print in document order, so the new one
        // takes the position of the table it goes before, and every later
        // table moves down one.
        let tables = block
            .get("steps")
            .and_then(Item::as_array_of_tables)
            .context("`steps` changed form")?;
        let mut positions: Vec<usize> = tables.iter().filter_map(Table::position).collect();
        positions.sort_unstable();
        let at = ins.at.unwrap_or(positions.len()).min(positions.len());
        let position = match positions.get(at) {
            Some(p) => *p,
            None => positions.last().map_or(0, |p| p + 1),
        };
        bump_positions(self.doc.as_item_mut(), position);
        let mut table = Table::new();
        for (key, v) in step.iter() {
            let mut v = v.clone();
            v.decor_mut().clear();
            table.insert(key, Item::Value(v));
        }
        table.decor_mut().set_prefix(match comment {
            Some(c) => format!("\n# {}\n", one_line(c)),
            None => "\n".to_string(),
        });
        table.set_position(position);
        self.doc
            .get_mut("block")
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|b| b.get_mut(ins.block))
            .and_then(|b| b.get_mut("steps"))
            .and_then(Item::as_array_of_tables_mut)
            .context("`steps` changed form")?
            .push(table);
        Ok(at)
    }

    /// Parse the text again, so the document's order matches what it prints.
    fn reparse(&mut self) -> Result<()> {
        self.doc = self
            .doc
            .to_string()
            .parse()
            .context("the draft no longer parses")?;
        Ok(())
    }
}

/// The indent of an array's elements, from its first element; two spaces
/// when it has none.
fn element_indent(steps: &toml_edit::Array) -> String {
    steps
        .iter()
        .next()
        .and_then(|v| v.decor().prefix())
        .and_then(|p| p.as_str())
        .filter(|p| p.contains('\n'))
        .and_then(|p| p.rsplit('\n').next())
        .filter(|i| i.chars().all(|c| c == ' ' || c == '\t'))
        .map_or_else(|| "  ".to_string(), str::to_string)
}

fn bump_positions(item: &mut Item, from: usize) {
    let bump = |t: &mut Table| {
        if let Some(p) = t.position() {
            if p >= from {
                t.set_position(p + 1);
            }
        }
    };
    match item {
        Item::Table(t) => {
            bump(t);
            for (_, v) in t.iter_mut() {
                bump_positions(v, from);
            }
        }
        Item::ArrayOfTables(a) => {
            for t in a.iter_mut() {
                bump(t);
                for (_, v) in t.iter_mut() {
                    bump_positions(v, from);
                }
            }
        }
        _ => {}
    }
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

fn toml_string(s: &str) -> String {
    Value::from(s).to_string().trim().to_string()
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INLINE: &str = r#"[flow]
name = "Login"

[[flow.apps]]
name = "app"
[[flow.apps.devices]]
os = "android:latest"

# Open the form first.
[[block]]
name = "main"
steps = [
  # Launch fresh.
  { action = "launch", app = "app", restart = true },
  { action = "tap", on_text = "Log in" },  # the header button
]

[[block]]
name = "after"
steps = [{ action = "assert_visible", on_text = "Hi" }]
"#;

    const TABLES: &str = r#"[flow]
name = "Tables"

[[block]]
name = "main"

[[block.steps]]
action = "launch"
app = "app"

# Then tap.
[[block.steps]]
action = "tap"
on_text = "Go"

[[block]]
name = "end"
steps = [{ action = "assert_visible", on_text = "Done" }]
"#;

    fn draft_of(text: &str) -> (tempfile::TempDir, Draft) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f.test.toml");
        std::fs::write(&path, text).expect("write");
        let d = Draft::from_file(&path).expect("draft");
        (dir, d)
    }

    #[test]
    fn steps_append_to_an_inline_block_and_every_existing_byte_stays() {
        let (_dir, mut d) = draft_of(INLINE);
        d.insert_at("main", None).expect("insert");
        d.record(
            r#"{ action = "type", on_text = "Email", input = "a@b.test" }"#,
            None,
        )
        .expect("record");
        d.record(
            r#"{ action = "tap", on_text = "Submit" }"#,
            Some("Send the form"),
        )
        .expect("record");
        let want = INLINE.replace(
            "  { action = \"tap\", on_text = \"Log in\" },  # the header button\n]",
            "  { action = \"tap\", on_text = \"Log in\" },  # the header button\n  \
             { action = \"type\", on_text = \"Email\", input = \"a@b.test\" },\n  \
             # Send the form\n  { action = \"tap\", on_text = \"Submit\" },\n]",
        );
        assert_eq!(d.text(), want);
    }

    #[test]
    fn steps_insert_mid_block_where_the_flow_stopped() {
        let (_dir, mut d) = draft_of(INLINE);
        d.insert_at("main", Some(2)).expect("insert");
        d.record(
            r#"{ action = "assert_visible", on_text = "Welcome" }"#,
            None,
        )
        .expect("record");
        d.record(r#"{ action = "hide_keyboard" }"#, None)
            .expect("record");
        let want = INLINE.replace(
            "  { action = \"launch\", app = \"app\", restart = true },\n",
            "  { action = \"launch\", app = \"app\", restart = true },\n  \
             { action = \"assert_visible\", on_text = \"Welcome\" },\n  \
             { action = \"hide_keyboard\" },\n",
        );
        assert_eq!(d.text(), want, "steps SHALL go in order at the stop point");
    }

    #[test]
    fn a_one_line_steps_array_becomes_one_step_per_line() {
        let (_dir, mut d) = draft_of(INLINE);
        d.insert_at("after", Some(1)).expect("insert");
        d.record(r#"{ action = "tap", on_text = "+" }"#, Some("Count one"))
            .expect("record");
        let want = INLINE.replace(
            "steps = [{ action = \"assert_visible\", on_text = \"Hi\" }]",
            "steps = [\n  # Count one\n  { action = \"tap\", on_text = \"+\" },\n  \
             { action = \"assert_visible\", on_text = \"Hi\" },\n]",
        );
        assert_eq!(d.text(), want, "only the array's own lines SHALL change");
    }

    #[test]
    fn a_tables_block_gets_a_table_with_the_comment_above() {
        let (_dir, mut d) = draft_of(TABLES);
        d.insert_at("main", None).expect("insert");
        d.record(
            r#"{ action = "assert_visible", on_text = "Next" }"#,
            Some("Check"),
        )
        .expect("record");
        let want = TABLES.replace(
            "on_text = \"Go\"\n\n[[block]]",
            "on_text = \"Go\"\n\n# Check\n[[block.steps]]\naction = \"assert_visible\"\non_text = \"Next\"\n\n[[block]]",
        );
        assert_eq!(d.text(), want);
        let flow = golem_parser::parse_flow(&d.text()).expect("parse");
        assert_eq!(flow.block[0].steps.len(), 3);
    }

    #[test]
    fn a_tables_block_takes_a_step_mid_block() {
        let (_dir, mut d) = draft_of(TABLES);
        d.insert_at("main", Some(2)).expect("insert");
        d.record(r#"{ action = "hide_keyboard" }"#, None)
            .expect("record");
        let flow = golem_parser::parse_flow(&d.text()).expect("parse");
        let actions: Vec<&str> = flow.block[0]
            .steps
            .iter()
            .map(|s| s.action.as_str())
            .collect();
        assert_eq!(actions, ["launch", "hide_keyboard", "tap"]);
        assert!(
            d.text()
                .contains("# Then tap.\n[[block.steps]]\naction = \"tap\""),
            "{}",
            d.text()
        );
    }

    #[test]
    fn a_new_draft_gets_a_main_block_in_the_house_form() {
        let mut d = Draft::new("Checkout", Some(("app", "com.acme", "ios:latest")));
        d.record(r#"{ action = "launch", app = "app" }"#, Some("Start clean"))
            .expect("record");
        d.record(r#"{ action = "tap", on_text = "Buy" }"#, None)
            .expect("record");
        assert_eq!(
            d.text(),
            "[flow]\nname = \"Checkout\"\n\n[[flow.apps]]\nname = \"app\"\nbundle = \"com.acme\"\n\
             [[flow.apps.devices]]\nos = \"ios:latest\"\n\n[[block]]\nname = \"main\"\nsteps = [\n  \
             # Start clean\n  { action = \"launch\", app = \"app\" },\n  \
             { action = \"tap\", on_text = \"Buy\" },\n]\n"
        );
    }

    #[test]
    fn export_round_trips_the_steps() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut d = Draft::new("Round trip", Some(("app", "com.acme", "android:latest")));
        let lines = [
            r#"{ action = "launch", app = "app" }"#,
            r#"{ action = "type", on_label = "Email", input = "x" }"#,
            r#"{ action = "assert_visible", on_text = "Hi", auto_scroll = true }"#,
        ];
        for l in &lines[..1] {
            d.record(l, None).expect("record");
        }
        d.record(
            r#"{ action = "tap", on = { text = "OK", below = "Title" } }"#,
            None,
        )
        .expect("record");
        let out = dir.path().join("flows/r.test.toml");
        let done = d.export(&out, false).expect("export");
        assert_eq!(done.steps, 2);
        let flow =
            golem_parser::parse_flow(&std::fs::read_to_string(&out).expect("read")).expect("parse");
        assert_eq!(flow.block[0].steps[0].action, "launch");
        assert_eq!(
            flow.block[0].steps[1]
                .on
                .as_ref()
                .and_then(|g| g.text.as_deref()),
            Some("OK")
        );
    }

    #[test]
    fn export_refuses_a_draft_that_does_not_validate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut d = Draft::new("Bad", Some(("app", "com.acme", "android:latest")));
        d.record(r#"{ action = "open_link" }"#, None)
            .expect("record");
        let err = d
            .export(&dir.path().join("b.test.toml"), false)
            .expect_err("invalid")
            .to_string();
        assert!(err.contains("open_link requires 'url'"), "{err}");
    }

    #[test]
    fn export_will_not_replace_another_file_without_overwrite() {
        let (dir, mut d) = draft_of(INLINE);
        let other = dir.path().join("other.test.toml");
        std::fs::write(&other, "keep me").expect("write");
        let err = d.export(&other, false).expect_err("protected").to_string();
        assert!(err.contains("overwrite = true"), "{err}");
        assert_eq!(std::fs::read_to_string(&other).expect("read"), "keep me");
        d.export(&other, true).expect("overwrite");
        d.export(&dir.path().join("f.test.toml"), false)
            .expect("the source file SHALL take an export without overwrite");
    }

    #[test]
    fn an_unverified_step_carries_its_marker_and_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut d = Draft::new("U", Some(("app", "com.acme", "android:latest")));
        d.record_unverified(
            r#"{ action = "tap", on_text = "Retry" }"#,
            Some("error path"),
        )
        .expect("record");
        assert!(
            d.text()
                .contains("# unverified: error path\n  { action = \"tap\", on_text = \"Retry\" },"),
            "{}",
            d.text()
        );
        let done = d
            .export(&dir.path().join("u.test.toml"), false)
            .expect("export");
        assert_eq!(
            done.unverified,
            [r#"{ action = "tap", on_text = "Retry" }"#]
        );
    }
}
