//! Microsoft Visio drawings: the Open Packaging `.vsdx` / `.vsdm` (a zip of `visio/pages/page*.xml`
//! parts in the 2012 `main` namespace, with masters in `visio/masters/`) and the older XML `.vdx`
//! (`VisioDocument` in the 2003 `core` namespace). Each page becomes an artboard; shapes keep
//! their geometry (move, line, arc, elliptical arc, ellipse and Bézier rows, relative rows,
//! splines and NURBS as polylines), line weight and colour, fill colour, text at the shape's
//! centre and group nesting; cells a shape leaves out come from its master. Visio units are
//! inches, y up from the page's bottom-left corner. The binary `.vsd` format isn't read.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Read as _;

use redrob_vector_doc::Document;
use usvg::roxmltree::{self, Node as XNode};

use crate::{ImportOptions, SvgError, xml_escape};

const PT_PER_IN: f64 = 72.0;

/// Does `bytes` start a zip package?
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

/// Does `text` hold a `.vdx` document?
pub fn is_vdx(text: &str) -> bool {
    let head = &text[..text.len().min(4096)];
    head.contains("schemas.microsoft.com/visio/2003/core") || head.contains("<VisioDocument")
}

/// Does `bytes` hold a `.vsd` binary (an OLE compound file)?
pub fn is_vsd(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1])
}

// ---------- a stored / deflated zip reader ----------

/// The entries of a zip archive by name (stored or deflated).
fn zip_entries(zip: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    let u16_at = |at: usize| zip.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or("truncated zip");
    let u32_at = |at: usize| zip.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or("truncated zip");
    // The end-of-central-directory record, searched back from the end (a comment may follow it).
    let mut end = zip.len().checked_sub(22).ok_or("not a zip")?;
    loop {
        if u32_at(end)? == 0x0605_4b50 {
            break;
        }
        if end == 0 || zip.len() - end > 65_558 {
            return Err("no zip end record".into());
        }
        end -= 1;
    }
    let count = u16_at(end + 10)? as usize;
    let mut at = u32_at(end + 16)? as usize;
    let mut out = HashMap::new();
    for _ in 0..count.min(10_000) {
        if u32_at(at)? != 0x0201_4b50 {
            return Err("bad central record".into());
        }
        let method = u16_at(at + 10)?;
        let (csize, usize_) = (u32_at(at + 20)? as usize, u32_at(at + 24)? as usize);
        let name_len = u16_at(at + 28)? as usize;
        let extra = u16_at(at + 30)? as usize + u16_at(at + 32)? as usize;
        let local = u32_at(at + 42)? as usize;
        let name = String::from_utf8_lossy(zip.get(at + 46..at + 46 + name_len).ok_or("truncated zip")?).into_owned();
        if u32_at(local)? != 0x0403_4b50 {
            return Err(format!("{name}: no local header"));
        }
        let data = local + 30 + u16_at(local + 26)? as usize + u16_at(local + 28)? as usize;
        let raw = zip.get(data..data + csize).ok_or("truncated zip")?;
        if usize_ > 256 * 1024 * 1024 {
            return Err(format!("{name}: too large"));
        }
        let bytes = match method {
            0 => raw.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_);
                flate2::read::DeflateDecoder::new(raw).take(usize_ as u64 + 1).read_to_end(&mut v).map_err(|e| format!("{name}: {e}"))?;
                v
            }
            m => return Err(format!("{name}: compression method {m} isn't supported")),
        };
        out.insert(name, bytes);
        at += 46 + name_len + extra;
    }
    Ok(out)
}

// ---------- cells, in both syntaxes ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Syntax {
    /// 2012+: `<Cell N="PinX" V="1"/>`, `<Section N="Geometry"><Row T="MoveTo">`.
    Main,
    /// 2003: `<XForm><PinX>1</PinX></XForm>`, `<Geom><MoveTo><X>`.
    Core,
}

/// Where a 2003 cell lives: its section element.
fn core_section(cell: &str) -> &'static str {
    match cell {
        "PinX" | "PinY" | "Width" | "Height" | "LocPinX" | "LocPinY" | "Angle" | "FlipX" | "FlipY" => "XForm",
        "LineWeight" | "LineColor" | "LinePattern" | "LineColorTrans" => "Line",
        "FillForegnd" | "FillBkgnd" | "FillPattern" | "FillForegndTrans" => "Fill",
        "Size" | "Color" => "Char",
        _ => "",
    }
}

