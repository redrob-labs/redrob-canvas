//! WordPerfect Graphics (`.wpg`), level 1: a 16-byte prefix (`FF 'W' 'P' 'C'`, the data offset,
//! product and file type, version), then records of a type byte, a length (one byte; `FF` then two
//! bytes; with the high bit set, four bytes) and data. Read: fill and line attributes, the colour
//! map, lines, polylines, rectangles, polygons, ellipses, polycurves (Bézier) and graphics text;
//! bitmaps are left out with a warning. Coordinates are 1/1200 inch, y up.

use kurbo::BezPath;
use redrob_vector_doc::{LineCap, LineJoin};
use redrob_vector_geom::{Affine, Point, Rect, Shape};

use super::{Builder, Bytes, Imported, Stroke, polyline, rect_path};

const PT_PER_UNIT: f64 = 72.0 / 1200.0;

/// Does `bytes` carry the WPG prefix?
pub fn is_wpg(bytes: &[u8]) -> bool {
    bytes.len() >= 16 && bytes[0] == 0xff && &bytes[1..4] == b"WPC" && bytes[8] == 1 && bytes[9] == 0x16
}

/// The default 256-entry colour map (the 16 standard colours, then a spread of shades).
fn default_palette() -> Vec<[u8; 3]> {
    let mut p = vec![
        [0, 0, 0],
        [0, 0, 170],
        [0, 170, 0],
        [0, 170, 170],
        [170, 0, 0],
        [170, 0, 170],
        [170, 85, 0],
        [170, 170, 170],
        [85, 85, 85],
        [85, 85, 255],
        [85, 255, 85],
        [85, 255, 255],
        [255, 85, 85],
        [255, 85, 255],
        [255, 255, 85],
        [255, 255, 255],
    ];
    for i in 16..256u32 {
        let v = ((i - 16) * 255 / 239) as u8;
        p.push([v, v, v]);
    }
    p
}

struct Reader {
    b: Builder,
    palette: Vec<[u8; 3]>,
    fill: Option<u8>,
    line: Option<(u8, f64)>,
    /// 1/1200 inch.
    size: Option<(f64, f64)>,
    skipped: usize,
}

impl Reader {
    fn stroke(&self) -> Option<Stroke> {
        let (c, w) = self.line?;
        let rgb = self.palette.get(usize::from(c)).copied().unwrap_or([0, 0, 0]);
        Some(Stroke { rgb, width: w.max(1.0), cap: LineCap::Round, join: LineJoin::Round, dash: vec![] })
    }

    fn fill(&self) -> Option<[u8; 3]> {
        self.fill.map(|c| self.palette.get(usize::from(c)).copied().unwrap_or([0, 0, 0]))
    }

    fn shape(&mut self, bp: BezPath, closed: bool) {
        let fill = if closed { self.fill() } else { None };
        let st = self.stroke();
        self.b.path(bp, fill, st, false);
    }

