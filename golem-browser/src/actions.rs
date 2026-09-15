use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use chromiumoxide::element::Element;
use chromiumoxide::page::{Page, ScreenshotParams};
use golem_element::glob::glob_match;
use golem_events::FailureCode;
use golem_parser::Step;
use golem_vars::{ScopeLevel, VarValue, VariableStore};

use crate::pool::BrowserPool;
use crate::selector::{resolve_target, BrowserTarget};

/// How long to keep looking for an element before giving up, when the step
/// doesn't say. A page reached by `browse_navigate` is parsed, but anything
/// rendered by its scripts arrives later, so a single-shot query would make
/// every dynamic page a race. Explicit waiting is still `browse_wait` (#101) —
/// this is only the floor that stops ordinary steps flaking.
const DEFAULT_FIND_TIMEOUT_MS: u64 = 5_000;
/// A wait is an explicit "this may take a while", so it gets a longer budget
/// than the incidental lookup an ordinary action does.
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 10_000;
const FIND_POLL_MS: u64 = 50;

/// Run one `browse_*` step.
pub async fn execute_browser_action(
    pool: &mut BrowserPool,
    step: &Step,
    vars: &mut VariableStore,
) -> Result<()> {
    match step.action.as_str() {
        "browse_navigate" => navigate(pool, step).await,
        "browse_tap" => tap(pool, step).await,
        "browse_type" => type_text(pool, step).await,
        "browse_read" => read(pool, step, vars).await,
        "browse_screenshot" => screenshot(pool, step).await,
        "browse_close" => close(pool, step).await,
        "browse_assert_exists" => assert_exists(pool, step).await,
        "browse_assert_not_exists" => assert_not_exists(pool, step).await,
        "browse_assert_text" => assert_text(pool, step).await,
        "browse_wait_exists" => wait_exists(pool, step).await,
        "browse_wait_not_exists" => wait_not_exists(pool, step).await,
        other => Err(golem_events::coded(
            FailureCode::ParseUnknownAction,
            anyhow!("unknown browser action `{other}`"),
        )),
    }
}

/// Block until the element is in the DOM.
///
/// Reports a step timeout rather than "not found": a wait that runs out is a
/// synchronisation failure — the page never got where the flow expected — while
/// `browse_assert_exists` failing says the page is wrong. Both poll the same
/// way; they differ in what the report will tell you afterwards.
async fn wait_exists(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let target = resolve_target(step)?;
    let (page, ua) = page_for(pool, step).await?;
    let timeout_ms = wait_timeout(step);

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if locate(&page, &target).await.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let url = page.url().await.ok().flatten().unwrap_or_default();
            return Err(golem_events::coded(
                FailureCode::FlowStepTimeout,
                anyhow!("{target} never appeared within {timeout_ms}ms on {url} [browser: {ua}]"),
            ));
        }
        tokio::time::sleep(Duration::from_millis(FIND_POLL_MS)).await;
    }
}

/// Block until the element is gone from the DOM.
///
/// The one thing no assertion can do: `browse_assert_not_exists` answers "is it
/// gone now", this one answers "let it finish going". Spinners, toasts and
/// progress rows are the reason it exists.
async fn wait_not_exists(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let target = resolve_target(step)?;
    let (page, ua) = page_for(pool, step).await?;
    let timeout_ms = wait_timeout(step);

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if locate(&page, &target).await.is_none() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let url = page.url().await.ok().flatten().unwrap_or_default();
            return Err(golem_events::coded(
                FailureCode::FlowStepTimeout,
                anyhow!("{target} was still present after {timeout_ms}ms on {url} [browser: {ua}]"),
            ));
        }
        tokio::time::sleep(Duration::from_millis(FIND_POLL_MS)).await;
    }
}

/// The element must be in the DOM.
///
/// DOM presence, not visibility: the browser is instrumentation, and what a
/// headless Chrome "sees" is not what a user sees. Visibility judgements belong
/// to the mobile app under test.
async fn assert_exists(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let target = resolve_target(step)?;
    let (page, ua) = page_for(pool, step).await?;
    find(&page, &target, find_timeout(step), &ua)
        .await
        .map(drop)
}

