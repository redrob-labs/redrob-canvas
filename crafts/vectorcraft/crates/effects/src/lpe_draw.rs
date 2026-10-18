//! Live path effects that draw from the path (Effect › Path Effects): Show Handles, Measure
//! Segments, Ruler, Interpolate Points, Knot, VonKoch, Gears, Embroidery Stitch, Stitch
//! Sub-Paths, Hatches, Sketch, Join Type, Simplify and Slice. Each takes the path's own anchors
//! (and, where it matters, the stroke the effect sits on) and gives back geometry; "random"
//! effects use the seeded noise of [`crate::util::noise`], so a redraw shows the same picture.

use redrob_vector_doc::StrokeLayer;
use redrob_vector_geom::kurbo::{self, ParamCurveArclen};
use redrob_vector_geom::shapes;
use redrob_vector_geom::{Affine, Anchor, AnchorKind, BezPath, CubicBez, FillRule, ParamCurve, PathData, Point, Rect, SubPath, Vec2};
use serde_json::Value;

use crate::lpe_util::*;
use crate::util::{flag, num, text};

pub const SHOW_HANDLES: &str = "path.showHandles";
pub const MEASURE_SEGMENTS: &str = "path.measureSegments";
pub const RULER: &str = "path.ruler";
pub const INTERPOLATE_POINTS: &str = "path.interpolatePoints";
pub const KNOT: &str = "path.knot";
pub const VONKOCH: &str = "path.vonKoch";
pub const GEARS: &str = "path.gears";
pub const EMBROIDERY_STITCH: &str = "path.embroideryStitch";
pub const STITCH_SUBPATHS: &str = "path.stitchSubpaths";
pub const HATCHES: &str = "path.hatches";
pub const SKETCH: &str = "path.sketch";
pub const JOIN_TYPE: &str = "path.joinType";
pub const SIMPLIFY: &str = "path.simplify";
pub const SLICE: &str = "path.slice";

/// The interpolators of Interpolate Points: (label, value).
pub const INTERPOLATORS: [(&str, &str); 4] =
    [("Cubic Bezier Fit", "cubicBezierFit"), ("Centripetal Catmull-Rom", "centripetalCatmullRom"), ("Spiro", "spiro"), ("Linear", "linear")];

/// The joins of Join Type: (label, value).
pub const JOIN_KINDS: [(&str, &str); 7] = [
    ("Bevel", "bevel"),
    ("Round", "round"),
    ("Miter", "miter"),
    ("Miter Clip", "miterClip"),
    ("Extrapolated Arc", "extrapolated"),
    ("Extrapolated Arc Alt", "extrapolatedAlt"),
    ("Extrapolated Smooth", "extrapolatedSmooth"),
];

/// The caps of Join Type: (label, value).
pub const CAP_KINDS: [(&str, &str); 3] = [("Butt", "butt"), ("Round", "round"), ("Square", "square")];

/// The sides Slice keeps: (label, value).
pub const SLICE_SIDES: [(&str, &str); 3] = [("Left", "left"), ("Right", "right"), ("Both", "both")];

/// The ways Ruler points its marks: (label, value).
pub const RULER_SIDES: [(&str, &str); 3] = [("Left", "left"), ("Right", "right"), ("Both", "both")];

/// The orderings of Embroidery Stitch: (label, value).
pub const STITCH_ORDERS: [(&str, &str); 2] = [("As Is", "asIs"), ("Nearest Neighbour", "nearest")];

/// The connections of Embroidery Stitch: (label, value).
pub const STITCH_CONNECTIONS: [(&str, &str); 3] = [("Straight", "straight"), ("Curve", "curve"), ("Jump", "jump")];

/// The strands Knot switches: (label, value).
pub const KNOT_SWITCHERS: [(&str, &str); 3] = [("Alternate", "alternate"), ("First Over", "firstOver"), ("Second Over", "secondOver")];

/// The reference segment of VonKoch: (label, value).
pub const VONKOCH_REFERENCES: [(&str, &str); 2] = [("Endpoints", "endpoints"), ("First Segment", "firstSegment")];

const UNIT: Rect = Rect::new(-1.0, -1.0, 1.0, 1.0);

fn circle(c: Point, r: f64) -> SubPath {
    shapes::ellipse(UNIT).transformed(Affine::translate(c.to_vec2()) * Affine::scale(r.max(1e-9))).subpaths.remove(0)
}

fn square(c: Point, half: f64) -> SubPath {
    shapes::rectangle(Rect::new(c.x - half, c.y - half, c.x + half, c.y + half)).subpaths.remove(0)
}

// ----- Show Handles -----

/// Show Handles: the path's nodes as squares, its handles as lines from the node ending in a
/// circle, and (with `showPath`) the path itself, sized by `nodeSize` and `handleSize`.
pub fn show_handles(path: &PathData, p: &Value) -> PathData {
    let node = num(p, "nodeSize", 4.0).clamp(0.0, 100.0) / 2.0;
    let handle = num(p, "handleSize", 3.0).clamp(0.0, 100.0) / 2.0;
    let (nodes, handles, show_path) = (flag(p, "showNodes", true), flag(p, "showHandles", true), flag(p, "showPath", true));
    let mut out = Vec::new();
    if show_path {
        out.extend(path.subpaths.iter().cloned());
    }
    for sp in &path.subpaths {
        for a in &sp.anchors {
            if handles {
                for h in [a.h_in, a.h_out] {
                    if h.distance(a.p) > 1e-9 {
                        out.push(line(a.p, h));
                        if handle > 0.0 {
                            out.push(circle(h, handle));
                        }
                    }
                }
            }
            if nodes && node > 0.0 {
                out.push(square(a.p, node));
            }
        }
    }
    PathData::new(out)
}

// ----- Measure Segments -----

/// An arrowhead of `size` at `tip` pointing along `dir`, as an open V.
fn arrow_head(tip: Point, dir: Vec2, size: f64) -> Option<SubPath> {
    let d = unit(dir)?;
    let n = Vec2::new(d.y, -d.x);
    let back = tip - d * size;
    Some(SubPath::polyline(&[back + n * (size * 0.4), tip, back - n * (size * 0.4)], false))
}

