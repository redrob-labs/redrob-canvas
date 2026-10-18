//! HPGL and HPGL/2 plotter files (`.hpgl`, `.plt`): two-letter instructions with numeric
//! parameters. Read: `IN`, `SP` (pen), `PU`/`PD` (pen up/down, with coordinates), `PA`/`PR`
//! (absolute/relative plotting), `CI` (circle), `AA`/`AR` (arcs about a centre), `EA`/`ER`
//! (rectangles), `RA`/`RR` (filled rectangles), `PW` (pen width, mm), `PC` (pen colour), `LT`
//! (line type: dashes), `SC`/`IP` (scaling), `LB` (labels), `PE` (encoded polylines).
//! Coordinates are plotter units (1/40 mm, 1016 per inch), y up.

use kurbo::BezPath;
use redrob_vector_geom::{Affine, Point, Rect};

use super::{Builder, Imported, Stroke, arc_to};

/// Points per plotter unit.
const PT_PER_UNIT: f64 = 72.0 / 1016.0;

/// The default pen colours of an 8-pen carousel.
const PENS: [[u8; 3]; 9] =
    [[255, 255, 255], [0, 0, 0], [255, 0, 0], [0, 128, 0], [255, 255, 0], [0, 0, 255], [255, 0, 255], [0, 255, 255], [128, 64, 0]];

/// Does `bytes` look like an HPGL file (an `IN`, `SP`, `PU`, `PD` or `PA` instruction near the start)?
pub fn is_hpgl(bytes: &[u8]) -> bool {
    let head: Vec<u8> = bytes.iter().take(256).copied().filter(|b| !b.is_ascii_whitespace()).collect();
    if head.is_empty() || !head.iter().all(|b| b.is_ascii_graphic() || *b == 0x1b) {
        return false;
    }
    // An escape sequence (PCL/PJL wrapper) may come first.
    let start = head.iter().position(|b| b.is_ascii_alphabetic()).unwrap_or(0);
    let h = &head[start..];
    [b"IN", b"SP", b"PU", b"PD", b"PA", b"DF", b"BP", b"PS", b"RO", b"IP", b"SC"].iter().any(|m| h.starts_with(m.as_slice())) && h.contains(&b';')
}

struct Plotter {
    b: Builder,
    pos: Point,
    down: bool,
    relative: bool,
    pen: usize,
    pens: Vec<[u8; 3]>,
    /// Pen width in plotter units.
    width: f64,
    dash: Vec<f64>,
    /// The polyline being drawn with the pen down.
    path: Option<BezPath>,
    /// `SC` scaling: user units → plotter units.
    scale: Affine,
    /// The `IP` frame (plotter units).
    frame: Option<Rect>,
    label_size: f64,
    skipped: usize,
}

impl Plotter {
    fn new() -> Self {
        Self {
            b: Builder::new(),
            pos: Point::ZERO,
            down: false,
            relative: false,
            pen: 1,
            pens: PENS.to_vec(),
            width: 0.35 * 40.0,
            dash: vec![],
            path: None,
            scale: Affine::IDENTITY,
            frame: None,
            label_size: 0.375 * 40.0 * 25.4 / 2.54,
            skipped: 0,
        }
    }

    fn stroke(&self) -> Stroke {
        let rgb = self.pens.get(self.pen).copied().unwrap_or([0, 0, 0]);
        Stroke { rgb, width: self.width.max(1.0), dash: self.dash.clone(), ..Stroke::new(rgb, 1.0) }
    }

    fn flush(&mut self) {
        if let Some(bp) = self.path.take() {
            let st = self.stroke();
            self.b.path(bp, None, Some(st), false);
        }
    }

    /// Move or draw to `p` (plotter units).
    fn to(&mut self, p: Point) {
        if self.down {
            match &mut self.path {
                Some(bp) => bp.line_to(p),
                None => {
                    let mut bp = BezPath::new();
                    bp.move_to(self.pos);
                    bp.line_to(p);
                    self.path = Some(bp);
                }
            }
        } else {
            self.flush();
        }
        self.pos = p;
    }

    /// A parameter pair as a point (user units through the scale; relative when `PR`).
    fn point(&self, x: f64, y: f64) -> Point {
        let p = self.scale * Point::new(x, y);
        if self.relative { self.pos + (p - (self.scale * Point::ZERO)) } else { p }
    }

    fn plot(&mut self, nums: &[f64]) {
        let mut i = 0;
        while i + 1 < nums.len() {
            let p = self.point(nums[i], nums[i + 1]);
            self.to(p);
            i += 2;
        }
    }

