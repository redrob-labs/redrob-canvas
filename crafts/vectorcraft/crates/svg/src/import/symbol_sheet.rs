//! A symbol sheet: an SVG whose art is only `<symbol>` definitions that nothing shows (an icon
//! sprite, where a page picks icons with `<use href="#id">` of its own).
//!
//! usvg drops an unreferenced `<symbol>`, so such a file opened as an empty artboard with no
//! objects and none of its paths could be edited. Before usvg runs, the sheet gets one `<use>` of
//! every symbol, laid out in a grid, and the root `<svg>` is sized to fit the grid: each `<use>`
//! then becomes a symbol instance the way a `<use>` written in the file does, and the Symbols panel
//! lists every symbol of the sprite.

use std::collections::HashSet;
use std::str::FromStr;

use usvg::roxmltree;

use super::{Edits, MADE_UP_ID, XNode, href};

/// The elements that paint: a sheet has none outside its definitions.
const DRAWN: &[&str] = &["path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "image", "text", "use", "foreignObject"];
/// The elements whose content is a definition, not art of the page.
const DEFINITIONS: &[&str] = &["defs", "symbol", "clipPath", "mask", "pattern", "marker", "linearGradient", "radialGradient", "filter"];
/// The size of a symbol with neither a `viewBox` nor a width and height, in user units.
const DEFAULT_SIZE: f64 = 100.0;

/// The id prefix of the `<use>` elements written in, after [`MADE_UP_ID`] so they count as made up.
const SHEET_ID: &str = "sheet-";

/// When `xml` is a symbol sheet: writes the `<use>` grid and the root's size into `edits` and gives
/// each `<use>`'s id with the id of the symbol it shows, else nothing.
pub(super) fn lay_out(svg: &str, xml: &roxmltree::Document, edits: &mut Edits) -> Vec<(String, String)> {
    let root = xml.root_element();
    let symbols: Vec<XNode> =
        xml.descendants().filter(|n| n.is_element() && n.tag_name().name() == "symbol" && n.attribute("id").is_some_and(|id| !id.is_empty())).collect();
    if symbols.is_empty() || !is_sheet(root, &symbols) {
        return Vec::new();
    }
    let sizes: Vec<(f64, f64)> = symbols.iter().map(|s| symbol_size(*s)).collect();
    let (cw, ch) = sizes.iter().fold((0.0f64, 0.0f64), |(w, h), &(sw, sh)| (w.max(sw), h.max(sh)));
    let gap = (cw.max(ch) / 4.0).max(1.0);
    let n = symbols.len();
    let cols = (n as f64).sqrt().ceil().max(1.0) as usize;
    let rows = n.div_ceil(cols);
    let (width, height) = (gap + cols as f64 * (cw + gap), gap + rows as f64 * (ch + gap));
    edits.set(svg, root, "width", &fmt(width));
    edits.set(svg, root, "height", &fmt(height));
    edits.set(svg, root, "viewBox", &format!("0 0 {} {}", fmt(width), fmt(height)));

    // The `<use>` elements go at the end of the root, after every definition they refer to.
    let end = root.range().end;
    let at = svg.get(..end).and_then(|s| s.rfind("</")).unwrap_or(end);
    let mut text = String::new();
    let mut uses = Vec::with_capacity(n);
    for (i, (s, &(w, h))) in symbols.iter().zip(&sizes).enumerate() {
        let sym = s.attribute("id").unwrap_or_default();
        let id = format!("{MADE_UP_ID}{SHEET_ID}{i}");
        let (col, row) = (i % cols, i / cols);
        // Each symbol is centred in its cell.
        let x = gap + col as f64 * (cw + gap) + (cw - w) / 2.0;
        let y = gap + row as f64 * (ch + gap) + (ch - h) / 2.0;
        text.push_str(&format!(
            "<use id=\"{id}\" href=\"#{}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
            crate::xml_escape(sym),
            fmt(x),
            fmt(y),
            fmt(w),
            fmt(h)
        ));
        uses.push((id, sym.to_string()));
    }
    edits.insert(at, text);
    uses
}

/// Is `root` a sheet: no `<use>` of any of `symbols`, and no art outside definitions?
fn is_sheet(root: XNode, symbols: &[XNode]) -> bool {
    let ids: HashSet<&str> = symbols.iter().filter_map(|s| s.attribute("id")).collect();
    let used = root.descendants().filter(|n| n.is_element() && n.tag_name().name() == "use").any(|u| href(u).and_then(|h| h.strip_prefix('#')).is_some_and(|s| ids.contains(s)));
    if used {
        return false;
    }
    let mut todo: Vec<XNode> = root.children().filter(XNode::is_element).collect();
    while let Some(n) = todo.pop() {
        let tag = n.tag_name().name();
        if DEFINITIONS.contains(&tag) {
            continue;
        }
        if DRAWN.contains(&tag) {
            return false;
        }
        todo.extend(n.children().filter(XNode::is_element));
    }
    true
}

/// The size of symbol `s` in user units: its `viewBox`'s, else its `width` × `height`, else
/// [`DEFAULT_SIZE`] square.
fn symbol_size(s: XNode) -> (f64, f64) {
    if let Some(vb) = s.attribute("viewBox") {
        let v: Vec<f64> = vb.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).filter_map(|t| f64::from_str(t).ok()).collect();
        if let [_, _, w, h] = v[..]
            && w > 0.0
            && h > 0.0
        {
            return (w, h);
        }
    }
    let len = |name: &str| s.attribute(name).and_then(|v| f64::from_str(v.trim().trim_end_matches("px")).ok()).filter(|v| *v > 0.0);
    match (len("width"), len("height")) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w),
        (None, Some(h)) => (h, h),
        (None, None) => (DEFAULT_SIZE, DEFAULT_SIZE),
    }
}

