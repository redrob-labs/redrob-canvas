// SPDX-License-Identifier: GPL-3.0-or-later

//! Mantiuk, Myszkowski and Seidel 2006 contrast-domain tone mapping (K.10).
//!
//! Ported from `gegl/operations/common/mantiuk06.c` at the `[gegl_operations]` pin, itself derived
//! from pfstmo. GIMP only NAMES this operator. At 1654 lines it is the largest single filter in
//! this backlog.
//!
//! # What it does, and why it is not `fattal02` with different constants
//!
//! `fattal02` attenuates gradients of log-luminance directly. This one works in a PERCEPTUAL
//! contrast domain: the log-luminance gradients are pushed through a human-contrast transducer,
//! scaled there, and pushed back. The quantity being compressed is a response, not a difference,
//! and the transducer is the part that cannot be derived from the operator's name.
//!
//! ```text
//! clip        Y and RGB at 1e-7 * max Y        so the logarithm has a floor
//! rgb        /= Y                              colour becomes a ratio
//! Y           = log10(Y)
//! G_k         = forward differences of Y       per pyramid level, 0 at the far edge
//! R_k         = T(10^|G_k| - 1)                gradient -> stimulus -> response
//! R_k        *= contrast                       the compression, in response space
//! G_k         = log10(|T^-1(R_k)| + 1)         back again
//! G_k        /= C_k                            C = 1 / (a * max(detectT, |G|)^b)
//! b           = sum of divergences over levels coarse maps upsampled and added
//! solve       lap Y = b                        conjugate gradient, 200 steps, tol 1e-3
//! Y           = rescale to 2.3 log10 units     by 0.1% percentiles, then 10^Y
//! rgb         = rgb^saturation * Y
//! ```
//!
//! # THE TRANSDUCER TABLE IS COPIED, and that is a licence fact rather than a detail
//!
//! [`W_TABLE`] is 107 measured values taken verbatim from upstream. It is the inverse of the
//! transducer's response curve sampled on a uniform response grid, and it comes from the 2006
//! paper's psychophysics — there is no closed form anywhere in GEGL's tree to re-derive it from.
//! Inventing a curve and shipping it under three researchers' names is precisely what K.10's
//! attribution blocker forbade, so the honest options were "copy with attribution" or "do not ship
//! the operator".
//!
//! Copying is lawful here: GEGL's `operations/common/` is LGPL-3.0-or-later and this repository is
//! GPL-3.0-or-later, which is the permitted inbound direction. What it is NOT is free of
//! obligations, so the paperwork is done rather than skipped — `docs/upstream-sources.toml` carries
//! a separate `[gegl_transducer]` entry with `kind = "code"` and a declared boundary, and
//! `UPSTREAM_NOTICES.md` records the pinned commit. `scripts/verify-upstream.sh` enforces both.
//! The entry is separate from `[gegl_operations]` because one entry cannot say "nothing is copied"
//! and "this table is" at the same time.
//!
//! **The companion response table is NOT copied, because it is derivable.** Measured against
//! upstream's own 107 values it is `i / 106` to within 5e-7 — a uniform ramp rounded to six
//! decimals — so [`response_table`] computes it.
//!
//! # No Numerical Recipes code is involved, unlike `fattal02`
//!
//! This file also carries a `linbcg` derived from Numerical Recipes, but **upstream's `process`
//! passes `bcg = FALSE`**, so the shipped path never calls it. The live solver is `lincg`: plain,
//! UNPRECONDITIONED conjugate gradient — `p = r`, not `p = M^-1 r` — with a restart-on-divergence
//! guard and a best-so-far snapshot. That is what is ported, and it needs no derivation argument at
//! all.
//!
//! # `detail` is a DEAD property upstream
//!
//! `property_double (detail, _("Detail"), 1.0)` is declared with a description and a
//! `value_range (1.0, 99.0)`, and `o->detail` appears NOWHERE else in the file — it is never passed
//! to `contmap` and never read. So it is not exposed here. A parameter that does nothing is worse
//! than an absent one: a user who moves it and sees no change learns that this product's controls
//! lie. `audit4` is told about it by name so the resulting one-parameter gap reads as a decision
//! rather than an oversight.