    fn arc(&mut self, cx: f64, cy: f64, sweep_deg: f64, relative: bool) {
        let c = if relative { self.pos + (self.scale * Point::new(cx, cy) - self.scale * Point::ZERO) } else { self.scale * Point::new(cx, cy) };
        let r = (self.pos - c).hypot();
        if r <= 0.0 {
            return;
        }
        let start = (self.pos - c).atan2();
        let sweep = sweep_deg.to_radians();
        if self.down {
            let mut bp = self.path.take().unwrap_or_else(|| {
                let mut bp = BezPath::new();
                bp.move_to(self.pos);
                bp
            });
            arc_to(&mut bp, c, r, r, start, sweep, 0.0);
            self.path = Some(bp);
        }
        let end = start + sweep;
        self.pos = c + redrob_vector_geom::Vec2::new(r * end.cos(), r * end.sin());
    }

    fn circle(&mut self, r: f64) {
        self.flush();
        let r = (self.scale * Point::new(r, 0.0) - self.scale * Point::ZERO).hypot();
        if r > 0.0 {
            let bp = super::ellipse(self.pos, r, r, 0.0);
            let st = self.stroke();
            self.b.path(bp, None, Some(st), false);
        }
    }

    fn rect(&mut self, x: f64, y: f64, relative: bool, fill: bool) {
        self.flush();
        let was = self.relative;
        self.relative = relative;
        let corner = self.point(x, y);
        self.relative = was;
        let bp = super::rect_path(self.pos, corner);
        let st = self.stroke();
        if fill { self.b.path(bp, Some(st.rgb), None, false) } else { self.b.path(bp, None, Some(st), false) }
    }

    fn instruction(&mut self, mn: &str, nums: &[f64], text: &str) {
        match mn {
            "IN" | "DF" => {
                self.flush();
                self.down = false;
                self.relative = false;
                self.scale = Affine::IDENTITY;
                self.dash.clear();
            }
            "SP" => {
                self.flush();
                self.pen = nums.first().map(|n| *n as usize).unwrap_or(0);
            }
            "PU" => {
                self.flush();
                self.down = false;
                self.plot(nums);
            }
            "PD" => {
                self.down = true;
                if nums.len() >= 2 {
                    self.plot(nums);
                }
            }
            "PA" => {
                self.relative = false;
                self.plot(nums);
            }
            "PR" => {
                self.relative = true;
                self.plot(nums);
            }
            "CI" => {
                if let Some(r) = nums.first() {
                    self.circle(*r);
                }
            }
            "AA" | "AR" => {
                if let [cx, cy, a, ..] = nums {
                    self.arc(*cx, *cy, *a, mn == "AR");
                }
            }
            "EA" | "ER" | "RA" | "RR" => {
                if let [x, y, ..] = nums {
                    self.rect(*x, *y, mn.ends_with('R'), mn.starts_with('R'));
                }
            }
            "PW" => {
                // Pen width in mm (metric by default).
                if let Some(w) = nums.first() {
                    self.width = (w * 40.0).max(0.1);
                }
            }
            "PC" => {
                if let [pen, r, g, b, ..] = nums {
                    let i = *pen as usize;
                    if i < 256 {
                        while self.pens.len() <= i {
                            self.pens.push([0, 0, 0]);
                        }
                        let q = |v: f64| v.clamp(0.0, 255.0).round() as u8;
                        self.pens[i] = [q(*r), q(*g), q(*b)];
                    }
                }
            }
            "LT" => {
                self.flush();
                self.dash = match nums.first().map(|n| *n as i64) {
                    None | Some(0) => vec![],
                    Some(1) => vec![1.0, 80.0],
                    Some(2) => vec![40.0, 40.0],
                    Some(3) => vec![60.0, 20.0],
                    Some(4) => vec![50.0, 10.0, 10.0, 10.0],
                    Some(5) => vec![40.0, 10.0, 20.0, 10.0],
                    Some(6) => vec![40.0, 10.0, 10.0, 10.0, 10.0, 10.0],
                    Some(_) => vec![30.0, 30.0],
                };
            }
            "IP" => {
                if let [x0, y0, x1, y1, ..] = nums {
                    self.frame = Some(Rect::new(*x0, *y0, *x1, *y1));
                }
            }
            "SC" => {
                self.flush();
                if let [x0, x1, y0, y1, ..] = nums {
                    let f = self.frame.unwrap_or(Rect::new(0.0, 0.0, 10_000.0, 7_000.0));
                    let (dx, dy) = (x1 - x0, y1 - y0);
                    if dx.abs() > 1e-9 && dy.abs() > 1e-9 {
                        let sx = f.width() / dx;
                        let sy = f.height() / dy;
                        self.scale = Affine::translate((f.x0, f.y0)) * Affine::scale_non_uniform(sx, sy) * Affine::translate((-x0, -y0));
                    }
                } else {
                    self.scale = Affine::IDENTITY;
                }
            }
            "SI" => {
                // Character size in cm (width, height).
                if let [_, h, ..] = nums {
                    self.label_size = h * 400.0;
                }
            }
            "LB" => {
                self.flush();
                let rgb = self.pens.get(self.pen).copied().unwrap_or([0, 0, 0]);
                self.b.text(self.pos, text, self.label_size.max(10.0), rgb, 0.0);
                // The pen moves on past the label.
                self.pos.x += self.label_size * 0.6 * text.chars().count() as f64;
            }
            "PE" => self.b.warn("polyline encoded (PE) instructions are compressed and aren't read: those lines are left out"),
            "FT" | "PT" | "LA" | "TR" | "WU" | "NP" | "PS" | "RO" | "BP" | "PG" | "AF" | "PM" | "EP" | "FP" | "CO" | "DI" | "DR" | "SM" | "SL"
            | "ES" | "DT" | "DV" | "LO" | "SR" | "SD" | "SS" | "SA" | "CP" | "CT" | "AT" | "RT" | "EW" | "WG" | "PP" | "IW" | "QL" | "MC" | "MG"
            | "MT" | "AC" | "TD" | "SB" | "FI" | "FN" | "AD" | "UL" | "VS" | "ST" => {}
            _ => self.skipped += 1,
        }
    }
}

