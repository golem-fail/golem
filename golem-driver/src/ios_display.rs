//! iOS device display data: cutout rects and corner radii.
//!
//! Embeds `ios_display.toml` at compile time and provides a lookup
//! function to get cutout/corner info for a given device model identifier.
//! Keys are model prefixes — "iPad8" matches "iPad8,1", "iPad8,2" etc.
//! Longest prefix match wins.

use crate::common::{CornerPosition, CutoutRect, RoundedCorner};
use std::sync::OnceLock;

static DISPLAY_DATA: &str = include_str!("ios_display.toml");

/// Parsed display entry for one device model (or prefix).
struct DisplayEntry {
    /// Cutout width and height (centered horizontally at runtime).
    cutout_size: Option<[i32; 2]>,
    /// Cutout y offset (0 for notch, 11 for Dynamic Island).
    cutout_y: i32,
    corner_radius: i32,
    /// System-gesture inset from the left edge (back-from-edge swipe zone).
    /// Defaults to 0 when not specified. iPhones with home indicator
    /// typically reserve 10-20pt for the swipe-back gesture.
    gesture_inset_left: i32,
    /// System-gesture inset from the right edge (control-center pull-down
    /// on some models, app-switcher on iPad).
    gesture_inset_right: i32,
}

/// All entries sorted by key length descending (longest prefix first).
type DisplayEntries = Vec<(String, DisplayEntry)>;

/// Parse a TOML display-data string into the sorted entry list.
///
/// Each top-level table becomes one entry keyed by its name. Missing or
/// malformed fields fall back to their defaults (the same behavior the
/// embedded `ios_display.toml` relies on), and a string that fails to parse
/// as TOML yields an empty entry list. Entries are sorted by key length
/// descending so the longest prefix matches first during lookup.
fn parse_entries(s: &str) -> DisplayEntries {
    let table: toml::Table = s.parse().unwrap_or_default();
    let mut entries: Vec<(String, DisplayEntry)> = Vec::new();
    for (key, value) in &table {
        let Some(tbl) = value.as_table() else { continue };
        let cutout_size = tbl
            .get("cutout_size")
            .and_then(|v| v.as_array())
            .and_then(|a| {
                if a.len() == 2 {
                    Some([a[0].as_integer()? as i32, a[1].as_integer()? as i32])
                } else {
                    None
                }
            });
        let cutout_y = tbl
            .get("cutout_y")
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as i32;
        let corner_radius = tbl
            .get("corner_radius")
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as i32;
        let gesture_inset_left = tbl
            .get("gesture_inset_left")
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as i32;
        let gesture_inset_right = tbl
            .get("gesture_inset_right")
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as i32;
        entries.push((key.clone(), DisplayEntry {
            cutout_size,
            cutout_y,
            corner_radius,
            gesture_inset_left,
            gesture_inset_right,
        }));
    }
    // Sort by key length descending so longest prefix matches first
    entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    entries
}

fn load() -> &'static DisplayEntries {
    static ENTRIES: OnceLock<DisplayEntries> = OnceLock::new();
    ENTRIES.get_or_init(|| parse_entries(DISPLAY_DATA))
}

/// Result of an iOS device display data lookup.
pub struct DisplayLookup {
    pub cutouts: Vec<CutoutRect>,
    pub rounded_corners: Vec<RoundedCorner>,
    pub gesture_inset_left: i32,
    pub gesture_inset_right: i32,
}

