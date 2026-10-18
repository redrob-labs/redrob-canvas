//! Distortions of the fourth batch: Apply Lens, Mosaic (tiles), Newsprint, Curve Bend, Warp
//! (gradient of a map) and Panorama Projection.

use redrob_photo_geom::Rect;

use super::super::extra::{fbm, luma_rgb, map_px, rgba_at, sstep, value_noise};
use crate::image::{Edge, Image};
use crate::noise::hash01;
use crate::{Ctx, MosaicTile, NewsprintModel, PanoSampler, ScreenPattern, WarpEdge};

/// Bilinear straight-RGBA sample of the source at `(x, y)` (pixel centres at .5) with `edge`.
fn sample_rgba(src: &Image, ctx: &Ctx, x: f32, y: f32, edge: Edge) -> [f32; 4] {
    let mut tmp = [0.0f32; crate::fxutil::MAXC];
    let n = src.ch.min(crate::fxutil::MAXC);
    src.sample(x, y, edge, ctx.bounds, ctx.alpha, &mut tmp[..n]);
    crate::fxutil::rgba(ctx, &tmp[..n])
}

/// Apply Lens: a spherical lens of refraction index `refraction` fills the ellipse inscribed in
/// the bounds; a ray through each pixel refracts at the lens surface (Snell's law) and samples
/// the image behind it. Outside the lens the image stays (`keep_surroundings`) or becomes
/// `background`.
pub(crate) fn apply_lens(src: &Image, out: Rect, ctx: &Ctx, refraction: f32, keep_surroundings: bool, background: [f32; 4]) -> Vec<f32> {
    let b = ctx.bounds;
    let (cx, cy) = (b.x0 as f32 + b.width() as f32 / 2.0, b.y0 as f32 + b.height() as f32 / 2.0);
    let (a, bb) = ((b.width() as f32 / 2.0).max(1.0), (b.height() as f32 / 2.0).max(1.0));
    let n = refraction.clamp(1.0, 100.0);
    let bg = if ctx.alpha { background } else { [background[0], background[1], background[2], 1.0] };
    map_px(src, out, ctx, |x, y, me| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let (u, v) = ((fx - cx) / a, (fy - cy) / bb);
        let r2 = u * u + v * v;
        if r2 >= 1.0 {
            return if keep_surroundings { me } else { bg };
        }
        // Height of the lens surface over the image plane, in units of the lens radius.
        let z = (1.0 - r2).sqrt();
        let r = r2.sqrt();
        if r < 1e-4 {
            return me;
        }
        // Incidence against the surface normal (u, v, z), refraction by Snell's law; the ray
        // bends toward the normal and crosses the lens height to the image plane.
        let theta_i = z.clamp(-1.0, 1.0).acos();
        let theta_r = (theta_i.sin() / n).clamp(-1.0, 1.0).asin();
        let shift = (theta_i - theta_r).tan() * z;
        let (sx, sy) = (fx - u / r * shift * a, fy - v / r * shift * bb);
        sample_rgba(src, ctx, sx, sy, Edge::Repeat)
    })
}

/// Mosaic parameters.
pub(crate) struct MosaicSpec {
    pub tile: MosaicTile,
    pub size: f32,
    pub height: f32,
    pub spacing: f32,
    pub neatness: f32,
    pub allow_splitting: bool,
    pub color_variation: f32,
    pub light_direction: f32,
    pub antialiasing: bool,
    pub color_averaging: bool,
    pub joints_color: [f32; 4],
    pub light_color: [f32; 4],
    pub seed: u32,
}

/// Pixels Mosaic reads around an output pixel.
pub(crate) fn mosaic_reach(size: f32) -> f32 {
    size.clamp(1.0, 1000.0) * 2.5 + 3.0
}

/// A tile centre candidate: position, extra weight (octagon tiling) and a hash key.
#[derive(Clone, Copy)]
struct Site {
    x: f32,
    y: f32,
    w: f32,
    kx: i32,
    ky: i32,
}

