//! Coverage strategy types and set-cover helpers used by the Plan and
//! (eventually) Smart scheduler.
//!
//! A "tick box" is a [`DeviceSlot`] with axis fields either pinned
//! (`Some(_)`) or open (`None`). A device `ticks` a box when every pinned
//! axis matches the device's attributes. The flow's coverage goal is to
//! tick every box at least once.
//!
//! Four strategies consume the tick-box pool differently:
//!
//! - `Full` — Cartesian: each box is fully pinned; one FlowRun per box.
//! - `Min` — plan-time greedy set-cover: fewest devices ticking every box.
//! - `Smart` — execute-time adaptive: one FlowRun per reachable pool
//!   entry, all sharing a [`crate::plan::CoverageGroup`]. The scheduler
//!   gates each spawn on live progress: the group stops dispatching
//!   once every pool index has been ticked. Bonus ticks (a picked
//!   device happens to satisfy extra pool entries) credit too.
//! - `One` — like Smart, but the group has `max_runs = Some(1)` so the
//!   first successful run ends dispatch for its siblings.

use golem_devices::DeviceInfo;

use crate::plan::{device_matches_slot, DeviceSlot};

/// Re-export of the parser's strategy enum under the orchestrator crate
/// so downstream code doesn't need `golem_parser` in scope just to name
/// the strategy.
pub use golem_parser::CoverageStrategy;

/// Greedy set-cover: pick the fewest devices whose combined ticks cover
/// every box in `boxes`. Returns the chosen device indices (into
/// `candidates`) in pick order.
///
/// Each round: pick the candidate that ticks the most still-uncovered
/// boxes (ties broken by lower index). Stops when no candidate ticks any
/// remaining box — those boxes go uncovered (caller decides whether that
/// is an error).
///
/// O(n × m × k) where n = |candidates|, m = |boxes|, k = picks. Fine for
/// the scales we hit in practice (< a few hundred of either).
pub fn set_cover_greedy(boxes: &[DeviceSlot], candidates: &[DeviceInfo]) -> Vec<usize> {
    use std::collections::HashSet;

    let mut remaining: HashSet<usize> = (0..boxes.len()).collect();
    let mut picked: Vec<usize> = Vec::new();
    let mut picked_set: HashSet<usize> = HashSet::new();

    while !remaining.is_empty() {
        let choice = (0..candidates.len())
            .filter(|i| !picked_set.contains(i))
            .map(|i| {
                let ticks: Vec<usize> = remaining
                    .iter()
                    .copied()
                    .filter(|b| device_matches_slot(&candidates[i], &boxes[*b]))
                    .collect();
                (i, ticks)
            })
            .filter(|(_, ticks)| !ticks.is_empty())
            .max_by_key(|(_, ticks)| ticks.len());

        match choice {
            Some((idx, ticks)) => {
                for b in ticks {
                    remaining.remove(&b);
                }
                picked.push(idx);
                picked_set.insert(idx);
            }
            None => break, // no candidate ticks any remaining box
        }
    }

    picked
}

