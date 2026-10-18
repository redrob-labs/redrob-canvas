//! Select › Sharpen, Remove Holes, Distort and Rounded Rectangle, plus the floating selection's
//! two exits: Layer › Anchor Layer (merge the floating piece back where it came from) and
//! Layer › To New Layer (turn the floating piece into an ordinary layer).
//!
//! * `select.sharpen` drops partial coverage: every pixel is fully in or out.
//! * `select.removeHoles` fills unselected islands enclosed by the selection.
//! * `select.distort` roughens the outline: the mask is displaced by smooth noise (`spread`
//!   pixels, cells of `granularity`), box-smoothed (`smooth`, horizontally and/or vertically) and
//!   re-thresholded (`threshold`).
//! * `select.roundedRectangle` replaces the selection's rectangle with a rounded (or concave)
//!   version, the corner radius a percentage of half the shorter side.
//! * `layer.anchor` drops the floating piece into its layer (the selection moves with it).
//! * `layer.floatingToLayer` makes the floating piece a new layer above its source.

use redrob_photo_algo::selection::{self as sel, SelectionMode};
use redrob_photo_doc::{Layer, LayerContent};
use redrob_photo_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_selection(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.doc.selection.is_some()).map(|_| ()).ok_or_else(|| "no selection".into())
}
fn is_floating(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|st| crate::float_cmds::floating(st).is_some()).map(|_| ()).ok_or_else(|| "nothing is floating".into())
}
fn f(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(d, |v| v as f32)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// The largest canvas a whole-mask operation works on (f32 per pixel, ~4 GB).
const MAX_PIXELS: u64 = 1 << 30;

/// The current selection as a full-canvas mask.
fn mask(s: &Session, cmd: &str) -> Result<(Rect, Vec<f32>)> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let area = d.doc.bounds();
    if area.size().area() > MAX_PIXELS {
        return Err(bad(cmd, "the selection is too large to modify"));
    }
    Ok((area, sel::mask_from_surface(d.doc.selection.as_ref(), area)))
}

fn set_mask(s: &mut Session, label: &str, area: Rect, m: Vec<f32>) -> Result<Value> {
    let selected = s.edit(label, |doc, _| {
        doc.selection = sel::combine(None, &m, area, SelectionMode::Replace);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({ "selected": selected }))
}

// ---------------------------------------------------------------- Sharpen

/// Every pixel fully in (coverage ≥ ½) or out.
pub fn sharpen_mask(m: &[f32]) -> Vec<f32> {
    m.iter().map(|v| if *v >= 0.5 { 1.0 } else { 0.0 }).collect()
}

fn sharpen(s: &mut Session, _p: &Value) -> Result<Value> {
    let (area, m) = mask(s, "select.sharpen")?;
    set_mask(s, "Sharpen Selection", area, sharpen_mask(&m))
}

// ---------------------------------------------------------------- Remove holes

/// Fills every unselected region that does not touch the canvas edge: a flood from the border
/// over the unselected pixels marks the outside; what it never reaches is a hole.
pub fn remove_holes_mask(m: &[f32], w: usize, h: usize) -> Vec<f32> {
    if w == 0 || h == 0 {
        return m.to_vec();
    }
    let outside_at = |i: usize| m[i] < 0.5;
    let mut outside = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    let seed = |i: usize, outside: &mut Vec<bool>, stack: &mut Vec<usize>| {
        if outside_at(i) && !outside[i] {
            outside[i] = true;
            stack.push(i);
        }
    };
    for x in 0..w {
        seed(x, &mut outside, &mut stack);
        seed((h - 1) * w + x, &mut outside, &mut stack);
    }
    for y in 0..h {
        seed(y * w, &mut outside, &mut stack);
        seed(y * w + w - 1, &mut outside, &mut stack);
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        let mut push = |j: usize| {
            if outside_at(j) && !outside[j] {
                outside[j] = true;
                stack.push(j);
            }
        };
        if x > 0 {
            push(i - 1);
        }
        if x + 1 < w {
            push(i + 1);
        }
        if y > 0 {
            push(i - w);
        }
        if y + 1 < h {
            push(i + w);
        }
    }
    m.iter().zip(&outside).map(|(v, out)| if *v < 0.5 && !*out { 1.0 } else { *v }).collect()
}

fn remove_holes(s: &mut Session, _p: &Value) -> Result<Value> {
    let (area, m) = mask(s, "select.removeHoles")?;
    let (w, h) = (area.width() as usize, area.height() as usize);
    set_mask(s, "Remove Holes", area, remove_holes_mask(&m, w, h))
}

// ---------------------------------------------------------------- Distort

/// Deterministic noise in −1..1 for a lattice point (`seed` picks the field).
fn lattice_noise(x: i64, y: i64, seed: u64) -> f32 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x7FB5_D329_728E_A185);
    h ^= h >> 27;
    (h >> 11) as f32 / (1u64 << 53) as f32 * 2.0 - 1.0
}