/// Smallest pyramid dimension upstream will build a level for.
const PYRAMID_MIN_PIXELS: usize = 3;

/// Conjugate-gradient iteration cap, from upstream's `process` call site.
const SOLVER_MAX_ITERATIONS: usize = 200;

/// Convergence tolerance, from the same call site.
const SOLVER_TOLERANCE: f32 = 1e-3;

/// Consecutive non-improving iterations before the solver restarts from its best snapshot.
const BACKWARDS_CEILING: usize = 3;

/// The display's assumed dynamic range in log10 units, from `contmap`.
const DISPLAY_DYNAMIC_RANGE: f32 = 2.3;

/// Percentile margin, in percent, for the final rescale. Upstream's `CUT_MARGIN`.
const CUT_MARGIN: f64 = 0.1;

/// Gradient magnitude below which the scale factor stops shrinking. Upstream's `detectT`.
const DETECT_THRESHOLD: f32 = 0.001;

/// First constant of the contrast-sensitivity scale factor `C = 1 / (a * g^b)`.
const SCALE_A: f32 = 0.038737;

/// Second constant of the same.
const SCALE_B: f32 = 0.537756;

/// The transducer's stimulus values, **COPIED verbatim from upstream**.
///
/// This is the one piece of this repository taken from GEGL rather than re-derived; see the module
/// header for why, and `docs/upstream-sources.toml`'s `[gegl_transducer]` entry plus
/// `UPSTREAM_NOTICES.md` for the attribution the copy carries.
///
/// # Why `excessive_precision` is allowed here
///
/// Clippy is right that an `f32` cannot hold `1081.727632`'s last digits. Trimming them would be
/// the wrong fix: this array's value is that it is byte-for-byte what upstream wrote, which is the
/// claim the notices file makes and the thing a future re-sync diffs against. The digits are
/// PROVENANCE, not precision — rounding them to what `f32` can represent would leave a table that
/// no longer matches its source while still claiming to.
#[allow(clippy::excessive_precision)]
const W_TABLE: [f32; 107] = [
    0.000000,
    0.010000,
    0.021180,
    0.031830,
    0.042628,
    0.053819,
    0.065556,
    0.077960,
    0.091140,
    0.105203,
    0.120255,
    0.136410,
    0.153788,
    0.172518,
    0.192739,
    0.214605,
    0.238282,
    0.263952,
    0.291817,
    0.322099,
    0.355040,
    0.390911,
    0.430009,
    0.472663,
    0.519238,
    0.570138,
    0.625811,
    0.686754,
    0.753519,
    0.826720,
    0.907041,
    0.995242,
    1.092169,
    1.198767,
    1.316090,
    1.445315,
    1.587756,
    1.744884,
    1.918345,
    2.109983,
    2.321863,
    2.556306,
    2.815914,
    3.103613,
    3.422694,
    3.776862,
    4.170291,
    4.607686,
    5.094361,
    5.636316,
    6.240338,
    6.914106,
    7.666321,
    8.506849,
    9.446889,
    10.499164,
    11.678143,
    13.000302,
    14.484414,
    16.151900,
    18.027221,
    20.138345,
    22.517282,
    25.200713,
    28.230715,
    31.655611,
    35.530967,
    39.920749,
    44.898685,
    50.549857,
    56.972578,
    64.280589,
    72.605654,
    82.100619,
    92.943020,
    105.339358,
    119.530154,
    135.795960,
    154.464484,
    175.919088,
    200.608905,
    229.060934,
    261.894494,
    299.838552,
    343.752526,
    394.651294,
    453.735325,
    522.427053,
    602.414859,
    695.706358,
    804.693100,
    932.229271,
    1081.727632,
    1257.276717,
    1463.784297,
    1707.153398,
    1994.498731,
    2334.413424,
    2737.298517,
    3215.770944,
    3785.169959,
    4464.187290,
    5275.653272,
    6247.520102,
    7414.094945,
    8817.590551,
    10510.080619,
];

/// The transducer's response values, DERIVED rather than copied.
///
/// Upstream ships a second 107-entry table beside [`W_TABLE`]; measured, every entry is `i / 106`
/// to within 5e-7. A uniform ramp is not data worth copying, so this computes it — which also
/// means a future reader can see at a glance that the response axis is uniform, where a table of
/// printed decimals hides it.
fn response_table() -> [f32; 107] {
    let mut table = [0.0_f32; 107];
    for (index, value) in table.iter_mut().enumerate() {
        *value = index as f32 / 106.0;
    }
    table
}