    fn record(&mut self, kind: u8, data: &[u8]) {
        let mut r = Bytes::new(data);
        let pt = |r: &mut Bytes| -> Option<Point> {
            let x = r.i16le()?;
            let y = r.i16le()?;
            Some(Point::new(f64::from(x), f64::from(y)))
        };
        match kind {
            0x01 => {
                // Fill attributes: type (0 none, 1 solid, else patterns), colour index.
                let ty = r.u8().unwrap_or(0);
                let c = r.u8().unwrap_or(0);
                self.fill = (ty != 0).then_some(c);
            }
            0x02 => {
                // Line attributes: type (0 none), colour index, width.
                let ty = r.u8().unwrap_or(1);
                let c = r.u8().unwrap_or(0);
                let w = f64::from(r.u16le().unwrap_or(1));
                self.line = (ty != 0).then_some((c, w));
            }
            0x05 => {
                if let (Some(a), Some(b)) = (pt(&mut r), pt(&mut r)) {
                    self.shape(polyline(&[a, b], false), false);
                }
            }
            0x06 | 0x08 => {
                let n = usize::from(r.u16le().unwrap_or(0));
                let mut pts = vec![];
                for _ in 0..n.min(100_000) {
                    let Some(p) = pt(&mut r) else { break };
                    pts.push(p);
                }
                let closed = kind == 0x08;
                self.shape(polyline(&pts, closed), closed);
            }
            0x07 => {
                // Rectangle: x, y (lower-left), width, height, corner radii.
                if let (Some(o), Some(sz)) = (pt(&mut r), pt(&mut r)) {
                    let rx = f64::from(r.i16le().unwrap_or(0));
                    let ry = f64::from(r.i16le().unwrap_or(0));
                    let rect = Rect::new(o.x, o.y, o.x + sz.x, o.y + sz.y);
                    let bp = if rx > 0.0 && ry > 0.0 {
                        kurbo::RoundedRect::from_rect(rect, kurbo::RoundedRectRadii::new(rx.min(ry), rx.min(ry), rx.min(ry), rx.min(ry)))
                            .to_path(0.1)
                    } else {
                        rect_path(Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y1))
                    };
                    self.shape(bp, true);
                }
            }
            0x09 => {
                // Ellipse: centre, radii, rotation angle, start and end angles (degrees).
                if let (Some(c), Some(rad)) = (pt(&mut r), pt(&mut r)) {
                    let rot = f64::from(r.i16le().unwrap_or(0)).to_radians();
                    let start = f64::from(r.i16le().unwrap_or(0));
                    let end = f64::from(r.i16le().unwrap_or(0));
                    let span = (end - start).rem_euclid(360.0);
                    if span < 1e-9 || start == end {
                        self.shape(super::ellipse(c, rad.x.abs(), rad.y.abs(), rot), true);
                    } else {
                        let mut bp = BezPath::new();
                        bp.move_to(c);
                        super::arc_to(&mut bp, c, rad.x.abs(), rad.y.abs(), start.to_radians(), span.to_radians(), rot);
                        bp.close_path();
                        self.shape(bp, true);
                    }
                }
            }
            0x0b | 0x14 => self.b.warn("WPG bitmaps aren't read: they are left out"),
            0x0c => {
                // Graphics text: width, height, x, y, then attributes, then the text.
                let w = f64::from(r.u16le().unwrap_or(0));
                let h = f64::from(r.u16le().unwrap_or(0));
                let _ = w;
                if let Some(at) = pt(&mut r) {
                    let _ = r.take(20);
                    let text: String = String::from_utf8_lossy(r.take(r.left()).unwrap_or(&[])).trim_end_matches('\0').to_string();
                    let rgb = self.stroke().map(|s| s.rgb).unwrap_or([0, 0, 0]);
                    self.b.text(at, &text, if h > 0.0 { h } else { 200.0 }, rgb, 0.0);
                }
            }
            0x0e => {
                // Colour map: start index, count, RGB triples.
                let start = usize::from(r.u16le().unwrap_or(0));
                let n = usize::from(r.u16le().unwrap_or(0));
                for i in 0..n {
                    let Some(c) = r.take(3) else { break };
                    if start + i < 256 {
                        self.palette[start + i] = [c[0], c[1], c[2]];
                    }
                }
            }
            0x0f => {
                // Start of WPG data: version, flags, width, height (1/1200 inch).
                let _ = r.u8();
                let _ = r.u8();
                let w = f64::from(r.u16le().unwrap_or(0));
                let h = f64::from(r.u16le().unwrap_or(0));
                if w > 0.0 && h > 0.0 {
                    self.size = Some((w, h));
                }
            }
            0x10 => {}
            0x11 => self.b.warn("embedded PostScript isn't read: it is left out"),
            0x12 => {
                // Polycurve: a count, then for each anchor its incoming control, the anchor and
                // its outgoing control.
                let _ = r.u16le();
                let n = usize::from(r.u16le().unwrap_or(0));
                let mut triples: Vec<[Point; 3]> = vec![];
                for _ in 0..n.min(100_000) {
                    let (Some(a), Some(b), Some(c)) = (pt(&mut r), pt(&mut r), pt(&mut r)) else { break };
                    triples.push([a, b, c]);
                }
                if triples.len() >= 2 {
                    let mut bp = BezPath::new();
                    bp.move_to(triples[0][1]);
                    for i in 1..triples.len() {
                        bp.curve_to(triples[i - 1][2], triples[i][0], triples[i][1]);
                    }
                    let closed = (triples[0][1] - triples[triples.len() - 1][1]).hypot() < 1.0;
                    if closed {
                        bp.close_path();
                    }
                    self.shape(bp, closed);
                }
            }
            0x1a => {
                // Polyline type 2: a count of points as 32-bit coordinates.
                let n = r.u32le().unwrap_or(0) as usize;
                let mut pts = vec![];
                for _ in 0..n.min(100_000) {
                    let (Some(x), Some(y)) = (r.u32le(), r.u32le()) else { break };
                    pts.push(Point::new(f64::from(x as i32), f64::from(y as i32)));
                }
                self.shape(polyline(&pts, false), false);
            }
            _ => self.skipped += 1,
        }
    }
}

/// Read a WPG file into a document.
pub fn import(bytes: &[u8], title: &str) -> Result<Imported, String> {
    if !is_wpg(bytes) {
        return Err("not a WordPerfect Graphics file".into());
    }
    let major = bytes[10];
    if major >= 2 {
        return Err("WPG version 2 files (WordPerfect 6 and later) aren't read yet: save the graphic as WPG 1 or another vector format".into());
    }
    let mut r = Bytes::new(bytes);
    let offset = {
        let _ = r.take(4);
        r.u32le().unwrap_or(16) as usize
    };
    r.at = offset.max(16);
    let mut rd = Reader { b: Builder::new(), palette: default_palette(), fill: None, line: Some((0, 1.0)), size: None, skipped: 0 };
    while r.left() >= 2 {
        let Some(kind) = r.u8() else { break };
        let Some(l0) = r.u8() else { break };
        let len = if l0 == 0xff {
            let Some(l1) = r.u16le() else { break };
            if l1 & 0x8000 != 0 {
                let Some(l2) = r.u16le() else { break };
                ((usize::from(l1 & 0x7fff)) << 16) | usize::from(l2)
            } else {
                usize::from(l1)
            }
        } else {
            usize::from(l0)
        };
        let Some(data) = r.take(len) else { break };
        rd.record(kind, data);
        if kind == 0x10 {
            break;
        }
    }
    for _ in 0..rd.skipped {
        rd.b.skip();
    }
    let Some(fb) = rd.b.bounds() else {
        return Err("the graphic draws nothing Redrob Canvas reads".into());
    };
    let frame = match rd.size {
        Some((w, h)) => Rect::new(0.0, 0.0, w, h).union(fb),
        None => fb.inflate(60.0, 60.0),
    };
    let map = Affine::scale(PT_PER_UNIT) * Affine::new([1.0, 0.0, 0.0, -1.0, -frame.x0, frame.y1]);
    let board = Rect::new(0.0, 0.0, frame.width() * PT_PER_UNIT, frame.height() * PT_PER_UNIT);
    Ok(rd.b.finish(map, Some(board), title))
}
