// SPDX-License-Identifier: MIT OR Apache-2.0
//! Noise-driven generators: Cell Noise, Simplex Noise, Lava, Circuit, Qbist and the CML
//! Explorer.

use redrob_photo_geom::Rect;

use crate::fxutil::{rgba, set_rgba, xy};
use crate::gen_util::{Stream, generate, hash, hsv, ramp, simplex, to_hsv, value_noise};
use crate::image::Image;
use crate::{CmlFunction, Ctx, LavaGradient};

/// Cell Noise settings: feature size in px, distance metric exponent (1 = Manhattan,
/// 2 = Euclidean, higher tends to Chebyshev), which nearest feature (rank), octaves,
/// palettize (flat colour per cell) and seed.
pub(crate) struct CellSpec {
    pub scale: f32,
    pub shape: f32,
    pub rank: u32,
    pub iterations: u32,
    pub palettize: bool,
    pub seed: u32,
}

/// Worley distance of rank `rank` to jittered feature points, 0…1.
fn worley(x: f32, y: f32, shape: f32, rank: usize, palettize: bool, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let p = shape.clamp(1.0, 8.0);
    let mut dists = [f32::INFINITY; 4];
    let mut nearest_cell = 0.0f32;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (cx, cy) = (ix + dx, iy + dy);
            let fx = cx as f32 + hash(cx, cy, seed);
            let fy = cy as f32 + hash(cx, cy, seed ^ 0x9e37);
            let (ax, ay) = ((x - fx).abs(), (y - fy).abs());
            let d = (ax.powf(p) + ay.powf(p)).powf(1.0 / p);
            // Insert into the sorted smallest-four list.
            let mut k = 4;
            while k > 0 && dists[k - 1] > d {
                k -= 1;
            }
            if k < 4 {
                for m in (k + 1..4).rev() {
                    dists[m] = dists[m - 1];
                }
                dists[k] = d;
                if k == 0 {
                    nearest_cell = hash(cx, cy, seed ^ 0x51f1);
                }
            }
        }
    }
    if palettize {
        return nearest_cell;
    }
    dists[rank.min(3)].clamp(0.0, 1.5) / 1.5
}

pub(crate) fn cell_noise(src: &Image, out: Rect, ctx: &Ctx, spec: &CellSpec) -> Vec<f32> {
    let scale = spec.scale.max(1.0);
    let octaves = spec.iterations.clamp(1, 8);
    let rank = (spec.rank.clamp(1, 3) - 1) as usize;
    generate(src, out, ctx, |x, y| {
        let (mut sum, mut amp, mut norm, mut f) = (0.0, 1.0, 0.0, 1.0 / scale);
        for o in 0..octaves {
            sum += worley(x * f, y * f, spec.shape, rank, spec.palettize, spec.seed.wrapping_add(o * 131)) * amp;
            norm += amp;
            amp *= 0.5;
            f *= 2.0;
        }
        let v = (sum / norm).clamp(0.0, 1.0);
        [v, v, v, 1.0]
    })
}

pub(crate) fn simplex_noise(src: &Image, out: Rect, ctx: &Ctx, scale: f32, iterations: u32, seed: u32) -> Vec<f32> {
    let scale = scale.max(0.5);
    let octaves = iterations.clamp(1, 8);
    generate(src, out, ctx, |x, y| {
        let (mut sum, mut amp, mut norm, mut f) = (0.0, 1.0, 0.0, 1.0 / scale);
        for o in 0..octaves {
            sum += simplex(x * f, y * f, seed.wrapping_add(o * 977)) * amp;
            norm += amp;
            amp *= 0.5;
            f *= 2.0;
        }
        let v = (sum / norm * 0.5 + 0.5).clamp(0.0, 1.0);
        [v, v, v, 1.0]
    })
}

