// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pattern generators: Bayer Matrix, Checkerboard, Diffraction Patterns, Grid, Linear Sinusoid,
//! Maze, Sinus, Spiral, Jigsaw, Line Nova and Spirograph. Generators replace the pixels of the
//! reference bounds; the drawing ones (Grid, Jigsaw, Line Nova, Spirograph) paint over them.

use redrob_photo_geom::Rect;

use crate::gen_util::{Stream, Strokes, fbm, frame, generate, hash, mix, paint_over};
use crate::image::Image;
use crate::{Ctx, JigsawStyle, MazeAlgorithm, QuarterTurn, SinusBlend, SpinDirection, SpiralType, SpyroCurve, SpyroShape};

/// Grey level as an opaque colour.
#[inline]
fn grey(v: f32) -> [f32; 4] {
    let v = v.clamp(0.0, 1.0);
    [v, v, v, 1.0]
}

/// Bayer Matrix settings: `subdivisions` doubles the matrix side each step (2^n), scales repeat
/// each cell, `amplitude`/`offset`/`exponent` shape the ramp.
pub(crate) struct BayerSpec {
    pub subdivisions: u32,
    pub x_scale: u32,
    pub y_scale: u32,
    pub rotation: QuarterTurn,
    pub reflect: bool,
    pub amplitude: f32,
    pub offset: f32,
    pub exponent: f32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// The ordered-dither threshold of cell `(x, y)` in a 2^k matrix, 0…1.
fn bayer(x: i32, y: i32, k: u32) -> f32 {
    let size = 1i32 << k;
    let (x, y) = (x.rem_euclid(size), y.rem_euclid(size));
    let mut v = 0u32;
    for bit in 0..k {
        let xb = ((x >> bit) & 1) as u32;
        let yb = ((y >> bit) & 1) as u32;
        v = (v << 2) | ((xb ^ yb) << 1) | yb;
    }
    (v as f32 + 0.5) / (size * size) as f32
}

pub(crate) fn bayer_matrix(src: &Image, out: Rect, ctx: &Ctx, spec: &BayerSpec) -> Vec<f32> {
    let k = spec.subdivisions.clamp(0, 15);
    let (xs, ys) = (spec.x_scale.max(1) as i32, spec.y_scale.max(1) as i32);
    let b = ctx.bounds;
    generate(src, out, ctx, |x, y| {
        let (mut ix, iy) =
            (((x - b.x0 as f32).floor() as i32 + spec.x_offset).div_euclid(xs), ((y - b.y0 as f32).floor() as i32 + spec.y_offset).div_euclid(ys));
        if spec.reflect {
            ix = -ix - 1;
        }
        let (rx, ry) = match spec.rotation {
            QuarterTurn::None => (ix, iy),
            QuarterTurn::Quarter => (-iy - 1, ix),
            QuarterTurn::Half => (-ix - 1, -iy - 1),
            QuarterTurn::ThreeQuarters => (iy, -ix - 1),
        };
        let v = bayer(rx, ry, k).powf(spec.exponent.max(0.01)) * spec.amplitude + spec.offset;
        grey(v)
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn checkerboard(src: &Image, out: Rect, ctx: &Ctx, width: f32, height: f32, x_offset: f32, y_offset: f32, c1: [f32; 4], c2: [f32; 4]) -> Vec<f32> {
    let (w, h) = (width.max(1.0), height.max(1.0));
    let b = ctx.bounds;
    generate(src, out, ctx, |x, y| {
        let i = ((x - b.x0 as f32 + x_offset) / w).floor() as i64;
        let j = ((y - b.y0 as f32 + y_offset) / h).floor() as i64;
        if (i + j).rem_euclid(2) == 0 { c1 } else { c2 }
    })
}

/// Diffraction Patterns settings: per channel the fringe frequency, the number of angular
/// contours and the edge sharpness; then overall brightness, slit scattering and polarisation.
pub(crate) struct DiffractionSpec {
    pub frequencies: [f32; 3],
    pub contours: [f32; 3],
    pub edges: [f32; 3],
    pub brightness: f32,
    pub scattering: f32,
    pub polarization: f32,
}

/// Diffraction Patterns: two-slit interference fringes per channel, modulated by angular
/// contours, with sharpness shaping the fringe profile.
pub(crate) fn diffraction(src: &Image, out: Rect, ctx: &Ctx, spec: &DiffractionSpec) -> Vec<f32> {
    let (cx, cy, w, h) = frame(ctx);
    let scale = 2.0 / w.min(h).max(1.0);
    let (sp, cp) = spec.polarization.to_radians().sin_cos();
    let slit = spec.scattering.clamp(0.0, 100.0) / 100.0;
    generate(src, out, ctx, |x, y| {
        let u = (x - cx) * scale;
        let v = (y - cy) * scale;
        // Rotate by the polarisation angle; slits sit on the rotated x axis.
        let (ru, rv) = (u * cp - v * sp, u * sp + v * cp);
        let ang = rv.atan2(ru);
        let r1 = ((ru - slit) * (ru - slit) + rv * rv).sqrt();
        let r2 = ((ru + slit) * (ru + slit) + rv * rv).sqrt();
        let mut c = [0.0f32; 4];
        for k in 0..3 {
            let f = spec.frequencies[k].max(0.0) * 6.0;
            let wave = 0.5 * ((f * r1).sin() + (f * r2).sin());
            let contour = 1.0 + 0.5 * spec.contours[k].max(0.0) * (ang * 4.0).cos();
            let t = (wave * contour).abs().clamp(0.0, 1.0);
            let sharp = 1.0 / (1.0 + spec.edges[k].max(0.0));
            c[k] = (t.powf(sharp) * spec.brightness.max(0.0)).clamp(0.0, 1.0);
        }
        c[3] = 1.0;
        c
    })
}

/// Grid: vertical lines every `x_spacing` px and horizontal lines every `y_spacing` px, of the
/// given width / height, painted over the image.
#[allow(clippy::too_many_arguments)]
pub(crate) fn grid(
    src: &Image,
    out: Rect,
    ctx: &Ctx,
    x_spacing: f32,
    y_spacing: f32,
    x_offset: f32,
    y_offset: f32,
    line_width: f32,
    line_height: f32,
    color: [f32; 4],
) -> Vec<f32> {
    let b = ctx.bounds;
    let (xs, ys) = (x_spacing.max(1.0), y_spacing.max(1.0));
    paint_over(src, out, ctx, |x, y| {
        // Coverage of a stripe of width `lw` starting at every multiple of `xs`.
        let cover = |p: f32, spacing: f32, lw: f32| {
            let m = p.rem_euclid(spacing);
            // Pixel spans m-0.5..m+0.5; the stripe spans 0..lw (and spacing..spacing+lw).
            let a = (lw.min(m + 0.5) - (m - 0.5).max(0.0)).clamp(0.0, 1.0);
            let b2 = ((spacing + lw).min(m + 0.5) - (m - 0.5).max(spacing)).clamp(0.0, 1.0);
            (a + b2).min(1.0)
        };
        let v = cover(x - b.x0 as f32 - x_offset, xs, line_width.max(0.0));
        let hcov = cover(y - b.y0 as f32 - y_offset, ys, line_height.max(0.0));
        let c = v.max(hcov);
        [color[0], color[1], color[2], color[3] * c]
    })
}

/// Linear Sinusoid settings (periods in px, phases in degrees, angle between the two waves and
/// a rotation of the whole pattern in degrees).
pub(crate) struct SinusoidSpec {
    pub x_period: f32,
    pub y_period: f32,
    pub x_amplitude: f32,
    pub y_amplitude: f32,
    pub x_phase: f32,
    pub y_phase: f32,
    pub angle: f32,
    pub offset: f32,
    pub exponent: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub rotation: f32,
    pub supersampling: u32,
}

pub(crate) fn linear_sinusoid(src: &Image, out: Rect, ctx: &Ctx, spec: &SinusoidSpec) -> Vec<f32> {
    let (cx, cy, _, _) = frame(ctx);
    let (sr, cr) = spec.rotation.to_radians().sin_cos();
    let (sa, ca) = spec.angle.to_radians().sin_cos();
    let ss = spec.supersampling.clamp(1, 8) as i32;
    let value = |x: f32, y: f32| {
        let (dx, dy) = (x - cx - spec.x_offset, y - cy - spec.y_offset);
        let (rx, ry) = (dx * cr - dy * sr, dx * sr + dy * cr);
        // The second wave runs along an axis `angle` degrees from the first.
        let along_x = rx;
        let along_y = rx * ca + ry * sa;
        let wx = spec.x_amplitude * (std::f32::consts::TAU * (along_x / spec.x_period.max(0.1) + spec.x_phase / 360.0)).sin();
        let wy = spec.y_amplitude * (std::f32::consts::TAU * (along_y / spec.y_period.max(0.1) + spec.y_phase / 360.0)).sin();
        let norm = (spec.x_amplitude.abs() + spec.y_amplitude.abs()).max(1e-6);
        let v = ((wx + wy) / norm * 0.5 + 0.5 + spec.offset).clamp(0.0, 1.0);
        v.powf(spec.exponent.max(0.01))
    };
    generate(src, out, ctx, |x, y| {
        let mut sum = 0.0;
        for j in 0..ss {
            for i in 0..ss {
                let ox = (i as f32 + 0.5) / ss as f32 - 0.5;
                let oy = (j as f32 + 0.5) / ss as f32 - 0.5;
                sum += value(x + ox, y + oy);
            }
        }
        grey(sum / (ss * ss) as f32)
    })
}

/// Maze settings. Cells are `cell_width` × `cell_height` px; walls are one cell wide.
pub(crate) struct MazeSpec {
    pub cell_width: f32,
    pub cell_height: f32,
    pub algorithm: MazeAlgorithm,
    pub tileable: bool,
    pub seed: u32,
    pub foreground: [f32; 4],
    pub background: [f32; 4],
}

/// Carves a maze on a `gw` × `gh` grid (odd cells are passages, walls between them) and returns
/// the wall mask (true = wall).
fn carve(gw: usize, gh: usize, algorithm: MazeAlgorithm, tileable: bool, seed: u32) -> Vec<bool> {
    let mut wall = vec![true; gw * gh];
    if gw < 3 || gh < 3 {
        return wall;
    }
    let mut rng = Stream::new(seed);
    let idx = |x: usize, y: usize| y * gw + x;
    let step = |x: usize, y: usize, d: usize| -> Option<(usize, usize, usize, usize)> {
        // Neighbour two cells away in direction d and the wall cell between.
        let (dx, dy): (isize, isize) = [(2, 0), (-2, 0), (0, 2), (0, -2)][d];
        let (nx, ny) = (x as isize + dx, y as isize + dy);
        let (wx, wy) = (x as isize + dx / 2, y as isize + dy / 2);
        if tileable {
            let (w, h) = (gw as isize, gh as isize);
            Some((nx.rem_euclid(w) as usize, ny.rem_euclid(h) as usize, wx.rem_euclid(w) as usize, wy.rem_euclid(h) as usize))
        } else if nx >= 1 && ny >= 1 && nx < gw as isize - 1 && ny < gh as isize - 1 {
            Some((nx as usize, ny as usize, wx as usize, wy as usize))
        } else {
            None
        }
    };
    let start = (1usize, 1usize);
    wall[idx(start.0, start.1)] = false;
    match algorithm {
        MazeAlgorithm::DepthFirst => {
            let mut stack = vec![start];
            while let Some(&(x, y)) = stack.last() {
                let mut dirs = [0usize, 1, 2, 3];
                for i in (1..4).rev() {
                    let j = rng.below(i + 1);
                    dirs.swap(i, j);
                }
                let mut moved = false;
                for d in dirs {
                    if let Some((nx, ny, wx, wy)) = step(x, y, d)
                        && wall[idx(nx, ny)]
                    {
                        wall[idx(wx, wy)] = false;
                        wall[idx(nx, ny)] = false;
                        stack.push((nx, ny));
                        moved = true;
                        break;
                    }
                }
                if !moved {
                    stack.pop();
                }
            }
        }
        MazeAlgorithm::Prim => {
            let mut frontier: Vec<(usize, usize, usize, usize)> = Vec::new();
            let push_frontier = |x: usize, y: usize, frontier: &mut Vec<(usize, usize, usize, usize)>, wall: &[bool]| {
                for d in 0..4 {
                    if let Some((nx, ny, wx, wy)) = step(x, y, d)
                        && wall[idx(nx, ny)]
                    {
                        frontier.push((nx, ny, wx, wy));
                    }
                }
            };
            push_frontier(start.0, start.1, &mut frontier, &wall);
            while !frontier.is_empty() {
                let k = rng.below(frontier.len());
                let (nx, ny, wx, wy) = frontier.swap_remove(k);
                if !wall[idx(nx, ny)] {
                    continue;
                }
                wall[idx(wx, wy)] = false;
                wall[idx(nx, ny)] = false;
                push_frontier(nx, ny, &mut frontier, &wall);
            }
        }
    }
    wall
}

/// The Circuit filter's trace maze (depth-first, not tileable).
pub(crate) fn carve_for_circuit(gw: usize, gh: usize, seed: u32) -> Vec<bool> {
    carve(gw, gh, MazeAlgorithm::DepthFirst, false, seed)
}

pub(crate) fn maze(src: &Image, out: Rect, ctx: &Ctx, spec: &MazeSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (cw, ch) = (spec.cell_width.max(1.0), spec.cell_height.max(1.0));
    let mut gw = ((b.width() as f32 / cw).floor() as usize).max(3);
    let mut gh = ((b.height() as f32 / ch).floor() as usize).max(3);
    if spec.tileable {
        // Even grids wrap: passage columns at odd indices, wall columns at even ones.
        gw &= !1;
        gh &= !1;
        gw = gw.max(4);
        gh = gh.max(4);
    } else {
        gw |= 1;
        gh |= 1;
    }
    let wall = carve(gw, gh, spec.algorithm, spec.tileable, spec.seed);
    generate(src, out, ctx, |x, y| {
        let i = (((x - b.x0 as f32) / cw).floor() as i64).rem_euclid(gw as i64) as usize;
        let j = (((y - b.y0 as f32) / ch).floor() as i64).rem_euclid(gh as i64) as usize;
        if wall[j * gw + i] { spec.foreground } else { spec.background }
    })
}

/// Sinus settings: wave scales (periods over the bounds), harmonic complexity, randomness, blend
/// law and exponent between the two colours.
pub(crate) struct SinusSpec {
    pub x_scale: f32,
    pub y_scale: f32,
    pub complexity: f32,
    pub seed: u32,
    pub tileable: bool,
    pub perturbation: bool,
    pub blend: SinusBlend,
    pub exponent: f32,
    pub color1: [f32; 4],
    pub color2: [f32; 4],
}

pub(crate) fn sinus(src: &Image, out: Rect, ctx: &Ctx, spec: &SinusSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (w, h) = (b.width() as f32, b.height() as f32);
    let mut rng = Stream::new(spec.seed);
    let harmonics = (spec.complexity.clamp(0.0, 15.0).round() as usize).max(1);
    // Random wave set: direction, frequency (integer when tileable) and phase per harmonic.
    let waves: Vec<(f32, f32, f32, f32)> = (0..harmonics)
        .map(|k| {
            let fx = spec.x_scale * (0.5 + rng.unit() * (k as f32 + 1.0));
            let fy = spec.y_scale * (0.5 + rng.unit() * (k as f32 + 1.0));
            let (fx, fy) = if spec.tileable { (fx.round().max(1.0), fy.round().max(1.0)) } else { (fx, fy) };
            let phase = rng.unit() * std::f32::consts::TAU;
            let amp = 1.0 / (k as f32 + 1.0);
            (fx, fy, phase, amp)
        })
        .collect();
    let total: f32 = waves.iter().map(|w| w.3).sum();
    generate(src, out, ctx, |x, y| {
        let mut u = (x - b.x0 as f32) / w.max(1.0);
        let mut v = (y - b.y0 as f32) / h.max(1.0);
        if spec.perturbation {
            u += (fbm(u * 3.0, v * 3.0, 3, spec.seed ^ 0x55) - 0.5) * 0.15;
            v += (fbm(u * 3.0 + 7.0, v * 3.0, 3, spec.seed ^ 0xaa) - 0.5) * 0.15;
        }
        let mut s = 0.0;
        for (fx, fy, phase, amp) in &waves {
            s += amp * (std::f32::consts::TAU * (u * fx + v * fy) + phase).sin();
        }
        let t = (s / total * 0.5 + 0.5).clamp(0.0, 1.0);
        let t = match spec.blend {
            SinusBlend::Linear => t,
            SinusBlend::Bilinear => 1.0 - (2.0 * t - 1.0).abs(),
            SinusBlend::Sinusoidal => 0.5 - 0.5 * (std::f32::consts::PI * t).cos(),
        };
        mix(spec.color1, spec.color2, t.powf(spec.exponent.max(0.01)))
    })
}

/// Spiral settings: origin as fractions of the bounds, radius in px between turns, log base,
/// balance of the two colours (−1…1), rotation in degrees.
pub(crate) struct SpiralSpec {
    pub kind: SpiralType,
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub base: f32,
    pub balance: f32,
    pub rotation: f32,
    pub direction: SpinDirection,
    pub color1: [f32; 4],
    pub color2: [f32; 4],
}

pub(crate) fn spiral(src: &Image, out: Rect, ctx: &Ctx, spec: &SpiralSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (ox, oy) = (b.x0 as f32 + spec.x * b.width() as f32, b.y0 as f32 + spec.y * b.height() as f32);
    let rot = spec.rotation.to_radians();
    let dir = match spec.direction {
        SpinDirection::Clockwise => 1.0,
        SpinDirection::CounterClockwise => -1.0,
    };
    let split = (0.5 + spec.balance.clamp(-1.0, 1.0) * 0.5).clamp(0.01, 0.99);
    let phase = |x: f32, y: f32| -> f32 {
        let (dx, dy) = (x - ox, y - oy);
        let r = (dx * dx + dy * dy).sqrt().max(1e-3);
        let a = (dy.atan2(dx) * dir + rot) / std::f32::consts::TAU;
        let turns = match spec.kind {
            SpiralType::Linear => r / spec.radius.max(0.5),
            SpiralType::Logarithmic => (r / spec.radius.max(0.5)).ln() / spec.base.max(1.01).ln(),
        };
        (turns - a).rem_euclid(1.0)
    };
    generate(src, out, ctx, |x, y| {
        // 2×2 supersampling smooths the band edges.
        let mut t = 0.0;
        for (ax, ay) in [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)] {
            if phase(x + ax, y + ay) < split {
                t += 0.25;
            }
        }
        mix(spec.color2, spec.color1, t)
    })
}

/// Jigsaw settings: piece counts, bevel width in px, highlight strength 0…1 and the tab style.
pub(crate) struct JigsawSpec {
    pub x_tiles: u32,
    pub y_tiles: u32,
    pub bevel_width: f32,
    pub highlight: f32,
    pub style: JigsawStyle,
}

/// Tab profile along an edge: offset (in units of the piece size across the edge) at position
/// `u` (0…1 along the edge). Positive = toward the next piece.
fn tab(u: f32, style: JigsawStyle) -> f32 {
    let d = (u - 0.5).abs();
    match style {
        JigsawStyle::Curved => {
            if d < 0.1 {
                // Knob: a bulb with a slight neck.
                let k = (1.0 - (d / 0.1) * (d / 0.1)).max(0.0).sqrt();
                0.08 + 0.12 * k
            } else if d < 0.16 {
                0.08 * (1.0 - (d - 0.1) / 0.06)
            } else {
                0.0
            }
        }
        JigsawStyle::Square => {
            if d < 0.12 {
                0.18
            } else {
                0.0
            }
        }
    }
}

pub(crate) fn jigsaw(src: &Image, out: Rect, ctx: &Ctx, spec: &JigsawSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (w, h) = (b.width() as f32, b.height() as f32);
    let (nx, ny) = (spec.x_tiles.clamp(1, 64) as f32, spec.y_tiles.clamp(1, 64) as f32);
    let (tw, th) = (w / nx, h / ny);
    let bevel = spec.bevel_width.max(0.5);
    let hl = spec.highlight.clamp(0.0, 1.0);
    paint_over(src, out, ctx, |x, y| {
        let (u, v) = (x - b.x0 as f32, y - b.y0 as f32);
        // Nearest vertical boundary (between columns i and i+1) with its tab.
        let col = (u / tw).round();
        let mut best = f32::INFINITY;
        let mut side = 0.0f32;
        if col >= 1.0 && col <= nx - 1.0 {
            let row = (v / th).floor();
            let along = (v - row * th) / th;
            let sign = if hash(col as i32, row as i32, 17) < 0.5 { 1.0 } else { -1.0 };
            let bx = col * tw + sign * tab(along, spec.style) * tw;
            let d = (u - bx).abs();
            if d < best {
                best = d;
                side = (u - bx).signum();
            }
        }
        let row = (v / th).round();
        if row >= 1.0 && row <= ny - 1.0 {
            let colf = (u / tw).floor();
            let along = (u - colf * tw) / tw;
            let sign = if hash(colf as i32, row as i32, 29) < 0.5 { 1.0 } else { -1.0 };
            let by = row * th + sign * tab(along, spec.style) * th;
            let d = (v - by).abs();
            if d < best {
                best = d;
                side = (v - by).signum();
            }
        }
        if best > bevel + 1.0 {
            return [0.0, 0.0, 0.0, 0.0];
        }
        // Outline: a dark 1 px line; bevel: light on one side, dark on the other, fading out.
        let line = (1.0 - best).clamp(0.0, 1.0);
        let fall = (1.0 - best / bevel).clamp(0.0, 1.0);
        let shade = fall * hl * 0.6;
        let a = line.max(shade);
        if side > 0.0 {
            [0.0, 0.0, 0.0, a.min(1.0)]
        } else if line > 0.0 {
            [0.0, 0.0, 0.0, line]
        } else {
            [1.0, 1.0, 1.0, shade.min(1.0)]
        }
    })
}

/// Line Nova settings: `lines` rays from the centre, `sharpness` (1 = hairline, 10 = wide),
/// hole radius before the rays start, and random jitter of angle and start (0…1).
pub(crate) struct LineNovaSpec {
    pub lines: u32,
    pub sharpness: f32,
    pub offset_radius: f32,
    pub randomness: f32,
    pub seed: u32,
    pub color: [f32; 4],
}

pub(crate) fn line_nova(src: &Image, out: Rect, ctx: &Ctx, spec: &LineNovaSpec) -> Vec<f32> {
    let (cx, cy, w, h) = frame(ctx);
    let n = spec.lines.clamp(1, 1000) as usize;
    let mut rng = Stream::new(spec.seed);
    let sector = std::f32::consts::TAU / n as f32;
    let jitter = spec.randomness.clamp(0.0, 1.0);
    let rays: Vec<(f32, f32)> = (0..n)
        .map(|k| {
            let a = k as f32 * sector + (rng.unit() - 0.5) * sector * jitter;
            let start = spec.offset_radius.max(0.0) * (1.0 + (rng.unit() - 0.5) * jitter);
            (a, start)
        })
        .collect();
    // Half opening angle of a ray: sharpness 10 fills the sector, 1 is a hairline.
    let half = sector / 2.0 * (spec.sharpness.clamp(1.0, 10.0) / 10.0);
    let reach = (w * w + h * h).sqrt();
    paint_over(src, out, ctx, |x, y| {
        let (dx, dy) = (x - cx, y - cy);
        let r = (dx * dx + dy * dy).sqrt();
        let ang = dy.atan2(dx);
        let mut cov = 0.0f32;
        // Rays near this angle: check the nearest few.
        let k0 = ((ang / sector).round() as i64).rem_euclid(n as i64) as usize;
        for off in [-1i64, 0, 1] {
            let k = (k0 as i64 + off).rem_euclid(n as i64) as usize;
            let (ra, start) = rays[k];
            if r < start || r > reach {
                continue;
            }
            let da = (ang - ra + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            // Perpendicular distance from the ray's centre line and its half width here.
            let perp = (r * da.sin()).abs();
            let hw = ((r - start) * half.tan()).max(0.5);
            let c = (hw - perp + 0.5).clamp(0.0, 1.0);
            cov = cov.max(c);
        }
        [spec.color[0], spec.color[1], spec.color[2], spec.color[3] * cov]
    })
}

/// Spirograph settings. Gear teeth counts fix the ratio, `hole` is the pen distance from the
/// moving gear's centre in % of its radius, `sides`/`morph` shape the fixed gear outline.
pub(crate) struct SpyroSpec {
    pub curve: SpyroCurve,
    pub shape: SpyroShape,
    pub fixed_teeth: u32,
    pub moving_teeth: u32,
    pub hole: f32,
    pub sides: u32,
    pub morph: f32,
    pub rotation: f32,
    pub line_width: f32,
    pub color: [f32; 4],
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// Radius of the fixed gear outline at angle `t` (unit circle = 1).
fn outline(shape: SpyroShape, sides: u32, morph: f32, t: f32) -> f32 {
    let n = sides.clamp(3, 32) as f32;
    let m = morph.clamp(0.0, 1.0);
    match shape {
        SpyroShape::Circle => 1.0,
        SpyroShape::Polygon => {
            // Regular polygon: distance to the edge along angle t.
            let a = (t * n / std::f32::consts::TAU).fract() * std::f32::consts::TAU / n - std::f32::consts::PI / n;
            let poly = (std::f32::consts::PI / n).cos() / a.cos().max(1e-3);
            1.0 + (poly - 1.0) * (1.0 - m * 0.8)
        }
        SpyroShape::Star => {
            let a = ((t * n / std::f32::consts::TAU).fract() - 0.5).abs() * 2.0;
            1.0 - (0.3 + 0.5 * m) * (1.0 - a)
        }
        SpyroShape::Sine => 1.0 + 0.1 * (1.0 + m) * (t * n).sin(),
        SpyroShape::Bumps => 1.0 + 0.1 * (1.0 + m) * (t * n).sin().abs(),
        SpyroShape::Rack => 1.0,
        SpyroShape::Frame => {
            let (s, c) = t.sin_cos();
            1.0 / s.abs().max(c.abs()).max(1e-3)
        }
    }
}

pub(crate) fn spyro(src: &Image, out: Rect, ctx: &Ctx, spec: &SpyroSpec) -> Vec<f32> {
    let (cx, cy, w, h) = frame(ctx);
    let big = spec.fixed_teeth.clamp(1, 1000);
    let small = spec.moving_teeth.clamp(1, 1000);
    let g = gcd(big, small);
    let turns = (small / g).clamp(1, 60) as f32;
    let (rb, rs) = (big as f32, small as f32);
    let d = rs * spec.hole.clamp(0.0, 100.0) / 100.0;
    // Fit the fixed gear's extent into the bounds.
    let extent = match spec.curve {
        SpyroCurve::Epitrochoid => rb + 2.0 * rs,
        _ => rb.max(rs),
    };
    let unit = 0.45 * w.min(h) / extent.max(1.0);
    let rot = spec.rotation.to_radians();
    let steps = ((turns * 360.0) as usize).clamp(360, 20_000);
    let mut strokes = Strokes::new();
    let mut pts: Vec<[f32; 2]> = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = turns * std::f32::consts::TAU * i as f32 / steps as f32;
        let (mut px, mut py) = match spec.curve {
            SpyroCurve::Spirograph => {
                let rho = outline(spec.shape, spec.sides, spec.morph, t) * rb;
                let k = (rho - rs) / rs;
                let (gx, gy) = if spec.shape == SpyroShape::Rack {
                    (t / std::f32::consts::TAU * rs * 2.0 - rb, 0.0)
                } else {
                    ((rho - rs) * t.cos(), (rho - rs) * t.sin())
                };
                (gx + d * (k * t).cos(), gy - d * (k * t).sin())
            }
            SpyroCurve::Epitrochoid => {
                let rho = outline(spec.shape, spec.sides, spec.morph, t) * rb;
                let k = (rho + rs) / rs;
                ((rho + rs) * t.cos() - d * (k * t).cos(), (rho + rs) * t.sin() - d * (k * t).sin())
            }
            SpyroCurve::Sine => {
                let rho = outline(spec.shape, spec.sides, spec.morph, t) * rb;
                let r = rho - rs + d * (t * rb / rs).sin();
                (r * t.cos(), r * t.sin())
            }
            SpyroCurve::Lissajous => (rb * (t * rs / rb.max(1.0)).sin(), rb * (t + rot).cos() * (1.0 - d / rb.max(1.0) * 0.5)),
        };
        if spec.curve != SpyroCurve::Lissajous {
            let (s, c) = rot.sin_cos();
            let (qx, qy) = (px * c - py * s, px * s + py * c);
            px = qx;
            py = qy;
        }
        pts.push([cx + px * unit, cy + py * unit]);
    }
    // Thin out nearly collinear runs so the stroke list stays small on large curves.
    let mut simplified: Vec<[f32; 2]> = Vec::with_capacity(pts.len());
    for p in pts {
        if let Some(last) = simplified.last()
            && (last[0] - p[0]).abs() + (last[1] - p[1]).abs() < 0.75
        {
            continue;
        }
        simplified.push(p);
    }
    strokes.polyline(&simplified, spec.line_width.max(0.5), false);
    let strokes = strokes.finish();
    paint_over(src, out, ctx, |x, y| {
        let c = strokes.coverage(x, y);
        [spec.color[0], spec.color[1], spec.color[2], spec.color[3] * c]
    })
}
