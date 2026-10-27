//! Workflow helpers: action search (`actions.search`), the reference images docker
//! (`reference.*`), the pre-installed script set (`plugins.*`: channels to layers, quick
//! settings, filter manager, last documents, script starter, ten scripts, a comics project
//! manager), numbered backups on save (`backups::rotate`) and the performance settings the
//! engine applies (worker threads).
//!
//! Everything here is built from the engine's own commands: a plugin is a command (or a few)
//! an agent, a shortcut or the UI runs, with no second scripting runtime.

use std::sync::atomic::{AtomicU32, Ordering};

use redrob_photo_paint::tile::b64_encode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, has_paintable};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn str_opt<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Action search
// ---------------------------------------------------------------------------

/// Score a command or action against a query: 0 = no match; higher is better. Every word of
/// the query must match the id, label, menu path or keywords (prefix matches score more).
fn score(query_words: &[String], id: &str, label: &str, menu: &[&str], keywords: Option<&str>) -> u32 {
    let hay_label = label.to_lowercase();
    let hay_id = id.to_lowercase();
    let hay_menu = menu.join(" ").to_lowercase();
    let hay_kw = keywords.map(str::to_lowercase).unwrap_or_default();
    let mut total = 0u32;
    for w in query_words {
        let s = if hay_label == *w {
            100
        } else if hay_label.starts_with(w) {
            60
        } else if hay_label.split(|c: char| !c.is_alphanumeric()).any(|t| t.starts_with(w.as_str())) {
            40
        } else if hay_label.contains(w.as_str()) {
            25
        } else if hay_id.contains(w.as_str()) {
            15
        } else if hay_menu.contains(w.as_str()) {
            10
        } else if hay_kw.contains(w.as_str()) {
            8
        } else {
            return 0;
        };
        total += s;
    }
    total
}

/// `actions.search`: type to find any command or recorded action; the best matches first with
/// whether each can run now.
fn actions_search(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.search";
    let query = str_opt(p, "query").ok_or_else(|| bad(cmd, "missing `query`"))?.to_lowercase();
    let limit = p.get("limit").and_then(Value::as_u64).map_or(20, |l| l.clamp(1, 200) as usize);
    let words: Vec<String> = query.split_whitespace().map(str::to_string).collect();
    let mut hits: Vec<(u32, Value)> = Vec::new();
    for c in crate::command_specs() {
        // Internal queries and the search itself stay out of the way.
        if c.id == "actions.search" || c.id.starts_with("command.") || c.id.starts_with("engine.") {
            continue;
        }
        let sc = score(&words, c.id, c.label, c.menu, crate::commands::SEARCH_KEYWORDS.iter().find(|(id, _)| *id == c.id).map(|(_, k)| *k));
        if sc == 0 {
            continue;
        }
        let reason = (c.enabled)(s).err();
        let mut entry = crate::commands::list_entry(s, c, reason);
        entry["kind"] = json!("command");
        entry["score"] = json!(sc);
        hits.push((sc, entry));
    }
    for a in &s.actions.list {
        let sc = score(&words, &a.name, &a.name, &["Actions"], None);
        if sc > 0 {
            hits.push((sc, json!({ "kind": "action", "id": "actions.play", "label": a.name, "params": json!({"action": a.name}), "enabled": true, "score": sc, "steps": a.steps.len() })));
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1["label"].as_str().cmp(&b.1["label"].as_str())));
    let total = hits.len();
    let results: Vec<Value> = hits.into_iter().take(limit).map(|(_, v)| v).collect();
    Ok(json!({ "query": query, "total": total, "results": results }))
}

/// `actions.runSearch`: run the best match of `query` (or the command `id`) with `params`.
fn actions_run_search(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.runSearch";
    let params = p.get("params").cloned().unwrap_or_else(|| json!({}));
    if let Some(id) = str_opt(p, "id") {
        return s.execute(id, params);
    }
    let found = actions_search(s, &json!({ "query": str_opt(p, "query").ok_or_else(|| bad(cmd, "missing `query` or `id`"))?, "limit": 1 }))?;
    let top = found["results"].as_array().and_then(|a| a.first()).cloned().ok_or_else(|| bad(cmd, "nothing matches the query"))?;
    let id = top["id"].as_str().unwrap_or("").to_string();
    let params = if top["kind"] == "action" { top["params"].clone() } else { params };
    s.execute(&id, params)
}

