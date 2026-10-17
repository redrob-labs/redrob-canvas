//! Third filter batch: edge-preserving blurs (Mean Curvature, Tileable, Antialias, Symmetric
//! Nearest Neighbour, NL), distortions (Engrave, Kaleidoscope, Shift, Value Propagate, Video
//! Degradation, Blinds, Page Curl), light and shadow (Supernova, Gradient Flare, Sparkle, Long
//! Shadow, Perspective Shadow, Xach-Effect), noise (LCh, HSV, Hurl, Pick, Slur), edges (Difference
//! of Gaussians, Laplace, Image Gradient, Distance Map, Normal Map), the Operation Graph, artistic
//! renders (Cubism, SLIC, Waterpixels, Impressionist, Van Gogh, Weave) and decorations (Filmstrip,
//! Fuzzy Border).
//!
//! Every kernel here reads the source through straight sRGB RGBA (`fxutil::rgba` /
//! `fxutil::set_rgba`) where colour semantics matter (white, hue, saturation), so CMYK, Lab and
//! greyscale documents behave like RGB ones; per-channel smoothing runs on the native channels.

pub(crate) mod artistic;
pub(crate) mod blur;
pub(crate) mod decor;
pub(crate) mod distort;
pub(crate) mod edge;
pub(crate) mod graph;
pub(crate) mod light;
pub(crate) mod noise;

use redrob_photo_geom::Rect;

use crate::Ctx;
use crate::fxutil::{MAXC, rgba, set_rgba};
use crate::image::{Edge, Image};

/// Straight sRGB RGBA of the source pixel at `(x, y)`, repeating the nearest pixel of the
/// source window outside it.
#[inline]
pub(crate) fn rgba_at(src: &Image, ctx: &Ctx, x: i32, y: i32) -> [f32; 4] {
    let mut buf = [0.0f32; MAXC];
    let n = src.ch.min(MAXC);
    for (c, v) in buf.iter_mut().enumerate().take(n) {
        *v = src.get_edge(x, y, c, Edge::Repeat, src.rect);
    }
    rgba(ctx, &buf[..n])
}

/// Alpha of the source pixel (1 without an alpha channel); 0 outside the window.
#[inline]
pub(crate) fn alpha_at(src: &Image, ctx: &Ctx, x: i32, y: i32) -> f32 {
    if !src.rect.contains(x, y) {
        return 0.0;
    }
    if ctx.alpha { src.get(x, y, src.ch - 1) } else { 1.0 }
}

/// Rec. 601 luma of a straight RGBA colour.
#[inline]
pub(crate) fn luma_rgb(c: [f32; 4]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// Maps every output pixel through `f(x, y, rgba) -> rgba` (document coordinates), converting
/// from and to the document's colour model.
pub(crate) fn map_px(src: &Image, out: Rect, ctx: &Ctx, mut f: impl FnMut(i32, i32, [f32; 4]) -> [f32; 4]) -> Vec<f32> {
    let n = src.ch;
    let mut res = src.crop(out);
    let w = out.width() as usize;
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32);
        let c = rgba(ctx, px);
        let o = f(x, y, c);
        set_rgba(ctx, px, [o[0].clamp(0.0, 1.0), o[1].clamp(0.0, 1.0), o[2].clamp(0.0, 1.0), o[3].clamp(0.0, 1.0)]);
    }
    res
}

/// Premultiplied RGBA window of `src` over `win` (4 floats per pixel, row-major), with the
/// source repeated outside its own window.
pub(crate) fn rgba_window(src: &Image, ctx: &Ctx, win: Rect) -> Vec<f32> {
    let mut v = Vec::with_capacity(win.width() as usize * win.height() as usize * 4);
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            let c = rgba_at(src, ctx, x, y);
            v.extend_from_slice(&[c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]);
        }
    }
    v
}

/// Reads a straight RGBA pixel back out of a premultiplied window buffer (clamped to the window).
#[inline]
pub(crate) fn window_px(buf: &[f32], win: Rect, x: i32, y: i32) -> [f32; 4] {
    let xx = x.clamp(win.x0, win.x1 - 1);
    let yy = y.clamp(win.y0, win.y1 - 1);
    let i = ((yy - win.y0) as usize * win.width() as usize + (xx - win.x0) as usize) * 4;
    let a = buf[i + 3];
    if a > 1e-7 { [buf[i] / a, buf[i + 1] / a, buf[i + 2] / a, a] } else { [0.0, 0.0, 0.0, 0.0] }
}

/// Source-over composite of a straight RGBA colour `top` onto `bottom`.
#[inline]
pub(crate) fn over(top: [f32; 4], bottom: [f32; 4]) -> [f32; 4] {
    let a = top[3] + bottom[3] * (1.0 - top[3]);
    if a <= 1e-7 {
        return [0.0; 4];
    }
    let mix = |t: f32, b: f32| (t * top[3] + b * bottom[3] * (1.0 - top[3])) / a;
    [mix(top[0], bottom[0]), mix(top[1], bottom[1]), mix(top[2], bottom[2]), a]
}

/// Screen (additive-like) light `light` with weight `k` onto an opaque-ish colour.
#[inline]
pub(crate) fn add_light(base: [f32; 4], light: [f32; 3], k: f32) -> [f32; 4] {
    let k = k.clamp(0.0, 1.0);
    let s = |b: f32, l: f32| 1.0 - (1.0 - b) * (1.0 - (l * k).clamp(0.0, 1.0));
    [s(base[0], light[0]), s(base[1], light[1]), s(base[2], light[2]), base[3].max(k * light.iter().fold(0.0f32, |m, v| m.max(*v)))]
}

/// Linear blend of two straight RGBA colours.
#[inline]
pub(crate) fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

/// HSV (all 0..1) of an RGB triple.
pub(crate) fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    crate::other2::rgb_to_hsb(c)
}

/// RGB of an HSV triple (all 0..1).
pub(crate) fn hsv_to_rgb(c: [f32; 3]) -> [f32; 3] {
    crate::other2::hsb_to_rgb([c[0].rem_euclid(1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)])
}

/// Smooth value noise in [0, 1) at `(x, y)`, keyed by `seed` (position based, tile independent).
pub(crate) fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let h = |a: i32, b: i32| crate::noise::hash01(a, b, 0, seed);
    let top = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * sx;
    let bot = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * sx;
    top + (bot - top) * sy
}

/// Four-octave fractal value noise in [0, 1).
pub(crate) fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut f = 1.0;
    let mut norm = 0.0;
    for o in 0..4u32 {
        sum += value_noise(x * f, y * f, seed.wrapping_add(o.wrapping_mul(7919))) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

/// Output of a centred-difference gradient of the luma at `(x, y)`.
#[inline]
pub(crate) fn luma_gradient(src: &Image, ctx: &Ctx, x: i32, y: i32) -> (f32, f32) {
    let l = |dx: i32, dy: i32| luma_rgb(rgba_at(src, ctx, x + dx, y + dy));
    ((l(1, 0) - l(-1, 0)) * 0.5, (l(0, 1) - l(0, -1)) * 0.5)
}

/// Smoothstep from `e0` to `e1` that also accepts a falling edge (`e0 > e1`).
#[inline]
pub(crate) fn sstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e0 <= e1 { crate::fxutil::smoothstep(e0, e1, x) } else { 1.0 - crate::fxutil::smoothstep(e1, e0, x) }
}

/// Reach (in pixels) of a Gaussian of sigma `s`.
#[inline]
pub(crate) fn gauss_reach(s: f32) -> f32 {
    (s.max(0.0) * 3.0).ceil() + 1.0
}
