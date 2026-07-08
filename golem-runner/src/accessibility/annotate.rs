//! Annotation/reporting half of the accessibility module: draws numbered
//! finding markers, rects, and dimension lines onto screenshots, embeds a
//! self-describing audit record in the PNG, and reads it back out.
//!
//! Split out of [`crate::accessibility`] on a clean seam — the audit/analysis
//! half stays there; this half is pure rendering + PNG metadata I/O.

use golem_element::Viewport;
use golem_events::{A11yIssue, Rect, Severity};

use super::screenshot_scale;

/// Run + device context embedded in the annotated PNG's metadata so the image
/// is a self-describing artifact (shareable standalone — extractable by any PNG
/// tool, and enough to replay via `seed`).
pub struct A11yMeta {
    pub app: String,
    pub device: String,
    /// Platform string (`ios`/`android`) — lets `a11y-extract` emit an exact
    /// `--platform` in the replay command.
    pub platform: String,
    pub flow: String,
    pub block: String,
    pub iteration: u32,
    pub seed: u64,
    pub level: String,
}

/// Annotate the screenshot with one numbered finding per issue and re-encode
/// as PNG. `viewport` maps element bounds (points/dp) to screenshot px; `meta`
/// is embedded as iTXt metadata.
///
/// Visual channels keep findings legible when several land on one element:
/// - **rect** per element (orange=warning drawn first, red=error on top).
/// - **marker** = the issue's 1-based index (canonical order shared with every
///   report surface). Single-element findings put it at the top-left corner
///   (colliding corners cascade right); grouped findings put one marker at the
///   group centroid.
/// - **size checks** (`touch_target_too_small`, `text_too_small`) draw an
///   industrial dimension line on the limiting axis with the measurement —
///   off the corner, so it never collides with the contrast token.
/// - **`low_contrast`** draws its ratio token semi-translucent bottom-left.
/// - **grouped findings** (`duplicate_labels`, `overlapping_interactive`) draw
///   a rect on every member; duplicates also connect members with dashed lines.
///
/// The PNG carries three iTXt chunks: `Software` (`Golem`), a human
/// `Golem-Summary` one-liner, and `Golem-Audit` — a JSON record of the context
/// plus every finding (with screenshot-pixel `bounds` for hover tooling).
pub fn annotate_screenshot(
    screenshot_png: &[u8],
    issues: &[A11yIssue],
    viewport: &Viewport,
    meta: &A11yMeta,
) -> anyhow::Result<Vec<u8>> {
    let img = image::load_from_memory(screenshot_png)?;
    annotate_image(&img, issues, viewport, meta)
}

