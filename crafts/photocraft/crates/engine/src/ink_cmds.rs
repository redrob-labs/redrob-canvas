//! The Ink tool (`paint.ink`): calligraphy strokes computed from a nib, not from a brush tip.
//! The nib is a circle, square or diamond of `size` pixels with an `aspect` ratio and `angle`;
//! its size follows the pen's pressure, thins with speed and its angle follows the pen's tilt,
//! each by an adjustable sensitivity. The stroke is the union of the nib outlines swept along the
//! path (the convex hull of every pair of neighbouring nibs), rasterised anti-aliased and painted
//! in the foreground colour.

use redrob_photo_color::BlendMode;
use redrob_photo_doc::vector::{Path, Subpath};
use redrob_photo_geom::Rect;
use redrob_photo_vector as vector;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "paint.ink";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// Nib shapes.
pub const SHAPES: &[&str] = &["circle", "square", "diamond"];

/// One input point of an ink stroke.
#[derive(Clone, Copy, Debug)]
pub struct InkPoint {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
    pub tilt_x: f64,
    pub tilt_y: f64,
    pub time_ms: f64,
}

/// The nib and its sensitivities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nib {
    pub size: f64,
    pub angle_deg: f64,
    /// 1 = round, up to 10 = ten times wider than tall.
    pub aspect: f64,
    pub shape: u8,
    pub sens_size: f64,
    pub sens_tilt: f64,
    pub sens_speed: f64,
}

