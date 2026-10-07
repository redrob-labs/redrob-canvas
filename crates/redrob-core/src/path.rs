// SPDX-License-Identifier: GPL-3.0-or-later

//! Stored paths: geometry the document keeps that draws NOTHING on its own (J.4).
//!
//! Re-derived from the reference implementation's Paths dockable (`app/actions/paths-actions.c`
//! and `app/actions/paths-commands.c`, GPL-3.0-or-later), pinned and attributed in
//! `docs/upstream-sources.toml`.
//!
//! # Why this is not the vector layer we already have
//!
//! `NodeContent::Vector` is a LAYER: it has a fill and a stroke, it renders, and it composites with
//! everything above and below it. A stored path is the opposite — it is geometry with no appearance
//! at all. You convert it to a selection, you stroke it with the current brush, you export it, and
//! none of those make it visible by itself.
//!
//! Modelling a path as a vector layer with no fill and no stroke was the obvious shortcut and is
//! wrong: an invisible layer still occupies the layer stack, still takes part in group opacity and
//! blend modes, still gets flattened on export, and the moment someone gives it a stroke to see what
//! they are editing it starts painting into the image. Upstream keeps paths off the drawable
//! hierarchy for exactly these reasons, and that is the part worth copying.

use serde::{Deserialize, Serialize};

use crate::{CoreError, PathCommand, Result};

/// A stored path's identity.
pub type PathId = uuid::Uuid;

/// The largest number of stored paths a document may carry.
///
/// 256, the same order as the channel cap. Not a technical limit — a list nobody can navigate is
/// already useless well before this.
pub const MAX_PATHS: usize = 256;

/// Geometry the document stores and never draws by itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Path {
    pub id: PathId,
    pub name: String,
    pub commands: Vec<PathCommand>,
    /// Whether the path's on-canvas outline is shown.
    ///
    /// This governs the EDITING outline only — the dashed guide an editor draws so you can see what
    /// you are dragging. It is not a layer's visibility, because the path contributes no pixels
    /// either way.
    #[serde(default = "default_visible")]
    pub visible: bool,
}

pub(crate) fn default_visible() -> bool {
    true
}

impl Path {
    pub fn new(name: impl Into<String>, commands: Vec<PathCommand>) -> Self {
        Self {
            id: PathId::new_v4(),
            name: name.into(),
            commands,
            visible: true,
        }
    }
}

/// Traces the outline of a coverage mask as closed polygons, for selection → path.
///
/// # What this does and does not promise
///
/// Upstream fits BÉZIER curves to the boundary (`plug-ins/selection-to-path`, a port of Schneider's
/// curve-fitting algorithm with a dozen tunable parameters, which is why its dialog has an
/// "advanced" mode). This traces the boundary as straight segments instead.
///
/// That difference is worth stating plainly rather than implying parity: a traced rectangle is
/// exact, a traced circle is a polygon with one vertex per boundary step. For converting a
/// rectangular or polygonal selection — most of what the conversion is used for — the result is
/// identical to a fitted one. For a feathered or round selection it is heavier and editing its
/// anchors is unpleasant. Curve fitting is filed as its own backlog item.
///
/// The mask is thresholded at 128: a path has no notion of partial membership, so a feathered
/// selection's gradient has to collapse somewhere, and the half-way point is the only choice that
/// is not a preference.
pub fn trace_mask_outline(mask: &[u8], width: u32, height: u32) -> Vec<PathCommand> {
    let mut commands = Vec::new();
    for loop_points in trace_loops(mask, width, height) {
        // Collapse runs that continue in the same direction: a straight 100-pixel side is two
        // anchors, not a hundred. Without this a rectangular selection converts to a path nobody
        // can edit.
        commands.push(PathCommand::MoveTo {
            x: loop_points[0].0 as f32,
            y: loop_points[0].1 as f32,
        });
        for index in 1..loop_points.len() {
            let previous = loop_points[index - 1];
            let current = loop_points[index];
            let next = loop_points[(index + 1) % loop_points.len()];
            let incoming = (current.0 - previous.0, current.1 - previous.1);
            let outgoing = (next.0 - current.0, next.1 - current.1);
            if incoming != outgoing {
                commands.push(PathCommand::LineTo {
                    x: current.0 as f32,
                    y: current.1 as f32,
                });
            }
        }
        commands.push(PathCommand::Close);
    }
    commands
}

/// Traces a mask's outline and fits cubic Béziers to it (J.4-b).
///
/// The same boundary walk as [`trace_mask_outline`]; only what is done with each loop differs. A
/// rectangle still comes out as four straight sides, because a straight run fits a line with no
/// measurable error — that is the test of whether the fitter behaves, not a special case in it.
pub fn trace_mask_outline_fitted(mask: &[u8], width: u32, height: u32) -> Vec<PathCommand> {
    let mut commands = Vec::new();
    for loop_points in trace_loops(mask, width, height) {
        let points: Vec<(f32, f32)> = loop_points
            .iter()
            .map(|(x, y)| (*x as f32, *y as f32))
            .collect();
        commands.extend(crate::curve_fit::fit_closed_loop(&points));
    }
    commands
}

