//! Steps of the draft that a project mixin already holds: golem hints at
//! them while the model writes the flow, and `export_flow(mixins)` puts a
//! `load_mixin` step in their place.
//!
//! A mixin step is a template. A value that is exactly `${x}` matches any
//! value and binds `x`; the same `x` must bind the same value at each
//! place. A value with a template inside it, such as `"Hi ${name}"`,
//! matches as a pattern. `${fake:…}` and `${_…}` match only themselves.
//! golem compares parsed steps, so comments, key order and the step form
//! do not matter.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use toml_edit::{Item, Value};

use super::{parse_inline, Draft, StepStatus};

/// A mixin that a `load_mixin` in the draft can name.
#[derive(Debug, Clone)]
pub struct Mixin {
    pub name: String,
    steps: Vec<toml::Table>,
}

/// Where a run of steps is: a block, or the draft's `[[teardown]]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Place {
    Block(usize),
    Teardown,
}

/// A run of draft steps that a mixin holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixinMatch {
    pub place: Place,
    /// The first step, from 0.
    pub start: usize,
    pub len: usize,
    pub mixin: String,
    /// What each of the mixin's `${var}`s bound to.
    pub vars: BTreeMap<String, String>,
}

impl MixinMatch {
    /// The `load_mixin` step that runs the same steps.
    pub fn load_mixin_line(&self) -> String {
        let mut line = toml_edit::InlineTable::new();
        line.insert("action", "load_mixin".into());
        line.insert("mixin", self.mixin.as_str().into());
        if !self.vars.is_empty() {
            let mut vars = toml_edit::InlineTable::new();
            for (k, v) in &self.vars {
                vars.insert(k, v.as_str().into());
            }
            line.insert("vars", Value::InlineTable(vars));
        }
        line.fmt();
        Value::InlineTable(line).to_string().trim().to_string()
    }
}

/// The mixins a flow in `flow_dir` can load: each `__mixins__/` from
/// `flow_dir` up to `project_root`, the closest first for a name, as
/// `load_mixin` resolves them. A mixin that does not parse is left out.
pub fn mixins_for(flow_dir: &Path, project_root: &Path) -> Vec<Mixin> {
    let mut found: BTreeMap<String, PathBuf> = BTreeMap::new();
    for dir in flow_dir.ancestors() {
        let Ok(files) = std::fs::read_dir(dir.join("__mixins__")) else {
            if dir == project_root {
                break;
            }
            continue;
        };
        for file in files.flatten().map(|f| f.path()) {
            if file.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            if let Some(name) = file.file_stem().and_then(|s| s.to_str()) {
                found.entry(name.to_string()).or_insert(file);
            }
        }
        if dir == project_root {
            break;
        }
    }
    found
        .into_iter()
        .filter_map(|(name, path)| read_mixin(name, &path))
        .collect()
}

fn read_mixin(name: String, path: &Path) -> Option<Mixin> {
    let text = std::fs::read_to_string(path).ok()?;
    golem_parser::mixin::parse_mixin(&text).ok()?;
    let table: toml::Table = text.parse().ok()?;
    let steps = table
        .get("step")?
        .as_array()?
        .iter()
        .filter_map(|s| s.as_table().cloned())
        .collect::<Vec<_>>();
    (!steps.is_empty()).then_some(Mixin { name, steps })
}

/// Every mixin in the project, the shallowest first for a name: for a
/// draft that is not a file yet, which no directory limits.
pub fn all_mixins(project_root: &Path) -> Vec<Mixin> {
    let mut by_name: BTreeMap<String, (usize, PathBuf)> = BTreeMap::new();
    for r in super::reusables(project_root) {
        if r.kind != super::ReuseKind::Mixin {
            continue;
        }
        let depth = r.path.components().count();
        if by_name.get(&r.name).is_none_or(|(d, _)| depth < *d) {
            by_name.insert(r.name.clone(), (depth, r.path.clone()));
        }
    }
    by_name
        .into_iter()
        .filter_map(|(name, (_, path))| read_mixin(name, &path))
        .collect()
}

/// Whether the `steps` match the mixin's first `steps.len()` steps, with
/// one binding for each `${var}`.
fn match_run(template: &[toml::Table], steps: &[toml::Table]) -> Option<BTreeMap<String, String>> {
    if template.len() != steps.len() {
        return None;
    }
    let mut binds = BTreeMap::new();
    for (t, s) in template.iter().zip(steps) {
        if !match_table(t, s, &mut binds) {
            return None;
        }
    }
    Some(binds)
}