/// Measure Segments: a dimension line beside every segment (`offset` away, on the left by default)
/// with arrowheads and the segment's length as a label in the line font: `unit`, `precision`,
/// `scale` (a factor applied to the length) and `fontSize`; `helperLines` adds the ticks from the
/// segment's ends to the dimension line, `arcLength` measures along a curve instead of its chord,
/// and `hideCollinear` merges runs of segments that continue straight on into one measurement.
pub fn measure_segments(path: &PathData, p: &Value) -> PathData {
    let offset = num(p, "offset", 10.0).clamp(-1e4, 1e4);
    let size = num(p, "fontSize", 8.0).clamp(0.5, 1000.0);
    let precision = num(p, "precision", 2.0).clamp(0.0, 8.0) as usize;
    let scale = num(p, "scale", 100.0).max(0.0) / 100.0;
    let per_unit = unit_scale(p, "unit");
    let unit_name = text(p, "unit", "pt").trim().to_ascii_lowercase();
    let show_unit = flag(p, "showUnit", true);
    let arrows = flag(p, "arrows", true);
    let helpers = flag(p, "helperLines", true);
    let arc = flag(p, "arcLength", false);
    let collinear = flag(p, "hideCollinear", true);
    let flip = flag(p, "flip", false);
    let text_offset = num(p, "textOffset", 2.0);
    let mut out = Vec::new();
    if flag(p, "showPath", true) {
        out.extend(path.subpaths.iter().cloned());
    }
    for sp in &path.subpaths {
        let n = sp.segment_count();
        if n == 0 {
            continue;
        }
        // Runs of segments measured as one: a straight segment continuing the previous straight
        // one joins its run.
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut start = 0;
        for i in 0..n {
            let continues = collinear && i > 0 && sp.segment_is_line(i - 1) && sp.segment_is_line(i) && {
                let (a, b) = (sp.segment(i - 1), sp.segment(i));
                match (unit(a.p3 - a.p0), unit(b.p3 - b.p0)) {
                    (Some(u), Some(v)) => u.cross(v).abs() < 1e-6 && u.dot(v) > 0.0,
                    _ => false,
                }
            };
            if !continues && i > start {
                runs.push((start, i));
                start = i;
            }
        }
        runs.push((start, n));
        for (from, to) in runs {
            let a = sp.segment(from).p0;
            let b = sp.segment(to - 1).p3;
            let length = if arc { (from..to).map(|i| sp.segment(i).arclen(ARCLEN)).sum::<f64>() } else { a.distance(b) };
            let Some(d) = unit(b - a) else { continue };
            let side = if flip { -1.0 } else { 1.0 };
            let nrm = Vec2::new(d.y, -d.x) * side;
            let (a2, b2) = (a + nrm * offset, b + nrm * offset);
            if helpers && offset.abs() > 1e-9 {
                out.push(line(a, a2));
                out.push(line(b, b2));
            }
            out.push(line(a2, b2));
            let head = (size * 0.8).min(a.distance(b) / 3.0);
            if arrows && head > 0.2 {
                out.extend(arrow_head(a2, -d, head));
                out.extend(arrow_head(b2, d, head));
            }
            let mut label_text = format_number(length * scale / per_unit, precision);
            if show_unit && !unit_name.is_empty() {
                label_text.push(' ');
                label_text.push_str(&unit_name);
            }
            // The label reads left to right: along the segment, or against it when it runs leftwards.
            let dir = if d.x < -1e-9 || (d.x.abs() <= 1e-9 && d.y > 0.0) { -d } else { d };
            let up = Vec2::new(dir.y, -dir.x);
            // Above the dimension line on the side away from the path.
            let above = if nrm.dot(up) >= 0.0 { up } else { -up };
            let mid = a2.midpoint(b2) + above * text_offset;
            let w = label_width(&label_text, size);
            let origin = mid - dir * (w / 2.0) + if above == up { Vec2::ZERO } else { -up * size };
            out.extend(label(&label_text, origin, dir, size));
        }
    }
    PathData::new(out)
}

// ----- Ruler -----

/// Ruler: tick marks along the path every `markDistance` (in `unit`), `minorLength` long, every
/// `majorEvery`-th one `majorLength`, from `offset` along the path, on the left, right or both
/// sides (`direction`); `border` draws the path itself and `endMarks` puts marks at both ends.
pub fn ruler(path: &PathData, p: &Value) -> PathData {
    let per_unit = unit_scale(p, "unit");
    let step = (num(p, "markDistance", 20.0) * per_unit).max(0.1);
    let minor = num(p, "minorLength", 5.0).clamp(0.0, 1e4);
    let major = num(p, "majorLength", 10.0).clamp(0.0, 1e4);
    let every = num(p, "majorEvery", 5.0).clamp(1.0, 1000.0).round() as usize;
    let offset = num(p, "offset", 0.0) * per_unit;
    let side = text(p, "direction", "left").to_ascii_lowercase();
    let shift = num(p, "shift", 0.0);
    let mut out = Vec::new();
    if flag(p, "border", true) {
        out.extend(path.subpaths.iter().cloned());
    }
    let end_marks = flag(p, "endMarks", true);
    for sp in &path.subpaths {
        let m = Measured::new(sp);
        if m.is_empty() {
            continue;
        }
        let count = ((m.total - offset) / step).floor().max(0.0) as usize;
        let mut marks: Vec<(f64, bool)> = (0..=count).map(|k| (offset + k as f64 * step, k.is_multiple_of(every))).collect();
        if end_marks && !m.closed && marks.last().is_none_or(|(s, _)| m.total - s > 1e-6) {
            marks.push((m.total, true));
        }
        for (s, is_major) in marks {
            if s < -1e-9 || s > m.total + 1e-9 {
                continue;
            }
            let (q, t) = m.at(s);
            let n = Vec2::new(t.y, -t.x);
            let len = if is_major { major } else { minor };
            let base = q + n * shift;
            let (lo, hi) = match side.as_str() {
                "right" => (0.0, -len),
                "both" => (len / 2.0, -len / 2.0),
                _ => (len, 0.0),
            };
            out.push(line(base + n * hi, base + n * lo));
        }
    }
    PathData::new(out)
}

// ----- Interpolate Points -----

/// Interpolate Points: a smooth curve through the path's anchors by `interpolator` (see
/// [`INTERPOLATORS`]); the handles the anchors had are ignored.
pub fn interpolate_points(path: &PathData, p: &Value) -> PathData {
    let kind = text(p, "interpolator", "centripetalCatmullRom").to_ascii_lowercase();
    PathData::new(
        path.subpaths
            .iter()
            .map(|sp| {
                let pts: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
                match kind.as_str() {
                    "linear" => SubPath::polyline(&pts, sp.closed),
                    "spiro" => redrob_vector_geom::spiro_approx_to_bezier(&pts, sp.closed),
                    "cubicbezierfit" => crate::util::catmull_rom(&pts, sp.closed, 1.0),
                    _ => centripetal(&pts, sp.closed),
                }
            })
            .collect(),
    )
}

/// A centripetal Catmull-Rom spline through `pts` (chord lengths to the power ½ parameterise
/// it, so the curve neither overshoots nor loops at closely spaced points).
fn centripetal(pts: &[Point], closed: bool) -> SubPath {
    let n = pts.len();
    if n < 3 {
        return SubPath::polyline(pts, closed);
    }
    let at = |i: isize| -> Point { if closed { pts[i.rem_euclid(n as isize) as usize] } else { pts[i.clamp(0, n as isize - 1) as usize] } };
    let anchors = (0..n as isize)
        .map(|i| {
            let (p0, p1, p2) = (at(i - 1), at(i), at(i + 1));
            let d1 = p1.distance(p0).sqrt().max(1e-6);
            let d2 = p2.distance(p1).sqrt().max(1e-6);
            // Barry–Goldman tangent at p1.
            let t = ((p1 - p0) / d1 - (p2 - p0) / (d1 + d2) + (p2 - p1) / d2) * (d1 * d2 / (d1 + d2));
            let (h_in, h_out) = if !closed && i == 0 {
                (p1, p1 + t * (d2 / 3.0))
            } else if !closed && i == n as isize - 1 {
                (p1 - t * (d1 / 3.0), p1)
            } else {
                (p1 - t * (d1 / 3.0), p1 + t * (d2 / 3.0))
            };
            Anchor { p: p1, h_in, h_out, kind: AnchorKind::Smooth }
        })
        .collect();
    SubPath::new(anchors, closed)
}

// ----- Knot -----

/// One crossing between two strands: (subpath, distance along it) of each.
#[derive(Clone, Copy, Debug)]
struct Crossing {
    a: (usize, f64),
    b: (usize, f64),
}