/// Lattice sites of `tile` around lattice cell `(i, j)` (one or two sites per cell), jittered by
/// `(1 - neatness)`.
fn sites(spec: &MosaicSpec, b: Rect, i: i32, j: i32, out: &mut Vec<Site>) {
    let s = spec.size.clamp(1.0, 1000.0);
    let jit = (1.0 - spec.neatness.clamp(0.0, 1.0)) * s * 0.45;
    let (ox, oy) = (b.x0 as f32, b.y0 as f32);
    let mut push = |x: f32, y: f32, w: f32, kx: i32, ky: i32| {
        let dx = (hash01(kx, ky, 81, spec.seed) - 0.5) * 2.0 * jit;
        let dy = (hash01(kx, ky, 82, spec.seed) - 0.5) * 2.0 * jit;
        out.push(Site { x: x + dx, y: y + dy, w, kx, ky });
    };
    match spec.tile {
        MosaicTile::Squares => push(ox + (i as f32 + 0.5) * s, oy + (j as f32 + 0.5) * s, 0.0, i, j),
        MosaicTile::Hexagons => {
            // Hexagonal lattice: rows s·√3/2 apart, odd rows shifted half a cell.
            let row_h = s * 0.866_025_4;
            let shift = if j.rem_euclid(2) == 1 { 0.5 } else { 0.0 };
            push(ox + (i as f32 + 0.5 + shift) * s, oy + (j as f32 + 0.5) * row_h, 0.0, i, j);
        }
        MosaicTile::Octagons => {
            // Truncated square tiling: big sites on the grid, small ones (negative weight) on
            // the grid corners, which the weighted nearest-site rule turns into small squares.
            push(ox + (i as f32 + 0.5) * s, oy + (j as f32 + 0.5) * s, 0.0, i * 2, j * 2);
            push(ox + i as f32 * s, oy + j as f32 * s, -s * 0.29, i * 2 + 1, j * 2 + 1);
        }
        MosaicTile::Triangles => {
            // Each lattice cell holds an up and a down triangle (centroids a third of the way).
            let row_h = s * 0.866_025_4;
            let shift = if j.rem_euclid(2) == 1 { 0.5 } else { 0.0 };
            let x0 = ox + (i as f32 + shift) * s;
            let y0 = oy + j as f32 * row_h;
            push(x0 + s * 0.5, y0 + row_h / 3.0, 0.0, i * 2, j);
            push(x0 + s, y0 + row_h * 2.0 / 3.0, 0.0, i * 2 + 1, j);
        }
    }
}

/// Lattice cell containing document point `(x, y)` for `tile` of `size`.
fn cell_of(spec: &MosaicSpec, b: Rect, x: f32, y: f32) -> (i32, i32) {
    let s = spec.size.clamp(1.0, 1000.0);
    let (lx, ly) = (x - b.x0 as f32, y - b.y0 as f32);
    match spec.tile {
        MosaicTile::Squares | MosaicTile::Octagons => ((lx / s).floor() as i32, (ly / s).floor() as i32),
        MosaicTile::Hexagons | MosaicTile::Triangles => ((lx / s).floor() as i32, (ly / (s * 0.866_025_4)).floor() as i32),
    }
}

