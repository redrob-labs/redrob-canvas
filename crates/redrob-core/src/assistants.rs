//! Drawing assistants (Krita's assistants): guides that snap each brush point onto a construction
//! line before it is painted, so freehand strokes follow a vanishing point, a ruler, or an ellipse.
//! Re-derived from Krita's assistant tools — the maths is our own; only the behaviour is shared.

use serde::{Deserialize, Serialize};

/// A single drawing assistant. Each variant snaps a stroke point onto its guide.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrushAssistant {
    /// Vanishing point: every painted point is pulled onto the ray from the vanishing point through
    /// the stroke's first point, so the whole stroke stays radial to `(x, y)`.
    VanishingPoint { x: f32, y: f32 },
    /// Parallel ruler: points snap onto the infinite line through `(ax, ay)`-`(bx, by)`.
    ParallelRuler { ax: f32, ay: f32, bx: f32, by: f32 },
    /// Ellipse guide: points snap onto the axis-aligned ellipse centred at `(cx, cy)` with radii
    /// `(rx, ry)` (rotation is left to a later pass; Krita's default ellipse is axis-aligned).
    Ellipse { cx: f32, cy: f32, rx: f32, ry: f32 },
}

impl BrushAssistant {
    /// True when every number is finite and the guide is non-degenerate.
    pub fn is_valid(&self) -> bool {
        match *self {
            BrushAssistant::VanishingPoint { x, y } => x.is_finite() && y.is_finite(),
            BrushAssistant::ParallelRuler { ax, ay, bx, by } => {
                ax.is_finite()
                    && ay.is_finite()
                    && bx.is_finite()
                    && by.is_finite()
                    && (f64::from(bx - ax).hypot(f64::from(by - ay)) > 1e-6)
            }
            BrushAssistant::Ellipse { cx, cy, rx, ry } => {
                cx.is_finite()
                    && cy.is_finite()
                    && rx.is_finite()
                    && ry.is_finite()
                    && rx > 0.0
                    && ry > 0.0
            }
        }
    }

    /// Snap point `(px, py)` onto this guide. `anchor` is the stroke's first point, used by the
    /// vanishing-point assistant to pick which ray to follow.
    pub fn snap(&self, px: f32, py: f32, anchor: (f32, f32)) -> (f32, f32) {
        match *self {
            BrushAssistant::VanishingPoint { x, y } => {
                // Direction of the ray: vanishing point -> stroke start.
                let dx = f64::from(anchor.0 - x);
                let dy = f64::from(anchor.1 - y);
                let len2 = dx * dx + dy * dy;
                if len2 < 1e-9 {
                    return (px, py);
                }
                // Project (px,py) onto that ray (through the vanishing point).
                let ox = f64::from(px - x);
                let oy = f64::from(py - y);
                let t = (ox * dx + oy * dy) / len2;
                ((x as f64 + dx * t) as f32, (y as f64 + dy * t) as f32)
            }
            BrushAssistant::ParallelRuler { ax, ay, bx, by } => {
                let dx = f64::from(bx - ax);
                let dy = f64::from(by - ay);
                let len2 = dx * dx + dy * dy;
                if len2 < 1e-9 {
                    return (px, py);
                }
                let ox = f64::from(px - ax);
                let oy = f64::from(py - ay);
                let t = (ox * dx + oy * dy) / len2;
                ((ax as f64 + dx * t) as f32, (ay as f64 + dy * t) as f32)
            }
            BrushAssistant::Ellipse { cx, cy, rx, ry } => {
                // Nearest point on the ellipse along the ray from the centre (good enough for a
                // freehand guide; a true closest-point solve is overkill here).
                let dx = f64::from(px - cx);
                let dy = f64::from(py - cy);
                if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
                    return (cx + rx, cy);
                }
                let angle = dy.atan2(dx);
                (
                    (cx as f64 + f64::from(rx) * angle.cos()) as f32,
                    (cy as f64 + f64::from(ry) * angle.sin()) as f32,
                )
            }
        }
    }
}
