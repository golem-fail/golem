//! Edits to steps and blocks already in the draft. None of them runs the
//! device, so each sets the statuses a change sets (see `status`).

use anyhow::{bail, Context, Result};
use toml_edit::{InlineTable, Item, Table, Value};

use super::status::parse_address;
use super::{parse_inline, Draft, Insertion, StepStatus, StepsQuery};

/// Steps on each side of a change in the listing an edit returns.
const EDIT_CONTEXT: usize = 2;

impl Draft {
    /// The block and 0-based step an address names; the step must exist.
    fn locate(&self, at: &str) -> Result<(usize, usize)> {
        let (name, i) = parse_address(at)?;
        let b = self
            .block_index(&name)
            .with_context(|| format!("the draft has no block named {name:?}"))?;
        let n = self.steps_len(b);
        if i >= n {
            bail!("block {name:?} has {n} steps; there is no step {}", i + 1);
        }
        Ok((b, i))
    }

    /// The listing around step `i` of block `b`, or around the block's end
    /// when it has no step `i`.
    pub fn listing_near(&self, b: usize, i: usize) -> String {
        let n = self.steps_len(b);
        let q = if n == 0 {
            StepsQuery {
                block: Some(self.block_label(b)),
                ..StepsQuery::default()
            }
        } else {
            StepsQuery {
                around: Some(self.address(b, i.min(n - 1))),
                context: Some(EDIT_CONTEXT),
                ..StepsQuery::default()
            }
        };
        self.steps(&q).unwrap_or_default()
    }

    /// Replace a step, its comment, or both. The step does not run. A
    /// change to the comment alone, or a larger timeout alone, keeps the
    /// status; any other change makes the step unverified. Returns where
    /// the step is.
    pub fn step_edit(
        &mut self,
        at: &str,
        line: Option<&str>,
        comment: Option<&str>,
    ) -> Result<(usize, usize)> {
        let (b, i) = self.locate(at)?;
        if line.is_none() && comment.is_none() {
            bail!("step_edit needs a step, a comment, or both");
        }
        let mut changed = false;
        if let Some(line) = line {
            let old = self.step_line(b, i);
            changed = !self.same_but_longer_timeout(&old, line)?;
            self.replace_step(b, i, parse_inline(line)?)?;
        }
        if let Some(comment) = comment {
            let lines: Vec<String> = match comment.trim() {
                "" => Vec::new(),
                c => vec![super::one_line(c)],
            };
            self.set_comments(b, i, &lines)?;
        }
        if changed {
            self.set_status(b, i, StepStatus::Unverified)?;
            self.after_change(b, i, i + 1)?;
        } else {
            self.reparse()?;
        }
        Ok((b, i))
    }

    /// Whether `new` is `old` with only a larger effective timeout, or the
    /// same step.
    fn same_but_longer_timeout(&self, old: &str, new: &str) -> Result<bool> {
        let without_timeout = |line: &str| -> Result<Vec<(String, String)>> {
            let mut pairs: Vec<(String, String)> = parse_inline(line)?
                .iter()
                .filter(|(k, _)| *k != "timeout")
                .map(|(k, v)| (k.to_string(), v.to_string().trim().to_string()))
                .collect();
            pairs.sort();
            Ok(pairs)
        };
        if without_timeout(old)? != without_timeout(new)? {
            return Ok(false);
        }
        let parse = golem_parser::inline::parse_step_inline;
        let base = self.base_timeout_ms();
        let before = golem_runner::policy::effective_timeout(&parse(old)?.step, base);
        let after = golem_runner::policy::effective_timeout(&parse(new)?.step, base);
        Ok(after >= before)
    }

    /// `[flow.options] step_timeout`, else golem's default.
    fn base_timeout_ms(&self) -> u64 {
        self.doc
            .get("flow")
            .and_then(|f| f.get("options"))
            .and_then(|o| o.get("step_timeout"))
            .and_then(Item::as_integer)
            .and_then(|t| u64::try_from(t).ok())
            .unwrap_or(golem_runner::policy::DEFAULT_BASE_TIMEOUT_MS)
    }

