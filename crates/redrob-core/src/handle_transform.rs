// SPDX-License-Identifier: GPL-3.0-or-later

//! Handle transform: 1 to 4 pinned handles deform the layer (L.3).
//!
//! Re-derived from `app/tools/gimphandletransformtool.c`,
//! `app/display/gimptoolhandlegrid.c` and `app/core/gimp-transform-utils.c` (GPL-3.0-or-later),
//! which this repository pins and attributes in `docs/upstream-sources.toml`.
//!
//! # The handle COUNT is the transform class, and that is read rather than guessed
//!
//! The solver upstream reaches in the end is always the same one — `gimp_transform_matrix_generic`
//! takes four input points and four output points and solves one 8-unknown system. The count never
//! reaches it. What the count decides is `gimptoolhandlegrid.c`'s `switch (n_handles)` inside a
//! DRAG: when the user moves one handle by `diff`, that switch moves the other three first, so the
//! four correspondences handed to the solver already encode a restricted transform.
//!
//! - **1** — `for (i = 0; i < 4; i++) newpos[i] = oldpos[i] + diff`. Every corner moves by the same
//!   amount: a translation.
//! - **2** — a `scale` from the length ratio and an `angle`, then corners 2 and 3 are rotated and
//!   scaled about the one other visible handle. Rotation plus uniform scale: a similarity.
//! - **3** — only `newpos[3]`, the invisible fourth corner, is slid by `scale * diff`, keeping the
//!   quadrilateral a parallelogram: an affine map.
//! - **4** — no case in the switch, so only the dragged handle moves: a full projective map.
//!
//! # Closed form rather than the drag arithmetic, and the equivalence was checked
//!
//! A command has no mouse and no `diff`, so the incremental form cannot be ported literally. What
//! is ported is the class each count restricts to, in closed form: the unique translation,
//! similarity, affine or projective map carrying the given handles to their destinations.
//!
//! That is faithful rather than an approximation, and the reason is worth writing down. Each drag
//! step in `case 2` fixes the other visible handle and carries the dragged one from its old to its
//! new position; a composition of such steps is still a similarity, and it still sends each visible
//! handle's original position to its current one, because the handle that is not being dragged does
//! not move during the step. So the cumulative map is exactly the similarity determined by the two
//! correspondences. The same argument gives affine for three and projective for four — and for one
//! handle the composition of translations is the translation between that handle's endpoints.
//!
//! # What is NOT ported, because it belongs to a layer that does not exist here
//!
//! Two pieces of the drag arithmetic are artefacts of the incremental formulation and have no
//! closed-form counterpart to be faithful to:
//!
//! - `calc_angle` returns `direction < 0 ? angle : 2 * G_PI - angle`, and `case 2` then applies
//!   `[[cos, sin], [-sin, cos]]` — the TRANSPOSE of the usual rotation matrix. The two conventions
//!   are chosen to cancel: with a standard `atan2` difference and a standard rotation matrix the
//!   result is the same, and mixing one with the other rotates the opposite way. There is no angle
//!   to choose a convention for when the similarity is solved from two point pairs.
//! - `calc_lineintersect_ratio` returns **1.0** when its denominator is exactly zero, with the
//!   comment *"u is infinite, so u/(u-1) is 1"*. That keeps the shear working when the two lines
//!   are parallel. It is reached only from `case 3`'s per-drag slide.
//!
//! # Validity, read from `gimp_transform_matrix_generic`
//!
//! After solving, it checks every input point's `w = h20 * x + h21 * y + 1`. `|w| <= EPSILON`
//! (1e-6) means that point maps to infinity, and a DIFFERENCE IN SIGN between points means they
//! straddle the camera plane; either one makes the transform invalid. Both are implemented here.
//!
//! Its third step is not: when every `w` is negative it negates all nine coefficients. That is
//! observationally inert for mapping points — `(-n) / (-w)` is `n / w` — and exists because that
//! function also hands the matrix to a dialog that prints it. Checked rather than assumed.

use crate::{CoreError, Result};

/// The largest number of handles. Upstream's arrays are `[4]` and its add path is gated on
/// `n_handles < 4`; a fifth correspondence would also over-determine the projective solve.
pub const MAX_HANDLES: usize = 4;

