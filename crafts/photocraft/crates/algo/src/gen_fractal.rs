// SPDX-License-Identifier: MIT OR Apache-2.0
//! Fractal and ray-traced generators: Fractal Explorer, IFS Fractal and Sphere Designer.

use redrob_photo_geom::Rect;

use crate::fxutil::{set_rgba, xy};
use crate::gen_util::{Stream, fbm, frame, generate, hash, mix, over, value_noise};
use crate::image::{Edge, Image};
use crate::{Ctx, ExplorerFractal, IfsPreset, SphereTexture};

/// Fractal Explorer settings: the complex window, iteration limit, Julia-type seed, number of
/// colour bands and per-channel intensity / inversion.
pub(crate) struct ExplorerSpec {
    pub kind: ExplorerFractal,
    pub x1: f32,
    pub x2: f32,
    pub y1: f32,
    pub y2: f32,
    pub iterations: u32,
    pub cx: f32,
    pub cy: f32,
    pub colors: u32,
    pub intensity: [f32; 3],
    pub invert: [bool; 3],
}

/// Escape-time value 0…1 of the point `(x, y)` (1 = never escaped).
fn escape(kind: ExplorerFractal, x: f64, y: f64, cx: f64, cy: f64, limit: u32) -> f64 {
    let (mut zr, mut zi) = (x, y);
    let (mut cr, mut ci) = match kind {
        ExplorerFractal::Mandelbrot | ExplorerFractal::Spider => {
            zr = 0.0;
            zi = 0.0;
            (x, y)
        }
        ExplorerFractal::Sierpinski => (0.0, 0.0),
        _ => (cx, cy),
    };
    let (mut pr, mut pi) = (0.0f64, 0.0f64);
    for i in 0..limit {
        let (nr, ni) = match kind {
            ExplorerFractal::Mandelbrot | ExplorerFractal::Julia => (zr * zr - zi * zi + cr, 2.0 * zr * zi + ci),
            ExplorerFractal::Barnsley1 => {
                let (tr, ti) = if zr >= 0.0 { (zr - 1.0, zi) } else { (zr + 1.0, zi) };
                (tr * cr - ti * ci, tr * ci + ti * cr)
            }
            ExplorerFractal::Barnsley2 => {
                let (tr, ti) = if zi * cr + zr * ci >= 0.0 { (zr - 1.0, zi) } else { (zr + 1.0, zi) };
                (tr * cr - ti * ci, tr * ci + ti * cr)
            }
            ExplorerFractal::Barnsley3 => {
                let (sr, si) = (zr * zr - zi * zi, 2.0 * zr * zi);
                if zr > 0.0 { (sr - 1.0, si) } else { (sr - 1.0 + cr * zr, si + ci * zr) }
            }
            ExplorerFractal::Spider => {
                let n = (zr * zr - zi * zi + cr, 2.0 * zr * zi + ci);
                cr = cr / 2.0 + n.0;
                ci = ci / 2.0 + n.1;
                n
            }
            ExplorerFractal::ManOWar => {
                let n = (zr * zr - zi * zi + pr + cr, 2.0 * zr * zi + pi + ci);
                pr = zr;
                pi = zi;
                n
            }
            ExplorerFractal::Lambda => {
                // c · z · (1 − z)
                let (ur, ui) = (zr - (zr * zr - zi * zi), zi - 2.0 * zr * zi);
                (ur * cr - ui * ci, ur * ci + ui * cr)
            }
            ExplorerFractal::Sierpinski => {
                if zi > 0.5 {
                    (2.0 * zr, 2.0 * zi - 1.0)
                } else if zr > 0.5 {
                    (2.0 * zr - 1.0, 2.0 * zi)
                } else {
                    (2.0 * zr, 2.0 * zi)
                }
            }
        };
        zr = nr;
        zi = ni;
        let m = zr * zr + zi * zi;
        if m > 1e6 || !m.is_finite() {
            // Smooth iteration count.
            let smooth = i as f64 + 1.0 - (m.max(1.0001).ln().ln() / std::f64::consts::LN_2).clamp(0.0, 1.0);
            return (smooth / limit as f64).clamp(0.0, 1.0);
        }
    }
    1.0
}

