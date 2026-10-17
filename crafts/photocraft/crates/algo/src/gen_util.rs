// SPDX-License-Identifier: MIT OR Apache-2.0
//! Shared pieces of the generator and map filters: a float random stream, value / simplex
//! noise, colour mixing, and a small anti-aliased line and disc rasteriser that composites
//! straight sRGB colour over a native tile.

use redrob_photo_geom::Rect;

use crate::Ctx;
use crate::fxutil::{rgba, set_rgba, xy};
use crate::image::Image;
use crate::photo_util::Rng;

/// Random stream: [`Rng`] with float helpers.
pub(crate) struct Stream(pub Rng);

impl Stream {
    pub fn new(seed: u32) -> Self {
        Stream(Rng(u64::from(seed) ^ 0x5DEE_CE66_D1CE_4E5B))
    }
    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.0.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
    pub fn below(&mut self, n: usize) -> usize {
        self.0.below(n)
    }
}

/// Integer lattice hash in `[0, 1)`.
#[inline]
pub(crate) fn hash(x: i32, y: i32, seed: u32) -> f32 {
    crate::noise::hash01(x, y, 0, seed)
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Smooth value noise in `[0, 1)`.
pub(crate) fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (sx, sy) = (fade(x - ix as f32), fade(y - iy as f32));
    let h = |a: i32, b: i32| hash(a, b, seed);
    let top = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * sx;
    let bot = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * sx;
    top + (bot - top) * sy
}

/// Fractal (fBm) value noise with `octaves` octaves, in `[0, 1)`.
pub(crate) fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut f) = (0.0, 1.0, 0.0, 1.0);
    for o in 0..octaves.max(1) {
        sum += value_noise(x * f, y * f, seed.wrapping_add(o.wrapping_mul(7919))) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

/// 2-D simplex noise in `[-1, 1]` (skewed triangular lattice, gradient per corner).
pub(crate) fn simplex(x: f32, y: f32, seed: u32) -> f32 {
    const F2: f32 = 0.366_025_4; // (sqrt(3) - 1) / 2
    const G2: f32 = 0.211_324_87; // (3 - sqrt(3)) / 6
    let s = (x + y) * F2;
    let (i, j) = ((x + s).floor(), (y + s).floor());
    let t = (i + j) * G2;
    let (x0, y0) = (x - (i - t), y - (j - t));
    let (i1, j1) = if x0 > y0 { (1.0, 0.0) } else { (0.0, 1.0) };
    let (x1, y1) = (x0 - i1 + G2, y0 - j1 + G2);
    let (x2, y2) = (x0 - 1.0 + 2.0 * G2, y0 - 1.0 + 2.0 * G2);
    let grad = |ci: f32, cj: f32, dx: f32, dy: f32| {
        let h = hash(ci as i32, cj as i32, seed);
        let a = h * std::f32::consts::TAU;
        let (gx, gy) = (a.cos(), a.sin());
        let mut t = 0.5 - dx * dx - dy * dy;
        if t < 0.0 {
            0.0
        } else {
            t *= t;
            t * t * (gx * dx + gy * dy)
        }
    };
    let n = grad(i, j, x0, y0) + grad(i + i1, j + j1, x1, y1) + grad(i + 1.0, j + 1.0, x2, y2);
    (n * 70.0).clamp(-1.0, 1.0)
}

/// Linear mix of two straight RGBA colours.
#[inline]
pub(crate) fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

/// HSV (0–1 each) to straight RGB.
pub(crate) fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Straight RGB to HSV (0–1 each).
pub(crate) fn to_hsv(c: [f32; 3]) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let d = max - min;
    let s = if max > 0.0 { d / max } else { 0.0 };
    let h = if d <= 0.0 {
        0.0
    } else if max == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0) / 6.0
    } else if max == c[1] {
        ((c[2] - c[0]) / d + 2.0) / 6.0
    } else {
        ((c[0] - c[1]) / d + 4.0) / 6.0
    };
    [h, s, max]
}

/// Fills every pixel of `out` with the straight RGBA colour `f(x, y)` returns (document
/// coordinates of the pixel centre), replacing the tile.
pub(crate) fn generate(src: &Image, out: Rect, ctx: &Ctx, f: impl Fn(f32, f32) -> [f32; 4]) -> Vec<f32> {
    let n = src.ch;
    let mut res = vec![0.0f32; out.width() as usize * out.height() as usize * n];
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        set_rgba(ctx, px, f(x as f32 + 0.5, y as f32 + 0.5));
    }
    res
}

/// Composites the straight RGBA colour `f(x, y)` returns over the source pixel (its alpha is the
/// coverage): what the drawing filters use to paint lines over the image.
pub(crate) fn paint_over(src: &Image, out: Rect, ctx: &Ctx, f: impl Fn(f32, f32) -> [f32; 4]) -> Vec<f32> {
    let n = src.ch;
    let mut res = src.crop(out);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let c = f(x as f32 + 0.5, y as f32 + 0.5);
        if c[3] <= 0.0 {
            continue;
        }
        let under = rgba(ctx, px);
        set_rgba(ctx, px, over(c, under));
    }
    res
}

