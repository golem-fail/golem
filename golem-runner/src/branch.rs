//! A block's branch: its arms race on one tree per poll, and the first arm
//! that matches, in order, wins.

use std::time::Duration;

use tokio::time::Instant;

use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_element::glob::GlobMatcher;
use golem_element::selector::Selector;
use golem_element::Element;
use golem_parser::{Block, BranchCondition};
use golem_vars::VariableStore;

const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How many texts of the last screen a no-match failure quotes.
const SCREEN_TEXTS: usize = 12;

/// Where the flow goes after `block`'s steps: the goto of the first arm that
/// matches, else the else arm's goto, else the block's `next`.
///
/// Each poll reads one tree and checks each live arm on it, in order. A
/// visibility arm stays live until its `timeout` (default: `assert_visible`'s)
/// runs out, so `timeout = 0` checks once. An `if_var` arm is checked on the
/// first poll only. The first poll reads a settled tree, so `if_not_visible`
/// does not match a screen that is not drawn yet.
///
/// With no match and no else, the flow fails with F406: falling through to
/// the next block in the file would run the flow in the wrong block.
pub async fn evaluate_branch(
    block: &Block,
    driver: &dyn PlatformDriver,
    vars: &VariableStore,
    base_timeout_ms: u64,
) -> Result<String> {
    let (arms, else_arm) = match block.branch.split_last() {
        Some((last, rest)) if last.is_else() => (rest, Some(last)),
        _ => (block.branch.as_slice(), None),
    };
    let default_ms = crate::policy::effective_timeout(
        &golem_parser::Step {
            action: "assert_visible".into(),
            ..Default::default()
        },
        base_timeout_ms,
    );
    let start = Instant::now();
    let deadlines: Vec<Option<Instant>> = arms
        .iter()
        .map(|a| {
            a.is_screen()
                .then(|| start + Duration::from_millis(a.timeout.unwrap_or(default_ms)))
        })
        .collect();
    let mut live = vec![true; arms.len()];
    let mut screen: Option<Element> = None;
    let mut first = true;
    loop {
        let wants_tree = arms.iter().zip(&live).any(|(a, l)| *l && a.is_screen());
        let tree = if wants_tree {
            match read_tree(driver, first).await {
                Ok(t) => Some(t),
                Err(e) if crate::recovery::is_companion_death_err(&e) => return Err(e),
                Err(_) if deadlines.iter().flatten().any(|d| Instant::now() < *d) => None,
                Err(e) => return Err(e),
            }
        } else {
            None
        };
        for (i, arm) in arms.iter().enumerate() {
            if !live[i] {
                continue;
            }
            let hit = if arm.is_screen() {
                match &tree {
                    Some((root, kb)) => screen_matches(arm, root, *kb),
                    None => false,
                }
            } else {
                live[i] = false;
                var_matches(arm, vars)
            };
            if hit {
                return Ok(arm.goto.clone());
            }
        }
        if let Some((root, _)) = tree {
            screen = Some(root);
        }
        let now = Instant::now();
        for (l, d) in live.iter_mut().zip(&deadlines) {
            if d.is_some_and(|d| now >= d) {
                *l = false;
            }
        }
        first = false;
        if !live.iter().any(|l| *l) {
            break;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    if let Some(arm) = else_arm {
        return Ok(arm.goto.clone());
    }
    if let Some(next) = &block.next {
        return Ok(next.clone());
    }
    Err(golem_events::coded(
        golem_events::FailureCode::FlowNoBranchMatched,
        anyhow::anyhow!("{}", no_match(block, arms, vars, screen.as_ref())),
    ))
}

/// The tree to check: settled on the first poll, as read after that. With
/// the keyboard's height, which hides what lies under it.
async fn read_tree(driver: &dyn PlatformDriver, settle: bool) -> Result<(Element, i32)> {
    if settle {
        let (root, meta, _) = crate::resolution::wait_for_settle(driver).await?;
        return Ok((root, meta.keyboard_height));
    }
    let (root, meta) = crate::resolution::get_hierarchy_bounded(driver).await?;
    Ok((root, meta.keyboard_height))
}

/// Whether a screen arm holds: a visibility arm on `root`'s visible tree,
/// the tree `assert_visible` judges; `if_alert` as `assert_alert` judges.
fn screen_matches(arm: &BranchCondition, root: &Element, keyboard_height: i32) -> bool {
    if let Some(pattern) = &arm.if_alert {
        return crate::actions::alert_matches(root, Some(pattern));
    }
    let (text, want) = match (&arm.if_visible, &arm.if_not_visible) {
        (Some(t), _) => (t, true),
        (_, Some(t)) => (t, false),
        _ => return false,
    };
    let selector = Selector {
        text: Some(text.clone()),
        ..Selector::default()
    };
    let (_, _, results) = crate::resolution::visible_matches(root, keyboard_height, &selector);
    results.is_empty() != want
}

/// Whether an `if_var` arm holds. An unset variable, or one that is not a
/// string, matches nothing.
fn var_matches(arm: &BranchCondition, vars: &VariableStore) -> bool {
    let Some(name) = &arm.if_var else {
        return false;
    };
    let Some(value) = vars.get(name).and_then(|v| v.as_str().map(str::to_string)) else {
        return false;
    };
    if let Some(expected) = &arm.equals {
        return &value == expected;
    }
    if let Some(pattern) = &arm.matches {
        return GlobMatcher::new(pattern).is_match(&value);
    }
    if let Some(threshold) = arm.gte {
        return value.parse::<i64>().is_ok_and(|n| n >= threshold);
    }
    false
}

/// The F406 message: the block, each arm and why it did not match, and
/// what the screen showed.
fn no_match(
    block: &Block,
    arms: &[BranchCondition],
    vars: &VariableStore,
    screen: Option<&Element>,
) -> String {
    let name = block.name.as_deref().unwrap_or("<unnamed>");
    let mut out = format!("block {name:?}: no branch arm matched, and the block has no else");
    for arm in arms {
        let why = if arm.if_alert.is_some() {
            let ms = arm
                .timeout
                .map_or(String::new(), |t| format!(", timeout {t}ms"));
            format!("no matching alert{ms}")
        } else if arm.is_screen() {
            let ms = arm
                .timeout
                .map_or(String::new(), |t| format!(", timeout {t}ms"));
            format!("not on screen{ms}")
        } else {
            match arm.if_var.as_deref().and_then(|n| vars.get(n)) {
                Some(v) => format!("value {:?}", v.as_str().unwrap_or("<not a string>")),
                None => "not set".to_string(),
            }
        };
        out.push_str(&format!("\n  {} → {}: {why}", arm.describe(), arm.goto));
    }
    if let Some(root) = screen {
        let mut texts = Vec::new();
        collect_texts(root, &mut texts);
        let more = texts.len().saturating_sub(SCREEN_TEXTS);
        texts.truncate(SCREEN_TEXTS);
        if texts.is_empty() {
            out.push_str("\n  screen: no text");
        } else {
            let quoted: Vec<String> = texts.iter().map(|t| format!("{t:?}")).collect();
            out.push_str(&format!("\n  screen: {}", quoted.join(", ")));
        }
        if more > 0 {
            out.push_str(&format!(", and {more} more"));
        }
    }
    out
}

fn collect_texts(e: &Element, out: &mut Vec<String>) {
    if let Some(t) = e
        .text
        .as_deref()
        .or(e.accessibility_label.as_deref())
        .filter(|t| !t.trim().is_empty())
    {
        out.push(t.to_string());
    }
    for c in &e.children {
        collect_texts(c, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One poll, as the branch checked before arms waited: each screen arm
    /// with `timeout = 0`, and `None` when no arm matches.
    async fn eval(
        conditions: &[BranchCondition],
        driver: &dyn PlatformDriver,
        vars: &VariableStore,
    ) -> Result<Option<String>> {
        let mut block: Block = toml::from_str("name = \"b\"").expect("block");
        block.branch = conditions
            .iter()
            .map(|c| BranchCondition {
                timeout: c.is_screen().then_some(0),
                ..c.clone()
            })
            .collect();
        match evaluate_branch(&block, driver, vars, 1000).await {
            Ok(target) => Ok(Some(target)),
            Err(e)
                if golem_events::extract_code(&e)
                    == Some(golem_events::FailureCode::FlowNoBranchMatched) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
    use golem_driver::MockPlatformDriver;
    use golem_element::{Bounds, Element};
    use golem_vars::{Scope, ScopeLevel, VarValue, VariableStore};

    fn make_element(element_type: &str, text: Option<&str>, bounds: Bounds) -> Element {
        Element {
            element_type: element_type.to_string(),
            text: text.map(|s| s.to_string()),
            accessibility_label: None,
            accessibility_id: None,
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
            accessibility_id: None,
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
            if_alert: None,
            if_var: None,
            equals: None,
            matches: None,
            gte: None,
            timeout: None,
            goto: goto.to_string(),
        }
    }

    fn cond_if_not_visible(text: &str, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: Some(text.to_string()),
            if_alert: None,
            if_var: None,
            equals: None,
            matches: None,
            gte: None,
            timeout: None,
            goto: goto.to_string(),
        }
    }

    fn cond_if_var_equals(var: &str, equals: &str, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_alert: None,
            if_var: Some(var.to_string()),
            equals: Some(equals.to_string()),
            matches: None,
            gte: None,
            timeout: None,
            goto: goto.to_string(),
        }
    }

    fn cond_if_var_matches(var: &str, pattern: &str, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_alert: None,
            if_var: Some(var.to_string()),
            equals: None,
            matches: Some(pattern.to_string()),
            gte: None,
            timeout: None,
            goto: goto.to_string(),
        }
    }

    fn cond_if_var_gte(var: &str, threshold: i64, goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_alert: None,
            if_var: Some(var.to_string()),
            equals: None,
            matches: None,
            gte: Some(threshold),
            timeout: None,
            goto: goto.to_string(),
        }
    }

    fn cond_default(goto: &str) -> BranchCondition {
        BranchCondition {
            if_visible: None,
            if_not_visible: None,
            if_alert: None,
            if_var: None,
            equals: None,
            matches: None,
            gte: None,
            timeout: None,
            goto: goto.to_string(),
        }
    }

    // ---------------------------------------------------------------
    // 1. if_visible matches when element exists
    // ---------------------------------------------------------------
    fn block_of(branch: Vec<BranchCondition>) -> Block {
        let mut block: Block = toml::from_str("name = \"start\"").expect("block");
        block.branch = branch;
        block
    }

    #[tokio::test(start_paused = true)]
    async fn an_if_alert_arm_waits_for_an_alert_whose_text_matches() {
        let mut alert_screen = empty_hierarchy();
        alert_screen.children.push(make_element(
            "Alert",
            Some("Allow notifications?"),
            Bounds::new(50, 200, 275, 150),
        ));
        let driver = MockPlatformDriver::new(alert_screen);
        for _ in 0..3 {
            driver.push_hierarchy(hierarchy_with_text(&["Home"]));
        }
        let arm = |pattern: &str| BranchCondition {
            if_alert: Some(pattern.into()),
            goto: "answer".into(),
            ..Default::default()
        };
        let vars = make_vars(&[]);
        assert_eq!(
            evaluate_branch(&block_of(vec![arm("Allow*")]), &driver, &vars, 10_000)
                .await
                .expect("the alert shows"),
            "answer"
        );
        let mut other = arm("Delete*");
        other.timeout = Some(0);
        let err = evaluate_branch(&block_of(vec![other]), &driver, &vars, 10_000)
            .await
            .expect_err("no matching alert");
        assert!(
            err.to_string()
                .contains("if_alert \"Delete*\" → answer: no matching alert"),
            "{err}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_arm_whose_screen_appears_first_wins_as_soon_as_it_appears() {
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Home"]));
        for _ in 0..4 {
            driver.push_hierarchy(empty_hierarchy());
        }
        let block = block_of(vec![
            cond_if_visible("Allow notifications", "permissions"),
            cond_if_visible("Home", "home"),
        ]);
        let start = Instant::now();
        let target = evaluate_branch(&block, &driver, &make_vars(&[]), 10_000)
            .await
            .expect("an arm matches");
        assert_eq!(target, "home");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "SHALL win when the screen appears, not at the timeout: {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_screen_arm_waits_its_timeout_and_timeout_0_checks_once() {
        let vars = make_vars(&[]);
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut arm = cond_if_visible("Home", "home");
        let assert_ms = crate::policy::effective_timeout(
            &golem_parser::Step {
                action: "assert_visible".into(),
                ..Default::default()
            },
            10_000,
        );
        for (timeout, at_least, below) in [
            (None, assert_ms, assert_ms + 1_000),
            (Some(2_000), 2_000, 3_000),
            (Some(0), 0, 1_000),
        ] {
            arm.timeout = timeout;
            let start = Instant::now();
            let err = evaluate_branch(&block_of(vec![arm.clone()]), &driver, &vars, 10_000)
                .await
                .expect_err("Home never shows");
            let ms = start.elapsed().as_millis() as u64;
            assert!(
                (at_least..below).contains(&ms),
                "{timeout:?}: waited {ms}ms"
            );
            assert_eq!(
                golem_events::extract_code(&err),
                Some(golem_events::FailureCode::FlowNoBranchMatched)
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_if_var_arm_is_checked_once_and_a_later_screen_arm_still_waits() {
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Home"]));
        for _ in 0..3 {
            driver.push_hierarchy(empty_hierarchy());
        }
        let vars = make_vars(&[("plan", "free")]);
        let block = block_of(vec![
            cond_if_var_equals("plan", "pro", "pro"),
            cond_if_visible("Home", "home"),
        ]);
        assert_eq!(
            evaluate_branch(&block, &driver, &vars, 10_000)
                .await
                .expect("Home shows"),
            "home"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn with_no_match_the_else_arm_wins_without_waiting_on_it() {
        let driver = MockPlatformDriver::new(empty_hierarchy());
        let mut arm = cond_if_visible("Home", "home");
        arm.timeout = Some(0);
        let block = block_of(vec![arm, cond_default("fallback")]);
        assert_eq!(
            evaluate_branch(&block, &driver, &make_vars(&[]), 10_000)
                .await
                .expect("the else"),
            "fallback"
        );
    }

    #[tokio::test]
    async fn if_visible_matches_when_element_exists() {
        let driver = MockPlatformDriver::new(hierarchy_with_text(&["Try Premium Free"]));
        let vars = VariableStore::new();
        let conditions = vec![cond_if_visible("Try Premium Free", "premium")];

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
            .await
            .expect("should not error");
        assert_eq!(result, None);
    }

    // ---------------------------------------------------------------
    // 4b. screen conditions judge the visible tree: an element that is
    //     in the hierarchy but off screen is not visible
    // ---------------------------------------------------------------
    fn hierarchy_with_offscreen_text(text: &str) -> Element {
        root_with_children(vec![make_element(
            "Label",
            Some(text),
            Bounds::new(10, 5000, 200, 40),
        )])
    }

    #[tokio::test]
    async fn if_not_visible_matches_when_element_only_off_screen() {
        let driver = MockPlatformDriver::new(hierarchy_with_offscreen_text("Try Premium Free"));
        let conditions = vec![cond_if_not_visible("Try Premium Free", "free_flow")];

        let result = eval(&conditions, &driver, &VariableStore::new())
            .await
            .expect("should not error");
        assert_eq!(result, Some("free_flow".to_string()));
    }

    #[tokio::test]
    async fn if_visible_skips_when_element_only_off_screen() {
        let driver = MockPlatformDriver::new(hierarchy_with_offscreen_text("Try Premium Free"));
        let conditions = vec![cond_if_visible("Try Premium Free", "premium")];

        let result = eval(&conditions, &driver, &VariableStore::new())
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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
        let result = eval(&conditions, &driver, &vars)
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
        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &store)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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
            if_alert: None,
            if_var: Some("variant".to_string()),
            equals: None,
            matches: None,
            gte: None,
            timeout: None,
            goto: "orphan".to_string(),
        }];

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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
            if_alert: None,
            if_var: Some("variant".to_string()),
            equals: Some("premium".to_string()),
            matches: None,
            gte: None,
            timeout: None,
            goto: "target".to_string(),
        }];

        let result = eval(&conditions, &driver, &vars)
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
            if_alert: None,
            if_var: Some("variant".to_string()),
            equals: Some("free".to_string()),
            matches: None,
            gte: None,
            timeout: None,
            goto: "target".to_string(),
        }];

        let result = eval(&conditions, &driver, &vars)
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
            if_alert: None,
            if_var: Some("variant".to_string()),
            equals: Some("premium".to_string()),
            matches: Some("*".to_string()),
            gte: Some(0),
            timeout: None,
            goto: "target".to_string(),
        }];

        let result = eval(&conditions, &driver, &vars)
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
            if_alert: None,
            if_var: Some("variant".to_string()),
            equals: None,
            matches: Some("JP*".to_string()),
            gte: Some(0),
            timeout: None,
            goto: "target".to_string(),
        }];

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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

        let result = eval(&conditions, &driver, &vars)
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
        let result = eval(&conditions, &driver, &vars).await;

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

        let result = eval(&conditions, &driver, &vars).await;

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
        let failed = eval(&conditions, &driver, &vars).await;
        assert!(failed.is_err(), "set_error SHALL make evaluation fail");

        // 2. After clearing, evaluation succeeds and the condition matches.
        driver.clear_error("get_hierarchy");
        let recovered = eval(&conditions, &driver, &vars)
            .await
            .expect("should not error after clear_error");
        assert_eq!(
            recovered,
            Some("premium".to_string()),
            "clear_error SHALL restore normal hierarchy-based matching"
        );
    }
}
