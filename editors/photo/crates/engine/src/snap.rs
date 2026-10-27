//! Snapping (View › Snap / Snap To) and smart guides, as pure geometry.
//!
//! A [`SnapTargets`] set is built once per gesture from the document (guides, grid, document
//! bounds and centre, layer edges and centres, selection edges), then every pointer move asks it
//! to snap a point ([`SnapTargets::snap_point`]: marquee corners, crop handles, guide positions,
//! pen anchors, transform handles) or a moving rectangle ([`SnapTargets::snap_rect`]: the Move
//! tool, Free Transform drags). Distances are in document pixels; callers convert their screen
//! threshold with the zoom (`8 px / zoom`). Results carry [`SnapLine`]s so the UI can draw what
//! was snapped to (smart guides are the layer-to-layer subset of these, drawn in magenta).

use redrob_photo_doc::{Document, LayerContent, LayerId};
use serde::Serialize;

/// What a snap target came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapKind {
    Guide,
    Grid,
    DocumentEdge,
    DocumentCenter,
    LayerEdge,
    LayerCenter,
    Selection,
    /// An anchor (or the bounds) of the active path (the work path, else the active layer's
    /// shape path or vector mask).
    Path,
    /// A position that puts the moving object the same distance from a neighbour as that
    /// neighbour is from the next object (equal spacing).
    Equidistance,
    /// A path anchor as a point: both axes snap at once.
    Node,
    /// The extension of a path segment past its end, a line at any angle.
    Extension,
    /// Where two guides (or a guide and the document's edge) cross.
    Intersection,
}

impl SnapKind {
    /// Layer-to-layer and document-centre alignments are what Photoshop shows as smart guides.
    pub fn is_smart(self) -> bool {
        matches!(self, SnapKind::LayerEdge | SnapKind::LayerCenter | SnapKind::DocumentCenter | SnapKind::Equidistance)
    }
}

/// A line to snap to: `pos` on one axis (x for vertical lines), `span` its extent on the other
/// axis (for drawing alignment lines between objects).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub pos: f64,
    pub kind: SnapKind,
    pub span: (f64, f64),
}

/// A snapped alignment, for drawing: a vertical line at x = `pos` from y = `from` to `to`
/// (or horizontal at y = `pos`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SnapLine {
    pub vertical: bool,
    pub pos: f64,
    pub from: f64,
    pub to: f64,
    pub kind: SnapKind,
}

/// Which target families are active (View › Snap To).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapOptions {
    pub guides: bool,
    pub grid: bool,
    pub layers: bool,
    pub document: bool,
    pub selection: bool,
    /// The active path's anchors and bounds.
    pub path: bool,
    /// Equal spacing between layers (a moved layer lands as far from its neighbour as that
    /// neighbour is from the next).
    pub equidistance: bool,
    /// Grid subdivision spacing in document pixels, independently for x and y.
    pub grid_step: [f64; 2],
    /// Position of a gridline from the document's top-left, document pixels (an image's own grid
    /// may be offset; the Preferences grid starts at 0).
    pub grid_offset: [f64; 2],
    /// Path anchors as points (both axes at once).
    pub node: bool,
    /// The extensions of path segments past their ends.
    pub extension: bool,
    /// Guide crossings as points.
    pub intersection: bool,
}

impl Default for SnapOptions {
    fn default() -> Self {
        Self {
            guides: true,
            grid: false,
            layers: true,
            document: true,
            selection: true,
            path: false,
            equidistance: false,
            grid_step: [18.0; 2],
            grid_offset: [0.0; 2],
            node: false,
            extension: false,
            intersection: false,
        }
    }
}

/// A point to snap to on both axes at once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointTarget {
    pub pos: [f64; 2],
    pub kind: SnapKind,
}

