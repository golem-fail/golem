//! What the session knows about each step of the draft, and the step
//! listing `draft_steps` shows.
//!
//! A step is `✓` when it passed in this session, `·` when it comes from the
//! file and has not run, `?` when it never ran in its current form, and `~`
//! when a step before it changed after it passed or was saved. Only `?` is
//! in the file, as a `# unverified` comment above the step, so a later
//! session reads it back.
//!
//! A change to a step (an insert, `record_only`, an edit, a delete) makes
//! the next active step `?`: its screen before it is no longer the one it
//! ran on. Each later step that can run after the change goes `~`. "Later"
//! follows the flow's own block order: `branch` targets and the next block
//! in the file, else `next`, else the next block.

use std::collections::VecDeque;

use anyhow::{bail, Context, Result};
use toml_edit::{InlineTable, Item, Table, Value};

use super::{expand_one_line, Draft, Insertion};

/// What the session knows about one step of the draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    /// Passed in this session.
    Passed,
    /// From the file; not run in this session.
    NotRun,
    /// Never run in its current form: `record_only`, edited, moved, or the
    /// first active step after a change.
    Unverified,
    /// A step before it changed after it passed or was saved.
    Stale,
}

impl StepStatus {
    pub fn mark(self) -> &'static str {
        match self {
            StepStatus::Passed => "✓",
            StepStatus::NotRun => "·",
            StepStatus::Unverified => "?",
            StepStatus::Stale => "~",
        }
    }
}

/// How many steps of the draft have each status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub passed: usize,
    pub not_run: usize,
    pub unverified: usize,
    pub stale: usize,
}

impl std::fmt::Display for StatusCounts {
    /// `5 ✓ passed, 1 ? unverified`: the counts that are not zero, each
    /// with its mark, so the listing needs no legend.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = [
            (self.passed, StepStatus::Passed, "passed"),
            (self.not_run, StepStatus::NotRun, "not run"),
            (self.unverified, StepStatus::Unverified, "unverified"),
            (self.stale, StepStatus::Stale, "stale"),
        ]
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, s, word)| format!("{n} {} {word}", s.mark()))
        .collect();
        f.write_str(&parts.join(", "))
    }
}

/// What `draft_steps` lists.
#[derive(Debug, Clone, Default)]
pub struct StepsQuery {
    /// `cursor` (the default), `block` or `block:step`.
    pub around: Option<String>,
    /// Steps on each side of `around`.
    pub context: Option<usize>,
    /// List this block's steps instead.
    pub block: Option<String>,
    /// At most this many steps.
    pub limit: Option<usize>,
}

pub(super) const MARKER: &str = "unverified";
const DEFAULT_CONTEXT: usize = 5;

/// Parse a step address: `block` (its first step) or `block:step`, steps
/// counting from 1. Returns the block name and the 0-based step.
pub(super) fn parse_address(s: &str) -> Result<(String, usize)> {
    let (block, step) = match s.rsplit_once(':') {
        Some((b, n)) => (
            b,
            n.parse::<usize>()
                .ok()
                .filter(|n| *n >= 1)
                .with_context(|| format!("{s:?}: the step after ':' counts from 1"))?,
        ),
        None => (s, 1),
    };
    if block.is_empty() {
        bail!("{s:?} names no block");
    }
    Ok((block.to_string(), step - 1))
}

impl Draft {
    pub(super) fn block_count(&self) -> usize {
        self.blocks().map_or(0, |b| b.len())
    }

    pub(super) fn block_table(&self, b: usize) -> Option<&Table> {
        self.blocks()?.get(b)
    }

