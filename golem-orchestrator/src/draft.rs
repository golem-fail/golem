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
    /// A comment for the next step into a `[[block.steps]]` block, which
    /// has no place for a standalone comment between tables.
    pending_comment: Option<String>,
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
            pending_comment: None,
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
            pending_comment: None,
        })
    }

    /// The draft as TOML text.
    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    pub fn insertion(&self) -> Option<&Insertion> {
        self.insertion.as_ref()
    }

    /// Where the next step goes, for people: `block "main", before step 3`.
    pub fn describe_insertion(&self) -> String {
        let Some(ins) = &self.insertion else {
            return "a new block \"main\"".to_string();
        };
        let name = self
            .blocks()
            .and_then(|b| b.get(ins.block))
            .and_then(|b| b.get("name"))
            .and_then(Item::as_str)
            .unwrap_or("?");
        match ins.at {
            Some(at) => format!("block {name:?}, before step {}", at + 1),
            None => format!("the end of block {name:?}"),
        }
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
        self.insert_step_in("block", ins.block, ins.at, step, comment)
    }

    /// Insert `step` into the steps of table `index` of the `array_key`
    /// array of tables (`block` or `teardown`), in the form that table
    /// already uses. Returns the index it went to.
    fn insert_step_in(
        &mut self,
        array_key: &str,
        index: usize,
        at: Option<usize>,
        step: InlineTable,
        comment: Option<&str>,
    ) -> Result<usize> {
        let ins = Insertion { block: index, at };
        let block = self
            .doc
            .get_mut(array_key)
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|b| b.get_mut(ins.block))
            .context("the draft has no such table to insert into")?;
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
        let pending = self.pending_comment.take().map(|c| format!("# {c}\n"));
        table.decor_mut().set_prefix(match comment {
            Some(c) => format!("\n{}# {}\n", pending.unwrap_or_default(), one_line(c)),
            None => format!("\n{}", pending.unwrap_or_default()),
        });
        table.set_position(position);
        self.doc
            .get_mut(array_key)
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

/// The `[flow]` fields `flow_set` changes; `None` leaves a field as it is.
#[derive(Debug, Clone, Default)]
pub struct FlowSet {
    pub name: Option<String>,
    pub tags: Option<Vec<String>>,
    /// Merged into `[flow] vars`.
    pub vars: Option<serde_json::Map<String, serde_json::Value>>,
    pub seed: Option<u64>,
    pub explicit_only: Option<bool>,
    pub start: Option<String>,
}

/// A mixin the project has: its name, its file and the `${var}`s it uses.
#[derive(Debug, Clone)]
pub struct Mixin {
    pub name: String,
    pub path: PathBuf,
    pub vars: Vec<String>,
}

impl Draft {
    /// Set fields of `[flow]`.
    pub fn flow_set(&mut self, set: &FlowSet) -> Result<()> {
        let flow = self.flow_table()?;
        if let Some(name) = &set.name {
            flow.insert("name", toml_edit::value(name.as_str()));
        }
        if let Some(tags) = &set.tags {
            let mut array = toml_edit::Array::new();
            for t in tags {
                array.push(t.as_str());
            }
            flow.insert("tags", Item::Value(Value::Array(array)));
        }
        if let Some(seed) = set.seed {
            flow.insert(
                "seed",
                toml_edit::value(i64::try_from(seed).context("seed is too large")?),
            );
        }
        if let Some(explicit_only) = set.explicit_only {
            flow.insert("explicit_only", toml_edit::value(explicit_only));
        }
        if let Some(start) = &set.start {
            flow.insert("start", toml_edit::value(start.as_str()));
        }
        if let Some(vars) = &set.vars {
            match flow.get_mut("vars") {
                Some(Item::Table(t)) => {
                    for (k, v) in vars {
                        t.insert(k, Item::Value(json_to_value(v)?));
                    }
                }
                Some(Item::Value(Value::InlineTable(t))) => {
                    for (k, v) in vars {
                        t.insert(k, json_to_value(v)?);
                    }
                }
                _ => {
                    let mut t = InlineTable::new();
                    for (k, v) in vars {
                        t.insert(k, json_to_value(v)?);
                    }
                    flow.insert("vars", Item::Value(Value::InlineTable(t)));
                }
            }
        }
        self.reparse()
    }

