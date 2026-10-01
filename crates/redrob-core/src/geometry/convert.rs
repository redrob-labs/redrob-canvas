// SPDX-License-Identifier: GPL-3.0-or-later
//! Conversions between kurbo's `Point` and glam's `DVec2`.
//!
//! Ported from Graphite (`node-graph/libraries/vector-types/src/vector/misc.rs`) at
//! commit d10ccbedac06497ee4766a7061d29281ee5df811, under Apache-2.0. CHANGED: extracted
//! these two functions from a 769-line module whose remainder this product does not use,
//! and documented here rather than left as a bare pair.
//!
//! The two libraries exist side by side on purpose rather than by accident. kurbo owns
//! the curve maths -- it is what the ported algorithms are written against -- and glam
//! owns the vector algebra. Neither converts to the other, so the boundary is these two
//! functions and they are deliberately the only place it lives.

use glam::DVec2;
use kurbo::Point;

pub fn point_to_dvec2(point: Point) -> DVec2 {
    DVec2 {
        x: point.x,
        y: point.y,
    }
}

pub fn dvec2_to_point(value: DVec2) -> Point {
    Point {
        x: value.x,
        y: value.y,
    }
}