/// Mosaic: the image broken into tiles (squares, hexagons, octagons with small squares, or
/// triangles) separated by joints of `spacing` pixels, each tile a slightly varied flat colour
/// (the colour under its centre, or the average over it) bevelled over `height` pixels and lit
/// from `light_direction`. Neatness jitters the tile corners; splitting lets tiles break along
/// strong image edges.
pub(crate) fn mosaic(src: &Image, out: Rect, ctx: &Ctx, spec: &MosaicSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let s = spec.size.clamp(1.0, 1000.0);
    let half_gap = spec.spacing.clamp(0.0, 1000.0) * 0.5;
    let bevel = spec.height.clamp(0.0, 1000.0).max(0.5);
    let (ls, lc) = spec.light_direction.to_radians().sin_cos();
    let light = [lc, -ls];
    let samples: &[(f32, f32)] = if spec.antialiasing { &[(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] } else { &[(0.5, 0.5)] };
    let joints = [spec.joints_color[0], spec.joints_color[1], spec.joints_color[2], 1.0];
    let mut cand: Vec<Site> = Vec::with_capacity(32);
    // Tile colour: averaged over the tile, or the pixel under its centre.
    let tile_colour = |st: &Site| -> [f32; 4] {
        if spec.color_averaging {
            let mut acc = [0.0f32; 4];
            let mut k = 0.0;
            let r = s * 0.3;
            for (dx, dy) in
                [(0.0, 0.0), (r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r), (r * 0.7, r * 0.7), (-r * 0.7, -r * 0.7), (r * 0.7, -r * 0.7), (-r * 0.7, r * 0.7)]
            {
                let c = rgba_at(src, ctx, (st.x + dx).floor() as i32, (st.y + dy).floor() as i32);
                for i in 0..4 {
                    acc[i] += c[i];
                }
                k += 1.0;
            }
            acc.map(|v| v / k)
        } else {
            rgba_at(src, ctx, st.x.floor() as i32, st.y.floor() as i32)
        }
    };
    let mut shade_one = |fx: f32, fy: f32, me: [f32; 4]| -> [f32; 4] {
        let (ci, cj) = cell_of(spec, b, fx, fy);
        cand.clear();
        for dj in -2..=2 {
            for di in -2..=2 {
                sites(spec, b, ci + di, cj + dj, &mut cand);
            }
        }
        // Nearest two weighted sites: the tile and the neighbour across the closest joint.
        let (mut d1, mut d2, mut best) = (f32::MAX, f32::MAX, cand[0]);
        for st in &cand {
            let d = ((fx - st.x).powi(2) + (fy - st.y).powi(2)).sqrt() + st.w;
            if d < d1 {
                d2 = d1;
                d1 = d;
                best = *st;
            } else if d < d2 {
                d2 = d;
            }
        }
        // Distance to the joint (half the gap between the two nearest sites), wobbled by the
        // neatness noise so untidy tiles get ragged edges.
        let wobble = (1.0 - spec.neatness.clamp(0.0, 1.0)) * s * 0.08 * (fbm(fx / (s * 0.5), fy / (s * 0.5), spec.seed ^ 0x55) - 0.5);
        let mut border = (d2 - d1) * 0.5 + wobble;
        if spec.allow_splitting {
            // A strong image edge inside the tile splits it with a joint.
            let (gx, gy) = super::super::extra::luma_gradient(src, ctx, fx.floor() as i32, fy.floor() as i32);
            let g = (gx * gx + gy * gy).sqrt();
            if g > 0.18 {
                border = border.min(half_gap * (1.0 - sstep(0.18, 0.4, g)));
            }
        }
        if border <= half_gap {
            return joints;
        }
        let mut col = tile_colour(&best);
        let var = spec.color_variation.clamp(0.0, 1.0);
        let k = 1.0 + (hash01(best.kx, best.ky, 83, spec.seed) - 0.5) * var;
        for c in col.iter_mut().take(3) {
            *c = (*c * k).clamp(0.0, 1.0);
        }
        // Bevel: within `bevel` pixels of the joint the surface slopes toward the tile centre.
        let slope = 1.0 - sstep(half_gap, half_gap + bevel, border);
        if slope > 0.0 {
            let (ox, oy) = (fx - best.x, fy - best.y);
            let len = (ox * ox + oy * oy).sqrt().max(1e-3);
            let dot = (ox / len) * light[0] + (oy / len) * light[1];
            let lit = dot * slope * 0.6;
            let lc = spec.light_color;
            for i in 0..3 {
                col[i] = if lit >= 0.0 { col[i] + (lc[i] - col[i]) * lit } else { col[i] * (1.0 + lit) }.clamp(0.0, 1.0);
            }
        }
        [col[0], col[1], col[2], if ctx.alpha { me[3].max(col[3]) } else { 1.0 }]
    };
    let n = src.ch;
    let mut tmp = src.crop(out);
    let w = out.width() as usize;
    for (i, px) in tmp.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32);
        let me = crate::fxutil::rgba(ctx, px);
        let mut acc = [0.0f32; 4];
        for (sx, sy) in samples {
            let c = shade_one(x as f32 + sx, y as f32 + sy, me);
            for k in 0..4 {
                acc[k] += c[k] / samples.len() as f32;
            }
        }
        crate::fxutil::set_rgba(ctx, px, acc);
    }
    tmp
}