/// Annotate an already-decoded image — lets the block-end audit decode the
/// screenshot once and share it between the contrast check and the annotator.
pub(crate) fn annotate_image(
    img: &image::DynamicImage,
    issues: &[A11yIssue],
    viewport: &Viewport,
    meta: &A11yMeta,
) -> anyhow::Result<Vec<u8>> {
    use image::Rgba;

    let mut img = img.to_rgba8();
    let sf = screenshot_scale(img.width(), viewport.width);
    let red = Rgba([220u8, 40, 40, 255]);
    let orange = Rgba([240u8, 150, 30, 255]);
    let white = Rgba([255u8, 255, 255, 255]);
    // Marker glyph scale, sized off the screenshot so numbers stay legible on
    // both phone (~1170px) and tablet (~2048px) captures.
    let gscale = (img.width() / 350).clamp(2, 5);
    let img_w = img.width() as i32;
    let img_h = img.height() as i32;

    let scale_rect = |r: &Rect| -> (i32, i32, i32, i32) {
        (
            (r.x as f64 * sf).round() as i32,
            (r.y as f64 * sf).round() as i32,
            (r.width as f64 * sf).round().max(1.0) as i32,
            (r.height as f64 * sf).round().max(1.0) as i32,
        )
    };

    // Top-left corner markers cascade right when they collide; shared across
    // both passes so numbering layout is stable regardless of severity order.
    let mut corner_shift: std::collections::HashMap<(i32, i32), i32> =
        std::collections::HashMap::new();

    // Warnings first, errors last → red drawn on top.
    for pass in [Severity::Warning, Severity::Error] {
        let color = if pass == Severity::Error { red } else { orange };
        for (idx, issue) in issues.iter().enumerate() {
            if issue.severity != pass {
                continue;
            }
            let Some(pb) = issue.element_bounds else {
                continue;
            };
            let n = idx + 1;
            let primary = scale_rect(&pb);
            let members: Vec<(i32, i32, i32, i32)> = std::iter::once(primary)
                .chain(issue.related_bounds.iter().map(scale_rect))
                .collect();

            for m in &members {
                draw_rect(&mut img, *m, color);
            }

            let chip_h = (crate::glyph::text_height(gscale) + 2 * gscale) as i32;
            let chip_w = marker_chip_width(n, gscale) as i32;
            if members.len() > 1 && issue.check_id == "duplicate_labels" {
                // Connect group members with dashed lines (nearest corners) and
                // stamp the marker on EACH segment — for a triplicate that's the
                // same number on both links, making the grouping unmistakable.
                for w in members.windows(2) {
                    let (p, q) = nearest_corners(w[0], w[1]);
                    draw_dashed_line(&mut img, p, q, color, gscale);
                    let mx = ((p.0 + q.0) / 2.0) as i32;
                    let my = ((p.1 + q.1) / 2.0) as i32;
                    draw_marker(
                        &mut img,
                        mx - chip_w / 2,
                        my - chip_h / 2,
                        gscale,
                        color,
                        white,
                        n,
                    );
                }
            } else if members.len() > 1 {
                // Other grouped findings (overlapping): both rects already
                // intersect, so a connector would be degenerate — one centred
                // marker over the group.
                let (cx, cy) = centroid(&members);
                draw_marker(
                    &mut img,
                    cx - chip_w / 2,
                    cy - chip_h / 2,
                    gscale,
                    color,
                    white,
                    n,
                );
            } else {
                let (x, y, _, _) = primary;
                let shift = corner_shift.entry((x, y)).or_insert(0);
                let chip_x = (x + *shift).min((img_w - chip_w).max(0));
                *shift += chip_w + gscale as i32;
                draw_marker(&mut img, chip_x, y, gscale, color, white, n);
            }

            // Measurement channel.
            let label = issue.detail.clone().unwrap_or_default();
            match issue.check_id.as_str() {
                "text_too_small" => {
                    // Dimension anchors to the measured text line when the pixel
                    // pass set one (padded / multi-line); the box-height pass
                    // leaves it None → spans the whole box. Left side — text is
                    // usually left-aligned (touch-target uses the right).
                    let (x, y, w, h) = issue
                        .measure_bounds
                        .as_ref()
                        .map(&scale_rect)
                        .unwrap_or(primary);
                    draw_v_dimension(
                        &mut img,
                        DimensionRect {
                            rect_x: x,
                            rect_y: y,
                            rect_w: w,
                            rect_h: h,
                            color,
                            label: &label,
                            scale: gscale,
                        },
                        img_w,
                        false,
                    );
                }
                "touch_target_too_small" => {
                    let (x, y, w, h) = primary;
                    // Dimension the limiting (smaller) axis — that's what failed.
                    // Height-limited → vertical on the RIGHT; width-limited → below.
                    if h <= w {
                        draw_v_dimension(
                            &mut img,
                            DimensionRect {
                                rect_x: x,
                                rect_y: y,
                                rect_w: w,
                                rect_h: h,
                                color,
                                label: &label,
                                scale: gscale,
                            },
                            img_w,
                            true,
                        );
                    } else {
                        draw_h_dimension(
                            &mut img,
                            DimensionRect {
                                rect_x: x,
                                rect_y: y,
                                rect_w: w,
                                rect_h: h,
                                color,
                                label: &label,
                                scale: gscale,
                            },
                            img_h,
                        );
                    }
                }
                "low_contrast" => {
                    if let Some(d) = &issue.detail {
                        let (x, y, _, h) = primary;
                        let ds = gscale.saturating_sub(1).max(2);
                        let dh = crate::glyph::text_height(ds) as i32;
                        let faint = Rgba([color.0[0], color.0[1], color.0[2], 180]);
                        // +2px clears the rect's 2px border so the token isn't
                        // flush against it.
                        crate::glyph::draw_str(
                            &mut img,
                            x + gscale as i32 + 2,
                            y + h - dh - gscale as i32,
                            ds,
                            faint,
                            d,
                        );
                    }
                }
                "occluded_element" => {
                    // A 3×3 mini-map at the bottom-right showing the *pattern* of
                    // occlusion — which sampled zones are covered (solid) vs
                    // reachable (faint outline); untested zones stay blank.
                    let cells: Vec<OcclusionCell> = issue
                        .occlusion
                        .iter()
                        .map(|c| (scale_rect(&c.bounds), c.reachable))
                        .collect();
                    draw_occlusion_minimap(&mut img, primary, &cells, gscale, color);
                }
                "missing_label" => {
                    // No measurement to show — mark it with a "?" in the
                    // bottom-right (its "what is this control?" indicator), so a
                    // label-less control reads as deliberately flagged, not bare.
                    let (x, y, w, h) = primary;
                    let q = "?";
                    let qw = crate::glyph::text_width(q, gscale) as i32;
                    let qh = crate::glyph::text_height(gscale) as i32;
                    let m = gscale as i32 + 2;
                    crate::glyph::draw_str(
                        &mut img,
                        x + w - qw - m,
                        y + h - qh - m,
                        gscale,
                        color,
                        q,
                    );
                }
                _ => {}
            }
        }
    }

    // Encode via the `png` crate (not `image`) so we can attach iTXt metadata.
    let (iw, ih) = (img.width(), img.height());
    let errors = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let warnings = issues.len() - errors;
    let (summary, audit_json) = build_metadata(MetadataInputs {
        issues,
        meta,
        sf,
        img_w: iw,
        img_h: ih,
        viewport,
        errors,
        warnings,
    });

    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, iw, ih);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        // iTXt (UTF-8) — element text may be non-Latin. Keyword failures are
        // non-fatal (the image still encodes), so ignore them.
        let _ = enc.add_itxt_chunk("Software".to_string(), "Golem".to_string());
        let _ = enc.add_itxt_chunk("Golem-Summary".to_string(), summary);
        let _ = enc.add_itxt_chunk("Golem-Audit".to_string(), audit_json);
        let mut writer = enc.write_header()?;
        writer.write_image_data(img.as_raw())?;
    }
    Ok(out)
}

