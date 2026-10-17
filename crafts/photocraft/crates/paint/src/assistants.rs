//! Painting assistants: rulers, vanishing points and perspective grids that stroke input snaps
//! to. Pure geometry on document coordinates; the engine keeps a document's assistants and the
//! canvas feeds pointer positions through [`Assistant::snap`].
//!
//! Every assistant turns a stroke into a straight line: the Ruler onto its own line, the Parallel
//! Ruler onto the line through the stroke's start in the ruler's direction, the Vanishing Point
//! onto the line through the start and the point, and the Perspective Grid onto whichever of its
//! two receding directions the stroke follows more closely (its vanishing points are where the
//! quadrilateral's opposite sides meet; parallel sides give a direction instead).

use serde::{Deserialize, Serialize};

pub const MAX_COORDINATE: f64 = 1_000_000.0;
/// Most assistants one document holds.
pub const MAX_ASSISTANTS: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Assistant {
    /// Strokes land on the infinite line through `a` and `b`.
    Ruler { a: [f64; 2], b: [f64; 2] },
    /// Strokes stay parallel to `a → b`, wherever they start.
    ParallelRuler { a: [f64; 2], b: [f64; 2] },
    /// Strokes head towards (or away from) `p`.
    VanishingPoint { p: [f64; 2] },
    /// A plane in perspective: the four corners of a quadrilateral, clockwise from the top left;
    /// strokes follow its two receding directions.
    PerspectiveGrid { corners: [[f64; 2]; 4] },
}

/// How far a point may be snapped (in document pixels) before the assistant is left alone.
pub const MAX_SNAP_DISTANCE: f64 = 1.0e6;

/// The result of snapping `p` with an assistant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapped {
    /// Where the point lands.
    pub point: [f64; 2],
    /// How far it moved.
    pub distance: f64,
}

/// Intersection of lines `a → b` and `c → d`, or `None` when they are (nearly) parallel.
pub fn intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<[f64; 2]> {
    let (r, s) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let den = r[0] * s[1] - r[1] * s[0];
    let len = (r[0].hypot(r[1]) * s[0].hypot(s[1])).max(1e-12);
    if (den / len).abs() < 1e-6 {
        return None;
    }
    let t = ((c[0] - a[0]) * s[1] - (c[1] - a[1]) * s[0]) / den;
    let p = [a[0] + t * r[0], a[1] + t * r[1]];
    p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE * 1e3).then_some(p)
}

/// `p` projected onto the infinite line through `origin` in direction `dir` (unit length not
/// required). A zero direction returns `p` unchanged.
fn project(origin: [f64; 2], dir: [f64; 2], p: [f64; 2]) -> [f64; 2] {
    let len2 = dir[0] * dir[0] + dir[1] * dir[1];
    if len2 <= 1e-18 {
        return p;
    }
    let t = ((p[0] - origin[0]) * dir[0] + (p[1] - origin[1]) * dir[1]) / len2;
    [origin[0] + t * dir[0], origin[1] + t * dir[1]]
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn finite(p: [f64; 2]) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
}

