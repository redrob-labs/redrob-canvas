//! Brush dab shape: how much a dab covers a pixel at a given offset from its centre.
//!
//! TRANSLATED from Krita, pinned at `fdbf33b2146735465bb8aa59928fbc1890ceb160`, GPL-2.0-or-later, taken
//! into this GPL-3.0-or-later product under the or-later grant:
//!
//! - `libs/image/kis_circle_mask_generator.cpp` — `KisCircleMaskGenerator::valueAt` and its coefficients
//! - `libs/image/kis_base_mask_generator.{h,cpp}` — the shared `fade`, `softness` and `ratio` handling
//!
//! Licence change notice: the originals are GPL-2.0-or-later; this file is GPL-3.0-or-later, which the
//! or-later grant permits. Attribution is in `UPSTREAM_NOTICES.md`.
//!
//! # Why this, and why before the brush file formats
//!
//! Item 1c.2 block 4 is the ABR, GBR and GIH readers — Photoshop's and GIMP's brush-tip formats, which
//! the plan rightly calls unobtainable elsewhere. Measured first: this product had **no brush tip model
//! at all** to read them into. Its dab was a hard-coded circle with a fixed one-pixel linear feather and
//! no hardness control, so a reader would have had nowhere to put a loaded tip.
//!
//! The plan orders 1-c by verifiability rather than dependency, deliberately. That criterion is what moves
//! this first: a format reader with no model to populate cannot be verified at all, only compiled.
//!
//! # The convention is inverted, and that is the first thing to get wrong
//!
//! Krita's mask values run **0 for fully opaque and 255 for fully transparent**. This product's dab loop
//! multiplies a coverage where 1.0 is opaque. The translation converts, and there is a test asserting the
//! centre of a dab is full coverage rather than none — which is exactly what a missed inversion produces.

use serde::{Deserialize, Serialize};

/// A dab's shape.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DabShape {
    /// Fraction of the radius that is solid before the falloff begins, `0.0..=1.0`.
    ///
    /// Krita calls this `fade` and inverts the sense in its UI. `hardness` is what every paint program
    /// calls it, and here it maps directly: 1.0 is a hard edge, 0.0 the softest.
    pub hardness: f32,
    /// Sharpens or spreads the falloff, `0.01..=2.0`. Krita's `softness`, same sense.
    pub softness: f32,
    /// Height as a fraction of width. 1.0 is round.
    pub ratio: f32,
    /// Krita's `antialiasEdges`: offsets the falloff sample by one pixel so the outer edge is not a hard
    /// step. Measured to shift mid-falloff values by up to 14 of 255, so it is not cosmetic.
    pub antialias_edges: bool,
    /// GIMP's pencil: the falloff is thresholded to a solid 0/1 disc, so the dab has a hard, aliased
    /// edge regardless of hardness. Defaults false (the paintbrush's soft edge).
    #[serde(default)]
    pub pencil: bool,
}

impl Default for DabShape {
    /// The shape matching this product's behaviour before a shape existed: effectively hard-edged, round.
    ///
    /// `Default` rather than a named constructor because `BrushStroke` carries it behind
    /// `#[serde(default)]`, so every document saved before this field existed must deserialise to the
    /// behaviour it was drawn with.
    fn default() -> Self {
        Self {
            hardness: 1.0,
            softness: 1.0,
            ratio: 1.0,
            antialias_edges: true,
            pencil: false,
        }
    }
}

impl DabShape {
    pub const MIN_SOFTNESS: f32 = 0.01;
    pub const MAX_SOFTNESS: f32 = 2.0;

    /// A round dab of the given hardness, with everything else at its default.
    pub fn round(hardness: f32) -> Self {
        Self {
            hardness,
            ..Self::default()
        }
    }

