use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use chromiumoxide::cdp::browser_protocol::network::{CookieParam, SetUserAgentOverrideParams};
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

/// Where a step's file params resolve from, mirroring the `run` action: a
/// leading `/` means project-root-relative, anything else is relative to the
/// flow file's own directory.
#[derive(Debug, Clone, Copy)]
pub struct ScriptPaths<'a> {
    pub flow_dir: &'a Path,
    pub project_root: &'a Path,
}

/// Run one `browse_*` step.
pub async fn execute_browser_action(
    pool: &mut BrowserPool,
    step: &Step,
    vars: &mut VariableStore,
    paths: ScriptPaths<'_>,
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
        "browse_execute_js" => execute_js(pool, step, vars, paths).await,
        "browse_select" => select(pool, step).await,
        "browse_scroll_by" => scroll_by(pool, step).await,
        "browse_scroll_to" => scroll_to(pool, step).await,
        "browse_mcp_list_tools" => mcp_list_tools(pool, step, vars).await,
        "browse_mcp_call" => mcp_call(pool, step, vars).await,
        "browse_set_cookie" => set_cookie(pool, step).await,
        "browse_get_cookie" => get_cookie(pool, step, vars).await,
        "browse_set_local_storage" => set_storage(pool, step, Storage::Local).await,
        "browse_get_local_storage" => get_storage(pool, step, vars, Storage::Local).await,
        "browse_set_session_storage" => set_storage(pool, step, Storage::Session).await,
        "browse_get_session_storage" => get_storage(pool, step, vars, Storage::Session).await,
        other => Err(golem_events::coded(
            FailureCode::ParseUnknownAction,
            anyhow!("unknown browser action `{other}`"),
        )),
    }
}

/// The tools a page has registered with WebMCP.
///
/// A page that opts into WebMCP describes what it can do — "fulfil an order",
/// "issue a refund" — as named tools with argument schemas. Driving those beats
/// clicking through its UI: the page states its own contract, so a flow that
/// calls one isn't coupled to a layout that may be redesigned next quarter.
async fn mcp_list_tools(
    pool: &mut BrowserPool,
    step: &Step,
    vars: &mut VariableStore,
) -> Result<()> {
    let (page, ua) = page_for(pool, step).await?;
    require_webmcp(&page, &ua).await?;

    let listed: serde_json::Value = page
        .evaluate_function(
            "async () => {
                const tools = await document.modelContext.getTools();
                return Object.fromEntries(tools.map((t) => [t.name, t.description ?? '']));
            }",
        )
        .await
        .map_err(|e| external_failure(format!("listing WebMCP tools: {e}"), &ua))?
        .into_value()
        .unwrap_or(serde_json::Value::Null);

    if let Some(var_name) = &step.save_to {
        // Keyed by tool name so `${tools.fulfil_order}` reads as a presence
        // check as well as a description — an array would only be greppable
        // text in a flow file.
        vars.set_in_scope(ScopeLevel::Flow, var_name, json_to_var(listed));
    }
    Ok(())
}

/// Run one of the page's registered WebMCP tools.
async fn mcp_call(pool: &mut BrowserPool, step: &Step, vars: &mut VariableStore) -> Result<()> {
    let tool = required_param(step, "tool")?;
    let arguments = match step.params.get("arguments") {
        Some(value) => toml_to_json(value),
        None => serde_json::Value::Object(serde_json::Map::new()),
    };
    let (page, ua) = page_for(pool, step).await?;
    require_webmcp(&page, &ua).await?;

    // `executeTool` takes its arguments as a JSON *string* and answers with
    // one, which the explainer's `executeTool(tool, inputs)` signature doesn't
    // convey — passing the object itself fails with "Failed to parse input
    // arguments". Established against Chrome 153 rather than assumed.
    let script = format!(
        "async () => {{
            const tools = await document.modelContext.getTools();
            const tool = tools.find((t) => t.name === {name});
            if (!tool) return {{ missing: true }};
            const answer = await document.modelContext.executeTool(tool, {args});
            let result = answer;
            if (typeof answer === 'string') {{
                try {{ result = JSON.parse(answer); }} catch {{ result = answer; }}
            }}
            return {{ result }};
        }}",
        name = js_literal(tool),
        args = js_literal(&arguments.to_string()),
    );
    let outcome: serde_json::Value = page
        .evaluate_function(script)
        .await
        .map_err(|e| external_failure(format!("calling WebMCP tool `{tool}`: {e}"), &ua))?
        .into_value()
        .unwrap_or(serde_json::Value::Null);

    if outcome.get("missing").and_then(|v| v.as_bool()) == Some(true) {
        return Err(golem_events::coded(
            FailureCode::FlowElementNotFound,
            anyhow!("this page registers no WebMCP tool named `{tool}` [browser: {ua}]"),
        ));
    }

    if let Some(var_name) = &step.save_to {
        let result = outcome.get("result").cloned().unwrap_or_default();
        vars.set_in_scope(ScopeLevel::Flow, var_name, mcp_result_to_var(result));
    }
    Ok(())
}

/// Fail early, and specifically, when the browser has no WebMCP.
///
/// The API ships switched off, so "undefined" is the normal state of a browser
/// golem didn't launch for this. Reported as a host problem: the flow is
/// valid, the browser can't serve it.
async fn require_webmcp(page: &Page, ua: &str) -> Result<()> {
    let present: bool = page
        .evaluate_expression("typeof document.modelContext !== 'undefined'")
        .await
        .ok()
        .and_then(|r| r.into_value().ok())
        .unwrap_or(false);
    if present {
        return Ok(());
    }
    Err(golem_events::coded(
        FailureCode::HostBrowserFeatureMissing,
        anyhow!(
            "this page has no WebMCP API (`document.modelContext`). It needs a browser \
             that supports it, and a secure origin — an https:// or localhost page, never \
             a data: URL. [browser: {ua}]"
        ),
    ))
}

