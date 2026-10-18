//! Object › Object Attributes…: the raw SVG attributes of the selected element as a file would
//! carry them (`x`, `y`, `width`, `height`, `rx`, `ry` of a rectangle; `cx`, `cy`, `r` / `rx`,
//! `ry` of an ellipse; `points`; `d`; `x1`…; `href` of an image; `id`, `transform`, `fill`,
//! `stroke`, `stroke-width`, `opacity`, `title`, `desc`), read with `object.xmlAttributes` and
//! edited with `object.setXmlAttribute`, which maps each attribute back onto the live shape
//! (through the shape, paint, transparency and attribute commands) as one undo step.

use redrob_vector_doc::{Document, LiveShape, Node, NodeKind, xml_id};
use redrob_vector_geom::{Affine, PathEl, Point};
use serde_json::{Map, Value, json};

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "object.xmlAttributes",
            "Object Attributes…",
            ["Object"],
            None,
            "{id?} the selected element's attributes as an SVG file carries them → {id, element: rect|ellipse|circle|polygon|path|image|text|g|use…, attributes: {name: value} (x, y, width, height, rx, ry, cx, cy, r, points, d, x1, y1, x2, y2, href, id, transform, fill, stroke, stroke-width, opacity, title, desc as the element has them), editable: [names object.setXmlAttribute takes for it]}",
            has_selection,
            attributes
        ),
        cmd!(
            "object.setXmlAttribute",
            "Set Object Attribute",
            [],
            None,
            "{id?, name, value: string} set one attribute of the selected element as object.xmlAttributes lists them: x, y, width, height (the bounds), rx/ry (a rectangle's corner radii), cx, cy, r, rx, ry (an ellipse), id (the XML id, stored as the object name), transform (matrix(a,b,c,d,e,f) / translate / scale / rotate, applied over the current one), fill, stroke (a colour, or none), stroke-width, opacity (0–1), title, desc → {changed}; `values: {name: value}` sets several in one undo step",
            has_selection,
            set_attribute
        ),
    ]
}

fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.to_string() }
}

/// `xf` as an SVG `transform` value (empty: identity).
fn transform_attr(xf: Affine) -> String {
    let c = xf.as_coeffs();
    if (c[0] - 1.0).abs() < 1e-9 && c[1].abs() < 1e-9 && c[2].abs() < 1e-9 && (c[3] - 1.0).abs() < 1e-9 {
        if c[4].abs() < 1e-9 && c[5].abs() < 1e-9 {
            return String::new();
        }
        return format!("translate({} {})", num(c[4]), num(c[5]));
    }
    format!("matrix({} {} {} {} {} {})", num(c[0]), num(c[1]), num(c[2]), num(c[3]), num(c[4]), num(c[5]))
}

/// The rotation, scale and shear of `xf` as a transform about `at` (its translation dropped).
fn about(xf: Affine, at: Point) -> Affine {
    let c = xf.as_coeffs();
    let linear = Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0]);
    Affine::translate(at.to_vec2()) * linear * Affine::translate(-at.to_vec2())
}

fn paint_attr(p: &redrob_vector_color::Paint) -> String {
    match p {
        redrob_vector_color::Paint::None => "none".into(),
        redrob_vector_color::Paint::Solid { color, .. } => color.to_hex(),
        redrob_vector_color::Paint::Gradient(_) => "url(#gradient)".into(),
        redrob_vector_color::Paint::Pattern { pattern, .. } => format!("url(#{pattern})"),
    }
}