fn cell_of(shape: XNode, syntax: Syntax, name: &str) -> Option<String> {
    match syntax {
        Syntax::Main => shape
            .children()
            .find(|c| c.is_element() && c.tag_name().name() == "Cell" && c.attribute("N") == Some(name))
            .and_then(|c| c.attribute("V"))
            .map(str::to_string),
        Syntax::Core => {
            let sec = core_section(name);
            let section = if sec.is_empty() { shape } else { shape.children().find(|c| c.is_element() && c.tag_name().name() == sec)? };
            section.children().find(|c| c.is_element() && c.tag_name().name() == name).and_then(|c| c.text()).map(|t| t.trim().to_string())
        }
    }
}

/// A shape with its master shape, for cells the shape leaves out.
#[derive(Clone, Copy)]
struct Sh<'a, 'i> {
    node: XNode<'a, 'i>,
    master: Option<XNode<'a, 'i>>,
    syntax: Syntax,
}

impl<'a, 'i> Sh<'a, 'i> {
    fn cell(&self, name: &str) -> Option<String> {
        cell_of(self.node, self.syntax, name).or_else(|| self.master.and_then(|m| cell_of(m, self.syntax, name)))
    }
    fn num(&self, name: &str) -> Option<f64> {
        self.cell(name).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite())
    }
    fn num_or(&self, name: &str, d: f64) -> f64 {
        self.num(name).unwrap_or(d)
    }
    /// The geometry sections: the shape's own, else the master's.
    fn geometry(&self) -> Vec<XNode<'a, 'i>> {
        let own: Vec<XNode> = geometry_sections(self.node, self.syntax);
        if !own.is_empty() {
            return own;
        }
        self.master.map(|m| geometry_sections(m, self.syntax)).unwrap_or_default()
    }
    fn children_shapes(&self) -> Vec<XNode<'a, 'i>> {
        self.node
            .children()
            .find(|c| c.is_element() && c.tag_name().name() == "Shapes")
            .map(|s| s.children().filter(|c| c.is_element() && c.tag_name().name() == "Shape").collect())
            .unwrap_or_default()
    }
    fn text(&self) -> String {
        let t = self.node.children().find(|c| c.is_element() && c.tag_name().name() == "Text");
        let t = t.or_else(|| self.master.and_then(|m| m.children().find(|c| c.is_element() && c.tag_name().name() == "Text")));
        t.map(|t| t.descendants().filter(|c| c.is_text()).filter_map(|c| c.text()).collect::<String>()).unwrap_or_default()
    }
}

fn geometry_sections<'a, 'i>(shape: XNode<'a, 'i>, syntax: Syntax) -> Vec<XNode<'a, 'i>> {
    match syntax {
        Syntax::Main => shape.children().filter(|c| c.is_element() && c.tag_name().name() == "Section" && c.attribute("N") == Some("Geometry")).collect(),
        Syntax::Core => shape.children().filter(|c| c.is_element() && c.tag_name().name() == "Geom").collect(),
    }
}

/// A geometry row: its kind and named values (`X`, `Y`, `A`…).
struct Row {
    kind: String,
    v: HashMap<String, f64>,
    /// `NoFill` / `NoLine` cells of the section (not rows).
    del: bool,
}