pub(crate) fn fractal_explorer(src: &Image, out: Rect, ctx: &Ctx, spec: &ExplorerSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (w, h) = (b.width() as f64, b.height() as f64);
    let (x1, x2, y1, y2) = (spec.x1 as f64, spec.x2 as f64, spec.y1 as f64, spec.y2 as f64);
    let limit = spec.iterations.clamp(1, 10_000);
    let bands = spec.colors.clamp(2, 8192) as f64;
    generate(src, out, ctx, |x, y| {
        let u = (x as f64 - b.x0 as f64) / w.max(1.0);
        let v = (y as f64 - b.y0 as f64) / h.max(1.0);
        let t = escape(spec.kind, x1 + (x2 - x1) * u, y1 + (y2 - y1) * v, spec.cx as f64, spec.cy as f64, limit);
        if t >= 1.0 {
            return [0.0, 0.0, 0.0, 1.0];
        }
        let mut c = [0.0f32; 4];
        for k in 0..3 {
            let phase = (t * bands).fract() as f32;
            let mut v = 0.5 + 0.5 * (std::f32::consts::TAU * (phase + k as f32 / 3.0 * spec.intensity[k].clamp(0.0, 1.0))).sin();
            v *= spec.intensity[k].clamp(0.0, 1.0);
            if spec.invert[k] {
                v = 1.0 - v;
            }
            c[k] = v.clamp(0.0, 1.0);
        }
        c[3] = 1.0;
        c
    })
}

/// IFS Fractal settings: the preset map set, chaos-game iterations (×1000), spot radius in px,
/// a global offset (fractions of the bounds), scale and angle (degrees) of the attractor,
/// and the colour the points are drawn in.
pub(crate) struct IfsSpec {
    pub preset: IfsPreset,
    pub iterations: u32,
    pub spot_radius: f32,
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub angle: f32,
    pub color: [f32; 4],
    pub seed: u32,
}

/// Affine maps `[a, b, c, d, e, f]` (x' = ax + by + e, y' = cx + dy + f) with their weights.
fn ifs_maps(preset: IfsPreset) -> Vec<([f32; 6], f32)> {
    match preset {
        IfsPreset::Fern => vec![
            ([0.0, 0.0, 0.0, 0.16, 0.0, 0.0], 0.01),
            ([0.85, 0.04, -0.04, 0.85, 0.0, 1.6], 0.85),
            ([0.2, -0.26, 0.23, 0.22, 0.0, 1.6], 0.07),
            ([-0.15, 0.28, 0.26, 0.24, 0.0, 0.44], 0.07),
        ],
        IfsPreset::Sierpinski => vec![([0.5, 0.0, 0.0, 0.5, 0.0, 0.0], 1.0), ([0.5, 0.0, 0.0, 0.5, 0.5, 0.0], 1.0), ([0.5, 0.0, 0.0, 0.5, 0.25, 0.5], 1.0)],
        IfsPreset::Dragon => vec![([0.5, -0.5, 0.5, 0.5, 0.0, 0.0], 1.0), ([-0.5, -0.5, 0.5, -0.5, 1.0, 0.0], 1.0)],
        IfsPreset::Spiral => vec![
            ([0.787879, -0.424242, 0.242424, 0.859848, 1.758647, 1.408065], 0.9),
            ([-0.121212, 0.257576, 0.151515, 0.053030, -6.721654, 1.377236], 0.05),
            ([0.181818, -0.136364, 0.090909, 0.181818, 6.086107, 1.568035], 0.05),
        ],
        IfsPreset::Tree => vec![
            ([0.0, 0.0, 0.0, 0.5, 0.0, 0.0], 0.05),
            ([0.42, -0.42, 0.42, 0.42, 0.0, 0.2], 0.4),
            ([0.42, 0.42, -0.42, 0.42, 0.0, 0.2], 0.4),
            ([0.1, 0.0, 0.0, 0.1, 0.0, 0.2], 0.15),
        ],
        IfsPreset::Snowflake => vec![
            ([0.333, 0.0, 0.0, 0.333, 0.0, 0.0], 1.0),
            ([0.333, 0.0, 0.0, 0.333, 0.667, 0.0], 1.0),
            ([0.333, 0.0, 0.0, 0.333, 0.333, 0.577], 1.0),
            ([0.333, 0.0, 0.0, 0.333, 0.333, -0.577], 1.0),
            ([0.333, 0.0, 0.0, 0.333, 0.333, 0.0], 1.0),
        ],
    }
}

