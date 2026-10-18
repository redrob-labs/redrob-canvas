//! Fourth filter batch (v0.11 parity): Variable Blur, Bloom, Sobel, Cartoon, Clothify, the
//! lens / mosaic / newsprint / curve-bend / warp / panorama distortions, Bump Map and Stencil
//! Chrome, and the Perlin / Plasma / Solid Noise generators.
//!
//! Like [`super::extra`], every kernel reads the source as straight sRGB RGBA where colour
//! semantics matter, so every document colour mode behaves like RGB.

pub(crate) mod distort;
pub(crate) mod noise;
pub(crate) mod surface;

use redrob_photo_geom::Rect;

use super::extra::{add_light, luma_rgb, map_px, rgba_at, rgba_window, sstep, window_px};
use crate::fxutil::gauss_blur_n;
use crate::image::Image;
use crate::noise::hash01;
use crate::{Ctx, MaskSource};

/// Light direction (unit vector, z up) from an azimuth (degrees, counter-clockwise from three
/// o'clock on screen) and an elevation (degrees above the surface).
pub(crate) fn light_dir(azimuth: f32, elevation: f32) -> [f32; 3] {
    let (sa, ca) = azimuth.to_radians().sin_cos();
    let (se, ce) = elevation.clamp(0.0, 90.0).to_radians().sin_cos();
    [ca * ce, -sa * ce, se]
}

/// Value of a greyscale mask at document pixel `(x, y)`: an external map is stretched over the
/// bounds; a layer mask is read in document coordinates (edge pixels repeat outside it); `None`
/// (no map) reads `fallback`.
pub(crate) fn mask_at(map: Option<&Image>, source: MaskSource, b: Rect, x: i32, y: i32, fallback: f32) -> f32 {
    let Some(m) = map.filter(|m| !m.rect.is_empty()) else { return fallback };
    let r = m.rect;
    let (mx, my) = match source {
        MaskSource::Map => {
            let u = (x - b.x0) as f32 / b.width().max(1) as f32 * r.width() as f32;
            let v = (y - b.y0) as f32 / b.height().max(1) as f32 * r.height() as f32;
            (r.x0 + (u.floor() as i32).clamp(0, r.width() as i32 - 1), r.y0 + (v.floor() as i32).clamp(0, r.height() as i32 - 1))
        }
        MaskSource::LayerMask | MaskSource::SelfImage => (x.clamp(r.x0, r.x1 - 1), y.clamp(r.y0, r.y1 - 1)),
    };
    // Grey maps: channel 0; RGB(A) maps: Rec. 601 luma of the colour channels.
    if m.ch >= 3 { 0.299 * m.get(mx, my, 0) + 0.587 * m.get(mx, my, 1) + 0.114 * m.get(mx, my, 2) } else { m.get(mx, my, 0) }
}

/// Variable Blur parameters.
pub(crate) struct VariableBlurSpec<'a> {
    pub radius: f32,
    pub linear_mask: bool,
    pub high_quality: bool,
    pub invert: bool,
    pub source: MaskSource,
    pub map: Option<&'a Image>,
}

/// Pixels Variable Blur reads around an output pixel.
pub(crate) fn variable_blur_reach(radius: f32) -> f32 {
    radius.clamp(0.0, 1000.0) + 2.0
}

/// Variable Blur: a Gaussian blur whose radius at every pixel is `radius` times the mask value
/// there (an external map, the layer mask, or the layer's own alpha). The blur is built from a
/// ladder of uniformly blurred copies of the window and interpolated per pixel between the two
/// rungs nearest the local radius; `high_quality` doubles the rungs.
pub(crate) fn variable_blur(src: &Image, out: Rect, ctx: &Ctx, spec: &VariableBlurSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let radius = spec.radius.clamp(0.0, 1000.0);
    if radius < 0.05 {
        return src.crop(out);
    }
    let reach = variable_blur_reach(radius).ceil() as i32;
    let win = out.inflate(reach);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let base = rgba_window(src, ctx, win);
    let rungs = if spec.high_quality { 12 } else { 6 };
    // Rung k blurs with sigma_k = radius / 3 · k / rungs (radius = 3 sigma).
    let mut ladder: Vec<Vec<f32>> = Vec::with_capacity(rungs + 1);
    ladder.push(base.clone());
    for k in 1..=rungs {
        let mut buf = base.clone();
        let sigma = radius / 3.0 * k as f32 / rungs as f32;
        if sigma >= 0.3 {
            gauss_blur_n(&mut buf, ww, wh, 4, sigma);
        }
        ladder.push(buf);
    }
    map_px(src, out, ctx, |x, y, me| {
        let mut m = mask_at(spec.map, spec.source, b, x, y, if spec.source == MaskSource::SelfImage { me[3] } else { 1.0 });
        if spec.invert {
            m = 1.0 - m;
        }
        let m = m.clamp(0.0, 1.0);
        // A perceptual mask curves the radius so mid-greys blur less than linearly.
        let m = if spec.linear_mask { m } else { m * m };
        let level = m * rungs as f32;
        let k0 = (level.floor() as usize).min(rungs);
        let k1 = (k0 + 1).min(rungs);
        let t = level - k0 as f32;
        let a = window_px(&ladder[k0], win, x, y);
        let bb = window_px(&ladder[k1], win, x, y);
        super::extra::lerp4(a, bb, t.clamp(0.0, 1.0))
    })
}