fn match_table(t: &toml::Table, s: &toml::Table, binds: &mut BTreeMap<String, String>) -> bool {
    t.len() == s.len()
        && t.iter()
            .all(|(k, tv)| s.get(k).is_some_and(|sv| match_value(tv, sv, binds)))
}

fn match_value(t: &toml::Value, s: &toml::Value, binds: &mut BTreeMap<String, String>) -> bool {
    match (t, s) {
        (toml::Value::String(t), toml::Value::String(s)) => match_template(t, s, binds),
        (toml::Value::Array(t), toml::Value::Array(s)) => {
            t.len() == s.len() && t.iter().zip(s).all(|(t, s)| match_value(t, s, binds))
        }
        (toml::Value::Table(t), toml::Value::Table(s)) => match_table(t, s, binds),
        _ => t == s,
    }
}

/// A template's parts: literal text, and the `${var}`s that bind.
#[derive(Debug, PartialEq)]
enum Part {
    Text(String),
    Var(String),
}

fn parts(template: &str) -> Vec<Part> {
    let mut out = Vec::new();
    let mut rest = template;
    let push_text = |out: &mut Vec<Part>, text: &str| {
        if text.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(Part::Text(t)) => t.push_str(text),
            _ => out.push(Part::Text(text.to_string())),
        }
    };
    while let Some(at) = rest.find("${") {
        let Some(end) = rest[at..].find('}') else {
            break;
        };
        let name = &rest[at + 2..at + end];
        push_text(&mut out, &rest[..at]);
        // A generator or a builtin is the same text in the mixin and in
        // the flow; only the mixin's own vars bind.
        if name.is_empty() || name.starts_with('_') || name.contains(':') {
            push_text(&mut out, &rest[at..=at + end]);
        } else {
            out.push(Part::Var(name.to_string()));
        }
        rest = &rest[at + end + 1..];
    }
    push_text(&mut out, rest);
    out
}

fn match_template(template: &str, value: &str, binds: &mut BTreeMap<String, String>) -> bool {
    let parts = parts(template);
    let mut trial = binds.clone();
    if match_parts(&parts, value, &mut trial) {
        *binds = trial;
        true
    } else {
        false
    }
}

/// Match `parts` against all of `value`, trying each split for a var.
fn match_parts(parts: &[Part], value: &str, binds: &mut BTreeMap<String, String>) -> bool {
    let Some((first, rest)) = parts.split_first() else {
        return value.is_empty();
    };
    match first {
        Part::Text(t) => value
            .strip_prefix(t.as_str())
            .is_some_and(|after| match_parts(rest, after, binds)),
        Part::Var(name) => {
            if let Some(bound) = binds.get(name).cloned() {
                return value
                    .strip_prefix(bound.as_str())
                    .is_some_and(|after| match_parts(rest, after, binds));
            }
            let ends: Vec<usize> = value
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(value.len()))
                .collect();
            for end in ends {
                let mut trial = binds.clone();
                trial.insert(name.clone(), value[..end].to_string());
                if match_parts(rest, &value[end..], &mut trial) {
                    *binds = trial;
                    return true;
                }
            }
            false
        }
    }
}