/// The element and attributes of node `n` in document `d`.
fn describe(d: &Document, n: &Node) -> (String, Map<String, Value>, Vec<&'static str>) {
    let mut a = Map::new();
    let mut editable: Vec<&'static str> = vec!["id", "transform", "opacity", "title", "desc"];
    let put = |a: &mut Map<String, Value>, k: &str, v: String| {
        a.insert(k.to_string(), Value::String(v));
    };
    if let Some(name) = &n.name {
        put(&mut a, "id", xml_id::from_object_name(name));
    } else {
        put(&mut a, "id", String::new());
    }
    let element = match &n.kind {
        NodeKind::Path { live: Some(l), path, .. } => match &**l {
            LiveShape::Rectangle { w, h, radii, xf, .. } => {
                let o = *xf * Point::ZERO;
                put(&mut a, "x", num(o.x));
                put(&mut a, "y", num(o.y));
                put(&mut a, "width", num(*w));
                put(&mut a, "height", num(*h));
                put(&mut a, "rx", num(radii[0]));
                put(&mut a, "ry", num(radii[0]));
                put(&mut a, "transform", transform_attr(about(*xf, o)));
                editable.extend(["x", "y", "width", "height", "rx", "ry"]);
                "rect"
            }
            LiveShape::Ellipse { w, h, xf, .. } => {
                let c = *xf * Point::new(w / 2.0, h / 2.0);
                put(&mut a, "cx", num(c.x));
                put(&mut a, "cy", num(c.y));
                if (w - h).abs() < 1e-6 {
                    put(&mut a, "r", num(w / 2.0));
                    editable.extend(["cx", "cy", "r"]);
                } else {
                    put(&mut a, "rx", num(w / 2.0));
                    put(&mut a, "ry", num(h / 2.0));
                    editable.extend(["cx", "cy", "rx", "ry"]);
                }
                put(&mut a, "transform", transform_attr(about(*xf, c)));
                if (w - h).abs() < 1e-6 { "circle" } else { "ellipse" }
            }
            LiveShape::Polygon { .. } | LiveShape::Star { .. } => {
                let bp = path.to_bezpath();
                let pts: Vec<String> = bp
                    .elements()
                    .iter()
                    .filter_map(|e| match e {
                        PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(format!("{},{}", num(p.x), num(p.y))),
                        _ => None,
                    })
                    .collect();
                put(&mut a, "points", pts.join(" "));
                editable.extend(["x", "y", "width", "height"]);
                "polygon"
            }
            LiveShape::Line { .. } => {
                let bp = path.to_bezpath();
                let pts: Vec<Point> = bp
                    .elements()
                    .iter()
                    .filter_map(|e| match e {
                        PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
                        _ => None,
                    })
                    .collect();
                if let [p1, p2, ..] = pts.as_slice() {
                    put(&mut a, "x1", num(p1.x));
                    put(&mut a, "y1", num(p1.y));
                    put(&mut a, "x2", num(p2.x));
                    put(&mut a, "y2", num(p2.y));
                }
                editable.extend(["x", "y", "width", "height"]);
                "line"
            }
            _ => {
                put(&mut a, "d", path.to_bezpath().to_svg());
                editable.extend(["x", "y", "width", "height"]);
                "path"
            }
        },
        NodeKind::Path { path, .. } => {
            put(&mut a, "d", path.to_bezpath().to_svg());
            editable.extend(["x", "y", "width", "height"]);
            "path"
        }
        NodeKind::Compound { .. } => {
            editable.extend(["x", "y", "width", "height"]);
            "path"
        }
        NodeKind::Image(im) => {
            let o = im.xf * Point::ZERO;
            put(&mut a, "x", num(o.x));
            put(&mut a, "y", num(o.y));
            let sz = im.xf * Point::new(f64::from(im.width), f64::from(im.height));
            put(&mut a, "width", num((sz.x - o.x).abs()));
            put(&mut a, "height", num((sz.y - o.y).abs()));
            let href = im
                .link
                .as_ref()
                .map(|l| l.path.clone())
                .unwrap_or_else(|| format!("data:{};base64,…", d.images.get(&im.key).map(|b| b.mime.as_str()).unwrap_or("image/png")));
            put(&mut a, "href", href);
            if let Some(par) = n.extras().attrs.as_deref().and_then(|at| at.preserve_aspect_ratio.clone()) {
                put(&mut a, "preserveAspectRatio", par);
            }
            editable.extend(["x", "y", "width", "height", "preserveAspectRatio"]);
            "image"
        }
        NodeKind::Text(t) => {
            let o = t.xf * Point::ZERO;
            put(&mut a, "x", num(o.x));
            put(&mut a, "y", num(o.y));
            if let Some(r) = t.runs.first() {
                put(&mut a, "font-family", r.style.font_family.clone());
                put(&mut a, "font-size", num(r.style.size));
            }
            editable.extend(["x", "y"]);
            "text"
        }
        NodeKind::SymbolInstance { symbol, .. } => {
            put(&mut a, "href", format!("#{}", xml_id::from_object_name(symbol)));
            editable.extend(["x", "y", "width", "height"]);
            "use"
        }
        NodeKind::Group { .. } | NodeKind::Layer { .. } => {
            if n.url().is_some() {
                put(&mut a, "href", n.url().unwrap_or("").to_string());
                "a"
            } else {
                "g"
            }
        }
        NodeKind::Mesh(_) => "meshgradient",
        _ => "g",
    };
    // Paint and transparency, as presentation attributes.
    for it in &n.appearance.items {
        match it {
            redrob_vector_doc::AppearanceItem::Fill(f) if !a.contains_key("fill") => put(&mut a, "fill", paint_attr(&f.paint)),
            redrob_vector_doc::AppearanceItem::Stroke(s) if !a.contains_key("stroke") => {
                put(&mut a, "stroke", paint_attr(&s.paint));
                put(&mut a, "stroke-width", num(s.width));
            }
            _ => {}
        }
    }
    if !matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. } | NodeKind::Image(_)) {
        a.entry("fill").or_insert(json!("none"));
        a.entry("stroke").or_insert(json!("none"));
        editable.extend(["fill", "stroke", "stroke-width"]);
    }
    put(&mut a, "opacity", num(f64::from(n.opacity)));
    if let Some(at) = n.extras().attrs.as_deref() {
        if !at.title.is_empty() {
            put(&mut a, "title", at.title.clone());
        }
        if !at.description.is_empty() {
            put(&mut a, "desc", at.description.clone());
        }
    }
    a.entry("title").or_insert(json!(""));
    a.entry("desc").or_insert(json!(""));
    if !a.contains_key("transform") {
        put(&mut a, "transform", String::new());
    }
    (element.to_string(), a, editable)
}