/// The element must not be in the DOM.
///
/// One look, deliberately. Retrying would mean waiting the whole timeout to
/// confirm every absence — slow when the assertion passes, which is the common
/// case. Waiting for something to *go away* is `browse_wait_not` (#101).
async fn assert_not_exists(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let target = resolve_target(step)?;
    let (page, ua) = page_for(pool, step).await?;
    if locate(&page, &target).await.is_none() {
        return Ok(());
    }
    let url = page.url().await.ok().flatten().unwrap_or_default();
    Err(golem_events::coded(
        FailureCode::FlowUnexpectedlyPresent,
        anyhow!("{target} is still present on {url} [browser: {ua}]"),
    ))
}

/// The element's text (or one attribute) must match a pattern.
///
/// Polls until it matches rather than reading once: a page that updates text
/// after a click would otherwise be judged on whatever it happened to say when
/// the step began. The wait is bounded by the step's timeout, and the failure
/// quotes what the page actually said.
async fn assert_text(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let expected = required_param(step, "text")?;
    let target = resolve_target(step)?;
    let attribute = optional_param(step, "attribute");
    let (page, ua) = page_for(pool, step).await?;

    let deadline = Instant::now() + Duration::from_millis(find_timeout(step));
    let mut seen: Option<String> = None;
    loop {
        if let Some(element) = locate(&page, &target).await {
            let actual = match attribute {
                Some(name) => element.attribute(name).await.ok().flatten(),
                None => element.inner_text().await.ok().flatten(),
            };
            if actual.as_deref().is_some_and(|a| glob_match(expected, a)) {
                return Ok(());
            }
            seen = actual;
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(FIND_POLL_MS)).await;
    }

    let what = match attribute {
        Some(name) => format!("`{name}` of {target}"),
        None => format!("text of {target}"),
    };
    let actual = match seen {
        Some(text) => format!("{text:?}"),
        None => "nothing (no such element or attribute)".to_string(),
    };
    Err(golem_events::coded(
        FailureCode::FlowAssertionMismatch,
        anyhow!("{what} is {actual}, expected {expected:?} [browser: {ua}]"),
    ))
}

/// Hand back a tab the flow is finished with. Flow-end teardown still closes
/// whatever is left, so this is an early release, never a requirement.
async fn close(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    pool.close_session(optional_param(step, "session")).await
}

async fn navigate(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let url = required_param(step, "url")?;
    let wait = WaitUntil::from_step(step)?;
    let (page, ua) = page_for(pool, step).await?;

    page.goto(url)
        .await
        .map_err(|e| external_failure(format!("navigating to {url}: {e}"), &ua))?;
    wait.settle(&page, &ua).await
}

async fn tap(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let target = resolve_target(step)?;
    let (page, ua) = page_for(pool, step).await?;
    let element = find(&page, &target, find_timeout(step), &ua).await?;

    element
        .click()
        .await
        .map_err(|e| external_failure(format!("clicking {target}: {e}"), &ua))?;
    Ok(())
}

async fn type_text(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let value = typed_value(step)?;
    let target = resolve_target(step)?;
    let (page, ua) = page_for(pool, step).await?;
    let element = find(&page, &target, find_timeout(step), &ua).await?;

    // Click before typing: CDP types into whatever holds focus, so an
    // unfocused field would silently send the keystrokes somewhere else.
    element
        .click()
        .await
        .map_err(|e| external_failure(format!("focusing {target}: {e}"), &ua))?;
    element
        .type_str(&value)
        .await
        .map_err(|e| external_failure(format!("typing into {target}: {e}"), &ua))?;
    Ok(())
}

