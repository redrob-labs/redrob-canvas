//! Edge and map filters: Difference of Gaussians, Laplace, Image Gradient, Distance Map, Normal
//! Map.

use redrob_photo_geom::Rect;

use super::{gauss_reach, luma_gradient, luma_rgb, map_px, rgba_at, rgba_window, window_px};
use crate::fxutil::gauss_blur_n;
use crate::image::{Edge, Image};
use crate::{Ctx, DistanceMetric, GradientOutput, NormalComponent};

/// Pixels a Difference of Gaussians reads around an output pixel.
pub(crate) fn dog_reach(radius1: f32, radius2: f32) -> f32 {
    gauss_reach(radius1.max(radius2))
}

/// Difference of Gaussians: the blur of radius `radius2` subtracted from that of `radius1`, per
/// channel, which keeps detail between the two scales. Normalize centres and amplifies the
/// difference (0.5 + 4Δ) instead of clipping it at black; invert flips the result.
pub(crate) fn difference_of_gaussians(src: &Image, out: Rect, ctx: &Ctx, radius1: f32, radius2: f32, normalize: bool, invert: bool) -> Vec<f32> {
    let reach = dog_reach(radius1, radius2) as i32;
    let win = out.inflate(reach);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let base = rgba_window(src, ctx, win);
    let mut b1 = base.clone();
    let mut b2 = base;
    gauss_blur_n(&mut b1, ww, wh, 4, radius1.max(0.0));
    gauss_blur_n(&mut b2, ww, wh, 4, radius2.max(0.0));
    map_px(src, out, ctx, |x, y, c| {
        let p = window_px(&b1, win, x, y);
        let q = window_px(&b2, win, x, y);
        let mut o = [0.0f32; 4];
        for k in 0..3 {
            let d = p[k] - q[k];
            let v = if normalize { 0.5 + 4.0 * d } else { d };
            o[k] = if invert { 1.0 - v.clamp(0.0, 1.0) } else { v.clamp(0.0, 1.0) };
        }
        o[3] = c[3];
        o
    })
}

/// Laplace: the magnitude of the 3×3 Laplacian per channel, light edges on black.
pub(crate) fn laplace(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    map_px(src, out, ctx, |x, y, c| {
        let p = |dx: i32, dy: i32| rgba_at(src, ctx, x + dx, y + dy);
        let (l, r, u, d) = (p(-1, 0), p(1, 0), p(0, -1), p(0, 1));
        let mut o = [0.0f32; 4];
        for k in 0..3 {
            o[k] = (4.0 * c[k] - l[k] - r[k] - u[k] - d[k]).abs().min(1.0);
        }
        o[3] = c[3];
        o
    })
}

/// Image Gradient: magnitude of the luma gradient, its direction (0..1 over a full turn), or
/// both (magnitude in red, direction in green).
pub(crate) fn image_gradient(src: &Image, out: Rect, ctx: &Ctx, output: GradientOutput) -> Vec<f32> {
    map_px(src, out, ctx, |x, y, c| {
        let (gx, gy) = luma_gradient(src, ctx, x, y);
        let mag = (gx * gx + gy * gy).sqrt().min(1.0);
        let dir = gy.atan2(gx).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
        match output {
            GradientOutput::Magnitude => [mag, mag, mag, c[3]],
            GradientOutput::Direction => [dir, dir, dir, c[3]],
            GradientOutput::Both => [mag, dir, 0.0, c[3]],
        }
    })
}

