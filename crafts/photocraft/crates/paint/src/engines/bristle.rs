//! Bristle engine: individual bristles seeded from the tip trail along the stroke, deplete their
//! ink as they go and may soak up the colour of the canvas they cross.

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use super::buffer::{PaintBuf, hsv_to_rgb, points_bounds, rgb_to_hsv, surface_rgba};
use crate::curve::ResponseCurve;
use crate::dynamics::DabGenerator;
use crate::rng::{rand_signed, rand01};
use crate::{Stroke, TipShape};

/// Most bristles one brush carries.
pub const MAX_BRISTLES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BristleSettings {
    /// Scale of the bristle spread relative to the tip (0.1..4).
    pub scale: f32,
    /// Pixels of random offset per bristle per step.
    pub random_offset: f32,
    /// Shear of the bristle positions (-2..2).
    pub shear: f32,
    /// 0..1 fraction of the tip's pixels that seed a bristle.
    pub density: f32,
    /// A mouse's speed stands in for pressure (fast = light).
    pub mouse_pressure: bool,
    /// Tip value a pixel needs to seed a bristle (0..1).
    pub threshold: f32,
    /// Connect each bristle's successive positions with a line (off: dots).
    pub connect_hairs: bool,
    pub antialias: bool,
    /// Bristles blend over each other (off: they only raise the coverage).
    pub composite_bristles: bool,
    /// Ink runs out along the stroke.
    pub ink_depletion: bool,
    /// Depletion lowers the opacity.
    pub deplete_opacity: bool,
    /// Depletion lowers the saturation.
    pub deplete_saturation: bool,
    /// Ink left (output) over the stroke's progress (input, 0..1 over `ink_amount` steps).
    pub depletion_curve: ResponseCurve,
    /// Steps of stroke the ink lasts.
    pub ink_amount: u32,
    /// Bristles pick up the colour of the canvas they cross.
    pub soak_ink: bool,
    /// Bristle line width in pixels.
    pub width: f32,
}

impl Default for BristleSettings {
    fn default() -> Self {
        Self {
            scale: 1.0,
            random_offset: 0.5,
            shear: 0.0,
            density: 0.3,
            mouse_pressure: false,
            threshold: 0.3,
            connect_hairs: true,
            antialias: true,
            composite_bristles: false,
            ink_depletion: false,
            deplete_opacity: true,
            deplete_saturation: true,
            depletion_curve: ResponseCurve::new(&[[0.0, 1.0], [1.0, 0.0]]),
            ink_amount: 1000,
            soak_ink: false,
            width: 1.0,
        }
    }
}

/// One bristle: its offset from the tip centre (in tip units, -1..1) and its ink colour.
#[derive(Clone, Copy, Debug)]
struct Bristle {
    ox: f32,
    oy: f32,
    color: [f32; 4],
}

/// Seed bristles from the tip: where the tip's value passes the threshold, with probability `density`.
fn seed_bristles(stroke: &Stroke, s: &BristleSettings) -> Vec<Bristle> {
    let b = &stroke.brush;
    let seed = b.seed;
    let mut out = Vec::new();
    let density = s.density.clamp(0.0, 1.0);
    let threshold = s.threshold.clamp(0.0, 1.0);
    // Sample the tip on a grid fine enough for a few hundred candidates.
    let n = 48u32;
    for gy in 0..n {
        for gx in 0..n {
            let (u, v) = ((gx as f32 + 0.5) / n as f32, (gy as f32 + 0.5) / n as f32);
            let value = match &b.tip {
                TipShape::Round => {
                    let d = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt() * 2.0;
                    crate::tip_falloff(d, 1.0, b.hardness)
                }
                other => other.bitmap().filter(|t| t.is_valid()).map_or(0.0, |t| {
                    let x = ((u * t.width as f32) as u32).min(t.width - 1);
                    let y = ((v * t.height as f32) as u32).min(t.height - 1);
                    t.get(x, y)
                }),
            };
            if value < threshold.max(1e-3) {
                continue;
            }
            let idx = u64::from(gy * n + gx);
            if rand01(seed, idx, 501) >= density {
                continue;
            }
            out.push(Bristle { ox: (u - 0.5) * 2.0, oy: (v - 0.5) * 2.0, color: b.color });
            if out.len() >= MAX_BRISTLES {
                return out;
            }
        }
    }
    if out.is_empty() {
        out.push(Bristle { ox: 0.0, oy: 0.0, color: b.color });
    }
    out
}