async fn read(pool: &mut BrowserPool, step: &Step, vars: &mut VariableStore) -> Result<()> {
    let target = resolve_target(step)?;
    let attribute = optional_param(step, "attribute");
    let (page, ua) = page_for(pool, step).await?;
    let element = find(&page, &target, find_timeout(step), &ua).await?;

    let text = match attribute {
        // An attribute often carries the value a test actually wants — a
        // `data-total` holding `1499` where the rendered text says `£14.99`.
        Some(name) => element
            .attribute(name)
            .await
            .map_err(|e| external_failure(format!("reading `{name}` of {target}: {e}"), &ua))?
            .ok_or_else(|| {
                golem_events::coded(
                    FailureCode::FlowElementNotFound,
                    anyhow!(
                        "{target} has no `{name}` attribute — the element matched, \
                         the attribute you asked to read doesn't exist [browser: {ua}]"
                    ),
                )
            })?,
        None => element
            .inner_text()
            .await
            .map_err(|e| external_failure(format!("reading text of {target}: {e}"), &ua))?
            .unwrap_or_default(),
    };

    if let Some(var_name) = &step.save_to {
        vars.set_in_scope(ScopeLevel::Flow, var_name, VarValue::string(&text));
    }
    Ok(())
}

async fn screenshot(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let (page, ua) = page_for(pool, step).await?;
    let png = page
        .screenshot(ScreenshotParams::builder().build())
        .await
        .map_err(|e| external_failure(format!("capturing the page: {e}"), &ua))?;

    // Written only when a `path` is given, exactly like the mobile `screenshot`
    // action: an unsaved capture still proves the page was reachable, and
    // choosing a directory here would duplicate the capture pipeline (#110).
    if let Some(path) = optional_param(step, "path") {
        tokio::fs::write(path, &png)
            .await
            .map_err(|e| external_failure(format!("writing screenshot to {path}: {e}"), &ua))?;
    }
    Ok(())
}

/// When to consider a navigation finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitUntil {
    /// Return as soon as Chrome accepts the navigation.
    None,
    /// The DOM is parsed (`readyState` past `loading`). The default: it's the
    /// point where a selector can match markup the server sent.
    DomContentLoaded,
    /// Sub-resources are done too (`readyState == "complete"`).
    Load,
}

impl WaitUntil {
    fn from_step(step: &Step) -> Result<Self> {
        match optional_param(step, "wait_until") {
            None => Ok(Self::DomContentLoaded),
            Some("none") => Ok(Self::None),
            Some("domcontentloaded") => Ok(Self::DomContentLoaded),
            Some("load") => Ok(Self::Load),
            Some(other) => Err(golem_events::coded(
                FailureCode::ParseMissingParam,
                anyhow!("unknown wait_until `{other}` — expected none, domcontentloaded, or load"),
            )),
        }
    }

    /// Poll `document.readyState` rather than chromiumoxide's
    /// `wait_for_navigation`: that resolves on the navigation *response*, which
    /// is a network fact, not a statement about the DOM a selector will query.
    async fn settle(self, page: &Page, ua: &str) -> Result<()> {
        let want: &[&str] = match self {
            Self::None => return Ok(()),
            Self::DomContentLoaded => &["interactive", "complete"],
            Self::Load => &["complete"],
        };
        let deadline = Instant::now() + Duration::from_millis(DEFAULT_FIND_TIMEOUT_MS);
        loop {
            let state: Option<String> = page
                .evaluate_expression("document.readyState")
                .await
                .ok()
                .and_then(|r| r.into_value().ok());
            if state.as_deref().is_some_and(|s| want.contains(&s)) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(golem_events::coded(
                    FailureCode::FlowStepTimeout,
                    anyhow!(
                        "page never reached `{self:?}` (readyState {}) [browser: {ua}]",
                        state.unwrap_or_else(|| "unknown".into())
                    ),
                ));
            }
            tokio::time::sleep(Duration::from_millis(FIND_POLL_MS)).await;
        }
    }
}

/// The tab this step acts on, plus the user agent to quote if it fails.
async fn page_for(pool: &mut BrowserPool, step: &Step) -> Result<(Page, String)> {
    let ua = pool.user_agent().await?.to_string();
    let page = pool
        .get_or_create_session(optional_param(step, "session"))
        .await?;
    Ok((page, ua))
}

