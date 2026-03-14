// Data-driven row expansion: turns [[data]] rows into independent runs.

use std::collections::HashMap;

use golem_vars::{ScopeLevel, VarValue, VariableStore};

/// A single data-driven run configuration.
pub struct DataRun {
    /// Zero-based index of this run in the data row list.
    pub index: usize,
    /// Variables from the corresponding `[[data]]` row.
    pub vars: HashMap<String, String>,
    /// Human-readable label, e.g. `"data[0]: payment=credit_card"`.
    pub label: String,
}

/// Expand data rows into individual run configurations.
///
/// Each entry in `data` becomes a separate [`DataRun`] with its own variable set.
pub fn expand_data_rows(data: &[HashMap<String, String>]) -> Vec<DataRun> {
    data.iter()
        .enumerate()
        .map(|(i, row)| {
            let label = format_label(i, row);
            DataRun {
                index: i,
                vars: row.clone(),
                label,
            }
        })
        .collect()
}

/// Merge data-row variables into a [`VariableStore`] at the [`ScopeLevel::Flow`] level.
///
/// This ensures data-row values override project/fixture/generator variables while
/// still being overridable by CLI-level variables.
pub fn apply_data_vars(store: &mut VariableStore, data_vars: &HashMap<String, String>) {
    for (key, value) in data_vars {
        store.set_in_scope(ScopeLevel::Flow, key, VarValue::String(value.clone()));
    }
}

/// Return the list of runs for a flow.
///
/// - If `data` is empty, returns a single default run (the normal non-data-driven case).
/// - Otherwise, calls [`expand_data_rows`] to produce one run per row.
pub fn get_runs(data: &[HashMap<String, String>]) -> Vec<DataRun> {
    if data.is_empty() {
        vec![DataRun {
            index: 0,
            vars: HashMap::new(),
            label: "default".to_string(),
        }]
    } else {
        expand_data_rows(data)
    }
}