/// Paint a bristle stroke. Returns the damaged rectangle.
pub fn paint(target: &mut Surface, stroke: &Stroke, s: &BristleSettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let b = &stroke.brush;
    let mut g = DabGenerator::new(b, 1.0);
    let (mut ds, mut dual) = (Vec::new(), Vec::new());
    g.push(&stroke.points, &mut ds, &mut dual);
    g.finish(&mut ds, &mut dual);
    if ds.is_empty() {
        return Rect::EMPTY;
    }
    let scale = s.scale.clamp(0.1, 4.0);
    let reach = b.size.max(1.0) * scale * (1.0 + s.shear.abs()) + s.random_offset.abs() * 4.0 + s.width + 2.0;
    let mut buf = PaintBuf::new(points_bounds(&stroke.points, reach));
    buf.accumulate = s.composite_bristles;
    let seed = b.seed;
    let mut bristles = seed_bristles(stroke, s);
    let full_r = (b.size / 2.0).max(0.5);
    let mut prev: Option<Vec<[f32; 2]>> = None;
    let mut prev_center: Option<[f64; 2]> = None;
    let ink_steps = s.ink_amount.max(1) as f32;
    let aa = s.antialias;
    let width = s.width.max(0.5);
    let mouse = stroke.points.iter().all(|p| (p.pressure - 1.0).abs() < 1e-6);
    for (si, d) in ds.iter().enumerate() {
        let (cx, cy) = (d.center.x as f32, d.center.y as f32);
        // Pressure from the dab size; for a mouse, optionally from the speed between steps.
        let mut pressure = (d.radius / full_r).clamp(0.05, 1.0);
        if mouse
            && s.mouse_pressure
            && let Some(q) = prev_center
        {
            let dist = ((d.center.x - q[0]).hypot(d.center.y - q[1])) as f32;
            pressure = (1.0 - (dist / (b.size.max(1.0))).clamp(0.0, 0.9)).max(0.1);
        }
        prev_center = Some([d.center.x, d.center.y]);
        let r = d.radius.max(0.5) * scale;
        let (ca, sa) = (d.angle.cos(), -d.angle.sin());
        let progress = (si as f32 / ink_steps).min(1.0);
        let ink_left = if s.ink_depletion { s.depletion_curve.eval(progress) } else { 1.0 };
        let mut positions = Vec::with_capacity(bristles.len());
        for (bi, br) in bristles.iter_mut().enumerate() {
            let idx = ((si as u64) << 16) | bi as u64;
            // Tip space → document: scale, shear, rotate, plus the random offset.
            let (mut ox, oy) = (br.ox * r, br.oy * r * d.roundness.max(0.05));
            ox += s.shear.clamp(-2.0, 2.0) * oy;
            let (dx, dy) = (ox * ca - oy * sa, ox * sa + oy * ca);
            let jitter = s.random_offset.max(0.0);
            let x = cx + dx + rand_signed(seed, idx, 502) * jitter;
            let y = cy + dy + rand_signed(seed, idx, 503) * jitter;
            positions.push([x, y]);
            // Soak: the bristle's colour drifts towards the canvas it crosses.
            if s.soak_ink {
                let under = surface_rgba(target, x, y);
                if under[3] > 0.0 {
                    for c in 0..3 {
                        br.color[c] += (under[c] - br.color[c]) * 0.1 * under[3];
                    }
                }
            }
            let mut color = br.color;
            let mut k = d.opacity * d.alpha * pressure;
            if s.ink_depletion {
                if s.deplete_opacity {
                    k *= ink_left;
                }
                if s.deplete_saturation {
                    let [h, sat, v] = rgb_to_hsv([color[0], color[1], color[2]]);
                    let o = hsv_to_rgb(h, sat * ink_left, v);
                    color = [o[0], o[1], o[2], color[3]];
                }
            }
            if k <= 0.0 {
                continue;
            }
            match (s.connect_hairs, prev.as_ref().and_then(|p| p.get(bi))) {
                (true, Some(from)) => buf.line(from[0], from[1], x, y, width, color, k, aa),
                _ => {
                    if aa {
                        buf.point_aa(x, y, color, k);
                    } else {
                        buf.point(x, y, color, k);
                    }
                }
            }
        }
        prev = Some(positions);
    }
    buf.composite(target, b.opacity, selection, lock, b.mode)
}
