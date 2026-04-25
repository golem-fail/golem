//! Install matrix: the de-duplicated list of `(platform, app)` entries
//! derived from every `DeviceSlot` across every `FlowRun`.
//!
//! Apps in `golem.toml [[apps]]` that no flow references are **not** in the
//! matrix — they won't be built or installed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use golem_devices::Platform;
use golem_parser::DeviceConstraint;
use golem_runner::installer::DEFAULT_INSTALL_TIMEOUT_MS;

use crate::plan::{FlowRun, ParsedFlow};

/// One install task: build + install `app_name` (bundle `bundle_id`) onto
/// a device of `platform` via `script_path`.
///
/// `device_constraints` carries the per-app `[[apps.devices]]` filter so the
/// Execute phase can skip installing on a device that doesn't match (resolver
/// normally prevents the pairing upstream; this is a safety net).
#[derive(Debug, Clone)]
pub struct InstallEntry {
    pub platform: Platform,
    pub app_name: String,
    pub bundle_id: String,
    pub script_path: PathBuf,
    pub timeout_ms: u64,
    pub device_constraints: Vec<DeviceConstraint>,
}

/// Walk every `FlowRun.slots[*].apps` and emit one `InstallEntry` per
/// unique `(slot.platform, app_name)`. Apps missing a bundle or a
/// platform-matching `install_script` are silently skipped — the per-flow
/// install check in the Execute phase handles them as `FailedNoScript`
/// if launch later needs them.
pub fn build_install_matrix(
    flows: &[ParsedFlow],
    flow_runs: &[FlowRun],
    project_root: &Path,
) -> Vec<InstallEntry> {
    let mut seen: HashSet<(Platform, String)> = HashSet::new();
    let mut entries: Vec<InstallEntry> = Vec::new();

    for run in flow_runs {
        // Safe lookup: callers in-tree always supply matched flow_idx, but
        // external callers may construct `FlowRun`s independently (e.g. tests,
        // future scheduler harnesses) — a stale idx shouldn't panic the plan.
        let Some(pf) = flows.get(run.flow_idx) else {
            continue;
        };
        for slot in &run.slots {
            // Platform-None slots (responsive-design) may run on either
            // platform — emit install entries for every platform the app
            // actually has an install_script for. The scheduler's
            // install-matrix intersection narrows to installable platforms
            // at slot-pick time.
            let target_platforms: Vec<Platform> = match slot.platform {
                Some(p) => vec![p],
                None => vec![Platform::Ios, Platform::Android],
            };
            for app_name in &slot.apps {
                let Some(app) = pf.flow.flow.apps.iter().find(|a| &a.name == app_name) else {
                    continue;
                };
                let Some(bundle) = app.bundle.as_deref() else {
                    continue;
                };
                for platform in &target_platforms {
                    let key = (*platform, app_name.clone());
                    if seen.contains(&key) {
                        continue;
                    }
                    let platform_str = platform.to_string();
                    let Some(script) = app
                        .install_script
                        .as_ref()
                        .and_then(|v| v.for_platform(&platform_str))
                    else {
                        continue;
                    };
                    let timeout_ms = app
                        .install_timeout_ms
                        .unwrap_or(DEFAULT_INSTALL_TIMEOUT_MS);
                    entries.push(InstallEntry {
                        platform: *platform,
                        app_name: app_name.clone(),
                        bundle_id: bundle.to_string(),
                        script_path: project_root.join(script),
                        timeout_ms,
                        device_constraints: app.devices.clone(),
                    });
                    seen.insert(key);
                }
            }
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::plan;
    use golem_parser::{InstallScriptValue, ProjectAppConfig};
    use std::io::Write;
    use tempfile::TempDir;

    fn write_flow(dir: &std::path::Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    #[tokio::test]
    async fn matrix_dedupes_same_platform_app_across_flows() {
        let tmp = TempDir::new().unwrap();
        let a = write_flow(tmp.path(), "a.test.toml", r#"
            [flow]
            name = "a"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios"
        "#);
        let b = write_flow(tmp.path(), "b.test.toml", r#"
            [flow]
            name = "b"
            [[flow.apps]]
            name = "app"
            [[flow.apps.devices]]
            os = "ios"
        "#);
        let apps = vec![ProjectAppConfig {
            name: "app".into(),
            bundle: Some("com.app".into()),
            devices: Vec::new(),
            install_script: Some(InstallScriptValue::Single("scripts/i.sh".into())),
            install_timeout_ms: None,
        }];
        let suite = plan(&[a, b], &apps, tmp.path(), None, None).await.unwrap();
        assert_eq!(suite.install_matrix.len(), 1,
            "two flows on the same (platform, app) SHALL produce one entry");
    }

    #[tokio::test]
    async fn matrix_separates_per_platform() {
        let tmp = TempDir::new().unwrap();
        let f = write_flow(tmp.path(), "f.test.toml", r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "client"
            [[flow.apps.devices]]
            os = "ios"
            [[flow.apps]]
            name = "supplier"
            [[flow.apps.devices]]
            os = "android"
        "#);
        let apps = vec![
            ProjectAppConfig {
                name: "client".into(),
                bundle: Some("com.c".into()),
                devices: Vec::new(),
                install_script: Some(InstallScriptValue::Single("scripts/c.sh".into())),
                install_timeout_ms: None,
            },
            ProjectAppConfig {
                name: "supplier".into(),
                bundle: Some("com.s".into()),
                devices: Vec::new(),
                install_script: Some(InstallScriptValue::Single("scripts/s.sh".into())),
                install_timeout_ms: None,
            },
        ];
        let suite = plan(&[f], &apps, tmp.path(), None, None).await.unwrap();
        assert_eq!(suite.install_matrix.len(), 2,
            "chat-test apps on different platforms SHALL produce 2 install entries");
    }

    #[tokio::test]
    async fn matrix_skips_app_without_install_script() {
        let tmp = TempDir::new().unwrap();
        let f = write_flow(tmp.path(), "f.test.toml", r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            bundle = "com.app"
            [[flow.apps.devices]]
            os = "ios"
        "#);
        let suite = plan(&[f], &[], tmp.path(), None, None).await.unwrap();
        assert!(suite.install_matrix.is_empty(),
            "apps without install_script SHALL NOT appear in install_matrix");
    }

    #[tokio::test]
    async fn matrix_carries_device_constraints() {
        let tmp = TempDir::new().unwrap();
        let f = write_flow(tmp.path(), "f.test.toml", r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            bundle = "com.app"
            install_script = "scripts/i.sh"
            [[flow.apps.devices]]
            os = "ios"
            type = "tablet"
        "#);
        let suite = plan(&[f], &[], tmp.path(), None, None).await.unwrap();
        assert_eq!(suite.install_matrix.len(), 1);
        let entry = &suite.install_matrix[0];
        assert_eq!(entry.device_constraints.len(), 1);
        let dt = entry.device_constraints[0].device_type.as_ref().unwrap();
        assert_eq!(dt.to_vec(), vec!["tablet".to_string()]);
    }
}