    pub(super) fn block_table_mut(&mut self, b: usize) -> Option<&mut Table> {
        self.doc
            .get_mut("block")
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|a| a.get_mut(b))
    }

    /// A block's name; `block_N` for a block without one, as a run labels
    /// it.
    pub(super) fn block_label(&self, b: usize) -> String {
        self.block_table(b)
            .and_then(|t| t.get("name"))
            .and_then(Item::as_str)
            .map_or_else(|| format!("block_{b}"), str::to_string)
    }

    /// The block a name or a `block_N` label names.
    pub(super) fn block_by_label(&self, label: &str) -> Option<usize> {
        self.block_index(label).or_else(|| {
            let n: usize = label.strip_prefix("block_")?.parse().ok()?;
            let unnamed = self.block_table(n)?.get("name").is_none();
            unnamed.then_some(n)
        })
    }

    pub(super) fn steps_len(&self, b: usize) -> usize {
        match self.block_table(b).and_then(|t| t.get("steps")) {
            Some(Item::Value(Value::Array(a))) => a.len(),
            Some(Item::ArrayOfTables(a)) => a.len(),
            _ => 0,
        }
    }

    /// Every step, as (block, step), in file order.
    pub(super) fn addresses(&self) -> Vec<(usize, usize)> {
        (0..self.block_count())
            .flat_map(|b| (0..self.steps_len(b)).map(move |i| (b, i)))
            .collect()
    }

    /// `block:step`, 1-based.
    pub(super) fn address(&self, b: usize, i: usize) -> String {
        format!("{}:{}", self.block_label(b), i + 1)
    }

    pub fn status_of(&self, b: usize, i: usize) -> Option<StepStatus> {
        self.status.get(b).and_then(|s| s.get(i)).copied()
    }

    pub fn counts(&self) -> StatusCounts {
        let mut c = StatusCounts::default();
        for s in self.status.iter().flatten() {
            match s {
                StepStatus::Passed => c.passed += 1,
                StepStatus::NotRun => c.not_run += 1,
                StepStatus::Unverified => c.unverified += 1,
                StepStatus::Stale => c.stale += 1,
            }
        }
        c
    }

    /// The step's leading text: its comment lines and indent. In an inline
    /// array the text before the first newline ends the step before it.
    pub(super) fn step_prefix(&self, b: usize, i: usize) -> Option<(String, bool)> {
        match self.block_table(b)?.get("steps")? {
            Item::Value(Value::Array(a)) => {
                let p = a.get(i)?.decor().prefix().and_then(|p| p.as_str());
                Some((p.unwrap_or_default().to_string(), true))
            }
            Item::ArrayOfTables(a) => {
                let p = a.get(i)?.decor().prefix().and_then(|p| p.as_str());
                Some((p.unwrap_or_default().to_string(), false))
            }
            _ => None,
        }
    }

    /// Change the step's leading text with `f(prefix, inline)`.
    pub(super) fn edit_step_prefix(
        &mut self,
        b: usize,
        i: usize,
        f: impl FnOnce(&str, bool) -> String,
    ) -> Result<()> {
        let table = self.block_table_mut(b).context("no such block")?;
        match table.get_mut("steps") {
            Some(Item::Value(Value::Array(a))) => {
                expand_one_line(a);
                let v = a.get_mut(i).context("no such step")?;
                let old = v.decor().prefix().and_then(|p| p.as_str()).unwrap_or("");
                let new = f(old, true);
                v.decor_mut().set_prefix(new);
            }
            Some(Item::ArrayOfTables(a)) => {
                let t = a.get_mut(i).context("no such step")?;
                let old = t.decor().prefix().and_then(|p| p.as_str()).unwrap_or("");
                let new = f(old, false);
                t.decor_mut().set_prefix(new);
            }
            _ => bail!("the block has no steps"),
        }
        Ok(())
    }

    pub(super) fn has_marker(&self, b: usize, i: usize) -> bool {
        self.step_prefix(b, i)
            .is_some_and(|(p, inline)| own_lines(&p, inline).any(|l| is_marker(l).is_some()))
    }

    pub(super) fn step_value(&self, b: usize, i: usize) -> Option<InlineTable> {
        match self.block_table(b)?.get("steps")? {
            Item::Value(Value::Array(a)) => a.get(i)?.as_inline_table().cloned(),
            Item::ArrayOfTables(a) => {
                let mut t = InlineTable::new();
                for (k, v) in a.get(i)?.iter() {
                    if let Some(v) = v.as_value() {
                        t.insert(k, v.clone());
                    }
                }
                Some(t)
            }
            _ => None,
        }
    }

    pub(super) fn step_action(&self, b: usize, i: usize) -> Option<String> {
        self.step_value(b, i)?
            .get("action")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    /// The step in the canonical one-line form.
    pub(super) fn step_line(&self, b: usize, i: usize) -> String {
        let Some(mut t) = self.step_value(b, i) else {
            return String::new();
        };
        t.decor_mut().clear();
        for (_, v) in t.iter_mut() {
            v.decor_mut().clear();
        }
        t.fmt();
        Value::InlineTable(t).to_string().trim().to_string()
    }

    /// The step's comments, without the `# unverified` marker: the lines
    /// above it, then the comment that ends its line.
    fn step_comments(&self, b: usize, i: usize) -> Vec<String> {
        let mut out = Vec::new();
        if let Some((p, inline)) = self.step_prefix(b, i) {
            for line in own_lines(&p, inline) {
                let Some(text) = line.trim().strip_prefix('#') else {
                    continue;
                };
                match is_marker(line) {
                    Some(Some(rest)) => out.push(rest.to_string()),
                    Some(None) => {}
                    None => out.push(text.trim().to_string()),
                }
            }
        }
        let ends_line = match self.block_table(b).and_then(|t| t.get("steps")) {
            Some(Item::Value(Value::Array(a))) => match a.get(i + 1) {
                Some(next) => next
                    .decor()
                    .prefix()
                    .and_then(|p| p.as_str())
                    .map(str::to_string),
                None => a.trailing().as_str().map(str::to_string),
            },
            _ => None,
        };
        if let Some(text) = ends_line
            .as_deref()
            .and_then(|t| t.split('\n').next())
            .and_then(|l| l.trim().strip_prefix('#'))
        {
            out.push(text.trim().to_string());
        }
        out
    }

    /// Set a step's status, and write or remove its `# unverified` marker
    /// to match.
    pub(super) fn set_status(&mut self, b: usize, i: usize, status: StepStatus) -> Result<()> {
        let marked = self.has_marker(b, i);
        if status == StepStatus::Unverified && !marked {
            self.edit_step_prefix(b, i, add_marker)?;
        } else if status != StepStatus::Unverified && marked {
            self.edit_step_prefix(b, i, remove_marker)?;
        }
        if let Some(s) = self.status.get_mut(b).and_then(|s| s.get_mut(i)) {
            *s = status;
        }
        Ok(())
    }

    /// The blocks that can run right after block `b`, as the runner picks
    /// them: each `branch` target and the next block, else `next`, else
    /// the next block.
    pub(super) fn successors(&self, b: usize) -> Vec<usize> {
        let Some(table) = self.block_table(b) else {
            return Vec::new();
        };
        let gotos: Vec<String> = match table.get("branch") {
            Some(Item::Value(Value::Array(a))) => a
                .iter()
                .filter_map(|v| v.as_inline_table()?.get("goto")?.as_str())
                .map(str::to_string)
                .collect(),
            Some(Item::ArrayOfTables(a)) => a
                .iter()
                .filter_map(|t| t.get("goto")?.as_str())
                .map(str::to_string)
                .collect(),
            _ => Vec::new(),
        };
        let following = (b + 1 < self.block_count()).then_some(b + 1);
        let mut out: Vec<usize> = gotos.iter().filter_map(|g| self.block_index(g)).collect();
        if !gotos.is_empty() {
            out.extend(following);
        } else if let Some(next) = table.get("next").and_then(Item::as_str) {
            out.extend(self.block_index(next));
        } else {
            out.extend(following);
        }
        out
    }

    /// A step that changes the screen: anything but `screenshot`.
    fn is_active(&self, b: usize, i: usize) -> bool {
        self.step_action(b, i).as_deref() != Some("screenshot")
    }

    /// Mark what a change in block `block` affects. The steps from `from`
    /// on in the block, and every step in a block that can run after it,
    /// are later steps: the first active one on each way out goes `?`, the
    /// rest `~`. Steps before `before` count only when a loop leads back
    /// into the block.
    pub(super) fn after_change(&mut self, block: usize, before: usize, from: usize) -> Result<()> {
        let count = self.block_count();
        let len = self.steps_len(block);
        let mut later: Vec<(usize, usize)> = (from..len).map(|i| (block, i)).collect();
        let mut next: Vec<(usize, usize)> = Vec::new();
        let first_here = (from..len).find(|i| self.is_active(block, *i));
        if let Some(i) = first_here {
            next.push((block, i));
        }
        let mut seen = vec![false; count];
        if let Some(s) = seen.get_mut(block) {
            *s = true;
        }
        let mut looped = false;
        // Each queued block carries whether a way out of the changed block
        // reaches it before any active step.
        let mut queue: VecDeque<(usize, bool)> = self
            .successors(block)
            .into_iter()
            .map(|b| (b, first_here.is_none()))
            .collect();
        while let Some((b, open)) = queue.pop_front() {
            if b == block {
                if !looped {
                    looped = true;
                    let upto = before.min(len);
                    later.extend((0..upto).map(|i| (block, i)));
                    if open {
                        next.extend(
                            (0..upto)
                                .find(|i| self.is_active(block, *i))
                                .map(|i| (block, i)),
                        );
                    }
                }
                continue;
            }
            if seen[b] {
                continue;
            }
            seen[b] = true;
            let n = self.steps_len(b);
            later.extend((0..n).map(|i| (b, i)));
            let first = (0..n).find(|i| self.is_active(b, *i));
            if open {
                next.extend(first.map(|i| (b, i)));
            }
            let still_open = open && first.is_none();
            queue.extend(self.successors(b).into_iter().map(|s| (s, still_open)));
        }
        for (b, i) in later {
            match self.status_of(b, i) {
                _ if next.contains(&(b, i)) => self.set_status(b, i, StepStatus::Unverified)?,
                Some(StepStatus::Passed | StepStatus::NotRun) => {
                    self.set_status(b, i, StepStatus::Stale)?
                }
                _ => {}
            }
        }
        self.reparse()
    }

    /// The file's step for step `index` of block `b` as a run numbers it,
    /// after each `load_mixin` expanded: the run's block has `ran_len`
    /// steps. A step inside a mixin maps to its `load_mixin` step, and the
    /// flag says whether it is the mixin's last. `None` when the mapping is
    /// unclear: more than one `load_mixin` whose sizes the counts cannot
    /// tell apart.
    pub(super) fn file_step(
        &self,
        b: usize,
        index: usize,
        ran_len: usize,
    ) -> Option<(usize, bool)> {
        let n = self.steps_len(b);
        let mixins: Vec<usize> = (0..n)
            .filter(|i| self.step_action(b, *i).as_deref() == Some("load_mixin"))
            .collect();
        match mixins.as_slice() {
            [p] if ran_len != n => {
                let span = (ran_len + 1).checked_sub(n)?;
                if index < *p {
                    Some((index, true))
                } else if index < p + span {
                    Some((*p, index + 1 == p + span))
                } else {
                    let i = index + 1 - span;
                    (i < n).then_some((i, true))
                }
            }
            _ if ran_len == n => (index < n).then_some((index, true)),
            _ => None,
        }
    }

    /// Mark the steps that passed in the flow run the session opened from:
    /// `passed` holds each as the run labels it (block, 0-based step), and
    /// `ran` is the flow as it ran.
    pub fn mark_ran(
        &mut self,
        ran: &golem_parser::FlowFile,
        passed: &[(String, usize)],
    ) -> Result<()> {
        for (label, index) in passed {
            let Some(b) = self.block_by_label(label) else {
                continue;
            };
            let Some(ran_len) = ran_block_len(ran, label) else {
                continue;
            };
            if let Some((i, true)) = self.file_step(b, *index, ran_len) {
                self.set_status(b, i, StepStatus::Passed)?;
            }
        }
        self.reparse()
    }

    /// Put the cursor before the run's 1-based `step` of block `label`.
    /// Returns false when the draft has no such place, for example a block
    /// that a mixin added.
    pub fn place_cursor(&mut self, ran: &golem_parser::FlowFile, label: &str, step: usize) -> bool {
        let Some(b) = self.block_by_label(label) else {
            return false;
        };
        let index = step.saturating_sub(1);
        let ran_len = ran_block_len(ran, label).unwrap_or(self.steps_len(b));
        let at = if index >= ran_len {
            Some(self.steps_len(b))
        } else {
            self.file_step(b, index, ran_len).map(|(i, _)| i)
        };
        match at {
            Some(at) => {
                self.insertion = Some(Insertion {
                    block: b,
                    at: Some(at),
                });
                true
            }
            None => false,
        }
    }

    /// The step listing: a header with the counts and the cursor, then
    /// each block's header and the steps that `q` selects.
    pub fn steps(&self, q: &StepsQuery) -> Result<String> {
        let all = self.addresses();
        let cursor = self.insertion.as_ref().map(|ins| {
            let len = self.steps_len(ins.block);
            (ins.block, ins.at.unwrap_or(len).min(len))
        });
        // The cursor as a position in `all`: the count of steps before it.
        let cursor_pos = cursor.map(|(cb, ci)| {
            all.iter()
                .position(|&(b, i)| b > cb || (b == cb && i >= ci))
                .unwrap_or(all.len())
        });
        let context = q.context.unwrap_or(DEFAULT_CONTEXT);
        // The selected steps, as a range of `all`, and whether the cursor
        // shows.
        let (range, show_cursor) = match (&q.block, q.around.as_deref()) {
            (Some(name), _) => {
                let b = self
                    .block_index(name)
                    .with_context(|| format!("the draft has no block named {name:?}"))?;
                let start = all.iter().position(|&(x, _)| x == b).unwrap_or(0);
                let n = self.steps_len(b);
                (start..start + n, cursor.is_some_and(|(cb, _)| cb == b))
            }
            (None, Some(around)) if around != "cursor" => {
                let (name, i) = parse_address(around)?;
                let b = self
                    .block_index(&name)
                    .with_context(|| format!("the draft has no block named {name:?}"))?;
                let n = self.steps_len(b);
                if i >= n {
                    bail!("block {name:?} has {n} steps; there is no step {}", i + 1);
                }
                let pos = all
                    .iter()
                    .position(|&a| a == (b, i))
                    .context("the step went away")?;
                let range = pos.saturating_sub(context)..(pos + context + 1).min(all.len());
                let shows = cursor_pos.is_some_and(|c| range.start <= c && c <= range.end);
                (range, shows)
            }
            _ => match cursor_pos {
                Some(c) => (
                    c.saturating_sub(context)..(c + context).min(all.len()),
                    true,
                ),
                None => (all.len().saturating_sub(context)..all.len(), false),
            },
        };
        let range = match q.limit {
            Some(limit) => range.start..range.end.min(range.start + limit),
            None => range,
        };
        let shown = &all[range];

        let mut out = format!("draft · {} steps", all.len());
        let counts = self.counts().to_string();
        if !counts.is_empty() {
            out.push_str(&format!(": {counts}"));
        }
        out.push_str(&format!(" · cursor {}\n", self.cursor_text()));
        for b in 0..self.block_count() {
            out.push_str(&self.block_header(b));
            let n = self.steps_len(b);
            for i in 0..=n {
                if show_cursor && cursor == Some((b, i)) {
                    out.push_str("  ▸ cursor\n");
                }
                if i < n && shown.contains(&(b, i)) {
                    let mark = self.status_of(b, i).map_or(" ", StepStatus::mark);
                    out.push_str(&format!(
                        "  {} {mark} {}",
                        self.address(b, i),
                        self.step_line(b, i)
                    ));
                    let comments = self.step_comments(b, i);
                    if !comments.is_empty() {
                        out.push_str(&format!("  # {}", comments.join(" / ")));
                    }
                    out.push('\n');
                }
            }
        }
        Ok(out)
    }

    fn cursor_text(&self) -> String {
        match &self.insertion {
            None => "in a new block \"main\"".to_string(),
            Some(ins) => {
                let len = self.steps_len(ins.block);
                match ins.at.filter(|at| *at < len) {
                    Some(at) => format!("before {}", self.address(ins.block, at)),
                    None => format!("at the end of {}", self.block_label(ins.block)),
                }
            }
        }
    }

    /// `[main] 3 steps · next after · goto retry if { if_visible = "Error" }`
    fn block_header(&self, b: usize) -> String {
        let n = self.steps_len(b);
        let mut out = format!(
            "[{}] {n} step{}",
            self.block_label(b),
            if n == 1 { "" } else { "s" }
        );
        let Some(table) = self.block_table(b) else {
            return out + "\n";
        };
        let mut branches: Vec<InlineTable> = match table.get("branch") {
            Some(Item::Value(Value::Array(a))) => a
                .iter()
                .filter_map(|v| v.as_inline_table().cloned())
                .collect(),
            Some(Item::ArrayOfTables(a)) => a
                .iter()
                .map(|t| {
                    let mut it = InlineTable::new();
                    for (k, v) in t.iter() {
                        if let Some(v) = v.as_value() {
                            it.insert(k, v.clone());
                        }
                    }
                    it
                })
                .collect(),
            _ => Vec::new(),
        };
        for t in &mut branches {
            let goto = t
                .remove("goto")
                .and_then(|g| g.as_str().map(str::to_string))
                .unwrap_or_default();
            t.decor_mut().clear();
            for (_, v) in t.iter_mut() {
                v.decor_mut().clear();
            }
            t.fmt();
            let cond = Value::InlineTable(t.clone()).to_string();
            let cond = cond.trim();
            if cond == "{}" {
                out.push_str(&format!(" · goto {goto}"));
            } else {
                out.push_str(&format!(" · goto {goto} if {cond}"));
            }
        }
        if let Some(next) = table.get("next").and_then(Item::as_str) {
            out.push_str(&format!(" · next {next}"));
        }
        if let Some(each) = table.get("for_each").and_then(Item::as_str) {
            out.push_str(&format!(" · for_each {each}"));
        }
        out + "\n"
    }
}