/// Every crossing of the path with itself (within a subpath and between subpaths).
fn crossings(ms: &[Measured]) -> Vec<Crossing> {
    let polys: Vec<Vec<(Point, f64)>> = ms.iter().map(|m| flatten(m, 0.2)).collect();
    let mut out = Vec::new();
    for i in 0..polys.len() {
        for j in i..polys.len() {
            let (pa, pb) = (&polys[i], &polys[j]);
            for (ka, wa) in pa.windows(2).enumerate() {
                let from = if i == j { ka + 2 } else { 0 };
                for (kb, wb) in pb.windows(2).enumerate().skip(from) {
                    // A closed polyline's last edge meets its first at the start: not a crossing.
                    if i == j && ms[i].closed && ka == 0 && kb == pb.len() - 2 {
                        continue;
                    }
                    if let Some((t, u)) = segment_cross(wa[0].0, wa[1].0, wb[0].0, wb[1].0) {
                        let sa = wa[0].1 + (wa[1].1 - wa[0].1) * t;
                        let sb = wb[0].1 + (wb[1].1 - wb[0].1) * u;
                        out.push(Crossing { a: (i, sa), b: (j, sb) });
                    }
                }
            }
        }
    }
    out
}

/// Knot: where the path crosses itself the strand underneath is interrupted by a gap of
/// `gapWidth` (plus the stroke's width with `addStrokeWidth`), so the path reads as a knot. The
/// strands alternate over and under along the path (`switcher`: alternate, the first-met strand
/// over, or under); `switch` lists crossings (by number along the path) whose over and under are
/// swapped, `inverse` swaps them all.
pub fn knot(path: &PathData, p: &Value, stroke: Option<&StrokeLayer>) -> PathData {
    let mut gap = num(p, "gapWidth", 3.0).clamp(0.0, 1e4);
    if flag(p, "addStrokeWidth", true) {
        gap += stroke.map_or(1.0, |s| s.width.max(0.0));
    }
    let ms: Vec<Measured> = path.subpaths.iter().map(Measured::new).collect();
    let cr = crossings(&ms);
    if cr.is_empty() || gap <= 0.0 {
        return path.clone();
    }
    // Events in path order: (subpath, distance, crossing index, which end).
    let mut events: Vec<(usize, f64, usize, bool)> = Vec::with_capacity(cr.len() * 2);
    for (k, c) in cr.iter().enumerate() {
        events.push((c.a.0, c.a.1, k, true));
        events.push((c.b.0, c.b.1, k, false));
    }
    events.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)));
    let switcher = text(p, "switcher", "alternate").to_ascii_lowercase();
    let switches = numbers(p, "switch").iter().map(|v| v.round() as usize).collect::<Vec<_>>();
    let inverse = flag(p, "inverse", false);
    // Which strand (a: true) goes under, per crossing; None until decided.
    let mut under: Vec<Option<bool>> = vec![None; cr.len()];
    let mut over_next = true;
    let mut met = 0usize;
    for (_, _, k, is_a) in &events {
        if under[*k].is_some() {
            continue;
        }
        let over_here = match switcher.as_str() {
            "firstover" => true,
            "secondover" => false,
            _ => over_next,
        };
        over_next = !over_next;
        met += 1;
        let mut this_under = !over_here;
        if switches.contains(&met) {
            this_under = !this_under;
        }
        if inverse {
            this_under = !this_under;
        }
        // The strand met first is `is_a`; under[k] says whether strand a is under.
        under[*k] = Some(if *is_a { this_under } else { !this_under });
    }
    // Gaps per subpath.
    let mut gaps: Vec<Vec<(f64, f64)>> = vec![vec![]; ms.len()];
    for (k, c) in cr.iter().enumerate() {
        let a_under = under[k].unwrap_or(true);
        let (sp, s) = if a_under { c.a } else { c.b };
        gaps[sp].push((s - gap / 2.0, s + gap / 2.0));
    }
    let mut out = Vec::new();
    for (i, m) in ms.iter().enumerate() {
        if m.is_empty() {
            continue;
        }
        let g = &mut gaps[i];
        if g.is_empty() {
            out.push(path.subpaths[i].clone());
            continue;
        }
        g.sort_by(|x, y| x.0.total_cmp(&y.0));
        // Kept intervals between the gaps (a closed subpath wraps round from the last gap to the first).
        let mut keep: Vec<(f64, f64)> = Vec::new();
        if m.closed {
            for w in 0..g.len() {
                let end = g[w].1;
                let next = if w + 1 < g.len() { g[w + 1].0 } else { g[0].0 + m.total };
                if next > end + 1e-6 {
                    keep.push((end, next));
                }
            }
        } else {
            let mut at = 0.0;
            for (lo, hi) in g.iter() {
                if *lo > at + 1e-6 {
                    keep.push((at, lo.min(m.total)));
                }
                at = at.max(*hi);
            }
            if m.total > at + 1e-6 {
                keep.push((at, m.total));
            }
        }
        for (lo, hi) in keep {
            out.extend(m.slice(lo, hi));
        }
    }
    PathData::new(out)
}

// ----- VonKoch -----

/// VonKoch: a fractal from the path as generator. The reference segment (the generator's first
/// and last anchor, or its first segment) stands for one segment of the figure; `iterations`
/// times, every segment of the figure is replaced by the generator fitted onto it (turned and
/// scaled so the reference segment lands on that segment). `maxComplexity` caps the number of
/// anchors drawn; `drawAll` keeps every generation; `showGenerator` keeps the generator too.
pub fn von_koch(path: &PathData, p: &Value) -> PathData {
    let Some(sp) = path.subpaths.iter().find(|s| s.anchors.len() >= 2) else { return path.clone() };
    let iterations = num(p, "iterations", 3.0).clamp(0.0, 12.0).round() as usize;
    let max_complexity = num(p, "maxComplexity", 5000.0).clamp(10.0, 200_000.0) as usize;
    let (r0, r1) = match text(p, "reference", "endpoints").to_ascii_lowercase().as_str() {
        "firstsegment" => (sp.anchors[0].p, sp.anchors[1].p),
        _ => (sp.anchors[0].p, sp.anchors[sp.anchors.len() - 1].p),
    };
    if r0.distance(r1) <= 1e-9 {
        return path.clone();
    }
    let generator: Vec<Anchor> = sp.anchors.clone();
    let mut figure: Vec<Anchor> = vec![Anchor::corner(r0), Anchor::corner(r1)];
    let mut generations: Vec<Vec<Anchor>> = Vec::new();
    for _ in 0..iterations {
        let segments = figure.len().saturating_sub(1);
        if segments * (generator.len() - 1) + 1 > max_complexity {
            break;
        }
        let mut next: Vec<Anchor> = Vec::with_capacity(segments * (generator.len() - 1) + 1);
        for w in figure.windows(2) {
            let xf = similarity(r0, r1, w[0].p, w[1].p);
            let piece: Vec<Anchor> = generator.iter().map(|a| a.transform(xf)).collect();
            if let Some(last) = next.last_mut()
                && let Some(first) = piece.first()
            {
                last.h_out = first.h_out;
                next.extend(piece.iter().skip(1).copied());
            } else {
                next.extend(piece.iter().copied());
            }
        }
        if flag(p, "drawAll", false) {
            generations.push(figure);
        }
        figure = next;
    }
    let mut out = Vec::new();
    if flag(p, "showGenerator", false) {
        out.push(sp.clone());
    }
    for g in generations {
        out.push(SubPath::new(g, false));
    }
    out.push(SubPath::new(figure, false));
    PathData::new(out)
}

// ----- Gears -----

