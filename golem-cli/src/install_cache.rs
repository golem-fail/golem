//! Install-cache gate logic and warm-device ranking.
//!
//! Pure decision functions (`gate_decision`) and their I/O wrappers live
//! here so they can be unit-tested in isolation from suite orchestration.

use golem_devices::{DeviceInfo, Platform};
use golem_orchestrator::{DeviceSlot, InstallEntry};

/// Outcome of consulting the install cache for a single `(device, bundle)`
/// pair. Hits are summarised optimistically; misses carry a specific
/// reason so a verbose log can explain *why* a build was needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CacheVerdict {
    /// All gates passed. `label` is the source-fingerprint identity.
    Hit { label: String },
    /// At least one gate failed. `reason` is human-readable.
    Miss { reason: String },
}

/// Pure gate decision over already-fetched inputs. Returns the verdict
/// plus a specific miss reason when applicable. Split out from
/// [`evaluate_cache_gates`] so unit tests can exercise every gate
/// combination without faking I/O.
pub(crate) fn gate_decision(
    entry: Option<&golem_runner::installer::PersistedInstall>,
    current_fingerprint: &golem_runner::fingerprint::Fingerprint,
    info: &golem_runner::installed_state::DeviceInstallInfo,
) -> CacheVerdict {
    if !current_fingerprint.is_some() {
        return CacheVerdict::Miss {
            reason: "fingerprint unavailable (no git, no readable source tree)".into(),
        };
    }
    let Some(entry) = entry else {
        return CacheVerdict::Miss {
            reason: "no prior cache entry for this (device, bundle)".into(),
        };
    };
    if &entry.fingerprint != current_fingerprint {
        return CacheVerdict::Miss {
            reason: format!(
                "source fingerprint changed ({} → {})",
                entry.fingerprint.short_label(),
                current_fingerprint.short_label(),
            ),
        };
    }
    if !info.installed {
        return CacheVerdict::Miss {
            reason: "bundle no longer installed on device".into(),
        };
    }
    // Install-time check: only fires when both sides recorded a
    // timestamp. If either is `None`, the gate is skipped silently —
    // fingerprint+presence alone are sufficient.
    if let (Some(stored), Some(current)) = (entry.device_install_time, info.install_time) {
        // 2s tolerance for filesystem mtime quantisation.
        if (stored - current).num_seconds().abs() > 2 {
            return CacheVerdict::Miss {
                reason: format!(
                    "device install-time differs ({} cached, {} on device — external reinstall?)",
                    stored.format("%Y-%m-%dT%H:%M:%SZ"),
                    current.format("%Y-%m-%dT%H:%M:%SZ"),
                ),
            };
        }
    }
    CacheVerdict::Hit {
        label: current_fingerprint.short_label(),
    }
}

/// Evaluate the three integrity gates and return a [`CacheVerdict`].
///
/// Gates:
/// 1. Persistent entry exists for `(udid, bundle)` with a non-`None`
///    fingerprint
/// 2. Stored fingerprint equals the current source fingerprint
/// 3. Device reports the bundle present AND its current install time
///    matches the stored `device_install_time` (when available) — catches
///    external reinstalls
///
/// Any gate failing → `CacheVerdict::Miss { reason }` with a specific
/// human-readable cause. The caller emits the reason on a verbose log
/// line and falls through to the build path.
pub(crate) async fn evaluate_cache_gates(
    cache: &golem_runner::installer::InstallCache,
    device: &DeviceInfo,
    bundle_id: &str,
    current_fingerprint: &golem_runner::fingerprint::Fingerprint,
) -> CacheVerdict {
    let entry = cache.get_persistent(&device.udid, bundle_id).await;
    let info = golem_runner::installed_state::query(device, bundle_id).await;
    gate_decision(entry.as_ref(), current_fingerprint, &info)
}