    /// Add or replace the `[[flow.apps]]` entry named `app.name`: `bundle`,
    /// `install_script`, `devices` (a list of constraint tables) and
    /// `permissions`. These can differ from the device the session uses.
    pub fn app_set(&mut self, app: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
        let name = app
            .get("name")
            .and_then(serde_json::Value::as_str)
            .context("an app needs a `name`")?
            .to_string();
        let existing = self
            .doc
            .get("flow")
            .and_then(|f| f.get("apps"))
            .and_then(Item::as_array_of_tables)
            .and_then(|apps| {
                apps.iter()
                    .position(|a| a.get("name").and_then(Item::as_str) == Some(name.as_str()))
            });
        let fill = |t: &mut Table| -> Result<()> {
            for (key, v) in app {
                if key == "name" {
                    continue;
                }
                t.insert(key, Item::Value(json_to_value(v)?));
            }
            Ok(())
        };
        match existing {
            Some(i) => {
                let t = self
                    .doc
                    .get_mut("flow")
                    .and_then(|f| f.get_mut("apps"))
                    .and_then(Item::as_array_of_tables_mut)
                    .and_then(|a| a.get_mut(i))
                    .context("the app went away")?;
                fill(t)?;
            }
            None => {
                let position = self.max_position_in("flow").map_or(1, |p| p + 1);
                bump_positions(self.doc.as_item_mut(), position);
                let mut t = Table::new();
                t.insert("name", toml_edit::value(name.as_str()));
                fill(&mut t)?;
                t.decor_mut().set_prefix("\n");
                t.set_position(position);
                let flow = self.flow_table()?;
                if !flow.contains_key("apps") {
                    flow.insert("apps", Item::ArrayOfTables(ArrayOfTables::new()));
                }
                flow["apps"]
                    .as_array_of_tables_mut()
                    .context("`flow.apps` is not [[flow.apps]] tables")?
                    .push(t);
            }
        }
        self.reparse()
    }

    /// Move the insertion point to the end of block `name`, creating the
    /// block when the draft has none by that name. `next` sets the block
    /// that follows it.
    pub fn block_begin(&mut self, name: &str, next: Option<&str>) -> Result<()> {
        if self.block_index(name).is_some() {
            self.insert_at(name, None)?;
        } else {
            self.new_block(name)?;
        }
        if next.is_some() {
            self.block_link(name, next, &[])?;
        }
        Ok(())
    }

