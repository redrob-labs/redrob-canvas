//! Particle engine: a bundle of particles trails the pointer, each pulled towards it by a gravity
//! and damped, leaving a thin wire. The wires spread with the distance scales and settle when the
//! pointer stops.

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use super::buffer::{PaintBuf, points_bounds};
use crate::Stroke;
use crate::rng::rand_signed;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ParticleSettings {
    /// Number of wires (1..500).
    pub particles: u32,
    /// 0..1 opacity of each wire step.
    pub opacity_weight: f32,
    /// How strongly a particle is pulled towards the pointer per iteration (0..1).
    pub gravity: f32,
    /// Simulation steps per input point (1..200).
    pub iterations: u32,
    /// Scale of the pull along x and y (1 = as is; smaller spreads the wires).
    pub dx_scale: f32,
    pub dy_scale: f32,
    /// Velocity kept per iteration (0..1).
    pub damping: f32,
    pub antialias: bool,
}

impl Default for ParticleSettings {
    fn default() -> Self {
        Self { particles: 50, opacity_weight: 0.3, gravity: 0.98, iterations: 10, dx_scale: 1.0, dy_scale: 1.0, damping: 0.9, antialias: true }
    }
}

/// Paint a particle stroke. Returns the damaged rectangle.
pub fn paint(target: &mut Surface, stroke: &Stroke, s: &ParticleSettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let b = &stroke.brush;
    let pts: Vec<[f32; 2]> = stroke.points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).map(|p| [p.x as f32, p.y as f32]).collect();
    let Some(first) = pts.first().copied() else { return Rect::EMPTY };
    let n = s.particles.clamp(1, 500) as usize;
    let seed = b.seed;
    let spread = b.size.max(1.0);
    // Particles start scattered around the first point, at rest.
    let mut pos: Vec<[f32; 2]> =
        (0..n).map(|i| [first[0] + rand_signed(seed, i as u64, 301) * spread, first[1] + rand_signed(seed, i as u64, 302) * spread]).collect();
    let mut vel: Vec<[f32; 2]> = vec![[0.0; 2]; n];
    // The wires may swing out by a few sizes.
    let mut buf = PaintBuf::new(points_bounds(&stroke.points, spread * 6.0 + 4.0));
    buf.accumulate = true;
    let g = s.gravity.clamp(0.0, 1.0);
    let damp = s.damping.clamp(0.0, 1.0);
    let (sx, sy) = (s.dx_scale.clamp(0.01, 10.0), s.dy_scale.clamp(0.01, 10.0));
    let k = s.opacity_weight.clamp(0.0, 1.0);
    let iters = s.iterations.clamp(1, 200);
    let color = b.color;
    for target_pt in &pts {
        for _ in 0..iters {
            for i in 0..n {
                let (dx, dy) = ((target_pt[0] - pos[i][0]) * sx, (target_pt[1] - pos[i][1]) * sy);
                // Gravity: a fraction of the remaining distance per step, spread by the index so
                // the wires don't collapse onto one path.
                let gi = g * (0.02 + 0.08 * (i as f32 / n as f32));
                vel[i][0] = (vel[i][0] + dx * gi) * damp;
                vel[i][1] = (vel[i][1] + dy * gi) * damp;
                let from = pos[i];
                pos[i][0] += vel[i][0];
                pos[i][1] += vel[i][1];
                buf.line(from[0], from[1], pos[i][0], pos[i][1], 1.0, color, k, s.antialias);
            }
        }
    }
    buf.composite(target, b.opacity, selection, lock, b.mode)
}
