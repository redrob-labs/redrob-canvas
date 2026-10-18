//! Type › SVG Font Editor: SVG fonts made inside the document (`svgFont.*`): create a font, list
//! its glyphs with their unicode, name and advance, take a glyph's curves from the selection or
//! put them back as objects, set kerning pairs, set a preview string as paths, and turn glyph
//! layers into a font and a font into glyph layers (the typography round trip).

use std::sync::Arc;

use redrob_vector_color::{Color, Paint};
use redrob_vector_doc::{Appearance, LayerColor, Node, NodeKind, SvgFont, SvgGlyph, SvgKern};
use redrob_vector_geom::{Affine, BezPath, PathData, Point, Shape};
use serde_json::{Value, json};

use super::edit::selected_roots;
use super::path::paths_in;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "svgFont.create",
            "New SVG Font",
            [],
            None,
            "{family?: name (default SVG Font 1, 2…), unitsPerEm?: 1000, advance?: 1000 (the default horiz-adv-x)} make an SVG font in the document → {id, family}",
            has_doc,
            create
        ),
        cmd!(
            query "svgFont.list",
            "SVG Fonts",
            [],
            None,
            "{} → {fonts: [{id, family, unitsPerEm, advance, glyphs: [{unicode, name, advance, hasPath}], kerning: [{left, right, k}]}]}",
            has_doc,
            list
        ),
        cmd!("svgFont.delete", "Delete SVG Font", [], None, "{font: id|family} remove an SVG font → {removed}", has_doc, delete),
        cmd!(
            "svgFont.set",
            "SVG Font Options",
            [],
            None,
            "{font, family?, unitsPerEm?, advance?, ascent?, descent?} change a font's settings → {id}",
            has_doc,
            set
        ),
        cmd!(
            "svgFont.setGlyph",
            "Set Glyph",
            [],
            None,
            "{font, unicode: the character(s) (a new glyph when none has it), name?, advance?: font units|null, newUnicode?} add a glyph or change its name, advance or character → {unicode}",
            has_doc,
            set_glyph
        ),
        cmd!("svgFont.removeGlyph", "Remove Glyph", [], None, "{font, unicode} remove a glyph → {removed}", has_doc, remove_glyph),
        cmd!(
            "svgFont.glyphFromSelection",
            "Get Curves from Selection",
            [],
            None,
            "{font, unicode, name?, emSize?: pt (how tall one em is in the drawing; default: the selection's height), baseline?: y (document; default: the selection's bottom), left?: x (default: the selection's left), ids?} the selected paths' outlines become the glyph's curves, scaled to the font's units → {unicode, anchors}",
            has_selection,
            glyph_from_selection
        ),
        cmd!(
            "svgFont.glyphToSelection",
            "Glyph to Objects",
            [],
            None,
            "{font, unicode, at?: [x, y] (the baseline start; default: the first artboard's centre), emSize?: 100} place the glyph's curves as a path and select it → {id}",
            has_doc,
            glyph_to_objects
        ),
        cmd!(
            "svgFont.setKerning",
            "Set Kerning Pair",
            [],
            None,
            "{font, left, right, k: font units (0 removes the pair)} → {pairs}",
            has_doc,
            set_kerning
        ),
        cmd!(
            "svgFont.preview",
            "Preview SVG Font",
            [],
            None,
            "{font, text, at?: [x, y] (the baseline start; default: the first artboard's centre), emSize?: 72 (pt)} set `text` in the font as a group of paths (kerning applied) → {id, glyphs, missing: [characters without a glyph]}",
            has_doc,
            preview
        ),
        cmd!(
            "svgFont.fromLayers",
            "Glyph Layers to SVG Font",
            [],
            None,
            "{font?: id|family (default: a new font named after the document), emSize?: pt (default: the tallest layer's art), baseline?: y (default: the bottom of the tallest layer's art)} every layer named by one character (or `space`, `period`… a glyph name) becomes that glyph, its art the glyph's curves → {id, glyphs}",
            has_doc,
            from_layers
        ),
        cmd!(
            "svgFont.toLayers",
            "SVG Font to Glyph Layers",
            [],
            None,
            "{font, emSize?: 100 (pt), at?: [x, y] (the baseline start of every glyph; default: the first artboard's top-left plus the em)} one layer per glyph, named by its character, holding its curves as a path → {layers}",
            has_doc,
            to_layers
        ),
    ]
}