/// Read an HPGL file into a document.
pub fn import(bytes: &[u8], title: &str) -> Result<Imported, String> {
    if bytes.is_empty() {
        return Err("the file is empty".into());
    }
    let text: String = bytes.iter().map(|b| if *b < 0x80 { *b as char } else { '?' }).collect();
    let mut p = Plotter::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == 0x1b as char {
            // PCL / PJL escape sequence: skip to its terminator (an upper-case letter).
            i += 1;
            while i < chars.len() && !chars[i].is_ascii_uppercase() {
                i += 1;
            }
            i += 1;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        if i + 1 >= chars.len() || !chars[i + 1].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let mn: String = chars[i..i + 2].iter().collect::<String>().to_ascii_uppercase();
        i += 2;
        if mn == "LB" {
            // A label runs to the terminator character (ETX by default).
            let mut s = String::new();
            while i < chars.len() && chars[i] != '\u{3}' && chars[i] != ';' && chars[i] != '\n' && chars[i] != '\r' {
                s.push(chars[i]);
                i += 1;
            }
            i += 1;
            p.instruction("LB", &[], &s);
            continue;
        }
        // Parameters up to the next instruction or `;`.
        let mut params = String::new();
        while i < chars.len() && chars[i] != ';' && !chars[i].is_ascii_alphabetic() && chars[i] != (0x1b as char) {
            params.push(chars[i]);
            i += 1;
        }
        if mn == "PE" {
            // Encoded data up to `;`.
            while i < chars.len() && chars[i] != ';' {
                i += 1;
            }
        }
        if i < chars.len() && chars[i] == ';' {
            i += 1;
        }
        let nums: Vec<f64> = params
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.trim_end_matches(['+']).parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .collect();
        p.instruction(&mn, &nums, "");
    }
    p.flush();
    if p.skipped > 0 {
        for _ in 0..p.skipped {
            p.b.skip();
        }
    }
    let bounds = p.b.bounds();
    let Some(fb) = bounds else {
        return Err("the file draws nothing Redrob Canvas reads".into());
    };
    // Plotter units, y up → points, y down with the drawing's top-left at the origin.
    let margin = 40.0 * 5.0;
    let frame = p.frame.map(|f| f.union(fb)).unwrap_or(fb).inflate(margin, margin);
    let map = Affine::scale(PT_PER_UNIT) * Affine::new([1.0, 0.0, 0.0, -1.0, -frame.x0, frame.y1]);
    let board = Rect::new(0.0, 0.0, frame.width() * PT_PER_UNIT, frame.height() * PT_PER_UNIT);
    Ok(p.b.finish(map, Some(board), title))
}
