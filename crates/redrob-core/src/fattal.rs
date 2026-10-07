// SPDX-License-Identifier: GPL-3.0-or-later

//! Fattal, Lischinski and Werman 2002 gradient-domain tone mapping (K.10).
//!
//! Ported from `gegl/operations/common/fattal02.c` at the `[gegl_operations]` pin, which is itself
//! derived from pfstmo. GIMP only NAMES this operator — an action label, the `_Tone Mapping`
//! category and an ellipsis — so GEGL's tree is the only readable source, and reading it is what
//! unparked K.10.
//!
//! # What the operator does
//!
//! It works on the gradients of the log-luminance rather than on the luminance itself. Large
//! gradients — the edges between a bright window and a dark room — are attenuated, small ones are
//! left alone or amplified, and then an image is recovered from the modified gradient field by
//! solving a Poisson equation. That is why it preserves local detail that a global curve flattens.
//!
//! ```text
//! H          = ln(100 * Y / max Y + 1e-4)          log-luminance, normalised by the MAX only
//! G_k        = |grad H_k| / 2^(k+1)                gradient magnitude per pyramid level
//! FI_k       = a/(G_k+n) * ((G_k+n)/a)^beta        attenuation, a = alpha * mean(G_k)
//! FI         = product of FI_k, coarse to fine     upsampled and blurred between levels
//! (Gx, Gy)   = grad H * FI                         attenuated gradient field
//! div G      = Gx + Gy - Gx(west) - Gy(north)
//! lap U      = div G                               Poisson solve, Neumann boundary
//! L          = exp(U) - 1e-4
//! ```
//!
//! and finally each colour channel becomes `(C / Y)^saturation * L`.
//!
//! # Numerical equivalence with GEGL is NOT achievable, and saying so is the honest position
//!
//! Upstream's Poisson solve is truncated, not converged: 20 iterations of a biconjugate-gradient
//! smoother per level, two V-cycles, and a coarsest-level "exact solution" that is literally an
//! array of zeros — upstream's own comment says the successive over-relaxation that belongs there
//! was *"commented out due to 'incorrect results'"*. Two implementations of a truncated iteration
//! do not agree digit for digit. So the tests here assert STRUCTURE — ranges, monotonicity, what
//! each parameter does, which inputs are refused — and never a pixel value copied from GEGL.
//!
//! # The solver is conjugate gradient, and that is a derivation rather than a substitution
//!
//! Upstream calls `linbcg`, a Numerical Recipes routine, and the file header says *"Some code from
//! Numerical Recipes in C"*. Two things follow. First, this repository reads upstream for behaviour
//! and copies nothing (`kind = algorithm`), so none of that code is transcribed here. Second, the
//! biconjugate gradient method exists for NON-symmetric systems, and this system is symmetric: the
//! operator is the 5-point Laplacian with Neumann boundaries, which is the grid's own graph
//! Laplacian, negated. **For a symmetric matrix with a symmetric preconditioner, biconjugate
//! gradient generates the same iterates as plain conjugate gradient**, at half the work per step.
//! So CG is not a weaker stand-in; it is the same method with the redundancy removed.
//!
//! The matrix is also SINGULAR — a Neumann Laplacian annihilates the constant vector, so `U` is
//! determined only up to an additive constant. That is harmless here and it is why upstream can get
//! away with zeroing its coarse grid: the constant is removed by the percentile rescaling that
//! follows the exponential.

/// Below this, a pyramid level is not built. Upstream's `MINIMUM_PYRAMID`.
const MINIMUM_PYRAMID: usize = 32;

/// Below this, the multigrid hierarchy stops. Upstream's `MINS`, and deliberately NOT the same
/// number as [`MINIMUM_PYRAMID`]: the attenuation pyramid and the PDE hierarchy are different
/// structures with different floors, and conflating them would change both.
const MULTIGRID_MINIMUM: usize = 16;

/// Smoothing iterations per level. Upstream's `SMOOTH_IT`.
const SMOOTH_ITERATIONS: usize = 1;

/// Conjugate-gradient steps per smoothing call. Upstream's `BCG_STEPS`.
const SOLVER_STEPS: usize = 20;

/// V-cycles per nested iteration. Upstream's `V_CYCLE`.
const V_CYCLES: usize = 2;

/// The epsilon inside the logarithm, and subtracted again after the exponential.
const LOG_EPSILON: f32 = 1e-4;

/// One grid level: a buffer and the extent it is read with.
struct Level {
    data: Vec<f32>,
    width: usize,
    height: usize,
}