fn target(s: &Session, p: &Value) -> Result<NodeId> {
    const C: &str = "object.xmlAttributes";
    let st = s.doc()?;
    let id = match id_param(p, "id") {
        Some(id) => id,
        None => *st.selection.objects.first().ok_or_else(|| bad(C, "select an object"))?,
    };
    st.doc.node(id).map(|_| id).ok_or_else(|| bad(C, "no such object"))
}

fn attributes(s: &mut Session, p: &Value) -> Result<Value> {
    let id = target(s, p)?;
    let st = s.doc()?;
    let n = st.doc.node(id).ok_or_else(|| bad("object.xmlAttributes", "no such object"))?;
    let (element, a, editable) = describe(&st.doc, n);
    Ok(json!({ "id": id.0, "element": element, "attributes": a, "editable": editable }))
}

/// An SVG transform list (`matrix(…)`, `translate(…)`, `scale(…)`, `rotate(…)`, `skewX`, `skewY`).
fn parse_transform(s: &str) -> Option<Affine> {
    let mut out = Affine::IDENTITY;
    let mut rest = s.trim();
    if rest.is_empty() {
        return Some(out);
    }
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let close = rest.find(')')?;
        let name = rest[..open].trim();
        let nums: Vec<f64> = rest[open + 1..close]
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .map(|t| t.parse().ok())
            .collect::<Option<_>>()?;
        let t = match (name, nums.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine::new([*a, *b, *c, *d, *e, *f]),
            ("translate", [x]) => Affine::translate((*x, 0.0)),
            ("translate", [x, y]) => Affine::translate((*x, *y)),
            ("scale", [k]) => Affine::scale(*k),
            ("scale", [x, y]) => Affine::scale_non_uniform(*x, *y),
            ("rotate", [a]) => Affine::rotate(a.to_radians()),
            ("rotate", [a, cx, cy]) => Affine::translate((*cx, *cy)) * Affine::rotate(a.to_radians()) * Affine::translate((-cx, -cy)),
            ("skewX", [a]) => Affine::skew(a.to_radians().tan(), 0.0),
            ("skewY", [a]) => Affine::skew(0.0, a.to_radians().tan()),
            _ => return None,
        };
        out *= t;
        rest = rest[close + 1..].trim_start_matches([',', ' ', '\t', '\n']);
    }
    Some(out)
}

