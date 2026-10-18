//! Grid engine: particles on a document-space grid under the cursor, for halftone and retro
//! effects. Every grid cell the dab covers is split into `division × division` sub-cells and each
//! gets one particle.

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use super::buffer::{PaintBuf, points_bounds, shift_hsv, surface_rgba};
use crate::rng::{hash2, rand_signed, rand01};
use crate::{Stroke, dab_coverage, dabs};

/// The particle drawn in each sub-cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GridParticle {
    #[default]
    Ellipse,
    Rectangle,
    /// A diagonal line across the sub-cell.
    Line,
    /// One whole pixel at the sub-cell centre.
    Pixel,
    /// An anti-aliased pixel at the sub-cell centre.
    AntialiasedPixel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GridSettings {
    /// Grid cell size in pixels.
    pub grid_width: f32,
    pub grid_height: f32,
    /// Sub-cells per cell side (1..64).
    pub division: u32,
    /// Pressure lowers the division (light = coarser).
    pub division_by_pressure: bool,
    /// Pixels kept clear at each sub-cell's sides.
    pub horizontal_border: f32,
    pub vertical_border: f32,
    /// 0..1: random offset of each particle within its sub-cell.
    pub jitter: f32,
    /// 0..1 scale of the particle inside its sub-cell.
    pub scale: f32,
    pub particle: GridParticle,
    /// 0..1 random hue, saturation and value variation per particle.
    pub random_hsv: [f32; 3],
    /// 0..1 random opacity per particle.
    pub random_opacity: f32,
    /// A random colour (full hue wheel) per particle.
    pub color_per_particle: bool,
    /// Take each particle's colour from the layer under its sub-cell.
    pub sample_input: bool,
    /// Fill the sub-cell with the background colour behind the particle.
    pub fill_background: bool,
    /// Mix each particle's colour with the background colour by a random amount.
    pub mix_background: bool,
}

impl Default for GridSettings {
    fn default() -> Self {
        Self {
            grid_width: 25.0,
            grid_height: 25.0,
            division: 2,
            division_by_pressure: false,
            horizontal_border: 0.0,
            vertical_border: 0.0,
            jitter: 0.0,
            scale: 1.0,
            particle: GridParticle::Ellipse,
            random_hsv: [0.0; 3],
            random_opacity: 0.0,
            color_per_particle: false,
            sample_input: false,
            fill_background: false,
            mix_background: false,
        }
    }
}