    /// Set `next` on block `name`, and add `branches`: each a table with a
    /// condition (`if_visible`, `if_not_visible`, or `if_var` with
    /// `equals`, `matches` or `gte`) and a `goto`.
    pub fn block_link(
        &mut self,
        name: &str,
        next: Option<&str>,
        branches: &[serde_json::Value],
    ) -> Result<()> {
        let index = self
            .block_index(name)
            .with_context(|| format!("the draft has no block named {name:?}"))?;
        let block = self
            .doc
            .get_mut("block")
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|b| b.get_mut(index))
            .context("the block went away")?;
        if let Some(next) = next {
            block.insert("next", toml_edit::value(next));
        }
        if !branches.is_empty() {
            let mut values = Vec::new();
            for b in branches {
                let Value::InlineTable(t) = json_to_value(b)? else {
                    bail!("a branch is a table: {{ if_visible = \"…\", goto = \"…\" }}");
                };
                values.push(t);
            }
            match block.get_mut("branch") {
                Some(Item::ArrayOfTables(_)) => {
                    bail!("block {name:?} writes its branches as [[block.branch]] tables; edit them in the file")
                }
                Some(Item::Value(Value::Array(array))) => {
                    for t in values {
                        let mut v = Value::InlineTable(t);
                        v.decor_mut().set_prefix("\n  ");
                        array.push_formatted(v);
                    }
                    array.set_trailing("\n");
                    array.set_trailing_comma(true);
                }
                _ => {
                    let mut array = toml_edit::Array::new();
                    for t in values {
                        let mut v = Value::InlineTable(t);
                        v.decor_mut().set_prefix("\n  ");
                        array.push_formatted(v);
                    }
                    array.set_trailing("\n");
                    array.set_trailing_comma(true);
                    block.insert("branch", Item::Value(Value::Array(array)));
                }
            }
        }
        self.reparse()
    }

    /// Add a step to the first `[[teardown]]`, creating one when the draft
    /// has none. The step does not run.
    pub fn teardown_add(&mut self, line: &str, comment: Option<&str>) -> Result<()> {
        let step = parse_inline(line)?;
        if self
            .doc
            .get("teardown")
            .and_then(Item::as_array_of_tables)
            .is_none_or(|t| t.is_empty())
        {
            let position = self.max_position_in_doc().map_or(1, |p| p + 1);
            let mut t = Table::new();
            let mut steps = toml_edit::Array::new();
            steps.set_trailing("\n");
            steps.set_trailing_comma(true);
            t.insert("steps", Item::Value(Value::Array(steps)));
            t.decor_mut().set_prefix("\n");
            t.set_position(position);
            let root = self.doc.as_table_mut();
            if !root.contains_key("teardown") {
                root.insert("teardown", Item::ArrayOfTables(ArrayOfTables::new()));
            }
            root["teardown"]
                .as_array_of_tables_mut()
                .context("`teardown` is not [[teardown]] tables")?
                .push(t);
        }
        self.insert_step_in("teardown", 0, None, step, comment)?;
        self.reparse()
    }

    /// Add a `[[data]]` row.
    pub fn data_add(&mut self, row: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
        if row.is_empty() {
            bail!("a data row needs at least one column");
        }
        let position = self.max_position_in_doc().map_or(1, |p| p + 1);
        let mut t = Table::new();
        for (k, v) in row {
            // Rows are string maps: a number or a bool becomes its text.
            let text = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            t.insert(k, toml_edit::value(text));
        }
        t.decor_mut().set_prefix("\n");
        t.set_position(position);
        let root = self.doc.as_table_mut();
        if !root.contains_key("data") {
            root.insert("data", Item::ArrayOfTables(ArrayOfTables::new()));
        }
        root["data"]
            .as_array_of_tables_mut()
            .context("`data` is not [[data]] tables")?
            .push(t);
        self.reparse()
    }

    /// Add a standalone comment line at the insertion point.
    pub fn comment_add(&mut self, text: &str) -> Result<()> {
        let insertion = match self.insertion.clone() {
            Some(i) => i,
            None => {
                self.new_block("main")?;
                self.insertion
                    .clone()
                    .context("a new block sets the insertion point")?
            }
        };
        let text = one_line(text);
        let block = self
            .doc
            .get_mut("block")
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|b| b.get_mut(insertion.block))
            .context("the insertion point names a block the draft no longer has")?;
        if !block.contains_key("steps") {
            let mut steps = toml_edit::Array::new();
            steps.set_trailing("\n");
            steps.set_trailing_comma(true);
            block.insert("steps", Item::Value(Value::Array(steps)));
        }
        match block.get_mut("steps") {
            Some(Item::Value(Value::Array(steps))) => {
                let indent = element_indent(steps);
                match insertion.at.filter(|at| *at < steps.len()) {
                    Some(at) => {
                        let v = steps
                            .get_mut(at)
                            .context("no element at the insertion point")?;
                        let prefix = v
                            .decor()
                            .prefix()
                            .and_then(|p| p.as_str())
                            .unwrap_or("\n")
                            .to_string();
                        v.decor_mut()
                            .set_prefix(format!("\n{indent}# {text}{prefix}"));
                    }
                    None => {
                        let trailing = steps.trailing().as_str().unwrap_or("\n").to_string();
                        let kept = trailing.trim_end_matches([' ', '\t']);
                        let kept = if kept.ends_with('\n') {
                            kept.to_string()
                        } else {
                            format!("{kept}\n")
                        };
                        steps.set_trailing(format!("{kept}{indent}# {text}\n"));
                    }
                }
            }
            _ => {
                // Between `[[block.steps]]` tables a comment can only lead
                // the next table.
                let pending = self.pending_comment.take();
                self.pending_comment = Some(match pending {
                    Some(p) => format!("{p}\n# {text}"),
                    None => text,
                });
                return Ok(());
            }
        }
        self.reparse()
    }

    fn flow_table(&mut self) -> Result<&mut Table> {
        let root = self.doc.as_table_mut();
        if !root.contains_key("flow") {
            root.insert("flow", Item::Table(Table::new()));
        }
        root["flow"].as_table_mut().context("`flow` is not a table")
    }

    fn max_position_in(&self, key: &str) -> Option<usize> {
        self.doc.get(key).and_then(max_position)
    }

    fn max_position_in_doc(&self) -> Option<usize> {
        max_position(self.doc.as_item())
    }
}