/// The involute function `tan φ − φ`.
fn involute(phi: f64) -> f64 {
    phi.tan() - phi
}

/// One gear: `teeth` teeth on the pitch circle of radius `r` about `c`, module `m`, pressure
/// angle `pressure` (radians), a tooth centred at angle `phase`.
fn gear(c: Point, r: f64, teeth: usize, m: f64, pressure: f64, phase: f64) -> SubPath {
    let n = teeth.max(3);
    let ra = r + m;
    let rd = (r - 1.25 * m).max(r * 0.2);
    let rb = r * pressure.cos();
    let step = std::f64::consts::TAU / n as f64;
    // Half the tooth's angular thickness at radius `rho` (the involute's).
    let half = |rho: f64| -> f64 {
        let at_pitch = std::f64::consts::PI / (2.0 * n as f64);
        if rho <= rb {
            return at_pitch + involute(pressure);
        }
        let phi = (rb / rho).acos();
        (at_pitch + involute(pressure) - involute(phi)).max(0.01)
    };
    let at = |rho: f64, ang: f64| Point::new(c.x + rho * ang.cos(), c.y + rho * ang.sin());
    let flank_start = rb.max(rd);
    const FLANK: usize = 4;
    let mut pts = Vec::with_capacity(n * (2 * FLANK + 4));
    for k in 0..n {
        let a = phase + k as f64 * step;
        pts.push(at(rd, a - step / 2.0));
        pts.push(at(rd, a - half(flank_start)));
        for s in 0..=FLANK {
            let rho = flank_start + (ra - flank_start) * s as f64 / FLANK as f64;
            pts.push(at(rho, a - half(rho)));
        }
        for s in (0..=FLANK).rev() {
            let rho = flank_start + (ra - flank_start) * s as f64 / FLANK as f64;
            pts.push(at(rho, a + half(rho)));
        }
        pts.push(at(rd, a + half(flank_start)));
    }
    SubPath::polyline(&pts, true)
}

/// Gears: the path's anchors are the centres of a chain of meshing gears. The first gear has
/// `teeth` teeth and reaches halfway to the second centre (which sets the module, the size of
/// one tooth, shared by the chain); every later gear fills the distance to the one before it
/// with as many teeth as fit, its teeth turned to mesh. `pressureAngle` shapes the flanks.
pub fn gears(path: &PathData, p: &Value) -> PathData {
    let centres = points(path);
    if centres.len() < 2 {
        return path.clone();
    }
    let teeth0 = num(p, "teeth", 10.0).clamp(3.0, 500.0).round() as usize;
    let pressure = num(p, "pressureAngle", 20.0).clamp(5.0, 45.0).to_radians();
    let d0 = centres[0].distance(centres[1]);
    if d0 <= 1e-6 {
        return path.clone();
    }
    let module = d0 / teeth0 as f64;
    let mut out = Vec::with_capacity(centres.len());
    let mut r_prev = d0 / 2.0;
    let mut n_prev = teeth0;
    let mut phase_prev = 0.0;
    out.push(gear(centres[0], r_prev, n_prev, module, pressure, phase_prev));
    for w in centres.windows(2) {
        let (c0, c1) = (w[0], w[1]);
        let d = c0.distance(c1);
        if d <= 1e-6 {
            continue;
        }
        let r_raw = (d - r_prev).max(module * 1.5);
        let n = ((2.0 * r_raw / module).round() as usize).max(3);
        let r = n as f64 * module / 2.0;
        // The line of centres: gear 0's tooth nearest it, mapped onto gear 1 as a gap.
        let beta = (c1 - c0).atan2();
        let step0 = std::f64::consts::TAU / n_prev as f64;
        let delta = (beta - phase_prev).rem_euclid(step0);
        let delta1 = delta * n_prev as f64 / n as f64;
        let step1 = std::f64::consts::TAU / n as f64;
        let phase = beta + std::f64::consts::PI - delta1 - step1 / 2.0;
        out.push(gear(c1, r, n, module, pressure, phase));
        r_prev = r;
        n_prev = n;
        phase_prev = phase;
    }
    PathData::new(out)
}

// ----- Embroidery Stitch -----

/// Embroidery Stitch: the subpaths become one continuous stitching path. `ordering` keeps them
/// as they are or visits each next the nearest end of one not yet visited (reversing a subpath
/// when its end is nearer); `connection` joins consecutive subpaths with a straight line, a
/// curve following their end tangents, or a jump (no geometry: a new subpath).
pub fn embroidery_stitch(path: &PathData, p: &Value) -> PathData {
    let mut parts: Vec<SubPath> = path
        .subpaths
        .iter()
        .filter(|s| s.anchors.len() >= 2)
        .map(|s| {
            // Closed subpaths are stitched round and back to their start.
            let mut o = s.clone();
            if o.closed {
                o.closed = false;
                let first = o.anchors[0];
                o.anchors.push(Anchor { p: first.p, h_in: first.h_in, h_out: first.p, kind: AnchorKind::Corner });
                o.anchors[0].h_in = first.p;
            }
            o
        })
        .collect();
    if parts.is_empty() {
        return path.clone();
    }
    if text(p, "ordering", "asIs").eq_ignore_ascii_case("nearest") {
        let mut ordered = vec![parts.remove(0)];
        while !parts.is_empty() {
            let end = ordered[ordered.len() - 1].anchors.last().map(|a| a.p).unwrap_or(Point::ZERO);
            let (k, reverse) = parts
                .iter()
                .enumerate()
                .flat_map(|(k, s)| {
                    let (f, l) = (s.anchors[0].p, s.anchors[s.anchors.len() - 1].p);
                    [(k, false, end.distance(f)), (k, true, end.distance(l))]
                })
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .map(|(k, r, _)| (k, r))
                .unwrap_or((0, false));
            let mut next = parts.remove(k);
            if reverse {
                next.reverse();
            }
            ordered.push(next);
        }
        parts = ordered;
    }
    let connection = text(p, "connection", "straight").to_ascii_lowercase();
    if connection == "jump" {
        return PathData::new(parts);
    }
    let mut anchors: Vec<Anchor> = Vec::new();
    for s in parts {
        if let Some(last) = anchors.last_mut() {
            let first = s.anchors[0];
            if connection == "curve" {
                let d = last.p.distance(first.p);
                let t_out = unit(last.p - last.h_in).unwrap_or(Vec2::ZERO);
                let t_in = unit(first.h_out - first.p).unwrap_or(Vec2::ZERO);
                last.h_out = last.p + t_out * (d / 3.0);
                anchors.push(Anchor { p: first.p, h_in: first.p - t_in * (d / 3.0), h_out: first.h_out, kind: AnchorKind::Corner });
            } else {
                last.h_out = last.p;
                anchors.push(Anchor { h_in: first.p, ..first });
            }
            anchors.extend(s.anchors.iter().skip(1).copied());
        } else {
            anchors.extend(s.anchors.iter().copied());
        }
    }
    PathData::single(SubPath::new(anchors, false))
}

// ----- Stitch Sub-Paths -----

