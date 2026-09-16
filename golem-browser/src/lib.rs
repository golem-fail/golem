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
mod selector;
mod session;

pub use preflight::{
    flow_uses_browser, flow_uses_webmcp, preflight, BROWSE_MCP_PREFIX, BROWSE_PREFIX,
};
pub use selector::{resolve_target, BrowserTarget};
pub use session::{parse_session, SessionRef, DEFAULT_CONTEXT, DEFAULT_SESSION};

#[cfg(feature = "browser")]
mod actions;
#[cfg(feature = "browser")]
mod chrome;
#[cfg(feature = "browser")]
mod pool;

#[cfg(feature = "browser")]
pub use actions::{execute_browser_action, ScriptPaths};
#[cfg(feature = "browser")]
pub use chrome::locate;
#[cfg(feature = "browser")]
pub use pool::{BrowserPool, PoolConfig, TabCapture};