impl Level {
    fn zeros(width: usize, height: usize) -> Self {
        Self {
            data: vec![0.0; width * height],
            width,
            height,
        }
    }

    fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.width + x]
    }
}

/// Level dimensions, halved per level exactly as upstream's `LEVEL_WIDTH`/`LEVEL_HEIGHT` do —
/// integer division of the ORIGINAL extent, not repeated halving of the previous level. The two
/// differ once a dimension is odd, so this is not a simplification.
fn level_extent(width: usize, height: usize, level: usize) -> (usize, usize) {
    (width >> level, height >> level)
}

/// Blur with a one-pixel radius, `[1 2 1] / 4` separably, with the edge tap folded back.
///
/// The edge rule is upstream's and is not a clamp: the first column becomes
/// `(3 * self + neighbour) / 4`, so the missing tap is given to the pixel itself rather than to a
/// repeated neighbour. A clamped read would weight the neighbour twice.
fn gaussian_blur(input: &[f32], width: usize, height: usize) -> Vec<f32> {
    let mut temp = vec![0.0_f32; width * height];
    for y in 0..height {
        for x in 1..width.saturating_sub(1) {
            temp[y * width + x] =
                (2.0 * input[y * width + x] + input[y * width + x - 1] + input[y * width + x + 1])
                    / 4.0;
        }
        if width == 1 {
            temp[y * width] = input[y * width];
        } else {
            temp[y * width] = (3.0 * input[y * width] + input[y * width + 1]) / 4.0;
            temp[y * width + width - 1] =
                (3.0 * input[y * width + width - 1] + input[y * width + width - 2]) / 4.0;
        }
    }

    let mut output = vec![0.0_f32; width * height];
    for x in 0..width {
        for y in 1..height.saturating_sub(1) {
            output[y * width + x] =
                (2.0 * temp[y * width + x] + temp[(y - 1) * width + x] + temp[(y + 1) * width + x])
                    / 4.0;
        }
        if height == 1 {
            output[x] = temp[x];
        } else {
            output[x] = (3.0 * temp[x] + temp[width + x]) / 4.0;
            output[(height - 1) * width + x] =
                (3.0 * temp[(height - 1) * width + x] + temp[(height - 2) * width + x]) / 4.0;
        }
    }
    output
}

/// Halve by averaging each 2x2 block. Upstream's `fattal02_downsample`.
fn downsample(input: &[f32], width: usize, height: usize) -> Vec<f32> {
    let (out_w, out_h) = (width / 2, height / 2);
    let mut output = vec![0.0_f32; out_w * out_h];
    for y in 0..out_h {
        for x in 0..out_w {
            let sum = input[(2 * y) * width + 2 * x]
                + input[(2 * y) * width + 2 * x + 1]
                + input[(2 * y + 1) * width + 2 * x]
                + input[(2 * y + 1) * width + 2 * x + 1];
            output[y * out_w + x] = sum / 4.0;
        }
    }
    output
}

/// Double by nearest-neighbour replication. Upstream's `fattal02_upsample`, and it is NOT bilinear
/// — the smoothing that makes the attenuation field continuous is the blur applied after it.
fn upsample(input: &[f32], width: usize, height: usize, out_w: usize, out_h: usize) -> Vec<f32> {
    let mut output = vec![0.0_f32; out_w * out_h];
    for y in 0..out_h {
        for x in 0..out_w {
            let sx = (x / 2).min(width.saturating_sub(1));
            let sy = (y / 2).min(height.saturating_sub(1));
            output[y * out_w + x] = input[sy * width + sx];
        }
    }
    output
}

