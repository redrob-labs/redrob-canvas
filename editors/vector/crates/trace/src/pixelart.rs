//! Pixel art tracing: a low-resolution image whose every pixel is one flat colour becomes clean
//! shapes. Pixels of similar colour are joined into a similarity graph (8-connected); where both
//! diagonals of a 2 × 2 block would cross, heuristics decide which one stays, each with a weight:
//! **curves** (the diagonal that continues the longer thin curve), **sparse pixels** (the
//! diagonal of the colour that is rarer around the block) and **islands** (a diagonal whose loss
//! would leave a pixel on its own). The graph reshapes the pixel squares into cells whose corners
//! are cut or bridged at each kept diagonal; the output is those cells (`Voronoi`), the outline
//! of each connected region as a polygon (`Polygon`), or that outline smoothed into curves
//! (`BSpline`).

use std::collections::HashMap;

use kurbo::{BezPath, Point};

use crate::Raster;

/// What the tracer produces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PixelArtOutput {
    /// One cell per pixel, cut and bridged at the kept diagonals.
    Voronoi,
    /// The outline of each region of connected pixels, as straight segments.
    Polygon,
    /// That outline smoothed into a B-spline (cubic curves).
    #[default]
    BSpline,
}

impl PixelArtOutput {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "voronoi" | "cells" => Some(Self::Voronoi),
            "polygon" | "polygons" => Some(Self::Polygon),
            "bspline" | "b-spline" | "spline" | "curves" => Some(Self::BSpline),
            _ => None,
        }
    }
}

/// The tracer's parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct PixelArtParams {
    pub output: PixelArtOutput,
    /// Weight of the curves heuristic (0 turns it off; 1 is the usual).
    pub curves: f64,
    /// Weight of the islands heuristic.
    pub islands: f64,
    /// Weight of the sparse pixels heuristic.
    pub sparse: f64,
    /// How different two pixels may be (0–255, in luma) and still be one colour; chroma is held
    /// to about a seventh of it.
    pub similarity: f64,
    /// Transparent pixels (alpha below this) are background and traced as nothing.
    pub alpha_threshold: u8,
}

impl Default for PixelArtParams {
    fn default() -> Self {
        Self { output: PixelArtOutput::BSpline, curves: 1.0, islands: 1.0, sparse: 1.0, similarity: 48.0, alpha_threshold: 128 }
    }
}

/// One traced shape: its colour (straight RGBA) and its outline in image pixel space.
#[derive(Clone, Debug)]
pub struct PixelArtShape {
    pub rgba: [u8; 4],
    pub path: BezPath,
    /// The region (connected pixels) the shape belongs to, so cells can be grouped.
    pub region: usize,
}

/// The pixel image as the tracer sees it: colours and the similarity graph.
struct Graph {
    w: usize,
    h: usize,
    px: Vec<[u8; 4]>,
    /// Is pixel `i` background (transparent)?
    bg: Vec<bool>,
    /// Edges to the 8 neighbours, as a bit per direction: 0 E, 1 SE, 2 S, 3 SW, 4 W, 5 NW, 6 N, 7 NE.
    edges: Vec<u8>,
}

const DIRS: [(i64, i64); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];

fn yuv(c: [u8; 4]) -> (f64, f64, f64) {
    let (r, g, b) = (f64::from(c[0]), f64::from(c[1]), f64::from(c[2]));
    let y = 0.299 * r + 0.587 * g + 0.114 * b;
    let u = -0.14713 * r - 0.28886 * g + 0.436 * b;
    let v = 0.615 * r - 0.51499 * g - 0.10001 * b;
    (y, u, v)
}