/// The built-in gradients Lava colours with.
fn lava_stops(g: LavaGradient) -> &'static [(f32, [f32; 3])] {
    match g {
        LavaGradient::Incandescent => {
            &[(0.0, [0.0, 0.0, 0.0]), (0.35, [0.6, 0.05, 0.0]), (0.65, [1.0, 0.5, 0.0]), (0.85, [1.0, 0.95, 0.4]), (1.0, [1.0, 1.0, 1.0])]
        }
        LavaGradient::GermanFlag => &[(0.0, [0.0, 0.0, 0.0]), (0.5, [0.85, 0.05, 0.05]), (1.0, [1.0, 0.8, 0.0])],
        LavaGradient::CaribbeanBlues => &[(0.0, [0.0, 0.1, 0.3]), (0.5, [0.0, 0.6, 0.8]), (1.0, [0.8, 1.0, 1.0])],
        LavaGradient::Golden => &[(0.0, [0.3, 0.15, 0.0]), (0.5, [0.9, 0.7, 0.2]), (1.0, [1.0, 1.0, 0.8])],
        LavaGradient::FullSpectrum => &[
            (0.0, [1.0, 0.0, 0.0]),
            (0.17, [1.0, 1.0, 0.0]),
            (0.33, [0.0, 1.0, 0.0]),
            (0.5, [0.0, 1.0, 1.0]),
            (0.67, [0.0, 0.0, 1.0]),
            (0.83, [1.0, 0.0, 1.0]),
            (1.0, [1.0, 0.0, 0.0]),
        ],
    }
}

/// Lava: turbulent (folded) fractal noise of feature size `size` px and `roughness` octaves,
/// coloured through a gradient.
pub(crate) fn lava(src: &Image, out: Rect, ctx: &Ctx, seed: u32, size: f32, roughness: f32, gradient: LavaGradient) -> Vec<f32> {
    let size = size.max(2.0);
    let octaves = roughness.clamp(1.0, 10.0).round() as u32;
    let stops = lava_stops(gradient);
    generate(src, out, ctx, |x, y| {
        let (mut sum, mut amp, mut norm, mut f) = (0.0, 1.0, 0.0, 1.0 / size);
        for o in 0..octaves {
            // Folded noise gives the sharp creases of cooling lava.
            sum += (value_noise(x * f, y * f, seed.wrapping_add(o * 7331)) * 2.0 - 1.0).abs() * amp;
            norm += amp;
            amp *= 0.55;
            f *= 2.1;
        }
        let t = 1.0 - (sum / norm).clamp(0.0, 1.0);
        ramp(stops, t * t)
    })
}

/// Circuit: a maze of traces at the `mask_size` pitch drawn as light tracks with dark gaps
/// (the look of a printed board after an oil-paint smoothing). With `no_background` the gaps
/// are transparent instead of dark.
pub(crate) fn circuit(src: &Image, out: Rect, ctx: &Ctx, mask_size: f32, seed: u32, no_background: bool) -> Vec<f32> {
    let b = ctx.bounds;
    let cell = (mask_size.clamp(3.0, 50.0) / 2.0).max(1.5);
    let gw = (((b.width() as f32 / cell).floor() as usize) | 1).max(5);
    let gh = (((b.height() as f32 / cell).floor() as usize) | 1).max(5);
    let wall = crate::gen_pattern::carve_for_circuit(gw, gh, seed);
    let n = src.ch;
    let mut res = src.crop(out);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let u = (fx - b.x0 as f32) / cell;
        let v = (fy - b.y0 as f32) / cell;
        let (gi, gj) = ((u.floor() as i64).rem_euclid(gw as i64) as usize, (v.floor() as i64).rem_euclid(gh as i64) as usize);
        // Trace coverage: passages are tracks; smooth the edges with the distance to the cell centre.
        let track = if wall[gj * gw + gi] { 0.0 } else { 1.0 };
        let (lu, lv) = (u.fract() - 0.5, v.fract() - 0.5);
        let edge = 1.0 - (lu.abs().max(lv.abs()) * 2.0).powf(4.0);
        let cov = (track * (0.55 + 0.45 * edge)).clamp(0.0, 1.0);
        let grain = 0.85 + 0.15 * value_noise(fx * 0.5, fy * 0.5, seed ^ 0x77);
        let tone = 0.25 + 0.55 * cov * grain;
        let c = if no_background {
            [tone, tone, tone, cov]
        } else {
            [tone * cov + 0.08 * (1.0 - cov), tone * cov + 0.08 * (1.0 - cov), tone * cov + 0.08 * (1.0 - cov), 1.0]
        };
        if no_background && cov <= 0.0 && ctx.alpha {
            px.iter_mut().for_each(|s| *s = 0.0);
        } else {
            set_rgba(ctx, px, c);
        }
    }
    res
}

