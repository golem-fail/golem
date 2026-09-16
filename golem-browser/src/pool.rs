use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use chromiumoxide::browser::BrowserConfigBuilder;
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use tempfile::TempDir;
use tokio::task::JoinHandle;

use crate::session::parse_session;

/// How a flow's browser is launched.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Run without a visible window. Headed is for watching a flow drive the
    /// portal by hand; CI always wants headless.
    pub headless: bool,
    /// Turn on Chrome's WebMCP API, which ships switched off.
    ///
    /// Only set for flows that actually use `browse_mcp_*`: enabling an
    /// experimental Blink feature changes what a page can feature-detect, and
    /// a portal that behaves differently under it would be a strange thing to
    /// inflict on flows that never asked.
    pub webmcp: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            headless: true,
            webmcp: false,
        }
    }
}

/// What a tab looked like at a moment worth recording.
///
/// The `html` is trimmed: a post-mortem wants to see what the page was showing,
/// and an un-capped dump of a real portal would bury that in framework markup
/// while filling the results directory.
#[derive(Debug, Clone)]
pub struct TabCapture {
    pub png: Vec<u8>,
    pub url: String,
    pub title: String,
    pub html: String,
}

/// How much `outerHTML` a capture keeps.
const HTML_BUDGET: usize = 8 * 1024;

/// One flow's browser: at most one Chrome process, the contexts inside it, and
/// the named tabs inside those.
///
/// Scoped per flowrun rather than per suite. Concurrent flows must never share
/// cookies or storage, and the cheapest way to guarantee that is to give each
/// its own browser — one Chrome per browser-using flow is the accepted cost.
///
/// Launch is lazy: constructing a pool for every flow costs nothing, so the
/// runner can hold one unconditionally and only pay when a `browse_*` step
/// actually runs.
pub struct BrowserPool {
    config: PoolConfig,
    running: Option<Running>,
}

struct Running {
    browser: Browser,
    /// Drains the CDP event stream. The browser is inert without it — drop
    /// this task and every later command hangs — so the pool owns it for the
    /// browser's whole life and aborts it only in `close`.
    handler: JoinHandle<()>,
    user_agent: String,
    /// context name → session name → tab. Only the default context exists in
    /// v1 (see [`crate::session::parse_session`]), but the map is the shape
    /// #109 needs, so adding contexts won't move the tabs around.
    contexts: HashMap<String, HashMap<String, Page>>,
    /// This browser's own Chrome profile, removed when the pool drops. Held
    /// only to keep the directory alive for the process's lifetime.
    _profile: TempDir,
}

impl BrowserPool {
    pub fn new(config: PoolConfig) -> Self {
        Self {
            config,
            running: None,
        }
    }

    /// The tab a step should act on, creating it (and the browser) on first use.
    pub async fn get_or_create_session(&mut self, session: Option<&str>) -> Result<Page> {
        let session = parse_session(session)?;
        let running = self.ensure_running().await?;

        if let Some(page) = running
            .contexts
            .get(&session.context)
            .and_then(|tabs| tabs.get(&session.session))
        {
            return Ok(page.clone());
        }

        let page = running
            .browser
            .new_page("about:blank")
            .await
            .with_context(|| format!("opening browser tab `{}`", session.session))?;
        running
            .contexts
            .entry(session.context)
            .or_default()
            .insert(session.session, page.clone());
        Ok(page)
    }