impl Draft {
    /// The steps of each place, as parsed tables, in file order.
    fn places(&self) -> Vec<(Place, Vec<toml::Table>)> {
        let parse = |line: String| -> toml::Table {
            format!("s = {line}")
                .parse::<toml::Table>()
                .ok()
                .and_then(|t| t.get("s").and_then(|s| s.as_table()).cloned())
                .unwrap_or_default()
        };
        let mut out: Vec<(Place, Vec<toml::Table>)> = (0..self.block_count())
            .map(|b| {
                let steps = (0..self.steps_len(b))
                    .map(|i| parse(self.step_line(b, i)))
                    .collect();
                (Place::Block(b), steps)
            })
            .collect();
        let teardown: Vec<toml::Table> = self
            .teardown_steps()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_inline_table())
                    .map(|t| {
                        let mut t = t.clone();
                        t.decor_mut().clear();
                        t.fmt();
                        parse(Value::InlineTable(t).to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !teardown.is_empty() {
            out.push((Place::Teardown, teardown));
        }
        out
    }

    fn teardown_steps(&self) -> Option<&toml_edit::Array> {
        self.doc
            .get("teardown")
            .and_then(Item::as_array_of_tables)
            .and_then(|t| t.get(0))
            .and_then(|t| t.get("steps"))
            .and_then(Item::as_array)
    }

    fn teardown_steps_mut(&mut self) -> Option<&mut toml_edit::Array> {
        self.doc
            .get_mut("teardown")
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|t| t.get_mut(0))
            .and_then(|t| t.get_mut("steps"))
            .and_then(Item::as_array_mut)
    }

    /// Each run of steps that one of `mixins` holds whole.
    pub fn mixin_matches(&self, mixins: &[Mixin]) -> Vec<MixinMatch> {
        let mut out = Vec::new();
        for (place, steps) in self.places() {
            for start in 0..steps.len() {
                for m in mixins {
                    let end = start + m.steps.len();
                    if end > steps.len() {
                        continue;
                    }
                    if let Some(vars) = match_run(&m.steps, &steps[start..end]) {
                        out.push(MixinMatch {
                            place,
                            start,
                            len: m.steps.len(),
                            mixin: m.name.clone(),
                            vars,
                        });
                    }
                }
            }
        }
        out
    }

    /// Each mixin whose first steps (two or more, but not all) are the
    /// steps of block `b` that end just before step `before`.
    pub fn mixin_starts(
        &self,
        b: usize,
        before: usize,
        mixins: &[Mixin],
    ) -> Vec<(usize, usize, String, usize)> {
        let Some((_, steps)) = self
            .places()
            .into_iter()
            .find(|(p, _)| *p == Place::Block(b))
        else {
            return Vec::new();
        };
        let before = before.min(steps.len());
        let mut out = Vec::new();
        for m in mixins {
            for k in (2..m.steps.len()).rev() {
                let Some(start) = before.checked_sub(k) else {
                    continue;
                };
                if match_run(&m.steps[..k], &steps[start..before]).is_some() {
                    out.push((start, k, m.name.clone(), m.steps.len()));
                    break;
                }
            }
        }
        out
    }

    /// How many steps block `b` has.
    pub fn steps_in_block(&self, b: usize) -> usize {
        self.steps_len(b)
    }

    /// `main:3` for a block step, `teardown:3` for a teardown step.
    pub fn place_address(&self, place: Place, i: usize) -> String {
        match place {
            Place::Block(b) => self.address(b, i),
            Place::Teardown => format!("teardown:{}", i + 1),
        }
    }

    /// The place and 0-based step of `main:3` or `teardown:3`.
    pub fn parse_place(&self, at: &str) -> Result<(Place, usize)> {
        if let Some(n) = at.strip_prefix("teardown:") {
            let n: usize = n
                .parse()
                .ok()
                .filter(|n| *n >= 1)
                .with_context(|| format!("{at:?}: the step after ':' counts from 1"))?;
            return Ok((Place::Teardown, n - 1));
        }
        let (b, i) = self.address_of(at)?;
        Ok((Place::Block(b), i))
    }

    /// Put `line` in place of the `len` steps from `start`. The new step
    /// keeps their status when they all share one, else it is unverified.
    pub fn replace_run(
        &mut self,
        place: Place,
        start: usize,
        len: usize,
        line: &str,
    ) -> Result<()> {
        let step = parse_inline(line)?;
        match place {
            Place::Block(b) => {
                let statuses: Vec<StepStatus> = (start..start + len)
                    .filter_map(|i| self.status_of(b, i))
                    .collect();
                let status = match statuses.split_first() {
                    Some((first, rest)) if rest.iter().all(|s| s == first) => *first,
                    _ => StepStatus::Unverified,
                };
                // remove_step drops each status and moves the insertion point.
                for i in (start + 1..start + len).rev() {
                    self.remove_step(b, i)?;
                }
                self.replace_step(b, start, step)?;
                self.reparse()?;
                self.set_status(b, start, status)?;
            }
            Place::Teardown => {
                let steps = self
                    .teardown_steps_mut()
                    .context("the draft has no [[teardown]] steps")?;
                if start + len > steps.len() {
                    bail!("the teardown has {} steps", steps.len());
                }
                let decor = steps.get(start).map(|v| v.decor().clone());
                for i in (start + 1..start + len).rev() {
                    steps.remove(i);
                }
                let mut value = Value::InlineTable(step);
                if let Some(decor) = decor {
                    *value.decor_mut() = decor;
                }
                steps.replace(start, value);
            }
        }
        self.reparse()
    }
}

/// The steps of each block and the teardown of a flow file's text, as
/// parsed tables, labelled as an address prefix (`main`, `teardown`).
fn file_places(text: &str) -> Vec<(String, Vec<toml::Table>)> {
    let Ok(doc) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let steps_of = |t: &toml::Table| -> Vec<toml::Table> {
        t.get("steps")
            .and_then(toml::Value::as_array)
            .map(|a| a.iter().filter_map(|s| s.as_table().cloned()).collect())
            .unwrap_or_default()
    };
    let mut out: Vec<(String, Vec<toml::Table>)> = doc
        .get("block")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_table)
        .enumerate()
        .map(|(b, t)| {
            let label = t
                .get("name")
                .and_then(toml::Value::as_str)
                .map_or_else(|| format!("block_{b}"), str::to_string);
            (label, steps_of(t))
        })
        .collect();
    if let Some(t) = doc
        .get("teardown")
        .and_then(toml::Value::as_array)
        .and_then(|a| a.first())
        .and_then(toml::Value::as_table)
    {
        out.push(("teardown".to_string(), steps_of(t)));
    }
    out
}