/// A line at any angle to snap onto: `origin` and a unit `dir`; a ray when `ray` (only ahead of
/// the origin), else the whole line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: [f64; 2],
    pub dir: [f64; 2],
    pub ray: bool,
    pub kind: SnapKind,
}

impl Ray {
    /// `p` projected onto the ray, and how far it moved.
    pub fn project(&self, p: [f64; 2]) -> ([f64; 2], f64) {
        let mut t = (p[0] - self.origin[0]) * self.dir[0] + (p[1] - self.origin[1]) * self.dir[1];
        if self.ray {
            t = t.max(0.0);
        }
        let q = [self.origin[0] + t * self.dir[0], self.origin[1] + t * self.dir[1]];
        (q, (q[0] - p[0]).hypot(q[1] - p[1]))
    }
}

/// Snap targets on both axes.
#[derive(Clone, Debug, Default)]
pub struct SnapTargets {
    /// Vertical lines (x positions).
    pub x: Vec<Target>,
    /// Horizontal lines (y positions).
    pub y: Vec<Target>,
    /// Grid subdivision spacing on the x and y axes (document px).
    pub grid: Option<[f64; 2]>,
    /// A gridline's position on each axis (document px): the grid's lines are at
    /// `grid_offset + n × step`.
    pub grid_offset: [f64; 2],
    /// Extent used for grid lines and guides ([x0, y0, x1, y1]).
    pub extent: [f64; 4],
    /// Point targets (nodes, intersections): both axes at once.
    pub points: Vec<PointTarget>,
    /// Lines at any angle (segment extensions).
    pub rays: Vec<Ray>,
}

/// Bounds of a layer for snapping: what Free Transform would show (`None` for empty layers,
/// groups and maskless adjustment layers).
pub fn layer_rect(doc: &Document, id: LayerId) -> Option<[f64; 4]> {
    rect_of(doc, doc.layer(id)?)
}

/// Most layer rectangles [`SnapTargets::add_equidistance`] pairs up.
const MAX_EQUIDISTANCE_RECTS: usize = 64;

/// The active path: the work path, else the shape path or vector mask of the first visible
/// shape or masked layer not being moved (the document has no notion of an active layer here).
fn active_path<'a>(doc: &'a Document, exclude: &[LayerId]) -> Option<&'a redrob_photo_doc::Path> {
    if let Some(p) = &doc.work_path {
        return Some(p);
    }
    doc.walk().into_iter().filter(|(_, _, l)| l.visible && !exclude.contains(&l.id)).find_map(|(_, _, l)| match &l.content {
        LayerContent::Shape(sh) => Some(&sh.path),
        _ => l.vector_mask.as_ref().map(|v| &v.path),
    })
}

fn rect_of(doc: &Document, l: &redrob_photo_doc::Layer) -> Option<[f64; 4]> {
    if l.is_group() || (matches!(l.content, LayerContent::Adjustment(_)) && l.mask.is_none()) {
        return None;
    }
    let b = crate::transform_cmds::transform_bounds(doc, l);
    (!b.is_empty()).then_some([b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64])
}

