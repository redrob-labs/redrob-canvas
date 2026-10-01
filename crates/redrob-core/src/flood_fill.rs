//! Flood fill: filling a contiguous region of similar colour from a seed point.
//!
//! TRANSLATED from Krita, pinned at `fdbf33b2146735465bb8aa59928fbc1890ceb160`, GPL-2.0-or-later, taken
//! into this GPL-3.0-or-later product under the or-later grant:
//!
//! - `libs/image/floodfill/kis_scanline_fill.cpp` — the interval-based scanline walk
//! - `libs/image/KisColorSelectionPolicies.h` — the threshold and opacity-spread arithmetic
//! - `plugins/color/lcms2engine/LcmsColorSpace.h` — `differenceA`, the Lab colour distance
//!
//! Licence change notice: the originals are GPL-2.0-or-later; this file is GPL-3.0-or-later, which the
//! or-later grant permits. Attribution is in `UPSTREAM_NOTICES.md`.
//!
//! # Why this, out of a 7,503-line block
//!
//! Item 1c.2 block 5 is lazybrush and flood fill. Lazybrush is Krita's colourise mask, a multi-label
//! optimisation over a whole layer with no counterpart here. Flood fill is the tool every paint program
//! has and this product did not: it had `Fill`, which paints the whole layer or the selection, and no way
//! to fill a contiguous region from a click.
//!
//! # The tolerance is a Lab distance, and that had to be measured
//!
//! Krita compares colours with `differenceA`, which transforms both pixels to Lab through the document's
//! ICC profile and returns `sqrt(dL² + da² + db² + dAlpha²)` truncated to a byte. Not an RGB distance, and
//! the difference is not small: mid-grey 128 against 138 is 3 in Lab and 10 in RGB.
//!
//! This product's [`crate::Pixel`] is fixed sRGB with no profile, and this crate cannot use the optional
//! lcms2 adapter -- that is a C shim for the Qt layer. So the conversion is closed-form, which is only
//! honest if it agrees. Verified against real lcms2 2.x driving Krita's own transform: **exact agreement**
//! on every representative pair, and a maximum difference of 1 from rounding across 2,920 swept pairs.

use crate::Pixel;
use serde::{Deserialize, Serialize};

/// How a flood fill decides what to include.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FloodFillOptions {
    /// Krita's `threshold`: how far a pixel's Lab distance from the seed colour may be, 0 to 255.
    pub tolerance: u8,
    /// Krita's `opacitySpread`, 0 to 100. At 100 the edge is crisp; below it the boundary fades.
    ///
    /// Krita derives `softness = 100 - opacitySpread` and switches to its hard policy when softness
    /// reaches zero, so 100 is not merely "very little spread" but a different code path.
    pub opacity_spread: u8,
}

impl Default for FloodFillOptions {
    /// A crisp fill with a small tolerance, which is what a bucket tool does when first picked up.
    fn default() -> Self {
        Self {
            tolerance: 15,
            opacity_spread: 100,
        }
    }
}

impl FloodFillOptions {
    pub fn is_valid(&self) -> bool {
        self.opacity_spread <= 100
    }
}

/// The Lab distance between two sRGB pixels, in Krita's `differenceA` sense.
///
/// Verified against lcms2: exact on every representative pair, and within 1 across a 2,920-pair sweep.
pub fn colour_difference(a: Pixel, b: Pixel) -> u8 {
    // Krita returns an alpha-only distance when either pixel is fully transparent, before any colour
    // conversion. A transparent pixel has no meaningful colour to convert.
    if a.a == 0 || b.a == 0 {
        let scale = 100.0 / 255.0;
        return (scale * (f64::from(a.a) - f64::from(b.a)).abs()).round() as u8;
    }
    let (l1, a1, b1) = srgb_to_lab(a);
    let (l2, a2, b2) = srgb_to_lab(b);
    let d_l = (l1 - l2).abs();
    let d_a = (a1 - a2).abs();
    let d_b = (b1 - b2).abs();
    // Krita's Lab buffer holds alpha as a u16, so the 8-bit alpha is scaled by 257 and the term uses
    // 100/65535. That is 100/255 to within rounding, but the scaling is kept explicit to match.
    let alpha_scale = 100.0 / 65535.0;
    let d_alpha = ((f64::from(a.a) * 257.0) - (f64::from(b.a) * 257.0)).abs() * alpha_scale;
    let diff = (d_l * d_l + d_a * d_a + d_b * d_b + d_alpha * d_alpha).sqrt();
    // Truncated, not rounded: Krita casts a qreal to quint8.
    if diff > 255.0 { 255 } else { diff as u8 }
}

