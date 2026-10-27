//! Edge-preserving and tileable smoothing: Mean Curvature Blur, Tileable Blur, Antialias,
//! Symmetric Nearest Neighbour, NL Filter.

use redrob_photo_geom::Rect;

use super::{rgba_window, window_px};
use crate::blur::gaussian_kernel;
use crate::fxutil::{MAXC, set_rgba};
use crate::image::{Edge, Image, premultiply, unpremultiply};
use crate::{Ctx, NlMode, TileableBlurType};

/// Native-channel window of `src` over `win` (repeating the source edge), premultiplied.
fn native_window(src: &Image, win: Rect, alpha: bool) -> Vec<f32> {
    let n = src.ch;
    let mut v = Vec::with_capacity(win.width() as usize * win.height() as usize * n);
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            for c in 0..n {
                v.push(src.get_edge(x, y, c, Edge::Repeat, src.rect));
            }
        }
    }
    premultiply(&mut v, n, alpha);
    v
}

/// Copies the `out` part of a `win` buffer (`n` channels) into a fresh result, unpremultiplied.
fn crop_window(buf: &[f32], win: Rect, out: Rect, n: usize, alpha: bool) -> Vec<f32> {
    let ww = win.width() as usize;
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    for y in out.y0..out.y1 {
        let row = (y - win.y0) as usize * ww;
        let a = (row + (out.x0 - win.x0) as usize) * n;
        let b = (row + (out.x1 - win.x0) as usize) * n;
        res.extend_from_slice(&buf[a..b]);
    }
    unpremultiply(&mut res, n, alpha);
    res
}

/// Mean Curvature Blur: `iterations` explicit steps of mean curvature flow
/// `u_t = |∇u| · div(∇u / |∇u|)` on every channel. Flat areas and straight edges are fixed points,
/// so noise and small details vanish while contours stay sharp.
pub(crate) fn mean_curvature(src: &Image, out: Rect, ctx: &Ctx, iterations: u32) -> Vec<f32> {
    let n = src.ch;
    let it = iterations.clamp(1, 500) as i32;
    let win = out.inflate(it + 1);
    let (w, h) = (win.width() as usize, win.height() as usize);
    let mut cur = native_window(src, win, ctx.alpha);
    let mut next = cur.clone();
    let idx = |x: usize, y: usize| (y * w + x) * n;
    for _ in 0..it {
        for y in 0..h {
            let (ym, yp) = (y.saturating_sub(1), (y + 1).min(h - 1));
            for x in 0..w {
                let (xm, xp) = (x.saturating_sub(1), (x + 1).min(w - 1));
                for c in 0..n {
                    let u = cur[idx(x, y) + c];
                    let (ul, ur) = (cur[idx(xm, y) + c], cur[idx(xp, y) + c]);
                    let (uu, ud) = (cur[idx(x, ym) + c], cur[idx(x, yp) + c]);
                    let ux = (ur - ul) * 0.5;
                    let uy = (ud - uu) * 0.5;
                    let uxx = ur - 2.0 * u + ul;
                    let uyy = ud - 2.0 * u + uu;
                    let uxy = (cur[idx(xp, yp) + c] - cur[idx(xm, yp) + c] - cur[idx(xp, ym) + c] + cur[idx(xm, ym) + c]) * 0.25;
                    let g2 = ux * ux + uy * uy;
                    let k = if g2 > 1e-10 { (uy * uy * uxx - 2.0 * ux * uy * uxy + ux * ux * uyy) / g2 } else { 0.5 * (uxx + uyy) };
                    next[idx(x, y) + c] = (u + 0.25 * k).clamp(0.0, 1.0);
                }
            }
        }
        std::mem::swap(&mut cur, &mut next);
    }
    crop_window(&cur, win, out, n, ctx.alpha)
}

