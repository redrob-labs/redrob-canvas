//! Brush engine commands: strokes painted by an engine other than the tip-stamping Pixel engine
//! (`paint.engineStroke`), any filter through a brush (`paint.filterBrush`), text as a brush tip
//! (`brush.textTip`) and MyPaint-style brush files (`brush.mypaint.parse`).
//!
//! The engine settings travel in the command params (`"engine": {"kind": "sketch", …}`), so a
//! journaled stroke replays with exactly the engine it was painted with; the UI keeps the current
//! engine in its tool options.

use redrob_photo_geom::Rect;
use redrob_photo_paint::engines::{EngineSettings, KINDS, MyPaintSettings};
use redrob_photo_paint::retouch::{Region, apply_coverage, stroke_coverage};
use redrob_photo_paint::{GrayTile, Stroke, TipShape};
use redrob_photo_raster::Surface;
use serde_json::{Value, json};

use crate::brush_cmds::{parse_points, resolve_brush};
use crate::commands::{CommandSpec, has_paintable};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn flag(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}

fn layer_id(s: &Session, p: &Value) -> Result<redrob_photo_doc::LayerId> {
    match p.get("layer").and_then(Value::as_u64) {
        Some(id) => Ok(redrob_photo_doc::LayerId(id)),
        None => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into())),
    }
}

fn damage_json(s: &mut Session, dmg: Rect) -> Value {
    if let Some(st) = s.active_mut() {
        st.last_damage = Some(dmg);
    }
    json!({ "damage": [dmg.x0, dmg.y0, dmg.width(), dmg.height()] })
}

/// The engine settings of a stroke command: the `engine` object (`kind` plus that engine's
/// fields), or a bare `kind` string for the engine's defaults.
pub fn parse_engine(p: &Value, cmd: &str) -> Result<EngineSettings> {
    match p.get("engine") {
        None | Some(Value::Null) => Ok(EngineSettings::Pixel),
        Some(Value::String(kind)) => EngineSettings::by_kind(kind).ok_or_else(|| bad(cmd, format!("unknown engine `{kind}` (one of {})", KINDS.join(", ")))),
        Some(v) => {
            let kind = v.get("kind").and_then(Value::as_str).ok_or_else(|| bad(cmd, "`engine` needs a `kind`"))?;
            // Unknown fields are ignored so a newer file still loads; the kind must exist.
            let mut defaults = serde_json::to_value(EngineSettings::by_kind(kind).ok_or_else(|| bad(cmd, format!("unknown engine `{kind}`")))?)
                .map_err(|e| bad(cmd, e.to_string()))?;
            if let (Some(d), Some(o)) = (defaults.as_object_mut(), v.as_object()) {
                for (k, val) in o {
                    d.insert(k.clone(), val.clone());
                }
            }
            serde_json::from_value(defaults).map_err(|e| bad(cmd, format!("bad `engine`: {e}")))
        }
    }
}

/// A flattened composite of the visible layers over `area` (Sample All Layers).
fn composite_sample(doc: &redrob_photo_doc::Document, area: Rect) -> Surface {
    let mut surf = Surface::new(redrob_photo_color::PixelFormat::RGBA32F);
    if !area.is_empty() {
        let buf = redrob_photo_compose::render(doc, area);
        let flat: Vec<f32> = buf.px.iter().flatten().copied().collect();
        surf.write_region(area, &flat);
    }
    surf
}

/// `paint.engineStroke`: a stroke painted by one of the alternative engines.
pub fn engine_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.engineStroke";
    let pts = parse_points(p, cmd)?;
    crate::brush_cmds::check_coords(&pts, cmd)?;
    let brush = resolve_brush(s, p, cmd)?;
    let engine = parse_engine(p, cmd)?;
    let sample_all = flag(p, "sampleAllLayers", false) && engine.samples_canvas();
    let id = if crate::channel_cmds::is_channel_target(p) { None } else { Some(layer_id(s, p)?) };
    let stroke = Stroke { brush, points: pts };
    let label = match &engine {
        EngineSettings::Pixel => "Brush Tool",
        EngineSettings::Sketch(_) => "Sketch Brush",
        EngineSettings::Hatching(_) => "Hatching Brush",
        EngineSettings::Shape(_) => "Shape Brush",
        EngineSettings::Grid(_) => "Grid Brush",
        EngineSettings::Particle(_) => "Particle Brush",
        EngineSettings::Curve(_) => "Curve Brush",
        EngineSettings::Dyna(_) => "Dyna Brush",
        EngineSettings::TangentNormal(_) => "Tangent Normal Brush",
        EngineSettings::MyPaint(_) => "MyPaint Brush",
        EngineSettings::Spray(_) => "Spray Brush",
        EngineSettings::Bristle(_) => "Bristle Brush",
        EngineSettings::Deform(_) => "Deform Brush",
    };
    let cmyk = crate::brush_cmds::doc_cmyk_space(s);
    let dmg = redrob_photo_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        s.edit(label, |doc, _| {
            let sel = doc.selection.clone();
            let sample = sample_all.then(|| {
                let area = redrob_photo_paint::engines::buffer::points_bounds(&stroke.points, stroke.brush.size.max(1.0) * 2.0 + 8.0);
                composite_sample(doc, area)
            });
            let (surf, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
            Ok(match redrob_photo_paint::engines::paint(surf, sample.as_ref(), &stroke, &engine, sel.as_ref(), lock) {
                Some(r) => r,
                None => redrob_photo_paint::render_stroke(surf, &stroke.brush, &stroke.points, sel.as_ref(), lock, 1.0),
            })
        })
    })?;
    Ok(damage_json(s, dmg))
}