/// Newsprint parameters: one screen per ink (grey models use `[0]`; RGB and CMYK use all).
pub(crate) struct NewsprintSpec {
    pub model: NewsprintModel,
    pub pattern: [ScreenPattern; 4],
    pub period: [f32; 4],
    pub angle: [f32; 4],
    pub black_pullout: f32,
    pub antialiasing: u32,
    pub turbulence: f32,
    pub seed: u32,
}

/// Spot function in 0..1 for cell coordinates in -0.5..0.5 (0 at the cell centre).
fn spot(pattern: ScreenPattern, u: f32, v: f32) -> f32 {
    match pattern {
        ScreenPattern::Line => v.abs() * 2.0,
        ScreenPattern::Circle => (u * u + v * v).sqrt() * 2.0 / std::f32::consts::SQRT_2,
        ScreenPattern::Diamond => u.abs() + v.abs(),
        ScreenPattern::PsSquare => u.abs().max(v.abs()) * 2.0,
        ScreenPattern::Cross => u.abs().min(v.abs()) * 2.0,
    }
}

/// Ink coverage (0 or 1, averaged over the sub-samples) of a screen with `amount` ink at `(x, y)`.
fn screened(spec: &NewsprintSpec, k: usize, amount: f32, x: f32, y: f32) -> f32 {
    let period = spec.period[k].clamp(1.0, 1000.0);
    let (s, c) = spec.angle[k].to_radians().sin_cos();
    let aa = spec.antialiasing.clamp(1, 16) as i32;
    let mut on = 0.0;
    for j in 0..aa {
        for i in 0..aa {
            let (px, py) = (x + (i as f32 + 0.5) / aa as f32, y + (j as f32 + 0.5) / aa as f32);
            let (rx, ry) = (px * c + py * s, -px * s + py * c);
            let (u, v) = ((rx / period).rem_euclid(1.0) - 0.5, (ry / period).rem_euclid(1.0) - 0.5);
            let mut sp = spot(spec.pattern[k], u, v);
            if spec.turbulence > 0.0 {
                sp += (value_noise(px / period * 2.0, py / period * 2.0, spec.seed ^ (k as u32 * 977)) - 0.5) * spec.turbulence;
            }
            // Coverage grows with ink: lines and crosses grow linearly with the threshold,
            // the area-based spots with its square.
            let a = amount.clamp(0.0, 1.0);
            let thr = match spec.pattern[k] {
                ScreenPattern::Line | ScreenPattern::Cross => a,
                _ => a.sqrt(),
            };
            if sp <= thr * 1.0001 {
                on += 1.0;
            }
        }
    }
    on / (aa * aa) as f32
}