async fn find(page: &Page, target: &BrowserTarget, timeout_ms: u64, ua: &str) -> Result<Element> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if let Some(element) = locate(page, target).await {
            return Ok(element);
        }
        if Instant::now() >= deadline {
            let url = page.url().await.ok().flatten().unwrap_or_default();
            return Err(golem_events::coded(
                FailureCode::FlowElementNotFound,
                anyhow!("{target} not found after {timeout_ms}ms on {url} [browser: {ua}]"),
            ));
        }
        tokio::time::sleep(Duration::from_millis(FIND_POLL_MS)).await;
    }
}

async fn locate(page: &Page, target: &BrowserTarget) -> Option<Element> {
    if target.index == 0 {
        return page.find_element(&target.selector).await.ok();
    }
    page.find_elements(&target.selector)
        .await
        .ok()?
        .into_iter()
        .nth(target.index)
}

fn find_timeout(step: &Step) -> u64 {
    step.timeout.unwrap_or(DEFAULT_FIND_TIMEOUT_MS)
}

fn wait_timeout(step: &Step) -> u64 {
    step.timeout.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS)
}

fn optional_param<'a>(step: &'a Step, name: &str) -> Option<&'a str> {
    step.params
        .get(name)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn required_param<'a>(step: &'a Step, name: &str) -> Result<&'a str> {
    optional_param(step, name).ok_or_else(|| {
        golem_events::coded(
            FailureCode::ParseMissingParam,
            anyhow!("{} action requires a `{name}` param", step.action),
        )
    })
}

/// The text `browse_type` sends.
///
/// `text` is the documented spelling — in a browser step the selector lives in
/// `selector`, so `text` is free to mean the value. `input` is accepted too
/// because that's what the mobile `type` action calls it, and a flow author
/// switching between the two shouldn't have to remember which is which.
fn typed_value(step: &Step) -> Result<String> {
    if let Some(text) = optional_param(step, "text") {
        return Ok(text.to_string());
    }
    if let Some(input) = step
        .input
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(input.to_string());
    }
    Err(golem_events::coded(
        FailureCode::ParseMissingParam,
        anyhow!("browse_type requires a `text` param (or `input`) holding the value to type"),
    ))
}

