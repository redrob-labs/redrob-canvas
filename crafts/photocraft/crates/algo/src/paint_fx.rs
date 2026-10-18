//! Fifth filter batch (v0.11 Paint-mode parity): Dodge / Burn filters, Index Colors, the
//! polygon Blur, Edge Detection, the Emboss family, Phong Bump Map, Palettize, Cross-Channel
//! Adjustment, Slope / Offset / Power, Raindrops, Maximize / Minimize Channel, Propagate
//! Colors, Height to Normal Map, Mean Removal, Wavelet Noise Reducer, Normalize, Reset
//! Transparent and Random Pick.
//!
//! Like [`super::extra`], every kernel reads the source as straight sRGB RGBA where colour
//! semantics matter, so every document colour mode behaves like RGB.

use redrob_photo_color::convert::srgb_to_lab;
use redrob_photo_geom::Rect;

use crate::extra::{luma_rgb, map_px, rgba_at, rgba_window, window_px};
use crate::image::{Edge, Image};
use crate::noise::hash01;
use crate::other2::{hsl_to_rgb, rgb_to_hsl};
use crate::quantize::bayer8;
use crate::{
    CrossChannel, CrossCurve, Ctx, EdgeMethod, EdgeOutput, EmbossDirection, HeightChannel, IndexDither, NormalSwizzle, PaletteAlphaMode, PaletteColorMode,
    PaletteDither, PaletteSpace, PhongLight, PolyShape, ToneRange,
};

// ---- helpers ----

/// Value of the height channel of the source pixel at `(x, y)` (edge pixels repeat).
#[inline]
fn height_at(src: &Image, ctx: &Ctx, x: i32, y: i32, channel: HeightChannel) -> f32 {
    let c = rgba_at(src, ctx, x, y);
    match channel {
        HeightChannel::Red => c[0],
        HeightChannel::Green => c[1],
        HeightChannel::Blue => c[2],
        HeightChannel::Alpha => c[3],
        HeightChannel::Luminance => luma_rgb(c),
    }
}

/// Gradient `(d/dx, d/dy)` of the height channel at `(x, y)` with a derivative operator of
/// horizontal radius `rx` and vertical radius `ry`: the difference between the samples on each
/// side (weighted by distance for Sobel-style cross smoothing, uniformly for Prewitt, and on the
/// centre row / column only for Simple), normalized so a unit step reads as 1.
#[allow(clippy::too_many_arguments)]
pub(crate) fn gradient(src: &Image, ctx: &Ctx, x: i32, y: i32, method: EdgeMethod, rx: i32, ry: i32, channel: HeightChannel) -> (f32, f32) {
    let rx = rx.clamp(1, 100);
    let ry = ry.clamp(1, 100);
    let weight = |d: i32, r: i32| -> f32 {
        match method {
            EdgeMethod::Prewitt => 1.0,
            EdgeMethod::Sobel => (r + 1 - d.abs()) as f32,
            EdgeMethod::Simple => {
                if d == 0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    };
    let (mut gx, mut gy) = (0.0f32, 0.0f32);
    let (mut nx, mut ny) = (0.0f32, 0.0f32);
    // d/dx: smooth across y (radius ry), differentiate across x (radius rx).
    for dy in -ry..=ry {
        let w = weight(dy, ry);
        if w == 0.0 {
            continue;
        }
        nx += w;
        for dx in 1..=rx {
            gx += w * (height_at(src, ctx, x + dx, y + dy, channel) - height_at(src, ctx, x - dx, y + dy, channel));
        }
    }
    for dx in -rx..=rx {
        let w = weight(dx, rx);
        if w == 0.0 {
            continue;
        }
        ny += w;
        for dy in 1..=ry {
            gy += w * (height_at(src, ctx, x + dx, y + dy, channel) - height_at(src, ctx, x + dx, y - dy, channel));
        }
    }
    (gx / (nx * rx as f32).max(1e-6), gy / (ny * ry as f32).max(1e-6))
}

#[inline]
fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n > 1e-9 { [v[0] / n, v[1] / n, v[2] / n] } else { [0.0, 0.0, 1.0] }
}

/// Unit light direction (z towards the viewer) of an azimuth and an inclination in degrees.
#[inline]
fn light_dir(azimuth: f32, inclination: f32) -> [f32; 3] {
    let (sa, ca) = azimuth.to_radians().sin_cos();
    let (si, ci) = inclination.clamp(0.0, 90.0).to_radians().sin_cos();
    [ca * ci, -sa * ci, si]
}

/// Piecewise-linear curve through `points` (`[x, y]`, sorted by x here) evaluated at `t`.
fn curve_at(points: &[[f32; 2]], t: f32) -> f32 {
    if points.is_empty() {
        return 0.5;
    }
    let mut pts: Vec<[f32; 2]> = points.to_vec();
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
    if t <= pts[0][0] {
        return pts[0][1];
    }
    for w in pts.windows(2) {
        if t <= w[1][0] {
            let span = (w[1][0] - w[0][0]).max(1e-6);
            let k = ((t - w[0][0]) / span).clamp(0.0, 1.0);
            return w[0][1] + (w[1][1] - w[0][1]) * k;
        }
    }
    pts[pts.len() - 1][1]
}

// ---- Dodge / Burn ----

/// Dodge (lighten) or Burn (darken) the chosen tone range of every channel by `exposure` (0..1):
/// shadows are lifted or crushed in proportion to their darkness, midtones through a gamma curve,
/// highlights by scaling.
pub(crate) fn dodge_burn(src: &Image, out: Rect, ctx: &Ctx, burn: bool, range: ToneRange, exposure: f32) -> Vec<f32> {
    let e = exposure.clamp(0.0, 1.0);
    let f = |v: f32| -> f32 {
        let v = v.clamp(0.0, 1.0);
        match (range, burn) {
            (ToneRange::Shadows, false) => v + e * (1.0 - v) * (1.0 - v) * 0.5,
            (ToneRange::Shadows, true) => v * (1.0 - e * (1.0 - v) * 0.75),
            (ToneRange::Midtones, false) => v.powf(1.0 / (1.0 + e)),
            (ToneRange::Midtones, true) => v.powf(1.0 + e),
            (ToneRange::Highlights, false) => v * (1.0 + e * 0.5),
            (ToneRange::Highlights, true) => v * (1.0 - e * 0.5 * v),
        }
    };
    map_px(src, out, ctx, |_, _, c| [f(c[0]), f(c[1]), f(c[2]), c[3]])
}

// ---- Index Colors ----

/// Index Colors parameters.
pub(crate) struct IndexColorsSpec<'a> {
    /// Base colours of each ramp (dark to light), straight sRGB.
    pub ramps: &'a [Vec<[f32; 3]>],
    /// Colours interpolated between neighbouring base colours of a ramp.
    pub in_between: u32,
    /// Ramps interpolated between neighbouring ramps.
    pub between_ramps: u32,
    pub dither: IndexDither,
    /// Dither amplitude 0..1.
    pub amount: f32,
    /// Alpha levels (256 = untouched).
    pub alpha_steps: u32,
    /// How much a lightness difference counts against a chroma difference, 0..1.
    pub luma_weight: f32,
}