/// Newsprint: halftone screens. Grey models screen the luminance as white dots on black or
/// black dots on white; RGB screens each channel; CMYK separates with `black_pullout` and
/// screens the four inks. Each screen has its own pattern, period and angle.
pub(crate) fn newsprint(src: &Image, out: Rect, ctx: &Ctx, spec: &NewsprintSpec) -> Vec<f32> {
    map_px(src, out, ctx, |x, y, me| {
        let (fx, fy) = (x as f32, y as f32);
        match spec.model {
            NewsprintModel::BlackOnWhite => {
                let ink = screened(spec, 0, 1.0 - luma_rgb(me), fx, fy);
                [1.0 - ink, 1.0 - ink, 1.0 - ink, me[3]]
            }
            NewsprintModel::WhiteOnBlack => {
                let ink = screened(spec, 0, luma_rgb(me), fx, fy);
                [ink, ink, ink, me[3]]
            }
            NewsprintModel::Rgb => [screened(spec, 0, me[0], fx, fy), screened(spec, 1, me[1], fx, fy), screened(spec, 2, me[2], fx, fy), me[3]],
            NewsprintModel::Cmyk => {
                let (c, m, yy) = (1.0 - me[0], 1.0 - me[1], 1.0 - me[2]);
                let k = c.min(m).min(yy) * spec.black_pullout.clamp(0.0, 1.0);
                let div = (1.0 - k).max(1e-4);
                let inks = [(c - k) / div, (m - k) / div, (yy - k) / div, k];
                let cov = [
                    screened(spec, 0, inks[0], fx, fy),
                    screened(spec, 1, inks[1], fx, fy),
                    screened(spec, 2, inks[2], fx, fy),
                    screened(spec, 3, inks[3], fx, fy),
                ];
                // Inks absorb their complement; black absorbs everything.
                let r = (1.0 - cov[0]) * (1.0 - cov[3]);
                let g = (1.0 - cov[1]) * (1.0 - cov[3]);
                let bl = (1.0 - cov[2]) * (1.0 - cov[3]);
                [r, g, bl, me[3]]
            }
        }
    })
}

/// Curve Bend parameters: the upper and lower edge curves as `(x, y)` points in 0..1, `y` = 0.5
/// being the straight edge.
pub(crate) struct CurveBendSpec {
    pub upper: Vec<[f32; 2]>,
    pub lower: Vec<[f32; 2]>,
    pub rotation: f32,
    pub smoothing: bool,
    pub antialias: bool,
}

/// Value of a curve through sorted `pts` at `t` (0..1): Catmull-Rom when `smooth`, else linear.
fn curve_value(pts: &[[f32; 2]], t: f32, smooth: bool) -> f32 {
    match pts.len() {
        0 => 0.5,
        1 => pts[0][1],
        _ => {
            if t <= pts[0][0] {
                return pts[0][1];
            }
            let last = pts.len() - 1;
            if t >= pts[last][0] {
                return pts[last][1];
            }
            let i = pts.windows(2).position(|w| t >= w[0][0] && t <= w[1][0]).unwrap_or(0);
            let (p1, p2) = (pts[i], pts[i + 1]);
            let span = (p2[0] - p1[0]).max(1e-6);
            let s = ((t - p1[0]) / span).clamp(0.0, 1.0);
            if !smooth {
                return p1[1] + (p2[1] - p1[1]) * s;
            }
            let p0 = if i > 0 { pts[i - 1] } else { p1 };
            let p3 = if i + 2 <= last { pts[i + 2] } else { p2 };
            let m1 = if i > 0 { (p2[1] - p0[1]) / (p2[0] - p0[0]).max(1e-6) * span } else { p2[1] - p1[1] };
            let m2 = if i + 2 <= last { (p3[1] - p1[1]) / (p3[0] - p1[0]).max(1e-6) * span } else { p2[1] - p1[1] };
            let (s2, s3) = (s * s, s * s * s);
            (2.0 * s3 - 3.0 * s2 + 1.0) * p1[1] + (s3 - 2.0 * s2 + s) * m1 + (-2.0 * s3 + 3.0 * s2) * p2[1] + (s3 - s2) * m2
        }
    }
}