fn rows(section: XNode, syntax: Syntax) -> (Vec<Row>, bool, bool) {
    let mut out = vec![];
    let (mut no_fill, mut no_line) = (false, false);
    for c in section.children().filter(|c| c.is_element()) {
        match syntax {
            Syntax::Main => {
                if c.tag_name().name() == "Cell" {
                    let on = c.attribute("V") == Some("1");
                    match c.attribute("N") {
                        Some("NoFill") => no_fill = on,
                        Some("NoLine") => no_line = on,
                        _ => {}
                    }
                } else if c.tag_name().name() == "Row" {
                    let kind = c.attribute("T").unwrap_or("").to_string();
                    let mut v = HashMap::new();
                    for cell in c.children().filter(|k| k.is_element() && k.tag_name().name() == "Cell") {
                        if let (Some(n), Some(val)) = (cell.attribute("N"), cell.attribute("V").and_then(|x| x.trim().parse::<f64>().ok())) {
                            v.insert(n.to_string(), val);
                        }
                    }
                    out.push(Row { kind, v, del: c.attribute("Del") == Some("1") });
                }
            }
            Syntax::Core => {
                let name = c.tag_name().name();
                match name {
                    "NoFill" => no_fill = c.text().is_some_and(|t| t.trim() == "1"),
                    "NoLine" => no_line = c.text().is_some_and(|t| t.trim() == "1"),
                    "NoShow" | "NoSnap" | "NoQuickDrag" => {}
                    _ => {
                        let mut v = HashMap::new();
                        for k in c.children().filter(|k| k.is_element()) {
                            if let Some(val) = k.text().and_then(|t| t.trim().parse::<f64>().ok()) {
                                v.insert(k.tag_name().name().to_string(), val);
                            }
                        }
                        out.push(Row { kind: name.to_string(), v, del: c.attribute("Del") == Some("1") });
                    }
                }
            }
        }
    }
    (out, no_fill, no_line)
}

fn n(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.to_string() }
}

/// A Visio colour: `#rrggbb`, an index into the default palette, or a theme reference (black).
fn color(v: &str, default: &str) -> String {
    let t = v.trim();
    if let Some(h) = t.strip_prefix('#')
        && h.len() == 6
    {
        return format!("#{h}");
    }
    const PALETTE: [&str; 24] = [
        "#000000", "#ffffff", "#ff0000", "#00ff00", "#0000ff", "#ffff00", "#ff00ff", "#00ffff", "#800000", "#008000", "#000080", "#808000", "#800080",
        "#008080", "#c0c0c0", "#e6e6e6", "#cdcdcd", "#b3b3b3", "#9a9a9a", "#808080", "#666666", "#4d4d4d", "#333333", "#1a1a1a",
    ];
    if let Ok(i) = t.parse::<usize>()
        && let Some(c) = PALETTE.get(i)
    {
        return (*c).to_string();
    }
    default.to_string()
}

/// Converts one page's shapes to SVG.
struct Page<'a, 'i> {
    syntax: Syntax,
    masters: &'a HashMap<String, roxmltree::Document<'i>>,
    body: String,
    warnings: Vec<String>,
    shapes: usize,
}

impl<'a, 'i> Page<'a, 'i> {
    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    /// The master shape a page shape inherits from: the master's first shape (`Master`), or the
    /// sub-shape `MasterShape` names inside the master.
    fn master_of(&self, shape: XNode<'a, 'i>, parent_master: Option<XNode<'a, 'i>>) -> Option<XNode<'a, 'i>> {
        if let Some(ms) = shape.attribute("MasterShape")
            && let Some(pm) = parent_master
        {
            // Within the parent's master: the sub-shape with that ID.
            return pm.descendants().find(|d| d.is_element() && d.tag_name().name() == "Shape" && d.attribute("ID") == Some(ms));
        }
        let id = shape.attribute("Master")?;
        let doc = self.masters.get(id)?;
        doc.descendants().find(|d| d.is_element() && d.tag_name().name() == "Shape")
    }

    /// The transform from `sh`'s local space into its parent's.
    fn transform(sh: &Sh) -> String {
        let (w, h) = (sh.num_or("Width", 0.0), sh.num_or("Height", 0.0));
        let (pin_x, pin_y) = (sh.num_or("PinX", 0.0), sh.num_or("PinY", 0.0));
        let (loc_x, loc_y) = (sh.num_or("LocPinX", w / 2.0), sh.num_or("LocPinY", h / 2.0));
        let angle = sh.num_or("Angle", 0.0);
        let (fx, fy) = (sh.num_or("FlipX", 0.0) != 0.0, sh.num_or("FlipY", 0.0) != 0.0);
        let mut t = format!("translate({} {})", n(pin_x), n(pin_y));
        if angle != 0.0 {
            let _ = write!(t, " rotate({})", n(angle.to_degrees()));
        }
        if fx || fy {
            let _ = write!(t, " scale({} {})", if fx { -1 } else { 1 }, if fy { -1 } else { 1 });
        }
        let _ = write!(t, " translate({} {})", n(-loc_x), n(-loc_y));
        t
    }