/// Execute-time single-device pick: from a set of `free` candidates,
/// return the one that ticks the most still-uncovered boxes, along with
/// the box indices it ticks. `None` when no free candidate ticks any
/// remaining box — caller should wait for a device to free.
///
/// Available for schedulers that want to re-rank at spawn time (e.g. a
/// future "pick hottest uncovered axis first" extension). Today's Smart
/// scheduler uses the simpler credit-on-success model where any device
/// matching the slot suffices — bonus ticks are applied post-hoc.
pub fn pick_best_covering<'a>(
    boxes: &[DeviceSlot],
    remaining: &std::collections::HashSet<usize>,
    free: &'a [DeviceInfo],
) -> Option<(&'a DeviceInfo, Vec<usize>)> {
    free.iter()
        .map(|d| {
            let ticks: Vec<usize> = remaining
                .iter()
                .copied()
                .filter(|b| device_matches_slot(d, &boxes[*b]))
                .collect();
            (d, ticks)
        })
        .filter(|(_, ticks)| !ticks.is_empty())
        .max_by_key(|(_, ticks)| ticks.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_devices::{DeviceState, DeviceType, OsVersionSpec, Platform};

    fn device(name: &str, udid: &str, platform: Platform, major: u32, dt: DeviceType) -> DeviceInfo {
        DeviceInfo {
            name: name.into(),
            udid: udid.into(),
            platform,
            device_type: dt,
            os_major: major,
            os_version: format!("{major}.0"),
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

    fn empty_slot() -> DeviceSlot {
        DeviceSlot {
            platform: None,
            os_version: None,
            device_type: None,
            physical: None,
            name: None,
            playstore: None,
            accessibility_label: None,
            booted: None,
            apps: Vec::new(),
        }
    }

    fn os_box(platform: Platform, major: u32) -> DeviceSlot {
        DeviceSlot {
            os_version: Some(OsVersionSpec::Exact { platform, major }),
            platform: Some(platform),
            ..empty_slot()
        }
    }

    fn type_box(dt: DeviceType) -> DeviceSlot {
        DeviceSlot {
            device_type: Some(dt),
            ..empty_slot()
        }
    }

    // One device covers everything → 1 pick.
    #[test]
    fn set_cover_single_device_covers_all_axes() {
        let boxes = vec![
            os_box(Platform::Ios, 26),
            type_box(DeviceType::Tablet),
        ];
        let candidates = vec![device("iPad", "u1", Platform::Ios, 26, DeviceType::Tablet)];
        let picked = set_cover_greedy(&boxes, &candidates);
        assert_eq!(picked, vec![0], "one iPad v26 SHALL tick both boxes");
    }

    // Responsive-design case: 4 partial boxes, 2 devices cover all.
    #[test]
    fn set_cover_two_devices_cover_four_partial_boxes() {
        let boxes = vec![
            os_box(Platform::Ios, 26),
            os_box(Platform::Android, 34),
            type_box(DeviceType::Phone),
            type_box(DeviceType::Tablet),
        ];
        let candidates = vec![
            device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone),
            device("Pixel-tab", "u2", Platform::Android, 34, DeviceType::Tablet),
        ];
        let picked = set_cover_greedy(&boxes, &candidates);
        assert_eq!(picked.len(), 2, "two devices SHALL cover all 4 boxes");
        assert!(picked.contains(&0));
        assert!(picked.contains(&1));
    }

    // When the perfect picker isn't in the candidate list, covers what it can.
    #[test]
    fn set_cover_leaves_uncovered_when_no_candidate_matches() {
        let boxes = vec![os_box(Platform::Ios, 26), os_box(Platform::Ios, 18)];
        let candidates = vec![device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone)];
        let picked = set_cover_greedy(&boxes, &candidates);
        assert_eq!(picked, vec![0], "SHALL pick the v26 device; v18 box unreachable");
    }

    // Greedy picks the device that covers more first — tie-break by index.
    #[test]
    fn set_cover_picks_higher_coverage_first() {
        let boxes = vec![
            os_box(Platform::Ios, 26),
            type_box(DeviceType::Tablet),
            type_box(DeviceType::Phone),
        ];
        let candidates = vec![
            device("iPhone-18", "u-low", Platform::Ios, 18, DeviceType::Phone),
            device("iPad-26", "u-top", Platform::Ios, 26, DeviceType::Tablet),
        ];
        let picked = set_cover_greedy(&boxes, &candidates);
        // iPad-26 ticks {v26, tablet}; iPhone-18 ticks {phone}. Greedy picks iPad first.
        assert_eq!(picked[0], 1);
        assert_eq!(picked[1], 0);
        assert_eq!(picked.len(), 2);
    }

    // Empty input safety.
    #[test]
    fn set_cover_empty_boxes_picks_nothing() {
        let boxes: Vec<DeviceSlot> = Vec::new();
        let candidates = vec![device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone)];
        assert!(set_cover_greedy(&boxes, &candidates).is_empty());
    }

    #[test]
    fn set_cover_empty_candidates_leaves_all_uncovered() {
        let boxes = vec![os_box(Platform::Ios, 26)];
        let candidates: Vec<DeviceInfo> = Vec::new();
        assert!(set_cover_greedy(&boxes, &candidates).is_empty());
    }

    // pick_best_covering basics — used by Smart.
    #[test]
    fn pick_best_covering_picks_highest_tick_count() {
        let boxes = vec![
            os_box(Platform::Ios, 26),
            type_box(DeviceType::Tablet),
        ];
        let remaining: std::collections::HashSet<usize> = [0usize, 1].into_iter().collect();
        let free = vec![
            device("iPhone-26", "u-a", Platform::Ios, 26, DeviceType::Phone),  // ticks box 0
            device("iPad-26", "u-b", Platform::Ios, 26, DeviceType::Tablet),   // ticks boxes 0+1
        ];
        let pick = pick_best_covering(&boxes, &remaining, &free).expect("SHALL pick");
        assert_eq!(pick.0.udid, "u-b");
        assert_eq!(pick.1.len(), 2);
    }

    #[test]
    fn pick_best_covering_returns_none_when_no_device_ticks_remaining() {
        let boxes = vec![os_box(Platform::Android, 34)];
        let remaining: std::collections::HashSet<usize> = [0usize].into_iter().collect();
        let free = vec![device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone)];
        assert!(pick_best_covering(&boxes, &remaining, &free).is_none());
    }

    // No free candidates at all → nothing to pick.
    #[test]
    fn pick_best_covering_empty_free_returns_none() {
        let boxes = vec![os_box(Platform::Ios, 26)];
        let remaining: std::collections::HashSet<usize> = [0usize].into_iter().collect();
        let free: Vec<DeviceInfo> = Vec::new();
        assert!(
            pick_best_covering(&boxes, &remaining, &free).is_none(),
            "no free device SHALL yield no pick"
        );
    }

    // Nothing left to cover → every candidate ticks zero, so None.
    #[test]
    fn pick_best_covering_empty_remaining_returns_none() {
        let boxes = vec![os_box(Platform::Ios, 26)];
        let remaining: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let free = vec![device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone)];
        assert!(
            pick_best_covering(&boxes, &remaining, &free).is_none(),
            "no remaining box SHALL yield no pick even with a matching device"
        );
    }

    // An empty (all-axes-open) slot is ticked by any device; the returned
    // tick list SHALL name that box index.
    #[test]
    fn pick_best_covering_open_slot_ticked_by_any_device() {
        let boxes = vec![empty_slot()];
        let remaining: std::collections::HashSet<usize> = [0usize].into_iter().collect();
        let free = vec![device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone)];
        let pick = pick_best_covering(&boxes, &remaining, &free).expect("open slot SHALL match");
        assert_eq!(pick.0.udid, "u1");
        assert_eq!(pick.1, vec![0], "open slot SHALL be reported as ticked");
    }

    // A candidate that ticks nothing is never picked; greedy stops once the
    // only useful device is exhausted, leaving the unreachable box uncovered
    // and not looping forever.
    #[test]
    fn set_cover_skips_useless_candidate_and_terminates() {
        let boxes = vec![os_box(Platform::Ios, 26), os_box(Platform::Android, 34)];
        let candidates = vec![
            device("android-34", "u-droid", Platform::Android, 34, DeviceType::Phone),
            device("ios-18", "u-old", Platform::Ios, 18, DeviceType::Phone),
        ];
        let picked = set_cover_greedy(&boxes, &candidates);
        // Only the android device ticks anything (box 1); the ios-18 device
        // ticks neither box and is skipped; box 0 stays uncovered.
        assert_eq!(picked, vec![0], "SHALL pick only the android device");
    }

    // The picked_set guard: across multiple rounds a device is never picked
    // twice. Forces two rounds — round 1 picks the broadest device, round 2
    // must pick the other device for the box the first cannot tick — then
    // asserts no index repeats in the pick list.
    #[test]
    fn set_cover_never_picks_same_device_twice() {
        // 1. iPad-26 ticks {box0 ios26, box1 tablet}; Pixel-34 ticks {box2 android34}.
        //    No single device covers all three, so greedy runs two rounds.
        let boxes = vec![
            os_box(Platform::Ios, 26),
            type_box(DeviceType::Tablet),
            os_box(Platform::Android, 34),
        ];
        let candidates = vec![
            device("iPad-26", "u-pad", Platform::Ios, 26, DeviceType::Tablet),
            device("Pixel-34", "u-pix", Platform::Android, 34, DeviceType::Phone),
        ];
        let picked = set_cover_greedy(&boxes, &candidates);
        // 2. Round 1 picks the broadest (iPad, 2 ticks); round 2 picks Pixel.
        assert_eq!(picked, vec![0, 1], "SHALL pick iPad then Pixel across two rounds");
        // 3. No device index appears more than once — the picked_set guard held.
        let mut seen = std::collections::HashSet::new();
        for idx in &picked {
            assert!(seen.insert(*idx), "device {idx} SHALL be picked at most once");
        }
    }

    // An all-open box is ticked equally (1 tick) by every candidate. On a tie,
    // max_by_key returns the LAST maximal element, so greedy picks the highest
    // index — not the first candidate. Documents the tie-break ordering.
    #[test]
    fn set_cover_open_box_tie_break_picks_last() {
        let boxes = vec![empty_slot()];
        let candidates = vec![
            device("iPhone", "u1", Platform::Ios, 26, DeviceType::Phone),
            device("Pixel", "u2", Platform::Android, 34, DeviceType::Phone),
        ];
        let picked = set_cover_greedy(&boxes, &candidates);
        // 1. Both tick the open box once; max_by_key keeps the last max → index 1.
        assert_eq!(picked, vec![1], "equal-tick tie SHALL resolve to the last (highest-index) candidate");
    }
}
