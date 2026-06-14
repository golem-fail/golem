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
    use crate::plan::{plan, DeviceSlot};
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
        let suite = plan(&[a, b], &apps, tmp.path(), None, None, 1).await.unwrap();
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
        let suite = plan(&[f], &apps, tmp.path(), None, None, 1).await.unwrap();
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
        let suite = plan(&[f], &[], tmp.path(), None, None, 1).await.unwrap();
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
        let suite = plan(&[f], &[], tmp.path(), None, None, 1).await.unwrap();
        assert_eq!(suite.install_matrix.len(), 1);
        let entry = &suite.install_matrix[0];
        assert_eq!(entry.device_constraints.len(), 1);
        let dt = entry.device_constraints[0].device_type.as_ref().unwrap();
        assert_eq!(dt.to_vec(), vec!["tablet".to_string()]);
    }

    // Direct-call helpers: build_install_matrix branches not easily reachable
    // through `plan` (stale flow_idx, platform-None slots, per-platform script
    // gaps, custom timeout, app/bundle lookup misses).

    fn parsed_flow(toml: &str) -> ParsedFlow {
        let flow = golem_parser::parse_flow(toml).expect("flow TOML SHALL parse");
        ParsedFlow {
            path: PathBuf::from("flow.test.toml"),
            flow,
        }
    }

    fn run_with_slot(flow_idx: usize, slot: DeviceSlot) -> FlowRun {
        FlowRun {
            flow_idx,
            slots: vec![slot],
            coverage_group: None,
            covers_boxes: Vec::new(),
            repeat_index: 0,
        }
    }

    fn slot(platform: Option<Platform>, apps: &[&str]) -> DeviceSlot {
        DeviceSlot {
            platform,
            os_version: None,
            device_type: None,
            physical: None,
            name: None,
            playstore: None,
            accessibility_label: None,
            booted: None,
            apps: apps.iter().map(|s| s.to_string()).collect(),
        }
    }

    const ONE_APP: &str = r#"
        [flow]
        name = "f"
        [[flow.apps]]
        name = "app"
        bundle = "com.app"
        install_script = "scripts/i.sh"
    "#;

    // 5. A FlowRun whose flow_idx is out of range SHALL be skipped, not panic.
    #[test]
    fn stale_flow_idx_is_skipped() {
        let flows = vec![parsed_flow(ONE_APP)];
        let runs = vec![run_with_slot(99, slot(Some(Platform::Ios), &["app"]))];
        let entries = build_install_matrix(&flows, &runs, Path::new("/root"));
        assert!(entries.is_empty(),
            "out-of-range flow_idx SHALL produce no install entries");
    }

    // 6. A platform-None (responsive) slot SHALL fan out to every platform the
    //    app has an install_script for. Single script => both ios and android.
    #[test]
    fn platform_none_slot_emits_both_platforms() {
        let flows = vec![parsed_flow(ONE_APP)];
        let runs = vec![run_with_slot(0, slot(None, &["app"]))];
        let mut entries = build_install_matrix(&flows, &runs, Path::new("/root"));
        entries.sort_by_key(|e| e.platform.to_string());
        assert_eq!(entries.len(), 2,
            "platform-None slot SHALL emit one entry per installable platform");
        assert_eq!(entries[0].platform, Platform::Android);
        assert_eq!(entries[1].platform, Platform::Ios);
    }

    // 7. Per-platform install_script with only one platform key SHALL skip the
    //    platform that has no script, even for a platform-None slot.
    #[test]
    fn per_platform_script_skips_platform_without_entry() {
        let toml = r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            bundle = "com.app"
            [flow.apps.install_script]
            ios = "scripts/ios.sh"
        "#;
        let flows = vec![parsed_flow(toml)];
        let runs = vec![run_with_slot(0, slot(None, &["app"]))];
        let entries = build_install_matrix(&flows, &runs, Path::new("/root"));
        assert_eq!(entries.len(), 1,
            "per-platform script with only ios SHALL emit a single ios entry");
        assert_eq!(entries[0].platform, Platform::Ios);
    }

    // 8. A slot referencing an app name absent from the flow's apps SHALL be
    //    skipped silently.
    #[test]
    fn unknown_app_name_is_skipped() {
        let flows = vec![parsed_flow(ONE_APP)];
        let runs = vec![run_with_slot(0, slot(Some(Platform::Ios), &["nope"]))];
        let entries = build_install_matrix(&flows, &runs, Path::new("/root"));
        assert!(entries.is_empty(),
            "slot referencing a non-existent app SHALL produce no entries");
    }

    // 9. An app with no bundle SHALL be skipped even when it has a script.
    #[test]
    fn app_without_bundle_is_skipped() {
        let toml = r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            install_script = "scripts/i.sh"
        "#;
        let flows = vec![parsed_flow(toml)];
        let runs = vec![run_with_slot(0, slot(Some(Platform::Ios), &["app"]))];
        let entries = build_install_matrix(&flows, &runs, Path::new("/root"));
        assert!(entries.is_empty(),
            "app missing a bundle SHALL produce no install entry");
    }

    // 10. install_timeout_ms SHALL be carried through; absent => the default.
    #[test]
    fn timeout_uses_override_then_default() {
        let custom = r#"
            [flow]
            name = "f"
            [[flow.apps]]
            name = "app"
            bundle = "com.app"
            install_script = "scripts/i.sh"
            install_timeout_ms = 12345
        "#;
        let flows = vec![parsed_flow(custom)];
        let runs = vec![run_with_slot(0, slot(Some(Platform::Ios), &["app"]))];
        let entries = build_install_matrix(&flows, &runs, Path::new("/root"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].timeout_ms, 12345,
            "explicit install_timeout_ms SHALL be carried into the entry");

        let flows = vec![parsed_flow(ONE_APP)];
        let runs = vec![run_with_slot(0, slot(Some(Platform::Ios), &["app"]))];
        let entries = build_install_matrix(&flows, &runs, Path::new("/root"));
        assert_eq!(entries[0].timeout_ms, DEFAULT_INSTALL_TIMEOUT_MS,
            "absent install_timeout_ms SHALL fall back to the default");
    }

    // 11. script_path SHALL be project_root joined with the configured script,
    //     and bundle_id mirrors the app's bundle.
    #[test]
    fn script_path_is_joined_under_project_root() {
        let flows = vec![parsed_flow(ONE_APP)];
        let runs = vec![run_with_slot(0, slot(Some(Platform::Ios), &["app"]))];
        let entries = build_install_matrix(&flows, &runs, Path::new("/root/proj"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].script_path, PathBuf::from("/root/proj/scripts/i.sh"),
            "script_path SHALL be project_root joined with the script path");
        assert_eq!(entries[0].bundle_id, "com.app",
            "bundle_id SHALL mirror the app's configured bundle");
    }

    // 12. The dedup key is (platform, app_name) only — independent of which
    //     FlowRun/slot reaches it. A platform-None slot fans out to ios+android,
    //     then a later ios-only run hitting the same app SHALL find ios already
    //     in `seen` and add only the android entry's absence, leaving exactly
    //     the two distinct-platform entries from the first run.
    #[test]
    fn duplicate_key_across_runs_is_deduped() {
        let flows = vec![parsed_flow(ONE_APP)];
        // 1. First run fans out the responsive slot to both platforms.
        // 2. Second run (distinct repeat_index) re-references the same app on ios.
        let mut run_b = run_with_slot(0, slot(Some(Platform::Ios), &["app"]));
        run_b.repeat_index = 1;
        let runs = vec![
            run_with_slot(0, slot(None, &["app"])),
            run_b,
        ];
        let entries = build_install_matrix(&flows, &runs, Path::new("/root"));
        // 3. ios from run B collapses into ios from run A; android survives once.
        assert_eq!(entries.len(), 2,
            "(platform, app) dedup SHALL ignore which run/slot reaches the key");
        let mut platforms: Vec<String> =
            entries.iter().map(|e| e.platform.to_string()).collect();
        platforms.sort();
        assert_eq!(platforms, vec!["android".to_string(), "ios".to_string()],
            "exactly one entry per distinct platform SHALL remain after dedup");
    }

    // 13. Empty inputs SHALL produce an empty matrix.
    #[test]
    fn empty_inputs_produce_empty_matrix() {
        let entries = build_install_matrix(&[], &[], Path::new("/root"));
        assert!(entries.is_empty(), "no flow runs SHALL produce no entries");
    }
}
