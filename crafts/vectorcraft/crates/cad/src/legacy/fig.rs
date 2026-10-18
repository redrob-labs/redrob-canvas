//! XFig drawings (`.fig`, format 3.2): a text header (orientation, justification, units, paper,
//! magnification, pages, transparent colour, resolution and coordinate system), then objects:
//! colour definitions (0), ellipses (1), polylines, boxes, polygons, arc-boxes and pictures (2),
//! splines (3), text (4), arcs (5) and compounds (6). Coordinates are in `resolution` units per
//! inch (1200 by default), y down; line thickness is in 1/80 inch; fonts in points.

use kurbo::BezPath;
use redrob_vector_doc::{LineCap, LineJoin};
use redrob_vector_geom::{Affine, Point, Rect};

use super::{Builder, Imported, Stroke, polyline};

/// Does `bytes` start with the FIG header?
pub fn is_fig(bytes: &[u8]) -> bool {
    bytes.starts_with(b"#FIG")
}

/// The 32 standard colours.
const COLORS: [[u8; 3]; 32] = [
    [0, 0, 0],
    [0, 0, 255],
    [0, 255, 0],
    [0, 255, 255],
    [255, 0, 0],
    [255, 0, 255],
    [255, 255, 0],
    [255, 255, 255],
    [0, 0, 144],
    [0, 0, 176],
    [0, 0, 208],
    [135, 207, 255],
    [0, 144, 0],
    [0, 176, 0],
    [0, 208, 0],
    [0, 144, 144],
    [0, 176, 176],
    [0, 208, 208],
    [144, 0, 0],
    [176, 0, 0],
    [208, 0, 0],
    [144, 0, 144],
    [176, 0, 176],
    [208, 0, 208],
    [128, 48, 0],
    [160, 64, 0],
    [192, 96, 0],
    [255, 128, 128],
    [255, 160, 160],
    [255, 192, 192],
    [255, 224, 224],
    [255, 215, 0],
];

struct Fig {
    b: Builder,
    user_colors: Vec<(i64, [u8; 3])>,
    /// Units per inch.
    res: f64,
    skipped: usize,
}

impl Fig {
    fn color(&self, i: i64) -> [u8; 3] {
        if i < 0 {
            return [0, 0, 0];
        }
        if let Some(c) = COLORS.get(i as usize) {
            return *c;
        }
        self.user_colors.iter().find(|(n, _)| *n == i).map(|(_, c)| *c).unwrap_or([0, 0, 0])
    }

    /// The fill of `area_fill` in `fill_color`: shades of black for black, shades then tints for
    /// other colours; patterns as the colour.
    fn fill(&self, fill_color: i64, area_fill: i64) -> Option<[u8; 3]> {
        if area_fill < 0 {
            return None;
        }
        let c = self.color(fill_color);
        let mix = |a: [u8; 3], b: [u8; 3], t: f64| -> [u8; 3] {
            let m = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round().clamp(0.0, 255.0) as u8;
            [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
        };
        let is_black = fill_color <= 0;
        Some(match area_fill {
            0..=20 if is_black => mix([255, 255, 255], [0, 0, 0], area_fill as f64 / 20.0),
            0..=20 => mix([0, 0, 0], c, area_fill as f64 / 20.0),
            21..=40 => mix(c, [255, 255, 255], (area_fill - 20) as f64 / 20.0),
            _ => c,
        })
    }

    /// A stroke of `thickness` (1/80 inch) in `pen_color`, dashed by `line_style`.
    fn stroke(&self, thickness: i64, pen_color: i64, line_style: i64, style_val: f64, cap: i64, join: i64) -> Option<Stroke> {
        if thickness <= 0 {
            return None;
        }
        let w = thickness as f64 / 80.0 * self.res;
        let unit = if style_val > 0.0 { style_val / 80.0 * self.res } else { w * 3.0 };
        let dash = match line_style {
            1 => vec![unit, unit],
            2 => vec![w.max(1.0), unit],
            3 => vec![unit, unit, w.max(1.0), unit],
            4 => vec![unit, unit, w.max(1.0), unit, w.max(1.0), unit],
            5 => vec![unit, unit, w.max(1.0), unit, w.max(1.0), unit, w.max(1.0), unit],
            _ => vec![],
        };
        Some(Stroke {
            rgb: self.color(pen_color),
            width: w,
            cap: match cap {
                1 => LineCap::Round,
                2 => LineCap::Square,
                _ => LineCap::Butt,
            },
            join: match join {
                1 => LineJoin::Round,
                2 => LineJoin::Bevel,
                _ => LineJoin::Miter,
            },
            dash,
        })
    }
}

fn nums(line: &str) -> Vec<f64> {
    line.split_whitespace().filter_map(|t| t.parse::<f64>().ok()).collect()
}

fn n(v: &[f64], i: usize) -> f64 {
    v.get(i).copied().unwrap_or(0.0)
}

fn ni(v: &[f64], i: usize) -> i64 {
    n(v, i) as i64
}

/// Read points, two numbers each, from the lines that follow until `count` are read.
fn read_points<'a>(lines: &mut impl Iterator<Item = &'a str>, count: usize) -> Vec<Point> {
    let mut pts = vec![];
    while pts.len() < count {
        let Some(l) = lines.next() else { break };
        let v = nums(l);
        for pair in v.chunks(2) {
            if pair.len() == 2 {
                pts.push(Point::new(pair[0], pair[1]));
            }
        }
        if v.is_empty() {
            break;
        }
    }
    pts
}