/// Box-average restriction onto a coarser grid, upstream's `fattal02_restrict`.
///
/// Note the sample centres: `sx` starts at `dx/2 - 0.5` and the window is `+/- dx/2`, so for the
/// 2:1 case this averages exactly the 2x2 block. It is written generally because the multigrid
/// hierarchy's levels are integer divisions of the original extent and so are not always 2:1.
fn restrict(input: &Level, out_w: usize, out_h: usize) -> Level {
    let mut output = Level::zeros(out_w, out_h);
    let dx = input.width as f32 / out_w as f32;
    let dy = input.height as f32 / out_h as f32;
    const FILTER: f32 = 0.5;

    for y in 0..out_h {
        let sy = dy / 2.0 - 0.5 + dy * y as f32;
        for x in 0..out_w {
            let sx = dx / 2.0 - 0.5 + dx * x as f32;
            let mut sum = 0.0_f32;
            let mut weight = 0.0_f32;
            // Upstream uses `dx` for BOTH axes of the window here. Kept, because it is the
            // behaviour, and on the square 2:1 levels this hierarchy uses the two are equal.
            let x_from = (sx - dx * FILTER).ceil().max(0.0) as usize;
            let x_to = ((sx + dx * FILTER).floor() as isize).min(input.width as isize - 1);
            let y_from = (sy - dx * FILTER).ceil().max(0.0) as usize;
            let y_to = ((sy + dx * FILTER).floor() as isize).min(input.height as isize - 1);
            for ix in x_from..=(x_to.max(x_from as isize) as usize) {
                for iy in y_from..=(y_to.max(y_from as isize) as usize) {
                    if ix >= input.width || iy >= input.height {
                        continue;
                    }
                    sum += input.at(ix, iy);
                    weight += 1.0;
                }
            }
            output.data[y * out_w + x] = if weight > 0.0 { sum / weight } else { 0.0 };
        }
    }
    output
}

/// Bilinear prolongation onto a finer grid, upstream's `fattal02_prolongate`.
fn prolongate(input: &Level, out_w: usize, out_h: usize) -> Level {
    let mut output = Level::zeros(out_w, out_h);
    let dx = input.width as f32 / out_w as f32;
    let dy = input.height as f32 / out_h as f32;
    const FILTER: f32 = 1.0;

    for y in 0..out_h {
        let sy = -dy / 2.0 + dy * y as f32;
        for x in 0..out_w {
            let sx = -dx / 2.0 + dx * x as f32;
            let mut sum = 0.0_f32;
            let mut weight = 0.0_f32;
            let x_from = (sx - FILTER).ceil().max(0.0) as usize;
            let x_to = ((sx + FILTER).floor() as isize).min(input.width as isize - 1);
            let y_from = (sy - FILTER).ceil().max(0.0) as usize;
            let y_to = ((sy + FILTER).floor() as isize).min(input.height as isize - 1);
            for ix in x_from..=(x_to.max(x_from as isize) as usize) {
                for iy in y_from..=(y_to.max(y_from as isize) as usize) {
                    if ix >= input.width || iy >= input.height {
                        continue;
                    }
                    let fx = (sx - ix as f32).abs();
                    let fy = (sy - iy as f32).abs();
                    let value = (1.0 - fx) * (1.0 - fy);
                    sum += input.at(ix, iy) * value;
                    weight += value;
                }
            }
            output.data[y * out_w + x] = if weight != 0.0 { sum / weight } else { 0.0 };
        }
    }
    output
}

/// Gradient magnitude at a pyramid level, and the level's mean gradient.
///
/// The divider is `2^(k+1)`, which is the level's own pixel spacing: a gradient measured between
/// neighbours on a grid halved `k` times spans `2^k` original pixels, and the central difference
/// spans two of those.
fn calculate_gradients(
    input: &[f32],
    width: usize,
    height: usize,
    level: usize,
) -> (Vec<f32>, f32) {
    let divider = 2.0_f32.powi(level as i32 + 1);
    let mut output = vec![0.0_f32; width * height];
    let mut total = 0.0_f32;
    for y in 0..height {
        for x in 0..width {
            let w = if x == 0 { 0 } else { x - 1 };
            let e = if x + 1 == width { x } else { x + 1 };
            let n = if y == 0 { 0 } else { y - 1 };
            let s = if y + 1 == height { y } else { y + 1 };
            // West MINUS east and south MINUS north, which is upstream's sign and the opposite of
            // the usual convention. It cannot matter: only the magnitude is taken.
            let gx = (input[y * width + w] - input[y * width + e]) / divider;
            let gy = (input[s * width + x] - input[n * width + x]) / divider;
            let magnitude = (gx * gx + gy * gy).sqrt();
            output[y * width + x] = magnitude;
            total += magnitude;
        }
    }
    (output, total / (width * height) as f32)
}

/// The parameters the attenuation field needs, grouped.
///
/// Grouped because clippy counted eight arguments, and the grouping is the honest one: `alpha`,
/// `beta` and `noise` are the operator's dialog values while the extent and the pyramid are the
/// data. Two different kinds of thing were in one list.
struct Attenuation {
    width: usize,
    height: usize,
    levels: usize,
    alpha: f32,
    beta: f32,
    noise: f32,
}

