//! Artistic renders: Cubism, SLIC, Waterpixels, Impressionist, Van Gogh (LIC), Weave.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use redrob_photo_color::convert::{lab_to_srgb, srgb_to_lab};
use redrob_photo_geom::Rect;

use super::{fbm, luma_gradient, luma_rgb, map_px, over, rgb_to_hsv, rgba_at, rgba_window, window_px};
use crate::fxutil::gauss_blur_n;
use crate::image::Image;
use crate::noise::hash01;
use crate::{Ctx, LicChannel, LicConvolve, LicOperator, PaperTexture, StrokeBackground, StrokeColor, StrokeOrientation, StrokePlacement, StrokeShape, SuperpixelColor};

/// Pixels Cubism reads around an output pixel.
pub(crate) fn cubism_reach(tile_size: f32, saturation: f32) -> f32 {
    tile_size.clamp(1.0, 256.0) * (0.75 * saturation.clamp(0.0, 10.0) + 1.0) + 2.0
}

/// Cubism: one translucent square per grid cell of `tile_size`, randomly rotated and jittered,
/// coloured by the pixel under its centre, painted in cell order over the background colour.
/// `saturation` scales the squares (above 1 they overlap).
pub(crate) fn cubism(src: &Image, out: Rect, ctx: &Ctx, tile_size: f32, saturation: f32, background: [f32; 4], seed: u32) -> Vec<f32> {
    let t = tile_size.clamp(1.0, 256.0);
    let s = saturation.clamp(0.0, 10.0);
    let b = ctx.bounds;
    let reach = cubism_reach(t, s);
    let cells = (reach / t).ceil() as i32 + 1;
    map_px(src, out, ctx, |x, y, _| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let (cx, cy) = (((fx - b.x0 as f32) / t).floor() as i32, ((fy - b.y0 as f32) / t).floor() as i32);
        let mut acc = [background[0], background[1], background[2], 1.0];
        for j in -cells..=cells {
            for i in -cells..=cells {
                let (gx, gy) = (cx + i, cy + j);
                let centre = (
                    b.x0 as f32 + (gx as f32 + 0.2 + 0.6 * hash01(gx, gy, 41, seed)) * t,
                    b.y0 as f32 + (gy as f32 + 0.2 + 0.6 * hash01(gx, gy, 42, seed)) * t,
                );
                let half = t * 0.5 * (0.6 + 0.8 * hash01(gx, gy, 43, seed)) * (0.5 + 0.75 * s);
                let ang = hash01(gx, gy, 44, seed) * std::f32::consts::PI;
                let (sn, cs) = ang.sin_cos();
                let (dx, dy) = (fx - centre.0, fy - centre.1);
                let (u, v) = (dx * cs + dy * sn, -dx * sn + dy * cs);
                if u.abs() <= half && v.abs() <= half {
                    let c = rgba_at(src, ctx, centre.0.floor() as i32, centre.1.floor() as i32);
                    // Soft edge of one pixel and a random translucency.
                    let edge = ((half - u.abs()).min(half - v.abs())).clamp(0.0, 1.0);
                    let a = c[3] * (0.55 + 0.4 * hash01(gx, gy, 45, seed)) * edge;
                    acc = over([c[0], c[1], c[2], a], acc);
                }
            }
        }
        acc
    })
}

/// Pixels SLIC reads around an output pixel (the clusters that can touch it).
pub(crate) fn slic_reach(cluster_size: f32) -> f32 {
    cluster_size.clamp(2.0, 1024.0) * 2.0 + 2.0
}