/// Curve Bend: the bounds' top and bottom edges follow the upper and lower curves (each point's
/// `y` moves the edge by up to half the height either way) and the rows in between stretch
/// linearly; outside the bent shape the layer becomes transparent. `rotation` turns the frame
/// the curves are drawn in.
pub(crate) fn curve_bend(src: &Image, out: Rect, ctx: &Ctx, spec: &CurveBendSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (bw, bh) = (b.width().max(1) as f32, b.height().max(1) as f32);
    let (cx, cy) = (b.x0 as f32 + bw / 2.0, b.y0 as f32 + bh / 2.0);
    let (rs, rc) = spec.rotation.to_radians().sin_cos();
    let mut upper = spec.upper.clone();
    let mut lower = spec.lower.clone();
    upper.sort_by(|a, c| a[0].total_cmp(&c[0]));
    lower.sort_by(|a, c| a[0].total_cmp(&c[0]));
    let samples: &[(f32, f32)] = if spec.antialias { &[(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] } else { &[(0.5, 0.5)] };
    let clear = if ctx.alpha { [0.0, 0.0, 0.0, 0.0] } else { [1.0, 1.0, 1.0, 1.0] };
    let one = |fx: f32, fy: f32| -> [f32; 4] {
        // Into the rotated frame (local coordinates with the bounds' centre at the origin).
        let (dx, dy) = (fx - cx, fy - cy);
        let (lx, ly) = (dx * rc + dy * rs, -dx * rs + dy * rc);
        let t = (lx / bw + 0.5).clamp(0.0, 1.0);
        let top = -bh / 2.0 + (curve_value(&upper, t, spec.smoothing) - 0.5) * bh;
        let bottom = bh / 2.0 + (curve_value(&lower, t, spec.smoothing) - 0.5) * bh;
        let span = bottom - top;
        if span.abs() < 1e-3 {
            return clear;
        }
        let v = (ly - top) / span;
        if !(0.0..=1.0).contains(&v) {
            return clear;
        }
        let sy_local = -bh / 2.0 + v * bh;
        // Back out of the rotated frame.
        let (sx, sy) = (cx + lx * rc - sy_local * rs, cy + lx * rs + sy_local * rc);
        sample_rgba(src, ctx, sx, sy, Edge::Transparent)
    };
    map_px(src, out, ctx, |x, y, _| {
        let mut acc = [0.0f32; 4];
        for (sx, sy) in samples {
            let c = one(x as f32 + sx, y as f32 + sy);
            // Premultiplied average so transparent samples do not darken the edge.
            acc[0] += c[0] * c[3];
            acc[1] += c[1] * c[3];
            acc[2] += c[2] * c[3];
            acc[3] += c[3];
        }
        let n = samples.len() as f32;
        if acc[3] <= 1e-6 { clear } else { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3] / n] }
    })
}

/// Warp (map) parameters.
pub(crate) struct WarpMapSpec<'a> {
    pub step_size: f32,
    pub iterations: u32,
    pub on_edges: WarpEdge,
    pub dither: f32,
    pub rotation_angle: f32,
    pub substeps: u32,
    pub edge_color: [f32; 4],
    pub seed: u32,
    pub map: Option<&'a Image>,
    pub magnitude: Option<&'a Image>,
}

