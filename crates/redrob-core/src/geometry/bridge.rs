// SPDX-License-Identifier: GPL-3.0-or-later
//! The bridge between this product's `VectorPath` and kurbo's `BezPath`.
//!
//! Written here, not ported. Everything else in this module came from Graphite; this file
//! is what makes any of it reachable from the document.
//!
//! Without it the 3,532 ported lines compile, pass their tests, and can never touch a
//! document — which is a real failure mode for a port and not a hypothetical one. Two
//! cycles of geometry landed before this file existed, and the matrix rows they were meant
//! to fill could not move until it did.
//!
//! # Two deliberate asymmetries
//!
//! **f32 in, f64 out.** `PathCommand` stores `f32` because the document format does, and
//! kurbo works in `f64`. Widening is exact; narrowing is not. So a round trip through
//! kurbo is lossy at the edges, and that is stated rather than hidden: a caller that
//! offsets a path and writes it back should expect the coordinates to have been rounded to
//! `f32`, because the document cannot hold anything finer.
//!
//! **Quadratics become cubics.** `PathCommand` has `MoveTo`, `LineTo`, `CubicTo` and
//! `Close`, with no quadratic. kurbo has `QuadTo`, and several ported constructors emit
//! one. Rather than refuse those paths, a quadratic is raised to the cubic that describes
//! exactly the same curve — the standard degree elevation, control points at
//! `p0 + 2/3 (p1 - p0)` and `p2 + 2/3 (p1 - p2)`. Nothing is approximated.

use kurbo::{BezPath, PathEl, Point};

use crate::document::{PathCommand, VectorPath};

/// Convert a document path into a kurbo path, ready for the ported algorithms.
pub fn vector_path_to_bezpath(path: &VectorPath) -> BezPath {
    let mut bez = BezPath::new();
    for command in &path.commands {
        match *command {
            PathCommand::MoveTo { x, y } => bez.move_to(point(x, y)),
            PathCommand::LineTo { x, y } => bez.line_to(point(x, y)),
            PathCommand::CubicTo {
                control1_x,
                control1_y,
                control2_x,
                control2_y,
                x,
                y,
            } => bez.curve_to(
                point(control1_x, control1_y),
                point(control2_x, control2_y),
                point(x, y),
            ),
            PathCommand::Close => bez.close_path(),
        }
    }
    bez
}

/// Convert a kurbo path back into document commands, keeping the source path's paint.
///
/// Infallible, and that is a property of kurbo rather than of optimism. This function first
/// returned `Option` to refuse a path opening with anything but a `MoveTo` — a path whose
/// start point is undefined here, where inventing one at the origin would leave a stray
/// segment in the user's document. Measured on kurbo 0.13: BOTH construction paths panic on
/// such a path, `push` and `from_vec` alike, so a `BezPath` in hand is guaranteed to start
/// with a `MoveTo` or be empty. The branch was unreachable, so it is gone rather than kept
/// as dead defensive code with a test that could not exercise it.
pub fn bezpath_to_vector_path(bez: &BezPath, paint: &VectorPath) -> VectorPath {
    let mut commands = Vec::with_capacity(bez.elements().len());
    let mut current = Point::ZERO;

    for element in bez.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                commands.push(PathCommand::MoveTo {
                    x: p.x as f32,
                    y: p.y as f32,
                });
                current = p;
            }
            PathEl::LineTo(p) => {
                commands.push(PathCommand::LineTo {
                    x: p.x as f32,
                    y: p.y as f32,
                });
                current = p;
            }
            PathEl::QuadTo(control, end) => {
                // Degree elevation, exact rather than approximate: a quadratic and this
                // cubic describe the same curve.
                let c1 = Point::new(
                    current.x + 2.0 / 3.0 * (control.x - current.x),
                    current.y + 2.0 / 3.0 * (control.y - current.y),
                );
                let c2 = Point::new(
                    end.x + 2.0 / 3.0 * (control.x - end.x),
                    end.y + 2.0 / 3.0 * (control.y - end.y),
                );
                commands.push(PathCommand::CubicTo {
                    control1_x: c1.x as f32,
                    control1_y: c1.y as f32,
                    control2_x: c2.x as f32,
                    control2_y: c2.y as f32,
                    x: end.x as f32,
                    y: end.y as f32,
                });
                current = end;
            }
            PathEl::CurveTo(c1, c2, end) => {
                commands.push(PathCommand::CubicTo {
                    control1_x: c1.x as f32,
                    control1_y: c1.y as f32,
                    control2_x: c2.x as f32,
                    control2_y: c2.y as f32,
                    x: end.x as f32,
                    y: end.y as f32,
                });
                current = end;
            }
            PathEl::ClosePath => commands.push(PathCommand::Close),
        }
    }

    VectorPath {
        commands,
        fill: paint.fill,
        stroke: paint.stroke,
        fill_rule: paint.fill_rule,
    }
}