/// Unwrap the `{ content: [{ type: "text", text }] }` envelope MCP tools return.
///
/// A flow wants the answer, not the envelope. Text parts are joined; a result
/// that is itself JSON nests, matching what reading storage does, so
/// `${result.order_id}` works either way.
fn mcp_result_to_var(result: serde_json::Value) -> VarValue {
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        });
    match text {
        Some(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value @ serde_json::Value::Object(_)) => json_to_var(value),
            _ => VarValue::string(text),
        },
        // Not an MCP envelope — hand back whatever the tool did return rather
        // than inventing an empty string.
        None => json_to_var(result),
    }
}

/// Step params are TOML; a WebMCP tool's arguments are JSON.
fn toml_to_json(value: &toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::from(*i),
        toml::Value::Float(f) => serde_json::Value::from(*f),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
        toml::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(toml_to_json).collect())
        }
        toml::Value::Table(table) => serde_json::Value::Object(
            table
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect(),
        ),
    }
}

/// Set a cookie for the current page.
///
/// Through CDP rather than `document.cookie`, which is the whole point: the
/// cookie a portal login hands out is usually `HttpOnly`, and script can
/// neither read nor write those. `domain` and `path` are optional because the
/// common case is "this cookie, for the page I'm on".
async fn set_cookie(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let name = required_param(step, "name")?;
    let value = required_param(step, "value")?;
    let (page, ua) = page_for(pool, step).await?;

    let mut cookie = CookieParam::new(name, value);
    cookie.domain = optional_param(step, "domain").map(str::to_string);
    cookie.path = optional_param(step, "path").map(str::to_string);
    page.set_cookie(cookie)
        .await
        .map_err(|e| external_failure(format!("setting cookie `{name}`: {e}"), &ua))?;
    Ok(())
}

/// Read a cookie visible to the current page into a variable.
async fn get_cookie(pool: &mut BrowserPool, step: &Step, vars: &mut VariableStore) -> Result<()> {
    let name = required_param(step, "name")?;
    let (page, ua) = page_for(pool, step).await?;

    let cookies = page
        .get_cookies()
        .await
        .map_err(|e| external_failure(format!("reading cookies: {e}"), &ua))?;
    let Some(cookie) = cookies.into_iter().find(|c| c.name == name) else {
        return Err(golem_events::coded(
            FailureCode::FlowElementNotFound,
            anyhow!("no cookie named `{name}` for this page [browser: {ua}]"),
        ));
    };

    if let Some(var_name) = &step.save_to {
        vars.set_in_scope(ScopeLevel::Flow, var_name, VarValue::string(&cookie.value));
    }
    Ok(())
}

/// Which web-storage area a step is talking about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Storage {
    Local,
    Session,
}

impl Storage {
    fn js(self) -> &'static str {
        match self {
            Self::Local => "localStorage",
            Self::Session => "sessionStorage",
        }
    }
}

async fn set_storage(pool: &mut BrowserPool, step: &Step, area: Storage) -> Result<()> {
    let key = required_param(step, "key")?;
    let value = required_param(step, "value")?;
    let (page, ua) = page_for(pool, step).await?;

    let script = format!(
        "async () => {{ {}.setItem({}, {}); }}",
        area.js(),
        js_literal(key),
        js_literal(value)
    );
    page.evaluate_function(script)
        .await
        .map_err(|e| external_failure(format!("writing {} `{key}`: {e}", area.js()), &ua))?;
    Ok(())
}

/// Read a storage key into a variable, parsing JSON objects as they go.
///
/// Web apps keep structured state in storage as JSON text, so a raw string
/// would force every flow to pick it apart by hand. An object nests for
/// `${session.user.id}`; anything else — an array, a number, plain text —
/// stays the text it was, because inventing an indexing dialect for arrays
/// would be a second thing to learn.
async fn get_storage(
    pool: &mut BrowserPool,
    step: &Step,
    vars: &mut VariableStore,
    area: Storage,
) -> Result<()> {
    let key = required_param(step, "key")?;
    let (page, ua) = page_for(pool, step).await?;

    let script = format!("async () => {}.getItem({})", area.js(), js_literal(key));
    let raw: Option<String> = page
        .evaluate_function(script)
        .await
        .map_err(|e| external_failure(format!("reading {} `{key}`: {e}", area.js()), &ua))?
        .into_value()
        .ok()
        .flatten();

    let Some(raw) = raw else {
        return Err(golem_events::coded(
            FailureCode::FlowElementNotFound,
            anyhow!("no `{key}` in {} for this page [browser: {ua}]", area.js()),
        ));
    };

    if let Some(var_name) = &step.save_to {
        let parsed = match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(value @ serde_json::Value::Object(_)) => json_to_var(value),
            _ => VarValue::string(&raw),
        };
        vars.set_in_scope(ScopeLevel::Flow, var_name, parsed);
    }
    Ok(())
}

/// A string as a JS literal — quotes, newlines and backslashes handled by the
/// JSON encoder rather than by hand.
fn js_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