fn set_attribute(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.setXmlAttribute";
    let id = target(s, p)?;
    let mut pairs: Vec<(String, String)> = vec![];
    if let Some(Value::Object(o)) = p.get("values") {
        for (k, v) in o {
            let v = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            pairs.push((k.clone(), v));
        }
    }
    if let Some(name) = str_param(p, "name") {
        let value = match p.get("value") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Bool(b)) => b.to_string(),
            _ => return Err(bad(C, "missing `value`")),
        };
        pairs.push((name.to_string(), value));
    }
    if pairs.is_empty() {
        return Err(bad(C, "give `name` and `value`, or `values`"));
    }
    // Current attributes, for the ones whose change is relative to them.
    let (element, current) = {
        let st = s.doc()?;
        let n = st.doc.node(id).ok_or_else(|| bad(C, "no such object"))?;
        let (e, a, _) = describe(&st.doc, n);
        (e, a)
    };
    let cur = |k: &str| current.get(k).and_then(Value::as_str).and_then(|v| v.parse::<f64>().ok());
    let mut steps: Vec<(String, Value)> = vec![];
    let ids = json!([id.0]);
    for (name, value) in &pairs {
        let value = value.trim();
        let number = || value.parse::<f64>().map_err(|_| bad(C, format!("`{name}` must be a number, not {value:?}")));
        match name.as_str() {
            "id" => {
                let object_name = if value.is_empty() { String::new() } else { xml_id::to_object_name(value) };
                steps.push(("layer.setProps".into(), json!({ "ids": ids, "name": object_name })));
            }
            "x" | "y" | "width" | "height" => {
                let v = number()?;
                if matches!(name.as_str(), "width" | "height") && v <= 0.0 {
                    return Err(bad(C, format!("`{name}` must be above 0")));
                }
                steps.push(("object.setBounds".into(), json!({ "ids": ids, name: v, "reference": 0 })));
            }
            "cx" | "cy" => {
                let v = number()?;
                let (w, h) = (
                    cur("rx").map(|r| r * 2.0).or_else(|| cur("r").map(|r| r * 2.0)).or_else(|| cur("width")).unwrap_or(0.0),
                    cur("ry").map(|r| r * 2.0).or_else(|| cur("r").map(|r| r * 2.0)).or_else(|| cur("height")).unwrap_or(0.0),
                );
                let key = if name == "cx" { "x" } else { "y" };
                let half = if name == "cx" { w / 2.0 } else { h / 2.0 };
                steps.push(("object.setBounds".into(), json!({ "ids": ids, key: v - half, "reference": 0 })));
            }
            "r" => {
                let v = number()?;
                if v <= 0.0 {
                    return Err(bad(C, "`r` must be above 0"));
                }
                steps.push(("object.setBounds".into(), json!({ "ids": ids, "width": v * 2.0, "height": v * 2.0, "reference": 4 })));
            }
            "rx" | "ry" if element == "ellipse" || element == "circle" => {
                let v = number()?;
                if v <= 0.0 {
                    return Err(bad(C, format!("`{name}` must be above 0")));
                }
                let key = if name == "rx" { "width" } else { "height" };
                steps.push(("object.setBounds".into(), json!({ "ids": ids, key: v * 2.0, "reference": 4 })));
            }
            "rx" | "ry" => {
                let v = number()?;
                steps.push(("object.setLiveShape".into(), json!({ "ids": ids, "radius": v.max(0.0) })));
            }
            "transform" => {
                let want = parse_transform(value).ok_or_else(|| {
                    bad(C, format!("`transform` must be matrix(…), translate(…), scale(…), rotate(…), skewX(…) or skewY(…), not {value:?}"))
                })?;
                let have = current.get("transform").and_then(Value::as_str).and_then(parse_transform).unwrap_or(Affine::IDENTITY);
                // The new transform replaces the old: apply the difference.
                let delta = want * have.inverse();
                let c = delta.as_coeffs();
                if c.iter().any(|v| !v.is_finite()) {
                    return Err(bad(C, "`transform` isn't invertible"));
                }
                steps.push(("object.transform".into(), json!({ "ids": ids, "matrix": c })));
            }
            "fill" | "stroke" => {
                let cmd = if name == "fill" { "paint.setFill" } else { "paint.setStroke" };
                if value.eq_ignore_ascii_case("none") || value.is_empty() {
                    steps.push((cmd.into(), json!({ "ids": ids, "none": true })));
                } else {
                    let color = redrob_vector_color::Color::from_hex(value)
                        .ok_or_else(|| bad(C, format!("`{name}` must be a colour such as #ff8800, or none")))?;
                    steps.push((cmd.into(), json!({ "ids": ids, "color": color.to_hex() })));
                }
            }
            "stroke-width" => {
                let v = number()?;
                steps.push(("stroke.set".into(), json!({ "ids": ids, "weight": v.max(0.0) })));
            }
            "opacity" => {
                let v = number()?;
                let pct = if v <= 1.0 { v * 100.0 } else { v };
                steps.push(("transparency.set".into(), json!({ "ids": ids, "opacity": pct.clamp(0.0, 100.0) })));
            }
            "title" => steps.push(("attributes.set".into(), json!({ "ids": ids, "title": value }))),
            "desc" | "description" => steps.push(("attributes.set".into(), json!({ "ids": ids, "description": value }))),
            "preserveAspectRatio" => steps.push((
                "attributes.set".into(),
                json!({ "ids": ids, "preserveAspectRatio": if value.is_empty() { Value::Null } else { json!(value) } }),
            )),
            other => return Err(bad(C, format!("`{other}` can't be set here: object.xmlAttributes lists the editable attributes"))),
        }
    }
    // One undo step for them all.
    let commands: Vec<Value> = steps.into_iter().map(|(c, p)| json!({ "command": c, "params": p })).collect();
    s.execute("command.batch", &json!({ "label": "Object Attributes", "commands": commands }))?;
    Ok(json!({ "changed": pairs.len() }))
}