    /// The path data of `sh`'s geometry sections, in its local space (inches, y up).
    fn path(&mut self, sh: &Sh) -> Vec<(String, bool, bool)> {
        let (w, h) = (sh.num_or("Width", 0.0), sh.num_or("Height", 0.0));
        let mut out = vec![];
        for section in sh.geometry() {
            let (rows, no_fill, no_line) = rows(section, sh.syntax);
            let mut d = String::new();
            let mut cur = (0.0, 0.0);
            let mut start = (0.0, 0.0);
            let g = |r: &Row, k: &str| r.v.get(k).copied().unwrap_or(0.0);
            for r in &rows {
                if r.del {
                    continue;
                }
                let rel = r.kind.starts_with("Rel");
                let (x, y) = if rel { (g(r, "X") * w, g(r, "Y") * h) } else { (g(r, "X"), g(r, "Y")) };
                match r.kind.as_str() {
                    "MoveTo" | "RelMoveTo" => {
                        let _ = write!(d, "M{} {}", n(x), n(y));
                        cur = (x, y);
                        start = cur;
                    }
                    "LineTo" | "RelLineTo" => {
                        let _ = write!(d, "L{} {}", n(x), n(y));
                        cur = (x, y);
                    }
                    "ArcTo" => {
                        // A circular arc from the current point to (x, y) whose bow is `A` off the chord.
                        let a = g(r, "A");
                        let (dx, dy) = (x - cur.0, y - cur.1);
                        let chord = (dx * dx + dy * dy).sqrt();
                        if a.abs() < 1e-9 || chord < 1e-12 {
                            let _ = write!(d, "L{} {}", n(x), n(y));
                        } else {
                            let r_ = (chord * chord / 4.0 + a * a) / (2.0 * a.abs());
                            let large = a.abs() > chord / 2.0;
                            let sweep = a > 0.0;
                            let _ = write!(d, "A{} {} 0 {} {} {} {}", n(r_), n(r_), u8::from(large), u8::from(sweep), n(x), n(y));
                        }
                        cur = (x, y);
                    }
                    "EllipticalArcTo" | "RelEllipticalArcTo" => {
                        // Control point (A, B) on the arc, C the major axis angle, D the ratio of
                        // major to minor axis.
                        let (a, b) = if rel { (g(r, "A") * w, g(r, "B") * h) } else { (g(r, "A"), g(r, "B")) };
                        let c = g(r, "C");
                        let ratio = g(r, "D").abs().max(1e-6);
                        match ellipse_through(cur, (x, y), (a, b), c, ratio) {
                            Some((rx, ry, large, sweep)) => {
                                let _ = write!(d, "A{} {} {} {} {} {} {}", n(rx), n(ry), n(c.to_degrees()), u8::from(large), u8::from(sweep), n(x), n(y));
                            }
                            None => {
                                let _ = write!(d, "L{} {}", n(x), n(y));
                            }
                        }
                        cur = (x, y);
                    }
                    "Ellipse" => {
                        // Centre (X, Y), a point on the major axis (A, B), on the minor (C, D).
                        let (a, b, c, dd) = (g(r, "A"), g(r, "B"), g(r, "C"), g(r, "D"));
                        let rx = ((a - x).powi(2) + (b - y).powi(2)).sqrt();
                        let ry = ((c - x).powi(2) + (dd - y).powi(2)).sqrt();
                        let rot = (b - y).atan2(a - x).to_degrees();
                        let (sa, ca) = rot.to_radians().sin_cos();
                        let p0 = (x + rx * ca, y + rx * sa);
                        let p1 = (x - rx * ca, y - rx * sa);
                        let _ = write!(
                            d,
                            "M{} {}A{} {} {} 1 0 {} {}A{} {} {} 1 0 {} {}Z",
                            n(p0.0),
                            n(p0.1),
                            n(rx),
                            n(ry),
                            n(rot),
                            n(p1.0),
                            n(p1.1),
                            n(rx),
                            n(ry),
                            n(rot),
                            n(p0.0),
                            n(p0.1)
                        );
                        cur = p0;
                        start = p0;
                    }
                    "RelCubBezTo" => {
                        let (a, b, c, dd) = (g(r, "A") * w, g(r, "B") * h, g(r, "C") * w, g(r, "D") * h);
                        let _ = write!(d, "C{} {} {} {} {} {}", n(a), n(b), n(c), n(dd), n(x), n(y));
                        cur = (x, y);
                    }
                    "RelQuadBezTo" => {
                        let (a, b) = (g(r, "A") * w, g(r, "B") * h);
                        let _ = write!(d, "Q{} {} {} {}", n(a), n(b), n(x), n(y));
                        cur = (x, y);
                    }
                    "NURBSTo" | "SplineStart" | "SplineKnot" | "PolylineTo" | "InfiniteLine" => {
                        if r.kind == "InfiniteLine" {
                            continue;
                        }
                        self.warn("spline, NURBS and polyline-formula rows are read as straight lines to their end points");
                        let _ = write!(d, "L{} {}", n(x), n(y));
                        cur = (x, y);
                    }
                    _ => {
                        let _ = write!(d, "L{} {}", n(x), n(y));
                        cur = (x, y);
                    }
                }
            }
            if d.is_empty() {
                continue;
            }
            let closed = (cur.0 - start.0).abs() < 1e-6 && (cur.1 - start.1).abs() < 1e-6;
            if closed && !d.ends_with('Z') {
                d.push('Z');
            }
            out.push((d, no_fill || !closed, no_line));
        }
        out
    }