/// Stitch Sub-Paths: `lines` stitches between each pair of consecutive subpaths, from evenly
/// spaced points along the first to matching points along the second (`startSpacing` and
/// `endSpacing` scale how much of each subpath's length the stitches spread over, 1 the whole;
/// `startVariation` and `endVariation` jitter them), each stitch the `pattern` (a polyline in a
/// unit box, 0,0 its start and 1,0 its end; a straight line by default) fitted between the two
/// points. `keepPaths` keeps the subpaths themselves.
pub fn stitch_subpaths(path: &PathData, p: &Value) -> PathData {
    let n = num(p, "lines", 10.0).clamp(1.0, 2000.0).round() as usize;
    let (ss, es) = (num(p, "startSpacing", 1.0).clamp(0.0, 1.0), num(p, "endSpacing", 1.0).clamp(0.0, 1.0));
    let (sv, ev) = (num(p, "startVariation", 0.0).clamp(0.0, 1.0), num(p, "endVariation", 0.0).clamp(0.0, 1.0));
    let mut pattern = point_list(p, "pattern");
    if pattern.len() < 2 {
        pattern = vec![Point::ZERO, Point::new(1.0, 0.0)];
    }
    let (pat0, pat1) = (pattern[0], pattern[pattern.len() - 1]);
    let ms: Vec<Measured> = path.subpaths.iter().map(Measured::new).collect();
    let mut out = Vec::new();
    if flag(p, "keepPaths", true) {
        out.extend(path.subpaths.iter().cloned());
    }
    for (k, w) in ms.windows(2).enumerate() {
        let (a, b) = (&w[0], &w[1]);
        if a.is_empty() || b.is_empty() {
            continue;
        }
        for i in 0..n {
            let f = if n == 1 { 0.5 } else { i as f64 / (n - 1) as f64 };
            let fa = (0.5 + (f - 0.5) * ss + sv * noise(p, k as u64, i as u64, 1) / n as f64).clamp(0.0, 1.0);
            let fb = (0.5 + (f - 0.5) * es + ev * noise(p, k as u64, i as u64, 2) / n as f64).clamp(0.0, 1.0);
            let (qa, _) = a.at(fa * a.total);
            let (qb, _) = b.at(fb * b.total);
            if qa.distance(qb) <= 1e-9 {
                continue;
            }
            let xf = similarity(pat0, pat1, qa, qb);
            let pts: Vec<Point> = pattern.iter().map(|q| xf * *q).collect();
            out.push(SubPath::polyline(&pts, false));
        }
    }
    PathData::new(out)
}

// ----- Hatches -----

/// Hatches (rough): the shape filled with hand-drawn-looking hatch lines at `angle`, about
/// `100 / frequency` points apart (`growth` widens the spacing from the first line to the last),
/// each bowed by `bend` points (towards `bendAngle`), shifted at random by `randomness` of the
/// spacing, and drawn as a stroke `thickness` wide varying by `thicknessVariation`; the lines are
/// clipped to the shape unless `clip` is off.
pub fn hatches(path: &PathData, b: Rect, p: &Value, rule: FillRule) -> PathData {
    let angle = num(p, "angle", 45.0).to_radians();
    let frequency = num(p, "frequency", 10.0).clamp(0.1, 1000.0);
    let growth = num(p, "growth", 0.0).clamp(-0.9, 10.0);
    let bend = num(p, "bend", 0.0).clamp(-1e4, 1e4);
    let bend_angle = num(p, "bendAngle", 90.0).to_radians();
    let randomness = num(p, "randomness", 0.2).clamp(0.0, 1.0);
    let thickness = num(p, "thickness", 1.0).clamp(0.05, 1000.0);
    let variation = num(p, "thicknessVariation", 0.3).clamp(0.0, 1.0);
    let clip = flag(p, "clip", true);
    let c = b.center();
    let radius = (b.width().hypot(b.height())) / 2.0 + 1.0;
    if !radius.is_finite() || radius <= 1.0 {
        return PathData::default();
    }
    // The hatching's axes: `d` along the lines, `n` across them.
    let d = Vec2::new(angle.cos(), angle.sin());
    let n = Vec2::new(-d.y, d.x);
    let bow = Vec2::new(bend_angle.cos(), bend_angle.sin()) * bend;
    let base_spacing = (100.0 / frequency).max(0.2);
    let mut outlines: Vec<BezPath> = Vec::new();
    let mut t = -radius;
    let mut k = 0u64;
    while t <= radius && k < 20_000 {
        let fraction = ((t + radius) / (2.0 * radius)).clamp(0.0, 1.0);
        let spacing = base_spacing * (1.0 + growth * fraction);
        let shift = randomness * spacing * noise(p, k, 1, 0) * 0.5;
        let mid = c + n * (t + shift);
        let (a0, a1) = (mid - d * radius, mid + d * radius);
        let jitter = Vec2::new(noise(p, k, 2, 0), noise(p, k, 3, 0)) * (randomness * spacing * 0.25);
        let cubic = CubicBez::new(a0, a0.lerp(a1, 1.0 / 3.0) + bow + jitter, a0.lerp(a1, 2.0 / 3.0) + bow - jitter, a1);
        let width = thickness * (1.0 + variation * noise(p, k, 4, 0)).max(0.05);
        let mut bp = BezPath::new();
        bp.move_to(cubic.p0);
        bp.curve_to(cubic.p1, cubic.p2, cubic.p3);
        let style = kurbo::Stroke::new(width).with_caps(kurbo::Cap::Round).with_join(kurbo::Join::Round);
        outlines.push(kurbo::stroke(bp.iter(), &style, &kurbo::StrokeOpts::default(), 0.05));
        t += spacing;
        k += 1;
    }
    let hatch = redrob_vector_pathops::stroke_region(&outlines);
    if !clip || hatch.is_empty() {
        return hatch;
    }
    redrob_vector_pathops::boolean(&hatch, FillRule::NonZero, path, rule, redrob_vector_pathops::BoolOp::Intersect)
}

// ----- Sketch -----

/// Sketch: the path redrawn as `strokes` layers of overlapping pencil strokes about
/// `strokeLength` long, each overlapping the next by `overlap` of its length, pushed off the path
/// by up to `displacement` and shaken by `tremble`; `constructionLines` tangent lines about
/// `constructionLength` long are scattered along the path too, as a draughtsman's guides.
pub fn sketch(path: &PathData, p: &Value) -> PathData {
    let layers = num(p, "strokes", 2.0).clamp(1.0, 20.0).round() as usize;
    let stroke_len = num(p, "strokeLength", 60.0).clamp(1.0, 1e5);
    let overlap = num(p, "overlap", 0.3).clamp(0.0, 0.95);
    let displacement = num(p, "displacement", 2.0).clamp(0.0, 1e4);
    let tremble = num(p, "tremble", 0.5).clamp(0.0, 1e4);
    let lines = num(p, "constructionLines", 3.0).clamp(0.0, 1000.0).round() as usize;
    let line_len = num(p, "constructionLength", 1.0).clamp(0.0, 100.0);
    let mut out = Vec::new();
    for (si, sp) in path.subpaths.iter().enumerate() {
        let m = Measured::new(sp);
        if m.is_empty() {
            continue;
        }
        let total = m.total;
        let advance = (stroke_len * (1.0 - overlap)).max(0.5);
        let count = ((total / advance).ceil() as usize).max(1);
        for layer in 0..layers {
            for k in 0..count {
                let seed = |c: u64| noise(p, (si * 131 + layer) as u64, k as u64, c);
                let start = k as f64 * advance + seed(1) * advance * 0.5;
                let len = stroke_len * (1.0 + 0.3 * seed(2));
                let (lo, hi) = if m.closed { (start, start + len) } else { (start.max(0.0), (start + len).min(total)) };
                if hi <= lo + 1e-6 {
                    continue;
                }
                let Some(piece) = m.slice(lo, hi) else { continue };
                // Pushed off the path: one offset at the start, another at the end, blended along.
                let (o0, o1) = (displacement * seed(3), displacement * seed(4));
                let count_a = piece.anchors.len().max(2);
                let anchors: Vec<Anchor> = piece
                    .anchors
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let u = i as f64 / (count_a - 1) as f64;
                        let (_, tan) = m.at(lo + (hi - lo) * u);
                        let nrm = Vec2::new(tan.y, -tan.x);
                        let off = nrm * (o0 + (o1 - o0) * u + tremble * noise(p, (si * 131 + layer) as u64, (k * 977 + i) as u64, 5));
                        Anchor { p: a.p + off, h_in: a.h_in + off, h_out: a.h_out + off, kind: a.kind }
                    })
                    .collect();
                out.push(SubPath::new(anchors, false));
            }
        }
        for k in 0..lines {
            let seed = |c: u64| noise(p, (si * 131 + 7919) as u64, k as u64, c);
            let s = (seed(1) * 0.5 + 0.5) * total;
            let (q, tan) = m.at(s);
            let half = line_len * stroke_len * (0.6 + 0.4 * seed(2)) / 2.0;
            let a = seed(3) * 0.05;
            let dir = Vec2::new(tan.x * a.cos() - tan.y * a.sin(), tan.x * a.sin() + tan.y * a.cos());
            let shift = Vec2::new(tan.y, -tan.x) * (displacement * seed(4));
            out.push(line(q + shift - dir * half, q + shift + dir * half));
        }
    }
    PathData::new(out)
}