/// Nudge the page, or one scrollable element, by a fixed distance.
///
/// Named for `scrollBy`, not for mobile's `scroll`. Mobile `scroll` keeps
/// swiping until an element appears — a search that has no meaning here, since
/// a CSS selector reaches an element whether or not it is on screen. Borrowing
/// the bare word would promise that search; `_by` and `_to` say plainly which
/// of the two jobs each action does.
async fn scroll_by(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let direction = Direction::from_step(step)?;
    let amount = scroll_amount(step)?;
    let (dx, dy) = direction.delta(amount);
    let (page, ua) = page_for(pool, step).await?;

    // `container` scrolls that element; without one the window moves. It is
    // deliberately not `selector`: every other browser action uses `selector`
    // for the element the step acts *on*, and here the element being scrolled
    // is scenery around the movement, not its subject.
    match optional_param(step, "container") {
        Some(container) => {
            let target = BrowserTarget {
                selector: container.to_string(),
                index: 0,
            };
            let element = find(&page, &target, find_timeout(step), &ua).await?;
            element
                .call_js_fn(
                    format!("function() {{ this.scrollBy({dx}, {dy}); return true; }}"),
                    false,
                )
                .await
                .map_err(|e| external_failure(format!("scrolling {target}: {e}"), &ua))?;
        }
        None => {
            page.evaluate_function(format!("async () => {{ window.scrollBy({dx}, {dy}); }}"))
                .await
                .map_err(|e| external_failure(format!("scrolling the page: {e}"), &ua))?;
        }
    }
    Ok(())
}

/// Bring an element into view.
async fn scroll_to(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let target = resolve_target(step)?;
    let (page, ua) = page_for(pool, step).await?;
    let element = find(&page, &target, find_timeout(step), &ua).await?;
    element
        .scroll_into_view()
        .await
        .map_err(|e| external_failure(format!("scrolling {target} into view: {e}"), &ua))?;
    Ok(())
}

/// Which way `browse_scroll` moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    /// Defaults to down, like the mobile `scroll` action — the same word should
    /// mean the same thing whichever tree a step is aimed at.
    fn from_step(step: &Step) -> Result<Self> {
        match optional_param(step, "direction") {
            None | Some("down") => Ok(Self::Down),
            Some("up") => Ok(Self::Up),
            Some("left") => Ok(Self::Left),
            Some("right") => Ok(Self::Right),
            Some(other) => Err(golem_events::coded(
                FailureCode::ParseMissingParam,
                anyhow!("unknown scroll direction `{other}` — expected up, down, left, or right"),
            )),
        }
    }

    fn delta(self, amount: i64) -> (i64, i64) {
        match self {
            Self::Down => (0, amount),
            Self::Up => (0, -amount),
            Self::Right => (amount, 0),
            Self::Left => (-amount, 0),
        }
    }
}

/// How far to scroll, in CSS pixels.
fn scroll_amount(step: &Step) -> Result<i64> {
    let Some(raw) = step.params.get("amount") else {
        return Ok(300);
    };
    let invalid = |detail: String| {
        golem_events::coded(
            FailureCode::ParseMissingParam,
            anyhow!("browser step `amount` {detail} — expected a positive number of pixels"),
        )
    };
    let n = raw
        .as_integer()
        .ok_or_else(|| invalid(format!("must be a number, got `{raw}`")))?;
    if n <= 0 {
        return Err(invalid(format!("must be positive, got `{n}`")));
    }
    Ok(n)
}

/// Run JavaScript in the page.
///
/// A file and an inline script can both be given, and the file runs first: it
/// is the natural home for reusable functions, and the inline script is then
/// the one-liner that calls one. They are concatenated into a single
/// evaluation rather than run as two — separate evaluations would rely on the
/// file's top-level declarations leaking into the realm, which `const` and
/// `let` don't reliably do.
///
/// Only the inline script sees golem variables. Step params are interpolated
/// before any handler runs, so `${order_id}` resolves there; the file is read
/// straight off disk, because a shared helper shouldn't silently change
/// meaning based on which flow imported it.
async fn execute_js(
    pool: &mut BrowserPool,
    step: &Step,
    vars: &mut VariableStore,
    paths: ScriptPaths<'_>,
) -> Result<()> {
    let inline = optional_param(step, "script");
    let file = optional_param(step, "file");
    if inline.is_none() && file.is_none() {
        return Err(golem_events::coded(
            FailureCode::ParseMissingParam,
            anyhow!("browse_execute_js requires a `script` param, a `file` param, or both"),
        ));
    }

    // Wrapped in an async arrow and run through `evaluate_function`, for three
    // reasons: a bare `function foo(){}` at the start of a script is otherwise
    // mistaken for the function to call; `await` works, which a portal that
    // fetches needs; and the value a step saves is whatever the script
    // `return`s, which is one rule rather than "the last expression, unless…".
    let mut source = String::from("async () => {\n");
    if let Some(file) = file {
        let path = resolve_script_file(file, paths)?;
        let contents = std::fs::read_to_string(&path).map_err(|e| {
            golem_events::coded(
                FailureCode::ParseMissingParam,
                anyhow!("reading browse_execute_js file {}: {e}", path.display()),
            )
        })?;
        source.push_str(&contents);
        source.push('\n');
    }
    if let Some(inline) = inline {
        source.push_str(inline);
    }
    source.push_str("\n}");

    let (page, ua) = page_for(pool, step).await?;
    let result = page
        .evaluate_function(source.as_str())
        .await
        .map_err(|e| external_failure(format!("evaluating script: {e}"), &ua))?;

    if let Some(var_name) = &step.save_to {
        let value: serde_json::Value = result.into_value().unwrap_or(serde_json::Value::Null);
        vars.set_in_scope(ScopeLevel::Flow, var_name, json_to_var(value));
    }
    Ok(())
}

/// Resolve a script file the way the `run` action resolves its scripts, so a
/// flow author has one rule to remember rather than one per action.
fn resolve_script_file(file: &str, paths: ScriptPaths<'_>) -> Result<PathBuf> {
    if file.contains("..") {
        return Err(golem_events::coded(
            FailureCode::ParseMissingParam,
            anyhow!("browse_execute_js: path traversal ('..') is not allowed in `file`"),
        ));
    }
    Ok(if let Some(rooted) = file.strip_prefix('/') {
        paths.project_root.join(rooted)
    } else {
        paths.flow_dir.join(file)
    })
}