fn ran_block_len(ran: &golem_parser::FlowFile, label: &str) -> Option<usize> {
    let by_name = ran.block.iter().find(|b| b.name.as_deref() == Some(label));
    let by_index = || {
        let n: usize = label.strip_prefix("block_")?.parse().ok()?;
        ran.block.get(n).filter(|b| b.name.is_none())
    };
    by_name.or_else(by_index).map(|b| b.steps.len())
}

/// The lines of a step's leading text that belong to it: in an inline
/// array the first line ends the step before, and the last is the indent.
pub(super) fn own_lines(prefix: &str, inline: bool) -> impl Iterator<Item = &str> {
    let lines: Vec<&str> = prefix.split('\n').collect();
    let end = lines.len().saturating_sub(1);
    let start = usize::from(inline).min(end);
    lines.into_iter().take(end).skip(start)
}

/// `Some(None)` for a `# unverified` line, `Some(Some(rest))` for
/// `# unverified: rest`, `None` for any other line.
pub(super) fn is_marker(line: &str) -> Option<Option<&str>> {
    let text = line.trim().strip_prefix('#')?.trim();
    if text == MARKER {
        return Some(None);
    }
    text.strip_prefix(MARKER)?
        .strip_prefix(':')
        .map(|rest| Some(rest.trim()))
}