impl SnapTargets {
    /// Targets of `doc` for the enabled families; layers in `exclude` (the ones being moved)
    /// are skipped.
    pub fn from_document(doc: &Document, opts: &SnapOptions, exclude: &[LayerId]) -> Self {
        let (w, h) = (doc.size.width as f64, doc.size.height as f64);
        let mut t = SnapTargets { extent: [0.0, 0.0, w, h], ..Default::default() };
        if opts.document {
            t.add_rect([0.0, 0.0, w, h], SnapKind::DocumentEdge, SnapKind::DocumentCenter);
        }
        if opts.guides {
            for g in &doc.guides.vertical {
                t.x.push(Target { pos: *g, kind: SnapKind::Guide, span: (0.0, h) });
            }
            for g in &doc.guides.horizontal {
                t.y.push(Target { pos: *g, kind: SnapKind::Guide, span: (0.0, w) });
            }
        }
        if opts.grid && opts.grid_step.iter().all(|step| step.is_finite() && *step > 0.0) {
            t.grid = Some(opts.grid_step);
            t.grid_offset = opts.grid_offset.map(|o| if o.is_finite() { o } else { 0.0 });
        }
        if opts.layers || opts.equidistance {
            let mut rects = Vec::new();
            for (_, _, l) in doc.walk() {
                if !l.visible || exclude.contains(&l.id) {
                    continue;
                }
                if let Some(r) = rect_of(doc, l) {
                    if opts.layers {
                        t.add_rect(r, SnapKind::LayerEdge, SnapKind::LayerCenter);
                    }
                    rects.push(r);
                }
            }
            if opts.equidistance {
                t.add_equidistance(&rects);
            }
        }
        if opts.path
            && let Some(path) = active_path(doc, exclude)
        {
            t.add_path(path);
        }
        if (opts.node || opts.extension)
            && let Some(path) = active_path(doc, exclude)
        {
            t.add_nodes_and_extensions(path, opts.node, opts.extension);
        }
        if opts.intersection {
            t.add_guide_intersections(doc, opts.document);
        }
        if opts.selection
            && let Some(sel) = &doc.selection
        {
            let b = sel.content_bounds();
            if !b.is_empty() {
                let r = [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64];
                t.x.push(Target { pos: r[0], kind: SnapKind::Selection, span: (r[1], r[3]) });
                t.x.push(Target { pos: r[2], kind: SnapKind::Selection, span: (r[1], r[3]) });
                t.y.push(Target { pos: r[1], kind: SnapKind::Selection, span: (r[0], r[2]) });
                t.y.push(Target { pos: r[3], kind: SnapKind::Selection, span: (r[0], r[2]) });
            }
        }
        t
    }

    /// Equal-spacing targets from every pair of layer rectangles that face each other on an axis
    /// with a gap `g`: a moving object snaps where it sits `g` past the pair's outer edges.
    /// Bounded to the first [`MAX_EQUIDISTANCE_RECTS`] rectangles (pairs grow quadratically).
    pub fn add_equidistance(&mut self, rects: &[[f64; 4]]) {
        let rects = &rects[..rects.len().min(MAX_EQUIDISTANCE_RECTS)];
        for (i, a) in rects.iter().enumerate() {
            for b in rects.iter().skip(i + 1) {
                for (lo, hi) in [(a, b), (b, a)] {
                    // Horizontal neighbours: `lo` left of `hi`, overlapping vertically.
                    let gap = hi[0] - lo[2];
                    if gap > 0.0 && lo[3] > hi[1] && hi[3] > lo[1] {
                        let span = (lo[1].min(hi[1]), lo[3].max(hi[3]));
                        self.x.push(Target { pos: hi[2] + gap, kind: SnapKind::Equidistance, span });
                        self.x.push(Target { pos: lo[0] - gap, kind: SnapKind::Equidistance, span });
                    }
                    // Vertical neighbours: `lo` above `hi`, overlapping horizontally.
                    let gap = hi[1] - lo[3];
                    if gap > 0.0 && lo[2] > hi[0] && hi[2] > lo[0] {
                        let span = (lo[0].min(hi[0]), lo[2].max(hi[2]));
                        self.y.push(Target { pos: hi[3] + gap, kind: SnapKind::Equidistance, span });
                        self.y.push(Target { pos: lo[1] - gap, kind: SnapKind::Equidistance, span });
                    }
                }
            }
        }
    }