    /// Put `step` in place of step `i`, keeping its comments and layout.
    fn replace_step(&mut self, b: usize, i: usize, mut step: InlineTable) -> Result<()> {
        let table = self.block_table_mut(b).context("no such block")?;
        match table.get_mut("steps") {
            Some(Item::Value(Value::Array(a))) => {
                let old = a.get_mut(i).context("no such step")?;
                step.decor_mut().clear();
                let decor = old.decor().clone();
                *old = Value::InlineTable(step);
                *old.decor_mut() = decor;
            }
            Some(Item::ArrayOfTables(a)) => {
                let old = a.get_mut(i).context("no such step")?;
                let keys: Vec<String> = old.iter().map(|(k, _)| k.to_string()).collect();
                for k in keys {
                    old.remove(&k);
                }
                for (k, v) in step.iter() {
                    let mut v = v.clone();
                    v.decor_mut().clear();
                    old.insert(k, Item::Value(v));
                }
            }
            _ => bail!("the block has no steps"),
        }
        Ok(())
    }

    /// Replace a step's comment lines with `lines`; the `# unverified`
    /// marker stays.
    fn set_comments(&mut self, b: usize, i: usize, lines: &[String]) -> Result<()> {
        let marked = self.has_marker(b, i);
        self.edit_step_prefix(b, i, |prefix, inline| {
            let parts: Vec<&str> = prefix.split('\n').collect();
            let indent = parts.last().copied().unwrap_or_default();
            let mut out = String::new();
            if inline {
                // The text before the first newline ends the step before.
                out.push_str(parts.first().copied().unwrap_or_default());
                out.push('\n');
            } else if prefix.starts_with('\n') {
                out.push('\n');
            }
            for l in lines {
                out.push_str(&format!("{indent}# {l}\n"));
            }
            if marked {
                out.push_str(&format!("{indent}# {}\n", super::status::MARKER));
            }
            out.push_str(indent);
            out
        })
    }

    /// Remove a step and its comments. The next active step becomes
    /// unverified. Returns where the steps after it start.
    pub fn step_delete(&mut self, at: &str) -> Result<(usize, usize)> {
        let (b, i) = self.locate(at)?;
        self.remove_step(b, i)?;
        self.reparse()?;
        self.after_change(b, i, i)?;
        Ok((b, i))
    }

    /// Take step `i` out of block `b`, with its status; returns it with
    /// its comments.
    fn remove_step(&mut self, b: usize, i: usize) -> Result<(InlineTable, Vec<String>)> {
        let step = self.step_value(b, i).context("no such step")?;
        let comments = self.own_comments(b, i);
        let table = self.block_table_mut(b).context("no such block")?;
        match table.get_mut("steps") {
            Some(Item::Value(Value::Array(a))) => {
                let removed = a.remove(i);
                // The removed step's first prefix line ends the step before
                // it; the next step's first line ended the removed one.
                let lead = removed
                    .decor()
                    .prefix()
                    .and_then(|p| p.as_str())
                    .and_then(|p| p.split_once('\n'))
                    .map_or("", |(first, _)| first)
                    .to_string();
                match a.get_mut(i) {
                    Some(next) => {
                        let p = next
                            .decor()
                            .prefix()
                            .and_then(|p| p.as_str())
                            .unwrap_or("\n")
                            .to_string();
                        let rest = p.split_once('\n').map_or(p.as_str(), |(_, r)| r);
                        next.decor_mut().set_prefix(format!("{lead}\n{rest}"));
                    }
                    None => {
                        let t = a.trailing().as_str().unwrap_or("\n").to_string();
                        let rest = t.split_once('\n').map_or("", |(_, r)| r);
                        a.set_trailing(format!("{lead}\n{rest}"));
                    }
                }
            }
            Some(Item::ArrayOfTables(a)) => a.remove(i),
            _ => bail!("the block has no steps"),
        }
        if let Some(s) = self.status.get_mut(b) {
            if i < s.len() {
                s.remove(i);
            }
        }
        if let Some(ins) = self.insertion.as_mut().filter(|ins| ins.block == b) {
            if let Some(at) = ins.at.as_mut().filter(|at| **at > i) {
                *at -= 1;
            }
        }
        Ok((step, comments))
    }