/// `paint.filterBrush`: a filter applied through the brush's coverage.
pub fn filter_brush(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.filterBrush";
    let pts = parse_points(p, cmd)?;
    crate::brush_cmds::check_coords(&pts, cmd)?;
    let brush = resolve_brush(s, p, cmd)?;
    let filter =
        p.get("filter").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `filter` (a filter command id, e.g. `filter.blur.gaussianBlur`)"))?.to_string();
    let fparams = p.get("filterParams").cloned().unwrap_or_else(|| json!({}));
    if crate::filters::params_for(&filter, &fparams).is_none() && !filter.starts_with("image.adjustments.") {
        return Err(bad(cmd, format!("`{filter}` is not a pixel filter")));
    }
    let id = if crate::channel_cmds::is_channel_target(p) { None } else { Some(layer_id(s, p)?) };
    let sample_all = flag(p, "sampleAllLayers", false);
    let stroke = Stroke { brush, points: pts };
    let canvas = s.active().map_or(Rect::EMPTY, |st| Rect::from_size(st.doc.size));
    let dmg = s.edit("Filter Brush", |doc, _| {
        let sel = doc.selection.clone();
        let (rect, cov) = stroke_coverage(&stroke);
        if rect.is_empty() {
            return Ok(Rect::EMPTY);
        }
        // The filtered image of the stroke's area (plus a margin for neighbourhood filters),
        // from the layer or from everything visible.
        let margin = Rect::new(rect.x0 - 64, rect.y0 - 64, rect.x1 + 64, rect.y1 + 64);
        let source: Surface = if sample_all { composite_sample(doc, margin) } else { crate::channel_cmds::target_surface(doc, id, p)?.0.clone() };
        let (surf, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
        let filtered = crate::filters::apply_filter_to_surface(&filter, &fparams, &source, margin, None, canvas)
            .ok_or_else(|| EngineError::Other(format!("`{filter}` could not be applied")))?;
        let filtered = if filtered.format() == surf.format() { filtered } else { filtered.convert(surf.format()) };
        let paint = Region::read(&filtered, rect);
        Ok(apply_coverage(surf, rect, &cov, stroke.brush.opacity, sel.as_ref(), lock, &paint, stroke.brush.mode))
    })?;
    Ok(damage_json(s, dmg))
}

/// Rasterize one string as a grayscale tile (1 = ink), cropped to its ink; `None` for a string
/// that leaves no ink (spaces).
fn text_cell(text: &str, font: &str, size_px: f32) -> Option<GrayTile> {
    let mut layer = redrob_photo_doc::TextLayer { text: text.to_string(), font_family: font.to_string(), size_pt: size_px, ..Default::default() };
    layer.color = redrob_photo_color::Color::BLACK;
    let (_, r) = redrob_photo_text::shared().lock().unwrap_or_else(|e| e.into_inner()).render(&layer, 72.0, redrob_photo_color::PixelFormat::GRAYA8);
    let rect = r.rect.intersect(&r.surface.content_bounds());
    if rect.is_empty() {
        return None;
    }
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let mut v = vec![0.0f32; w * h];
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let a = r.surface.rgba(rect.x0 + x as i32, rect.y0 + y as i32)[3];
            v[y * w + x] = a;
            if a > 1e-3 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (tw, th) = (x1 - x0, y1 - y0);
    let mut data = Vec::with_capacity(tw * th);
    for y in y0..y1 {
        data.extend((x0..x1).map(|x| v[y * w + x]));
    }
    Some(GrayTile::from_f32(tw as u32, th as u32, &data))
}

/// `brush.textTip`: a string in a font becomes the brush tip (whole word, or one character per
/// dab in pipe mode).
pub fn text_tip(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.textTip";
    let text = p.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).ok_or_else(|| bad(cmd, "missing `text`"))?;
    if text.chars().count() > 256 {
        return Err(bad(cmd, "`text` is longer than 256 characters"));
    }
    let font = p.get("font").and_then(Value::as_str).unwrap_or("");
    let size = p.get("size").and_then(Value::as_f64).map_or(s.tools.brush.size.max(4.0), |v| v as f32);
    if !size.is_finite() || !(4.0..=redrob_photo_paint::MAX_BRUSH_SIZE).contains(&size) {
        return Err(bad(cmd, "`size` must be 4..5000 px"));
    }
    let pipe = flag(p, "pipe", false);
    let cells: Vec<GrayTile> =
        if pipe { text.chars().filter_map(|c| text_cell(&c.to_string(), font, size)).collect() } else { text_cell(text, font, size).into_iter().collect() };
    if cells.is_empty() {
        return Err(bad(cmd, "the text leaves no ink (is the font installed?)"));
    }
    let (tip, hose) = redrob_photo_paint::engines::text_tip(cells, pipe);
    let (w, h) = match &tip {
        TipShape::Sampled(t) => (t.width, t.height),
        _ => (0, 0),
    };
    let b = &mut s.tools.brush;
    b.tip = tip;
    b.hose = hose;
    b.size = w.max(h).max(1) as f32;
    b.pressure_size = false;
    if let Some(sp) = p.get("spacing").and_then(Value::as_f64) {
        b.spacing = (sp as f32).clamp(0.01, 10.0);
    } else if pipe {
        // One character after the other: a dab every tip width.
        b.spacing = 1.0;
    }
    s.tools.current_preset = None;
    s.tools.brush_changed();
    Ok(json!({ "width": w, "height": h, "cells": s.tools.brush.hose.as_ref().map_or(1, |h| h.cells.len()) }))
}