/// The closed loops of a coverage mask's boundary, each as a list of corner-to-corner edge points.
fn trace_loops(mask: &[u8], width: u32, height: u32) -> Vec<Vec<(i64, i64)>> {
    let width = width as usize;
    let height = height as usize;
    let inside = |x: isize, y: isize| -> bool {
        if x < 0 || y < 0 || x >= width as isize || y >= height as isize {
            return false;
        }
        mask[y as usize * width + x as usize] >= 128
    };

    // Walk the boundary EDGES between pixels, not the pixel centres. A centre-based trace cuts the
    // corner of every step and loses half a pixel all the way round, so the path no longer selects
    // the region it came from -- which is the one thing the round trip has to preserve.
    //
    // Each inside pixel contributes up to four unit edges, one per side whose neighbour is outside.
    // Collected as directed segments (keeping the inside on the left) so they chain into loops.
    //
    // A BTreeMap, NOT a HashMap. The walk below starts at whichever key comes out first, so with a
    // hash map the traced path's starting anchor -- and therefore its whole command list -- changed
    // between runs of the same binary. Caught by an anchor-count assertion passing once and failing
    // the next time on identical input. A path that is not byte-stable cannot be compared, cannot
    // be tested, and makes a saved document differ from itself.
    let mut edges: std::collections::BTreeMap<(i64, i64), (i64, i64)> =
        std::collections::BTreeMap::new();
    for y in 0..height as isize {
        for x in 0..width as isize {
            if !inside(x, y) {
                continue;
            }
            let (x0, y0) = (x as i64, y as i64);
            if !inside(x, y - 1) {
                edges.insert((x0, y0), (x0 + 1, y0));
            }
            if !inside(x + 1, y) {
                edges.insert((x0 + 1, y0), (x0 + 1, y0 + 1));
            }
            if !inside(x, y + 1) {
                edges.insert((x0 + 1, y0 + 1), (x0, y0 + 1));
            }
            if !inside(x - 1, y) {
                edges.insert((x0, y0 + 1), (x0, y0));
            }
        }
    }

    let mut loops = Vec::new();
    while let Some(start) = edges.keys().next().copied() {
        let mut point = start;
        let mut loop_points = vec![point];
        while let Some(next) = edges.remove(&point) {
            if next == start {
                break;
            }
            loop_points.push(next);
            point = next;
        }
        if loop_points.len() < 3 {
            continue;
        }
        // Rotate the loop so it BEGINS at a direction change. The walk starts wherever the edge
        // map's first key happens to be, which is usually the middle of a straight side -- and an
        // anchor there is one the user has to drag off a line it should be part of. Also what makes
        // a traced rectangle four anchors instead of five.
        if let Some(corner) = (0..loop_points.len()).find(|index| {
            let previous = loop_points[(index + loop_points.len() - 1) % loop_points.len()];
            let current = loop_points[*index];
            let next = loop_points[(index + 1) % loop_points.len()];
            (current.0 - previous.0, current.1 - previous.1)
                != (next.0 - current.0, next.1 - current.1)
        }) {
            loop_points.rotate_left(corner);
        }
        loops.push(loop_points);
    }
    loops
}

/// Flattens a path to the polyline points a brush can be dragged along, for stroke-a-path.
///
/// Cubics are subdivided to a fixed number of steps per segment rather than adaptively. Adaptive
/// subdivision is better and is not needed here: the consumer is a brush whose dab spacing already
/// resamples the result, so extra points past the dab spacing cost time and change nothing.
pub fn flatten_to_points(commands: &[PathCommand]) -> Result<Vec<(f32, f32)>> {
    const STEPS: usize = 16;
    let mut points: Vec<(f32, f32)> = Vec::new();
    let mut start: Option<(f32, f32)> = None;
    let mut cursor = (0.0f32, 0.0f32);
    for command in commands {
        match command {
            PathCommand::MoveTo { x, y } => {
                cursor = (*x, *y);
                start = Some(cursor);
                points.push(cursor);
            }
            PathCommand::LineTo { x, y } => {
                cursor = (*x, *y);
                points.push(cursor);
            }
            PathCommand::CubicTo {
                control1_x,
                control1_y,
                control2_x,
                control2_y,
                x,
                y,
            } => {
                let (x0, y0) = cursor;
                for step in 1..=STEPS {
                    let t = step as f32 / STEPS as f32;
                    let inverse = 1.0 - t;
                    let a = inverse * inverse * inverse;
                    let b = 3.0 * inverse * inverse * t;
                    let c = 3.0 * inverse * t * t;
                    let d = t * t * t;
                    points.push((
                        a * x0 + b * control1_x + c * control2_x + d * x,
                        a * y0 + b * control1_y + c * control2_y + d * y,
                    ));
                }
                cursor = (*x, *y);
            }
            PathCommand::Close => {
                // Closing returns to the subpath's own start, not to the origin. Returning to 0,0
                // would drag a stroke across the whole canvas.
                if let Some(first) = start {
                    points.push(first);
                    cursor = first;
                }
            }
        }
    }
    if points.len() < 2 {
        return Err(CoreError::PathTooShortToStroke);
    }
    Ok(points)
}