    fn shape(&mut self, node: XNode<'a, 'i>, parent_master: Option<XNode<'a, 'i>>, depth: u32) {
        if depth > 32 || self.shapes > 200_000 {
            return;
        }
        self.shapes += 1;
        let master = self.master_of(node, parent_master);
        let sh = Sh { node, master, syntax: self.syntax };
        let t = Self::transform(&sh);
        let name = node.attribute("NameU").or_else(|| node.attribute("Name")).map(|s| format!(" id=\"{}\"", xml_escape(s))).unwrap_or_default();
        let _ = writeln!(self.body, "<g transform=\"{t}\"{name}>");
        // Line and fill.
        let line_pattern = sh.num_or("LinePattern", 1.0);
        let weight = sh.num_or("LineWeight", 0.01);
        let line_color = color(&sh.cell("LineColor").unwrap_or_default(), "#000000");
        let line_alpha = 1.0 - sh.num_or("LineColorTrans", 0.0).clamp(0.0, 1.0);
        let fill_pattern = sh.num_or("FillPattern", 1.0);
        let fill_color = color(&sh.cell("FillForegnd").unwrap_or_default(), "#ffffff");
        let fill_alpha = 1.0 - sh.num_or("FillForegndTrans", 0.0).clamp(0.0, 1.0);
        let dash = match line_pattern as i64 {
            2 => " stroke-dasharray=\"0.08 0.04\"",
            3 => " stroke-dasharray=\"0.02 0.02\"",
            4 => " stroke-dasharray=\"0.08 0.04 0.02 0.04\"",
            9 | 10 | 11 => " stroke-dasharray=\"0.04 0.02\"",
            _ => "",
        };
        for (d, no_fill, no_line) in self.path(&sh) {
            let fill = if no_fill || fill_pattern == 0.0 { "none".to_string() } else { fill_color.clone() };
            let stroke = if no_line || line_pattern == 0.0 { "none".to_string() } else { line_color.clone() };
            let _ = writeln!(
                self.body,
                "<path d=\"{}\" fill=\"{fill}\" fill-opacity=\"{}\" stroke=\"{stroke}\" stroke-width=\"{}\" stroke-opacity=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"{dash}/>",
                xml_escape(&d),
                n(fill_alpha),
                n(weight.max(0.001)),
                n(line_alpha)
            );
        }
        // Text at the shape's centre (its own size is in points).
        let text = sh.text();
        let text = text.trim();
        if !text.is_empty() {
            let (w, h) = (sh.num_or("Width", 0.0), sh.num_or("Height", 0.0));
            let size_pt = sh.num("Size").map(|s| if s < 2.0 { s * 72.0 } else { s }).unwrap_or(8.0);
            let size = size_pt / PT_PER_IN;
            let fill = color(&sh.cell("Color").unwrap_or_default(), "#000000");
            // Local space is y up: the text is flipped back so it reads upright.
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            let total = lines.len() as f64 * size * 1.2;
            for (i, line) in lines.iter().enumerate() {
                let y = h / 2.0 + total / 2.0 - size * 1.2 * (i as f64 + 0.8);
                let _ = writeln!(
                    self.body,
                    "<text x=\"{}\" y=\"{}\" transform=\"translate(0 {}) scale(1 -1)\" font-size=\"{}\" font-family=\"sans-serif\" text-anchor=\"middle\" fill=\"{fill}\">{}</text>",
                    n(w / 2.0),
                    n(-y),
                    n(0.0),
                    n(size),
                    xml_escape(line.trim())
                );
            }
        }
        for child in sh.children_shapes() {
            self.shape(child, master, depth + 1);
        }
        let _ = writeln!(self.body, "</g>");
    }
}