impl Graph {
    fn new(img: &Raster, p: &PixelArtParams) -> Self {
        let (w, h) = (img.width as usize, img.height as usize);
        let mut px = Vec::with_capacity(w * h);
        let mut bg = Vec::with_capacity(w * h);
        for y in 0..img.height {
            for x in 0..img.width {
                let c = img.pixel(x, y);
                px.push(c);
                bg.push(c[3] < p.alpha_threshold);
            }
        }
        let mut g = Self { w, h, px, bg, edges: vec![0; w * h] };
        let similar = |a: [u8; 4], b: [u8; 4]| {
            let (ya, ua, va) = yuv(a);
            let (yb, ub, vb) = yuv(b);
            (ya - yb).abs() <= p.similarity && (ua - ub).abs() <= p.similarity / 7.0 && (va - vb).abs() <= p.similarity / 8.0
        };
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if g.bg[i] {
                    continue;
                }
                for (d, (dx, dy)) in DIRS.iter().enumerate() {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if !g.bg[j] && similar(g.px[i], g.px[j]) {
                        g.edges[i] |= 1 << d;
                    }
                }
            }
        }
        g.resolve(p);
        g
    }

    fn has(&self, i: usize, d: usize) -> bool {
        self.edges[i] & (1 << d) != 0
    }

    fn cut(&mut self, i: usize, d: usize) {
        self.edges[i] &= !(1 << d);
        let (x, y) = ((i % self.w) as i64, (i / self.w) as i64);
        let (nx, ny) = (x + DIRS[d].0, y + DIRS[d].1);
        if nx >= 0 && ny >= 0 && nx < self.w as i64 && ny < self.h as i64 {
            let j = ny as usize * self.w + nx as usize;
            self.edges[j] &= !(1 << ((d + 4) % 8));
        }
    }

    fn valence(&self, i: usize) -> u32 {
        self.edges[i].count_ones()
    }

    /// Where both diagonals of a 2 × 2 block connect, keep one: fully connected blocks lose both
    /// (the pixels already connect through their sides); otherwise the heuristics vote.
    fn resolve(&mut self, p: &PixelArtParams) {
        let (w, h) = (self.w, self.h);
        for y in 0..h.saturating_sub(1) {
            for x in 0..w.saturating_sub(1) {
                let a = y * w + x;
                let b = a + 1;
                let c = a + w;
                let d = c + 1;
                let ad = self.has(a, 1);
                let bc = self.has(b, 3);
                if !(ad && bc) {
                    continue;
                }
                // All four alike: the diagonals are redundant.
                if self.has(a, 0) && self.has(a, 2) && self.has(d, 4) && self.has(d, 6) {
                    self.cut(a, 1);
                    self.cut(b, 3);
                    continue;
                }
                let mut score_ad = 0.0;
                let mut score_bc = 0.0;
                // Curves: the longer chain of valence-2 nodes through the diagonal wins.
                if p.curves > 0.0 {
                    let ca = self.curve_length(a, d) as f64;
                    let cb = self.curve_length(b, c) as f64;
                    score_ad += p.curves * ca;
                    score_bc += p.curves * cb;
                }
                // Sparse pixels: the colour that is rarer in the 8 × 8 window around the block wins.
                if p.sparse > 0.0 {
                    let na = self.count_component_window(a, x as i64, y as i64) as f64;
                    let nb = self.count_component_window(b, x as i64, y as i64) as f64;
                    score_ad += p.sparse * nb;
                    score_bc += p.sparse * na;
                }
                // Islands: a diagonal whose node would otherwise be alone stays.
                if p.islands > 0.0 {
                    if self.valence(a) == 1 || self.valence(d) == 1 {
                        score_ad += 5.0 * p.islands;
                    }
                    if self.valence(b) == 1 || self.valence(c) == 1 {
                        score_bc += 5.0 * p.islands;
                    }
                }
                if score_ad >= score_bc {
                    self.cut(b, 3);
                } else {
                    self.cut(a, 1);
                }
            }
        }
    }

    /// The length of the thin curve (nodes of valence 2) running through the edge `i`–`j`.
    fn curve_length(&self, i: usize, j: usize) -> usize {
        let mut len = 1;
        for (start, from) in [(i, j), (j, i)] {
            let (mut cur, mut prev) = (start, from);
            let mut steps = 0;
            while self.valence(cur) == 2 && steps < 10_000 {
                let Some(next) = self.neighbours(cur).into_iter().find(|n| *n != prev) else { break };
                if next == start {
                    break;
                }
                len += 1;
                prev = cur;
                cur = next;
                steps += 1;
            }
        }
        len
    }

    fn neighbours(&self, i: usize) -> Vec<usize> {
        let (x, y) = ((i % self.w) as i64, (i / self.w) as i64);
        (0..8)
            .filter(|d| self.has(i, *d))
            .filter_map(|d| {
                let (nx, ny) = (x + DIRS[d].0, y + DIRS[d].1);
                (nx >= 0 && ny >= 0 && nx < self.w as i64 && ny < self.h as i64).then(|| ny as usize * self.w + nx as usize)
            })
            .collect()
    }

    /// How many pixels in the 8 × 8 window round block (`bx`, `by`) are connected (through the
    /// graph) to pixel `i`.
    fn count_component_window(&self, i: usize, bx: i64, by: i64) -> usize {
        let (x0, y0) = ((bx - 3).max(0), (by - 3).max(0));
        let (x1, y1) = ((bx + 4).min(self.w as i64 - 1), (by + 4).min(self.h as i64 - 1));
        let inside = |j: usize| {
            let (x, y) = ((j % self.w) as i64, (j / self.w) as i64);
            x >= x0 && x <= x1 && y >= y0 && y <= y1
        };
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![i];
        while let Some(j) = stack.pop() {
            if !inside(j) || !seen.insert(j) {
                continue;
            }
            stack.extend(self.neighbours(j));
        }
        seen.len()
    }

    /// Connected regions of the graph: a region index per pixel (background: none).
    fn regions(&self) -> (Vec<Option<usize>>, usize) {
        let mut region = vec![None; self.w * self.h];
        let mut count = 0;
        for start in 0..self.w * self.h {
            if self.bg[start] || region[start].is_some() {
                continue;
            }
            let mut stack = vec![start];
            while let Some(j) = stack.pop() {
                if region[j].is_some() {
                    continue;
                }
                region[j] = Some(count);
                stack.extend(self.neighbours(j));
            }
            count += 1;
        }
        (region, count)
    }

    /// The cell of pixel `i`: its square, each corner cut where the diagonal across it joins the
    /// two other pixels, and bridged (the cut taken over) where this pixel's own diagonal does.
    /// Coordinates in quarter pixels, as integers.
    fn cell(&self, i: usize) -> Vec<(i64, i64)> {
        let (x, y) = ((i % self.w) as i64, (i / self.w) as i64);
        let (x0, y0, x1, y1) = (x * 4, y * 4, x * 4 + 4, y * 4 + 4);
        let mut pts: Vec<(i64, i64)> = vec![];
        // Corners clockwise from the top-left: each decided by the 2 × 2 block it sits in.
        // Top-left corner: block with this pixel at bottom-right (D). Its own diagonal is NW (5);
        // the crossing one joins the pixel above (N) and the one to the left (W).
        let corners = [
            ((x0, y0), 5, (6, 3)), // top-left: own NW; others N going SW
            ((x1, y0), 7, (6, 1)), // top-right: own NE; others N going SE
            ((x1, y1), 1, (0, 3)), // bottom-right: own SE; others E going SW
            ((x0, y1), 3, (4, 1)), // bottom-left: own SW; others W going SE
        ];
        for (k, ((cx, cy), own, (other_dir, other_diag))) in corners.iter().enumerate() {
            let own_diag = self.has(i, *own);
            // The pixel in direction `other_dir` and whether it connects across the corner.
            let other = {
                let (nx, ny) = (x + DIRS[*other_dir].0, y + DIRS[*other_dir].1);
                (nx >= 0 && ny >= 0 && nx < self.w as i64 && ny < self.h as i64).then(|| ny as usize * self.w + nx as usize)
            };
            let cut = other.is_some_and(|o| self.has(o, *other_diag));
            // Along the two edges meeting at this corner, one quarter in.
            let (ax, ay, bx, by) = match k {
                0 => ((cx + 1, *cy), (*cx, cy + 1)), // top edge point, left edge point
                1 => ((*cx, cy + 1), (cx - 1, *cy)), // right edge point, top edge point
                2 => ((cx - 1, *cy), (*cx, cy - 1)), // bottom edge point, right edge point
                _ => ((*cx, cy - 1), (cx + 1, *cy)), // left edge point, bottom edge point
            }
            .into_flat();
            if cut && !own_diag {
                // Clockwise order: the point before the corner, then the one after.
                pts.push((bx, by));
                pts.push((ax, ay));
            } else if own_diag && !cut {
                // Bridge: the corner takes over the triangle cut from the neighbour after it
                // (clockwise), so the two diagonal cells meet through the corner.
                let (ex, ey) = match k {
                    0 => (*cx, cy - 1),
                    1 => (cx + 1, *cy),
                    2 => (*cx, cy + 1),
                    _ => (cx - 1, *cy),
                };
                pts.push((*cx, *cy));
                pts.push((ex, ey));
                pts.push((ax, ay));
            } else {
                pts.push((*cx, *cy));
            }
        }
        pts
    }
}