/// Which class of transform a handle count restricts to.
///
/// Derived from the count rather than stored beside it. Upstream keeps `n_handles` in a separate
/// slot because its handle storage is a fixed `[4]` array that is always full; a list of exactly
/// the handles the user placed carries its own count, and storing it twice would let the two drift.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandleTransformClass {
    /// One handle: translation.
    Translation,
    /// Two handles: rotation and uniform scale about them.
    Similarity,
    /// Three handles: shear and non-uniform scale included.
    Affine,
    /// Four handles: full perspective.
    Projective,
}

impl HandleTransformClass {
    /// The class `n` handles restrict to, or `None` outside 1..=4.
    pub fn for_handle_count(n: usize) -> Option<Self> {
        match n {
            1 => Some(Self::Translation),
            2 => Some(Self::Similarity),
            3 => Some(Self::Affine),
            4 => Some(Self::Projective),
            _ => None,
        }
    }
}

/// `EPSILON` from `gimp-transform-utils.c`, used for the maps-to-infinity test.
const EPSILON: f64 = 1e-6;

/// The forward matrix `[a, b, c, d, e, f, g, h, 1]` for
/// `x' = (a x + b y + c) / (g x + h y + 1)`, `y' = (d x + e y + f) / (g x + h y + 1)`.
///
/// `src` and `dst` must be the same length, 1 to 4. The length picks the class.
pub(crate) fn handle_transform_matrix(src: &[(f64, f64)], dst: &[(f64, f64)]) -> Result<[f64; 9]> {
    if src.len() != dst.len() {
        return Err(CoreError::InvalidTransform);
    }
    let class =
        HandleTransformClass::for_handle_count(src.len()).ok_or(CoreError::InvalidTransform)?;

    match class {
        HandleTransformClass::Translation => {
            let tx = dst[0].0 - src[0].0;
            let ty = dst[0].1 - src[0].1;
            Ok([1.0, 0.0, tx, 0.0, 1.0, ty, 0.0, 0.0, 1.0])
        }
        HandleTransformClass::Similarity => {
            // A similarity is one complex multiply plus a translation, so the rotation and the
            // uniform scale come out together as `a + b i = (dst1 - dst0) / (src1 - src0)`. Writing
            // it this way is why no angle convention has to be chosen: there is no `acos` and no
            // sign branch to get backwards.
            let (dx, dy) = (src[1].0 - src[0].0, src[1].1 - src[0].1);
            let (ex, ey) = (dst[1].0 - dst[0].0, dst[1].1 - dst[0].1);
            let len2 = dx * dx + dy * dy;
            // The two handles coincide, so the direction they define does not exist.
            if len2 <= EPSILON * EPSILON {
                return Err(CoreError::InvalidTransform);
            }
            let a = (ex * dx + ey * dy) / len2;
            let b = (ey * dx - ex * dy) / len2;
            let tx = dst[0].0 - (a * src[0].0 - b * src[0].1);
            let ty = dst[0].1 - (b * src[0].0 + a * src[0].1);
            Ok([a, -b, tx, b, a, ty, 0.0, 0.0, 1.0])
        }
        HandleTransformClass::Affine => {
            // Two independent 3x3 solves sharing one matrix, so one determinant decides both. It is
            // zero exactly when the three source handles are collinear, which is the degenerate
            // case a user reaches by lining three handles up.
            let [(x0, y0), (x1, y1), (x2, y2)] = [src[0], src[1], src[2]];
            let det = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
            if det.abs() <= EPSILON {
                return Err(CoreError::InvalidTransform);
            }
            let solve = |u0: f64, u1: f64, u2: f64| -> (f64, f64, f64) {
                let a = ((u1 - u0) * (y2 - y0) - (u2 - u0) * (y1 - y0)) / det;
                let b = ((u2 - u0) * (x1 - x0) - (u1 - u0) * (x2 - x0)) / det;
                let c = u0 - a * x0 - b * y0;
                (a, b, c)
            };
            let (a, b, c) = solve(dst[0].0, dst[1].0, dst[2].0);
            let (d, e, f) = solve(dst[0].1, dst[1].1, dst[2].1);
            Ok([a, b, c, d, e, f, 0.0, 0.0, 1.0])
        }
        HandleTransformClass::Projective => {
            let src4 = [src[0], src[1], src[2], src[3]];
            let dst4 = [dst[0], dst[1], dst[2], dst[3]];
            crate::document::homography(src4, dst4).ok_or(CoreError::InvalidTransform)
        }
    }
}

