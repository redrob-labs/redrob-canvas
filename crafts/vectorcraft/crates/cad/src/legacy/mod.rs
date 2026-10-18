//! Legacy and interchange vector formats read into a document: HPGL plotter files ([`hpgl`]),
//! binary and clear-text Computer Graphics Metafiles ([`cgm`]), WordPerfect Graphics ([`wpg`]),
//! XFig drawings ([`fig`]) and CorelDRAW's RIFF files ([`cdr`], as their preview bitmap). Each
//! reader is written from the public description of its format and shares [`Builder`]: items
//! are gathered in the file's own space, then mapped into points (y down) and placed on one
//! layer of one artboard.

pub mod cdr;
pub mod cgm;
pub mod fig;
pub mod hpgl;
pub mod wpg;

use std::sync::Arc;

use kurbo::BezPath;
use redrob_vector_color::{Color, Paint};
use redrob_vector_doc::{
    Appearance, AppearanceItem, CharStyle, Dash, Document, FillLayer, ImageBlob, ImageObject, LayerColor, LineCap, LineJoin, Node, NodeId, NodeKind,
    StrokeLayer, TextObject,
};
use redrob_vector_geom::{Affine, FillRule, PathData, Point, Rect, Shape};

pub use crate::import::Imported;

/// The most objects an import makes.
const MAX_NODES: usize = 500_000;
/// How far from the origin art may lie (points).
const MAX_EXTENT: f64 = 4.0e6;

/// A stroke as a file describes it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stroke {
    pub rgb: [u8; 3],
    /// File units.
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    /// Dash, gap… in file units.
    pub dash: Vec<f64>,
}

impl Stroke {
    pub fn new(rgb: [u8; 3], width: f64) -> Self {
        Self { rgb, width, cap: LineCap::Butt, join: LineJoin::Miter, dash: vec![] }
    }
}

/// One item in file space.
enum Item {
    Path { bp: BezPath, fill: Option<[u8; 3]>, stroke: Option<Stroke>, even_odd: bool },
    Text { at: Point, text: String, size: f64, rgb: [u8; 3], angle: f64 },
    Image { png: Vec<u8>, width: u32, height: u32, xf: Affine },
}

/// Gathers a file's items in its own coordinate space; [`Builder::finish`] maps them into points.
pub(crate) struct Builder {
    items: Vec<Item>,
    warnings: Vec<String>,
    skipped: usize,
}

impl Builder {
    pub fn new() -> Self {
        Self { items: vec![], warnings: vec![], skipped: 0 }
    }

    pub fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    /// A record of a kind the reader doesn't draw.
    pub fn skip(&mut self) {
        self.skipped += 1;
    }

    pub fn path(&mut self, bp: BezPath, fill: Option<[u8; 3]>, stroke: Option<Stroke>, even_odd: bool) {
        if bp.elements().len() < 2 || (fill.is_none() && stroke.is_none()) || self.items.len() >= MAX_NODES {
            return;
        }
        self.items.push(Item::Path { bp, fill, stroke, even_odd });
    }

    /// Text at `at` (its baseline start), `size` in file units, turned by `angle` radians
    /// (counterclockwise as seen on screen).
    pub fn text(&mut self, at: Point, text: &str, size: f64, rgb: [u8; 3], angle: f64) {
        if text.trim().is_empty() || self.items.len() >= MAX_NODES {
            return;
        }
        self.items.push(Item::Text { at, text: text.to_string(), size, rgb, angle });
    }

    /// A PNG placed by `xf` (pixel space → file space).
    pub fn image(&mut self, png: Vec<u8>, width: u32, height: u32, xf: Affine) {
        if width == 0 || height == 0 || self.items.len() >= MAX_NODES {
            return;
        }
        self.items.push(Item::Image { png, width, height, xf });
    }