    /// The comments above a step, without the marker.
    fn own_comments(&self, b: usize, i: usize) -> Vec<String> {
        let Some((p, inline)) = self.step_prefix(b, i) else {
            return Vec::new();
        };
        super::status::own_lines(&p, inline)
            .filter_map(|line| match super::status::is_marker(line) {
                Some(Some(rest)) => Some(rest.to_string()),
                Some(None) => None,
                None => line.trim().strip_prefix('#').map(|t| t.trim().to_string()),
            })
            .collect()
    }

    /// Move a step so that it becomes step `to` (`block:step`; one past a
    /// block's last step appends). Where it left counts as a delete, and
    /// the step itself is unverified where it lands.
    pub fn step_move(&mut self, from: &str, to: &str) -> Result<(usize, usize)> {
        let (fb, fi) = self.locate(from)?;
        let (to_name, ti) = parse_address(to)?;
        let tb = self
            .block_index(&to_name)
            .with_context(|| format!("the draft has no block named {to_name:?}"))?;
        let room = self.steps_len(tb) - usize::from(tb == fb);
        if ti > room {
            bail!(
                "block {to_name:?} has room for step 1 to {} after the move; there is no step {}",
                room + 1,
                ti + 1
            );
        }
        let (step, comments) = self.remove_step(fb, fi)?;
        self.reparse()?;
        self.after_change(fb, fi, fi)?;
        let pending = self.pending_comment.take();
        let at = self.insert_step(
            &Insertion {
                block: tb,
                at: Some(ti),
            },
            step,
            None,
        );
        self.pending_comment = pending;
        let at = at?;
        self.reparse()?;
        if let Some(s) = self.status.get_mut(tb) {
            s.insert(at.min(s.len()), StepStatus::NotRun);
        }
        if let Some(ins) = self.insertion.as_mut().filter(|ins| ins.block == tb) {
            if let Some(cursor) = ins.at.as_mut().filter(|c| **c >= at) {
                *cursor += 1;
            }
        }
        self.set_comments(tb, at, &comments)?;
        self.set_status(tb, at, StepStatus::Unverified)?;
        self.after_change(tb, at, at + 1)?;
        Ok((tb, at))
    }

    /// Rename a block, and each `next`, `goto` and `[flow] start` that
    /// names it.
    pub fn block_rename(&mut self, name: &str, to: &str) -> Result<usize> {
        let b = self
            .block_index(name)
            .with_context(|| format!("the draft has no block named {name:?}"))?;
        if to.is_empty() {
            bail!("a block needs a name");
        }
        if self.block_index(to).is_some() {
            bail!("the draft already has a block named {to:?}");
        }
        if let Some(table) = self.block_table_mut(b) {
            table.insert("name", toml_edit::value(to));
        }
        for k in 0..self.block_count() {
            let Some(table) = self.block_table_mut(k) else {
                continue;
            };
            if table.get("next").and_then(Item::as_str) == Some(name) {
                table.insert("next", toml_edit::value(to));
            }
            match table.get_mut("branch") {
                Some(Item::Value(Value::Array(a))) => {
                    for v in a.iter_mut() {
                        if let Some(t) = v.as_inline_table_mut() {
                            rename_goto_inline(t, name, to);
                        }
                    }
                }
                Some(Item::ArrayOfTables(a)) => {
                    for t in a.iter_mut() {
                        rename_goto_table(t, name, to);
                    }
                }
                _ => {}
            }
        }
        if let Some(flow) = self.doc.get_mut("flow").and_then(Item::as_table_mut) {
            if flow.get("start").and_then(Item::as_str) == Some(name) {
                flow.insert("start", toml_edit::value(to));
            }
        }
        self.reparse()?;
        Ok(b)
    }

