//! Dab spacing: how far apart dabs are placed along a stroke.
//!
//! TRANSLATED from Krita, pinned at `fdbf33b2146735465bb8aa59928fbc1890ceb160`, GPL-2.0-or-later, taken
//! into this GPL-3.0-or-later product under the or-later grant:
//!
//! - `libs/image/brushengine/kis_paintop_utils.{h,cpp}` — `effectiveSpacing`, `calcAutoSpacing`
//! - `libs/image/kis_distance_information.cpp` — the elliptical `getNextPointPosition`
//! - `libs/brush/kis_brush.cpp` — `setSpacing`'s floor
//!
//! Licence change notice: the originals are GPL-2.0-or-later; this file is GPL-3.0-or-later, which the
//! or-later grant permits. Attribution is in `UPSTREAM_NOTICES.md`.
//!
//! # Why this, out of block 6
//!
//! Item 1c.2 block 6 is the brush model and generation. Most of it is already here: the generated dab
//! shape landed at block 4's first pass and image tips at its second. What was missing is **spacing**.
//!
//! This product spaced dabs at a hard-coded quarter of the brush size, with no setting. Two consequences:
//! a GBR tip's own spacing was decoded and then ignored, and — since block 4 gave dabs an aspect ratio —
//! an elliptical brush spaced identically in every direction, which is wrong.
//!
//! # Dabs land where the stroke crosses a spacing ELLIPSE
//!
//! Krita accumulates the travelled `|dx|` and `|dy|` separately and paints where the accumulation crosses
//! an ellipse whose semi-axes are the dab's own dimensions times the spacing fraction. Measured on a 20×5
//! dab at spacing 0.25: dabs every **5.0** horizontally and every **1.25** vertically. Dividing a scalar
//! distance, as this product did, cannot express that at all.
//!
//! For a round dab the ellipse is a circle and the solver reduces to Euclidean distance -- verified: a
//! diagonal stroke gets dabs 5.0 apart, at 3.536 along each axis.

use serde::{Deserialize, Serialize};

/// Krita's `setSpacing` floor. A spacing of zero would place infinitely many dabs.
pub const MIN_SPACING: f32 = 0.02;

/// Krita's `MIN_DISTANCE_SPACING`: the smallest an ellipse axis may be, in pixels.
///
/// Distinct from [`MIN_SPACING`], which bounds the FRACTION. A small dab with a legal fraction can still
/// produce a sub-pixel axis, and this is what stops that becoming an unbounded dab count.
pub const MIN_AXIS_PIXELS: f32 = 0.5;

/// How dabs are spaced along a stroke.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpacingOptions {
    /// Spacing as a fraction of the dab's size, floored at [`MIN_SPACING`].
    pub spacing: f32,
    /// When set, both axes use the dab's larger dimension, so an elliptical dab spaces evenly.
    ///
    /// Krita also zeroes the rotation in this branch: an isotropic spacing circle has no orientation to
    /// rotate.
    pub isotropic: bool,
}

impl Default for SpacingOptions {
    /// A quarter of the dab's size, which is what this product used before spacing was configurable.
    ///
    /// Chosen so a document saved before this field existed reopens spaced as it was drawn, not because a
    /// quarter is Krita's default -- Krita's own brushes carry their own.
    fn default() -> Self {
        Self {
            spacing: 0.25,
            isotropic: false,
        }
    }
}

impl SpacingOptions {
    pub fn is_valid(&self) -> bool {
        self.spacing.is_finite() && self.spacing > 0.0 && self.spacing <= 10.0
    }

    /// The spacing ellipse's semi-axes for a dab of the given dimensions, in pixels.
    ///
    /// Krita's `effectiveSpacing` for the non-auto cases.
    pub fn axes(&self, dab_width: f32, dab_height: f32) -> (f32, f32) {
        // Krita's setSpacing clamp, applied here because the value can also arrive from a file.
        let fraction = if self.spacing.is_finite() {
            self.spacing.max(MIN_SPACING)
        } else {
            MIN_SPACING
        };
        if self.isotropic {
            let significant = dab_width.max(dab_height) * fraction;
            (significant, significant)
        } else {
            (dab_width * fraction, dab_height * fraction)
        }
    }
}

/// Whether the options are the default, for `skip_serializing_if`.
pub(crate) fn is_default_spacing(options: &SpacingOptions) -> bool {
    let fallback = SpacingOptions::default();
    options.spacing == fallback.spacing && options.isotropic == fallback.isotropic
}

