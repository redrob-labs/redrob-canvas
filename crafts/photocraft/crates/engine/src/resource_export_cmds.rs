//! Resource and animation export commands (Photo mode, v0.11 parity):
//! `file.export.layersAsAnimation` (GIF / WebP / MNG from the layers, timing and disposal from
//! the `(delay)(combine|replace)` layer-name tags), `file.export.brush` (`.gbr`),
//! `file.export.imageHose` (`.gih`, one cell per layer), `file.export.pattern` (`.gpat` / open
//! `.pat`), `pattern.importGpat` (a `GPAT` pattern into the Patterns panel) and
//! `layer.animationTags` (set a layer's frame delay and disposal tags).

use redrob_photo_io::anim_layers::{Disposal, frame_name, strip_tags, tags};
use redrob_photo_io::gbr_export::BrushDepth;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn path_param<'a>(p: &'a Value, cmd: &str, what: &str) -> Result<&'a str> {
    p.get("path").and_then(Value::as_str).filter(|v| !v.trim().is_empty()).ok_or_else(|| bad(cmd, format!("pass `path` ({what})")))
}

fn ext_of(path: &str) -> String {
    std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn depth(p: &Value) -> BrushDepth {
    if p.get("color").and_then(Value::as_bool).unwrap_or(false) { BrushDepth::Color } else { BrushDepth::Mask }
}

fn spacing(p: &Value) -> u32 {
    p.get("spacing").and_then(Value::as_f64).map_or(25, |v| v.clamp(1.0, 1000.0).round() as u32)
}

/// `file.export.layersAsAnimation`: the layers as one animated GIF, WebP or MNG.
fn layers_as_animation(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.export.layersAsAnimation";
    let path = path_param(p, CMD, "the .gif, .webp or .mng to write")?;
    let format = p.get("format").and_then(Value::as_str).map(str::to_ascii_lowercase).unwrap_or_else(|| ext_of(path));
    if !matches!(format.as_str(), "gif" | "webp" | "mng") {
        return Err(bad(CMD, format!("`format` must be gif, webp or mng (got `{format}`)")));
    }
    let loop_forever = p.get("loop").and_then(Value::as_bool).unwrap_or(true);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let frames = d.doc.layers.iter().filter(|l| l.visible).count().max(1);
    let r = redrob_photo_io::anim_export::export_animation(&d.doc, &format, loop_forever).map_err(|e| EngineError::Other(e.to_string()))?;
    crate::file_cmds::write_file(path, &r.bytes)?;
    Ok(json!({ "path": path, "format": format, "frames": frames, "bytes": r.bytes.len(), "warnings": r.warnings }))
}

/// `file.export.brush`: the flattened image as a `.gbr` bitmap brush.
fn export_brush(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.export.brush";
    let path = path_param(p, CMD, "the .gbr to write")?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let r = redrob_photo_io::gbr_export::export_gbr(&d.doc, depth(p), spacing(p)).map_err(|e| EngineError::Other(e.to_string()))?;
    crate::file_cmds::write_file(path, &r.bytes)?;
    Ok(json!({ "path": path, "bytes": r.bytes.len(), "warnings": r.warnings }))
}

/// `file.export.imageHose`: the layers as the cells of a `.gih` animated brush.
fn export_hose(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.export.imageHose";
    let path = path_param(p, CMD, "the .gih to write")?;
    let selection = p.get("selection").and_then(Value::as_str).unwrap_or("incremental");
    if !redrob_photo_io::gbr_export::GIH_SELECTIONS.contains(&selection) {
        return Err(bad(CMD, format!("`selection` must be one of {}", redrob_photo_io::gbr_export::GIH_SELECTIONS.join(", "))));
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let cells = d.doc.layers.iter().filter(|l| l.visible).count().max(1);
    let r = redrob_photo_io::gbr_export::export_gih(&d.doc, depth(p), spacing(p), selection).map_err(|e| EngineError::Other(e.to_string()))?;
    crate::file_cmds::write_file(path, &r.bytes)?;
    Ok(json!({ "path": path, "cells": cells, "selection": selection, "bytes": r.bytes.len(), "warnings": r.warnings }))
}

/// `file.export.pattern`: the flattened image as a `GPAT` pattern file.
fn export_pattern(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.export.pattern";
    let path = path_param(p, CMD, "the .pat or .gpat to write")?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let r = redrob_photo_io::gbr_export::export_gpat(&d.doc).map_err(|e| EngineError::Other(e.to_string()))?;
    crate::file_cmds::write_file(path, &r.bytes)?;
    Ok(json!({ "path": path, "bytes": r.bytes.len(), "warnings": r.warnings }))
}

/// `file.export.postScript`: the flattened image as EPS or PostScript with encoding, preview,
/// level and encapsulation options.
fn export_postscript(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.export.postScript";
    use redrob_photo_io::eps::{EpsEncoding, EpsOptions};
    let path = path_param(p, CMD, "the .eps or .ps to write")?;
    let encoding = match p.get("encoding").and_then(Value::as_str).unwrap_or("ascii85") {
        "hex" => EpsEncoding::Hex,
        "ascii85" => EpsEncoding::Ascii85,
        "binary" => EpsEncoding::Binary,
        other => return Err(bad(CMD, format!("`encoding` must be hex, ascii85 or binary (got `{other}`)"))),
    };
    let encapsulated = p.get("encapsulated").and_then(Value::as_bool).unwrap_or_else(|| ext_of(path) != "ps");
    let tiff_preview = p.get("preview").and_then(Value::as_bool).unwrap_or(false);
    let level = p.get("level").and_then(Value::as_u64).map_or(2, |l| l.clamp(1, 3) as u8);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let r = redrob_photo_io::eps::export(&d.doc, EpsOptions { encapsulated, encoding, tiff_preview, level }).map_err(|e| EngineError::Other(e.to_string()))?;
    crate::file_cmds::write_file(path, &r.bytes)?;
    Ok(json!({ "path": path, "encapsulated": encapsulated, "level": level, "bytes": r.bytes.len(), "warnings": r.warnings }))
}

/// `pattern.importGpat`: a `GPAT` pattern file into the Patterns panel.
fn import_gpat(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "pattern.importGpat";
    let (bytes, stem) = crate::preset_import_cmds::file_bytes(p, CMD, "a GPAT .pat file")?;
    let (name, w, h, rgba) = redrob_photo_io::gbr_export::read_gpat(&bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    let name = if name.is_empty() { if stem.is_empty() { "Pattern".to_string() } else { stem } } else { name };
    let rect = redrob_photo_geom::Rect::new(0, 0, w as i32, h as i32);
    let surface = redrob_photo_raster::Surface::from_interleaved(redrob_photo_color::PixelFormat::RGBA8, rect, &rgba);
    let pat = redrob_photo_doc::Pattern::new(name.clone(), surface, w, h);
    let id = pat.id.clone();
    match s.patterns.items.iter_mut().find(|q| q.id == pat.id) {
        Some(q) => *q = pat,
        None => s.patterns.items.push(pat),
    }
    Ok(json!({ "pattern": id, "name": name, "width": w, "height": h }))
}

/// `layer.animationTags`: set (or clear) the active layer's frame delay and disposal tags.
fn animation_tags(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.animationTags";
    let delay = p.get("delay").and_then(Value::as_u64).map(|d| d.min(0xFF_FFFF) as u32);
    let disposal = match p.get("disposal").and_then(Value::as_str) {
        None => None,
        Some("combine") => Some(Disposal::Combine),
        Some("replace") => Some(Disposal::Replace),
        Some(other) => return Err(bad(CMD, format!("`disposal` must be combine or replace (got `{other}`)"))),
    };
    let clear = p.get("clear").and_then(Value::as_bool).unwrap_or(false);
    let mut name_out = String::new();
    s.edit("Animation Tags", |doc, active| {
        let id = active.ok_or_else(|| EngineError::Other("no active layer".into()))?;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let base = strip_tags(&l.name);
        let old = tags(&l.name);
        l.name = if clear {
            base
        } else {
            let d = delay.or(old.delay_ms).unwrap_or(100);
            let disp = disposal.or(old.disposal).unwrap_or(Disposal::Combine);
            let n = frame_name(&base, 0, d, disp);
            // `frame_name` numbers frames; keep the layer's own base name instead.
            n.replacen(&format!("{base} 0 "), &format!("{base} "), 1)
        };
        name_out = l.name.clone();
        Ok(())
    })?;
    let t = tags(&name_out);
    Ok(
        json!({ "name": name_out, "delay": t.delay_ms, "disposal": t.disposal.map(|d| match d { Disposal::Combine => "combine", Disposal::Replace => "replace" }) }),
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "file.export.layersAsAnimation",
            label: "Export Layers as Animation…",
            menu: &["File", "Export"],
            shortcut: None,
            params: r##"{"path":"file to write","format":"gif|webp|mng"? (from the extension),"loop":bool=true} → {path, format, frames, bytes, warnings}"##,
            enabled: has_doc,
            run: layers_as_animation,
            journal: true,
        },
        CommandSpec {
            id: "file.export.brush",
            label: "Export as Brush (.gbr)…",
            menu: &["File", "Export"],
            shortcut: None,
            params: r##"{"path":".gbr to write","color":bool=false (4-byte colour brush instead of a mask),"spacing":number=25 (% of the diameter)} → {path, bytes, warnings}"##,
            enabled: has_doc,
            run: export_brush,
            journal: true,
        },
        CommandSpec {
            id: "file.export.imageHose",
            label: "Export Layers as Image Hose (.gih)…",
            menu: &["File", "Export"],
            shortcut: None,
            params: r##"{"path":".gih to write","selection":"incremental|angular|random|velocity|pressure|xtilt|ytilt"="incremental","color":bool=false,"spacing":number=25} → {path, cells, selection, bytes, warnings}"##,
            enabled: has_doc,
            run: export_hose,
            journal: true,
        },
        CommandSpec {
            id: "file.export.pattern",
            label: "Export as Pattern (.pat)…",
            menu: &["File", "Export"],
            shortcut: None,
            params: r##"{"path":".pat or .gpat to write"} → {path, bytes, warnings}"##,
            enabled: has_doc,
            run: export_pattern,
            journal: true,
        },
        CommandSpec {
            id: "file.export.postScript",
            label: "Export as PostScript (.eps)…",
            menu: &["File", "Export"],
            shortcut: None,
            params: r##"{"path":".eps or .ps to write","encapsulated":bool? (default: not for .ps),"encoding":"hex|ascii85|binary"="ascii85","preview":bool=false (TIFF preview in a DOS EPS header),"level":1|2|3=2} → {path, encapsulated, level, bytes, warnings}"##,
            enabled: has_doc,
            run: export_postscript,
            journal: true,
        },
        CommandSpec {
            id: "pattern.importGpat",
            label: "Import Pattern File (GPAT)…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":"GPAT .pat file"?,"data":base64?} → {pattern, name, width, height}"##,
            enabled: always,
            run: import_gpat,
            journal: true,
        },
        CommandSpec {
            id: "layer.animationTags",
            label: "Animation Frame Tags…",
            menu: &["Layer"],
            shortcut: None,
            params: r##"{"delay":ms?,"disposal":"combine|replace"?,"clear":bool=false} → {name, delay, disposal}"##,
            enabled: has_doc,
            run: animation_tags,
            journal: true,
        },
    ]
}