/// Flatten a JSON result into the variable store's two shapes.
///
/// Objects nest so `${result.total}` works; everything else becomes the text a
/// flow would compare against. Arrays keep their JSON form rather than becoming
/// index-keyed objects — a `${rows.0}` that only worked for arrays would be a
/// second indexing dialect to learn.
fn json_to_var(value: serde_json::Value) -> VarValue {
    match value {
        serde_json::Value::Object(map) => VarValue::Object(
            map.into_iter()
                .map(|(k, v)| (k, json_to_var(v)))
                .collect::<std::collections::HashMap<_, _>>(),
        ),
        serde_json::Value::String(s) => VarValue::string(s),
        other => VarValue::string(other.to_string()),
    }
}

/// Choose an option in a `<select>`.
///
/// Sets the value and fires `input` + `change` the way a user's choice would:
/// frameworks listen for those events, and a select whose value changed without
/// them leaves the page's own state stale.
async fn select(pool: &mut BrowserPool, step: &Step) -> Result<()> {
    let (wanted, by_text) = match (optional_param(step, "value"), optional_param(step, "text")) {
        (Some(value), None) => (value, false),
        (None, Some(text)) => (text, true),
        (Some(_), Some(_)) => {
            return Err(golem_events::coded(
                FailureCode::ParseMissingParam,
                anyhow!("browse_select takes `value` or `text`, not both"),
            ))
        }
        (None, None) => {
            return Err(golem_events::coded(
                FailureCode::ParseMissingParam,
                anyhow!("browse_select requires a `value` or `text` param naming the option"),
            ))
        }
    };
    let target = resolve_target(step)?;
    let (page, ua) = page_for(pool, step).await?;
    let element = find(&page, &target, find_timeout(step), &ua).await?;

    // The wanted value is inlined as a JSON literal because `call_js_fn` takes
    // no arguments — JSON encoding is what makes an option label containing a
    // quote or a newline safe to embed.
    let wanted_literal = js_literal(wanted);
    let script = format!(
        "function() {{
            const wanted = {wanted_literal};
            const byText = {by_text};
            const option = Array.from(this.options).find(
                (o) => (byText ? o.text : o.value) === wanted
            );
            if (!option) return false;
            this.value = option.value;
            this.dispatchEvent(new Event('input', {{ bubbles: true }}));
            this.dispatchEvent(new Event('change', {{ bubbles: true }}));
            return true;
        }}"
    );
    let chosen = element
        .call_js_fn(script, false)
        .await
        .map_err(|e| external_failure(format!("selecting in {target}: {e}"), &ua))?
        .result
        .value
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !chosen {
        let how = if by_text { "text" } else { "value" };
        return Err(golem_events::coded(
            FailureCode::FlowElementNotFound,
            anyhow!("{target} has no option with {how} {wanted:?} [browser: {ua}]"),
        ));
    }
    Ok(())
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
    let (page, mut ua) = page_for(pool, step).await?;

    // Identity is applied before the navigation, so the very first request
    // carries it — a portal that serves a different page to a phone decides
    // that on the request, not afterwards.
    if let Some(applied) = apply_identity(pool, &page, step, &ua).await? {
        ua = applied;
    }

    page.goto(url)
        .await
        .map_err(|e| external_failure(format!("navigating to {url}: {e}"), &ua))?;
    wait.settle(&page, &ua).await
}