/// The ramp `ramp` expanded with `k` interpolated colours between neighbours.
fn expand_ramp(ramp: &[[f32; 3]], k: u32) -> Vec<[f32; 3]> {
    let mut v = Vec::new();
    for (i, c) in ramp.iter().enumerate() {
        v.push(*c);
        if let Some(n) = ramp.get(i + 1) {
            for j in 1..=k {
                let t = j as f32 / (k + 1) as f32;
                v.push([c[0] + (n[0] - c[0]) * t, c[1] + (n[1] - c[1]) * t, c[2] + (n[2] - c[2]) * t]);
            }
        }
    }
    v
}

/// The palette of an Index Colors spec: every ramp expanded, plus the interpolated ramps.
pub(crate) fn index_palette(spec: &IndexColorsSpec) -> Vec<[f32; 3]> {
    let ramps: Vec<Vec<[f32; 3]>> = spec.ramps.iter().filter(|r| !r.is_empty()).map(|r| expand_ramp(r, spec.in_between.min(64))).collect();
    let mut pal: Vec<[f32; 3]> = Vec::new();
    for (i, r) in ramps.iter().enumerate() {
        pal.extend_from_slice(r);
        if let Some(n) = ramps.get(i + 1) {
            for j in 1..=spec.between_ramps.min(16) {
                let t = j as f32 / (spec.between_ramps.min(16) + 1) as f32;
                let len = r.len().max(n.len());
                for k in 0..len {
                    let a = r[(k * r.len() / len).min(r.len() - 1)];
                    let b = n[(k * n.len() / len).min(n.len() - 1)];
                    pal.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]);
                }
            }
        }
    }
    if pal.is_empty() {
        pal.push([0.0, 0.0, 0.0]);
        pal.push([1.0, 1.0, 1.0]);
    }
    pal.truncate(4096);
    pal
}

/// Nearest palette entry by a lightness-weighted RGB distance.
fn nearest_weighted(pal: &[[f32; 3]], c: [f32; 3], luma_weight: f32) -> usize {
    let lc = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let mut best = (f32::MAX, 0usize);
    for (i, p) in pal.iter().enumerate() {
        let lp = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
        let dc = ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)) / 3.0;
        let d = luma_weight * (lp - lc).powi(2) + (1.0 - luma_weight) * dc;
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

/// Index Colors: every pixel snaps to the nearest colour of the ramp palette, with an ordered or
/// noise dither of `amount`, and alpha is quantized to `alpha_steps` levels.
pub(crate) fn index_colors(src: &Image, out: Rect, ctx: &Ctx, spec: &IndexColorsSpec) -> Vec<f32> {
    let pal = index_palette(spec);
    let spread = (1.0 / (pal.len() as f32).cbrt().max(1.0)).min(0.5) * spec.amount.clamp(0.0, 1.0);
    let lw = spec.luma_weight.clamp(0.0, 1.0);
    let steps = spec.alpha_steps.clamp(1, 256);
    map_px(src, out, ctx, |x, y, c| {
        let t = match spec.dither {
            IndexDither::None => 0.0,
            IndexDither::Ordered => bayer8(x.rem_euclid(8) as usize, y.rem_euclid(8) as usize) * spread,
            IndexDither::Noise => (hash01(x, y, 11, 0) - 0.5) * spread,
        };
        let q = [(c[0] + t).clamp(0.0, 1.0), (c[1] + t).clamp(0.0, 1.0), (c[2] + t).clamp(0.0, 1.0)];
        let p = pal[nearest_weighted(&pal, q, lw)];
        let a = if steps >= 256 {
            c[3]
        } else if steps == 1 {
            if c[3] > 0.0 { 1.0 } else { 0.0 }
        } else {
            let n = (steps - 1) as f32;
            ((c[3] + t).clamp(0.0, 1.0) * n).round() / n
        };
        [p[0], p[1], p[2], a]
    })
}

