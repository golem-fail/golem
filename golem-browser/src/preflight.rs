use anyhow::Result;
use golem_parser::FlowFile;

/// Verb prefix marking a step as host-side browser automation.
pub const BROWSE_PREFIX: &str = "browse_";
/// Verb prefix for the WebMCP subset, which needs a browser feature that ships
/// switched off.
pub const BROWSE_MCP_PREFIX: &str = "browse_mcp_";

/// Whether any step in this flow file drives the browser.
///
/// A prefix scan, not a keyword list: the canonical set of actions is the
/// runner's dispatch match, and a second enumeration here would drift from it
/// every time an action lands. The prefix has nothing to drift against.
///
/// Scans the flow's own blocks and teardown only, on the already-mixin-expanded
/// flow the planner produces. Steps reached indirectly — a sub-flow named by
/// `run_flow`, or a mixin loaded from a teardown block (which the planner
/// doesn't expand) — are resolved at execution time; chasing them here would
/// mean loading and expanding arbitrary paths from inside a preflight. Those
/// flows still fail at their first browser step rather than before boot.
pub fn flow_uses_browser(flow: &FlowFile) -> bool {
    flow.block
        .iter()
        .flat_map(|b| &b.steps)
        .chain(flow.teardown.iter().flat_map(|t| &t.steps))
        .any(|s| s.action.starts_with(BROWSE_PREFIX))
}

/// Whether this flow drives the page's WebMCP tools.
///
/// Kept separate from [`flow_uses_browser`] because the answer decides whether
/// golem launches Chrome with an experimental Blink feature enabled. That
/// changes what any page on the run can feature-detect, so it is worth doing
/// only for the flows that asked.
pub fn flow_uses_webmcp(flow: &FlowFile) -> bool {
    flow.block
        .iter()
        .flat_map(|b| &b.steps)
        .chain(flow.teardown.iter().flat_map(|t| &t.steps))
        .any(|s| s.action.starts_with(BROWSE_MCP_PREFIX))
}

/// Check the host can serve whatever browser steps this suite contains.
///
/// Runs once per suite, before any device work: a missing Chrome is an
/// operator problem, and surfacing it after a 70-second emulator boot wastes
/// the run. Suites with no `browse_*` step never probe, so Chrome stays a
/// dependency of the flows that actually want it rather than of golem.
pub fn preflight<'a>(flows: impl IntoIterator<Item = &'a FlowFile>) -> Result<()> {
    let Some(flow) = flows.into_iter().find(|f| flow_uses_browser(f)) else {
        return Ok(());
    };
    browser_support(&flow.flow.name)
}

#[cfg(feature = "browser")]
fn browser_support(flow_name: &str) -> Result<()> {
    crate::chrome::locate()
        .map(|_| ())
        .map_err(|e| e.context(format!("flow `{flow_name}` uses browse_* steps")))
}

#[cfg(not(feature = "browser"))]
fn browser_support(flow_name: &str) -> Result<()> {
    Err(golem_events::coded(
        golem_events::FailureCode::HostBrowserUnsupported,
        anyhow::anyhow!(
            "flow `{flow_name}` uses browse_* steps, but this golem was built without \
             browser support — rebuild without `--no-default-features`, or with \
             `--features browser`"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(toml_src: &str) -> FlowFile {
        toml::from_str(toml_src).expect("fixture SHALL parse")
    }

    // 1. A flow with no browser step is not a browser flow.
    #[test]
    fn mobile_only_flow_does_not_use_browser() {
        let f = flow(
            r#"
            [flow]
            name = "mobile"
            [[block]]
            steps = [{ action = "tap", on_text = "Login" }]
            "#,
        );
        assert!(!flow_uses_browser(&f));
    }

    // 2. Any `browse_`-prefixed step marks the flow, including future
    //    actions this crate has never heard of.
    #[test]
    fn any_browse_prefixed_step_marks_the_flow() {
        let f = flow(
            r#"
            [flow]
            name = "web"
            [[block]]
            steps = [
              { action = "tap", on_text = "Login" },
              { action = "browse_navigate_to_some_action_that_does_not_exist_yet" },
            ]
            "#,
        );
        assert!(flow_uses_browser(&f));
    }

    // 3. Teardown counts — cleanup through a web portal is the motivating
    //    case for the whole feature.
    #[test]
    fn teardown_steps_count() {
        let f = flow(
            r#"
            [flow]
            name = "web-teardown"
            [[block]]
            steps = [{ action = "tap", on_text = "Login" }]
            [[teardown]]
            steps = [{ action = "browse_navigate" }]
            "#,
        );
        assert!(flow_uses_browser(&f));
    }

    // 4. A near-miss name is not a browser step.
    #[test]
    fn similar_action_names_do_not_match() {
        let f = flow(
            r#"
            [flow]
            name = "not-web"
            [[block]]
            steps = [{ action = "browser_navigate" }, { action = "browse" }]
            "#,
        );
        assert!(!flow_uses_browser(&f));
    }

    // 4b. WebMCP is detected on its own, so only the flows that use it pay for
    //     an experimental browser feature.
    #[test]
    fn webmcp_is_detected_separately_from_ordinary_browser_use() {
        let web = flow(
            r#"
            [flow]
            name = "web"
            [[block]]
            steps = [{ action = "browse_navigate" }]
            "#,
        );
        assert!(flow_uses_browser(&web));
        assert!(
            !flow_uses_webmcp(&web),
            "a plain browser flow SHALL NOT need WebMCP"
        );

        let mcp = flow(
            r#"
            [flow]
            name = "mcp"
            [[block]]
            steps = [{ action = "browse_mcp_call", tool = "fulfil" }]
            "#,
        );
        assert!(flow_uses_webmcp(&mcp));
        assert!(
            flow_uses_browser(&mcp),
            "an mcp step SHALL also count as browser use"
        );
    }

    // 5. A suite without browser steps never consults the host, so it passes
    //    on a machine with no Chrome and no browser support compiled in.
    #[test]
    fn preflight_passes_when_no_flow_uses_the_browser() {
        let f = flow(
            r#"
            [flow]
            name = "mobile"
            [[block]]
            steps = [{ action = "tap", on_text = "Login" }]
            "#,
        );
        assert!(preflight([&f]).is_ok());
    }

    // 6. Without the feature, a browser flow fails preflight as a HOST
    //    problem — the flow is valid, this binary just can't serve it.
    #[cfg(not(feature = "browser"))]
    #[test]
    fn preflight_without_feature_is_a_host_failure() {
        let f = flow(
            r#"
            [flow]
            name = "web"
            [[block]]
            steps = [{ action = "browse_navigate" }]
            "#,
        );
        let e = preflight([&f]).expect_err("browser flow SHALL fail without the feature");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(golem_events::FailureCode::HostBrowserUnsupported)
        );
        let msg = format!("{e:#}");
        assert!(
            msg.contains("web"),
            "message SHALL name the flow, got: {msg}"
        );
        assert!(
            msg.contains("--features browser"),
            "message SHALL say how to fix it, got: {msg}"
        );
    }
}
