//! One-off commands the daemon runs for a client outside any suite:
//! `golem do` runs one step on one device, and `golem probe` reports what
//! a selector matches there.
//!
//! They run in the daemon, not in the client, so that a step takes its
//! device from the same `ResourceManager` as the suite runs: a `golem do`
//! never acts on a device a run is using.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use golem_devices::resource_manager::ResourceManager;
use golem_element::toon::{encode_tree, TreeHeader};
use golem_element::{filter_viewport, Viewport};
use golem_report::StepReport;

use crate::target::{self, TargetQuery};

/// A `golem do` request.
#[derive(Debug, Clone)]
pub struct DoRequest {
    /// The step, in the canonical one-line notation.
    pub step: String,
    pub query: TargetQuery,
    /// The client's project root: `golem.toml` is read from here, and
    /// relative script and fixture paths resolve against it.
    pub project_root: PathBuf,
    /// Also return the visible tree after the step.
    pub tree: bool,
    pub child_env: Option<golem_common::command::ChildEnv>,
}

/// What a `golem do` returns.
pub struct DoResult {
    pub report: StepReport,
    /// The visible tree after the step, as TOON, when asked for.
    pub tree: Option<String>,
    /// `<platform>/<name>` of the device the step ran on.
    pub device: String,
}

/// Run one step on the device and app `req` names.
pub async fn run_do(req: &DoRequest, resource_mgr: &Arc<ResourceManager>) -> Result<DoResult> {
    let step = golem_parser::inline::parse_step_inline(&req.step)?.step;
    let (project, _) = crate::project::ProjectConfig::load_from(&req.project_root)?;
    let selection = target::select(&req.query, &project.apps).await?;
    // Lease before any companion work: see `target::connect`.
    let _lease = lease(resource_mgr, &selection.device)?;
    let target = target::connect(selection).await?;
    let driver = target.driver();
    let apps = app_configs(&project.apps);
    let capture = golem_runner::capture::CaptureConfig {
        screenshot_on_failure: false,
        output_dir: req.project_root.join(".golem/results"),
        ..Default::default()
    };
    let mut ctx = golem_runner::context::ExecutionContext {
        device: Some(&target.device),
        child_env: req.child_env.as_ref(),
        ..golem_runner::context::ExecutionContext::new(
            &req.project_root,
            &req.project_root,
            &capture,
            "golem do",
        )
    };
    let mut vars = golem_vars::VariableStore::new();
    vars.push_scope(golem_vars::Scope::new(golem_vars::ScopeLevel::Flow));

    let report = golem_runner::single_step::execute_single_step(
        &step,
        driver.as_ref(),
        &mut vars,
        &mut ctx,
        &apps,
        golem_runner::policy::DEFAULT_BASE_TIMEOUT_MS,
    )
    .await;

    let tree = if req.tree {
        Some(visible_tree(driver.as_ref()).await?)
    } else {
        None
    };
    Ok(DoResult {
        report,
        tree,
        device: format!("{}/{}", target.device.platform, target.device.name),
    })
}

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
    let as_do = parse_do_request(&serde_json::json!({
        "step": "",
        "query": msg["query"],
        "project_root": msg["project_root"],
    }))?;
    Ok(ProbeRequest {
        selector,
        query: as_do.query,
        project_root: as_do.project_root,
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
    let mut msg = do_request_json("", query, project_root, false);
    msg["type"] = serde_json::json!("probe");
    msg["selector"] = serde_json::json!(selector);
    msg["timeout_ms"] = serde_json::json!(timeout_ms);
    msg
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

/// Read a `do` request from a submit-style JSON message.
pub fn parse_do_request(msg: &serde_json::Value) -> Result<DoRequest> {
    let step = msg["step"]
        .as_str()
        .context("a do request needs a `step`")?
        .to_string();
    let q = &msg["query"];
    let text = |v: &serde_json::Value| v.as_str().map(str::to_string);
    let platform = match q["platform"].as_str() {
        None => None,
        Some("ios") => Some(golem_devices::Platform::Ios),
        Some("android") => Some(golem_devices::Platform::Android),
        Some(p) => anyhow::bail!("unknown platform: {p}. Use 'ios' or 'android'."),
    };
    let project_root = msg["project_root"]
        .as_str()
        .map(PathBuf::from)
        .context("a do request needs a `project_root`")?;
    Ok(DoRequest {
        step,
        query: TargetQuery {
            platform,
            device: text(&q["device"]),
            bundle: text(&q["bundle"]),
            app: text(&q["app"]),
        },
        project_root,
        tree: msg["tree"].as_bool().unwrap_or(false),
        child_env: crate::ipc::parse_child_env(msg),
    })
}

/// The `do` request message for `req`, without the client context that
/// [`crate::ipc`] adds.
pub fn do_request_json(
    step: &str,
    query: &TargetQuery,
    project_root: &Path,
    tree: bool,
) -> serde_json::Value {
    serde_json::json!({
        "type": "do",
        "step": step,
        "query": {
            "platform": query.platform.map(|p| p.to_string()),
            "device": query.device,
            "bundle": query.bundle,
            "app": query.app,
        },
        "project_root": project_root.display().to_string(),
        "tree": tree,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_do_request_round_trips() {
        let query = TargetQuery {
            platform: Some(golem_devices::Platform::Ios),
            device: Some("iPhone 17".into()),
            bundle: None,
            app: Some("app".into()),
        };
        let mut msg = do_request_json(
            r#"{ action = "tap", on_text = "+" }"#,
            &query,
            Path::new("/proj"),
            true,
        );
        msg["client_cwd"] = serde_json::json!("/proj/sub");
        msg["client_env"] = serde_json::json!([["A", "1"]]);
        let req = parse_do_request(&msg).expect("parse");
        assert_eq!(req.step, r#"{ action = "tap", on_text = "+" }"#);
        assert_eq!(req.query.platform, Some(golem_devices::Platform::Ios));
        assert_eq!(req.query.device.as_deref(), Some("iPhone 17"));
        assert_eq!(req.query.app.as_deref(), Some("app"));
        assert!(req.query.bundle.is_none());
        assert_eq!(req.project_root, Path::new("/proj"));
        assert!(req.tree);
        let env = req.child_env.expect("env");
        assert_eq!(env.cwd.as_deref(), Some(Path::new("/proj/sub")));
        assert_eq!(env.vars, vec![("A".to_string(), "1".to_string())]);
    }

    #[test]
    fn a_probe_request_round_trips() {
        let query = TargetQuery {
            platform: Some(golem_devices::Platform::Android),
            ..TargetQuery::default()
        };
        let msg = probe_request_json(r#"{ on_text = "OK" }"#, &query, Path::new("/proj"), 750);
        let req = parse_probe_request(&msg).expect("parse");
        assert_eq!(req.selector, r#"{ on_text = "OK" }"#);
        assert_eq!(req.query.platform, Some(golem_devices::Platform::Android));
        assert_eq!(req.project_root, Path::new("/proj"));
        assert_eq!(req.timeout_ms, 750);
    }

    #[test]
    fn a_do_request_without_a_step_is_refused() {
        let msg = serde_json::json!({ "type": "do", "project_root": "/p" });
        assert!(parse_do_request(&msg)
            .expect_err("no step")
            .to_string()
            .contains("`step`"));
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
