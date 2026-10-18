//! Hatching engine: a fixed pattern of parallel lines in document space, revealed through the tip.
//! The lines never move with the stroke, so overlapping strokes continue the same hatching.

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use super::buffer::{PaintBuf, points_bounds};
use crate::{BrushContext, Stroke, dabs};

/// Which extra line families cross the base hatching.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Crosshatch {
    #[default]
    None,
    /// The base lines plus the perpendicular family.
    Perpendicular,
    /// The base lines, then a -45° family, then a +45° family as the pressure grows.
    MinusThenPlus,
    /// The base lines, then +45°, then -45°.
    PlusThenMinus,
    /// Perpendicular lines at a slightly different separation: interference fringes.
    Moire,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HatchingSettings {
    /// Degrees, counter-clockwise.
    pub angle: f32,
    /// Distance between lines in pixels.
    pub separation: f32,
    /// Line thickness in pixels.
    pub thickness: f32,
    /// Document point one line passes through.
    pub origin_x: f32,
    pub origin_y: f32,
    pub crosshatch: Crosshatch,
    /// Pen pressure narrows the separation (1 at full pressure, wider when light).
    pub separation_dynamics: bool,
    /// Pen pressure thickens the lines.
    pub thickness_dynamics: bool,
    pub antialias: bool,
    /// Paint the background colour between the lines.
    pub colored_background: bool,
    /// Pressure from which the second crosshatch family appears (0..1).
    pub second_threshold: f32,
    /// Pressure from which the third family appears.
    pub third_threshold: f32,
}

impl Default for HatchingSettings {
    fn default() -> Self {
        Self {
            angle: 45.0,
            separation: 8.0,
            thickness: 1.0,
            origin_x: 0.0,
            origin_y: 0.0,
            crosshatch: Crosshatch::None,
            separation_dynamics: false,
            thickness_dynamics: false,
            antialias: true,
            colored_background: false,
            second_threshold: 0.4,
            third_threshold: 0.7,
        }
    }
}

/// Anti-aliased coverage of a line family at a point: lines every `sep` pixels, `thick` wide,
/// along `angle` radians through the origin.
fn family(x: f32, y: f32, angle: f32, sep: f32, thick: f32, ox: f32, oy: f32, aa: bool) -> f32 {
    let (ca, sa) = (angle.cos(), angle.sin());
    // Signed distance across the lines (perpendicular to their direction).
    let d = -(x - ox) * sa + (y - oy) * ca;
    let sep = sep.max(0.5);
    let m = d.rem_euclid(sep);
    let dist = m.min(sep - m);
    let hw = (thick.max(0.1)) / 2.0;
    if aa {
        (hw + 0.5 - dist).clamp(0.0, 1.0)
    } else if dist <= hw.max(0.5) {
        1.0
    } else {
        0.0
    }
}

/// Paint a hatching stroke. Returns the damaged rectangle.
pub fn paint(target: &mut Surface, stroke: &Stroke, s: &HatchingSettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let b = &stroke.brush;
    let ds = dabs(stroke);
    if ds.is_empty() {
        return Rect::EMPTY;
    }
    let ctx = BrushContext::new(b);
    let mut buf = PaintBuf::new(points_bounds(&stroke.points, crate::tip_reach(b.size / 2.0, b.hardness) + 2.0));
    let base_angle = s.angle.to_radians();
    let full_r = (b.size / 2.0).max(0.5);
    for (i, d) in ds.iter().enumerate() {
        let fp = ctx.footprint(d, i);
        // Pressure through the dab radius (Shape Dynamics › Size or the pressure button).
        let pressure = if b.pressure_size || b.shape_dynamics.enabled { (d.radius / full_r).clamp(0.0, 1.0) } else { 1.0 };
        let sep = if s.separation_dynamics { s.separation * (1.5 - 0.5 * pressure).max(0.3) } else { s.separation };
        let thick = if s.thickness_dynamics { s.thickness * (0.3 + 0.7 * pressure) } else { s.thickness };
        let mut families: Vec<(f32, f32)> = vec![(base_angle, sep)];
        match s.crosshatch {
            Crosshatch::None => {}
            Crosshatch::Perpendicular => families.push((base_angle + std::f32::consts::FRAC_PI_2, sep)),
            Crosshatch::MinusThenPlus => {
                if pressure >= s.second_threshold.clamp(0.0, 1.0) {
                    families.push((base_angle - std::f32::consts::FRAC_PI_4, sep));
                }
                if pressure >= s.third_threshold.clamp(0.0, 1.0) {
                    families.push((base_angle + std::f32::consts::FRAC_PI_4, sep));
                }
            }
            Crosshatch::PlusThenMinus => {
                if pressure >= s.second_threshold.clamp(0.0, 1.0) {
                    families.push((base_angle + std::f32::consts::FRAC_PI_4, sep));
                }
                if pressure >= s.third_threshold.clamp(0.0, 1.0) {
                    families.push((base_angle - std::f32::consts::FRAC_PI_4, sep));
                }
            }
            Crosshatch::Moire => {
                families.push((base_angle + std::f32::consts::FRAC_PI_2, sep));
                families.push((base_angle + std::f32::consts::FRAC_PI_2 + 0.06, sep * 1.07));
            }
        }
        let r = fp.rect.intersect(&buf.rect);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let c = fp.at(x, y) * d.opacity;
                if c <= 0.0 {
                    continue;
                }
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let h = families.iter().map(|&(a, sp)| family(px, py, a, sp, thick, s.origin_x, s.origin_y, s.antialias)).fold(0.0f32, f32::max);
                if h > 0.0 {
                    buf.max(x, y, d.color, c * h);
                }
                if s.colored_background && h < 1.0 {
                    buf.max(x, y, b.background, c * (1.0 - h));
                }
            }
        }
    }
    buf.composite(target, b.opacity, selection, lock, b.mode)
}