// ---- Polygon blur ----

/// Pixels the polygon blur reads around an output pixel.
pub(crate) fn poly_blur_reach(width: f32, height: f32) -> f32 {
    width.max(height).clamp(0.0, 1000.0).ceil() + 1.0
}

/// Whether the unit-square point `(u, v)` (−1..1) lies inside the kernel outline.
fn in_shape(shape: PolyShape, u: f32, v: f32) -> bool {
    let r = (u * u + v * v).sqrt();
    if r > 1.0 {
        return false;
    }
    let n = match shape {
        PolyShape::Circle => return true,
        PolyShape::Triangle => 3.0,
        PolyShape::Quadrilateral => 4.0,
        PolyShape::Pentagon => 5.0,
        PolyShape::Hexagon => 6.0,
        PolyShape::Octagon => 8.0,
    };
    // Regular n-gon of circumradius 1 with a vertex at twelve o'clock.
    let theta = v.atan2(u) + std::f32::consts::FRAC_PI_2;
    let sector = std::f32::consts::TAU / n;
    let local = theta.rem_euclid(sector) - sector / 2.0;
    r * local.cos() <= (sector / 2.0).cos()
}

/// Blur: the mean of the pixels under a `2·width × 2·height` kernel of the given outline, rotated
/// by `angle` degrees, mixed with the source by `strength` (0..1).
#[allow(clippy::too_many_arguments)]
pub(crate) fn poly_blur(src: &Image, out: Rect, ctx: &Ctx, width: f32, height: f32, strength: f32, angle: f32, shape: PolyShape) -> Vec<f32> {
    let (w, h) = (width.clamp(0.0, 1000.0), height.clamp(0.0, 1000.0));
    let k = strength.clamp(0.0, 1.0);
    if (w < 0.5 && h < 0.5) || k <= 0.0 {
        return src.crop(out);
    }
    let reach = poly_blur_reach(w, h) as i32;
    let win = out.inflate(reach);
    let buf = rgba_window(src, ctx, win);
    let (sa, ca) = (-angle).to_radians().sin_cos();
    // Kernel offsets (relative), computed once.
    let mut taps: Vec<(i32, i32)> = Vec::new();
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let (fx, fy) = (dx as f32, dy as f32);
            let (rx, ry) = (fx * ca - fy * sa, fx * sa + fy * ca);
            let (u, v) = (rx / w.max(0.5), ry / h.max(0.5));
            if in_shape(shape, u, v) {
                taps.push((dx, dy));
            }
        }
    }
    if taps.is_empty() {
        taps.push((0, 0));
    }
    let inv = 1.0 / taps.len() as f32;
    let n = src.ch;
    let mut res = src.crop(out);
    let ow = out.width() as usize;
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % ow) as i32, out.y0 + (i / ow) as i32);
        let mut acc = [0.0f32; 4];
        for &(dx, dy) in &taps {
            let xx = (x + dx).clamp(win.x0, win.x1 - 1);
            let yy = (y + dy).clamp(win.y0, win.y1 - 1);
            let j = ((yy - win.y0) as usize * win.width() as usize + (xx - win.x0) as usize) * 4;
            for c in 0..4 {
                acc[c] += buf[j + c];
            }
        }
        let a = acc[3] * inv;
        let blurred = if a > 1e-7 { [acc[0] * inv / a, acc[1] * inv / a, acc[2] * inv / a, a] } else { [0.0, 0.0, 0.0, 0.0] };
        let orig = crate::fxutil::rgba(ctx, px);
        let mixed = [
            orig[0] + (blurred[0] - orig[0]) * k,
            orig[1] + (blurred[1] - orig[1]) * k,
            orig[2] + (blurred[2] - orig[2]) * k,
            orig[3] + (blurred[3] - orig[3]) * k,
        ];
        crate::fxutil::set_rgba(ctx, px, mixed.map(|v| v.clamp(0.0, 1.0)));
    }
    res
}

// ---- Edge Detection ----

/// Edge Detection: the luma gradient by `method` over `rx` × `ry`, written as grey (or into
/// alpha when `to_alpha`, keeping the colours).
#[allow(clippy::too_many_arguments)]
pub(crate) fn edge_detection(src: &Image, out: Rect, ctx: &Ctx, method: EdgeMethod, rx: i32, ry: i32, output: EdgeOutput, to_alpha: bool) -> Vec<f32> {
    map_px(src, out, ctx, |x, y, c| {
        let (gx, gy) = gradient(src, ctx, x, y, method, rx, ry, HeightChannel::Luminance);
        let v = match output {
            EdgeOutput::Pythagorean => (gx * gx + gy * gy).sqrt(),
            EdgeOutput::XOnly => 0.5 + gx * 0.5,
            EdgeOutput::YOnly => 0.5 + gy * 0.5,
            EdgeOutput::Radians => gy.atan2(gx) / std::f32::consts::TAU + 0.5,
        }
        .clamp(0.0, 1.0);
        if to_alpha { [c[0], c[1], c[2], c[3] * v] } else { [v, v, v, c[3]] }
    })
}

