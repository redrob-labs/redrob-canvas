//! A user-editable tone curve: a natural cubic spline through control points, sampled into a transfer
//! table.
//!
//! TRANSLATED from Krita, pinned at `fdbf33b2146735465bb8aa59928fbc1890ceb160`, GPL-2.0-or-later, taken
//! into this GPL-3.0-or-later product under the or-later grant:
//!
//! - `libs/image/kis_cubic_curve.{h,cpp}` — `KisCubicCurve`, its clamping and its transfer table
//! - `libs/image/kis_cubic_curve_spline.h` — `KisCubicSpline`, `KisTridiagonalSystem`
//!
//! Licence change notice: the originals are GPL-2.0-or-later; this file is GPL-3.0-or-later, which the
//! or-later grant permits. Attribution is in `UPSTREAM_NOTICES.md`.
//!
//! # Why this one, when most of its block was already covered
//!
//! Item 1c.2 block 3 was planned as 8,716 lines. Measured against the current product, nearly all of it
//! is transform tooling this product does not have — liquify, warp, perspective, grid interpolation and
//! four-point interpolators, none of which has anything to call it. What is left is this: the product
//! ships a `Levels` filter, which is a black point, a white point and a gamma, and has no way to express
//! an arbitrary transfer curve. That is a real gap with a real consumer, and `KisCubicCurve` is the
//! authority for it.
//!
//! # The algorithm, and where it departs
//!
//! Krita's current implementation assembles a 4n × 4n sparse system in the monomial basis and solves it
//! with Eigen. This solves the same spline with the **tridiagonal (Thomas) algorithm** in O(n) and needs
//! no linear-algebra dependency, which the product does not carry and would not be worth adding for a
//! curve of a dozen points.
//!
//! That is only valid because a corner knot makes the system SEPARABLE, which was measured rather than
//! assumed: a corner imposes a zero second derivative on both sides instead of matching derivatives, so
//! each run between corners is an independent natural spline. Compiling Krita's own Eigen assembly and
//! comparing a five-point curve with a corner against the two runs solved separately gives a maximum
//! difference of **1.7e-15**. Every reference value in this module's tests came from that binary.

use serde::{Deserialize, Serialize};

/// One control point. `corner` breaks smoothness at this point, giving a deliberate kink.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
    /// When set, the curve is continuous here but its slope need not be.
    ///
    /// Krita spells this `isSetAsCorner()`. Mathematically it replaces the two continuity equations at
    /// this knot with a zero second derivative on each side, which is what makes the runs independent.
    #[serde(default)]
    pub corner: bool,
}

impl CurvePoint {
    pub const fn smooth(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            corner: false,
        }
    }

    pub const fn corner(x: f32, y: f32) -> Self {
        Self { x, y, corner: true }
    }
}

/// The largest number of control points a curve may carry.
///
/// A bound rather than an opinion: the points arrive from a serialised command, and an unbounded list
/// would let a document allocate without limit. 64 is far past any hand-placed curve.
pub const MAX_CURVE_POINTS: usize = 64;

/// One cubic piece, in the offset form `a + b·dx + c/2·dx² + d/6·dx³`.
///
/// Offset from the interval start rather than Krita's absolute-x monomial basis. Same curve, better
/// conditioned — `dx` is bounded by the interval width where `x³` is not — and it is the form the
/// tridiagonal solution produces directly.
#[derive(Clone, Copy, Debug)]
struct Piece {
    x0: f32,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
}

/// A transfer curve built from control points.
#[derive(Clone, Debug)]
pub struct ToneCurve {
    points: Vec<CurvePoint>,
    pieces: Vec<Piece>,
}

