//! SVG 2 paint servers and legacy scaling read before usvg (which drops them): mesh gradients
//! (`<meshgradient>`, kept as gradient mesh objects), hatches (`<hatch>`, rewritten as patterns
//! of their hatch paths) and the 90-dpi scaling of files written by 0.9x-era editors (their
//! unitless sizes meant 1/90 inch: the root gets a `viewBox` and a 96-dpi size so the drawing
//! keeps its physical size).

use std::collections::HashMap;
use std::str::FromStr;

use redrob_vector_color::Color;
use redrob_vector_doc::live::{GradientMesh, H_DOWN, H_LEFT, H_RIGHT, H_UP, MeshPoint};
use redrob_vector_geom::{Affine, Point, Vec2};

use super::css::XNode;
use super::{Edits, Found, OBJECT_ELEMENTS};

/// The namespace of the editor whose 0.9x files are at 90 dpi.
const LEGACY_NS: &str = "http://www.inkscape.org/namespaces/inkscape";

pub(super) const TOKENS: [&str; 6] = ["meshgradient", "<hatch", ":hatch", "<font", ":font", LEGACY_NS];

/// Does `svg` need this pass?
pub(super) fn wanted(svg: &str) -> bool {
    TOKENS.iter().any(|t| svg.contains(t))
}

/// Find the SVG 2 features in `xml`, editing the text so usvg keeps the shapes they paint.
pub(super) fn find(svg: &str, xml: &usvg::roxmltree::Document, edits: &mut Edits, found: &mut Found, legacy_dpi: bool) {
    let elements = || xml.descendants().filter(|n| n.is_element());
    // Mesh gradients, by id.
    let mut meshes: HashMap<String, GradientMesh> = HashMap::new();
    for m in elements().filter(|n| n.tag_name().name() == "meshgradient") {
        let Some(id) = m.attribute("id").filter(|i| !i.is_empty()) else { continue };
        match parse_mesh(m) {
            Some(mesh) => {
                meshes.insert(id.to_string(), mesh);
            }
            None => found.warnings.push(format!("mesh gradient `{id}` isn't a regular grid of patches and is left out")),
        }
    }
    if !meshes.is_empty() {
        let css = super::css::Styles::new(xml);
        for e in elements().filter(|e| OBJECT_ELEMENTS.contains(&e.tag_name().name())) {
            let Some(fill) = css.own(e, "fill") else { continue };
            let Some(mesh_id) = url_id(&fill) else { continue };
            let Some(mesh) = meshes.get(mesh_id) else { continue };
            // usvg keeps the shape when it has a plain fill; the mesh replaces it on import.
            let avg = average(mesh);
            edits.style(svg, e, &format!("fill:{avg}"));
            found.meshes.insert(edits.id(svg, e), mesh.clone());
        }
        found.warnings.push("mesh gradients are read as gradient mesh objects in place of the shapes they filled".into());
    }
    // Hatches → patterns.
    for h in elements().filter(|n| n.tag_name().name() == "hatch") {
        let Some(id) = h.attribute("id").filter(|i| !i.is_empty()) else { continue };
        let pattern = hatch_pattern(h, id);
        edits.insert(h.range().start, pattern);
        edits.set(svg, h, "id", &format!("{id}-hatch"));
        found.warnings.push("hatches are read as patterns of their hatch paths (one pitch per tile)".into());
    }
    // SVG fonts, kept as the document's.
    for font in elements().filter(|n| n.tag_name().name() == "font") {
        if let Some(f) = parse_font(font) {
            found.svg_fonts.push(f);
        }
    }
    // Legacy 90-dpi files.
    let root = xml.root_element();
    if let Some(version) = root.attributes().find(|a| a.name() == "version" && a.namespace() == Some(LEGACY_NS)).map(|a| a.value())
        && is_legacy_version(version)
        && root.attribute("viewBox").is_none()
    {
        let num = |name: &str| root.attribute(name).and_then(unitless).filter(|v| v.is_finite() && *v > 0.0);
        if let (Some(w), Some(h)) = (num("width"), num("height")) {
            if legacy_dpi {
                edits.set(svg, root, "viewBox", &format!("0 0 {} {}", n(w), n(h)));
                edits.set(svg, root, "width", &n(w * 96.0 / 90.0));
                edits.set(svg, root, "height", &n(h * 96.0 / 90.0));
                found.warnings.push(format!(
                    "the file was saved by a 90-dpi editor (version {version}): its drawing was scaled by 96/90 to keep its physical size (document.open legacyDpi: false keeps the user units)"
                ));
            } else {
                found.warnings.push(format!(
                    "the file was saved by a 90-dpi editor (version {version}): its user units are read at 96 dpi, so it is 6.25% smaller"
                ));
            }
        }
    }
}

