//! Flat vector outputs written from the metafile crate's drawing operations: HTML5 canvas
//! (`canvas`), Flash FXG (`fxg`), POV-Ray (`pov`), Synfig (`sif`), XAML (`xaml`), OpenDocument
//! Drawing (`odg`), TikZ LaTeX (`tex`), PDF + LaTeX overlay (`pdftex`) and HPGL plotter files
//! (`hpgl`). Each writes one artboard (default the first) or, with `useArtboards`, one file per
//! chosen artboard. Type is written as glyph outlines (PDF + LaTeX keeps it for LaTeX to set),
//! live effects are applied first and what a format can't hold comes back as a warning.

use std::fmt::Write as _;

use redrob_vector_doc::{Document, LineCap, LineJoin, NodeKind};
use redrob_vector_geom::{Affine, BezPath, FillRule, PathEl, Point, Rect};
use redrob_vector_metafile::{Op, Pen};
use serde_json::Value;

use super::super::*;
use super::{ArtboardPick, Encoded, FormatOption};
use crate::EngineError;

const C: &str = "document.export";

/// Points per millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;

/// The options `document.formats` lists for the flat text formats.
pub(super) const OPTIONS: &[FormatOption] = &[super::ARTBOARD, super::ARTBOARDS, super::RANGE, super::USE_ARTBOARDS];

pub(super) const XAML_OPTIONS: &[FormatOption] = &[
    super::ARTBOARD,
    super::ARTBOARDS,
    super::RANGE,
    super::USE_ARTBOARDS,
    FormatOption {
        name: "output",
        ty: "string",
        default: "\"canvas\"",
        description: "canvas (a Canvas of Path elements, for Silverlight and WPF pages) or drawingBrush (a DrawingBrush of GeometryDrawings, for resources)",
    },
];

pub(super) const TEX_OPTIONS: &[FormatOption] = &[
    super::ARTBOARD,
    super::ARTBOARDS,
    super::RANGE,
    super::USE_ARTBOARDS,
    FormatOption {
        name: "standalone",
        ty: "boolean",
        default: "false",
        description: "wrap the tikzpicture in a standalone document (\\documentclass{standalone}) so it compiles by itself",
    },
];

pub(super) const HPGL_OPTIONS: &[FormatOption] = &[
    super::ARTBOARD,
    super::ARTBOARDS,
    super::RANGE,
    super::USE_ARTBOARDS,
    FormatOption { name: "resolution", ty: "number", default: "1016", description: "plotter units per inch (1016 = 40 per mm, most plotters)" },
    FormatOption {
        name: "pen",
        ty: "integer",
        default: "1",
        description: "the pen number every path is drawn with (0: one pen per stroke colour, 1–8)",
    },
    FormatOption {
        name: "overcut",
        ty: "number",
        default: "0",
        description: "mm to draw past the start of closed paths, so cutters close them cleanly",
    },
    FormatOption {
        name: "toolOffset",
        ty: "number",
        default: "0",
        description: "mm the knife trails its holder: corners get a small arc so the blade turns",
    },
    FormatOption {
        name: "precut",
        ty: "boolean",
        default: "false",
        description: "draw a short line before the first path so a drag knife faces the right way",
    },
    FormatOption { name: "fills", ty: "boolean", default: "true", description: "plot the outlines of filled shapes too (false: strokes only)" },
    FormatOption { name: "mirrorX", ty: "boolean", default: "false", description: "mirror the plot left to right" },
    FormatOption { name: "mirrorY", ty: "boolean", default: "false", description: "mirror the plot top to bottom" },
    FormatOption { name: "flat", ty: "number", default: "0.1", description: "curve flatness in mm (smaller: more, shorter segments)" },
];

/// The artboards an export writes: with `use_artboards` every chosen one (default all), else the one chosen.
fn boards(doc: &Document, f: &super::Format, p: &Value, use_artboards: Option<bool>) -> Result<Vec<usize>> {
    let pick: ArtboardPick = super::encode::options(f, p)?;
    let n = doc.artboards.len();
    Ok(match use_artboards {
        Some(true) => pick.resolve(n).map_err(|e| bad(C, e))?.unwrap_or_else(|| (0..n).collect()),
        _ => vec![pick.one(n).map_err(|e| bad(C, e))?],
    })
}

/// Encode `doc` as flat format `id` (one of this module's), as [`super::metafile::encode`] does.
pub(super) fn encode(doc: &Document, id: &str, p: &Value, use_artboards: Option<bool>) -> Result<Encoded> {
    let f = super::format(id).ok_or_else(|| bad(C, format!("no format {id}")))?;
    let boards = boards(doc, f, p, use_artboards)?;
    let flat = crate::flatten_raster_effects(doc);
    let src = flat.as_ref().unwrap_or(doc);
    let mut enc = Encoded::default();
    for b in boards {
        let region = doc.artboards.get(b).ok_or_else(|| bad(C, format!("no artboard {}", b + 1)))?.rect.abs();
        let clips = matches!(id, "canvas" | "fxg" | "xaml" | "tex");
        let (ops, mut warnings) = redrob_vector_metafile::operations(src, region, clips).map_err(|e| bad(C, e))?;
        let mut w = Writer { region, warnings: &mut warnings };
        let bytes = match id {
            "canvas" => w.canvas(&ops, &doc.title),
            "fxg" => w.fxg(&ops),
            "pov" => w.pov(&ops),
            "sif" => w.sif(&ops, &doc.title),
            "xaml" => w.xaml(&ops, str_param(p, "output").unwrap_or("canvas"))?,
            "odg" => w.odg(&ops)?,
            "tex" => w.tex(&ops, p.get("standalone").and_then(Value::as_bool).unwrap_or(false)),
            "hpgl" => w.hpgl(&ops, p)?,
            _ => return Err(bad(C, format!("{id} isn't a flat vector format"))),
        };
        enc.files.push((Some(b), bytes));
        for w in warnings {
            if !enc.warnings.contains(&w) {
                enc.warnings.push(w);
            }
        }
    }
    if enc.files.is_empty() {
        return Err(bad(C, "the document has no artboard"));
    }
    Ok(enc)
}

