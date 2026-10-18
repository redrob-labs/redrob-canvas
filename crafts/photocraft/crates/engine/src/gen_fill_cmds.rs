// SPDX-License-Identifier: MIT OR Apache-2.0
//! Generated fill layers: screentone (halftone dots and lines), simplex noise, the de Bruijn
//! multigrid tiling and procedural textures written as a small expression script. Each is a
//! Pattern Fill layer whose pattern is rendered here from its parameters; the parameters ride in
//! the pattern's id (`gen:<json>`), so the layer stays editable and re-renders on change.

use redrob_photo_color::{ColorMode, PixelFormat, SampleType};
use redrob_photo_doc::{Fill, Layer, LayerContent, Pattern};
use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_generated(s: &Session) -> std::result::Result<(), String> {
    let l = crate::active_layer_of(s)?;
    match &l.content {
        LayerContent::Fill(Fill::Pattern { id, .. }) if id.starts_with("gen:") => Ok(()),
        _ => Err("select a generated fill layer".into()),
    }
}

// ---------- generators ----------

/// The generators and their parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "generator", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Generator {
    Screentone {
        #[serde(default = "d_dots")]
        pattern: String,
        #[serde(default = "d_round")]
        shape: String,
        #[serde(default = "d_size")]
        size: f64,
        #[serde(default)]
        rotation: f64,
        #[serde(default = "d_linear")]
        interpolation: String,
        #[serde(default)]
        brightness: f64,
        #[serde(default)]
        contrast: f64,
        #[serde(default = "d_black")]
        foreground: [f32; 4],
        #[serde(default = "d_white")]
        background: [f32; 4],
        #[serde(default)]
        invert: bool,
    },
    SimplexNoise {
        #[serde(default = "d_freq")]
        frequency: f64,
        #[serde(default = "d_one")]
        ratio: f64,
        #[serde(default)]
        seed: u32,
        #[serde(default = "d_true")]
        seamless: bool,
        #[serde(default = "d_one_u")]
        octaves: u32,
    },
    Multigrid {
        #[serde(default = "d_five")]
        dimensions: u32,
        #[serde(default = "d_divisions")]
        divisions: u32,
        #[serde(default = "d_offset")]
        offset: f64,
        #[serde(default = "d_ten")]
        line_width: f64,
        #[serde(default = "d_black")]
        line_color: [f32; 4],
        #[serde(default = "d_color1")]
        color1: [f32; 4],
        #[serde(default = "d_color2")]
        color2: [f32; 4],
        #[serde(default)]
        color_by: String,
        #[serde(default = "d_one")]
        scale: f64,
    },
    Script {
        script: String,
        #[serde(default)]
        time: f64,
    },
}

fn d_dots() -> String {
    "dots".into()
}
fn d_round() -> String {
    "round".into()
}
fn d_linear() -> String {
    "linear".into()
}
fn d_size() -> f64 {
    10.0
}
fn d_black() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}
fn d_white() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}
fn d_color1() -> [f32; 4] {
    [0.9, 0.9, 0.9, 1.0]
}
fn d_color2() -> [f32; 4] {
    [0.6, 0.7, 0.9, 1.0]
}
fn d_freq() -> f64 {
    25.0
}
fn d_one() -> f64 {
    1.0
}
fn d_one_u() -> u32 {
    1
}
fn d_true() -> bool {
    true
}
fn d_five() -> u32 {
    5
}
fn d_divisions() -> u32 {
    4
}
fn d_offset() -> f64 {
    0.2
}
fn d_ten() -> f64 {
    2.0
}

impl Generator {
    pub fn name(&self) -> &'static str {
        match self {
            Generator::Screentone { .. } => "Screentone",
            Generator::SimplexNoise { .. } => "Simplex Noise",
            Generator::Multigrid { .. } => "Multigrid",
            Generator::Script { .. } => "Script",
        }
    }

    /// Whether the rendered tile repeats seamlessly (so a small tile is enough).
    fn tile_size(&self, canvas: Rect) -> (u32, u32) {
        let (w, h) = (canvas.width().clamp(1, 4000), canvas.height().clamp(1, 4000));
        match self {
            Generator::Screentone { size, rotation, .. } if *rotation == 0.0 => {
                // One tile of dots: the cell rounded to whole pixels, repeated across the canvas.
                let c = size.round().clamp(2.0, 512.0) as u32;
                (c, c)
            }
            _ => (w, h),
        }
    }
}

/// Render the generator's pattern for a canvas.
pub fn render(g: &Generator, canvas: Rect) -> Result<Pattern> {
    let (w, h) = g.tile_size(canvas);
    let n = (w as usize) * (h as usize);
    let mut px = vec![[0.0f32; 4]; n];
    match g {
        Generator::Screentone { pattern, shape, size, rotation, interpolation, brightness, contrast, foreground, background, invert } => {
            screentone(&mut px, w, h, pattern, shape, *size, *rotation, interpolation, *brightness, *contrast, *foreground, *background, *invert);
        }
        Generator::SimplexNoise { frequency, ratio, seed, seamless, octaves } => simplex_fill(&mut px, w, h, *frequency, *ratio, *seed, *seamless, *octaves),
        Generator::Multigrid { dimensions, divisions, offset, line_width, line_color, color1, color2, color_by, scale } => {
            multigrid(&mut px, w, h, *dimensions, *divisions, *offset, *line_width, *line_color, *color1, *color2, color_by, *scale);
        }
        Generator::Script { script, time } => script_fill(&mut px, w, h, script, *time)?,
    }
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let mut data = vec![0.0f32; n * 4];
    for (p, out) in px.iter().zip(data.chunks_exact_mut(4)) {
        redrob_photo_raster::from_rgba_into(&fmt, *p, out);
    }
    let mut s = Surface::new(fmt);
    s.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
    let mut pat = Pattern::new(g.name(), s, w, h);
    pat.id = format!("gen:{}", serde_json::to_string(g).unwrap_or_default());
    Ok(pat)
}