/// SLIC superpixels: k-means in (L, a, b, x, y) from a grid of seeds `cluster_size` apart, with
/// `compactness` weighting position against colour, for `iterations` rounds. Every pixel takes
/// its cluster's mean colour. Clusters are computed over the tile's neighbourhood.
pub(crate) fn slic(src: &Image, out: Rect, ctx: &Ctx, cluster_size: f32, compactness: f32, iterations: u32) -> Vec<f32> {
    let s = cluster_size.clamp(2.0, 1024.0);
    let b = ctx.bounds;
    let win = out.inflate(slic_reach(s) as i32).intersect(&b.union(&out));
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    if ww == 0 || wh == 0 {
        return src.crop(out);
    }
    let labs: Vec<[f32; 3]> = (0..ww * wh)
        .map(|i| {
            let c = rgba_at(src, ctx, win.x0 + (i % ww) as i32, win.y0 + (i / ww) as i32);
            srgb_to_lab([c[0], c[1], c[2]])
        })
        .collect();
    // Seeds on the bounds-anchored grid that fall in the window.
    let gx0 = ((win.x0 - b.x0) as f32 / s).floor() as i32 - 1;
    let gx1 = ((win.x1 - b.x0) as f32 / s).ceil() as i32 + 1;
    let gy0 = ((win.y0 - b.y0) as f32 / s).floor() as i32 - 1;
    let gy1 = ((win.y1 - b.y0) as f32 / s).ceil() as i32 + 1;
    let mut centres: Vec<[f32; 5]> = Vec::new();
    for gy in gy0..gy1 {
        for gx in gx0..gx1 {
            let (px, py) = (b.x0 as f32 + (gx as f32 + 0.5) * s, b.y0 as f32 + (gy as f32 + 0.5) * s);
            let (ix, iy) = ((px.floor() as i32).clamp(win.x0, win.x1 - 1), (py.floor() as i32).clamp(win.y0, win.y1 - 1));
            let l = labs[(iy - win.y0) as usize * ww + (ix - win.x0) as usize];
            centres.push([l[0], l[1], l[2], px, py]);
        }
    }
    if centres.is_empty() {
        return src.crop(out);
    }
    let m = compactness.clamp(1.0, 40.0);
    let m2 = (m / s) * (m / s);
    let mut label = vec![0usize; ww * wh];
    for _ in 0..iterations.clamp(1, 30) {
        // Assign: each pixel to the nearest centre among those within 2S.
        for (i, lab) in labs.iter().enumerate() {
            let (px, py) = (win.x0 as f32 + (i % ww) as f32 + 0.5, win.y0 as f32 + (i / ww) as f32 + 0.5);
            let mut best = (f32::MAX, 0usize);
            for (k, c) in centres.iter().enumerate() {
                let (dx, dy) = (px - c[3], py - c[4]);
                if dx.abs() > 2.0 * s || dy.abs() > 2.0 * s {
                    continue;
                }
                let dc = (lab[0] - c[0]).powi(2) + (lab[1] - c[1]).powi(2) + (lab[2] - c[2]).powi(2);
                let d = dc + (dx * dx + dy * dy) * m2;
                if d < best.0 {
                    best = (d, k);
                }
            }
            label[i] = best.1;
        }
        // Update centres.
        let mut sums = vec![[0.0f32; 6]; centres.len()];
        for (i, &k) in label.iter().enumerate() {
            let (px, py) = (win.x0 as f32 + (i % ww) as f32 + 0.5, win.y0 as f32 + (i / ww) as f32 + 0.5);
            let l = labs[i];
            let sacc = &mut sums[k];
            sacc[0] += l[0];
            sacc[1] += l[1];
            sacc[2] += l[2];
            sacc[3] += px;
            sacc[4] += py;
            sacc[5] += 1.0;
        }
        for (c, sacc) in centres.iter_mut().zip(&sums) {
            if sacc[5] > 0.0 {
                for j in 0..5 {
                    c[j] = sacc[j] / sacc[5];
                }
            }
        }
    }
    // Mean colour per cluster (recomputed after the last assignment).
    let mut sums = vec![[0.0f32; 4]; centres.len()];
    for (i, &k) in label.iter().enumerate() {
        let l = labs[i];
        sums[k][0] += l[0];
        sums[k][1] += l[1];
        sums[k][2] += l[2];
        sums[k][3] += 1.0;
    }
    let colours: Vec<[f32; 3]> = sums.iter().map(|s| if s[3] > 0.0 { lab_to_srgb([s[0] / s[3], s[1] / s[3], s[2] / s[3]]) } else { [0.0; 3] }).collect();
    map_px(src, out, ctx, |x, y, c| {
        let k = label[(y - win.y0) as usize * ww + (x - win.x0) as usize];
        let col = colours[k];
        [col[0], col[1], col[2], c[3]]
    })
}

/// Pixels Waterpixels reads around an output pixel.
pub(crate) fn waterpixels_reach(size: f32) -> f32 {
    size.clamp(8.0, 256.0) * 2.0 + 2.0
}