/// Luma of a map stretched over the bounds at document point `(x, y)` (bilinear, clamped).
fn map_luma(map: &Image, b: Rect, x: f32, y: f32) -> f32 {
    let r = map.rect;
    let (mw, mh) = (r.width() as f32, r.height() as f32);
    let u = (x - b.x0 as f32) / b.width().max(1) as f32 * mw - 0.5;
    let v = (y - b.y0 as f32) / b.height().max(1) as f32 * mh - 0.5;
    let (x0, y0) = (u.floor(), v.floor());
    let (ax, ay) = (u - x0, v - y0);
    let g = |ix: i32, iy: i32| {
        let (px, py) = (r.x0 + ix.clamp(0, mw as i32 - 1), r.y0 + iy.clamp(0, mh as i32 - 1));
        if map.ch >= 3 { 0.299 * map.get(px, py, 0) + 0.587 * map.get(px, py, 1) + 0.114 * map.get(px, py, 2) } else { map.get(px, py, 0) }
    };
    let (x0, y0) = (x0 as i32, y0 as i32);
    let top = g(x0, y0) + (g(x0 + 1, y0) - g(x0, y0)) * ax;
    let bot = g(x0, y0 + 1) + (g(x0 + 1, y0 + 1) - g(x0, y0 + 1)) * ax;
    top + (bot - top) * ay
}

/// Warp (map): every pixel is pushed along the gradient of the displacement map (turned by
/// `rotation_angle`), `step_size` pixels per iteration for `iterations` iterations, each split
/// into `substeps`; the magnitude map scales the step per pixel and dither jitters it. Pixels
/// pushed past the edge wrap, smear the edge, or take black / the foreground colour.
pub(crate) fn warp_map(src: &Image, out: Rect, ctx: &Ctx, spec: &WarpMapSpec) -> Vec<f32> {
    let Some(map) = spec.map.filter(|m| !m.rect.is_empty()) else { return src.crop(out) };
    let b = ctx.bounds;
    let (rs, rc) = spec.rotation_angle.to_radians().sin_cos();
    let substeps = spec.substeps.clamp(1, 16) as f32;
    let step = spec.step_size.clamp(-1000.0, 1000.0) / substeps;
    let total = spec.iterations.clamp(1, 100) * spec.substeps.clamp(1, 16);
    let dither = spec.dither.clamp(0.0, 100.0);
    // Gradient of the map by central differences one pixel apart, scaled so a full black-to-white
    // ramp across the bounds moves by the step.
    let grad = |x: f32, y: f32| -> (f32, f32) {
        let gx = (map_luma(map, b, x + 1.0, y) - map_luma(map, b, x - 1.0, y)) * 0.5;
        let gy = (map_luma(map, b, x, y + 1.0) - map_luma(map, b, x, y - 1.0)) * 0.5;
        // Scale to a sensible displacement per unit step (the map's full range over 8 px = 1).
        (gx * 8.0, gy * 8.0)
    };
    let edge_fill = if ctx.alpha { spec.edge_color } else { [spec.edge_color[0], spec.edge_color[1], spec.edge_color[2], 1.0] };
    let edge = match spec.on_edges {
        WarpEdge::Wrap => Edge::Wrap,
        WarpEdge::Smear => Edge::Repeat,
        WarpEdge::Black | WarpEdge::ForegroundColor => Edge::Transparent,
    };
    map_px(src, out, ctx, |x, y, _| {
        let (mut px, mut py) = (x as f32 + 0.5, y as f32 + 0.5);
        for k in 0..total {
            let (gx, gy) = grad(px, py);
            let (dx, dy) = (gx * rc - gy * rs, gx * rs + gy * rc);
            let mag = spec.magnitude.map_or(1.0, |m| map_luma(m, b, px, py));
            px -= dx * step * mag;
            py -= dy * step * mag;
            if dither > 0.0 {
                px += (hash01(x, y, 90 + k, spec.seed) - 0.5) * dither * 0.1;
                py += (hash01(x, y, 190 + k, spec.seed) - 0.5) * dither * 0.1;
            }
        }
        let inside = b.contains(px.floor() as i32, py.floor() as i32);
        if !inside && matches!(spec.on_edges, WarpEdge::Black | WarpEdge::ForegroundColor) {
            return if spec.on_edges == WarpEdge::Black { [0.0, 0.0, 0.0, edge_fill[3]] } else { edge_fill };
        }
        sample_rgba(src, ctx, px, py, edge)
    })
}