/// Parses `points`: `[x, y, pressure?, tiltX?, tiltY?, rotation?, timeMs?]` (3, 4, 6 or 7 values).
fn parse_points(p: &Value) -> Result<Vec<InkPoint>> {
    let a = p.get("points").and_then(Value::as_array).ok_or_else(|| bad("pass `points`: [[x, y, pressure?, tiltX?, tiltY?, rotation?, timeMs?], ...]"))?;
    if a.len() > 100_000 {
        return Err(bad("too many points"));
    }
    let mut out = Vec::with_capacity(a.len());
    for (i, v) in a.iter().enumerate() {
        let n: Vec<f64> = v.as_array().map(|q| q.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        if n.len() < 2 || n.iter().any(|x| !x.is_finite()) {
            return Err(bad(format!("point {i} must be [x, y, ...] finite numbers")));
        }
        let pressure = n.get(2).copied().unwrap_or(1.0).clamp(0.0, 1.0);
        let (tilt_x, tilt_y) = if n.len() >= 6 { (n[3], n[4]) } else { (0.0, 0.0) };
        let time_ms = match n.len() {
            4 => n[3],
            7 => n[6],
            _ => i as f64 * 10.0,
        };
        out.push(InkPoint { x: n[0], y: n[1], pressure, tilt_x, tilt_y, time_ms });
    }
    if out.is_empty() {
        return Err(bad("`points` is empty"));
    }
    Ok(out)
}

/// The nib outline (document coordinates) at `pt`, given the previous point for the speed.
pub fn nib_outline(nib: &Nib, pt: &InkPoint, prev: Option<&InkPoint>) -> Vec<(f64, f64)> {
    // Size: pressure scales it between 1/3 and 1 by the size sensitivity.
    let pressure_k = 1.0 - nib.sens_size * (1.0 - pt.pressure) * (2.0 / 3.0);
    // Speed: a fast stroke thins down to a quarter by the speed sensitivity.
    let speed_k = match prev {
        Some(q) => {
            let dt = (pt.time_ms - q.time_ms).max(1.0);
            let v = ((pt.x - q.x).hypot(pt.y - q.y) / dt).min(5.0) / 5.0; // 0..1 at 5 px/ms
            1.0 - nib.sens_speed * v * 0.75
        }
        None => 1.0,
    };
    let size = (nib.size * pressure_k * speed_k).max(0.5);
    // Tilt turns the nib towards the pen's lean and stretches it along it.
    let tilt_mag = (pt.tilt_x.hypot(pt.tilt_y) / 90.0).min(1.0);
    let tilt_angle = if tilt_mag > 1e-6 { pt.tilt_y.atan2(pt.tilt_x).to_degrees() } else { 0.0 };
    let angle = (nib.angle_deg + nib.sens_tilt * tilt_angle).to_radians();
    let aspect = (nib.aspect * (1.0 + nib.sens_tilt * tilt_mag)).clamp(1.0, 20.0);
    let (rx, ry) = (size / 2.0, size / 2.0 / aspect);
    let (cs, sn) = (angle.cos(), angle.sin());
    let rot = |u: f64, v: f64| (pt.x + u * cs - v * sn, pt.y + u * sn + v * cs);
    match nib.shape {
        1 => vec![rot(-rx, -ry), rot(rx, -ry), rot(rx, ry), rot(-rx, ry)],
        2 => vec![rot(0.0, -ry), rot(rx, 0.0), rot(0.0, ry), rot(-rx, 0.0)],
        _ => {
            let n = if size > 40.0 { 32 } else { 16 };
            (0..n)
                .map(|i| {
                    let t = i as f64 / n as f64 * std::f64::consts::TAU;
                    rot(rx * t.cos(), ry * t.sin())
                })
                .collect()
        }
    }
}

/// Convex hull (Andrew's monotone chain) of `pts`, counter-clockwise.
pub fn convex_hull(mut pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<(f64, f64)> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<(f64, f64)> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The stroke's outline path: one convex hull per pair of neighbouring nibs (a lone point is its
/// nib), all wound the same way so non-zero filling unions them.
pub fn stroke_path(nib: &Nib, pts: &[InkPoint]) -> Path {
    let mut subpaths = Vec::with_capacity(pts.len());
    let mut prev_outline: Option<Vec<(f64, f64)>> = None;
    for (i, pt) in pts.iter().enumerate() {
        let outline = nib_outline(nib, pt, i.checked_sub(1).and_then(|j| pts.get(j)));
        match prev_outline.take() {
            Some(mut a) => {
                a.extend(outline.iter().copied());
                let hull = convex_hull(a);
                if hull.len() >= 3 {
                    subpaths.push(Subpath::polygon(&hull));
                }
            }
            None => {
                let hull = convex_hull(outline.clone());
                if hull.len() >= 3 {
                    subpaths.push(Subpath::polygon(&hull));
                }
            }
        }
        prev_outline = Some(outline);
    }
    Path::new(subpaths)
}

/// Reads the nib from the params (the brush's size when `size` is absent).
fn nib_param(s: &Session, p: &Value) -> Result<Nib> {
    let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d);
    let shape = match p.get("shape").and_then(Value::as_str).unwrap_or("circle") {
        "circle" => 0,
        "square" => 1,
        "diamond" => 2,
        other => return Err(bad(format!("`shape` must be circle, square or diamond (got `{other}`)"))),
    };
    let sens = p.get("sensitivity").cloned().unwrap_or(Value::Null);
    let sf = |k: &str, d: f64| sens.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d).clamp(0.0, 1.0);
    Ok(Nib {
        size: f("size", f64::from(s.tools.brush.size)).clamp(0.5, 2000.0),
        angle_deg: f("angle", 0.0).rem_euclid(360.0),
        aspect: f("aspect", 1.0).clamp(1.0, 10.0),
        shape,
        sens_size: sf("size", 1.0),
        sens_tilt: sf("tilt", 0.4),
        sens_speed: sf("speed", 0.8),
    })
}