/// Bloom parameters.
pub(crate) struct BloomSpec {
    pub threshold: f32,
    pub softness: f32,
    pub radius: f32,
    pub strength: f32,
    pub limit_exposure: bool,
}

/// Pixels Bloom reads around an output pixel.
pub(crate) fn bloom_reach(radius: f32) -> f32 {
    radius.clamp(0.0, 1500.0) / 2.0 * 3.0 + 2.0
}

/// Bloom: the parts of the image brighter than `threshold` % (with a `softness` % knee) are
/// blurred by `radius` and added back at `strength` %. `limit_exposure` screens the glow in
/// instead of adding it, so highlights cannot clip.
pub(crate) fn bloom(src: &Image, out: Rect, ctx: &Ctx, spec: &BloomSpec) -> Vec<f32> {
    let reach = bloom_reach(spec.radius).ceil() as i32;
    let win = out.inflate(reach);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let t = (spec.threshold / 100.0).clamp(0.0, 1.0);
    let soft = (spec.softness / 100.0).clamp(0.0, 1.0) * 0.5 + 0.001;
    let mut glow = Vec::with_capacity(ww * wh * 3);
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            let c = rgba_at(src, ctx, x, y);
            let l = luma_rgb(c) * c[3];
            let k = sstep(t - soft, t + soft, l);
            glow.extend_from_slice(&[c[0] * k * c[3], c[1] * k * c[3], c[2] * k * c[3]]);
        }
    }
    let sigma = spec.radius.clamp(0.0, 1500.0) / 2.0;
    if sigma >= 0.3 {
        gauss_blur_n(&mut glow, ww, wh, 3, sigma);
    }
    let strength = (spec.strength / 100.0).clamp(0.0, 10.0);
    map_px(src, out, ctx, |x, y, me| {
        let i = ((y - win.y0) as usize * ww + (x - win.x0) as usize) * 3;
        let g = [glow[i] * strength, glow[i + 1] * strength, glow[i + 2] * strength];
        if spec.limit_exposure {
            add_light(me, g, 1.0)
        } else {
            [me[0] + g[0], me[1] + g[1], me[2] + g[2], me[3].max((g[0].max(g[1]).max(g[2])).min(1.0))]
        }
    })
}

/// Sobel edge detection on each colour channel: `horizontal` / `vertical` pick the gradient
/// directions, `keep_sign` keeps the signed gradient around mid-grey instead of its magnitude.
pub(crate) fn sobel(src: &Image, out: Rect, ctx: &Ctx, horizontal: bool, vertical: bool, keep_sign: bool) -> Vec<f32> {
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    let sub = crate::fxutil::subtractive(ctx);
    let mut res = src.crop(out);
    let w = out.width() as usize;
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32);
        for (c, v) in px.iter_mut().enumerate().take(cc) {
            let g = |dx: i32, dy: i32| src.get_edge(x + dx, y + dy, c, crate::image::Edge::Repeat, src.rect);
            let gx = if horizontal { (g(1, -1) + 2.0 * g(1, 0) + g(1, 1)) - (g(-1, -1) + 2.0 * g(-1, 0) + g(-1, 1)) } else { 0.0 };
            let gy = if vertical { (g(-1, 1) + 2.0 * g(0, 1) + g(1, 1)) - (g(-1, -1) + 2.0 * g(0, -1) + g(1, -1)) } else { 0.0 };
            let e = if keep_sign {
                let s = match (horizontal, vertical) {
                    (true, true) => (gx + gy) * 0.5,
                    (true, false) => gx,
                    (false, true) => gy,
                    (false, false) => 0.0,
                };
                (0.5 + s / 8.0).clamp(0.0, 1.0)
            } else {
                ((gx * gx + gy * gy).sqrt() / 4.0).clamp(0.0, 1.0)
            };
            *v = if sub && !keep_sign { 1.0 - e } else { e };
        }
    }
    res
}

/// Pixels Cartoon reads around an output pixel.
pub(crate) fn cartoon_reach(mask_radius: f32) -> f32 {
    mask_radius.clamp(1.0, 50.0) * 1.5 + 2.0
}