trait Flat {
    fn into_flat(self) -> (i64, i64, i64, i64);
}

impl Flat for ((i64, i64), (i64, i64)) {
    fn into_flat(self) -> (i64, i64, i64, i64) {
        (self.0.0, self.0.1, self.1.0, self.1.1)
    }
}

/// The cell's corner order is clockwise on screen (y down): top-left, top-right, bottom-right,
/// bottom-left; the edge points before/after each corner follow that.
fn poly_path(pts: &[(i64, i64)]) -> BezPath {
    let mut bp = BezPath::new();
    for (k, (x, y)) in pts.iter().enumerate() {
        let p = Point::new(*x as f64 / 4.0, *y as f64 / 4.0);
        if k == 0 {
            bp.move_to(p);
        } else {
            bp.line_to(p);
        }
    }
    bp.close_path();
    bp
}

/// Closed loops from the directed boundary edges of a region (an edge whose reverse is also
/// present is interior and dropped).
fn loops(edges: &[((i64, i64), (i64, i64))]) -> Vec<Vec<(i64, i64)>> {
    let mut count: HashMap<((i64, i64), (i64, i64)), i32> = HashMap::new();
    for (a, b) in edges {
        *count.entry((*a, *b)).or_insert(0) += 1;
    }
    let mut next: HashMap<(i64, i64), Vec<(i64, i64)>> = HashMap::new();
    for ((a, b), n) in &count {
        if *n > 0 && count.get(&(*b, *a)).copied().unwrap_or(0) == 0 {
            next.entry(*a).or_default().push(*b);
        }
    }
    let mut out = vec![];
    let mut starts: Vec<(i64, i64)> = next.keys().copied().collect();
    starts.sort_unstable();
    for s in starts {
        while let Some(b) = next.get_mut(&s).and_then(|v| v.pop()) {
            let mut loop_pts = vec![s];
            let mut cur = b;
            let mut guard = 0;
            while cur != s && guard < 1_000_000 {
                loop_pts.push(cur);
                let Some(n) = next.get_mut(&cur).and_then(|v| v.pop()) else { break };
                cur = n;
                guard += 1;
            }
            if loop_pts.len() >= 3 {
                out.push(loop_pts);
            }
        }
    }
    out
}