/// Why an annotated PNG can't be read as a Golem a11y audit.
#[derive(Debug)]
pub enum AuditReadError {
    /// The bytes aren't a decodable PNG.
    NotPng(String),
    /// No `Software = Golem` chunk — not produced by golem (or stripped by a
    /// re-encode). We refuse to interpret a foreign image's metadata.
    NotGolem,
    /// A golem PNG with no `Golem-Audit` chunk (e.g. an older build, or the
    /// chunk was stripped).
    MissingAudit,
}

impl std::fmt::Display for AuditReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotPng(e) => write!(f, "not a readable PNG: {e}"),
            Self::NotGolem => write!(
                f,
                "no Golem metadata (Software != \"Golem\") — not a golem annotated screenshot"
            ),
            Self::MissingAudit => write!(f, "Golem PNG has no Golem-Audit chunk"),
        }
    }
}

impl std::error::Error for AuditReadError {}

/// Read the embedded `Golem-Audit` JSON out of an annotated PNG. Validates the
/// `Software = Golem` iTXt chunk first, so a non-golem image is rejected rather
/// than mis-parsed. Returns the raw JSON string (the caller deserializes it).
pub fn read_embedded_audit(png_bytes: &[u8]) -> Result<String, AuditReadError> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let reader = decoder
        .read_info()
        .map_err(|e| AuditReadError::NotPng(e.to_string()))?;
    let info = reader.info();

    let text_of = |keyword: &str| {
        info.utf8_text
            .iter()
            .find(|c| c.keyword == keyword)
            .and_then(|c| {
                let mut c = c.clone();
                c.decompress_text().ok()?;
                c.get_text().ok()
            })
    };

    if text_of("Software").as_deref() != Some("Golem") {
        return Err(AuditReadError::NotGolem);
    }
    text_of("Golem-Audit").ok_or(AuditReadError::MissingAudit)
}

