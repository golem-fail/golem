use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_element::glob::GlobMatcher;
use golem_element::selector::{find_elements, Selector};
use golem_parser::BranchCondition;
use golem_vars::VariableStore;

/// Evaluate an ordered list of branch conditions and return the target block name
/// of the first matching condition. Returns `None` if no condition matches.
pub async fn evaluate_branch(
    conditions: &[BranchCondition],
    driver: &dyn PlatformDriver,
    vars: &VariableStore,
) -> Result<Option<String>> {
    for cond in conditions {
        if matches_condition(cond, driver, vars).await? {
            return Ok(Some(cond.goto.clone()));
        }
    }
    Ok(None)
}

/// Check whether a single branch condition matches the current state.
async fn matches_condition(
    cond: &BranchCondition,
    driver: &dyn PlatformDriver,
    vars: &VariableStore,
) -> Result<bool> {
    // Screen-based: if_visible
    if let Some(ref text_pattern) = cond.if_visible {
        let (hierarchy, _meta) = driver.get_hierarchy().await?;
        let selector = Selector {
            text: Some(text_pattern.clone()),
            ..Selector::default()
        };
        let results = find_elements(&hierarchy, &selector);
        return Ok(!results.is_empty());
    }

    // Screen-based: if_not_visible
    if let Some(ref text_pattern) = cond.if_not_visible {
        let (hierarchy, _meta) = driver.get_hierarchy().await?;
        let selector = Selector {
            text: Some(text_pattern.clone()),
            ..Selector::default()
        };
        let results = find_elements(&hierarchy, &selector);
        return Ok(results.is_empty());
    }

    // Variable-based: if_var + (equals | matches | gte)
    if let Some(ref var_name) = cond.if_var {
        let value = vars.get(var_name);

        // if_var + equals
        if let Some(ref expected) = cond.equals {
            return match value {
                Some(val) => match val.as_str() {
                    Some(s) => Ok(s == expected),
                    None => Ok(false),
                },
                None => Ok(false),
            };
        }

        // if_var + matches (glob)
        if let Some(ref pattern) = cond.matches {
            return match value {
                Some(val) => match val.as_str() {
                    Some(s) => {
                        let matcher = GlobMatcher::new(pattern);
                        Ok(matcher.is_match(s))
                    }
                    None => Ok(false),
                },
                None => Ok(false),
            };
        }

        // if_var + gte
        if let Some(threshold) = cond.gte {
            return match value {
                Some(val) => match val.as_str() {
                    Some(s) => match s.parse::<i64>() {
                        Ok(num) => Ok(num >= threshold),
                        Err(_) => Ok(false),
                    },
                    None => Ok(false),
                },
                None => Ok(false),
            };
        }

        // if_var specified but no comparison operator -- treat as undefined, skip
        return Ok(false);
    }

    // Default condition: no if_visible, if_not_visible, or if_var -- always matches
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::{Bounds, Element};
    use golem_vars::{Scope, ScopeLevel, VarValue, VariableStore};

    fn make_element(element_type: &str, text: Option<&str>, bounds: Bounds) -> Element {
        Element {
            element_type: element_type.to_string(),
            text: text.map(|s| s.to_string()),
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds,
            visible_bounds: None,
            children: Vec::new(),
        }
    }

    fn root_with_children(children: Vec<Element>) -> Element {
        Element {
            element_type: "View".to_string(),
            text: None,
            accessibility_label: None,
            placeholder: None,
            enabled: true,
            checked: false,
            clickable: true,
            focused: false,
            bounds: Bounds::new(0, 0, 375, 812),
            visible_bounds: None,
            children,
        }
    }

    fn empty_hierarchy() -> Element {
        root_with_children(Vec::new())
    }

    fn hierarchy_with_text(texts: &[&str]) -> Element {
        let children = texts
            .iter()
            .enumerate()
            .map(|(i, t)| {
                make_element(
                    "Label",
                    Some(t),
                    Bounds::new(10, (i as i32) * 50, 200, 40),
                )
            })
            .collect();
        root_with_children(children)
    }

    fn make_vars(entries: &[(&str, &str)]) -> VariableStore {
        let mut store = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Flow);
        for (k, v) in entries {
            scope.set(*k, VarValue::string(*v));
        }
        store.push_scope(scope);
        store
    }

    fn cond_if_visible(text: &str, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: Some(text.to_string()),
            if_not_visible: None,
            if_var: None,
            equals: None,
            matches: None,
            gte: None,
            goto: goto.to_string(),
        }
    }

    fn cond_if_not_visible(text: &str, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: Some(text.to_string()),
            if_var: None,
            equals: None,
            matches: None,
            gte: None,
            goto: goto.to_string(),
        }
    }

    fn cond_if_var_equals(var: &str, equals: &str, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_var: Some(var.to_string()),
            equals: Some(equals.to_string()),
            matches: None,
            gte: None,
            goto: goto.to_string(),
        }
    }

    fn cond_if_var_matches(var: &str, pattern: &str, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_var: Some(var.to_string()),
            equals: None,
            matches: Some(pattern.to_string()),
            gte: None,
            goto: goto.to_string(),
        }
    }

    fn cond_if_var_gte(var: &str, threshold: i64, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_var: Some(var.to_string()),
            equals: None,
            matches: None,
            gte: Some(threshold),
            goto: goto.to_string(),
        }
    }

    fn cond_default(goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_var: None,
            equals: None,
            matches: None,
            gte: None,
            goto: goto.to_string(),
        }
    }

    // ---------------------------------------------------------------
    // 1. if_visible matches when element exists
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_visible_matches_when_element_exists() {
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Try Premium Free"]));
        let vars = VariableStore::new();
        let conditions = vec![cond_if_visible("Try Premium Free", "premium")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("premium".to_string()));
    }

    // ---------------------------------------------------------------
    // 2. if_visible doesn't match when element absent
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_visible_skips_when_element_absent() {
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Get Started"]));
        let vars = VariableStore::new();
        let conditions = vec![cond_if_visible("Try Premium Free", "premium")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 3. if_not_visible matches when element absent
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_not_visible_matches_when_element_absent() {
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Get Started"]));
        let vars = VariableStore::new();
        let conditions = vec![cond_if_not_visible("Try Premium Free", "free_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("free_flow".to_string()));
    }

    // ---------------------------------------------------------------
    // 4. if_not_visible doesn't match when element exists
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_not_visible_skips_when_element_exists() {
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Try Premium Free"]));
        let vars = VariableStore::new();
        let conditions = vec![cond_if_not_visible("Try Premium Free", "free_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 5. if_var equals matches exact string
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_equals_matches_exact_string() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("variant", "premium")]);
        let conditions = vec![cond_if_var_equals("variant", "premium", "premium_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("premium_flow".to_string()));
    }

    // ---------------------------------------------------------------
    // 6. if_var equals doesn't match
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_equals_skips_when_no_match() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("variant", "free")]);
        let conditions = vec![cond_if_var_equals("variant", "premium", "premium_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 7. if_var matches glob pattern
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_matches_glob_pattern() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("country", "JP-Tokyo")]);
        let conditions = vec![cond_if_var_matches("country", "JP*", "japan_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("japan_flow".to_string()));
    }

    // ---------------------------------------------------------------
    // 8. if_var matches glob doesn't match
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_matches_glob_skips_when_no_match() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("country", "US-California")]);
        let conditions = vec![cond_if_var_matches("country", "JP*", "japan_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 9. if_var gte matches when value >= threshold
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_gte_matches_when_value_gte_threshold() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("_loop", "10")]);
        let conditions = vec![cond_if_var_gte("_loop", 10, "timeout_error")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("timeout_error".to_string()));
    }

    // ---------------------------------------------------------------
    // 10. if_var gte doesn't match when value < threshold
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_gte_skips_when_value_below_threshold() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("_loop", "5")]);
        let conditions = vec![cond_if_var_gte("_loop", 10, "timeout_error")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 11. Default (goto only, no condition) always matches
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn default_condition_always_matches() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = VariableStore::new();
        let conditions = vec![cond_default("fallback")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("fallback".to_string()));
    }

    // ---------------------------------------------------------------
    // 12. First matching condition wins
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn first_matching_condition_wins() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("variant", "premium")]);
        let conditions = vec![
            cond_if_var_equals("variant", "premium", "first_match"),
            cond_if_var_equals("variant", "premium", "second_match"),
            cond_default("fallback"),
        ];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("first_match".to_string()));
    }

    // ---------------------------------------------------------------
    // 13. No conditions match returns None
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn no_conditions_match_returns_none() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("variant", "free")]);
        let conditions = vec![
            cond_if_var_equals("variant", "premium", "premium_flow"),
            cond_if_var_equals("variant", "enterprise", "enterprise_flow"),
        ];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 14. Empty conditions list returns None
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn empty_conditions_returns_none() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = VariableStore::new();
        let conditions: Vec<BranchCondition> = vec![];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 15. Multiple conditions, only last default matches
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn multiple_conditions_only_default_matches() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("variant", "free")]);
        let conditions = vec![
            cond_if_var_equals("variant", "premium", "premium_flow"),
            cond_if_var_equals("variant", "enterprise", "enterprise_flow"),
            cond_default("fallback"),
        ];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("fallback".to_string()));
    }

    // ---------------------------------------------------------------
    // 16. if_var with undefined variable skips
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_with_undefined_variable_skips() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = VariableStore::new(); // no variables set
        let conditions = vec![cond_if_var_equals("missing_var", "anything", "target")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 17. if_visible with glob pattern in text
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_visible_with_glob_pattern() {
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Try Premium Free"]));
        let vars = VariableStore::new();
        let conditions = vec![cond_if_visible("Try *", "premium")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("premium".to_string()));
    }

    // ---------------------------------------------------------------
    // 18. if_var gte with non-numeric value skips
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_gte_with_non_numeric_value_skips() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("_loop", "not_a_number")]);
        let conditions = vec![cond_if_var_gte("_loop", 10, "timeout_error")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 19. Mixed conditions — screen + variable
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn mixed_screen_and_variable_conditions() {
        let driver =
            MockPlatformDriver::new(hierarchy_with_text(&["Welcome Back"]));
        let vars = make_vars(&[("variant", "premium")]);
        let conditions = vec![
            cond_if_visible("Try Premium Free", "premium_onboarding"),
            cond_if_var_equals("variant", "premium", "premium_flow"),
            cond_default("fallback"),
        ];

        // "Try Premium Free" is NOT visible, so first condition fails.
        // variant == "premium" matches, so second condition wins.
        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("premium_flow".to_string()));
    }

    // ---------------------------------------------------------------
    // 20. Multiple if_visible conditions tested in order
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn multiple_if_visible_conditions_tested_in_order() {
        let driver =
            MockPlatformDriver::new(hierarchy_with_text(&["Get Started", "Welcome"]));
        let vars = VariableStore::new();
        let conditions = vec![
            cond_if_visible("Try Premium Free", "premium"),
            cond_if_visible("Get Started", "free"),
            cond_if_visible("Welcome", "welcome"),
        ];

        // "Try Premium Free" is absent, skip.
        // "Get Started" is present — wins.
        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("free".to_string()));
    }

    // ---------------------------------------------------------------
    // 21. if_var with object value skips (not a string)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_with_object_value_skips() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut store = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Flow);
        scope.set(
            "user",
            VarValue::object(vec![("name", VarValue::string("Alice"))]),
        );
        store.push_scope(scope);

        let conditions = vec![cond_if_var_equals("user", "Alice", "user_flow")];

        let result = evaluate_branch(&conditions, &driver, &store)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 22. if_var gte matches when value exceeds threshold
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_gte_matches_when_value_exceeds_threshold() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("_loop", "15")]);
        let conditions = vec![cond_if_var_gte("_loop", 10, "timeout_error")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("timeout_error".to_string()));
    }

    // ---------------------------------------------------------------
    // 23. if_var matches with exact match (no glob chars)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_matches_exact_string_no_glob() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("country", "US")]);
        let conditions = vec![cond_if_var_matches("country", "US", "us_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("us_flow".to_string()));
    }

    // ---------------------------------------------------------------
    // 24. if_var matches with question mark glob
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_matches_question_mark_glob() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("region", "A1")]);
        let conditions = vec![cond_if_var_matches("region", "A?", "region_a_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("region_a_flow".to_string()));
    }

    // ---------------------------------------------------------------
    // 25. if_var gte with undefined variable skips
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_gte_with_undefined_variable_skips() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = VariableStore::new();
        let conditions = vec![cond_if_var_gte("counter", 5, "loop_end")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 26. if_var matches with undefined variable skips
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_matches_with_undefined_variable_skips() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = VariableStore::new();
        let conditions = vec![cond_if_var_matches("missing", "JP*", "japan_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 27. if_not_visible with glob pattern
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_not_visible_with_glob_pattern() {
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Welcome Back"]));
        let vars = VariableStore::new();
        // "Error *" is not present — should match
        let conditions = vec![cond_if_not_visible("Error *", "success_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("success_flow".to_string()));
    }

    // ---------------------------------------------------------------
    // 28. if_var gte with negative threshold
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_gte_with_negative_threshold() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("score", "-3")]);
        let conditions = vec![cond_if_var_gte("score", -5, "above_min")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("above_min".to_string()));
    }

    // ---------------------------------------------------------------
    // 29. if_var with no comparison operator skips
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_with_no_comparison_operator_skips() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("variant", "premium")]);
        // if_var set but no equals/matches/gte
        let conditions = vec![BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_var: Some("variant".to_string()),
            equals: None,
            matches: None,
            gte: None,
            goto: "orphan".to_string(),
        }];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 30. if_visible on empty hierarchy returns no match
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_visible_on_empty_hierarchy_returns_no_match() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = VariableStore::new();
        let conditions = vec![cond_if_visible("Anything", "target")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 31. if_not_visible on empty hierarchy returns match
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_not_visible_on_empty_hierarchy_returns_match() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = VariableStore::new();
        let conditions = vec![cond_if_not_visible("Anything", "empty_screen")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, Some("empty_screen".to_string()));
    }
}
