// SPDX-License-Identifier: GPL-3.0-or-later
//! Shape construction exposed to the document as a bounded, serialisable choice.
//!
//! Written here, not ported. `shapes.rs` next door provides ten constructors from Graphite;
//! this file decides which of them the document may ask for, and in what units.
//!
//! # Why a closed enum rather than passing the constructors through
//!
//! A `Command` is serialised into the undo history and the `.rrg` project file, so every
//! shape the document can hold has to survive a round trip through JSON and back — which
//! means a closed set with named fields, not a function pointer. It also means a shape added
//! later is a format change, which is exactly the kind of thing that should be visible.
//!
//! Three of the ten constructors are deliberately absent. `polyline_bezpath` is what a
//! freehand path already is, so exposing it as a shape would give two ways to make the same
//! thing. `spiral_bezpath` and `arc_bezpath` take Graphite's `SpiralType` and `ArcType`,
//! whose graph and UI metadata was stripped when they were ported; they can come later with
//! their own serialisation, and guessing one now would bake in a format decision for a shape
//! nobody has asked for.

use glam::DVec2;
use serde::{Deserialize, Serialize};

use super::shapes;
use crate::document::VectorPath;
use crate::error::{CoreError, Result};

/// Upper bound on a polygon's or star's side count.
///
/// Each side is a path segment, and a path is stored in the document, so an unbounded side
/// count is an unbounded allocation driven by one number in a command. 512 is far past any
/// shape a person draws and far short of anything that hurts.
pub const MAX_SHAPE_SIDES: u64 = 512;

/// A shape the document can be asked to construct.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Shape {
    /// Axis-aligned rectangle through two opposite corners.
    Rectangle { x1: f64, y1: f64, x2: f64, y2: f64 },
    /// Rectangle with a uniform corner radius.
    RoundedRectangle {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        radius: f64,
    },
    /// Ellipse inscribed in the box through two opposite corners.
    Ellipse { x1: f64, y1: f64, x2: f64, y2: f64 },
    /// Regular polygon: `sides` vertices on a circle of `radius`.
    RegularPolygon {
        center_x: f64,
        center_y: f64,
        sides: u64,
        radius: f64,
    },
    /// Star: `sides` outer points alternating with `sides` inner points.
    Star {
        center_x: f64,
        center_y: f64,
        sides: u64,
        radius: f64,
        inner_radius: f64,
    },
    /// Straight segment between two points.
    Line { x1: f64, y1: f64, x2: f64, y2: f64 },
}

impl Shape {
    /// Build the shape as a document path, carrying `paint`'s fill, stroke and fill rule.
    ///
    /// Rejects rather than clamps. A polygon with two sides, a circle of radius zero or a
    /// side count in the millions are all the result of a caller computing something wrong,
    /// and silently producing a degenerate path hides that at the point where it is cheapest
    /// to notice.
    pub fn to_vector_path(self, paint: &VectorPath) -> Result<VectorPath> {
        let bez = match self {
            Shape::Rectangle { x1, y1, x2, y2 } => {
                shapes::rectangle_bezpath(DVec2::new(x1, y1), DVec2::new(x2, y2))
            }
            Shape::RoundedRectangle {
                x1,
                y1,
                x2,
                y2,
                radius,
            } => {
                if !radius.is_finite() || radius < 0.0 {
                    return Err(CoreError::InvalidCornerRadius);
                }
                shapes::rounded_rectangle_bezpath(
                    DVec2::new(x1, y1),
                    DVec2::new(x2, y2),
                    [radius; 4],
                )
            }
            Shape::Ellipse { x1, y1, x2, y2 } => {
                shapes::ellipse_bezpath(DVec2::new(x1, y1), DVec2::new(x2, y2))
            }
            Shape::RegularPolygon {
                center_x,
                center_y,
                sides,
                radius,
            } => {
                check_sides(sides)?;
                check_radius(radius)?;
                shapes::regular_polygon_bezpath(DVec2::new(center_x, center_y), sides, radius)
            }
            Shape::Star {
                center_x,
                center_y,
                sides,
                radius,
                inner_radius,
            } => {
                check_sides(sides)?;
                check_radius(radius)?;
                check_radius(inner_radius)?;
                shapes::star_polygon_bezpath(
                    DVec2::new(center_x, center_y),
                    sides,
                    radius,
                    inner_radius,
                )
            }
            Shape::Line { x1, y1, x2, y2 } => {
                shapes::line_bezpath(DVec2::new(x1, y1), DVec2::new(x2, y2))
            }
        };
        Ok(super::bezpath_to_vector_path(&bez, paint))
    }
}

fn check_sides(sides: u64) -> Result<()> {
    if !(3..=MAX_SHAPE_SIDES).contains(&sides) {
        return Err(CoreError::InvalidShapeSides {
            max: MAX_SHAPE_SIDES,
        });
    }
    Ok(())
}