/// `brush.mypaint.parse`: read a MyPaint-style brush file (its text or a path) into engine
/// settings for `paint.engineStroke`.
pub fn mypaint_parse(_: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.mypaint.parse";
    let text = match (p.get("text").and_then(Value::as_str), p.get("path").and_then(Value::as_str)) {
        (Some(t), _) => t.to_string(),
        (None, Some(path)) => std::fs::read_to_string(path).map_err(|e| EngineError::Other(format!("read `{path}`: {e}")))?,
        (None, None) => return Err(bad(cmd, "missing `text` (the brush file's contents) or `path`")),
    };
    let mut settings = MyPaintSettings::from_myb(&text).map_err(|e| bad(cmd, e))?;
    if settings.name.is_empty()
        && let Some(path) = p.get("path").and_then(Value::as_str)
    {
        settings.name = std::path::Path::new(path).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    }
    if let Some(v) = p.get("useBrushColor").and_then(Value::as_bool) {
        settings.use_brush_color = v;
    }
    let engine = serde_json::to_value(EngineSettings::MyPaint(settings)).map_err(|e| EngineError::Other(e.to_string()))?;
    Ok(json!({ "engine": engine }))
}

/// `brush.engines.list`: the engine kinds and their default settings.
pub fn engines_list(_: &mut Session, _: &Value) -> Result<Value> {
    let list: Vec<Value> = KINDS
        .iter()
        .filter_map(|k| EngineSettings::by_kind(k))
        .map(|e| json!({ "kind": e.kind(), "defaults": serde_json::to_value(&e).unwrap_or(Value::Null) }))
        .collect();
    Ok(json!({ "engines": list }))
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $en:expr, $run:expr, $journal:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: $en, run: $run, journal: $journal }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "paint.engineStroke",
            "Brush Engine Stroke",
            r##"{"points":[[x,y,pressure?,tiltX?,tiltY?,rotation?,timeMs?,wheel?],…],"engine":{"kind":"sketch|hatching|shape|grid|particle|curve|dyna|tangentNormal|myPaint|spray|bristle|deform|pixel",…that engine's settings (camelCase)} or "kind","brush":{…}?,"preset":name?,"size":px?,"opacity":0..1?,"color":"#rrggbb"?=foreground,"sampleAllLayers":bool=false (engines that read the canvas sample every visible layer),"seed":u64?,"target":"pixels"|"mask"|"quickMask"|{"channel":i}}"##,
            has_paintable,
            engine_stroke,
            true
        ),
        spec!(
            "paint.filterBrush",
            "Filter Brush",
            r##"{"points":[…],"filter":"filter.blur.gaussianBlur|filter.sharpen.unsharpMask|image.adjustments.levels|… (any pixel filter id)","filterParams":{…that filter's params}={},"brush":{…}?,"size":px?,"opacity":0..1?,"flow":0..1?,"sampleAllLayers":bool=false,"seed":u64?,"target":"pixels"|"mask"|"quickMask"|{"channel":i}}"##,
            has_paintable,
            filter_brush,
            true
        ),
        spec!(
            "brush.textTip",
            "Text as Brush Tip",
            r##"{"text":string,"font":family?=default,"size":4..5000 px=brush size,"pipe":bool=false (one character per dab along the stroke),"spacing":0.01..10?}"##,
            always,
            text_tip,
            true
        ),
        spec!(
            "brush.mypaint.parse",
            "Read MyPaint Brush",
            r##"{"text":string? (the .myb file's contents),"path":string? (a .myb file),"useBrushColor":bool=false (paint with the foreground instead of the file's colour)} → {"engine":{"kind":"myPaint",…}}"##,
            always,
            mypaint_parse,
            false
        ),
        spec!("brush.engines.list", "List Brush Engines", r##"{} → {"engines":[{"kind","defaults"}]}"##, always, engines_list, false),
    ]
}