// ----- Join Type -----

/// The offset of segment `c` (a line when `is_line`) by `d` (positive: to the right of travel on
/// screen), as the cubics of the offset curve.
fn offset_segment(c: &CubicBez, is_line: bool, d: f64) -> Vec<CubicBez> {
    if is_line || (c.p1 == c.p0 && c.p2 == c.p3) {
        let Some(t) = unit(c.p3 - c.p0) else { return vec![] };
        let n = Vec2::new(-t.y, t.x) * d;
        let (a, b) = (c.p0 + n, c.p3 + n);
        return vec![CubicBez::new(a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b)];
    }
    let mut bp = BezPath::new();
    kurbo::offset::offset_cubic(*c, d, 0.05, &mut bp);
    bp.segments()
        .map(|s| match s {
            kurbo::PathSeg::Cubic(c) => c,
            kurbo::PathSeg::Quad(q) => q.raise(),
            kurbo::PathSeg::Line(l) => CubicBez::new(l.p0, l.p0.lerp(l.p1, 1.0 / 3.0), l.p0.lerp(l.p1, 2.0 / 3.0), l.p1),
        })
        .collect()
}

/// The arc from `a` to `b` round `centre`, as cubics (the shorter way, or the way `ccw` says).
fn arc_cubics(centre: Point, a: Point, b: Point, ccw: bool) -> Vec<CubicBez> {
    let r = centre.distance(a);
    if r <= 1e-9 {
        return vec![];
    }
    let (sa, sb) = ((a - centre).atan2(), (b - centre).atan2());
    let mut sweep = sb - sa;
    if ccw {
        if sweep < 0.0 {
            sweep += std::f64::consts::TAU;
        }
    } else if sweep > 0.0 {
        sweep -= std::f64::consts::TAU;
    }
    let arc = kurbo::Arc::new(centre, Vec2::new(r, r), sa, sweep, 0.0);
    let mut out = Vec::new();
    arc.to_cubic_beziers(0.05, |p1, p2, p3| {
        let p0 = out.last().map_or(a, |c: &CubicBez| c.p3);
        out.push(CubicBez::new(p0, p1, p2, p3));
    });
    out
}

/// A corner of a stroke's outer side: the path's vertex `v`, the end `e` of the incoming offset
/// (tangent `te`) and the start `s` of the outgoing one (tangent `ts`); `ccw` whether the outer
/// side goes counterclockwise round `v` from `e` to `s`.
#[derive(Clone, Copy)]
struct Corner {
    v: Point,
    e: Point,
    te: Vec2,
    s: Point,
    ts: Vec2,
    ccw: bool,
}

/// The join on the outer side of `corner`, as cubics from its `e` to its `s`; `w` the half width.
fn join_cubics(kind: &str, corner: &Corner, w: f64, miter_limit: f64) -> Vec<CubicBez> {
    let Corner { v, e, te, s, ts, ccw } = *corner;
    let seg = |a: Point, b: Point| CubicBez::new(a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b);
    if e.distance(s) <= 1e-9 {
        return vec![];
    }
    // Where the tangent lines through e and s meet.
    let miter_point = || -> Option<Point> {
        let den = te.cross(ts);
        if den.abs() <= 1e-9 {
            return None;
        }
        let t = (s - e).cross(ts) / den;
        (t > 0.0).then(|| e + te * t)
    };
    match kind {
        "round" => arc_cubics(v, e, s, ccw),
        "miter" | "miterclip" => {
            let Some(m) = miter_point() else { return vec![seg(e, s)] };
            let limit = miter_limit.max(1.0) * w;
            if m.distance(v) <= limit {
                vec![seg(e, m), seg(m, s)]
            } else if kind == "miterclip" {
                // Clipped at the limit: the miter's tip cut off square to its direction.
                let dir = unit(m - v).unwrap_or(Vec2::ZERO);
                let clip_at = v + dir * limit;
                let cut = |from: Point, along: Vec2| -> Point {
                    let den = along.dot(dir);
                    if den.abs() <= 1e-9 { from } else { from + along * ((clip_at - from).dot(dir) / den) }
                };
                let (c1, c2) = (cut(e, te), cut(s, -ts));
                vec![seg(e, c1), seg(c1, c2), seg(c2, s)]
            } else {
                vec![seg(e, s)]
            }
        }
        "extrapolated" | "extrapolatedalt" => {
            // The circle tangent to one side at its end and through the other side's start.
            let (from, tan, to) = if kind == "extrapolated" { (e, te, s) } else { (s, -ts, e) };
            let n = Vec2::new(-tan.y, tan.x);
            let den = 2.0 * n.dot(to - from);
            if den.abs() <= 1e-9 {
                return vec![seg(e, s)];
            }
            let centre = from + n * ((to - from).hypot2() / den);
            let arc = arc_cubics(centre, from, to, (from - centre).cross(to - centre) > 0.0);
            if kind == "extrapolated" { arc } else { arc.into_iter().rev().map(|c| CubicBez::new(c.p3, c.p2, c.p1, c.p0)).collect() }
        }
        "extrapolatedsmooth" => {
            let d = e.distance(s) / 3.0;
            vec![CubicBez::new(e, e + te * d, s - ts * d, s)]
        }
        _ => vec![seg(e, s)],
    }
}

/// The cap at `a` (an end of the path running in direction `dir`, the stroke's left point `l`
/// and right point `r`), from `l` round to `r`.
fn cap_cubics(kind: &str, a: Point, dir: Vec2, l: Point, r: Point, w: f64) -> Vec<CubicBez> {
    let seg = |a: Point, b: Point| CubicBez::new(a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b);
    match kind {
        "round" => arc_cubics(a, l, r, (l - a).cross(r - a) > 0.0),
        "square" => {
            let (l2, r2) = (l + dir * w, r + dir * w);
            vec![seg(l, l2), seg(l2, r2), seg(r2, r)]
        }
        _ => vec![seg(l, r)],
    }
}

