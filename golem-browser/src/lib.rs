//! Host-side browser automation backing the `browse_*` actions.
//!
//! The browser is **instrumentation, not the system under test**: a flow uses
//! it to drive external web state the mobile app depends on (a supplier portal
//! with no API, an admin console). Browser steps are not judged for coverage,
//! never feed the a11y audit, and — unlike every mobile action — assert on DOM
//! presence rather than the visible tree, because what a headless Chrome
//! "sees" is not what a user sees and pretending otherwise would be theatre.
//!
//! The engine lives behind the `browser` cargo feature. Without it this crate
//! still compiles, still parses session labels, and still explains itself when
//! a flow asks for a browser it can't provide.

mod preflight;
mod session;

pub use preflight::{flow_uses_browser, preflight, BROWSE_PREFIX};
pub use session::{parse_session, SessionRef, DEFAULT_CONTEXT, DEFAULT_SESSION};

#[cfg(feature = "browser")]
mod chrome;
#[cfg(feature = "browser")]
mod pool;

#[cfg(feature = "browser")]
pub use chrome::locate;
#[cfg(feature = "browser")]
pub use pool::{BrowserPool, PoolConfig};

/// Dispatch one `browse_*` step. The actions themselves land in #99 onward;
/// today this resolves the target tab and rejects everything, which is enough
/// for the runner seam to exist without inventing behaviour the docs don't
/// describe yet.
#[cfg(feature = "browser")]
pub async fn execute_browser_action(
    pool: &mut BrowserPool,
    action: &str,
    session: Option<&str>,
) -> anyhow::Result<()> {
    let _page = pool.get_or_create_session(session).await?;
    Err(golem_events::coded(
        golem_events::FailureCode::ParseUnknownAction,
        anyhow::anyhow!("browser action `{action}` is not implemented yet"),
    ))
}