impl ToneCurve {
    /// The identity curve, which is where a tone-curve editor starts.
    pub fn identity() -> Self {
        Self::new(vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::smooth(1.0, 1.0),
        ])
        .expect("the identity curve is valid")
    }

    /// Builds a curve from control points, sorting them by `x`.
    ///
    /// Krita keeps its point list sorted and leaves an out-of-range or duplicate `x` to chance. Here a
    /// non-finite coordinate and a duplicated `x` are both refused: a duplicate gives a zero-width
    /// interval, and the spline's first step divides by that width. Krita divides by it too.
    pub fn new(mut points: Vec<CurvePoint>) -> Result<Self, ToneCurveError> {
        if points.is_empty() {
            return Err(ToneCurveError::NoPoints);
        }
        if points.len() > MAX_CURVE_POINTS {
            return Err(ToneCurveError::TooManyPoints {
                max: MAX_CURVE_POINTS,
            });
        }
        for point in &points {
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(ToneCurveError::NonFiniteCoordinate);
            }
        }
        points.sort_by(|left, right| left.x.total_cmp(&right.x));
        if points.windows(2).any(|pair| pair[0].x == pair[1].x) {
            return Err(ToneCurveError::DuplicateX);
        }

        let pieces = build_pieces(&points);
        Ok(Self { points, pieces })
    }

    pub fn points(&self) -> &[CurvePoint] {
        &self.points
    }

    /// The curve's value at `x`.
    ///
    /// Both clamps are Krita's and both matter. `x` is clamped into the control points' range, so the
    /// curve extends flat rather than letting the outermost cubic run away — a cubic continued past its
    /// last knot leaves [0, 1] almost immediately. `y` is then clamped to [0, 1], because a natural
    /// spline through points inside the unit square can still overshoot between them.
    pub fn value(&self, x: f32) -> f32 {
        let first = self.points[0].x;
        let last = self.points[self.points.len() - 1].x;
        let x = x.clamp(first, last);
        let piece = self.piece_for(x);
        let dx = f64::from(x - piece.x0);
        let y =
            piece.a + piece.b * dx + 0.5 * piece.c * dx * dx + (1.0 / 6.0) * piece.d * dx * dx * dx;
        (y as f32).clamp(0.0, 1.0)
    }

    fn piece_for(&self, x: f32) -> &Piece {
        // The last piece whose start is at or before x. Krita scans forward for the first interval whose
        // right edge exceeds x, which is the same choice expressed from the other side.
        let mut chosen = &self.pieces[0];
        for piece in &self.pieces {
            if piece.x0 <= x {
                chosen = piece;
            } else {
                break;
            }
        }
        chosen
    }

    /// Samples the curve into a transfer table of `size` entries, mapping `0..=size-1` onto the curve's
    /// `x` domain `0..=1`.
    ///
    /// This is Krita's `updateTransfer`. The step is `1 / (size - 1)` so the last entry samples exactly
    /// `x = 1`; a step of `1 / size` would stop short and darken the white point.
    pub fn transfer_table(&self, size: usize) -> Vec<u8> {
        if size == 0 {
            return Vec::new();
        }
        if size == 1 {
            return vec![(self.value(0.0) * 255.0).round() as u8];
        }
        let step = 1.0 / (size - 1) as f32;
        (0..size)
            .map(|index| {
                let value = self.value(index as f32 * step);
                (value * 255.0).round().clamp(0.0, 255.0) as u8
            })
            .collect()
    }

    /// The 256-entry table an 8-bit channel is remapped through.
    pub fn transfer_table_8bit(&self) -> Vec<u8> {
        self.transfer_table(256)
    }
}

/// Why a curve could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ToneCurveError {
    #[error("a tone curve needs at least one point")]
    NoPoints,
    #[error("a tone curve may carry at most {max} points")]
    TooManyPoints { max: usize },
    #[error("a tone curve point must have finite coordinates")]
    NonFiniteCoordinate,
    #[error("two tone curve points share an x coordinate, which has no curve through it")]
    DuplicateX,
}

/// Splits the points at corners and solves each run as an independent natural cubic spline.
fn build_pieces(points: &[CurvePoint]) -> Vec<Piece> {
    // A single point is a constant, which is Krita's own special case.
    if points.len() == 1 {
        return vec![Piece {
            x0: points[0].x,
            a: f64::from(points[0].y),
            b: 0.0,
            c: 0.0,
            d: 0.0,
        }];
    }

    // Run boundaries: the ends, plus every interior corner. A corner belongs to both runs it divides,
    // as the last point of one and the first of the next, so the curve stays continuous there.
    let mut boundaries = vec![0usize];
    for (index, point) in points.iter().enumerate().skip(1).take(points.len() - 2) {
        if point.corner {
            boundaries.push(index);
        }
    }
    boundaries.push(points.len() - 1);

    let mut pieces = Vec::with_capacity(points.len() - 1);
    for pair in boundaries.windows(2) {
        pieces.extend(natural_spline(&points[pair[0]..=pair[1]]));
    }
    pieces
}

