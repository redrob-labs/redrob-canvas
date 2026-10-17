//! Noise of the third batch: CIE lch Noise, HSV Noise, Hurl, Pick, Slur.

use redrob_photo_color::convert::{lab_to_srgb, srgb_to_lab};
use redrob_photo_geom::Rect;

use super::{hsv_to_rgb, map_px, rgb_to_hsv, rgba_window, window_px};
use crate::Ctx;
use crate::fxutil::set_rgba;
use crate::image::Image;
use crate::noise::hash01;

/// Mean of `dulling` uniform deviates in [-1, 1): higher dulling clusters the noise near zero.
#[inline]
fn dulled(x: i32, y: i32, channel: u32, seed: u32, dulling: u32) -> f32 {
    let d = dulling.clamp(1, 8);
    let mut sum = 0.0;
    for k in 0..d {
        sum += hash01(x, y, channel * 16 + k, seed) * 2.0 - 1.0;
    }
    sum / d as f32
}

/// CIE lch Noise: random offsets of lightness (0..100), chroma (0..100) and hue (degrees) in
/// LCh(ab), averaged over `dulling` deviates.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lch_noise(src: &Image, out: Rect, ctx: &Ctx, dulling: u32, lightness: f32, chroma: f32, hue: f32, seed: u32) -> Vec<f32> {
    map_px(src, out, ctx, |x, y, c| {
        let lab = srgb_to_lab([c[0], c[1], c[2]]);
        let (l, a, b) = (lab[0], lab[1], lab[2]);
        let (ch, h) = ((a * a + b * b).sqrt(), b.atan2(a));
        let l2 = (l + dulled(x, y, 0, seed, dulling) * lightness).clamp(0.0, 100.0);
        let c2 = (ch + dulled(x, y, 1, seed, dulling) * chroma).max(0.0);
        let h2 = h + dulled(x, y, 2, seed, dulling) * hue.to_radians();
        let rgb = lab_to_srgb([l2, c2 * h2.cos(), c2 * h2.sin()]);
        [rgb[0], rgb[1], rgb[2], c[3]]
    })
}

/// HSV Noise: random offsets of hue (degrees), saturation and value (0..1), averaged over
/// `dulling` deviates.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hsv_noise(src: &Image, out: Rect, ctx: &Ctx, dulling: u32, hue: f32, saturation: f32, value: f32, seed: u32) -> Vec<f32> {
    map_px(src, out, ctx, |x, y, c| {
        let hsv = rgb_to_hsv([c[0], c[1], c[2]]);
        let h = hsv[0] + dulled(x, y, 0, seed, dulling) * hue / 360.0;
        let s = (hsv[1] + dulled(x, y, 1, seed, dulling) * saturation).clamp(0.0, 1.0);
        let v = (hsv[2] + dulled(x, y, 2, seed, dulling) * value).clamp(0.0, 1.0);
        let rgb = hsv_to_rgb([h, s, v]);
        [rgb[0], rgb[1], rgb[2], c[3]]
    })
}

/// Hurl: `randomization` percent of the pixels take a random colour, `repeat` times over.
pub(crate) fn hurl(src: &Image, out: Rect, ctx: &Ctx, randomization: f32, repeat: u32, seed: u32) -> Vec<f32> {
    let p = (randomization / 100.0).clamp(0.0, 1.0);
    let reps = repeat.clamp(1, 100);
    map_px(src, out, ctx, |x, y, c| {
        let mut o = c;
        for r in 0..reps {
            if hash01(x, y, r * 4, seed) < p {
                o = [hash01(x, y, r * 4 + 1, seed), hash01(x, y, r * 4 + 2, seed), hash01(x, y, r * 4 + 3, seed), c[3]];
            }
        }
        o
    })
}

/// Shared driver of Pick and Slur: `repeat` passes over a window large enough that the output
/// never depends on the tiling, each pass replacing `randomization` percent of the pixels by a
/// neighbour chosen by `pick(x, y, pass) -> (dx, dy)`.
fn neighbour_passes(src: &Image, out: Rect, ctx: &Ctx, randomization: f32, repeat: u32, seed: u32, pick: impl Fn(i32, i32, u32) -> (i32, i32)) -> Vec<f32> {
    let p = (randomization / 100.0).clamp(0.0, 1.0);
    let reps = repeat.clamp(1, 100);
    let win = out.inflate(reps as i32);
    let mut cur = rgba_window(src, ctx, win);
    let mut next = cur.clone();
    let ww = win.width() as usize;
    for r in 0..reps {
        for y in win.y0..win.y1 {
            for x in win.x0..win.x1 {
                let i = ((y - win.y0) as usize * ww + (x - win.x0) as usize) * 4;
                if hash01(x, y, r * 2, seed) < p {
                    let (dx, dy) = pick(x, y, r);
                    let (sx, sy) = ((x + dx).clamp(win.x0, win.x1 - 1), (y + dy).clamp(win.y0, win.y1 - 1));
                    let j = ((sy - win.y0) as usize * ww + (sx - win.x0) as usize) * 4;
                    next[i..i + 4].copy_from_slice(&cur[j..j + 4]);
                } else {
                    next[i..i + 4].copy_from_slice(&cur[i..i + 4]);
                }
            }
        }
        std::mem::swap(&mut cur, &mut next);
    }
    let n = src.ch;
    let mut res = src.crop(out);
    let w = out.width() as usize;
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32);
        set_rgba(ctx, px, window_px(&cur, win, x, y));
    }
    res
}

/// Pick: `randomization` percent of the pixels take a random neighbour's colour (any of the eight,
/// or themselves), `repeat` times.
pub(crate) fn pick(src: &Image, out: Rect, ctx: &Ctx, randomization: f32, repeat: u32, seed: u32) -> Vec<f32> {
    neighbour_passes(src, out, ctx, randomization, repeat, seed, |x, y, r| {
        let k = (hash01(x, y, r * 2 + 1, seed) * 9.0) as i32;
        (k % 3 - 1, k / 3 - 1)
    })
}

/// Slur: `randomization` percent of the pixels melt downward, taking the pixel above (80 %) or
/// one of the pixels above-left and above-right (10 % each), `repeat` times.
pub(crate) fn slur(src: &Image, out: Rect, ctx: &Ctx, randomization: f32, repeat: u32, seed: u32) -> Vec<f32> {
    neighbour_passes(src, out, ctx, randomization, repeat, seed, |x, y, r| {
        let k = hash01(x, y, r * 2 + 1, seed);
        if k < 0.1 {
            (-1, -1)
        } else if k < 0.2 {
            (1, -1)
        } else {
            (0, -1)
        }
    })
}