/// The SVG arc through control point `ctrl` from `from` to `to`, with the major axis at angle
/// `c` and axis ratio `ratio` → (rx, ry, large arc, sweep).
fn ellipse_through(from: (f64, f64), to: (f64, f64), ctrl: (f64, f64), c: f64, ratio: f64) -> Option<(f64, f64, bool, bool)> {
    // Into the ellipse's frame, scaled to a circle.
    let (s, co) = c.sin_cos();
    let map = |p: (f64, f64)| {
        let x = p.0 * co + p.1 * s;
        let y = (-p.0 * s + p.1 * co) * ratio;
        (x, y)
    };
    let (p1, p2, p3) = (map(from), map(ctrl), map(to));
    // The circle through three points.
    let d = 2.0 * (p1.0 * (p2.1 - p3.1) + p2.0 * (p3.1 - p1.1) + p3.0 * (p1.1 - p2.1));
    if d.abs() < 1e-12 {
        return None;
    }
    let sq = |p: (f64, f64)| p.0 * p.0 + p.1 * p.1;
    let ux = (sq(p1) * (p2.1 - p3.1) + sq(p2) * (p3.1 - p1.1) + sq(p3) * (p1.1 - p2.1)) / d;
    let uy = (sq(p1) * (p3.0 - p2.0) + sq(p2) * (p1.0 - p3.0) + sq(p3) * (p2.0 - p1.0)) / d;
    let r = ((p1.0 - ux).powi(2) + (p1.1 - uy).powi(2)).sqrt();
    if !(r.is_finite() && r > 0.0) {
        return None;
    }
    // Sweep direction and size from the control point's side.
    let a1 = (p1.1 - uy).atan2(p1.0 - ux);
    let a2 = (p2.1 - uy).atan2(p2.0 - ux);
    let a3 = (p3.1 - uy).atan2(p3.0 - ux);
    let ccw_span = (a3 - a1).rem_euclid(std::f64::consts::TAU);
    let ccw = (a2 - a1).rem_euclid(std::f64::consts::TAU) < ccw_span;
    let span = if ccw { ccw_span } else { std::f64::consts::TAU - ccw_span };
    Some((r, r / ratio, span > std::f64::consts::PI, ccw))
}

/// A page's SVG: the shapes over a page of `w × h` inches, y up → y down.
fn page_svg<'a, 'i>(syntax: Syntax, shapes: XNode<'a, 'i>, masters: &'a HashMap<String, roxmltree::Document<'i>>, w: f64, h: f64) -> (String, Vec<String>) {
    let mut p = Page { syntax, masters, body: String::new(), warnings: vec![], shapes: 0 };
    for s in shapes.children().filter(|c| c.is_element() && c.tag_name().name() == "Shape") {
        p.shape(s, None, 0);
    }
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}pt\" height=\"{}pt\" viewBox=\"0 0 {} {}\"><g transform=\"translate(0 {}) scale(1 -1)\">{}</g></svg>",
        n(w * PT_PER_IN),
        n(h * PT_PER_IN),
        n(w),
        n(h),
        n(h),
        p.body
    );
    (svg, p.warnings)
}