/// Inputs to [`build_metadata`]: the finding set plus the run/image context
/// needed to render the summary line and JSON audit record.
struct MetadataInputs<'a> {
    issues: &'a [A11yIssue],
    meta: &'a A11yMeta,
    sf: f64,
    img_w: u32,
    img_h: u32,
    viewport: &'a Viewport,
    errors: usize,
    warnings: usize,
}

/// Build the `(Golem-Summary, Golem-Audit)` metadata for the annotated PNG: a
/// human one-liner and a JSON record (context + every finding, with bounds in
/// screenshot pixels so hover tooling can overlay directly on the image).
fn build_metadata(inputs: MetadataInputs) -> (String, String) {
    let MetadataInputs {
        issues,
        meta,
        sf,
        img_w,
        img_h,
        viewport,
        errors,
        warnings,
    } = inputs;
    let px = |r: &Rect| {
        serde_json::json!({
            "x": (r.x as f64 * sf).round() as i32,
            "y": (r.y as f64 * sf).round() as i32,
            "w": (r.width as f64 * sf).round() as i32,
            "h": (r.height as f64 * sf).round() as i32,
        })
    };
    let issues_json: Vec<serde_json::Value> = issues
        .iter()
        .enumerate()
        .map(|(i, iss)| {
            serde_json::json!({
                "marker": i + 1,
                "check": iss.check_id,
                "severity": if iss.severity == Severity::Error { "error" } else { "warning" },
                "message": iss.message,
                "detail": iss.detail,
                "confidence": iss.confidence,
                "bounds": iss.element_bounds.as_ref().map(&px),
                "related": iss.related_bounds.iter().map(&px).collect::<Vec<_>>(),
            })
        })
        .collect();

    let summary = format!(
        "golem a11y · flow \"{}\" block \"{}\" · {} · {} error(s), {} warning(s) · seed {} · level {}",
        meta.flow, meta.block, meta.device, errors, warnings, meta.seed, meta.level
    );
    let audit = serde_json::json!({
        "software": "Golem",
        "app": meta.app,
        "device": meta.device,
        "platform": meta.platform,
        "flow": meta.flow,
        "block": meta.block,
        "iteration": meta.iteration,
        "seed": meta.seed,
        "a11y_level": meta.level,
        "image": { "w": img_w, "h": img_h },
        "viewport": { "w": viewport.width, "h": viewport.height },
        "errors": errors,
        "warnings": warnings,
        "issues": issues_json,
    });
    (summary, audit.to_string())
}

/// Pixel width of the marker chip for finding number `n` at glyph `scale` —
/// the rendered digits plus `scale` padding on each side. Mirrors the chip
/// geometry in [`draw_marker`] so cascade layout and drawing agree.
pub(crate) fn marker_chip_width(n: usize, scale: u32) -> u32 {
    crate::glyph::text_width(&n.to_string(), scale) + 2 * scale.max(1)
}