fn n(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".into() } else { s.to_string() }
}

/// `version` is `0.x` with `x` below 92 (`0.91 r13725`, `0.48.5 r10040`…).
fn is_legacy_version(version: &str) -> bool {
    let v = version.split_whitespace().next().unwrap_or("");
    let mut parts = v.split('.');
    let major: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(99);
    let minor: u32 = parts.next().and_then(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()).unwrap_or(99);
    major == 0 && minor < 92
}

/// A length with no unit, or in `px`.
fn unitless(s: &str) -> Option<f64> {
    let t = s.trim();
    let t = t.strip_suffix("px").unwrap_or(t);
    if t.chars().any(|c| c.is_ascii_alphabetic() || c == '%') {
        return None;
    }
    t.trim().parse().ok()
}

/// The id a `url(#id)` paint names.
fn url_id(paint: &str) -> Option<&str> {
    let p = paint.trim();
    let inner = p.strip_prefix("url(")?.split(')').next()?.trim().trim_matches(['"', '\'']);
    inner.strip_prefix('#')
}

/// A stop's colour and opacity from its attributes or `style`.
fn stop_color(s: XNode) -> (Color, f32) {
    let style = s.attribute("style").unwrap_or("");
    let decl = |name: &str| -> Option<String> {
        style.split(';').find_map(|d| {
            let (k, v) = d.split_once(':')?;
            (k.trim() == name).then(|| v.trim().to_string())
        })
    };
    let color = decl("stop-color").or_else(|| s.attribute("stop-color").map(str::to_string)).unwrap_or_else(|| "black".into());
    let opacity =
        decl("stop-opacity").or_else(|| s.attribute("stop-opacity").map(str::to_string)).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(1.0);
    let c = svgtypes::Color::from_str(&color).unwrap_or(svgtypes::Color::new_rgb(0, 0, 0));
    (Color::rgb8(c.red, c.green, c.blue), opacity.clamp(0.0, 1.0) * f32::from(c.alpha) / 255.0)
}

/// One edge of a patch: its two control points and its end, from `path` (`c`/`C` a cubic,
/// `l`/`L` a line) starting at `from`.
fn edge(path: &str, from: Point) -> Option<(Point, Point, Point)> {
    let mut it = path.trim().chars();
    let cmd = it.next()?;
    let rest: Vec<f64> =
        it.as_str().split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect();
    let rel = cmd.is_ascii_lowercase();
    let pt = |x: f64, y: f64| if rel { from + Vec2::new(x, y) } else { Point::new(x, y) };
    match cmd.to_ascii_lowercase() {
        'c' if rest.len() >= 6 => Some((pt(rest[0], rest[1]), pt(rest[2], rest[3]), pt(rest[4], rest[5]))),
        'l' if rest.len() >= 2 => {
            let to = pt(rest[0], rest[1]);
            Some((from.lerp(to, 1.0 / 3.0), from.lerp(to, 2.0 / 3.0), to))
        }
        _ => None,
    }
}

