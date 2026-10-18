// SPDX-License-Identifier: MIT OR Apache-2.0
//! Colorize mask: flat-colours line art from a few key strokes. The mask is a pair of raster
//! layers above the line-art layer: a keys layer the user paints colour strokes on, and the
//! output layer this module fills by growing every key's colour through the regions the lines
//! enclose (a multi-source flood over the non-ink pixels, then under the lines themselves so no
//! gap stays). Lock keeps the output as it is; Show Output switches between keys and result.

use redrob_photo_color::PixelFormat;
use redrob_photo_doc::{Document, Layer, LayerContent, LayerId};
use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::layer_meta::{self, Key};
use crate::{EngineError, Result, Session};

/// Layer record on the output layer of a colorize mask.
pub const KEY: Key = *b"rCMk";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Colorize {
    /// The line-art layer.
    pub source: u64,
    /// The keys layer (colour strokes).
    pub keys: u64,
    /// Edits to the keys no longer change the output.
    pub locked: bool,
    /// Show the filled output (else the keys over the line art).
    pub show_output: bool,
    /// Ink threshold: how dark and opaque a pixel must be to count as a line (0..1).
    pub edge_detection: f32,
    /// How far colours grow under the lines, in pixels.
    pub clean_up: u32,
    /// Fill regions no key reaches with the nearest key's colour instead of leaving them clear.
    pub fill_all: bool,
}

impl Default for Colorize {
    fn default() -> Self {
        Colorize { source: 0, keys: 0, locked: false, show_output: true, edge_detection: 0.35, clean_up: 3, fill_all: false }
    }
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    crate::active_layer_of(s).map(|_| ())
}

/// The colorize record of the active layer, which may be the output or the keys layer.
fn find(doc: &Document, id: LayerId) -> Option<(LayerId, Colorize)> {
    if let Some(l) = doc.layer(id)
        && let Some(c) = layer_meta::get::<Colorize>(l, KEY)
    {
        return Some((id, c));
    }
    doc.walk().into_iter().find_map(|(_, _, l)| layer_meta::get::<Colorize>(l, KEY).filter(|c| c.keys == id.0 || c.source == id.0).map(|c| (l.id, c)))
}

fn has_colorize(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    find(&d.doc, id).map(|_| ()).ok_or_else(|| "select a colorize mask".into())
}

/// Layer › New › Colorize Mask on the line-art layer (default active).
fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.new.colorizeMask";
    let src = match p.get("source") {
        None | Some(Value::Null) => layer_param(s, &json!({}))?,
        Some(v) => crate::commands::explicit_layer(s, &json!({"layer": v}))?.ok_or_else(|| bad(cmd, "`source` is a layer id or name"))?,
    };
    let (out, keys) = s.edit("New Colorize Mask", |doc, active| {
        let base = doc.layer(src).ok_or(EngineError::NoLayer(src))?;
        let name = base.name.clone();
        let fmt = doc.pixel_format();
        let keys = Layer::raster(format!("{name} colorize keys"), fmt);
        let kid = doc.insert_above(Some(src), keys);
        let mut out = Layer::raster(format!("{name} colorize"), fmt);
        out.blend = redrob_photo_color::BlendMode::Multiply;
        let mut rec = Colorize { source: src.0, keys: kid.0, ..Default::default() };
        if let Some(e) = p.get("edgeDetection").and_then(Value::as_f64) {
            rec.edge_detection = e.clamp(0.0, 1.0) as f32;
        }
        if let Some(c) = p.get("cleanUp").and_then(Value::as_u64) {
            rec.clean_up = c.min(64) as u32;
        }
        layer_meta::set(&mut out, KEY, &rec);
        // The output sits under the keys so strokes stay visible while keying; it starts hidden
        // until the first update.
        out.visible = false;
        let oid = doc.insert_above(Some(src), out);
        *active = Some(kid);
        Ok((oid, kid))
    })?;
    Ok(json!({"output": out.0, "keys": keys.0, "source": src.0}))
}

/// Luminance (0..1) and alpha of a surface over `canvas`.
fn read_rgba(s: &Surface, canvas: Rect) -> Vec<[f32; 4]> {
    let mut px = vec![[0.0f32; 4]; (canvas.width() * canvas.height()) as usize];
    s.read_rgba_into(canvas, &mut px);
    px
}