    /// The bounds of everything gathered, in file space.
    pub fn bounds(&self) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        let mut add = |r: Rect| out = Some(out.map_or(r, |o| o.union(r)));
        for it in &self.items {
            match it {
                Item::Path { bp, .. } => add(bp.bounding_box()),
                Item::Text { at, size, text, .. } => add(Rect::new(at.x, at.y - size, at.x + size * 0.6 * text.chars().count() as f64, at.y)),
                Item::Image { width, height, xf, .. } => add(xf.transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(*width), f64::from(*height)))),
            }
        }
        out.filter(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()))
    }

    /// The document: every item mapped by `map` (file space → points, y down) on one layer, the
    /// artboard `board` (points; `None`: the art's bounds with a margin) named `title`.
    pub fn finish(mut self, map: Affine, board: Option<Rect>, title: &str) -> Imported {
        let scale = map.determinant().abs().sqrt().max(1e-9);
        let mut nodes: Vec<Arc<Node>> = vec![];
        let items = std::mem::take(&mut self.items);
        let mut far = false;
        let mut doc = Document::new(1.0, 1.0);
        for it in items {
            let node = match it {
                Item::Path { bp, fill, stroke, even_odd } => {
                    let bp = map * bp;
                    let mut items = vec![];
                    if let Some(c) = fill {
                        items.push(AppearanceItem::Fill(FillLayer::new(solid(c))));
                    }
                    if let Some(st) = stroke {
                        let mut s = StrokeLayer::new(solid(st.rgb), (st.width * scale).max(0.1));
                        s.cap = st.cap;
                        s.join = st.join;
                        let pattern: Vec<f64> = st.dash.iter().map(|d| (d * scale).max(0.01)).collect();
                        if pattern.len() >= 2 {
                            s.dash = Some(Dash { pattern, offset: 0.0, align_corners: false });
                        }
                        items.push(AppearanceItem::Stroke(s));
                    }
                    let mut n = Node::path(NodeId(0), PathData::from_bezpath(&bp), Appearance { items, ..Appearance::default() });
                    if let NodeKind::Path { rule, .. } = &mut n.kind {
                        *rule = if even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
                    }
                    n
                }
                Item::Text { at, text, size, rgb, angle } => {
                    let size = (size * scale).clamp(0.5, 5000.0);
                    let style = CharStyle { size, fill: solid(rgb), stroke: Paint::None, ..CharStyle::default() };
                    let mut t = TextObject::point(Point::ZERO, &text, style);
                    // Counterclockwise on screen is a negative turn in document space (y down).
                    t.xf = Affine::translate((map * at).to_vec2()) * Affine::rotate(-angle);
                    t.cached_bounds = Some(redrob_vector_text::layout(redrob_vector_text::FontDb::global(), &t).bounds);
                    Node::new(NodeId(0), NodeKind::Text(Box::new(t)))
                }
                Item::Image { png, width, height, xf } => {
                    let blob = ImageBlob::new("image/png", png);
                    let key = blob.content_key();
                    doc.images.entry(key.clone()).or_insert(blob);
                    let xf = map * xf;
                    if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
                        continue;
                    }
                    Node::new(NodeId(0), NodeKind::Image(Box::new(ImageObject { key, width, height, xf, link: None, placement: Default::default() })))
                }
            };
            if let Some(b) = node.visual_bounds()
                && !(b.x0.abs() < MAX_EXTENT && b.y0.abs() < MAX_EXTENT && b.x1.abs() < MAX_EXTENT && b.y1.abs() < MAX_EXTENT)
            {
                far = true;
                continue;
            }
            let mut node = node;
            node.id = doc.alloc_id();
            nodes.push(Arc::new(node));
        }
        if far {
            self.warn("objects too far from the origin to draw are left out");
        }
        if self.skipped > 0 {
            let n = self.skipped;
            self.warnings.push(format!(
                "{n} record{} of kinds Redrob Canvas doesn't read {} skipped",
                if n == 1 { "" } else { "s" },
                if n == 1 { "was" } else { "were" }
            ));
        }
        let mut layer = Node::layer(doc.alloc_id(), "Layer 1", LayerColor::Preset(0));
        if let Some(c) = layer.children_mut() {
            *c = nodes;
        }
        doc.layers = vec![Arc::new(layer)];
        let board = board.or_else(|| doc.layers.iter().filter_map(|l| l.visual_bounds()).reduce(|a, b| a.union(b)).map(|b| b.inflate(8.0, 8.0)));
        let board = board.filter(|b| b.width() > 0.0 && b.height() > 0.0 && [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite())).unwrap_or(Rect::new(0.0, 0.0, 612.0, 792.0));
        if let Some(a) = doc.artboards.first_mut() {
            a.rect = board;
        }
        doc.title = title.to_string();
        let mut warnings = self.warnings;
        doc.limit_depth_noting(&mut warnings);
        Imported { document: doc, warnings }
    }
}

