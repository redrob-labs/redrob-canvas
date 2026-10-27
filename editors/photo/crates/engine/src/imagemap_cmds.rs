//! Image Map (Filter › Web › Image Map): clickable rectangle, circle and polygon areas over the
//! image, exported as an HTML client-side image map (`<img usemap>` + `<map><area …>`).
//!
//! The areas live in the document (`Document::metadata.text` under [`KEY`], as JSON), so they
//! travel with it, undo with it, and survive a save to a format that keeps text metadata. Slices
//! can be imported as rectangle areas, so a layout sliced for a table export also works as a map.

use redrob_photo_doc::Document;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The metadata text key that holds the map (JSON array of [`Area`]).
pub const KEY: &str = "redrob:imageMap";

/// One clickable area.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Area {
    pub id: u32,
    /// `rect`, `circle` or `poly`.
    pub shape: String,
    /// `rect`: `[x0, y0, x1, y1]`; `circle`: `[cx, cy, r]`; `poly`: `[x0, y0, x1, y1, …]`
    /// (document pixels).
    pub coords: Vec<f64>,
    #[serde(default)]
    pub href: String,
    #[serde(default)]
    pub alt: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub title: String,
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_areas(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| !areas_of(&d.doc).is_empty()) { Ok(()) } else { Err("the document has no image map areas".into()) }
}

/// The document's areas (empty without a map).
pub fn areas_of(doc: &Document) -> Vec<Area> {
    doc.metadata.text.iter().find(|(k, _)| k == KEY).and_then(|(_, v)| serde_json::from_str(v).ok()).unwrap_or_default()
}

fn store(doc: &mut Document, areas: &[Area]) {
    doc.metadata.text.retain(|(k, _)| k != KEY);
    if !areas.is_empty() {
        doc.metadata.text.push((KEY.to_string(), serde_json::to_string(areas).unwrap_or_else(|_| "[]".into())));
    }
}

