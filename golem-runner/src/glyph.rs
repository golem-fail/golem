//! Tiny hand-rolled bitmap font for on-image annotation text.
//!
//! The a11y annotated screenshot needs to draw a marker number inside each
//! finding's rectangle (and, later, a compact detail token like `3.5:1` or
//! `32dp`). A real TTF would mean either a non-portable system-font lookup
//! (paths differ per OS, absent in CI containers) or vendoring a binary asset.
//! For our needs — a handful of glyphs, integer-scaled, deterministic in tests
//! — a bitmap font is simpler, dependency-free, and pixel-exact.
//!
//! Each glyph is a 5-wide × 8-tall cell. Rows are stored MSB-first in the low
//! 5 bits of a `u8` (bit 4 = leftmost column). Digits and `d` use the cap-
//! height rows 0–6; `p` uses the descender row 7. Glyphs not in the charset
//! (`0-9 . : d p` + space) render as blank. Icon glyphs (contrast/text-size)
//! are a deliberate follow-up — the table is the only place to add them.

use image::{Rgba, RgbaImage};

const CELL_W: u32 = 5;
const CELL_H: u32 = 8;
/// Columns of blank space between glyphs, in unscaled cells.
const TRACKING: u32 = 1;

/// 8 rows × 5 bits for one glyph, MSB = leftmost column.
fn glyph_rows(c: char) -> Option<[u8; 8]> {
    let rows = match c {
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110, 0b00000,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110, 0b00000,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111, 0b00000,
        ],
        '3' => [
            0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110, 0b00000,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010, 0b00000,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110, 0b00000,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110, 0b00000,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110, 0b00000,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100, 0b00000,
        ],
        '.' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00110, 0b00110, 0b00000,
        ],
        ':' => [
            0b00000, 0b00110, 0b00110, 0b00000, 0b00110, 0b00110, 0b00000, 0b00000,
        ],
        // lowercase d — right-side ascender (rows 0-1) + bowl (rows 2-6)
        'd' => [
            0b00001, 0b00001, 0b01111, 0b10001, 0b10001, 0b10001, 0b01111, 0b00000,
        ],
        // lowercase p — x-height bowl (rows 2-5) + left descender stem (rows 6-7)
        'p' => [
            0b00000, 0b00000, 0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000,
        ],
        // '?' doubles as the missing-label indicator ("what is this control?").
        '?' => [
            0b01110, 0b10001, 0b00001, 0b00110, 0b00100, 0b00000, 0b00100, 0b00000,
        ],
        ' ' => [0; 8],
        _ => return None,
    };
    Some(rows)
}

/// Width in pixels a string occupies at `scale` (glyph cells + tracking
/// between them, no trailing tracking).
pub fn text_width(text: &str, scale: u32) -> u32 {
    let n = text.chars().count() as u32;
    if n == 0 {
        return 0;
    }
    (n * CELL_W + (n - 1) * TRACKING) * scale
}

/// Height in pixels a glyph cell occupies at `scale`.
pub fn text_height(scale: u32) -> u32 {
    CELL_H * scale
}

/// Alpha-blend `color` over the existing pixel at (px, py). `color.0[3]` is
/// the source alpha (255 = opaque). Out-of-bounds writes are skipped.
fn blend_pixel(img: &mut RgbaImage, px: i32, py: i32, color: Rgba<u8>) {
    if px < 0 || py < 0 || px as u32 >= img.width() || py as u32 >= img.height() {
        return;
    }
    let a = color.0[3] as u32;
    if a == 0 {
        return;
    }
    if a == 255 {
        img.put_pixel(px as u32, py as u32, color);
        return;
    }
    let dst = img.get_pixel(px as u32, py as u32).0;
    let inv = 255 - a;
    let mix = |s: u8, d: u8| ((s as u32 * a + d as u32 * inv) / 255) as u8;
    img.put_pixel(
        px as u32,
        py as u32,
        Rgba([
            mix(color.0[0], dst[0]),
            mix(color.0[1], dst[1]),
            mix(color.0[2], dst[2]),
            255,
        ]),
    );
}

/// Draw `text` with its top-left at (x, y), each bitmap pixel expanded to a
/// `scale`×`scale` block, alpha-blended in `color`. Returns the advance width
/// in pixels (`text_width`). Unknown glyphs render blank but still advance.
pub fn draw_str(
    img: &mut RgbaImage,
    x: i32,
    y: i32,
    scale: u32,
    color: Rgba<u8>,
    text: &str,
) -> u32 {
    let scale = scale.max(1);
    let mut cx = x;
    for c in text.chars() {
        if let Some(rows) = glyph_rows(c) {
            for (ry, bits) in rows.iter().enumerate() {
                for col in 0..CELL_W {
                    // bit (CELL_W-1-col) is column `col` (MSB-first).
                    if bits & (1 << (CELL_W - 1 - col)) != 0 {
                        for dy in 0..scale {
                            for dx in 0..scale {
                                blend_pixel(
                                    img,
                                    cx + (col * scale + dx) as i32,
                                    y + (ry as u32 * scale + dy) as i32,
                                    color,
                                );
                            }
                        }
                    }
                }
            }
        }
        cx += ((CELL_W + TRACKING) * scale) as i32;
    }
    text_width(text, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 255]))
    }

    #[test]
    fn width_and_height_math() {
        // "12" at scale 2: 2 cells (5px) + 1 tracking (1px) = 11 cells × 2 = 22
        assert_eq!(text_width("12", 2), 22);
        assert_eq!(text_width("", 2), 0);
        assert_eq!(text_width("1", 3), 15);
        assert_eq!(text_height(2), 16);
    }

    #[test]
    fn draws_some_ink_for_known_glyph() {
        let mut img = blank(40, 16);
        let white = Rgba([255, 255, 255, 255]);
        draw_str(&mut img, 0, 0, 1, white, "1");
        let lit = img.pixels().filter(|p| p.0[0] == 255).count();
        assert!(lit > 0, "digit '1' should light some pixels");
    }

    #[test]
    fn question_mark_renders() {
        let mut img = blank(40, 16);
        let white = Rgba([255, 255, 255, 255]);
        draw_str(&mut img, 0, 0, 1, white, "?");
        assert!(
            img.pixels().any(|p| p.0[0] == 255),
            "'?' should light pixels"
        );
    }

    #[test]
    fn space_and_unknown_draw_nothing() {
        let mut img = blank(40, 16);
        let white = Rgba([255, 255, 255, 255]);
        let adv = draw_str(&mut img, 0, 0, 1, white, " ~");
        assert!(
            img.pixels().all(|p| p.0[0] == 0),
            "blank glyphs leave no ink"
        );
        // both glyphs still advance.
        assert_eq!(adv, text_width(" ~", 1));
    }

    #[test]
    fn alpha_blends_halfway() {
        let mut img = blank(8, 8);
        // 50% white over black ≈ 127.
        draw_str(&mut img, 0, 0, 1, Rgba([255, 255, 255, 128]), "1");
        let blended = img.pixels().find(|p| p.0[0] > 0).expect("some ink");
        assert!(
            (120..=135).contains(&blended.0[0]),
            "expected ~half blend, got {}",
            blended.0[0]
        );
    }

    #[test]
    fn out_of_bounds_is_safe() {
        let mut img = blank(4, 4);
        // Draw partly off the right/bottom edge — must not panic.
        draw_str(&mut img, 2, 2, 2, Rgba([255, 255, 255, 255]), "8");
    }
}
