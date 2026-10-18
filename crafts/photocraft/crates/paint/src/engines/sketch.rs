//! Sketch engine: a line-based "harmony" brush. Every dab connects the current point to the earlier
//! points that lie inside the tip area, so a slow, overlapping stroke shades itself with a web of
//! lines (behaviour from the reference manual; the model is our own).

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use super::buffer::{PaintBuf, points_bounds, shift_hsv};
use crate::rng::{rand_signed, rand01};
use crate::{Stroke, dabs};

/// Most earlier points one dab may connect to (a long stroke stays bounded).
pub const MAX_HISTORY: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SketchSettings {
    /// Width of the connecting lines in pixels.
    pub line_width: f32,
    /// 0..1: how far past both points a line extends (0 = exactly between them).
    pub offset_scale: f32,
    /// 0..1 fraction of the candidate points a dab connects to.
    pub density: f32,
    /// Fewer connections to far points (density falls off with the distance).
    pub distance_density: bool,
    /// Lines join the points themselves; off, each line is drawn from the current point towards
    /// the earlier one but stops at `offset_scale` of the way.
    pub magnetify: bool,
    /// 0..1 random variation of the line colour (hue, saturation, value).
    pub random_rgb: f32,
    /// 0..1 random variation of the line opacity.
    pub random_opacity: f32,
    /// Fainter lines to far points.
    pub distance_opacity: bool,
    /// Only connect to the previous point (a plain polyline).
    pub simple_mode: bool,
    pub antialias: bool,
    /// Draw the stroke's own path besides the connections.
    pub paint_connection: bool,
}

impl Default for SketchSettings {
    fn default() -> Self {
        Self {
            line_width: 1.0,
            offset_scale: 0.0,
            density: 0.5,
            distance_density: true,
            magnetify: true,
            random_rgb: 0.0,
            random_opacity: 0.0,
            distance_opacity: true,
            simple_mode: false,
            antialias: true,
            paint_connection: true,
        }
    }
}

/// Paint a sketch stroke. Returns the damaged rectangle.
pub fn paint(target: &mut Surface, stroke: &Stroke, s: &SketchSettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let b = &stroke.brush;
    let ds = dabs(stroke);
    if ds.is_empty() {
        return Rect::EMPTY;
    }
    let reach = b.size.max(1.0) * (1.0 + s.offset_scale.clamp(0.0, 1.0)) + s.line_width.max(1.0);
    let mut buf = PaintBuf::new(points_bounds(&stroke.points, reach));
    let seed = b.seed;
    let mut history: Vec<[f32; 2]> = Vec::new();
    let aa = s.antialias;
    let off = s.offset_scale.clamp(0.0, 1.0);
    for (i, d) in ds.iter().enumerate() {
        let p = [d.center.x as f32, d.center.y as f32];
        let r = d.radius.max(0.5);
        let base = d.color;
        let flow = d.alpha * d.opacity;
        let width = s.line_width.max(0.1) * if b.pressure_size { (d.radius / (b.size / 2.0).max(0.5)).clamp(0.05, 1.0) } else { 1.0 };
        let mut line_no: u64 = 0;
        let mut draw = |buf: &mut PaintBuf, q: [f32; 2], k: f32| {
            let li = ((i as u64) << 20) | line_no;
            line_no += 1;
            let mut color = base;
            if s.random_rgb > 0.0 {
                let j = s.random_rgb.clamp(0.0, 1.0);
                color = shift_hsv(color, rand_signed(seed, li, 101) * j * 0.5, rand_signed(seed, li, 102) * j, rand_signed(seed, li, 103) * j);
            }
            let mut k = k;
            if s.random_opacity > 0.0 {
                k *= 1.0 - s.random_opacity.clamp(0.0, 1.0) * rand01(seed, li, 104);
            }
            let (ax, ay, bx, by) = if s.magnetify {
                // Between the points, extended past both by the offset.
                (p[0] + (p[0] - q[0]) * off, p[1] + (p[1] - q[1]) * off, q[0] + (q[0] - p[0]) * off, q[1] + (q[1] - p[1]) * off)
            } else {
                // From the current point towards the earlier one, `offset` of the way (half by default).
                let t = if off > 0.0 { off } else { 0.5 };
                (p[0], p[1], p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t)
            };
            buf.line(ax, ay, bx, by, width, color, k.clamp(0.0, 1.0), aa);
        };
        if s.paint_connection || s.simple_mode {
            if let Some(prev) = history.last() {
                draw(&mut buf, *prev, flow);
            } else {
                buf.disc(p[0], p[1], width / 2.0, base, flow, aa);
            }
        }
        if !s.simple_mode {
            let n = history.len();
            for (hi, q) in history.iter().enumerate().take(n.saturating_sub(1)) {
                let dist = (q[0] - p[0]).hypot(q[1] - p[1]);
                if dist > r {
                    continue;
                }
                let near = 1.0 - dist / r;
                let mut prob = s.density.clamp(0.0, 1.0);
                if s.distance_density {
                    prob *= near;
                }
                let key = (i as u64) * 0x9E37 + hi as u64;
                if rand01(seed, key, 100) >= prob {
                    continue;
                }
                let k = if s.distance_opacity { flow * near } else { flow };
                draw(&mut buf, *q, k);
            }
        }
        history.push(p);
        if history.len() > MAX_HISTORY {
            history.remove(0);
        }
    }
    buf.composite(target, b.opacity, selection, lock, b.mode)
}