/// Piecewise-linear interpolation of `value` through `(input, output)`, upstream's
/// `mantiuk06_lookup_table`.
///
/// Both ends SATURATE rather than extrapolate: below the first input the first output is returned,
/// above the last the last. That is upstream's behaviour and it is what keeps an out-of-gamut
/// gradient from being mapped to a nonsense response.
fn lookup(input: &[f32; 107], output: &[f32; 107], value: f32) -> f32 {
    if value < input[0] {
        return output[0];
    }
    for index in 1..107 {
        if value < input[index] {
            let span = input[index] - input[index - 1];
            if span == 0.0 {
                return output[index - 1];
            }
            let fraction = (value - input[index - 1]) / span;
            return output[index - 1] + (output[index] - output[index - 1]) * fraction;
        }
    }
    output[106]
}

/// One pyramid level: the two gradient planes and the extent they live on.
struct GradientLevel {
    gx: Vec<f32>,
    gy: Vec<f32>,
    cols: usize,
    rows: usize,
}

impl GradientLevel {
    fn len(&self) -> usize {
        self.cols * self.rows
    }
}

/// The level extents, halving while both sides stay at or above [`PYRAMID_MIN_PIXELS`].
fn pyramid_extents(cols: usize, rows: usize) -> Vec<(usize, usize)> {
    let mut extents = Vec::new();
    let (mut c, mut r) = (cols, rows);
    while r >= PYRAMID_MIN_PIXELS && c >= PYRAMID_MIN_PIXELS {
        extents.push((c, r));
        c /= 2;
        r /= 2;
    }
    extents
}

/// Forward differences, with a hard zero on the far edge.
///
/// Not a clamped read: upstream writes an explicit `0` in the last column and row, so the gradient
/// field genuinely has no component there. The divergence that follows depends on it.
fn calculate_gradient(lum: &[f32], cols: usize, rows: usize) -> (Vec<f32>, Vec<f32>) {
    let mut gx = vec![0.0_f32; cols * rows];
    let mut gy = vec![0.0_f32; cols * rows];
    for y in 0..rows {
        for x in 0..cols {
            let index = y * cols + x;
            gx[index] = if x + 1 == cols {
                0.0
            } else {
                lum[index + 1] - lum[index]
            };
            gy[index] = if y + 1 == rows {
                0.0
            } else {
                lum[index + cols] - lum[index]
            };
        }
    }
    (gx, gy)
}

/// Area-integrating halving, upstream's `mantiuk06_matrix_downsample`.
///
/// Credited in upstream to Ed Brambley and described there as integrating over each output pixel
/// to find the average of what shows through. For an EVEN input this reduces to the 2x2 mean; it
/// is written generally because the pyramid halves by integer division, so an odd dimension gives
/// genuinely fractional edge weights and the 2x2 shortcut would drift.
fn downsample(data: &[f32], in_cols: usize, in_rows: usize) -> Vec<f32> {
    let out_cols = in_cols / 2;
    let out_rows = in_rows / 2;
    if out_cols == 0 || out_rows == 0 {
        return Vec::new();
    }
    let dx = in_cols as f32 / out_cols as f32;
    let dy = in_rows as f32 / out_rows as f32;
    let normalise = 1.0 / (dx * dy);
    let mut out = vec![0.0_f32; out_cols * out_rows];

    for y in 0..out_rows {
        let iy1 = (y * in_rows) / out_rows;
        let iy2 = ((y + 1) * in_rows) / out_rows;
        let fy1 = (iy1 + 1) as f32 - y as f32 * dy;
        let fy2 = (y + 1) as f32 * dy - iy2 as f32;

        for x in 0..out_cols {
            let ix1 = (x * in_cols) / out_cols;
            let ix2 = ((x + 1) * in_cols) / out_cols;
            let fx1 = (ix1 + 1) as f32 - x as f32 * dx;
            let fx2 = (x + 1) as f32 * dx - ix2 as f32;

            let mut total = 0.0_f32;
            let mut row = iy1;
            while row <= iy2 && row < in_rows {
                let factor_y = if row == iy1 {
                    fy1
                } else if row == iy2 {
                    fy2
                } else {
                    1.0
                };
                let mut col = ix1;
                while col <= ix2 && col < in_cols {
                    let factor_x = if col == ix1 {
                        fx1
                    } else if col == ix2 {
                        fx2
                    } else {
                        1.0
                    };
                    total += data[row * in_cols + col] * factor_x * factor_y;
                    col += 1;
                }
                row += 1;
            }
            out[y * out_cols + x] = total * normalise;
        }
    }
    out
}