fn external_failure(message: String, ua: &str) -> anyhow::Error {
    golem_events::coded(
        FailureCode::FlowExternalFailed,
        anyhow!("{message} [browser: {ua}]"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::PoolConfig;
    use golem_vars::Scope;

    fn step(toml_src: &str) -> Step {
        toml::from_str(toml_src).expect("fixture SHALL parse")
    }

    fn vars() -> VariableStore {
        let mut store = VariableStore::new();
        store.push_scope(Scope::new(ScopeLevel::Flow));
        store
    }

    fn pool() -> BrowserPool {
        BrowserPool::new(PoolConfig::default())
    }

    /// Skip when the host has no browser — a dev box without Chrome shouldn't
    /// fail the suite. The live tests below drive a real one, so they run long
    /// (nextest SLOW) by nature and are feature-gated besides.
    fn chrome_available() -> bool {
        crate::chrome::locate().is_ok()
    }

    /// A page built from markup, in the default session. Written through the
    /// page rather than served, so the tests need no localhost server: the
    /// e2e flow in #107 is where a real server earns its keep.
    async fn page_with(pool: &mut BrowserPool, html: &str) -> Page {
        let page = pool
            .get_or_create_session(None)
            .await
            .expect("session SHALL open");
        page.set_content(html).await.expect("content SHALL load");
        page
    }

    async fn run(pool: &mut BrowserPool, s: &Step, v: &mut VariableStore) -> Result<()> {
        execute_browser_action(pool, s, v).await
    }

    /// `expect_err` that names the case being exercised.
    trait UnwrapErrOrElse<T> {
        fn unwrap_err_or_else(self, what: &str) -> anyhow::Error;
    }
    impl<T: std::fmt::Debug> UnwrapErrOrElse<T> for Result<T> {
        fn unwrap_err_or_else(self, what: &str) -> anyhow::Error {
            match self {
                Err(e) => e,
                Ok(v) => panic!("{what} SHALL fail, got Ok({v:?})"),
            }
        }
    }

    fn saved(v: &VariableStore, key: &str) -> String {
        match v.get(key) {
            Some(VarValue::String(s)) => s.clone(),
            other => panic!("expected a string in `{key}`, got {other:?}"),
        }
    }

    // ── param validation: no browser needed, so these run everywhere ──

    // 1. A param error is raised before anything launches — a malformed step
    //    never costs a browser start.
    #[tokio::test]
    async fn missing_url_is_a_param_error_without_launching() {
        let mut p = pool();
        let e = run(&mut p, &step(r#"action = "browse_navigate""#), &mut vars())
            .await
            .expect_err("navigate SHALL require a url");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::ParseMissingParam)
        );
        assert!(!p.is_running(), "a param error SHALL NOT launch a browser");
    }

    // 2. Every core action validates before launching, so a typo in any of
    //    them fails fast and cheaply rather than after a browser start.
    #[tokio::test]
    async fn every_action_validates_before_launching() {
        let cases = [
            ("tap without a selector", "action = \"browse_tap\""),
            ("read without a selector", "action = \"browse_read\""),
            (
                "type without a value",
                "action = \"browse_type\"\nselector = \"#name\"",
            ),
            ("assert_exists without a selector", "action = \"browse_assert_exists\""),
            ("wait_exists without a selector", "action = \"browse_wait_exists\""),
            (
                "wait_not_exists without a selector",
                "action = \"browse_wait_not_exists\"",
            ),
            (
                "assert_not_exists without a selector",
                "action = \"browse_assert_not_exists\"",
            ),
            (
                "assert_text without an expected value",
                "action = \"browse_assert_text\"\nselector = \"#status\"",
            ),
            (
                "navigate with an unknown settle point",
                "action = \"browse_navigate\"\nurl = \"https://example.com\"\nwait_until = \"eventually\"",
            ),
        ];
        for (what, src) in cases {
            let mut p = pool();
            let e = run(&mut p, &step(src), &mut vars())
                .await
                .unwrap_err_or_else(what);
            assert_eq!(
                golem_events::extract_code(&e),
                Some(FailureCode::ParseMissingParam),
                "{what} SHALL be a param error"
            );
            assert!(!p.is_running(), "{what} SHALL NOT launch a browser");
        }
    }

    // 3. An unrecognised browse_* verb is an unknown action, not a silent no-op.
    #[tokio::test]
    async fn unknown_browse_action_is_rejected() {
        let mut p = pool();
        let e = run(&mut p, &step(r#"action = "browse_teleport""#), &mut vars())
            .await
            .expect_err("an unknown browser action SHALL fail");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::ParseUnknownAction)
        );
    }

    // 4. `browse_type` takes the value from `text`, and from `input` too so a
    //    flow author moving between mobile and browser steps isn't tripped up.
    #[test]
    fn typed_value_accepts_text_and_input() {
        assert_eq!(
            typed_value(&step("action = \"browse_type\"\ntext = \"hello\"")).expect("text"),
            "hello"
        );
        assert_eq!(
            typed_value(&step("action = \"browse_type\"\ninput = \"hello\"")).expect("input"),
            "hello"
        );
        let e = typed_value(&step("action = \"browse_type\"")).expect_err("neither SHALL fail");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::ParseMissingParam)
        );
    }

    // 5. wait_until parses the three documented settle points and nothing else.
    #[test]
    fn wait_until_parses_known_values_only() {
        let cases = [
            (r#"action = "browse_navigate""#, WaitUntil::DomContentLoaded),
            (
                "action = \"browse_navigate\"\nwait_until = \"none\"",
                WaitUntil::None,
            ),
            (
                "action = \"browse_navigate\"\nwait_until = \"load\"",
                WaitUntil::Load,
            ),
        ];
        for (src, want) in cases {
            assert_eq!(
                WaitUntil::from_step(&step(src)).expect("SHALL parse"),
                want,
                "for {src}"
            );
        }
        let e = WaitUntil::from_step(&step(
            "action = \"browse_navigate\"\nwait_until = \"whenever\"",
        ))
        .expect_err("an unknown settle point SHALL be rejected");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::ParseMissingParam)
        );
    }

    // ── live browser ──

    // 5b. A wait gets a longer default budget than an incidental lookup, and
    //     both still honour an explicit step timeout.
    #[test]
    fn waits_get_a_longer_default_budget_than_lookups() {
        let bare = step(r#"action = "browse_wait_exists""#);
        assert_eq!(wait_timeout(&bare), 10_000);
        assert_eq!(find_timeout(&bare), 5_000);

        let explicit = step("action = \"browse_wait_exists\"\ntimeout = 250");
        assert_eq!(wait_timeout(&explicit), 250);
        assert_eq!(find_timeout(&explicit), 250);
    }

    // 6. navigate then read: the default settle leaves the DOM queryable, so
    //    the very next step finds server-sent markup without an explicit wait.
    #[tokio::test]
    async fn live_navigate_then_read_reaches_the_new_page() {
        if !chrome_available() {
            return;
        }
        let mut p = pool();
        let mut v = vars();
        run(
            &mut p,
            &step(
                r#"action = "browse_navigate"
                   url = "data:text/html,<h1>Greetings</h1>""#,
            ),
            &mut v,
        )
        .await
        .expect("navigate SHALL succeed");
        run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \"h1\"\nsave_to = \"title\""),
            &mut v,
        )
        .await
        .expect("read SHALL succeed");
        assert_eq!(saved(&v, "title"), "Greetings");
        p.close().await.expect("close SHALL succeed");
    }

    // 7. One page, the three interacting actions: the click reaches the page's
    //    own handler (a real click, not a synthetic event a listener could
    //    miss), typing produces keystrokes `oninput` observes, and each result
    //    is read back. Grouped into one test because every live test costs a
    //    browser launch, and these only mean anything together anyway.
    #[tokio::test]
    async fn live_tap_and_type_drive_the_page() {
        if !chrome_available() {
            return;
        }
        let mut p = pool();
        let mut v = vars();
        page_with(
            &mut p,
            "<button id='go' onclick=\"document.getElementById('out').textContent='clicked'\">Go</button>\
             <span id='out'></span>\
             <input id='name' oninput=\"document.getElementById('echo').textContent=this.value\">\
             <span id='echo'></span>",
        )
        .await;

        run(
            &mut p,
            &step("action = \"browse_tap\"\nselector = \"#go\""),
            &mut v,
        )
        .await
        .expect("tap SHALL succeed");
        run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \"#out\"\nsave_to = \"out\""),
            &mut v,
        )
        .await
        .expect("read SHALL succeed");
        assert_eq!(saved(&v, "out"), "clicked", "the click handler SHALL run");

        run(
            &mut p,
            &step("action = \"browse_type\"\nselector = \"#name\"\ntext = \"ada\""),
            &mut v,
        )
        .await
        .expect("type SHALL succeed");
        run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \"#echo\"\nsave_to = \"echo\""),
            &mut v,
        )
        .await
        .expect("read SHALL succeed");
        assert_eq!(saved(&v, "echo"), "ada", "typing SHALL fire `oninput`");

        // `browse_close` hands the tab back early; doing it twice is fine,
        // because the caller's intent (this tab should not be open) holds
        // either way, and flow-end teardown will close whatever is left.
        run(&mut p, &step(r#"action = "browse_close""#), &mut v)
            .await
            .expect("close SHALL succeed");
        run(&mut p, &step(r#"action = "browse_close""#), &mut v)
            .await
            .expect("closing an already-closed session SHALL be a no-op");

        p.close().await.expect("close SHALL succeed");
    }

    // 8. Reading: rendered text by default, a raw attribute on request (the
    //    motivating case being `data-total` over a formatted price), a loud
    //    failure when that attribute is absent, and `index` for the nth match.
    #[tokio::test]
    async fn live_read_text_attributes_and_indexed_matches() {
        if !chrome_available() {
            return;
        }
        let mut p = pool();
        let mut v = vars();
        page_with(
            &mut p,
            "<span id='total' data-total='1499'>£14.99</span>\
             <p class='row'>first</p><p class='row'>second</p><p class='row'>third</p>",
        )
        .await;

        run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \"#total\"\nsave_to = \"shown\""),
            &mut v,
        )
        .await
        .expect("text read SHALL succeed");
        run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \"#total\"\nattribute = \"data-total\"\nsave_to = \"raw\""),
            &mut v,
        )
        .await
        .expect("attribute read SHALL succeed");
        assert_eq!(saved(&v, "shown"), "£14.99");
        assert_eq!(saved(&v, "raw"), "1499");

        // A missing attribute fails loudly: the silent alternative is an empty
        // variable that passes a later assertion for the wrong reason.
        let e = run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \"#total\"\nattribute = \"data-absent\"\nsave_to = \"nope\""),
            &mut v,
        )
        .await
        .expect_err("a missing attribute SHALL fail");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::FlowElementNotFound)
        );
        assert!(v.get("nope").is_none(), "nothing SHALL be saved on failure");

        run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \".row\"\nindex = 2\nsave_to = \"row\""),
            &mut v,
        )
        .await
        .expect("indexed read SHALL succeed");
        assert_eq!(saved(&v, "row"), "third");

        p.close().await.expect("close SHALL succeed");
    }

    // 9. Assertions, on one page: presence and absence hold; a present element
    //    fails an absence check; text matches exactly and by glob; a mismatch
    //    quotes what the page actually said; and an attribute can be asserted
    //    the same way it can be read.
    #[tokio::test]
    async fn live_assertions_judge_dom_presence_and_text() {
        if !chrome_available() {
            return;
        }
        let mut p = pool();
        let mut v = vars();
        page_with(
            &mut p,
            "<span id='status'>Fulfilled</span><span id='total' data-total='1499'>Total: £14.99</span>",
        )
        .await;

        run(
            &mut p,
            &step("action = \"browse_assert_exists\"\nselector = \"#status\""),
            &mut v,
        )
        .await
        .expect("a present element SHALL satisfy assert_exists");
        run(
            &mut p,
            &step("action = \"browse_assert_not_exists\"\nselector = \"#error-banner\""),
            &mut v,
        )
        .await
        .expect("an absent element SHALL satisfy assert_not_exists");

        let e = run(
            &mut p,
            &step("action = \"browse_assert_not_exists\"\nselector = \"#status\""),
            &mut v,
        )
        .await
        .expect_err("a present element SHALL fail assert_not_exists");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::FlowUnexpectedlyPresent)
        );

        run(
            &mut p,
            &step("action = \"browse_assert_text\"\nselector = \"#status\"\ntext = \"Fulfilled\""),
            &mut v,
        )
        .await
        .expect("an exact match SHALL pass");
        run(
            &mut p,
            &step("action = \"browse_assert_text\"\nselector = \"#total\"\ntext = \"Total: *\""),
            &mut v,
        )
        .await
        .expect("a glob SHALL match the way mobile text matchers do");
        run(
            &mut p,
            &step(
                "action = \"browse_assert_text\"\nselector = \"#total\"\nattribute = \"data-total\"\ntext = \"1499\"",
            ),
            &mut v,
        )
        .await
        .expect("an attribute SHALL be assertable");

        let e = run(
            &mut p,
            &step(
                "action = \"browse_assert_text\"\nselector = \"#status\"\ntext = \"Cancelled\"\ntimeout = 150",
            ),
            &mut v,
        )
        .await
        .expect_err("a mismatch SHALL fail");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::FlowAssertionMismatch)
        );
        let msg = format!("{e:#}");
        assert!(
            msg.contains("Fulfilled") && msg.contains("Cancelled"),
            "the failure SHALL quote both what was found and what was expected: {msg}"
        );

        p.close().await.expect("close SHALL succeed");
    }

    // 10. A text assertion waits for the page to catch up, so an element
    //     updated after a click isn't judged on what it said beforehand.
    #[tokio::test]
    async fn live_assert_text_waits_for_the_page_to_update() {
        if !chrome_available() {
            return;
        }
        let mut p = pool();
        let mut v = vars();
        page_with(
            &mut p,
            "<span id='status'>Pending</span>\
             <script>setTimeout(() => document.getElementById('status').textContent = 'Fulfilled', 300)</script>",
        )
        .await;

        run(
            &mut p,
            &step("action = \"browse_assert_text\"\nselector = \"#status\"\ntext = \"Fulfilled\""),
            &mut v,
        )
        .await
        .expect("the assertion SHALL wait for the later update");

        p.close().await.expect("close SHALL succeed");
    }

    // 11. Waits track the page: one element arrives late and the other leaves
    //     late, and both waits return as soon as that happens rather than
    //     burning their budget.
    #[tokio::test]
    async fn live_waits_follow_elements_appearing_and_disappearing() {
        if !chrome_available() {
            return;
        }
        let mut p = pool();
        let mut v = vars();
        page_with(
            &mut p,
            "<div class='spinner'></div>\
             <script>setTimeout(() => {\
               document.querySelector('.spinner').remove();\
               const row = document.createElement('p');\
               row.className = 'order-row';\
               document.body.appendChild(row);\
             }, 300)</script>",
        )
        .await;

        let started = std::time::Instant::now();
        run(
            &mut p,
            &step("action = \"browse_wait_not_exists\"\nselector = \".spinner\""),
            &mut v,
        )
        .await
        .expect("the spinner SHALL be waited out");
        run(
            &mut p,
            &step("action = \"browse_wait_exists\"\nselector = \".order-row\""),
            &mut v,
        )
        .await
        .expect("the late row SHALL be waited for");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "a wait SHALL return when the page changes, not when its budget runs out"
        );

        p.close().await.expect("close SHALL succeed");
    }

    // 12. A wait that runs out is a step timeout, not a missing element: the
    //     page never got where the flow expected, which is a different report
    //     from "the page is wrong".
    #[tokio::test]
    async fn live_expired_waits_report_step_timeouts() {
        if !chrome_available() {
            return;
        }
        let mut p = pool();
        let mut v = vars();
        page_with(&mut p, "<div class='spinner'></div>").await;

        for src in [
            "action = \"browse_wait_exists\"\nselector = \"#never\"\ntimeout = 150",
            "action = \"browse_wait_not_exists\"\nselector = \".spinner\"\ntimeout = 150",
        ] {
            let e = run(&mut p, &step(src), &mut v)
                .await
                .unwrap_err_or_else(src);
            assert_eq!(
                golem_events::extract_code(&e),
                Some(FailureCode::FlowStepTimeout),
                "an expired wait SHALL be a step timeout: {src}"
            );
            assert!(
                format!("{e:#}").contains("browser:"),
                "the failure SHALL quote the user agent: {src}"
            );
        }

        p.close().await.expect("close SHALL succeed");
    }

    // 13. Diagnostics: a missing element fails as not-found once the step's
    //    timeout is up, naming the selector and quoting the browser; and a
    //    screenshot writes a real PNG when a path is given.
    #[tokio::test]
    async fn live_screenshot_writes_png_and_missing_element_reports_not_found() {
        if !chrome_available() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir SHALL be created");
        let path = dir.path().join("shot.png");
        let mut p = pool();
        let mut v = vars();
        page_with(&mut p, "<h1>shot</h1>").await;

        run(
            &mut p,
            &step(&format!(
                "action = \"browse_screenshot\"\npath = \"{}\"",
                path.display()
            )),
            &mut v,
        )
        .await
        .expect("screenshot SHALL succeed");
        let bytes = std::fs::read(&path).expect("screenshot file SHALL exist");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "the file SHALL be a PNG");

        let e = run(
            &mut p,
            &step("action = \"browse_tap\"\nselector = \"#absent\"\ntimeout = 150"),
            &mut v,
        )
        .await
        .expect_err("a missing element SHALL fail");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::FlowElementNotFound)
        );
        let msg = format!("{e:#}");
        assert!(msg.contains("#absent"), "SHALL name the selector: {msg}");
        assert!(
            msg.contains("browser:"),
            "SHALL quote the user agent: {msg}"
        );

        p.close().await.expect("close SHALL succeed");
    }
}