/// Panorama Projection parameters.
pub(crate) struct PanoramaSpec {
    pub pan: f32,
    pub tilt: f32,
    pub spin: f32,
    pub zoom: f32,
    pub inverse: bool,
    pub sampler: PanoSampler,
}

/// Rotates `v` by the view's spin (about z), tilt (about x) and pan (about y), in that order.
fn rotate_view(v: [f32; 3], spec: &PanoramaSpec, inverse: bool) -> [f32; 3] {
    let rot = |v: [f32; 3], axis: usize, deg: f32| -> [f32; 3] {
        let (s, c) = deg.to_radians().sin_cos();
        match axis {
            0 => [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c],
            1 => [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c],
            _ => [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]],
        }
    };
    if inverse { rot(rot(rot(v, 1, -spec.pan), 0, -spec.tilt), 2, -spec.spin) } else { rot(rot(rot(v, 2, spec.spin), 0, spec.tilt), 1, spec.pan) }
}

/// Panorama Projection: the bounds hold an equirectangular panorama; the output is the
/// rectilinear view a camera panned, tilted and spun by the angles sees, at `zoom` % (100 = a
/// 90° field of view). `inverse` maps a rectilinear picture back onto the sphere.
pub(crate) fn panorama(src: &Image, out: Rect, ctx: &Ctx, spec: &PanoramaSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (bw, bh) = (b.width().max(1) as f32, b.height().max(1) as f32);
    let (cx, cy) = (b.x0 as f32 + bw / 2.0, b.y0 as f32 + bh / 2.0);
    let fov = (90.0 / (spec.zoom.clamp(1.0, 1000.0) / 100.0)).clamp(1.0, 170.0_f32).to_radians();
    let focal = (bw / 2.0) / (fov / 2.0).tan();
    let edge = Edge::Wrap;
    let sample = |x: f32, y: f32| -> [f32; 4] {
        match spec.sampler {
            PanoSampler::Nearest => rgba_at(src, ctx, x.floor() as i32, y.floor() as i32),
            _ => sample_rgba(src, ctx, x, y, edge),
        }
    };
    if !spec.inverse {
        remap_rgba(src, out, ctx, |fx, fy| {
            let d = [(fx - cx) / focal, (fy - cy) / focal, 1.0];
            let d = rotate_view(d, spec, false);
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
            let lon = d[0].atan2(d[2]);
            let lat = (d[1] / len).clamp(-1.0, 1.0).asin();
            let u = b.x0 as f32 + (lon / std::f32::consts::TAU + 0.5).rem_euclid(1.0) * bw;
            let v = b.y0 as f32 + (lat / std::f32::consts::PI + 0.5).clamp(0.0, 1.0) * bh;
            sample(u, v)
        })
    } else {
        let clear = if ctx.alpha { [0.0, 0.0, 0.0, 0.0] } else { [0.0, 0.0, 0.0, 1.0] };
        remap_rgba(src, out, ctx, |fx, fy| {
            let lon = ((fx - b.x0 as f32) / bw - 0.5) * std::f32::consts::TAU;
            let lat = ((fy - b.y0 as f32) / bh - 0.5) * std::f32::consts::PI;
            let d = [lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos()];
            let d = rotate_view(d, spec, true);
            if d[2] <= 1e-4 {
                return clear;
            }
            let (px, py) = (cx + d[0] / d[2] * focal, cy + d[1] / d[2] * focal);
            if !b.contains(px.floor() as i32, py.floor() as i32) {
                return clear;
            }
            sample(px, py)
        })
    }
}

/// [`map_px`] over output pixel centres with a straight-RGBA sampler.
fn remap_rgba(src: &Image, out: Rect, ctx: &Ctx, f: impl Fn(f32, f32) -> [f32; 4]) -> Vec<f32> {
    map_px(src, out, ctx, |x, y, _| f(x as f32 + 0.5, y as f32 + 0.5))
}