// ---------------------------------------------------------------------------
// Reference images
// ---------------------------------------------------------------------------

/// Image file extensions the reference browser lists.
const IMAGE_EXTS: [&str; 10] = ["png", "jpg", "jpeg", "webp", "tif", "tiff", "bmp", "gif", "tga", "jxl"];
/// Largest side a reference image is kept at.
const MAX_REFERENCE_SIDE: u32 = 2048;

/// The Reference Images docker: a browsed folder, its image files, the one shown and its zoom.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReferenceState {
    pub folder: String,
    pub files: Vec<String>,
    pub index: Option<usize>,
    /// 0 = fit the panel; otherwise a zoom factor.
    pub zoom: f32,
    /// Pan offset (fraction of the image) while zoomed.
    pub pan: [f32; 2],
    /// The shown image: width, height and 8-bit RGBA, with a revision the UI keys textures on.
    #[serde(skip)]
    pub image: Option<(u32, u32, std::sync::Arc<[u8]>)>,
    #[serde(skip)]
    pub rev: u64,
}

fn list_images(folder: &str) -> Result<Vec<String>> {
    let rd = std::fs::read_dir(folder).map_err(|e| EngineError::Other(format!("{folder}: {e}")))?;
    let mut files: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| IMAGE_EXTS.contains(&x.to_ascii_lowercase().as_str())))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    files.sort_by_key(|f| f.to_lowercase());
    Ok(files)
}

/// `reference.browse`: list the images of a folder (the docker's file list).
fn reference_browse(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "reference.browse";
    let folder = str_opt(p, "folder").ok_or_else(|| bad(cmd, "missing `folder`"))?.to_string();
    let files = list_images(&folder)?;
    let st = &mut s.reference_images;
    st.folder = folder.clone();
    st.files = files;
    st.index = None;
    st.image = None;
    st.rev += 1;
    Ok(json!({ "folder": folder, "files": s.reference_images.files, "count": s.reference_images.files.len() }))
}