fn mix4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

#[allow(clippy::too_many_arguments)]
fn screentone(
    px: &mut [[f32; 4]],
    w: u32,
    h: u32,
    pattern: &str,
    shape: &str,
    size: f64,
    rotation: f64,
    interpolation: &str,
    brightness: f64,
    contrast: f64,
    fg: [f32; 4],
    bg: [f32; 4],
    invert: bool,
) {
    let cell = size.max(1.0);
    let (sin, cos) = rotation.to_radians().sin_cos();
    let (fg, bg) = if invert { (bg, fg) } else { (fg, bg) };
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            // Rotate about the tile centre, then take the position within the cell (0..1).
            let (hx, hy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
            let (rx, ry) = ((cx - hx) * cos + (cy - hy) * sin + hx, -(cx - hx) * sin + (cy - hy) * cos + hy);
            let (u, v) = ((rx / cell).rem_euclid(1.0) - 0.5, (ry / cell).rem_euclid(1.0) - 0.5);
            // Coverage 0..1 from the shape's distance field.
            let d = match (pattern, shape) {
                ("lines", _) => v.abs() * 2.0,
                (_, "square") => u.abs().max(v.abs()) * 2.0,
                (_, "diamond") => (u.abs() + v.abs()),
                (_, "ellipse") => ((u * 2.0).powi(2) + (v * 1.2).powi(2)).sqrt(),
                (_, "line") => u.abs() * 2.0,
                _ => (u * u + v * v).sqrt() * 2.0,
            };
            let mut t = (1.0 - d).clamp(0.0, 1.0);
            if interpolation == "sinusoidal" {
                t = 0.5 - 0.5 * (t * std::f64::consts::PI).cos();
            }
            // Brightness lifts the whole tone, contrast steepens it around the middle.
            let c = (contrast / 100.0).clamp(-1.0, 1.0);
            let k = if c >= 0.0 { 1.0 + c * 4.0 } else { 1.0 + c };
            t = ((t - 0.5) * k + 0.5 + brightness / 100.0).clamp(0.0, 1.0);
            px[(y * w + x) as usize] = mix4(bg, fg, t as f32);
        }
    }
}

// ---- simplex noise (2D, gradient on a skewed simplex grid) ----

fn perm(seed: u32) -> [u8; 512] {
    let mut p: Vec<u8> = (0..=255).collect();
    let mut s = seed.wrapping_mul(0x9E37_79B9).wrapping_add(0x7F4A_7C15);
    for i in (1..256).rev() {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let j = (s as usize) % (i + 1);
        p.swap(i, j);
    }
    let mut out = [0u8; 512];
    for i in 0..512 {
        out[i] = p[i & 255];
    }
    out
}

const GRAD: [[f64; 2]; 8] = [[1.0, 1.0], [-1.0, 1.0], [1.0, -1.0], [-1.0, -1.0], [1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]];

/// 2D simplex noise in about -1..1.
fn simplex2(p: &[u8; 512], x: f64, y: f64) -> f64 {
    let f2 = 0.5 * (3f64.sqrt() - 1.0);
    let g2 = (3.0 - 3f64.sqrt()) / 6.0;
    let s = (x + y) * f2;
    let (i, j) = ((x + s).floor(), (y + s).floor());
    let t = (i + j) * g2;
    let (x0, y0) = (x - (i - t), y - (j - t));
    let (i1, j1) = if x0 > y0 { (1.0, 0.0) } else { (0.0, 1.0) };
    let (x1, y1) = (x0 - i1 + g2, y0 - j1 + g2);
    let (x2, y2) = (x0 - 1.0 + 2.0 * g2, y0 - 1.0 + 2.0 * g2);
    let (ii, jj) = ((i as i64 & 255) as usize, (j as i64 & 255) as usize);
    let corner = |xx: f64, yy: f64, di: usize, dj: usize| {
        let mut t = 0.5 - xx * xx - yy * yy;
        if t < 0.0 {
            return 0.0;
        }
        t *= t;
        let g = GRAD[(p[ii + di + p[jj + dj] as usize] & 7) as usize];
        t * t * (g[0] * xx + g[1] * yy)
    };
    70.0 * (corner(x0, y0, 0, 0) + corner(x1, y1, i1 as usize, j1 as usize) + corner(x2, y2, 1, 1))
}

fn simplex_fill(px: &mut [[f32; 4]], w: u32, h: u32, frequency: f64, ratio: f64, seed: u32, seamless: bool, octaves: u32) {
    let p = perm(seed);
    let fx = frequency.max(0.01) / f64::from(w.max(1));
    let fy = fx * ratio.max(0.01);
    let octaves = octaves.clamp(1, 8);
    let sample = |x: f64, y: f64| {
        let (mut v, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for _ in 0..octaves {
            v += amp * simplex2(&p, x * f, y * f);
            norm += amp;
            amp *= 0.5;
            f *= 2.0;
        }
        v / norm
    };
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = (f64::from(x) * fx, f64::from(y) * fy);
            let v = if seamless {
                // Cross-fade the tile with itself shifted by one period in each direction.
                let (u, vv) = (f64::from(x) / f64::from(w), f64::from(y) / f64::from(h));
                let (px_, py_) = (f64::from(w) * fx, f64::from(h) * fy);
                sample(sx, sy) * (1.0 - u) * (1.0 - vv)
                    + sample(sx - px_, sy) * u * (1.0 - vv)
                    + sample(sx, sy - py_) * (1.0 - u) * vv
                    + sample(sx - px_, sy - py_) * u * vv
            } else {
                sample(sx, sy)
            };
            let g = ((v + 1.0) * 0.5).clamp(0.0, 1.0) as f32;
            px[(y * w + x) as usize] = [g, g, g, 1.0];
        }
    }
}