#[derive(PartialEq)]
struct Flood {
    cost: f32,
    index: usize,
    label: usize,
}
impl Eq for Flood {}
impl PartialOrd for Flood {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Flood {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap on cost.
        other.cost.total_cmp(&self.cost).then_with(|| other.index.cmp(&self.index))
    }
}

/// Waterpixels: a watershed of the smoothed gradient magnitude, flooded from one seed per grid
/// cell of `size`; `regularization` adds distance to the seed so regions stay compact. Regions
/// are filled with their mean colour or a random one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn waterpixels(src: &Image, out: Rect, ctx: &Ctx, size: f32, smoothness: f32, regularization: f32, color: SuperpixelColor, seed: u32) -> Vec<f32> {
    let s = size.clamp(8.0, 256.0);
    let b = ctx.bounds;
    let win = out.inflate(waterpixels_reach(s) as i32).intersect(&b.union(&out));
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    if ww == 0 || wh == 0 {
        return src.crop(out);
    }
    let mut grad: Vec<f32> = (0..ww * wh)
        .map(|i| {
            let (gx, gy) = luma_gradient(src, ctx, win.x0 + (i % ww) as i32, win.y0 + (i / ww) as i32);
            (gx * gx + gy * gy).sqrt()
        })
        .collect();
    if smoothness > 0.0 {
        gauss_blur_n(&mut grad, ww, wh, 1, (smoothness / 10.0).clamp(0.3, 50.0));
    }
    // Seeds: the lowest-gradient pixel near each grid cell centre.
    let gx0 = ((win.x0 - b.x0) as f32 / s).floor() as i32;
    let gx1 = ((win.x1 - b.x0) as f32 / s).ceil() as i32;
    let gy0 = ((win.y0 - b.y0) as f32 / s).floor() as i32;
    let gy1 = ((win.y1 - b.y0) as f32 / s).ceil() as i32;
    let mut seeds: Vec<(usize, (f32, f32), (i32, i32))> = Vec::new();
    for gy in gy0..gy1 {
        for gx in gx0..gx1 {
            let (cx, cy) = (b.x0 as f32 + (gx as f32 + 0.5) * s, b.y0 as f32 + (gy as f32 + 0.5) * s);
            let r = (s * 0.2) as i32;
            let mut best = (f32::MAX, None);
            for dy in -r..=r {
                for dx in -r..=r {
                    let (x, y) = (cx.floor() as i32 + dx, cy.floor() as i32 + dy);
                    if !win.contains(x, y) {
                        continue;
                    }
                    let i = (y - win.y0) as usize * ww + (x - win.x0) as usize;
                    if grad[i] < best.0 {
                        best = (grad[i], Some(i));
                    }
                }
            }
            if let Some(i) = best.1 {
                seeds.push((i, (cx, cy), (gx, gy)));
            }
        }
    }
    if seeds.is_empty() {
        return src.crop(out);
    }
    let reg = regularization.clamp(0.0, 50.0) / s;
    let mut label = vec![usize::MAX; ww * wh];
    let mut heap = BinaryHeap::new();
    for (k, (i, _, _)) in seeds.iter().enumerate() {
        heap.push(Flood { cost: 0.0, index: *i, label: k });
    }
    while let Some(Flood { cost, index, label: k }) = heap.pop() {
        if label[index] != usize::MAX {
            continue;
        }
        label[index] = k;
        let (x, y) = ((index % ww) as i32, (index / ww) as i32);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if !(0..ww as i32).contains(&nx) || !(0..wh as i32).contains(&ny) {
                continue;
            }
            let j = ny as usize * ww + nx as usize;
            if label[j] != usize::MAX {
                continue;
            }
            let (cx, cy) = seeds[k].1;
            let d = ((win.x0 + nx) as f32 + 0.5 - cx).hypot((win.y0 + ny) as f32 + 0.5 - cy);
            let step = grad[j] + reg * d * 0.05;
            heap.push(Flood { cost: cost.max(step), index: j, label: k });
        }
    }
    let mut sums = vec![[0.0f32; 4]; seeds.len()];
    for (i, &k) in label.iter().enumerate() {
        if k == usize::MAX {
            continue;
        }
        let c = rgba_at(src, ctx, win.x0 + (i % ww) as i32, win.y0 + (i / ww) as i32);
        sums[k][0] += c[0];
        sums[k][1] += c[1];
        sums[k][2] += c[2];
        sums[k][3] += 1.0;
    }
    let colours: Vec<[f32; 3]> = seeds
        .iter()
        .enumerate()
        .map(|(k, (_, _, (gx, gy)))| match color {
            SuperpixelColor::Average => {
                let s = sums[k];
                if s[3] > 0.0 { [s[0] / s[3], s[1] / s[3], s[2] / s[3]] } else { [0.0; 3] }
            }
            SuperpixelColor::Random => [hash01(*gx, *gy, 51, seed), hash01(*gx, *gy, 52, seed), hash01(*gx, *gy, 53, seed)],
        })
        .collect();
    map_px(src, out, ctx, |x, y, c| {
        let k = label[(y - win.y0) as usize * ww + (x - win.x0) as usize];
        if k == usize::MAX {
            return c;
        }
        let col = colours[k];
        [col[0], col[1], col[2], c[3]]
    })
}