/// The attenuation field, accumulated from the coarsest level to the finest.
///
/// Each level contributes `a/(G+n) * ((G+n)/a)^beta` with `a = alpha * mean(G)`, and the running
/// product is upsampled and blurred on the way down. A gradient at or below `1e-4` contributes
/// exactly 1 — upstream's guard, and the reason a flat image comes out of here untouched rather
/// than divided by zero.
fn fi_matrix(gradients: &[Vec<f32>], averages: &[f32], params: &Attenuation) -> Vec<f32> {
    let Attenuation {
        width,
        height,
        levels,
        alpha,
        beta,
        noise,
    } = *params;
    let (top_w, top_h) = level_extent(width, height, levels - 1);
    let mut fi = vec![1.0_f32; top_w * top_h];
    let mut fi_w = top_w;
    let mut fi_h = top_h;

    for level in (0..levels).rev() {
        let (lw, lh) = level_extent(width, height, level);
        let a = alpha * averages[level];
        for index in 0..(lw * lh) {
            let grad = gradients[level][index];
            let value = if grad > 1e-4 && a > 0.0 {
                a / (grad + noise) * ((grad + noise) / a).powf(beta)
            } else {
                1.0
            };
            fi[index] *= value;
        }
        fi_w = lw;
        fi_h = lh;

        if level > 0 {
            let (nw, nh) = level_extent(width, height, level - 1);
            let raised = upsample(&fi, lw, lh, nw, nh);
            fi = gaussian_blur(&raised, nw, nh);
            fi_w = nw;
            fi_h = nh;
        }
    }

    debug_assert_eq!(
        (fi_w, fi_h),
        (width, height),
        "the attenuation field must end at the full extent"
    );
    fi
}

/// The 5-point Laplacian with Neumann boundaries — upstream's `atimes`.
///
/// Interior pixels take four neighbours and `-4` of themselves; an edge takes three and `-3`; a
/// corner two and `-2`. Writing it as "clamp the read and always use -4" is NOT the same operator:
/// that double-counts the repeated neighbour and is no longer symmetric, which would break the
/// conjugate-gradient argument this module rests on.
fn laplacian(u: &Level) -> Vec<f32> {
    let (w, h) = (u.width, u.height);
    let mut out = vec![0.0_f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0_f32;
            let mut count = 0.0_f32;
            if x > 0 {
                sum += u.at(x - 1, y);
                count += 1.0;
            }
            if x + 1 < w {
                sum += u.at(x + 1, y);
                count += 1.0;
            }
            if y > 0 {
                sum += u.at(x, y - 1);
                count += 1.0;
            }
            if y + 1 < h {
                sum += u.at(x, y + 1);
                count += 1.0;
            }
            out[y * w + x] = sum - count * u.at(x, y);
        }
    }
    out
}

/// Jacobi-preconditioned conjugate gradient, `SOLVER_STEPS` iterations, in place on `u`.
///
/// See the module header for why this is CG and not the biconjugate gradient upstream calls: the
/// matrix is symmetric, so the two methods generate the same iterates.
///
/// The preconditioner divides by the diagonal, which for an interior pixel is `-4`. Upstream's
/// `asolve` MULTIPLIES by `-4` instead, which is not the Jacobi preconditioner it is labelled as.
/// It only scales the search direction, so it changes the convergence rate rather than the fixed
/// point, and this is recorded rather than reproduced.
fn smooth(u: &mut Level, f: &[f32]) {
    let n = u.data.len();
    if n == 0 {
        return;
    }

    let mut residual = vec![0.0_f32; n];
    let au = laplacian(u);
    for i in 0..n {
        residual[i] = f[i] - au[i];
    }

    let precondition = |v: &[f32]| -> Vec<f32> { v.iter().map(|value| value / -4.0).collect() };

    let mut z = precondition(&residual);
    let mut direction = z.clone();
    let mut rz: f32 = residual.iter().zip(&z).map(|(r, zz)| r * zz).sum();

    for _ in 0..SOLVER_STEPS {
        if !rz.is_finite() || rz == 0.0 {
            break;
        }
        let probe = Level {
            data: direction.clone(),
            width: u.width,
            height: u.height,
        };
        let ad = laplacian(&probe);
        let denominator: f32 = direction.iter().zip(&ad).map(|(d, a)| d * a).sum();
        if denominator == 0.0 || !denominator.is_finite() {
            break;
        }
        let step = rz / denominator;
        for i in 0..n {
            u.data[i] += step * direction[i];
            residual[i] -= step * ad[i];
        }
        z = precondition(&residual);
        let rz_next: f32 = residual.iter().zip(&z).map(|(r, zz)| r * zz).sum();
        if !rz_next.is_finite() {
            break;
        }
        let beta = rz_next / rz;
        for i in 0..n {
            direction[i] = z[i] + beta * direction[i];
        }
        rz = rz_next;
    }
}