/// Decode an image file to 8-bit RGBA no bigger than [`MAX_REFERENCE_SIDE`] a side.
fn load_reference(path: &str) -> Result<(u32, u32, Vec<u8>)> {
    let bytes = crate::file_cmds::read_file(path)?;
    let img = redrob_photo_codecs::decode(&bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err(EngineError::Other(format!("{path}: empty image")));
    }
    let rgba = img.to_rgba8();
    let k = (w.max(h) as f32 / MAX_REFERENCE_SIDE as f32).max(1.0);
    if k <= 1.0 {
        return Ok((w, h, rgba));
    }
    let (nw, nh) = (((w as f32 / k).round() as u32).max(1), ((h as f32 / k).round() as u32).max(1));
    let mut out = Vec::with_capacity((nw * nh * 4) as usize);
    for y in 0..nh {
        let sy = (((y as f32 + 0.5) * k) as u32).min(h - 1);
        for x in 0..nw {
            let sx = (((x as f32 + 0.5) * k) as u32).min(w - 1);
            let i = ((sy * w + sx) * 4) as usize;
            out.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    Ok((nw, nh, out))
}

/// `reference.show`: show one image (`index` into the browsed folder, or a `path`).
fn reference_show(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "reference.show";
    let (path, index) = match (p.get("index").and_then(Value::as_u64), str_opt(p, "path")) {
        (Some(i), _) => {
            let st = &s.reference_images;
            let f = st.files.get(i as usize).ok_or_else(|| bad(cmd, format!("no image {i} in the browsed folder")))?;
            (std::path::Path::new(&st.folder).join(f).to_string_lossy().into_owned(), Some(i as usize))
        }
        (None, Some(path)) => {
            let folder = std::path::Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
            let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if s.reference_images.folder != folder {
                s.reference_images.folder = folder.clone();
                s.reference_images.files = list_images(&folder).unwrap_or_default();
            }
            let index = s.reference_images.files.iter().position(|f| *f == name);
            (path.to_string(), index)
        }
        (None, None) => return Err(bad(cmd, "give `index` or `path`")),
    };
    let (w, h, rgba) = load_reference(&path)?;
    let st = &mut s.reference_images;
    st.index = index;
    st.image = Some((w, h, rgba.into()));
    st.rev += 1;
    Ok(json!({ "path": path, "index": index, "width": w, "height": h }))
}

/// `reference.next` / `reference.previous`: step through the browsed folder.
fn reference_step(s: &mut Session, forward: bool, cmd: &str) -> Result<Value> {
    let n = s.reference_images.files.len();
    if n == 0 {
        return Err(bad(cmd, "no folder browsed (reference.browse first)"));
    }
    let i = match (s.reference_images.index, forward) {
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
        (None, true) => 0,
        (None, false) => n - 1,
    };
    reference_show(s, &json!({ "index": i }))
}

/// `reference.zoom`: the docker's zoom (0 = fit) and pan.
fn reference_zoom(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "reference.zoom";
    let st = &mut s.reference_images;
    if let Some(z) = p.get("zoom") {
        let z = z.as_f64().filter(|z| z.is_finite() && (0.0..=64.0).contains(z)).ok_or_else(|| bad(cmd, "`zoom` must be 0 (fit) or 0.01..64"))?;
        st.zoom = z as f32;
    }
    if let Some(pan) = p.get("pan").and_then(Value::as_array).filter(|a| a.len() == 2) {
        let v = |i: usize| pan[i].as_f64().filter(|v| v.is_finite()).map_or(0.0, |v| v.clamp(-1.0, 1.0) as f32);
        st.pan = [v(0), v(1)];
    }
    st.rev += 1;
    Ok(json!({ "zoom": st.zoom, "pan": st.pan }))
}

/// `reference.get`: the docker's state; with `pixels: true` the shown image as base64 RGBA.
fn reference_get(s: &mut Session, p: &Value) -> Result<Value> {
    let st = &s.reference_images;
    let mut v = json!({ "folder": st.folder, "files": st.files, "index": st.index, "zoom": st.zoom, "pan": st.pan });
    if let Some((w, h, px)) = &st.image {
        v["width"] = json!(w);
        v["height"] = json!(h);
        if p.get("pixels").and_then(Value::as_bool).unwrap_or(false) {
            v["rgba"] = json!(b64_encode(px));
        }
    }
    Ok(v)
}

/// `reference.close`: drop the shown image.
fn reference_close(s: &mut Session, _: &Value) -> Result<Value> {
    let st = &mut s.reference_images;
    let had = st.image.take().is_some();
    st.index = None;
    st.rev += 1;
    Ok(json!({ "closed": had }))
}

// ---------------------------------------------------------------------------
// Pre-installed scripts
// ---------------------------------------------------------------------------

/// `plugins.channelsToLayers`: each colour channel of the merged image (or of `layer`) becomes a
/// grayscale layer above the active one, named after the channel.
fn channels_to_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugins.channelsToLayers";
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let area = doc.bounds();
    let fmt = doc.pixel_format();
    let names = crate::channel_cmds::color_names(fmt.mode);
    let nc = fmt.mode.color_channels();
    // Pixels of the merged image (or one layer), in the document's channels.
    let native: Vec<f32> = match p.get("layer").and_then(Value::as_u64) {
        Some(id) => {
            let l = doc.layer(redrob_photo_doc::LayerId(id)).ok_or(EngineError::NoLayer(redrob_photo_doc::LayerId(id)))?;
            let surf = l.surface().ok_or_else(|| bad(cmd, format!("layer \"{}\" has no pixels", l.name)))?;
            let sf = surf.format();
            let raw = surf.read_region(area);
            if sf == fmt {
                raw
            } else {
                raw.chunks_exact(sf.channels()).flat_map(|px| redrob_photo_raster::from_rgba(&fmt, redrob_photo_raster::to_rgba(&sf, px))).collect()
            }
        }
        None => {
            let buf = redrob_photo_compose::render(&doc, area);
            buf.px.iter().flat_map(|c| redrob_photo_raster::from_rgba(&fmt, *c)).collect()
        }
    };
    let n = fmt.channels();
    let count = names.len().min(nc);
    let ids = s.edit("Channels to Layers", |doc, active| {
        let mut ids = Vec::new();
        for c in 0..count {
            let mut layer = redrob_photo_doc::Layer::raster(names[c].to_string(), doc.pixel_format());
            let f = layer.surface_mut().map(|s| s.format()).ok_or_else(|| bad(cmd, "a new layer has no pixels"))?;
            let mut data = Vec::with_capacity(native.len() / n * f.channels());
            for px in native.chunks_exact(n) {
                let v = px[c].clamp(0.0, 1.0);
                data.extend_from_slice(&redrob_photo_raster::from_rgba(&f, [v, v, v, 1.0]));
            }
            if let Some(surf) = layer.surface_mut() {
                surf.write_region(area, &data);
            }
            let id = doc.insert_above(*active, layer);
            *active = Some(id);
            ids.push(id.0);
        }
        Ok(ids)
    })?;
    Ok(json!({ "layers": ids, "channels": names[..count].to_vec() }))
}

/// Quick Settings: the size and opacity steps the docker offers.
pub const QUICK_SIZES: [f32; 10] = [1.0, 3.0, 5.0, 10.0, 20.0, 40.0, 80.0, 160.0, 320.0, 640.0];
pub const QUICK_OPACITIES: [f32; 10] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];