/// Drop points along straight runs.
fn simplify(pts: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let n = pts.len();
    if n < 4 {
        return pts.to_vec();
    }
    let mut out = vec![];
    for i in 0..n {
        let (p, q, r) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let cross = (q.0 - p.0) * (r.1 - q.1) - (q.1 - p.1) * (r.0 - q.0);
        if cross != 0 {
            out.push(q);
        }
    }
    if out.len() < 3 { pts.to_vec() } else { out }
}

/// A closed quadratic B-spline through the control polygon `pts` (Chaikin's corner cutting,
/// twice), as a cubic path. Corners sharper than a right angle at axis-aligned steps are kept
/// crisp, as pixel art's corners are.
fn bspline(pts: &[(i64, i64)]) -> BezPath {
    let n = pts.len();
    let mut bp = BezPath::new();
    if n < 3 {
        return poly_path(pts);
    }
    let at = |i: usize| Point::new(pts[i % n].0 as f64 / 4.0, pts[i % n].1 as f64 / 4.0);
    // Each control point is a quadratic knot between the midpoints of its two edges; a point is a
    // corner when both its edges are long axis-aligned runs (a step of the pixel grid).
    let is_corner = |i: usize| {
        let (p, q, r) = (at(i + n - 1), at(i), at(i + 1));
        let (a, b) = (q - p, r - q);
        let axis = |v: kurbo::Vec2| (v.x.abs() < 1e-9) ^ (v.y.abs() < 1e-9);
        axis(a) && axis(b) && a.hypot() >= 1.0 && b.hypot() >= 1.0
    };
    let mid = |i: usize| at(i).midpoint(at(i + 1));
    let start = if is_corner(0) { at(0) } else { mid(n - 1) };
    bp.move_to(start);
    for i in 0..n {
        let q = at(i);
        let m_next = if is_corner(i + 1) { at(i + 1) } else { mid(i) };
        if is_corner(i) {
            // A crisp corner: straight to the next knot.
            bp.line_to(m_next);
        } else {
            // Quadratic from the previous knot through control `q` to the next knot, as a cubic.
            let prev = current(&bp);
            let c1 = prev + (q - prev) * (2.0 / 3.0);
            let c2 = m_next + (q - m_next) * (2.0 / 3.0);
            bp.curve_to(c1, c2, m_next);
        }
    }
    bp.close_path();
    bp
}