/// The `*.test.toml` files of the project, as `reusables` walks it.
fn flow_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.')
            || matches!(name.as_str(), "node_modules" | "target" | "build" | "dist")
        {
            continue;
        }
        if path.is_dir() {
            if depth < 6 && name != "__mixins__" && name != "__fixtures__" {
                flow_files(&path, out, depth + 1);
            }
        } else if name.ends_with(".test.toml") {
            out.push(path);
        }
    }
}

/// The fewest equal steps in a row that make a repeated run.
const MIN_RUN: usize = 3;

impl Draft {
    /// Each run of three or more steps of the draft that another flow of
    /// the project repeats, longest first for each place: a hint that the
    /// steps belong in a mixin. `skip` names the draft's own files.
    pub fn repeated_runs(&self, project_root: &Path, skip: &[PathBuf]) -> Vec<String> {
        let mut files = Vec::new();
        flow_files(project_root, &mut files, 0);
        let same = |a: &Path, b: &Path| {
            a == b
                || a.canonicalize()
                    .ok()
                    .zip(b.canonicalize().ok())
                    .is_some_and(|(a, b)| a == b)
        };
        let mut out = Vec::new();
        for (place, mine) in self.places() {
            for file in files.iter().filter(|f| !skip.iter().any(|s| same(s, f))) {
                let Ok(text) = std::fs::read_to_string(file) else {
                    continue;
                };
                let rel = file.strip_prefix(project_root).unwrap_or(file).display();
                for (label, theirs) in file_places(&text) {
                    for (i, j, k) in common_runs(&mine, &theirs) {
                        out.push(format!(
                            "hint: {}–{} also in {rel} {label}:{}–{} · consider a mixin",
                            self.place_address(place, i),
                            i + k,
                            j + 1,
                            j + k
                        ));
                    }
                }
            }
        }
        out
    }
}