/// `plugins.quickSettings`: the size and opacity grid; `size` or `opacity` applies one.
fn quick_settings(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugins.quickSettings";
    let mut patch = serde_json::Map::new();
    if let Some(v) = p.get("size") {
        let v = v.as_f64().filter(|v| v.is_finite() && (0.5..=5000.0).contains(v)).ok_or_else(|| bad(cmd, "`size` must be 0.5..5000"))?;
        patch.insert("size".into(), json!(v));
    }
    if let Some(v) = p.get("opacity") {
        let v = v.as_f64().filter(|v| v.is_finite() && (0.0..=1.0).contains(v)).ok_or_else(|| bad(cmd, "`opacity` must be 0..1"))?;
        patch.insert("opacity".into(), json!(v));
    }
    if let Some(v) = p.get("flow") {
        let v = v.as_f64().filter(|v| v.is_finite() && (0.0..=1.0).contains(v)).ok_or_else(|| bad(cmd, "`flow` must be 0..1"))?;
        patch.insert("flow".into(), json!(v));
    }
    if !patch.is_empty() {
        crate::brush_cmds::set_brush(s, &json!({ "brush": Value::Object(patch) }))?;
    }
    let b = &s.tools.brush;
    Ok(json!({ "sizes": QUICK_SIZES, "opacities": QUICK_OPACITIES, "size": b.size, "opacity": b.opacity, "flow": b.flow }))
}

/// `plugins.filterManager`: the filters with their parameter descriptions (the Filter Manager's
/// list); `apply` runs one with `params`.
fn filter_manager(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(id) = str_opt(p, "apply") {
        if !id.starts_with("filter.") {
            return Err(bad("plugins.filterManager", "`apply` must be a filter.* command"));
        }
        return s.execute(id, p.get("params").cloned().unwrap_or_else(|| json!({})));
    }
    let filters: Vec<Value> = crate::command_specs()
        .iter()
        .filter(|c| c.id.starts_with("filter.") && !c.menu.is_empty())
        .map(|c| json!({ "id": c.id, "label": c.label, "menu": c.menu, "params": c.params, "enabled": (c.enabled)(s).is_ok() }))
        .collect();
    Ok(json!({ "filters": filters, "count": filters.len() }))
}

/// `plugins.lastDocuments`: the recent files with whether each still exists; `open` opens one.
fn last_documents(s: &mut Session, p: &Value) -> Result<Value> {
    let recent = s.prefs().file_handling.recent_files.clone();
    if let Some(i) = p.get("open").and_then(Value::as_u64) {
        let path = recent.get(i as usize).cloned().ok_or_else(|| bad("plugins.lastDocuments", format!("no recent document {i}")))?;
        let bytes = crate::file_cmds::read_file(&path)?;
        let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
        let doc = crate::file_cmds::import(&name, &bytes)?;
        let idx = s.add_document(doc, Some(path.clone()));
        return Ok(json!({ "opened": path, "document": idx }));
    }
    let items: Vec<Value> = recent
        .iter()
        .map(|f| {
            let exists = std::path::Path::new(f).is_file();
            let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.clone());
            json!({ "path": f, "name": name, "exists": exists })
        })
        .collect();
    Ok(json!({ "documents": items }))
}

/// The script template the Script Starter writes: a recorded-action file with one step.
pub fn script_template(name: &str) -> String {
    serde_json::to_string_pretty(&json!({
        "format": "redrob-actions",
        "version": 1,
        "actions": [{
            "name": name,
            "steps": [
                ["layer.new.layer", { "name": "From script" }],
                ["filter.other.highPass", { "radius": 10.0 }]
            ]
        }]
    }))
    .unwrap_or_default()
}