    /// Whether every field is in range and finite.
    ///
    /// Checked rather than clamped at the command boundary, because a silently clamped brush draws
    /// something the caller did not ask for and gives no way to notice. [`DabMask`] clamps internally so
    /// the arithmetic cannot divide by zero even if this is bypassed.
    pub fn is_valid(&self) -> bool {
        self.hardness.is_finite()
            && (0.0..=1.0).contains(&self.hardness)
            && self.softness.is_finite()
            && (Self::MIN_SOFTNESS..=Self::MAX_SOFTNESS).contains(&self.softness)
            && self.ratio.is_finite()
            && (0.01..=100.0).contains(&self.ratio)
    }
}

/// A dab shape with its coefficients resolved for one diameter.
///
/// Krita recomputes these in `setScale` and `setSoftness` rather than per sample, and the same split is
/// kept: a dab covers hundreds of pixels and the coefficients depend only on the shape and the size.
#[derive(Clone, Copy, Debug)]
pub struct DabMask {
    x_coefficient: f32,
    y_coefficient: f32,
    fade_x: f32,
    fade_y: f32,
    antialias_edges: bool,
    pencil: bool,
    empty: bool,
}

impl DabMask {
    /// Resolves a shape for a dab of the given diameter, in pixels.
    pub fn new(shape: DabShape, diameter: f32) -> Self {
        if !diameter.is_finite() || diameter <= 0.0 {
            return Self {
                x_coefficient: 0.0,
                y_coefficient: 0.0,
                fade_x: 0.0,
                fade_y: 0.0,
                antialias_edges: shape.antialias_edges,
                pencil: shape.pencil,
                empty: true,
            };
        }

        let ratio = if shape.ratio.is_finite() {
            shape.ratio.clamp(0.01, 100.0)
        } else {
            1.0
        };
        let hardness = if shape.hardness.is_finite() {
            shape.hardness.clamp(0.0, 1.0)
        } else {
            1.0
        };
        // Krita's `qMax(0.01, softness)` then reciprocal. The floor is what stops a division by zero.
        let softness = if shape.softness.is_finite() {
            shape
                .softness
                .clamp(DabShape::MIN_SOFTNESS, DabShape::MAX_SOFTNESS)
        } else {
            1.0
        };
        let softness_coefficient = 1.0 / softness.max(DabShape::MIN_SOFTNESS);

        let width = diameter;
        let height = diameter * ratio;
        let x_coefficient = 2.0 / width;
        let y_coefficient = 2.0 / height;
        // Krita special-cases a zero fade to a coefficient of 1 rather than dividing by zero. Measured
        // consequence at diameter 40: fade 0 and softness 0.1 give byte-identical masks, because both
        // land on a transformed coefficient of 1.0.
        let x_fade_coefficient = if hardness == 0.0 {
            1.0
        } else {
            2.0 / (hardness * width)
        };
        let y_fade_coefficient = if hardness == 0.0 {
            1.0
        } else {
            2.0 / (hardness * height)
        };

        Self {
            x_coefficient,
            y_coefficient,
            fade_x: x_fade_coefficient * softness_coefficient,
            fade_y: y_fade_coefficient * softness_coefficient,
            antialias_edges: shape.antialias_edges,
            pencil: shape.pencil,
            empty: false,
        }
    }