    /// Photograph an already-open session, for a failure or a trace boundary.
    ///
    /// Never opens a tab: a step that failed before reaching the browser has
    /// nothing to photograph, and creating a blank one to satisfy the capture
    /// would file a picture of nothing as evidence.
    pub async fn capture_session(&mut self, session: Option<&str>) -> Result<Option<TabCapture>> {
        let session = parse_session(session)?;
        let Some(page) = self.running.as_ref().and_then(|running| {
            running
                .contexts
                .get(&session.context)
                .and_then(|tabs| tabs.get(&session.session))
                .cloned()
        }) else {
            return Ok(None);
        };

        let png = page
            .screenshot(chromiumoxide::page::ScreenshotParams::builder().build())
            .await
            .map_err(|e| anyhow!("capturing the browser tab: {e}"))?;
        let url = page.url().await.ok().flatten().unwrap_or_default();
        let title = page.get_title().await.ok().flatten().unwrap_or_default();
        let mut html = page.content().await.unwrap_or_default();
        html.truncate(
            html.char_indices()
                .nth(HTML_BUDGET)
                .map_or(html.len(), |(i, _)| i),
        );

        Ok(Some(TabCapture {
            png,
            url,
            title,
            html,
        }))
    }

    /// Close one named tab, leaving the browser (and every other tab) alone.
    ///
    /// Backs `browse_close`, which exists so a long flow can hand back a tab it
    /// is finished with. Closing a session that was never opened is a no-op —
    /// the caller's intent (this tab should not be open) already holds.
    pub async fn close_session(&mut self, session: Option<&str>) -> Result<()> {
        let session = parse_session(session)?;
        let Some(running) = self.running.as_mut() else {
            return Ok(());
        };
        let Some(page) = running
            .contexts
            .get_mut(&session.context)
            .and_then(|tabs| tabs.remove(&session.session))
        else {
            return Ok(());
        };
        page.close()
            .await
            .map_err(|e| anyhow!("closing browser tab `{}`: {e}", session.session))
    }

    /// The launched browser's user-agent string.
    pub async fn user_agent(&mut self) -> Result<&str> {
        Ok(&self.ensure_running().await?.user_agent)
    }