/// Build a human-readable label from a data row.
///
/// The output is deterministic: keys are sorted alphabetically so the label is
/// stable across runs regardless of `HashMap` iteration order.
fn format_label(index: usize, row: &HashMap<String, String>) -> String {
    let mut pairs: Vec<String> = row.iter().map(|(k, v)| format!("{k}={v}")).collect();
    pairs.sort();
    format!("data[{index}]: {}", pairs.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_vars::Scope;

    // ---------------------------------------------------------------
    // 1. Single data row produces one DataRun
    // ---------------------------------------------------------------
    #[test]
    fn single_row_produces_one_run() {
        let data = vec![HashMap::from([
            ("payment".to_string(), "credit_card".to_string()),
        ])];

        let runs = expand_data_rows(&data);
        assert_eq!(runs.len(), 1);
    }

    // ---------------------------------------------------------------
    // 2. Multiple data rows produce correct number of DataRuns
    // ---------------------------------------------------------------
    #[test]
    fn multiple_rows_produce_correct_count() {
        let data = vec![
            HashMap::from([("x".to_string(), "1".to_string())]),
            HashMap::from([("x".to_string(), "2".to_string())]),
            HashMap::from([("x".to_string(), "3".to_string())]),
        ];

        let runs = expand_data_rows(&data);
        assert_eq!(runs.len(), 3);
    }

    // ---------------------------------------------------------------
    // 3. Empty data produces single default run
    // ---------------------------------------------------------------
    #[test]
    fn empty_data_produces_default_run() {
        let runs = get_runs(&[]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].label, "default");
        assert!(runs[0].vars.is_empty());
        assert_eq!(runs[0].index, 0);
    }

    // ---------------------------------------------------------------
    // 4. Data vars merged into VariableStore correctly
    // ---------------------------------------------------------------
    #[test]
    fn data_vars_merged_into_store() {
        let mut store = VariableStore::new();
        let data_vars = HashMap::from([
            ("payment".to_string(), "credit_card".to_string()),
            ("expected".to_string(), "$29.99".to_string()),
        ]);

        apply_data_vars(&mut store, &data_vars);

        let payment = store.resolve("payment").expect("payment should resolve");
        assert_eq!(payment, &VarValue::String("credit_card".to_string()));

        let expected = store.resolve("expected").expect("expected should resolve");
        assert_eq!(expected, &VarValue::String("$29.99".to_string()));
    }

    // ---------------------------------------------------------------
    // 5. Data vars override existing flow vars
    // ---------------------------------------------------------------
    #[test]
    fn data_vars_override_existing_flow_vars() {
        let mut store = VariableStore::new();

        // Pre-populate a Flow-level variable
        store.set_in_scope(ScopeLevel::Flow, "payment", VarValue::String("cash".to_string()));

        // Override it via data row
        let data_vars = HashMap::from([("payment".to_string(), "paypal".to_string())]);
        apply_data_vars(&mut store, &data_vars);

        let val = store.resolve("payment").expect("should resolve");
        assert_eq!(val, &VarValue::String("paypal".to_string()));
    }

    // ---------------------------------------------------------------
    // 6. Label format is readable and deterministic
    // ---------------------------------------------------------------
    #[test]
    fn label_format_is_readable() {
        let row = HashMap::from([
            ("payment".to_string(), "credit_card".to_string()),
            ("expected".to_string(), "$29.99".to_string()),
        ]);

        let label = format_label(0, &row);

        // Keys are sorted alphabetically
        assert_eq!(label, "data[0]: expected=$29.99, payment=credit_card");
    }

    // ---------------------------------------------------------------
    // 7. DataRun index matches position
    // ---------------------------------------------------------------
    #[test]
    fn data_run_index_matches_position() {
        let data = vec![
            HashMap::from([("a".to_string(), "1".to_string())]),
            HashMap::from([("a".to_string(), "2".to_string())]),
            HashMap::from([("a".to_string(), "3".to_string())]),
        ];

        let runs = expand_data_rows(&data);
        for (i, run) in runs.iter().enumerate() {
            assert_eq!(run.index, i, "run at position {i} should have index {i}");
        }
    }

    // ---------------------------------------------------------------
    // 8. Each DataRun has independent variables
    // ---------------------------------------------------------------
    #[test]
    fn each_run_has_independent_variables() {
        let data = vec![
            HashMap::from([
                ("user".to_string(), "alice".to_string()),
                ("role".to_string(), "admin".to_string()),
            ]),
            HashMap::from([
                ("user".to_string(), "bob".to_string()),
                ("role".to_string(), "viewer".to_string()),
            ]),
        ];

        let runs = expand_data_rows(&data);

        assert_eq!(runs[0].vars.get("user").map(|s| s.as_str()), Some("alice"));
        assert_eq!(runs[0].vars.get("role").map(|s| s.as_str()), Some("admin"));

        assert_eq!(runs[1].vars.get("user").map(|s| s.as_str()), Some("bob"));
        assert_eq!(runs[1].vars.get("role").map(|s| s.as_str()), Some("viewer"));
    }

    // ---------------------------------------------------------------
    // 9. get_runs with data delegates to expand_data_rows
    // ---------------------------------------------------------------
    #[test]
    fn get_runs_with_data_delegates_to_expand() {
        let data = vec![
            HashMap::from([("k".to_string(), "v1".to_string())]),
            HashMap::from([("k".to_string(), "v2".to_string())]),
        ];

        let runs = get_runs(&data);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].vars.get("k").map(|s| s.as_str()), Some("v1"));
        assert_eq!(runs[1].vars.get("k").map(|s| s.as_str()), Some("v2"));
    }

    // ---------------------------------------------------------------
    // 10. Data vars do not override CLI-level vars (CLI wins)
    // ---------------------------------------------------------------
    #[test]
    fn data_vars_do_not_override_cli_vars() {
        let mut store = VariableStore::new();

        // CLI-level variable has highest priority
        let mut cli_scope = Scope::new(ScopeLevel::Cli);
        cli_scope.set("env", VarValue::String("staging".to_string()));
        store.push_scope(cli_scope);

        // Data row tries to set the same variable at Flow level
        let data_vars = HashMap::from([("env".to_string(), "production".to_string())]);
        apply_data_vars(&mut store, &data_vars);

        // CLI should still win
        let val = store.resolve("env").expect("should resolve");
        assert_eq!(val, &VarValue::String("staging".to_string()));
    }

    // ---------------------------------------------------------------
    // 11. Label for empty row
    // ---------------------------------------------------------------
    #[test]
    fn label_for_empty_row() {
        let row = HashMap::new();
        let label = format_label(0, &row);
        assert_eq!(label, "data[0]: ");
    }
}