/// Impressionist parameters.
pub(crate) struct ImpressionistSpec<'a> {
    pub brush_size: f32,
    pub size_variation: f32,
    pub orientation: f32,
    pub orientation_type: StrokeOrientation,
    pub density: f32,
    pub shape: StrokeShape,
    pub paper_relief: f32,
    pub placement: StrokePlacement,
    pub color: StrokeColor,
    pub background: StrokeBackground,
    pub background_color: [f32; 4],
    pub seed: u32,
    /// Paper texture under the relief (and its scale in pixels per weave).
    pub paper: PaperTexture,
    pub paper_scale: f32,
    /// Greyscale paper image for `PaperTexture::Image`, tiled over the bounds.
    pub paper_map: Option<&'a Image>,
    /// Greyscale brush image for `StrokeShape::Image`: bright pixels are the stroke.
    pub brush_map: Option<&'a Image>,
}

/// Luma of a greyscale/colour image at integer pixel `(x, y)`, wrapping (tiled).
fn tiled_luma(m: &Image, x: i32, y: i32) -> f32 {
    let r = m.rect;
    let (w, h) = (r.width().max(1) as i32, r.height().max(1) as i32);
    let (px, py) = (r.x0 + x.rem_euclid(w), r.y0 + y.rem_euclid(h));
    let l = if m.ch >= 3 { 0.299 * m.get(px, py, 0) + 0.587 * m.get(px, py, 1) + 0.114 * m.get(px, py, 2) } else { m.get(px, py, 0) };
    let a = if m.ch == 2 || m.ch == 4 { m.get(px, py, m.ch - 1) } else { 1.0 };
    l * a
}

/// Paper height (-0.5..0.5) at `(x, y)` for the chosen texture.
fn paper_height(spec: &ImpressionistSpec, x: f32, y: f32) -> f32 {
    let sc = spec.paper_scale.clamp(1.0, 200.0);
    match spec.paper {
        PaperTexture::Plain => fbm(x / 3.0, y / 3.0, spec.seed ^ 0x9a5f) - 0.5,
        PaperTexture::Canvas => {
            let (a, b) = ((x / sc * std::f32::consts::TAU).sin(), (y / sc * std::f32::consts::TAU).sin());
            // Two crossed threads: the one on top at this cell is the higher of the two.
            let over_x = ((x / sc).floor() as i32 + (y / sc).floor() as i32).rem_euclid(2) == 0;
            (if over_x { a.abs() * 0.7 + b.abs() * 0.3 } else { b.abs() * 0.7 + a.abs() * 0.3 }) - 0.5
        }
        PaperTexture::Burlap => {
            let s2 = sc * 2.2;
            let (a, b) = ((x / s2 * std::f32::consts::TAU).sin(), (y / s2 * std::f32::consts::TAU).sin());
            (a.abs().max(b.abs()) * 0.8 + fbm(x / 2.0, y / 2.0, spec.seed ^ 0x77) * 0.2) - 0.5
        }
        PaperTexture::Marble => {
            let v = fbm(x / (sc * 4.0), y / (sc * 4.0), spec.seed ^ 0x31);
            // Ridged: veins where the noise crosses the middle.
            0.5 - (v - 0.5).abs() * 2.0 - 0.25
        }
        PaperTexture::Rough => fbm(x / (sc * 1.5), y / (sc * 1.5), spec.seed ^ 0x9a5f) * 0.6 + fbm(x / 2.0, y / 2.0, spec.seed ^ 0x13) * 0.4 - 0.5,
        PaperTexture::Image => spec.paper_map.map_or(0.0, |m| tiled_luma(m, x.floor() as i32, y.floor() as i32) - 0.5),
    }
}

