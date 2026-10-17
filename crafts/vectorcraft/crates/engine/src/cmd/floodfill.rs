//! Paint Bucket › Fill Rendered Pixels (`paint.floodFill`): a click fills the region of the
//! rendered picture around it, whatever draws it (paths, images, type, effects), with a new path.
//! The artboard under the click (else the art's bounds) is rendered as the screen shows it, the
//! pixels within `threshold` of the clicked one and 4-connected to it are flooded, the region is
//! grown or shrunk by `grow`, and its boundary (holes included) becomes a path filled with the
//! current fill, as Live Paint fills a face.

use redrob_vector_doc::NodeKind;
use redrob_vector_geom::{Anchor, FillRule, PathData, Point, Rect, SubPath};
use redrob_vector_render::RenderOptions;
use redrob_vector_trace::trace_mask;
use serde_json::{Value, json};

use super::create::add_look;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "paint.floodFill",
        "Fill Rendered Pixels",
        [],
        None,
        "{x, y: document point, threshold?: 0..255 (32; how far a pixel's colour may be from the clicked one, per channel), grow?: pt (0; grows the region, negative shrinks it), scale?: pixels per point (2, up to 8; the picture is rendered at this resolution, at most 4 million pixels)} fill the region of the rendered picture around the point with a new path in the current fill → {id, pixels}",
        has_doc,
        flood_fill
    )]
}

/// The region the picture is rendered over: the artboard under `p`, else the art's bounds, else
/// the first artboard; inflated a little so a region touching the edge closes.
fn region_of(d: &redrob_vector_doc::Document, p: Point) -> Option<Rect> {
    let r = match d.artboard_at(p) {
        Some(i) => d.artboards.get(i)?.rect,
        None => d.art_bounds().filter(|b| b.contains(p)).or_else(|| d.artboards.first().map(|a| a.rect))?,
    };
    Some(r.inflate(2.0, 2.0))
}

fn flood_fill(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "paint.floodFill";
    let at = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    let threshold = f64_or(p, "threshold", 32.0).clamp(0.0, 255.0) as i32;
    let grow = f64_or(p, "grow", 0.0).clamp(-1000.0, 1000.0);
    let doc = &s.doc()?.doc;
    let region = region_of(doc, at).ok_or_else(|| bad(C, "the point lies on no artboard and outside the art"))?;
    // At most four million pixels, however large the artboard.
    let mut scale = f64_or(p, "scale", 2.0).clamp(0.1, 8.0);
    let max_px = 4.0e6;
    if region.area() * scale * scale > max_px {
        scale = (max_px / region.area()).sqrt();
    }
    let mut r = redrob_vector_render::Renderer::new();
    r.threads = 0;
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), skip_templates: true, ..Default::default() };
    let img = r.render_region_with(doc, region, scale, &opts);
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 || img.pixels.len() < w * h * 4 {
        return Err(bad(C, "nothing rendered"));
    }
    let px = |x: usize, y: usize| -> [i32; 3] {
        let i = (y * w + x) * 4;
        [i32::from(img.pixels[i]), i32::from(img.pixels[i + 1]), i32::from(img.pixels[i + 2])]
    };
    let sx = ((at.x - region.x0) * scale).floor();
    let sy = ((at.y - region.y0) * scale).floor();
    if !(sx >= 0.0 && sy >= 0.0 && (sx as usize) < w && (sy as usize) < h) {
        return Err(bad(C, "the point lies outside the rendered picture"));
    }
    let (sx, sy) = (sx as usize, sy as usize);
    let seed = px(sx, sy);
    let near = |c: [i32; 3]| (0..3).all(|k| (c[k] - seed[k]).abs() <= threshold);
    // 4-connected flood from the click.
    let mut mask = vec![false; w * h];
    let mut stack = vec![(sx, sy)];
    mask[sy * w + sx] = true;
    while let Some((x, y)) = stack.pop() {
        let mut visit = |nx: usize, ny: usize| {
            let i = ny * w + nx;
            if !mask[i] && near(px(nx, ny)) {
                mask[i] = true;
                stack.push((nx, ny));
            }
        };
        if x > 0 {
            visit(x - 1, y);
        }
        if x + 1 < w {
            visit(x + 1, y);
        }
        if y > 0 {
            visit(x, y - 1);
        }
        if y + 1 < h {
            visit(x, y + 1);
        }
    }
    // Grow or shrink the region by `grow` points (a box dilation or erosion).
    let steps = (grow.abs() * scale).round() as usize;
    for _ in 0..steps.min(64) {
        let src = mask.clone();
        let on = |x: isize, y: isize| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && src[y as usize * w + x as usize];
        for y in 0..h {
            for x in 0..w {
                let (xi, yi) = (x as isize, y as isize);
                let around = [on(xi - 1, yi), on(xi + 1, yi), on(xi, yi - 1), on(xi, yi + 1)];
                mask[y * w + x] = if grow > 0.0 { src[y * w + x] || around.iter().any(|v| *v) } else { src[y * w + x] && around.iter().all(|v| *v) };
            }
        }
    }
    let pixels = mask.iter().filter(|m| **m).count();
    if pixels == 0 {
        return Err(bad(C, "the region is empty"));
    }
    // The region's boundary as a path: the pixel-crack corners, back in document coordinates.
    let to_doc = |(x, y): (i32, i32)| Point::new(region.x0 + f64::from(x) / scale, region.y0 + f64::from(y) / scale);
    let mut subs = vec![];
    for comp in trace_mask(&mask, w, h) {
        for lp in std::iter::once(&comp.outer).chain(comp.holes.iter()) {
            let pts: Vec<Anchor> = lp.pts.iter().map(|q| Anchor::corner(to_doc(*q))).collect();
            if pts.len() >= 3 {
                subs.push(SubPath::new(pts, true));
            }
        }
    }
    if subs.is_empty() {
        return Err(bad(C, "the region has no outline"));
    }
    let path = PathData::new(subs);
    let fill = s.paint.fill.clone();
    let look = s.new_art_look(fill, redrob_vector_color::Paint::None, 0.0);
    let kind = NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: false };
    let mut out = add_look(s, "Paint Bucket", kind, look, None)?;
    if let Some(o) = out.as_object_mut() {
        o.insert("pixels".into(), json!(pixels));
    }
    Ok(out)
}
