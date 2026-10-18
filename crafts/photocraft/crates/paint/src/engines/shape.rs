//! Shape engine: the stroke's outline becomes one filled shape. The points are smoothed, displaced
//! along their normals by the pointer speed, closed and filled with the winding rule of choice.

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use super::buffer::{PaintBuf, points_bounds};
use crate::Stroke;

/// Most outline points the shape keeps (a very long stroke is decimated).
pub const MAX_POINTS: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShapeSettings {
    /// 0..1: how much the pointer speed pushes the outline outwards (0 = the path as drawn).
    pub speed: f32,
    /// Smoothing passes over the outline (0..8).
    pub smooth: u32,
    /// Pixels the outline is pushed along its normal (plus the speed term).
    pub displace: f32,
    /// Fill with the even-odd rule instead of non-zero winding.
    pub even_odd: bool,
    /// No anti-aliasing on the edge.
    pub hard_edge: bool,
}

impl Default for ShapeSettings {
    fn default() -> Self {
        Self { speed: 0.3, smooth: 2, displace: 0.0, even_odd: false, hard_edge: false }
    }
}

/// The closed outline (pixel-centre coordinates) a stroke's points make.
pub fn outline(stroke: &Stroke, s: &ShapeSettings) -> Vec<[f32; 2]> {
    let pts = &stroke.points;
    let mut raw: Vec<[f32; 3]> = Vec::with_capacity(pts.len());
    let step = (pts.len() / MAX_POINTS).max(1);
    for (i, p) in pts.iter().enumerate().step_by(step) {
        if !p.x.is_finite() || !p.y.is_finite() {
            continue;
        }
        // Speed in px/ms over the segment into this point (0 without timestamps).
        let speed = if i > 0 {
            let q = &pts[i - 1];
            let dt = p.time - q.time;
            if dt > 0.0 && dt.is_finite() { ((p.x - q.x).hypot(p.y - q.y) / dt) as f32 } else { 0.0 }
        } else {
            0.0
        };
        raw.push([p.x as f32, p.y as f32, speed]);
    }
    raw.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3);
    if raw.len() < 2 {
        return raw.iter().map(|p| [p[0], p[1]]).collect();
    }
    // Displace along the normal: the displacement plus the speed term, scaled by the brush size.
    let scale = stroke.brush.size.max(1.0);
    let n = raw.len();
    let mut out: Vec<[f32; 2]> = Vec::with_capacity(n);
    for i in 0..n {
        let prev = raw[i.saturating_sub(1)];
        let next = raw[(i + 1).min(n - 1)];
        let (dx, dy) = (next[0] - prev[0], next[1] - prev[1]);
        let len = dx.hypot(dy).max(1e-6);
        let (nx, ny) = (-dy / len, dx / len);
        let speed_term = s.speed.clamp(0.0, 1.0) * raw[i][2].min(4.0) * scale * 0.5;
        let d = s.displace + speed_term;
        out.push([raw[i][0] + nx * d, raw[i][1] + ny * d]);
    }
    // Chaikin smoothing on the open path (endpoints kept).
    for _ in 0..s.smooth.min(8) {
        if out.len() < 3 {
            break;
        }
        let mut sm = Vec::with_capacity(out.len() * 2);
        sm.push(out[0]);
        for w in out.windows(2) {
            let (a, b) = (w[0], w[1]);
            sm.push([a[0] * 0.75 + b[0] * 0.25, a[1] * 0.75 + b[1] * 0.25]);
            sm.push([a[0] * 0.25 + b[0] * 0.75, a[1] * 0.25 + b[1] * 0.75]);
        }
        sm.push(out[out.len() - 1]);
        out = sm;
        if out.len() > MAX_POINTS * 2 {
            break;
        }
    }
    out
}

/// Paint a shape stroke. Returns the damaged rectangle.
pub fn paint(target: &mut Surface, stroke: &Stroke, s: &ShapeSettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let b = &stroke.brush;
    let poly = outline(stroke, s);
    if poly.is_empty() {
        return Rect::EMPTY;
    }
    let reach = s.displace.abs() + s.speed.clamp(0.0, 1.0) * 2.0 * b.size.max(1.0) + b.size.max(1.0);
    let mut buf = PaintBuf::new(points_bounds(&stroke.points, reach));
    let aa = !s.hard_edge;
    if poly.len() < 3 {
        // A click or a two-point stroke: a dot or a line of the brush size.
        let w = b.size.max(1.0);
        match poly.as_slice() {
            [p] => buf.disc(p[0], p[1], w / 2.0, b.color, 1.0, aa),
            [p, q] => buf.line(p[0], p[1], q[0], q[1], w, b.color, 1.0, aa),
            _ => {}
        }
    } else {
        buf.polygon(&poly, b.color, 1.0, s.even_odd, s.hard_edge);
    }
    buf.composite(target, b.opacity, selection, lock, b.mode)
}