/// A smooth curve through `pts` (Catmull-Rom tangents), closed or open.
fn smooth_through(pts: &[Point], closed: bool) -> BezPath {
    let mut bp = BezPath::new();
    let k = pts.len();
    if k < 2 {
        return bp;
    }
    let at = |i: isize| -> Point { if closed { pts[i.rem_euclid(k as isize) as usize] } else { pts[i.clamp(0, k as isize - 1) as usize] } };
    bp.move_to(pts[0]);
    let segs = if closed { k } else { k - 1 };
    for i in 0..segs as isize {
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let c1 = p1 + (p2 - p0) / 6.0;
        let c2 = p2 - (p3 - p1) / 6.0;
        bp.curve_to(c1, c2, p2);
    }
    if closed {
        bp.close_path();
    }
    bp
}

/// A quadratic B-spline approximating `pts` (XFig's "approximated" splines), as cubics.
fn approximated(pts: &[Point], closed: bool) -> BezPath {
    let mut bp = BezPath::new();
    let k = pts.len();
    if k < 3 {
        return polyline(pts, closed);
    }
    let at = |i: isize| -> Point { if closed { pts[i.rem_euclid(k as isize) as usize] } else { pts[i.clamp(0, k as isize - 1) as usize] } };
    let mid = |a: Point, b: Point| a.midpoint(b);
    if closed {
        bp.move_to(mid(at(0), at(1)));
        for i in 1..=k as isize {
            let (p, q) = (at(i), at(i + 1));
            let end = mid(p, q);
            let last = current_point(&bp);
            bp.curve_to(last + (p - last) * (2.0 / 3.0), end + (p - end) * (2.0 / 3.0), end);
        }
        bp.close_path();
    } else {
        bp.move_to(pts[0]);
        for i in 1..k as isize - 1 {
            let p = at(i);
            let end = if i == k as isize - 2 { at(i + 1) } else { mid(p, at(i + 1)) };
            let last = current_point(&bp);
            bp.curve_to(last + (p - last) * (2.0 / 3.0), end + (p - end) * (2.0 / 3.0), end);
        }
    }
    bp
}

fn current_point(bp: &BezPath) -> Point {
    match bp.elements().last() {
        Some(kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) | kurbo::PathEl::QuadTo(_, p) | kurbo::PathEl::CurveTo(_, _, p)) => *p,
        _ => Point::ZERO,
    }
}