/// Fill the regions: returns the output pixels.
fn fill(ink: &[bool], keys: &[[f32; 4]], w: usize, h: usize, clean_up: u32, fill_all: bool) -> Vec<[f32; 4]> {
    let n = w * h;
    // Distinct key colours, quantised to 8 bits.
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut label = vec![u16::MAX; n];
    let mut queue = std::collections::VecDeque::new();
    for i in 0..n {
        let k = keys[i];
        if k[3] < 0.5 {
            continue;
        }
        let c = [(k[0] * 255.0).round() as u8, (k[1] * 255.0).round() as u8, (k[2] * 255.0).round() as u8];
        let li = match palette.iter().position(|p| *p == c) {
            Some(i) => i,
            None => {
                if palette.len() >= u16::MAX as usize - 1 {
                    continue;
                }
                palette.push(c);
                palette.len() - 1
            }
        };
        label[i] = li as u16;
        queue.push_back(i);
    }
    // Grow through the open (non-ink) pixels: each pixel takes the first colour to reach it.
    let neighbours = |i: usize| {
        let (x, y) = (i % w, i / w);
        let mut out = [usize::MAX; 4];
        if x > 0 {
            out[0] = i - 1;
        }
        if x + 1 < w {
            out[1] = i + 1;
        }
        if y > 0 {
            out[2] = i - w;
        }
        if y + 1 < h {
            out[3] = i + w;
        }
        out
    };
    let mut second = Vec::new();
    while let Some(i) = queue.pop_front() {
        let li = label[i];
        for j in neighbours(i) {
            if j == usize::MAX || label[j] != u16::MAX {
                continue;
            }
            if ink[j] {
                second.push((j, li));
                continue;
            }
            label[j] = li;
            queue.push_back(j);
        }
    }
    // Then under the lines, a few pixels deep, so the colour meets under every stroke.
    let mut depth = vec![0u32; n];
    for (j, li) in second {
        if label[j] == u16::MAX {
            label[j] = li;
            depth[j] = 1;
            queue.push_back(j);
        }
    }
    while let Some(i) = queue.pop_front() {
        if depth[i] >= clean_up.max(1) {
            continue;
        }
        let li = label[i];
        for j in neighbours(i) {
            if j != usize::MAX && label[j] == u16::MAX {
                label[j] = li;
                depth[j] = depth[i] + 1;
                queue.push_back(j);
            }
        }
    }
    if fill_all && !palette.is_empty() {
        // Anything still unreached (regions with no key, pixels beyond the clean-up depth) takes
        // the colour of the nearest labelled pixel.
        for (i, l) in label.iter().enumerate().take(n) {
            if *l != u16::MAX {
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            let li = label[i];
            for j in neighbours(i) {
                if j != usize::MAX && label[j] == u16::MAX {
                    label[j] = li;
                    queue.push_back(j);
                }
            }
        }
    }
    label
        .iter()
        .map(|l| match palette.get(*l as usize) {
            Some(c) => [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0],
            None => [0.0; 4],
        })
        .collect()
}

/// Recompute the output from the line art and the keys.
pub fn update_layer(doc: &mut Document, out_id: LayerId) -> Result<()> {
    let canvas = doc.bounds();
    let fmt = doc.pixel_format();
    let rec: Colorize = doc.layer(out_id).and_then(|l| layer_meta::get(l, KEY)).ok_or_else(|| EngineError::Other("not a colorize mask".into()))?;
    let src = doc.layer(LayerId(rec.source)).ok_or(EngineError::NoLayer(LayerId(rec.source)))?;
    let keys = doc.layer(LayerId(rec.keys)).ok_or(EngineError::NoLayer(LayerId(rec.keys)))?;
    let (w, h) = (canvas.width() as usize, canvas.height() as usize);
    let mut shown = src.clone();
    shown.visible = true;
    let line = read_rgba(&crate::live_layers::render_layer(&shown, canvas, PixelFormat::RGBA32F), canvas);
    let key_px = match &keys.content {
        LayerContent::Raster(s) => read_rgba(s, canvas),
        _ => read_rgba(&crate::live_layers::render_layer(keys, canvas, PixelFormat::RGBA32F), canvas),
    };
    let threshold = rec.edge_detection.clamp(0.0, 1.0);
    let ink: Vec<bool> = line
        .iter()
        .map(|p| {
            let lum = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
            p[3] * (1.0 - lum) > threshold
        })
        .collect();
    let out = fill(&ink, &key_px, w, h, rec.clean_up, rec.fill_all);
    let surface = crate::live_layers::buffer_to_surface(&redrob_photo_compose::Buffer { rect: canvas, px: out }, fmt);
    let l = doc.layer_mut(out_id).ok_or(EngineError::NoLayer(out_id))?;
    l.content = LayerContent::Raster(surface);
    Ok(())
}

fn target(s: &Session, p: &Value) -> Result<(LayerId, Colorize)> {
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    find(&st.doc, id).ok_or_else(|| bad("layer.colorizeMask", "the layer is not part of a colorize mask"))
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let (out, rec) = target(s, p)?;
    if rec.locked && !p.get("force").and_then(Value::as_bool).unwrap_or(false) {
        return Err(EngineError::Other("the colorize mask is locked".into()));
    }
    s.edit("Update Colorize Mask", |doc, _| {
        update_layer(doc, out)?;
        if rec.show_output {
            doc.layer_mut(out).ok_or(EngineError::NoLayer(out))?.visible = true;
        }
        Ok(())
    })?;
    Ok(json!({"output": out.0}))
}

fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    let (out, _) = target(s, p)?;
    let rec = s.edit("Colorize Mask Properties", |doc, _| {
        let l = doc.layer_mut(out).ok_or(EngineError::NoLayer(out))?;
        let mut rec: Colorize = layer_meta::get(l, KEY).unwrap_or_default();
        if let Some(v) = p.get("locked").and_then(Value::as_bool) {
            rec.locked = v;
        }
        if let Some(v) = p.get("showOutput").and_then(Value::as_bool) {
            rec.show_output = v;
        }
        if let Some(v) = p.get("edgeDetection").and_then(Value::as_f64) {
            rec.edge_detection = v.clamp(0.0, 1.0) as f32;
        }
        if let Some(v) = p.get("cleanUp").and_then(Value::as_u64) {
            rec.clean_up = v.min(64) as u32;
        }
        if let Some(v) = p.get("fillAll").and_then(Value::as_bool) {
            rec.fill_all = v;
        }
        layer_meta::set(l, KEY, &rec);
        l.visible = rec.show_output;
        let keys = LayerId(rec.keys);
        if let Some(k) = doc.layer_mut(keys) {
            k.visible = !rec.show_output;
        }
        Ok(rec)
    })?;
    Ok(json!({"output": out.0, "colorizeMask": rec}))
}

