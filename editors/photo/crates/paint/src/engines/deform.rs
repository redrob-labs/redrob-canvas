//! Deform engine: a live pixel-pushing brush. Every dab remaps the pixels under it (grow, shrink,
//! swirl, move, lens zoom) or shifts their colour, sampling the image as it was before the dab (or
//! before the whole stroke, with `use_undeformed`).

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use super::buffer::{hsv_to_rgb, rgb_to_hsv};
use crate::Stroke;
use crate::retouch::{Region, alpha_index, apply_dab_stroke};
use redrob_photo_raster::{from_rgba, to_rgba};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeformMode {
    #[default]
    Grow,
    Shrink,
    SwirlCw,
    SwirlCcw,
    /// Push the pixels along the stroke.
    Move,
    LensZoomIn,
    LensZoomOut,
    /// Turn the hue of the pixels under the dab.
    ColorDeform,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DeformSettings {
    pub mode: DeformMode,
    /// 0..1 strength per dab.
    pub amount: f32,
    /// Bilinear interpolation of the sampled pixels (off: nearest).
    pub bilinear: bool,
    /// Deform on every n-th dab only (1 = every dab).
    pub counter: u32,
    /// Sample the image as it was before the stroke instead of after the previous dab.
    pub use_undeformed: bool,
}

impl Default for DeformSettings {
    fn default() -> Self {
        Self { mode: DeformMode::Grow, amount: 0.2, bilinear: true, counter: 1, use_undeformed: false }
    }
}

/// Bilinear (or nearest) sample of a region at a document position, in native channels.
fn sample(src: &Region, x: f32, y: f32, bilinear: bool, out: &mut [f32]) -> bool {
    let r = src.rect;
    if !x.is_finite() || !y.is_finite() {
        return false;
    }
    if !bilinear {
        let (xi, yi) = (x.floor() as i32, y.floor() as i32);
        if !r.contains(xi, yi) {
            return false;
        }
        out.copy_from_slice(src.px(xi, yi));
        return true;
    }
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let (x0, y0) = (x0 as i32, y0 as i32);
    if !r.contains(x0, y0) || !r.contains(x0 + 1, y0 + 1) {
        let (xi, yi) = (x.floor() as i32, y.floor() as i32);
        if !r.contains(xi, yi) {
            return false;
        }
        out.copy_from_slice(src.px(xi, yi));
        return true;
    }
    let (a, b, c, d) = (src.px(x0, y0), src.px(x0 + 1, y0), src.px(x0, y0 + 1), src.px(x0 + 1, y0 + 1));
    for (i, o) in out.iter_mut().enumerate() {
        let top = a[i] + (b[i] - a[i]) * tx;
        let bot = c[i] + (d[i] - c[i]) * tx;
        *o = top + (bot - top) * ty;
    }
    true
}

/// Paint a deform stroke. Returns the damaged rectangle.
pub fn paint(target: &mut Surface, stroke: &Stroke, s: &DeformSettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let fmt = target.format();
    let n = fmt.channels();
    let a = alpha_index(&fmt);
    let amount = s.amount.clamp(0.0, 1.0);
    let counter = s.counter.max(1) as usize;
    // The halo: a dab may pull pixels from up to one radius (shrink doubles the reach) around it.
    let halo = (stroke.brush.size.max(1.0) * (1.0 + amount)).ceil() as i32 + 2;
    let mut original: Option<Region> = None;
    let mut prev_center: Option<[f64; 2]> = None;
    let mut px = vec![0.0f32; n];
    apply_dab_stroke(target, stroke, selection, lock, halo, |work, fp| {
        if s.use_undeformed && original.is_none() {
            original = Some(work.clone());
        }
        let center = [fp.dab.center.x, fp.dab.center.y];
        let dir = prev_center.map(|q| ((center[0] - q[0]) as f32, (center[1] - q[1]) as f32));
        prev_center = Some(center);
        if fp.index % counter != 0 {
            return;
        }
        let r = fp.rect.intersect(&work.rect);
        if r.is_empty() {
            return;
        }
        let radius = fp.dab.radius.max(0.5);
        let (cx, cy) = (center[0] as f32, center[1] as f32);
        // What this dab samples from: the undeformed image, or the work copy before this dab
        // (a snapshot of its reach, so the remap does not read pixels it has just written).
        let snap_rect = Rect::new(r.x0 - halo, r.y0 - halo, r.x1 + halo, r.y1 + halo).intersect(&work.rect);
        let snapshot;
        let src: &Region = match &original {
            Some(o) => o,
            None => {
                snapshot = work.crop(snap_rect);
                &snapshot
            }
        };
        let k_dab = fp.dab.alpha.clamp(0.0, 1.0) * fp.dab.opacity.clamp(0.0, 1.0);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let f = fp.at(x, y);
                if f <= 0.0 {
                    continue;
                }
                let (pxf, pyf) = (x as f32 + 0.5, y as f32 + 0.5);
                let (dx, dy) = (pxf - cx, pyf - cy);
                let dist = dx.hypot(dy);
                let dn = (dist / radius).min(1.0);
                let strength = amount * f * k_dab;
                if s.mode == DeformMode::ColorDeform {
                    let dst = work.px_mut(x, y);
                    let c = to_rgba(&fmt, dst);
                    let [h, sat, v] = rgb_to_hsv([c[0], c[1], c[2]]);
                    let o = hsv_to_rgb(h + strength * 0.5, sat, v);
                    let enc = from_rgba(&fmt, [o[0], o[1], o[2], c[3]]);
                    dst.copy_from_slice(&enc[..n]);
                    continue;
                }
                let (sx, sy) = match s.mode {
                    DeformMode::Grow => (cx + dx * (1.0 - strength), cy + dy * (1.0 - strength)),
                    DeformMode::Shrink => (cx + dx * (1.0 + strength), cy + dy * (1.0 + strength)),
                    DeformMode::SwirlCw | DeformMode::SwirlCcw => {
                        let sign = if s.mode == DeformMode::SwirlCw { 1.0 } else { -1.0 };
                        let ang = sign * strength * std::f32::consts::FRAC_PI_2;
                        let (ca, sa) = (ang.cos(), ang.sin());
                        (cx + dx * ca - dy * sa, cy + dx * sa + dy * ca)
                    }
                    DeformMode::Move => match dir {
                        Some((mx, my)) => (pxf - mx * strength * 2.0, pyf - my * strength * 2.0),
                        None => continue,
                    },
                    DeformMode::LensZoomIn => {
                        let z = 1.0 - strength * (1.0 - dn * dn);
                        (cx + dx * z, cy + dy * z)
                    }
                    DeformMode::LensZoomOut => {
                        let z = 1.0 + strength * (1.0 - dn * dn);
                        (cx + dx * z, cy + dy * z)
                    }
                    DeformMode::ColorDeform => (pxf, pyf),
                };
                if !sample(src, sx, sy, s.bilinear, &mut px) {
                    continue;
                }
                let dst = work.px_mut(x, y);
                match a {
                    Some(ai) if lock => {
                        for c in 0..n {
                            if c != ai {
                                dst[c] += (px[c] - dst[c]) * f;
                            }
                        }
                    }
                    _ => {
                        for c in 0..n {
                            dst[c] += (px[c] - dst[c]) * f;
                        }
                    }
                }
            }
        }
    })
}