fn ink(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = parse_points(p)?;
    let nib = nib_param(s, p)?;
    let id = crate::commands::layer_param(s, p)?;
    crate::commands::check_pixels_unlocked(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
    let fg = s.tools.foreground;
    let color = p
        .get("color")
        .and_then(Value::as_str)
        .and_then(crate::prefs::parse_hex)
        .map(|c| [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0])
        .unwrap_or(fg);
    let erase = p.get("erase").and_then(Value::as_bool).unwrap_or(false);
    let opacity = (p.get("opacity").and_then(Value::as_f64).unwrap_or(f64::from(s.tools.brush.opacity)) / 100.0).clamp(0.0, 1.0) as f32;
    let path = stroke_path(&nib, &pts);
    let mut painted = Rect::new(0, 0, 0, 0);
    s.edit("Ink", |doc, _| {
        let canvas = doc.bounds();
        let r = vector::fill_rasterizer(&path, vector::DEFAULT_TOLERANCE);
        let area = r.pixel_bounds().map_or(canvas, |b| b.inflate(1).intersect(&canvas));
        if area.is_empty() {
            return Ok(());
        }
        let mut cov = r.render(area);
        if let Some(selection) = doc.selection.as_ref() {
            let mask = redrob_photo_algo::selection::mask_from_surface(Some(selection), area);
            cov.iter_mut().zip(&mask).for_each(|(c, m)| *c *= m);
        }
        let locks = doc.effective_locks(id);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("the Ink tool needs a pixel layer".into()))?;
        if erase {
            // Erase: take the coverage out of the alpha.
            let fmt = surf.format();
            let ch = fmt.channels();
            let mut vals = surf.read_region(area);
            for (px, c) in vals.chunks_exact_mut(ch).zip(&cov) {
                if *c <= 0.0 {
                    continue;
                }
                let mut rgba = redrob_photo_raster::to_rgba(&fmt, px);
                rgba[3] *= 1.0 - c * opacity;
                redrob_photo_raster::from_rgba_into(&fmt, rgba, px);
            }
            surf.write_region(area, &vals);
            surf.prune();
        } else {
            crate::vector_cmds::paint_coverage(surf, area, &cov, color, opacity, BlendMode::Normal, locks.transparency);
        }
        painted = area;
        Ok(())
    })?;
    Ok(json!({ "points": pts.len(), "bounds": [painted.x0, painted.y0, painted.width(), painted.height()] }))
}

fn has_pixels(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    if d.doc.layer(id).and_then(redrob_photo_doc::Layer::surface).is_some() { Ok(()) } else { Err("the active layer has no pixels".into()) }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Ink",
        menu: &[],
        shortcut: None,
        params: r##"{"points":[[x,y,pressure?,tiltX?,tiltY?,rotation?,timeMs?],...],"size":px? (the brush size),"angle":deg=0,"aspect":1-10=1,"shape":"circle|square|diamond"="circle","sensitivity":{"size":0-1=1,"tilt":0-1=0.4,"speed":0-1=0.8},"color":"#rrggbb"?,"opacity":0-100?,"erase":bool=false,"layer":id?} → {points, bounds}"##,
        enabled: has_pixels,
        run: ink,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nib_shapes_and_hull() {
        let nib = Nib { size: 10.0, angle_deg: 0.0, aspect: 2.0, shape: 1, sens_size: 1.0, sens_tilt: 0.0, sens_speed: 0.0 };
        let p = InkPoint { x: 0.0, y: 0.0, pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, time_ms: 0.0 };
        let o = nib_outline(&nib, &p, None);
        assert_eq!(o.len(), 4);
        assert!((o[1].0 - 5.0).abs() < 1e-9 && (o[1].1 + 2.5).abs() < 1e-9);
        let light = InkPoint { pressure: 0.0, ..p };
        let o2 = nib_outline(&nib, &light, None);
        assert!(o2[1].0 < o[1].0, "light pressure thins the nib");
        let hull = convex_hull(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.5, 0.5)]);
        assert_eq!(hull.len(), 4);
        let path = stroke_path(&nib, &[p, InkPoint { x: 20.0, y: 0.0, ..p }]);
        assert_eq!(path.subpaths.len(), 2);
    }

    #[test]
    fn ink_paints_the_active_layer() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 60, "height": 40})).unwrap_or_else(|e| panic!("{e}"));
        let r = s.execute(CMD, json!({"points": [[10, 20, 1.0], [50, 20, 0.5]], "size": 8, "shape": "diamond"})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r["points"], 2);
        let b = r["bounds"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect::<Vec<_>>()).unwrap_or_default();
        assert!(b[2] >= 40, "{b:?}");
    }
}