/// PDF + LaTeX: the artboard as a PDF without its type, and a `.tex` overlay that places each
/// text object with `\put` so LaTeX sets it in the document's fonts. The `.tex` is the file,
/// the PDF is written beside it under the document's name.
pub(super) fn encode_pdftex(doc: &Document, p: &Value) -> Result<Encoded> {
    let f = super::format("pdftex").ok_or_else(|| bad(C, "no pdftex format"))?;
    let board = boards(doc, f, p, None)?[0];
    let region = doc.artboards.get(board).ok_or_else(|| bad(C, format!("no artboard {}", board + 1)))?.rect.abs();
    // The PDF of the art without its type.
    let mut without = doc.clone();
    let mut texts: Vec<(Point, String, f64)> = vec![];
    doc.walk(|n| {
        if let NodeKind::Text(t) = &n.kind
            && n.visible
        {
            let s = t.plain_text();
            if !s.trim().is_empty() {
                let size = t.runs.first().map(|r| r.style.size).unwrap_or(12.0);
                texts.push((t.xf * Point::ZERO, s, size));
            }
        }
    });
    hide_text(&mut without);
    let mut q = p.clone();
    if let Some(o) = q.as_object_mut() {
        o.insert("artboards".into(), serde_json::json!([board]));
        o.remove("range");
    }
    let (pdf, mut warnings) = super::pdf::encode(C, &without, &q)?;
    let stem = safe_stem(&doc.title);
    let pdf_name = format!("{stem}.pdf");
    let mut tex = String::new();
    let _ = writeln!(
        tex,
        "%% {stem}.tex: the overlay for {pdf_name}; include with \\input{{{stem}.tex}} after \\usepackage{{graphicx}} \\usepackage{{color}}"
    );
    let _ = writeln!(tex, "\\begingroup%");
    let _ = writeln!(tex, "  \\makeatletter%");
    let _ = writeln!(
        tex,
        "  \\providecommand\\color[2][]{{\\errmessage{{(overlay) Color is used, but the color package is not loaded}}\\renewcommand\\color[2][]{{}}}}%"
    );
    let _ = writeln!(tex, "  \\providecommand\\transparent[1]{{\\renewcommand\\transparent[1]{{}}}}%");
    let _ = writeln!(tex, "  \\providecommand\\rotatebox[2]{{#2}}%");
    let _ = writeln!(
        tex,
        "  \\ifx\\svgwidth\\undefined\\setlength{{\\unitlength}}{{{}bp}}\\else\\setlength{{\\unitlength}}{{\\svgwidth}}\\fi%",
        fmt(region.width())
    );
    let _ = writeln!(tex, "  \\begin{{picture}}(1,{})%", fmt(region.height() / region.width().max(1e-9)));
    let _ = writeln!(tex, "    \\put(0,0){{\\includegraphics[width=\\unitlength]{{{pdf_name}}}}}%");
    let w = region.width().max(1e-9);
    for (at, s, size) in &texts {
        let x = (at.x - region.x0) / w;
        let y = (region.y1 - at.y) / w;
        let _ = writeln!(
            tex,
            "    \\put({},{}){{\\makebox(0,0)[lb]{{\\smash{{\\fontsize{{{}}}{{{}}}\\selectfont {}}}}}}}%",
            fmt(x),
            fmt(y),
            fmt(*size),
            fmt(size * 1.2),
            tex_escape(s)
        );
    }
    let _ = writeln!(tex, "  \\end{{picture}}%");
    let _ = writeln!(tex, "\\endgroup%");
    if texts.is_empty() {
        warnings.push("the artboard has no type: the overlay only includes the PDF".into());
    }
    let mut enc = Encoded::one(tex.into_bytes());
    enc.linked.push(redrob_vector_svg::LinkedImage { name: pdf_name, bytes: std::sync::Arc::new(pdf) });
    enc.warnings = warnings;
    Ok(enc)
}

/// Hide every text object of `doc` (so an export leaves the type out).
fn hide_text(doc: &mut Document) {
    let mut ids = vec![];
    doc.walk(|n| {
        if matches!(n.kind, NodeKind::Text(_)) {
            ids.push(n.id);
        }
    });
    for id in ids {
        if let Some(n) = doc.node_mut(id) {
            n.visible = false;
        }
    }
}

/// A file stem from a title: letters, digits, `-` and `_` only.
fn safe_stem(title: &str) -> String {
    let stem = std::path::Path::new(title).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let s: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.is_empty() { "drawing".into() } else { s }
}

fn tex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\textbackslash{}"),
            '{' | '}' | '$' | '&' | '#' | '%' | '_' => {
                out.push('\\');
                out.push(c);
            }
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn fmt(v: f64) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" || s.is_empty() { "0".into() } else { s }
}

fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The pattern of a dashed pen in points (empty: solid).
fn dashes(pen: &Pen) -> Vec<f64> {
    pen.dash.as_ref().map(|d| d.pattern.iter().copied().filter(|v| v.is_finite() && *v >= 0.0).collect()).unwrap_or_default()
}

struct Writer<'w> {
    region: Rect,
    warnings: &'w mut Vec<String>,
}

