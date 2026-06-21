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
        let (hierarchy, _meta) = crate::resolution::get_hierarchy_bounded(driver).await?;
        let selector = Selector {
            text: Some(text_pattern.clone()),
            ..Selector::default()
        };
        let results = find_elements(&hierarchy, &selector);
        return Ok(!results.is_empty());
    }

    // Screen-based: if_not_visible
    if let Some(ref text_pattern) = cond.if_not_visible {
        let (hierarchy, _meta) = crate::resolution::get_hierarchy_bounded(driver).await?;
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
            hit_points: vec![],
            drawing_order: None,
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
            hit_points: vec![],
            drawing_order: None,
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
            .map(|(i, t)| make_element("Label", Some(t), Bounds::new(10, (i as i32) * 50, 200, 40)))
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
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Welcome Back"]));
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
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Get Started", "Welcome"]));
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

    // ---------------------------------------------------------------
    // 32. if_visible takes precedence over if_var on the same condition.
    //     The screen check runs first and returns, so if_var/equals are
    //     never consulted even though they would match.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_visible_short_circuits_before_if_var() {
        // Screen does NOT contain the target text, so if_visible -> false
        // and the condition is skipped, despite the var equals matching.
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Get Started"]));
        let vars = make_vars(&[("variant", "premium")]);
        let conditions = vec![BranchCondition {
            if_visible: Some("Try Premium Free".to_string()),
            if_not_visible: None,
            if_var: Some("variant".to_string()),
            equals: Some("premium".to_string()),
            matches: None,
            gte: None,
            goto: "target".to_string(),
        }];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(
            result, None,
            "if_visible SHALL be evaluated before if_var and short-circuit it"
        );
    }

    // ---------------------------------------------------------------
    // 33. if_not_visible takes precedence over if_var on the same
    //     condition. The screen check returns first; if_var is ignored.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_not_visible_short_circuits_before_if_var() {
        // Screen contains the text, so if_not_visible -> false and the
        // condition is skipped, despite the var equals matching.
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Try Premium Free"]));
        let vars = make_vars(&[("variant", "free")]);
        let conditions = vec![BranchCondition {
            if_visible: None,
            if_not_visible: Some("Try Premium Free".to_string()),
            if_var: Some("variant".to_string()),
            equals: Some("free".to_string()),
            matches: None,
            gte: None,
            goto: "target".to_string(),
        }];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(
            result, None,
            "if_not_visible SHALL be evaluated before if_var and short-circuit it"
        );
    }

    // ---------------------------------------------------------------
    // 34. With equals + matches + gte all set on one if_var condition,
    //     equals is checked first and its result is returned — matches
    //     and gte are never consulted.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn equals_takes_precedence_over_matches_and_gte() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("variant", "free")]);
        // equals="premium" fails. matches="*" and gte would otherwise
        // affect the result, but equals returns first.
        let conditions = vec![BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_var: Some("variant".to_string()),
            equals: Some("premium".to_string()),
            matches: Some("*".to_string()),
            gte: Some(0),
            goto: "target".to_string(),
        }];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(
            result, None,
            "equals SHALL be evaluated before matches/gte and short-circuit them"
        );
    }

    // ---------------------------------------------------------------
    // 35. With matches + gte set (no equals), matches is checked first
    //     and its result is returned — gte is never consulted.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn matches_takes_precedence_over_gte() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        // 1. Value "5" is chosen so the two operators disagree: matches="JP*"
        //    fails on "5", but gte=0 WOULD match because 5 >= 0. Only the
        //    short-circuit (matches checked first, returns its false) explains
        //    a None result; if gte were consulted the goto would be returned.
        let vars = make_vars(&[("variant", "5")]);
        let conditions = vec![BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_var: Some("variant".to_string()),
            equals: None,
            matches: Some("JP*".to_string()),
            gte: Some(0),
            goto: "target".to_string(),
        }];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(
            result, None,
            "matches SHALL be evaluated before gte and short-circuit it"
        );
    }

    // ---------------------------------------------------------------
    // 36. if_var equals matches an empty-string value against an
    //     empty-string expected value.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_equals_empty_string_matches() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("note", "")]);
        let conditions = vec![cond_if_var_equals("note", "", "blank_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(
            result,
            Some("blank_flow".to_string()),
            "empty stored value SHALL equal empty expected value"
        );
    }

    // ---------------------------------------------------------------
    // 37. if_var gte at exact boundary (value == threshold) matches.
    //     Confirms the comparison is >= and not strictly >.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_gte_at_exact_boundary_matches() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("count", "0")]);
        let conditions = vec![cond_if_var_gte("count", 0, "boundary")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(
            result,
            Some("boundary".to_string()),
            "value equal to threshold SHALL satisfy gte"
        );
    }

    // ---------------------------------------------------------------
    // 38. if_var gte one below a negative threshold does not match,
    //     confirming signed comparison.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_var_gte_below_negative_threshold_skips() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let vars = make_vars(&[("score", "-6")]);
        let conditions = vec![cond_if_var_gte("score", -5, "above_min")];

        let result = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(
            result, None,
            "value below a negative threshold SHALL NOT satisfy gte"
        );
    }

    // ---------------------------------------------------------------
    // 39. A driver whose hierarchy fetch returns Err surfaces the error
    //     out of evaluate_branch for an if_visible condition, rather than
    //     being silently treated as "not visible".
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_visible_propagates_hierarchy_fetch_error() {
        // 1. A driver whose get_hierarchy is wired to fail.
        let driver = MockPlatformDriver::new(empty_hierarchy());
        driver.set_error("get_hierarchy", "device disconnected");
        let vars = VariableStore::new();
        let conditions = vec![cond_if_visible("Try Premium Free", "premium")];

        // 2. evaluate_branch SHALL return the underlying fetch error.
        let result = evaluate_branch(&conditions, &driver, &vars).await;

        assert!(
            result.is_err(),
            "hierarchy-fetch Err SHALL propagate out of evaluate_branch"
        );
        let message = result
            .expect_err("expected Err from failing fetch")
            .to_string();
        assert!(
            message.contains("device disconnected"),
            "error SHALL carry the underlying fetch message, got: {message}"
        );
    }

    // ---------------------------------------------------------------
    // 40. The same hierarchy-fetch Err also surfaces for an
    //     if_not_visible condition (the other screen-based branch path).
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn if_not_visible_propagates_hierarchy_fetch_error() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        driver.set_error("get_hierarchy", "adb broken pipe");
        let vars = VariableStore::new();
        let conditions = vec![cond_if_not_visible("Try Premium Free", "free_flow")];

        let result = evaluate_branch(&conditions, &driver, &vars).await;

        let message = result
            .expect_err("expected Err from failing fetch")
            .to_string();
        assert!(
            message.contains("adb broken pipe"),
            "if_not_visible SHALL propagate the fetch error, got: {message}"
        );
    }

    // ---------------------------------------------------------------
    // 41. clearing the injected error restores normal evaluation:
    //     after clear_error, the same if_visible condition matches the
    //     steady hierarchy as usual.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn cleared_hierarchy_fetch_error_resumes_normal_evaluation() {
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Try Premium Free"]));
        driver.set_error("get_hierarchy", "transient failure");
        let vars = VariableStore::new();
        let conditions = vec![cond_if_visible("Try Premium Free", "premium")];

        // 1. While the error is set, evaluation fails.
        let failed = evaluate_branch(&conditions, &driver, &vars).await;
        assert!(failed.is_err(), "set_error SHALL make evaluation fail");

        // 2. After clearing, evaluation succeeds and the condition matches.
        driver.clear_error("get_hierarchy");
        let recovered = evaluate_branch(&conditions, &driver, &vars)
            .await
            .expect("should not error after clear_error");
        assert_eq!(
            recovered,
            Some("premium".to_string()),
            "clear_error SHALL restore normal hierarchy-based matching"
        );
    }
}