/// Bilinear doubling, upstream's `mantiuk06_matrix_upsample`.
///
/// Upstream's comment calls the `1/(dx*dy)` factor *"a genuine upsampling matrix, not the transpose
/// of the downsampling matrix"*, and records that the theoretically better factor of 1 is the one
/// it did NOT take. Kept as written: this is the behaviour, and the alternative is commented out
/// upstream rather than chosen.
fn upsample(data: &[f32], out_cols: usize, out_rows: usize) -> Vec<f32> {
    let in_cols = out_cols / 2;
    let in_rows = out_rows / 2;
    let mut out = vec![0.0_f32; out_cols * out_rows];
    if in_cols == 0 || in_rows == 0 {
        return out;
    }
    let dx = in_cols as f32 / out_cols as f32;
    let dy = in_rows as f32 / out_rows as f32;
    let factor = 1.0 / (dx * dy);

    for y in 0..out_rows {
        let sy = y as f32 * dy;
        let iy1 = (y * in_rows) / out_rows;
        let iy2 = (((y + 1) * in_rows) / out_rows).min(in_rows - 1);
        for x in 0..out_cols {
            let sx = x as f32 * dx;
            let ix1 = (x * in_cols) / out_cols;
            let ix2 = (((x + 1) * in_cols) / out_cols).min(in_cols - 1);

            let wx1 = (ix1 + 1) as f32 - sx;
            let wx2 = sx + dx - (ix1 + 1) as f32;
            let wy1 = (iy1 + 1) as f32 - sy;
            let wy2 = sy + dy - (iy1 + 1) as f32;

            out[y * out_cols + x] = (wx1 * wy1 * data[iy1 * in_cols + ix1]
                + wx1 * wy2 * data[iy2 * in_cols + ix1]
                + wx2 * wy1 * data[iy1 * in_cols + ix2]
                + wx2 * wy2 * data[iy2 * in_cols + ix2])
                * factor;
        }
    }
    out
}

/// `divG += div(Gx, Gy)`, upstream's `mantiuk06_calculate_and_add_divergence`.
///
/// The first column and row take the gradient itself rather than a difference, which is the
/// Neumann boundary written into the divergence instead of into the operator.
fn add_divergence(level: &GradientLevel, target: &mut [f32]) {
    for y in 0..level.rows {
        for x in 0..level.cols {
            let index = y * level.cols + x;
            let dx = if x == 0 {
                level.gx[index]
            } else {
                level.gx[index] - level.gx[index - 1]
            };
            let dy = if y == 0 {
                level.gy[index]
            } else {
                level.gy[index] - level.gy[index - level.cols]
            };
            target[index] += dx + dy;
        }
    }
}

/// The divergence summed over the pyramid, coarse maps upsampled into finer ones.
fn divergence_sum(pyramid: &[GradientLevel]) -> Vec<f32> {
    let mut accumulated: Vec<f32> = Vec::new();
    for (index, level) in pyramid.iter().enumerate().rev() {
        let mut target = if index + 1 == pyramid.len() {
            vec![0.0_f32; level.len()]
        } else {
            upsample(&accumulated, level.cols, level.rows)
        };
        add_divergence(level, &mut target);
        accumulated = target;
    }
    accumulated
}