/// Pixels Impressionist reads around an output pixel.
pub(crate) fn impressionist_reach(brush_size: f32, size_variation: f32) -> f32 {
    brush_size.clamp(1.0, 200.0) * (1.0 + size_variation.clamp(0.0, 1.0)) * 1.2 + 3.0
}

/// Impressionist: paint strokes of the chosen shape, sized around `brush_size`, oriented by the
/// orientation map (value, distance or direction to the centre, random, flowing noise, hue, or
/// adaptive along edges), coloured from the image, laid over the original, a solid colour or
/// transparency, with an optional paper relief.
pub(crate) fn impressionist(src: &Image, out: Rect, ctx: &Ctx, spec: &ImpressionistSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let size = spec.brush_size.clamp(1.0, 200.0);
    let var = spec.size_variation.clamp(0.0, 1.0);
    let cell = (size * 0.8).max(1.0);
    let per_cell = (spec.density.clamp(1.0, 50.0) / 10.0).ceil() as i32;
    let reach = impressionist_reach(size, var);
    let cells = (reach / cell).ceil() as i32 + 1;
    let (bcx, bcy) = (b.x0 as f32 + b.width() as f32 / 2.0, b.y0 as f32 + b.height() as f32 / 2.0);
    let bdiag = (b.width() as f32).hypot(b.height() as f32).max(1.0);
    let relief = spec.paper_relief.clamp(0.0, 100.0) / 100.0;
    // A stroke: centre, half-length along, half-width across, angle, colour.
    struct Stroke {
        c: (f32, f32),
        len: f32,
        wid: f32,
        ang: f32,
        col: [f32; 4],
        rough: u32,
    }
    let make = |gx: i32, gy: i32, k: i32| -> Stroke {
        let h = |ch: u32| hash01(gx, gy, 60 + k as u32 * 8 + ch, spec.seed);
        let (jx, jy) = match spec.placement {
            StrokePlacement::Evenly => (0.5 + (h(0) - 0.5) * 0.3, 0.5 + (h(1) - 0.5) * 0.3),
            StrokePlacement::Randomly => (h(0), h(1)),
        };
        let c = (b.x0 as f32 + (gx as f32 + jx) * cell, b.y0 as f32 + (gy as f32 + jy) * cell);
        let (ix, iy) = (c.0.floor() as i32, c.1.floor() as i32);
        let under = rgba_at(src, ctx, ix, iy);
        let s = size * (1.0 + (h(2) - 0.5) * 2.0 * var);
        let ang = match spec.orientation_type {
            StrokeOrientation::Value => spec.orientation.to_radians() + luma_rgb(under) * std::f32::consts::PI,
            StrokeOrientation::Radius => spec.orientation.to_radians() + ((c.0 - bcx).hypot(c.1 - bcy) / bdiag) * std::f32::consts::TAU,
            StrokeOrientation::Random => h(3) * std::f32::consts::PI,
            StrokeOrientation::Radial => (c.1 - bcy).atan2(c.0 - bcx) + spec.orientation.to_radians(),
            StrokeOrientation::Flowing => spec.orientation.to_radians() + fbm(c.0 / (size * 6.0), c.1 / (size * 6.0), spec.seed) * std::f32::consts::TAU,
            StrokeOrientation::Hue => spec.orientation.to_radians() + rgb_to_hsv([under[0], under[1], under[2]])[0] * std::f32::consts::PI,
            StrokeOrientation::Adaptive => {
                let (gx, gy) = luma_gradient(src, ctx, ix, iy);
                if gx.abs() + gy.abs() < 1e-3 { spec.orientation.to_radians() } else { gy.atan2(gx) + std::f32::consts::FRAC_PI_2 }
            }
        };
        let col = match spec.color {
            StrokeColor::Center => under,
            StrokeColor::Average => {
                let (sn, cs) = ang.sin_cos();
                let mut acc = [0.0f32; 4];
                for t in [-0.6f32, -0.2, 0.2, 0.6] {
                    let p = rgba_at(src, ctx, (c.0 + cs * t * s).floor() as i32, (c.1 + sn * t * s).floor() as i32);
                    for i in 0..4 {
                        acc[i] += p[i] * 0.25;
                    }
                }
                acc
            }
        };
        let (len, wid) = match spec.shape {
            StrokeShape::Ellipse => (s * 0.5, s * 0.2),
            StrokeShape::Square => (s * 0.4, s * 0.4),
            StrokeShape::Line => (s * 0.6, (s * 0.06).max(0.6)),
            StrokeShape::Splotch => (s * 0.45, s * 0.3),
            StrokeShape::Circle | StrokeShape::Star | StrokeShape::Diamond => (s * 0.4, s * 0.4),
            StrokeShape::Triangle => (s * 0.45, s * 0.4),
            StrokeShape::Image => (s * 0.5, s * 0.5),
        };
        Stroke { c, len, wid, ang, col, rough: (h(4) * 1000.0) as u32 }
    };
    map_px(src, out, ctx, |x, y, me| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut acc = match spec.background {
            StrokeBackground::KeepOriginal => me,
            StrokeBackground::SolidColor => [spec.background_color[0], spec.background_color[1], spec.background_color[2], 1.0],
            StrokeBackground::Transparent => [0.0, 0.0, 0.0, 0.0],
        };
        let (cx, cy) = (((fx - b.x0 as f32) / cell).floor() as i32, ((fy - b.y0 as f32) / cell).floor() as i32);
        for j in -cells..=cells {
            for i in -cells..=cells {
                for k in 0..per_cell {
                    let st = make(cx + i, cy + j, k);
                    let (dx, dy) = (fx - st.c.0, fy - st.c.1);
                    if dx.abs() > reach || dy.abs() > reach {
                        continue;
                    }
                    let (sn, cs) = st.ang.sin_cos();
                    let (u, v) = (dx * cs + dy * sn, -dx * sn + dy * cs);
                    let inside = match spec.shape {
                        StrokeShape::Ellipse | StrokeShape::Line | StrokeShape::Circle => (u / st.len).powi(2) + (v / st.wid).powi(2) <= 1.0,
                        StrokeShape::Square => u.abs() <= st.len && v.abs() <= st.wid,
                        StrokeShape::Splotch => {
                            let wob = 0.75 + 0.5 * fbm(u / st.len * 2.0 + st.rough as f32, v / st.wid * 2.0, spec.seed);
                            (u / st.len).powi(2) + (v / st.wid).powi(2) <= wob
                        }
                        StrokeShape::Diamond => (u / st.len).abs() + (v / st.wid).abs() <= 1.0,
                        StrokeShape::Triangle => {
                            // Apex along +u, base across v.
                            let a = (u / st.len + 1.0) * 0.5;
                            (0.0..=1.0).contains(&a) && (v / st.wid).abs() <= 1.0 - a
                        }
                        StrokeShape::Star => {
                            let r = (u * u + v * v).sqrt() / st.len.max(1e-3);
                            let ang = v.atan2(u);
                            let k = ((ang * 2.5).cos() * 0.5 + 0.5).powf(0.6);
                            r <= 0.45 + 0.55 * k
                        }
                        StrokeShape::Image => spec.brush_map.is_some_and(|m| {
                            let r = m.rect;
                            let bu = ((u / st.len + 1.0) * 0.5 * r.width() as f32).floor() as i32;
                            let bv = ((v / st.wid + 1.0) * 0.5 * r.height() as f32).floor() as i32;
                            (0..r.width() as i32).contains(&bu) && (0..r.height() as i32).contains(&bv) && tiled_luma(m, bu, bv) > 0.5
                        }),
                    };
                    if inside {
                        // Bristle texture along the stroke.
                        let bristle = 0.85 + 0.15 * ((v / st.wid * 7.0 + st.rough as f32).sin() * 0.5 + 0.5);
                        let col = [st.col[0] * bristle, st.col[1] * bristle, st.col[2] * bristle, st.col[3]];
                        acc = over(col, acc);
                    }
                }
            }
        }
        if relief > 0.0 {
            let paper = paper_height(spec, fx, fy);
            let k = 1.0 + paper * relief * 0.6;
            acc = [(acc[0] * k).clamp(0.0, 1.0), (acc[1] * k).clamp(0.0, 1.0), (acc[2] * k).clamp(0.0, 1.0), acc[3]];
        }
        acc
    })
}