/// The transform a `gradientTransform` attribute names (translate, scale, rotate, matrix).
fn gradient_transform(m: XNode) -> Affine {
    let Some(t) = m.attribute("gradientTransform") else { return Affine::IDENTITY };
    let mut out = Affine::IDENTITY;
    if let Ok(ts) = svgtypes::Transform::from_str(t) {
        out = Affine::new([ts.a, ts.b, ts.c, ts.d, ts.e, ts.f]);
    }
    out
}

/// A `<meshgradient>` as a gradient mesh: a regular grid of patches whose corners meet.
fn parse_mesh(m: XNode) -> Option<GradientMesh> {
    let xf = gradient_transform(m);
    let x: f64 = m.attribute("x").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
    let y: f64 = m.attribute("y").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
    let rows: Vec<XNode> = m.children().filter(|c| c.is_element() && c.tag_name().name() == "meshrow").collect();
    let nrows = rows.len();
    let ncols = rows.first()?.children().filter(|c| c.is_element() && c.tag_name().name() == "meshpatch").count();
    if nrows == 0 || ncols == 0 || nrows * ncols > 4096 {
        return None;
    }
    let cols1 = ncols + 1;
    let blank = MeshPoint { p: Point::ZERO, color: Color::rgb8(0, 0, 0), opacity: 1.0, handles: [Vec2::ZERO; 4] };
    let mut pts: Vec<MeshPoint> = vec![blank; (nrows + 1) * cols1];
    let mut set: Vec<bool> = vec![false; (nrows + 1) * cols1];
    let idx = |r: usize, c: usize| r * cols1 + c;
    for (r, row) in rows.iter().enumerate() {
        let patches: Vec<XNode> = row.children().filter(|c| c.is_element() && c.tag_name().name() == "meshpatch").collect();
        if patches.len() != ncols {
            return None;
        }
        for (c, patch) in patches.iter().enumerate() {
            let stops: Vec<XNode> = patch.children().filter(|s| s.is_element() && s.tag_name().name() == "stop").collect();
            // Which edges this patch gives: the top is shared with the row above, the left
            // with the patch before.
            let has_top = r == 0;
            let has_left = c == 0;
            let want = 2 + usize::from(has_top) + usize::from(has_left);
            if stops.len() < want {
                return None;
            }
            // Corners: 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left.
            let corners = [idx(r, c), idx(r, c + 1), idx(r + 1, c + 1), idx(r + 1, c)];
            if r == 0 && c == 0 {
                pts[corners[0]].p = Point::new(x, y);
                set[corners[0]] = true;
            }
            let mut si = 0;
            for e in 0..4 {
                let shared = (e == 0 && !has_top) || (e == 3 && !has_left);
                if shared {
                    continue;
                }
                let stop = *stops.get(si)?;
                si += 1;
                let (from_i, to_i) = (corners[e], corners[(e + 1) % 4]);
                let from = pts[from_i].p;
                let (c1, c2, to) = edge(stop.attribute("path").unwrap_or(""), from)?;
                if !set[to_i] {
                    pts[to_i].p = to;
                    set[to_i] = true;
                }
                let (color, opacity) = stop_color(stop);
                // The stop's colour is its starting corner's, unless that corner is shared.
                let start_shared = match e {
                    0 => !has_left || !has_top,
                    1 => !has_top,
                    2 => false,
                    _ => !has_left,
                };
                if !start_shared || (r == 0 && c == 0) {
                    pts[from_i].color = color;
                    pts[from_i].opacity = opacity;
                }
                // Handles: from's handle toward to, and to's handle back.
                let (h_from, h_to) = match e {
                    0 => (H_RIGHT, H_LEFT),
                    1 => (H_DOWN, H_UP),
                    2 => (H_LEFT, H_RIGHT),
                    _ => (H_UP, H_DOWN),
                };
                pts[from_i].handles[h_from] = c1 - from;
                pts[to_i].handles[h_to] = c2 - pts[to_i].p;
            }
        }
    }
    // Into user space.
    for p in &mut pts {
        let at = xf * p.p;
        for h in &mut p.handles {
            *h = (xf * (p.p + *h)) - at;
        }
        p.p = at;
    }
    Some(GradientMesh { rows: nrows as u32, cols: ncols as u32, points: pts })
}