    /// Targets on a path: a vertical and a horizontal line through every anchor, plus the bounds
    /// of each subpath (its curves' extreme control points).
    pub fn add_path(&mut self, path: &redrob_photo_doc::Path) {
        for sp in &path.subpaths {
            if sp.knots.is_empty() {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for k in &sp.knots {
                for p in [k.anchor, k.in_ctrl, k.out_ctrl] {
                    x0 = x0.min(p.x);
                    y0 = y0.min(p.y);
                    x1 = x1.max(p.x);
                    y1 = y1.max(p.y);
                }
            }
            if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
                continue;
            }
            for k in &sp.knots {
                self.x.push(Target { pos: k.anchor.x, kind: SnapKind::Path, span: (y0, y1) });
                self.y.push(Target { pos: k.anchor.y, kind: SnapKind::Path, span: (x0, x1) });
            }
            if sp.knots.len() > 1 {
                self.x.push(Target { pos: x0, kind: SnapKind::Path, span: (y0, y1) });
                self.x.push(Target { pos: x1, kind: SnapKind::Path, span: (y0, y1) });
                self.y.push(Target { pos: y0, kind: SnapKind::Path, span: (x0, x1) });
                self.y.push(Target { pos: y1, kind: SnapKind::Path, span: (x0, x1) });
            }
        }
    }

    /// Add a rectangle's edges and centre lines.
    pub fn add_rect(&mut self, r: [f64; 4], edge: SnapKind, center: SnapKind) {
        let (xs, ys) = ((r[1], r[3]), (r[0], r[2]));
        self.x.push(Target { pos: r[0], kind: edge, span: xs });
        self.x.push(Target { pos: r[2], kind: edge, span: xs });
        self.x.push(Target { pos: (r[0] + r[2]) / 2.0, kind: center, span: xs });
        self.y.push(Target { pos: r[1], kind: edge, span: ys });
        self.y.push(Target { pos: r[3], kind: edge, span: ys });
        self.y.push(Target { pos: (r[1] + r[3]) / 2.0, kind: center, span: ys });
    }

    /// Path anchors as point targets (`node`) and segment extensions as rays (`extension`): a
    /// straight segment extends past both ends; a curve extends along its end tangents.
    pub fn add_nodes_and_extensions(&mut self, path: &redrob_photo_doc::Path, node: bool, extension: bool) {
        let unit = |a: [f64; 2], b: [f64; 2]| {
            let d = [b[0] - a[0], b[1] - a[1]];
            let l = d[0].hypot(d[1]);
            (l > 1e-9).then(|| [d[0] / l, d[1] / l])
        };
        for sp in &path.subpaths {
            let n = sp.knots.len();
            for (i, k) in sp.knots.iter().enumerate() {
                let a = [k.anchor.x, k.anchor.y];
                if node && a.iter().all(|v| v.is_finite()) {
                    self.points.push(PointTarget { pos: a, kind: SnapKind::Node });
                }
                if !extension || n < 2 {
                    continue;
                }
                // The segment leaving this anchor: its direction at the anchor is the out
                // control (or the next anchor for a straight segment); extend backwards past
                // the anchor. Likewise the segment arriving here extends forwards.
                if i + 1 < n {
                    let next = sp.knots[i + 1];
                    let toward = if (k.out_ctrl.x - k.anchor.x).abs() + (k.out_ctrl.y - k.anchor.y).abs() > 1e-9 {
                        [k.out_ctrl.x, k.out_ctrl.y]
                    } else {
                        [next.anchor.x, next.anchor.y]
                    };
                    if let Some(d) = unit(toward, a) {
                        self.rays.push(Ray { origin: a, dir: d, ray: true, kind: SnapKind::Extension });
                    }
                }
                if i > 0 {
                    let prev = sp.knots[i - 1];
                    let from = if (k.in_ctrl.x - k.anchor.x).abs() + (k.in_ctrl.y - k.anchor.y).abs() > 1e-9 {
                        [k.in_ctrl.x, k.in_ctrl.y]
                    } else {
                        [prev.anchor.x, prev.anchor.y]
                    };
                    if let Some(d) = unit(from, a) {
                        self.rays.push(Ray { origin: a, dir: d, ray: true, kind: SnapKind::Extension });
                    }
                }
            }
        }
    }