/// Cartoon: pixels noticeably darker than their `mask_radius` neighbourhood become ink lines.
/// `pct_black` (0..1) is how much of the image may turn black: a higher value lowers the
/// darkness a pixel needs to be inked.
pub(crate) fn cartoon(src: &Image, out: Rect, ctx: &Ctx, mask_radius: f32, pct_black: f32) -> Vec<f32> {
    let reach = cartoon_reach(mask_radius).ceil() as i32;
    let win = out.inflate(reach);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let mut lum: Vec<f32> = Vec::with_capacity(ww * wh);
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            let c = rgba_at(src, ctx, x, y);
            lum.push(luma_rgb(c));
        }
    }
    let mut blurred = lum.clone();
    let sigma = mask_radius.clamp(1.0, 50.0) / 2.0;
    gauss_blur_n(&mut blurred, ww, wh, 1, sigma);
    let pct = pct_black.clamp(0.0, 1.0);
    // The knee: more black means a smaller darkness step is already ink.
    let lo = 0.12 * (1.0 - pct) + 0.01;
    let hi = lo + 0.06 + 0.1 * (1.0 - pct);
    map_px(src, out, ctx, |x, y, me| {
        let i = (y - win.y0) as usize * ww + (x - win.x0) as usize;
        let diff = blurred[i] - lum[i];
        // Relative darkness: a dark step in a dark region counts as much as in a light one.
        let rel = diff / blurred[i].max(0.08);
        let ink = sstep(lo, hi, rel.max(diff));
        [me[0] * (1.0 - ink), me[1] * (1.0 - ink), me[2] * (1.0 - ink), me[3]]
    })
}

/// Clothify parameters.
pub(crate) struct ClothifySpec {
    pub blur_x: f32,
    pub blur_y: f32,
    pub azimuth: f32,
    pub elevation: f32,
    pub depth: f32,
    pub seed: u32,
}

/// Pixels Clothify reads around an output pixel.
pub(crate) fn clothify_reach(blur_x: f32, blur_y: f32) -> f32 {
    blur_x.max(blur_y).clamp(0.0, 100.0) * 1.5 + 3.0
}

/// Separable box blur of a one-channel buffer with different radii per axis.
fn blur_xy(buf: &mut [f32], w: usize, h: usize, rx: usize, ry: usize) {
    let mut tmp = buf.to_vec();
    if rx > 0 {
        let k = (2 * rx + 1) as f32;
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for d in -(rx as i32)..=rx as i32 {
                    let xx = (x as i32 + d).clamp(0, w as i32 - 1) as usize;
                    acc += buf[y * w + xx];
                }
                tmp[y * w + x] = acc / k;
            }
        }
        buf.copy_from_slice(&tmp);
    }
    if ry > 0 {
        let k = (2 * ry + 1) as f32;
        for x in 0..w {
            for y in 0..h {
                let mut acc = 0.0;
                for d in -(ry as i32)..=ry as i32 {
                    let yy = (y as i32 + d).clamp(0, h as i32 - 1) as usize;
                    acc += buf[yy * w + x];
                }
                tmp[y * w + x] = acc / k;
            }
        }
        buf.copy_from_slice(&tmp);
    }
}

/// Clothify: white noise blurred by `blur_x` × `blur_y` into a thread-like texture that bump-maps
/// the image, lit from `azimuth` / `elevation` with relief `depth`.
pub(crate) fn clothify(src: &Image, out: Rect, ctx: &Ctx, spec: &ClothifySpec) -> Vec<f32> {
    let reach = clothify_reach(spec.blur_x, spec.blur_y).ceil() as i32;
    let win = out.inflate(reach);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let mut height: Vec<f32> = (0..ww * wh).map(|i| hash01(win.x0 + (i % ww) as i32, win.y0 + (i / ww) as i32, 71, spec.seed)).collect();
    blur_xy(&mut height, ww, wh, (spec.blur_x.clamp(0.0, 100.0) / 2.0).round() as usize, (spec.blur_y.clamp(0.0, 100.0) / 2.0).round() as usize);
    // Stretch the blurred noise back to full contrast.
    let (lo, hi) = height.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let span = (hi - lo).max(1e-4);
    for v in &mut height {
        *v = (*v - lo) / span;
    }
    let light = light_dir(spec.azimuth, spec.elevation);
    let depth = spec.depth.clamp(0.0, 50.0);
    let flat = light[2].max(0.05);
    map_px(src, out, ctx, |x, y, me| {
        let (ix, iy) = ((x - win.x0) as usize, (y - win.y0) as usize);
        let h = |dx: i32, dy: i32| height[(iy as i32 + dy).clamp(0, wh as i32 - 1) as usize * ww + (ix as i32 + dx).clamp(0, ww as i32 - 1) as usize];
        let nx = (h(-1, 0) - h(1, 0)) * depth;
        let ny = (h(0, -1) - h(0, 1)) * depth;
        let len = (nx * nx + ny * ny + 1.0).sqrt();
        let shade = ((nx * light[0] + ny * light[1] + light[2]) / len).max(0.0) / flat;
        [(me[0] * shade).clamp(0.0, 1.0), (me[1] * shade).clamp(0.0, 1.0), (me[2] * shade).clamp(0.0, 1.0), me[3]]
    })
}