pub(crate) fn ifs_fractal(src: &Image, out: Rect, ctx: &Ctx, spec: &IfsSpec) -> Vec<f32> {
    let maps = ifs_maps(spec.preset);
    let total: f32 = maps.iter().map(|m| m.1).sum();
    let mut rng = Stream::new(spec.seed);
    let count = (spec.iterations.clamp(1, 2000) as usize) * 1000;
    // First pass: the attractor's extent, so it is fitted into the bounds.
    let step = |p: (f32, f32), rng: &mut Stream| {
        let mut r = rng.unit() * total;
        for (m, w) in &maps {
            if r <= *w {
                return (m[0] * p.0 + m[1] * p.1 + m[4], m[2] * p.0 + m[3] * p.1 + m[5]);
            }
            r -= w;
        }
        p
    };
    let mut p = (0.0f32, 0.0f32);
    let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    let probe = count.min(20_000);
    let mut pts: Vec<(f32, f32)> = Vec::with_capacity(probe);
    for i in 0..probe {
        p = step(p, &mut rng);
        if i > 20 && p.0.is_finite() && p.1.is_finite() {
            lo[0] = lo[0].min(p.0);
            lo[1] = lo[1].min(p.1);
            hi[0] = hi[0].max(p.0);
            hi[1] = hi[1].max(p.1);
            pts.push(p);
        }
    }
    if !lo[0].is_finite() || hi[0] - lo[0] < 1e-6 || hi[1] - lo[1] < 1e-6 {
        return src.crop(out);
    }
    let (cx, cy, w, h) = frame(ctx);
    let ext = (hi[0] - lo[0]).max(hi[1] - lo[1]);
    let unit = 0.9 * w.min(h) / ext * spec.scale.clamp(0.01, 10.0);
    let (sa, ca) = spec.angle.to_radians().sin_cos();
    let mid = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    let place = |q: (f32, f32)| {
        let (dx, dy) = ((q.0 - mid.0) * unit, -(q.1 - mid.1) * unit);
        (cx + spec.x * w + dx * ca - dy * sa, cy + spec.y * h + dx * sa + dy * ca)
    };
    // Density over the tile grown by the spot radius.
    let r = spec.spot_radius.clamp(0.3, 20.0);
    let margin = r.ceil() as i32 + 1;
    let area = out.inflate(margin);
    let (aw, ah) = (area.width() as usize, area.height() as usize);
    let mut density = vec![0.0f32; aw * ah];
    let mut plot = |q: (f32, f32)| {
        let (px, py) = place(q);
        let (ix, iy) = (px.floor() as i32, py.floor() as i32);
        if ix < area.x0 || iy < area.y0 || ix >= area.x1 || iy >= area.y1 {
            return;
        }
        density[(iy - area.y0) as usize * aw + (ix - area.x0) as usize] += 1.0;
    };
    for &q in &pts {
        plot(q);
    }
    for _ in probe..count {
        p = step(p, &mut rng);
        plot(p);
    }
    // Each hit lights a disc of radius `r`; coverage saturates.
    let n = src.ch;
    let mut res = src.crop(out);
    let rr = r * r;
    let gain = 0.6 / r.max(1.0);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let mut sum = 0.0f32;
        for dy in -margin..=margin {
            for dx in -margin..=margin {
                if (dx * dx + dy * dy) as f32 > rr + 0.5 {
                    continue;
                }
                let (sx, sy) = (x + dx - area.x0, y + dy - area.y0);
                if sx >= 0 && sy >= 0 && (sx as usize) < aw && (sy as usize) < ah {
                    sum += density[sy as usize * aw + sx as usize];
                }
            }
        }
        let cov = (sum * gain).min(1.0);
        if cov <= 0.0 {
            continue;
        }
        let under = crate::fxutil::rgba(ctx, px);
        set_rgba(ctx, px, over([spec.color[0], spec.color[1], spec.color[2], spec.color[3] * cov], under));
    }
    res
}

/// Sphere Designer settings: texture and its two colours, texture scale, turbulence and amount
/// (texture strength), specular exponent, light direction (bounds units; the sphere is the unit
/// ball), ambient 0…1, bump strength 0…1, sphere radius as a fraction of the shorter side.
pub(crate) struct SphereSpec {
    pub texture: SphereTexture,
    pub color1: [f32; 4],
    pub color2: [f32; 4],
    pub scale: f32,
    pub turbulence: f32,
    pub amount: f32,
    pub exponent: f32,
    pub light_x: f32,
    pub light_y: f32,
    pub light_z: f32,
    pub ambient: f32,
    pub bump: f32,
    pub radius: f32,
    pub transparent_background: bool,
}