// ---- Emboss family ----

/// Pixels the Emboss family reads around an output pixel.
pub(crate) fn emboss_reach(direction: EmbossDirection, depth: f32) -> f32 {
    match direction {
        EmbossDirection::VariableDepth => depth.clamp(1.0, 10.0).ceil() + 1.0,
        _ => 2.0,
    }
}

/// Emboss family: grey relief from the luma slopes in the chosen directions (`depth` scales the
/// relief); the Laplacian variant uses the second derivative, Variable Depth lights the slopes
/// from the upper left with a derivative radius of `depth`.
pub(crate) fn emboss_family(src: &Image, out: Rect, ctx: &Ctx, direction: EmbossDirection, depth: f32) -> Vec<f32> {
    let d = depth.clamp(1.0, 10.0);
    let lum = |x: i32, y: i32| luma_rgb(rgba_at(src, ctx, x, y));
    map_px(src, out, ctx, |x, y, c| {
        let v = match direction {
            EmbossDirection::Horizontal => 0.5 + d * (lum(x + 1, y) - lum(x - 1, y)) * 0.5,
            EmbossDirection::Vertical => 0.5 + d * (lum(x, y + 1) - lum(x, y - 1)) * 0.5,
            EmbossDirection::Both => 0.5 + d * (lum(x + 1, y) - lum(x - 1, y) + lum(x, y + 1) - lum(x, y - 1)) * 0.25,
            EmbossDirection::AllDirections => {
                let mut s = 0.0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if dx != 0 || dy != 0 {
                            s += lum(x + dx, y + dy);
                        }
                    }
                }
                0.5 + d * (8.0 * lum(x, y) - s) * 0.125
            }
            EmbossDirection::Laplacian => 0.5 + d * (4.0 * lum(x, y) - lum(x + 1, y) - lum(x - 1, y) - lum(x, y + 1) - lum(x, y - 1)) * 0.25,
            EmbossDirection::VariableDepth => {
                let r = d.ceil() as i32;
                let (gx, gy) = gradient(src, ctx, x, y, EdgeMethod::Sobel, r, r, HeightChannel::Luminance);
                let n = normalize3([-gx * d, -gy * d, 1.0]);
                let l = light_dir(135.0, 45.0);
                (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).clamp(0.0, 1.0)
            }
        }
        .clamp(0.0, 1.0);
        [v, v, v, c[3]]
    })
}

// ---- Phong Bump Map ----

/// Phong Bump Map parameters.
pub(crate) struct PhongSpec {
    pub lights: [PhongLight; 4],
    pub ambient: f32,
    pub diffuse: f32,
    pub specular: f32,
    pub shininess: f32,
    pub use_normal_map: bool,
    pub height_channel: HeightChannel,
}

/// Phong Bump Map: the height (or normal) map lit by up to four directional lights with ambient,
/// diffuse and specular (Phong, exponent `shininess`) reflection; the result replaces the colours,
/// alpha is kept.
pub(crate) fn phong_bumpmap(src: &Image, out: Rect, ctx: &Ctx, spec: &PhongSpec) -> Vec<f32> {
    let lights: Vec<([f32; 3], [f32; 3])> = spec.lights.iter().filter(|l| l.enabled).map(|l| (light_dir(l.azimuth, l.inclination), l.color)).collect();
    let ambient_color = if lights.is_empty() {
        [1.0, 1.0, 1.0]
    } else {
        let mut s = [0.0f32; 3];
        for (_, c) in &lights {
            for k in 0..3 {
                s[k] += c[k];
            }
        }
        s.map(|v| v / lights.len() as f32)
    };
    let shin = spec.shininess.clamp(1.0, 200.0);
    map_px(src, out, ctx, |x, y, c| {
        let n = if spec.use_normal_map {
            normalize3([c[0] * 2.0 - 1.0, c[1] * 2.0 - 1.0, c[2] * 2.0 - 1.0])
        } else {
            let (gx, gy) = gradient(src, ctx, x, y, EdgeMethod::Sobel, 1, 1, spec.height_channel);
            normalize3([-gx * 4.0, -gy * 4.0, 1.0])
        };
        let mut o = [spec.ambient * ambient_color[0], spec.ambient * ambient_color[1], spec.ambient * ambient_color[2]];
        for (l, col) in &lights {
            let ndl = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0);
            // Reflection of the light about the normal, seen from straight above.
            let r = [2.0 * ndl * n[0] - l[0], 2.0 * ndl * n[1] - l[1], 2.0 * ndl * n[2] - l[2]];
            let spec_term = if ndl > 0.0 { r[2].max(0.0).powf(shin) } else { 0.0 };
            for k in 0..3 {
                o[k] += (spec.diffuse * ndl + spec.specular * spec_term) * col[k];
            }
        }
        [o[0].clamp(0.0, 1.0), o[1].clamp(0.0, 1.0), o[2].clamp(0.0, 1.0), c[3]]
    })
}

// ---- Palettize ----