    /// Coverage at an offset from the dab's centre, `0.0` outside and `1.0` fully covered.
    ///
    /// **Inverted from Krita**, which returns 0 for opaque and 255 for transparent. This returns coverage
    /// because that is what the dab loop multiplies.
    pub fn coverage_at(&self, dx: f32, dy: f32) -> f32 {
        if self.empty {
            return 0.0;
        }
        let mut x = dx;
        let mut y = dy.abs();

        // Normalised squared distance. Krita's `norme` is the sum of squares, without the root -- the
        // comparison against 1.0 works the same and the root is never paid for.
        let n = squared(x * self.x_coefficient) + squared(y * self.y_coefficient);
        if n > 1.0 {
            return 0.0;
        }

        if self.antialias_edges {
            // Krita's own comment: "we add +1.0 to ensure correct antialiasing on the border". It shifts
            // only the falloff sample, not the containment test above.
            x = x.abs() + 1.0;
            y = y.abs() + 1.0;
        }

        let nf = squared(x * self.fade_x) + squared(y * self.fade_y);
        if nf < 1.0 {
            return 1.0;
        }
        // Krita: `255 * n * (nf - 1) / (nf - n)`, a transparency. One minus it, as coverage.
        //
        // `nf - n` cannot be zero here: reaching this line needs `nf >= 1 >= n`, and `nf == n` would put
        // both at exactly 1, which the `nf < 1.0` branch above does not catch but the subtraction would
        // divide by. Measured: hardness 1.0 and softness 2.0 both make `nf == n` for every sample, and
        // every sample then takes the `nf < 1.0` branch or the `n > 1.0` one. The guard is kept anyway
        // because it costs a comparison and the alternative is a NaN written into a pixel.
        let denominator = nf - n;
        if denominator <= 0.0 {
            return 1.0;
        }
        let transparency = n * (nf - 1.0) / denominator;
        let coverage = (1.0 - transparency).clamp(0.0, 1.0);
        if self.pencil {
            // Pencil (GIMP): a hard, aliased edge. The whole falloff is thresholded to a solid 0/1 at
            // the half-way point, independent of the Krita antialias_edges border shift above.
            if coverage >= 0.5 { 1.0 } else { 0.0 }
        } else {
            coverage
        }
    }

    /// The same value as a byte in Krita's own convention, for comparing against upstream.
    ///
    /// Exists so the tests can assert against values printed by Krita's compiled `valueAt` without
    /// inverting by hand in each assertion, which is where an off-by-one would hide.
    pub fn krita_value_at(&self, dx: f32, dy: f32) -> u8 {
        let coverage = self.coverage_at(dx, dy);
        (255.0 * (1.0 - coverage)).round().clamp(0.0, 255.0) as u8
    }

    /// Whether this mask covers nothing at all.
    pub fn is_empty(&self) -> bool {
        self.empty
    }
}

/// Whether a shape is the default, for `skip_serializing_if`.
///
/// Field-by-field rather than `PartialEq` against `default()` so a NaN could never make a shape compare
/// unequal to itself and start being written when it need not be.
pub(crate) fn is_default_shape(shape: &DabShape) -> bool {
    let fallback = DabShape::default();
    shape.hardness == fallback.hardness
        && shape.softness == fallback.softness
        && shape.ratio == fallback.ratio
        && shape.antialias_edges == fallback.antialias_edges
        && shape.pencil == fallback.pencil
}