impl Writer<'_> {
    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    /// Document space to the output: the region's top-left corner at the origin.
    fn map(&self) -> Affine {
        Affine::translate((-self.region.x0, -self.region.y0))
    }

    /// `path` moved to the output space (y down).
    fn local(&self, path: &BezPath) -> BezPath {
        self.map() * path.clone()
    }

    /// SVG / XAML / FXG path data of `path` (output space).
    fn data(&self, path: &BezPath) -> String {
        let mut s = String::new();
        for el in self.local(path).elements() {
            match el {
                PathEl::MoveTo(p) => {
                    let _ = write!(s, "M{} {}", fmt(p.x), fmt(p.y));
                }
                PathEl::LineTo(p) => {
                    let _ = write!(s, "L{} {}", fmt(p.x), fmt(p.y));
                }
                PathEl::QuadTo(a, p) => {
                    let _ = write!(s, "Q{} {} {} {}", fmt(a.x), fmt(a.y), fmt(p.x), fmt(p.y));
                }
                PathEl::CurveTo(a, b, p) => {
                    let _ = write!(s, "C{} {} {} {} {} {}", fmt(a.x), fmt(a.y), fmt(b.x), fmt(b.y), fmt(p.x), fmt(p.y));
                }
                PathEl::ClosePath => s.push('Z'),
            }
        }
        s
    }

    /// `path` flattened into polygons (output space, `tol` points): one point list per subpath,
    /// with whether it closes.
    fn polylines(&self, path: &BezPath, tol: f64) -> Vec<(Vec<Point>, bool)> {
        let mut out: Vec<(Vec<Point>, bool)> = vec![];
        self.local(path).flatten(tol, |el| match el {
            PathEl::MoveTo(p) => out.push((vec![p], false)),
            PathEl::LineTo(p) => {
                if let Some(last) = out.last_mut() {
                    last.0.push(p);
                } else {
                    out.push((vec![p], false));
                }
            }
            PathEl::ClosePath => {
                if let Some(last) = out.last_mut() {
                    last.1 = true;
                }
            }
            _ => {}
        });
        out.retain(|(pts, _)| pts.len() >= 2);
        out
    }

    // ---------- HTML5 canvas ----------

    fn canvas(&mut self, ops: &[Op], title: &str) -> Vec<u8> {
        let (w, h) = (self.region.width(), self.region.height());
        let mut js = String::new();
        let mut imgs: Vec<String> = vec![];
        for op in ops {
            match op {
                Op::Fill { path, rule, rgb } => {
                    let _ = writeln!(js, "  ctx.fillStyle = '{}';", hex(*rgb));
                    let _ = writeln!(
                        js,
                        "  ctx.fill(new Path2D('{}'), '{}');",
                        self.data(path),
                        if *rule == FillRule::EvenOdd { "evenodd" } else { "nonzero" }
                    );
                }
                Op::Stroke { path, pen } => {
                    let _ = writeln!(js, "  ctx.strokeStyle = '{}';", hex(pen.rgb));
                    let _ = writeln!(js, "  ctx.lineWidth = {};", fmt(pen.width));
                    let _ = writeln!(js, "  ctx.lineCap = '{}';", cap_name(pen.cap));
                    let _ = writeln!(js, "  ctx.lineJoin = '{}';", join_name(pen.join));
                    let _ = writeln!(js, "  ctx.miterLimit = {};", fmt(pen.miter.max(1.0)));
                    let d = dashes(pen);
                    let _ = writeln!(js, "  ctx.setLineDash([{}]);", d.iter().map(|v| fmt(*v)).collect::<Vec<_>>().join(", "));
                    let _ = writeln!(js, "  ctx.stroke(new Path2D('{}'));", self.data(path));
                }
                Op::Image { image, xf, opacity } => match image.to_png() {
                    Ok(png) => {
                        let m = (self.map() * *xf).as_coeffs();
                        imgs.push(format!("data:image/png;base64,{}", redrob_vector_svg::base64_encode(&png)));
                        let i = imgs.len() - 1;
                        let _ = writeln!(js, "  ctx.save(); ctx.globalAlpha = {};", fmt(f64::from(*opacity)));
                        let _ = writeln!(
                            js,
                            "  ctx.setTransform({}, {}, {}, {}, {}, {});",
                            fmt(m[0]),
                            fmt(m[1]),
                            fmt(m[2]),
                            fmt(m[3]),
                            fmt(m[4]),
                            fmt(m[5])
                        );
                        let _ = writeln!(js, "  ctx.drawImage(images[{i}], 0, 0, {}, {});", image.width, image.height);
                        let _ = writeln!(js, "  ctx.restore();");
                    }
                    Err(_) => self.warn("an image couldn't be encoded and is left out"),
                },
                Op::Clip { path, rule } => {
                    let _ = writeln!(
                        js,
                        "  ctx.save(); ctx.clip(new Path2D('{}'), '{}');",
                        self.data(path),
                        if *rule == FillRule::EvenOdd { "evenodd" } else { "nonzero" }
                    );
                }
                Op::Unclip => {
                    let _ = writeln!(js, "  ctx.restore();");
                }
            }
        }
        let mut html = String::new();
        let _ = writeln!(html, "<!DOCTYPE html>");
        let _ = writeln!(html, "<html><head><meta charset=\"utf-8\"><title>{}</title></head>", xml_escape(title));
        let _ = writeln!(html, "<body style=\"margin:0\">");
        let _ = writeln!(html, "<canvas id=\"drawing\" width=\"{}\" height=\"{}\"></canvas>", fmt(w.ceil()), fmt(h.ceil()));
        let _ = writeln!(html, "<script>");
        let _ = writeln!(html, "const sources = [{}];", imgs.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", "));
        let _ = writeln!(html, "const images = sources.map(s => {{ const i = new Image(); i.src = s; return i; }});");
        let _ = writeln!(html, "function draw() {{");
        let _ = writeln!(html, "  const ctx = document.getElementById('drawing').getContext('2d');");
        html.push_str(&js);
        let _ = writeln!(html, "}}");
        let _ = writeln!(html, "Promise.all(images.map(i => i.decode().catch(() => {{}}))).then(draw);");
        let _ = writeln!(html, "</script></body></html>");
        html.into_bytes()
    }

    // ---------- FXG ----------

    fn fxg(&mut self, ops: &[Op]) -> Vec<u8> {
        let mut s = String::new();
        let _ = writeln!(s, "<?xml version=\"1.0\" encoding=\"utf-8\"?>");
        let _ = writeln!(
            s,
            "<Graphic xmlns=\"http://ns.adobe.com/fxg/2008\" version=\"2.0\" viewWidth=\"{}\" viewHeight=\"{}\">",
            fmt(self.region.width()),
            fmt(self.region.height())
        );
        let mut depth = 1usize;
        let ind = |d: usize| "  ".repeat(d);
        for op in ops {
            match op {
                Op::Fill { path, rule, rgb } => {
                    let _ = writeln!(
                        s,
                        "{}<Path data=\"{}\" winding=\"{}\"><fill><SolidColor color=\"{}\"/></fill></Path>",
                        ind(depth),
                        self.data(path),
                        if *rule == FillRule::EvenOdd { "evenOdd" } else { "nonZero" },
                        hex(*rgb)
                    );
                }
                Op::Stroke { path, pen } => {
                    let _ = writeln!(
                        s,
                        "{}<Path data=\"{}\"><stroke><SolidColorStroke color=\"{}\" weight=\"{}\" caps=\"{}\" joints=\"{}\" miterLimit=\"{}\"/></stroke></Path>",
                        ind(depth),
                        self.data(path),
                        hex(pen.rgb),
                        fmt(pen.width),
                        match pen.cap {
                            LineCap::Butt => "none",
                            LineCap::Round => "round",
                            LineCap::Square => "square",
                        },
                        join_name(pen.join),
                        fmt(pen.miter.max(1.0))
                    );
                    if !dashes(pen).is_empty() {
                        self.warn("FXG has no dashed strokes: dashes are written solid");
                    }
                }
                Op::Image { .. } => self.warn("FXG images need separate files: images are left out"),
                Op::Clip { path, .. } => {
                    let _ = writeln!(s, "{}<Group maskType=\"clip\">", ind(depth));
                    let _ = writeln!(
                        s,
                        "{}<mask><Group><Path data=\"{}\"><fill><SolidColor color=\"#000000\"/></fill></Path></Group></mask>",
                        ind(depth + 1),
                        self.data(path)
                    );
                    depth += 1;
                }
                Op::Unclip => {
                    depth = depth.saturating_sub(1).max(1);
                    let _ = writeln!(s, "{}</Group>", ind(depth));
                }
            }
        }
        while depth > 1 {
            depth -= 1;
            let _ = writeln!(s, "{}</Group>", ind(depth));
        }
        let _ = writeln!(s, "</Graphic>");
        s.into_bytes()
    }

    // ---------- POV-Ray ----------

    fn pov(&mut self, ops: &[Op]) -> Vec<u8> {
        let (w, h) = (self.region.width(), self.region.height());
        let mut s = String::new();
        let _ = writeln!(s, "// POV-Ray scene: the drawing as prisms in the x–z plane, seen from above (y up).");
        let _ = writeln!(s, "// Each shape sits a little higher than the one below, so they paint in order.");
        let _ = writeln!(s, "#version 3.7;");
        let _ = writeln!(s, "global_settings {{ assumed_gamma 1.0 }}");
        let _ = writeln!(s, "#declare DrawingWidth = {};", fmt(w));
        let _ = writeln!(s, "#declare DrawingHeight = {};", fmt(h));
        let _ = writeln!(
            s,
            "camera {{ orthographic location <{}, 100, {}> look_at <{}, 0, {}> right x*DrawingWidth up z*DrawingHeight }}",
            fmt(w / 2.0),
            fmt(-h / 2.0),
            fmt(w / 2.0),
            fmt(-h / 2.0)
        );
        let _ = writeln!(s, "light_source {{ <{}, 1000, {}> color rgb 1 shadowless }}", fmt(w / 2.0), fmt(-h / 2.0));
        let _ = writeln!(s, "background {{ color rgb 1 }}");
        let mut level = 0.0;
        let mut n = 0usize;
        let step = 0.01;
        for op in ops {
            let (path, rgb) = match op {
                Op::Fill { path, rgb, .. } => (path.clone(), *rgb),
                Op::Stroke { path, pen } => {
                    let style = kurbo::Stroke::new(pen.width.max(0.1))
                        .with_caps(match pen.cap {
                            LineCap::Butt => kurbo::Cap::Butt,
                            LineCap::Round => kurbo::Cap::Round,
                            LineCap::Square => kurbo::Cap::Square,
                        })
                        .with_join(match pen.join {
                            LineJoin::Miter => kurbo::Join::Miter,
                            LineJoin::Round => kurbo::Join::Round,
                            LineJoin::Bevel => kurbo::Join::Bevel,
                        })
                        .with_miter_limit(pen.miter.max(1.0));
                    let d = dashes(pen);
                    let style = if d.is_empty() { style } else { style.with_dashes(0.0, d) };
                    (kurbo::stroke(path.elements().iter().copied(), &style, &kurbo::StrokeOpts::default(), 0.1), pen.rgb)
                }
                Op::Image { .. } => {
                    self.warn("POV-Ray scenes hold no images: images are left out");
                    continue;
                }
                Op::Clip { .. } | Op::Unclip => continue,
            };
            let polys = self.polylines(&path, 0.1);
            let points: usize = polys.iter().map(|(p, _)| p.len() + 1).sum();
            if points < 4 {
                continue;
            }
            n += 1;
            let _ = writeln!(s, "#declare Shape{n} = prism {{");
            let _ = writeln!(s, "  linear_spline {} {} {}", fmt(level), fmt(level + step), points);
            let mut lines = vec![];
            for (pts, _) in &polys {
                for p in pts {
                    lines.push(format!("<{}, {}>", fmt(p.x), fmt(-p.y)));
                }
                // A prism's subpaths close back on their first point.
                lines.push(format!("<{}, {}>", fmt(pts[0].x), fmt(-pts[0].y)));
            }
            let _ = writeln!(s, "  {}", lines.join(", "));
            let _ = writeln!(
                s,
                "  pigment {{ color rgb <{}, {}, {}> }} finish {{ ambient 1 diffuse 0 }}",
                fmt(f64::from(rgb[0]) / 255.0),
                fmt(f64::from(rgb[1]) / 255.0),
                fmt(f64::from(rgb[2]) / 255.0)
            );
            let _ = writeln!(s, "}}");
            let _ = writeln!(s, "object {{ Shape{n} }}");
            level += step;
        }
        if n == 0 {
            self.warn("the artboard has no paths: the scene is empty");
        }
        s.into_bytes()
    }

    // ---------- Synfig ----------

    fn sif(&mut self, ops: &[Op], title: &str) -> Vec<u8> {
        // Synfig units: 60 pixels per unit, y up, the view box centred.
        const PX_PER_UNIT: f64 = 60.0;
        let (w, h) = (self.region.width(), self.region.height());
        let u = |p: Point| ((p.x - w / 2.0) / PX_PER_UNIT, (h / 2.0 - p.y) / PX_PER_UNIT);
        let mut s = String::new();
        let _ = writeln!(s, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
        let _ = writeln!(
            s,
            "<canvas version=\"1.2\" width=\"{}\" height=\"{}\" xres=\"2834.645669\" yres=\"2834.645669\" view-box=\"{} {} {} {}\" antialias=\"1\" fps=\"24\" begin-time=\"0f\" end-time=\"5s\" bgcolor=\"0 0 0 0\">",
            fmt(w.ceil()),
            fmt(h.ceil()),
            fmt(-w / 2.0 / PX_PER_UNIT),
            fmt(h / 2.0 / PX_PER_UNIT),
            fmt(w / 2.0 / PX_PER_UNIT),
            fmt(-h / 2.0 / PX_PER_UNIT)
        );
        let _ = writeln!(s, "  <name>{}</name>", xml_escape(title));
        let mut z = 0usize;
        for op in ops {
            let (path, rgb, stroke) = match op {
                Op::Fill { path, rgb, .. } => (path, *rgb, None),
                Op::Stroke { path, pen } => (path, pen.rgb, Some(pen)),
                Op::Image { .. } => {
                    self.warn("Synfig files link images from files: images are left out");
                    continue;
                }
                Op::Clip { .. } | Op::Unclip => continue,
            };
            let local = self.local(path);
            // One layer per subpath: a bline is one closed or open curve.
            for sub in subpaths(&local) {
                let pts = bline_points(&sub);
                if pts.len() < 2 {
                    continue;
                }
                z += 1;
                let kind = if stroke.is_some() { "outline" } else { "region" };
                let _ = writeln!(s, "  <layer type=\"{kind}\" active=\"true\" exclude_from_rendering=\"false\" version=\"0.3\" desc=\"Shape {z}\">");
                let _ = writeln!(s, "    <param name=\"z_depth\"><real value=\"0\"/></param>");
                let _ = writeln!(s, "    <param name=\"amount\"><real value=\"1\"/></param>");
                let _ = writeln!(s, "    <param name=\"blend_method\"><integer value=\"0\"/></param>");
                let _ = writeln!(
                    s,
                    "    <param name=\"color\"><color><r>{}</r><g>{}</g><b>{}</b><a>1</a></color></param>",
                    fmt(srgb_to_linear(rgb[0])),
                    fmt(srgb_to_linear(rgb[1])),
                    fmt(srgb_to_linear(rgb[2]))
                );
                let _ = writeln!(s, "    <param name=\"origin\"><vector><x>0</x><y>0</y></vector></param>");
                let _ = writeln!(s, "    <param name=\"invert\"><bool value=\"false\"/></param>");
                let _ = writeln!(s, "    <param name=\"antialias\"><bool value=\"true\"/></param>");
                let _ = writeln!(s, "    <param name=\"feather\"><real value=\"0\"/></param>");
                let _ = writeln!(s, "    <param name=\"blurtype\"><integer value=\"1\"/></param>");
                let _ = writeln!(
                    s,
                    "    <param name=\"winding_style\"><integer value=\"{}\"/></param>",
                    if matches!(op, Op::Fill { rule: FillRule::EvenOdd, .. }) { 1 } else { 0 }
                );
                let closed = sub.closed;
                let _ = writeln!(s, "    <param name=\"bline\"><bline type=\"bline_point\" loop=\"{}\">", if closed { "true" } else { "false" });
                for bp in &pts {
                    let (x, y) = u(bp.point);
                    let t1 = ((bp.t_in.x) / PX_PER_UNIT, (-bp.t_in.y) / PX_PER_UNIT);
                    let t2 = ((bp.t_out.x) / PX_PER_UNIT, (-bp.t_out.y) / PX_PER_UNIT);
                    let _ = writeln!(s, "      <entry><composite type=\"bline_point\">");
                    let _ = writeln!(s, "        <point><vector><x>{}</x><y>{}</y></vector></point>", fmt(x), fmt(y));
                    let _ = writeln!(s, "        <width><real value=\"1\"/></width>");
                    let _ = writeln!(s, "        <origin><real value=\"0.5\"/></origin>");
                    let _ = writeln!(s, "        <split><bool value=\"true\"/></split>");
                    let _ = writeln!(s, "        <t1><vector><x>{}</x><y>{}</y></vector></t1>", fmt(t1.0), fmt(t1.1));
                    let _ = writeln!(s, "        <t2><vector><x>{}</x><y>{}</y></vector></t2>", fmt(t2.0), fmt(t2.1));
                    let _ = writeln!(s, "        <split_radius><bool value=\"true\"/></split_radius>");
                    let _ = writeln!(s, "        <split_angle><bool value=\"true\"/></split_angle>");
                    let _ = writeln!(s, "      </composite></entry>");
                }
                let _ = writeln!(s, "    </bline></param>");
                if let Some(pen) = stroke {
                    let _ = writeln!(s, "    <param name=\"width\"><real value=\"{}\"/></param>", fmt(pen.width / PX_PER_UNIT));
                    let _ = writeln!(s, "    <param name=\"expand\"><real value=\"0\"/></param>");
                    let _ = writeln!(
                        s,
                        "    <param name=\"sharp_cusps\"><bool value=\"{}\"/></param>",
                        if pen.join == LineJoin::Miter { "true" } else { "false" }
                    );
                    let round = pen.cap == LineCap::Round;
                    let _ = writeln!(s, "    <param name=\"round_tip[0]\"><bool value=\"{round}\"/></param>");
                    let _ = writeln!(s, "    <param name=\"round_tip[1]\"><bool value=\"{round}\"/></param>");
                    let _ = writeln!(s, "    <param name=\"homogeneous_width\"><bool value=\"true\"/></param>");
                    if !dashes(pen).is_empty() {
                        self.warn("Synfig outlines have no dashes: dashes are written solid");
                    }
                }
                let _ = writeln!(s, "  </layer>");
            }
        }
        let _ = writeln!(s, "</canvas>");
        s.into_bytes()
    }

    // ---------- XAML ----------

    fn xaml(&mut self, ops: &[Op], output: &str) -> Result<Vec<u8>> {
        let brush = match output {
            "canvas" => false,
            "drawingBrush" | "drawing" => true,
            o => return Err(bad(C, format!("output `{o}`: canvas or drawingBrush"))),
        };
        let (w, h) = (self.region.width(), self.region.height());
        let mut s = String::new();
        let mut depth = 1usize;
        let ind = |d: usize| "  ".repeat(d);
        let ns = "xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\" xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"";
        if brush {
            let _ = writeln!(s, "<DrawingBrush {ns} Stretch=\"Uniform\">");
            let _ = writeln!(s, "  <DrawingBrush.Drawing>");
            let _ = writeln!(s, "    <DrawingGroup>");
            let _ = writeln!(
                s,
                "      <GeometryDrawing Geometry=\"F1 M0 0 L{} 0 L{} {} L0 {} Z\"><GeometryDrawing.Brush><SolidColorBrush Color=\"#00FFFFFF\"/></GeometryDrawing.Brush></GeometryDrawing>",
                fmt(w),
                fmt(w),
                fmt(h),
                fmt(h)
            );
            depth = 3;
        } else {
            let _ = writeln!(s, "<Canvas {ns} Width=\"{}\" Height=\"{}\">", fmt(w), fmt(h));
        }
        let rule = |r: FillRule| if r == FillRule::EvenOdd { "F0" } else { "F1" };
        for op in ops {
            match op {
                Op::Fill { path, rule: r, rgb } => {
                    if brush {
                        let _ = writeln!(s, "{}<GeometryDrawing Brush=\"{}\" Geometry=\"{} {}\"/>", ind(depth), hex(*rgb), rule(*r), self.data(path));
                    } else {
                        let _ = writeln!(s, "{}<Path Fill=\"{}\" Data=\"{} {}\"/>", ind(depth), hex(*rgb), rule(*r), self.data(path));
                    }
                }
                Op::Stroke { path, pen } => {
                    let d = dashes(pen);
                    let dash = if d.is_empty() {
                        String::new()
                    } else {
                        // XAML dash arrays are in multiples of the thickness.
                        let unit = pen.width.max(1e-6);
                        format!(" StrokeDashArray=\"{}\"", d.iter().map(|v| fmt(v / unit)).collect::<Vec<_>>().join(" "))
                    };
                    let cap = match pen.cap {
                        LineCap::Butt => "Flat",
                        LineCap::Round => "Round",
                        LineCap::Square => "Square",
                    };
                    let join = match pen.join {
                        LineJoin::Miter => "Miter",
                        LineJoin::Round => "Round",
                        LineJoin::Bevel => "Bevel",
                    };
                    if brush {
                        let _ = writeln!(s, "{}<GeometryDrawing Geometry=\"F1 {}\">", ind(depth), self.data(path));
                        let _ = write!(
                            s,
                            "{}<GeometryDrawing.Pen><Pen Brush=\"{}\" Thickness=\"{}\" StartLineCap=\"{cap}\" EndLineCap=\"{cap}\" DashCap=\"{cap}\" LineJoin=\"{join}\" MiterLimit=\"{}\"",
                            ind(depth + 1),
                            hex(pen.rgb),
                            fmt(pen.width),
                            fmt(pen.miter.max(1.0))
                        );
                        if d.is_empty() {
                            let _ = writeln!(s, "/></GeometryDrawing.Pen>");
                        } else {
                            let unit = pen.width.max(1e-6);
                            let _ = writeln!(
                                s,
                                "><Pen.DashStyle><DashStyle Dashes=\"{}\"/></Pen.DashStyle></Pen></GeometryDrawing.Pen>",
                                d.iter().map(|v| fmt(v / unit)).collect::<Vec<_>>().join(" ")
                            );
                        }
                        let _ = writeln!(s, "{}</GeometryDrawing>", ind(depth));
                    } else {
                        let _ = writeln!(
                            s,
                            "{}<Path Stroke=\"{}\" StrokeThickness=\"{}\" StrokeStartLineCap=\"{cap}\" StrokeEndLineCap=\"{cap}\" StrokeDashCap=\"{cap}\" StrokeLineJoin=\"{join}\" StrokeMiterLimit=\"{}\"{dash} Data=\"F1 {}\"/>",
                            ind(depth),
                            hex(pen.rgb),
                            fmt(pen.width),
                            fmt(pen.miter.max(1.0)),
                            self.data(path)
                        );
                    }
                }
                Op::Image { .. } => self.warn("XAML images are loaded from files: images are left out"),
                Op::Clip { path, rule: r } => {
                    if brush {
                        let _ = writeln!(s, "{}<DrawingGroup ClipGeometry=\"{} {}\">", ind(depth), rule(*r), self.data(path));
                    } else {
                        let _ = writeln!(s, "{}<Canvas>", ind(depth));
                        let _ = writeln!(
                            s,
                            "{}<Canvas.Clip><PathGeometry Figures=\"{}\" FillRule=\"{}\"/></Canvas.Clip>",
                            ind(depth + 1),
                            self.data(path),
                            if *r == FillRule::EvenOdd { "EvenOdd" } else { "Nonzero" }
                        );
                    }
                    depth += 1;
                }
                Op::Unclip => {
                    depth = depth.saturating_sub(1).max(1);
                    let _ = writeln!(s, "{}</{}>", ind(depth), if brush { "DrawingGroup" } else { "Canvas" });
                }
            }
        }
        if brush {
            while depth > 3 {
                depth -= 1;
                let _ = writeln!(s, "{}</DrawingGroup>", ind(depth));
            }
            let _ = writeln!(s, "    </DrawingGroup>");
            let _ = writeln!(s, "  </DrawingBrush.Drawing>");
            let _ = writeln!(s, "</DrawingBrush>");
        } else {
            while depth > 1 {
                depth -= 1;
                let _ = writeln!(s, "{}</Canvas>", ind(depth));
            }
            let _ = writeln!(s, "</Canvas>");
        }
        Ok(s.into_bytes())
    }

    // ---------- OpenDocument Drawing ----------

    fn odg(&mut self, ops: &[Op]) -> Result<Vec<u8>> {
        let (w, h) = (self.region.width(), self.region.height());
        let cm = |v: f64| format!("{}cm", fmt(v / 72.0 * 2.54));
        let mut styles = String::new();
        let mut body = String::new();
        let mut n = 0usize;
        let mut pictures: Vec<(String, Vec<u8>)> = vec![];
        let mut dash_styles = String::new();
        for op in ops {
            let shape = match op {
                Op::Fill { path, rgb, .. } => Some((path, Some(*rgb), None)),
                Op::Stroke { path, pen } => Some((path, None, Some(pen))),
                _ => None,
            };
            match op {
                Op::Fill { .. } | Op::Stroke { .. } => {
                    n += 1;
                    let Some((path, fill, stroke)) = shape else { continue };
                    let _ = write!(styles, "<style:style style:name=\"gr{n}\" style:family=\"graphic\"><style:graphic-properties");
                    match fill {
                        Some(rgb) => {
                            let _ = write!(styles, " draw:fill=\"solid\" draw:fill-color=\"{}\"", hex(rgb));
                        }
                        None => {
                            let _ = write!(styles, " draw:fill=\"none\"");
                        }
                    }
                    match stroke {
                        Some(pen) => {
                            let d = dashes(pen);
                            let _ = write!(
                                styles,
                                " draw:stroke=\"{}\" svg:stroke-color=\"{}\" svg:stroke-width=\"{}\" svg:stroke-linejoin=\"{}\" svg:stroke-linecap=\"{}\"",
                                if d.is_empty() { "solid" } else { "dash" },
                                hex(pen.rgb),
                                cm(pen.width),
                                join_name(pen.join),
                                cap_name(pen.cap)
                            );
                            if let [on, rest @ ..] = d.as_slice() {
                                let off = rest.first().copied().unwrap_or(*on);
                                let _ = write!(styles, " draw:stroke-dash=\"Dash{n}\"");
                                let _ = writeln!(
                                    dash_styles,
                                    "<draw:stroke-dash draw:name=\"Dash{n}\" draw:style=\"rect\" draw:dots1=\"1\" draw:dots1-length=\"{}\" draw:distance=\"{}\"/>",
                                    cm(*on),
                                    cm(off)
                                );
                            }
                        }
                        None => {
                            let _ = write!(styles, " draw:stroke=\"none\"");
                        }
                    }
                    let _ = writeln!(styles, "/></style:style>");
                    let local = self.local(path);
                    let b = kurbo::Shape::bounding_box(&local);
                    let (bw, bh) = (b.width().max(1e-6), b.height().max(1e-6));
                    // The path's own space is its bounding box: viewBox in 1/100 mm.
                    let to_box = Affine::scale(100.0 / PT_PER_MM) * Affine::translate((-b.x0, -b.y0));
                    let boxed = to_box * local;
                    let mut d = String::new();
                    for el in boxed.elements() {
                        match el {
                            PathEl::MoveTo(p) => {
                                let _ = write!(d, "M{} {}", fmt(p.x), fmt(p.y));
                            }
                            PathEl::LineTo(p) => {
                                let _ = write!(d, "L{} {}", fmt(p.x), fmt(p.y));
                            }
                            PathEl::QuadTo(a, p) => {
                                let _ = write!(d, "Q{} {} {} {}", fmt(a.x), fmt(a.y), fmt(p.x), fmt(p.y));
                            }
                            PathEl::CurveTo(a, c, p) => {
                                let _ = write!(d, "C{} {} {} {} {} {}", fmt(a.x), fmt(a.y), fmt(c.x), fmt(c.y), fmt(p.x), fmt(p.y));
                            }
                            PathEl::ClosePath => d.push('Z'),
                        }
                    }
                    let _ = writeln!(
                        body,
                        "<draw:path draw:style-name=\"gr{n}\" draw:layer=\"layout\" svg:x=\"{}\" svg:y=\"{}\" svg:width=\"{}\" svg:height=\"{}\" svg:viewBox=\"0 0 {} {}\" svg:d=\"{}\"/>",
                        cm(b.x0),
                        cm(b.y0),
                        cm(bw),
                        cm(bh),
                        fmt((bw * 100.0 / PT_PER_MM).round()),
                        fmt((bh * 100.0 / PT_PER_MM).round()),
                        d
                    );
                }
                Op::Image { image, xf, opacity } => match image.to_png() {
                    Ok(png) => {
                        n += 1;
                        let m = self.map() * *xf;
                        // The frame is the image's axis-aligned box; rotation is kept as a transform.
                        let (iw, ih) = (f64::from(image.width), f64::from(image.height));
                        let c = m.as_coeffs();
                        let sx = (c[0] * c[0] + c[1] * c[1]).sqrt();
                        let sy = (c[2] * c[2] + c[3] * c[3]).sqrt();
                        let angle = c[1].atan2(c[0]);
                        let name = format!("Pictures/image{n}.png");
                        pictures.push((name.clone(), png));
                        let _ = writeln!(
                            styles,
                            "<style:style style:name=\"gr{n}\" style:family=\"graphic\"><style:graphic-properties draw:stroke=\"none\" draw:fill=\"none\" draw:image-opacity=\"{}%\"/></style:style>",
                            fmt(f64::from(*opacity) * 100.0)
                        );
                        let _ = writeln!(
                            body,
                            "<draw:frame draw:style-name=\"gr{n}\" draw:layer=\"layout\" svg:width=\"{}\" svg:height=\"{}\" draw:transform=\"rotate({}) translate({} {})\"><draw:image xlink:href=\"{name}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/></draw:frame>",
                            cm(iw * sx),
                            cm(ih * sy),
                            fmt(-angle),
                            cm(c[4]),
                            cm(c[5])
                        );
                    }
                    Err(_) => self.warn("an image couldn't be encoded and is left out"),
                },
                Op::Clip { .. } | Op::Unclip => {}
            }
        }
        let ns = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" office:version=\"1.2\"";
        let content = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {ns}><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:drawing><draw:page draw:name=\"page1\" draw:style-name=\"dp1\" draw:master-page-name=\"Default\">{body}</draw:page></office:drawing></office:body></office:document-content>\n"
        );
        let styles_xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {ns}><office:styles>{dash_styles}</office:styles><office:automatic-styles><style:page-layout style:name=\"PM1\"><style:page-layout-properties fo:margin-top=\"0cm\" fo:margin-bottom=\"0cm\" fo:margin-left=\"0cm\" fo:margin-right=\"0cm\" fo:page-width=\"{}\" fo:page-height=\"{}\" style:print-orientation=\"{}\"/></style:page-layout><style:style style:name=\"dp1\" style:family=\"drawing-page\"/></office:automatic-styles><office:master-styles><style:master-page style:name=\"Default\" style:page-layout-name=\"PM1\" draw:style-name=\"dp1\"/></office:master-styles></office:document-styles>\n",
            cm(w),
            cm(h),
            if w >= h { "landscape" } else { "portrait" }
        );
        let mut manifest = String::new();
        let _ = write!(
            manifest,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.2\">"
        );
        let _ = write!(
            manifest,
            "<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.2\" manifest:media-type=\"application/vnd.oasis.opendocument.graphics\"/>"
        );
        let _ = write!(manifest, "<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>");
        let _ = write!(manifest, "<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>");
        for (name, _) in &pictures {
            let _ = write!(manifest, "<manifest:file-entry manifest:full-path=\"{name}\" manifest:media-type=\"image/png\"/>");
        }
        let _ = writeln!(manifest, "</manifest:manifest>");
        let mut files: Vec<(String, Vec<u8>)> = vec![
            ("mimetype".into(), b"application/vnd.oasis.opendocument.graphics".to_vec()),
            ("META-INF/manifest.xml".into(), manifest.into_bytes()),
            ("content.xml".into(), content.into_bytes()),
            ("styles.xml".into(), styles_xml.into_bytes()),
        ];
        files.extend(pictures);
        super::zip::store(&files).map_err(|e| bad(C, e))
    }

    // ---------- TikZ ----------

    fn tex(&mut self, ops: &[Op], standalone: bool) -> Vec<u8> {
        let mut s = String::new();
        if standalone {
            let _ = writeln!(s, "\\documentclass{{standalone}}");
            let _ = writeln!(s, "\\usepackage{{tikz}}");
            let _ = writeln!(s, "\\begin{{document}}");
        }
        let _ = writeln!(s, "% TikZ picture in points, y down (yscale=-1); include with \\usepackage{{tikz}}");
        let _ = writeln!(s, "\\begin{{tikzpicture}}[x=1pt, y=1pt, yscale=-1, inner sep=0pt, outer sep=0pt]");
        let _ = writeln!(s, "\\clip (0,0) rectangle ({},{});", fmt(self.region.width()), fmt(self.region.height()));
        let color = |rgb: [u8; 3]| format!("{{rgb,255:red,{};green,{};blue,{}}}", rgb[0], rgb[1], rgb[2]);
        let mut depth = 0usize;
        for op in ops {
            match op {
                Op::Fill { path, rule, rgb } => {
                    let _ = writeln!(
                        s,
                        "\\path[fill={}{}] {};",
                        color(*rgb),
                        if *rule == FillRule::EvenOdd { ", even odd rule" } else { ", nonzero rule" },
                        self.tikz_path(path)
                    );
                }
                Op::Stroke { path, pen } => {
                    let d = dashes(pen);
                    let mut opts = format!(
                        "draw={}, line width={}pt, line cap={}, line join={}, miter limit={}",
                        color(pen.rgb),
                        fmt(pen.width),
                        match pen.cap {
                            LineCap::Butt => "butt",
                            LineCap::Round => "round",
                            LineCap::Square => "rect",
                        },
                        join_name(pen.join),
                        fmt(pen.miter.max(1.0))
                    );
                    if !d.is_empty() {
                        let mut pat = String::new();
                        for (i, v) in d.iter().enumerate() {
                            let _ = write!(pat, "{} {}pt", if i % 2 == 0 { "on" } else { "off" }, fmt(*v));
                            if i + 1 < d.len() {
                                pat.push(' ');
                            }
                        }
                        let _ = write!(opts, ", dash pattern={pat}");
                    }
                    let _ = writeln!(s, "\\path[{opts}] {};", self.tikz_path(path));
                }
                Op::Image { .. } => self.warn("TikZ pictures include images from files: images are left out"),
                Op::Clip { path, rule } => {
                    let _ = writeln!(s, "\\begin{{scope}}");
                    let _ = writeln!(
                        s,
                        "\\clip[{}] {};",
                        if *rule == FillRule::EvenOdd { "even odd rule" } else { "nonzero rule" },
                        self.tikz_path(path)
                    );
                    depth += 1;
                }
                Op::Unclip => {
                    if depth > 0 {
                        depth -= 1;
                        let _ = writeln!(s, "\\end{{scope}}");
                    }
                }
            }
        }
        while depth > 0 {
            depth -= 1;
            let _ = writeln!(s, "\\end{{scope}}");
        }
        let _ = writeln!(s, "\\end{{tikzpicture}}");
        if standalone {
            let _ = writeln!(s, "\\end{{document}}");
        }
        s.into_bytes()
    }

    fn tikz_path(&self, path: &BezPath) -> String {
        let mut s = String::new();
        let pt = |p: Point| format!("({},{})", fmt(p.x), fmt(p.y));
        let mut last = Point::ZERO;
        for el in self.local(path).elements() {
            match el {
                PathEl::MoveTo(p) => {
                    if !s.is_empty() {
                        s.push(' ');
                    }
                    s.push_str(&pt(*p));
                    last = *p;
                }
                PathEl::LineTo(p) => {
                    let _ = write!(s, " -- {}", pt(*p));
                    last = *p;
                }
                PathEl::QuadTo(a, p) => {
                    // Degree-raised to a cubic.
                    let c1 = last + (*a - last) * (2.0 / 3.0);
                    let c2 = *p + (*a - *p) * (2.0 / 3.0);
                    let _ = write!(s, " .. controls {} and {} .. {}", pt(c1), pt(c2), pt(*p));
                    last = *p;
                }
                PathEl::CurveTo(a, b, p) => {
                    let _ = write!(s, " .. controls {} and {} .. {}", pt(*a), pt(*b), pt(*p));
                    last = *p;
                }
                PathEl::ClosePath => s.push_str(" -- cycle"),
            }
        }
        s
    }

    // ---------- HPGL ----------

    fn hpgl(&mut self, ops: &[Op], p: &Value) -> Result<Vec<u8>> {
        let num = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d);
        let flag = |k: &str, d: bool| p.get(k).and_then(Value::as_bool).unwrap_or(d);
        let resolution = num("resolution", 1016.0);
        if resolution <= 0.0 {
            return Err(bad(C, "`resolution` must be positive plotter units per inch"));
        }
        let pen_fixed = p.get("pen").and_then(Value::as_u64).unwrap_or(1);
        if pen_fixed > 8 {
            return Err(bad(C, "`pen` is 0 (one pen per colour) or 1–8"));
        }
        let overcut = num("overcut", 0.0).max(0.0) * PT_PER_MM;
        let tool_offset = num("toolOffset", 0.0).max(0.0) * PT_PER_MM;
        let precut = flag("precut", false);
        let fills = flag("fills", true);
        let (mx, my) = (flag("mirrorX", false), flag("mirrorY", false));
        let flat = num("flat", 0.1).clamp(0.01, 5.0) * PT_PER_MM;
        let (w, h) = (self.region.width(), self.region.height());
        // Plotter units, y up from the bottom-left corner.
        let unit = resolution / 72.0;
        let to_plot = |q: Point| {
            let x = if mx { w - q.x } else { q.x };
            let y = if my { q.y } else { h - q.y };
            ((x * unit).round() as i64, (y * unit).round() as i64)
        };
        let mut s = String::new();
        let _ = write!(s, "IN;");
        let mut pens: Vec<[u8; 3]> = vec![];
        let mut current_pen = 0u64;
        let mut select = |s: &mut String, rgb: [u8; 3]| {
            let pen = if pen_fixed == 0 {
                let i = match pens.iter().position(|c| *c == rgb) {
                    Some(i) => i,
                    None => {
                        pens.push(rgb);
                        pens.len() - 1
                    }
                };
                (i as u64 % 8) + 1
            } else {
                pen_fixed
            };
            if pen != current_pen {
                current_pen = pen;
                let _ = write!(s, "SP{pen};");
            }
        };
        let mut first = true;
        let mut drew = false;
        for op in ops {
            let (path, rgb) = match op {
                Op::Fill { path, rgb, .. } if fills => (path, *rgb),
                Op::Fill { .. } => continue,
                Op::Stroke { path, pen } => (path, pen.rgb),
                Op::Image { .. } => {
                    self.warn("plotters draw lines: images are left out");
                    continue;
                }
                Op::Clip { .. } | Op::Unclip => continue,
            };
            select(&mut s, rgb);
            for (pts, closed) in self.polylines(path, flat) {
                let mut pts = pts;
                if closed && pts.first() != pts.last() {
                    pts.push(pts[0]);
                }
                if tool_offset > 0.0 {
                    pts = knife_corners(&pts, tool_offset);
                }
                if closed && overcut > 0.0 && pts.len() >= 2 {
                    // Draw on past the start so the cut closes.
                    let mut left = overcut;
                    let mut i = 0;
                    while left > 0.0 && i + 1 < pts.len() {
                        let (a, b) = (pts[i], pts[i + 1]);
                        let seg = (b - a).hypot();
                        if seg >= left {
                            pts.push(a + (b - a) * (left / seg.max(1e-9)));
                            break;
                        }
                        pts.push(b);
                        left -= seg;
                        i += 1;
                    }
                }
                if first && precut {
                    let (x, y) = to_plot(pts[0]);
                    let (x0, y0) = (x - (2.0 * PT_PER_MM * unit) as i64, y);
                    let _ = write!(s, "PU{},{};PD{},{};", x0.max(0), y0, x, y);
                }
                first = false;
                let (x, y) = to_plot(pts[0]);
                let _ = write!(s, "PU{x},{y};PD");
                let coords: Vec<String> = pts[1..]
                    .iter()
                    .map(|q| {
                        let (x, y) = to_plot(*q);
                        format!("{x},{y}")
                    })
                    .collect();
                s.push_str(&coords.join(","));
                s.push(';');
                drew = true;
            }
        }
        if !drew {
            self.warn("the artboard has no lines to plot");
        }
        let _ = write!(s, "PU;SP0;IN;");
        Ok(s.into_bytes())
    }
}