/// Palettize parameters.
pub(crate) struct PalettizeSpec<'a> {
    /// Palette, straight sRGB.
    pub colors: &'a [[f32; 3]],
    pub space: PaletteSpace,
    pub dither: PaletteDither,
    /// Dither amplitude 0..1.
    pub threshold: f32,
    pub color_mode: PaletteColorMode,
    pub alpha_mode: PaletteAlphaMode,
    /// Palette entry transparent pixels take in `Index` mode.
    pub alpha_index: usize,
    /// Alpha at or below which a pixel counts as transparent (`Index` mode).
    pub alpha_threshold: f32,
}

/// A colour in the palette's measuring space.
fn to_space(space: PaletteSpace, c: [f32; 3]) -> [f32; 3] {
    match space {
        PaletteSpace::Rgb => c,
        PaletteSpace::Lab => {
            let l = srgb_to_lab(c);
            [l[0] / 100.0, l[1] / 128.0, l[2] / 128.0]
        }
    }
}

#[inline]
fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// The two nearest palette entries (by index) to `c` in the measuring space.
fn two_nearest(pal: &[[f32; 3]], c: [f32; 3]) -> (usize, usize) {
    let (mut b1, mut b2) = ((f32::MAX, 0usize), (f32::MAX, 0usize));
    for (i, p) in pal.iter().enumerate() {
        let d = dist2(*p, c);
        if d < b1.0 {
            b2 = b1;
            b1 = (d, i);
        } else if d < b2.0 {
            b2 = (d, i);
        }
    }
    if pal.len() == 1 {
        return (0, 0);
    }
    (b1.1, b2.1)
}

/// Palettize: every pixel snaps to the nearest palette colour (measured in RGB or Lab), with an
/// optional pattern, noise or error-diffusion dither, and the chosen treatment of alpha.
pub(crate) fn palettize(src: &Image, out: Rect, ctx: &Ctx, spec: &PalettizeSpec) -> Vec<f32> {
    let colors: Vec<[f32; 3]> = if spec.colors.is_empty() { vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]] } else { spec.colors.to_vec() };
    let pal: Vec<[f32; 3]> = colors.iter().map(|c| to_space(spec.space, *c)).collect();
    let spread = (1.0 / (pal.len() as f32).cbrt().max(1.0)).min(0.5) * spec.threshold.clamp(0.0, 1.0);
    let alpha_index = spec.alpha_index.min(colors.len() - 1);
    let w = out.width() as usize;
    // Error diffusion carries error between pixels: one buffer per channel for this row and
    // the next.
    let mut err = vec![[0.0f32; 3]; 2 * (w + 2)];
    let mut row_y = i32::MIN;
    let mut res = src.crop(out);
    let n = src.ch;
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32);
        if spec.dither == PaletteDither::Diffusion && y != row_y {
            let (cur, next) = err.split_at_mut(w + 2);
            cur.copy_from_slice(next);
            next.iter_mut().for_each(|e| *e = [0.0; 3]);
            row_y = y;
        }
        let c = crate::fxutil::rgba(ctx, px);
        let t = match spec.dither {
            PaletteDither::Pattern => bayer8(x.rem_euclid(8) as usize, y.rem_euclid(8) as usize),
            PaletteDither::Noise => hash01(x, y, 13, 0) - 0.5,
            _ => 0.0,
        };
        // Alpha first: it decides whether the colour matters.
        let (alpha, forced) = match spec.alpha_mode {
            PaletteAlphaMode::Clip => (if c[3] < 0.5 { 0.0 } else { 1.0 }, None),
            PaletteAlphaMode::Index => {
                if c[3] <= spec.alpha_threshold.clamp(0.0, 1.0) {
                    (1.0, Some(alpha_index))
                } else {
                    (1.0, None)
                }
            }
            PaletteAlphaMode::Dither => (if c[3] + t * spec.threshold.clamp(0.0, 1.0) < 0.5 { 0.0 } else { 1.0 }, None),
            PaletteAlphaMode::Unchanged => (c[3], None),
        };
        let k = if let Some(k) = forced {
            k
        } else {
            let mut m = to_space(spec.space, [c[0], c[1], c[2]]);
            match (spec.dither, spec.color_mode) {
                (PaletteDither::Diffusion, _) => {
                    let lx = (x - out.x0) as usize;
                    for q in 0..3 {
                        m[q] += err[lx + 1][q];
                    }
                    let k = two_nearest(&pal, m).0;
                    let e = pal[k];
                    let d: [f32; 3] = std::array::from_fn(|q| (m[q] - e[q]) * spec.threshold.clamp(0.0, 1.0));
                    let (cur, next) = err.split_at_mut(w + 2);
                    for q in 0..3 {
                        cur[lx + 2][q] += d[q] * 7.0 / 16.0;
                        next[lx][q] += d[q] * 3.0 / 16.0;
                        next[lx + 1][q] += d[q] * 5.0 / 16.0;
                        next[lx + 2][q] += d[q] / 16.0;
                    }
                    k
                }
                (PaletteDither::None, _) => two_nearest(&pal, m).0,
                (_, PaletteColorMode::PerChannelOffset) => two_nearest(&pal, m.map(|v| v + t * spread)).0,
                (_, PaletteColorMode::NearestColors) => {
                    let (k1, k2) = two_nearest(&pal, m);
                    if k1 == k2 {
                        k1
                    } else {
                        // Position of the pixel along the segment between the two candidates.
                        let (a, b) = (pal[k1], pal[k2]);
                        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                        let am = [m[0] - a[0], m[1] - a[1], m[2] - a[2]];
                        let len2 = dist2(a, b).max(1e-9);
                        let p = ((ab[0] * am[0] + ab[1] * am[1] + ab[2] * am[2]) / len2).clamp(0.0, 1.0);
                        let p = 0.5 + (p - 0.5) / spec.threshold.clamp(0.02, 1.0);
                        if p > t + 0.5 { k2 } else { k1 }
                    }
                }
            }
        };
        let o = colors[k];
        crate::fxutil::set_rgba(ctx, px, [o[0], o[1], o[2], alpha.clamp(0.0, 1.0)]);
    }
    res
}