/// Each maximal run of `MIN_RUN` or more equal steps that `a` and `b`
/// share, as (start in `a`, start in `b`, length).
fn common_runs(a: &[toml::Table], b: &[toml::Table]) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for i in 0..a.len() {
        for j in 0..b.len() {
            if a[i] != b[j] || (i > 0 && j > 0 && a[i - 1] == b[j - 1]) {
                continue;
            }
            let k = a[i..]
                .iter()
                .zip(&b[j..])
                .take_while(|(x, y)| x == y)
                .count();
            if k >= MIN_RUN {
                out.push((i, j, k));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(line: &str) -> toml::Table {
        format!("s = {line}")
            .parse::<toml::Table>()
            .expect("toml")
            .get("s")
            .and_then(|s| s.as_table())
            .cloned()
            .expect("table")
    }

    #[test]
    fn a_var_binds_the_same_value_at_each_place() {
        let template = [
            table(r#"{ action = "type", on_text = "Email", input = "${email}" }"#),
            table(r#"{ action = "assert_visible", on_text = "${email}" }"#),
        ];
        let same = [
            table(r#"{ input = "a@b.test", action = "type", on_text = "Email" }"#),
            table(r#"{ action = "assert_visible", on_text = "a@b.test" }"#),
        ];
        let binds = match_run(&template, &same).expect("match");
        assert_eq!(binds.get("email").map(String::as_str), Some("a@b.test"));
        let other = [
            table(r#"{ action = "type", on_text = "Email", input = "a@b.test" }"#),
            table(r#"{ action = "assert_visible", on_text = "c@d.test" }"#),
        ];
        assert!(match_run(&template, &other).is_none());
    }

    #[test]
    fn a_partial_template_matches_as_a_pattern() {
        let mut binds = BTreeMap::new();
        assert!(match_template(
            "Hi ${name}!",
            "Hi Ada Lovelace!",
            &mut binds
        ));
        assert_eq!(binds.get("name").map(String::as_str), Some("Ada Lovelace"));
        assert!(!match_template(
            "Hi ${name}!",
            "Hello Ada!",
            &mut BTreeMap::new()
        ));
        let mut two = BTreeMap::new();
        assert!(match_template("${a}-${b}", "x-y-z", &mut two));
        // The first var takes the shortest text that lets the rest match.
        assert_eq!((two["a"].as_str(), two["b"].as_str()), ("x", "y-z"));
    }

    #[test]
    fn a_generator_or_a_builtin_matches_only_itself() {
        assert!(match_template(
            "${fake:email}",
            "${fake:email}",
            &mut BTreeMap::new()
        ));
        assert!(!match_template(
            "${fake:email}",
            "a@b.test",
            &mut BTreeMap::new()
        ));
        assert!(match_template(
            "on ${_device}",
            "on ${_device}",
            &mut BTreeMap::new()
        ));
        assert!(!match_template(
            "on ${_device}",
            "on Pixel",
            &mut BTreeMap::new()
        ));
    }

    #[test]
    fn keys_must_be_the_same_and_values_of_other_types_equal() {
        let template = [table(
            r#"{ action = "tap", on_text = "${x}", timeout = 300 }"#,
        )];
        assert!(match_run(
            &template,
            &[table(
                r#"{ action = "tap", on_text = "Go", timeout = 300 }"#
            )]
        )
        .is_some());
        assert!(match_run(
            &template,
            &[table(
                r#"{ action = "tap", on_text = "Go", timeout = 500 }"#
            )]
        )
        .is_none());
        assert!(match_run(&template, &[table(r#"{ action = "tap", on_text = "Go" }"#)]).is_none());
        let nested = [table(
            r#"{ action = "tap", on = { text = "${x}", below = "Total" } }"#,
        )];
        assert!(match_run(
            &nested,
            &[table(
                r#"{ action = "tap", on = { below = "Total", text = "Pay" } }"#
            )]
        )
        .is_some());
    }

    #[test]
    fn a_match_gives_the_load_mixin_step() {
        let m = MixinMatch {
            place: Place::Block(0),
            start: 0,
            len: 2,
            mixin: "login".into(),
            vars: [("email".to_string(), "a@b.test".to_string())]
                .into_iter()
                .collect(),
        };
        assert_eq!(
            m.load_mixin_line(),
            r#"{ action = "load_mixin", mixin = "login", vars = { email = "a@b.test" } }"#
        );
    }

    #[test]
    fn only_runs_of_three_or_more_equal_steps_count() {
        let t = |x: &str| table(&format!(r#"{{ action = "tap", on_text = "{x}" }}"#));
        let mine = [t("a"), t("b"), t("c"), t("d"), t("x"), t("a"), t("b")];
        let theirs = [t("z"), t("a"), t("b"), t("c"), t("d"), t("e")];
        assert_eq!(common_runs(&mine, &theirs), [(0, 1, 4)]);
        let reordered = [
            table(r#"{ on_text = "a", action = "tap" }"#),
            t("b"),
            t("c"),
        ];
        assert_eq!(
            common_runs(&mine, &reordered),
            [(0, 0, 3)],
            "key order SHALL NOT matter"
        );
    }

    #[test]
    fn a_flow_file_gives_its_blocks_and_teardown() {
        let places = file_places(
            "[flow]\nname = \"f\"\n\n[[block]]\nname = \"auth\"\n[[block.steps]]\naction = \"tap\"\non_text = \"a\"\n\n[[block]]\nsteps = [{ action = \"tap\", on_text = \"b\" }]\n\n[[teardown]]\nsteps = [{ action = \"stop\" }]\n",
        );
        let labels: Vec<(&str, usize)> =
            places.iter().map(|(l, s)| (l.as_str(), s.len())).collect();
        assert_eq!(labels, [("auth", 1), ("block_1", 1), ("teardown", 1)]);
    }
}