/// A drag knife trails its holder by `offset`: at each corner the holder swings round a small arc
/// so the blade turns to the new direction before the next segment.
fn knife_corners(pts: &[Point], offset: f64) -> Vec<Point> {
    let mut out = vec![];
    for (i, p) in pts.iter().enumerate() {
        out.push(*p);
        if i == 0 || i + 1 >= pts.len() {
            continue;
        }
        let (a, b) = (pts[i - 1], pts[i + 1]);
        let d1 = (*p - a).normalize();
        let d2 = (b - *p).normalize();
        let turn = d1.cross(d2).atan2(d1.dot(d2));
        if turn.abs() < 0.1 || !turn.is_finite() {
            continue;
        }
        // Swing from the incoming direction to the outgoing one on a circle of radius `offset`.
        let steps = ((turn.abs() / 0.3).ceil() as usize).clamp(1, 12);
        for k in 1..=steps {
            let t = turn * (k as f64 / steps as f64);
            let (sn, cs) = t.sin_cos();
            let d = redrob_vector_geom::Vec2::new(d1.x * cs - d1.y * sn, d1.x * sn + d1.y * cs);
            out.push(*p + d * offset - d1 * offset);
        }
    }
    out
}

fn cap_name(c: LineCap) -> &'static str {
    match c {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    }
}