// ---- Cross-Channel Adjustment ----

#[inline]
fn channel_value(ch: CrossChannel, rgb: [f32; 3], a: f32, hsl: [f32; 3]) -> f32 {
    match ch {
        CrossChannel::Red => rgb[0],
        CrossChannel::Green => rgb[1],
        CrossChannel::Blue => rgb[2],
        CrossChannel::Alpha => a,
        CrossChannel::Hue => hsl[0],
        CrossChannel::Saturation => hsl[1],
        CrossChannel::Lightness => hsl[2],
    }
}

/// Cross-Channel Adjustment: each curve offsets its `driven` channel by the curve's value at the
/// pixel's `driver` channel (0.5 = unchanged; a full swing is ±1 of the channel's range, ±½ a turn
/// for hue), applied in order.
pub(crate) fn cross_channel(src: &Image, out: Rect, ctx: &Ctx, curves: &[CrossCurve]) -> Vec<f32> {
    if curves.is_empty() {
        return src.crop(out);
    }
    map_px(src, out, ctx, |_, _, c| {
        let mut rgb = [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)];
        let mut a = c[3];
        for cv in curves {
            let hsl = rgb_to_hsl(rgb);
            let drv = channel_value(cv.driver, rgb, a, hsl);
            let offset = (curve_at(&cv.points, drv.clamp(0.0, 1.0)) - 0.5) * 2.0;
            match cv.driven {
                CrossChannel::Red => rgb[0] = (rgb[0] + offset).clamp(0.0, 1.0),
                CrossChannel::Green => rgb[1] = (rgb[1] + offset).clamp(0.0, 1.0),
                CrossChannel::Blue => rgb[2] = (rgb[2] + offset).clamp(0.0, 1.0),
                CrossChannel::Alpha => a = (a + offset).clamp(0.0, 1.0),
                CrossChannel::Hue => rgb = hsl_to_rgb([(hsl[0] + offset * 0.5).rem_euclid(1.0), hsl[1], hsl[2]]),
                CrossChannel::Saturation => rgb = hsl_to_rgb([hsl[0], (hsl[1] + offset).clamp(0.0, 1.0), hsl[2]]),
                CrossChannel::Lightness => rgb = hsl_to_rgb([hsl[0], hsl[1], (hsl[2] + offset).clamp(0.0, 1.0)]),
            }
        }
        [rgb[0], rgb[1], rgb[2], a]
    })
}

// ---- Slope, Offset, Power ----

/// ASC CDL grade: per channel `(v · slope + offset) ^ power`, then saturation about the Rec. 709
/// luma.
#[allow(clippy::too_many_arguments)]
pub(crate) fn slope_offset_power(src: &Image, out: Rect, ctx: &Ctx, slope: [f32; 3], offset: [f32; 3], power: [f32; 3], saturation: f32) -> Vec<f32> {
    map_px(src, out, ctx, |_, _, c| {
        let mut v = [0.0f32; 3];
        for k in 0..3 {
            v[k] = (c[k] * slope[k] + offset[k]).max(0.0).powf(power[k].max(1e-3));
        }
        let l = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
        for q in v.iter_mut() {
            *q = (l + (*q - l) * saturation).clamp(0.0, 1.0);
        }
        [v[0], v[1], v[2], c[3]]
    })
}

// ---- Raindrops ----

/// Pixels Raindrops reads around an output pixel.
pub(crate) fn raindrops_reach(drop_size: f32) -> f32 {
    drop_size.clamp(1.0, 200.0) * 2.0 + 2.0
}