/// A natural cubic spline through `points`, which must hold at least two.
fn natural_spline(points: &[CurvePoint]) -> Vec<Piece> {
    let intervals = points.len() - 1;
    debug_assert!(intervals >= 1);

    let widths: Vec<f64> = (0..intervals)
        .map(|i| f64::from(points[i + 1].x) - f64::from(points[i].x))
        .collect();
    let heights: Vec<f64> = points.iter().map(|point| f64::from(point.y)).collect();

    // Second derivatives at the knots. Natural boundary conditions put zero at each end, so only the
    // interior ones are solved for. Two points leave nothing to solve and give a straight line, which is
    // Krita's other special case.
    let mut second = vec![0.0; points.len()];
    if intervals > 1 {
        let unknowns = intervals - 1;
        // The tridiagonal system, exactly Krita's KisTridiagonalSystem: `off` is both the sub- and the
        // super-diagonal because a natural spline's matrix is symmetric.
        let mut diagonal = Vec::with_capacity(unknowns);
        let mut rhs = Vec::with_capacity(unknowns);
        for i in 0..unknowns {
            diagonal.push(2.0 * (widths[i] + widths[i + 1]));
            rhs.push(
                6.0 * ((heights[i + 2] - heights[i + 1]) / widths[i + 1]
                    - (heights[i + 1] - heights[i]) / widths[i]),
            );
        }
        let off: Vec<f64> = (1..unknowns).map(|i| widths[i]).collect();
        let solved = solve_tridiagonal(&off, &diagonal, &rhs);
        second[1..=unknowns].copy_from_slice(&solved);
    }

    (0..intervals)
        .map(|i| {
            let h = widths[i];
            let d = (second[i + 1] - second[i]) / h;
            let b =
                (heights[i + 1] - heights[i]) / h - 0.5 * second[i] * h - (1.0 / 6.0) * d * h * h;
            Piece {
                x0: points[i].x,
                a: heights[i],
                b,
                c: second[i],
                d,
            }
        })
        .collect()
}