/// Tileable Blur: a Gaussian (or box) blur that wraps around the reference bounds, so the result
/// tiles seamlessly. Either axis can be switched off.
#[allow(clippy::needless_range_loop)]
pub(crate) fn tileable(src: &Image, out: Rect, ctx: &Ctx, radius: f32, horizontal: bool, vertical: bool, kind: TileableBlurType) -> Vec<f32> {
    let n = src.ch;
    let b = ctx.bounds;
    if b.is_empty() || radius <= 0.0 || (!horizontal && !vertical) {
        return src.crop(out);
    }
    let kernel: Vec<f32> = match kind {
        TileableBlurType::Gaussian => gaussian_kernel(radius),
        TileableBlurType::Box => {
            let r = radius.ceil() as usize;
            vec![1.0 / (2 * r + 1) as f32; 2 * r + 1]
        }
    };
    let r = (kernel.len() / 2) as i32;
    let (bw, bh) = (b.width() as i32, b.height() as i32);
    let wrap_x = |x: i32| b.x0 + (x - b.x0).rem_euclid(bw);
    let wrap_y = |y: i32| b.y0 + (y - b.y0).rem_euclid(bh);
    // Horizontal pass over the rows the vertical pass will read, full bounds width (wrapped).
    let ry = if vertical { r } else { 0 };
    let rows = Rect::new(b.x0, out.y0 - ry, b.x1, out.y1 + ry);
    let rw = rows.width() as usize;
    let mut tmp = vec![0.0f32; rw * rows.height() as usize * n];
    let mut px = [0.0f32; MAXC];
    for y in rows.y0..rows.y1 {
        let sy = wrap_y(y);
        for x in rows.x0..rows.x1 {
            let o = ((y - rows.y0) as usize * rw + (x - rows.x0) as usize) * n;
            if horizontal {
                px[..n].fill(0.0);
                for (k, wgt) in kernel.iter().enumerate() {
                    let sx = wrap_x(x + k as i32 - r);
                    let a = if ctx.alpha { src.get_edge(sx, sy, n - 1, Edge::Repeat, src.rect) } else { 1.0 };
                    for c in 0..n {
                        let s = src.get_edge(sx, sy, c, Edge::Repeat, src.rect);
                        px[c] += wgt * if ctx.alpha && c < n - 1 { s * a } else { s };
                    }
                }
            } else {
                let a = if ctx.alpha { src.get_edge(x, sy, n - 1, Edge::Repeat, src.rect) } else { 1.0 };
                for c in 0..n {
                    let s = src.get_edge(x, sy, c, Edge::Repeat, src.rect);
                    px[c] = if ctx.alpha && c < n - 1 { s * a } else { s };
                }
            }
            tmp[o..o + n].copy_from_slice(&px[..n]);
        }
    }
    // Vertical pass for the output columns.
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let xi = (wrap_x(x) - rows.x0) as usize;
            px[..n].fill(0.0);
            if vertical {
                for (k, wgt) in kernel.iter().enumerate() {
                    let yy = y + k as i32 - r;
                    // Rows outside the computed strip only happen when the bounds are shorter
                    // than the kernel: wrap them back into the strip.
                    let yi =
                        if (rows.y0..rows.y1).contains(&yy) { (yy - rows.y0) as usize } else { (wrap_y(yy).clamp(rows.y0, rows.y1 - 1) - rows.y0) as usize };
                    let o = (yi * rw + xi) * n;
                    for (c, v) in px.iter_mut().enumerate().take(n) {
                        *v += wgt * tmp[o + c];
                    }
                }
            } else {
                let o = ((y - rows.y0) as usize * rw + xi) * n;
                px[..n].copy_from_slice(&tmp[o..o + n]);
            }
            unpremul(&mut px[..n], ctx.alpha);
            res.extend_from_slice(&px[..n]);
        }
    }
    res
}

#[inline]
fn unpremul(px: &mut [f32], alpha: bool) {
    if !alpha {
        return;
    }
    let n = px.len();
    let a = px[n - 1];
    for v in px.iter_mut().take(n - 1) {
        *v = if a > 1e-7 { *v / a } else { 0.0 };
    }
}

/// Antialias: a Scale3X-style edge smoother. Each pixel is expanded to the nine sub-pixels the
/// scaler would produce from its 3×3 neighbourhood (a sub-pixel takes a neighbour's colour where
/// two neighbours agree against the opposite side), and the pixel becomes their average, which
/// rounds staircase edges without blurring flat areas.
pub(crate) fn antialias(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    let n = src.ch;
    let win = out.inflate(1);
    let buf = rgba_window(src, ctx, win);
    let same = |a: [f32; 4], b: [f32; 4]| -> bool {
        let d = (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs() + (a[3] - b[3]).abs();
        d < 0.12
    };
    let mut res = src.crop(out);
    let w = out.width() as usize;
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32);
        let p = |dx: i32, dy: i32| window_px(&buf, win, x + dx, y + dy);
        let (a, b, c) = (p(-1, -1), p(0, -1), p(1, -1));
        let (d, e, f) = (p(-1, 0), p(0, 0), p(1, 0));
        let (g, h, k) = (p(-1, 1), p(0, 1), p(1, 1));
        if same(b, h) || same(d, f) {
            continue;
        }
        let pick = |cond: bool, v: [f32; 4]| if cond { v } else { e };
        let subs = [
            pick(same(d, b), d),
            pick((same(d, b) && !same(e, c)) || (same(b, f) && !same(e, a)), b),
            pick(same(b, f), f),
            pick((same(d, b) && !same(e, g)) || (same(d, h) && !same(e, a)), d),
            e,
            pick((same(b, f) && !same(e, k)) || (same(h, f) && !same(e, c)), f),
            pick(same(d, h), d),
            pick((same(d, h) && !same(e, k)) || (same(h, f) && !same(e, g)), h),
            pick(same(h, f), f),
        ];
        let mut acc = [0.0f32; 4];
        for s in subs {
            acc[0] += s[0] * s[3];
            acc[1] += s[1] * s[3];
            acc[2] += s[2] * s[3];
            acc[3] += s[3];
        }
        let o = if acc[3] > 1e-7 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3] / 9.0] } else { [0.0; 4] };
        set_rgba(ctx, px, o);
    }
    res
}

