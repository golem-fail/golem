//! Edits to a flow's setup that no step carries: `[flow.options]`, a
//! block's own fields, and the `[[teardown]]` steps.

use anyhow::{bail, Context, Result};
use toml_edit::{Item, Table, Value};

use super::{json_to_value, Draft};

/// The `[flow.options]` keys, as `golem_parser::FlowOptions` reads them.
pub const OPTION_KEYS: &[&str] = &[
    "max_concurrency",
    "min_free_ram_mb",
    "min_free_disk_mb",
    "create_if_missing",
    "ignore_missing_physical",
    "step_timeout",
    "screenshot_on_failure",
    "screenshot_dir",
    "record",
    "max_steps",
    "max_runtime",
    "max_device_wait",
    "suite_concurrency",
    "keep_devices",
    "coverage",
    "app_lifecycle",
    "browser_headless",
    "perf",
    "perf_memory_warn_mb",
    "perf_memory_error_mb",
    "perf_cpu_warn_percent",
    "perf_cpu_error_percent",
    "perf_threads_warn",
    "perf_threads_error",
    "perf_fd_warn",
    "perf_fd_error",
];

/// The keys of a `[[block]]` that `block_set` sets: the ones that are not
/// its name, steps, `next` or branches, which other tools own.
pub const BLOCK_KEYS: &[&str] = &[
    "app", "for_each", "where", "run_flow", "vars", "save_to", "record",
];

impl Draft {
    /// Run `change`; if the draft then does not parse as a flow, or has a
    /// validation error it did not have before, undo it.
    pub(super) fn checked(&mut self, change: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        let before = self.doc.clone();
        let had = edit_errors(&self.text()).unwrap_or_default();
        let result = change(self).and_then(|()| {
            let new: Vec<String> = edit_errors(&self.text())?
                .into_iter()
                .filter(|e| !had.contains(e))
                .collect();
            if new.is_empty() {
                Ok(())
            } else {
                bail!("{}", new.join("; "))
            }
        });
        if result.is_err() {
            self.doc = before;
        }
        result
    }