fn join_name(j: LineJoin) -> &'static str {
    match j {
        LineJoin::Miter => "miter",
        LineJoin::Round => "round",
        LineJoin::Bevel => "bevel",
    }
}

fn srgb_to_linear(c: u8) -> f64 {
    let v = f64::from(c) / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// One subpath as cubic segments.
struct Sub {
    start: Point,
    /// `(c1, c2, end)` per segment.
    segs: Vec<(Point, Point, Point)>,
    closed: bool,
}

fn subpaths(path: &BezPath) -> Vec<Sub> {
    let mut out: Vec<Sub> = vec![];
    let mut last = Point::ZERO;
    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => {
                out.push(Sub { start: *p, segs: vec![], closed: false });
                last = *p;
            }
            PathEl::LineTo(p) => {
                if let Some(s) = out.last_mut() {
                    s.segs.push((last + (*p - last) / 3.0, last + (*p - last) * (2.0 / 3.0), *p));
                }
                last = *p;
            }
            PathEl::QuadTo(a, p) => {
                if let Some(s) = out.last_mut() {
                    s.segs.push((last + (*a - last) * (2.0 / 3.0), *p + (*a - *p) * (2.0 / 3.0), *p));
                }
                last = *p;
            }
            PathEl::CurveTo(a, b, p) => {
                if let Some(s) = out.last_mut() {
                    s.segs.push((*a, *b, *p));
                }
                last = *p;
            }
            PathEl::ClosePath => {
                if let Some(s) = out.last_mut() {
                    s.closed = true;
                    last = s.start;
                }
            }
        }
    }
    out.retain(|s| !s.segs.is_empty());
    out
}

