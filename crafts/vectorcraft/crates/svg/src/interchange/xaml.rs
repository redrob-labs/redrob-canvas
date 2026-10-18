//! Microsoft XAML drawings (Silverlight and WPF: `Canvas` pages of `Path`, `Rectangle`,
//! `Ellipse`, `Line`, `Polygon`, `Polyline`, `TextBlock` and nested canvases, and
//! `DrawingBrush` / `DrawingImage` resources of `GeometryDrawing`s) read by translating the markup
//! into SVG and importing that: the path mini-language is SVG's with an `F0`/`F1` fill-rule
//! prefix, brushes become fills and gradients, transforms become `transform` attributes.

use std::fmt::Write as _;
use std::str::FromStr;

use redrob_vector_doc::Document;
use usvg::roxmltree::{self, Node as XNode};

use crate::{ImportOptions, SvgError, xml_escape};

/// Does `text` look like a XAML document (its presentation namespace)?
pub fn is_xaml(text: &str) -> bool {
    let head = &text[..text.len().min(4096)];
    head.contains("schemas.microsoft.com/winfx/2006/xaml/presentation") || head.contains("schemas.microsoft.com/client/2007")
}

fn n(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.to_string() }
}

/// Numbers separated by commas or spaces.
fn nums(s: &str) -> Vec<f64> {
    s.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect()
}

/// A XAML colour (`#AARRGGBB`, `#RRGGBB`, `#ARGB`, `#RGB`, a name, `sc#`) → (CSS colour, opacity).
fn color(s: &str) -> Option<(String, f64)> {
    let t = s.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("Transparent") || t.starts_with('{') {
        return None;
    }
    if let Some(h) = t.strip_prefix('#') {
        let hex = |s: &str| u8::from_str_radix(s, 16).ok();
        let expand = |c: char| hex(&format!("{c}{c}"));
        let cs: Vec<char> = h.chars().collect();
        return match (h.len(), cs.as_slice()) {
            (8, _) => Some((format!("#{}", &h[2..]), f64::from(hex(&h[..2])?) / 255.0)),
            (6, _) => Some((format!("#{h}"), 1.0)),
            (4, [a, r, g, b]) => Some((format!("#{:02x}{:02x}{:02x}", expand(*r)?, expand(*g)?, expand(*b)?), f64::from(expand(*a)?) / 255.0)),
            (3, [r, g, b]) => Some((format!("#{:02x}{:02x}{:02x}", expand(*r)?, expand(*g)?, expand(*b)?), 1.0)),
            _ => None,
        };
    }
    if let Some(sc) = t.strip_prefix("sc#") {
        let v = nums(sc);
        let (a, rgb) = match v.as_slice() {
            [a, r, g, b] => (*a, [*r, *g, *b]),
            [r, g, b] => (1.0, [*r, *g, *b]),
            _ => return None,
        };
        let q = |x: f64| {
            let s = if x <= 0.0031308 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
            (s.clamp(0.0, 1.0) * 255.0).round() as u8
        };
        return Some((format!("#{:02x}{:02x}{:02x}", q(rgb[0]), q(rgb[1]), q(rgb[2])), a.clamp(0.0, 1.0)));
    }
    // Named colours are the SVG names (XAML capitalises them).
    let c = svgtypes::Color::from_str(&t.to_ascii_lowercase()).ok()?;
    Some((format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue), f64::from(c.alpha) / 255.0))
}

/// Converts XAML elements to SVG text.
struct Conv {
    defs: String,
    body: String,
    ids: usize,
    warnings: Vec<String>,
    /// The size the root declares (or the largest child), for the SVG's.
    size: (f64, f64),
}

