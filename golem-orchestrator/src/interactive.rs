//! `golem probe`, which the daemon runs for a client outside any suite:
//! what a selector matches on one device. Also the helpers that sessions
//! share with it: the target query, the device lease, the app list and
//! the visible tree.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use golem_devices::resource_manager::ResourceManager;
use golem_element::toon::{encode_tree, TreeHeader};
use golem_element::{filter_viewport, Viewport};

use crate::target::{self, TargetQuery};

/// A `golem probe` request.
#[derive(Debug, Clone)]
pub struct ProbeRequest {
    /// The selector, in the canonical step notation; `action` is ignored.
    pub selector: String,
    pub query: TargetQuery,
    pub project_root: PathBuf,
    /// Poll for up to this long while nothing visible matches.
    pub timeout_ms: u64,
}

/// Probe a selector on the device and app `req` names. It only reads the
/// screen, so it takes no lease: it can look at a device a run is using.
pub async fn run_probe(req: &ProbeRequest) -> Result<(golem_runner::probe::ProbeReport, String)> {
    let parsed = golem_parser::inline::parse_selector_inline(&req.selector)?;
    let (project, _) = crate::project::ProjectConfig::load_from(&req.project_root)?;
    let target = target::resolve(&req.query, &project.apps).await?;
    let driver = target.driver();
    let report =
        golem_runner::probe::probe(driver.as_ref(), &parsed.step, &parsed.line, req.timeout_ms)
            .await?;
    Ok((
        report,
        format!("{}/{}", target.device.platform, target.device.name),
    ))
}

/// Read a `probe` request.
pub fn parse_probe_request(msg: &serde_json::Value) -> Result<ProbeRequest> {
    let selector = msg["selector"]
        .as_str()
        .context("a probe request needs a `selector`")?
        .to_string();
    Ok(ProbeRequest {
        selector,
        query: parse_query(&msg["query"])?,
        project_root: project_root(msg)?,
        timeout_ms: msg["timeout_ms"].as_u64().unwrap_or(0),
    })
}

/// The `probe` request message.
pub fn probe_request_json(
    selector: &str,
    query: &TargetQuery,
    project_root: &Path,
    timeout_ms: u64,
) -> serde_json::Value {
    serde_json::json!({
        "type": "probe",
        "selector": selector,
        "query": query_json(query),
        "project_root": project_root.display().to_string(),
        "timeout_ms": timeout_ms,
    })
}

/// Lease `device` for an interactive command, or say who holds it.
pub fn lease(
    resource_mgr: &Arc<ResourceManager>,
    device: &golem_devices::DeviceInfo,
) -> Result<golem_devices::resource_manager::DeviceLease> {
    resource_mgr.try_lease(device, 0).with_context(|| {
        format!(
            "{} ({}) is in use by a run or a session; wait for it, or pick another device",
            device.name, device.udid
        )
    })
}

/// The visible tree on `driver`'s screen, as TOON.
pub async fn visible_tree(driver: &dyn golem_driver::PlatformDriver) -> Result<String> {
    let (root, meta) = driver.get_hierarchy().await?;
    let mut viewport = Viewport::from_root(&root);
    viewport.height -= meta.keyboard_height;
    let header = TreeHeader {
        full: false,
        keyboard_height: meta.keyboard_height,
    };
    Ok(encode_tree(&filter_viewport(&root, &viewport), &header))
}

/// The project's app registry as the flow-level app list a step resolves
/// `app = "…"` against.
pub(crate) fn app_configs(apps: &[golem_parser::ProjectAppConfig]) -> Vec<golem_parser::AppConfig> {
    apps.iter()
        .map(|a| golem_parser::AppConfig {
            name: a.name.clone(),
            bundle: a.bundle.clone(),
            devices: a.devices.clone(),
            install_script: a.install_script.clone(),
            install_timeout_ms: a.install_timeout_ms,
            install_env: a.install_env.clone(),
            profile: a.profile.clone(),
            permissions: Default::default(),
        })
        .collect()
}

/// The `project_root` of a request.
pub fn project_root(msg: &serde_json::Value) -> Result<PathBuf> {
    msg["project_root"]
        .as_str()
        .map(PathBuf::from)
        .context("the request needs a `project_root`")
}

/// A target query from its JSON form: `os`, `type`, `device`, `bundle`
/// and `app`, each optional.
pub fn parse_query(q: &serde_json::Value) -> Result<TargetQuery> {
    let text = |v: &serde_json::Value| v.as_str().map(str::to_string);
    Ok(TargetQuery {
        os: q["os"]
            .as_str()
            .map(crate::target::OsQuery::parse)
            .transpose()?,
        device_type: q["type"]
            .as_str()
            .map(crate::target::parse_device_type)
            .transpose()?,
        device: text(&q["device"]),
        bundle: text(&q["bundle"]),
        app: text(&q["app"]),
    })
}

/// The JSON form of `query`, as [`parse_query`] reads it.
pub fn query_json(query: &TargetQuery) -> serde_json::Value {
    serde_json::json!({
        "os": query.os.as_ref().map(|o| o.text.clone()),
        "type": query.device_type.map(|t| t.to_string()),
        "device": query.device,
        "bundle": query.bundle,
        "app": query.app,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_round_trips() {
        let query = TargetQuery {
            os: Some(crate::target::OsQuery::parse("ios:26").expect("os")),
            device_type: Some(golem_devices::DeviceType::Phone),
            device: Some("iPhone 17".into()),
            bundle: None,
            app: Some("app".into()),
        };
        let back = parse_query(&query_json(&query)).expect("parse");
        assert_eq!(back.os, query.os);
        assert_eq!(back.device_type, query.device_type);
        assert_eq!(back.device, query.device);
        assert_eq!(back.app, query.app);
        assert!(back.bundle.is_none());
    }

    #[test]
    fn a_probe_request_round_trips() {
        let query = TargetQuery {
            os: Some(crate::target::OsQuery::parse("android").expect("os")),
            ..TargetQuery::default()
        };
        let msg = probe_request_json(r#"{ on_text = "OK" }"#, &query, Path::new("/proj"), 750);
        let req = parse_probe_request(&msg).expect("parse");
        assert_eq!(req.selector, r#"{ on_text = "OK" }"#);
        assert_eq!(req.query.platform(), Some(golem_devices::Platform::Android));
        assert_eq!(req.project_root, Path::new("/proj"));
        assert_eq!(req.timeout_ms, 750);
    }

    #[test]
    fn the_registry_becomes_the_step_app_list() {
        let apps = app_configs(&[golem_parser::ProjectAppConfig {
            name: "app".into(),
            bundle: Some("fail.golem.test".into()),
            devices: Vec::new(),
            install_script: None,
            install_timeout_ms: None,
            install_env: None,
            profile: None,
        }]);
        assert_eq!(apps[0].name, "app");
        assert_eq!(apps[0].bundle.as_deref(), Some("fail.golem.test"));
    }
}