/// Smooth noise in −1..1 at (x, y): bilinear interpolation of a lattice with cells of `cell` pixels.
fn smooth_noise(x: f32, y: f32, cell: f32, seed: u64) -> f32 {
    let (gx, gy) = (x / cell, y / cell);
    let (x0, y0) = (gx.floor(), gy.floor());
    let (tx, ty) = (gx - x0, gy - y0);
    let (tx, ty) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (x0, y0) = (x0 as i64, y0 as i64);
    let n = |dx: i64, dy: i64| lattice_noise(x0 + dx, y0 + dy, seed);
    let top = n(0, 0) + (n(1, 0) - n(0, 0)) * tx;
    let bottom = n(0, 1) + (n(1, 1) - n(0, 1)) * tx;
    top + (bottom - top) * ty
}

/// Box-smooths `m` by `r` pixels along x and/or y (edges replicated).
fn box_smooth(m: &[f32], w: usize, h: usize, r: usize, horizontal: bool, vertical: bool) -> Vec<f32> {
    if r == 0 || (!horizontal && !vertical) {
        return m.to_vec();
    }
    let mut out = m.to_vec();
    if horizontal {
        let src = out.clone();
        for y in 0..h {
            let row = &src[y * w..(y + 1) * w];
            for x in 0..w {
                let mut acc = 0.0;
                for k in -(r as i64)..=(r as i64) {
                    let xi = (x as i64 + k).clamp(0, w as i64 - 1) as usize;
                    acc += row[xi];
                }
                out[y * w + x] = acc / (2 * r + 1) as f32;
            }
        }
    }
    if vertical {
        let src = out.clone();
        for x in 0..w {
            for y in 0..h {
                let mut acc = 0.0;
                for k in -(r as i64)..=(r as i64) {
                    let yi = (y as i64 + k).clamp(0, h as i64 - 1) as usize;
                    acc += src[yi * w + x];
                }
                out[y * w + x] = acc / (2 * r + 1) as f32;
            }
        }
    }
    out
}

/// Parameters of [`distort_mask`].
#[derive(Clone, Copy, Debug)]
pub struct Distort {
    /// Coverage (0..255) above which a displaced pixel is selected.
    pub threshold: f32,
    /// Displacement amplitude in pixels.
    pub spread: f32,
    /// Size of the noise cells in pixels (larger = smoother raggedness).
    pub granularity: f32,
    /// Box-smoothing radius in pixels applied after displacement.
    pub smooth: f32,
    pub smooth_horizontally: bool,
    pub smooth_vertically: bool,
    pub seed: u64,
}

/// Roughens the outline of `m`: each pixel samples the mask displaced by smooth noise, the result
/// is box-smoothed and re-thresholded. The interior and the far outside are unchanged.
pub fn distort_mask(m: &[f32], w: usize, h: usize, d: Distort) -> Vec<f32> {
    if w == 0 || h == 0 {
        return m.to_vec();
    }
    let cell = d.granularity.max(1.0);
    let spread = d.spread.max(0.0);
    let sample = |x: i64, y: i64| -> f32 {
        let xi = x.clamp(0, w as i64 - 1) as usize;
        let yi = y.clamp(0, h as i64 - 1) as usize;
        m[yi * w + xi]
    };
    let mut displaced = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let nx = smooth_noise(x as f32, y as f32, cell, d.seed) * spread;
            let ny = smooth_noise(x as f32, y as f32, cell, d.seed ^ 0x5bd1_e995) * spread;
            let sx = x as f32 + nx;
            let sy = y as f32 + ny;
            let (x0, y0) = (sx.floor(), sy.floor());
            let (tx, ty) = (sx - x0, sy - y0);
            let (x0, y0) = (x0 as i64, y0 as i64);
            let top = sample(x0, y0) * (1.0 - tx) + sample(x0 + 1, y0) * tx;
            let bottom = sample(x0, y0 + 1) * (1.0 - tx) + sample(x0 + 1, y0 + 1) * tx;
            displaced[y * w + x] = top * (1.0 - ty) + bottom * ty;
        }
    }
    let smoothed = box_smooth(&displaced, w, h, d.smooth.max(0.0).round() as usize, d.smooth_horizontally, d.smooth_vertically);
    let t = (d.threshold / 255.0).clamp(0.0, 1.0);
    smoothed.into_iter().map(|v| if v > t { 1.0 } else { 0.0 }).collect()
}

