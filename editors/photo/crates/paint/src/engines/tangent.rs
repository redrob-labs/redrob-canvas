//! Tangent normal engine: paints the colours of a tangent-space normal map from the stylus tilt (or
//! the drawing direction, or the barrel rotation), so a normal map can be painted directly.

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use super::buffer::{PaintBuf, points_bounds};
use crate::dynamics::DabGenerator;
use crate::{BrushContext, Stroke, StrokePoint};

/// Where the normal comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NormalSource {
    /// The stylus tilt: the normal leans the way the pen leans.
    #[default]
    Tilt,
    /// The drawing direction (a mouse paints with this).
    Direction,
    /// The barrel rotation.
    Rotation,
}

/// Which axis a colour channel encodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Axis {
    #[default]
    PlusX,
    MinusX,
    PlusY,
    MinusY,
    PlusZ,
    MinusZ,
}

impl Axis {
    fn pick(self, n: [f32; 3]) -> f32 {
        match self {
            Axis::PlusX => n[0],
            Axis::MinusX => -n[0],
            Axis::PlusY => n[1],
            Axis::MinusY => -n[1],
            Axis::PlusZ => n[2],
            Axis::MinusZ => -n[2],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TangentSettings {
    pub source: NormalSource,
    /// Channel swizzle: the red, green and blue channels' axes (defaults to the OpenGL-style
    /// +X, +Y, +Z encoding; DirectX maps green to -Y).
    pub red: Axis,
    pub green: Axis,
    pub blue: Axis,
    /// 0..1 how strongly the tilt elevation (how far the pen leans) tips the normal.
    pub elevation_sensitivity: f32,
    /// Degrees added to the pen's azimuth.
    pub angle_offset: f32,
    /// Mix the painted normal with the normal already on the layer (0 = replace).
    pub mix: f32,
}

impl Default for TangentSettings {
    fn default() -> Self {
        Self { source: NormalSource::Tilt, red: Axis::PlusX, green: Axis::PlusY, blue: Axis::PlusZ, elevation_sensitivity: 1.0, angle_offset: 0.0, mix: 0.0 }
    }
}

/// The unit normal a stroke point gives.
pub fn normal_for(s: &TangentSettings, p: &StrokePoint, direction: f32) -> [f32; 3] {
    let az_offset = s.angle_offset.to_radians();
    let (azimuth, lean) = match s.source {
        NormalSource::Tilt => {
            let tilt = p.tilt_x.hypot(p.tilt_y).clamp(0.0, 90.0);
            if tilt < 1e-3 {
                (0.0, 0.0)
            } else {
                // Pointer-events tilt: +tiltX leans right, +tiltY leans towards the user (down).
                ((-p.tilt_y).atan2(p.tilt_x), tilt / 90.0)
            }
        }
        NormalSource::Direction => (direction, 1.0),
        NormalSource::Rotation => (p.rotation.to_radians(), 1.0),
    };
    let lean = (lean * s.elevation_sensitivity.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let elev = lean * std::f32::consts::FRAC_PI_2;
    let a = azimuth + az_offset;
    [elev.sin() * a.cos(), elev.sin() * a.sin(), elev.cos()]
}

/// The normal map colour (straight RGB) of a normal.
pub fn encode(s: &TangentSettings, n: [f32; 3]) -> [f32; 4] {
    let c = |axis: Axis| (axis.pick(n) * 0.5 + 0.5).clamp(0.0, 1.0);
    [c(s.red), c(s.green), c(s.blue), 1.0]
}

/// Paint a tangent-normal stroke. Returns the damaged rectangle.
pub fn paint(target: &mut Surface, stroke: &Stroke, s: &TangentSettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let b = &stroke.brush;
    // Walk the stroke step by step to keep each dab's input point (tilt, rotation) and direction.
    let mut g = DabGenerator::new(b, 1.0);
    let (mut ds, mut dual) = (Vec::new(), Vec::new());
    g.push(&stroke.points, &mut ds, &mut dual);
    g.finish(&mut ds, &mut dual);
    if ds.is_empty() {
        return Rect::EMPTY;
    }
    let ctx = BrushContext::new(b);
    let mut buf = PaintBuf::new(points_bounds(&stroke.points, crate::tip_reach(b.size / 2.0, b.hardness) + 2.0));
    // The input point nearest each dab (the dabs follow the smoothed path, in order), found by
    // walking the points along with the dabs.
    let pts: Vec<StrokePoint> = stroke.points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).copied().collect();
    let mut cursor = 0usize;
    let dist2 = |p: &StrokePoint, x: f64, y: f64| (p.x - x).powi(2) + (p.y - y).powi(2);
    let mut prev: Option<[f64; 2]> = None;
    let mix = s.mix.clamp(0.0, 1.0);
    for (i, d) in ds.iter().enumerate() {
        let direction = match prev {
            Some(q) => (-(d.center.y - q[1])).atan2(d.center.x - q[0]) as f32,
            None => 0.0,
        };
        prev = Some([d.center.x, d.center.y]);
        while cursor + 1 < pts.len() && dist2(&pts[cursor + 1], d.center.x, d.center.y) <= dist2(&pts[cursor], d.center.x, d.center.y) {
            cursor += 1;
        }
        let p = pts.get(cursor).copied().unwrap_or_default();
        let n = normal_for(s, &p, direction);
        let mut color = encode(s, n);
        if mix > 0.0 {
            let under = target.rgba(d.center.x.floor() as i32, d.center.y.floor() as i32);
            if under[3] > 0.0 {
                for c in 0..3 {
                    color[c] += (under[c] - color[c]) * mix;
                }
            }
        }
        let fp = ctx.footprint(d, i);
        let r = fp.rect.intersect(&buf.rect);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let c = fp.at(x, y) * d.opacity;
                if c > 0.0 {
                    buf.max(x, y, color, c);
                }
            }
        }
    }
    buf.composite(target, b.opacity, selection, lock, b.mode)
}