// ---- de Bruijn multigrid ----

/// Scanline-fill a convex polygon into the buffer.
fn fill_polygon(px: &mut [[f32; 4]], w: u32, h: u32, pts: &[[f64; 2]], color: [f32; 4]) {
    let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
    let (ys, ye) = (y0.floor().max(0.0) as i64, y1.ceil().min(f64::from(h)) as i64);
    for y in ys..ye {
        let yc = y as f64 + 0.5;
        let mut xs: Vec<f64> = Vec::new();
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            if (a[1] <= yc && b[1] > yc) || (b[1] <= yc && a[1] > yc) {
                xs.push(a[0] + (yc - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
            }
        }
        if xs.len() < 2 {
            continue;
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let (xa, xb) = (xs[0].round().max(0.0) as i64, xs[xs.len() - 1].round().min(f64::from(w)) as i64);
        for x in xa..xb {
            px[(y as u32 * w + x as u32) as usize] = color;
        }
    }
}

/// Draw a line of `width` pixels.
fn stroke_line(px: &mut [[f32; 4]], w: u32, h: u32, a: [f64; 2], b: [f64; 2], width: f64, color: [f32; 4]) {
    if width <= 0.0 {
        return;
    }
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt().max(1e-9);
    let (nx, ny) = (-dy / len * width / 2.0, dx / len * width / 2.0);
    fill_polygon(px, w, h, &[[a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]], color);
}

#[allow(clippy::too_many_arguments)]
fn multigrid(
    px: &mut [[f32; 4]],
    w: u32,
    h: u32,
    dimensions: u32,
    divisions: u32,
    offset: f64,
    line_width: f64,
    line_color: [f32; 4],
    color1: [f32; 4],
    color2: [f32; 4],
    color_by: &str,
    scale: f64,
) {
    let n = dimensions.clamp(3, 12) as usize;
    let div = divisions.clamp(1, 40) as i64;
    let angles: Vec<[f64; 2]> = (0..n)
        .map(|k| {
            let a = std::f64::consts::PI * k as f64 / n as f64;
            [a.cos(), a.sin()]
        })
        .collect();
    let unit = f64::from(w.min(h)) / (2.0 * div as f64 + 1.0) * scale.max(0.05);
    let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    // Dual vertex of a point in grid space: the sum over families of the ceiling of its offset
    // line index times that family's direction.
    let dual = |p: [f64; 2]| {
        let mut v = [0.0, 0.0];
        for e in &angles {
            let k = (p[0] * e[0] + p[1] * e[1] - offset).ceil();
            v[0] += k * e[0];
            v[1] += k * e[1];
        }
        v
    };
    let mut rhombs = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            let (ei, ej) = (angles[i], angles[j]);
            let det = ei[0] * ej[1] - ei[1] * ej[0];
            if det.abs() < 1e-9 {
                continue;
            }
            for ki in -div..=div {
                for kj in -div..=div {
                    // Intersection of line ki of family i and line kj of family j.
                    let (ci, cj) = (ki as f64 + offset, kj as f64 + offset);
                    let x = (ci * ej[1] - cj * ei[1]) / det;
                    let y = (cj * ei[0] - ci * ej[0]) / det;
                    let eps = 1e-6;
                    let corners = [
                        dual([x - eps * (ei[0] + ej[0]), y - eps * (ei[1] + ej[1])]),
                        dual([x + eps * (ei[0] - ej[0]), y + eps * (ei[1] - ej[1])]),
                        dual([x + eps * (ei[0] + ej[0]), y + eps * (ei[1] + ej[1])]),
                        dual([x - eps * (ei[0] - ej[0]), y - eps * (ei[1] - ej[1])]),
                    ];
                    let pts: Vec<[f64; 2]> = corners.iter().map(|c| [cx + c[0] * unit, cy + c[1] * unit]).collect();
                    let shape = ((j - i) as f64 * std::f64::consts::PI / n as f64).sin().abs();
                    rhombs.push((pts, shape, (ki + kj).rem_euclid(2) == 0));
                }
            }
        }
    }
    for (pts, shape, parity) in &rhombs {
        let t = match color_by {
            "parity" => {
                if *parity {
                    1.0
                } else {
                    0.0
                }
            }
            _ => *shape as f32,
        };
        fill_polygon(px, w, h, pts, mix4(color1, color2, t));
    }
    for (pts, _, _) in &rhombs {
        for k in 0..4 {
            stroke_line(px, w, h, pts[k], pts[(k + 1) % 4], line_width, line_color);
        }
    }
}

// ---- expression scripts ----

/// A value: scalars and 3-vectors, as in the expression language.
#[derive(Clone, Copy, Debug)]
enum Val {
    S(f64),
    V([f64; 3]),
}

impl Val {
    fn v(self) -> [f64; 3] {
        match self {
            Val::S(s) => [s, s, s],
            Val::V(v) => v,
        }
    }
    fn s(self) -> f64 {
        match self {
            Val::S(s) => s,
            Val::V(v) => (v[0] + v[1] + v[2]) / 3.0,
        }
    }
    fn map2(self, o: Val, f: impl Fn(f64, f64) -> f64) -> Val {
        match (self, o) {
            (Val::S(a), Val::S(b)) => Val::S(f(a, b)),
            (a, b) => {
                let (a, b) = (a.v(), b.v());
                Val::V([f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])])
            }
        }
    }
    fn map(self, f: impl Fn(f64) -> f64) -> Val {
        match self {
            Val::S(a) => Val::S(f(a)),
            Val::V(a) => Val::V([f(a[0]), f(a[1]), f(a[2])]),
        }
    }
}