/// The average colour of a mesh's corners, as a CSS colour.
fn average(m: &GradientMesh) -> String {
    let n = m.points.len().max(1) as f64;
    let mut acc = [0.0f64; 3];
    for p in &m.points {
        let [r, g, b, _] = p.color.to_rgba8(1.0);
        acc[0] += f64::from(r);
        acc[1] += f64::from(g);
        acc[2] += f64::from(b);
    }
    format!("#{:02x}{:02x}{:02x}", (acc[0] / n).round() as u8, (acc[1] / n).round() as u8, (acc[2] / n).round() as u8)
}

/// Mesh `m` (user space) placed by `acc` as a mesh object.
pub(super) fn mesh_node(m: &GradientMesh, acc: Affine) -> GradientMesh {
    let mut out = m.clone();
    for p in &mut out.points {
        let at = acc * p.p;
        for h in &mut p.handles {
            *h = (acc * (p.p + *h)) - at;
        }
        p.p = at;
    }
    out
}

fn esc(s: &str) -> String {
    crate::xml_escape(s)
}

/// A `<pattern>` drawing hatch `h`: one tile per pitch, each hatch path a stroked line (or its
/// own `d`) at its offset, the whole turned by `rotate`.
fn hatch_pattern(h: XNode, id: &str) -> String {
    let num = |name: &str, d: f64| h.attribute(name).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(d);
    let pitch = num("pitch", 10.0).abs().max(0.01);
    let rotate = num("rotate", 0.0);
    let (x, y) = (num("x", 0.0), num("y", 0.0));
    let mut height = pitch;
    let mut paths = String::new();
    for hp in h.children().filter(|c| c.is_element() && c.tag_name().name() == "hatchpath") {
        let offset = hp.attribute("offset").and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0);
        let mut attrs = String::new();
        let mut has_stroke = false;
        for a in hp.attributes().filter(|a| a.namespace().is_none()) {
            match a.name() {
                "stroke" | "stroke-width" | "stroke-dasharray" | "stroke-dashoffset" | "stroke-linecap" | "stroke-linejoin" | "stroke-opacity"
                | "style" => {
                    if a.name() == "stroke" || (a.name() == "style" && a.value().contains("stroke")) {
                        has_stroke = true;
                    }
                    attrs.push_str(&format!(" {}=\"{}\"", a.name(), esc(a.value())));
                }
                _ => {}
            }
        }
        if !has_stroke {
            attrs.push_str(" stroke=\"black\"");
        }
        match hp.attribute("d").filter(|d| !d.trim().is_empty()) {
            Some(d) => {
                // The path repeats down the tile: a tile as tall as the path when it says.
                if let Some(h) = path_height(d) {
                    height = height.max(h);
                }
                paths.push_str(&format!("<path d=\"{}\" transform=\"translate({} 0)\" fill=\"none\"{attrs}/>", esc(d), n(offset)));
            }
            None => paths.push_str(&format!("<path d=\"M{} -1 V {}\" fill=\"none\"{attrs}/>", n(offset), n(height + 1.0))),
        }
    }
    if paths.is_empty() {
        paths.push_str(&format!("<path d=\"M0 0 V {}\" stroke=\"black\" fill=\"none\"/>", n(pitch)));
    }
    format!(
        "<pattern id=\"{}\" patternUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" patternTransform=\"rotate({})\">{paths}</pattern>",
        esc(id),
        n(x),
        n(y),
        n(pitch),
        n(height),
        n(rotate)
    )
}