#[inline]
fn squared(value: f32) -> f32 {
    value * value
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Diameter 40, hardness 0.5, softness 1.0, antialias off — the exact byte values printed by Krita's
    /// compiled `KisCircleMaskGenerator::valueAt` along the x axis.
    const KRITA_D40_HARDNESS_HALF: [(f32, u8); 22] = [
        (0.0, 0),
        (1.0, 0),
        (2.0, 0),
        (3.0, 0),
        (4.0, 0),
        (5.0, 0),
        (6.0, 0),
        (7.0, 0),
        (8.0, 0),
        (9.0, 0),
        (10.0, 0),
        (11.0, 17),
        (12.0, 37),
        (13.0, 58),
        (14.0, 81),
        (15.0, 106),
        (16.0, 132),
        (17.0, 160),
        (18.0, 190),
        (19.0, 221),
        (20.0, 255),
        (21.0, 255),
    ];

    fn plain(hardness: f32, diameter: f32) -> DabMask {
        DabMask::new(
            DabShape {
                hardness,
                softness: 1.0,
                ratio: 1.0,
                antialias_edges: false,
                pencil: false,
            },
            diameter,
        )
    }

    #[test]
    fn the_falloff_matches_kritas_byte_for_byte() {
        let mask = plain(0.5, 40.0);
        for (x, want) in KRITA_D40_HARDNESS_HALF {
            let got = mask.krita_value_at(x, 0.0);
            assert!(
                got.abs_diff(want) <= 1,
                "at x={x}: Krita says {want}, we say {got}"
            );
        }
    }

    /// The single most likely translation error: Krita's 0 means opaque.
    #[test]
    fn the_centre_of_a_dab_is_full_coverage_not_none() {
        let mask = plain(0.5, 40.0);
        assert_eq!(
            mask.coverage_at(0.0, 0.0),
            1.0,
            "the centre must be fully covered; 0.0 here means the inversion was missed"
        );
        assert_eq!(
            mask.krita_value_at(0.0, 0.0),
            0,
            "and 0 in Krita's convention"
        );
        assert_eq!(
            mask.coverage_at(30.0, 0.0),
            0.0,
            "and well outside the radius, nothing"
        );
    }

    /// Coverage falls monotonically from centre to edge, for every hardness.
    #[test]
    fn coverage_never_rises_with_distance() {
        for hardness in [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
            let mask = plain(hardness, 40.0);
            let mut previous = 1.0;
            for step in 0..=400 {
                let x = step as f32 * 0.1;
                let coverage = mask.coverage_at(x, 0.0);
                assert!(
                    coverage <= previous + 1e-6,
                    "hardness {hardness}: coverage rose from {previous} to {coverage} at x={x}"
                );
                assert!(
                    (0.0..=1.0).contains(&coverage),
                    "hardness {hardness}: coverage {coverage} out of range at x={x}"
                );
                previous = coverage;
            }
        }
    }

    /// Hardness 1.0 is a hard edge: full coverage everywhere inside, nothing outside.
    ///
    /// Measured in Krita: with fade 1.0 the fade coefficient equals the containment coefficient, so `nf`
    /// equals `n` and every interior sample takes the fully-opaque branch.
    #[test]
    fn full_hardness_is_a_hard_edge() {
        let mask = plain(1.0, 40.0);
        for x in [0.0, 5.0, 10.0, 15.0, 19.0, 19.9] {
            assert_eq!(
                mask.coverage_at(x, 0.0),
                1.0,
                "inside a hard-edged dab, x={x} must be fully covered"
            );
        }
        assert_eq!(mask.coverage_at(20.1, 0.0), 0.0);
    }

    /// Softness at its ceiling also collapses to a hard edge, for the same reason.
    #[test]
    fn maximum_softness_also_collapses_to_hard() {
        let mask = DabMask::new(
            DabShape {
                hardness: 0.5,
                softness: 2.0,
                ratio: 1.0,
                antialias_edges: false,
                pencil: false,
            },
            40.0,
        );
        for x in [0.0, 10.0, 15.0, 19.0] {
            assert_eq!(mask.coverage_at(x, 0.0), 1.0, "at x={x}");
        }
        assert_eq!(mask.coverage_at(20.5, 0.0), 0.0);
    }

    /// Zero hardness and softness 0.1 produce identical masks, which is Krita's arithmetic and not a bug
    /// in either. Recorded as a test so the coincidence is not "fixed" by someone later.
    #[test]
    fn zero_hardness_equals_softness_one_tenth_at_this_diameter() {
        let zero_hardness = plain(0.0, 40.0);
        let soft = DabMask::new(
            DabShape {
                hardness: 0.5,
                softness: 0.1,
                ratio: 1.0,
                antialias_edges: false,
                pencil: false,
            },
            40.0,
        );
        // Krita printed 0, 15, 63, 143, 206, 255 for both at radius fractions 0, .25, .5, .75, .9, 1.
        for (fraction, want) in [(0.0, 0), (0.25, 15), (0.5, 63), (0.75, 143), (0.9, 206)] {
            let x = fraction * 20.0;
            assert!(
                zero_hardness.krita_value_at(x, 0.0).abs_diff(want) <= 1,
                "zero hardness at {fraction}R: want {want}, got {}",
                zero_hardness.krita_value_at(x, 0.0)
            );
            assert_eq!(
                zero_hardness.krita_value_at(x, 0.0),
                soft.krita_value_at(x, 0.0),
                "the two must coincide at {fraction}R, as measured in Krita"
            );
        }
    }

    /// Antialiasing shifts the falloff and is not cosmetic.
    #[test]
    fn antialias_edges_changes_the_falloff() {
        let shape = DabShape {
            hardness: 0.5,
            softness: 1.0,
            ratio: 1.0,
            antialias_edges: true,
            pencil: false,
        };
        let on = DabMask::new(shape, 40.0);
        let off = DabMask::new(
            DabShape {
                antialias_edges: false,
                pencil: false,
                ..shape
            },
            40.0,
        );
        // Krita printed 14 against 0 at half the radius, and 112 against 106 at three quarters.
        assert!(
            on.krita_value_at(10.0, 0.0).abs_diff(14) <= 1,
            "antialias on at 0.5R: Krita says 14, got {}",
            on.krita_value_at(10.0, 0.0)
        );
        assert_eq!(off.krita_value_at(10.0, 0.0), 0);
        assert!(
            on.krita_value_at(15.0, 0.0).abs_diff(112) <= 1,
            "antialias on at 0.75R: Krita says 112, got {}",
            on.krita_value_at(15.0, 0.0)
        );
        assert!(off.krita_value_at(15.0, 0.0).abs_diff(106) <= 1);
    }

    /// `ratio` makes the dab an ellipse: the same offset reads differently along each axis.
    #[test]
    fn ratio_makes_the_dab_elliptical() {
        let tall = DabMask::new(
            DabShape {
                hardness: 0.5,
                softness: 1.0,
                ratio: 0.5,
                antialias_edges: false,
                pencil: false,
            },
            40.0,
        );
        // Height is 20, so y = 10 is the vertical edge while x = 10 is still inside horizontally.
        assert_eq!(tall.coverage_at(0.0, 10.1), 0.0, "past the short axis");
        assert!(
            tall.coverage_at(10.0, 0.0) > 0.0,
            "but still inside along the long axis"
        );
        // A round dab reads the same on both axes; this one must not.
        let round = plain(0.5, 40.0);
        assert_eq!(round.coverage_at(0.0, 15.0), round.coverage_at(15.0, 0.0));
        assert_ne!(tall.coverage_at(0.0, 7.0), tall.coverage_at(7.0, 0.0));
    }

    /// The mask is symmetric in y, because Krita takes the absolute value.
    #[test]
    fn the_mask_is_symmetric() {
        let mask = plain(0.4, 31.0);
        for (x, y) in [(3.0, 5.0), (7.5, 1.25), (0.0, 12.0), (11.0, 0.0)] {
            assert_eq!(
                mask.coverage_at(x, y),
                mask.coverage_at(x, -y),
                "y symmetry"
            );
            assert_eq!(
                mask.coverage_at(x, y),
                mask.coverage_at(-x, y),
                "x symmetry"
            );
        }
    }

    /// A dab with no size covers nothing, and says so rather than dividing by zero.
    #[test]
    fn a_degenerate_dab_is_empty() {
        for diameter in [0.0, -5.0, f32::NAN, f32::INFINITY] {
            let mask = DabMask::new(DabShape::default(), diameter);
            assert!(
                mask.is_empty(),
                "diameter {diameter} must give an empty mask"
            );
            assert_eq!(mask.coverage_at(0.0, 0.0), 0.0);
        }
    }

    /// Non-finite and out-of-range shape fields are clamped inside the mask rather than producing NaN,
    /// even though the command boundary refuses them.
    #[test]
    fn an_invalid_shape_still_cannot_produce_nan() {
        for shape in [
            DabShape {
                hardness: f32::NAN,
                softness: 1.0,
                ratio: 1.0,
                antialias_edges: false,
                pencil: false,
            },
            DabShape {
                hardness: 0.5,
                softness: 0.0,
                ratio: 1.0,
                antialias_edges: false,
                pencil: false,
            },
            DabShape {
                hardness: -3.0,
                softness: 1.0,
                ratio: 0.0,
                antialias_edges: true,
                pencil: false,
            },
            DabShape {
                hardness: 0.5,
                softness: f32::INFINITY,
                ratio: f32::NAN,
                antialias_edges: false,
                pencil: false,
            },
        ] {
            let mask = DabMask::new(shape, 24.0);
            for step in 0..=240 {
                let coverage = mask.coverage_at(step as f32 * 0.1, 0.0);
                assert!(
                    coverage.is_finite() && (0.0..=1.0).contains(&coverage),
                    "shape {shape:?} produced {coverage}"
                );
            }
        }
    }

    /// The validity check is the command boundary's, and it must reject what it claims to.
    #[test]
    fn shape_validation_rejects_out_of_range_fields() {
        assert!(DabShape::default().is_valid());
        assert!(DabShape::round(0.0).is_valid());
        assert!(DabShape::round(1.0).is_valid());

        for shape in [
            DabShape::round(-0.01),
            DabShape::round(1.01),
            DabShape::round(f32::NAN),
            DabShape {
                softness: 0.0,
                ..DabShape::default()
            },
            DabShape {
                softness: 2.01,
                ..DabShape::default()
            },
            DabShape {
                ratio: 0.0,
                ..DabShape::default()
            },
            DabShape {
                ratio: f32::INFINITY,
                ..DabShape::default()
            },
        ] {
            assert!(!shape.is_valid(), "{shape:?} should be refused");
        }
    }

    /// The default shape must be the round dab a document saved before this field existed was drawn with.
    ///
    /// Full hardness with antialiasing on is NOT perfectly hard, and that is Krita's design rather than a
    /// defect. The +1.0 is added to BOTH coordinates, so a sample on the x axis gets `y = 1.0`, and the
    /// outermost pixel or so is feathered even at maximum hardness — which is what "correct antialiasing
    /// on the border" means. Measured in Krita at diameter 40: x=19 gives 5 of 255 and x=19.5 gives 125.
    ///
    /// The first version of this test asserted full coverage at x=19 and failed against a faithful
    /// translation.
    #[test]
    fn the_default_shape_is_round_and_antialiased() {
        let shape = DabShape::default();
        assert_eq!(shape.hardness, 1.0);
        assert_eq!(shape.ratio, 1.0);
        assert!(shape.antialias_edges);

        let mask = DabMask::new(shape, 40.0);
        assert_eq!(mask.coverage_at(0.0, 0.0), 1.0, "the centre is solid");
        assert_eq!(
            mask.coverage_at(18.0, 0.0),
            1.0,
            "and solid up to the last pixel"
        );
        // Krita: 5 of 255, so coverage 0.980.
        assert!(
            (mask.coverage_at(19.0, 0.0) - 0.980).abs() < 0.01,
            "the border is feathered, Krita gives 0.980, got {}",
            mask.coverage_at(19.0, 0.0)
        );
        assert!(
            (mask.coverage_at(19.5, 0.0) - 0.510).abs() < 0.01,
            "Krita gives 0.510 at x=19.5, got {}",
            mask.coverage_at(19.5, 0.0)
        );
        assert_eq!(
            mask.coverage_at(21.0, 0.0),
            0.0,
            "and nothing past the radius"
        );

        // With antialiasing off the same shape IS perfectly hard, which is the contrast that shows the
        // feathering above comes from the flag and not from the hardness.
        let sharp = DabMask::new(
            DabShape {
                antialias_edges: false,
                pencil: false,
                ..shape
            },
            40.0,
        );
        for x in [0.0, 10.0, 18.0, 19.0, 19.5, 19.9] {
            assert_eq!(sharp.coverage_at(x, 0.0), 1.0, "hard to the edge at x={x}");
        }
        assert_eq!(sharp.coverage_at(20.1, 0.0), 0.0);
    }

    #[test]
    fn pencil_thresholds_a_soft_dab_to_a_hard_edge() {
        // B.1 pencil: a soft dab (hardness < 1 feathers over its whole extent) still comes out as a
        // crisp 0/1 disc when antialiasing is off -- the falloff is thresholded at coverage 0.5. The
        // antialiased twin feathers at the same samples, which is the contrast that proves it.
        let soft = DabShape {
            hardness: 0.3,
            antialias_edges: true,
            pencil: false,
            ..DabShape::default()
        };
        let hard = DabShape {
            pencil: true,
            ..soft
        };
        let soft_mask = DabMask::new(soft, 40.0);
        let hard_mask = DabMask::new(hard, 40.0);
        let mut saw_feather = false;
        for x in [6.0_f32, 8.0, 10.0, 12.0, 14.0, 16.0] {
            let c = soft_mask.coverage_at(x, 0.0);
            if c > 0.0 && c < 1.0 {
                saw_feather = true;
            }
            let h = hard_mask.coverage_at(x, 0.0);
            assert!(
                h == 0.0 || h == 1.0,
                "pencil coverage is 0 or 1, got {h} at x={x}"
            );
        }
        assert!(
            saw_feather,
            "the soft twin must actually feather, or the test proves nothing"
        );
    }

    /// At exactly the edge with full hardness, Krita divides zero by zero.
    ///
    /// `n` and `nf` are both exactly 1.0 there, so `255 * n * (nf - 1) / (nf - n)` is `0/0`. Krita casts
    /// the resulting NaN to `quint8` — undefined behaviour that happens to produce 0, fully opaque, on
    /// x86. This returns full coverage at that sample for the same outcome by a stated rule instead of an
    /// accident, because a NaN reaching the dab loop would be written into a pixel.
    #[test]
    fn the_exact_edge_does_not_produce_nan() {
        let mask = DabMask::new(
            DabShape {
                hardness: 1.0,
                softness: 1.0,
                ratio: 1.0,
                antialias_edges: false,
                pencil: false,
            },
            40.0,
        );
        let at_edge = mask.coverage_at(20.0, 0.0);
        assert!(at_edge.is_finite(), "the exact edge must not be NaN");
        assert_eq!(at_edge, 1.0, "and matches Krita's observed 0, fully opaque");

        // Every sample of every hardness must be finite, which is the general form of the same worry.
        for hardness in [0.0, 0.5, 1.0] {
            for softness in [0.01, 1.0, 2.0] {
                let mask = DabMask::new(
                    DabShape {
                        hardness,
                        softness,
                        ratio: 1.0,
                        antialias_edges: true,
                        pencil: false,
                    },
                    40.0,
                );
                for step in 0..=500 {
                    let coverage = mask.coverage_at(step as f32 * 0.05, 0.0);
                    assert!(
                        coverage.is_finite(),
                        "hardness {hardness} softness {softness} gave {coverage}"
                    );
                }
            }
        }
    }

    /// Scaling the diameter scales the mask, rather than changing its shape.
    #[test]
    fn the_mask_scales_with_diameter() {
        let small = plain(0.5, 20.0);
        let large = plain(0.5, 80.0);
        for fraction in [0.0, 0.25, 0.5, 0.6, 0.7, 0.8, 0.95] {
            let from_small = small.coverage_at(fraction * 10.0, 0.0);
            let from_large = large.coverage_at(fraction * 40.0, 0.0);
            assert!(
                (from_small - from_large).abs() < 0.02,
                "at {fraction} of the radius: {from_small} against {from_large}"
            );
        }
    }
}
