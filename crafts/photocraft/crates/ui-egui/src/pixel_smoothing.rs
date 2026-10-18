//! Pixel smoothing for pixel art: a one-pixel line stays one pixel wide with no L-shaped corners.
//! The stroke's points are snapped to pixel centres, duplicates dropped, and wherever three
//! consecutive pixels turn a right angle (a step of the staircase) the corner pixel is removed so
//! the line runs diagonally through it. Applied to the Pencil's (and an aliased 1 px brush's)
//! committed stroke when the option is on.

use crate::{PhotocraftApp, state::Tool};

/// Does `tool`'s stroke take pixel smoothing (a hard, one-pixel line)?
pub fn applies(app: &PhotocraftApp, tool: Tool) -> bool {
    app.ui.tool_options.pixel_smoothing
        && (tool == Tool::Pencil || (tool == Tool::Brush && app.session.tools.brush.size <= 1.0 && app.session.tools.brush.hardness >= 1.0))
}

/// `points` (`[x, y, pressure]`) snapped to pixel centres with corner pixels removed.
pub fn smooth(points: &[[f64; 3]]) -> Vec<[f64; 3]> {
    // Snap and drop repeats.
    let mut px: Vec<[f64; 3]> = Vec::with_capacity(points.len());
    for p in points {
        let q = [p[0].floor() + 0.5, p[1].floor() + 0.5, p[2]];
        if px.last().is_none_or(|l| l[0] != q[0] || l[1] != q[1]) {
            px.push(q);
        }
    }
    if px.len() < 3 {
        return px;
    }
    // Fill in the pixels a longer jump crosses (a fast pointer), so corners are visible to the
    // rule below: a 4-connected walk, horizontal first.
    let mut dense: Vec<[f64; 3]> = vec![px[0]];
    for w in px.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (mut x, mut y) = (a[0], a[1]);
        let (dx, dy) = ((b[0] - a[0]).abs() as i64, (b[1] - a[1]).abs() as i64);
        let (sx, sy) = ((b[0] - a[0]).signum(), (b[1] - a[1]).signum());
        if dx + dy > 4096 {
            dense.push(b);
            continue;
        }
        // Bresenham-like: step along the longer axis, diagonal steps allowed.
        let steps = dx.max(dy);
        let mut err = 0i64;
        for _ in 0..steps {
            if dx >= dy {
                x += sx;
                err += dy;
                if err * 2 >= dx {
                    y += sy;
                    err -= dx;
                }
            } else {
                y += sy;
                err += dx;
                if err * 2 >= dy {
                    x += sx;
                    err -= dy;
                }
            }
            dense.push([x, y, b[2]]);
        }
    }
    // Remove the corner of every L: three pixels where the first two differ in one axis only and
    // the last two in the other.
    let mut out: Vec<[f64; 3]> = Vec::with_capacity(dense.len());
    for p in dense {
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let ab_h = a[1] == b[1] && (a[0] - b[0]).abs() == 1.0;
            let ab_v = a[0] == b[0] && (a[1] - b[1]).abs() == 1.0;
            let bp_h = b[1] == p[1] && (b[0] - p[0]).abs() == 1.0;
            let bp_v = b[0] == p[0] && (b[1] - p[1]).abs() == 1.0;
            if (ab_h && bp_v) || (ab_v && bp_h) {
                out.pop();
            }
        }
        if out.last().is_none_or(|l| l[0] != p[0] || l[1] != p[1]) {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xy(v: &[[f64; 3]]) -> Vec<(i32, i32)> {
        v.iter().map(|p| (p[0].floor() as i32, p[1].floor() as i32)).collect()
    }

    #[test]
    fn staircase_corners_become_diagonals() {
        // A staircase: right, down, right, down.
        let pts = [[0.2, 0.3, 1.0], [1.5, 0.5, 1.0], [1.5, 1.5, 1.0], [2.5, 1.5, 1.0], [2.5, 2.5, 1.0]];
        let s = smooth(&pts);
        assert_eq!(xy(&s), vec![(0, 0), (1, 1), (2, 2)]);
        // A straight run is untouched; repeats collapse.
        let line = [[0.5, 0.5, 1.0], [1.5, 0.5, 1.0], [1.5, 0.5, 0.9], [2.5, 0.5, 1.0]];
        assert_eq!(xy(&smooth(&line)), vec![(0, 0), (1, 0), (2, 0)]);
        // A fast jump is filled in, and its corner removed.
        let jump = [[0.5, 0.5, 1.0], [3.5, 0.5, 1.0], [3.5, 1.5, 1.0]];
        assert_eq!(xy(&smooth(&jump)), vec![(0, 0), (1, 0), (2, 0), (3, 1)]);
        assert!(smooth(&[]).is_empty());
    }
}