/// The gradients of `x` on every pyramid level, scaled by the stored `C`.
///
/// This is upstream's `multiplyA`: the operator the solver applies is not a bare Laplacian but
/// "take gradients at every scale, weight them by the contrast sensitivity, sum the divergences".
fn multiply_a(x: &[f32], scale: &[GradientLevel]) -> Vec<f32> {
    let mut levels: Vec<GradientLevel> = Vec::with_capacity(scale.len());
    let mut current = x.to_vec();
    for (index, level) in scale.iter().enumerate() {
        if index > 0 {
            let (pc, pr) = (scale[index - 1].cols, scale[index - 1].rows);
            current = downsample(&current, pc, pr);
        }
        let (mut gx, mut gy) = calculate_gradient(&current, level.cols, level.rows);
        for i in 0..level.len() {
            gx[i] *= level.gx[i];
            gy[i] *= level.gy[i];
        }
        levels.push(GradientLevel {
            gx,
            gy,
            cols: level.cols,
            rows: level.rows,
        });
    }
    divergence_sum(&levels)
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Unpreconditioned conjugate gradient with a restart guard, upstream's `mantiuk06_lincg`.
///
/// Two details are upstream's and neither is ornamental. It keeps a **best-so-far snapshot** and
/// returns that rather than the last iterate, so a solve that goes unstable late cannot make the
/// result worse than the one it already had. And after [`BACKWARDS_CEILING`] consecutive
/// non-improving steps it **restarts** from that snapshot with a freshly computed residual, which
/// is what keeps the loss of orthogonality in a long single-precision CG from derailing it.
fn solve(b: &[f32], scale: &[GradientLevel], x: &mut [f32]) {
    let n = b.len();
    if n == 0 {
        return;
    }
    let bnrm2 = dot(b, b);
    if !bnrm2.is_finite() {
        return;
    }
    let tolerance = SOLVER_TOLERANCE * SOLVER_TOLERANCE;

    let ax = multiply_a(x, scale);
    let mut residual: Vec<f32> = b.iter().zip(&ax).map(|(bv, av)| bv - av).collect();
    let mut direction = residual.clone();
    let mut rdotr = dot(&residual, &residual);
    let mut saved_rdotr = rdotr;
    let mut best = x.to_vec();
    let mut backwards = 0_usize;

    for _ in 0..SOLVER_MAX_ITERATIONS {
        if rdotr / bnrm2 < tolerance || !rdotr.is_finite() {
            break;
        }
        let ap = multiply_a(&direction, scale);
        let denominator = dot(&direction, &ap);
        if denominator == 0.0 || !denominator.is_finite() {
            break;
        }
        let alpha = rdotr / denominator;
        let old_rdotr = rdotr;

        for i in 0..n {
            residual[i] -= alpha * ap[i];
        }
        rdotr = dot(&residual, &residual);

        if rdotr < saved_rdotr {
            saved_rdotr = rdotr;
            for i in 0..n {
                best[i] = x[i] + alpha * direction[i];
            }
            backwards = 0;
        } else {
            backwards += 1;
        }

        for i in 0..n {
            x[i] += alpha * direction[i];
        }

        if rdotr / bnrm2 < tolerance {
            break;
        }

        if backwards > BACKWARDS_CEILING {
            backwards = 0;
            x.copy_from_slice(&best);
            let ax = multiply_a(x, scale);
            for i in 0..n {
                residual[i] = b[i] - ax[i];
            }
            rdotr = dot(&residual, &residual);
            saved_rdotr = rdotr;
            direction.copy_from_slice(&residual);
        } else {
            let beta = rdotr / old_rdotr;
            for i in 0..n {
                direction[i] = residual[i] + beta * direction[i];
            }
        }
    }

    // Upstream's last act: prefer the snapshot when the final iterate is worse than it.
    if rdotr > saved_rdotr {
        x.copy_from_slice(&best);
    }
}

/// Gradient to contrast response, upstream's `transform_to_R`.
///
/// Two steps with the sign carried separately: `G -> W` undoes the logarithm
/// (`W = (10^|G| - 1) * sign`), then `W -> R` runs the stimulus through the transducer. The sign is
/// re-read BETWEEN the steps upstream, which matters only because `10^|G| - 1` is never negative,
/// so the second read is of the sign just written.
fn transform_to_r(values: &mut [f32], w: &[f32; 107], r: &[f32; 107]) {
    for value in values.iter_mut() {
        let sign = if *value < 0.0 { -1.0 } else { 1.0 };
        let stimulus = (10.0_f32.powf(value.abs()) - 1.0) * sign;
        let sign = if stimulus < 0.0 { -1.0 } else { 1.0 };
        *value = sign * lookup(w, r, stimulus.abs());
    }
}

/// Contrast response back to gradient, upstream's `transform_to_G`: the inverse lookup, then the
/// logarithm.
fn transform_to_g(values: &mut [f32], w: &[f32; 107], r: &[f32; 107]) {
    for value in values.iter_mut() {
        let sign = if *value < 0.0 { -1.0 } else { 1.0 };
        let stimulus = sign * lookup(r, w, value.abs());
        let sign = if stimulus < 0.0 { -1.0 } else { 1.0 };
        *value = (stimulus.abs() + 1.0).log10() * sign;
    }
}

/// Upstream's contrast-equalisation branch, and why it is NOT implemented here.
///
/// # It is unreachable in any distinguishable way, and that is a proof rather than a guess
///
/// Upstream branches `contrastFactor > 0 ? gradient_multiply(contrastFactor)
/// : contrast_equalization(-contrastFactor)`, and `contrast`'s declared range is `0.0 .. 1.0`. So
/// the ONLY value that selects equalisation is exactly 0, which selects it with a factor of
/// `-0.0`. And there:
///
/// * `gradient_multiply(0)` sets every gradient to `g * 0` = **0**.
/// * `contrast_equalization(0)` sets every gradient to `g * (0 * cdf / magnitude)` = **0**.
///
/// The two branches produce an identical all-zero gradient field, so no reachable parameter value
/// can tell them apart. Implementing the histogram equalisation — a global sort across every
/// pyramid level — would add code that no test could ever discriminate, which is the thing this
/// loop's reverse-verification rule exists to prevent me from shipping.
///
/// **This was found BY a reverse-verification.** Widening the branch to `contrast >= 0.0`, so that
/// 0 takes the multiply path instead, left every test green — and the right conclusion was not
/// "the test is weak" but "the branch is unobservable".
///
/// It joins two other dead things in the same upstream file: the `detail` property, declared and
/// never read, and `linbcg`, present and never called because `process` passes `bcg = FALSE`.
///
/// If upstream ever widens `contrast`'s range below zero, this stops being dead and the
/// equalisation has to be written. The reference is `mantiuk06_contrast_equalization`: magnitudes
/// ranked across ALL levels together, scale `factor * cdf / magnitude`.
const _CONTRAST_EQUALISATION_IS_UNREACHABLE: () = ();

/// Why [`tonemap`] declined to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MantiukRefusal {
    /// No positive luminance anywhere, so the clip floor and the logarithm have nothing to work
    /// from.
    NoLuminance,
}