/// The prefix with a `# unverified` line just above the step.
fn add_marker(prefix: &str, inline: bool) -> String {
    if inline {
        match prefix.rsplit_once('\n') {
            Some((head, indent)) => format!("{head}\n{indent}# {MARKER}\n{indent}"),
            None => format!("\n  # {MARKER}\n  "),
        }
    } else if prefix.is_empty() || prefix.ends_with('\n') {
        format!("{prefix}# {MARKER}\n")
    } else {
        format!("{prefix}\n# {MARKER}\n")
    }
}

/// The prefix without its `# unverified` line; `# unverified: note`
/// keeps its note as `# note`.
fn remove_marker(prefix: &str, inline: bool) -> String {
    let mut lines: Vec<String> = prefix.split('\n').map(str::to_string).collect();
    let end = lines.len().saturating_sub(1);
    let start = usize::from(inline).min(end);
    let mut k = start;
    let mut end = end;
    while k < end {
        match is_marker(&lines[k]) {
            Some(None) => {
                lines.remove(k);
                end -= 1;
            }
            Some(Some(rest)) => {
                let indent: String = lines[k].chars().take_while(|c| c.is_whitespace()).collect();
                lines[k] = format!("{indent}# {rest}");
                k += 1;
            }
            None => k += 1,
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOW: &str = r#"[flow]
name = "S"

[[block]]
name = "main"
branch = [{ if_visible = "Error", goto = "retry" }]
steps = [
  # Launch fresh.
  { action = "launch", app = "app" },
  { action = "screenshot" },
  { action = "tap", on_text = "Log in" },  # the header button
]

[[block]]
name = "done"
steps = [
  { action = "assert_visible", on_text = "Hi" },
]

[[block]]
name = "retry"
next = "main"
steps = [
  # unverified
  { action = "tap", on_text = "Retry" },
]

[[block]]
name = "lead"
next = "main"
steps = [{ action = "tap", on_text = "Lead" }]
"#;

    fn draft_of(text: &str) -> (tempfile::TempDir, Draft) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f.test.toml");
        std::fs::write(&path, text).expect("write");
        let d = Draft::from_file(&path).expect("draft");
        (dir, d)
    }

    /// Each step's address and mark, in file order.
    fn marks(d: &Draft) -> Vec<String> {
        d.addresses()
            .into_iter()
            .map(|(b, i)| {
                format!(
                    "{} {}",
                    d.address(b, i),
                    d.status_of(b, i).map_or("-", StepStatus::mark)
                )
            })
            .collect()
    }

    #[test]
    fn a_file_step_is_not_run_unless_it_carries_the_marker() {
        let (_dir, d) = draft_of(FLOW);
        assert_eq!(
            marks(&d),
            [
                "main:1 ·",
                "main:2 ·",
                "main:3 ·",
                "done:1 ·",
                "retry:1 ?",
                "lead:1 ·"
            ]
        );
    }

    #[test]
    fn an_insert_makes_the_next_active_step_unverified_and_later_reachable_steps_stale() {
        let (_dir, mut d) = draft_of(FLOW);
        d.insert_at("main", Some(2)).expect("cursor");
        d.record(r#"{ action = "tap", on_text = "X" }"#, None)
            .expect("record");
        assert_eq!(
            marks(&d),
            [
                // A loop (retry → main) leads back to the steps before it.
                "main:1 ~",
                "main:2 ✓",
                // A screenshot does not change the screen: the tap after it
                // is the next active step.
                "main:3 ~",
                "main:4 ?",
                "done:1 ~",
                "retry:1 ?",
                // Nothing that runs after main reaches lead.
                "lead:1 ·",
            ]
        );
        assert!(
            d.text().contains(
                "  { action = \"screenshot\" },\n  # unverified\n  { action = \"tap\", on_text = \"Log in\" },  # the header button\n"
            ),
            "the next active step SHALL carry the marker in the file: {}",
            d.text()
        );
    }

    #[test]
    fn an_insert_at_the_end_of_a_block_marks_the_first_step_on_each_way_out() {
        let (_dir, mut d) = draft_of(FLOW);
        d.insert_at("main", None).expect("cursor");
        d.record(r#"{ action = "tap", on_text = "X" }"#, None)
            .expect("record");
        assert_eq!(
            marks(&d),
            [
                "main:1 ~",
                "main:2 ~",
                "main:3 ~",
                "main:4 ✓",
                "done:1 ?",
                "retry:1 ?",
                "lead:1 ·",
            ]
        );
    }

    #[test]
    fn next_goes_only_to_its_block() {
        let text = "[[block]]\nname = \"a\"\nnext = \"c\"\nsteps = [\n  { action = \"tap\", on_text = \"A\" },\n]\n\n\
                    [[block]]\nname = \"b\"\nsteps = [\n  { action = \"tap\", on_text = \"B\" },\n]\n\n\
                    [[block]]\nname = \"c\"\nsteps = [\n  { action = \"tap\", on_text = \"C1\" },\n  { action = \"tap\", on_text = \"C2\" },\n]\n";
        let (_dir, mut d) = draft_of(text);
        d.insert_at("a", None).expect("cursor");
        d.record(r#"{ action = "tap", on_text = "X" }"#, None)
            .expect("record");
        assert_eq!(marks(&d), ["a:1 ·", "a:2 ✓", "b:1 ·", "c:1 ?", "c:2 ~"]);
    }

    #[test]
    fn record_only_is_unverified_and_so_is_the_step_after_it() {
        let (_dir, mut d) = draft_of(FLOW);
        d.insert_at("done", Some(1)).expect("cursor");
        d.record_unverified(r#"{ action = "tap", on_text = "Y" }"#, None)
            .expect("record");
        assert_eq!(&marks(&d)[3..5], ["done:1 ?", "done:2 ?"]);
        assert_eq!(d.counts().unverified, 3);
    }

    #[test]
    fn a_step_that_passes_loses_its_marker_and_keeps_its_note() {
        let (_dir, mut d) = draft_of(FLOW);
        d.set_status(2, 0, StepStatus::Passed).expect("pass");
        assert!(
            d.text()
                .contains("steps = [\n  { action = \"tap\", on_text = \"Retry\" },\n]"),
            "{}",
            d.text()
        );
        let mut d = Draft::new("N", None);
        d.record_unverified(r#"{ action = "tap", on_text = "Z" }"#, Some("error path"))
            .expect("record");
        d.set_status(0, 0, StepStatus::Passed).expect("pass");
        assert!(
            d.text()
                .contains("  # error path\n  { action = \"tap\", on_text = \"Z\" },"),
            "{}",
            d.text()
        );
    }

    #[test]
    fn a_marker_in_a_tables_block_leads_the_table() {
        let text =
            "[[block]]\nname = \"main\"\n\n[[block.steps]]\naction = \"tap\"\non_text = \"A\"\n\n\
                    # Then B.\n[[block.steps]]\naction = \"tap\"\non_text = \"B\"\n";
        let (_dir, mut d) = draft_of(text);
        d.insert_at("main", Some(2)).expect("cursor");
        d.record(r#"{ action = "tap", on_text = "X" }"#, None)
            .expect("record");
        assert_eq!(marks(&d), ["main:1 ·", "main:2 ✓", "main:3 ?"]);
        assert!(
            d.text().contains(
                "# Then B.\n# unverified\n[[block.steps]]\naction = \"tap\"\non_text = \"B\""
            ),
            "{}",
            d.text()
        );
        let (_dir, d) = draft_of(&d.text());
        assert_eq!(marks(&d)[2], "main:3 ?", "the marker SHALL read back");
    }

    #[test]
    fn a_flow_run_marks_its_passed_steps_through_a_mixin() {
        let text = "[[block]]\nname = \"main\"\nsteps = [\n  { action = \"launch\", app = \"app\" },\n  \
                    { action = \"load_mixin\", mixin = \"login\" },\n  { action = \"tap\", on_text = \"Go\" },\n  \
                    { action = \"tap\", on_text = \"Next\" },\n]\n";
        let (_dir, mut d) = draft_of(text);
        // The mixin ran as three steps: the run's block has six.
        let ran = golem_parser::parse_flow(
            "[flow]\nname = \"R\"\n[[block]]\nname = \"main\"\nsteps = [\n  { action = \"launch\", app = \"app\" },\n  \
             { action = \"tap\", on_text = \"m1\" },\n  { action = \"tap\", on_text = \"m2\" },\n  \
             { action = \"tap\", on_text = \"m3\" },\n  { action = \"tap\", on_text = \"Go\" },\n  \
             { action = \"tap\", on_text = \"Next\" },\n]\n",
        )
        .expect("ran");
        let passed: Vec<(String, usize)> = (0..5).map(|i| ("main".to_string(), i)).collect();
        d.mark_ran(&ran, &passed).expect("mark");
        assert_eq!(marks(&d), ["main:1 ✓", "main:2 ✓", "main:3 ✓", "main:4 ·"]);
        assert!(d.place_cursor(&ran, "main", 6));
        assert_eq!(d.cursor_text(), "before main:4");
        assert!(d.place_cursor(&ran, "main", 3));
        assert_eq!(
            d.cursor_text(),
            "before main:2",
            "a stop inside a mixin SHALL put the cursor before the load_mixin"
        );
    }

    #[test]
    fn the_listing_shows_every_block_header_and_the_steps_around_the_cursor() {
        let (_dir, mut d) = draft_of(FLOW);
        d.insert_at("main", Some(3)).expect("cursor");
        let out = d
            .steps(&StepsQuery {
                context: Some(1),
                ..StepsQuery::default()
            })
            .expect("steps");
        assert_eq!(
            out,
            "draft · 6 steps: 5 · not run, 1 ? unverified · cursor before main:3\n\
             [main] 3 steps · goto retry if { if_visible = \"Error\" }\n  \
             main:2 · { action = \"screenshot\" }\n  \
             ▸ cursor\n  \
             main:3 · { action = \"tap\", on_text = \"Log in\" }  # the header button\n\
             [done] 1 step\n\
             [retry] 1 step · next main\n\
             [lead] 1 step · next main\n"
        );
    }

    #[test]
    fn the_listing_takes_an_address_a_block_and_a_limit() {
        let (_dir, mut d) = draft_of(FLOW);
        d.insert_at("main", None).expect("cursor");
        let around = d
            .steps(&StepsQuery {
                around: Some("retry:1".into()),
                context: Some(0),
                ..StepsQuery::default()
            })
            .expect("around");
        assert!(
            around.contains("  retry:1 ? { action = \"tap\", on_text = \"Retry\" }\n"),
            "{around}"
        );
        assert!(!around.contains("main:1"), "{around}");
        assert!(!around.contains("cursor\n"), "{around}");

        let block = d
            .steps(&StepsQuery {
                block: Some("main".into()),
                limit: Some(1),
                ..StepsQuery::default()
            })
            .expect("block");
        assert!(
            block.contains("  main:1 · { action = \"launch\", app = \"app\" }  # Launch fresh.\n"),
            "{block}"
        );
        assert!(!block.contains("main:2"), "{block}");

        let err = d
            .steps(&StepsQuery {
                around: Some("main:9".into()),
                ..StepsQuery::default()
            })
            .expect_err("no step");
        assert!(err.to_string().contains("has 3 steps"), "{err}");
    }

    #[test]
    fn export_reports_the_counts_and_each_unverified_step() {
        let (dir, mut d) = draft_of(FLOW);
        let done = d
            .export(&dir.path().join("f.test.toml"), false)
            .expect("export");
        assert_eq!(done.counts.to_string(), "5 · not run, 1 ? unverified");
        assert_eq!(
            done.unverified,
            [r#"retry:1 { action = "tap", on_text = "Retry" }"#]
        );
    }
}
