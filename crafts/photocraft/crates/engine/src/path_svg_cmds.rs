//! Paths panel › Import Path… / Export Path… and copy/paste as SVG text (`path.importSvg`,
//! `path.exportSvg`, `path.toSvg`): a path-only exchange with SVG, apart from opening an SVG as a
//! document. Imported paths join the Paths panel as saved paths; export writes the chosen paths
//! as unfilled outlines ([`redrob_photo_io::svg_paths`]).

use redrob_photo_doc::NamedPath;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_paths(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.paths.is_empty() && d.doc.work_path.is_none() { Err("the document has no paths".into()) } else { Ok(()) }
}

/// The SVG bytes of `svg` (text), `data` (base64) or `path` (a file).
fn svg_bytes(p: &Value, cmd: &str) -> Result<Vec<u8>> {
    if let Some(t) = p.get("svg").and_then(Value::as_str) {
        if t.trim().is_empty() {
            return Err(bad(cmd, "`svg` is empty"));
        }
        return Ok(t.as_bytes().to_vec());
    }
    crate::preset_import_cmds::file_bytes(p, cmd, "an .svg or .svgz file").map(|(b, _)| b)
}

/// `path.importSvg`: every path of an SVG as saved paths of the document.
fn import_svg(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.importSvg";
    let bytes = svg_bytes(p, CMD)?;
    let paths = redrob_photo_io::svg_paths::import_paths(&bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    if paths.is_empty() {
        return Err(bad(CMD, "the SVG holds no paths"));
    }
    // One saved path per SVG path by default; `merge` folds them into one named path.
    let merge = p.get("merge").and_then(Value::as_bool).unwrap_or(false);
    let base = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    let mut names = Vec::new();
    s.edit("Import Path", |doc, _| {
        let taken = |doc: &redrob_photo_doc::Document, n: &str| doc.paths.iter().any(|x| x.name.eq_ignore_ascii_case(n));
        let unique = |doc: &redrob_photo_doc::Document, want: &str| -> String {
            if !taken(doc, want) {
                return want.to_string();
            }
            (2..).map(|i| format!("{want} {i}")).find(|n| !taken(doc, n)).unwrap_or_else(|| want.to_string())
        };
        if merge {
            let mut all = redrob_photo_doc::vector::Path::new(Vec::new());
            for (_, path) in &paths {
                all.subpaths.extend(path.subpaths.iter().cloned());
            }
            let name = unique(doc, base.as_deref().unwrap_or("Imported Path"));
            doc.paths.push(NamedPath { name: name.clone(), path: all, psd_raw: None });
            names.push(name);
        } else {
            for (i, (svg_name, path)) in paths.iter().enumerate() {
                let want = match &base {
                    Some(b) if paths.len() == 1 => b.clone(),
                    Some(b) => format!("{b} {}", i + 1),
                    None => svg_name.clone(),
                };
                let name = unique(doc, &want);
                doc.paths.push(NamedPath { name: name.clone(), path: path.clone(), psd_raw: None });
                names.push(name);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "imported": names, "count": names.len() }))
}

/// The paths `which` selects: `all`, `work`, or one saved path's name.
fn select<'a>(doc: &'a redrob_photo_doc::Document, which: &str) -> Vec<(&'a str, &'a redrob_photo_doc::vector::Path)> {
    match which {
        "all" => {
            let mut v: Vec<(&str, &redrob_photo_doc::vector::Path)> = doc.paths.iter().map(|n| (n.name.as_str(), &n.path)).collect();
            if let Some(w) = doc.work_path.as_ref() {
                v.push(("Work Path", w));
            }
            v
        }
        "work" => doc.work_path.as_ref().map(|w| vec![("Work Path", w)]).unwrap_or_default(),
        name => doc.paths.iter().filter(|n| n.name == name).map(|n| (n.name.as_str(), &n.path)).collect(),
    }
}

/// `path.toSvg`: the SVG text of the chosen paths (for the clipboard).
fn to_svg(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.toSvg";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let which = p.get("paths").and_then(Value::as_str).unwrap_or("all");
    let sel = select(&d.doc, which);
    if sel.is_empty() {
        return Err(bad(CMD, format!("no paths match `{which}`")));
    }
    let n = sel.len();
    let svg = redrob_photo_io::svg_paths::export_paths(&sel, d.doc.size.width, d.doc.size.height);
    Ok(json!({ "svg": svg, "paths": n }))
}

/// `path.exportSvg`: write the chosen paths to an `.svg` file.
fn export_svg(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.exportSvg";
    let path = p.get("path").and_then(Value::as_str).filter(|v| !v.trim().is_empty()).ok_or_else(|| bad(CMD, "pass `path` (the .svg file to write)"))?.to_string();
    let r = to_svg(s, p)?;
    let svg = r.get("svg").and_then(Value::as_str).unwrap_or_default();
    crate::file_cmds::write_file(&path, svg.as_bytes())?;
    Ok(json!({ "path": path, "paths": r.get("paths").cloned().unwrap_or(Value::Null) }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "path.importSvg",
            label: "Import Path from SVG…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".svg or .svgz file"?,"data":base64?,"svg":text? (SVG source, e.g. pasted),"name":string? (name for the imported path(s)),"merge":bool=false (one path from all of them)} → {imported, count}"##,
            enabled: has_doc,
            run: import_svg,
            journal: true,
        },
        CommandSpec {
            id: "path.exportSvg",
            label: "Export Path to SVG…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".svg file to write","paths":"all|work|<path name>"="all"} → {path, paths}"##,
            enabled: has_paths,
            run: export_svg,
            journal: true,
        },
        CommandSpec {
            id: "path.toSvg",
            label: "Copy Path as SVG",
            menu: &[],
            shortcut: None,
            params: r##"{"paths":"all|work|<path name>"="all"} → {svg, paths}"##,
            enabled: has_paths,
            run: to_svg,
            journal: false,
        },
    ]
}