/// `D = F - laplacian(U)`, upstream's `fattal02_calculate_defect`.
fn calculate_defect(u: &Level, f: &[f32]) -> Vec<f32> {
    let au = laplacian(u);
    f.iter().zip(&au).map(|(fv, av)| fv - av).collect()
}

/// Solve `laplacian(U) = F` by nested multigrid iteration, upstream's structure step for step.
///
/// The coarsest-level solve is **zeros**, which is upstream's `fattal02_exact_solution` — its own
/// comment records that the over-relaxation belonging there was removed for giving incorrect
/// results. It is tolerable precisely because the Neumann Laplacian is singular: an additive
/// constant in `U` is removed by the rescaling after the exponential.
fn solve_pde_multigrid(divergence: &[f32], width: usize, height: usize) -> Vec<f32> {
    let mut requested_levels = 0_usize;
    {
        let mut mins = width.min(height);
        while mins >= MULTIGRID_MINIMUM {
            requested_levels += 1;
            mins /= 2;
        }
    }
    // `levels` counts the COARSER grids below the fine one, so a small image gets none and the
    // solve below reduces to zeros -- which is upstream's behaviour, not a shortcut here.
    let mut rhs: Vec<Level> = Vec::with_capacity(requested_levels + 1);
    rhs.push(Level {
        data: divergence.to_vec(),
        width,
        height,
    });
    for level in 0..requested_levels {
        let (nw, nh) = level_extent(width, height, level + 1);
        if nw == 0 || nh == 0 {
            break;
        }
        let coarser = restrict(&rhs[level], nw, nh);
        rhs.push(coarser);
    }
    let levels = rhs.len() - 1;

    let mut iu: Vec<Level> = (0..=levels)
        .map(|level| {
            let (lw, lh) = level_extent(width, height, level);
            Level::zeros(lw.max(1), lh.max(1))
        })
        .collect();
    let mut vf: Vec<Vec<f32>> = (0..=levels)
        .map(|level| vec![0.0; iu[level].data.len()])
        .collect();

    // Step 2: the coarsest grid's "exact" solution.
    iu[levels].data.iter_mut().for_each(|value| *value = 0.0);

    for k in (0..levels).rev() {
        // Step 4: carry the coarser solution up.
        let raised = prolongate(&iu[k + 1], iu[k].width, iu[k].height);
        iu[k] = raised;
        vf[k].copy_from_slice(&rhs[k].data);

        for _cycle in 0..V_CYCLES {
            // Step 6: downward stroke.
            for k2 in k..levels {
                if k2 != k {
                    iu[k2].data.iter_mut().for_each(|value| *value = 0.0);
                }
                for _ in 0..SMOOTH_ITERATIONS {
                    let target = vf[k2].clone();
                    smooth(&mut iu[k2], &target);
                }
                let defect = calculate_defect(&iu[k2], &vf[k2]);
                let defect_level = Level {
                    data: defect,
                    width: iu[k2].width,
                    height: iu[k2].height,
                };
                let coarser = restrict(&defect_level, iu[k2 + 1].width, iu[k2 + 1].height);
                vf[k2 + 1].copy_from_slice(&coarser.data);
            }

            // Step 10: coarsest solve again, on the restricted defect.
            iu[levels].data.iter_mut().for_each(|value| *value = 0.0);

            // Step 11: upward stroke.
            for k2 in (k..levels).rev() {
                let correction = prolongate(&iu[k2 + 1], iu[k2].width, iu[k2].height);
                for (value, delta) in iu[k2].data.iter_mut().zip(&correction.data) {
                    *value += delta;
                }
                for _ in 0..SMOOTH_ITERATIONS {
                    let target = vf[k2].clone();
                    smooth(&mut iu[k2], &target);
                }
            }
        }
    }

    iu.swap_remove(0).data
}

/// The value at a percentile of the sorted samples, upstream's `fattal02_find_percentiles`.
fn percentile(sorted: &[f32], fraction: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f32 * fraction) as usize).min(sorted.len() - 1);
    sorted[index]
}

/// Why [`tonemap`] declined to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FattalRefusal {
    /// Every luminance sample is zero, so the log-luminance normalisation divides by zero.
    NoLuminance,
}