    /// Where guides cross each other (and, with `edges`, the document's edges) as points.
    pub fn add_guide_intersections(&mut self, doc: &Document, edges: bool) {
        let (w, h) = (doc.size.width as f64, doc.size.height as f64);
        let mut xs: Vec<f64> = doc.guides.vertical.clone();
        let mut ys: Vec<f64> = doc.guides.horizontal.clone();
        if edges {
            xs.extend([0.0, w]);
            ys.extend([0.0, h]);
        }
        for x in &xs {
            for y in &ys {
                if x.is_finite() && y.is_finite() {
                    self.points.push(PointTarget { pos: [*x, *y], kind: SnapKind::Intersection });
                }
            }
        }
    }

    /// The point target nearest to `p` within `tol`, else the ray `p` is nearest to.
    pub fn snap_free(&self, p: [f64; 2], tol: f64) -> Option<([f64; 2], SnapKind)> {
        let point = self.points.iter().map(|t| (t, (t.pos[0] - p[0]).hypot(t.pos[1] - p[1]))).filter(|(_, d)| *d <= tol).min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((t, _)) = point {
            return Some((t.pos, t.kind));
        }
        self.rays.iter().map(|r| (r, r.project(p))).filter(|(_, (_, d))| *d <= tol).min_by(|a, b| a.1.1.total_cmp(&b.1.1)).map(|(r, (q, _))| (q, r.kind))
    }

