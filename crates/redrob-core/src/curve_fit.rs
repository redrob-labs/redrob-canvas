// SPDX-License-Identifier: GPL-3.0-or-later

//! Fitting cubic Béziers to a traced outline (J.4-b).
//!
//! Re-derived from the reference implementation's `plug-ins/selection-to-path/fit.c`
//! (GPL-3.0-or-later), pinned and attributed in `docs/upstream-sources.toml`, which is itself a
//! port of Schneider's curve-fitting algorithm. Its four governing parameters are reused with
//! upstream's own default values, named below, because they are tuned against the one input that
//! matters here — a pixel staircase, not a smooth sampled curve.
//!
//! # Why J.4's straight trace was not enough
//!
//! J.4 traced boundaries as straight segments. A rectangular or polygonal selection converts
//! exactly that way, and that covers most uses. A round or feathered one came back as a polygon
//! with a vertex per boundary step: a 60-pixel circle became roughly 200 anchors, which is correct
//! in coverage and useless to edit.

use crate::PathCommand;

/// How many points each side of a candidate are examined when looking for a corner.
///
/// 4, upstream's `corner_surround`. The window exists because a pixel staircase has a direction
/// change at EVERY step: judged one step at a time, every point on a circle is a 90° corner. Four
/// steps of context is what distinguishes a real corner from the staircase.
const CORNER_SURROUND: usize = 4;

/// An angle at or below this is a corner no matter what its neighbours look like.
///
/// 60 degrees, upstream's `corner_always_threshold`.
const CORNER_ALWAYS_DEGREES: f32 = 60.0;

/// An angle at or below this is a corner if it is the sharpest within its window.
///
/// 100 degrees, upstream's `corner_threshold`.
const CORNER_DEGREES: f32 = 100.0;

/// How far, in pixels, a fitted spline may stray from the traced outline before it is subdivided.
///
/// 0.4, upstream's `error_threshold`. Below half a pixel, so the fit cannot move the boundary into
/// a neighbouring pixel — which is what would make the path select a different region than it was
/// traced from.
///
/// This threshold is only achievable because the outline is SMOOTHED first. Measured: applied to the
/// raw boundary it can never be met, because a pixel staircase deviates from the smooth shape it
/// approximates by up to half a pixel BY CONSTRUCTION. Every fit then failed, subdivided to
/// two-point runs, and a 50-pixel circle came out as 106 commands against the straight trace's 117 —
/// the fitting bought nothing. That measurement is what sent me back to upstream's filter.
const ERROR_THRESHOLD: f32 = 0.4;

/// How many smoothing passes are run over the outline before fitting.
///
/// 4, upstream's `filter_iteration_count`.
const FILTER_ITERATIONS: usize = 4;

/// How far each point is moved toward its neighbours' mean per pass.
///
/// 0.33, upstream's `filter_percent`. Enough to take the staircase off a boundary in four passes,
/// little enough that four passes do not shrink the shape measurably.
const FILTER_PERCENT: f32 = 0.33;

/// How many neighbours each side contribute to a point's smoothed position.
///
/// 2, upstream's `filter_surround`.
const FILTER_SURROUND: usize = 2;

/// A cap on recursive subdivision.
///
/// Without one a run that cannot be fitted — which a degenerate tangent can produce — subdivides
/// until the stack gives out. 24 is far past what any real outline needs.
const MAX_DEPTH: usize = 24;

type Point = (f32, f32);