/// A page's SVG imported and merged into `doc` as a new artboard beside the others.
fn add_page(doc: &mut Option<Document>, svg: &str, name: &str, warnings: &mut Vec<String>) -> Result<(), SvgError> {
    let (mut page, w) = crate::import_with(svg, &ImportOptions::default())?;
    warnings.extend(w);
    if let Some(a) = page.artboards.first_mut() {
        a.name = name.to_string();
    }
    match doc {
        None => *doc = Some(page),
        Some(d) => {
            // Beside the last artboard, its art on a layer of its own.
            let right = d.artboards.iter().map(|a| a.rect.x1).fold(f64::NEG_INFINITY, f64::max);
            let gap = 36.0;
            let dx = if right.is_finite() { right + gap } else { 0.0 };
            let shift = redrob_vector_geom::Affine::translate((dx, 0.0));
            for l in &page.layers {
                let mut layer = d.reid(l);
                layer.transform(shift, false);
                layer.name = Some(name.to_string());
                d.layers.push(std::sync::Arc::new(layer));
            }
            for a in &page.artboards {
                let mut a = a.clone();
                a.rect = shift.transform_rect_bbox(a.rect);
                a.id = d.artboards.iter().map(|b| b.id).max().unwrap_or(0) + 1;
                d.artboards.push(a);
            }
            for (k, v) in page.images {
                d.images.entry(k).or_insert(v);
            }
        }
    }
    Ok(())
}

/// Read a `.vsdx` / `.vsdm` package.
pub fn import_vsdx(bytes: &[u8], title: &str) -> Result<(Document, Vec<String>), SvgError> {
    let entries = zip_entries(bytes).map_err(SvgError::Parse)?;
    // Masters by ID: masters.xml lists them with a relationship to each part.
    let texts: HashMap<String, String> = entries.iter().map(|(k, v)| (k.clone(), String::from_utf8_lossy(v).into_owned())).collect();
    let mut masters: HashMap<String, roxmltree::Document<'_>> = HashMap::new();
    if let (Some(list), Some(rels)) = (texts.get("visio/masters/masters.xml"), texts.get("visio/masters/_rels/masters.xml.rels"))
        && let (Ok(list), Ok(rels)) = (parse(list), parse(rels))
    {
        let targets: HashMap<String, String> = rels
            .descendants()
            .filter(|r| r.is_element() && r.tag_name().name() == "Relationship")
            .filter_map(|r| Some((r.attribute("Id")?.to_string(), r.attribute("Target")?.to_string())))
            .collect();
        for m in list.descendants().filter(|m| m.is_element() && m.tag_name().name() == "Master") {
            let Some(id) = m.attribute("ID") else { continue };
            let rel = m.descendants().find(|r| r.is_element() && r.tag_name().name() == "Rel").and_then(|r| r.attributes().find(|a| a.name() == "id")).map(|a| a.value());
            let Some(target) = rel.and_then(|r| targets.get(r)) else { continue };
            let path = format!("visio/masters/{}", target.trim_start_matches("/visio/masters/").trim_start_matches('/'));
            if let Some(text) = texts.get(&path)
                && let Ok(doc) = parse(text)
            {
                masters.insert(id.to_string(), doc);
            }
        }
    }
    // Pages: pages.xml gives names, sizes and the relationship to each part.
    let mut pages: Vec<(String, String, f64, f64)> = vec![];
    if let (Some(list), Some(rels)) = (texts.get("visio/pages/pages.xml"), texts.get("visio/pages/_rels/pages.xml.rels"))
        && let (Ok(list), Ok(rels)) = (parse(list), parse(rels))
    {
        let targets: HashMap<String, String> = rels
            .descendants()
            .filter(|r| r.is_element() && r.tag_name().name() == "Relationship")
            .filter_map(|r| Some((r.attribute("Id")?.to_string(), r.attribute("Target")?.to_string())))
            .collect();
        for (i, pg) in list.descendants().filter(|m| m.is_element() && m.tag_name().name() == "Page").enumerate() {
            if pg.attribute("Background") == Some("1") {
                continue;
            }
            let name = pg.attribute("NameU").or_else(|| pg.attribute("Name")).map(str::to_string).unwrap_or_else(|| format!("Page-{}", i + 1));
            let sheet = pg.children().find(|c| c.is_element() && c.tag_name().name() == "PageSheet");
            let cell = |n: &str| sheet.and_then(|s| cell_of(s, Syntax::Main, n)).and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0);
            let (w, h) = (cell("PageWidth").unwrap_or(8.5), cell("PageHeight").unwrap_or(11.0));
            let rel = pg.descendants().find(|r| r.is_element() && r.tag_name().name() == "Rel").and_then(|r| r.attributes().find(|a| a.name() == "id")).map(|a| a.value());
            let Some(target) = rel.and_then(|r| targets.get(r)) else { continue };
            let path = format!("visio/pages/{}", target.trim_start_matches("/visio/pages/").trim_start_matches('/'));
            pages.push((name, path, w, h));
        }
    }
    if pages.is_empty() {
        // No index: every page part, in name order.
        let mut names: Vec<&String> = texts.keys().filter(|k| k.starts_with("visio/pages/page") && k.ends_with(".xml")).collect();
        names.sort();
        pages = names.into_iter().enumerate().map(|(i, k)| (format!("Page-{}", i + 1), k.clone(), 8.5, 11.0)).collect();
    }
    if pages.is_empty() {
        return Err(SvgError::Parse("the package has no Visio pages".into()));
    }
    let mut doc: Option<Document> = None;
    let mut warnings = vec![];
    for (name, path, w, h) in pages {
        let Some(text) = texts.get(&path) else { continue };
        let xml = parse(text).map_err(|e| SvgError::Parse(e.to_string()))?;
        let Some(shapes) = xml.descendants().find(|c| c.is_element() && c.tag_name().name() == "Shapes") else { continue };
        let (svg, w_) = page_svg(Syntax::Main, shapes, &masters, w, h);
        warnings.extend(w_);
        add_page(&mut doc, &svg, &name, &mut warnings)?;
    }
    let mut doc = doc.ok_or_else(|| SvgError::Parse("the drawing has no shapes".into()))?;
    doc.title = title.to_string();
    dedup(&mut warnings);
    Ok((doc, warnings))
}