#[derive(Clone, Debug)]
enum Expr {
    Num(f64),
    Var(String),
    Vec(Vec<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(String, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        loop {
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.src[self.pos..].starts_with(b"#") {
                while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }
    fn peek(&mut self, s: &str) -> bool {
        self.ws();
        self.src[self.pos..].starts_with(s.as_bytes())
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.peek(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> std::result::Result<(), String> {
        if self.eat(s) { Ok(()) } else { Err(format!("expected `{s}` at {}", self.pos)) }
    }
    fn ident(&mut self) -> Option<String> {
        self.ws();
        let start = self.pos;
        while self.pos < self.src.len() && (self.src[self.pos].is_ascii_alphanumeric() || self.src[self.pos] == b'_') {
            self.pos += 1;
        }
        if self.pos == start || self.src[start].is_ascii_digit() {
            self.pos = start;
            return None;
        }
        Some(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned())
    }
    fn number(&mut self) -> Option<f64> {
        self.ws();
        let start = self.pos;
        while self.pos < self.src.len() && (self.src[self.pos].is_ascii_digit() || self.src[self.pos] == b'.') {
            self.pos += 1;
        }
        if self.pos < self.src.len() && (self.src[self.pos] == b'e' || self.src[self.pos] == b'E') {
            let save = self.pos;
            self.pos += 1;
            if self.pos < self.src.len() && (self.src[self.pos] == b'-' || self.src[self.pos] == b'+') {
                self.pos += 1;
            }
            if self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
            } else {
                self.pos = save;
            }
        }
        if self.pos == start {
            return None;
        }
        std::str::from_utf8(&self.src[start..self.pos]).ok()?.parse().ok()
    }

    /// `$name = expr;`* expr
    fn program(&mut self) -> std::result::Result<(Vec<(String, Expr)>, Expr), String> {
        let mut assigns = Vec::new();
        loop {
            self.ws();
            let save = self.pos;
            if self.eat("$") {
                if let Some(name) = self.ident()
                    && self.eat("=")
                    && !self.peek("=")
                {
                    let e = self.expr()?;
                    self.expect(";")?;
                    assigns.push((name, e));
                    continue;
                }
                self.pos = save;
            }
            break;
        }
        let e = self.expr()?;
        self.ws();
        let _ = self.eat(";");
        self.ws();
        if self.pos != self.src.len() {
            return Err(format!("unexpected input at {}", self.pos));
        }
        Ok((assigns, e))
    }

    fn expr(&mut self) -> std::result::Result<Expr, String> {
        let c = self.or()?;
        if self.eat("?") {
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.expr()?;
            return Ok(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }
    fn or(&mut self) -> std::result::Result<Expr, String> {
        let mut a = self.and()?;
        while self.eat("||") {
            a = Expr::Bin("||".into(), Box::new(a), Box::new(self.and()?));
        }
        Ok(a)
    }
    fn and(&mut self) -> std::result::Result<Expr, String> {
        let mut a = self.cmp()?;
        while self.eat("&&") {
            a = Expr::Bin("&&".into(), Box::new(a), Box::new(self.cmp()?));
        }
        Ok(a)
    }
    fn cmp(&mut self) -> std::result::Result<Expr, String> {
        let mut a = self.add()?;
        loop {
            let op = ["==", "!=", "<=", ">=", "<", ">"].iter().find(|o| self.peek(o)).copied();
            let Some(op) = op else { break };
            self.pos += op.len();
            a = Expr::Bin(op.into(), Box::new(a), Box::new(self.add()?));
        }
        Ok(a)
    }
    fn add(&mut self) -> std::result::Result<Expr, String> {
        let mut a = self.mul()?;
        loop {
            if self.eat("+") {
                a = Expr::Bin("+".into(), Box::new(a), Box::new(self.mul()?));
            } else if self.eat("-") {
                a = Expr::Bin("-".into(), Box::new(a), Box::new(self.mul()?));
            } else {
                break;
            }
        }
        Ok(a)
    }
    fn mul(&mut self) -> std::result::Result<Expr, String> {
        let mut a = self.unary()?;
        loop {
            if self.eat("*") {
                a = Expr::Bin("*".into(), Box::new(a), Box::new(self.unary()?));
            } else if self.eat("/") {
                a = Expr::Bin("/".into(), Box::new(a), Box::new(self.unary()?));
            } else if self.eat("%") {
                a = Expr::Bin("%".into(), Box::new(a), Box::new(self.unary()?));
            } else {
                break;
            }
        }
        Ok(a)
    }
    fn unary(&mut self) -> std::result::Result<Expr, String> {
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        let base = self.atom()?;
        if self.eat("^") {
            let e = self.unary()?;
            return Ok(Expr::Bin("^".into(), Box::new(base), Box::new(e)));
        }
        Ok(base)
    }
    fn atom(&mut self) -> std::result::Result<Expr, String> {
        if self.eat("(") {
            let e = self.expr()?;
            self.expect(")")?;
            return Ok(e);
        }
        if self.eat("[") {
            let mut items = vec![self.expr()?];
            while self.eat(",") {
                items.push(self.expr()?);
            }
            self.expect("]")?;
            return Ok(Expr::Vec(items));
        }
        if self.eat("$") {
            return self.ident().map(Expr::Var).ok_or_else(|| format!("variable name expected at {}", self.pos));
        }
        if let Some(n) = self.number() {
            return Ok(Expr::Num(n));
        }
        if let Some(name) = self.ident() {
            if self.eat("(") {
                let mut args = Vec::new();
                if !self.peek(")") {
                    args.push(self.expr()?);
                    while self.eat(",") {
                        args.push(self.expr()?);
                    }
                }
                self.expect(")")?;
                return Ok(Expr::Call(name, args));
            }
            return Ok(Expr::Var(name));
        }
        Err(format!("unexpected character at {}", self.pos))
    }
}

struct Env<'a> {
    vars: std::collections::HashMap<String, Val>,
    perm: &'a [u8; 512],
}

fn noise2(p: &[u8; 512], x: f64, y: f64) -> f64 {
    (simplex2(p, x, y) + 1.0) * 0.5
}

fn eval(e: &Expr, env: &Env) -> std::result::Result<Val, String> {
    Ok(match e {
        Expr::Num(n) => Val::S(*n),
        Expr::Var(v) => *env.vars.get(v).ok_or_else(|| format!("unknown variable ${v}"))?,
        Expr::Vec(items) => {
            let vals: Vec<f64> = items.iter().map(|i| eval(i, env).map(Val::s)).collect::<std::result::Result<_, _>>()?;
            match vals.len() {
                1 => Val::S(vals[0]),
                2 => Val::V([vals[0], vals[1], 0.0]),
                _ => Val::V([vals[0], vals[1], vals[2]]),
            }
        }
        Expr::Neg(a) => eval(a, env)?.map(|x| -x),
        Expr::Not(a) => Val::S(if eval(a, env)?.s() != 0.0 { 0.0 } else { 1.0 }),
        Expr::Cond(c, a, b) => {
            if eval(c, env)?.s() != 0.0 {
                eval(a, env)?
            } else {
                eval(b, env)?
            }
        }
        Expr::Bin(op, a, b) => {
            let (a, b) = (eval(a, env)?, eval(b, env)?);
            let bool_ = |x: bool| Val::S(if x { 1.0 } else { 0.0 });
            match op.as_str() {
                "+" => a.map2(b, |x, y| x + y),
                "-" => a.map2(b, |x, y| x - y),
                "*" => a.map2(b, |x, y| x * y),
                "/" => a.map2(b, |x, y| if y != 0.0 { x / y } else { 0.0 }),
                "%" => a.map2(b, |x, y| if y != 0.0 { x.rem_euclid(y) } else { 0.0 }),
                "^" => a.map2(b, f64::powf),
                "==" => bool_(a.s() == b.s()),
                "!=" => bool_(a.s() != b.s()),
                "<" => bool_(a.s() < b.s()),
                ">" => bool_(a.s() > b.s()),
                "<=" => bool_(a.s() <= b.s()),
                ">=" => bool_(a.s() >= b.s()),
                "&&" => bool_(a.s() != 0.0 && b.s() != 0.0),
                "||" => bool_(a.s() != 0.0 || b.s() != 0.0),
                o => return Err(format!("unknown operator {o}")),
            }
        }
        Expr::Call(name, args) => {
            let vals: Vec<Val> = args.iter().map(|a| eval(a, env)).collect::<std::result::Result<_, _>>()?;
            let arg = |i: usize| vals.get(i).copied().ok_or_else(|| format!("{name}: missing argument {}", i + 1));
            let f1 = |f: fn(f64) -> f64| -> std::result::Result<Val, String> { Ok(arg(0)?.map(f)) };
            match name.as_str() {
                "sin" => f1(f64::sin)?,
                "cos" => f1(f64::cos)?,
                "tan" => f1(f64::tan)?,
                "asin" => f1(f64::asin)?,
                "acos" => f1(f64::acos)?,
                "atan" => f1(f64::atan)?,
                "sinh" => f1(f64::sinh)?,
                "cosh" => f1(f64::cosh)?,
                "tanh" => f1(f64::tanh)?,
                "abs" => f1(f64::abs)?,
                "floor" => f1(f64::floor)?,
                "ceil" => f1(f64::ceil)?,
                "round" => f1(f64::round)?,
                "trunc" => f1(f64::trunc)?,
                "sqrt" => f1(|x| x.max(0.0).sqrt())?,
                "exp" => f1(f64::exp)?,
                "log" => f1(|x| if x > 0.0 { x.ln() } else { 0.0 })?,
                "log10" => f1(|x| if x > 0.0 { x.log10() } else { 0.0 })?,
                "deg" => f1(f64::to_degrees)?,
                "rad" => f1(f64::to_radians)?,
                "fract" => f1(|x| x - x.floor())?,
                "atan2" => arg(0)?.map2(arg(1)?, f64::atan2),
                "pow" => arg(0)?.map2(arg(1)?, f64::powf),
                "fmod" => arg(0)?.map2(arg(1)?, |x, y| if y != 0.0 { x % y } else { 0.0 }),
                "min" => vals.iter().copied().reduce(|a, b| a.map2(b, f64::min)).ok_or("min: no arguments")?,
                "max" => vals.iter().copied().reduce(|a, b| a.map2(b, f64::max)).ok_or("max: no arguments")?,
                "clamp" => {
                    let (x, lo, hi) = (arg(0)?, arg(1)?, arg(2)?);
                    x.map2(lo, f64::max).map2(hi, f64::min)
                }
                "mix" | "lerp" => {
                    let (a, b, t) = (arg(0)?, arg(1)?, arg(2)?.s());
                    a.map2(b, |x, y| x + (y - x) * t)
                }
                "smoothstep" => {
                    let (a, b, x) = (arg(0)?.s(), arg(1)?.s(), arg(2)?);
                    x.map(|x| {
                        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
                        t * t * (3.0 - 2.0 * t)
                    })
                }
                "step" => {
                    let (edge, x) = (arg(0)?.s(), arg(1)?);
                    x.map(|x| if x < edge { 0.0 } else { 1.0 })
                }
                "fit" => {
                    let (x, a1, b1, a2, b2) = (arg(0)?, arg(1)?.s(), arg(2)?.s(), arg(3)?.s(), arg(4)?.s());
                    x.map(|x| if (b1 - a1).abs() < 1e-12 { a2 } else { a2 + (x - a1) / (b1 - a1) * (b2 - a2) })
                }
                "length" => {
                    let v = arg(0)?.v();
                    Val::S((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt())
                }
                "dist" => {
                    let (a, b) = (arg(0)?.v(), arg(1)?.v());
                    Val::S(((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt())
                }
                "dot" => {
                    let (a, b) = (arg(0)?.v(), arg(1)?.v());
                    Val::S(a[0] * b[0] + a[1] * b[1] + a[2] * b[2])
                }
                "norm" => {
                    let v = arg(0)?.v();
                    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
                    Val::V([v[0] / l, v[1] / l, v[2] / l])
                }
                "hsi" | "hsitorgb" | "hsv" => {
                    let c = arg(0)?.v();
                    let (h, s, v) = (c[0].rem_euclid(1.0) * 6.0, c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0));
                    let i = h.floor();
                    let f = h - i;
                    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
                    Val::V(match i as i32 % 6 {
                        0 => [v, t, p],
                        1 => [q, v, p],
                        2 => [p, v, t],
                        3 => [p, q, v],
                        4 => [t, p, v],
                        _ => [v, p, q],
                    })
                }
                "noise" | "snoise" => {
                    let v = arg(0)?.v();
                    let (x, y) = if vals.len() >= 2 { (vals[0].s(), vals[1].s()) } else { (v[0], v[1]) };
                    Val::S(if name == "snoise" { simplex2(env.perm, x, y) } else { noise2(env.perm, x, y) })
                }
                "cnoise" => {
                    let v = arg(0)?.v();
                    Val::V([noise2(env.perm, v[0], v[1]), noise2(env.perm, v[0] + 31.7, v[1] + 17.3), noise2(env.perm, v[0] - 47.1, v[1] + 59.9)])
                }
                "fbm" => {
                    let v = arg(0)?.v();
                    let octaves = vals.get(1).map_or(6.0, |o| o.s()).clamp(1.0, 12.0) as u32;
                    let lacunarity = vals.get(2).map_or(2.0, |o| o.s());
                    let gain = vals.get(3).map_or(0.5, |o| o.s());
                    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
                    for _ in 0..octaves {
                        sum += amp * simplex2(env.perm, v[0] * f, v[1] * f);
                        norm += amp;
                        amp *= gain;
                        f *= lacunarity;
                    }
                    Val::S((sum / norm + 1.0) * 0.5)
                }
                "turbulence" => {
                    let v = arg(0)?.v();
                    let octaves = vals.get(1).map_or(6.0, |o| o.s()).clamp(1.0, 12.0) as u32;
                    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
                    for _ in 0..octaves {
                        sum += amp * simplex2(env.perm, v[0] * f, v[1] * f).abs();
                        norm += amp;
                        amp *= 0.5;
                        f *= 2.0;
                    }
                    Val::S(sum / norm)
                }
                "voronoi" | "cellnoise" => {
                    let v = arg(0)?.v();
                    let (cx, cy) = (v[0].floor(), v[1].floor());
                    let mut best = f64::MAX;
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let (gx, gy) = (cx + f64::from(dx), cy + f64::from(dy));
                            let (jx, jy) = (noise2(env.perm, gx * 7.1 + 3.3, gy * 5.7 + 1.1), noise2(env.perm, gx * 3.9 - 2.2, gy * 9.3 + 4.4));
                            let d = ((gx + jx - v[0]).powi(2) + (gy + jy - v[1]).powi(2)).sqrt();
                            best = best.min(d);
                        }
                    }
                    Val::S(best.min(1.0))
                }
                "hash" | "rand" => {
                    let v = arg(0)?.v();
                    Val::S(noise2(env.perm, v[0] * 113.1 + 7.7, v[1] * 71.3 + 3.1))
                }
                "gamma" => {
                    let (x, g) = (arg(0)?, arg(1)?.s());
                    x.map(|x| x.max(0.0).powf(1.0 / g.max(1e-6)))
                }
                "bias" => {
                    let (x, b) = (arg(0)?, arg(1)?.s());
                    x.map(|x| x.max(0.0).powf((b.clamp(1e-6, 1.0 - 1e-6)).ln() / 0.5f64.ln()))
                }
                "contrast" => {
                    let (x, c) = (arg(0)?, arg(1)?.s());
                    x.map(|x| ((x - 0.5) * (1.0 + c) + 0.5).clamp(0.0, 1.0))
                }
                "invert" => arg(0)?.map(|x| 1.0 - x),
                "vec" => Val::V(arg(0)?.v()),
                o => return Err(format!("unknown function {o}()")),
            }
        }
    })
}

/// Parse a script; `Err` names the first problem.
fn compile(script: &str) -> std::result::Result<(Vec<(String, Expr)>, Expr), String> {
    let mut p = Parser { src: script.as_bytes(), pos: 0 };
    p.program()
}

fn script_fill(px: &mut [[f32; 4]], w: u32, h: u32, script: &str, time: f64) -> Result<()> {
    let (assigns, body) = compile(script).map_err(|e| EngineError::Other(format!("script: {e}")))?;
    let perm = perm(7);
    let mut env = Env { vars: std::collections::HashMap::new(), perm: &perm };
    env.vars.insert("w".into(), Val::S(f64::from(w)));
    env.vars.insert("h".into(), Val::S(f64::from(h)));
    env.vars.insert("t".into(), Val::S(time));
    env.vars.insert("time".into(), Val::S(time));
    env.vars.insert("PI".into(), Val::S(std::f64::consts::PI));
    for name in ["u", "v", "x", "y", "P"] {
        env.vars.insert(name.into(), Val::S(0.0));
    }
    for y in 0..h {
        for x in 0..w {
            let (u, v) = ((f64::from(x) + 0.5) / f64::from(w), (f64::from(y) + 0.5) / f64::from(h));
            for (name, val) in [("u", Val::S(u)), ("v", Val::S(v)), ("x", Val::S(f64::from(x))), ("y", Val::S(f64::from(y))), ("P", Val::V([u, v, 0.0]))] {
                if let Some(slot) = env.vars.get_mut(name) {
                    *slot = val;
                }
            }
            for (name, e) in &assigns {
                let val = eval(e, &env).map_err(|e| EngineError::Other(format!("script: {e}")))?;
                env.vars.insert(name.clone(), val);
            }
            let out = eval(&body, &env).map_err(|e| EngineError::Other(format!("script: {e}")))?;
            let c = out.v();
            px[(y * w + x) as usize] = [c[0].clamp(0.0, 1.0) as f32, c[1].clamp(0.0, 1.0) as f32, c[2].clamp(0.0, 1.0) as f32, 1.0];
        }
    }
    Ok(())
}

// ---------- commands ----------

/// The generator `p` describes (with `generator` naming the kind), defaults filled in.
fn generator_param(p: &Value, cmd: &str, kind: Option<&str>) -> Result<Generator> {
    let mut v = p.clone();
    if let Some(k) = kind {
        v["generator"] = json!(k);
    }
    if v.get("generator").is_none() {
        return Err(bad(cmd, "needs `generator` (screentone, simplexNoise, multigrid, script)"));
    }
    for key in ["foreground", "background", "lineColor", "color1", "color2"] {
        if v.get(key).is_some() {
            let c = crate::commands::color_param(p, key, [0.0, 0.0, 0.0, 1.0]);
            v[key] = json!(c);
        }
    }
    serde_json::from_value(v).map_err(|e| bad(cmd, format!("{e}")))
}

/// Layer › New Fill Layer › <generator>: a Pattern Fill layer with the rendered pattern.
fn new_generated(s: &mut Session, p: &Value, kind: &str) -> Result<Value> {
    let cmd = "layer.newFillLayer.generated";
    let g = generator_param(p, cmd, Some(kind))?;
    let canvas = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    let pat = render(&g, canvas)?;
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let id = s.edit(&format!("New {} Fill Layer", g.name()), |doc, active| {
        crate::pattern_cmds::ensure_in_doc(doc, &pat);
        let fill = Fill::Pattern { name: pat.name.clone(), scale: 1.0, id: pat.id.clone(), angle: 0.0, link: false, phase: (0.0, 0.0) };
        let mut l = Layer::new(name.clone().unwrap_or_else(|| doc.next_layer_name(g.name())), LayerContent::Fill(fill));
        crate::commands::mask_new_layer(doc, &mut l, p, cmd)?;
        let id = doc.insert_above(*active, l);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "generator": g}))
}

/// The generator of a generated fill layer.
pub fn generator_of(l: &Layer) -> Option<Generator> {
    match &l.content {
        LayerContent::Fill(Fill::Pattern { id, .. }) => id.strip_prefix("gen:").and_then(|j| serde_json::from_str(j).ok()),
        _ => None,
    }
}

/// Edit a generated fill layer's parameters: the given fields replace the layer's.
fn edit_generated(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.fillLayer.generated.edit";
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let cur = generator_of(l).ok_or_else(|| bad(cmd, "the layer is not a generated fill layer"))?;
    let mut merged = serde_json::to_value(&cur).unwrap_or_default();
    if let (Some(m), Some(o)) = (merged.as_object_mut(), p.as_object()) {
        for (k, v) in o {
            if k != "layer" {
                m.insert(k.clone(), v.clone());
            }
        }
    }
    let g = generator_param(&merged, cmd, None)?;
    let pat = render(&g, st.doc.bounds())?;
    s.edit(&format!("Edit {} Fill", g.name()), |doc, _| {
        crate::pattern_cmds::ensure_in_doc(doc, &pat);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if let LayerContent::Fill(Fill::Pattern { name, id: pid, .. }) = &mut l.content {
            *name = pat.name.clone();
            *pid = pat.id.clone();
        }
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "generator": g}))
}

/// Re-render every generated fill layer for the current canvas (after a canvas size change).
fn regenerate(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = st.doc.bounds();
    let todo: Vec<(redrob_photo_doc::LayerId, Pattern)> = st
        .doc
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| generator_of(l).map(|g| (l.id, g)))
        .map(|(id, g)| render(&g, canvas).map(|p| (id, p)))
        .collect::<Result<_>>()?;
    let n = todo.len();
    s.edit("Regenerate Fill Layers", |doc, _| {
        for (id, pat) in &todo {
            crate::pattern_cmds::ensure_in_doc(doc, pat);
            if let Some(l) = doc.layer_mut(*id)
                && let LayerContent::Fill(Fill::Pattern { id: pid, .. }) = &mut l.content
            {
                *pid = pat.id.clone();
            }
        }
        Ok(())
    })?;
    Ok(json!({"regenerated": n}))
}

/// Check a script and render a small preview (base64 PNG) without touching the document.
fn script_preview(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.fillLayer.script.preview";
    let script = p.get("script").and_then(Value::as_str).ok_or_else(|| bad(cmd, "needs `script`"))?;
    compile(script).map_err(|e| bad(cmd, e))?;
    let size = p.get("size").and_then(Value::as_u64).unwrap_or(128).clamp(8, 512) as i32;
    let pat =
        render(&Generator::Script { script: script.to_string(), time: p.get("time").and_then(Value::as_f64).unwrap_or(0.0) }, Rect::new(0, 0, size, size))?;
    let mut doc = redrob_photo_doc::Document::new("preview", redrob_photo_doc::Size::new(pat.width, pat.height), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new("p", LayerContent::Raster(pat.surface.clone())));
    let png =
        redrob_photo_io::export(&doc, "png", &redrob_photo_io::ExportOptions::default()).map(|r| r.bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    Ok(json!({"ok": true, "width": pat.width, "height": pat.height, "png": crate::anim_cmds::base64(&png)}))
}

/// Saved scripts travel with the document (`redrob.scripts`), with a few built-ins.
const BUILTIN_SCRIPTS: [(&str, &str); 4] = [
    ("Gradient", "[$u, $v, 0.5]"),
    ("Checker", "$c = (floor($u * 8) + floor($v * 8)) % 2; [$c, $c, $c]"),
    ("Clouds", "$n = fbm([$u * 4, $v * 4], 6); [$n, $n, 1]"),
    ("Rings", "$d = dist([$u, $v, 0], [0.5, 0.5, 0]); $r = 0.5 + 0.5 * sin($d * 60); [$r, $r * 0.6, 0.2]"),
];

fn scripts_of(doc: &redrob_photo_doc::Document) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = BUILTIN_SCRIPTS.iter().map(|(n, s)| (n.to_string(), s.to_string())).collect();
    let saved: Vec<(String, String)> = crate::layer_meta::doc_get(doc, "scripts").unwrap_or_default();
    for (n, s) in saved {
        v.retain(|(k, _)| *k != n);
        v.push((n, s));
    }
    v
}

fn script_save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.fillLayer.script.save";
    let name = p.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).ok_or_else(|| bad(cmd, "needs `name`"))?.to_string();
    let script = p.get("script").and_then(Value::as_str).ok_or_else(|| bad(cmd, "needs `script`"))?.to_string();
    compile(&script).map_err(|e| bad(cmd, e))?;
    s.edit("Save Script", |doc, _| {
        let mut saved: Vec<(String, String)> = crate::layer_meta::doc_get(doc, "scripts").unwrap_or_default();
        saved.retain(|(k, _)| *k != name);
        saved.push((name.clone(), script.clone()));
        crate::layer_meta::doc_set(doc, "scripts", &saved);
        Ok(())
    })?;
    Ok(json!({"saved": name}))
}