/// Raindrops: `number` lens droplets of up to `drop_size` px radius scattered by `seed` over the
/// bounds, each magnifying what is under it like a fish-eye (`fish_eye` 1..100 sets the strength)
/// with a highlight on its upper-left rim and a shadow on the lower-right.
pub(crate) fn raindrops(src: &Image, out: Rect, ctx: &Ctx, drop_size: f32, number: u32, fish_eye: f32, seed: u32) -> Vec<f32> {
    let b = ctx.bounds;
    if b.is_empty() {
        return src.crop(out);
    }
    let size = drop_size.clamp(1.0, 200.0);
    let count = number.clamp(1, 500) as i32;
    let drops: Vec<(f32, f32, f32)> = (0..count)
        .map(|i| {
            let cx = b.x0 as f32 + hash01(i, 0, 1, seed) * b.width() as f32;
            let cy = b.y0 as f32 + hash01(i, 1, 1, seed) * b.height() as f32;
            let r = size * (0.4 + 0.6 * hash01(i, 2, 1, seed));
            (cx, cy, r)
        })
        .collect();
    let k = 1.0 + fish_eye.clamp(1.0, 100.0) / 30.0;
    let n = src.ch;
    let mut res = src.crop(out);
    let w = out.width() as usize;
    let mut tmp = [0.0f32; crate::fxutil::MAXC];
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32);
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let Some(&(cx, cy, r)) = drops.iter().find(|(cx, cy, r)| (fx - cx).powi(2) + (fy - cy).powi(2) <= r * r) else { continue };
        let (dx, dy) = (fx - cx, fy - cy);
        let d = (dx * dx + dy * dy).sqrt().max(1e-6);
        let u = d / r;
        // Fish-eye: the sampled point sits closer to the centre than the pixel.
        let s = u.powf(k) * r;
        let (sx, sy) = (cx + dx / d * s, cy + dy / d * s);
        src.sample(sx, sy, Edge::Repeat, src.rect, ctx.alpha, &mut tmp[..n]);
        let mut c = crate::fxutil::rgba(ctx, &tmp[..n]);
        // Rim shading: light from the upper left.
        let shade = ((-dx - dy) / (d * std::f32::consts::SQRT_2)) * 0.35 * u * u;
        for q in c.iter_mut().take(3) {
            *q = (*q + shade).clamp(0.0, 1.0);
        }
        crate::fxutil::set_rgba(ctx, px, c);
    }
    res
}

// ---- Maximize / Minimize Channel ----

/// Maximize Channel: the strongest colour channel of each pixel becomes 1.
pub(crate) fn maximize_channel(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    map_px(src, out, ctx, |_, _, c| {
        let m = c[0].max(c[1]).max(c[2]);
        let f = |v: f32| if v >= m { 1.0 } else { v };
        [f(c[0]), f(c[1]), f(c[2]), c[3]]
    })
}

/// Minimize Channel: the weakest colour channel of each pixel becomes 0.
pub(crate) fn minimize_channel(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    map_px(src, out, ctx, |_, _, c| {
        let m = c[0].min(c[1]).min(c[2]);
        let f = |v: f32| if v <= m { 0.0 } else { v };
        [f(c[0]), f(c[1]), f(c[2]), c[3]]
    })
}

// ---- Propagate Colors ----

/// Propagate Colors: a transparent pixel takes the alpha-weighted colour of the nearest ring of
/// non-transparent pixels within `radius`; alpha is unchanged, so texture edges stop bleeding
/// black when filtered or scaled.
pub(crate) fn propagate_colors(src: &Image, out: Rect, ctx: &Ctx, radius: i32) -> Vec<f32> {
    let r = radius.clamp(1, 256);
    map_px(src, out, ctx, |x, y, c| {
        if c[3] > 1e-4 {
            return c;
        }
        for ring in 1..=r {
            let mut acc = [0.0f32; 4];
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let (sx, sy) = (x + dx, y + dy);
                    if !src.rect.contains(sx, sy) {
                        continue;
                    }
                    let p = rgba_at(src, ctx, sx, sy);
                    if p[3] > 1e-4 {
                        acc[0] += p[0] * p[3];
                        acc[1] += p[1] * p[3];
                        acc[2] += p[2] * p[3];
                        acc[3] += p[3];
                    }
                }
            }
            if acc[3] > 0.0 {
                return [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], c[3]];
            }
        }
        c
    })
}

// ---- Height to Normal Map ----

/// Height to Normal Map parameters.
pub(crate) struct HeightToNormalSpec {
    pub method: EdgeMethod,
    pub h_radius: i32,
    pub v_radius: i32,
    pub channel: HeightChannel,
    /// What the red, green and blue outputs carry.
    pub swizzle: [NormalSwizzle; 3],
}

/// Height to Normal Map: the slope of the height channel (by `method` over the radii) as a unit
/// normal, each colour channel carrying the component its swizzle names, encoded −1..1 → 0..1.
pub(crate) fn height_to_normal(src: &Image, out: Rect, ctx: &Ctx, spec: &HeightToNormalSpec) -> Vec<f32> {
    map_px(src, out, ctx, |x, y, c| {
        let (gx, gy) = gradient(src, ctx, x, y, spec.method, spec.h_radius, spec.v_radius, spec.channel);
        let n = normalize3([-gx, -gy, 1.0]);
        let pick = |s: NormalSwizzle| -> f32 {
            let v = match s {
                NormalSwizzle::XPlus => n[0],
                NormalSwizzle::XMinus => -n[0],
                NormalSwizzle::YPlus => n[1],
                NormalSwizzle::YMinus => -n[1],
                NormalSwizzle::ZPlus => n[2],
                NormalSwizzle::ZMinus => -n[2],
            };
            (v * 0.5 + 0.5).clamp(0.0, 1.0)
        };
        [pick(spec.swizzle[0]), pick(spec.swizzle[1]), pick(spec.swizzle[2]), c[3]]
    })
}

// ---- Mean Removal ----

/// Mean Removal: sharpening by `9·centre − Σ(8 neighbours)` on every colour channel.
pub(crate) fn mean_removal(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    map_px(src, out, ctx, |x, y, c| {
        let mut s = [0.0f32; 3];
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx != 0 || dy != 0 {
                    let p = rgba_at(src, ctx, x + dx, y + dy);
                    for k in 0..3 {
                        s[k] += p[k];
                    }
                }
            }
        }
        [(9.0 * c[0] - s[0]).clamp(0.0, 1.0), (9.0 * c[1] - s[1]).clamp(0.0, 1.0), (9.0 * c[2] - s[2]).clamp(0.0, 1.0), c[3]]
    })
}