/// Read a `.vdx` XML drawing.
pub fn import_vdx(text: &str, title: &str) -> Result<(Document, Vec<String>), SvgError> {
    let xml = roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
        .map_err(|e| SvgError::Parse(e.to_string()))?;
    // Masters are inline: each `<Master ID>` holds its shapes.
    let mut masters: HashMap<String, roxmltree::Document<'_>> = HashMap::new();
    let mut master_texts: Vec<(String, String)> = vec![];
    for m in xml.descendants().filter(|m| m.is_element() && m.tag_name().name() == "Master") {
        if let Some(id) = m.attribute("ID") {
            let range = m.range();
            if let Some(src) = text.get(range) {
                master_texts.push((id.to_string(), src.to_string()));
            }
        }
    }
    for (id, src) in &master_texts {
        if let Ok(d) = roxmltree::Document::parse(src) {
            masters.insert(id.clone(), d);
        }
    }
    let mut doc: Option<Document> = None;
    let mut warnings = vec![];
    for (i, pg) in xml.descendants().filter(|p| p.is_element() && p.tag_name().name() == "Page").enumerate() {
        if pg.attribute("Background") == Some("1") {
            continue;
        }
        let name = pg.attribute("NameU").or_else(|| pg.attribute("Name")).map(str::to_string).unwrap_or_else(|| format!("Page-{}", i + 1));
        let sheet = pg.children().find(|c| c.is_element() && c.tag_name().name() == "PageSheet");
        let cell = |n: &str| {
            sheet
                .and_then(|s| s.children().find(|c| c.is_element() && c.tag_name().name() == "PageProps"))
                .and_then(|p| p.children().find(|c| c.is_element() && c.tag_name().name() == n))
                .and_then(|c| c.text())
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| *v > 0.0)
        };
        let (w, h) = (cell("PageWidth").unwrap_or(8.5), cell("PageHeight").unwrap_or(11.0));
        let Some(shapes) = pg.children().find(|c| c.is_element() && c.tag_name().name() == "Shapes") else { continue };
        let (svg, w_) = page_svg(Syntax::Core, shapes, &masters, w, h);
        warnings.extend(w_);
        add_page(&mut doc, &svg, &name, &mut warnings)?;
    }
    let mut doc = doc.ok_or_else(|| SvgError::Parse("the drawing has no pages with shapes".into()))?;
    doc.title = title.to_string();
    dedup(&mut warnings);
    Ok((doc, warnings))
}

fn parse(text: &str) -> Result<roxmltree::Document<'_>, roxmltree::Error> {
    roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
}

fn dedup(w: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    w.retain(|x| seen.insert(x.clone()));
}