fn script_list(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let list: Vec<Value> = scripts_of(&st.doc).into_iter().map(|(n, sc)| json!({"name": n, "script": sc})).collect();
    Ok(json!({"scripts": list}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $params:expr, $en:expr, $journal:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $en, journal: $journal, run: $run }
        };
    }
    const NEW: &[&str] = &["Layer", "New Fill Layer"];
    vec![
        spec!(
            "layer.newFillLayer.screentone",
            "Screentone…",
            NEW,
            r##"{"pattern":"dots"|"lines"=dots,"shape":"round"|"ellipse"|"diamond"|"square"|"line"=round,"size":px=10,"rotation":degrees=0,"interpolation":"linear"|"sinusoidal"=linear,"brightness":-100..100=0,"contrast":-100..100=0,"foreground":"#000000"?,"background":"#ffffff"?,"invert":bool?,"name":str?}"##,
            has_doc,
            true,
            |s, p| new_generated(s, p, "screentone")
        ),
        spec!(
            "layer.newFillLayer.simplexNoise",
            "Simplex Noise…",
            NEW,
            r##"{"frequency":number=25,"ratio":number=1 (vertical/horizontal),"seed":int=0,"seamless":bool=true,"octaves":1..8=1,"name":str?}"##,
            has_doc,
            true,
            |s, p| new_generated(s, p, "simplexNoise")
        ),
        spec!(
            "layer.newFillLayer.multigrid",
            "Multigrid…",
            NEW,
            r##"{"dimensions":3..12=5 (5: Penrose-like),"divisions":1..40=4,"offset":number=0.2,"lineWidth":px=2,"lineColor":"#000000"?,"color1":"#e6e6e6"?,"color2":"#99b3e6"?,"colorBy":"shape"|"parity"=shape,"scale":number=1,"name":str?}"##,
            has_doc,
            true,
            |s, p| new_generated(s, p, "multigrid")
        ),
        spec!(
            "layer.newFillLayer.script",
            "Script…",
            NEW,
            r##"{"script":"$n = fbm([$u*4,$v*4]); [$n,$n,1]","time":number=0,"name":str?} (variables $u $v $x $y $w $h $t $P; functions sin cos abs floor pow min max clamp mix smoothstep fit length dist noise cnoise fbm turbulence voronoi hsi …)"##,
            has_doc,
            true,
            |s, p| new_generated(s, p, "script")
        ),
        spec!(
            "layer.fillLayer.generated.edit",
            "Edit Generated Fill…",
            &["Layer"],
            r##"{"layer":id?, …the generator's fields}"##,
            has_generated,
            true,
            edit_generated
        ),
        spec!("layer.fillLayer.generated.regenerate", "Regenerate Fill Layers", &[], "{}", has_doc, true, regenerate),
        spec!(
            "layer.fillLayer.script.preview",
            "Preview Script",
            &[],
            r##"{"script":str,"size":8..512=128,"time":number?} → {ok,png (base64)}"##,
            has_doc,
            false,
            script_preview
        ),
        spec!("layer.fillLayer.script.save", "Save Script…", &[], r##"{"name":str,"script":str}"##, has_doc, true, script_save),
        spec!("layer.fillLayer.script.list", "Saved Scripts", &[], "{} → {scripts:[{name,script}]}", has_doc, false, script_list),
    ]
}