fn point(x: f32, y: f32) -> Point {
    Point::new(x as f64, y as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cubic_path() -> VectorPath {
        VectorPath {
            commands: vec![
                PathCommand::MoveTo { x: 0.0, y: 0.0 },
                PathCommand::LineTo { x: 10.0, y: 0.0 },
                PathCommand::CubicTo {
                    control1_x: 12.0,
                    control1_y: 0.0,
                    control2_x: 14.0,
                    control2_y: 2.0,
                    x: 14.0,
                    y: 4.0,
                },
                PathCommand::Close,
            ],
            ..VectorPath::default()
        }
    }

    #[test]
    fn round_trip_preserves_every_command() {
        let original = cubic_path();
        let bez = vector_path_to_bezpath(&original);
        let back = bezpath_to_vector_path(&bez, &original);
        assert_eq!(back.commands, original.commands);
    }

    #[test]
    fn round_trip_preserves_paint() {
        let mut original = cubic_path();
        original.fill = Some(crate::document::Pixel::rgba(1, 2, 3, 255));
        let bez = vector_path_to_bezpath(&original);
        let back = bezpath_to_vector_path(&bez, &original);
        assert_eq!(back.fill, original.fill);
        assert_eq!(back.fill_rule, original.fill_rule);
    }

    #[test]
    fn a_quadratic_becomes_the_exactly_equivalent_cubic() {
        // Degree elevation is exact, so evaluating both at the same t must agree. The
        // check is the curve, not the control points: getting the 2/3 factors wrong
        // produces plausible-looking handles and a different shape.
        let mut bez = BezPath::new();
        bez.move_to((0.0, 0.0));
        bez.quad_to((10.0, 20.0), (20.0, 0.0));

        let converted = bezpath_to_vector_path(&bez, &VectorPath::default());
        let reconverted = vector_path_to_bezpath(&converted);

        use kurbo::ParamCurve;
        let quad = match bez.elements()[1] {
            PathEl::QuadTo(c, e) => kurbo::QuadBez::new(Point::ZERO, c, e),
            _ => unreachable!(),
        };
        let cubic = match reconverted.elements()[1] {
            PathEl::CurveTo(c1, c2, e) => kurbo::CubicBez::new(Point::ZERO, c1, c2, e),
            _ => panic!("the quadratic should have become a cubic"),
        };
        // 1e-6, not 1e-9. Degree elevation is exact in f64; the loss is `PathCommand`
        // storing f32, which holds about seven decimal digits. A first attempt at 1e-9
        // failed by 5e-8 and was right to -- it measured the f32 boundary this module's
        // header documents, rather than an error in the elevation.
        for step in 0..=10 {
            let t = f64::from(step) / 10.0;
            let a = quad.eval(t);
            let b = cubic.eval(t);
            assert!(
                (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6,
                "at t={t} the quadratic gives {a:?} and the cubic {b:?}"
            );
        }
    }

    #[test]
    fn an_empty_path_round_trips_as_empty() {
        let empty = VectorPath::default();
        let bez = vector_path_to_bezpath(&empty);
        let back = bezpath_to_vector_path(&bez, &empty);
        assert!(back.commands.is_empty());
    }
}