/// Read an XFig file into a document.
pub fn import(bytes: &[u8], title: &str) -> Result<Imported, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines().filter(|l| !l.trim_start().starts_with('#') || l.starts_with("#FIG"));
    let Some(header) = lines.next() else { return Err("the file is empty".into()) };
    if !header.starts_with("#FIG") {
        return Err("not an XFig file".into());
    }
    let version: f64 = header.trim_start_matches("#FIG").trim().split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(3.2);
    let mut fig = Fig { b: Builder::new(), user_colors: vec![], res: 1200.0, skipped: 0 };
    // Header lines: orientation, justification, units, papersize, magnification, multiple-page,
    // transparent colour (3.2), then resolution and coordinate system.
    let header_lines = if version >= 3.2 { 7 } else { 3 };
    for _ in 0..header_lines {
        let _ = lines.next();
    }
    let mut y_down = true;
    if let Some(l) = lines.next() {
        let v = nums(l);
        if n(&v, 0) > 0.0 {
            fig.res = n(&v, 0);
        }
        y_down = ni(&v, 1) != 1;
    }
    // Objects, deepest first: gather with depths, then draw in order.
    let mut items: Vec<(i64, Item)> = vec![];
    let mut lines = lines.peekable();
    while let Some(l) = lines.next() {
        let v = nums(l);
        if v.is_empty() {
            continue;
        }
        match ni(&v, 0) {
            0 => {
                // `0 num #rrggbb`
                let mut parts = l.split_whitespace();
                let _ = parts.next();
                if let (Some(num), Some(hex)) = (parts.next().and_then(|s| s.parse::<i64>().ok()), parts.next()) {
                    let h = hex.trim_start_matches('#');
                    if h.len() == 6
                        && let Ok(rgb) = u32::from_str_radix(h, 16)
                    {
                        fig.user_colors.push((num, [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]));
                    }
                }
            }
            1 => {
                // Ellipse: sub line_style thickness pen fill depth pen_style area_fill style_val direction angle cx cy rx ry sx sy ex ey
                let depth = ni(&v, 6);
                let c = Point::new(n(&v, 12), n(&v, 13));
                let (rx, ry) = (n(&v, 14).abs(), n(&v, 15).abs());
                let angle = n(&v, 11);
                let bp = super::ellipse(c, rx, ry, if y_down { -angle } else { angle });
                let st = fig.stroke(ni(&v, 3), ni(&v, 4), ni(&v, 2), n(&v, 9), 0, 0);
                items.push((depth, Item::Path(bp, fig.fill(ni(&v, 5), ni(&v, 8)), st, false)));
            }
            2 => {
                // Polyline: sub line_style thickness pen fill depth pen_style area_fill style_val join cap radius fwd bwd npoints
                let sub = ni(&v, 1);
                let depth = ni(&v, 6);
                let npoints = ni(&v, 15).max(0) as usize;
                if ni(&v, 13) != 0 {
                    let _ = lines.next();
                }
                if ni(&v, 14) != 0 {
                    let _ = lines.next();
                }
                if sub == 5 {
                    // The picture line: flipped flag and file name.
                    let _ = lines.next();
                }
                let pts = read_points(&mut lines, npoints);
                if sub == 5 {
                    fig.b.warn("pictures (imported image files) aren't read: their frames are drawn");
                    items.push((depth, Item::Path(polyline(&pts, true), None, Some(Stroke::new([128, 128, 128], fig.res / 80.0)), false)));
                    continue;
                }
                let closed = matches!(sub, 2 | 3 | 4);
                let mut pts = pts;
                if closed && pts.len() > 1 && pts.first() == pts.last() {
                    pts.pop();
                }
                let bp = if sub == 4 && pts.len() == 4 {
                    // Arc-box: a rounded rectangle of `radius` (1/80 inch).
                    let r = Rect::from_points(pts[0], pts[2]);
                    let rad = (n(&v, 12) / 80.0 * fig.res).min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
                    kurbo::Shape::to_path(&kurbo::RoundedRect::from_rect(r, rad), 0.1)
                } else {
                    polyline(&pts, closed)
                };
                let st = fig.stroke(ni(&v, 3), ni(&v, 4), ni(&v, 2), n(&v, 9), ni(&v, 11), ni(&v, 10));
                let fill = if closed { fig.fill(ni(&v, 5), ni(&v, 8)) } else { None };
                items.push((depth, Item::Path(bp, fill, st, false)));
            }
            3 => {
                // Spline: sub line_style thickness pen fill depth pen_style area_fill style_val cap fwd bwd npoints
                let sub = ni(&v, 1);
                let depth = ni(&v, 6);
                let npoints = ni(&v, 13).max(0) as usize;
                if ni(&v, 11) != 0 {
                    let _ = lines.next();
                }
                if ni(&v, 12) != 0 {
                    let _ = lines.next();
                }
                let pts = read_points(&mut lines, npoints);
                // The shape factors line (3.2): one per point.
                if version >= 3.2 {
                    let mut got = 0;
                    while got < npoints {
                        let Some(l) = lines.next() else { break };
                        let k = nums(l).len();
                        if k == 0 {
                            break;
                        }
                        got += k;
                    }
                }
                let closed = sub % 2 == 1;
                let bp = if sub <= 1 { approximated(&pts, closed) } else { smooth_through(&pts, closed) };
                let st = fig.stroke(ni(&v, 3), ni(&v, 4), ni(&v, 2), n(&v, 9), ni(&v, 10), 0);
                let fill = if closed { fig.fill(ni(&v, 5), ni(&v, 8)) } else { None };
                items.push((depth, Item::Path(bp, fill, st, false)));
            }
            4 => {
                // Text: sub color depth pen_style font font_size angle font_flags height length x y string\001
                let depth = ni(&v, 3);
                let size_pt = n(&v, 6);
                let angle = n(&v, 7);
                let x = n(&v, 11);
                let y = n(&v, 12);
                // The string: after the 13th field, up to `\001`.
                let mut fields = l.splitn(14, ' ');
                let s = fields.nth(13).unwrap_or("").to_string();
                let s = s.split("\\001").next().unwrap_or("").to_string();
                let s = unescape(&s);
                let width = n(&v, 10);
                let sub = ni(&v, 1);
                let x = match sub {
                    1 => x - width / 2.0,
                    2 => x - width,
                    _ => x,
                };
                items.push((depth, Item::Text(Point::new(x, y), s, size_pt / 72.0 * fig.res, fig.color(ni(&v, 2)), angle)));
            }
            5 => {
                // Arc: sub line_style thickness pen fill depth pen_style area_fill style_val cap direction fwd bwd cx cy x1 y1 x2 y2 x3 y3
                let sub = ni(&v, 1);
                let depth = ni(&v, 6);
                if ni(&v, 12) != 0 {
                    let _ = lines.next();
                }
                if ni(&v, 13) != 0 {
                    let _ = lines.next();
                }
                let c = Point::new(n(&v, 14), n(&v, 15));
                let (p1, p3) = (Point::new(n(&v, 16), n(&v, 17)), Point::new(n(&v, 20), n(&v, 21)));
                let r = (p1 - c).hypot();
                let (a1, a3) = ((p1 - c).atan2(), (p3 - c).atan2());
                // direction: 1 counterclockwise (in the file's y-down space it reads clockwise on screen).
                let ccw = ni(&v, 11) == 1;
                let sweep = if ccw != y_down { (a3 - a1).rem_euclid(std::f64::consts::TAU) } else { -((a1 - a3).rem_euclid(std::f64::consts::TAU)) };
                let mut bp = BezPath::new();
                if sub == 2 {
                    bp.move_to(c);
                }
                super::arc_to(&mut bp, c, r, r, a1, sweep, 0.0);
                let closed = sub == 2;
                if closed {
                    bp.close_path();
                }
                let st = fig.stroke(ni(&v, 3), ni(&v, 4), ni(&v, 2), n(&v, 9), ni(&v, 10), 0);
                let fill = if closed { fig.fill(ni(&v, 5), ni(&v, 8)) } else { None };
                items.push((depth, Item::Path(bp, fill, st, false)));
            }
            6 | -6 => {}
            _ => fig.skipped += 1,
        }
    }
    // Deeper objects are behind: draw them first.
    items.sort_by_key(|(d, _)| -*d);
    for (_, it) in items {
        match it {
            Item::Path(bp, fill, st, eo) => fig.b.path(bp, fill, st, eo),
            Item::Text(at, s, size, rgb, angle) => fig.b.text(at, &s, size, rgb, angle),
        }
    }
    for _ in 0..fig.skipped {
        fig.b.skip();
    }
    let Some(fb) = fig.b.bounds() else {
        return Err("the drawing has nothing Redrob Canvas reads".into());
    };
    let scale = 72.0 / fig.res;
    let margin = fig.res / 4.0;
    let frame = fb.inflate(margin, margin);
    let map = if y_down {
        Affine::scale(scale) * Affine::translate((-frame.x0, -frame.y0))
    } else {
        Affine::scale(scale) * Affine::new([1.0, 0.0, 0.0, -1.0, -frame.x0, frame.y1])
    };
    let board = Rect::new(0.0, 0.0, frame.width() * scale, frame.height() * scale);
    Ok(fig.b.finish(map, Some(board), title))
}

enum Item {
    Path(BezPath, Option<[u8; 3]>, Option<Stroke>, bool),
    Text(Point, String, f64, [u8; 3], f64),
}

/// `\ooo` octal escapes of a FIG string.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    let mut raw: Vec<u8> = vec![];
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            let v = (b[i + 1] - b'0') as u32 * 64 + (b[i + 2] - b'0') as u32 * 8 + (b[i + 3] - b'0') as u32;
            raw.push(v as u8);
            i += 4;
        } else if b[i] == b'\\' && i + 1 < b.len() && b[i + 1] == b'\\' {
            raw.push(b'\\');
            i += 2;
        } else {
            raw.push(b[i]);
            i += 1;
        }
    }
    out.push_str(&String::from_utf8_lossy(&raw));
    out
}