/// Draw a numbered marker chip at the top-left corner of a finding rectangle:
/// a solid `chip` background (severity colour) with the number rendered in
/// `text` colour on top, so it stays readable over any screenshot content.
fn draw_marker(
    img: &mut image::RgbaImage,
    x: i32,
    y: i32,
    scale: u32,
    chip: image::Rgba<u8>,
    text: image::Rgba<u8>,
    n: usize,
) {
    use imageproc::drawing::draw_filled_rect_mut;
    use imageproc::rect::Rect as IpRect;

    let s = n.to_string();
    let pad = scale.max(1) as i32;
    let tw = crate::glyph::text_width(&s, scale);
    let th = crate::glyph::text_height(scale);
    let chip_w = tw + 2 * pad as u32;
    let chip_h = th + 2 * pad as u32;
    draw_filled_rect_mut(img, IpRect::at(x, y).of_size(chip_w, chip_h), chip);
    crate::glyph::draw_str(img, x + pad, y + pad, scale, text, &s);
}

/// 2px hollow rectangle in `color` for a scaled `(x, y, w, h)`.
fn draw_rect(img: &mut image::RgbaImage, rect: (i32, i32, i32, i32), color: image::Rgba<u8>) {
    use imageproc::drawing::draw_hollow_rect_mut;
    use imageproc::rect::Rect as IpRect;
    let (x, y, w, h) = rect;
    for inset in 0..2 {
        let rw = (w - inset * 2).max(1) as u32;
        let rh = (h - inset * 2).max(1) as u32;
        draw_hollow_rect_mut(img, IpRect::at(x + inset, y + inset).of_size(rw, rh), color);
    }
}

/// Centre of a scaled rect.
fn rect_center((x, y, w, h): (i32, i32, i32, i32)) -> (f32, f32) {
    (x as f32 + w as f32 / 2.0, y as f32 + h as f32 / 2.0)
}

/// A cell in the occlusion minimap: a rectangle with a reachability flag.
type OcclusionCell = ((i32, i32, i32, i32), bool);

/// Draw a small 3×3 occupancy map at the bottom-right of `elem`, filling the
/// tested cells: **covered** ones solid, **reachable** ones a faint (opaque,
/// pre-blended pale) outline; untested zones are left blank so the map makes no
/// claim about them. `cells` are `((x,y,w,h), reachable)` sub-rects already
/// scaled to image px, positioned on the control's 3×3 lattice by their centre.
/// No-op when the element is too small to place the grid.
fn draw_occlusion_minimap(
    img: &mut image::RgbaImage,
    elem: (i32, i32, i32, i32),
    cells: &[OcclusionCell],
    gscale: u32,
    color: image::Rgba<u8>,
) {
    use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut};
    use imageproc::rect::Rect as IpRect;
    let (ex, ey, ew, eh) = elem;
    if ew <= 0 || eh <= 0 {
        return;
    }
    let cell = (gscale as i32 * 2).max(4);
    let grid = cell * 3;
    let m = gscale as i32 + 2;
    let gx = ex + ew - grid - m;
    let gy = ey + eh - grid - m;
    // Needs room inside the control; skip if it would spill past the top-left.
    if gx < ex || gy < ey {
        return;
    }
    // Reachable outline: the finding colour blended 50% toward white → an
    // *opaque* pale stroke. (A real alpha here would leave semi-transparent
    // pixels that composite dark over a dark viewer backdrop.)
    let pale = image::Rgba([
        ((color.0[0] as u16 + 255) / 2) as u8,
        ((color.0[1] as u16 + 255) / 2) as u8,
        ((color.0[2] as u16 + 255) / 2) as u8,
        255,
    ]);
    for &((cx, cy, cw, ch), reachable) in cells {
        let col = (((cx + cw / 2 - ex) * 3) / ew).clamp(0, 2);
        let row = (((cy + ch / 2 - ey) * 3) / eh).clamp(0, 2);
        let r = IpRect::at(gx + col * cell, gy + row * cell).of_size(cell as u32, cell as u32);
        if reachable {
            draw_hollow_rect_mut(img, r, pale);
        } else {
            draw_filled_rect_mut(img, r, color);
        }
    }
}

