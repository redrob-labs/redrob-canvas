//! Dyna engine: a satellite with mass and drag chases the pointer; its speed sets the width, and the
//! trail is drawn as circles, two circles, lines, polygons or a wire.

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use super::buffer::{PaintBuf, points_bounds};
use crate::Stroke;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DynaShape {
    #[default]
    Circle,
    /// Two circles, one at each side of the trail.
    Two,
    /// Line segments across the trail.
    Lines,
    Polygon,
    /// A wire: consecutive trail positions connected.
    Wire,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DynaSettings {
    /// Inertia of the satellite (0.01..2): heavier = lazier.
    pub mass: f32,
    /// Velocity lost per step (0..1).
    pub drag: f32,
    /// Base width in pixels.
    pub width: f32,
    /// 0..1: how much the speed thins the trail (0 = constant width).
    pub width_range: f32,
    /// Keep the shapes at `angle` instead of turning with the motion.
    pub fixed_angle: bool,
    /// Degrees.
    pub angle: f32,
    pub shape: DynaShape,
    /// Parallel trails (1..16).
    pub line_count: u32,
    /// Pixels between parallel trails.
    pub line_spacing: f32,
    /// Polygon sides (3..12).
    pub polygon_sides: u32,
    /// Draw the stroke's own path besides the satellite's.
    pub paint_connection: bool,
    pub antialias: bool,
}

impl Default for DynaSettings {
    fn default() -> Self {
        Self {
            mass: 0.1,
            drag: 0.15,
            width: 5.0,
            width_range: 0.5,
            fixed_angle: false,
            angle: 0.0,
            shape: DynaShape::Circle,
            line_count: 1,
            line_spacing: 4.0,
            polygon_sides: 5,
            paint_connection: false,
            antialias: true,
        }
    }
}

/// Paint a dyna stroke. Returns the damaged rectangle.
pub fn paint(target: &mut Surface, stroke: &Stroke, s: &DynaSettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let b = &stroke.brush;
    let pts: Vec<[f32; 2]> = stroke.points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).map(|p| [p.x as f32, p.y as f32]).collect();
    let Some(first) = pts.first().copied() else { return Rect::EMPTY };
    let lines = s.line_count.clamp(1, 16);
    let reach = s.width.max(1.0) * 2.0 + s.line_spacing.abs() * lines as f32 + b.size.max(1.0);
    let mut buf = PaintBuf::new(points_bounds(&stroke.points, reach));
    let (mut pos, mut vel) = (first, [0.0f32; 2]);
    let mass = s.mass.clamp(0.01, 2.0);
    let drag = s.drag.clamp(0.0, 1.0);
    let color = b.color;
    let aa = s.antialias;
    let mut prev_trail: Option<Vec<[f32; 2]>> = None;
    let mut last_input = first;
    for p in &pts {
        if s.paint_connection {
            buf.line(last_input[0], last_input[1], p[0], p[1], 1.0, color, 1.0, aa);
        }
        last_input = *p;
        // Sub-step so a fast pointer still leaves a continuous trail.
        let dist = (p[0] - pos[0]).hypot(p[1] - pos[1]);
        let steps = (dist / 2.0).ceil().clamp(1.0, 64.0) as u32;
        for _ in 0..steps {
            let (fx, fy) = ((p[0] - pos[0]) / mass * 0.02, (p[1] - pos[1]) / mass * 0.02);
            vel[0] = (vel[0] + fx) * (1.0 - drag);
            vel[1] = (vel[1] + fy) * (1.0 - drag);
            let from = pos;
            pos[0] += vel[0];
            pos[1] += vel[1];
            let speed = vel[0].hypot(vel[1]);
            // Faster = thinner, by the width range.
            let w = (s.width.max(0.5) * (1.0 - s.width_range.clamp(0.0, 1.0) * (speed / 10.0).clamp(0.0, 1.0))).max(0.5);
            let dir = if s.fixed_angle || speed < 1e-3 { s.angle.to_radians() } else { (-vel[1]).atan2(vel[0]) };
            let (nx, ny) = (-dir.sin(), -dir.cos());
            let mut trail = Vec::with_capacity(lines as usize);
            for li in 0..lines {
                let off = (li as f32 - (lines as f32 - 1.0) / 2.0) * s.line_spacing;
                let (cx, cy) = (pos[0] + nx * off, pos[1] + ny * off);
                trail.push([cx, cy]);
                match s.shape {
                    DynaShape::Circle => {
                        buf.line(from[0] + nx * off, from[1] + ny * off, cx, cy, w, color, 1.0, aa);
                    }
                    DynaShape::Two => {
                        buf.disc(cx + nx * w, cy + ny * w, w / 2.0, color, 1.0, aa);
                        buf.disc(cx - nx * w, cy - ny * w, w / 2.0, color, 1.0, aa);
                    }
                    DynaShape::Lines => {
                        buf.line(cx - nx * w, cy - ny * w, cx + nx * w, cy + ny * w, 1.0, color, 1.0, aa);
                    }
                    DynaShape::Polygon => {
                        let n = s.polygon_sides.clamp(3, 12);
                        let poly: Vec<[f32; 2]> = (0..n)
                            .map(|k| {
                                let a = dir + std::f32::consts::TAU * k as f32 / n as f32;
                                [cx + a.cos() * w, cy - a.sin() * w]
                            })
                            .collect();
                        buf.polygon(&poly, color, 1.0, false, !aa);
                    }
                    DynaShape::Wire => {
                        if let Some(prev) = prev_trail.as_ref().and_then(|t| t.get(li as usize)) {
                            buf.line(prev[0], prev[1], cx, cy, 1.0, color, 1.0, aa);
                        }
                    }
                }
            }
            if s.shape == DynaShape::Wire && lines > 1 {
                for w2 in trail.windows(2) {
                    buf.line(w2[0][0], w2[0][1], w2[1][0], w2[1][1], 1.0, color, 0.5, aa);
                }
            }
            prev_trail = Some(trail);
        }
    }
    buf.composite(target, b.opacity, selection, lock, b.mode)
}