/// A Synfig bline point: a vertex with its incoming and outgoing tangents (three times the
/// Bézier handle, as Synfig stores them).
struct BlinePoint {
    point: Point,
    t_in: redrob_vector_geom::Vec2,
    t_out: redrob_vector_geom::Vec2,
}

fn bline_points(sub: &Sub) -> Vec<BlinePoint> {
    let mut pts: Vec<Point> = vec![sub.start];
    for (_, _, p) in &sub.segs {
        pts.push(*p);
    }
    let closes_on_start = sub.closed && pts.len() > 1 && (pts[pts.len() - 1] - pts[0]).hypot() < 1e-6;
    let n = if closes_on_start { pts.len() - 1 } else { pts.len() };
    let mut out = vec![];
    for i in 0..n {
        let p = pts[i];
        // Outgoing tangent: the first control point of the segment leaving this vertex.
        let t_out = if i < sub.segs.len() {
            (sub.segs[i].0 - p) * 3.0
        } else if sub.closed && !sub.segs.is_empty() {
            (sub.segs[0].0 - p) * 3.0
        } else {
            redrob_vector_geom::Vec2::ZERO
        };
        // Incoming tangent: from the second control point of the segment arriving here.
        let t_in = if i > 0 {
            (p - sub.segs[i - 1].1) * 3.0
        } else if sub.closed && closes_on_start && !sub.segs.is_empty() {
            (p - sub.segs[sub.segs.len() - 1].1) * 3.0
        } else {
            t_out
        };
        out.push(BlinePoint { point: p, t_in, t_out });
    }
    out
}