fn distort(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "select.distort";
    let d = Distort {
        threshold: f(p, "threshold", 127.0),
        spread: f(p, "spread", 8.0),
        granularity: f(p, "granularity", 4.0),
        smooth: f(p, "smooth", 2.0),
        smooth_horizontally: b(p, "smoothHorizontally", true),
        smooth_vertically: b(p, "smoothVertically", true),
        seed: p.get("seed").and_then(Value::as_u64).unwrap_or(1),
    };
    if !(0.0..=255.0).contains(&d.threshold) {
        return Err(bad(CMD, "threshold must be 0..255"));
    }
    if !(0.0..=1000.0).contains(&d.spread) || !(1.0..=25.0).contains(&d.granularity) || !(0.0..=150.0).contains(&d.smooth) {
        return Err(bad(CMD, "spread 0..1000, granularity 1..25, smooth 0..150"));
    }
    let (area, m) = mask(s, CMD)?;
    let (w, h) = (area.width() as usize, area.height() as usize);
    set_mask(s, "Distort Selection", area, distort_mask(&m, w, h, d))
}

// ---------------------------------------------------------------- Rounded rectangle

/// Anti-aliased coverage of `r` with corners of `radius` pixels rounded (convex) or cut concave
/// (quarter circles removed from the corners), over `area`.
pub fn rounded_rect_mask(area: Rect, r: Rect, radius: f32, concave: bool) -> Vec<f32> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut out = vec![0.0f32; w * h];
    let radius = radius.clamp(0.0, (r.width().min(r.height()) as f32 / 2.0).max(0.0));
    const SUB: i32 = 4;
    let inside = |px: f32, py: f32| -> bool {
        if px < r.x0 as f32 || px >= r.x1 as f32 || py < r.y0 as f32 || py >= r.y1 as f32 {
            return false;
        }
        if radius <= 0.0 {
            return true;
        }
        // Corner circle centres.
        let cx = if px < r.x0 as f32 + radius {
            Some(r.x0 as f32 + radius)
        } else if px > r.x1 as f32 - radius {
            Some(r.x1 as f32 - radius)
        } else {
            None
        };
        let cy = if py < r.y0 as f32 + radius {
            Some(r.y0 as f32 + radius)
        } else if py > r.y1 as f32 - radius {
            Some(r.y1 as f32 - radius)
        } else {
            None
        };
        match (cx, cy) {
            (Some(cx), Some(cy)) => {
                let d2 = (px - cx).powi(2) + (py - cy).powi(2);
                if concave {
                    // The corner is scooped out: the quarter circle centred on the rectangle's
                    // corner itself is removed.
                    let kx = if px < cx { r.x0 as f32 } else { r.x1 as f32 };
                    let ky = if py < cy { r.y0 as f32 } else { r.y1 as f32 };
                    (px - kx).powi(2) + (py - ky).powi(2) >= radius * radius
                } else {
                    d2 <= radius * radius
                }
            }
            _ => true,
        }
    };
    let clip = r.intersect(&area);
    for y in clip.y0..clip.y1 {
        for x in clip.x0..clip.x1 {
            let mut hit = 0;
            for sy in 0..SUB {
                for sx in 0..SUB {
                    let px = x as f32 + (sx as f32 + 0.5) / SUB as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SUB as f32;
                    if inside(px, py) {
                        hit += 1;
                    }
                }
            }
            out[(y - area.y0) as usize * w + (x - area.x0) as usize] = hit as f32 / (SUB * SUB) as f32;
        }
    }
    out
}