/// Give the tab a user agent and/or an accept-language, if the step asks.
///
/// The override belongs to the tab, not the request: CDP keeps it until
/// something changes it, which is what a flow wants — a session that is a phone
/// stays a phone. Returns the agent now in effect so a failure quotes what the
/// site saw rather than the browser's own identity.
async fn apply_identity(
    pool: &mut BrowserPool,
    page: &Page,
    step: &Step,
    current_ua: &str,
) -> Result<Option<String>> {
    let wanted_ua = optional_param(step, "user_agent");
    let language = optional_param(step, "accept_language");
    if wanted_ua.is_none() && language.is_none() {
        return Ok(None);
    }

    // CDP has no "language only" call — the user agent is a required field —
    // so a step setting just the language keeps the agent the tab already has.
    let effective = wanted_ua.unwrap_or(current_ua);
    let mut params = SetUserAgentOverrideParams::new(effective);
    params.accept_language = language.map(str::to_string);

    page.set_user_agent(params)
        .await
        .map_err(|e| external_failure(format!("setting the user agent: {e}"), current_ua))?;

    let session = crate::session::parse_session(optional_param(step, "session"))?;
    pool.note_user_agent(&session, effective);
    Ok(Some(effective.to_string()))
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
    let ua = pool
        .session_user_agent(optional_param(step, "session"))
        .await?;
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

    /// A one-page HTTP server on an ephemeral port, returning its base URL.
    ///
    /// Cookies and web storage need a real origin. `about:blank` and `data:`
    /// URLs get an opaque one, where `localStorage` throws and a cookie has
    /// nowhere to live — so these tests serve the page for real rather than
    /// writing it into the tab.
    struct TestServer {
        url: String,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl TestServer {
        /// A page that shows the request headers the browser sent.
        ///
        /// `navigator.userAgent` only proves what the page can read; a portal
        /// decides what to serve from the request, so the request is what a
        /// test about identity has to look at.
        fn echoing() -> Self {
            Self::start_with(None)
        }

        fn start(html: &'static str) -> Self {
            Self::start_with(Some(html))
        }

        fn start_with(html: Option<&'static str>) -> Self {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("SHALL bind a port");
            let port = listener.local_addr().expect("SHALL have an address").port();
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = stop.clone();

            let thread = std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if flag.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let Ok(mut stream) = stream else { continue };
                    let mut buf = [0u8; 4096];
                    let read = stream.read(&mut buf).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..read]).to_string();
                    let body = match html {
                        Some(html) => html.to_string(),
                        None => {
                            let header = |name: &str| {
                                request
                                    .lines()
                                    .find(|l| l.to_lowercase().starts_with(name))
                                    .and_then(|l| l.split_once(':'))
                                    .map(|(_, v)| v.trim().to_string())
                                    .unwrap_or_default()
                            };
                            format!(
                                "<span id='ua'>{}</span><span id='lang'>{}</span>",
                                header("user-agent"),
                                header("accept-language"),
                            )
                        }
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            });

            Self {
                url: format!("http://127.0.0.1:{port}/"),
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for TestServer {
        /// Unblock the accept loop with one throwaway connection and join, so
        /// no thread outlives the test — nextest reports a lingering one as a
        /// leak, and a leak report that means nothing trains people to ignore
        /// the ones that do.
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let addr = self.url.trim_start_matches("http://").trim_end_matches('/');
            let _ = std::net::TcpStream::connect(addr);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
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
        run_from(pool, s, v, Path::new(".")).await
    }

    /// Same, with a directory for `file` params to resolve against.
    async fn run_from(
        pool: &mut BrowserPool,
        s: &Step,
        v: &mut VariableStore,
        dir: &Path,
    ) -> Result<()> {
        execute_browser_action(
            pool,
            s,
            v,
            ScriptPaths {
                flow_dir: dir,
                project_root: dir,
            },
        )
        .await
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
            ("execute_js with neither script nor file", "action = \"browse_execute_js\""),
            (
                "scroll in a direction that doesn't exist",
                "action = \"browse_scroll_by\"\ndirection = \"sideways\"",
            ),
            (
                "scroll by a negative distance",
                "action = \"browse_scroll_by\"\namount = -100",
            ),
            ("scroll_to without a selector", "action = \"browse_scroll_to\""),
            ("set_cookie without a name", "action = \"browse_set_cookie\""),
            (
                "set_cookie without a value",
                "action = \"browse_set_cookie\"\nname = \"session\"",
            ),
            ("get_cookie without a name", "action = \"browse_get_cookie\""),
            ("mcp_call without a tool", "action = \"browse_mcp_call\""),
            (
                "set_local_storage without a key",
                "action = \"browse_set_local_storage\"\nvalue = \"x\"",
            ),
            (
                "get_session_storage without a key",
                "action = \"browse_get_session_storage\"",
            ),
            (
                "select without an option",
                "action = \"browse_select\"\nselector = \"#status\"",
            ),
            (
                "select given both ways to name an option",
                "action = \"browse_select\"\nselector = \"#status\"\nvalue = \"a\"\ntext = \"A\"",
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

    // 5a. Scroll defaults match the mobile action a reader already knows, and
    //     each direction moves the axis it names.
    #[test]
    fn scroll_by_defaults_to_300px_down() {
        let bare = step(r#"action = "browse_scroll_by""#);
        assert_eq!(scroll_amount(&bare).expect("default"), 300);
        assert_eq!(
            Direction::from_step(&bare).expect("default"),
            Direction::Down
        );

        assert_eq!(Direction::Down.delta(300), (0, 300));
        assert_eq!(Direction::Up.delta(300), (0, -300));
        assert_eq!(Direction::Right.delta(300), (300, 0));
        assert_eq!(Direction::Left.delta(300), (-300, 0));
    }

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

    // 13. JavaScript: an inline script's result lands in a variable, a file
    //     runs first so the inline script can call what it declared, and an
    //     object result nests for `${var.field}` access.
    #[tokio::test]
    async fn live_execute_js_runs_inline_scripts_and_files() {
        if !chrome_available() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir SHALL be created");
        std::fs::write(
            dir.path().join("helpers.js"),
            "function orderTotal() { return { total: 1499, currency: 'GBP' } }",
        )
        .expect("helper SHALL be written");

        let mut p = pool();
        let mut v = vars();
        page_with(&mut p, "<h1>Portal</h1>").await;

        run(
            &mut p,
            &step(
                "action = \"browse_execute_js\"\nscript = \"document.title = 'Orders'; return document.title\"\nsave_to = \"title\"",
            ),
            &mut v,
        )
        .await
        .expect("an inline script SHALL run");
        assert_eq!(saved(&v, "title"), "Orders");

        run_from(
            &mut p,
            &step(
                "action = \"browse_execute_js\"\nfile = \"helpers.js\"\nscript = \"return orderTotal()\"\nsave_to = \"order\"",
            ),
            &mut v,
            dir.path(),
        )
        .await
        .expect("the inline script SHALL see what the file declared");
        match v.get("order") {
            Some(VarValue::Object(fields)) => {
                assert_eq!(fields.get("total"), Some(&VarValue::string("1499")));
                assert_eq!(fields.get("currency"), Some(&VarValue::string("GBP")));
            }
            other => panic!("an object result SHALL nest, got {other:?}"),
        }

        p.close().await.expect("close SHALL succeed");
    }

    // 14. A `file` that escapes the flow directory is refused before anything
    //     is read — the same rule the `run` action applies to its scripts.
    #[tokio::test]
    async fn execute_js_rejects_path_traversal() {
        let mut p = pool();
        let e = run(
            &mut p,
            &step("action = \"browse_execute_js\"\nfile = \"../secrets.js\""),
            &mut vars(),
        )
        .await
        .expect_err("traversal SHALL be refused");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::ParseMissingParam)
        );
        assert!(!p.is_running(), "a refused path SHALL NOT launch a browser");
    }

    // 15. A select can be driven by option value or by visible label, the page
    //     sees the `change` event either way, and an option that isn't there
    //     fails rather than silently leaving the old value.
    #[tokio::test]
    async fn live_select_chooses_by_value_or_label() {
        if !chrome_available() {
            return;
        }
        let mut p = pool();
        let mut v = vars();
        page_with(
            &mut p,
            "<select id='status' onchange=\"document.getElementById('seen').textContent = this.value\">\
               <option value='pending'>Pending</option>\
               <option value='fulfilled'>Fulfilled</option>\
             </select><span id='seen'></span>",
        )
        .await;

        run(
            &mut p,
            &step("action = \"browse_select\"\nselector = \"#status\"\nvalue = \"fulfilled\""),
            &mut v,
        )
        .await
        .expect("selecting by value SHALL work");
        run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \"#seen\"\nsave_to = \"seen\""),
            &mut v,
        )
        .await
        .expect("read SHALL succeed");
        assert_eq!(
            saved(&v, "seen"),
            "fulfilled",
            "the page's change handler SHALL see the new value"
        );

        run(
            &mut p,
            &step("action = \"browse_select\"\nselector = \"#status\"\ntext = \"Pending\""),
            &mut v,
        )
        .await
        .expect("selecting by label SHALL work");

        let e = run(
            &mut p,
            &step("action = \"browse_select\"\nselector = \"#status\"\nvalue = \"cancelled\""),
            &mut v,
        )
        .await
        .expect_err("an option that isn't there SHALL fail");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::FlowElementNotFound)
        );

        p.close().await.expect("close SHALL succeed");
    }

    // 16. Scrolling moves the window and a container independently, and
    //     `browse_scroll_to` brings a far-down element into view.
    #[tokio::test]
    async fn live_scroll_moves_the_page_and_a_container() {
        if !chrome_available() {
            return;
        }
        let mut p = pool();
        let mut v = vars();
        page_with(
            &mut p,
            "<div id='box' style='height:100px;overflow:auto'>\
               <div style='height:2000px'></div></div>\
             <div style='height:3000px'></div>\
             <p id='bottom'>bottom</p>",
        )
        .await;

        run(
            &mut p,
            &step("action = \"browse_scroll_by\"\namount = 500"),
            &mut v,
        )
        .await
        .expect("the page SHALL scroll");
        run(
            &mut p,
            &step("action = \"browse_execute_js\"\nscript = \"return String(window.scrollY)\"\nsave_to = \"y\""),
            &mut v,
        )
        .await
        .expect("reading scrollY SHALL work");
        assert_eq!(saved(&v, "y"), "500", "the window SHALL have moved");

        run(
            &mut p,
            &step("action = \"browse_scroll_by\"\ncontainer = \"#box\"\namount = 250"),
            &mut v,
        )
        .await
        .expect("a container SHALL scroll");
        run(
            &mut p,
            &step(
                "action = \"browse_execute_js\"\nscript = \"return String(document.getElementById('box').scrollTop)\"\nsave_to = \"box_y\"",
            ),
            &mut v,
        )
        .await
        .expect("reading scrollTop SHALL work");
        assert_eq!(
            saved(&v, "box_y"),
            "250",
            "the container SHALL have moved on its own"
        );

        run(
            &mut p,
            &step("action = \"browse_scroll_by\"\ndirection = \"up\"\namount = 500"),
            &mut v,
        )
        .await
        .expect("scrolling back up SHALL work");
        run(
            &mut p,
            &step("action = \"browse_execute_js\"\nscript = \"return String(window.scrollY)\"\nsave_to = \"y2\"",),
            &mut v,
        )
        .await
        .expect("reading scrollY SHALL work");
        assert_eq!(saved(&v, "y2"), "0", "up SHALL undo down");

        run(
            &mut p,
            &step("action = \"browse_scroll_to\"\nselector = \"#bottom\""),
            &mut v,
        )
        .await
        .expect("scroll_to SHALL reach a far-down element");
        run(
            &mut p,
            &step("action = \"browse_execute_js\"\nscript = \"return String(window.scrollY > 0)\"\nsave_to = \"moved\"",),
            &mut v,
        )
        .await
        .expect("reading scrollY SHALL work");
        assert_eq!(
            saved(&v, "moved"),
            "true",
            "scroll_to SHALL have moved the page"
        );

        p.close().await.expect("close SHALL succeed");
    }

    // 17. Cookies and storage, against a real origin. A cookie round-trips
    //     through CDP; storage round-trips per area; a JSON object nests for
    //     dot-path access while plain text stays text; and asking for
    //     something that was never set fails rather than saving nothing.
    #[tokio::test]
    async fn live_cookies_and_storage_round_trip() {
        if !chrome_available() {
            return;
        }
        let server = TestServer::start("<h1>Portal</h1>");
        let mut p = pool();
        let mut v = vars();
        run(
            &mut p,
            &step(&format!(
                "action = \"browse_navigate\"\nurl = \"{}\"",
                server.url
            )),
            &mut v,
        )
        .await
        .expect("the served page SHALL load");

        run(
            &mut p,
            &step("action = \"browse_set_cookie\"\nname = \"session\"\nvalue = \"abc123\""),
            &mut v,
        )
        .await
        .expect("setting a cookie SHALL work");
        run(
            &mut p,
            &step("action = \"browse_get_cookie\"\nname = \"session\"\nsave_to = \"session\""),
            &mut v,
        )
        .await
        .expect("reading it back SHALL work");
        assert_eq!(saved(&v, "session"), "abc123");

        for (set, get, area) in [
            (
                "browse_set_local_storage",
                "browse_get_local_storage",
                "local",
            ),
            (
                "browse_set_session_storage",
                "browse_get_session_storage",
                "session",
            ),
        ] {
            run(
                &mut p,
                &step(&format!(
                    "action = \"{set}\"\nkey = \"where\"\nvalue = \"{area}\""
                )),
                &mut v,
            )
            .await
            .unwrap_or_else(|e| panic!("writing {area} storage SHALL work: {e:#}"));
            run(
                &mut p,
                &step(&format!(
                    "action = \"{get}\"\nkey = \"where\"\nsave_to = \"where_{area}\""
                )),
                &mut v,
            )
            .await
            .unwrap_or_else(|e| panic!("reading {area} storage SHALL work: {e:#}"));
            assert_eq!(saved(&v, &format!("where_{area}")), area);
        }

        // JSON objects nest so a flow can reach into them.
        run(
            &mut p,
            &step(
                "action = \"browse_set_local_storage\"\nkey = \"user\"\nvalue = \"{\\\"id\\\": \\\"u-7\\\", \\\"plan\\\": \\\"pro\\\"}\"",
            ),
            &mut v,
        )
        .await
        .expect("writing JSON SHALL work");
        run(
            &mut p,
            &step("action = \"browse_get_local_storage\"\nkey = \"user\"\nsave_to = \"user\""),
            &mut v,
        )
        .await
        .expect("reading JSON SHALL work");
        match v.get("user") {
            Some(VarValue::Object(fields)) => {
                assert_eq!(fields.get("id"), Some(&VarValue::string("u-7")));
                assert_eq!(fields.get("plan"), Some(&VarValue::string("pro")));
            }
            other => panic!("a JSON object SHALL nest, got {other:?}"),
        }

        let e = run(
            &mut p,
            &step("action = \"browse_get_cookie\"\nname = \"absent\"\nsave_to = \"nope\""),
            &mut v,
        )
        .await
        .expect_err("an unset cookie SHALL fail");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::FlowElementNotFound)
        );
        let e = run(
            &mut p,
            &step("action = \"browse_get_local_storage\"\nkey = \"absent\"\nsave_to = \"nope\""),
            &mut v,
        )
        .await
        .expect_err("an unset storage key SHALL fail");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::FlowElementNotFound)
        );
        assert!(v.get("nope").is_none(), "nothing SHALL be saved on failure");

        p.close().await.expect("close SHALL succeed");
    }

    // 17b. Two contexts on the same origin keep separate cookie jars, while
    //      two tabs in one context share theirs. That split is the whole point
    //      of contexts: the same site logged in as two different users at once
    //      cannot be done with tabs.
    #[tokio::test]
    async fn live_contexts_isolate_cookies_while_tabs_share_them() {
        if !chrome_available() {
            return;
        }
        let server = TestServer::start("<h1>Portal</h1>");
        let mut p = pool();
        let mut v = vars();

        let visit = |session: &str| {
            format!(
                "action = \"browse_navigate\"\nurl = \"{}\"\nsession = \"{session}\"",
                server.url
            )
        };

        // Tenant A signs in.
        run(&mut p, &step(&visit("tenantA:main")), &mut v)
            .await
            .expect("tenant A SHALL load the page");
        run(
            &mut p,
            &step(
                "action = \"browse_set_cookie\"\nname = \"who\"\nvalue = \"userX\"\nsession = \"tenantA:main\"",
            ),
            &mut v,
        )
        .await
        .expect("tenant A SHALL get a cookie");

        // A second tab in the SAME context sees it — tabs share a jar.
        run(&mut p, &step(&visit("tenantA:second")), &mut v)
            .await
            .expect("a second tab SHALL load");
        run(
            &mut p,
            &step(
                "action = \"browse_get_cookie\"\nname = \"who\"\nsession = \"tenantA:second\"\nsave_to = \"shared\"",
            ),
            &mut v,
        )
        .await
        .expect("a sibling tab SHALL see the cookie");
        assert_eq!(saved(&v, "shared"), "userX");

        // A tab in a DIFFERENT context does not.
        run(&mut p, &step(&visit("tenantB:main")), &mut v)
            .await
            .expect("tenant B SHALL load the page");
        let e = run(
            &mut p,
            &step(
                "action = \"browse_get_cookie\"\nname = \"who\"\nsession = \"tenantB:main\"\nsave_to = \"leaked\"",
            ),
            &mut v,
        )
        .await
        .expect_err("a separate context SHALL NOT see tenant A's cookie");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::FlowElementNotFound)
        );
        assert!(v.get("leaked").is_none(), "nothing SHALL leak between jars");

        // Tenant B signs in as someone else, and tenant A is unaffected.
        run(
            &mut p,
            &step(
                "action = \"browse_set_cookie\"\nname = \"who\"\nvalue = \"userY\"\nsession = \"tenantB:main\"",
            ),
            &mut v,
        )
        .await
        .expect("tenant B SHALL get its own cookie");
        run(
            &mut p,
            &step(
                "action = \"browse_get_cookie\"\nname = \"who\"\nsession = \"tenantA:main\"\nsave_to = \"a_after\"",
            ),
            &mut v,
        )
        .await
        .expect("tenant A's cookie SHALL still be there");
        assert_eq!(
            saved(&v, "a_after"),
            "userX",
            "one tenant's login SHALL NOT overwrite another's"
        );

        p.close().await.expect("close SHALL succeed");
    }

    // 17c. A session can present itself as a different device, and the portal
    //      sees it in the request rather than only in `navigator.userAgent` —
    //      which is what decides whether a site serves its mobile variant.
    //      Two sessions carry different identities at the same time.
    #[tokio::test]
    async fn live_sessions_carry_their_own_user_agent() {
        if !chrome_available() {
            return;
        }
        let server = TestServer::echoing();
        let mut p = pool();
        let mut v = vars();
        const IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) \
                              AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";

        run(
            &mut p,
            &step(&format!(
                "action = \"browse_navigate\"\nurl = \"{}\"\nsession = \"phone\"\nuser_agent = \"{IPHONE}\"\naccept_language = \"fr-FR\"",
                server.url
            )),
            &mut v,
        )
        .await
        .expect("the phone session SHALL load");
        run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \"#ua\"\nsession = \"phone\"\nsave_to = \"phone_ua\""),
            &mut v,
        )
        .await
        .expect("read SHALL succeed");
        run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \"#lang\"\nsession = \"phone\"\nsave_to = \"phone_lang\""),
            &mut v,
        )
        .await
        .expect("read SHALL succeed");
        assert!(
            saved(&v, "phone_ua").contains("iPhone"),
            "the request SHALL carry the override, got {}",
            saved(&v, "phone_ua")
        );
        assert_eq!(saved(&v, "phone_lang"), "fr-FR");

        // A second session, untouched, is still the real browser.
        run(
            &mut p,
            &step(&format!(
                "action = \"browse_navigate\"\nurl = \"{}\"\nsession = \"desktop\"",
                server.url
            )),
            &mut v,
        )
        .await
        .expect("the desktop session SHALL load");
        run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \"#ua\"\nsession = \"desktop\"\nsave_to = \"desktop_ua\""),
            &mut v,
        )
        .await
        .expect("read SHALL succeed");
        assert!(
            !saved(&v, "desktop_ua").contains("iPhone"),
            "one session's identity SHALL NOT leak into another: {}",
            saved(&v, "desktop_ua")
        );

        // The override sticks to the tab: a later navigation keeps it without
        // repeating the param, which is how a session stays one device.
        run(
            &mut p,
            &step(&format!(
                "action = \"browse_navigate\"\nurl = \"{}\"\nsession = \"phone\"",
                server.url
            )),
            &mut v,
        )
        .await
        .expect("the phone session SHALL reload");
        run(
            &mut p,
            &step("action = \"browse_read\"\nselector = \"#ua\"\nsession = \"phone\"\nsave_to = \"phone_again\""),
            &mut v,
        )
        .await
        .expect("read SHALL succeed");
        assert!(
            saved(&v, "phone_again").contains("iPhone"),
            "the identity SHALL persist across navigations: {}",
            saved(&v, "phone_again")
        );

        p.close().await.expect("close SHALL succeed");
    }

    // 18. WebMCP: a page registers a tool, golem lists it and calls it, the
    //     `{content:[...]}` envelope is unwrapped, a JSON answer nests, and a
    //     tool the page never registered fails as not found.
    #[tokio::test]
    async fn live_webmcp_lists_and_calls_page_tools() {
        if !chrome_available() {
            return;
        }
        // A secure origin: WebMCP is absent on `data:` and `about:blank`.
        let server = TestServer::start(
            "<body><script>
               document.modelContext.registerTool({
                 name: 'fulfil_order',
                 description: 'Mark an order fulfilled',
                 inputSchema: { type: 'object', properties: { order_id: { type: 'string' } } },
                 execute: async ({ order_id }) => ({
                   content: [{ type: 'text', text: JSON.stringify({ order_id, status: 'fulfilled' }) }],
                 }),
               });
             </script></body>",
        );
        let mut p = BrowserPool::new(PoolConfig {
            headless: true,
            webmcp: true,
        });
        let mut v = vars();
        run(
            &mut p,
            &step(&format!(
                "action = \"browse_navigate\"\nurl = \"{}\"",
                server.url
            )),
            &mut v,
        )
        .await
        .expect("the served page SHALL load");

        run(
            &mut p,
            &step("action = \"browse_mcp_list_tools\"\nsave_to = \"tools\""),
            &mut v,
        )
        .await
        .expect("listing tools SHALL work");
        match v.get("tools") {
            Some(VarValue::Object(tools)) => assert_eq!(
                tools.get("fulfil_order"),
                Some(&VarValue::string("Mark an order fulfilled")),
                "tools SHALL be keyed by name"
            ),
            other => panic!("tools SHALL be an object, got {other:?}"),
        }

        run(
            &mut p,
            &step(
                "action = \"browse_mcp_call\"\ntool = \"fulfil_order\"\narguments = { order_id = \"o-42\" }\nsave_to = \"receipt\"",
            ),
            &mut v,
        )
        .await
        .expect("calling a tool SHALL work");
        match v.get("receipt") {
            Some(VarValue::Object(fields)) => {
                assert_eq!(fields.get("order_id"), Some(&VarValue::string("o-42")));
                assert_eq!(fields.get("status"), Some(&VarValue::string("fulfilled")));
            }
            other => panic!("the envelope SHALL be unwrapped and JSON nested, got {other:?}"),
        }

        let e = run(
            &mut p,
            &step("action = \"browse_mcp_call\"\ntool = \"refund_order\""),
            &mut v,
        )
        .await
        .expect_err("an unregistered tool SHALL fail");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::FlowElementNotFound)
        );

        p.close().await.expect("close SHALL succeed");
    }

    // 19. Without the browser feature there is no WebMCP, and saying so as a
    //     host problem is the difference between "upgrade your browser" and a
    //     flow author hunting a bug that isn't theirs.
    #[tokio::test]
    async fn live_webmcp_absent_is_a_host_failure() {
        if !chrome_available() {
            return;
        }
        let server = TestServer::start("<h1>No tools here</h1>");
        // Default config: the WebMCP switch is off, as it is for any flow that
        // doesn't use `browse_mcp_*`.
        let mut p = pool();
        let mut v = vars();
        run(
            &mut p,
            &step(&format!(
                "action = \"browse_navigate\"\nurl = \"{}\"",
                server.url
            )),
            &mut v,
        )
        .await
        .expect("the served page SHALL load");

        let e = run(
            &mut p,
            &step("action = \"browse_mcp_list_tools\"\nsave_to = \"tools\""),
            &mut v,
        )
        .await
        .expect_err("WebMCP SHALL be reported as missing");
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::HostBrowserFeatureMissing)
        );
        assert!(
            format!("{e:#}").contains("secure origin"),
            "the message SHALL mention the other reason it can be absent"
        );

        p.close().await.expect("close SHALL succeed");
    }

    // 20. Diagnostics: a missing element fails as not-found once the step's
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