/// sRGB to CIE Lab with a D50 white point, which is what an ICC Lab profile connection space uses.
fn srgb_to_lab(pixel: Pixel) -> (f64, f64, f64) {
    let r = srgb_to_linear(f64::from(pixel.r) / 255.0);
    let g = srgb_to_linear(f64::from(pixel.g) / 255.0);
    let b = srgb_to_linear(f64::from(pixel.b) / 255.0);
    // The Bradford-adapted sRGB-to-XYZ matrix for a D50 connection space, which is the one ICC profiles
    // carry. Using the D65 matrix instead shifts every Lab value and the tolerance with it.
    let x = 0.436_074_7 * r + 0.385_064_9 * g + 0.143_080_4 * b;
    let y = 0.222_504_5 * r + 0.716_878_6 * g + 0.060_616_9 * b;
    let z = 0.013_932_2 * r + 0.097_104_5 * g + 0.714_173_3 * b;
    // D50 white.
    let fx = lab_f(x / 0.964_212);
    let fy = lab_f(y / 1.0);
    let fz = lab_f(z / 0.825_188);
    (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
}

fn srgb_to_linear(channel: f64) -> f64 {
    if channel <= 0.040_45 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn lab_f(t: f64) -> f64 {
    // The CIE constants as exact fractions, which is how they are defined.
    const EPSILON: f64 = 216.0 / 24389.0;
    const KAPPA: f64 = 24389.0 / 27.0;
    if t > EPSILON {
        t.cbrt()
    } else {
        (KAPPA * t + 16.0) / 116.0
    }
}

/// How much a pixel at the given distance is filled, 0 to 255.
///
/// Two policies, exactly Krita's, and the seams between them are the interesting part:
///
/// - The **hard** policy tests `difference <= threshold`; the **soft** one tests `difference < threshold`.
///   So at a distance exactly equal to the tolerance, a crisp fill includes the pixel and a soft fill does
///   not. That is upstream's inconsistency, kept because the golden harness compares against it.
/// - The soft policy with a threshold of 0 returns **nothing at all**, not even for an exact colour match,
///   where the hard policy returns full coverage. A soft fill with zero tolerance therefore fills nothing.
pub fn opacity_from_difference(difference: u8, tolerance: u8, opacity_spread: u8) -> u8 {
    let softness = 100i32 - i32::from(opacity_spread.min(100));
    if softness <= 0 {
        // Krita's hard policy.
        return if difference <= tolerance { 255 } else { 0 };
    }
    let threshold = i32::from(tolerance);
    if threshold == 0 {
        return 0;
    }
    if i32::from(difference) < threshold {
        // Krita's integer form of (threshold - difference) / (threshold * softness), scaled by 100 because
        // softness is a percentage.
        let v = (threshold - i32::from(difference)) * 255 * 100 / (threshold * softness);
        v.min(255) as u8
    } else {
        0
    }
}

/// A horizontal run queued for the next row, which is what makes this a scanline fill.
#[derive(Clone, Copy, Debug)]
struct Interval {
    start: i64,
    end: i64,
    row: i64,
}

/// The result of a fill: which pixels to cover, and how much.
#[derive(Clone, Debug, PartialEq)]
pub struct FillMask {
    pub x0: u32,
    pub y0: u32,
    pub width: u32,
    pub height: u32,
    /// Row-major coverage over the bounding box, 0 for untouched.
    pub coverage: Vec<u8>,
}

impl FillMask {
    pub fn coverage_at(&self, x: u32, y: u32) -> u8 {
        if x < self.x0 || y < self.y0 || x >= self.x0 + self.width || y >= self.y0 + self.height {
            return 0;
        }
        let local_x = (x - self.x0) as usize;
        let local_y = (y - self.y0) as usize;
        self.coverage[local_y * self.width as usize + local_x]
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Computes which pixels a flood fill from `(seed_x, seed_y)` would cover.
///
/// Returns the coverage rather than writing pixels, so the caller composites it the way it composites
/// everything else -- through the selection and the blend path -- and so this is testable without a
/// document.
///
/// `visit_budget` bounds the work, as every other bulk command here is bounded. A fill of a large
/// same-colour canvas legitimately touches every pixel, so the budget is a ceiling on pixel VISITS rather
/// than on the filled area.
pub fn flood_fill_mask(
    pixels: &[u8],
    width: u32,
    height: u32,
    seed_x: u32,
    seed_y: u32,
    options: FloodFillOptions,
    visit_budget: u64,
) -> Option<FillMask> {
    if width == 0 || height == 0 || seed_x >= width || seed_y >= height {
        return None;
    }
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if pixels.len() < expected {
        return None;
    }

    let at = |x: i64, y: i64| -> Pixel {
        let offset = ((y as usize) * width as usize + x as usize) * 4;
        Pixel::rgba(
            pixels[offset],
            pixels[offset + 1],
            pixels[offset + 2],
            pixels[offset + 3],
        )
    };
    let seed_colour = at(i64::from(seed_x), i64::from(seed_y));

    let mut coverage = vec![0u8; width as usize * height as usize];
    // Whether a pixel has been decided. Separate from `coverage`, because a pixel can be decided AND have
    // zero coverage -- a soft edge pixel below the threshold. Without this the walk revisits every such
    // pixel from each neighbouring run.
    let mut decided = vec![false; width as usize * height as usize];
    let mut visits = 0u64;

    let index = |x: i64, y: i64| -> usize { (y as usize) * width as usize + x as usize };

    // Coverage at a pixel, or None when it is not part of the fill.
    let consider = |x: i64, y: i64, visits: &mut u64| -> Option<u8> {
        *visits += 1;
        let difference = colour_difference(seed_colour, at(x, y));
        let opacity =
            opacity_from_difference(difference, options.tolerance, options.opacity_spread);
        if opacity == 0 { None } else { Some(opacity) }
    };

    let mut stack: Vec<Interval> = Vec::new();
    let mut min_x = i64::from(seed_x);
    let mut max_x = min_x;
    let mut min_y = i64::from(seed_y);
    let mut max_y = min_y;

    // The seed itself must be fillable, or there is no fill.
    let seed_opacity = consider(i64::from(seed_x), i64::from(seed_y), &mut visits)?;
    coverage[index(i64::from(seed_x), i64::from(seed_y))] = seed_opacity;
    decided[index(i64::from(seed_x), i64::from(seed_y))] = true;
    stack.push(Interval {
        start: i64::from(seed_x),
        end: i64::from(seed_x),
        row: i64::from(seed_y),
    });

    while let Some(interval) = stack.pop() {
        if visits > visit_budget {
            return None;
        }
        // Extend this run left and right along its own row, which is the step that makes the walk
        // proportional to runs rather than to pixels.
        let mut start = interval.start;
        while start > 0 && !decided[index(start - 1, interval.row)] {
            match consider(start - 1, interval.row, &mut visits) {
                Some(opacity) => {
                    start -= 1;
                    coverage[index(start, interval.row)] = opacity;
                    decided[index(start, interval.row)] = true;
                }
                None => {
                    decided[index(start - 1, interval.row)] = true;
                    break;
                }
            }
        }
        let mut end = interval.end;
        while end + 1 < i64::from(width) && !decided[index(end + 1, interval.row)] {
            match consider(end + 1, interval.row, &mut visits) {
                Some(opacity) => {
                    end += 1;
                    coverage[index(end, interval.row)] = opacity;
                    decided[index(end, interval.row)] = true;
                }
                None => {
                    decided[index(end + 1, interval.row)] = true;
                    break;
                }
            }
        }

        min_x = min_x.min(start);
        max_x = max_x.max(end);
        min_y = min_y.min(interval.row);
        max_y = max_y.max(interval.row);

        // Queue the runs above and below. Krita pushes an interval per row and crops it on the way out;
        // this finds the sub-runs directly, which costs the same visits and needs no interval map.
        for next_row in [interval.row - 1, interval.row + 1] {
            if next_row < 0 || next_row >= i64::from(height) {
                continue;
            }
            let mut x = start;
            while x <= end {
                if decided[index(x, next_row)] {
                    x += 1;
                    continue;
                }
                match consider(x, next_row, &mut visits) {
                    Some(opacity) => {
                        coverage[index(x, next_row)] = opacity;
                        decided[index(x, next_row)] = true;
                        let run_start = x;
                        // Absorb the contiguous remainder of this run so it is queued once.
                        while x < end
                            && !decided[index(x + 1, next_row)]
                            && match consider(x + 1, next_row, &mut visits) {
                                Some(opacity) => {
                                    coverage[index(x + 1, next_row)] = opacity;
                                    decided[index(x + 1, next_row)] = true;
                                    true
                                }
                                None => {
                                    decided[index(x + 1, next_row)] = true;
                                    false
                                }
                            }
                        {
                            x += 1;
                        }
                        stack.push(Interval {
                            start: run_start,
                            end: x,
                            row: next_row,
                        });
                    }
                    None => {
                        decided[index(x, next_row)] = true;
                    }
                }
                x += 1;
            }
        }
    }

    let box_width = (max_x - min_x + 1) as u32;
    let box_height = (max_y - min_y + 1) as u32;
    let mut cropped = vec![0u8; box_width as usize * box_height as usize];
    for y in 0..box_height {
        for x in 0..box_width {
            let source = index(min_x + i64::from(x), min_y + i64::from(y));
            cropped[(y as usize) * box_width as usize + x as usize] = coverage[source];
        }
    }
    Some(FillMask {
        x0: min_x as u32,
        y0: min_y as u32,
        width: box_width,
        height: box_height,
        coverage: cropped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values printed by real lcms2 2.x driving Krita's own sRGB-to-Lab transform and `differenceA`.
    ///
    /// The first pair is a colour against ITSELF. It reads 0 here, and the first version of the probe that
    /// produced these numbers reported 34 for it -- because it read a fourth Lab channel lcms2 never wrote.
    /// A self-comparison is the cheapest canary there is for that class of mistake.
    const LCMS2_REFERENCE: &[(Pixel, Pixel, u8)] = &[
        (Pixel::rgba(0, 0, 0, 255), Pixel::rgba(0, 0, 0, 255), 0),
        (
            Pixel::rgba(0, 0, 0, 255),
            Pixel::rgba(255, 255, 255, 255),
            100,
        ),
        (
            Pixel::rgba(255, 0, 0, 255),
            Pixel::rgba(0, 255, 0, 255),
            163,
        ),
        (
            Pixel::rgba(128, 128, 128, 255),
            Pixel::rgba(129, 129, 129, 255),
            0,
        ),
        (
            Pixel::rgba(128, 128, 128, 255),
            Pixel::rgba(138, 138, 138, 255),
            3,
        ),
        (
            Pixel::rgba(255, 255, 255, 255),
            Pixel::rgba(250, 250, 250, 255),
            1,
        ),
        (Pixel::rgba(0, 0, 0, 0), Pixel::rgba(0, 0, 0, 255), 100),
        (
            Pixel::rgba(10, 20, 30, 128),
            Pixel::rgba(10, 20, 30, 255),
            49,
        ),
    ];

    #[test]
    fn the_colour_difference_matches_lcms2() {
        for &(a, b, want) in LCMS2_REFERENCE {
            let got = colour_difference(a, b);
            assert_eq!(
                got, want,
                "{a:?} against {b:?}: lcms2 says {want}, closed form says {got}"
            );
            assert_eq!(
                colour_difference(b, a),
                want,
                "the distance must be symmetric"
            );
        }
    }

    /// The tolerance is a LAB distance, and that is not a detail.
    ///
    /// Mid-grey 128 against 138 is 10 apart in RGB and 3 in Lab. A fill built on an RGB distance would
    /// include very different pixels at the same tolerance setting, and the golden harness compares against
    /// Krita.
    #[test]
    fn the_distance_is_perceptual_and_not_rgb() {
        let a = Pixel::rgba(128, 128, 128, 255);
        let b = Pixel::rgba(138, 138, 138, 255);
        assert_eq!(colour_difference(a, b), 3);
        let rgb_distance = 10;
        assert_ne!(
            u32::from(colour_difference(a, b)),
            rgb_distance,
            "a Lab distance is not the RGB one"
        );

        // And it is non-uniform, though NOT in the direction one would guess. sRGB's encoding curve
        // largely cancels Lab's cube root -- that is what the encoding is for -- so a fixed sRGB step is
        // worth a roughly similar Lab distance across most of the range. Measured ΔL* for a ten-level
        // step: 2.74 at black, rising to a peak of 4.81 around level 32, then falling to 3.46 at white.
        // The dip at the very bottom is sRGB's linear segment below 0.04045 compressing the deep shadows.
        //
        // The first version of this test asserted shadows > highlights using 8→18 against 240→250, which
        // is 3.27 against 3.48 -- the wrong way round, and it failed against correct code.
        let deep = colour_difference(Pixel::rgba(0, 0, 0, 255), Pixel::rgba(10, 10, 10, 255));
        let mid = colour_difference(Pixel::rgba(16, 16, 16, 255), Pixel::rgba(26, 26, 26, 255));
        let high = colour_difference(
            Pixel::rgba(240, 240, 240, 255),
            Pixel::rgba(250, 250, 250, 255),
        );
        assert_eq!(
            (deep, mid, high),
            (2, 4, 3),
            "the measured shape of a ten-level step"
        );
        assert!(
            mid > high && high > deep,
            "the mid-shadows are the widest and the deepest shadows the narrowest: {mid}, {high}, {deep}"
        );
    }

    /// The hard and soft policies disagree at exactly the tolerance, and that is upstream's.
    #[test]
    fn the_two_policies_disagree_at_exactly_the_tolerance() {
        // Crisp: `difference <= threshold`.
        assert_eq!(opacity_from_difference(10, 10, 100), 255);
        assert_eq!(opacity_from_difference(11, 10, 100), 0);
        // Soft: `difference < threshold`.
        assert_eq!(
            opacity_from_difference(10, 10, 50),
            0,
            "a soft fill excludes a pixel exactly at the tolerance"
        );
        assert!(opacity_from_difference(9, 10, 50) > 0);
    }

    /// A soft fill with zero tolerance fills nothing, where a crisp one fills the exact colour.
    #[test]
    fn zero_tolerance_behaves_differently_in_the_two_policies() {
        assert_eq!(
            opacity_from_difference(0, 0, 100),
            255,
            "a crisp fill at zero tolerance still takes an exact match"
        );
        assert_eq!(
            opacity_from_difference(0, 0, 0),
            0,
            "a soft fill at zero tolerance takes nothing at all, which is Krita's arithmetic"
        );
    }

    /// Opacity falls as the distance grows, and reaches full coverage at the centre.
    #[test]
    fn the_soft_policy_falls_off_monotonically() {
        let mut previous = 255u8;
        for difference in 0..=40u8 {
            let opacity = opacity_from_difference(difference, 40, 50);
            assert!(
                opacity <= previous,
                "opacity rose from {previous} to {opacity} at difference {difference}"
            );
            previous = opacity;
        }
        assert_eq!(
            opacity_from_difference(0, 40, 50),
            255,
            "an exact match is full"
        );
        assert_eq!(
            opacity_from_difference(40, 40, 50),
            0,
            "and the tolerance is the end"
        );
    }

    /// A spread of 100 is the crisp path, not merely a narrow soft one.
    #[test]
    fn full_spread_is_the_crisp_policy() {
        for difference in 0..=20u8 {
            let crisp = opacity_from_difference(difference, 20, 100);
            assert!(
                crisp == 0 || crisp == 255,
                "a crisp fill is all or nothing, got {crisp} at {difference}"
            );
        }
        // While a spread below 100 produces intermediate values.
        let soft: Vec<u8> = (0..=20)
            .map(|d| opacity_from_difference(d, 20, 40))
            .collect();
        assert!(
            soft.iter().any(|value| *value > 0 && *value < 255),
            "a soft fill must produce partial coverage: {soft:?}"
        );
    }

    /// Build a canvas of solid rectangles.
    fn canvas(width: u32, height: u32, fill: Pixel) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            pixels.extend_from_slice(&[fill.r, fill.g, fill.b, fill.a]);
        }
        pixels
    }

    fn set(pixels: &mut [u8], width: u32, x: u32, y: u32, colour: Pixel) {
        let offset = ((y * width + x) * 4) as usize;
        pixels[offset] = colour.r;
        pixels[offset + 1] = colour.g;
        pixels[offset + 2] = colour.b;
        pixels[offset + 3] = colour.a;
    }

    const WHITE: Pixel = Pixel::rgba(255, 255, 255, 255);
    const BLACK: Pixel = Pixel::rgba(0, 0, 0, 255);

    #[test]
    fn a_fill_on_a_uniform_canvas_covers_everything() {
        let pixels = canvas(8, 6, WHITE);
        let mask =
            flood_fill_mask(&pixels, 8, 6, 3, 3, FloodFillOptions::default(), 1_000_000).unwrap();
        assert_eq!((mask.x0, mask.y0, mask.width, mask.height), (0, 0, 8, 6));
        for y in 0..6 {
            for x in 0..8 {
                assert_eq!(mask.coverage_at(x, y), 255, "at {x},{y}");
            }
        }
    }

    /// A barrier stops the fill, which is the whole behaviour.
    #[test]
    fn a_barrier_confines_the_fill() {
        let mut pixels = canvas(9, 5, WHITE);
        // A vertical black line down the middle.
        for y in 0..5 {
            set(&mut pixels, 9, 4, y, BLACK);
        }
        let mask =
            flood_fill_mask(&pixels, 9, 5, 1, 2, FloodFillOptions::default(), 1_000_000).unwrap();

        // Everything left of the line is filled.
        for y in 0..5 {
            for x in 0..4 {
                assert_eq!(
                    mask.coverage_at(x, y),
                    255,
                    "left of the barrier at {x},{y}"
                );
            }
        }
        // The line itself and everything right of it is not.
        for y in 0..5 {
            assert_eq!(mask.coverage_at(4, y), 0, "the barrier at row {y}");
            for x in 5..9 {
                assert_eq!(mask.coverage_at(x, y), 0, "right of the barrier at {x},{y}");
            }
        }
        // And the bounding box is cropped to what was filled.
        assert_eq!((mask.x0, mask.width), (0, 4));
    }

    /// The fill reaches round an obstacle rather than through it.
    #[test]
    fn the_fill_goes_around_a_wall_with_a_gap() {
        let mut pixels = canvas(7, 7, WHITE);
        // A wall down column 3 with a gap at row 6.
        for y in 0..6 {
            set(&mut pixels, 7, 3, y, BLACK);
        }
        let mask =
            flood_fill_mask(&pixels, 7, 7, 0, 0, FloodFillOptions::default(), 1_000_000).unwrap();
        // It gets through the gap and up the far side.
        assert_eq!(mask.coverage_at(6, 0), 255, "round the wall and back up");
        assert_eq!(mask.coverage_at(3, 6), 255, "through the gap itself");
        // The wall is still untouched.
        for y in 0..6 {
            assert_eq!(mask.coverage_at(3, y), 0, "the wall at row {y}");
        }
    }

    /// A closed wall keeps the fill out entirely.
    #[test]
    fn a_closed_wall_is_not_crossed() {
        let mut pixels = canvas(7, 7, WHITE);
        for y in 0..7 {
            set(&mut pixels, 7, 3, y, BLACK);
        }
        let mask =
            flood_fill_mask(&pixels, 7, 7, 0, 0, FloodFillOptions::default(), 1_000_000).unwrap();
        assert_eq!(
            (mask.x0, mask.width),
            (0, 3),
            "only the left side is filled"
        );
        for y in 0..7 {
            assert_eq!(mask.coverage_at(4, y), 0, "nothing crossed at row {y}");
        }
    }

    /// Diagonal neighbours do not connect: this is a four-connected fill, as Krita's is.
    #[test]
    fn the_fill_is_four_connected() {
        let mut pixels = canvas(5, 5, BLACK);
        // Two white pixels touching only at a corner.
        set(&mut pixels, 5, 1, 1, WHITE);
        set(&mut pixels, 5, 2, 2, WHITE);
        let options = FloodFillOptions {
            tolerance: 5,
            opacity_spread: 100,
        };
        let mask = flood_fill_mask(&pixels, 5, 5, 1, 1, options, 1_000_000).unwrap();
        assert_eq!(mask.coverage_at(1, 1), 255, "the seed is filled");
        assert_eq!(
            mask.coverage_at(2, 2),
            0,
            "a diagonal neighbour is not reached by a four-connected fill"
        );
        assert_eq!((mask.width, mask.height), (1, 1), "so the box is one pixel");
    }

    /// Tolerance decides how much of a gradient comes in, and it is measured in Lab.
    #[test]
    fn tolerance_widens_the_fill_along_a_ramp() {
        let mut pixels = canvas(16, 1, WHITE);
        for x in 0..16u32 {
            let level = (x * 16).min(255) as u8;
            set(&mut pixels, 16, x, 0, Pixel::rgba(level, level, level, 255));
        }
        // Seeded at the dark end. A tolerance of 0 with a crisp edge takes only the exact colour.
        let tight = flood_fill_mask(
            &pixels,
            16,
            1,
            0,
            0,
            FloodFillOptions {
                tolerance: 0,
                opacity_spread: 100,
            },
            1_000_000,
        )
        .unwrap();
        assert_eq!(tight.width, 1, "zero tolerance takes the seed alone");

        let loose = flood_fill_mask(
            &pixels,
            16,
            1,
            0,
            0,
            FloodFillOptions {
                tolerance: 40,
                opacity_spread: 100,
            },
            1_000_000,
        )
        .unwrap();
        assert!(
            loose.width > tight.width,
            "a wider tolerance must reach further: {} against {}",
            loose.width,
            tight.width
        );
        assert!(
            loose.width < 16,
            "but not all the way to white, or the test proves nothing: {}",
            loose.width
        );
    }

    /// A soft edge produces partial coverage at the boundary.
    #[test]
    fn a_soft_spread_produces_partial_coverage() {
        let mut pixels = canvas(12, 1, WHITE);
        for x in 0..12u32 {
            let level = (x * 6).min(255) as u8;
            set(&mut pixels, 12, x, 0, Pixel::rgba(level, level, level, 255));
        }
        let mask = flood_fill_mask(
            &pixels,
            12,
            1,
            0,
            0,
            FloodFillOptions {
                tolerance: 30,
                opacity_spread: 20,
            },
            1_000_000,
        )
        .unwrap();
        let values: Vec<u8> = (0..mask.width)
            .map(|x| mask.coverage_at(mask.x0 + x, 0))
            .collect();
        assert!(
            values.iter().any(|v| *v > 0 && *v < 255),
            "a soft fill must have a partial edge: {values:?}"
        );
        assert_eq!(values[0], 255, "the seed itself is full");
    }

    /// A seed that cannot be filled yields nothing rather than an empty box.
    #[test]
    fn an_unfillable_seed_yields_nothing() {
        let pixels = canvas(4, 4, WHITE);
        // A soft fill with zero tolerance fills nothing at all, including its own seed.
        let options = FloodFillOptions {
            tolerance: 0,
            opacity_spread: 0,
        };
        assert!(
            flood_fill_mask(&pixels, 4, 4, 2, 2, options, 1_000_000).is_none(),
            "no fill is None, not a zero-coverage mask"
        );
    }

    /// Out-of-range seeds and malformed buffers are refused.
    #[test]
    fn bad_inputs_are_refused() {
        let pixels = canvas(4, 4, WHITE);
        let options = FloodFillOptions::default();
        assert!(
            flood_fill_mask(&pixels, 4, 4, 4, 0, options, 1_000).is_none(),
            "x past the edge"
        );
        assert!(
            flood_fill_mask(&pixels, 4, 4, 0, 4, options, 1_000).is_none(),
            "y past the edge"
        );
        assert!(
            flood_fill_mask(&pixels, 0, 4, 0, 0, options, 1_000).is_none(),
            "zero width"
        );
        assert!(
            flood_fill_mask(&pixels, 4, 0, 0, 0, options, 1_000).is_none(),
            "zero height"
        );
        assert!(
            flood_fill_mask(&pixels[..8], 4, 4, 0, 0, options, 1_000).is_none(),
            "a buffer shorter than the declared size"
        );
    }

    /// The visit budget stops a fill rather than letting it run away.
    #[test]
    fn the_visit_budget_is_enforced() {
        let pixels = canvas(64, 64, WHITE);
        let options = FloodFillOptions::default();
        assert!(
            flood_fill_mask(&pixels, 64, 64, 32, 32, options, 10).is_none(),
            "a budget of ten visits cannot fill four thousand pixels"
        );
        assert!(
            flood_fill_mask(&pixels, 64, 64, 32, 32, options, 1_000_000).is_some(),
            "and a generous budget succeeds, so the test is not vacuous"
        );
    }

    /// Every pixel is decided once. Without a separate decided set, a pixel rejected for colour is
    /// reconsidered from every neighbouring run, which turns the walk quadratic on a large region.
    #[test]
    fn each_pixel_is_visited_a_bounded_number_of_times() {
        let pixels = canvas(40, 40, WHITE);
        let options = FloodFillOptions::default();
        // 1,600 pixels. A correct scanline walk visits each a small constant number of times; allow four.
        assert!(
            flood_fill_mask(&pixels, 40, 40, 20, 20, options, 1_600 * 4).is_some(),
            "a uniform fill must complete within four visits per pixel"
        );
    }

    /// A fill seeded on transparency fills the transparent region, because alpha is part of the distance.
    #[test]
    fn transparency_is_part_of_the_distance() {
        let mut pixels = canvas(6, 3, Pixel::TRANSPARENT);
        // An opaque black block on the right.
        for y in 0..3 {
            for x in 3..6 {
                set(&mut pixels, 6, x, y, BLACK);
            }
        }
        let mask =
            flood_fill_mask(&pixels, 6, 3, 0, 0, FloodFillOptions::default(), 1_000_000).unwrap();
        assert_eq!((mask.x0, mask.width), (0, 3), "only the transparent side");
        for y in 0..3 {
            assert_eq!(
                mask.coverage_at(3, y),
                0,
                "the opaque block is excluded at row {y}"
            );
        }
        // The distance between fully transparent and fully opaque is 100, measured against lcms2.
        assert_eq!(colour_difference(Pixel::TRANSPARENT, BLACK), 100);
    }

    #[test]
    fn the_options_reject_an_out_of_range_spread() {
        assert!(FloodFillOptions::default().is_valid());
        assert!(
            FloodFillOptions {
                tolerance: 255,
                opacity_spread: 0
            }
            .is_valid()
        );
        assert!(
            !FloodFillOptions {
                tolerance: 10,
                opacity_spread: 101
            }
            .is_valid()
        );
    }
}
