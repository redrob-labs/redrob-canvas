// SPDX-License-Identifier: GPL-3.0-or-later
//!
//! Ported from Graphite `node-graph/libraries/vector-types/src/vector/algorithms/util.rs`
//! at commit d10ccbedac06497ee4766a7061d29281ee5df811, under Apache-2.0.
//! Apache section 4(b) notice: this file was changed on 2026-09-30.
//! CHANGED: reformatted to this project's rustfmt settings. Upstream uses hard tabs at
//! max_width 200; this project uses spaces at 100, and rustfmt's file-level `ignore` is
//! nightly-only so a carve-out was not available on stable. To diff against upstream
//! despite it, use `git diff --ignore-all-space` or re-run upstream's rustfmt.toml over a
//! copy before comparing.
//! CHANGED: 3 item(s) raised from `fn`/`pub(crate)` to `pub`. Upstream kept them
//! internal because its only callers were bezpath_algorithms.rs and shapes.rs in the
//! same crate, and those are not part of this tranche -- they reach into Graphite's
//! core-types. Here these functions ARE the value being ported, so they are the
//! crate's surface rather than a private detail.
//! CHANGED: `crate::vector::misc::point_to_dvec2` becomes `super::point_to_dvec2`, and
//! `super::consts` keeps its relative path now that the module sits one level shallower.

use glam::DVec2;
use kurbo::{ParamCurve, ParamCurveDeriv, PathSeg};

pub fn pathseg_tangent(segment: PathSeg, t: f64) -> DVec2 {
    // NOTE: .deriv() method gives inaccurate result when it is 1.
    let t = if t == 1. { 1. - f64::EPSILON } else { t };

    let tangent = match segment {
        PathSeg::Line(line) => line.deriv().eval(t),
        PathSeg::Quad(quad_bez) => quad_bez.deriv().eval(t),
        PathSeg::Cubic(cubic_bez) => cubic_bez.deriv().eval(t),
    };

    DVec2::new(tangent.x, tangent.y)
}

/// Compare points by allowing some maximum absolute difference to account for floating point errors
#[cfg(test)]
pub fn compare_points(p1: kurbo::Point, p2: kurbo::Point) -> bool {
    let (p1, p2) = (super::point_to_dvec2(p1), super::point_to_dvec2(p2));
    p1.abs_diff_eq(p2, super::consts::MAX_ABSOLUTE_DIFFERENCE)
}

/// Compare vectors of points by allowing some maximum absolute difference to account for floating point errors
#[cfg(test)]
pub fn compare_vec_of_points(
    a: Vec<kurbo::Point>,
    b: Vec<kurbo::Point>,
    max_absolute_difference: f64,
) -> bool {
    a.len() == b.len()
        && a.into_iter()
            .zip(b)
            .map(|(p1, p2)| (super::point_to_dvec2(p1), super::point_to_dvec2(p2)))
            .all(|(p1, p2)| p1.abs_diff_eq(p2, max_absolute_difference))
}

/// Compare the two values in a `DVec2` independently with a provided max absolute value difference.
#[cfg(test)]
pub fn dvec2_compare(a: kurbo::Point, b: kurbo::Point, max_abs_diff: f64) -> glam::BVec2 {
    glam::BVec2::new(
        (a.x - b.x).abs() < max_abs_diff,
        (a.y - b.y).abs() < max_abs_diff,
    )
}