/// The largest table position in `item` and the tables under it.
fn max_position(item: &Item) -> Option<usize> {
    let of_table = |t: &Table| {
        t.iter()
            .filter_map(|(_, v)| max_position(v))
            .chain(t.position())
            .max()
    };
    match item {
        Item::Table(t) => of_table(t),
        Item::ArrayOfTables(a) => a.iter().filter_map(of_table).max(),
        _ => None,
    }
}

fn parse_inline(line: &str) -> Result<InlineTable> {
    match line.parse::<Value>() {
        Ok(Value::InlineTable(t)) => Ok(t),
        _ => bail!("not a one-line step: {line}"),
    }
}

/// A JSON value as a TOML value: objects become inline tables.
fn json_to_value(v: &serde_json::Value) -> Result<Value> {
    Ok(match v {
        serde_json::Value::String(s) => Value::from(s.as_str()),
        serde_json::Value::Bool(b) => Value::from(*b),
        serde_json::Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Value::from(i),
            (None, Some(f)) => Value::from(f),
            _ => bail!("the number {n} does not fit TOML"),
        },
        serde_json::Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(json_to_value(item)?);
            }
            Value::Array(array)
        }
        serde_json::Value::Object(map) => {
            let mut t = InlineTable::new();
            for (k, item) in map {
                t.insert(k, json_to_value(item)?);
            }
            Value::InlineTable(t)
        }
        serde_json::Value::Null => bail!("TOML has no null"),
    })
}