/// Centroid of all member-rect centres (marker anchor for grouped findings).
fn centroid(members: &[(i32, i32, i32, i32)]) -> (i32, i32) {
    let n = members.len().max(1) as f32;
    let (sx, sy) = members
        .iter()
        .map(|m| rect_center(*m))
        .fold((0.0, 0.0), |(ax, ay), (cx, cy)| (ax + cx, ay + cy));
    ((sx / n).round() as i32, (sy / n).round() as i32)
}

/// The closest pair of corners between two scaled rects — endpoints for a
/// connector that visually links the two without crossing them.
fn nearest_corners(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> ((f32, f32), (f32, f32)) {
    let corners = |(x, y, w, h): (i32, i32, i32, i32)| {
        [
            (x as f32, y as f32),
            ((x + w) as f32, y as f32),
            (x as f32, (y + h) as f32),
            ((x + w) as f32, (y + h) as f32),
        ]
    };
    let (ca, cb) = (corners(a), corners(b));
    let mut best = (ca[0], cb[0]);
    let mut best_d = f32::MAX;
    for &p in &ca {
        for &q in &cb {
            let d = (p.0 - q.0).powi(2) + (p.1 - q.1).powi(2);
            if d < best_d {
                best_d = d;
                best = (p, q);
            }
        }
    }
    best
}

/// A 2px solid line (drawn twice, offset 1px perpendicular for weight).
fn draw_solid_line(
    img: &mut image::RgbaImage,
    p0: (f32, f32),
    p1: (f32, f32),
    color: image::Rgba<u8>,
) {
    use imageproc::drawing::draw_line_segment_mut;
    draw_line_segment_mut(img, p0, p1, color);
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let (px, py) = (-dy / len, dx / len); // unit perpendicular
    draw_line_segment_mut(img, (p0.0 + px, p0.1 + py), (p1.0 + px, p1.1 + py), color);
}

/// A dashed line in `color`; dash/gap scale with the glyph size.
fn draw_dashed_line(
    img: &mut image::RgbaImage,
    p0: (f32, f32),
    p1: (f32, f32),
    color: image::Rgba<u8>,
    scale: u32,
) {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let dash = (3 * scale) as f32;
    let period = dash + (2 * scale) as f32;
    let mut t = 0.0;
    while t < len {
        let e = (t + dash).min(len);
        draw_solid_line(
            img,
            (p0.0 + ux * t, p0.1 + uy * t),
            (p0.0 + ux * e, p0.1 + uy * e),
            color,
        );
        t += period;
    }
}

/// A small inward-pointing arrowhead at `tip`, opening along unit `dir`.
fn draw_arrowhead(
    img: &mut image::RgbaImage,
    tip: (f32, f32),
    dir: (f32, f32),
    color: image::Rgba<u8>,
    size: f32,
) {
    let (dx, dy) = dir;
    let (px, py) = (-dy, dx); // perpendicular
    let back = (tip.0 + dx * size, tip.1 + dy * size);
    draw_solid_line(
        img,
        tip,
        (back.0 + px * size * 0.6, back.1 + py * size * 0.6),
        color,
    );
    draw_solid_line(
        img,
        tip,
        (back.0 - px * size * 0.6, back.1 - py * size * 0.6),
        color,
    );
}

/// Geometry + style for a dimension annotation, shared by [`draw_v_dimension`]
/// and [`draw_h_dimension`]: the finding's scaled rect, the marker colour, the
/// measurement label, and the glyph scale.
struct DimensionRect<'a> {
    rect_x: i32,
    rect_y: i32,
    rect_w: i32,
    rect_h: i32,
    color: image::Rgba<u8>,
    label: &'a str,
    scale: u32,
}