fn solid(c: [u8; 3]) -> Paint {
    Paint::solid(Color::rgb8(c[0], c[1], c[2]))
}

/// Little-endian and big-endian readers over a byte slice, bounds-checked.
pub(crate) struct Bytes<'a> {
    pub data: &'a [u8],
    pub at: usize,
}

impl<'a> Bytes<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
    pub fn left(&self) -> usize {
        self.data.len().saturating_sub(self.at)
    }
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.data.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    pub fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    pub fn u16le(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn i16le(&mut self) -> Option<i16> {
        self.u16le().map(|v| v as i16)
    }
    pub fn u32le(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn u16be(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    pub fn i16be(&mut self) -> Option<i16> {
        self.u16be().map(|v| v as i16)
    }
    pub fn u32be(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn i32be(&mut self) -> Option<i32> {
        self.u32be().map(|v| v as i32)
    }
}

/// An elliptical arc as cubics: centre `c`, radii `rx`, `ry`, from `start` sweeping `sweep`
/// radians (positive counterclockwise in a y-up space), appended to `bp` (a line to the arc's
/// start when `bp` already has a point, else a move).
pub(crate) fn arc_to(bp: &mut BezPath, c: Point, rx: f64, ry: f64, start: f64, sweep: f64, rotation: f64) {
    if !(rx.is_finite() && ry.is_finite() && start.is_finite() && sweep.is_finite()) || rx <= 0.0 || ry <= 0.0 {
        return;
    }
    let arc = kurbo::Arc::new(c, (rx, ry), start, sweep, rotation);
    let (sa, ca) = start.sin_cos();
    let (sr, cr) = rotation.sin_cos();
    let v = redrob_vector_geom::Vec2::new(rx * ca, ry * sa);
    let p0 = c + redrob_vector_geom::Vec2::new(v.x * cr - v.y * sr, v.x * sr + v.y * cr);
    if bp.elements().is_empty() {
        bp.move_to(p0);
    } else {
        bp.line_to(p0);
    }
    for el in arc.append_iter(0.05) {
        bp.push(el);
    }
}

/// An ellipse as a closed path.
pub(crate) fn ellipse(c: Point, rx: f64, ry: f64, rotation: f64) -> BezPath {
    let mut bp = BezPath::new();
    arc_to(&mut bp, c, rx, ry, 0.0, std::f64::consts::TAU, rotation);
    bp.close_path();
    bp
}

/// A polygon or polyline through `pts`.
pub(crate) fn polyline(pts: &[Point], close: bool) -> BezPath {
    let mut bp = BezPath::new();
    for (i, p) in pts.iter().enumerate() {
        if i == 0 {
            bp.move_to(*p);
        } else {
            bp.line_to(*p);
        }
    }
    if close && pts.len() > 2 {
        bp.close_path();
    }
    bp
}

/// A rectangle from two corners.
pub(crate) fn rect_path(a: Point, b: Point) -> BezPath {
    polyline(&[a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)], true)
}

/// Is `bytes` one of the formats this module reads? → the format id.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if cdr::is_cdr(bytes) {
        return Some("cdr");
    }
    if wpg::is_wpg(bytes) {
        return Some("wpg");
    }
    if fig::is_fig(bytes) {
        return Some("fig");
    }
    if cgm::is_cgm(bytes) {
        return Some("cgm");
    }
    if hpgl::is_hpgl(bytes) {
        return Some("hpgl");
    }
    None
}

/// Read a file of format `id` (`cdr`, `cgm`, `fig`, `hpgl`, `wpg`) named `title`.
pub fn import(id: &str, bytes: &[u8], title: &str) -> Result<Imported, String> {
    match id {
        "cdr" => cdr::import(bytes, title),
        "cgm" => cgm::import(bytes, title),
        "fig" => fig::import(bytes, title),
        "hpgl" => hpgl::import(bytes, title),
        "wpg" => wpg::import(bytes, title),
        _ => Err(format!("{id} isn't a format this reader knows")),
    }
}