fn font_index(d: &Document, p: &Value, cmd: &str) -> Result<usize> {
    let key = str_param(p, "font")
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .ok_or_else(|| bad(cmd, "missing `font` (an id or family; see svgFont.list)"))?;
    d.svg_fonts.iter().position(|f| f.id == key || f.family == key).ok_or_else(|| bad(cmd, format!("no SVG font `{key}` (see svgFont.list)")))
}

fn font_id(family: &str, taken: &[SvgFont]) -> String {
    let base: String = family.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let base = if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) { format!("font-{base}") } else { base };
    let mut id = base.clone();
    let mut n = 2;
    while taken.iter().any(|f| f.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFont.create";
    let family = match str_param(p, "family").map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => f.to_string(),
        None => {
            let d = &s.doc()?.doc;
            (1..).map(|i| format!("SVG Font {i}")).find(|n| !d.svg_fonts.iter().any(|f| f.family == *n)).unwrap_or_else(|| "SVG Font".into())
        }
    };
    let upm = f64_or(p, "unitsPerEm", 1000.0);
    let adv = f64_or(p, "advance", upm);
    if !(upm.is_finite() && upm > 0.0) || !(adv.is_finite() && adv >= 0.0) {
        return Err(bad(C, "`unitsPerEm` must be above 0 and `advance` at least 0"));
    }
    let id = font_id(&family, &s.doc()?.doc.svg_fonts);
    let out = json!({ "id": id, "family": family });
    s.edit("New SVG Font", |d, _| {
        let mut f = SvgFont::new(&id, &family);
        f.units_per_em = upm;
        f.advance = adv;
        d.svg_fonts.push(f);
        Ok(())
    })?;
    Ok(out)
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let fonts: Vec<Value> = d
        .svg_fonts
        .iter()
        .map(|f| {
            json!({
                "id": f.id,
                "family": f.family,
                "unitsPerEm": f.units_per_em,
                "advance": f.advance,
                "ascent": f.ascent,
                "descent": f.descent,
                "glyphs": f.glyphs.iter().map(|g| json!({ "unicode": g.unicode, "name": g.name, "advance": g.advance, "hasPath": !g.path.is_empty() })).collect::<Vec<_>>(),
                "kerning": f.kerning.iter().map(|k| json!({ "left": k.left, "right": k.right, "k": k.k })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({ "fonts": fonts }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let i = font_index(&s.doc()?.doc, p, "svgFont.delete")?;
    let id = s.edit("Delete SVG Font", |d, _| Ok(d.svg_fonts.remove(i).id))?;
    Ok(json!({ "removed": id }))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFont.set";
    let i = font_index(&s.doc()?.doc, p, C)?;
    let family = str_param(p, "family").map(str::trim).filter(|f| !f.is_empty()).map(str::to_string);
    let num = |k: &str| -> Result<Option<f64>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_f64().filter(|x| x.is_finite()).map(Some).ok_or_else(|| bad(C, format!("`{k}` must be a number"))),
        }
    };
    let (upm, adv, asc, desc) = (num("unitsPerEm")?, num("advance")?, num("ascent")?, num("descent")?);
    let id = s.edit("SVG Font Options", |d, _| {
        let f = &mut d.svg_fonts[i];
        if let Some(fam) = family {
            f.family = fam;
        }
        if let Some(u) = upm.filter(|u| *u > 0.0) {
            f.units_per_em = u;
        }
        if let Some(a) = adv {
            f.advance = a.max(0.0);
        }
        if asc.is_some() {
            f.ascent = asc;
        }
        if desc.is_some() {
            f.descent = desc;
        }
        Ok(f.id.clone())
    })?;
    Ok(json!({ "id": id }))
}

fn unicode_param(p: &Value, key: &str, cmd: &str) -> Result<String> {
    str_param(p, key).filter(|u| !u.is_empty()).map(str::to_string).ok_or_else(|| bad(cmd, format!("missing `{key}`: the glyph's character(s)")))
}

fn set_glyph(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFont.setGlyph";
    let i = font_index(&s.doc()?.doc, p, C)?;
    let unicode = unicode_param(p, "unicode", C)?;
    let name = str_param(p, "name").map(|n| n.trim().to_string());
    let advance = match p.get("advance") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => {
            Some(Some(v.as_f64().filter(|a| a.is_finite() && *a >= 0.0).ok_or_else(|| bad(C, "`advance` must be a number at least 0, or null"))?))
        }
    };
    let new_unicode = str_param(p, "newUnicode").filter(|u| !u.is_empty()).map(str::to_string);
    let out = new_unicode.clone().unwrap_or_else(|| unicode.clone());
    s.edit("Set Glyph", |d, _| {
        let f = &mut d.svg_fonts[i];
        if let Some(nu) = &new_unicode
            && *nu != unicode
            && f.glyph(nu).is_some()
        {
            return Err(EngineError::Other(format!("the font has a glyph for `{nu}` already")));
        }
        let at = match f.glyphs.iter().position(|g| g.unicode == unicode) {
            Some(at) => at,
            None => {
                f.glyphs.push(SvgGlyph { unicode: unicode.clone(), name: String::new(), advance: None, path: PathData::default() });
                f.glyphs.len() - 1
            }
        };
        let g = &mut f.glyphs[at];
        if let Some(n) = &name {
            g.name.clone_from(n);
        }
        if let Some(a) = advance {
            g.advance = a;
        }
        if let Some(nu) = &new_unicode {
            g.unicode.clone_from(nu);
        }
        Ok(())
    })?;
    Ok(json!({ "unicode": out }))
}

fn remove_glyph(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFont.removeGlyph";
    let i = font_index(&s.doc()?.doc, p, C)?;
    let unicode = unicode_param(p, "unicode", C)?;
    if s.doc()?.doc.svg_fonts[i].glyph(&unicode).is_none() {
        return Err(bad(C, format!("no glyph for `{unicode}`")));
    }
    s.edit("Remove Glyph", |d, _| {
        d.svg_fonts[i].glyphs.retain(|g| g.unicode != unicode);
        Ok(())
    })?;
    Ok(json!({ "removed": unicode }))
}

/// The outlines of `roots`' paths, joined, in document space.
fn outlines(d: &Document, roots: &[NodeId]) -> BezPath {
    let mut bp = BezPath::new();
    for id in paths_in(d, roots) {
        let Some(n) = d.node(id) else { continue };
        if matches!(n.kind, NodeKind::Path { guide: true, .. }) {
            continue;
        }
        if let Some(path) = n.path_data() {
            bp.extend(path.to_bezpath().elements().iter().copied());
        }
    }
    bp
}

/// Document space → font units: `em` points per em, the baseline at `baseline`, `left` at x 0,
/// y up.
fn to_font_units(upm: f64, em: f64, baseline: f64, left: f64) -> Affine {
    let k = upm / em.max(1e-9);
    Affine::new([k, 0.0, 0.0, -k, -left * k, baseline * k])
}

fn glyph_from_selection(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFont.glyphFromSelection";
    let i = font_index(&s.doc()?.doc, p, C)?;
    let unicode = unicode_param(p, "unicode", C)?;
    let roots = if p.get("ids").is_some() { targets(s, p)? } else { selected_roots(s)? };
    let (bp, bounds, upm) = {
        let d = &s.doc()?.doc;
        let bp = outlines(d, &roots);
        if bp.elements().is_empty() {
            return Err(bad(C, "the selection has no paths (expand text and shapes first)"));
        }
        (bp.clone(), bp.bounding_box(), d.svg_fonts[i].units_per_em)
    };
    let em = p.get("emSize").and_then(Value::as_f64).filter(|e| e.is_finite() && *e > 0.0).unwrap_or(bounds.height().max(1e-6));
    let baseline = f64_or(p, "baseline", bounds.y1);
    let left = f64_or(p, "left", bounds.x0);
    let local = to_font_units(upm, em, baseline, left) * bp;
    let name = str_param(p, "name").map(|n| n.trim().to_string());
    let path = PathData::from_bezpath(&local);
    let anchors = path.anchor_count();
    s.edit("Get Curves from Selection", |d, _| {
        let f = &mut d.svg_fonts[i];
        match f.glyphs.iter_mut().find(|g| g.unicode == unicode) {
            Some(g) => {
                g.path = path;
                if let Some(n) = &name {
                    g.name.clone_from(n);
                }
            }
            None => f.glyphs.push(SvgGlyph { unicode: unicode.clone(), name: name.clone().unwrap_or_default(), advance: None, path }),
        }
        Ok(())
    })?;
    Ok(json!({ "unicode": unicode, "anchors": anchors }))
}

/// The pen origin `at` (default: the first artboard's centre).
fn origin(s: &Session, p: &Value) -> Point {
    if let Some(a) = p.get("at").and_then(Value::as_array)
        && let (Some(x), Some(y)) = (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64))
    {
        return Point::new(x, y);
    }
    s.active().and_then(|st| st.doc.artboards.first()).map(|a| a.rect.center()).unwrap_or(Point::new(100.0, 100.0))
}

/// A glyph's path placed at pen position `pen` (document) at `em` points per em.
fn glyph_path(f: &SvgFont, g: &SvgGlyph, pen: Point, em: f64) -> BezPath {
    let k = em / f.units_per_em.max(1e-9);
    Affine::new([k, 0.0, 0.0, -k, pen.x, pen.y]) * g.path.to_bezpath()
}

fn glyph_to_objects(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFont.glyphToSelection";
    let i = font_index(&s.doc()?.doc, p, C)?;
    let unicode = unicode_param(p, "unicode", C)?;
    let at = origin(s, p);
    let em = f64_or(p, "emSize", 100.0).max(0.1);
    let bp = {
        let f = &s.doc()?.doc.svg_fonts[i];
        let g = f.glyph(&unicode).ok_or_else(|| bad(C, format!("no glyph for `{unicode}`")))?;
        if g.path.is_empty() {
            return Err(bad(C, format!("the glyph for `{unicode}` has no curves yet")));
        }
        glyph_path(f, g, at, em)
    };
    let layer = s.doc()?.current_layer();
    let id = s.edit("Glyph to Objects", |d, sel| {
        let mut n = Node::path(d.alloc_id(), PathData::from_bezpath(&bp), Appearance::basic(Paint::solid(Color::rgb8(0, 0, 0)), Paint::None, 0.0));
        n.name = Some(format!("Glyph {unicode}"));
        let id = n.id;
        let layer = layer.or_else(|| d.layers.first().map(|l| l.id));
        d.insert(layer, usize::MAX, n)?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn set_kerning(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFont.setKerning";
    let i = font_index(&s.doc()?.doc, p, C)?;
    let left = unicode_param(p, "left", C)?;
    let right = unicode_param(p, "right", C)?;
    let k = f64_req(p, "k", C)?;
    if !k.is_finite() {
        return Err(bad(C, "`k` must be a number"));
    }
    let pairs = s.edit("Set Kerning Pair", |d, _| {
        let f = &mut d.svg_fonts[i];
        f.kerning.retain(|x| !(x.left == left && x.right == right));
        if k != 0.0 {
            f.kerning.push(SvgKern { left: left.clone(), right: right.clone(), k });
        }
        Ok(f.kerning.len())
    })?;
    Ok(json!({ "pairs": pairs }))
}

fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFont.preview";
    let i = font_index(&s.doc()?.doc, p, C)?;
    let text = str_param(p, "text").filter(|t| !t.is_empty()).ok_or_else(|| bad(C, "missing `text`"))?.to_string();
    let at = origin(s, p);
    let em = f64_or(p, "emSize", 72.0).max(0.1);
    let (paths, missing) = {
        let f = &s.doc()?.doc.svg_fonts[i];
        let k = em / f.units_per_em.max(1e-9);
        let mut paths = vec![];
        let mut missing: Vec<String> = vec![];
        let chars: Vec<char> = text.chars().collect();
        let mut ci = 0;
        for (g, x) in f.layout(&text) {
            match g {
                Some(g) if !g.path.is_empty() => {
                    paths.push((g.unicode.clone(), glyph_path(f, g, Point::new(at.x + x * k, at.y), em)));
                    ci += g.unicode.chars().count();
                }
                Some(g) => ci += g.unicode.chars().count(),
                None => {
                    if let Some(c) = chars.get(ci)
                        && !c.is_whitespace()
                        && !missing.iter().any(|m| m == &c.to_string())
                    {
                        missing.push(c.to_string());
                    }
                    ci += 1;
                }
            }
        }
        (paths, missing)
    };
    if paths.is_empty() {
        return Err(bad(C, "no glyph of the text has curves yet"));
    }
    let count = paths.len();
    let layer = s.doc()?.current_layer();
    let id = s.edit("Preview SVG Font", |d, sel| {
        let children: Vec<Arc<Node>> = paths
            .into_iter()
            .map(|(u, bp)| {
                let mut n =
                    Node::path(d.alloc_id(), PathData::from_bezpath(&bp), Appearance::basic(Paint::solid(Color::rgb8(0, 0, 0)), Paint::None, 0.0));
                n.name = Some(u);
                Arc::new(n)
            })
            .collect();
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.name = Some(format!("Preview: {text}"));
        let layer = layer.or_else(|| d.layers.first().map(|l| l.id));
        d.insert(layer, usize::MAX, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0, "glyphs": count, "missing": missing }))
}

/// The character a glyph layer's name stands for: one character, or a glyph name.
fn layer_char(name: &str) -> Option<String> {
    let n = name.trim();
    let mut chars = n.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(c.to_string());
    }
    let named = match n.to_ascii_lowercase().as_str() {
        "space" => " ",
        "period" => ".",
        "comma" => ",",
        "colon" => ":",
        "semicolon" => ";",
        "hyphen" | "minus" => "-",
        "underscore" => "_",
        "exclam" => "!",
        "question" => "?",
        "quotesingle" => "'",
        "quotedbl" => "\"",
        "slash" => "/",
        "backslash" => "\\",
        "ampersand" => "&",
        "at" => "@",
        "numbersign" => "#",
        "dollar" => "$",
        "percent" => "%",
        "asterisk" => "*",
        "plus" => "+",
        "equal" => "=",
        "parenleft" => "(",
        "parenright" => ")",
        "bracketleft" => "[",
        "bracketright" => "]",
        "braceleft" => "{",
        "braceright" => "}",
        "less" => "<",
        "greater" => ">",
        "bar" => "|",
        "asciitilde" => "~",
        "asciicircum" => "^",
        "grave" => "`",
        _ => return None,
    };
    Some(named.to_string())
}

fn from_layers(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFont.fromLayers";
    // Glyph layers: top-level layers named by a character, with their art's outlines.
    let (glyphs, em, baseline, upm_existing) = {
        let d = &s.doc()?.doc;
        let mut glyphs: Vec<(String, String, BezPath)> = vec![];
        let mut tallest: Option<redrob_vector_geom::Rect> = None;
        for l in &d.layers {
            let Some(name) = l.name.as_deref() else { continue };
            let Some(ch) = layer_char(name) else { continue };
            let bp = outlines(d, &[l.id]);
            if bp.elements().is_empty() {
                continue;
            }
            let b = bp.bounding_box();
            if tallest.is_none_or(|t| b.height() > t.height()) {
                tallest = Some(b);
            }
            glyphs.push((ch, name.trim().to_string(), bp));
        }
        if glyphs.is_empty() {
            return Err(bad(C, "no glyph layers: name a layer by the character it draws (A, b, 7, space, period…)"));
        }
        let t = tallest.unwrap_or_default();
        let em = p.get("emSize").and_then(Value::as_f64).filter(|e| e.is_finite() && *e > 0.0).unwrap_or(t.height().max(1e-6));
        let baseline = f64_or(p, "baseline", t.y1);
        let existing =
            str_param(p, "font").and_then(|k| d.svg_fonts.iter().position(|f| f.id == k || f.family == k)).map(|i| d.svg_fonts[i].units_per_em);
        (glyphs, em, baseline, existing)
    };
    let upm = upm_existing.unwrap_or(1000.0);
    let count = glyphs.len();
    let (id, family) = {
        let d = &s.doc()?.doc;
        match str_param(p, "font").and_then(|k| d.svg_fonts.iter().find(|f| f.id == k || f.family == k)) {
            Some(f) => (f.id.clone(), f.family.clone()),
            None => {
                let family = str_param(p, "font").map(str::to_string).unwrap_or_else(|| {
                    std::path::Path::new(&d.title)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "SVG Font".into())
                });
                (font_id(&family, &d.svg_fonts), family)
            }
        }
    };
    s.edit("Glyph Layers to SVG Font", |d, _| {
        let at = match d.svg_fonts.iter().position(|f| f.id == id) {
            Some(at) => at,
            None => {
                let mut f = SvgFont::new(&id, &family);
                f.units_per_em = upm;
                d.svg_fonts.push(f);
                d.svg_fonts.len() - 1
            }
        };
        let f = &mut d.svg_fonts[at];
        for (ch, name, bp) in &glyphs {
            // Each glyph starts at its own left edge; its advance is its width plus a tenth of an em.
            let b = bp.bounding_box();
            let local = to_font_units(upm, em, baseline, b.x0) * bp.clone();
            let advance = Some(((b.width() / em) * upm + upm * 0.1).round());
            let path = PathData::from_bezpath(&local);
            let glyph_name = if name.chars().count() > 1 { name.clone() } else { String::new() };
            match f.glyphs.iter_mut().find(|g| g.unicode == *ch) {
                Some(g) => {
                    g.path = path;
                    g.advance = advance;
                }
                None => f.glyphs.push(SvgGlyph { unicode: ch.clone(), name: glyph_name, advance, path }),
            }
        }
        Ok(())
    })?;
    Ok(json!({ "id": id, "glyphs": count }))
}

fn to_layers(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFont.toLayers";
    let i = font_index(&s.doc()?.doc, p, C)?;
    let em = f64_or(p, "emSize", 100.0).max(0.1);
    let at = match p.get("at").and_then(Value::as_array) {
        Some(a) if a.len() >= 2 => Point::new(a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)),
        _ => s.doc()?.doc.artboards.first().map(|a| Point::new(a.rect.x0 + em, a.rect.y0 + em)).unwrap_or(Point::new(em, em)),
    };
    let made = s.edit("SVG Font to Glyph Layers", |d, _| {
        let f = d.svg_fonts[i].clone();
        let mut made = 0;
        for g in &f.glyphs {
            if g.path.is_empty() {
                continue;
            }
            let bp = glyph_path(&f, g, at, em);
            let name = if g.unicode == " " { "space".to_string() } else { g.unicode.clone() };
            let mut layer = Node::layer(d.alloc_id(), &name, LayerColor::Preset((made % 7) as u8));
            let mut path =
                Node::path(d.alloc_id(), PathData::from_bezpath(&bp), Appearance::basic(Paint::solid(Color::rgb8(0, 0, 0)), Paint::None, 0.0));
            path.name = Some(name.clone());
            if let Some(c) = layer.children_mut() {
                c.push(Arc::new(path));
            }
            d.layers.push(Arc::new(layer));
            made += 1;
        }
        Ok(made)
    })?;
    Ok(json!({ "layers": made }))
}