fn check_radius(radius: f64) -> Result<()> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err(CoreError::InvalidShapeRadius);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::PathCommand;

    fn build(shape: Shape) -> VectorPath {
        shape.to_vector_path(&VectorPath::default()).unwrap()
    }

    #[test]
    fn a_rectangle_is_four_corners_and_a_close() {
        let path = build(Shape::Rectangle {
            x1: 0.0,
            y1: 0.0,
            x2: 10.0,
            y2: 20.0,
        });
        assert_eq!(
            path.commands.len(),
            6,
            "move, three lines, line back, close"
        );
        assert!(matches!(
            path.commands[0],
            PathCommand::MoveTo { x: 0.0, y: 0.0 }
        ));
        assert!(matches!(path.commands.last(), Some(PathCommand::Close)));
    }

    #[test]
    fn a_rectangles_corners_are_where_they_were_asked_for() {
        let path = build(Shape::Rectangle {
            x1: 3.0,
            y1: 4.0,
            x2: 13.0,
            y2: 24.0,
        });
        let xs: Vec<f32> = path
            .commands
            .iter()
            .filter_map(|c| match *c {
                PathCommand::MoveTo { x, .. } | PathCommand::LineTo { x, .. } => Some(x),
                _ => None,
            })
            .collect();
        assert!(xs.contains(&3.0) && xs.contains(&13.0), "got {xs:?}");
    }

    #[test]
    fn an_ellipse_is_built_from_curves_not_lines() {
        let path = build(Shape::Ellipse {
            x1: 0.0,
            y1: 0.0,
            x2: 10.0,
            y2: 10.0,
        });
        let cubics = path
            .commands
            .iter()
            .filter(|c| matches!(c, PathCommand::CubicTo { .. }))
            .count();
        assert_eq!(cubics, 4, "an ellipse is four cubic arcs");
    }

    #[test]
    fn a_polygon_has_one_segment_per_side() {
        for sides in [3_u64, 5, 12] {
            let path = build(Shape::RegularPolygon {
                center_x: 0.0,
                center_y: 0.0,
                sides,
                radius: 5.0,
            });
            let lines = path
                .commands
                .iter()
                .filter(|c| matches!(c, PathCommand::LineTo { .. }))
                .count();
            assert_eq!(lines as u64, sides, "{sides} sides");
        }
    }

    #[test]
    fn a_star_alternates_outer_and_inner_points() {
        let path = build(Shape::Star {
            center_x: 0.0,
            center_y: 0.0,
            sides: 5,
            radius: 10.0,
            inner_radius: 4.0,
        });
        let radii: Vec<f64> = path
            .commands
            .iter()
            .filter_map(|c| match *c {
                PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => {
                    Some(((x as f64).powi(2) + (y as f64).powi(2)).sqrt())
                }
                _ => None,
            })
            .collect();
        let outer = radii.iter().filter(|r| (**r - 10.0).abs() < 0.01).count();
        let inner = radii.iter().filter(|r| (**r - 4.0).abs() < 0.01).count();
        // SIX outer readings, not five. `polyline_bezpath(_, closed: true)` returns to the
        // start point with an explicit `LineTo` and then closes, so the first outer vertex
        // appears twice: once as the `MoveTo` and once as that closing segment. The
        // rectangle does the same -- four corners come out as six commands. A first
        // expectation of five was wrong about the constructor, not about the star.
        assert_eq!(
            outer, 6,
            "five outer vertices, the first counted twice: {radii:?}"
        );
        assert_eq!(inner, 5, "five inner vertices: {radii:?}");
    }

    #[test]
    fn a_line_is_a_move_and_a_line() {
        let path = build(Shape::Line {
            x1: 1.0,
            y1: 2.0,
            x2: 3.0,
            y2: 4.0,
        });
        assert_eq!(path.commands.len(), 2);
    }

    #[test]
    fn degenerate_shapes_are_refused_rather_than_clamped() {
        let paint = VectorPath::default();
        let bad = [
            Shape::RegularPolygon {
                center_x: 0.0,
                center_y: 0.0,
                sides: 2,
                radius: 5.0,
            },
            Shape::RegularPolygon {
                center_x: 0.0,
                center_y: 0.0,
                sides: MAX_SHAPE_SIDES + 1,
                radius: 5.0,
            },
            Shape::RegularPolygon {
                center_x: 0.0,
                center_y: 0.0,
                sides: 5,
                radius: 0.0,
            },
            Shape::RegularPolygon {
                center_x: 0.0,
                center_y: 0.0,
                sides: 5,
                radius: f64::NAN,
            },
            Shape::Star {
                center_x: 0.0,
                center_y: 0.0,
                sides: 5,
                radius: 10.0,
                inner_radius: -1.0,
            },
            Shape::RoundedRectangle {
                x1: 0.0,
                y1: 0.0,
                x2: 10.0,
                y2: 10.0,
                radius: f64::INFINITY,
            },
        ];
        for shape in bad {
            assert!(
                shape.to_vector_path(&paint).is_err(),
                "should have been refused: {shape:?}"
            );
        }
    }

    #[test]
    fn the_shape_choice_survives_a_json_round_trip() {
        // A Command is serialised into the undo history and the project file, so every
        // shape has to come back as itself.
        let shapes = [
            Shape::Rectangle {
                x1: 1.0,
                y1: 2.0,
                x2: 3.0,
                y2: 4.0,
            },
            Shape::RoundedRectangle {
                x1: 1.0,
                y1: 2.0,
                x2: 3.0,
                y2: 4.0,
                radius: 0.5,
            },
            Shape::Ellipse {
                x1: 0.0,
                y1: 0.0,
                x2: 8.0,
                y2: 6.0,
            },
            Shape::RegularPolygon {
                center_x: 1.0,
                center_y: 1.0,
                sides: 6,
                radius: 4.0,
            },
            Shape::Star {
                center_x: 1.0,
                center_y: 1.0,
                sides: 5,
                radius: 4.0,
                inner_radius: 2.0,
            },
            Shape::Line {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 1.0,
            },
        ];
        for shape in shapes {
            let json = serde_json::to_string(&shape).unwrap();
            let back: Shape = serde_json::from_str(&json).unwrap();
            assert_eq!(back, shape, "round trip via {json}");
        }
    }
}