// ---- Wavelet Noise Reducer ----

/// Decomposition scales of the wavelet noise reducer.
const WAVELET_SCALES: usize = 4;

/// Pixels the Wavelet Noise Reducer reads around an output pixel.
pub(crate) fn wavelet_reach() -> f32 {
    // The à trous B3 kernel at scale s reaches 2·2^s; the scales sum to 2·(2^S − 1).
    (2 * ((1usize << WAVELET_SCALES) - 1)) as f32 + 1.0
}

/// One à trous pass (separable 1-4-6-4-1 / 16 with taps `step` apart) over an interleaved
/// 4-channel buffer; edges repeat.
fn atrous(buf: &[f32], w: usize, h: usize, step: usize) -> Vec<f32> {
    const K: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];
    let mut tmp = vec![0.0f32; buf.len()];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 4];
            for (t, k) in K.iter().enumerate() {
                let sx = (x as isize + (t as isize - 2) * step as isize).clamp(0, w as isize - 1) as usize;
                let j = (y * w + sx) * 4;
                for c in 0..4 {
                    acc[c] += buf[j + c] * k;
                }
            }
            tmp[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&acc);
        }
    }
    let mut res = vec![0.0f32; buf.len()];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 4];
            for (t, k) in K.iter().enumerate() {
                let sy = (y as isize + (t as isize - 2) * step as isize).clamp(0, h as isize - 1) as usize;
                let j = (sy * w + x) * 4;
                for c in 0..4 {
                    acc[c] += tmp[j + c] * k;
                }
            }
            res[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&acc);
        }
    }
    res
}

/// Wavelet Noise Reducer: the window is split into à trous wavelet scales, every detail
/// coefficient is soft-thresholded by `threshold` (0..1, halving per coarser scale) and the
/// scales are summed back.
pub(crate) fn wavelet_noise_reducer(src: &Image, out: Rect, ctx: &Ctx, threshold: f32) -> Vec<f32> {
    let t0 = threshold.clamp(0.0, 1.0) * 0.25;
    if t0 <= 0.0 {
        return src.crop(out);
    }
    let reach = wavelet_reach() as i32;
    let win = out.inflate(reach);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let base = rgba_window(src, ctx, win);
    let mut cur = base.clone();
    let mut sum = vec![0.0f32; base.len()];
    for s in 0..WAVELET_SCALES {
        let next = atrous(&cur, ww, wh, 1usize << s);
        let t = t0 / (1u32 << s) as f32;
        for (i, (c, n)) in cur.iter().zip(next.iter()).enumerate() {
            if i % 4 == 3 {
                continue;
            }
            let d = c - n;
            let d = if d > t {
                d - t
            } else if d < -t {
                d + t
            } else {
                0.0
            };
            sum[i] += d;
        }
        cur = next;
    }
    for (i, (s, c)) in sum.iter_mut().zip(cur.iter()).enumerate() {
        if i % 4 == 3 {
            *s = *c;
        } else {
            *s += c;
        }
    }
    let n = src.ch;
    let mut res = src.crop(out);
    let ow = out.width() as usize;
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % ow) as i32, out.y0 + (i / ow) as i32);
        let c = window_px(&sum, win, x, y);
        crate::fxutil::set_rgba(ctx, px, c.map(|v| v.clamp(0.0, 1.0)));
    }
    res
}

// ---- Normalize ----

/// Normalize: every pixel read as a normal (−1..1 per channel) is brought to unit length.
pub(crate) fn normalize_normals(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    map_px(src, out, ctx, |_, _, c| {
        let n = normalize3([c[0] * 2.0 - 1.0, c[1] * 2.0 - 1.0, c[2] * 2.0 - 1.0]);
        [n[0] * 0.5 + 0.5, n[1] * 0.5 + 0.5, n[2] * 0.5 + 0.5, c[3]]
    })
}

// ---- Reset Transparent ----

/// Reset Transparent: the colour under fully transparent pixels becomes black.
pub(crate) fn reset_transparent(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    map_px(src, out, ctx, |_, _, c| if c[3] <= 0.0 { [0.0, 0.0, 0.0, 0.0] } else { c })
}

// ---- Random Pick ----

/// Random Pick: `level` (0..1) of the pixels take a random neighbour within `window` px, mixed
/// in by `opacity`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn random_pick(src: &Image, out: Rect, ctx: &Ctx, level: f32, window: i32, opacity: f32, seed: u32) -> Vec<f32> {
    let w = window.clamp(1, 100);
    let k = opacity.clamp(0.0, 1.0);
    map_px(src, out, ctx, |x, y, c| {
        if hash01(x, y, 3, seed) >= level.clamp(0.0, 1.0) {
            return c;
        }
        let dx = (hash01(x, y, 4, seed) * (2 * w + 1) as f32) as i32 - w;
        let dy = (hash01(x, y, 5, seed) * (2 * w + 1) as f32) as i32 - w;
        let p = rgba_at(src, ctx, x + dx, y + dy);
        [c[0] + (p[0] - c[0]) * k, c[1] + (p[1] - c[1]) * k, c[2] + (p[2] - c[2]) * k, c[3] + (p[3] - c[3]) * k]
    })
}