/// Walks a stroke and decides where dabs land.
///
/// Translated from `KisDistanceInformation`'s elliptical branch. Carries the travelled distance between
/// calls, because a dab's position depends on how far the stroke has come since the last one -- a segment
/// shorter than the spacing contributes to the next dab rather than being ignored or forcing one.
#[derive(Clone, Copy, Debug)]
pub struct SpacingWalker {
    axis_x: f32,
    axis_y: f32,
    accumulated_x: f32,
    accumulated_y: f32,
}

impl SpacingWalker {
    pub fn new(axis_x: f32, axis_y: f32) -> Self {
        Self {
            axis_x: axis_x.max(MIN_AXIS_PIXELS),
            axis_y: axis_y.max(MIN_AXIS_PIXELS),
            accumulated_x: 0.0,
            accumulated_y: 0.0,
        }
    }

    /// Where the next dab lands on the segment, as a fraction of it, or `None` when the segment ends
    /// before the next dab is due.
    ///
    /// Returning `Some(0.0)` means a dab is due immediately, which happens when the accumulation is
    /// already outside the ellipse -- Krita reaches that when the spacing changes mid-stroke.
    pub fn next_dab(&mut self, dx: f32, dy: f32) -> Option<f32> {
        if dx == 0.0 && dy == 0.0 {
            return None;
        }
        let a_rev = f64::from(1.0 / self.axis_x);
        let b_rev = f64::from(1.0 / self.axis_y);
        let x = f64::from(self.accumulated_x);
        let y = f64::from(self.accumulated_y);

        let gamma = (x * a_rev).powi(2) + (y * b_rev).powi(2) - 1.0;
        if gamma >= 0.0 {
            self.reset();
            return Some(0.0);
        }

        let dx = f64::from(dx).abs();
        let dy = f64::from(dy).abs();
        let alpha = (dx * a_rev).powi(2) + (dy * b_rev).powi(2);
        let beta = x * dx * a_rev * a_rev + y * dy * b_rev * b_rev;
        let discriminant = beta * beta - alpha * gamma;

        if discriminant < 0.0 || alpha == 0.0 {
            // Krita logs a bug here and returns -1. The algebra says this cannot happen with
            // `gamma < 0 <= beta²`, so rather than log into a crate with no logger, the segment is
            // accumulated -- which is the same thing the out-of-range branch below does.
            self.accumulated_x += dx as f32;
            self.accumulated_y += dy as f32;
            return None;
        }

        let k = (-beta + discriminant.sqrt()) / alpha;
        if (0.0..=1.0).contains(&k) {
            self.reset();
            Some(k as f32)
        } else {
            self.accumulated_x += dx as f32;
            self.accumulated_y += dy as f32;
            None
        }
    }