    /// Only the targets for which `keep` holds (e.g. smart guides: layer alignments).
    pub fn filtered(&self, keep: impl Fn(SnapKind) -> bool) -> SnapTargets {
        SnapTargets {
            x: self.x.iter().copied().filter(|t| keep(t.kind)).collect(),
            y: self.y.iter().copied().filter(|t| keep(t.kind)).collect(),
            grid: self.grid.filter(|_| keep(SnapKind::Grid)),
            grid_offset: self.grid_offset,
            extent: self.extent,
            points: self.points.iter().copied().filter(|t| keep(t.kind)).collect(),
            rays: self.rays.iter().copied().filter(|r| keep(r.kind)).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty() && self.y.is_empty() && self.grid.is_none() && self.points.is_empty() && self.rays.is_empty()
    }

    /// The closest target to `v` within `tol` on one axis (`vertical` = x positions). Guides win
    /// ties over other kinds, as in Photoshop.
    pub fn nearest(&self, vertical: bool, v: f64, tol: f64) -> Option<Target> {
        let list = if vertical { &self.x } else { &self.y };
        let mut best: Option<(f64, Target)> = None;
        let mut consider = |t: Target| {
            let d = (t.pos - v).abs();
            if d > tol {
                return;
            }
            let better = match best {
                None => true,
                Some((bd, bt)) => d < bd - 1e-9 || ((d - bd).abs() <= 1e-9 && t.kind == SnapKind::Guide && bt.kind != SnapKind::Guide),
            };
            if better {
                best = Some((d, t));
            }
        };
        list.iter().copied().for_each(&mut consider);
        if let Some(steps) = self.grid {
            let axis = usize::from(!vertical);
            let (step, o) = (steps[axis], self.grid_offset[axis]);
            let g = ((v - o) / step).round() * step + o;
            let span = if vertical { (self.extent[1], self.extent[3]) } else { (self.extent[0], self.extent[2]) };
            consider(Target { pos: g, kind: SnapKind::Grid, span });
        }
        best.map(|(_, t)| t)
    }

    /// Snap a point: a node or intersection within `tol` takes both axes at once, a segment
    /// extension pulls the point onto its line; else each axis independently to its nearest
    /// target within `tol`.
    pub fn snap_point(&self, p: [f64; 2], tol: f64) -> ([f64; 2], Vec<SnapLine>) {
        if let Some((q, kind)) = self.snap_free(p, tol) {
            let lines = match kind {
                SnapKind::Extension => Vec::new(),
                _ => vec![
                    SnapLine { vertical: true, pos: q[0], from: q[1] - tol * 2.0, to: q[1] + tol * 2.0, kind },
                    SnapLine { vertical: false, pos: q[1], from: q[0] - tol * 2.0, to: q[0] + tol * 2.0, kind },
                ],
            };
            return (q, lines);
        }
        let mut out = p;
        let mut lines = Vec::new();
        if let Some(t) = self.nearest(true, p[0], tol) {
            out[0] = t.pos;
            lines.push(line(true, t, p[1], p[1]));
        }
        if let Some(t) = self.nearest(false, p[1], tol) {
            out[1] = t.pos;
            lines.push(line(false, t, out[0], out[0]));
        }
        (out, lines)
    }

    /// Snap a moving rectangle `[x0, y0, x1, y1]`: on each axis the edge or centre closest to a
    /// target decides the correction. Returns the offset to add and the alignments made (every
    /// anchor that lines up after the correction, so equal-size neighbours show both edges).
    pub fn snap_rect(&self, r: [f64; 4], tol: f64) -> ([f64; 2], Vec<SnapLine>) {
        let mut delta = [0.0, 0.0];
        let mut lines = Vec::new();
        for (axis, vertical) in [(0usize, true), (1usize, false)] {
            let (a, b) = (r[axis], r[axis + 2]);
            let anchors = [a, (a + b) / 2.0, b];
            let mut best: Option<(f64, f64)> = None; // (|d|, d)
            for v in anchors {
                if let Some(t) = self.nearest(vertical, v, tol) {
                    let d = t.pos - v;
                    if best.is_none_or(|(bd, _)| d.abs() < bd) {
                        best = Some((d.abs(), d));
                    }
                }
            }
            let Some((_, d)) = best else { continue };
            delta[axis] = d;
            let other = if vertical { (r[1], r[3]) } else { (r[0], r[2]) };
            for v in anchors {
                let moved = v + d;
                let list = if vertical { &self.x } else { &self.y };
                let hit = list.iter().copied().filter(|t| (t.pos - moved).abs() < 1e-6).min_by_key(|t| u8::from(t.kind != SnapKind::Guide));
                let hit = hit.or_else(|| {
                    self.grid
                        .filter(|steps| {
                            let (step, o) = (steps[axis], self.grid_offset[axis]);
                            (((moved - o) / step).round() * step + o - moved).abs() < 1e-6
                        })
                        .map(|_| Target { pos: moved, kind: SnapKind::Grid, span: other })
                });
                if let Some(t) = hit {
                    lines.push(line(vertical, t, other.0, other.1));
                }
            }
        }
        (delta, lines)
    }
}

/// Alignment line through target `t` covering both its span and the snapped object (`a..b`).
fn line(vertical: bool, t: Target, a: f64, b: f64) -> SnapLine {
    SnapLine { vertical, pos: t.pos, from: t.span.0.min(a).min(b), to: t.span.1.max(a).max(b), kind: t.kind }
}

/// Union of rectangles (`None` when empty).
pub fn union(rects: impl IntoIterator<Item = [f64; 4]>) -> Option<[f64; 4]> {
    rects.into_iter().reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
}

#[cfg(test)]
mod tests {
    use super::*;
    use redrob_photo_doc::{Color, ColorMode, Layer, SampleType, Size};

    fn targets() -> SnapTargets {
        let mut t = SnapTargets { extent: [0.0, 0.0, 400.0, 300.0], ..Default::default() };
        t.add_rect([0.0, 0.0, 400.0, 300.0], SnapKind::DocumentEdge, SnapKind::DocumentCenter);
        t.x.push(Target { pos: 120.0, kind: SnapKind::Guide, span: (0.0, 300.0) });
        t
    }