fn str_of(p: &Value, k: &str) -> String {
    p.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Numbers from a JSON array, or a comma / space separated string (`"10,20,30,40"`).
fn numbers(v: &Value) -> Option<Vec<f64>> {
    match v {
        Value::Array(a) => a.iter().map(|x| x.as_f64().filter(|n| n.is_finite())).collect(),
        Value::String(s) => s.split([',', ' ', ';']).filter(|t| !t.trim().is_empty()).map(|t| t.trim().parse::<f64>().ok().filter(|n| n.is_finite())).collect(),
        _ => None,
    }
}

/// Validates and normalises the coordinates of `shape`.
fn coords_for(cmd: &str, shape: &str, p: &Value) -> Result<Vec<f64>> {
    let c = p.get("coords").and_then(numbers);
    match shape {
        "rect" => {
            let v = match c {
                Some(v) if v.len() == 4 => v,
                _ => {
                    let g = |k: &str| p.get(k).and_then(Value::as_f64).filter(|n| n.is_finite());
                    match (g("x"), g("y"), g("width"), g("height")) {
                        (Some(x), Some(y), Some(w), Some(h)) => vec![x, y, x + w, y + h],
                        _ => return Err(bad(cmd, "a rect needs `coords`: [x0, y0, x1, y1] or x, y, width, height")),
                    }
                }
            };
            Ok(vec![v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
        }
        "circle" => match c {
            Some(v) if v.len() == 3 && v[2] > 0.0 => Ok(v),
            _ => {
                let g = |k: &str| p.get(k).and_then(Value::as_f64).filter(|n| n.is_finite());
                match (g("x"), g("y"), g("radius")) {
                    (Some(x), Some(y), Some(r)) if r > 0.0 => Ok(vec![x, y, r]),
                    _ => Err(bad(cmd, "a circle needs `coords`: [cx, cy, r] or x, y, radius")),
                }
            }
        },
        "poly" | "polygon" => match c {
            Some(v) if v.len() >= 6 && v.len() % 2 == 0 => Ok(v),
            _ => Err(bad(cmd, "a polygon needs `coords`: [x0, y0, x1, y1, x2, y2, …] (at least three points)")),
        },
        other => Err(bad(cmd, format!("unknown shape `{other}` (rect, circle or poly)"))),
    }
}

fn area_json(a: &Area) -> Value {
    serde_json::to_value(a).unwrap_or(Value::Null)
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "web.imageMap.add";
    let shape = match str_of(p, "shape").as_str() {
        "" | "rect" | "rectangle" => "rect",
        "circle" => "circle",
        "poly" | "polygon" => "poly",
        other => return Err(bad(CMD, format!("unknown shape `{other}` (rect, circle or poly)"))),
    };
    let coords = coords_for(CMD, shape, p)?;
    let href = str_of(p, "href");
    let area = Area { id: 0, shape: shape.into(), coords, href, alt: str_of(p, "alt"), target: str_of(p, "target"), title: str_of(p, "title") };
    let id = s.edit("Add Image Map Area", |doc, _| {
        let mut areas = areas_of(doc);
        let id = areas.iter().map(|a| a.id).max().map_or(1, |m| m + 1);
        areas.push(Area { id, ..area });
        store(doc, &areas);
        Ok(id)
    })?;
    Ok(json!({ "area": id }))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "web.imageMap.set";
    let id = p.get("area").and_then(Value::as_u64).ok_or_else(|| bad(CMD, "`area` (id) is required"))? as u32;
    let out = s.edit("Image Map Area Options", |doc, _| {
        let mut areas = areas_of(doc);
        let a = areas.iter_mut().find(|a| a.id == id).ok_or_else(|| EngineError::Other(format!("no image map area {id}")))?;
        if let Some(shape) = p.get("shape").and_then(Value::as_str) {
            let shape = match shape {
                "rect" | "rectangle" => "rect",
                "circle" => "circle",
                "poly" | "polygon" => "poly",
                other => return Err(bad(CMD, format!("unknown shape `{other}`"))),
            };
            a.coords = coords_for(CMD, shape, p)?;
            a.shape = shape.into();
        } else if p.get("coords").is_some() {
            a.coords = coords_for(CMD, &a.shape.clone(), p)?;
        }
        for (key, field) in [("href", &mut a.href), ("alt", &mut a.alt), ("target", &mut a.target), ("title", &mut a.title)] {
            if let Some(v) = p.get(key).and_then(Value::as_str) {
                *field = v.to_string();
            }
        }
        let out = area_json(a);
        store(doc, &areas);
        Ok(out)
    })?;
    Ok(out)
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "web.imageMap.delete";
    let ids: Vec<u32> = match (p.get("area").and_then(Value::as_u64), p.get("areas").and_then(Value::as_array)) {
        (Some(id), _) => vec![id as u32],
        (None, Some(a)) => a.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect(),
        _ => return Err(bad(CMD, "`area` (id) or `areas` ([ids]) is required")),
    };
    let n = s.edit("Delete Image Map Area", |doc, _| {
        let mut areas = areas_of(doc);
        let before = areas.len();
        areas.retain(|a| !ids.contains(&a.id));
        store(doc, &areas);
        Ok(before - areas.len())
    })?;
    Ok(json!({ "deleted": n }))
}

fn clear(s: &mut Session) -> Result<Value> {
    let n = s.edit("Clear Image Map", |doc, _| {
        let n = areas_of(doc).len();
        store(doc, &[]);
        Ok(n)
    })?;
    Ok(json!({ "cleared": n }))
}

fn list(s: &Session) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    Ok(json!({ "areas": areas_of(&d.doc).iter().map(area_json).collect::<Vec<_>>() }))
}

/// Imports the document's user and layer-based slices as rectangle areas (with their URL, alt
/// and target), skipping rectangles already in the map.
fn from_slices(s: &mut Session) -> Result<Value> {
    let n = s.edit("Image Map From Slices", |doc, _| {
        let mut areas = areas_of(doc);
        let mut next = areas.iter().map(|a| a.id).max().map_or(1, |m| m + 1);
        let mut added = 0;
        for sl in &doc.slices.list {
            // Layer-based slices were re-fitted to their layer after the last edit; `rect` is current.
            let r = sl.rect;
            if r.is_empty() {
                continue;
            }
            let coords = vec![f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)];
            if areas.iter().any(|a| a.shape == "rect" && a.coords == coords) {
                continue;
            }
            areas.push(Area {
                id: next,
                shape: "rect".into(),
                coords,
                href: sl.url.clone(),
                alt: sl.alt.clone(),
                target: sl.target.clone(),
                title: sl.message.clone(),
            });
            next += 1;
            added += 1;
        }
        store(doc, &areas);
        Ok(added)
    })?;
    Ok(json!({ "added": n }))
}

/// The HTML of the map: an `<img>` referencing `image_src` with `usemap`, and the `<map>`.
pub fn html(doc: &Document, areas: &[Area], map_name: &str, image_src: &str) -> String {
    let esc = crate::web_cmds::html_escape;
    let name = if map_name.trim().is_empty() { "map".to_string() } else { map_name.trim().to_string() };
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>");
    out.push_str(&esc(&doc.name));
    out.push_str("</title>\n</head>\n<body>\n");
    out.push_str(&format!(
        "<img src=\"{}\" width=\"{}\" height=\"{}\" usemap=\"#{}\" alt=\"{}\">\n",
        esc(image_src),
        doc.size.width,
        doc.size.height,
        esc(&name),
        esc(&doc.name)
    ));
    out.push_str(&format!("<map name=\"{}\">\n", esc(&name)));
    for a in areas {
        let coords: Vec<String> = a.coords.iter().map(|v| format!("{}", v.round() as i64)).collect();
        out.push_str(&format!("  <area shape=\"{}\" coords=\"{}\"", esc(&a.shape), coords.join(",")));
        if !a.href.is_empty() && crate::web_cmds::safe_slice_url(&a.href) {
            out.push_str(&format!(" href=\"{}\"", esc(&a.href)));
        }
        if !a.alt.is_empty() {
            out.push_str(&format!(" alt=\"{}\"", esc(&a.alt)));
        }
        if !a.title.is_empty() {
            out.push_str(&format!(" title=\"{}\"", esc(&a.title)));
        }
        if !a.target.is_empty() {
            out.push_str(&format!(" target=\"{}\"", esc(&a.target)));
        }
        out.push_str(">\n");
    }
    out.push_str("</map>\n</body>\n</html>\n");
    out
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "web.imageMap.export";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let areas = areas_of(&d.doc);
    let path = p.get("path").and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or_else(|| bad(CMD, "`path` (the .html file to write) is required"))?;
    let map_name = str_of(p, "mapName");
    let image_src = {
        let v = str_of(p, "imageSrc");
        if v.is_empty() {
            let stem = std::path::Path::new(&d.doc.name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
            format!("{stem}.png")
        } else {
            v
        }
    };
    let text = html(&d.doc, &areas, &map_name, &image_src);
    std::fs::write(path, text.as_bytes()).map_err(|e| EngineError::Other(format!("cannot write `{path}`: {e}")))?;
    Ok(json!({ "path": path, "areas": areas.len() }))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $params:expr, $enabled:expr, $journal:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $enabled, journal: $journal, run: $run }
        };
    }
    vec![
        spec!(
            "web.imageMap.add",
            "Image Map Area…",
            &["Filter", "Web"],
            r##"{"shape":"rect|circle|poly"="rect","coords":text,"x":px,"y":px,"width":px,"height":px,"radius":px,"href":text,"alt":text,"title":text,"target":text} (coords: "x0,y0,x1,y1" for a rect, "cx,cy,r" for a circle, "x0,y0,x1,y1,…" for a polygon) → {area}"##,
            has_doc,
            true,
            add
        ),
        spec!(
            "web.imageMap.set",
            "Image Map Area Options",
            &[],
            r##"{"area":id,"shape":"rect|circle|poly"?,"coords":[…]|"…"?,"href"?,"alt"?,"title"?,"target"?} → the area"##,
            has_areas,
            true,
            set
        ),
        spec!("web.imageMap.delete", "Delete Image Map Area", &[], r##"{"area":id | "areas":[id…]} → {deleted}"##, has_areas, true, delete),
        spec!("web.imageMap.clear", "Clear Image Map", &["Filter", "Web"], "{} → {cleared}", has_areas, true, |s, _| clear(s)),
        spec!("web.imageMap.list", "List Image Map Areas", &[], "{} → {areas:[{id,shape,coords,href,alt,title,target}]}", has_doc, false, |s, _| list(s)),
        spec!(
            "web.imageMap.fromSlices",
            "Image Map From Slices",
            &["Filter", "Web"],
            "{} (every user and layer-based slice becomes a rectangle area with its URL, alt and target) → {added}",
            has_doc,
            true,
            |s, _| from_slices(s)
        ),
        spec!(
            "web.imageMap.export",
            "Export Image Map…",
            &["Filter", "Web"],
            r##"{"path":text,"mapName":text,"imageSrc":text} (writes an HTML page with <img usemap> and the <map>; imageSrc defaults to <document>.png) → {path, areas}"##,
            has_areas,
            true,
            export
        ),
    ]
}