fn rounded_rectangle(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "select.roundedRectangle";
    let pct = f(p, "radius", 50.0);
    if !(0.0..=100.0).contains(&pct) {
        return Err(bad(CMD, "radius must be a percentage 0..100"));
    }
    let concave = b(p, "concave", false);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let selection = d.doc.selection.as_ref().ok_or_else(|| EngineError::Other("no selection".into()))?;
    let area = d.doc.bounds();
    let r = selection.content_bounds().intersect(&area);
    if r.is_empty() {
        return Err(EngineError::Other("the selection is empty".into()));
    }
    let radius = pct / 100.0 * (r.width().min(r.height()) as f32 / 2.0);
    let m = rounded_rect_mask(area, r, radius, concave);
    set_mask(s, if concave { "Concave Rectangle" } else { "Rounded Rectangle" }, area, m)
}

// ---------------------------------------------------------------- Floating selection exits

fn anchor(s: &mut Session, _p: &Value) -> Result<Value> {
    let r = crate::float_cmds::drop_floating(s)?;
    Ok(json!({ "anchored": r["dropped"], "offset": r["offset"] }))
}

fn floating_to_layer(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let fl = crate::float_cmds::floating(st).cloned().ok_or_else(|| EngineError::Other("nothing is floating".into()))?;
    if fl.parts.is_mask() {
        return Err(EngineError::Other("a floated mask can only be anchored".into()));
    }
    st.floating = None;
    let (dx, dy) = fl.offset;
    let id = s.edit("Floating Selection to Layer", |doc, active| {
        let src = doc.layer_mut(fl.layer).ok_or(EngineError::NoLayer(fl.layer))?;
        let surf = src.surface_mut().ok_or_else(|| EngineError::Other("the layer has no pixels".into()))?;
        // The hole the piece left stays in its layer.
        *surf = fl.parts.rest().clone();
        let piece = redrob_photo_algo::resample::translate_surface(fl.parts.piece(), dx, dy);
        let name = doc.next_layer_name("Floating Selection");
        let layer = Layer::new(name, LayerContent::Raster(piece));
        let id = doc.insert_above(Some(fl.layer), layer);
        doc.selection = doc.selection.as_ref().map(|sel| redrob_photo_algo::resample::translate_surface(sel, dx, dy));
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({ "layer": id.0, "offset": [dx, dy] }))
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "select.sharpen",
            "Sharpen",
            ["Select", "Modify"],
            r##"{} (every pixel fully selected or not: antialiased and feathered edges become hard) → {selected}"##,
            has_selection,
            sharpen
        ),
        spec!(
            "select.removeHoles",
            "Remove Holes",
            ["Select", "Modify"],
            r##"{} (fills unselected islands enclosed by the selection) → {selected}"##,
            has_selection,
            remove_holes
        ),
        spec!(
            "select.distort",
            "Distort…",
            ["Select", "Modify"],
            r##"{"threshold":0..255=127,"spread":0..1000=8 (displacement, px),"granularity":1..25=4 (noise cell size, px),"smooth":0..150=2 (px),"smoothHorizontally":bool=true,"smoothVertically":bool=true,"seed":int=1} → {selected}"##,
            has_selection,
            distort
        ),
        spec!(
            "select.roundedRectangle",
            "Rounded Rectangle…",
            ["Select", "Modify"],
            r##"{"radius":0..100=50 (% of half the shorter side),"concave":bool=false} (replaces the selection with its rectangle, corners rounded or scooped) → {selected}"##,
            has_selection,
            rounded_rectangle
        ),
        spec!(
            "layer.anchor",
            "Anchor Layer",
            ["Layer"],
            r##"{} (merges the floating selection back into the layer it came from; the selection moves with it) → {anchored, offset}"##,
            is_floating,
            anchor
        ),
        spec!(
            "layer.floatingToLayer",
            "To New Layer",
            ["Layer"],
            r##"{} (turns the floating selection into a new layer above its source layer) → {layer, offset}"##,
            is_floating,
            floating_to_layer
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_sel() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 60, "height": 60})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
        s
    }

    fn cov(s: &Session, x: i32, y: i32) -> f32 {
        s.active().unwrap().doc.selection.as_ref().map_or(0.0, |sel| sel.sample_channel(x, y, 0))
    }

    #[test]
    fn sharpen_removes_partial_coverage() {
        let mut s = rect_sel();
        s.execute("select.modify.feather", json!({"radius": 3})).unwrap();
        let soft = cov(&s, 10, 25);
        assert!(soft > 0.0 && soft < 1.0, "{soft}");
        s.execute("select.sharpen", json!({})).unwrap();
        let sel = s.active().unwrap().doc.selection.clone().unwrap();
        for y in 0..60 {
            for x in 0..60 {
                let v = sel.sample_channel(x, y, 0);
                assert!(v == 0.0 || v == 1.0, "({x},{y}) = {v}");
            }
        }
    }

    #[test]
    fn remove_holes_fills_enclosed_islands_only() {
        let mut s = rect_sel();
        s.execute("select.rect", json!({"x": 20, "y": 20, "width": 5, "height": 5, "mode": "subtract"})).unwrap();
        assert_eq!(cov(&s, 22, 22), 0.0);
        s.execute("select.removeHoles", json!({})).unwrap();
        assert_eq!(cov(&s, 22, 22), 1.0);
        assert_eq!(cov(&s, 2, 2), 0.0, "the outside stays unselected");
        let m = remove_holes_mask(&[1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0], 3, 3);
        assert_eq!(m[4], 1.0);
    }

    #[test]
    fn distort_roughens_the_edge_and_keeps_the_interior() {
        let mut s = rect_sel();
        s.execute("select.distort", json!({"spread": 6, "granularity": 4, "smooth": 1})).unwrap();
        assert_eq!(cov(&s, 30, 25), 1.0, "deep inside stays selected");
        assert_eq!(cov(&s, 2, 2), 0.0, "far outside stays clear");
        let sel = s.active().unwrap().doc.selection.clone().unwrap();
        let edge: Vec<f32> = (10..50).map(|x| sel.sample_channel(x, 10, 0)).collect();
        assert!(edge.iter().any(|v| *v == 0.0) || (0..60).any(|x| sel.sample_channel(x, 8, 0) > 0.0), "the top edge moved somewhere");
        assert!(s.execute("select.distort", json!({"threshold": 300})).is_err());
    }

    #[test]
    fn rounded_rectangle_cuts_the_corners() {
        let mut s = rect_sel();
        s.execute("select.roundedRectangle", json!({"radius": 100})).unwrap();
        assert_eq!(cov(&s, 10, 10), 0.0, "the corner is rounded off");
        assert_eq!(cov(&s, 30, 25), 1.0);
        assert!(cov(&s, 10, 25) > 0.9, "the middle of the left edge stays");
        s.undo();
        s.execute("select.roundedRectangle", json!({"radius": 100, "concave": true})).unwrap();
        assert_eq!(cov(&s, 10, 10), 0.0);
        assert!(cov(&s, 24, 24) > 0.9, "a concave corner keeps the inside of the rectangle");
        assert!(cov(&s, 12, 12) < 0.5, "and scoops out near the corner");
    }

    #[test]
    fn floating_piece_anchors_or_becomes_a_layer() {
        let mut s = rect_sel();
        s.execute("edit.fill", json!({"color": "#ff0000"})).ok();
        assert!(!s.is_enabled("layer.anchor"));
        s.execute("select.float", json!({"dx": 5, "dy": 0})).unwrap();
        assert!(s.is_enabled("layer.anchor") && s.is_enabled("layer.floatingToLayer"));
        let n = s.active().unwrap().doc.layers.len();
        let r = s.execute("layer.floatingToLayer", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.len(), n + 1);
        assert_eq!(s.active().unwrap().active_layer.map(|l| l.0), r["layer"].as_u64());
        assert!(s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().name.starts_with("Floating Selection"));
        assert!(!s.is_enabled("layer.anchor"));
        s.execute("select.float", json!({"dx": 3, "dy": 2})).unwrap();
        let r = s.execute("layer.anchor", json!({})).unwrap();
        assert_eq!(r["anchored"], json!(true));
        assert_eq!(r["offset"], json!([3, 2]));
    }
}