    /// Remove a block and its steps. Refuses while a `next`, a `goto` or
    /// `[flow] start` names it.
    pub fn block_delete(&mut self, name: &str) -> Result<()> {
        let b = self
            .block_index(name)
            .with_context(|| format!("the draft has no block named {name:?}"))?;
        let mut refs = Vec::new();
        for k in 0..self.block_count() {
            let Some(table) = self.block_table(k) else {
                continue;
            };
            let label = self.block_label(k);
            if table.get("next").and_then(Item::as_str) == Some(name) {
                refs.push(format!("block {label:?} next"));
            }
            let gotos = match table.get("branch") {
                Some(Item::Value(Value::Array(a))) => a
                    .iter()
                    .filter(|v| {
                        v.as_inline_table()
                            .and_then(|t| t.get("goto"))
                            .and_then(Value::as_str)
                            == Some(name)
                    })
                    .count(),
                Some(Item::ArrayOfTables(a)) => a
                    .iter()
                    .filter(|t| t.get("goto").and_then(Item::as_str) == Some(name))
                    .count(),
                _ => 0,
            };
            if gotos > 0 {
                refs.push(format!("block {label:?} goto"));
            }
        }
        if self
            .doc
            .get("flow")
            .and_then(|f| f.get("start"))
            .and_then(Item::as_str)
            == Some(name)
        {
            refs.push("[flow] start".to_string());
        }
        if !refs.is_empty() {
            bail!(
                "block {name:?} is still named by: {}; change those first",
                refs.join(", ")
            );
        }
        // The block before it falls through to the block after it now.
        let before = b
            .checked_sub(1)
            .filter(|p| self.successors(*p).contains(&b));
        self.doc
            .get_mut("block")
            .and_then(Item::as_array_of_tables_mut)
            .context("the draft has no blocks")?
            .remove(b);
        if b < self.status.len() {
            self.status.remove(b);
        }
        match self.insertion.clone() {
            Some(ins) if ins.block == b => self.insert_at_end(),
            Some(ins) if ins.block > b => {
                self.insertion = Some(Insertion {
                    block: ins.block - 1,
                    at: ins.at,
                })
            }
            _ => {}
        }
        self.reparse()?;
        if let Some(p) = before {
            let n = self.steps_len(p);
            self.after_change(p, n, n)?;
        }
        Ok(())
    }
}

fn rename_goto_inline(t: &mut InlineTable, name: &str, to: &str) {
    if t.get("goto").and_then(Value::as_str) == Some(name) {
        t.insert("goto", Value::from(to));
    }
}