/// Fits one closed loop of traced boundary points to a path.
///
/// The loop is split at its corners and each run between corners is fitted separately, because a
/// corner is precisely the place a single smooth spline must not cross: fitting through one rounds
/// it off, and a rectangle with rounded corners is the wrong answer to "convert this selection".
pub(crate) fn fit_closed_loop(points: &[Point]) -> Vec<PathCommand> {
    if points.len() < 4 {
        return polyline(points);
    }
    let corners = find_corners(points);
    // Corners are found on the RAW outline and the smoothing is then forbidden to move them. Done
    // the other way round, the smoothing rounds a 90-degree corner off before anything has recorded
    // that it was there, and a rectangle converts with soft corners.
    let points = &smooth_outline(points, &corners);
    let mut commands = Vec::new();

    if corners.is_empty() {
        // A loop with no corner at all -- a circle, a blob. It CANNOT be fitted as one run: a
        // single cubic has two endpoints and a closed curve has none, so the start and end tangents
        // come out nearly parallel, the least-squares system is degenerate, and the error never
        // falls below the threshold. The subdivision then runs to its depth cap and emits a hundred
        // tiny segments -- which is what the first version of this did, measured at 105 anchors for
        // a circle whose straight trace was 116.
        //
        // Split into quarters instead. Four is the minimum that works: halves give each piece
        // antiparallel end tangents, which is degenerate in the same way.
        let count = points.len();
        let splits: Vec<usize> = (0..4).map(|quarter| quarter * count / 4).collect();
        commands.push(PathCommand::MoveTo {
            x: points[splits[0]].0,
            y: points[splits[0]].1,
        });
        for index in 0..4 {
            let from = splits[index];
            let to = splits[(index + 1) % 4];
            let run: Vec<Point> = if to > from {
                points[from..=to].to_vec()
            } else {
                points[from..]
                    .iter()
                    .chain(points[..=to].iter())
                    .copied()
                    .collect()
            };
            if run.len() >= 2 {
                emit_fitted(&run, &mut commands, 0);
            }
        }
        commands.push(PathCommand::Close);
        return commands;
    }

    commands.push(PathCommand::MoveTo {
        x: points[corners[0]].0,
        y: points[corners[0]].1,
    });
    for index in 0..corners.len() {
        let from = corners[index];
        let to = corners[(index + 1) % corners.len()];
        // Each run INCLUDES both corners, so consecutive runs share an endpoint and the path has no
        // gap at a corner.
        let run: Vec<Point> = if to > from {
            points[from..=to].to_vec()
        } else {
            points[from..]
                .iter()
                .chain(points[..=to].iter())
                .copied()
                .collect()
        };
        if run.len() < 2 {
            continue;
        }
        emit_fitted(&run, &mut commands, 0);
    }
    commands.push(PathCommand::Close);
    commands
}

fn polyline(points: &[Point]) -> Vec<PathCommand> {
    let mut commands = Vec::new();
    if let Some(first) = points.first() {
        commands.push(PathCommand::MoveTo {
            x: first.0,
            y: first.1,
        });
        for point in &points[1..] {
            commands.push(PathCommand::LineTo {
                x: point.0,
                y: point.1,
            });
        }
        commands.push(PathCommand::Close);
    }
    commands
}

/// Smooths a closed outline toward the shape its pixel staircase approximates, leaving corners put.
///
/// Re-derived from upstream's `filter` in `plug-ins/selection-to-path/fit.c`: each point is nudged
/// by the sum of the vectors to its surrounding neighbours, scaled by `FILTER_PERCENT`, repeated
/// `FILTER_ITERATIONS` times, with corners held fixed.
///
/// Upstream additionally compares the angle measured over a SMALLER surround and prefers it when it
/// is more than ten degrees larger, which keeps a small feature from dissolving into the curve
/// around it. That refinement is not implemented here, and the consequence is honest to state: a
/// detail only two or three pixels across is smoothed away more than upstream would smooth it.
fn smooth_outline(points: &[Point], corners: &[usize]) -> Vec<Point> {
    let count = points.len();
    if count < 5 {
        return points.to_vec();
    }
    let is_corner: Vec<bool> = {
        let mut flags = vec![false; count];
        for index in corners {
            flags[*index] = true;
        }
        flags
    };
    let mut current = points.to_vec();
    for _ in 0..FILTER_ITERATIONS {
        let mut next = current.clone();
        for index in 0..count {
            if is_corner[index] {
                continue;
            }
            // The summed vectors to the neighbours each side. Their sum is zero on a straight run
            // and points inward on a curve, which is what moves a staircase onto the smooth shape.
            let mut sum = (0.0f32, 0.0f32);
            let mut contributors = 0usize;
            for offset in 1..=FILTER_SURROUND {
                let before = current[(index + count - offset) % count];
                let after = current[(index + offset) % count];
                sum.0 += (before.0 - current[index].0) + (after.0 - current[index].0);
                sum.1 += (before.1 - current[index].1) + (after.1 - current[index].1);
                contributors += 2;
            }
            if contributors == 0 {
                continue;
            }
            next[index] = (
                current[index].0 + sum.0 * FILTER_PERCENT / contributors as f32,
                current[index].1 + sum.1 * FILTER_PERCENT / contributors as f32,
            );
        }
        current = next;
    }
    current
}