/// `plugins.scriptStarter`: write a starter script (a recorded-action file) to `path` and load
/// it into the Actions panel, so a new script begins from something that runs.
fn script_starter(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_opt(p, "name").unwrap_or("My Script").to_string();
    let text = script_template(&name);
    let mut out = json!({ "name": name, "bytes": text.len() });
    if let Some(path) = str_opt(p, "path") {
        crate::file_cmds::write_file(path, text.as_bytes())?;
        out["path"] = json!(path);
    } else {
        out["text"] = json!(text);
    }
    if p.get("load").and_then(Value::as_bool).unwrap_or(true) {
        let steps = vec![("layer.new.layer".to_string(), json!({ "name": "From script" })), ("filter.other.highPass".to_string(), json!({ "radius": 10.0 }))];
        s.actions.list.retain(|a| a.name != name);
        s.actions.list.push(crate::actions_cmds::Action { name: name.clone(), steps, ..Default::default() });
        s.actions.rev = s.actions.rev.wrapping_add(1);
        out["loaded"] = json!(true);
    }
    Ok(out)
}

/// Ten Scripts: action names on ten shortcut slots (kept in the preferences' `tenScripts`).
fn ten_scripts(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugins.tenScripts";
    let slot = p.get("slot").and_then(Value::as_u64).filter(|i| (1..=10).contains(i)).map(|i| i as usize - 1);
    let mut slots: Vec<Option<String>> = s.prefs().tools.ten_scripts.clone();
    slots.resize(10, None);
    match (slot, p.get("action"), p.get("run").and_then(Value::as_bool).unwrap_or(false)) {
        (Some(i), Some(Value::Null), _) => {
            slots[i] = None;
            s.prefs.edit(|pr| pr.tools.ten_scripts = slots.clone());
        }
        (Some(i), Some(Value::String(a)), _) => {
            if !s.actions.list.iter().any(|x| x.name == *a) {
                return Err(bad(cmd, format!("no action named `{a}`")));
            }
            slots[i] = Some(a.clone());
            s.prefs.edit(|pr| pr.tools.ten_scripts = slots.clone());
        }
        (Some(i), None, true) => {
            let a = slots[i].clone().ok_or_else(|| bad(cmd, format!("slot {} is empty", i + 1)))?;
            return s.execute("actions.play", json!({ "action": a }));
        }
        (Some(_), Some(_), _) => return Err(bad(cmd, "`action` must be an action name or null")),
        _ => {}
    }
    Ok(json!({ "slots": slots.iter().enumerate().map(|(i, a)| json!({"slot": i + 1, "action": a})).collect::<Vec<_>>() }))
}

/// A comics project: a folder with `comicConfig.json` listing its pages.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ComicConfig {
    pub project_name: String,
    pub pages_location: String,
    pub export_location: String,
    pub template_location: String,
    pub page_width: u32,
    pub page_height: u32,
    pub page_dpi: f32,
    pub pages: Vec<String>,
    pub metadata: std::collections::BTreeMap<String, String>,
}

const COMIC_CONFIG: &str = "comicConfig.json";

fn comic_read(dir: &str) -> Result<ComicConfig> {
    let path = std::path::Path::new(dir).join(COMIC_CONFIG);
    let bytes = crate::file_cmds::read_file(&path.to_string_lossy())?;
    serde_json::from_slice(&bytes).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))
}

fn comic_write(dir: &str, cfg: &ComicConfig) -> Result<()> {
    let path = std::path::Path::new(dir).join(COMIC_CONFIG);
    let text = serde_json::to_string_pretty(cfg).map_err(|e| EngineError::Other(e.to_string()))?;
    crate::file_cmds::write_file(&path.to_string_lossy(), text.as_bytes())
}