/// Qbist: a random program of 36 vector operations on 6 colour registers seeded from the
/// pixel position, run per pixel. The seed picks the program; `oversample` anti-aliases.
pub(crate) fn qbist(src: &Image, out: Rect, ctx: &Ctx, seed: u32, antialias: bool, oversample: u32) -> Vec<f32> {
    #[derive(Clone, Copy)]
    struct Op {
        kind: u8,
        src: usize,
        ctl: usize,
        dst: usize,
    }
    let mut rng = Stream::new(seed);
    let program: Vec<Op> = (0..36).map(|_| Op { kind: rng.below(9) as u8, src: rng.below(6), ctl: rng.below(6), dst: rng.below(6) }).collect();
    let (cx, cy, w, h) = crate::gen_util::frame(ctx);
    let scale = 1.0 / w.min(h).max(1.0);
    let eval = |x: f32, y: f32| -> [f32; 3] {
        let u = ((x - cx) * scale + 0.5).clamp(0.0, 1.0);
        let v = ((y - cy) * scale + 0.5).clamp(0.0, 1.0);
        let mut r: [[f32; 3]; 6] =
            [[u, v, (u + v) * 0.5], [v, u, u * v], [u * u, v * v, u * v], [1.0 - u, 1.0 - v, 0.5], [u, 1.0 - v, (u - v).abs()], [0.5, 0.5, 0.5]];
        for op in &program {
            let s = r[op.src];
            let c = r[op.ctl];
            let d = &mut r[op.dst];
            match op.kind {
                0 => {
                    // Projection onto the control vector.
                    let dot = s[0] * c[0] + s[1] * c[1] + s[2] * c[2];
                    *d = [dot * c[0], dot * c[1], dot * c[2]];
                }
                1 => *d = [(s[0] + c[0]).fract(), (s[1] + c[1]).fract(), (s[2] + c[2]).fract()],
                2 => *d = [(s[0] - c[0]).rem_euclid(1.0), (s[1] - c[1]).rem_euclid(1.0), (s[2] - c[2]).rem_euclid(1.0)],
                3 => *d = [s[1], s[2], s[0]],
                4 => *d = [s[2], s[0], s[1]],
                5 => *d = [s[0] * c[0], s[1] * c[1], s[2] * c[2]],
                6 => {
                    *d = [
                        0.5 + 0.5 * (std::f32::consts::TAU * s[0] * (0.5 + c[0])).sin(),
                        0.5 + 0.5 * (std::f32::consts::TAU * s[1] * (0.5 + c[1])).sin(),
                        0.5 + 0.5 * (std::f32::consts::TAU * s[2] * (0.5 + c[2])).sin(),
                    ]
                }
                7 => {
                    if c[0] + c[1] + c[2] > 1.5 {
                        *d = s;
                    }
                }
                _ => *d = [1.0 - s[0], 1.0 - s[1], 1.0 - s[2]],
            }
        }
        r[0]
    };
    let ss = if antialias { oversample.clamp(1, 4) as i32 } else { 1 };
    generate(src, out, ctx, |x, y| {
        let mut acc = [0.0f32; 3];
        for j in 0..ss {
            for i in 0..ss {
                let ox = (i as f32 + 0.5) / ss as f32 - 0.5;
                let oy = (j as f32 + 0.5) / ss as f32 - 0.5;
                let c = eval(x + ox, y + oy);
                acc[0] += c[0];
                acc[1] += c[1];
                acc[2] += c[2];
            }
        }
        let k = 1.0 / (ss * ss) as f32;
        [(acc[0] * k).clamp(0.0, 1.0), (acc[1] * k).clamp(0.0, 1.0), (acc[2] * k).clamp(0.0, 1.0), 1.0]
    })
}

/// CML Explorer settings: the map each HSV channel evolves by, the map parameter `k`
/// (0…10), the coupling to the neighbours (0…1), the lattice zoom (px per cell), the start
/// row offset and seed.
pub(crate) struct CmlSpec {
    pub hue: CmlFunction,
    pub saturation: CmlFunction,
    pub value: CmlFunction,
    pub k: f32,
    pub coupling: f32,
    pub zoom: f32,
    pub start_offset: u32,
    pub seed: u32,
}