fn rename_goto_table(t: &mut Table, name: &str, to: &str) {
    if t.get("goto").and_then(Item::as_str) == Some(name) {
        t.insert("goto", toml_edit::value(to));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOW: &str = r#"[flow]
name = "E"
start = "main"

[[block]]
name = "main"
branch = [{ if_visible = "Error", goto = "retry" }]
steps = [
  # Launch fresh.
  { action = "launch", app = "app" },
  { action = "tap", on_text = "Log in", timeout = 5000 },  # the header button
  { action = "type", on_text = "Email", input = "a@b.test" },
]

[[block]]
name = "retry"
next = "main"
steps = [
  { action = "tap", on_text = "Retry" },
]

[[block]]
name = "spare"
steps = [
  { action = "tap", on_text = "Spare" },
]
"#;

    fn draft_of(text: &str) -> (tempfile::TempDir, Draft) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f.test.toml");
        std::fs::write(&path, text).expect("write");
        let d = Draft::from_file(&path).expect("draft");
        (dir, d)
    }

    /// The draft with every step passed, as after a full run.
    fn passed(text: &str) -> (tempfile::TempDir, Draft) {
        let (dir, mut d) = draft_of(text);
        for (b, i) in d.addresses() {
            d.set_status(b, i, StepStatus::Passed).expect("pass");
        }
        (dir, d)
    }

    fn marks(d: &Draft) -> String {
        d.addresses()
            .into_iter()
            .map(|(b, i)| {
                format!(
                    "{}{}",
                    d.address(b, i),
                    d.status_of(b, i).map_or("-", StepStatus::mark)
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_comment_edit_keeps_the_status_and_replaces_the_comment() {
        let (_dir, mut d) = passed(FLOW);
        d.step_edit("main:1", None, Some("Start clean"))
            .expect("edit");
        assert_eq!(marks(&d), "main:1✓ main:2✓ main:3✓ retry:1✓ spare:1✓");
        assert!(
            d.text()
                .contains("steps = [\n  # Start clean\n  { action = \"launch\", app = \"app\" },"),
            "{}",
            d.text()
        );
        d.step_edit("main:1", None, Some("")).expect("remove");
        assert!(
            d.text()
                .contains("steps = [\n  { action = \"launch\", app = \"app\" },"),
            "{}",
            d.text()
        );
    }

    #[test]
    fn only_a_larger_timeout_keeps_the_status() {
        let (_dir, mut d) = passed(FLOW);
        d.step_edit(
            "main:2",
            Some(r#"{ action = "tap", on_text = "Log in", timeout = 9000 }"#),
            None,
        )
        .expect("larger");
        assert_eq!(marks(&d), "main:1✓ main:2✓ main:3✓ retry:1✓ spare:1✓");
        assert!(
            d.text().contains(
                "  { action = \"tap\", on_text = \"Log in\", timeout = 9000 },  # the header button\n"
            ),
            "the step SHALL keep its place and its line comment: {}",
            d.text()
        );
        d.step_edit(
            "main:2",
            Some(r#"{ action = "tap", on_text = "Log in", timeout = 1000 }"#),
            None,
        )
        .expect("smaller");
        assert_eq!(marks(&d), "main:1~ main:2? main:3? retry:1~ spare:1✓");
    }

    #[test]
    fn a_new_selector_makes_the_step_and_the_next_one_unverified() {
        let (_dir, mut d) = passed(FLOW);
        d.step_edit(
            "main:3",
            Some(r#"{ action = "type", on_label = "Email", input = "a@b.test" }"#),
            None,
        )
        .expect("edit");
        // After main comes retry, both as the branch and as the next block
        // in the file; nothing leads to spare.
        assert_eq!(marks(&d), "main:1~ main:2~ main:3? retry:1? spare:1✓");
        assert!(
            d.text()
                .contains("  # unverified\n  { action = \"type\", on_label = \"Email\""),
            "{}",
            d.text()
        );
    }

    #[test]
    fn a_delete_moves_the_line_comments_with_their_steps() {
        let (_dir, mut d) = passed(FLOW);
        d.step_delete("main:2").expect("delete");
        assert_eq!(marks(&d), "main:1~ main:2? retry:1~ spare:1✓");
        assert!(
            d.text().contains(
                "  { action = \"launch\", app = \"app\" },\n  # unverified\n  { action = \"type\""
            ),
            "the deleted step's line comment SHALL go with it: {}",
            d.text()
        );
        d.step_delete("main:2").expect("delete the last");
        assert!(
            d.text()
                .contains("  { action = \"launch\", app = \"app\" },\n]"),
            "{}",
            d.text()
        );
    }

    #[test]
    fn a_move_is_a_delete_then_an_unverified_insert() {
        let (_dir, mut d) = passed(FLOW);
        d.step_move("main:3", "main:1").expect("move");
        assert_eq!(
            d.step_line(0, 0),
            r#"{ action = "type", on_text = "Email", input = "a@b.test" }"#
        );
        assert!(marks(&d).starts_with("main:1? main:2?"), "{}", marks(&d));
        d.step_move("spare:1", "retry:2").expect("across blocks");
        assert_eq!(d.steps_len(1), 2);
        assert_eq!(d.steps_len(2), 0);
        assert_eq!(d.status_of(1, 1), Some(StepStatus::Unverified));
        let err = d.step_move("main:1", "retry:9").expect_err("no room");
        assert!(err.to_string().contains("step 1 to 3"), "{err}");
    }

    #[test]
    fn a_move_keeps_the_step_comments() {
        let (_dir, mut d) = passed(FLOW);
        d.step_move("main:1", "main:3").expect("move");
        assert!(
            d.text().contains(
                "  # Launch fresh.\n  # unverified\n  { action = \"launch\", app = \"app\" },\n]"
            ),
            "{}",
            d.text()
        );
    }

    #[test]
    fn a_rename_follows_every_reference() {
        let (_dir, mut d) = draft_of(FLOW);
        d.block_rename("main", "login").expect("rename");
        let flow = golem_parser::parse_flow(&d.text()).expect("parse");
        assert_eq!(flow.flow.start.as_deref(), Some("login"));
        assert_eq!(flow.block[0].name.as_deref(), Some("login"));
        assert_eq!(flow.block[1].next.as_deref(), Some("login"));
        let err = d.block_rename("retry", "login").expect_err("taken");
        assert!(err.to_string().contains("already has"), "{err}");
    }

    #[test]
    fn a_block_that_is_named_elsewhere_is_not_deleted() {
        let (_dir, mut d) = passed(FLOW);
        let err = d.block_delete("main").expect_err("named").to_string();
        assert!(
            err.contains("block \"retry\" next") && err.contains("[flow] start"),
            "{err}"
        );
        let err = d.block_delete("retry").expect_err("goto").to_string();
        assert!(err.contains("block \"main\" goto"), "{err}");
        d.block_delete("spare").expect("delete");
        assert_eq!(d.block_count(), 2);
        assert_eq!(marks(&d), "main:1✓ main:2✓ main:3✓ retry:1✓");
    }

    #[test]
    fn a_deleted_block_lets_the_block_before_fall_through_to_the_next() {
        let text = "[[block]]\nname = \"a\"\nsteps = [\n  { action = \"tap\", on_text = \"A\" },\n]\n\n\
                    [[block]]\nname = \"b\"\nsteps = [\n  { action = \"tap\", on_text = \"B\" },\n]\n\n\
                    [[block]]\nname = \"c\"\nsteps = [\n  { action = \"tap\", on_text = \"C\" },\n]\n";
        let (_dir, mut d) = passed(text);
        d.block_delete("b").expect("delete");
        assert_eq!(marks(&d), "a:1✓ c:1?");
    }

    #[test]
    fn edits_keep_the_tables_form() {
        let text = "[flow]\nname = \"T\"\n\n[[block]]\nname = \"main\"\n\n[[block.steps]]\naction = \"tap\"\non_text = \"A\"\n\n\
                    # Then B.\n[[block.steps]]\naction = \"tap\"\non_text = \"B\"\n\n\
                    [[block.steps]]\naction = \"tap\"\non_text = \"C\"\n";
        let (_dir, mut d) = passed(text);
        d.step_edit(
            "main:2",
            Some(r#"{ action = "tap", on_text = "B2" }"#),
            None,
        )
        .expect("edit");
        assert!(
            d.text().contains(
                "# Then B.\n# unverified\n[[block.steps]]\naction = \"tap\"\non_text = \"B2\"\n"
            ),
            "{}",
            d.text()
        );
        d.step_delete("main:1").expect("delete");
        let flow = golem_parser::parse_flow(&d.text()).expect("parse");
        let texts: Vec<_> = flow.block[0]
            .steps
            .iter()
            .filter_map(|s| s.on_text.clone())
            .collect();
        assert_eq!(texts, ["B2", "C"]);
    }
}