/// Van Gogh parameters.
pub(crate) struct LicSpec {
    pub channel: LicChannel,
    pub operator: LicOperator,
    pub convolve: LicConvolve,
    pub filter_length: f32,
    pub noise_magnitude: f32,
    pub steps: u32,
    pub min_value: f32,
    pub max_value: f32,
    pub seed: u32,
}

/// Pixels Van Gogh reads around an output pixel.
pub(crate) fn lic_reach(steps: u32) -> f32 {
    steps.clamp(1, 40) as f32 + 3.0
}

/// Van Gogh (line integral convolution): a flow field derived from one channel of the image (its
/// gradient, or the derivative along the isolines) guides streamlines along which either white
/// noise or the image itself is averaged over `filter_length`, for `steps` steps each way. The
/// noise result is stretched between `min_value` and `max_value`.
pub(crate) fn van_gogh(src: &Image, out: Rect, ctx: &Ctx, spec: &LicSpec) -> Vec<f32> {
    let steps = spec.steps.clamp(1, 40) as i32;
    let reach = lic_reach(spec.steps) as i32;
    let win = out.inflate(reach + 2);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let buf = rgba_window(src, ctx, win);
    // Scalar field, smoothed a little so the flow is coherent.
    let mut field: Vec<f32> = (0..ww * wh)
        .map(|i| {
            let c = window_px(&buf, win, win.x0 + (i % ww) as i32, win.y0 + (i / ww) as i32);
            match spec.channel {
                LicChannel::Hue => rgb_to_hsv([c[0], c[1], c[2]])[0],
                LicChannel::Saturation => rgb_to_hsv([c[0], c[1], c[2]])[1],
                LicChannel::Brightness => luma_rgb(c),
            }
        })
        .collect();
    gauss_blur_n(&mut field, ww, wh, 1, 1.0);
    let at = |x: f32, y: f32| crate::photo_util::bilinear(&field, ww, wh, 1, 0, x - win.x0 as f32 - 0.5, y - win.y0 as f32 - 0.5);
    let dir = |x: f32, y: f32| -> (f32, f32) {
        let gx = at(x + 1.0, y) - at(x - 1.0, y);
        let gy = at(x, y + 1.0) - at(x, y - 1.0);
        let (vx, vy) = match spec.operator {
            LicOperator::Gradient => (gx, gy),
            LicOperator::Derivative => (-gy, gx),
        };
        let m = (vx * vx + vy * vy).sqrt();
        if m < 1e-4 {
            let a = fbm(x / 40.0, y / 40.0, spec.seed) * std::f32::consts::TAU;
            (a.cos(), a.sin())
        } else {
            (vx / m, vy / m)
        }
    };
    let mag = spec.noise_magnitude.clamp(1.0, 5.0);
    let texture = |x: f32, y: f32| -> [f32; 4] {
        match spec.convolve {
            LicConvolve::WithWhiteNoise => {
                let v = hash01((x / mag).floor() as i32, (y / mag).floor() as i32, 71, spec.seed);
                [v, v, v, 1.0]
            }
            LicConvolve::WithSourceImage => window_px(&buf, win, x.floor() as i32, y.floor() as i32),
        }
    };
    let flen = spec.filter_length.clamp(0.1, 64.0);
    let lo = 0.5 + spec.min_value.clamp(-100.0, 0.0) / 200.0;
    let hi = 0.5 + spec.max_value.clamp(0.0, 100.0) / 200.0;
    map_px(src, out, ctx, |x, y, me| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut acc = [0.0f32; 4];
        let mut wsum = 0.0;
        for sign in [1.0f32, -1.0] {
            let (mut px, mut py) = (fx, fy);
            let (mut lx, mut ly) = (0.0f32, 0.0f32);
            for i in 0..=steps {
                if i > 0 {
                    let (mut vx, mut vy) = dir(px, py);
                    // Keep the streamline from reversing on itself.
                    if vx * lx + vy * ly < 0.0 {
                        vx = -vx;
                        vy = -vy;
                    }
                    px += vx * sign;
                    py += vy * sign;
                    lx = vx;
                    ly = vy;
                } else if sign < 0.0 {
                    continue;
                }
                let t = i as f32;
                let w = (1.0 - t / (flen.max(1.0) + 1.0)).max(0.0) * (-t / flen).exp();
                if w <= 0.0 {
                    break;
                }
                let tx = texture(px, py);
                for c in 0..4 {
                    acc[c] += tx[c] * w;
                }
                wsum += w;
            }
        }
        if wsum <= 0.0 {
            return me;
        }
        for v in acc.iter_mut() {
            *v /= wsum;
        }
        match spec.convolve {
            LicConvolve::WithWhiteNoise => {
                let v = ((acc[0] - lo) / (hi - lo).max(1e-3)).clamp(0.0, 1.0);
                // Modulate the image with the flow texture.
                [me[0] * (0.4 + 0.6 * v) + 0.2 * v, me[1] * (0.4 + 0.6 * v) + 0.2 * v, me[2] * (0.4 + 0.6 * v) + 0.2 * v, me[3]]
            }
            LicConvolve::WithSourceImage => [acc[0], acc[1], acc[2], me[3]],
        }
    })
}