    /// Whether a browser has actually been launched yet.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Tear down: tabs, then the incognito context, then the browser process,
    /// then the event-loop task.
    ///
    /// Every step runs even if an earlier one failed — a half-closed browser
    /// leaks a Chrome process, and the first error is more useful to a caller
    /// than an early return that skips the kill.
    pub async fn close(&mut self) -> Result<()> {
        let Some(mut running) = self.running.take() else {
            return Ok(());
        };
        let mut first_err: Option<anyhow::Error> = None;
        let mut record = |e: anyhow::Error| {
            if first_err.is_none() {
                first_err = Some(e);
            }
        };

        for (_, tabs) in running.contexts.drain() {
            for (name, page) in tabs {
                if let Err(e) = page.close().await {
                    record(anyhow!("closing browser tab `{name}`: {e}"));
                }
            }
        }
        if let Err(e) = running.browser.quit_incognito_context().await {
            record(anyhow!("disposing browser context: {e}"));
        }
        if let Err(e) = running.browser.close().await {
            record(anyhow!("closing browser: {e}"));
        }
        if let Err(e) = running.browser.wait().await {
            record(anyhow!("waiting for browser exit: {e}"));
        }
        running.handler.abort();

        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    async fn ensure_running(&mut self) -> Result<&mut Running> {
        if self.running.is_none() {
            self.running = Some(launch(&self.config).await?);
        }
        self.running
            .as_mut()
            .ok_or_else(|| anyhow!("browser pool lost its browser immediately after launching it"))
    }
}

impl Drop for BrowserPool {
    fn drop(&mut self) {
        // Only the event-loop task needs explicit help: the browser child is
        // spawned `kill_on_drop`, so dropping it is enough to reap Chrome, but
        // an orphaned handler task would linger for the process's lifetime.
        if let Some(running) = self.running.take() {
            running.handler.abort();
        }
    }
}

/// Switch on the browser's WebMCP API for this profile.
///
/// Written into the profile's `Local State` — the same place chrome://flags
/// records an enabled experiment — rather than relying on the command line
/// alone. chromiumoxide already passes `--enable-features=NetworkService,…`,
/// Chrome honours only the *last* `--enable-features` switch, and chromiumoxide
/// emits its arguments from a `HashMap`, so a second switch wins or loses on
/// iteration order. That is a coin flip per launch, and a flaky browser feature
/// is worse than an absent one.
///
/// The switch is passed too, carrying chromiumoxide's own values so that
/// whichever ends up last the feature set is a superset. Belt and braces: the
/// two mechanisms fail independently, and a flow that gets neither is told so
/// by the WebMCP probe rather than silently running without tools.
fn enable_webmcp(profile: &Path, builder: BrowserConfigBuilder) -> Result<BrowserConfigBuilder> {
    std::fs::write(
        profile.join("Local State"),
        r#"{"browser":{"enabled_labs_experiments":["enable-webmcp-testing@1"]}}"#,
    )
    .context("enabling WebMCP in the browser profile")?;
    Ok(builder.arg("enable-features=NetworkService,NetworkServiceInProcess,WebMCP"))
}

async fn launch(config: &PoolConfig) -> Result<Running> {
    // Resolve the binary ourselves and hand it over, so the browser a flow
    // drives is provably the one preflight approved.
    let executable = crate::chrome::locate()?;

    // Every pool gets its own profile directory. Chrome refuses to start a
    // second instance against a profile another process holds (ProcessSingleton
    // aborts "to avoid profile corruption"), and chromiumoxide's default is one
    // shared directory — so without this, two flows launching at the same
    // moment would kill the second, which is precisely the concurrency the
    // per-flow browser exists to support.
    let profile = tempfile::Builder::new()
        .prefix("golem-browser-")
        .tempdir()
        .context("creating the browser profile directory")?;
    let mut builder = BrowserConfig::builder()
        .chrome_executable(executable)
        .user_data_dir(profile.path());
    if !config.headless {
        builder = builder.with_head();
    }
    if config.webmcp {
        builder = enable_webmcp(profile.path(), builder)?;
    }
    let browser_config = builder
        .build()
        .map_err(|e| anyhow!("invalid browser config: {e}"))?;

    let (mut browser, mut handler) = Browser::launch(browser_config)
        .await
        .context("launching browser")?;
    let handler = tokio::spawn(async move { while handler.next().await.is_some() {} });

    // The handler only routes pages to contexts it was told about, and the
    // message that tells it is private to this call — creating a context via
    // `create_browser_context` instead would silently land every tab back in
    // the default context, i.e. lose the isolation this exists for.
    browser
        .start_incognito_context()
        .await
        .context("starting isolated browser context")?;
    let user_agent = browser.user_agent().await.context("reading user agent")?;

    Ok(Running {
        browser,
        handler,
        user_agent,
        contexts: HashMap::new(),
        _profile: profile,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skip when the host has no browser: these tests drive a real Chrome, and
    /// a dev box without one shouldn't fail the suite.
    fn chrome_available() -> bool {
        crate::chrome::locate().is_ok()
    }

    // 1. A fresh pool holds no browser — construction is free.
    #[test]
    fn new_pool_launches_nothing() {
        let pool = BrowserPool::new(PoolConfig::default());
        assert!(
            !pool.is_running(),
            "construction SHALL NOT launch a browser"
        );
    }

    // 2. Closing a pool that never launched is a no-op, not an error —
    //    teardown runs whether or not the flow reached a browser step.
    #[tokio::test]
    async fn closing_an_unlaunched_pool_is_a_noop() {
        let mut pool = BrowserPool::new(PoolConfig::default());
        pool.close().await.expect("closing SHALL succeed");
        assert!(!pool.is_running());
    }

    // 3. A context-prefixed session is rejected before anything launches, so
    //    an unsupported request never costs a browser start.
    #[tokio::test]
    async fn context_prefix_is_rejected_without_launching() {
        let mut pool = BrowserPool::new(PoolConfig::default());
        let e = pool
            .get_or_create_session(Some("tenantX:admin"))
            .await
            .expect_err("context prefix SHALL be rejected");
        assert!(format!("{e:#}").contains("not yet supported"));
        assert!(!pool.is_running(), "a rejected session SHALL NOT launch");
    }

    // 4. Two pools launch at the same time without fighting over a profile.
    //    Concurrent flows are the normal case for golem, and Chrome aborts on
    //    a profile another process holds, so a shared profile directory would
    //    make the second flow's browser die on start.
    #[tokio::test]
    async fn live_concurrent_pools_each_get_their_own_browser() {
        if !chrome_available() {
            return;
        }
        let mut first = BrowserPool::new(PoolConfig::default());
        let mut second = BrowserPool::new(PoolConfig::default());
        let (a, b) = tokio::join!(
            first.get_or_create_session(None),
            second.get_or_create_session(None)
        );
        let a = a.expect("the first browser SHALL launch");
        let b = b.expect("the second browser SHALL launch alongside it");
        assert_ne!(
            a.target_id(),
            b.target_id(),
            "separate pools SHALL NOT share a tab"
        );
        first.close().await.expect("closing SHALL succeed");
        second.close().await.expect("closing SHALL succeed");
    }

    // 4b. Capturing a session that was never opened yields nothing rather than
    //     opening a blank tab — a picture of nothing is not evidence.
    #[tokio::test]
    async fn capturing_an_unopened_session_yields_nothing() {
        let mut pool = BrowserPool::new(PoolConfig::default());
        assert!(pool
            .capture_session(None)
            .await
            .expect("capturing SHALL NOT error")
            .is_none());
        assert!(!pool.is_running(), "a capture SHALL NOT launch a browser");
    }

    // 4c. An open tab photographs itself, with the facts a post-mortem needs:
    //     a real PNG, where the tab was, and what it was showing.
    #[tokio::test]
    async fn live_capturing_an_open_session_records_the_page() {
        if !chrome_available() {
            return;
        }
        let mut pool = BrowserPool::new(PoolConfig::default());
        let page = pool
            .get_or_create_session(None)
            .await
            .expect("session SHALL open");
        page.set_content("<title>Portal</title><h1>Orders</h1>")
            .await
            .expect("content SHALL load");

        let capture = pool
            .capture_session(None)
            .await
            .expect("capturing SHALL succeed")
            .expect("an open session SHALL be capturable");
        assert_eq!(
            &capture.png[..8],
            b"\x89PNG\r\n\x1a\n",
            "the capture SHALL be a PNG"
        );
        assert_eq!(capture.title, "Portal");
        assert!(
            capture.html.contains("Orders"),
            "the sidecar HTML SHALL show what the page displayed: {}",
            capture.html
        );
        pool.close().await.expect("closing SHALL succeed");
    }

    // 5. The same session name returns the same tab; a different one opens a
    //    new tab. Both live in one browser.
    //
    //    Runs long (nextest SLOW) because it launches a real Chrome — the only
    //    way to prove tab reuse, user-agent capture and teardown actually work
    //    against CDP. It is feature-gated, so the default lane never pays it.
    #[tokio::test]
    async fn live_sessions_are_named_tabs_in_one_browser() {
        if !chrome_available() {
            return;
        }
        let mut pool = BrowserPool::new(PoolConfig::default());
        let default = pool
            .get_or_create_session(None)
            .await
            .expect("default session SHALL open");
        let again = pool
            .get_or_create_session(None)
            .await
            .expect("default session SHALL be reused");
        assert_eq!(
            default.target_id(),
            again.target_id(),
            "the same session name SHALL return the same tab"
        );

        let admin = pool
            .get_or_create_session(Some("admin"))
            .await
            .expect("named session SHALL open");
        assert_ne!(
            default.target_id(),
            admin.target_id(),
            "a different session name SHALL open a different tab"
        );

        assert!(!pool
            .user_agent()
            .await
            .expect("user agent SHALL be readable")
            .is_empty());
        pool.close().await.expect("closing SHALL succeed");
        assert!(!pool.is_running(), "close SHALL drop the browser");
    }
}