/// The vertical extent of path data `d` (its y range), for a hatch path's tile.
fn path_height(d: &str) -> Option<f64> {
    let mut ys: Vec<f64> = vec![];
    for seg in svgtypes::SimplifyingPathParser::from(d) {
        let Ok(seg) = seg else { break };
        match seg {
            svgtypes::SimplePathSegment::MoveTo { y, .. } | svgtypes::SimplePathSegment::LineTo { y, .. } => ys.push(y),
            svgtypes::SimplePathSegment::CurveTo { y, y1, y2, .. } => ys.extend([y, y1, y2]),
            svgtypes::SimplePathSegment::Quadratic { y, y1, .. } => ys.extend([y, y1]),
            svgtypes::SimplePathSegment::ClosePath => {}
        }
    }
    let (min, max) = ys.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), y| (a.min(*y), b.max(*y)));
    (max > min).then_some(max - min)
}

/// A `<font>` element as an SVG font: its face, glyphs (with their `d` outlines) and kerning.
fn parse_font(font: XNode) -> Option<redrob_vector_doc::SvgFont> {
    use redrob_vector_doc::{SvgFont, SvgGlyph, SvgKern};
    let num = |n: XNode, name: &str| n.attribute(name).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
    let face = font.children().find(|c| c.is_element() && c.tag_name().name() == "font-face");
    let family = face.and_then(|f| f.attribute("font-family")).map(|s| s.trim().trim_matches(['"', '\'']).to_string());
    let id = font.attribute("id").map(str::to_string).or_else(|| family.clone()).filter(|s| !s.is_empty())?;
    let mut f = SvgFont::new(&id, family.as_deref().unwrap_or(&id));
    if let Some(u) = face.and_then(|fc| num(fc, "units-per-em")).filter(|u| *u > 0.0) {
        f.units_per_em = u;
    }
    if let Some(a) = num(font, "horiz-adv-x") {
        f.advance = a.max(0.0);
    }
    f.ascent = face.and_then(|fc| num(fc, "ascent"));
    f.descent = face.and_then(|fc| num(fc, "descent"));
    for g in font.children().filter(|c| c.is_element() && c.tag_name().name() == "glyph") {
        let Some(unicode) = g.attribute("unicode").filter(|u| !u.is_empty()) else { continue };
        let mut bp = kurbo::BezPath::new();
        if let Some(d) = g.attribute("d") {
            for seg in svgtypes::SimplifyingPathParser::from(d) {
                let Ok(seg) = seg else { break };
                match seg {
                    svgtypes::SimplePathSegment::MoveTo { x, y } => bp.move_to((x, y)),
                    svgtypes::SimplePathSegment::LineTo { x, y } => bp.line_to((x, y)),
                    svgtypes::SimplePathSegment::Quadratic { x1, y1, x, y } => bp.quad_to((x1, y1), (x, y)),
                    svgtypes::SimplePathSegment::CurveTo { x1, y1, x2, y2, x, y } => bp.curve_to((x1, y1), (x2, y2), (x, y)),
                    svgtypes::SimplePathSegment::ClosePath => bp.close_path(),
                }
            }
        }
        f.glyphs.push(SvgGlyph {
            unicode: unicode.to_string(),
            name: g.attribute("glyph-name").unwrap_or("").to_string(),
            advance: num(g, "horiz-adv-x"),
            path: redrob_vector_geom::PathData::from_bezpath(&bp),
        });
        if f.glyphs.len() > 20_000 {
            break;
        }
    }
    for k in font.children().filter(|c| c.is_element() && c.tag_name().name() == "hkern") {
        if let (Some(u1), Some(u2), Some(kv)) = (k.attribute("u1"), k.attribute("u2"), num(k, "k")) {
            f.kerning.push(SvgKern { left: u1.to_string(), right: u2.to_string(), k: kv });
        }
    }
    Some(f)
}