/// One side of the stroke along `sp`: the offset of each segment by `d`, joined at the corners
/// where that side is the outer one (by `join`); on the inner side the offsets simply meet (their
/// overlap is removed when the region is filled). Returns the cubics in path order.
fn side(sp: &SubPath, d: f64, join: &str, miter_limit: f64) -> Vec<CubicBez> {
    let n = sp.segment_count();
    let w = d.abs();
    let segs: Vec<(CubicBez, Vec<CubicBez>)> =
        (0..n).map(|i| (sp.segment(i), offset_segment(&sp.segment(i), sp.segment_is_line(i), d))).filter(|(_, o)| !o.is_empty()).collect();
    let mut out: Vec<CubicBez> = Vec::new();
    let count = segs.len();
    for (k, (c, off)) in segs.iter().enumerate() {
        if k > 0 || sp.closed {
            let prev = if k > 0 { &segs[k - 1] } else { &segs[count - 1] };
            let (e_c, e_off) = (&prev.0, &prev.1);
            let e = e_off[e_off.len() - 1].p3;
            let s = off[0].p0;
            let te = unit(redrob_vector_geom::bez::tangent(&kurbo::PathSeg::Cubic(*e_c), 1.0)).unwrap_or(Vec2::ZERO);
            let ts = unit(redrob_vector_geom::bez::tangent(&kurbo::PathSeg::Cubic(*c), 0.0)).unwrap_or(Vec2::ZERO);
            // Turning towards this side makes it the inner one: the offsets cross, no join needed.
            let turn = te.cross(ts);
            let outer = if d > 0.0 { turn < -1e-9 } else { turn > 1e-9 };
            if outer {
                let v = c.p0;
                // Round the corner the way that stays on the outside of the turn.
                let ccw = (e - v).cross(s - v) > 0.0;
                out.extend(join_cubics(join, &Corner { v, e, te, s, ts, ccw }, w, miter_limit));
            } else if e.distance(s) > 1e-9 {
                out.push(CubicBez::new(e, e.lerp(s, 1.0 / 3.0), e.lerp(s, 2.0 / 3.0), s));
            }
        }
        out.extend(off.iter().copied());
    }
    out
}

/// Join Type: the stroke `width` wide along the path as a filled outline whose corners take
/// `join` (bevel, round, miter, miter clipped at `miterLimit`, or an arc extrapolating the
/// incoming side, the outgoing side, or a smooth curve between them) and whose ends take `cap`.
pub fn join_type(path: &PathData, p: &Value, stroke: Option<&StrokeLayer>) -> PathData {
    let width = p.get("width").and_then(Value::as_f64).filter(|w| w.is_finite() && *w > 0.0).or_else(|| stroke.map(|s| s.width)).unwrap_or(4.0);
    if width <= 0.0 {
        return PathData::default();
    }
    let w = width / 2.0;
    let join = text(p, "join", "extrapolated").to_ascii_lowercase();
    let cap = text(p, "cap", "butt").to_ascii_lowercase();
    let miter_limit = num(p, "miterLimit", 4.0).clamp(1.0, 500.0);
    let mut regions: Vec<PathData> = Vec::new();
    for sp in path.subpaths.iter().filter(|s| s.segment_count() > 0) {
        let right = side(sp, w, &join, miter_limit);
        let left = side(sp, -w, &join, miter_limit);
        if right.is_empty() || left.is_empty() {
            continue;
        }
        if sp.closed {
            let ring_r = PathData::single(chain(&right, true));
            let ring_l = PathData::single(chain(&left, true));
            let a = redrob_vector_pathops::normalize(&ring_r, FillRule::NonZero);
            let b = redrob_vector_pathops::normalize(&ring_l, FillRule::NonZero);
            regions.push(redrob_vector_pathops::boolean(&a, FillRule::NonZero, &b, FillRule::NonZero, redrob_vector_pathops::BoolOp::Xor));
        } else {
            let mut cubics = right.clone();
            let end_dir =
                unit(redrob_vector_geom::bez::tangent(&kurbo::PathSeg::Cubic(sp.segment(sp.segment_count() - 1)), 1.0)).unwrap_or(Vec2::ZERO);
            let end = sp.segment(sp.segment_count() - 1).p3;
            let (r_end, l_end) = (right[right.len() - 1].p3, left[left.len() - 1].p3);
            cubics.extend(cap_cubics(&cap, end, end_dir, r_end, l_end, w));
            cubics.extend(left.iter().rev().map(|c| CubicBez::new(c.p3, c.p2, c.p1, c.p0)));
            let start_dir = unit(redrob_vector_geom::bez::tangent(&kurbo::PathSeg::Cubic(sp.segment(0)), 0.0)).unwrap_or(Vec2::ZERO);
            cubics.extend(cap_cubics(&cap, sp.segment(0).p0, -start_dir, left[0].p0, right[0].p0, w));
            let mut bp = BezPath::new();
            chain(&cubics, true).to_bezpath_into(&mut bp);
            regions.push(redrob_vector_pathops::stroke_region(&[bp]));
        }
    }
    if regions.is_empty() {
        return PathData::default();
    }
    let refs: Vec<(&PathData, FillRule)> = regions.iter().map(|r| (r, FillRule::NonZero)).collect();
    redrob_vector_pathops::unite_all(&refs)
}

// ----- Simplify (live) -----

/// Simplify: the path refitted with fewer anchors, within `threshold` points of the original,
/// anchors turning by more than `smoothAngle` degrees kept as corners, `steps` times over
/// (each pass on the last's result); `straightLines` fits lines only; `separately` fits each
/// subpath alone (together, a shared tolerance of the whole path's size when off).
pub fn simplify(path: &PathData, b: Rect, p: &Value) -> PathData {
    let steps = num(p, "steps", 1.0).clamp(1.0, 10.0).round() as usize;
    let mut threshold = num(p, "threshold", 0.5).clamp(0.0, 1e4);
    if !flag(p, "separately", true) {
        // The threshold as a share of the object's size, so a scaled object simplifies alike.
        threshold = threshold * ((b.width() + b.height()) / 2.0).max(1.0) / 100.0;
    }
    let opts = redrob_vector_pathops::SimplifyOptions {
        tolerance: threshold.max(1e-4),
        corner_angle_deg: num(p, "smoothAngle", 30.0).clamp(0.0, 180.0),
        straight_lines: flag(p, "straightLines", false),
    };
    let mut cur = path.clone();
    for _ in 0..steps {
        cur = redrob_vector_pathops::simplify_with(&cur, &opts);
    }
    cur
}

// ----- Slice -----

/// The part of open subpath `sp` on the `keep_left` side of the line through `a` along `d`,
/// cut where it crosses the line.
fn slice_open(sp: &SubPath, a: Point, d: Vec2, keep_left: bool) -> Vec<SubPath> {
    let n = Vec2::new(d.y, -d.x);
    let side_of = |q: Point| (q - a).dot(n);
    let wanted = |q: Point| if keep_left { side_of(q) >= 0.0 } else { side_of(q) <= 0.0 };
    let mut out = Vec::new();
    let mut cur: Vec<CubicBez> = Vec::new();
    let far = 1e6;
    let cut_line = kurbo::Line::new(a - d * far, a + d * far);
    for i in 0..sp.segment_count() {
        let c = sp.segment(i);
        let mut ts: Vec<f64> = kurbo::PathSeg::Cubic(c).intersect_line(cut_line).iter().map(|x| x.segment_t).filter(|t| *t > 1e-9 && *t < 1.0 - 1e-9).collect();
        ts.sort_by(f64::total_cmp);
        ts.push(1.0);
        let mut t0 = 0.0;
        for t1 in ts {
            if t1 <= t0 + 1e-9 {
                continue;
            }
            let piece = c.subsegment(t0..t1);
            let mid = piece.eval(0.5);
            if wanted(mid) {
                cur.push(piece);
            } else if !cur.is_empty() {
                out.push(chain(&cur, false));
                cur.clear();
            }
            t0 = t1;
        }
    }
    if !cur.is_empty() {
        out.push(chain(&cur, false));
    }
    out
}