/// One-dimensional squared Euclidean distance transform (lower envelope of parabolas).
fn edt_1d(f: &[f32], out: &mut [f32], v: &mut Vec<usize>, z: &mut Vec<f32>) {
    let n = f.len();
    v.clear();
    z.clear();
    v.resize(n, 0);
    z.resize(n + 1, 0.0);
    let mut k = 0usize;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    let sq = |i: usize| (i * i) as f32;
    for q in 1..n {
        let mut s = ((f[q] + sq(q)) - (f[v[k]] + sq(v[k]))) / (2.0 * (q as f32 - v[k] as f32));
        while k > 0 && s <= z[k] {
            k -= 1;
            s = ((f[q] + sq(q)) - (f[v[k]] + sq(v[k]))) / (2.0 * (q as f32 - v[k] as f32));
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f32::INFINITY;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        *o = (q as f32 - v[k] as f32).powi(2) + f[v[k]];
    }
}

/// Distance Map: for every foreground pixel (luma between the thresholds) the distance to the
/// nearest background pixel, by the chosen metric, over the whole bounds. Averaging smooths the
/// map; normalize scales the farthest pixel to white, otherwise 255 px is white.
#[allow(clippy::too_many_arguments)]
pub(crate) fn distance_map(src: &Image, out: Rect, ctx: &Ctx, metric: DistanceMetric, low: f32, high: f32, averaging: f32, normalize: bool) -> Vec<f32> {
    let b = ctx.bounds;
    if b.is_empty() {
        return src.crop(out);
    }
    let (w, h) = (b.width() as usize, b.height() as usize);
    const FAR: f32 = 1.0e12;
    // Foreground = FAR (unknown distance); background = 0.
    let mut dist: Vec<f32> = (0..w * h)
        .map(|i| {
            let (x, y) = (b.x0 + (i % w) as i32, b.y0 + (i / w) as i32);
            let c = rgba_at(src, ctx, x, y);
            let l = luma_rgb(c) * c[3];
            if (low..=high).contains(&l) { FAR } else { 0.0 }
        })
        .collect();
    match metric {
        DistanceMetric::Euclidean => {
            let mut col = vec![0.0f32; h];
            let mut tmp = vec![0.0f32; h.max(w)];
            let (mut v, mut z) = (Vec::new(), Vec::new());
            for x in 0..w {
                for (y, cv) in col.iter_mut().enumerate() {
                    *cv = dist[y * w + x];
                }
                edt_1d(&col, &mut tmp[..h], &mut v, &mut z);
                for (y, tv) in tmp.iter().enumerate().take(h) {
                    dist[y * w + x] = *tv;
                }
            }
            let mut row = vec![0.0f32; w];
            for y in 0..h {
                row.copy_from_slice(&dist[y * w..(y + 1) * w]);
                edt_1d(&row, &mut tmp[..w], &mut v, &mut z);
                for (x, tv) in tmp.iter().enumerate().take(w) {
                    dist[y * w + x] = tv.sqrt();
                }
            }
        }
        DistanceMetric::Manhattan | DistanceMetric::Chebyshev => {
            let diag = metric == DistanceMetric::Chebyshev;
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let mut d = dist[i];
                    if x > 0 {
                        d = d.min(dist[i - 1] + 1.0);
                    }
                    if y > 0 {
                        d = d.min(dist[i - w] + 1.0);
                        if diag {
                            if x > 0 {
                                d = d.min(dist[i - w - 1] + 1.0);
                            }
                            if x + 1 < w {
                                d = d.min(dist[i - w + 1] + 1.0);
                            }
                        }
                    }
                    dist[i] = d;
                }
            }
            for y in (0..h).rev() {
                for x in (0..w).rev() {
                    let i = y * w + x;
                    let mut d = dist[i];
                    if x + 1 < w {
                        d = d.min(dist[i + 1] + 1.0);
                    }
                    if y + 1 < h {
                        d = d.min(dist[i + w] + 1.0);
                        if diag {
                            if x > 0 {
                                d = d.min(dist[i + w - 1] + 1.0);
                            }
                            if x + 1 < w {
                                d = d.min(dist[i + w + 1] + 1.0);
                            }
                        }
                    }
                    dist[i] = d;
                }
            }
        }
    }
    let far_max = (w.max(h) * 2) as f32;
    for d in dist.iter_mut() {
        if *d >= far_max {
            *d = far_max;
        }
    }
    if averaging > 0.0 {
        gauss_blur_n(&mut dist, w, h, 1, (averaging / 100.0).clamp(0.3, 50.0));
    }
    let scale = if normalize { 1.0 / dist.iter().fold(0.0f32, |m, v| m.max(*v)).max(1e-6) } else { 1.0 / 255.0 };
    map_px(src, out, ctx, |x, y, c| {
        let v = if b.contains(x, y) { (dist[(y - b.y0) as usize * w + (x - b.x0) as usize] * scale).clamp(0.0, 1.0) } else { 0.0 };
        [v, v, v, c[3]]
    })
}

/// Normal Map parameters.
pub(crate) struct NormalSpec {
    pub scale: f32,
    pub x_component: NormalComponent,
    pub y_component: NormalComponent,
    pub flip_x: bool,
    pub flip_y: bool,
    pub full_z: bool,
    pub tileable: bool,
}

/// Normal Map: the luma is a height field; its slopes become a unit normal written as colours
/// (X and Y in the chosen channels, Z in the remaining one). `scale` steepens the relief,
/// flips invert an axis, full Z stores the whole 0..1 range of Z instead of 0.5..1, and
/// tileable wraps the height field around the bounds.
pub(crate) fn normal_map(src: &Image, out: Rect, ctx: &Ctx, spec: &NormalSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let height = |x: i32, y: i32| -> f32 {
        let (xx, yy) = if spec.tileable && !b.is_empty() {
            (b.x0 + (x - b.x0).rem_euclid(b.width() as i32), b.y0 + (y - b.y0).rem_euclid(b.height() as i32))
        } else {
            (x, y)
        };
        let mut buf = [0.0f32; crate::fxutil::MAXC];
        let n = src.ch.min(buf.len());
        for (c, v) in buf.iter_mut().enumerate().take(n) {
            *v = src.get_edge(xx, yy, c, Edge::Repeat, src.rect);
        }
        let c = crate::fxutil::rgba(ctx, &buf[..n]);
        luma_rgb(c)
    };
    let k = spec.scale.max(0.0) * 0.5;
    map_px(src, out, ctx, |x, y, c| {
        let mut dx = (height(x + 1, y) - height(x - 1, y)) * k;
        let mut dy = (height(x, y + 1) - height(x, y - 1)) * k;
        if spec.flip_x {
            dx = -dx;
        }
        if spec.flip_y {
            dy = -dy;
        }
        let len = (dx * dx + dy * dy + 1.0).sqrt();
        let (nx, ny, nz) = (-dx / len, -dy / len, 1.0 / len);
        let z = if spec.full_z { nz } else { 0.5 + nz * 0.5 };
        let mut o = [z, z, z, c[3]];
        let slot = |comp: NormalComponent| match comp {
            NormalComponent::Red => 0,
            NormalComponent::Green => 1,
            NormalComponent::Blue => 2,
        };
        let (xs, ys) = (slot(spec.x_component), slot(spec.y_component));
        o[xs] = nx * 0.5 + 0.5;
        if ys != xs {
            o[ys] = ny * 0.5 + 0.5;
        }
        o
    })
}
