//! Spray engine: at every spacing step, many particles land in an elliptical area around the
//! pointer. Their radial and angular distributions, shape, size, rotation and colour are all
//! options; the particles may also be the brush's own sampled tip.

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use super::buffer::{PaintBuf, hsv_to_rgb, points_bounds, shift_hsv, surface_rgba};
use crate::curve::ResponseCurve;
use crate::dynamics::DabGenerator;
use crate::rng::{rand_signed, rand01};
use crate::tile::Mips;
use crate::{Stroke, TipShape};

/// Most particles one step may spray.
pub const MAX_PARTICLES: u32 = 4096;

/// How particle distances (or angles) spread over the area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Distribution {
    #[default]
    Uniform,
    /// Denser towards the centre (angle: towards the 0° direction).
    Gaussian,
    /// Pulled towards the centre by `clustering` (negative: towards the rim).
    Cluster,
    /// Through the `curve` (input: a uniform draw; output: the normalised radius or angle).
    Curve,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParticleShape {
    #[default]
    Ellipse,
    Rectangle,
    AntialiasedPixel,
    Pixel,
    /// A short line along the particle's rotation.
    Line,
    /// The brush's sampled tip bitmap (a computed tip falls back to an ellipse).
    Tip,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParticleRotation {
    #[default]
    Fixed,
    Random,
    /// Turn with the stroke direction.
    FollowCursor,
    /// Turn with the drawing angle plus a random weight.
    Weighted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SpraySettings {
    /// Area diameter in pixels (0 = the brush size).
    pub diameter: f32,
    /// Height / width of the area (1 = a circle).
    pub aspect: f32,
    /// Area rotation in degrees.
    pub angle: f32,
    /// 0..2 scale of the area with the dab size (pressure).
    pub scale: f32,
    /// 0..1 random variation of the area size per step.
    pub jitter: f32,
    /// Particles per step (with `use_density` off).
    pub count: u32,
    /// Particles as a density (per 1000 px²) instead of a count.
    pub use_density: bool,
    pub density: f32,
    pub radial: Distribution,
    pub angular: Distribution,
    /// Cluster amount for the Cluster distributions (-10..10).
    pub clustering: f32,
    /// Gaussian spread (0.05..1) for the Gaussian distributions.
    pub sigma: f32,
    pub radial_curve: ResponseCurve,
    pub angular_curve: ResponseCurve,
    pub shape: ParticleShape,
    /// Particle width and height in pixels.
    pub shape_width: f32,
    pub shape_height: f32,
    /// Size the particle with the dab (pressure) instead of fixed pixels.
    pub proportional: bool,
    /// Vary the particle size randomly (30..100 %).
    pub random_size: bool,
    pub rotation: ParticleRotation,
    /// Degrees for fixed rotation; the weight (0..1) for weighted rotation.
    pub rotation_angle: f32,
    pub rotation_weight: f32,
    /// 0..1 random opacity per particle.
    pub random_opacity: f32,
    /// 0..1 random hue, saturation, value per particle.
    pub random_hsv: [f32; 3],
    /// Take each particle's colour from the layer where it lands.
    pub sample_input: bool,
    /// A random colour (full hue wheel) per particle.
    pub color_per_particle: bool,
    /// Mix with the background colour by a random amount per particle.
    pub mix_background: bool,
    /// Colour from the gradient (foreground → background) by the particle's distance from the centre.
    pub gradient_by_distance: bool,
    pub antialias: bool,
}

impl Default for SpraySettings {
    fn default() -> Self {
        Self {
            diameter: 0.0,
            aspect: 1.0,
            angle: 0.0,
            scale: 1.0,
            jitter: 0.0,
            count: 12,
            use_density: false,
            density: 10.0,
            radial: Distribution::Uniform,
            angular: Distribution::Uniform,
            clustering: 0.0,
            sigma: 0.5,
            radial_curve: ResponseCurve::LINEAR,
            angular_curve: ResponseCurve::LINEAR,
            shape: ParticleShape::Ellipse,
            shape_width: 4.0,
            shape_height: 4.0,
            proportional: false,
            random_size: false,
            rotation: ParticleRotation::Fixed,
            rotation_angle: 0.0,
            rotation_weight: 0.5,
            random_opacity: 0.0,
            random_hsv: [0.0; 3],
            sample_input: false,
            color_per_particle: false,
            mix_background: false,
            gradient_by_distance: false,
            antialias: true,
        }
    }
}

/// A 0..1 draw shaped by a distribution: Uniform keeps it, Gaussian pulls it to 0 (centre),
/// Cluster pulls it by `clustering`, Curve maps it through `curve`.
fn distribute(u: f32, u2: f32, d: Distribution, clustering: f32, sigma: f32, curve: &ResponseCurve) -> f32 {
    match d {
        Distribution::Uniform => u,
        Distribution::Gaussian => {
            // Box–Muller magnitude, folded to 0..1 and scaled by sigma.
            let r = (-2.0 * u.max(1e-6).ln()).sqrt() * (std::f32::consts::TAU * u2).cos();
            (r.abs() * sigma.clamp(0.05, 1.0) * 0.5).min(1.0)
        }
        Distribution::Cluster => {
            let c = clustering.clamp(-10.0, 10.0);
            if c >= 0.0 { u.powf(1.0 + c) } else { 1.0 - (1.0 - u).powf(1.0 - c) }
        }
        Distribution::Curve => curve.eval(u),
    }
}

/// Paint a spray stroke. `sample` is the surface particle colours are read from with
/// `sample_input` (the layer itself when None). Returns the damaged rectangle.
pub fn paint(target: &mut Surface, sample: Option<&Surface>, stroke: &Stroke, s: &SpraySettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let b = &stroke.brush;
    let mut g = DabGenerator::new(b, 1.0);
    let (mut ds, mut dual) = (Vec::new(), Vec::new());
    g.push(&stroke.points, &mut ds, &mut dual);
    g.finish(&mut ds, &mut dual);
    if ds.is_empty() {
        return Rect::EMPTY;
    }
    let area_d = if s.diameter > 0.0 { s.diameter } else { b.size.max(1.0) };
    let aspect = s.aspect.clamp(0.05, 20.0);
    let max_shape = s.shape_width.max(s.shape_height).max(1.0) * if s.proportional { b.size.max(1.0) / 10.0 } else { 1.0 };
    let reach = area_d * (1.0 + s.jitter.clamp(0.0, 1.0)) * s.scale.clamp(0.0, 2.0).max(1.0) * aspect.max(1.0) / 2.0 + max_shape * 2.0 + 2.0;
    let mut buf = PaintBuf::new(points_bounds(&stroke.points, reach));
    let seed = b.seed;
    let full_r = (b.size / 2.0).max(0.5);
    let tip_mips = match &b.tip {
        TipShape::Round => None,
        other => other.bitmap().filter(|t| t.is_valid()).map(Mips::new),
    };
    let aa = s.antialias;
    let mut prev: Option<[f64; 2]> = None;
    for (si, d) in ds.iter().enumerate() {
        let direction = match prev {
            Some(q) => (-(d.center.y - q[1])).atan2(d.center.x - q[0]) as f32,
            None => 0.0,
        };
        prev = Some([d.center.x, d.center.y]);
        let pressure = (d.radius / full_r).clamp(0.05, 2.0);
        let step_key = si as u64;
        // Area for this step.
        let mut rx = area_d / 2.0 * (1.0 + (s.scale.clamp(0.0, 2.0) - 1.0) * (pressure - 1.0).max(-0.95));
        if s.jitter > 0.0 {
            rx *= 1.0 - s.jitter.clamp(0.0, 1.0) * rand01(seed, step_key, 401);
        }
        let rx = rx.max(0.5);
        let ry = (rx * aspect).max(0.5);
        let area_angle = s.angle.to_radians();
        let count = if s.use_density {
            let area_px = std::f32::consts::PI * rx * ry;
            ((s.density.max(0.0) * area_px / 1000.0).round() as u32).clamp(1, MAX_PARTICLES)
        } else {
            s.count.clamp(1, MAX_PARTICLES)
        };
        let (cx, cy) = (d.center.x as f32, d.center.y as f32);
        for k in 0..count {
            let idx = (step_key << 16) | u64::from(k);
            let r = |st: u64| rand01(seed, idx, st);
            // Position: radius and angle through their distributions.
            let rad = distribute(r(402), r(403), s.radial, s.clustering, s.sigma, &s.radial_curve).clamp(0.0, 1.0).sqrt();
            let ang = distribute(r(404), r(405), s.angular, s.clustering, s.sigma, &s.angular_curve) * std::f32::consts::TAU;
            let (ex, ey) = (rad * rx * ang.cos(), rad * ry * ang.sin());
            let (px, py) = (cx + ex * area_angle.cos() + ey * area_angle.sin(), cy - ex * area_angle.sin() + ey * area_angle.cos());
            // Size.
            let base_scale = if s.proportional { b.size.max(1.0) / 10.0 } else { 1.0 };
            let size_k = if s.random_size { 0.3 + 0.7 * r(406) } else { 1.0 };
            let (pw, ph) = ((s.shape_width.max(0.5) * base_scale * size_k).max(0.5), (s.shape_height.max(0.5) * base_scale * size_k).max(0.5));
            // Rotation.
            let rot = match s.rotation {
                ParticleRotation::Fixed => s.rotation_angle.to_radians(),
                ParticleRotation::Random => r(407) * std::f32::consts::TAU,
                ParticleRotation::FollowCursor => direction + s.rotation_angle.to_radians(),
                ParticleRotation::Weighted => direction + rand_signed(seed, idx, 407) * s.rotation_weight.clamp(0.0, 1.0) * std::f32::consts::PI,
            };
            // Colour.
            let mut color = if s.sample_input {
                let src = sample.map_or_else(|| surface_rgba(target, px, py), |sf| surface_rgba(sf, px, py));
                if src[3] <= 0.0 { d.color } else { [src[0], src[1], src[2], 1.0] }
            } else if s.color_per_particle {
                let o = hsv_to_rgb(r(408), 1.0, 1.0);
                [o[0], o[1], o[2], 1.0]
            } else if s.gradient_by_distance {
                let t = rad * rad;
                let mut c = d.color;
                for (ch, bg) in b.background.iter().enumerate().take(4) {
                    c[ch] += (bg - c[ch]) * t;
                }
                c
            } else {
                d.color
            };
            let [hj, sj, vj] = s.random_hsv;
            if hj > 0.0 || sj > 0.0 || vj > 0.0 {
                color = shift_hsv(
                    color,
                    rand_signed(seed, idx, 409) * hj.clamp(0.0, 1.0) * 0.5,
                    rand_signed(seed, idx, 410) * sj.clamp(0.0, 1.0),
                    rand_signed(seed, idx, 411) * vj.clamp(0.0, 1.0),
                );
            }
            if s.mix_background {
                let t = r(412);
                for (ch, bg) in b.background.iter().enumerate().take(3) {
                    color[ch] += (bg - color[ch]) * t;
                }
            }
            let mut opa = d.opacity * d.alpha;
            if s.random_opacity > 0.0 {
                opa *= 1.0 - s.random_opacity.clamp(0.0, 1.0) * r(413);
            }
            match (s.shape, tip_mips.as_ref()) {
                (ParticleShape::Tip, Some(m)) => {
                    // The tip bitmap scaled to the particle size, turned by the rotation.
                    let (hw, hh) = (pw / 2.0, ph / 2.0);
                    let level = m.level_for(pw.max(ph));
                    let reach = hw.hypot(hh).ceil() as i32 + 1;
                    let (ca, sa) = (rot.cos(), -rot.sin());
                    for y in (py.floor() as i32 - reach)..=(py.floor() as i32 + reach) {
                        for x in (px.floor() as i32 - reach)..=(px.floor() as i32 + reach) {
                            let (dx, dy) = (x as f32 + 0.5 - px, y as f32 + 0.5 - py);
                            let (u, v) = ((dx * ca - dy * sa) / (2.0 * hw) + 0.5, (dx * sa + dy * ca) / (2.0 * hh) + 0.5);
                            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                                continue;
                            }
                            let c = m.sample_clamped(level, u, v);
                            if c > 0.0 {
                                buf.max(x, y, color, opa * c);
                            }
                        }
                    }
                }
                (ParticleShape::Ellipse, _) | (ParticleShape::Tip, None) => buf.ellipse(px, py, pw / 2.0, ph / 2.0, rot, color, opa, aa),
                (ParticleShape::Rectangle, _) => buf.rect_shape(px, py, pw, ph, rot, color, opa, aa),
                (ParticleShape::AntialiasedPixel, _) => buf.point_aa(px, py, color, opa),
                (ParticleShape::Pixel, _) => buf.point(px, py, color, opa),
                (ParticleShape::Line, _) => {
                    let (lx, ly) = (rot.cos() * pw / 2.0, -rot.sin() * pw / 2.0);
                    buf.line(px - lx, py - ly, px + lx, py + ly, ph.min(pw).clamp(1.0, 3.0), color, opa, aa);
                }
            }
        }
    }
    buf.composite(target, b.opacity, selection, lock, b.mode)
}