/// The Thomas algorithm for a symmetric tridiagonal system.
///
/// `off` is the shared sub- and super-diagonal and holds one fewer entry than `diagonal`. Translated
/// from `KisTridiagonalSystem::calculate`, whose forward sweep and back substitution this is.
fn solve_tridiagonal(off: &[f64], diagonal: &[f64], rhs: &[f64]) -> Vec<f64> {
    let size = diagonal.len();
    debug_assert_eq!(off.len(), size.saturating_sub(1));
    debug_assert_eq!(rhs.len(), size);
    let mut solution = vec![0.0; size];
    if size == 0 {
        return solution;
    }
    if size == 1 {
        solution[0] = rhs[0] / diagonal[0];
        return solution;
    }

    let mut alpha = vec![0.0; size];
    let mut beta = vec![0.0; size];
    alpha[1] = -off[0] / diagonal[0];
    beta[1] = rhs[0] / diagonal[0];
    for i in 1..size - 1 {
        let denominator = off[i - 1] * alpha[i] + diagonal[i];
        alpha[i + 1] = -off[i] / denominator;
        beta[i + 1] = (rhs[i] - off[i - 1] * beta[i]) / denominator;
    }
    let last = size - 1;
    solution[last] =
        (rhs[last] - off[last - 1] * beta[last]) / (diagonal[last] + off[last - 1] * alpha[last]);
    for i in (0..last).rev() {
        solution[i] = alpha[i + 1] * solution[i + 1] + beta[i + 1];
    }
    solution
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every expected value here was produced by compiling Krita's own `KisCubicSpline` against real
    /// Eigen 3 and printing it at twelve decimal places. The tolerance is f32's, not the reference's.
    fn assert_curve(curve: &ToneCurve, expected: &[(f32, f32)]) {
        for &(x, want) in expected {
            let got = curve.value(x);
            assert!(
                (got - want).abs() < 2e-6,
                "at x={x}: expected Krita's {want}, got {got}"
            );
        }
    }

    #[test]
    fn the_identity_curve_matches_upstream() {
        let curve = ToneCurve::identity();
        assert_curve(
            &curve,
            &[
                (0.0, 0.0),
                (0.1, 0.1),
                (0.25, 0.25),
                (0.5, 0.5),
                (0.75, 0.75),
                (1.0, 1.0),
            ],
        );
    }

    /// The commonest adjustment: one interior point lifting the midtones.
    #[test]
    fn a_three_point_curve_matches_upstream() {
        let curve = ToneCurve::new(vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::smooth(0.5, 0.7),
            CurvePoint::smooth(1.0, 1.0),
        ])
        .unwrap();
        assert_curve(
            &curve,
            &[
                (0.0, 0.0),
                (0.1, 0.159_2),
                (0.25, 0.387_5),
                (0.4, 0.588_8),
                (0.5, 0.7),
                (0.6, 0.788_8),
                (0.75, 0.887_5),
                (0.9, 0.959_2),
                (1.0, 1.0),
            ],
        );
    }

    #[test]
    fn a_four_point_curve_matches_upstream() {
        let curve = ToneCurve::new(vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::smooth(0.25, 0.1),
            CurvePoint::smooth(0.75, 0.9),
            CurvePoint::smooth(1.0, 1.0),
        ])
        .unwrap();
        assert_curve(
            &curve,
            &[
                (0.0, 0.0),
                (0.1, 0.014_8),
                (0.25, 0.1),
                (0.4, 0.314_8),
                (0.5, 0.5),
                (0.6, 0.685_2),
                (0.75, 0.9),
                (0.9, 0.985_2),
                (1.0, 1.0),
            ],
        );
    }

    /// A corner knot. The values differ from the smooth case at every sampled point, which is what
    /// proves the flag is doing something rather than being carried and ignored.
    #[test]
    fn a_corner_point_matches_upstream_and_differs_from_smooth() {
        let cornered = ToneCurve::new(vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::corner(0.5, 0.7),
            CurvePoint::smooth(1.0, 1.0),
        ])
        .unwrap();
        // Krita's values: a corner at the only interior knot makes both halves straight lines, because a
        // two-interval run with zero second derivatives at both ends is linear.
        assert_curve(
            &cornered,
            &[
                (0.0, 0.0),
                (0.1, 0.14),
                (0.25, 0.35),
                (0.4, 0.56),
                (0.5, 0.7),
                (0.6, 0.76),
                (0.75, 0.85),
                (0.9, 0.94),
                (1.0, 1.0),
            ],
        );

        let smooth = ToneCurve::new(vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::smooth(0.5, 0.7),
            CurvePoint::smooth(1.0, 1.0),
        ])
        .unwrap();
        for x in [0.1, 0.25, 0.4, 0.6, 0.75, 0.9] {
            assert!(
                (cornered.value(x) - smooth.value(x)).abs() > 1e-3,
                "the corner flag must change the curve at x={x}"
            );
        }
    }

    /// Five points with an interior corner: the case that proved separability, so the case that would
    /// break if the runs were solved together or the boundary point were dropped from one of them.
    #[test]
    fn a_five_point_curve_with_a_corner_matches_upstream() {
        let curve = ToneCurve::new(vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::smooth(0.25, 0.3),
            CurvePoint::corner(0.5, 0.7),
            CurvePoint::smooth(0.75, 0.8),
            CurvePoint::smooth(1.0, 1.0),
        ])
        .unwrap();
        assert_curve(
            &curve,
            &[
                (0.0, 0.0),
                (0.1, 0.111_6),
                (0.25, 0.3),
                (0.4, 0.531_6),
                (0.5, 0.7),
                (0.6, 0.731_6),
                (0.75, 0.8),
                (0.9, 0.911_6),
                (1.0, 1.0),
            ],
        );
    }

    #[test]
    fn a_single_point_is_a_constant() {
        let curve = ToneCurve::new(vec![CurvePoint::smooth(0.3, 0.42)]).unwrap();
        for x in [0.0, 0.1, 0.3, 0.5, 0.9, 1.0] {
            assert!(
                (curve.value(x) - 0.42).abs() < 1e-6,
                "a one-point curve is flat everywhere, at x={x} got {}",
                curve.value(x)
            );
        }
    }

    /// The curve passes through every control point it was given.
    #[test]
    fn the_curve_interpolates_its_control_points() {
        let points = vec![
            CurvePoint::smooth(0.0, 0.05),
            CurvePoint::smooth(0.2, 0.4),
            CurvePoint::corner(0.55, 0.3),
            CurvePoint::smooth(0.8, 0.85),
            CurvePoint::smooth(1.0, 0.95),
        ];
        let curve = ToneCurve::new(points.clone()).unwrap();
        for point in &points {
            let got = curve.value(point.x);
            assert!(
                (got - point.y).abs() < 2e-6,
                "the curve must pass through ({}, {}), got {got}",
                point.x,
                point.y
            );
        }
    }

    /// x outside the control range extends flat rather than continuing the outermost cubic.
    #[test]
    fn x_is_clamped_into_the_control_range() {
        let curve = ToneCurve::new(vec![
            CurvePoint::smooth(0.2, 0.3),
            CurvePoint::smooth(0.5, 0.6),
            CurvePoint::smooth(0.8, 0.4),
        ])
        .unwrap();
        assert!((curve.value(0.0) - curve.value(0.2)).abs() < 1e-6);
        assert!((curve.value(-5.0) - curve.value(0.2)).abs() < 1e-6);
        assert!((curve.value(1.0) - curve.value(0.8)).abs() < 1e-6);
        assert!((curve.value(99.0) - curve.value(0.8)).abs() < 1e-6);
    }

    /// y is clamped to [0, 1], which a natural spline through in-range points genuinely needs.
    #[test]
    fn y_is_clamped_to_the_unit_range() {
        // A steep rise followed by a plateau overshoots above 1 between the knots.
        let curve = ToneCurve::new(vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::smooth(0.1, 0.9),
            CurvePoint::smooth(0.2, 0.98),
            CurvePoint::smooth(1.0, 1.0),
        ])
        .unwrap();
        for step in 0..=1_000 {
            let value = curve.value(step as f32 / 1_000.0);
            assert!(
                (0.0..=1.0).contains(&value),
                "value {value} escaped the unit range"
            );
        }

        // And a downward overshoot below 0 is clamped too.
        let dip = ToneCurve::new(vec![
            CurvePoint::smooth(0.0, 0.5),
            CurvePoint::smooth(0.4, 0.02),
            CurvePoint::smooth(0.6, 0.02),
            CurvePoint::smooth(1.0, 0.5),
        ])
        .unwrap();
        for step in 0..=1_000 {
            let value = dip.value(step as f32 / 1_000.0);
            assert!((0.0..=1.0).contains(&value), "value {value} escaped");
        }
    }

    /// The transfer table's last entry samples x = 1 exactly.
    #[test]
    fn the_transfer_table_reaches_both_ends() {
        let table = ToneCurve::identity().transfer_table_8bit();
        assert_eq!(table.len(), 256);
        assert_eq!(table[0], 0, "the identity maps black to black");
        assert_eq!(
            table[255], 255,
            "and white to white; a step of 1/size instead of 1/(size-1) darkens this entry"
        );
        // Monotone and evenly spaced, which is what makes it the identity.
        for index in 1..256 {
            assert!(table[index] >= table[index - 1]);
        }
        assert_eq!(table[128], 128);
    }

    #[test]
    fn a_lifting_curve_produces_a_lifting_table() {
        let curve = ToneCurve::new(vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::smooth(0.5, 0.7),
            CurvePoint::smooth(1.0, 1.0),
        ])
        .unwrap();
        let table = curve.transfer_table_8bit();
        assert_eq!(table[0], 0);
        assert_eq!(table[255], 255);
        // 0.7 * 255 = 178.5, and the midpoint entry samples x = 128/255 which is just above 0.5.
        assert!(
            (179..=180).contains(&table[128]),
            "the midtone should lift to about 179, got {}",
            table[128]
        );
        assert!(table[128] > 128, "and it must be a lift, not a no-op");
    }

    #[test]
    fn a_table_of_odd_sizes_does_not_panic() {
        let curve = ToneCurve::identity();
        assert!(curve.transfer_table(0).is_empty());
        assert_eq!(curve.transfer_table(1).len(), 1);
        assert_eq!(curve.transfer_table(2), vec![0, 255]);
        assert_eq!(curve.transfer_table(3), vec![0, 128, 255]);
    }

    /// Points arrive from a serialised command, so the constructor is the boundary.
    #[test]
    fn invalid_point_lists_are_refused() {
        assert_eq!(
            ToneCurve::new(Vec::new()).unwrap_err(),
            ToneCurveError::NoPoints
        );

        let too_many: Vec<CurvePoint> = (0..=MAX_CURVE_POINTS)
            .map(|i| CurvePoint::smooth(i as f32 / 100.0, 0.5))
            .collect();
        assert_eq!(
            ToneCurve::new(too_many).unwrap_err(),
            ToneCurveError::TooManyPoints {
                max: MAX_CURVE_POINTS
            }
        );

        assert_eq!(
            ToneCurve::new(vec![
                CurvePoint::smooth(0.0, 0.0),
                CurvePoint::smooth(f32::NAN, 1.0),
            ])
            .unwrap_err(),
            ToneCurveError::NonFiniteCoordinate
        );
        assert_eq!(
            ToneCurve::new(vec![
                CurvePoint::smooth(0.0, 0.0),
                CurvePoint::smooth(f32::INFINITY, 1.0),
            ])
            .unwrap_err(),
            ToneCurveError::NonFiniteCoordinate
        );

        // A duplicated x gives a zero-width interval, and the spline divides by that width.
        assert_eq!(
            ToneCurve::new(vec![
                CurvePoint::smooth(0.0, 0.0),
                CurvePoint::smooth(0.5, 0.2),
                CurvePoint::smooth(0.5, 0.8),
                CurvePoint::smooth(1.0, 1.0),
            ])
            .unwrap_err(),
            ToneCurveError::DuplicateX
        );
    }

    /// Points given out of order are sorted, as Krita's `keepSorted` does.
    #[test]
    fn points_are_sorted_on_construction() {
        let curve = ToneCurve::new(vec![
            CurvePoint::smooth(1.0, 1.0),
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::smooth(0.5, 0.7),
        ])
        .unwrap();
        let xs: Vec<f32> = curve.points().iter().map(|point| point.x).collect();
        assert_eq!(xs, vec![0.0, 0.5, 1.0]);
        // And it is the same curve as the sorted input.
        assert!((curve.value(0.25) - 0.387_5).abs() < 2e-6);
    }

    /// A corner at an END is not a run boundary: there is no run beyond it to separate from.
    #[test]
    fn a_corner_at_an_endpoint_changes_nothing() {
        let plain = ToneCurve::new(vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::smooth(0.5, 0.7),
            CurvePoint::smooth(1.0, 1.0),
        ])
        .unwrap();
        let ends_marked = ToneCurve::new(vec![
            CurvePoint::corner(0.0, 0.0),
            CurvePoint::smooth(0.5, 0.7),
            CurvePoint::corner(1.0, 1.0),
        ])
        .unwrap();
        for step in 0..=100 {
            let x = step as f32 / 100.0;
            assert!(
                (plain.value(x) - ends_marked.value(x)).abs() < 1e-6,
                "a corner on an endpoint should not alter the curve, differs at x={x}"
            );
        }
    }

    /// Two adjacent corners make the piece between them a straight line, since both its ends carry a
    /// zero second derivative.
    #[test]
    fn adjacent_corners_give_a_straight_segment() {
        let curve = ToneCurve::new(vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::corner(0.3, 0.2),
            CurvePoint::corner(0.7, 0.9),
            CurvePoint::smooth(1.0, 1.0),
        ])
        .unwrap();
        // Midway between the corners the value must be the linear interpolation of them.
        let expected = 0.2 + (0.9 - 0.2) * 0.5;
        assert!(
            (curve.value(0.5) - expected).abs() < 2e-6,
            "expected the straight-line value {expected}, got {}",
            curve.value(0.5)
        );
    }
}