/// Straight-alpha `src` over `dst`.
#[inline]
pub(crate) fn over(src: [f32; 4], dst: [f32; 4]) -> [f32; 4] {
    let a = src[3].clamp(0.0, 1.0);
    let out_a = a + dst[3] * (1.0 - a);
    if out_a <= 0.0 {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let c = |s: f32, d: f32| (s * a + d * dst[3] * (1.0 - a)) / out_a;
    [c(src[0], dst[0]), c(src[1], dst[1]), c(src[2], dst[2]), out_a]
}

/// A stroke segment list that, once finished, answers how much of a pixel a set of
/// anti-aliased lines covers (segments are bucketed on a coarse grid for the lookup).
pub(crate) struct Strokes {
    segments: Vec<([f32; 2], [f32; 2], f32)>,
}

impl Default for Strokes {
    fn default() -> Self {
        Self::new()
    }
}

impl Strokes {
    pub fn new() -> Self {
        Strokes { segments: Vec::new() }
    }
    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32) {
        self.segments.push((a, b, width.max(0.5)));
    }
    pub fn polyline(&mut self, pts: &[[f32; 2]], width: f32, closed: bool) {
        for w in pts.windows(2) {
            self.line(w[0], w[1], width);
        }
        if closed && pts.len() > 2 {
            self.line(pts[pts.len() - 1], pts[0], width);
        }
    }
    /// Buckets the segments for lookups.
    pub fn finish(self) -> StrokeMap {
        const CELL: f32 = 32.0;
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for (a, b, w) in &self.segments {
            let h = w / 2.0 + 1.0;
            min[0] = min[0].min(a[0].min(b[0]) - h);
            min[1] = min[1].min(a[1].min(b[1]) - h);
            max[0] = max[0].max(a[0].max(b[0]) + h);
            max[1] = max[1].max(a[1].max(b[1]) + h);
        }
        if self.segments.is_empty() || !min[0].is_finite() {
            return StrokeMap { segments: self.segments, origin: [0.0, 0.0], cell: CELL, cols: 0, rows: 0, cells: Vec::new() };
        }
        let cols = (((max[0] - min[0]) / CELL).ceil() as usize + 1).clamp(1, 4096);
        let rows = (((max[1] - min[1]) / CELL).ceil() as usize + 1).clamp(1, 4096);
        let mut cells: Vec<Vec<u32>> = vec![Vec::new(); cols * rows];
        for (k, (a, b, w)) in self.segments.iter().enumerate() {
            let h = w / 2.0 + 1.0;
            let c0 = (((a[0].min(b[0]) - h - min[0]) / CELL).floor().max(0.0) as usize).min(cols - 1);
            let c1 = (((a[0].max(b[0]) + h - min[0]) / CELL).floor().max(0.0) as usize).min(cols - 1);
            let r0 = (((a[1].min(b[1]) - h - min[1]) / CELL).floor().max(0.0) as usize).min(rows - 1);
            let r1 = (((a[1].max(b[1]) + h - min[1]) / CELL).floor().max(0.0) as usize).min(rows - 1);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    cells[r * cols + c].push(k as u32);
                }
            }
        }
        StrokeMap { segments: self.segments, origin: min, cell: CELL, cols, rows, cells }
    }
}

/// Finished strokes: coverage lookups.
pub(crate) struct StrokeMap {
    segments: Vec<([f32; 2], [f32; 2], f32)>,
    origin: [f32; 2],
    cell: f32,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

impl StrokeMap {
    /// Coverage in `0..=1` of the pixel centred at `(x, y)` (the union of all segments).
    pub fn coverage(&self, x: f32, y: f32) -> f32 {
        if self.cols == 0 {
            return 0.0;
        }
        let cx = (x - self.origin[0]) / self.cell;
        let cy = (y - self.origin[1]) / self.cell;
        if cx < 0.0 || cy < 0.0 || cx >= self.cols as f32 || cy >= self.rows as f32 {
            return 0.0;
        }
        let mut best = 0.0f32;
        for &k in &self.cells[cy as usize * self.cols + cx as usize] {
            let (a, b, w) = self.segments[k as usize];
            let d = segment_distance(x, y, a, b);
            let c = (w / 2.0 + 0.5 - d).clamp(0.0, 1.0);
            best = best.max(c);
            if best >= 1.0 {
                break;
            }
        }
        best
    }
}

/// Distance from `(x, y)` to the segment `a`–`b`.
pub(crate) fn segment_distance(x: f32, y: f32, a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= 0.0 { 0.0 } else { (((x - a[0]) * dx + (y - a[1]) * dy) / len2).clamp(0.0, 1.0) };
    let (px, py) = (a[0] + dx * t, a[1] + dy * t);
    ((x - px) * (x - px) + (y - py) * (y - py)).sqrt()
}

/// Document-space centre and half extents of the reference bounds.
pub(crate) fn frame(ctx: &Ctx) -> (f32, f32, f32, f32) {
    let b = ctx.bounds;
    let (w, h) = (b.width() as f32, b.height() as f32);
    (b.x0 as f32 + w / 2.0, b.y0 as f32 + h / 2.0, w, h)
}

/// Grey-level ramp through a small gradient (positions 0..1, straight RGB stops).
pub(crate) fn ramp(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    let Some(first) = stops.first() else { return [t, t, t, 1.0] };
    let mut prev = first;
    for s in stops {
        if t <= s.0 {
            let span = (s.0 - prev.0).max(1e-6);
            let k = if s.0 == prev.0 { 1.0 } else { ((t - prev.0) / span).clamp(0.0, 1.0) };
            let c = [prev.1[0] + (s.1[0] - prev.1[0]) * k, prev.1[1] + (s.1[1] - prev.1[1]) * k, prev.1[2] + (s.1[2] - prev.1[2]) * k];
            return [c[0], c[1], c[2], 1.0];
        }
        prev = s;
    }
    [prev.1[0], prev.1[1], prev.1[2], 1.0]
}