/// Show Output: the result instead of the keys (toggle).
fn show_output(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, rec) = target(s, p)?;
    let show = p.get("show").and_then(Value::as_bool).unwrap_or(!rec.show_output);
    let mut q = p.clone();
    q["showOutput"] = json!(show);
    set_props(s, &q)
}

/// Bake the output into a plain paint layer and drop the keys.
fn convert_to_paint(s: &mut Session, p: &Value) -> Result<Value> {
    let (out, rec) = target(s, p)?;
    s.edit("Convert Colorize Mask to Paint Layer", |doc, active| {
        if !rec.locked {
            update_layer(doc, out)?;
        }
        doc.remove(LayerId(rec.keys));
        let l = doc.layer_mut(out).ok_or(EngineError::NoLayer(out))?;
        layer_meta::remove(l, KEY);
        l.visible = true;
        *active = Some(out);
        Ok(())
    })?;
    Ok(json!({"layer": out.0}))
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let (out, rec) = target(s, p)?;
    Ok(json!({"output": out.0, "colorizeMask": rec}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $params:expr, $en:expr, $journal:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $en, journal: $journal, run: $run }
        };
    }
    const CM: &[&str] = &["Layer", "Colorize Mask"];
    vec![
        spec!(
            "layer.new.colorizeMask",
            "Colorize Mask",
            &["Layer", "New"],
            r##"{"source":id|name? (the line art, default active),"edgeDetection":0..1=0.35,"cleanUp":px=3} → {output,keys}; paint colour strokes on the keys layer, then Update"##,
            has_layer,
            true,
            create
        ),
        spec!("layer.colorizeMask.update", "Update", CM, r##"{"layer":id?,"force":bool? (ignore the lock)}"##, has_colorize, true, update),
        spec!("layer.colorizeMask.showOutput", "Show Output", CM, r##"{"layer":id?,"show":bool? (default toggle)}"##, has_colorize, true, show_output),
        spec!("layer.colorizeMask.lock", "Lock", CM, r##"{"layer":id?,"locked":bool}"##, has_colorize, true, |s, p| {
            let mut q = p.clone();
            if q.get("locked").is_none() {
                let (_, rec) = target(s, p)?;
                q["locked"] = json!(!rec.locked);
            }
            set_props(s, &q)
        }),
        spec!(
            "layer.colorizeMask.set",
            "Colorize Mask Properties…",
            CM,
            r##"{"layer":id?,"locked":bool?,"showOutput":bool?,"edgeDetection":0..1?,"cleanUp":px?,"fillAll":bool?}"##,
            has_colorize,
            true,
            set_props
        ),
        spec!("layer.colorizeMask.toPaintLayer", "Convert to Paint Layer", CM, r##"{"layer":id?}"##, has_colorize, true, convert_to_paint),
        spec!("layer.colorizeMask.info", "Colorize Mask Info", &[], r##"{"layer":id?} → {output,colorizeMask}"##, has_colorize, false, info),
    ]
}