/// One step of a map on a lattice value.
fn cml_map(f: CmlFunction, v: f32, k: f32, rnd: f32) -> f32 {
    match f {
        CmlFunction::KeepImage | CmlFunction::KeepFirstValue => v,
        CmlFunction::FillWithK => (k / 10.0).clamp(0.0, 1.0),
        CmlFunction::Logistic => {
            let p = 2.5 + 1.5 * (k / 10.0).clamp(0.0, 1.0);
            (p * v * (1.0 - v)).clamp(0.0, 1.0)
        }
        CmlFunction::DeltaSinCurve => (v + 0.1 * k * (std::f32::consts::TAU * v).sin()).rem_euclid(1.0),
        CmlFunction::SinCurve => 0.5 + 0.5 * (std::f32::consts::PI * (v * (1.0 + k * 0.2) - 0.5)).sin(),
        CmlFunction::SinCurveStep => {
            let s = 0.5 + 0.5 * (std::f32::consts::PI * (v * (1.0 + k * 0.2) - 0.5)).sin();
            (s * 8.0).round() / 8.0
        }
        CmlFunction::Random => rnd,
    }
}

/// Runs the lattice for channel `f`: returns `cells` columns × `rows` rows.
#[allow(clippy::too_many_arguments)]
fn cml_channel(f: CmlFunction, cells: usize, rows: usize, k: f32, coupling: f32, start_offset: usize, seed: u32, initial: &[f32]) -> Vec<f32> {
    let mut rng = Stream::new(seed);
    let mut row: Vec<f32> =
        (0..cells).map(|i| if matches!(f, CmlFunction::KeepImage) { initial[i.min(initial.len().saturating_sub(1))] } else { rng.unit() }).collect();
    let first = row.clone();
    let c = coupling.clamp(0.0, 1.0);
    let mut lattice = Vec::with_capacity(cells * rows);
    let mut next = row.clone();
    for r in 0..rows + start_offset {
        for i in 0..cells {
            let l = row[(i + cells - 1) % cells];
            let rgt = row[(i + 1) % cells];
            let mapped = cml_map(f, row[i], k, rng.unit());
            let ml = cml_map(f, l, k, rng.unit());
            let mr = cml_map(f, rgt, k, rng.unit());
            next[i] = match f {
                CmlFunction::KeepFirstValue => first[i],
                CmlFunction::KeepImage => initial[(r.min(rows - 1)) * cells + i.min(cells - 1)],
                _ => ((1.0 - c) * mapped + c * 0.5 * (ml + mr)).clamp(0.0, 1.0),
            };
        }
        std::mem::swap(&mut row, &mut next);
        if r >= start_offset {
            lattice.extend_from_slice(&row);
        }
    }
    lattice
}

pub(crate) fn cml_explorer(src: &Image, out: Rect, ctx: &Ctx, spec: &CmlSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let zoom = spec.zoom.clamp(1.0, 64.0);
    let cells = ((b.width() as f32 / zoom).ceil() as usize).max(1);
    let rows = ((b.height() as f32 / zoom).ceil() as usize).max(1);
    // The image's own HSV, sampled per cell, for the Keep Image function.
    let mut img = vec![[0.0f32; 3]; cells * rows];
    let n = src.ch;
    for r in 0..rows {
        for i in 0..cells {
            let x = b.x0 + ((i as f32 + 0.5) * zoom) as i32;
            let y = b.y0 + ((r as f32 + 0.5) * zoom) as i32;
            if src.rect.contains(x.min(b.x1 - 1), y.min(b.y1 - 1)) {
                let px = src.px(x.min(b.x1 - 1), y.min(b.y1 - 1));
                let c = rgba(ctx, &px[..n]);
                img[r * cells + i] = to_hsv([c[0], c[1], c[2]]);
            }
        }
    }
    let channel = |f: CmlFunction, ch: usize, salt: u32| {
        let initial: Vec<f32> = img.iter().map(|c| c[ch]).collect();
        cml_channel(f, cells, rows, spec.k, spec.coupling, spec.start_offset as usize, spec.seed.wrapping_add(salt), &initial)
    };
    let hue = channel(spec.hue, 0, 1);
    let sat = channel(spec.saturation, 1, 2);
    let val = channel(spec.value, 2, 3);
    generate(src, out, ctx, |x, y| {
        let i = (((x - b.x0 as f32) / zoom).floor() as usize).min(cells - 1);
        let r = (((y - b.y0 as f32) / zoom).floor() as usize).min(rows - 1);
        let k = r * cells + i;
        let c = hsv(hue[k], sat[k], val[k]);
        [c[0], c[1], c[2], 1.0]
    })
}