    /// Set `[flow.options]` keys; a null value removes the key.
    pub fn options_set(
        &mut self,
        options: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<()> {
        if let Some(k) = options.keys().find(|k| !OPTION_KEYS.contains(&k.as_str())) {
            bail!(
                "{k:?} is not a flow option; the options: {}",
                OPTION_KEYS.join(", ")
            );
        }
        self.checked(|d| {
            let flow = d.flow_table()?;
            if !flow.contains_key("options") {
                flow.insert("options", Item::Table(Table::new()));
            }
            let table = flow["options"]
                .as_table_like_mut()
                .context("`[flow] options` is not a table")?;
            set_keys(table, options)
        })?;
        self.reparse()
    }

    /// Set a block's own fields (`BLOCK_KEYS`); a null value removes one.
    /// Its steps then run in another way: the first goes `?`, later ones `~`.
    pub fn block_set(
        &mut self,
        name: &str,
        fields: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<()> {
        if let Some(k) = fields.keys().find(|k| !BLOCK_KEYS.contains(&k.as_str())) {
            bail!(
                "block_set does not set {k:?}; it sets {}. block_link sets next and branches.",
                BLOCK_KEYS.join(", ")
            );
        }
        let b = self
            .block_index(name)
            .with_context(|| format!("the draft has no block named {name:?}"))?;
        self.checked(|d| {
            let block = d
                .doc
                .get_mut("block")
                .and_then(Item::as_array_of_tables_mut)
                .and_then(|blocks| blocks.get_mut(b))
                .context("the block went away")?;
            set_keys(block, fields)
        })?;
        self.reparse()?;
        self.after_change(b, 0, 0)
    }

    /// Remove `[[teardown]]` step `n`, counted from 1.
    pub fn teardown_delete(&mut self, n: usize) -> Result<()> {
        let steps = self
            .doc
            .get_mut("teardown")
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|t| t.get_mut(0))
            .and_then(|t| t.get_mut("steps"))
            .context("the draft has no [[teardown]] steps")?;
        let Some(Value::Array(array)) = steps.as_value_mut() else {
            bail!("the [[teardown]] steps are [[teardown.steps]] tables; edit them in the file");
        };
        let len = array.len();
        if n == 0 || n > len {
            bail!("the teardown has {len} steps; {n} is not one of them");
        }
        array.remove(n - 1);
        self.reparse()
    }
}

/// The validation errors an edit can be refused for. Not a missing block,
/// which a later `block_begin` can add, nor an app's missing devices, which
/// golem.toml can supply: export checks those.
fn edit_errors(text: &str) -> Result<Vec<String>> {
    use golem_parser::validation::ValidationErrorKind as Kind;
    let flow = golem_parser::parse_flow(text)
        .map_err(|e| anyhow::anyhow!("the flow would not parse: {e:#}"))?;
    Ok(golem_parser::validation::validate_flow(&flow)
        .into_iter()
        .filter(|e| {
            !matches!(
                e.kind,
                Kind::InvalidGotoTarget | Kind::InvalidStartBlock | Kind::MissingDevices
            )
        })
        .map(|e| e.message)
        .collect())
}

fn set_keys(
    table: &mut dyn toml_edit::TableLike,
    fields: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    for (k, v) in fields {
        if v.is_null() {
            table.remove(k);
        } else {
            table.insert(k, Item::Value(json_to_value(v)?));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::draft_of;
    use super::*;

    const FLOW: &str = r#"[flow]
name = "Login"

[[block]]
name = "main"
steps = [
  { action = "tap", on_text = "Go" },
]

[[block]]
name = "rows"
steps = [
  { action = "type", on_text = "Email", input = "${_each.email}" },
]

[[teardown]]
steps = [
  { action = "stop", app = "app" },
  { action = "screenshot" },
]
"#;

    fn obj(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        match v {
            serde_json::Value::Object(m) => m,
            _ => panic!("not an object"),
        }
    }

    #[test]
    fn options_set_writes_a_flow_options_table_and_a_null_removes_a_key() {
        let (_dir, mut d) = draft_of(FLOW);
        d.options_set(&obj(
            serde_json::json!({ "step_timeout": 8000, "app_lifecycle": "manual" }),
        ))
        .expect("options_set");
        let flow = golem_parser::parse_flow(&d.text()).expect("parse");
        let options = flow.flow.options.expect("options");
        assert_eq!(options.step_timeout, Some(8000));
        assert!(
            d.text()
                .contains("[flow.options]\napp_lifecycle = \"manual\"\nstep_timeout = 8000\n"),
            "{}",
            d.text()
        );
        d.options_set(&obj(serde_json::json!({ "step_timeout": null })))
            .expect("remove");
        assert!(!d.text().contains("step_timeout"), "{}", d.text());
    }

    #[test]
    fn options_set_refuses_an_unknown_key_or_a_wrong_type_and_keeps_the_draft() {
        let (_dir, mut d) = draft_of(FLOW);
        let err = d
            .options_set(&obj(serde_json::json!({ "step_timout": 8000 })))
            .expect_err("a typo");
        assert!(err.to_string().contains("not a flow option"), "{err}");
        let err = d
            .options_set(&obj(serde_json::json!({ "step_timeout": "slow" })))
            .expect_err("a string timeout");
        assert!(err.to_string().contains("would not parse"), "{err:#}");
        assert_eq!(d.text(), FLOW);
    }

    #[test]
    fn every_option_key_is_a_flow_option() {
        for key in OPTION_KEYS {
            let value = match *key {
                "coverage" => "\"smart\"".to_string(),
                "app_lifecycle" => "\"manual\"".to_string(),
                "screenshot_dir" | "max_runtime" | "max_device_wait" => "\"1m\"".to_string(),
                k if k.starts_with("perf_") && (k.ends_with("_mb") || k.ends_with("_percent")) => {
                    "1.0".to_string()
                }
                "create_if_missing"
                | "ignore_missing_physical"
                | "screenshot_on_failure"
                | "record"
                | "keep_devices"
                | "browser_headless"
                | "perf" => "true".to_string(),
                _ => "1".to_string(),
            };
            let text = format!("[flow]\nname = \"x\"\n\n[flow.options]\n{key} = {value}\n");
            let flow = golem_parser::parse_flow(&text).unwrap_or_else(|e| panic!("{key}: {e:#}"));
            let options = format!("{:?}", flow.flow.options.expect("options"));
            assert!(
                options.contains(&format!("{key}: Some(")),
                "{key}: {options}"
            );
        }
    }

    #[test]
    fn block_set_sets_fields_and_marks_the_block_steps() {
        let (_dir, mut d) = draft_of(FLOW);
        d.block_set(
            "rows",
            &obj(serde_json::json!({ "for_each": "data", "where": { "os": "ios" } })),
        )
        .expect("block_set");
        let flow = golem_parser::parse_flow(&d.text()).expect("parse");
        assert_eq!(flow.block[1].for_each.as_deref(), Some("data"));
        assert_eq!(
            flow.block[1].r#where.as_ref().and_then(|w| w.os.as_deref()),
            Some("ios")
        );
        assert!(d.text().contains("# unverified"), "{}", d.text());
        let err = d
            .block_set("rows", &obj(serde_json::json!({ "next": "main" })))
            .expect_err("next is block_link's");
        assert!(err.to_string().contains("block_link"), "{err}");
    }

    #[test]
    fn teardown_delete_removes_one_step() {
        let (_dir, mut d) = draft_of(FLOW);
        d.teardown_delete(1).expect("delete");
        let flow = golem_parser::parse_flow(&d.text()).expect("parse");
        assert_eq!(flow.teardown[0].steps.len(), 1);
        assert_eq!(flow.teardown[0].steps[0].action, "screenshot");
        assert!(d.teardown_delete(2).is_err());
    }
}
