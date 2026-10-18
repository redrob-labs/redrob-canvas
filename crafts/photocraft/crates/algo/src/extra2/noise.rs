//! Noise generators of the fourth batch: Perlin Noise, Plasma and Solid Noise. All three render
//! from nothing (the layer's pixels are replaced inside the bounds).

use redrob_photo_geom::Rect;

use super::super::extra::map_px;
use crate::Ctx;
use crate::image::Image;
use crate::noise::hash01;

/// A pseudo-random unit-ish gradient at lattice point `(x, y, z)`.
fn gradient(x: i32, y: i32, z: i32, seed: u32) -> [f32; 3] {
    let h = |c: u32| hash01(x, y.wrapping_mul(31).wrapping_add(z.wrapping_mul(131)), c, seed) * 2.0 - 1.0;
    let g = [h(1), h(2), h(3)];
    let len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-3);
    [g[0] / len, g[1] / len, g[2] / len]
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Classic 3-D gradient noise in about -1..1.
pub(crate) fn perlin3(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let (ix, iy, iz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
    let (fx, fy, fz) = (x - ix as f32, y - iy as f32, z - iz as f32);
    let (u, v, w) = (fade(fx), fade(fy), fade(fz));
    let dot = |dx: i32, dy: i32, dz: i32| {
        let g = gradient(ix + dx, iy + dy, iz + dz, seed);
        g[0] * (fx - dx as f32) + g[1] * (fy - dy as f32) + g[2] * (fz - dz as f32)
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(dot(0, 0, 0), dot(1, 0, 0), u);
    let x10 = lerp(dot(0, 1, 0), dot(1, 1, 0), u);
    let x01 = lerp(dot(0, 0, 1), dot(1, 0, 1), u);
    let x11 = lerp(dot(0, 1, 1), dot(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// Perlin Noise: `iterations` octaves of gradient noise, each octave's weight falling by
/// `2^-alpha`, at `scale` cells across the bounds, sliced at `z_offset`. Greyscale.
pub(crate) fn perlin_noise(src: &Image, out: Rect, ctx: &Ctx, alpha: f32, scale: f32, z_offset: f32, iterations: u32) -> Vec<f32> {
    let b = ctx.bounds;
    let side = b.width().max(b.height()).max(1) as f32;
    let freq = scale.clamp(0.0, 20.0) * 4.0 / side;
    let octaves = iterations.clamp(1, 8);
    let alpha = alpha.clamp(0.0, 4.0);
    map_px(src, out, ctx, |x, y, _| {
        let (px, py) = ((x - b.x0) as f32 * freq, (y - b.y0) as f32 * freq);
        let mut sum = 0.0;
        let mut amp = 1.0;
        let mut norm = 0.0;
        let mut f = 1.0;
        for o in 0..octaves {
            sum += perlin3(px * f + 7.3, py * f + 3.1, z_offset * f, 0x51ed.wrapping_add(o)) * amp;
            norm += amp;
            amp *= 2f32.powf(-alpha);
            f *= 2.0;
        }
        let v = ((sum / norm.max(1e-6)) * 0.75 + 0.5).clamp(0.0, 1.0);
        [v, v, v, 1.0]
    })
}

/// Plasma: three independent fractal noise fields, one per colour channel, with `turbulence`
/// setting how rough (and how saturated) the cloud is.
pub(crate) fn plasma(src: &Image, out: Rect, ctx: &Ctx, turbulence: f32, seed: u32) -> Vec<f32> {
    let b = ctx.bounds;
    let side = b.width().max(b.height()).max(1) as f32;
    let turb = turbulence.clamp(0.1, 7.0);
    // Rougher plasma: more octaves that fall off more slowly.
    let octaves = (2.0 + turb).round().clamp(2.0, 9.0) as u32;
    let gain = (0.45 + turb * 0.08).min(0.9);
    let channel = |px: f32, py: f32, c: u32| -> f32 {
        let mut sum = 0.0;
        let mut amp = 1.0;
        let mut norm = 0.0;
        let mut f = 2.0 / side;
        for o in 0..octaves {
            sum += perlin3(px * f, py * f, c as f32 * 17.0 + 0.5, seed.wrapping_add(o.wrapping_mul(8191))) * amp;
            norm += amp;
            amp *= gain;
            f *= 2.0;
        }
        // Stretch the sum to fill the channel: steeper for turbulent plasma.
        let v = sum / norm.max(1e-6);
        (0.5 + v * (1.6 + turb * 0.35)).clamp(0.0, 1.0)
    };
    map_px(src, out, ctx, |x, y, _| {
        let (px, py) = ((x - b.x0) as f32, (y - b.y0) as f32);
        [channel(px, py, 0), channel(px, py, 1), channel(px, py, 2), 1.0]
    })
}

/// Lattice value hash that wraps every `period` cells when `period > 0`.
fn lattice(x: i32, y: i32, period_x: i32, period_y: i32, seed: u32) -> f32 {
    let (x, y) = if period_x > 0 && period_y > 0 { (x.rem_euclid(period_x), y.rem_euclid(period_y)) } else { (x, y) };
    hash01(x, y, 5, seed)
}

/// Smooth value noise over a lattice of `cells_x` × `cells_y` cells across the bounds (wrapping
/// when `tileable`).
fn solid_octave(u: f32, v: f32, cells_x: f32, cells_y: f32, tileable: bool, seed: u32) -> f32 {
    let (x, y) = (u * cells_x, v * cells_y);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (px, py) = if tileable { (cells_x.round().max(1.0) as i32, cells_y.round().max(1.0) as i32) } else { (0, 0) };
    let h = |a: i32, b: i32| lattice(a, b, px, py, seed);
    let top = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * sx;
    let bot = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * sx;
    top + (bot - top) * sy
}

/// Solid Noise: greyscale fractal value noise with `x_size` × `y_size` base cells across the
/// bounds, `detail` extra octaves, seamlessly tileable when asked, and `turbulent` (absolute
/// value) ridges.
pub(crate) fn solid_noise(src: &Image, out: Rect, ctx: &Ctx, x_size: f32, y_size: f32, detail: u32, tileable: bool, turbulent: bool, seed: u32) -> Vec<f32> {
    let b = ctx.bounds;
    let (bw, bh) = (b.width().max(1) as f32, b.height().max(1) as f32);
    let (xs, ys) = (x_size.clamp(0.1, 16.0), y_size.clamp(0.1, 16.0));
    let octaves = detail.clamp(0, 15) + 1;
    map_px(src, out, ctx, |x, y, _| {
        let (u, v) = ((x - b.x0) as f32 / bw, (y - b.y0) as f32 / bh);
        let mut sum = 0.0;
        let mut amp = 1.0;
        let mut norm = 0.0;
        let mut mul = 1.0;
        for o in 0..octaves {
            let n = solid_octave(u, v, xs * mul, ys * mul, tileable, seed.wrapping_add(o.wrapping_mul(7919)));
            sum += if turbulent { (n - 0.5).abs() * 2.0 } else { n } * amp;
            norm += amp;
            amp *= 0.5;
            mul *= 2.0;
        }
        let g = (sum / norm.max(1e-6)).clamp(0.0, 1.0);
        [g, g, g, 1.0]
    })
}