/// Tone-map a luminance plane, returning the compressed luminance.
///
/// `luminance` is LINEAR luminance, as upstream's babl `Y float` is. The returned plane is the
/// `L` the colour step multiplies back in.
pub(crate) fn tonemap(
    luminance: &[f32],
    width: usize,
    height: usize,
    alpha: f32,
    beta: f32,
    noise: f32,
) -> Result<Vec<f32>, FattalRefusal> {
    let size = width * height;
    if size == 0 {
        return Err(FattalRefusal::NoLuminance);
    }

    let max_input = luminance.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if max_input <= 0.0 || !max_input.is_finite() {
        // Upstream divides by this without checking, which on a black layer is a division by zero
        // followed by a NaN image. Refusing is the honest form of the same stop.
        return Err(FattalRefusal::NoLuminance);
    }

    // Normalised by the MAXIMUM ONLY, to 0..100, then the logarithm. Not a min-max stretch: the
    // dark end keeps its absolute relationship to the bright end, which is what the operator is
    // about.
    let h: Vec<f32> = luminance
        .iter()
        .map(|y| (100.0 * y / max_input + LOG_EPSILON).ln())
        .collect();

    // The attenuation pyramid's depth: halve while the SMALLER side still leaves a level at or
    // above MINIMUM_PYRAMID. An image too small for any level still gets one.
    let mut levels = 0_usize;
    {
        let mut min_size = width.min(height);
        while min_size / 2 >= MINIMUM_PYRAMID {
            levels += 1;
            min_size /= 2;
        }
    }
    if levels == 0 {
        levels = 1;
    }

    let mut pyramid: Vec<Vec<f32>> = Vec::with_capacity(levels);
    pyramid.push(h.clone());
    let mut blur = gaussian_blur(&h, width, height);
    for level in 1..levels {
        let (pw, ph) = level_extent(width, height, level - 1);
        let (lw, lh) = level_extent(width, height, level);
        let down = downsample(&blur, pw, ph);
        blur = gaussian_blur(&down, lw, lh);
        pyramid.push(down);
    }

    let mut gradients = Vec::with_capacity(levels);
    let mut averages = Vec::with_capacity(levels);
    for (level, plane) in pyramid.iter().enumerate() {
        let (lw, lh) = level_extent(width, height, level);
        let (grad, average) = calculate_gradients(plane, lw, lh, level);
        gradients.push(grad);
        averages.push(average);
    }

    let fi = fi_matrix(
        &gradients,
        &averages,
        &Attenuation {
            width,
            height,
            levels,
            alpha,
            beta,
            noise,
        },
    );

    // The attenuated gradient field. The forward difference clamps at the far edge, so the last
    // row and column have a zero gradient by construction.
    let mut gx = vec![0.0_f32; size];
    let mut gy = vec![0.0_f32; size];
    for y in 0..height {
        for x in 0..width {
            let e = if x + 1 == width { x } else { x + 1 };
            let s = if y + 1 == height { y } else { y + 1 };
            gx[y * width + x] = (h[y * width + e] - h[y * width + x]) * fi[y * width + x];
            gy[y * width + x] = (h[s * width + x] - h[y * width + x]) * fi[y * width + x];
        }
    }

    let mut divergence = vec![0.0_f32; size];
    for y in 0..height {
        for x in 0..width {
            let mut value = gx[y * width + x] + gy[y * width + x];
            if x > 0 {
                value -= gx[y * width + x - 1];
            }
            if y > 0 {
                value -= gy[(y - 1) * width + x];
            }
            divergence[y * width + x] = value;
        }
    }

    let u = solve_pde_multigrid(&divergence, width, height);
    let mut output: Vec<f32> = u.iter().map(|value| value.exp() - LOG_EPSILON).collect();

    // Upstream clips the extremes by PERCENTILE rather than by the true min and max -- 0.1% and
    // 99.5% -- so a handful of outlying pixels cannot set the whole image's scale.
    let mut sorted = output.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let min = percentile(&sorted, 0.001);
    let max = percentile(&sorted, 0.995);
    let range = max - min;
    if range > 0.0 && range.is_finite() {
        for value in output.iter_mut() {
            *value = (*value - min) / range;
            if *value <= 0.0 {
                *value = LOG_EPSILON;
            }
        }
    }
    // A zero range means every recovered sample is identical, where upstream divides by zero and
    // writes NaN across the image. Leaving the values alone is the same divergence `reinhard05`
    // makes for the same reason.

    Ok(output)
}