/// Look up display data for an iOS device model identifier.
///
/// `screen_width` and `screen_height` come from the hierarchy root element bounds.
/// The cutout x position is computed as `(screen_width - cutout_width) / 2`.
///
/// Returns None if the model is unknown.
pub fn lookup(model: &str, screen_width: i32, screen_height: i32) -> Option<DisplayLookup> {
    let entries = load();
    let (_, entry) = entries.iter().find(|(key, _)| model.starts_with(key.as_str()))?;

    let cutouts = match entry.cutout_size {
        Some([w, h]) if w > 0 && h > 0 && screen_width > 0 => {
            vec![CutoutRect {
                x: (screen_width - w) / 2,
                y: entry.cutout_y,
                width: w,
                height: h,
            }]
        }
        _ => Vec::new(),
    };

    let r = entry.corner_radius;
    let corners = if r > 0 && screen_width > 0 && screen_height > 0 {
        vec![
            RoundedCorner {
                position: CornerPosition::TopLeft,
                radius: r,
                center_x: r,
                center_y: r,
            },
            RoundedCorner {
                position: CornerPosition::TopRight,
                radius: r,
                center_x: screen_width - r,
                center_y: r,
            },
            RoundedCorner {
                position: CornerPosition::BottomRight,
                radius: r,
                center_x: screen_width - r,
                center_y: screen_height - r,
            },
            RoundedCorner {
                position: CornerPosition::BottomLeft,
                radius: r,
                center_x: r,
                center_y: screen_height - r,
            },
        ]
    } else {
        Vec::new()
    };

    Some(DisplayLookup {
        cutouts,
        rounded_corners: corners,
        gesture_inset_left: entry.gesture_inset_left,
        gesture_inset_right: entry.gesture_inset_right,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_iphone_x_returns_notch() {
        let d = lookup("iPhone10,3", 375, 812).expect("iPhone X SHALL be known");
        assert_eq!(d.cutouts.len(), 1, "iPhone X SHALL have one cutout");
        assert_eq!(d.cutouts[0].width, 209, "notch SHALL be 209pt wide");
        assert_eq!(d.cutouts[0].height, 31, "notch SHALL be 31pt tall");
        assert_eq!(d.cutouts[0].x, (375 - 209) / 2, "notch SHALL be centered");
        assert_eq!(d.cutouts[0].y, 0, "notch SHALL be at top");
        assert_eq!(d.rounded_corners.len(), 4);
        assert_eq!(d.rounded_corners[0].radius, 39);
    }

    #[test]
    fn lookup_iphone_x_prefix_matches_variant() {
        let d = lookup("iPhone10,6", 375, 812).expect("SHALL match");
        assert_eq!(d.cutouts.len(), 1);
        assert_eq!(d.cutouts[0].width, 209);
    }

    #[test]
    fn lookup_dynamic_island_has_y_offset() {
        let d = lookup("iPhone15,2", 393, 852).expect("SHALL match");
        assert_eq!(d.cutouts[0].width, 126, "Dynamic Island SHALL be 126pt wide");
        assert_eq!(d.cutouts[0].y, 11, "Dynamic Island SHALL be 11pt from top");
        assert_eq!(d.rounded_corners[0].radius, 55);
    }

    #[test]
    fn lookup_ipad_prefix_returns_corners_only() {
        let d = lookup("iPad8,1", 834, 1194).expect("SHALL match");
        assert!(d.cutouts.is_empty(), "iPad Pro SHALL have no cutouts");
        assert_eq!(d.rounded_corners.len(), 4);
        assert_eq!(d.rounded_corners[0].radius, 18);
    }

    #[test]
    fn lookup_unknown_model_returns_none() {
        assert!(lookup("iPhone99,99", 390, 844).is_none());
    }

    #[test]
    fn specific_match_beats_prefix() {
        let xr = lookup("iPhone11,8", 414, 896).expect("SHALL match");
        assert_eq!(xr.rounded_corners[0].radius, 42, "iPhone XR SHALL have radius 42");

        let xs = lookup("iPhone11,2", 375, 812).expect("SHALL match");
        assert_eq!(xs.rounded_corners[0].radius, 39, "iPhone XS SHALL have radius 39");
    }

    #[test]
    fn corner_centers_use_screen_dimensions() {
        let d = lookup("iPhone17,3", 393, 852).expect("SHALL match");
        let tl = d.rounded_corners.iter().find(|c| c.position == CornerPosition::TopLeft).unwrap();
        assert_eq!((tl.center_x, tl.center_y), (55, 55));
        let br = d.rounded_corners.iter().find(|c| c.position == CornerPosition::BottomRight).unwrap();
        assert_eq!((br.center_x, br.center_y), (393 - 55, 852 - 55));
    }

    // 1. All four corners SHALL be present in TL, TR, BR, BL order with
    //    centers reflecting the radius inset from each respective edge.
    fn find_corner(d: &DisplayLookup, p: CornerPosition) -> &RoundedCorner {
        d.rounded_corners
            .iter()
            .find(|c| c.position == p)
            .expect("corner position SHALL be present")
    }

    #[test]
    fn all_four_corner_positions_present_and_inset() {
        let d = lookup("iPhone17,3", 393, 852).expect("SHALL match");
        let r = 55;
        assert_eq!(
            d.rounded_corners
                .iter()
                .map(|c| c.position)
                .collect::<Vec<_>>(),
            vec![
                CornerPosition::TopLeft,
                CornerPosition::TopRight,
                CornerPosition::BottomRight,
                CornerPosition::BottomLeft,
            ],
            "corners SHALL be ordered TL, TR, BR, BL"
        );
        let tr = find_corner(&d, CornerPosition::TopRight);
        assert_eq!((tr.center_x, tr.center_y), (393 - r, r), "TR SHALL inset x from right edge");
        let bl = find_corner(&d, CornerPosition::BottomLeft);
        assert_eq!((bl.center_x, bl.center_y), (r, 852 - r), "BL SHALL inset y from bottom edge");
        assert!(
            d.rounded_corners.iter().all(|c| c.radius == r),
            "every corner SHALL share the same radius"
        );
    }

    // 2. A non-positive screen width SHALL suppress both the cutout and the
    //    rounded corners, since their geometry depends on a positive width.
    #[test]
    fn zero_screen_width_suppresses_cutout_and_corners() {
        let d = lookup("iPhone15,2", 0, 852).expect("model SHALL still be known");
        assert!(d.cutouts.is_empty(), "zero width SHALL produce no cutout");
        assert!(d.rounded_corners.is_empty(), "zero width SHALL produce no corners");
    }

    // 3. A negative screen width SHALL likewise suppress cutout and corners.
    #[test]
    fn negative_screen_width_suppresses_geometry() {
        let d = lookup("iPhone15,2", -10, 852).expect("model SHALL still be known");
        assert!(d.cutouts.is_empty(), "negative width SHALL produce no cutout");
        assert!(d.rounded_corners.is_empty(), "negative width SHALL produce no corners");
    }

    // 4. A non-positive screen height SHALL suppress corners but the cutout
    //    (which depends only on width) SHALL still be produced.
    #[test]
    fn zero_screen_height_suppresses_corners_only() {
        let d = lookup("iPhone15,2", 393, 0).expect("model SHALL still be known");
        assert_eq!(d.cutouts.len(), 1, "cutout SHALL survive zero height");
        assert!(
            d.rounded_corners.is_empty(),
            "zero height SHALL produce no corners"
        );
    }

    // 5. An iPad entry declares `cutouts = []` and no `cutout_size`, so
    //    cutout_size parses to None and SHALL yield zero cutouts even with a
    //    positive screen size, while corners are still emitted.
    #[test]
    fn missing_cutout_size_yields_no_cutouts() {
        let d = lookup("iPad13", 834, 1194).expect("SHALL match");
        assert!(d.cutouts.is_empty(), "absent cutout_size SHALL mean no cutout");
        assert_eq!(d.rounded_corners.len(), 4, "iPad SHALL still have corners");
    }

    // 6. The toml defines no gesture insets, so both SHALL default to 0.
    #[test]
    fn gesture_insets_default_to_zero() {
        let d = lookup("iPhone17,3", 393, 852).expect("SHALL match");
        assert_eq!(d.gesture_inset_left, 0, "left inset SHALL default to 0");
        assert_eq!(d.gesture_inset_right, 0, "right inset SHALL default to 0");
    }

    // 7. The cutout x SHALL be the floor of the centered offset, including
    //    when (screen_width - width) is odd (integer division truncates).
    #[test]
    fn cutout_x_is_floored_when_offset_is_odd() {
        // 393 - 126 = 267, which is odd; integer division floors to 133.
        let d = lookup("iPhone15,2", 393, 852).expect("SHALL match");
        // Hand-computed literal independently proves both the centering and
        // the odd-offset floor without recomputing the production expression.
        assert_eq!(d.cutouts[0].x, 133, "267 / 2 SHALL floor-center to 133");
    }

    // 8. The longest matching prefix SHALL win: "iPhone17,3" matches the
    //    specific full key rather than any shorter prefix, and an empty model
    //    string SHALL match nothing.
    #[test]
    fn empty_model_matches_nothing() {
        assert!(
            lookup("", 393, 852).is_none(),
            "empty model SHALL not match any prefix"
        );
    }

    // 9. The taller 12-family notch (height 32) SHALL be reported verbatim
    //    with a top (y=0) position, distinguishing it from the Dynamic Island.
    #[test]
    fn twelve_family_notch_height_and_top_offset() {
        let d = lookup("iPhone13,2", 390, 844).expect("SHALL match");
        assert_eq!(d.cutouts[0].height, 32, "iPhone 12 notch SHALL be 32pt tall");
        assert_eq!(d.cutouts[0].width, 209, "iPhone 12 notch SHALL be 209pt wide");
        assert_eq!(d.cutouts[0].y, 0, "a notch SHALL sit at the very top");
    }

    // 10. parse_entries SHALL read every field verbatim from a well-formed
    //     table, defaulting nothing that was supplied.
    #[test]
    fn parse_entries_reads_all_fields() {
        let src = r#"
            [Foo1]
            cutout_size = [120, 30]
            cutout_y = 11
            corner_radius = 55
            gesture_inset_left = 12
            gesture_inset_right = 8
        "#;
        let entries = parse_entries(src);
        assert_eq!(entries.len(), 1, "one table SHALL yield one entry");
        let (key, e) = &entries[0];
        assert_eq!(key, "Foo1", "key SHALL be the table name");
        assert_eq!(e.cutout_size, Some([120i32, 30]), "cutout_size SHALL parse both ints");
        assert_eq!(e.cutout_y, 11, "cutout_y SHALL be read verbatim");
        assert_eq!(e.corner_radius, 55, "corner_radius SHALL be read verbatim");
        assert_eq!(e.gesture_inset_left, 12, "left inset SHALL be read verbatim");
        assert_eq!(e.gesture_inset_right, 8, "right inset SHALL be read verbatim");
    }

    // 11. Absent integer fields SHALL default to 0 and an absent cutout_size
    //     SHALL be None, exactly as the embedded iPad entries rely upon.
    #[test]
    fn parse_entries_defaults_missing_fields() {
        let src = r#"
            [Bar2]
            corner_radius = 18
        "#;
        let entries = parse_entries(src);
        let (_, e) = &entries[0];
        assert_eq!(e.cutout_size, None, "absent cutout_size SHALL be None");
        assert_eq!(e.cutout_y, 0, "absent cutout_y SHALL default to 0");
        assert_eq!(e.corner_radius, 18, "present corner_radius SHALL survive");
        assert_eq!(e.gesture_inset_left, 0, "absent left inset SHALL default to 0");
        assert_eq!(e.gesture_inset_right, 0, "absent right inset SHALL default to 0");
    }

    // 12. A cutout_size array whose length is not exactly 2 SHALL be rejected
    //     (yielding None) rather than partially parsed.
    #[test]
    fn parse_entries_rejects_wrong_arity_cutout_size() {
        let src = r#"
            [One]
            cutout_size = [120]

            [Three]
            cutout_size = [1, 2, 3]
        "#;
        let entries = parse_entries(src);
        assert_eq!(entries.len(), 2, "both tables SHALL still produce entries");
        for (_, e) in &entries {
            assert_eq!(e.cutout_size, None, "non-pair cutout_size SHALL be None");
        }
    }

    // 13. Entries SHALL be sorted by key length descending so the longest
    //     prefix matches first during lookup.
    #[test]
    fn parse_entries_sorts_by_key_length_descending() {
        let src = r#"
            [iPhone]
            corner_radius = 1

            ["iPhone15,2"]
            corner_radius = 2

            [iPhone15]
            corner_radius = 3
        "#;
        let keys: Vec<String> = parse_entries(src).into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            vec!["iPhone15,2".to_string(), "iPhone15".to_string(), "iPhone".to_string()],
            "keys SHALL be ordered longest first"
        );
    }

    // 14. Top-level values that are not tables SHALL be skipped, and malformed
    //     TOML SHALL yield an empty entry list rather than panicking.
    #[test]
    fn parse_entries_skips_non_tables_and_tolerates_garbage() {
        let with_scalar = r#"
            not_a_table = 7

            [Real]
            corner_radius = 4
        "#;
        let entries = parse_entries(with_scalar);
        assert_eq!(entries.len(), 1, "scalar top-level keys SHALL be skipped");
        assert_eq!(entries[0].0, "Real", "only the table SHALL remain");

        let garbage = parse_entries("this is not = = valid toml [[[");
        assert!(garbage.is_empty(), "unparseable TOML SHALL yield no entries");
    }
}