/// Upstream's validity test on the solved matrix, run against the points it was solved from.
///
/// `|w| <= EPSILON` means a point maps to infinity; a sign difference between points means they
/// straddle the camera plane. Either makes the transform invalid.
pub(crate) fn projective_validity(matrix: &[f64; 9], points: &[(f64, f64)]) -> Result<()> {
    let mut negative: Option<bool> = None;
    for &(x, y) in points {
        let w = matrix[6] * x + matrix[7] * y + matrix[8];
        if w.abs() <= EPSILON {
            return Err(CoreError::InvalidTransform);
        }
        let neg = w < 0.0;
        match negative {
            None => negative = Some(neg),
            Some(first) if first != neg => return Err(CoreError::InvalidTransform),
            Some(_) => {}
        }
    }
    Ok(())
}

/// Apply a forward matrix to one point.
pub(crate) fn project(matrix: &[f64; 9], x: f64, y: f64) -> Result<(f64, f64)> {
    let w = matrix[6] * x + matrix[7] * y + matrix[8];
    if w.abs() <= EPSILON {
        return Err(CoreError::InvalidTransform);
    }
    Ok((
        (matrix[0] * x + matrix[1] * y + matrix[2]) / w,
        (matrix[3] * x + matrix[4] * y + matrix[5]) / w,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64)) {
        assert!(
            (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9,
            "{a:?} != {b:?}"
        );
    }

    /// One handle is a translation, and only a translation: the matrix is the identity with the
    /// offset in the last column, so no scale, rotation or shear can leak in from a single drag.
    #[test]
    fn one_handle_is_exactly_a_translation() {
        let m = handle_transform_matrix(&[(1.0, 1.0)], &[(3.0, 2.0)]).unwrap();
        assert_eq!(m, [1.0, 0.0, 2.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0]);
    }

    /// Two handles give rotation plus UNIFORM scale about them, with no angle convention to get
    /// backwards: `(0,0) -> (0,0)` and `(4,0) -> (0,4)` is the quarter turn `(x, y) -> (-y, x)`.
    ///
    /// The two far points are what pin the direction. A transform rotating the other way would send
    /// `(8,0)` to `(0,-8)` rather than `(0,8)`, which is upstream's `calc_angle`/transposed-matrix
    /// pair being mixed up — the mistake the closed form cannot make.
    #[test]
    fn two_handles_give_a_similarity_with_the_turn_in_the_right_direction() {
        let m =
            handle_transform_matrix(&[(0.0, 0.0), (4.0, 0.0)], &[(0.0, 0.0), (0.0, 4.0)]).unwrap();
        assert_eq!(m, [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        close(project(&m, 8.0, 0.0).unwrap(), (0.0, 8.0));
        close(project(&m, 0.0, 8.0).unwrap(), (-8.0, 0.0));
    }

    /// The class ladder at the bottom: a THIRD handle changes the answer while the first two
    /// correspondences are untouched.
    ///
    /// One assertion about the difference rather than two about the maps. `(0,4) -> (0,8)` with
    /// `(4,0)` held still is a stretch of one axis only, which no similarity can do — so at two
    /// handles the map is the identity and at three it is `(x, y) -> (x, 2y)`. An implementation
    /// that always solved the affine would answer the stretch both times.
    #[test]
    fn a_third_handle_buys_non_uniform_scale_the_second_cannot_express() {
        let src = [(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)];
        let dst = [(0.0, 0.0), (4.0, 0.0), (0.0, 8.0)];

        let three = handle_transform_matrix(&src, &dst).unwrap();
        assert_eq!(three, [1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0]);

        let two = handle_transform_matrix(&src[..2], &dst[..2]).unwrap();
        close(project(&two, 0.0, 4.0).unwrap(), (0.0, 4.0));
        close(project(&three, 0.0, 4.0).unwrap(), (0.0, 8.0));
    }

    /// The class ladder at the top, with the error the fourth handle removes measured exactly.
    ///
    /// A trapezoid is not an affine image of a square. With four handles every correspondence is
    /// met and the projective row is non-zero; with three, the affine map through them sends the
    /// fourth source to **(4, 8)** where its handle asks for **(0, 8)** — four pixels out.
    #[test]
    fn a_fourth_handle_buys_perspective_the_third_cannot_express() {
        let src = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        let dst = [(2.0, 0.0), (6.0, 0.0), (8.0, 8.0), (0.0, 8.0)];

        let four = handle_transform_matrix(&src, &dst).unwrap();
        for (point, want) in src.iter().zip(dst.iter()) {
            close(project(&four, point.0, point.1).unwrap(), *want);
        }
        // The projective row carries the perspective; an affine solve leaves it exactly zero.
        assert!(four[6] != 0.0 || four[7] != 0.0);

        let three = handle_transform_matrix(&src[..3], &dst[..3]).unwrap();
        assert_eq!((three[6], three[7]), (0.0, 0.0));
        close(project(&three, src[3].0, src[3].1).unwrap(), (4.0, 8.0));
    }

    /// The degenerate configurations a user reaches by hand, each refused rather than divided by.
    #[test]
    fn coincident_and_collinear_handles_are_refused() {
        assert!(matches!(
            handle_transform_matrix(&[(1.0, 1.0), (1.0, 1.0)], &[(0.0, 0.0), (5.0, 5.0)]),
            Err(CoreError::InvalidTransform)
        ));
        assert!(matches!(
            handle_transform_matrix(
                &[(0.0, 0.0), (2.0, 0.0), (4.0, 0.0)],
                &[(0.0, 0.0), (2.0, 1.0), (4.0, 5.0)]
            ),
            Err(CoreError::InvalidTransform)
        ));
        // Zero handles and five handles are both outside the class table.
        assert!(matches!(
            handle_transform_matrix(&[], &[]),
            Err(CoreError::InvalidTransform)
        ));
        assert!(matches!(
            handle_transform_matrix(&[(0.0, 0.0); 5], &[(1.0, 1.0); 5]),
            Err(CoreError::InvalidTransform)
        ));
    }

    /// The class table is the count, with nothing outside 1..=4.
    #[test]
    fn the_handle_count_names_the_class() {
        assert_eq!(HandleTransformClass::for_handle_count(0), None);
        assert_eq!(
            HandleTransformClass::for_handle_count(1),
            Some(HandleTransformClass::Translation)
        );
        assert_eq!(
            HandleTransformClass::for_handle_count(2),
            Some(HandleTransformClass::Similarity)
        );
        assert_eq!(
            HandleTransformClass::for_handle_count(3),
            Some(HandleTransformClass::Affine)
        );
        assert_eq!(
            HandleTransformClass::for_handle_count(4),
            Some(HandleTransformClass::Projective)
        );
        assert_eq!(HandleTransformClass::for_handle_count(5), None);
        assert_eq!(MAX_HANDLES, 4);
    }

    /// Upstream's two validity rules, each on its own.
    ///
    /// `|w| <= EPSILON` is a point mapping to infinity; a sign difference is two points straddling
    /// the camera plane. The third rule it has — negating a matrix whose every `w` is negative — is
    /// observationally inert for mapping points, and that is asserted here rather than assumed:
    /// the negated matrix projects a point to the same place.
    #[test]
    fn validity_catches_infinity_and_a_straddled_camera_plane() {
        // h20 = 1, so w = x + 1 and the point x = -1 maps to infinity.
        let infinite = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        assert!(matches!(
            projective_validity(&infinite, &[(-1.0, 0.0)]),
            Err(CoreError::InvalidTransform)
        ));
        // Either side of x = -1 the sign of w differs.
        assert!(matches!(
            projective_validity(&infinite, &[(0.0, 0.0), (-2.0, 0.0)]),
            Err(CoreError::InvalidTransform)
        ));
        // All on one side, either side, is valid.
        assert!(projective_validity(&infinite, &[(0.0, 0.0), (1.0, 0.0)]).is_ok());
        assert!(projective_validity(&infinite, &[(-2.0, 0.0), (-3.0, 0.0)]).is_ok());

        let negated = infinite.map(|c| -c);
        close(
            project(&negated, 2.0, 3.0).unwrap(),
            project(&infinite, 2.0, 3.0).unwrap(),
        );
    }
}