fn current(bp: &BezPath) -> Point {
    match bp.elements().last() {
        Some(kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) | kurbo::PathEl::QuadTo(_, p) | kurbo::PathEl::CurveTo(_, _, p)) => *p,
        _ => Point::ZERO,
    }
}

/// Trace pixel art `img` into shapes (image pixel space: one unit per pixel).
pub fn trace_pixel_art(img: &Raster, p: &PixelArtParams) -> Vec<PixelArtShape> {
    if img.width == 0 || img.height == 0 || (img.width as u64) * (img.height as u64) > 4_000_000 {
        return vec![];
    }
    let g = Graph::new(img, p);
    let (region, count) = g.regions();
    let mut out = vec![];
    match p.output {
        PixelArtOutput::Voronoi => {
            for (i, r) in region.iter().enumerate().take(g.w * g.h) {
                let Some(r) = *r else { continue };
                out.push(PixelArtShape { rgba: g.px[i], path: poly_path(&g.cell(i)), region: r });
            }
        }
        PixelArtOutput::Polygon | PixelArtOutput::BSpline => {
            // Each region's boundary from its cells' edges, coloured by the region's average.
            let mut edges: Vec<Vec<((i64, i64), (i64, i64))>> = vec![vec![]; count];
            let mut sums: Vec<([u64; 4], u64)> = vec![([0; 4], 0); count];
            for (i, r) in region.iter().enumerate().take(g.w * g.h) {
                let Some(r) = *r else { continue };
                let cell = g.cell(i);
                for k in 0..cell.len() {
                    // Axis-aligned edges in quarter steps, so a long edge cancels against the
                    // shorter ones a neighbour's cut splits it into.
                    let (a, b) = (cell[k], cell[(k + 1) % cell.len()]);
                    if a.0 == b.0 || a.1 == b.1 {
                        let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs());
                        let (sx, sy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
                        for t in 0..steps {
                            edges[r].push(((a.0 + sx * t, a.1 + sy * t), (a.0 + sx * (t + 1), a.1 + sy * (t + 1))));
                        }
                    } else {
                        edges[r].push((a, b));
                    }
                }
                for c in 0..4 {
                    sums[r].0[c] += u64::from(g.px[i][c]);
                }
                sums[r].1 += 1;
            }
            for (r, es) in edges.iter().enumerate() {
                let (s, n) = sums[r];
                let n = n.max(1);
                let rgba = [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8, (s[3] / n) as u8];
                let mut path = BezPath::new();
                for lp in loops(es) {
                    let lp = simplify(&lp);
                    let part = if p.output == PixelArtOutput::BSpline { bspline(&lp) } else { poly_path(&lp) };
                    path.extend(part.elements().iter().copied());
                }
                if !path.elements().is_empty() {
                    out.push(PixelArtShape { rgba, path, region: r });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diagonal_line_keeps_its_diagonal_and_cuts_the_crossing_one() {
        // A black diagonal on white: the black pixels connect; the white corners are cut.
        let img = Raster::from_fn(4, 4, |x, y| if x == y { [0, 0, 0, 255] } else { [255, 255, 255, 255] });
        let g = Graph::new(&img, &PixelArtParams::default());
        assert!(g.has(0, 1), "black (0,0) joins black (1,1)");
        assert!(!g.has(1, 3), "white (1,0) no longer crosses to white (0,1)");
        let shapes = trace_pixel_art(&img, &PixelArtParams { output: PixelArtOutput::Polygon, ..Default::default() });
        // Three regions: the black line and the two white triangles.
        assert_eq!(shapes.len(), 3, "{shapes:?}");
        let cells = trace_pixel_art(&img, &PixelArtParams { output: PixelArtOutput::Voronoi, ..Default::default() });
        assert_eq!(cells.len(), 16);
    }
}