/// Indices of the points that are corners, in order.
fn find_corners(points: &[Point]) -> Vec<usize> {
    let count = points.len();
    if count <= CORNER_SURROUND * 2 + 2 {
        return Vec::new();
    }
    let angle_at = |index: usize| -> f32 {
        let before = points[(index + count - CORNER_SURROUND) % count];
        let here = points[index];
        let after = points[(index + CORNER_SURROUND) % count];
        let (ax, ay) = (before.0 - here.0, before.1 - here.1);
        let (bx, by) = (after.0 - here.0, after.1 - here.1);
        let dot = ax * bx + ay * by;
        let magnitude = (ax * ax + ay * ay).sqrt() * (bx * bx + by * by).sqrt();
        if magnitude <= f32::EPSILON {
            // Coincident neighbours say nothing about the angle. 180 degrees means "not a corner",
            // which is the safe reading: a false corner inserts an anchor that cannot be justified.
            return 180.0;
        }
        (dot / magnitude).clamp(-1.0, 1.0).acos().to_degrees()
    };

    let angles: Vec<f32> = (0..count).map(angle_at).collect();
    let mut corners = Vec::new();
    for index in 0..count {
        let angle = angles[index];
        if angle > CORNER_DEGREES {
            continue;
        }
        if angle <= CORNER_ALWAYS_DEGREES {
            corners.push(index);
            continue;
        }
        // Between the two thresholds a point is a corner only if nothing sharper sits within the
        // window. Without this test one real corner reports as several adjacent ones, and the fit
        // gets a run two points long on either side of it.
        let sharpest = (1..=CORNER_SURROUND).all(|offset| {
            let left = angles[(index + count - offset) % count];
            let right = angles[(index + offset) % count];
            angle <= left && angle <= right
        });
        if sharpest {
            corners.push(index);
        }
    }
    // Adjacent survivors collapse to one: an exact tie inside the window lets two neighbours both
    // pass the test above, and two anchors on the same corner is one too many.
    corners.dedup_by(|a, b| a.abs_diff(*b) <= 1);
    corners
}

/// Fits `run` with one cubic, subdividing at the worst-fitting point while the error is too large.
fn emit_fitted(run: &[Point], commands: &mut Vec<PathCommand>, depth: usize) {
    if run.len() == 2 {
        commands.push(PathCommand::LineTo {
            x: run[1].0,
            y: run[1].1,
        });
        return;
    }
    let spline = fit_one_cubic(run);
    let (error, worst) = max_deviation(run, &spline);
    if error <= ERROR_THRESHOLD || depth >= MAX_DEPTH || worst == 0 || worst + 1 >= run.len() {
        commands.push(PathCommand::CubicTo {
            control1_x: spline[1].0,
            control1_y: spline[1].1,
            control2_x: spline[2].0,
            control2_y: spline[2].1,
            x: spline[3].0,
            y: spline[3].1,
        });
        return;
    }
    // Split AT the worst point and include it in both halves, so the two cubics meet there.
    emit_fitted(&run[..=worst], commands, depth + 1);
    emit_fitted(&run[worst..], commands, depth + 1);
}

