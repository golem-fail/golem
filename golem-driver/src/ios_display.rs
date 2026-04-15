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
}

/// All entries sorted by key length descending (longest prefix first).
type DisplayEntries = Vec<(String, DisplayEntry)>;

fn load() -> &'static DisplayEntries {
    static ENTRIES: OnceLock<DisplayEntries> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let table: toml::Table = DISPLAY_DATA.parse().unwrap_or_default();
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
            entries.push((key.clone(), DisplayEntry { cutout_size, cutout_y, corner_radius }));
        }
        // Sort by key length descending so longest prefix matches first
        entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        entries
    })
}

/// Look up cutout rects and rounded corners for an iOS device model identifier.
///
/// `screen_width` and `screen_height` come from the hierarchy root element bounds.
/// The cutout x position is computed as `(screen_width - cutout_width) / 2`.
///
/// Returns `(cutouts, rounded_corners)`. Both are empty if the model is unknown.
pub fn lookup(model: &str, screen_width: i32, screen_height: i32) -> (Vec<CutoutRect>, Vec<RoundedCorner>) {
    let entries = load();
    let entry = entries.iter().find(|(key, _)| model.starts_with(key.as_str()));
    let Some((_, entry)) = entry else {
        return (Vec::new(), Vec::new());
    };

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

    (cutouts, corners)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_iphone_x_returns_notch() {
        let (cutouts, corners) = lookup("iPhone10,3", 375, 812);
        assert_eq!(cutouts.len(), 1, "iPhone X SHALL have one cutout");
        assert_eq!(cutouts[0].width, 209, "notch SHALL be 209pt wide");
        assert_eq!(cutouts[0].height, 31, "notch SHALL be 31pt tall");
        assert_eq!(cutouts[0].x, (375 - 209) / 2, "notch SHALL be centered");
        assert_eq!(cutouts[0].y, 0, "notch SHALL be at top");
        assert_eq!(corners.len(), 4);
        assert_eq!(corners[0].radius, 39);
    }

    #[test]
    fn lookup_iphone_x_prefix_matches_variant() {
        let (cutouts, _) = lookup("iPhone10,6", 375, 812);
        assert_eq!(cutouts.len(), 1, "prefix match SHALL work for variant");
        assert_eq!(cutouts[0].width, 209);
    }

    #[test]
    fn lookup_dynamic_island_has_y_offset() {
        let (cutouts, corners) = lookup("iPhone15,2", 393, 852);
        assert_eq!(cutouts.len(), 1);
        assert_eq!(cutouts[0].width, 126, "Dynamic Island SHALL be 126pt wide");
        assert_eq!(cutouts[0].y, 11, "Dynamic Island SHALL be 11pt from top");
        assert_eq!(cutouts[0].x, (393 - 126) / 2, "SHALL be centered");
        assert_eq!(corners[0].radius, 55);
    }

    #[test]
    fn lookup_ipad_prefix_returns_corners_only() {
        let (cutouts, corners) = lookup("iPad8,1", 834, 1194);
        assert!(cutouts.is_empty(), "iPad Pro SHALL have no cutouts");
        assert_eq!(corners.len(), 4);
        assert_eq!(corners[0].radius, 18);
    }

    #[test]
    fn lookup_ipad13_prefix_matches() {
        let (cutouts, corners) = lookup("iPad13,4", 834, 1194);
        assert!(cutouts.is_empty());
        assert_eq!(corners[0].radius, 18);
    }

    #[test]
    fn lookup_unknown_model_returns_empty() {
        let (cutouts, corners) = lookup("iPhone99,99", 390, 844);
        assert!(cutouts.is_empty());
        assert!(corners.is_empty());
    }

    #[test]
    fn specific_match_beats_prefix() {
        // "iPhone11,8" (XR, r=42) is more specific than any "iPhone11" prefix
        let (_, corners) = lookup("iPhone11,8", 414, 896);
        assert_eq!(corners[0].radius, 42, "iPhone XR SHALL have radius 42");

        let (_, corners) = lookup("iPhone11,2", 375, 812);
        assert_eq!(corners[0].radius, 39, "iPhone XS SHALL have radius 39");
    }

    #[test]
    fn corner_centers_use_screen_dimensions() {
        let (_, corners) = lookup("iPhone17,3", 393, 852);
        let tl = corners.iter().find(|c| c.position == CornerPosition::TopLeft).unwrap();
        assert_eq!((tl.center_x, tl.center_y), (55, 55));
        let br = corners.iter().find(|c| c.position == CornerPosition::BottomRight).unwrap();
        assert_eq!((br.center_x, br.center_y), (393 - 55, 852 - 55));
    }
}