/// Weave parameters.
pub(crate) struct WeaveSpec {
    pub ribbon_width: f32,
    pub ribbon_spacing: f32,
    pub shadow_darkness: f32,
    pub shadow_depth: f32,
    pub thread_length: f32,
    pub thread_density: f32,
    pub thread_intensity: f32,
}

/// Weave: horizontal and vertical ribbons of `ribbon_width` with `ribbon_spacing` gaps, crossing
/// over and under alternately, with shadows where a ribbon passes under and a thread texture
/// along each ribbon; the pattern multiplies the image.
pub(crate) fn weave(src: &Image, out: Rect, ctx: &Ctx, spec: &WeaveSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let w = spec.ribbon_width.clamp(1.0, 256.0);
    let gap = spec.ribbon_spacing.clamp(0.0, 256.0);
    let period = w + gap;
    let dark = spec.shadow_darkness.clamp(0.0, 100.0) / 100.0;
    let depth = (spec.shadow_depth.clamp(0.0, 100.0) / 100.0 * w).max(0.5);
    let tlen = spec.thread_length.clamp(1.0, 256.0);
    let tden = spec.thread_density.clamp(1.0, 256.0);
    let tint = spec.thread_intensity.clamp(0.0, 100.0) / 100.0;
    let thread = |along: f32, across: f32| -> f32 {
        let stripes = ((across / w * tden * 0.25).fract() * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        let breaks = fbm(along / tlen, across / w, 7);
        1.0 - tint * (0.25 * stripes + 0.2 * breaks)
    };
    map_px(src, out, ctx, |x, y, c| {
        let (fx, fy) = (x as f32 + 0.5 - b.x0 as f32, y as f32 + 0.5 - b.y0 as f32);
        let (u, v) = (fx.rem_euclid(period), fy.rem_euclid(period));
        let (i, j) = ((fx / period).floor() as i32, (fy / period).floor() as i32);
        let on_v = u < w;
        let on_h = v < w;
        let mask = if !on_v && !on_h {
            0.12
        } else if on_v && on_h {
            // Crossing: alternate which ribbon is on top.
            let v_over = (i + j).rem_euclid(2) == 0;
            if v_over { thread(fy, u) } else { thread(fx, v) }
        } else if on_v {
            // Vertical ribbon between crossings: shaded where it dives under the next horizontal.
            let t = thread(fy, u);
            let v_over_prev = (i + j).rem_euclid(2) == 0;
            let edge = if v_over_prev { (period - v).max(0.0) } else { (v - w).max(0.0) };
            t * (1.0 - dark * (1.0 - (edge / depth).min(1.0)))
        } else {
            let t = thread(fx, v);
            let h_over_prev = (i + j).rem_euclid(2) == 1;
            let edge = if h_over_prev { (period - u).max(0.0) } else { (u - w).max(0.0) };
            t * (1.0 - dark * (1.0 - (edge / depth).min(1.0)))
        };
        let k = mask.clamp(0.0, 1.0);
        [c[0] * k, c[1] * k, c[2] * k, c[3]]
    })
}
