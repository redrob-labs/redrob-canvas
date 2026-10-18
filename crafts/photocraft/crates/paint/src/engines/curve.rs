//! Curve engine: the stroke as evenly spaced line segments, plus extra lines that connect each new
//! point to the earlier points still in the history, which gives a woven, curve-like fill.

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use super::buffer::{PaintBuf, points_bounds};
use crate::Stroke;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CurveSettings {
    /// Line width in pixels.
    pub line_width: f32,
    /// Width follows the pen pressure (a mouse gets a pressure-like width from its speed).
    pub width_by_pressure: bool,
    /// How many earlier points a new point connects to (1..1000).
    pub history_size: u32,
    /// 0..1 opacity of the connecting curve lines.
    pub curve_opacity: f32,
    /// Curve opacity follows the pen pressure (or speed for a mouse).
    pub opacity_by_pressure: bool,
    /// Draw the stroke's own path between consecutive points.
    pub connection_line: bool,
    /// Pixels between the evenly spaced points the lines connect.
    pub spacing: f32,
    pub antialias: bool,
}

impl Default for CurveSettings {
    fn default() -> Self {
        Self {
            line_width: 1.0,
            width_by_pressure: true,
            history_size: 30,
            curve_opacity: 0.3,
            opacity_by_pressure: false,
            connection_line: true,
            spacing: 4.0,
            antialias: true,
        }
    }
}

/// Resample a polyline every `spacing` pixels, carrying pressure (or a speed-derived pressure for
/// input without any) along.
fn resample(stroke: &Stroke, spacing: f32) -> Vec<[f32; 3]> {
    let pts = &stroke.points;
    let mouse = pts.iter().all(|p| (p.pressure - 1.0).abs() < 1e-6);
    let mut out: Vec<[f32; 3]> = Vec::new();
    let spacing = f64::from(spacing.max(0.5));
    let mut carry = 0.0f64;
    let press = |i: usize| -> f32 {
        let p = &pts[i];
        if !mouse {
            return p.pressure.clamp(0.0, 1.0);
        }
        // Mouse: slow = heavy, fast = light (2 px/ms counts as full speed).
        if i == 0 {
            return 1.0;
        }
        let q = &pts[i - 1];
        let dt = p.time - q.time;
        if dt > 0.0 && dt.is_finite() {
            let v = ((p.x - q.x).hypot(p.y - q.y) / dt) as f32;
            (1.0 - (v / 2.0).clamp(0.0, 1.0)).max(0.1)
        } else {
            1.0
        }
    };
    for i in 0..pts.len() {
        let p = &pts[i];
        if !p.x.is_finite() || !p.y.is_finite() {
            continue;
        }
        if i == 0 {
            out.push([p.x as f32, p.y as f32, press(0)]);
            continue;
        }
        let q = &pts[i - 1];
        let (dx, dy) = (p.x - q.x, p.y - q.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 1e-9 {
            continue;
        }
        let (pa, pb) = (press(i - 1), press(i));
        let mut t = spacing - carry;
        while t <= len {
            let f = t / len;
            out.push([(q.x + dx * f) as f32, (q.y + dy * f) as f32, pa + (pb - pa) * f as f32]);
            t += spacing;
            if out.len() > 200_000 {
                return out;
            }
        }
        carry = len - (t - spacing);
    }
    if let (Some(l), Some(end)) = (pts.last(), out.last())
        && (((l.x as f32) - end[0]).abs() > 0.5 || ((l.y as f32) - end[1]).abs() > 0.5)
    {
        out.push([l.x as f32, l.y as f32, press(pts.len() - 1)]);
    }
    out
}

/// Paint a curve stroke. Returns the damaged rectangle.
pub fn paint(target: &mut Surface, stroke: &Stroke, s: &CurveSettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let b = &stroke.brush;
    let pts = resample(stroke, s.spacing);
    if pts.is_empty() {
        return Rect::EMPTY;
    }
    let mut buf = PaintBuf::new(points_bounds(&stroke.points, s.line_width.max(1.0) + 2.0));
    let hist = s.history_size.clamp(1, 1000) as usize;
    let aa = s.antialias;
    let color = b.color;
    for (i, p) in pts.iter().enumerate() {
        let pressure = p[2];
        let width = if s.width_by_pressure { s.line_width.max(0.1) * pressure.max(0.05) } else { s.line_width.max(0.1) };
        let k_curve = if s.opacity_by_pressure { s.curve_opacity.clamp(0.0, 1.0) * pressure } else { s.curve_opacity.clamp(0.0, 1.0) };
        if i == 0 {
            buf.disc(p[0], p[1], width / 2.0, color, 1.0, aa);
            continue;
        }
        if s.connection_line {
            let q = pts[i - 1];
            buf.line(q[0], q[1], p[0], p[1], width, color, 1.0, aa);
        }
        if k_curve > 0.0 {
            let start = i.saturating_sub(hist);
            for q in &pts[start..i.saturating_sub(1)] {
                buf.line(q[0], q[1], p[0], p[1], width.min(1.0), color, k_curve, aa);
            }
        }
    }
    buf.composite(target, b.opacity, selection, lock, b.mode)
}