/// The mixins under every `__mixins__/` directory in the project, each
/// with the `${var}`s it uses (golem's own `${_…}` builtins and `fake:`
/// generators left out).
pub fn mixins(project_root: &Path) -> Vec<Mixin> {
    fn walk(dir: &Path, out: &mut Vec<Mixin>, depth: usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !path.is_dir()
                || name.starts_with('.')
                || matches!(name.as_str(), "node_modules" | "target" | "build" | "dist")
            {
                continue;
            }
            if name == "__mixins__" {
                let Ok(files) = std::fs::read_dir(&path) else {
                    continue;
                };
                for f in files.flatten() {
                    let file = f.path();
                    if file.extension().and_then(|e| e.to_str()) != Some("toml") {
                        continue;
                    }
                    let text = std::fs::read_to_string(&file).unwrap_or_default();
                    let mut vars: Vec<String> = text
                        .split("${")
                        .skip(1)
                        .filter_map(|rest| rest.split('}').next())
                        .filter(|v| !v.starts_with('_') && !v.starts_with("fake:"))
                        .map(str::to_string)
                        .collect();
                    vars.sort();
                    vars.dedup();
                    out.push(Mixin {
                        name: file
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        path: file,
                        vars,
                    });
                }
            } else if depth < 6 {
                walk(&path, out, depth + 1);
            }
        }
    }
    let mut out = Vec::new();
    walk(project_root, &mut out, 0);
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    out
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

    fn obj(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        match v {
            serde_json::Value::Object(m) => m,
            _ => panic!("not an object"),
        }
    }

    #[test]
    fn flow_set_changes_only_the_flow_fields() {
        let (_dir, mut d) = draft_of(INLINE);
        d.flow_set(&FlowSet {
            name: Some("Login, renamed".into()),
            tags: Some(vec!["smoke".into(), "auth".into()]),
            vars: Some(obj(serde_json::json!({ "email": "a@b.test" }))),
            seed: Some(7),
            ..FlowSet::default()
        })
        .expect("flow_set");
        let want = INLINE.replace(
            "[flow]\nname = \"Login\"\n",
            "[flow]\nname = \"Login, renamed\"\ntags = [\"smoke\", \"auth\"]\nseed = 7\nvars = { email = \"a@b.test\" }\n",
        );
        assert_eq!(d.text(), want);
    }

    #[test]
    fn app_set_adds_an_app_after_the_others_and_replaces_by_name() {
        let (_dir, mut d) = draft_of(INLINE);
        d.app_set(&obj(serde_json::json!({
            "name": "web",
            "bundle": "com.acme.web",
            "devices": [{ "os": "ios:latest", "type": "phone" }],
            "permissions": { "camera": "allow" },
        })))
        .expect("app_set");
        let want = INLINE.replace(
            "os = \"android:latest\"\n\n# Open the form first.",
            "os = \"android:latest\"\n\n[[flow.apps]]\nname = \"web\"\nbundle = \"com.acme.web\"\n\
             devices = [{ os = \"ios:latest\", type = \"phone\" }]\npermissions = { camera = \"allow\" }\n\n# Open the form first.",
        );
        assert_eq!(d.text(), want);
        d.app_set(&obj(
            serde_json::json!({ "name": "web", "bundle": "com.acme.web2" }),
        ))
        .expect("replace");
        assert!(
            d.text().contains("bundle = \"com.acme.web2\""),
            "{}",
            d.text()
        );
        let flow = golem_parser::parse_flow(&d.text()).expect("parse");
        assert_eq!(flow.flow.apps.len(), 2);
        assert_eq!(flow.flow.apps[1].devices.len(), 1);
    }

    #[test]
    fn block_begin_and_block_link_add_a_block_with_its_links() {
        let (_dir, mut d) = draft_of(INLINE);
        d.block_begin("retry", Some("after")).expect("begin");
        d.record(r#"{ action = "tap", on_text = "Retry" }"#, None)
            .expect("record");
        d.block_link(
            "main",
            None,
            &[serde_json::json!({ "if_visible": "Error", "goto": "retry" })],
        )
        .expect("link");
        let flow = golem_parser::parse_flow(&d.text()).expect("parse");
        assert!(golem_parser::validation::validate_flow(&flow).is_empty());
        let retry = &flow.block[2];
        assert_eq!(retry.name.as_deref(), Some("retry"));
        assert_eq!(retry.next.as_deref(), Some("after"));
        assert_eq!(retry.steps[0].on_text.as_deref(), Some("Retry"));
        assert_eq!(flow.block[0].branch[0].goto, "retry");
        assert!(
            d.text().starts_with(
                &INLINE[..INLINE
                    .find("]\n\n[[block]]\nname = \"after\"")
                    .expect("anchor")]
            ),
            "the main block's steps SHALL be untouched: {}",
            d.text()
        );
    }

    #[test]
    fn teardown_add_creates_a_teardown_at_the_end() {
        let (_dir, mut d) = draft_of(INLINE);
        d.teardown_add(
            r#"{ action = "stop", app = "app" }"#,
            Some("Leave it closed"),
        )
        .expect("teardown");
        assert_eq!(
            d.text(),
            format!("{INLINE}\n[[teardown]]\nsteps = [\n  # Leave it closed\n  {{ action = \"stop\", app = \"app\" }},\n]\n")
        );
        d.teardown_add(r#"{ action = "screenshot" }"#, None)
            .expect("second");
        let flow = golem_parser::parse_flow(&d.text()).expect("parse");
        assert_eq!(flow.teardown[0].steps.len(), 2);
    }

    #[test]
    fn data_add_appends_a_row() {
        let (_dir, mut d) = draft_of(INLINE);
        d.data_add(&obj(serde_json::json!({ "email": "a@b.test", "age": 30 })))
            .expect("data");
        assert_eq!(
            d.text(),
            format!("{INLINE}\n[[data]]\nage = \"30\"\nemail = \"a@b.test\"\n")
        );
        let flow = golem_parser::parse_flow(&d.text()).expect("parse");
        assert_eq!(flow.data[0].get("age").map(String::as_str), Some("30"));
    }

    #[test]
    fn comment_add_writes_a_line_at_the_insertion_point() {
        let (_dir, mut d) = draft_of(INLINE);
        d.insert_at("main", None).expect("insert");
        d.comment_add("Now the form").expect("comment");
        d.record(r#"{ action = "tap", on_text = "Next" }"#, None)
            .expect("record");
        assert!(
            d.text().contains("# the header button\n  # Now the form\n  { action = \"tap\", on_text = \"Next\" },\n]"),
            "{}",
            d.text()
        );
        d.insert_at("main", Some(1)).expect("insert");
        d.comment_add("Before launch").expect("comment");
        assert!(
            d.text()
                .contains("steps = [\n  # Before launch\n  # Launch fresh.\n"),
            "{}",
            d.text()
        );
    }

    #[test]
    fn a_comment_in_a_tables_block_leads_the_next_table() {
        let (_dir, mut d) = draft_of(TABLES);
        d.insert_at("main", None).expect("insert");
        d.comment_add("Standalone").expect("comment");
        d.record(r#"{ action = "hide_keyboard" }"#, None)
            .expect("record");
        assert!(
            d.text()
                .contains("# Standalone\n[[block.steps]]\naction = \"hide_keyboard\""),
            "{}",
            d.text()
        );
    }

    #[test]
    fn mixins_lists_each_mixin_with_its_vars() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = dir.path().join("e2e/__mixins__");
        std::fs::create_dir_all(&m).expect("mkdir");
        std::fs::write(
            m.join("login.toml"),
            "[[step]]\naction = \"type\"\non_text = \"${field}\"\ninput = \"${value}\"\n[[step]]\naction = \"assert_visible\"\non_text = \"${_device} ${fake:name}\"\n",
        )
        .expect("write");
        let found = mixins(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "login");
        assert_eq!(found[0].vars, ["field", "value"]);
    }

    #[test]
    fn a_draft_built_with_every_tool_validates_and_parses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut d = Draft::new("All", Some(("app", "com.acme", "android:latest")));
        d.flow_set(&FlowSet {
            tags: Some(vec!["smoke".into()]),
            vars: Some(obj(serde_json::json!({ "who": "Ada" }))),
            start: Some("main".into()),
            ..FlowSet::default()
        })
        .expect("flow");
        d.app_set(&obj(serde_json::json!({ "name": "other", "bundle": "com.other", "devices": [{ "os": "ios:latest" }] })))
            .expect("app");
        d.block_begin("main", Some("done")).expect("main");
        d.comment_add("Open").expect("comment");
        d.record(r#"{ action = "launch", app = "app" }"#, None)
            .expect("launch");
        d.record_unverified(
            r#"{ action = "tap", on_text = "Maybe" }"#,
            Some("error path"),
        )
        .expect("unverified");
        d.block_begin("done", None).expect("done");
        d.record(
            r#"{ action = "assert_visible", on_text = "Hi ${who}" }"#,
            None,
        )
        .expect("assert");
        d.block_link(
            "main",
            None,
            &[serde_json::json!({ "if_not_visible": "Home", "goto": "done" })],
        )
        .expect("link");
        d.teardown_add(r#"{ action = "stop", app = "app" }"#, None)
            .expect("teardown");
        d.data_add(&obj(serde_json::json!({ "who": "Bob" })))
            .expect("data");
        let out = dir.path().join("all.test.toml");
        let done = d.export(&out, false).expect("export");
        assert_eq!(done.unverified.len(), 1);
        let flow =
            golem_parser::parse_flow(&std::fs::read_to_string(&out).expect("read")).expect("parse");
        assert_eq!(flow.flow.apps.len(), 2);
        assert_eq!(flow.block.len(), 2);
        assert_eq!(flow.teardown.len(), 1);
        assert_eq!(flow.data.len(), 1);
    }
}