/// The texture value 0…1 at a surface point of the unit sphere.
fn texture(t: SphereTexture, p: [f32; 3], scale: f32, turb: f32, seed: u32) -> f32 {
    let s = scale.max(0.05) * 4.0;
    let noise = |k: f32| fbm(p[0] * s * k + 3.7, (p[1] + p[2] * 0.5) * s * k, 4, seed);
    match t {
        SphereTexture::Solid => 0.0,
        SphereTexture::Checker => {
            let lon = p[0].atan2(p[2]) / std::f32::consts::TAU + 0.5;
            let lat = p[1].clamp(-1.0, 1.0).asin() / std::f32::consts::PI + 0.5;
            let i = (lon * s * 2.0).floor() as i64 + (lat * s).floor() as i64;
            if i.rem_euclid(2) == 0 { 0.0 } else { 1.0 }
        }
        SphereTexture::Marble => (std::f32::consts::PI * (p[0] * s + turb * 2.0 * noise(1.0))).sin().abs(),
        SphereTexture::Lizard => {
            let v = value_noise(p[0] * s * 2.0, (p[1] + p[2]) * s * 2.0, seed);
            if v > 0.55 { 1.0 } else { 0.0 }
        }
        SphereTexture::Phong => 0.0,
        SphereTexture::Noise => noise(1.0 + turb),
        SphereTexture::Wood => {
            let r = (p[0] * p[0] + p[1] * p[1]).sqrt() * s + turb * noise(0.7);
            (r.fract() * 2.0 - 1.0).abs()
        }
        SphereTexture::Spiral => {
            let a = p[1].atan2(p[0]) / std::f32::consts::TAU;
            let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
            ((a + r * s + turb * 0.2 * noise(1.0)).rem_euclid(1.0) * 2.0 - 1.0).abs()
        }
        SphereTexture::Spots => {
            let (u, v) = (p[0] * s * 1.5, (p[1] + 0.3 * p[2]) * s * 1.5);
            let (iu, iv) = (u.floor(), v.floor());
            let cx = iu + 0.3 + 0.4 * hash(iu as i32, iv as i32, seed);
            let cy = iv + 0.3 + 0.4 * hash(iu as i32, iv as i32, seed ^ 0x33);
            let d = ((u - cx) * (u - cx) + (v - cy) * (v - cy)).sqrt();
            (1.0 - (d / 0.3).clamp(0.0, 1.0)).powf(0.5)
        }
    }
}

pub(crate) fn sphere_designer(src: &Image, out: Rect, ctx: &Ctx, spec: &SphereSpec) -> Vec<f32> {
    let (cx, cy, w, h) = frame(ctx);
    let radius = (spec.radius.clamp(0.05, 2.0) * w.min(h) / 2.0).max(1.0);
    let light = {
        let l = [spec.light_x, spec.light_y, spec.light_z];
        let len = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt().max(1e-6);
        [l[0] / len, l[1] / len, l[2] / len]
    };
    let seed = 0x5e4a;
    let n = src.ch;
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut tmp = vec![0.0f32; n];
    let amount = spec.amount.clamp(0.0, 1.0);
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let (u, v) = ((fx - cx) / radius, -(fy - cy) / radius);
            let d2 = u * u + v * v;
            if d2 > 1.0 {
                if spec.transparent_background && ctx.alpha {
                    tmp.iter_mut().for_each(|s| *s = 0.0);
                } else {
                    src.sample(fx, fy, Edge::Repeat, ctx.bounds, ctx.alpha, &mut tmp);
                }
                res.extend_from_slice(&tmp);
                continue;
            }
            let z = (1.0 - d2).sqrt();
            let p = [u, v, z];
            let t = texture(spec.texture, p, spec.scale, spec.turbulence, seed);
            // Bump: tilt the normal by the texture gradient.
            let mut nrm = p;
            if spec.bump > 0.0 {
                let e = 0.01;
                let tx = texture(spec.texture, [u + e, v, z], spec.scale, spec.turbulence, seed) - t;
                let ty = texture(spec.texture, [u, v + e, z], spec.scale, spec.turbulence, seed) - t;
                nrm = [u - tx * spec.bump * 20.0, v - ty * spec.bump * 20.0, z];
                let len = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt().max(1e-6);
                nrm = [nrm[0] / len, nrm[1] / len, nrm[2] / len];
            }
            let diff = (nrm[0] * light[0] + nrm[1] * light[1] + nrm[2] * light[2]).max(0.0);
            let refl = [2.0 * diff * nrm[0] - light[0], 2.0 * diff * nrm[1] - light[1], 2.0 * diff * nrm[2] - light[2]];
            let spec_term = refl[2].max(0.0).powf(spec.exponent.max(1.0));
            let shine = if spec.texture == SphereTexture::Phong { 1.0 } else { 0.5 };
            let base = mix(spec.color1, spec.color2, t * amount);
            let shade = (spec.ambient.clamp(0.0, 1.0) + (1.0 - spec.ambient.clamp(0.0, 1.0)) * diff).clamp(0.0, 1.0);
            // Edge anti-aliasing of the disc.
            let edge = ((1.0 - d2.sqrt()) * radius + 0.5).clamp(0.0, 1.0);
            let c = [
                (base[0] * shade + spec_term * shine).clamp(0.0, 1.0),
                (base[1] * shade + spec_term * shine).clamp(0.0, 1.0),
                (base[2] * shade + spec_term * shine).clamp(0.0, 1.0),
                edge,
            ];
            src.sample(fx, fy, Edge::Repeat, ctx.bounds, ctx.alpha, &mut tmp);
            let under = if spec.transparent_background && ctx.alpha { [0.0, 0.0, 0.0, 0.0] } else { crate::fxutil::rgba(ctx, &tmp) };
            set_rgba(ctx, &mut tmp, over(c, under));
            res.extend_from_slice(&tmp);
        }
    }
    res
}