/// Tone-map in the contrast domain.
///
/// `luminance` and `rgb` are LINEAR, as upstream's babl buffers are. `rgb` is modified in place and
/// holds four components per pixel; the alpha component is left untouched here, and the caller
/// writes it back unchanged.
pub(crate) fn tonemap(
    rgb: &mut [f32],
    luminance: &mut [f32],
    cols: usize,
    rows: usize,
    contrast: f32,
    saturation: f32,
) -> Result<(), MantiukRefusal> {
    let n = cols * rows;
    if n == 0 || rgb.len() < n * 4 || luminance.len() < n {
        return Err(MantiukRefusal::NoLuminance);
    }

    let y_max = luminance.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if y_max <= 0.0 || !y_max.is_finite() {
        return Err(MantiukRefusal::NoLuminance);
    }

    // The clip floor is RELATIVE to the brightest pixel, which is what makes the logarithm safe
    // without choosing an absolute black level.
    let clip_min = 1e-7 * y_max;
    for value in rgb.iter_mut() {
        if *value < clip_min {
            *value = clip_min;
        }
    }
    for value in luminance.iter_mut() {
        if *value < clip_min {
            *value = clip_min;
        }
    }

    // Colour becomes a RATIO to the luminance, and the luminance becomes its own logarithm. Every
    // later step works on one or the other, never on the original samples.
    for index in 0..n {
        for channel in 0..3 {
            rgb[index * 4 + channel] /= luminance[index];
        }
        luminance[index] = luminance[index].log10();
    }

    let extents = pyramid_extents(cols, rows);
    let w = W_TABLE;
    let r = response_table();

    // The gradient pyramid, each level measured on a successively downsampled log-luminance.
    let mut pyramid: Vec<GradientLevel> = Vec::with_capacity(extents.len());
    let mut current = luminance[..n].to_vec();
    for (index, (lc, lr)) in extents.iter().copied().enumerate() {
        if index > 0 {
            let (pc, pr) = extents[index - 1];
            current = downsample(&current, pc, pr);
        }
        let (gx, gy) = calculate_gradient(&current, lc, lr);
        pyramid.push(GradientLevel {
            gx,
            gy,
            cols: lc,
            rows: lr,
        });
    }
    if pyramid.is_empty() {
        // Smaller than PYRAMID_MIN_PIXELS in a dimension: upstream builds no level at all and its
        // pyramid pointer is NULL, which its own code would then dereference. Refusing is the
        // honest reading of "this operator has nothing to work on".
        return Err(MantiukRefusal::NoLuminance);
    }

    for level in pyramid.iter_mut() {
        transform_to_r(&mut level.gx, &w, &r);
        transform_to_r(&mut level.gy, &w, &r);
    }

    // The compression itself, in response space. See `_CONTRAST_EQUALISATION_IS_UNREACHABLE` for
    // why upstream's other branch is not here: at the only value that selects it the two are
    // provably identical.
    for level in pyramid.iter_mut() {
        for value in level.gx.iter_mut().chain(level.gy.iter_mut()) {
            *value *= contrast;
        }
    }

    for level in pyramid.iter_mut() {
        transform_to_g(&mut level.gx, &w, &r);
        transform_to_g(&mut level.gy, &w, &r);
    }

    // The contrast-sensitivity weights, and then the gradients scaled by them. Both the operator
    // the solver applies and its right-hand side are built from the same weights, which is why
    // they are computed once here and handed to the solver.
    let scale: Vec<GradientLevel> = pyramid
        .iter()
        .map(|level| {
            let weight = |g: &f32| -> f32 {
                let magnitude = DETECT_THRESHOLD.max(g.abs());
                1.0 / (SCALE_A * magnitude.powf(SCALE_B))
            };
            GradientLevel {
                gx: level.gx.iter().map(weight).collect(),
                gy: level.gy.iter().map(weight).collect(),
                cols: level.cols,
                rows: level.rows,
            }
        })
        .collect();

    for (level, weights) in pyramid.iter_mut().zip(&scale) {
        for index in 0..level.len() {
            level.gx[index] *= weights.gx[index];
            level.gy[index] *= weights.gy[index];
        }
    }

    let b = divergence_sum(&pyramid);
    let mut solved = luminance[..n].to_vec();
    solve(&b, &scale, &mut solved);
    luminance[..n].copy_from_slice(&solved);

    // Renormalise to the display's range. The percentiles are INTERPOLATED between the two
    // neighbouring samples rather than taken at an index, which is upstream's `delta` arithmetic
    // and matters on a small image where one sample is a large share of the distribution.
    let mut sorted = luminance[..n].to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let interpolate = |percent: f64| -> f32 {
        let trim = (n - 1) as f64 * percent * 0.01;
        let delta = trim - trim.floor();
        let low = sorted[trim.floor() as usize];
        let high = sorted[(trim.ceil() as usize).min(n - 1)];
        (f64::from(low) * delta + f64::from(high) * (1.0 - delta)) as f32
    };
    let l_min = interpolate(CUT_MARGIN);
    let l_max = interpolate(100.0 - CUT_MARGIN);

    let span = l_max - l_min;
    for index in 0..n {
        let scaled = if span > 0.0 && span.is_finite() {
            (luminance[index] - l_min) / span * DISPLAY_DYNAMIC_RANGE - DISPLAY_DYNAMIC_RANGE
        } else {
            // A zero span means the solve came back flat; upstream divides by zero here. Mapping
            // to the bottom of the display range keeps the image rather than filling it with NaN,
            // the same divergence `reinhard05` and `fattal02` make at the same place.
            -DISPLAY_DYNAMIC_RANGE
        };
        let recovered = 10.0_f32.powf(scaled);
        luminance[index] = recovered;
        for channel in 0..3 {
            rgb[index * 4 + channel] = rgb[index * 4 + channel].powf(saturation) * recovered;
        }
    }

    Ok(())
}