/// Paint a grid stroke. `sample` is the surface particle colours are read from with
/// `sample_input` (the layer itself when None). Returns the damaged rectangle.
pub fn paint(target: &mut Surface, sample: Option<&Surface>, stroke: &Stroke, s: &GridSettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let b = &stroke.brush;
    let ds = dabs(stroke);
    if ds.is_empty() {
        return Rect::EMPTY;
    }
    let (gw, gh) = (s.grid_width.max(1.0), s.grid_height.max(1.0));
    let mut buf = PaintBuf::new(points_bounds(&stroke.points, b.size.max(1.0) + gw.max(gh)));
    let seed = b.seed;
    let full_r = (b.size / 2.0).max(0.5);
    // Each cell is painted once per stroke, however many dabs cover it.
    let mut done = std::collections::HashSet::new();
    let aa = !b.aliased;
    for d in &ds {
        let (cx, cy, r) = (d.center.x as f32, d.center.y as f32, d.radius.max(0.5));
        let pressure = if b.pressure_size || b.shape_dynamics.enabled { (d.radius / full_r).clamp(0.05, 1.0) } else { 1.0 };
        let mut division = s.division.clamp(1, 64);
        if s.division_by_pressure {
            division = ((division as f32 * pressure).round() as u32).clamp(1, 64);
        }
        let (ix0, ix1) = (((cx - r) / gw).floor() as i32, ((cx + r) / gw).ceil() as i32);
        let (iy0, iy1) = (((cy - r) / gh).floor() as i32, ((cy + r) / gh).ceil() as i32);
        for gy in iy0..iy1 {
            for gx in ix0..ix1 {
                // The cell is covered when its centre lies inside the dab.
                let (ccx, ccy) = (gx as f32 * gw + gw / 2.0, gy as f32 * gh + gh / 2.0);
                let cov = dab_coverage((ccx - cx).hypot(ccy - cy), r, b.hardness);
                if cov <= 0.0 || !done.insert((gx, gy, division)) {
                    continue;
                }
                let (sw, sh) = (gw / division as f32, gh / division as f32);
                for sy in 0..division {
                    for sx in 0..division {
                        let key = hash2(gx * 64 + sx as i32, gy * 64 + sy as i32, seed);
                        let idx = ((gx as i64 as u64) << 40) ^ ((gy as i64 as u64) << 20) ^ ((sx as u64) << 8) ^ sy as u64;
                        let (x0, y0) = (gx as f32 * gw + sx as f32 * sw, gy as f32 * gh + sy as f32 * sh);
                        let (bw, bh) = (s.horizontal_border.clamp(0.0, sw / 2.0), s.vertical_border.clamp(0.0, sh / 2.0));
                        let (iw, ih) = ((sw - 2.0 * bw).max(0.5), (sh - 2.0 * bh).max(0.5));
                        let j = s.jitter.clamp(0.0, 1.0);
                        let (jx, jy) = (rand_signed(seed, idx, 201) * j * iw / 2.0, rand_signed(seed, idx, 202) * j * ih / 2.0);
                        let (pcx, pcy) = (x0 + sw / 2.0 + jx, y0 + sh / 2.0 + jy);
                        // Colour.
                        let mut color = if s.sample_input {
                            let src = sample.map_or_else(|| surface_rgba(target, pcx, pcy), |sf| surface_rgba(sf, pcx, pcy));
                            if src[3] <= 0.0 { d.color } else { [src[0], src[1], src[2], 1.0] }
                        } else if s.color_per_particle {
                            let o = super::buffer::hsv_to_rgb(key, 1.0, 1.0);
                            [o[0], o[1], o[2], 1.0]
                        } else {
                            d.color
                        };
                        let [hj, sj, vj] = s.random_hsv;
                        if hj > 0.0 || sj > 0.0 || vj > 0.0 {
                            color = shift_hsv(
                                color,
                                rand_signed(seed, idx, 203) * hj.clamp(0.0, 1.0) * 0.5,
                                rand_signed(seed, idx, 204) * sj.clamp(0.0, 1.0),
                                rand_signed(seed, idx, 205) * vj.clamp(0.0, 1.0),
                            );
                        }
                        if s.mix_background {
                            let t = rand01(seed, idx, 206);
                            for (c, bg) in b.background.iter().enumerate().take(3) {
                                color[c] += (bg - color[c]) * t;
                            }
                        }
                        let mut k = d.opacity * d.alpha;
                        if s.random_opacity > 0.0 {
                            k *= 1.0 - s.random_opacity.clamp(0.0, 1.0) * rand01(seed, idx, 207);
                        }
                        if s.fill_background {
                            buf.rect_shape(x0 + sw / 2.0, y0 + sh / 2.0, sw, sh, 0.0, b.background, k, false);
                        }
                        let scale = s.scale.clamp(0.01, 4.0);
                        let (pw, ph) = (iw * scale, ih * scale);
                        match s.particle {
                            GridParticle::Ellipse => buf.ellipse(pcx, pcy, pw / 2.0, ph / 2.0, 0.0, color, k, aa),
                            GridParticle::Rectangle => buf.rect_shape(pcx, pcy, pw, ph, 0.0, color, k, aa),
                            GridParticle::Line => buf.line(pcx - pw / 2.0, pcy + ph / 2.0, pcx + pw / 2.0, pcy - ph / 2.0, 1.0, color, k, aa),
                            GridParticle::Pixel => buf.point(pcx, pcy, color, k),
                            GridParticle::AntialiasedPixel => buf.point_aa(pcx, pcy, color, k),
                        }
                    }
                }
            }
        }
    }
    buf.composite(target, b.opacity, selection, lock, b.mode)
}
