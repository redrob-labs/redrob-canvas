// SPDX-License-Identifier: MIT OR Apache-2.0
//! Frame renderers behind the animation filters (Rippling, Waves, Spinning Globe): each takes
//! the source over the bounds and a phase 0…1 and returns one frame's native samples over the
//! bounds. The engine turns the frames into layers.

use redrob_photo_geom::Rect;

use crate::image::{Edge, Image};
use crate::{AnimEdge, Ctx};

fn edge(e: AnimEdge) -> Edge {
    match e {
        AnimEdge::Wrap => Edge::Wrap,
        AnimEdge::Smear => Edge::Repeat,
        AnimEdge::Black => Edge::Transparent,
    }
}

fn remap(src: &Image, ctx: &Ctx, e: Edge, f: impl Fn(f32, f32) -> (f32, f32)) -> Vec<f32> {
    let out: Rect = ctx.bounds;
    let n = src.ch;
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut tmp = vec![0.0f32; n];
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (sx, sy) = f(x as f32 + 0.5, y as f32 + 0.5);
            src.sample(sx, sy, e, ctx.bounds, ctx.alpha, &mut tmp);
            res.extend_from_slice(&tmp);
        }
    }
    res
}

/// Rippling: the image sways by `strength` px along travelling sine waves in both directions;
/// `phase` 0…1 walks one full cycle so the frames loop.
pub fn ripple_frame(src: &Image, ctx: &Ctx, phase: f32, strength: f32, e: AnimEdge) -> Vec<f32> {
    let b = ctx.bounds;
    let (w, h) = (b.width() as f32, b.height() as f32);
    let k = std::f32::consts::TAU * phase;
    let amp = strength.max(0.0);
    remap(src, ctx, edge(e), |x, y| {
        let u = (x - b.x0 as f32) / w.max(1.0);
        let v = (y - b.y0 as f32) / h.max(1.0);
        let dx = amp * (std::f32::consts::TAU * 3.0 * v + k).sin();
        let dy = amp * (std::f32::consts::TAU * 4.0 * u - k).sin();
        (x + dx, y + dy)
    })
}

/// Waves: concentric ripples of `amplitude` px and `wavelength` px travel out from the centre
/// (inward with `invert`); `phase` 0…1 is one wavelength of travel so the frames loop.
pub fn waves_frame(src: &Image, ctx: &Ctx, phase: f32, amplitude: f32, wavelength: f32, invert: bool, e: AnimEdge) -> Vec<f32> {
    let b = ctx.bounds;
    let (cx, cy) = (b.x0 as f32 + b.width() as f32 / 2.0, b.y0 as f32 + b.height() as f32 / 2.0);
    let wl = wavelength.max(1.0);
    let dir = if invert { -1.0 } else { 1.0 };
    remap(src, ctx, edge(e), |x, y| {
        let (dx, dy) = (x - cx, y - cy);
        let r = (dx * dx + dy * dy).sqrt().max(1e-3);
        let d = amplitude * (std::f32::consts::TAU * (r / wl - dir * phase)).sin();
        (x + dx / r * d, y + dy / r * d)
    })
}

/// Spinning Globe: the image wrapped round a sphere filling the bounds, turned by `turn` (0…1 of
/// a full revolution, to the left when `turn_left`). Outside the globe the frame is transparent
/// (or black without alpha).
pub fn globe_frame(src: &Image, ctx: &Ctx, turn: f32, turn_left: bool) -> Vec<f32> {
    let b = ctx.bounds;
    let (w, h) = (b.width() as f32, b.height() as f32);
    let (cx, cy) = (b.x0 as f32 + w / 2.0, b.y0 as f32 + h / 2.0);
    let radius = (w.min(h) / 2.0).max(1.0);
    let spin = std::f32::consts::TAU * turn * if turn_left { -1.0 } else { 1.0 };
    let n = src.ch;
    let mut res = Vec::with_capacity(b.width() as usize * b.height() as usize * n);
    let mut tmp = vec![0.0f32; n];
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            let u = (x as f32 + 0.5 - cx) / radius;
            let v = (y as f32 + 0.5 - cy) / radius;
            let d2 = u * u + v * v;
            if d2 > 1.0 {
                tmp.iter_mut().for_each(|s| *s = 0.0);
                res.extend_from_slice(&tmp);
                continue;
            }
            let z = (1.0 - d2).sqrt();
            let lon = u.atan2(z) + spin;
            let lat = v.clamp(-1.0, 1.0).asin();
            let sx = b.x0 as f32 + (lon / std::f32::consts::TAU + 0.5).rem_euclid(1.0) * w;
            let sy = b.y0 as f32 + (lat / std::f32::consts::PI + 0.5).clamp(0.0, 0.9999) * h;
            src.sample(sx, sy, Edge::Wrap, b, ctx.alpha, &mut tmp);
            // Soft rim.
            if ctx.alpha {
                let rim = ((1.0 - d2.sqrt()) * radius + 0.5).clamp(0.0, 1.0);
                tmp[n - 1] *= rim;
            }
            res.extend_from_slice(&tmp);
        }
    }
    res
}