    #[test]
    fn point_snaps_within_threshold_only() {
        let t = targets();
        let (p, lines) = t.snap_point([123.0, 151.0], 4.0);
        assert_eq!(p, [120.0, 150.0]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].kind, SnapKind::Guide);
        assert_eq!(lines[1].kind, SnapKind::DocumentCenter);
        let (p, lines) = t.snap_point([130.0, 160.0], 4.0);
        assert_eq!(p, [130.0, 160.0]);
        assert!(lines.is_empty());
    }

    #[test]
    fn threshold_scales_with_zoom() {
        // 8 screen px: at 400% that is 2 document px, at 25% it is 32.
        let t = targets();
        assert!(t.nearest(true, 125.0, 8.0 / 4.0).is_none());
        assert_eq!(t.nearest(true, 145.0, 8.0 / 0.25).map(|t| t.pos), Some(120.0));
    }

    #[test]
    fn guides_win_ties() {
        let mut t = targets();
        t.x.push(Target { pos: 120.0, kind: SnapKind::LayerEdge, span: (0.0, 10.0) });
        assert_eq!(t.nearest(true, 121.0, 5.0).unwrap().kind, SnapKind::Guide);
    }

    #[test]
    fn grid_snaps_to_nearest_line() {
        let t = SnapTargets { grid: Some([25.0; 2]), extent: [0.0, 0.0, 100.0, 100.0], ..Default::default() };
        assert_eq!(t.snap_point([48.0, 61.0], 3.0).0, [50.0, 61.0]);
        assert_eq!(t.nearest(false, 74.0, 3.0).unwrap().kind, SnapKind::Grid);
    }

    #[test]
    fn rectangular_grid_snaps_independently_on_both_axes() {
        // A 1280×720 document, 20% major spacing and two subdivisions:
        // vertical lines every 128 px, horizontal lines every 72 px.
        let t = SnapTargets { grid: Some([128.0, 72.0]), extent: [0.0, 0.0, 1280.0, 720.0], ..Default::default() };
        assert_eq!(t.snap_point([127.0, 71.0], 2.0).0, [128.0, 72.0]);
        assert_eq!(t.nearest(true, 255.0, 2.0).map(|t| t.pos), Some(256.0));
        assert_eq!(t.nearest(false, 143.0, 2.0).map(|t| t.pos), Some(144.0));
        let (delta, lines) = t.snap_rect([126.0, 70.0, 160.0, 100.0], 3.0);
        assert_eq!(delta, [2.0, 2.0]);
        assert!(lines.iter().any(|l| l.vertical && l.pos == 128.0 && l.kind == SnapKind::Grid));
        assert!(lines.iter().any(|l| !l.vertical && l.pos == 72.0 && l.kind == SnapKind::Grid));
    }

    #[test]
    fn rect_snaps_by_closest_anchor() {
        let t = targets();
        // A 50-wide box whose right edge is 2 px left of the guide: moves right by 2.
        let (d, lines) = t.snap_rect([68.0, 10.0, 118.0, 40.0], 4.0);
        assert_eq!(d[0], 2.0);
        assert!(lines.iter().any(|l| l.vertical && l.pos == 120.0));
        // Centre near the document centre (200, 150): centres both ways.
        let (d, lines) = t.snap_rect([148.0, 102.0, 248.0, 202.0], 4.0);
        assert_eq!(d, [2.0, -2.0]);
        assert!(lines.iter().any(|l| l.kind == SnapKind::DocumentCenter && l.vertical));
        assert!(lines.iter().any(|l| l.kind == SnapKind::DocumentCenter && !l.vertical));
    }

