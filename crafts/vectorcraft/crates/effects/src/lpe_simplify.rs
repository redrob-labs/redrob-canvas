//! Simplify as a live effect (`path.simplifyLive`): the Pencil's interactive simplify. The path
//! keeps every point the hand put down; the effect fits a smoother path over it to a threshold
//! that can be changed afterwards in the Appearance panel, so a freehand stroke is never smoothed
//! away for good.

use redrob_vector_geom::PathData;
use redrob_vector_pathops::{SimplifyOptions, simplify_with};
use serde_json::Value;

use crate::util::*;

pub const SIMPLIFY: &str = "path.simplifyLive";

/// The simplified path: `threshold` points of tolerance, corners kept where the path turns by
/// more than `cornerAngle` degrees, straight lines only when `straightLines`.
pub fn simplify(path: &PathData, p: &Value) -> PathData {
    let tolerance = num(p, "threshold", 1.5).clamp(0.05, 100.0);
    let corner_angle_deg = num(p, "cornerAngle", 60.0).clamp(0.0, 180.0);
    let straight_lines = flag(p, "straightLines", false);
    simplify_with(path, &SimplifyOptions { tolerance, corner_angle_deg, straight_lines })
}