impl Assistant {
    pub const KINDS: [&'static str; 4] = ["ruler", "parallelRuler", "vanishingPoint", "perspectiveGrid"];

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ruler { .. } => "ruler",
            Self::ParallelRuler { .. } => "parallelRuler",
            Self::VanishingPoint { .. } => "vanishingPoint",
            Self::PerspectiveGrid { .. } => "perspectiveGrid",
        }
    }

    /// How many handles a `kind` has (what [`Self::from_points`] needs).
    pub fn handle_count(kind: &str) -> Option<usize> {
        Some(match kind {
            "ruler" | "parallelRuler" => 2,
            "vanishingPoint" => 1,
            "perspectiveGrid" => 4,
            _ => return None,
        })
    }

    /// Build an assistant of `kind` from its handles (see [`Self::handle_count`]). Coincident
    /// ruler ends or a degenerate grid are refused.
    pub fn from_points(kind: &str, points: &[[f64; 2]]) -> Option<Self> {
        if points.len() != Self::handle_count(kind)? || !points.iter().all(|p| finite(*p)) {
            return None;
        }
        let a = match kind {
            "ruler" => Self::Ruler { a: points[0], b: points[1] },
            "parallelRuler" => Self::ParallelRuler { a: points[0], b: points[1] },
            "vanishingPoint" => Self::VanishingPoint { p: points[0] },
            "perspectiveGrid" => Self::PerspectiveGrid { corners: [points[0], points[1], points[2], points[3]] },
            _ => return None,
        };
        a.is_valid().then_some(a)
    }

    /// Rulers need two distinct ends; a grid needs four distinct corners.
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Ruler { a, b } | Self::ParallelRuler { a, b } => finite(*a) && finite(*b) && dist(*a, *b) >= 1e-6,
            Self::VanishingPoint { p } => finite(*p),
            Self::PerspectiveGrid { corners } => corners.iter().all(|c| finite(*c)) && (0..4).all(|i| (i + 1..4).all(|j| dist(corners[i], corners[j]) >= 1e-6)),
        }
    }

    /// The draggable points, in [`Self::from_points`] order.
    pub fn handles(&self) -> Vec<[f64; 2]> {
        match self {
            Self::Ruler { a, b } | Self::ParallelRuler { a, b } => vec![*a, *b],
            Self::VanishingPoint { p } => vec![*p],
            Self::PerspectiveGrid { corners } => corners.to_vec(),
        }
    }

    /// Move handle `i` to `to`; false when there is no such handle or the result is degenerate.
    pub fn set_handle(&mut self, i: usize, to: [f64; 2]) -> bool {
        if !finite(to) {
            return false;
        }
        let mut next = self.clone();
        match (&mut next, i) {
            (Self::Ruler { a, .. } | Self::ParallelRuler { a, .. }, 0) => *a = to,
            (Self::Ruler { b, .. } | Self::ParallelRuler { b, .. }, 1) => *b = to,
            (Self::VanishingPoint { p }, 0) => *p = to,
            (Self::PerspectiveGrid { corners }, i) if i < 4 => corners[i] = to,
            _ => return false,
        }
        if !next.is_valid() {
            return false;
        }
        *self = next;
        true
    }

    /// Move the whole assistant by `d`.
    pub fn translate(&mut self, d: [f64; 2]) -> bool {
        let moved: Vec<[f64; 2]> = self.handles().into_iter().map(|p| [p[0] + d[0], p[1] + d[1]]).collect();
        match Self::from_points(self.kind(), &moved) {
            Some(a) => {
                *self = a;
                true
            }
            None => false,
        }
    }

    /// The centre of the handles: where a move handle sits.
    pub fn center(&self) -> [f64; 2] {
        let h = self.handles();
        let n = h.len() as f64;
        [h.iter().map(|p| p[0]).sum::<f64>() / n, h.iter().map(|p| p[1]).sum::<f64>() / n]
    }

    /// A perspective grid's vanishing points: `Ok(point)` where opposite sides meet, `Err(dir)`
    /// (a direction) where they are parallel. First the horizontal pair (top and bottom sides),
    /// then the vertical pair (left and right sides).
    pub fn vanishing(&self) -> Option<[Result<[f64; 2], [f64; 2]>; 2]> {
        let Self::PerspectiveGrid { corners: [tl, tr, br, bl] } = self else { return None };
        let horizontal = intersect(*tl, *tr, *bl, *br).ok_or([tr[0] - tl[0], tr[1] - tl[1]]);
        let vertical = intersect(*tl, *bl, *tr, *br).ok_or([bl[0] - tl[0], bl[1] - tl[1]]);
        Some([horizontal, vertical])
    }

    /// The lines a stroke from `start` may follow: `(origin, direction)` pairs.
    fn lines(&self, start: [f64; 2]) -> Vec<([f64; 2], [f64; 2])> {
        match self {
            Self::Ruler { a, b } => vec![(*a, [b[0] - a[0], b[1] - a[1]])],
            Self::ParallelRuler { a, b } => vec![(start, [b[0] - a[0], b[1] - a[1]])],
            Self::VanishingPoint { p } => {
                let d = [p[0] - start[0], p[1] - start[1]];
                if d[0].hypot(d[1]) < 1e-9 { Vec::new() } else { vec![(start, d)] }
            }
            Self::PerspectiveGrid { .. } => self
                .vanishing()
                .into_iter()
                .flatten()
                .filter_map(|v| match v {
                    Ok(vp) => {
                        let d = [vp[0] - start[0], vp[1] - start[1]];
                        (d[0].hypot(d[1]) >= 1e-9).then_some((start, d))
                    }
                    Err(dir) => Some((start, dir)),
                })
                .collect(),
        }
    }

    /// Snap `p`, a point of a stroke that began at `start`. `None` when the assistant has no
    /// line for this stroke (a vanishing point the stroke started on).
    pub fn snap(&self, start: [f64; 2], p: [f64; 2]) -> Option<Snapped> {
        self.lines(start)
            .into_iter()
            .map(|(o, d)| project(o, d, p))
            .filter(|q| q.iter().all(|v| v.is_finite()))
            .map(|q| Snapped { point: q, distance: dist(q, p) })
            .filter(|s| s.distance <= MAX_SNAP_DISTANCE)
            .min_by(|x, y| x.distance.total_cmp(&y.distance))
    }

    /// How well this assistant fits a stroke from `start` heading to `p`: the snap distance,
    /// plus for a Ruler how far the start sits from its line (a stroke begun far from a ruler is
    /// not the ruler's). Lower is better; `None` when it cannot snap this stroke.
    pub fn fit(&self, start: [f64; 2], p: [f64; 2]) -> Option<f64> {
        let s = self.snap(start, p)?;
        let start_penalty = match self {
            Self::Ruler { a, b } => dist(project(*a, [b[0] - a[0], b[1] - a[1]], start), start),
            _ => 0.0,
        };
        Some(s.distance + start_penalty)
    }

    /// The segments drawn for the assistant on a `size` document (besides its handles): rulers
    /// across the canvas, a vanishing point's rays towards the corners, a grid's edges and inner
    /// lines.
    pub fn guides(&self, size: [f64; 2]) -> Vec<[[f64; 2]; 2]> {
        let reach = size[0].hypot(size[1]).max(1.0);
        let along = |o: [f64; 2], d: [f64; 2]| {
            let l = d[0].hypot(d[1]).max(1e-9);
            let u = [d[0] / l, d[1] / l];
            [[o[0] - reach * u[0], o[1] - reach * u[1]], [o[0] + reach * u[0], o[1] + reach * u[1]]]
        };
        match self {
            Self::Ruler { a, b } => vec![along(*a, [b[0] - a[0], b[1] - a[1]])],
            Self::ParallelRuler { a, b } => {
                // The ruler itself, and a faint family of parallels across the canvas.
                let d = [b[0] - a[0], b[1] - a[1]];
                let l = d[0].hypot(d[1]).max(1e-9);
                let n = [-d[1] / l, d[0] / l];
                let step = (size[0].min(size[1]) / 6.0).max(8.0);
                (-6..=6).map(|k| along([a[0] + n[0] * step * f64::from(k), a[1] + n[1] * step * f64::from(k)], d)).collect()
            }
            Self::VanishingPoint { p } => [
                [0.0, 0.0],
                [size[0], 0.0],
                [size[0], size[1]],
                [0.0, size[1]],
                [size[0] / 2.0, 0.0],
                [size[0] / 2.0, size[1]],
                [0.0, size[1] / 2.0],
                [size[0], size[1] / 2.0],
            ]
            .into_iter()
            .filter(|c| dist(*c, *p) >= 1e-6)
            .map(|c| [*p, c])
            .collect(),
            Self::PerspectiveGrid { corners } => {
                let [tl, tr, br, bl] = *corners;
                let mut out = vec![[tl, tr], [tr, br], [br, bl], [bl, tl]];
                // Inner lines: the sides divided in perspective where the vanishing points exist,
                // evenly where they do not.
                const DIVISIONS: usize = 4;
                let lerp = |a: [f64; 2], b: [f64; 2], t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                for i in 1..DIVISIONS {
                    let t = i as f64 / DIVISIONS as f64;
                    out.push([lerp(tl, tr, t), lerp(bl, br, t)]);
                    out.push([lerp(tl, bl, t), lerp(tr, br, t)]);
                }
                out
            }
        }
    }
}