    #[test]
    fn smart_guide_lines_span_both_objects() {
        let mut t = SnapTargets::default();
        t.add_rect([300.0, 200.0, 350.0, 260.0], SnapKind::LayerEdge, SnapKind::LayerCenter);
        // Moving box above-left whose left edge is near x = 300.
        let (d, lines) = t.snap_rect([302.0, 10.0, 330.0, 40.0], 5.0);
        assert_eq!(d[0], -2.0);
        let l = lines.iter().find(|l| l.vertical && l.pos == 300.0).unwrap();
        assert_eq!((l.from, l.to), (10.0, 260.0));
        assert!(l.kind.is_smart());
    }

    #[test]
    fn targets_from_document() {
        let mut doc = Document::with_background("s", Size::new(200, 100), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        doc.guides.vertical.push(40.0);
        let mut l = Layer::raster("box", doc.pixel_format());
        l.surface_mut().unwrap().fill_rect(redrob_photo_geom::Rect::new(10, 20, 30, 60), &[1.0, 0.0, 0.0, 1.0]);
        let id = doc.insert_above(None, l);
        let opts = SnapOptions::default();
        let t = SnapTargets::from_document(&doc, &opts, &[]);
        assert!(t.x.iter().any(|t| t.kind == SnapKind::Guide && t.pos == 40.0));
        assert!(t.x.iter().any(|t| t.kind == SnapKind::LayerEdge && t.pos == 30.0));
        assert!(t.y.iter().any(|t| t.kind == SnapKind::LayerCenter && t.pos == 40.0));
        assert!(t.x.iter().any(|t| t.kind == SnapKind::DocumentCenter && t.pos == 100.0));
        // The moving layer is excluded; guides off removes guides.
        let t = SnapTargets::from_document(&doc, &SnapOptions { guides: false, ..opts }, &[id]);
        assert!(!t.x.iter().any(|t| t.kind == SnapKind::Guide || (t.kind == SnapKind::LayerEdge && t.pos == 30.0)));
        assert_eq!(layer_rect(&doc, id), Some([10.0, 20.0, 30.0, 60.0]));
        assert_eq!(union([[0.0, 0.0, 1.0, 1.0], [2.0, -1.0, 3.0, 0.5]]), Some([0.0, -1.0, 3.0, 1.0]));
    }

    #[test]
    fn nodes_extensions_and_intersections_snap_points() {
        let mut doc = Document::with_background("s", Size::new(200, 100), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        doc.guides.vertical.push(40.0);
        doc.guides.horizontal.push(30.0);
        let mut t = SnapTargets::default();
        t.add_guide_intersections(&doc, true);
        assert!(t.points.iter().any(|p| p.pos == [40.0, 30.0] && p.kind == SnapKind::Intersection));
        assert!(t.points.iter().any(|p| p.pos == [200.0, 30.0]), "a guide meets the document's edge");
        let (q, lines) = t.snap_point([42.0, 28.0], 5.0);
        assert_eq!(q, [40.0, 30.0]);
        assert_eq!(lines.len(), 2);
        // A straight segment from (0,0) to (10,0) extends along y = 0 past both ends.
        let path = redrob_photo_doc::Path::new(vec![redrob_photo_doc::Subpath::polyline(&[(0.0, 0.0), (10.0, 0.0)])]);
        let mut t = SnapTargets::default();
        t.add_nodes_and_extensions(&path, true, true);
        assert_eq!(t.points.len(), 2);
        assert!(t.rays.iter().any(|r| r.origin == [10.0, 0.0] && (r.dir[0] - 1.0).abs() < 1e-9));
        let (q, _) = t.snap_point([50.0, 2.0], 5.0);
        assert_eq!(q, [50.0, 0.0], "pulled onto the extension");
        let (q, _) = t.snap_point([9.0, 1.0], 5.0);
        assert_eq!(q, [10.0, 0.0], "the node wins over the extension");
        let (q, _) = t.snap_point([50.0, 20.0], 5.0);
        assert_eq!(q, [50.0, 20.0], "too far from anything");
        let kept = t.filtered(|k| k == SnapKind::Node);
        assert!(kept.rays.is_empty() && kept.points.len() == 2);
    }
}