/// `plugins.comics`: a small comics project manager. `action` = new (a project folder with its
/// config), addPage (a new page document from the project's page size, saved into `pages/`),
/// list, exportPages (every page flattened to PNG in `export/`), setMetadata.
fn comics(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugins.comics";
    let action = str_opt(p, "action").ok_or_else(|| bad(cmd, "missing `action` (new, addPage, list, exportPages, setMetadata)"))?;
    let dir = str_opt(p, "project").ok_or_else(|| bad(cmd, "missing `project` (the project folder)"))?.to_string();
    match action {
        "new" => {
            std::fs::create_dir_all(std::path::Path::new(&dir).join("pages")).map_err(|e| EngineError::Other(format!("{dir}: {e}")))?;
            std::fs::create_dir_all(std::path::Path::new(&dir).join("export")).map_err(|e| EngineError::Other(format!("{dir}: {e}")))?;
            std::fs::create_dir_all(std::path::Path::new(&dir).join("templates")).map_err(|e| EngineError::Other(format!("{dir}: {e}")))?;
            let cfg = ComicConfig {
                project_name: str_opt(p, "name").unwrap_or("Comic").to_string(),
                pages_location: "pages".into(),
                export_location: "export".into(),
                template_location: "templates".into(),
                page_width: p.get("width").and_then(Value::as_u64).unwrap_or(2480).clamp(16, 30000) as u32,
                page_height: p.get("height").and_then(Value::as_u64).unwrap_or(3508).clamp(16, 30000) as u32,
                page_dpi: p.get("dpi").and_then(Value::as_f64).unwrap_or(300.0).clamp(1.0, 10000.0) as f32,
                pages: Vec::new(),
                metadata: Default::default(),
            };
            comic_write(&dir, &cfg)?;
            Ok(json!({ "project": dir, "config": cfg }))
        }
        "addPage" => {
            let mut cfg = comic_read(&dir)?;
            let n = cfg.pages.len() + 1;
            let name = format!("page_{n:03}.rphoto");
            let rel = format!("{}/{}", cfg.pages_location, name);
            let path = std::path::Path::new(&dir).join(&rel).to_string_lossy().into_owned();
            s.execute(
                "file.new",
                json!({ "name": format!("{} page {n}", cfg.project_name), "width": cfg.page_width, "height": cfg.page_height, "resolution": cfg.page_dpi }),
            )?;
            let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
            crate::file_cmds::save_doc(&doc, &path, None)?;
            if let Some(st) = s.active_mut() {
                st.saved_to(path.clone());
            }
            cfg.pages.push(rel.clone());
            comic_write(&dir, &cfg)?;
            Ok(json!({ "page": rel, "path": path, "pages": cfg.pages.len() }))
        }
        "list" => {
            let cfg = comic_read(&dir)?;
            Ok(json!({ "project": dir, "config": cfg }))
        }
        "setMetadata" => {
            let mut cfg = comic_read(&dir)?;
            if let Some(o) = p.get("metadata").and_then(Value::as_object) {
                for (k, v) in o {
                    match v {
                        Value::Null => {
                            cfg.metadata.remove(k);
                        }
                        v => {
                            cfg.metadata.insert(k.clone(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
                        }
                    }
                }
            }
            if let Some(n) = str_opt(p, "name") {
                cfg.project_name = n.to_string();
            }
            comic_write(&dir, &cfg)?;
            Ok(json!({ "config": cfg }))
        }
        "exportPages" => {
            let cfg = comic_read(&dir)?;
            let ext = str_opt(p, "format").unwrap_or("png");
            if !matches!(ext, "png" | "jpg" | "jpeg" | "webp" | "tif" | "tiff") {
                return Err(bad(cmd, "`format` must be png, jpg, webp or tif"));
            }
            let mut out = Vec::new();
            let mut errors = Vec::new();
            for (i, rel) in cfg.pages.iter().enumerate() {
                let src = std::path::Path::new(&dir).join(rel);
                let dst = std::path::Path::new(&dir).join(&cfg.export_location).join(format!("{}_{:03}.{ext}", cfg.project_name.replace(' ', "_"), i + 1));
                let r = (|| -> Result<()> {
                    let bytes = crate::file_cmds::read_file(&src.to_string_lossy())?;
                    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let doc = crate::file_cmds::import(&name, &bytes)?;
                    crate::file_cmds::save_doc(&doc, &dst.to_string_lossy(), None)?;
                    Ok(())
                })();
                match r {
                    Ok(()) => out.push(dst.to_string_lossy().into_owned()),
                    Err(e) => errors.push(json!({ "page": rel, "error": e.to_string() })),
                }
            }
            Ok(json!({ "exported": out, "errors": errors }))
        }
        other => Err(bad(cmd, format!("unknown action `{other}` (new, addPage, list, exportPages, setMetadata)"))),
    }
}

// ---------------------------------------------------------------------------
// Backups
// ---------------------------------------------------------------------------

/// Numbered backups kept beside a saved file: `name.ext~1` is the previous version, `~2` the one
/// before, up to the count. The shell sets the count from Preferences › File Handling before a
/// save; headless saves use the default.
pub mod backups {
    use super::*;

    static COUNT: AtomicU32 = AtomicU32::new(1);

    /// The number of numbered backups saves keep (0 = none).
    pub fn count() -> u32 {
        COUNT.load(Ordering::Relaxed)
    }

    /// Set the number of numbered backups saves keep.
    pub fn set_count(n: u32) {
        COUNT.store(n.min(99), Ordering::Relaxed);
    }

    /// The backup path number `n` of `path`.
    pub fn backup_path(path: &str, n: u32) -> String {
        format!("{path}~{n}")
    }

    /// Before `path` is overwritten: shift its existing backups up one number and copy the
    /// current file to `~1`, dropping the oldest beyond `count`. A file that does not exist yet
    /// leaves nothing. Errors are reported, never fatal to the save.
    pub fn rotate(path: &str, count: u32) -> Vec<String> {
        let mut warnings = Vec::new();
        if count == 0 || !std::path::Path::new(path).is_file() {
            return warnings;
        }
        let _ = std::fs::remove_file(backup_path(path, count));
        for i in (1..count).rev() {
            let (from, to) = (backup_path(path, i), backup_path(path, i + 1));
            if std::path::Path::new(&from).is_file()
                && let Err(e) = std::fs::rename(&from, &to)
            {
                warnings.push(format!("could not move backup {from}: {e}"));
            }
        }
        if let Err(e) = std::fs::copy(path, backup_path(path, 1)) {
            warnings.push(format!("could not write backup of {path}: {e}"));
        }
        warnings
    }

    /// [`rotate`] with the configured count.
    pub fn rotate_default(path: &str) -> Vec<String> {
        rotate(path, count())
    }

    /// The backups of `path` that exist, newest first.
    pub fn list(path: &str) -> Vec<String> {
        (1..=99).map(|i| backup_path(path, i)).take_while(|b| std::path::Path::new(b).is_file()).collect()
    }
}

/// `file.backups`: the numbered backups of a path (the active document's by default); `restore`
/// opens one as a new document; `delete` removes them all.
fn file_backups(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.backups";
    let path = match str_opt(p, "path") {
        Some(p) => p.to_string(),
        None => s.active().and_then(|d| d.path.clone()).ok_or_else(|| bad(cmd, "the document has no file; pass `path`"))?,
    };
    let list = backups::list(&path);
    if let Some(n) = p.get("restore").and_then(Value::as_u64) {
        let b = backups::backup_path(&path, n as u32);
        if !std::path::Path::new(&b).is_file() {
            return Err(bad(cmd, format!("no backup {n} of {path}")));
        }
        let bytes = crate::file_cmds::read_file(&b)?;
        let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let doc = crate::file_cmds::import(&name, &bytes)?;
        let idx = s.add_document(doc, None);
        return Ok(json!({ "restored": b, "document": idx }));
    }
    if p.get("delete").and_then(Value::as_bool).unwrap_or(false) {
        let mut n = 0;
        for b in &list {
            if std::fs::remove_file(b).is_ok() {
                n += 1;
            }
        }
        return Ok(json!({ "deleted": n }));
    }
    Ok(json!({ "path": path, "backups": list, "count": backups::count() }))
}

// ---------------------------------------------------------------------------
// Performance
// ---------------------------------------------------------------------------

/// Apply the performance preferences the engine owns: the worker thread count (0 = every
/// core). The global pool can only be built once per process, so later changes apply at the
/// next launch, which the result says.
pub fn apply_performance(prefs: &crate::prefs::Performance) -> Value {
    static APPLIED: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    let want = prefs.threads;
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get() as u32);
    let n = if want == 0 { cores } else { want.min(cores.max(1) * 4) };
    let applied = *APPLIED.get_or_init(|| {
        let ok = rayon::ThreadPoolBuilder::new().num_threads(n as usize).build_global().is_ok();
        if ok { n } else { rayon::current_num_threads() as u32 }
    });
    json!({ "threads": applied, "requested": want, "cores": cores, "restartNeeded": applied != n })
}

/// `performance.apply`: apply the thread count and report the live settings.
fn performance_apply(s: &mut Session, _: &Value) -> Result<Value> {
    let perf = s.prefs().performance.clone();
    let mut v = apply_performance(&perf);
    backups::set_count(s.prefs().file_handling.backup_count);
    v["backups"] = json!(backups::count());
    v["scalingFilter"] = json!(perf.scaling_filter.name());
    v["pixelGridThreshold"] = json!(perf.pixel_grid_threshold);
    v["swapDirectory"] = json!(perf.swap_directory);
    v["frameRateLimit"] = json!(perf.frame_rate_limit);
    Ok(v)
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $en:expr, $run:expr, $journal:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: $en, run: $run, journal: $journal }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "actions.search",
            "Search Actions",
            r##"{"query":string,"limit":1..200=20} → {query, total, results:[{kind, id, label, menu, params, enabled, score}]}"##,
            always,
            actions_search,
            false
        ),
        spec!(
            "actions.runSearch",
            "Run Found Action",
            r##"{"query":string? (the best match runs)|"id":string,"params":{…}?}"##,
            always,
            actions_run_search,
            true
        ),
        spec!("reference.browse", "Browse Reference Images", r##"{"folder":string} → {folder, files, count}"##, always, reference_browse, false),
        spec!("reference.show", "Show Reference Image", r##"{"index":n|"path":string} → {path, index, width, height}"##, always, reference_show, false),
        spec!("reference.next", "Next Reference Image", "{}", always, |s, _| reference_step(s, true, "reference.next"), false),
        spec!("reference.previous", "Previous Reference Image", "{}", always, |s, _| reference_step(s, false, "reference.previous"), false),
        spec!("reference.zoom", "Zoom Reference Image", r##"{"zoom":0 (fit)|0.01..64?,"pan":[x,y]?}"##, always, reference_zoom, false),
        spec!(
            "reference.get",
            "Reference Image State",
            r##"{"pixels":bool=false} → {folder, files, index, zoom, pan, width?, height?, rgba?}"##,
            always,
            reference_get,
            false
        ),
        spec!("reference.close", "Close Reference Image", "{}", always, reference_close, false),
        spec!(
            "plugins.channelsToLayers",
            "Channels to Layers",
            r##"{"layer":id?=merged image} → {layers, channels}"##,
            has_paintable,
            channels_to_layers,
            true
        ),
        spec!(
            "plugins.quickSettings",
            "Quick Settings",
            r##"{"size":0.5..5000?,"opacity":0..1?,"flow":0..1?} → {sizes, opacities, size, opacity, flow}"##,
            always,
            quick_settings,
            true
        ),
        spec!(
            "plugins.filterManager",
            "Filter Manager",
            r##"{"apply":"filter.*"?,"params":{…}?} → {filters:[{id, label, menu, params, enabled}]}"##,
            always,
            filter_manager,
            true
        ),
        spec!("plugins.lastDocuments", "Last Documents", r##"{"open":index?} → {documents:[{path, name, exists}]}"##, always, last_documents, true),
        spec!(
            "plugins.scriptStarter",
            "Script Starter",
            r##"{"name":string="My Script","path":".json file"?,"load":bool=true} → {name, path|text, loaded}"##,
            always,
            script_starter,
            true
        ),
        spec!(
            "plugins.tenScripts",
            "Ten Scripts",
            r##"{"slot":1..10?,"action":name|null?,"run":bool=false} → {slots:[{slot, action}]}"##,
            always,
            ten_scripts,
            true
        ),
        spec!(
            "plugins.comics",
            "Comics Manager",
            r##"{"action":"new|addPage|list|exportPages|setMetadata","project":folder,"name":string?,"width":px?,"height":px?,"dpi":n?,"format":"png|jpg|webp|tif"="png","metadata":{…}?}"##,
            always,
            comics,
            true
        ),
        spec!(
            "file.backups",
            "Numbered Backups",
            r##"{"path":string?=the document's file,"restore":n?,"delete":bool=false} → {path, backups, count}"##,
            always,
            file_backups,
            true
        ),
        spec!(
            "performance.apply",
            "Apply Performance Settings",
            "{} → {threads, requested, cores, restartNeeded, backups, scalingFilter, pixelGridThreshold, swapDirectory, frameRateLimit}",
            always,
            performance_apply,
            false
        ),
    ]
}