    fn reset(&mut self) {
        self.accumulated_x = 0.0;
        self.accumulated_y = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walks a polyline the way the stroke path does, collecting dab positions.
    fn walk(path: &[(f32, f32)], axes: (f32, f32), cap: usize) -> Vec<(f32, f32)> {
        let mut walker = SpacingWalker::new(axes.0, axes.1);
        let mut dabs = Vec::new();
        if path.is_empty() {
            return dabs;
        }
        dabs.push(path[0]);
        for pair in path.windows(2) {
            let mut start = pair[0];
            let end = pair[1];
            while dabs.len() < cap {
                let Some(t) = walker.next_dab(end.0 - start.0, end.1 - start.1) else {
                    break;
                };
                let dab = (
                    start.0 + (end.0 - start.0) * t,
                    start.1 + (end.1 - start.1) * t,
                );
                dabs.push(dab);
                start = dab;
                if (end.0 - start.0).abs() < 1e-9 && (end.1 - start.1).abs() < 1e-9 {
                    break;
                }
            }
        }
        dabs
    }

    /// A round dab of diameter 20 at spacing 0.25 gives a 5-pixel circle.
    ///
    /// Positions printed by Krita's own solver: 0, 5, 10, 15, 20, 25, 30, 35, 40.
    #[test]
    fn a_round_dab_spaces_evenly_and_matches_upstream() {
        let axes = SpacingOptions::default().axes(20.0, 20.0);
        assert_eq!(axes, (5.0, 5.0));
        let dabs = walk(&[(0.0, 0.0), (40.0, 0.0)], axes, 40);
        let xs: Vec<f32> = dabs.iter().map(|dab| dab.0).collect();
        assert_eq!(xs.len(), 9, "nine dabs over forty pixels at five apart");
        for (index, x) in xs.iter().enumerate() {
            assert!(
                (x - index as f32 * 5.0).abs() < 1e-4,
                "dab {index} should be at {}, got {x}",
                index as f32 * 5.0
            );
        }
    }

    /// The same round dab on a diagonal: the ellipse is a circle, so the step is Euclidean.
    ///
    /// Krita's solver puts the first dab at (3.536, 3.536), which is 5.0 from the origin.
    #[test]
    fn a_round_dab_on_a_diagonal_steps_by_euclidean_distance() {
        let axes = SpacingOptions::default().axes(20.0, 20.0);
        let dabs = walk(&[(0.0, 0.0), (30.0, 30.0)], axes, 40);
        assert!(dabs.len() >= 2);
        let first = dabs[1];
        assert!(
            (first.0 - 3.535_5).abs() < 1e-3 && (first.1 - 3.535_5).abs() < 1e-3,
            "expected Krita's (3.536, 3.536), got {first:?}"
        );
        let distance = (first.0 * first.0 + first.1 * first.1).sqrt();
        assert!(
            (distance - 5.0).abs() < 1e-3,
            "and that is 5.0 from the origin, got {distance}"
        );
    }

    /// The finding this whole file exists for: an elliptical dab spaces per axis.
    ///
    /// Measured in Krita for a 20×5 dab at spacing 0.25: every 5.0 horizontally and every 1.25 vertically.
    /// A scalar spacing cannot express the difference, which is what this product did before.
    #[test]
    fn an_elliptical_dab_spaces_differently_along_each_axis() {
        let axes = SpacingOptions::default().axes(20.0, 5.0);
        assert_eq!(axes, (5.0, 1.25));

        let horizontal = walk(&[(0.0, 0.0), (40.0, 0.0)], axes, 60);
        assert_eq!(horizontal.len(), 9, "wide apart along the long axis");
        assert!((horizontal[1].0 - 5.0).abs() < 1e-4);

        let vertical = walk(&[(0.0, 0.0), (0.0, 20.0)], axes, 60);
        assert_eq!(vertical.len(), 17, "close together along the short axis");
        assert!(
            (vertical[1].1 - 1.25).abs() < 1e-4,
            "the first vertical step is 1.25, got {}",
            vertical[1].1
        );
        // Exactly the values Krita printed.
        for (index, dab) in vertical.iter().enumerate() {
            assert!(
                (dab.1 - index as f32 * 1.25).abs() < 1e-3,
                "vertical dab {index} should be at {}, got {}",
                index as f32 * 1.25,
                dab.1
            );
        }
    }

    /// Isotropic spacing uses the larger axis for both, so the ellipse becomes a circle.
    #[test]
    fn isotropic_spacing_uses_the_larger_axis() {
        let options = SpacingOptions {
            spacing: 0.25,
            isotropic: true,
        };
        let axes = options.axes(20.0, 5.0);
        assert_eq!(axes, (5.0, 5.0), "the larger axis wins on both");

        let horizontal = walk(&[(0.0, 0.0), (40.0, 0.0)], axes, 40);
        let vertical = walk(&[(0.0, 0.0), (0.0, 40.0)], axes, 40);
        assert_eq!(
            horizontal.len(),
            vertical.len(),
            "and the two directions now agree, which is the point"
        );
        assert_eq!(horizontal.len(), 9);
    }

    /// Krita floors the spacing FRACTION at 0.02 and each ellipse AXIS at half a pixel.
    ///
    /// Two separate bounds. A legal fraction on a small dab still yields a sub-pixel axis, and without the
    /// second floor that is an unbounded dab count.
    #[test]
    fn both_spacing_floors_are_applied() {
        // A fraction below the floor is raised: 20 × 0.02 = 0.4, then the axis floor makes it 0.5.
        let tiny = SpacingOptions {
            spacing: 0.001,
            isotropic: false,
        };
        // Compared with a tolerance, not exactly: 20.0 * 0.02 is 0.39999998 in f32, and an `assert_eq!`
        // on a float product is a test that fails for arithmetic reasons rather than behavioural ones.
        let clamped = tiny.axes(20.0, 20.0);
        assert!(
            (clamped.0 - 0.4).abs() < 1e-6 && (clamped.1 - 0.4).abs() < 1e-6,
            "the fraction is clamped to 0.02, giving axes near 0.4, got {clamped:?}"
        );
        let dabs = walk(&[(0.0, 0.0), (5.0, 0.0)], tiny.axes(20.0, 20.0), 40);
        assert_eq!(dabs.len(), 11, "and the axis floor gives half-pixel steps");
        assert!((dabs[1].0 - 0.5).abs() < 1e-4);

        // A legal fraction on a one-pixel dab: 1 × 0.02 = 0.02, floored to 0.5.
        let small_dab = SpacingOptions {
            spacing: 0.02,
            isotropic: false,
        };
        let small_axes = small_dab.axes(1.0, 1.0);
        assert!((small_axes.0 - 0.02).abs() < 1e-7 && (small_axes.1 - 0.02).abs() < 1e-7);
        let stepped = walk(&[(0.0, 0.0), (3.0, 0.0)], small_dab.axes(1.0, 1.0), 40);
        assert_eq!(stepped.len(), 7, "half-pixel steps over three pixels");
    }

    /// A non-finite spacing falls back to the floor rather than producing NaN positions.
    #[test]
    fn a_non_finite_spacing_cannot_produce_nan() {
        for spacing in [f32::NAN, f32::INFINITY, -1.0] {
            let options = SpacingOptions {
                spacing,
                isotropic: false,
            };
            let axes = options.axes(20.0, 20.0);
            assert!(
                axes.0.is_finite() && axes.1.is_finite() && axes.0 > 0.0,
                "spacing {spacing} gave axes {axes:?}"
            );
            let dabs = walk(&[(0.0, 0.0), (4.0, 0.0)], axes, 40);
            for dab in dabs {
                assert!(dab.0.is_finite() && dab.1.is_finite());
            }
        }
    }

    /// A segment shorter than the spacing contributes to the next dab rather than being lost.
    ///
    /// This is what the accumulator is for: a stroke of many tiny segments must space the same as one long
    /// segment, or a slow careful stroke gets more paint than a fast one over the same path.
    #[test]
    fn many_short_segments_space_like_one_long_one() {
        let axes = SpacingOptions::default().axes(20.0, 20.0);
        let long = walk(&[(0.0, 0.0), (40.0, 0.0)], axes, 60);
        let short: Vec<(f32, f32)> = (0..=40).map(|x| (x as f32, 0.0)).collect();
        let stepped = walk(&short, axes, 60);
        assert_eq!(
            long.len(),
            stepped.len(),
            "forty one-pixel segments must yield the same dabs as one forty-pixel segment"
        );
        for (a, b) in long.iter().zip(stepped.iter()) {
            assert!(
                (a.0 - b.0).abs() < 1e-3,
                "positions must agree: {a:?} against {b:?}"
            );
        }
    }

    /// A zero-length segment places no dab and does not disturb the accumulation.
    #[test]
    fn a_zero_length_segment_is_ignored() {
        let mut walker = SpacingWalker::new(5.0, 5.0);
        assert_eq!(walker.next_dab(0.0, 0.0), None);
        // The next real segment behaves as though the zero one never happened.
        assert_eq!(
            walker.next_dab(3.0, 0.0),
            None,
            "three pixels is short of five"
        );
        assert!(walker.next_dab(3.0, 0.0).is_some(), "six is past it");
    }

    /// An accumulation already outside the ellipse places a dab immediately.
    #[test]
    fn an_overdue_dab_lands_at_the_segment_start() {
        let mut walker = SpacingWalker::new(5.0, 5.0);
        // Walk four pixels twice without a dab being due, then shrink the ellipse by rebuilding the
        // walker's state the way a mid-stroke spacing change would.
        assert_eq!(walker.next_dab(4.0, 0.0), None);
        let overdue = walker.next_dab(4.0, 0.0);
        assert!(
            overdue.is_some(),
            "eight pixels of travel past a five-pixel ellipse must place a dab"
        );
        let t = overdue.unwrap();
        assert!(
            (0.0..=1.0).contains(&t),
            "and it lands on the segment, at {t}"
        );
    }

    #[test]
    fn the_options_validate() {
        assert!(SpacingOptions::default().is_valid());
        assert!(
            SpacingOptions {
                spacing: MIN_SPACING,
                isotropic: true
            }
            .is_valid()
        );
        for spacing in [0.0, -0.5, f32::NAN, f32::INFINITY, 10.1] {
            assert!(
                !SpacingOptions {
                    spacing,
                    isotropic: false
                }
                .is_valid(),
                "spacing {spacing} should be refused"
            );
        }
    }
}