/// A number as SVG writes it: no trailing zeros.
fn fmt(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".to_string() } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPRITE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><symbol viewBox="0 0 16 16" id="a"><path d="M0 0h16v16H0z"/></symbol><symbol viewBox="0 0 16 16" id="b"><circle cx="8" cy="8" r="8"/><path d="M1 1h2v2H1z"/></symbol><symbol viewBox="0 0 16 16" id="c"><path d="M0 0h8v8H0z"/></symbol></svg>"##;

    #[test]
    fn a_sprite_gets_a_use_of_every_symbol_in_a_grid() {
        let xml = roxmltree::Document::parse(SPRITE).unwrap();
        let mut edits = Edits::default();
        let uses = lay_out(SPRITE, &xml, &mut edits);
        assert_eq!(uses.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert!(uses.iter().all(|(id, _)| super::super::made_up(id)));
        let out = edits.apply(SPRITE);
        // 3 symbols: 2 columns, 2 rows, 16 pt cells, 4 pt gaps.
        assert!(out.contains(r#"width="44" height="44" viewBox="0 0 44 44""#), "{out}");
        assert!(out.contains(r##"<use id="vectorcraft-id-sheet-0" href="#a" x="4" y="4" width="16" height="16"/>"##), "{out}");
        assert!(out.contains(r##"<use id="vectorcraft-id-sheet-2" href="#c" x="4" y="24" width="16" height="16"/>"##), "{out}");
        assert!(out.ends_with("/></svg>"), "{out}");
        // What usvg then reads is well formed.
        roxmltree::Document::parse(&out).unwrap();
    }

    #[test]
    fn a_file_that_shows_its_symbols_or_has_art_is_left_alone() {
        for svg in [
            r##"<svg xmlns="http://www.w3.org/2000/svg"><symbol id="a" viewBox="0 0 16 16"><path d="M0 0h16v16H0z"/></symbol><use href="#a" x="1" y="1" width="16" height="16"/></svg>"##,
            r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><symbol id="a" viewBox="0 0 16 16"><path d="M0 0h16v16H0z"/></symbol></defs><g><rect width="5" height="5"/></g></svg>"##,
            r##"<svg xmlns="http://www.w3.org/2000/svg"><rect width="5" height="5"/></svg>"##,
        ] {
            let xml = roxmltree::Document::parse(svg).unwrap();
            let mut edits = Edits::default();
            assert!(lay_out(svg, &xml, &mut edits).is_empty(), "{svg}");
            assert_eq!(edits.apply(svg), svg);
        }
    }
}