/// Pick the free candidate with the most install-cache `Succeeded` hits
/// for the slot's apps — saves re-running install scripts on a cold device
/// when a warm one is free. Ties are broken by the input order (stable).
///
/// If `install_cache` is `None`, `slot` is `None`, or the install matrix has
/// no entries matching this (platform, app), every candidate scores 0 and the
/// first is returned. That preserves the pre-ranking behaviour for test
/// harnesses and platform-only calls.
///
/// `Failed*` cache entries don't count — the suite still needs a device we
/// haven't yet installed on, and the per-flow skip logic handles re-use of
/// known-failed devices upstream.
pub(crate) async fn rank_by_install_cache<'a>(
    free: &'a [&'a DeviceInfo],
    platform: Option<Platform>,
    slot: Option<&DeviceSlot>,
    install_cache: Option<&golem_runner::installer::InstallCache>,
    install_matrix: &[InstallEntry],
) -> &'a DeviceInfo {
    let (Some(cache), Some(s)) = (install_cache, slot) else {
        return free[0];
    };
    let mut best: &DeviceInfo = free[0];
    let mut best_score = 0usize;
    for (i, dev) in free.iter().enumerate() {
        // Bundle list is per-device when the slot is platform-agnostic
        // (mixed-platform free pool). Each device picks its own platform's
        // install_matrix entries. When the slot pins a platform, we still
        // honour it as a sanity filter.
        let dev_platform = platform.unwrap_or(dev.platform);
        let bundles: Vec<&str> = s
            .apps
            .iter()
            .filter_map(|app_name| {
                install_matrix
                    .iter()
                    .find(|e| e.platform == dev_platform && &e.app_name == app_name)
                    .map(|e| e.bundle_id.as_str())
            })
            .collect();
        if bundles.is_empty() {
            continue;
        }
        let mut score = 0usize;
        for b in &bundles {
            if let Some(golem_runner::installer::InstallOutcome::Succeeded) =
                cache.get(&(dev.udid.clone(), (*b).to_string())).await
            {
                score += 1;
            }
        }
        if i == 0 || score > best_score {
            best = dev;
            best_score = score;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_devices::DeviceState;
    use std::path::PathBuf;

    // ---------------------------------------------------------------
    // gate_decision — every combination
    // ---------------------------------------------------------------
    fn make_entry(
        fp: golem_runner::fingerprint::Fingerprint,
        install_time: Option<chrono::DateTime<chrono::Utc>>,
    ) -> golem_runner::installer::PersistedInstall {
        golem_runner::installer::PersistedInstall {
            fingerprint: fp,
            device_install_time: install_time,
            installed_version: None,
            installed_at: chrono::Utc::now(),
        }
    }

    fn info_present(
        install_time: Option<chrono::DateTime<chrono::Utc>>,
    ) -> golem_runner::installed_state::DeviceInstallInfo {
        golem_runner::installed_state::DeviceInstallInfo {
            installed: true,
            install_time,
            version: None,
        }
    }

    fn fp_git(rev: &str) -> golem_runner::fingerprint::Fingerprint {
        golem_runner::fingerprint::Fingerprint::Git {
            rev: rev.into(),
            porcelain: "x".into(),
        }
    }

    fn miss_reason(v: CacheVerdict) -> String {
        match v {
            CacheVerdict::Miss { reason } => reason,
            CacheVerdict::Hit { .. } => panic!("expected Miss, got Hit"),
        }
    }

    #[test]
    fn gate_decision_none_fingerprint_misses_with_reason() {
        let entry = make_entry(fp_git("a"), None);
        let v = gate_decision(
            Some(&entry),
            &golem_runner::fingerprint::Fingerprint::None,
            &info_present(None),
        );
        let r = miss_reason(v);
        assert!(r.contains("fingerprint unavailable"), "got: {r}");
    }

    #[test]
    fn gate_decision_no_entry_misses_with_reason() {
        let v = gate_decision(None, &fp_git("a"), &info_present(None));
        let r = miss_reason(v);
        assert!(r.contains("no prior cache entry"), "got: {r}");
    }

    #[test]
    fn gate_decision_fingerprint_mismatch_reports_both_sides() {
        let entry = make_entry(fp_git("aaaaaaaaaaa"), None);
        let v = gate_decision(Some(&entry), &fp_git("bbbbbbbbbbb"), &info_present(None));
        let r = miss_reason(v);
        assert!(r.contains("fingerprint changed"), "got: {r}");
        assert!(
            r.contains("git:aaaaaaa") && r.contains("git:bbbbbbb"),
            "miss reason SHALL show stored → current: {r}"
        );
    }

    #[test]
    fn gate_decision_bundle_absent_misses_with_reason() {
        let entry = make_entry(fp_git("a"), None);
        let info = golem_runner::installed_state::DeviceInstallInfo::not_installed();
        let v = gate_decision(Some(&entry), &fp_git("a"), &info);
        let r = miss_reason(v);
        assert!(r.contains("no longer installed"), "got: {r}");
    }

    #[test]
    fn gate_decision_match_no_install_time_hits() {
        // When install-time isn't recorded on either side, fingerprint match
        // alone is sufficient.
        let entry = make_entry(fp_git("a"), None);
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(None));
        assert!(matches!(v, CacheVerdict::Hit { .. }));
    }

    #[test]
    fn gate_decision_install_time_match_hits() {
        let t = chrono::Utc::now();
        let entry = make_entry(fp_git("a"), Some(t));
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(Some(t)));
        assert!(matches!(v, CacheVerdict::Hit { .. }));
    }

    #[test]
    fn gate_decision_install_time_drift_misses_with_external_reinstall_hint() {
        let t = chrono::Utc::now();
        let drifted = t + chrono::Duration::seconds(10);
        let entry = make_entry(fp_git("a"), Some(t));
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(Some(drifted)));
        let r = miss_reason(v);
        assert!(r.contains("install-time differs"), "got: {r}");
        assert!(
            r.contains("external reinstall"),
            "miss reason SHALL hint at external reinstall cause: {r}"
        );
    }

    #[test]
    fn gate_decision_install_time_within_tolerance_hits() {
        let t = chrono::Utc::now();
        let close = t + chrono::Duration::seconds(1);
        let entry = make_entry(fp_git("a"), Some(t));
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(Some(close)));
        assert!(
            matches!(v, CacheVerdict::Hit { .. }),
            "1s drift SHALL be within tolerance"
        );
    }

    #[test]
    fn gate_decision_install_time_drift_negative_direction_misses() {
        // 1. Drift detection uses abs(): when the device's recorded time is
        //    *earlier* than the stored cache time, the gate still fires.
        let t = chrono::Utc::now();
        let earlier = t - chrono::Duration::seconds(30);
        let entry = make_entry(fp_git("a"), Some(t));
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(Some(earlier)));
        let r = miss_reason(v);
        assert!(
            r.contains("install-time differs"),
            "negative-direction drift SHALL also miss (abs comparison): {r}"
        );
    }

    #[test]
    fn gate_decision_install_time_at_exactly_two_seconds_hits() {
        // 2. The tolerance is `> 2`, so exactly 2 seconds is inclusive (a Hit).
        let t = chrono::Utc::now();
        let edge = t + chrono::Duration::seconds(2);
        let entry = make_entry(fp_git("a"), Some(t));
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(Some(edge)));
        assert!(
            matches!(v, CacheVerdict::Hit { .. }),
            "exactly 2s drift SHALL be within the inclusive tolerance"
        );
    }

    #[test]
    fn gate_decision_install_time_just_over_tolerance_misses() {
        // 3. 3 seconds is the first value strictly greater than the 2s tolerance.
        let t = chrono::Utc::now();
        let over = t + chrono::Duration::seconds(3);
        let entry = make_entry(fp_git("a"), Some(t));
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(Some(over)));
        let r = miss_reason(v);
        assert!(
            r.contains("install-time differs"),
            "3s drift SHALL exceed the 2s tolerance and miss: {r}"
        );
    }

    #[test]
    fn gate_decision_stored_install_time_none_skips_gate_and_hits() {
        // 4. When only the device reports an install-time (stored is None), the
        //    drift gate is skipped — fingerprint+presence carry the decision.
        let t = chrono::Utc::now();
        let entry = make_entry(fp_git("a"), None);
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(Some(t)));
        assert!(
            matches!(v, CacheVerdict::Hit { .. }),
            "missing stored install-time SHALL skip the drift gate"
        );
    }

    #[test]
    fn gate_decision_device_install_time_none_skips_gate_and_hits() {
        // 5. Symmetric to above: stored time present, device time None.
        let t = chrono::Utc::now();
        let entry = make_entry(fp_git("a"), Some(t));
        let v = gate_decision(Some(&entry), &fp_git("a"), &info_present(None));
        assert!(
            matches!(v, CacheVerdict::Hit { .. }),
            "missing device install-time SHALL skip the drift gate"
        );
    }

    #[test]
    fn gate_decision_no_entry_takes_precedence_over_absent_bundle() {
        // 6. Gate ordering: a missing entry is reported before the
        //    not-installed gate even when the device also lacks the bundle.
        let info = golem_runner::installed_state::DeviceInstallInfo::not_installed();
        let v = gate_decision(None, &fp_git("a"), &info);
        let r = miss_reason(v);
        assert!(
            r.contains("no prior cache entry"),
            "missing-entry gate SHALL fire before the presence gate: {r}"
        );
    }

    #[test]
    fn gate_decision_none_fingerprint_takes_precedence_over_no_entry() {
        // 7. Gate ordering: an unavailable fingerprint is reported even when
        //    there is no entry at all.
        let v = gate_decision(
            None,
            &golem_runner::fingerprint::Fingerprint::None,
            &info_present(None),
        );
        let r = miss_reason(v);
        assert!(
            r.contains("fingerprint unavailable"),
            "fingerprint gate SHALL fire before the missing-entry gate: {r}"
        );
    }

    #[test]
    fn gate_decision_hit_label_is_fingerprint_short_label() {
        let entry = make_entry(fp_git("abc1234567"), None);
        let v = gate_decision(Some(&entry), &fp_git("abc1234567"), &info_present(None));
        match v {
            CacheVerdict::Hit { label } => {
                assert!(
                    label.contains("git:abc1234"),
                    "hit label SHALL be the source-fingerprint short label: {label}"
                );
            }
            CacheVerdict::Miss { .. } => panic!("expected Hit"),
        }
    }

    // ---------------------------------------------------------------
    // Install-cache-hit ranking: helper picks the warm device when multiple
    // free devices could match — saves re-running the install script.
    // ---------------------------------------------------------------
    fn test_device(name: &str, udid: &str) -> DeviceInfo {
        DeviceInfo {
            name: name.to_string(),
            udid: udid.to_string(),
            platform: Platform::Ios,
            device_type: golem_devices::DeviceType::Phone,
            os_major: 18,
            os_version: "18.0".to_string(),
            state: DeviceState::Booted,
            physical: false,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        }
    }

    fn test_slot(apps: &[&str]) -> DeviceSlot {
        DeviceSlot {
            platform: Some(Platform::Ios),
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

    fn test_matrix_entry(app: &str, bundle: &str) -> InstallEntry {
        InstallEntry {
            platform: Platform::Ios,
            app_name: app.to_string(),
            bundle_id: bundle.to_string(),
            script_path: PathBuf::from("/tmp/noop.sh"),
            timeout_ms: 1000,
            device_constraints: Vec::new(),
        }
    }

    #[tokio::test]
    async fn rank_prefers_device_with_cache_hit_for_slot_app() {
        use golem_runner::installer::{InstallCache, InstallOutcome};
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];

        let cache = InstallCache::new();
        cache
            .set(
                ("udid-2".into(), "com.app".into()),
                InstallOutcome::Succeeded,
            )
            .await;

        let slot = test_slot(&["app"]);
        let matrix = vec![test_matrix_entry("app", "com.app")];

        let pick = rank_by_install_cache(
            &free,
            Some(Platform::Ios),
            Some(&slot),
            Some(&cache),
            &matrix,
        )
        .await;
        assert_eq!(
            pick.udid, "udid-2",
            "SHALL rank the sim with a Succeeded install cache entry above the cold one",
        );
    }

    #[tokio::test]
    async fn rank_without_cache_returns_first_candidate() {
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];
        let slot = test_slot(&["app"]);
        let matrix = vec![test_matrix_entry("app", "com.app")];

        let pick =
            rank_by_install_cache(&free, Some(Platform::Ios), Some(&slot), None, &matrix).await;
        assert_eq!(
            pick.udid, "udid-1",
            "SHALL fall back to input order when no cache is available",
        );
    }

    #[tokio::test]
    async fn rank_failed_cache_entry_does_not_count() {
        use golem_runner::installer::{InstallCache, InstallOutcome};
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];

        let cache = InstallCache::new();
        cache
            .set(
                ("udid-2".into(), "com.app".into()),
                InstallOutcome::FailedScript("nope".into()),
            )
            .await;

        let slot = test_slot(&["app"]);
        let matrix = vec![test_matrix_entry("app", "com.app")];

        let pick = rank_by_install_cache(
            &free,
            Some(Platform::Ios),
            Some(&slot),
            Some(&cache),
            &matrix,
        )
        .await;
        assert_eq!(
            pick.udid, "udid-1",
            "FailedScript SHALL NOT count as a cache hit — first candidate wins the tie",
        );
    }

    #[tokio::test]
    async fn rank_picks_device_with_more_hits_for_multi_app_slot() {
        use golem_runner::installer::{InstallCache, InstallOutcome};
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];

        let cache = InstallCache::new();
        // sim1 has one hit (app_a), sim2 has both (app_a + app_b).
        cache
            .set(("udid-1".into(), "com.a".into()), InstallOutcome::Succeeded)
            .await;
        cache
            .set(("udid-2".into(), "com.a".into()), InstallOutcome::Succeeded)
            .await;
        cache
            .set(("udid-2".into(), "com.b".into()), InstallOutcome::Succeeded)
            .await;

        let slot = test_slot(&["app_a", "app_b"]);
        let matrix = vec![
            test_matrix_entry("app_a", "com.a"),
            test_matrix_entry("app_b", "com.b"),
        ];

        let pick = rank_by_install_cache(
            &free,
            Some(Platform::Ios),
            Some(&slot),
            Some(&cache),
            &matrix,
        )
        .await;
        assert_eq!(
            pick.udid, "udid-2",
            "SHALL prefer the device with the higher cache-hit count across all slot apps",
        );
    }

    #[tokio::test]
    async fn rank_without_slot_returns_first_candidate() {
        // 1. A None slot disables ranking entirely — first input wins.
        use golem_runner::installer::{InstallCache, InstallOutcome};
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];

        let cache = InstallCache::new();
        // Even though sim2 has a hit, a None slot SHALL ignore it.
        cache
            .set(
                ("udid-2".into(), "com.app".into()),
                InstallOutcome::Succeeded,
            )
            .await;
        let matrix = vec![test_matrix_entry("app", "com.app")];

        let pick =
            rank_by_install_cache(&free, Some(Platform::Ios), None, Some(&cache), &matrix).await;
        assert_eq!(
            pick.udid, "udid-1",
            "a None slot SHALL fall back to input order regardless of cache",
        );
    }

    #[tokio::test]
    async fn rank_no_matrix_match_scores_zero_returns_first() {
        // 2. When the install matrix has no entry for the slot's app, every
        //    device's bundle list is empty and is skipped — first wins.
        use golem_runner::installer::{InstallCache, InstallOutcome};
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];

        let cache = InstallCache::new();
        cache
            .set(
                ("udid-2".into(), "com.app".into()),
                InstallOutcome::Succeeded,
            )
            .await;

        // Slot references "app" but the matrix only knows "other".
        let slot = test_slot(&["app"]);
        let matrix = vec![test_matrix_entry("other", "com.app")];

        let pick = rank_by_install_cache(
            &free,
            Some(Platform::Ios),
            Some(&slot),
            Some(&cache),
            &matrix,
        )
        .await;
        assert_eq!(
            pick.udid, "udid-1",
            "no matrix match SHALL leave every score at 0 and return the first candidate",
        );
    }

    #[tokio::test]
    async fn rank_platform_filter_excludes_mismatched_matrix_entry() {
        // 3. With the platform pinned, an InstallEntry for a different platform
        //    SHALL NOT match — the iOS device's hit is invisible, score 0.
        use golem_runner::installer::{InstallCache, InstallOutcome};
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];

        let cache = InstallCache::new();
        cache
            .set(
                ("udid-2".into(), "com.app".into()),
                InstallOutcome::Succeeded,
            )
            .await;

        let slot = test_slot(&["app"]);
        // Matrix entry is Android-only; pinned iOS platform filters it out.
        let android_entry = InstallEntry {
            platform: Platform::Android,
            app_name: "app".to_string(),
            bundle_id: "com.app".to_string(),
            script_path: PathBuf::from("/tmp/noop.sh"),
            timeout_ms: 1000,
            device_constraints: Vec::new(),
        };
        let matrix = vec![android_entry];

        let pick = rank_by_install_cache(
            &free,
            Some(Platform::Ios),
            Some(&slot),
            Some(&cache),
            &matrix,
        )
        .await;
        assert_eq!(
            pick.udid, "udid-1",
            "platform filter SHALL exclude a cross-platform matrix entry, leaving score 0",
        );
    }

    #[tokio::test]
    async fn rank_none_platform_uses_each_device_own_platform() {
        // 4. When platform is None (mixed-platform pool), each device matches
        //    the matrix entry for its own platform.
        use golem_runner::installer::{InstallCache, InstallOutcome};
        let ios = test_device("iPhone 16", "udid-ios");
        let mut android = test_device("Pixel", "udid-android");
        android.platform = Platform::Android;
        let free = vec![&ios, &android];

        let cache = InstallCache::new();
        // Only the Android device has a Succeeded hit on its own bundle.
        cache
            .set(
                ("udid-android".into(), "com.app.android".into()),
                InstallOutcome::Succeeded,
            )
            .await;

        let slot = test_slot(&["app"]);
        let matrix = vec![
            test_matrix_entry("app", "com.app.ios"),
            InstallEntry {
                platform: Platform::Android,
                app_name: "app".to_string(),
                bundle_id: "com.app.android".to_string(),
                script_path: PathBuf::from("/tmp/noop.sh"),
                timeout_ms: 1000,
                device_constraints: Vec::new(),
            },
        ];

        let pick = rank_by_install_cache(&free, None, Some(&slot), Some(&cache), &matrix).await;
        assert_eq!(
            pick.udid, "udid-android",
            "with no pinned platform each device SHALL resolve bundles via its own platform",
        );
    }

    #[tokio::test]
    async fn rank_equal_scores_keeps_first_candidate() {
        // 5. Ties are broken by input order: both devices have one hit, so the
        //    first device SHALL win (score > best_score is strict).
        use golem_runner::installer::{InstallCache, InstallOutcome};
        let sim1 = test_device("iPhone 16", "udid-1");
        let sim2 = test_device("iPhone 16 Pro", "udid-2");
        let free = vec![&sim1, &sim2];

        let cache = InstallCache::new();
        cache
            .set(
                ("udid-1".into(), "com.app".into()),
                InstallOutcome::Succeeded,
            )
            .await;
        cache
            .set(
                ("udid-2".into(), "com.app".into()),
                InstallOutcome::Succeeded,
            )
            .await;

        let slot = test_slot(&["app"]);
        let matrix = vec![test_matrix_entry("app", "com.app")];

        let pick = rank_by_install_cache(
            &free,
            Some(Platform::Ios),
            Some(&slot),
            Some(&cache),
            &matrix,
        )
        .await;
        assert_eq!(
            pick.udid, "udid-1",
            "equal cache-hit scores SHALL be broken by input order (first wins)",
        );
    }

    #[tokio::test]
    async fn rank_single_candidate_returned_even_with_no_hits() {
        // 6. A one-element free pool always returns that element.
        use golem_runner::installer::InstallCache;
        let only = test_device("iPhone 16", "udid-only");
        let free = vec![&only];

        let cache = InstallCache::new();
        let slot = test_slot(&["app"]);
        let matrix = vec![test_matrix_entry("app", "com.app")];

        let pick = rank_by_install_cache(
            &free,
            Some(Platform::Ios),
            Some(&slot),
            Some(&cache),
            &matrix,
        )
        .await;
        assert_eq!(
            pick.udid, "udid-only",
            "a single-candidate pool SHALL return that candidate",
        );
    }
}