/// Pick the assistant among `assistants` that best fits a stroke from `start` to `p`.
pub fn choose(assistants: &[Assistant], start: [f64; 2], p: [f64; 2]) -> Option<usize> {
    assistants.iter().enumerate().filter_map(|(i, a)| a.fit(start, p).map(|f| (i, f))).min_by(|x, y| x.1.total_cmp(&y.1)).map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        dist(a, b) < 1e-6
    }

    #[test]
    fn rulers_project_onto_their_lines() {
        let r = Assistant::from_points("ruler", &[[0.0, 10.0], [100.0, 10.0]]).unwrap();
        assert!(close(r.snap([5.0, 5.0], [40.0, 30.0]).unwrap().point, [40.0, 10.0]));
        let p = Assistant::from_points("parallelRuler", &[[0.0, 0.0], [10.0, 10.0]]).unwrap();
        // Parallel to the diagonal, through the stroke's start.
        assert!(close(p.snap([50.0, 0.0], [60.0, 0.0]).unwrap().point, [55.0, 5.0]));
        assert!(Assistant::from_points("ruler", &[[1.0, 1.0], [1.0, 1.0]]).is_none());
        assert!(Assistant::from_points("ruler", &[[1.0, 1.0]]).is_none());
        assert!(Assistant::from_points("nope", &[[1.0, 1.0]]).is_none());
    }

    #[test]
    fn vanishing_point_and_grid_snap_towards_their_points() {
        let vp = Assistant::from_points("vanishingPoint", &[[100.0, 0.0]]).unwrap();
        let s = vp.snap([0.0, 100.0], [40.0, 70.0]).unwrap();
        // On the line from (0,100) to (100,0): x + y = 100.
        assert!((s.point[0] + s.point[1] - 100.0).abs() < 1e-6);
        assert!(vp.snap([100.0, 0.0], [40.0, 70.0]).is_none(), "a stroke on the point has no direction");
        // A trapezoid: horizontal sides converge above, vertical sides are parallel.
        let g = Assistant::from_points("perspectiveGrid", &[[40.0, 0.0], [60.0, 0.0], [100.0, 100.0], [0.0, 100.0]]).unwrap();
        let [h, v] = g.vanishing().unwrap();
        assert!(h.is_err(), "top and bottom are parallel: {h:?}");
        let v = v.unwrap();
        assert!(close(v, [50.0, -25.0]), "{v:?}");
        // A near-horizontal stroke follows the horizontal direction; a near-vertical one heads to the point.
        assert!(close(g.snap([20.0, 50.0], [60.0, 52.0]).unwrap().point, [60.0, 50.0]));
        let up = g.snap([0.0, 100.0], [20.0, 20.0]).unwrap().point;
        let d = [v[0] - 0.0, v[1] - 100.0];
        let cross = (up[0] - 0.0) * d[1] - (up[1] - 100.0) * d[0];
        assert!(cross.abs() < 1e-6, "snapped point lies on the line to the vanishing point: {up:?}");
        assert_eq!(g.guides([100.0, 100.0]).len(), 4 + 6);
    }

    #[test]
    fn choice_handles_and_moves() {
        let mut list = vec![
            Assistant::from_points("ruler", &[[0.0, 0.0], [100.0, 0.0]]).unwrap(),
            Assistant::from_points("ruler", &[[0.0, 80.0], [100.0, 80.0]]).unwrap(),
            Assistant::from_points("vanishingPoint", &[[500.0, 500.0]]).unwrap(),
        ];
        assert_eq!(choose(&list, [10.0, 78.0], [40.0, 79.0]), Some(1));
        assert_eq!(choose(&list, [10.0, 2.0], [40.0, 1.0]), Some(0));
        assert_eq!(choose(&list, [10.0, 40.0], [20.0, 50.0]), Some(2));
        assert!(list[0].set_handle(1, [100.0, 50.0]));
        assert!(!list[0].set_handle(1, [0.0, 0.0]), "coincident ends are refused");
        assert!(!list[0].set_handle(2, [0.0, 0.0]));
        assert!(list[2].translate([1.0, 2.0]));
        assert_eq!(list[2].handles(), vec![[501.0, 502.0]]);
        assert_eq!(list[0].center(), [50.0, 25.0]);
        let json = serde_json::to_string(&list[0]).unwrap();
        assert!(json.contains("\"kind\":\"ruler\""));
        assert_eq!(serde_json::from_str::<Assistant>(&json).unwrap(), list[0]);
    }
}