/// Least-squares fit of one cubic through `run`, with the ends pinned and the tangents fixed.
///
/// Chord-length parameterisation, and the two interior control points solved along the end tangents
/// — Schneider's formulation. Uniform parameterisation was the simpler option and is wrong on a
/// staircase: the points are not evenly spaced along the curve (a diagonal step is 1.41 long, an
/// axis-aligned one 1.0), so uniform `t` pulls the fit toward the densely sampled stretches.
fn fit_one_cubic(run: &[Point]) -> [Point; 4] {
    let first = run[0];
    let last = run[run.len() - 1];
    let start_tangent = normalize(tangent(run, true));
    let end_tangent = normalize(tangent(run, false));
    let parameters = chord_lengths(run);

    // The two Bernstein terms that multiply the unknowns, accumulated into a 2x2 normal system.
    let mut c = [[0.0f32; 2]; 2];
    let mut x = [0.0f32; 2];
    for (index, point) in run.iter().enumerate() {
        let t = parameters[index];
        let inverse = 1.0 - t;
        let b0 = inverse * inverse * inverse;
        let b1 = 3.0 * inverse * inverse * t;
        let b2 = 3.0 * inverse * t * t;
        let b3 = t * t * t;
        let a1 = (start_tangent.0 * b1, start_tangent.1 * b1);
        let a2 = (end_tangent.0 * b2, end_tangent.1 * b2);
        c[0][0] += a1.0 * a1.0 + a1.1 * a1.1;
        c[0][1] += a1.0 * a2.0 + a1.1 * a2.1;
        c[1][0] = c[0][1];
        c[1][1] += a2.0 * a2.0 + a2.1 * a2.1;
        let target = (
            point.0 - (first.0 * (b0 + b1) + last.0 * (b2 + b3)),
            point.1 - (first.1 * (b0 + b1) + last.1 * (b2 + b3)),
        );
        x[0] += a1.0 * target.0 + a1.1 * target.1;
        x[1] += a2.0 * target.0 + a2.1 * target.1;
    }

    let determinant = c[0][0] * c[1][1] - c[1][0] * c[0][1];
    let (alpha1, alpha2) = if determinant.abs() <= f32::EPSILON {
        // A degenerate system -- collinear points, or a run whose tangents are parallel. Fall back
        // to control points a third of the way along the chord, which is the cubic that draws the
        // straight line between the ends.
        let chord = distance(first, last) / 3.0;
        (chord, chord)
    } else {
        (
            (x[0] * c[1][1] - x[1] * c[0][1]) / determinant,
            (c[0][0] * x[1] - c[1][0] * x[0]) / determinant,
        )
    };
    // A negative or absurd handle length turns the curve inside out. Clamped to the chord, past
    // which a handle can only loop back on itself.
    let chord = distance(first, last).max(f32::EPSILON);
    let alpha1 = alpha1.clamp(0.0, chord);
    let alpha2 = alpha2.clamp(0.0, chord);
    [
        first,
        (
            first.0 + start_tangent.0 * alpha1,
            first.1 + start_tangent.1 * alpha1,
        ),
        (
            last.0 + end_tangent.0 * alpha2,
            last.1 + end_tangent.1 * alpha2,
        ),
        last,
    ]
}

/// The tangent at a run's start or end, averaged over the surround window.
///
/// Averaged rather than taken from the single adjacent point, for the same reason corners need a
/// window: one step of a staircase points at 45 or 90 degrees regardless of where the boundary is
/// actually heading.
fn tangent(run: &[Point], at_start: bool) -> Point {
    let span = CORNER_SURROUND.min(run.len() - 1);
    if at_start {
        let base = run[0];
        let mut sum = (0.0, 0.0);
        for point in &run[1..=span] {
            sum = (sum.0 + point.0 - base.0, sum.1 + point.1 - base.1);
        }
        sum
    } else {
        let base = run[run.len() - 1];
        let mut sum = (0.0, 0.0);
        for point in &run[run.len() - 1 - span..run.len() - 1] {
            sum = (sum.0 + point.0 - base.0, sum.1 + point.1 - base.1);
        }
        sum
    }
}

fn normalize(vector: Point) -> Point {
    let length = (vector.0 * vector.0 + vector.1 * vector.1).sqrt();
    if length <= f32::EPSILON {
        return (0.0, 0.0);
    }
    (vector.0 / length, vector.1 / length)
}

fn distance(a: Point, b: Point) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Chord-length parameters in 0..=1, one per point.
fn chord_lengths(run: &[Point]) -> Vec<f32> {
    let mut lengths = Vec::with_capacity(run.len());
    let mut total = 0.0;
    lengths.push(0.0);
    for pair in run.windows(2) {
        total += distance(pair[0], pair[1]);
        lengths.push(total);
    }
    if total <= f32::EPSILON {
        return vec![0.0; run.len()];
    }
    lengths.iter().map(|length| length / total).collect()
}

/// The largest distance from any traced point to the fitted spline, and which point it was.
fn max_deviation(run: &[Point], spline: &[Point; 4]) -> (f32, usize) {
    let parameters = chord_lengths(run);
    let mut worst = (0.0f32, 0usize);
    for (index, point) in run.iter().enumerate() {
        let on_spline = evaluate(spline, parameters[index]);
        let error = distance(*point, on_spline);
        if error > worst.0 {
            worst = (error, index);
        }
    }
    worst
}

fn evaluate(spline: &[Point; 4], t: f32) -> Point {
    let inverse = 1.0 - t;
    let b0 = inverse * inverse * inverse;
    let b1 = 3.0 * inverse * inverse * t;
    let b2 = 3.0 * inverse * t * t;
    let b3 = t * t * t;
    (
        spline[0].0 * b0 + spline[1].0 * b1 + spline[2].0 * b2 + spline[3].0 * b3,
        spline[0].1 * b0 + spline[1].1 * b1 + spline[2].1 * b2 + spline[3].1 * b3,
    )
}