/// Vertical (height) dimension annotation in industrial style: extension lines
/// at top+bottom, an arrowed dimension line spanning the height, and the
/// measurement beside it. `right` chooses the side — `touch_target` uses the
/// left edge, `text_too_small` the right, so the two are distinguishable and
/// never overlap when both apply to one element. Falls back to the inner side
/// when there's no room (element flush to a screen edge).
fn draw_v_dimension(img: &mut image::RgbaImage, rect: DimensionRect, img_w: i32, right: bool) {
    let DimensionRect {
        rect_x,
        rect_y,
        rect_w,
        rect_h,
        color,
        label,
        scale,
    } = rect;
    let ds = scale.saturating_sub(1).max(2);
    let label_w = crate::glyph::text_width(label, ds) as i32;
    let lh = crate::glyph::text_height(ds) as i32;
    let off = (4 * scale) as i32;
    let gap = (2 * scale) as i32;
    let (top, bot) = (rect_y as f32, (rect_y + rect_h) as f32);
    let asz = (2 * scale) as f32;

    // Edge the dimension hangs off, and whether the preferred (outward) side
    // has room for the line + label.
    let (edge, outward) = if right {
        let e = rect_x + rect_w;
        (e, e + off + gap + label_w <= img_w)
    } else {
        (rect_x, rect_x - off - gap - label_w >= 0)
    };
    // Dimension-line x: outward from the edge, or inward on overflow.
    let lx = match (right, outward) {
        (true, true) => edge + off,
        (true, false) => edge - off,
        (false, true) => edge - off,
        (false, false) => edge + off,
    };
    let lxf = lx as f32;
    draw_solid_line(img, (edge as f32, top), (lxf, top), color);
    draw_solid_line(img, (edge as f32, bot), (lxf, bot), color);
    draw_solid_line(img, (lxf, top), (lxf, bot), color);
    draw_arrowhead(img, (lxf, top), (0.0, 1.0), color, asz);
    draw_arrowhead(img, (lxf, bot), (0.0, -1.0), color, asz);
    // Label vertically centred, on the far side of the dimension line.
    let ly = rect_y + rect_h / 2 - lh / 2;
    let label_left = (right && !outward) || (!right && outward);
    let label_x = if label_left {
        lx - gap - label_w
    } else {
        lx + gap
    };
    crate::glyph::draw_str(img, label_x, ly, ds, color, label);
}

/// Horizontal (width) dimension annotation, below the rect (inside on
/// overflow). Mirrors [`draw_v_dimension`] for the width axis.
fn draw_h_dimension(img: &mut image::RgbaImage, rect: DimensionRect, img_h: i32) {
    let DimensionRect {
        rect_x,
        rect_y,
        rect_w,
        rect_h,
        color,
        label,
        scale,
    } = rect;
    let ds = scale.saturating_sub(1).max(2);
    let lh = crate::glyph::text_height(ds) as i32;
    let off = (4 * scale) as i32;
    let gap = (2 * scale) as i32;
    let bottom = rect_y + rect_h;
    let outside = bottom + off + lh + gap <= img_h;
    let by = if outside { bottom + off } else { bottom - off };
    let byf = by as f32;
    let (left, right) = (rect_x as f32, (rect_x + rect_w) as f32);
    draw_solid_line(img, (left, bottom as f32), (left, byf), color);
    draw_solid_line(img, (right, bottom as f32), (right, byf), color);
    draw_solid_line(img, (left, byf), (right, byf), color);
    let asz = (2 * scale) as f32;
    draw_arrowhead(img, (left, byf), (1.0, 0.0), color, asz);
    draw_arrowhead(img, (right, byf), (-1.0, 0.0), color, asz);
    let label_w = crate::glyph::text_width(label, ds) as i32;
    let label_x = rect_x + rect_w / 2 - label_w / 2;
    let ly = if outside { by + gap } else { by - gap - lh };
    crate::glyph::draw_str(img, label_x, ly, ds, color, label);
}