/// `document.plot {device, …the HPGL options}`: the artboard's HPGL sent straight to a plotter's
/// serial device (`/dev/ttyUSB0`, `COM3`), which the system has set up (baud rate, flow
/// control); a path to a file writes the HPGL there instead.
pub(super) fn plot(s: &mut Session, p: &Value) -> Result<Value> {
    const PC: &str = "document.plot";
    let device = str_param(p, "device")
        .or_else(|| str_param(p, "path"))
        .filter(|d| !d.trim().is_empty())
        .ok_or_else(|| bad(PC, "give `device`: the plotter's serial device or a file to write the HPGL to"))?;
    let doc = s.doc()?.doc.clone();
    let enc = encode(&doc, "hpgl", p, None)?;
    let bytes = enc.files.first().map(|(_, b)| b.clone()).ok_or_else(|| bad(PC, "nothing to plot"))?;
    write_device(device, &bytes)?;
    Ok(serde_json::json!({ "device": device, "bytes": bytes.len(), "warnings": enc.warnings }))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_device(device: &str, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    crate::file_access::check_write(device).map_err(EngineError::Other)?;
    // A device can't take an atomic rename: it is written straight through.
    let mut f =
        std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(device).map_err(|e| EngineError::Other(format!("{device}: {e}")))?;
    f.write_all(bytes).and_then(|()| f.flush()).map_err(|e| EngineError::Other(format!("{device}: {e}")))
}

#[cfg(target_arch = "wasm32")]
fn write_device(device: &str, _: &[u8]) -> Result<()> {
    Err(EngineError::Other(format!("{device}: the web app has no serial devices or files: export the HPGL and download it")))
}
