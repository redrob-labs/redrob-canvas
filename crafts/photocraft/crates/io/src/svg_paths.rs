//! Paths as SVG, both ways: every `<path>` (and shape) of an SVG file as a document path for the
//! Paths panel ([`import_paths`]), and saved paths written as a plain SVG of outlines
//! ([`export_paths`]), the form the Paths panel's Import Path / Export Path and its copy and
//! paste as SVG text use. Coordinates are document pixels (the SVG's user units, through its
//! viewBox); fills and strokes are not carried, only the geometry.

use redrob_photo_doc::vector::{FillRule, Path, Subpath};
use resvg::usvg;

use crate::IoError;

/// Every path in `bytes` (an SVG or `.svgz`), with its `id` (or a numbered name) and its
/// geometry flattened through the element transforms. Empty when the drawing has no paths.
pub fn import_paths(bytes: &[u8]) -> Result<Vec<(String, Path)>, IoError> {
    let parsed = crate::svg::parse(bytes)?;
    let mut out = Vec::new();
    collect(parsed.tree.root(), &mut out, 0)?;
    Ok(out)
}

fn collect(group: &usvg::Group, out: &mut Vec<(String, Path)>, depth: usize) -> Result<(), IoError> {
    if depth > 64 {
        return Err(IoError::Svg("groups are nested too deeply".into()));
    }
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => collect(g, out, depth + 1)?,
            usvg::Node::Path(p) => {
                let Some(data) = (*p.data()).clone().transform(p.abs_transform()) else { continue };
                let subpaths = crate::svg::knots(&data);
                if subpaths.is_empty() {
                    continue;
                }
                let fill_rule = match p.fill().map(|f| f.rule()) {
                    Some(usvg::FillRule::EvenOdd) => FillRule::EvenOdd,
                    _ => FillRule::NonZero,
                };
                let name = if p.id().is_empty() { format!("Path {}", out.len() + 1) } else { p.id().to_string() };
                out.push((name, Path { subpaths, fill_rule, inverted: false }));
            }
            // Text and images carry no path geometry.
            _ => {}
        }
        if out.len() > 10_000 {
            return Err(IoError::Svg("more than 10000 paths".into()));
        }
    }
    Ok(())
}

fn num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-6 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// The `d` attribute of one path: `M`/`C`/`L` per knot, `Z` for closed subpaths.
pub fn path_d(path: &Path) -> String {
    let mut d = String::new();
    for sp in &path.subpaths {
        d.push_str(&subpath_d(sp));
    }
    d.trim().to_string()
}

fn subpath_d(sp: &Subpath) -> String {
    let Some(first) = sp.knots.first() else { return String::new() };
    let mut d = format!("M{} {} ", num(first.anchor.x), num(first.anchor.y));
    let n = sp.knots.len();
    let segs = if sp.closed { n } else { n.saturating_sub(1) };
    for i in 0..segs {
        let a = &sp.knots[i];
        let b = &sp.knots[(i + 1) % n];
        let straight = (a.out_ctrl.x - a.anchor.x).abs() < 1e-9
            && (a.out_ctrl.y - a.anchor.y).abs() < 1e-9
            && (b.in_ctrl.x - b.anchor.x).abs() < 1e-9
            && (b.in_ctrl.y - b.anchor.y).abs() < 1e-9;
        if straight {
            d.push_str(&format!("L{} {} ", num(b.anchor.x), num(b.anchor.y)));
        } else {
            d.push_str(&format!(
                "C{} {} {} {} {} {} ",
                num(a.out_ctrl.x),
                num(a.out_ctrl.y),
                num(b.in_ctrl.x),
                num(b.in_ctrl.y),
                num(b.anchor.x),
                num(b.anchor.y)
            ));
        }
    }
    if sp.closed {
        d.push_str("Z ");
    }
    d
}

fn escape(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// An SVG document of `width` × `height` user units holding `paths` as unfilled black outlines,
/// each with its name as `id`.
pub fn export_paths(paths: &[(&str, &Path)], width: u32, height: u32) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">\n"
    );
    for (name, path) in paths {
        let rule = match path.fill_rule {
            FillRule::EvenOdd => "evenodd",
            FillRule::NonZero => "nonzero",
        };
        s.push_str(&format!("  <path id=\"{}\" fill=\"none\" fill-rule=\"{rule}\" stroke=\"#000000\" stroke-width=\"1\" d=\"{}\"/>\n", escape(name), path_d(path)));
    }
    s.push_str("</svg>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_round_trip_through_svg() {
        let p = Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)])]);
        let svg = export_paths(&[("Tri", &p)], 20, 20);
        assert!(svg.contains("id=\"Tri\""));
        assert!(svg.contains("M0 0 L10 0 L10 10 Z"));
        let back = import_paths(svg.as_bytes()).unwrap_or_default();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].0, "Tri");
        assert_eq!(back[0].1.subpaths.len(), 1);
        assert!(back[0].1.subpaths[0].closed);
        assert_eq!(back[0].1.subpaths[0].knots.len(), 3);
    }
}