impl Conv {
    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.ids += 1;
        format!("{prefix}{}", self.ids)
    }

    /// The property element `<Owner.Name>` of `e`, if any.
    fn prop<'a, 'i>(e: XNode<'a, 'i>, name: &str) -> Option<XNode<'a, 'i>> {
        e.children().find(|c| c.is_element() && c.tag_name().name().rsplit('.').next() == Some(name) && c.tag_name().name().contains('.'))
    }

    /// A brush: the attribute `name`, or its property element → (SVG paint, opacity).
    fn brush(&mut self, e: XNode, name: &str) -> (String, f64) {
        if let Some(v) = e.attribute(name) {
            return match color(v) {
                Some((c, a)) => (c, a),
                None => ("none".into(), 1.0),
            };
        }
        let Some(p) = Self::prop(e, name) else { return ("none".into(), 1.0) };
        let Some(b) = p.children().find(|c| c.is_element()) else { return ("none".into(), 1.0) };
        self.brush_element(b)
    }

    fn brush_element(&mut self, b: XNode) -> (String, f64) {
        let opacity: f64 = b.attribute("Opacity").and_then(|v| v.trim().parse().ok()).unwrap_or(1.0);
        match b.tag_name().name() {
            "SolidColorBrush" => match b.attribute("Color").and_then(color) {
                Some((c, a)) => (c, a * opacity),
                None => ("none".into(), 1.0),
            },
            "LinearGradientBrush" | "RadialGradientBrush" => {
                let id = self.next_id("grad");
                let relative = !b.attribute("MappingMode").is_some_and(|m| m.eq_ignore_ascii_case("Absolute"));
                let units = if relative { "objectBoundingBox" } else { "userSpaceOnUse" };
                let spread = match b.attribute("SpreadMethod").unwrap_or("Pad") {
                    "Reflect" => "reflect",
                    "Repeat" => "repeat",
                    _ => "pad",
                };
                let mut stops = String::new();
                for s in b.descendants().filter(|c| c.is_element() && c.tag_name().name() == "GradientStop") {
                    let off: f64 = s.attribute("Offset").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
                    let (c, a) = s.attribute("Color").and_then(color).unwrap_or(("#000000".into(), 1.0));
                    let _ = write!(stops, "<stop offset=\"{}\" stop-color=\"{c}\" stop-opacity=\"{}\"/>", n(off.clamp(0.0, 1.0)), n(a));
                }
                let transform = Self::prop(b, "Transform")
                    .or_else(|| Self::prop(b, "RelativeTransform"))
                    .and_then(|t| t.children().find(|c| c.is_element()))
                    .map(|t| self.transform_element(t))
                    .or_else(|| b.attribute("Transform").map(matrix_attr))
                    .filter(|t| !t.is_empty())
                    .map(|t| format!(" gradientTransform=\"{t}\""))
                    .unwrap_or_default();
                if b.tag_name().name() == "LinearGradientBrush" {
                    let s = nums(b.attribute("StartPoint").unwrap_or("0,0"));
                    let e = nums(b.attribute("EndPoint").unwrap_or("1,1"));
                    let (x1, y1) = (s.first().copied().unwrap_or(0.0), s.get(1).copied().unwrap_or(0.0));
                    let (x2, y2) = (e.first().copied().unwrap_or(1.0), e.get(1).copied().unwrap_or(1.0));
                    let _ = writeln!(
                        self.defs,
                        "<linearGradient id=\"{id}\" gradientUnits=\"{units}\" spreadMethod=\"{spread}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"{transform}>{stops}</linearGradient>",
                        n(x1),
                        n(y1),
                        n(x2),
                        n(y2)
                    );
                } else {
                    let c = nums(b.attribute("Center").unwrap_or("0.5,0.5"));
                    let f = nums(b.attribute("GradientOrigin").unwrap_or("0.5,0.5"));
                    let rx: f64 = b.attribute("RadiusX").and_then(|v| v.trim().parse().ok()).unwrap_or(0.5);
                    let ry: f64 = b.attribute("RadiusY").and_then(|v| v.trim().parse().ok()).unwrap_or(0.5);
                    let _ = writeln!(
                        self.defs,
                        "<radialGradient id=\"{id}\" gradientUnits=\"{units}\" spreadMethod=\"{spread}\" cx=\"{}\" cy=\"{}\" fx=\"{}\" fy=\"{}\" r=\"{}\"{transform}>{stops}</radialGradient>",
                        n(c.first().copied().unwrap_or(0.5)),
                        n(c.get(1).copied().unwrap_or(0.5)),
                        n(f.first().copied().unwrap_or(0.5)),
                        n(f.get(1).copied().unwrap_or(0.5)),
                        n(rx.max(ry))
                    );
                    if (rx - ry).abs() > 1e-9 {
                        self.warn("radial gradients with unequal radii are read as circular");
                    }
                }
                (format!("url(#{id})"), opacity)
            }
            "ImageBrush" | "VisualBrush" | "DrawingBrush" => {
                self.warn("image, visual and drawing brushes used as paint are read as grey");
                ("#808080".into(), opacity)
            }
            _ => ("none".into(), 1.0),
        }
    }

    /// A transform element (`MatrixTransform`, `TranslateTransform`, `ScaleTransform`,
    /// `RotateTransform`, `SkewTransform`, `TransformGroup`) as SVG transform functions.
    fn transform_element(&mut self, t: XNode) -> String {
        let num = |name: &str, d: f64| t.attribute(name).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(d);
        match t.tag_name().name() {
            "MatrixTransform" => t.attribute("Matrix").map(matrix_attr).unwrap_or_default(),
            "TranslateTransform" => format!("translate({} {})", n(num("X", 0.0)), n(num("Y", 0.0))),
            "ScaleTransform" => {
                let (cx, cy) = (num("CenterX", 0.0), num("CenterY", 0.0));
                let (sx, sy) = (num("ScaleX", 1.0), num("ScaleY", 1.0));
                if cx != 0.0 || cy != 0.0 {
                    format!("translate({} {}) scale({} {}) translate({} {})", n(cx), n(cy), n(sx), n(sy), n(-cx), n(-cy))
                } else {
                    format!("scale({} {})", n(sx), n(sy))
                }
            }
            "RotateTransform" => format!("rotate({} {} {})", n(num("Angle", 0.0)), n(num("CenterX", 0.0)), n(num("CenterY", 0.0))),
            "SkewTransform" => {
                let (cx, cy) = (num("CenterX", 0.0), num("CenterY", 0.0));
                format!(
                    "translate({} {}) skewX({}) skewY({}) translate({} {})",
                    n(cx),
                    n(cy),
                    n(num("AngleX", 0.0)),
                    n(num("AngleY", 0.0)),
                    n(-cx),
                    n(-cy)
                )
            }
            "TransformGroup" => {
                // Applied in order: the first child first, so the last one is outermost.
                let parts: Vec<String> =
                    t.children().filter(|c| c.is_element()).map(|c| self.transform_element(c)).filter(|s| !s.is_empty()).collect();
                parts.into_iter().rev().collect::<Vec<_>>().join(" ")
            }
            _ => String::new(),
        }
    }

    /// The `transform` attribute of `e`: Canvas.Left/Top, then RenderTransform (about its origin).
    fn transform(&mut self, e: XNode) -> String {
        let mut parts = vec![];
        let left: f64 = e.attribute("Canvas.Left").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
        let top: f64 = e.attribute("Canvas.Top").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
        if left != 0.0 || top != 0.0 {
            parts.push(format!("translate({} {})", n(left), n(top)));
        }
        let render = e
            .attribute("RenderTransform")
            .map(matrix_attr)
            .or_else(|| Self::prop(e, "RenderTransform").and_then(|p| p.children().find(|c| c.is_element())).map(|t| self.transform_element(t)))
            .filter(|t| !t.is_empty());
        if let Some(r) = render {
            // RenderTransformOrigin is relative to the element's size: only the top-left corner
            // (the default) is known without laying it out.
            if e.attribute("RenderTransformOrigin").is_some_and(|o| nums(o).iter().any(|v| *v != 0.0)) {
                self.warn("RenderTransformOrigin is read as the element's top-left corner");
            }
            parts.push(r);
        }
        parts.join(" ")
    }

    /// The stroke attributes of a shape element.
    fn stroke_attrs(&mut self, e: XNode, prefix: &str) -> String {
        let brush_name = if e.tag_name().name() == "Pen" { "Brush".to_string() } else { format!("{prefix}Stroke") };
        let (stroke, so) = self.brush(e, &brush_name);
        if stroke == "none" {
            return " stroke=\"none\"".into();
        }
        let w: f64 = e
            .attribute(format!("{prefix}StrokeThickness").as_str())
            .or_else(|| e.attribute("Thickness"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(1.0);
        let mut s = format!(" stroke=\"{stroke}\" stroke-width=\"{}\"", n(w));
        if so < 1.0 {
            let _ = write!(s, " stroke-opacity=\"{}\"", n(so));
        }
        let cap = e.attribute(format!("{prefix}StrokeStartLineCap").as_str()).or_else(|| e.attribute("StartLineCap")).unwrap_or("Flat");
        let _ = write!(
            s,
            " stroke-linecap=\"{}\"",
            match cap {
                "Round" => "round",
                "Square" => "square",
                _ => "butt",
            }
        );
        let join = e.attribute(format!("{prefix}StrokeLineJoin").as_str()).or_else(|| e.attribute("LineJoin")).unwrap_or("Miter");
        let _ = write!(s, " stroke-linejoin=\"{}\"", join.to_ascii_lowercase());
        if let Some(m) = e.attribute(format!("{prefix}StrokeMiterLimit").as_str()).or_else(|| e.attribute("MiterLimit")) {
            let _ = write!(s, " stroke-miterlimit=\"{}\"", m.trim());
        }
        let dashes = e
            .attribute(format!("{prefix}StrokeDashArray").as_str())
            .map(nums)
            .or_else(|| Self::prop(e, "DashStyle").and_then(|p| p.children().find(|c| c.is_element())).and_then(|d| d.attribute("Dashes")).map(nums));
        if let Some(d) = dashes.filter(|d| !d.is_empty()) {
            let _ = write!(s, " stroke-dasharray=\"{}\"", d.iter().map(|v| n(v * w)).collect::<Vec<_>>().join(" "));
        }
        s
    }

    /// Common attributes: fill, stroke, opacity, transform, clip.
    fn shape_attrs(&mut self, e: XNode) -> String {
        let (fill, fo) = self.brush(e, "Fill");
        let mut s = format!(" fill=\"{fill}\"");
        if fo < 1.0 {
            let _ = write!(s, " fill-opacity=\"{}\"", n(fo));
        }
        s.push_str(&self.stroke_attrs(e, ""));
        s.push_str(&self.common_attrs(e));
        s
    }

    fn common_attrs(&mut self, e: XNode) -> String {
        let mut s = String::new();
        if let Some(o) = e.attribute("Opacity").and_then(|v| v.trim().parse::<f64>().ok()).filter(|o| *o < 1.0) {
            let _ = write!(s, " opacity=\"{}\"", n(o.max(0.0)));
        }
        if e.attribute("Visibility").is_some_and(|v| v != "Visible") {
            s.push_str(" display=\"none\"");
        }
        let t = self.transform(e);
        if !t.is_empty() {
            let _ = write!(s, " transform=\"{t}\"");
        }
        if let Some(clip) = Self::prop(e, "Clip").and_then(|p| p.children().find(|c| c.is_element())) {
            let d = self.geometry_data(clip);
            if !d.is_empty() {
                let id = self.next_id("clip");
                let _ = writeln!(self.defs, "<clipPath id=\"{id}\"><path d=\"{}\"/></clipPath>", xml_escape(&d));
                let _ = write!(s, " clip-path=\"url(#{id})\"");
            }
        }
        if let Some(name) = e.attribute("Name").or_else(|| e.attribute(("http://schemas.microsoft.com/winfx/2006/xaml", "Name"))) {
            let _ = write!(s, " id=\"{}\"", xml_escape(name));
        }
        s
    }

    /// Path data of a geometry element (or a `Figures`/mini-language string) as SVG `d`.
    fn geometry_data(&mut self, g: XNode) -> String {
        match g.tag_name().name() {
            "PathGeometry" => {
                if let Some(f) = g.attribute("Figures") {
                    return strip_fill_rule(f).1;
                }
                let mut d = String::new();
                for fig in g.descendants().filter(|c| c.is_element() && c.tag_name().name() == "PathFigure") {
                    let start = nums(fig.attribute("StartPoint").unwrap_or("0,0"));
                    let _ = write!(d, "M{} {}", n(start.first().copied().unwrap_or(0.0)), n(start.get(1).copied().unwrap_or(0.0)));
                    for seg in fig.descendants().filter(|c| c.is_element()) {
                        let p = |name: &str| nums(seg.attribute(name).unwrap_or(""));
                        match seg.tag_name().name() {
                            "LineSegment" => {
                                let v = p("Point");
                                if v.len() >= 2 {
                                    let _ = write!(d, "L{} {}", n(v[0]), n(v[1]));
                                }
                            }
                            "PolyLineSegment" => {
                                for c in p("Points").chunks(2) {
                                    if c.len() == 2 {
                                        let _ = write!(d, "L{} {}", n(c[0]), n(c[1]));
                                    }
                                }
                            }
                            "BezierSegment" => {
                                let (a, b, c) = (p("Point1"), p("Point2"), p("Point3"));
                                if a.len() >= 2 && b.len() >= 2 && c.len() >= 2 {
                                    let _ = write!(d, "C{} {} {} {} {} {}", n(a[0]), n(a[1]), n(b[0]), n(b[1]), n(c[0]), n(c[1]));
                                }
                            }
                            "PolyBezierSegment" => {
                                for c in p("Points").chunks(6) {
                                    if c.len() == 6 {
                                        let _ = write!(d, "C{} {} {} {} {} {}", n(c[0]), n(c[1]), n(c[2]), n(c[3]), n(c[4]), n(c[5]));
                                    }
                                }
                            }
                            "QuadraticBezierSegment" => {
                                let (a, b) = (p("Point1"), p("Point2"));
                                if a.len() >= 2 && b.len() >= 2 {
                                    let _ = write!(d, "Q{} {} {} {}", n(a[0]), n(a[1]), n(b[0]), n(b[1]));
                                }
                            }
                            "PolyQuadraticBezierSegment" => {
                                for c in p("Points").chunks(4) {
                                    if c.len() == 4 {
                                        let _ = write!(d, "Q{} {} {} {}", n(c[0]), n(c[1]), n(c[2]), n(c[3]));
                                    }
                                }
                            }
                            "ArcSegment" => {
                                let (to, size) = (p("Point"), p("Size"));
                                if to.len() >= 2 && size.len() >= 2 {
                                    let rot: f64 = seg.attribute("RotationAngle").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
                                    let large = seg.attribute("IsLargeArc").is_some_and(|v| v.eq_ignore_ascii_case("true"));
                                    let sweep = seg.attribute("SweepDirection").is_some_and(|v| v.eq_ignore_ascii_case("Clockwise"));
                                    let _ = write!(
                                        d,
                                        "A{} {} {} {} {} {} {}",
                                        n(size[0]),
                                        n(size[1]),
                                        n(rot),
                                        u8::from(large),
                                        u8::from(sweep),
                                        n(to[0]),
                                        n(to[1])
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                    if fig.attribute("IsClosed").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
                        d.push('Z');
                    }
                }
                d
            }
            "RectangleGeometry" => {
                let r = nums(g.attribute("Rect").unwrap_or(""));
                if r.len() < 4 {
                    return String::new();
                }
                let rx: f64 = g.attribute("RadiusX").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
                let ry: f64 = g.attribute("RadiusY").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
                let (x, y, w, h) = (r[0], r[1], r[2], r[3]);
                if rx > 0.0 && ry > 0.0 {
                    let (rx, ry) = (rx.min(w / 2.0), ry.min(h / 2.0));
                    format!(
                        "M{} {}H{}A{} {} 0 0 1 {} {}V{}A{} {} 0 0 1 {} {}H{}A{} {} 0 0 1 {} {}V{}A{} {} 0 0 1 {} {}Z",
                        n(x + rx),
                        n(y),
                        n(x + w - rx),
                        n(rx),
                        n(ry),
                        n(x + w),
                        n(y + ry),
                        n(y + h - ry),
                        n(rx),
                        n(ry),
                        n(x + w - rx),
                        n(y + h),
                        n(x + rx),
                        n(rx),
                        n(ry),
                        n(x),
                        n(y + h - ry),
                        n(y + ry),
                        n(rx),
                        n(ry),
                        n(x + rx),
                        n(y)
                    )
                } else {
                    format!("M{} {}H{}V{}H{}Z", n(x), n(y), n(x + w), n(y + h), n(x))
                }
            }
            "EllipseGeometry" => {
                let c = nums(g.attribute("Center").unwrap_or("0,0"));
                let rx: f64 = g.attribute("RadiusX").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
                let ry: f64 = g.attribute("RadiusY").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
                let (cx, cy) = (c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0));
                format!(
                    "M{} {}A{} {} 0 1 0 {} {}A{} {} 0 1 0 {} {}Z",
                    n(cx - rx),
                    n(cy),
                    n(rx),
                    n(ry),
                    n(cx + rx),
                    n(cy),
                    n(rx),
                    n(ry),
                    n(cx - rx),
                    n(cy)
                )
            }
            "LineGeometry" => {
                let (a, b) = (nums(g.attribute("StartPoint").unwrap_or("0,0")), nums(g.attribute("EndPoint").unwrap_or("0,0")));
                if a.len() >= 2 && b.len() >= 2 { format!("M{} {}L{} {}", n(a[0]), n(a[1]), n(b[0]), n(b[1])) } else { String::new() }
            }
            "GeometryGroup" | "CombinedGeometry" => {
                if g.tag_name().name() == "CombinedGeometry" {
                    self.warn("combined geometries are read as both of their geometries, not their boolean result");
                }
                let parts: Vec<String> =
                    g.children().filter(|c| c.is_element() && !c.tag_name().name().contains('.')).map(|c| self.geometry_data(c)).collect();
                parts.join("")
            }
            _ => String::new(),
        }
    }

    /// The `Data` of a `Path` (attribute or property element) → (fill rule, d).
    fn path_data(&mut self, e: XNode) -> (&'static str, String) {
        if let Some(d) = e.attribute("Data") {
            return strip_fill_rule(d);
        }
        let Some(p) = Self::prop(e, "Data") else { return ("nonzero", String::new()) };
        let Some(g) = p.children().find(|c| c.is_element()) else { return ("nonzero", String::new()) };
        let rule = if g.attribute("FillRule").is_some_and(|r| r.eq_ignore_ascii_case("Nonzero")) { "nonzero" } else { "evenodd" };
        (rule, self.geometry_data(g))
    }

    fn element(&mut self, e: XNode, depth: u32) {
        if depth > 64 {
            return;
        }
        let tag = e.tag_name().name();
        if tag.contains('.') {
            return;
        }
        let size = |name: &str| e.attribute(name).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite() && *v > 0.0);
        match tag {
            "Canvas" | "Grid" | "StackPanel" | "Border" | "Viewbox" | "UserControl" | "Page" | "Window" | "ContentControl" | "Panel" => {
                if let (Some(w), Some(h)) = (size("Width"), size("Height"))
                    && (w * h > self.size.0 * self.size.1 || depth == 0)
                {
                    self.size = (w, h);
                }
                let attrs = self.common_attrs(e);
                let (bg, bo) = self.brush(e, "Background");
                let _ = writeln!(self.body, "<g{attrs}>");
                if bg != "none"
                    && let (Some(w), Some(h)) = (size("Width"), size("Height"))
                {
                    let _ = writeln!(self.body, "<rect width=\"{}\" height=\"{}\" fill=\"{bg}\" fill-opacity=\"{}\"/>", n(w), n(h), n(bo));
                }
                for c in e.children().filter(|c| c.is_element()) {
                    self.element(c, depth + 1);
                }
                let _ = writeln!(self.body, "</g>");
            }
            "Path" => {
                let (rule, d) = self.path_data(e);
                if d.is_empty() {
                    return;
                }
                let attrs = self.shape_attrs(e);
                let _ = writeln!(self.body, "<path d=\"{}\" fill-rule=\"{rule}\"{attrs}/>", xml_escape(&d));
            }
            "Rectangle" => {
                let (w, h) = (size("Width").unwrap_or(0.0), size("Height").unwrap_or(0.0));
                let rx: f64 = e.attribute("RadiusX").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
                let ry: f64 = e.attribute("RadiusY").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
                let attrs = self.shape_attrs(e);
                let _ = writeln!(self.body, "<rect width=\"{}\" height=\"{}\" rx=\"{}\" ry=\"{}\"{attrs}/>", n(w), n(h), n(rx), n(ry));
            }
            "Ellipse" => {
                let (w, h) = (size("Width").unwrap_or(0.0), size("Height").unwrap_or(0.0));
                let attrs = self.shape_attrs(e);
                let _ =
                    writeln!(self.body, "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"{attrs}/>", n(w / 2.0), n(h / 2.0), n(w / 2.0), n(h / 2.0));
            }
            "Line" => {
                let num = |name: &str| e.attribute(name).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0);
                let attrs = self.shape_attrs(e);
                let _ = writeln!(
                    self.body,
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"{attrs}/>",
                    n(num("X1")),
                    n(num("Y1")),
                    n(num("X2")),
                    n(num("Y2"))
                );
            }
            "Polygon" | "Polyline" => {
                let pts = nums(e.attribute("Points").unwrap_or(""));
                let rule = if e.attribute("FillRule").is_some_and(|r| r.eq_ignore_ascii_case("Nonzero")) { "nonzero" } else { "evenodd" };
                let attrs = self.shape_attrs(e);
                let _ = writeln!(
                    self.body,
                    "<{} points=\"{}\" fill-rule=\"{rule}\"{attrs}/>",
                    tag.to_ascii_lowercase(),
                    pts.iter().map(|v| n(*v)).collect::<Vec<_>>().join(" ")
                );
            }
            "TextBlock" | "Label" => {
                let text = e
                    .attribute("Text")
                    .map(str::to_string)
                    .or_else(|| e.attribute("Content").map(str::to_string))
                    .unwrap_or_else(|| e.descendants().filter(|c| c.is_text()).filter_map(|c| c.text()).collect::<String>());
                let size: f64 = e.attribute("FontSize").and_then(|v| v.trim().parse().ok()).unwrap_or(12.0);
                let family = e.attribute("FontFamily").unwrap_or("sans-serif");
                let (fill, fo) = self.brush(e, "Foreground");
                let fill = if fill == "none" { "#000000".to_string() } else { fill };
                let weight = e.attribute("FontWeight").map(|w| w.to_ascii_lowercase()).unwrap_or_else(|| "normal".into());
                let style = e.attribute("FontStyle").map(|w| w.to_ascii_lowercase()).unwrap_or_else(|| "normal".into());
                let attrs = self.common_attrs(e);
                let _ = writeln!(
                    self.body,
                    "<text x=\"0\" y=\"{}\" font-size=\"{}\" font-family=\"{}\" font-weight=\"{weight}\" font-style=\"{style}\" fill=\"{fill}\" fill-opacity=\"{}\"{attrs}>{}</text>",
                    n(size * 0.8),
                    n(size),
                    xml_escape(family),
                    n(fo),
                    xml_escape(text.trim())
                );
            }
            "Image" => self.warn("Image elements load their pictures from files: they are left out"),
            "DrawingBrush" | "DrawingImage" | "ResourceDictionary" | "DrawingGroup" | "Drawing" => {
                let attrs = if tag == "DrawingGroup" { self.drawing_group_attrs(e) } else { String::new() };
                let _ = writeln!(self.body, "<g{attrs}>");
                for c in e.children().filter(|c| c.is_element()) {
                    if c.tag_name().name().contains('.') {
                        for d in c.children().filter(|c| c.is_element()) {
                            self.element(d, depth + 1);
                        }
                    } else {
                        self.element(c, depth + 1);
                    }
                }
                let _ = writeln!(self.body, "</g>");
            }
            "GeometryDrawing" => {
                let (rule, d) = match e.attribute("Geometry") {
                    Some(g) => strip_fill_rule(g),
                    None => {
                        let g = Self::prop(e, "Geometry").and_then(|p| p.children().find(|c| c.is_element()));
                        match g {
                            Some(g) => {
                                let rule =
                                    if g.attribute("FillRule").is_some_and(|r| r.eq_ignore_ascii_case("Nonzero")) { "nonzero" } else { "evenodd" };
                                (rule, self.geometry_data(g))
                            }
                            None => ("nonzero", String::new()),
                        }
                    }
                };
                if d.is_empty() {
                    return;
                }
                let (fill, fo) = self.brush(e, "Brush");
                let mut attrs = format!(" fill=\"{fill}\"");
                if fo < 1.0 {
                    let _ = write!(attrs, " fill-opacity=\"{}\"", n(fo));
                }
                let pen = Self::prop(e, "Pen").and_then(|p| p.children().find(|c| c.is_element() && c.tag_name().name() == "Pen"));
                match pen {
                    Some(pen) => attrs.push_str(&self.stroke_attrs(pen, "")),
                    None => attrs.push_str(" stroke=\"none\""),
                }
                let _ = writeln!(self.body, "<path d=\"{}\" fill-rule=\"{rule}\"{attrs}/>", xml_escape(&d));
            }
            "GlyphRunDrawing" | "ImageDrawing" | "VideoDrawing" => self.warn("glyph run, image and video drawings are left out"),
            _ => {
                // Unknown containers: their children still count.
                for c in e.children().filter(|c| c.is_element()) {
                    self.element(c, depth + 1);
                }
            }
        }
    }

    fn drawing_group_attrs(&mut self, e: XNode) -> String {
        let mut s = String::new();
        if let Some(o) = e.attribute("Opacity").and_then(|v| v.trim().parse::<f64>().ok()).filter(|o| *o < 1.0) {
            let _ = write!(s, " opacity=\"{}\"", n(o.max(0.0)));
        }
        let t = e
            .attribute("Transform")
            .map(matrix_attr)
            .or_else(|| Self::prop(e, "Transform").and_then(|p| p.children().find(|c| c.is_element())).map(|t| self.transform_element(t)))
            .filter(|t| !t.is_empty());
        if let Some(t) = t {
            let _ = write!(s, " transform=\"{t}\"");
        }
        let clip = e
            .attribute("ClipGeometry")
            .map(|g| strip_fill_rule(g).1)
            .or_else(|| Self::prop(e, "ClipGeometry").and_then(|p| p.children().find(|c| c.is_element())).map(|g| self.geometry_data(g)));
        if let Some(d) = clip.filter(|d| !d.is_empty()) {
            let id = self.next_id("clip");
            let _ = writeln!(self.defs, "<clipPath id=\"{id}\"><path d=\"{}\"/></clipPath>", xml_escape(&d));
            let _ = write!(s, " clip-path=\"url(#{id})\"");
        }
        s
    }
}

/// A `Matrix="a,b,c,d,e,f"` (or `Identity`) as an SVG transform.
fn matrix_attr(m: &str) -> String {
    if m.trim().eq_ignore_ascii_case("Identity") {
        return String::new();
    }
    let v = nums(m);
    if v.len() == 6 { format!("matrix({})", v.iter().map(|x| n(*x)).collect::<Vec<_>>().join(" ")) } else { String::new() }
}

/// The `F0`/`F1` fill-rule prefix of path data → (SVG fill rule, the data without it).
fn strip_fill_rule(d: &str) -> (&'static str, String) {
    let t = d.trim();
    if let Some(rest) = t.strip_prefix("F0") {
        return ("evenodd", rest.trim().to_string());
    }
    if let Some(rest) = t.strip_prefix("F1") {
        return ("nonzero", rest.trim().to_string());
    }
    ("evenodd", t.to_string())
}

/// XAML text as SVG text and conversion warnings.
pub fn to_svg(text: &str) -> Result<(String, Vec<String>), SvgError> {
    let xml = roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
        .map_err(|e| SvgError::Parse(e.to_string()))?;
    let mut c = Conv { defs: String::new(), body: String::new(), ids: 0, warnings: vec![], size: (0.0, 0.0) };
    c.element(xml.root_element(), 0);
    if c.body.trim().is_empty() {
        return Err(SvgError::Parse("the XAML has no drawing: no Canvas, Path, shape or Drawing elements".into()));
    }
    let (w, h) = if c.size.0 > 0.0 && c.size.1 > 0.0 { c.size } else { (800.0, 600.0) };
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><defs>{}</defs>{}</svg>",
        n(w),
        n(h),
        n(w),
        n(h),
        c.defs,
        c.body
    );
    Ok((svg, c.warnings))
}

/// Read a XAML drawing into a document.
pub fn import(text: &str) -> Result<(Document, Vec<String>), SvgError> {
    let (svg, mut warnings) = to_svg(text)?;
    let (doc, w) = crate::import_with(&svg, &ImportOptions::default())?;
    warnings.extend(w);
    Ok((doc, warnings))
}