/// Symmetric Nearest Neighbour: every pixel is the mean of, for each symmetric pair of
/// neighbours within `radius`, the member closest in colour to the centre (`pairs` = 2 also
/// considers the two mirrored pairs). Edges stay put because the far side of an edge always
/// loses the comparison.
pub(crate) fn snn(src: &Image, out: Rect, ctx: &Ctx, radius: f32, pairs: u32) -> Vec<f32> {
    let n = src.ch;
    let r = radius.clamp(1.0, 100.0).round() as i32;
    let win = out.inflate(r);
    let buf = rgba_window(src, ctx, win);
    let mut res = src.crop(out);
    let w = out.width() as usize;
    let dist = |a: [f32; 4], b: [f32; 4]| -> f32 { (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2) + (a[3] - b[3]).powi(2) };
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32);
        let centre = window_px(&buf, win, x, y);
        let mut acc = [0.0f32; 4];
        let mut cnt: f32 = 0.0;
        let mut push = |c: [f32; 4]| {
            acc[0] += c[0] * c[3];
            acc[1] += c[1] * c[3];
            acc[2] += c[2] * c[3];
            acc[3] += c[3];
            cnt += 1.0;
        };
        let pick = |cands: &[[f32; 4]]| -> [f32; 4] {
            let mut best = cands[0];
            let mut bd = dist(best, centre);
            for &c in &cands[1..] {
                let d = dist(c, centre);
                if d < bd {
                    bd = d;
                    best = c;
                }
            }
            best
        };
        // Half plane of offsets (dy < 0, or dy == 0 and dx < 0) and their mirrors.
        for dy in -r..=0 {
            for dx in -r..=r {
                if dy == 0 && dx >= 0 {
                    continue;
                }
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let p = window_px(&buf, win, x + dx, y + dy);
                let q = window_px(&buf, win, x - dx, y - dy);
                if pairs >= 2 {
                    let p2 = window_px(&buf, win, x + dx, y - dy);
                    let q2 = window_px(&buf, win, x - dx, y + dy);
                    push(pick(&[p, q, p2, q2]));
                } else {
                    push(pick(&[p, q]));
                }
            }
        }
        push(centre);
        let o = if acc[3] > 1e-7 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3] / cnt.max(1.0)] } else { [0.0; 4] };
        set_rgba(ctx, px, o);
    }
    res
}

/// NL Filter over the 3×3 neighbourhood (eight neighbours weighted by `radius`, 0 = the pixel
/// alone, 1 = equal weight). Alpha trimmed mean drops the `alpha` fraction of the sorted samples
/// at each end before averaging (0 = mean, 0.5 = median); optimal estimation shrinks the pixel
/// toward the local mean when the local variance is below the noise level `alpha`; edge
/// enhancement pushes it away from the mean by `alpha`.
pub(crate) fn nl_filter(src: &Image, out: Rect, ctx: &Ctx, alpha: f32, radius: f32, mode: NlMode) -> Vec<f32> {
    let n = src.ch;
    let a = alpha.clamp(0.0, 1.0);
    let rw = radius.clamp(0.0, 1.0);
    let win = out.inflate(1);
    let buf = rgba_window(src, ctx, win);
    let mut res = src.crop(out);
    let w = out.width() as usize;
    let mut samples: Vec<(f32, f32)> = Vec::with_capacity(9);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32);
        let centre = window_px(&buf, win, x, y);
        let mut o = centre;
        for c in 0..3 {
            samples.clear();
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let wgt = if dx == 0 && dy == 0 { 1.0 } else { rw };
                    if wgt <= 0.0 {
                        continue;
                    }
                    let p = window_px(&buf, win, x + dx, y + dy);
                    samples.push((p[c], wgt));
                }
            }
            let total: f32 = samples.iter().map(|s| s.1).sum();
            let mean = samples.iter().map(|s| s.0 * s.1).sum::<f32>() / total.max(1e-6);
            o[c] = match mode {
                NlMode::AlphaTrimmedMean => {
                    samples.sort_by(|p, q| p.0.total_cmp(&q.0));
                    let trim = total * a.min(0.5);
                    let (mut lo, mut hi) = (trim, total - trim);
                    let mut acc = 0.0;
                    let mut wsum = 0.0;
                    for &(v, wgt) in &samples {
                        let start = lo.max(0.0);
                        let end = (wgt).min(hi.max(0.0));
                        let take = (end - start).max(0.0);
                        acc += v * take;
                        wsum += take;
                        lo -= wgt;
                        hi -= wgt;
                    }
                    if wsum > 1e-6 { acc / wsum } else { mean }
                }
                NlMode::OptimalEstimation => {
                    let var = samples.iter().map(|s| s.1 * (s.0 - mean).powi(2)).sum::<f32>() / total.max(1e-6);
                    let noise = (a * 0.25).powi(2);
                    let gain = if var > 1e-9 { ((var - noise) / var).max(0.0) } else { 0.0 };
                    mean + gain * (centre[c] - mean)
                }
                NlMode::EdgeEnhancement => centre[c] + a * 2.0 * (centre[c] - mean),
            };
        }
        set_rgba(ctx, px, [o[0].clamp(0.0, 1.0), o[1].clamp(0.0, 1.0), o[2].clamp(0.0, 1.0), o[3]]);
    }
    res
}