/// Slice: the object cut along a line (through `point` in box coordinates, at `angle` degrees;
/// vertical through the centre by default), keeping its `side`: left, right, or both halves
/// pushed apart by `gap`. Closed subpaths are cut as filled shapes, open ones as lines. A pair
/// of objects with opposite sides makes two live halves that keep their own styles.
pub fn slice(path: &PathData, b: Rect, p: &Value, rule: FillRule) -> PathData {
    let mut pt = point_list(p, "point");
    if pt.is_empty() {
        pt.push(Point::new(0.5, 0.5));
    }
    let a = from_box(b, pt[0]);
    let angle = num(p, "angle", 90.0).to_radians();
    let d = Vec2::new(angle.cos(), -angle.sin());
    let n = Vec2::new(d.y, -d.x);
    let side = text(p, "side", "left").to_ascii_lowercase();
    let gap = num(p, "gap", 0.0).clamp(0.0, 1e4);
    let half = |keep_left: bool| -> PathData {
        let far = (b.width() + b.height()) * 4.0 + 1e3;
        let sign = if keep_left { 1.0 } else { -1.0 };
        let quad = [a - d * far, a + d * far, a + d * far + n * (sign * far), a - d * far + n * (sign * far)];
        let plane = PathData::single(SubPath::polyline(&quad, true));
        let closed = PathData::new(path.subpaths.iter().filter(|s| s.closed).cloned().collect());
        let open: Vec<SubPath> = path.subpaths.iter().filter(|s| !s.closed).cloned().collect();
        let mut out = if closed.is_empty() {
            PathData::default()
        } else {
            redrob_vector_pathops::boolean(&closed, rule, &plane, FillRule::NonZero, redrob_vector_pathops::BoolOp::Intersect)
        };
        for sp in &open {
            out.subpaths.extend(slice_open(sp, a, d, keep_left));
        }
        if gap > 0.0 {
            out.transform(Affine::translate(n * (sign * gap / 2.0)));
        }
        out
    };
    match side.as_str() {
        "right" => half(false),
        "both" => {
            let mut l = half(true);
            l.subpaths.extend(half(false).subpaths);
            l
        }
        _ => half(true),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn poly(pts: &[(f64, f64)], closed: bool) -> PathData {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    }

    #[test]
    fn handles_and_rulers_draw_marks() {
        let sq = poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
        let out = show_handles(&sq, &json!({"nodeSize": 4}));
        assert_eq!(out.subpaths.len(), 1 + 4);
        let r = ruler(&poly(&[(0.0, 0.0), (100.0, 0.0)], false), &json!({"markDistance": 10, "border": false}));
        assert_eq!(r.subpaths.len(), 11);
        let m = measure_segments(
            &poly(&[(0.0, 0.0), (100.0, 0.0)], false),
            &json!({"showPath": false, "helperLines": false, "arrows": false, "showUnit": false}),
        );
        // The dimension line and the three glyphs of "100".
        assert!(m.subpaths.len() >= 4, "{}", m.subpaths.len());
    }

    #[test]
    fn knot_cuts_the_under_strand_once_per_crossing() {
        // A figure of eight: one crossing.
        let eight = poly(&[(0.0, 0.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)], false);
        let out = knot(&eight, &json!({"gapWidth": 6, "addStrokeWidth": false}), None);
        assert_eq!(out.subpaths.len(), 2);
        let total: f64 = out.subpaths.iter().map(|s| PathData::single(s.clone()).length()).sum();
        assert!((eight.length() - total - 6.0).abs() < 0.5, "{total}");
    }

    #[test]
    fn von_koch_multiplies_segments_and_gears_mesh() {
        let generator = poly(&[(0.0, 0.0), (1.0, 0.0), (1.5, -0.866), (2.0, 0.0), (3.0, 0.0)], false);
        let out = von_koch(&generator, &json!({"iterations": 2}));
        assert_eq!(out.subpaths[0].anchors.len(), 17);
        let g = gears(&poly(&[(0.0, 0.0), (100.0, 0.0), (180.0, 0.0)], false), &json!({"teeth": 10}));
        assert_eq!(g.subpaths.len(), 3);
        assert!(g.subpaths.iter().all(|s| s.closed && s.anchors.len() >= 30));
    }

    #[test]
    fn stitching_connects_subpaths() {
        let two = PathData::new(vec![
            SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0)], false),
            SubPath::polyline(&[Point::new(0.0, 50.0), Point::new(100.0, 50.0)], false),
        ]);
        let e = embroidery_stitch(&two, &json!({"connection": "straight"}));
        assert_eq!(e.subpaths.len(), 1);
        assert_eq!(e.subpaths[0].anchors.len(), 4);
        let s = stitch_subpaths(&two, &json!({"lines": 5, "keepPaths": false}));
        assert_eq!(s.subpaths.len(), 5);
        assert!((s.subpaths[0].anchors[0].p.y).abs() < 1e-9 && (s.subpaths[0].anchors[1].p.y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn join_type_outlines_and_slice_halves() {
        use redrob_vector_geom::Shape;
        let l = poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false);
        for join in ["bevel", "round", "miter", "miterClip", "extrapolated", "extrapolatedAlt", "extrapolatedSmooth"] {
            let out = join_type(&l, &json!({"width": 10, "join": join}), None);
            let bp = out.to_bezpath();
            assert!(bp.contains(Point::new(50.0, 2.0)) && bp.contains(Point::new(98.0, 50.0)), "{join}");
            assert!(!bp.contains(Point::new(50.0, 50.0)), "{join}");
        }
        let sq = poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
        let b = Rect::new(0.0, 0.0, 100.0, 100.0);
        let left = slice(&sq, b, &json!({"side": "left"}), FillRule::NonZero);
        let lb = left.bounds().unwrap();
        assert!((lb.width() - 50.0).abs() < 1e-3, "{lb:?}");
        let both = slice(&sq, b, &json!({"side": "both", "gap": 10}), FillRule::NonZero);
        assert!((both.bounds().unwrap().width() - 110.0).abs() < 1e-3);
        let open = slice(&poly(&[(0.0, 50.0), (100.0, 50.0)], false), b, &json!({"side": "right"}), FillRule::NonZero);
        assert_eq!(open.subpaths.len(), 1);
        let h = hatches(&sq, b, &json!({"frequency": 10}), FillRule::NonZero);
        assert!(h.bounds().is_some_and(|hb| hb.x0 >= -1e-3 && hb.x1 <= 100.0 + 1e-3));
        let sk = sketch(&l, &json!({}));
        assert!(sk.subpaths.len() > 2);
        let simple = simplify(&poly(&[(0.0, 0.0), (10.0, 0.01), (20.0, 0.0), (30.0, -0.01), (40.0, 0.0)], false), b, &json!({"threshold": 0.5}));
        assert!(simple.subpaths[0].anchors.len() < 5);
        let ip = interpolate_points(&l, &json!({}));
        assert_eq!(ip.subpaths[0].anchors.len(), 3);
    }
}
