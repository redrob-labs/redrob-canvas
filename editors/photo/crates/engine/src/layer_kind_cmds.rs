// SPDX-License-Identifier: MIT OR Apache-2.0
//! Layer kinds and conversions: file layers (a linked image with a scaling mode), converting
//! layers between paint layer, file layer, transparency / filter / selection mask and colour
//! space, importing a file as a layer or as a mask, Split Alpha, local selection masks, and
//! selecting layers by state.

use redrob_photo_color::{ColorMode, PixelFormat, SampleType};
use redrob_photo_doc::{Affine, Document, Layer, LayerContent, LayerId, LayerMask, SmartObject, SmartSource};
use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::layer_meta::{self, Key};
use crate::{EngineError, Result, Session};

/// Layer record of a file layer's scaling mode.
pub const FILE_LAYER: Key = *b"rFLy";
/// Layer record of the local selection masks.
pub const LOCAL_SELECTIONS: Key = *b"rLSe";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    crate::active_layer_of(s).map(|_| ())
}

fn has_raster(s: &Session) -> std::result::Result<(), String> {
    let l = crate::active_layer_of(s)?;
    if matches!(l.content, LayerContent::Raster(_)) { Ok(()) } else { Err("select a pixel layer".into()) }
}

fn has_raster_mask(s: &Session) -> std::result::Result<(), String> {
    let l = crate::active_layer_of(s)?;
    if matches!(l.content, LayerContent::Raster(_)) && l.mask.is_some() { Ok(()) } else { Err("select a pixel layer with a layer mask".into()) }
}

fn has_file_layer(s: &Session) -> std::result::Result<(), String> {
    let l = crate::active_layer_of(s)?;
    if layer_meta::has(l, FILE_LAYER) { Ok(()) } else { Err("select a file layer".into()) }
}

// ---------- alpha / gray helpers ----------

/// The alpha of a raster surface as a grayscale mask surface at `depth`.
pub(crate) fn alpha_to_gray(s: &Surface, depth: SampleType) -> Surface {
    let fmt = s.format();
    let area = s.content_bounds();
    let mut out = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, depth, false), &[0.0]);
    if area.is_empty() || !fmt.alpha {
        if !fmt.alpha && !area.is_empty() {
            out.fill_rect(area, &[1.0]);
        }
        return out;
    }
    let n = fmt.channels();
    let px = s.read_region(area);
    let a: Vec<f32> = px.chunks_exact(n).map(|p| p[n - 1]).collect();
    out.write_region(area, &a);
    out.prune();
    out
}

/// The layer's alpha as a mask (white = opaque).
fn alpha_mask(l: &Layer, canvas: Rect, depth: SampleType) -> LayerMask {
    let surface = match &l.content {
        LayerContent::Raster(s) => alpha_to_gray(s, depth),
        _ => alpha_to_gray(&crate::live_layers::render_layer(l, canvas, PixelFormat::new(ColorMode::Rgb, depth, true)), depth),
    };
    LayerMask { surface, ..LayerMask::reveal_all() }
}

/// Write `gray` (0..1) into the alpha channel of `s` over `area`.
fn set_alpha(s: &mut Surface, gray: &Surface, area: Rect) {
    let fmt = s.format();
    if !fmt.alpha || area.is_empty() {
        return;
    }
    let n = fmt.channels();
    let mut px = s.read_region(area);
    let g = gray.read_region(area);
    for (p, a) in px.chunks_exact_mut(n).zip(g.chunks_exact(gray.channels().max(1))) {
        p[n - 1] = a[0];
    }
    s.write_region(area, &px);
    s.prune();
}

/// The sibling directly below `id`, if any.
fn below(doc: &Document, id: LayerId) -> Option<LayerId> {
    let path = doc.path_of(id)?;
    let (&idx, parent) = path.split_last()?;
    let list: &[Layer] = if parent.is_empty() { &doc.layers } else { doc.layer_at(parent)?.children()? };
    list.get(idx.checked_sub(1)?).map(|l| l.id)
}

// ---------- file layers ----------

/// How a file layer is scaled when its file is read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileScaling {
    /// Shown at the file's own pixel size.
    #[default]
    None,
    /// Scaled to the image's size (non-uniformly).
    ToImageSize,
    /// Scaled by the ratio of the image's resolution to the file's.
    ToImageResolution,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileLayer {
    pub scaling: FileScaling,
    /// Resampling filter for the scaled image: `bicubic`, `bilinear`, `nearest`, `lanczos`.
    #[serde(default)]
    pub filter: String,
    /// The file's size when last read, so a changed file re-fits.
    #[serde(default)]
    pub source_size: [u32; 2],
    #[serde(default)]
    pub source_dpi: f32,
}

fn scaling_param(p: &Value, cmd: &str) -> Result<Option<FileScaling>> {
    Ok(match p.get("scaling").and_then(Value::as_str) {
        None => None,
        Some("none") => Some(FileScaling::None),
        Some("toImageSize") | Some("image") => Some(FileScaling::ToImageSize),
        Some("toImageResolution") | Some("resolution") => Some(FileScaling::ToImageResolution),
        Some(o) => return Err(bad(cmd, format!("unknown scaling `{o}` (none, toImageSize, toImageResolution)"))),
    })
}

/// The transform that places a `sw`×`sh` source at `src_dpi` on `doc` under `scaling`.
fn file_transform(doc: &Document, sw: u32, sh: u32, src_dpi: f32, scaling: FileScaling) -> Affine {
    let (cw, ch) = (f64::from(doc.size.width), f64::from(doc.size.height));
    let (sw, sh) = (f64::from(sw.max(1)), f64::from(sh.max(1)));
    match scaling {
        FileScaling::None => Affine::IDENTITY,
        FileScaling::ToImageSize => Affine { m: [cw / sw, 0.0, 0.0, ch / sh, 0.0, 0.0] },
        FileScaling::ToImageResolution => {
            let k = if src_dpi > 0.0 { f64::from(doc.resolution_dpi) / f64::from(src_dpi) } else { 1.0 };
            Affine { m: [k, 0.0, 0.0, k, 0.0, 0.0] }
        }
    }
}

/// Re-fit a file layer from its file's current size and resolution.
fn fit_file_layer(doc: &mut Document, id: LayerId) -> Result<bool> {
    let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let Some(mut rec) = layer_meta::get::<FileLayer>(l, FILE_LAYER) else { return Ok(false) };
    let LayerContent::Smart(sm) = &l.content else { return Ok(false) };
    let SmartSource::Linked { path } = &sm.source else { return Ok(false) };
    let Ok(bytes) = crate::file_cmds::read_file(path) else { return Ok(false) };
    let Ok(imported) = redrob_photo_io::import(path, &bytes) else { return Ok(false) };
    let (sw, sh, dpi) = (imported.document.size.width, imported.document.size.height, imported.document.resolution_dpi);
    if rec.source_size == [sw, sh] && (rec.source_dpi - dpi).abs() < 1e-3 {
        return Ok(false);
    }
    rec.source_size = [sw, sh];
    rec.source_dpi = dpi;
    let t = file_transform(doc, sw, sh, dpi, rec.scaling);
    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    if let LayerContent::Smart(sm) = &mut l.content {
        sm.transform = t;
        sm.perspective = None;
    }
    layer_meta::set(l, FILE_LAYER, &rec);
    crate::smart_cmds::refresh(doc, id)?;
    Ok(true)
}

/// Layer › New › File Layer: a linked image that reloads when the file changes, scaled by mode.
fn new_file_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.new.fileLayer";
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad(cmd, "needs `path`"))?.to_string();
    let scaling = scaling_param(p, cmd)?.unwrap_or_default();
    let filter = p.get("filter").and_then(Value::as_str).unwrap_or("bicubic").to_string();
    let bytes = crate::file_cmds::read_file(&path)?;
    let imported = redrob_photo_io::import(&path, &bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let (sw, sh, dpi) = (imported.document.size.width, imported.document.size.height, imported.document.resolution_dpi);
    let name = p.get("name").and_then(Value::as_str).map_or_else(|| crate::file_cmds::stem(&path), str::to_string);
    let id = s.edit("New File Layer", |doc, active| {
        let t = file_transform(doc, sw, sh, dpi, scaling);
        let so = SmartObject::new(SmartSource::Linked { path: path.clone() }, t, None);
        let mut l = Layer::new(name.clone(), LayerContent::Smart(so));
        layer_meta::set(&mut l, FILE_LAYER, &FileLayer { scaling, filter: filter.clone(), source_size: [sw, sh], source_dpi: dpi });
        let id = doc.insert_above(*active, l);
        crate::smart_cmds::refresh(doc, id)?;
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "path": path, "scaling": scaling, "size": [sw, sh]}))
}

/// Change a file layer's scaling mode or filter (re-fits it), or its file.
fn set_file_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.fileLayer.set";
    let id = layer_param(s, p)?;
    let scaling = scaling_param(p, cmd)?;
    let filter = p.get("filter").and_then(Value::as_str).map(str::to_string);
    let path = p.get("path").and_then(Value::as_str).map(str::to_string);
    let rec = s.edit("File Layer Properties", |doc, _| {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let mut rec: FileLayer = layer_meta::get(l, FILE_LAYER).ok_or_else(|| bad(cmd, "the layer is not a file layer"))?;
        if let Some(sc) = scaling {
            rec.scaling = sc;
        }
        if let Some(f) = &filter {
            rec.filter = f.clone();
        }
        if let (Some(path), LayerContent::Smart(sm)) = (&path, &mut l.content) {
            sm.source = SmartSource::Linked { path: path.clone() };
        }
        rec.source_size = [0, 0];
        layer_meta::set(l, FILE_LAYER, &rec);
        fit_file_layer(doc, id)?;
        Ok(rec)
    })?;
    Ok(json!({"layer": id.0, "fileLayer": rec}))
}

/// Re-read every file layer whose file changed size or resolution (the linked-file watcher
/// re-renders changed pixels; this re-fits the placement).
pub fn refresh_file_layers(doc: &mut Document) {
    let ids: Vec<(LayerId, FileLayer)> = layer_meta::layers_with(doc, FILE_LAYER);
    for (id, _) in ids {
        let _ = fit_file_layer(doc, id);
    }
}

fn reload_file_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let changed = s.edit("Reload File Layer", |doc, _| {
        if let Some(l) = doc.layer_mut(id)
            && let Some(mut rec) = layer_meta::get::<FileLayer>(l, FILE_LAYER)
        {
            rec.source_size = [0, 0];
            layer_meta::set(l, FILE_LAYER, &rec);
        }
        fit_file_layer(doc, id)?;
        crate::smart_cmds::refresh(doc, id)
    })?;
    Ok(json!({"layer": id.0, "reloaded": changed}))
}

// ---------- conversions ----------

/// Layer › Convert › to File Layer: the layer's pixels are saved to `path` (PNG) and the layer
/// becomes a file layer linked to it.
fn to_file_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.convert.toFileLayer";
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let path = p.get("path").and_then(Value::as_str).map_or_else(|| format!("{}.png", l.name.replace(['/', '\\'], "_")), str::to_string);
    let canvas = st.doc.bounds();
    let fmt = st.doc.pixel_format();
    let surface = crate::live_layers::render_layer(l, canvas, fmt);
    let mut out = Document::new(l.name.clone(), st.doc.size, st.doc.mode, st.doc.depth);
    out.resolution_dpi = st.doc.resolution_dpi;
    out.layers.push(Layer::new(l.name.clone(), LayerContent::Raster(surface)));
    let bytes =
        redrob_photo_io::export(&out, "png", &redrob_photo_io::ExportOptions::default()).map(|r| r.bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    crate::file_cmds::write_file(&path, &bytes)?;
    let (sw, sh, dpi) = (out.size.width, out.size.height, out.resolution_dpi);
    let scaling = scaling_param(p, cmd)?.unwrap_or_default();
    s.edit("Convert to File Layer", |doc, _| {
        let t = file_transform(doc, sw, sh, dpi, scaling);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        l.content = LayerContent::Smart(SmartObject::new(SmartSource::Linked { path: path.clone() }, t, None));
        l.video = None;
        layer_meta::set(l, FILE_LAYER, &FileLayer { scaling, filter: "bicubic".into(), source_size: [sw, sh], source_dpi: dpi });
        crate::smart_cmds::refresh(doc, id)?;
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "path": path}))
}

/// Layer › Convert › to Paint Layer: any layer kind to pixels (its composite appearance).
fn to_paint_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    s.edit("Convert to Paint Layer", |doc, _| {
        let canvas = doc.bounds();
        let fmt = doc.pixel_format();
        let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let pixels = match &l.content {
            LayerContent::Raster(s) if !crate::live_layers::is_live(l) => s.clone(),
            _ => {
                let mut flat = l.clone();
                flat.mask = None;
                flat.vector_mask = None;
                flat.effects = Default::default();
                flat.opacity = 1.0;
                flat.fill_opacity = 1.0;
                flat.blend = redrob_photo_color::BlendMode::Normal;
                crate::live_layers::render_layer(&flat, canvas, fmt)
            }
        };
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        l.content = LayerContent::Raster(pixels);
        l.video = None;
        l.psd_blocks.clear();
        l.fill_cache = None;
        Ok(())
    })?;
    Ok(json!({"layer": id.0}))
}

/// Layer › Convert › to Transparency Mask: the layer's alpha becomes the layer mask of the layer
/// below it, and the layer is removed.
fn to_transparency_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.convert.toTransparencyMask";
    let id = layer_param(s, p)?;
    let target = s.edit("Convert to Transparency Mask", |doc, active| {
        let target = below(doc, id).ok_or_else(|| bad(cmd, "there is no layer below to mask"))?;
        let canvas = doc.bounds();
        let depth = doc.depth;
        let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let mask = alpha_mask(l, canvas, depth);
        doc.remove(id);
        doc.layer_mut(target).ok_or(EngineError::NoLayer(target))?.mask = Some(mask);
        *active = Some(target);
        Ok(target)
    })?;
    Ok(json!({"layer": target.0}))
}

/// Layer › Convert › to Filter Mask: the layer's alpha limits `filter` over the layer below.
fn to_filter_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.convert.toFilterMask";
    let id = layer_param(s, p)?;
    let (target, mask) = {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let target = below(&st.doc, id).ok_or_else(|| bad(cmd, "there is no layer below to filter"))?;
        let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        (target, alpha_mask(l, st.doc.bounds(), st.doc.depth))
    };
    let mut q = p.clone();
    q["source"] = json!(target.0);
    if let Some(o) = q.as_object_mut() {
        o.remove("layer");
    }
    let r = crate::live_layers::new_filter_mask(s, &q)?;
    let new_id = r.get("layer").and_then(Value::as_u64).map(LayerId).ok_or_else(|| bad(cmd, "no filter mask created"))?;
    s.edit("Convert to Filter Mask", |doc, active| {
        doc.remove(id);
        doc.layer_mut(new_id).ok_or(EngineError::NoLayer(new_id))?.mask = Some(mask.clone());
        *active = Some(new_id);
        Ok(())
    })?;
    Ok(json!({"layer": new_id.0, "source": target.0}))
}

/// Layer › Convert › to Selection Mask: the layer's alpha becomes a local selection of the
/// layer below.
fn to_selection_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.convert.toSelectionMask";
    let id = layer_param(s, p)?;
    let (target, mask, name) = {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let target = below(&st.doc, id).ok_or_else(|| bad(cmd, "there is no layer below"))?;
        let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        (target, alpha_mask(l, st.doc.bounds(), st.doc.depth), l.name.clone())
    };
    s.edit("Convert to Selection Mask", |doc, active| {
        doc.remove(id);
        add_local_selection(doc, target, &name, mask.surface.clone())?;
        *active = Some(target);
        Ok(())
    })?;
    Ok(json!({"layer": target.0, "selection": name}))
}

/// Layer › Convert › Layer Color Space: one pixel layer's own colour model and depth.
fn to_color_space(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.convert.colorSpace";
    let id = layer_param(s, p)?;
    let mode = match p.get("mode").and_then(Value::as_str) {
        None => None,
        Some("rgb") => Some(ColorMode::Rgb),
        Some("gray") | Some("grayscale") => Some(ColorMode::Grayscale),
        Some("cmyk") => Some(ColorMode::Cmyk),
        Some("lab") => Some(ColorMode::Lab),
        Some(o) => return Err(bad(cmd, format!("unknown mode `{o}` (rgb, gray, cmyk, lab)"))),
    };
    let depth = match p.get("depth").and_then(Value::as_u64) {
        None => None,
        Some(8) => Some(SampleType::U8),
        Some(16) => Some(SampleType::U16),
        Some(32) => Some(SampleType::F32),
        Some(o) => return Err(bad(cmd, format!("depth is 8, 16 or 32, not {o}"))),
    };
    let fmt = s.edit("Layer Color Space", |doc, _| {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let s = l.surface_mut().ok_or_else(|| bad(cmd, "select a pixel layer"))?;
        let cur = s.format();
        let to = PixelFormat::new(mode.unwrap_or(cur.mode), depth.unwrap_or(cur.sample), true);
        if to != cur {
            *s = s.try_convert(to)?;
        }
        if let Some(m) = l.mask.as_mut()
            && let Some(d) = depth
        {
            let mf = m.surface.format();
            m.surface = m.surface.try_convert(PixelFormat::new(mf.mode, d, mf.alpha))?;
        }
        Ok(to)
    })?;
    Ok(json!({"layer": id.0, "mode": format!("{:?}", fmt.mode), "depth": format!("{:?}", fmt.sample)}))
}

// ---------- import as ----------

/// Layer › Import › Image as Layer / Transparency Mask / Filter Mask / Selection Mask.
fn import_as(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.importAs";
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad(cmd, "needs `path`"))?.to_string();
    let as_ = p.get("as").and_then(Value::as_str).unwrap_or("layer").to_string();
    let bytes = crate::file_cmds::read_file(&path)?;
    let imported = redrob_photo_io::import(&path, &bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let fmt = st.doc.pixel_format();
    let depth = st.doc.depth;
    let pixels = crate::file_cmds::flattened(&imported.document, fmt);
    let name = crate::file_cmds::stem(&path);
    match as_.as_str() {
        "layer" => {
            let id = s.edit("Import Image as Layer", |doc, active| {
                let id = doc.insert_above(*active, Layer::new(name.clone(), LayerContent::Raster(pixels.clone())));
                *active = Some(id);
                Ok(id)
            })?;
            Ok(json!({"layer": id.0, "as": "layer"}))
        }
        "transparencyMask" | "filterMask" | "selectionMask" => {
            // A mask reads the image's luminosity (an opaque image) or its alpha (a cut-out).
            let gray = luminosity_or_alpha(&pixels, depth);
            let id = layer_param(s, p)?;
            match as_.as_str() {
                "transparencyMask" => {
                    s.edit("Import Image as Transparency Mask", |doc, _| {
                        doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.mask = Some(LayerMask { surface: gray.clone(), ..LayerMask::reveal_all() });
                        Ok(())
                    })?;
                    Ok(json!({"layer": id.0, "as": as_}))
                }
                "selectionMask" => {
                    s.edit("Import Image as Selection Mask", |doc, _| add_local_selection(doc, id, &name, gray.clone()).map(|_| ()))?;
                    Ok(json!({"layer": id.0, "as": as_, "selection": name}))
                }
                _ => {
                    let mut q = p.clone();
                    q["source"] = json!(id.0);
                    if let Some(o) = q.as_object_mut() {
                        o.remove("layer");
                    }
                    if q.get("filter").is_none() {
                        q["filter"] = json!("filter.blur.gaussian");
                    }
                    let r = crate::live_layers::new_filter_mask(s, &q)?;
                    let fid = r.get("layer").and_then(Value::as_u64).map(LayerId).ok_or_else(|| bad(cmd, "no filter mask created"))?;
                    s.edit("Import Image as Filter Mask", |doc, _| {
                        doc.layer_mut(fid).ok_or(EngineError::NoLayer(fid))?.mask = Some(LayerMask { surface: gray.clone(), ..LayerMask::reveal_all() });
                        Ok(())
                    })?;
                    Ok(json!({"layer": fid.0, "as": as_, "source": id.0}))
                }
            }
        }
        o => Err(bad(cmd, format!("unknown `as` `{o}` (layer, transparencyMask, filterMask, selectionMask)"))),
    }
}

/// A mask from an imported image: its alpha when it has transparent pixels, else its luminosity.
fn luminosity_or_alpha(s: &Surface, depth: SampleType) -> Surface {
    let area = s.content_bounds();
    let mut out = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, depth, false), &[0.0]);
    if area.is_empty() {
        return out;
    }
    let n = (area.width() * area.height()) as usize;
    let mut px = vec![[0.0f32; 4]; n];
    s.read_rgba_into(area, &mut px);
    let has_alpha = px.iter().any(|p| p[3] < 0.999);
    let g: Vec<f32> = px.iter().map(|p| if has_alpha { p[3] } else { 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2] }).collect();
    out.write_region(area, &g);
    out.prune();
    out
}

// ---------- split alpha ----------

/// Layer › Split Alpha › Alpha into Mask: the alpha becomes the layer mask, the pixels opaque.
fn alpha_into_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    s.edit("Alpha into Mask", |doc, _| {
        let depth = doc.depth;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("select a pixel layer".into()))?;
        let gray = alpha_to_gray(surf, depth);
        let area = surf.content_bounds();
        let fmt = surf.format();
        if fmt.alpha && !area.is_empty() {
            let n = fmt.channels();
            let mut px = surf.read_region(area);
            for q in px.chunks_exact_mut(n) {
                q[n - 1] = 1.0;
            }
            surf.write_region(area, &px);
        }
        l.mask = Some(LayerMask { surface: gray, ..LayerMask::reveal_all() });
        Ok(())
    })?;
    Ok(json!({"layer": id.0}))
}

/// Layer › Split Alpha › Write as Alpha: the mask becomes the layer's alpha; the mask is removed.
fn mask_into_alpha(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    s.edit("Write Mask as Alpha", |doc, _| {
        let canvas = doc.bounds();
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let mask = l.mask.take().ok_or_else(|| EngineError::Other("the layer has no layer mask".into()))?;
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("select a pixel layer".into()))?;
        let area = surf.content_bounds().union(&mask.surface.content_bounds()).union(&canvas);
        set_alpha(surf, &mask.surface, area);
        Ok(())
    })?;
    Ok(json!({"layer": id.0}))
}

/// Layer › Split Alpha › Save Merged: the layer with its mask written as alpha, saved to a file
/// (PNG keeps the colour under fully transparent pixels).
fn save_merged(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.splitAlpha.saveMerged";
    let id = layer_param(s, p)?;
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad(cmd, "needs `path`"))?.to_string();
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Raster(src) = &l.content else { return Err(bad(cmd, "select a pixel layer")) };
    let mut surf = src.clone();
    if let Some(m) = &l.mask {
        let area = surf.content_bounds().union(&m.surface.content_bounds()).union(&st.doc.bounds());
        set_alpha(&mut surf, &m.surface, area);
    }
    let mut out = Document::new(l.name.clone(), st.doc.size, st.doc.mode, st.doc.depth);
    out.resolution_dpi = st.doc.resolution_dpi;
    out.layers.push(Layer::new(l.name.clone(), LayerContent::Raster(surf)));
    let ext = std::path::Path::new(&path).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into());
    let bytes =
        redrob_photo_io::export(&out, &ext, &redrob_photo_io::ExportOptions::default()).map(|r| r.bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    crate::file_cmds::write_file(&path, &bytes)?;
    Ok(json!({"layer": id.0, "file": path}))
}

// ---------- local selection masks ----------

/// Local selections of a layer: alpha channels named after it, one active.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSelections {
    pub channels: Vec<String>,
    #[serde(default)]
    pub active: Option<usize>,
}

fn add_local_selection(doc: &mut Document, id: LayerId, name: &str, surface: Surface) -> Result<String> {
    let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let mut rec: LocalSelections = layer_meta::get(l, LOCAL_SELECTIONS).unwrap_or_default();
    let mut channel = format!("{}: {name}", l.name);
    let mut n = 2;
    while doc.channels.iter().any(|c| c.name == channel) {
        channel = format!("{}: {name} {n}", l.name);
        n += 1;
    }
    doc.channels.push(redrob_photo_doc::AlphaChannel::new(channel.clone(), surface));
    rec.channels.push(channel.clone());
    rec.active = Some(rec.channels.len() - 1);
    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    layer_meta::set(l, LOCAL_SELECTIONS, &rec);
    Ok(channel)
}

/// The current selection stored as a local selection mask of the layer.
fn local_add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.localSelection.add";
    let id = layer_param(s, p)?;
    let name = p.get("name").and_then(Value::as_str).unwrap_or("Selection").to_string();
    let channel = s.edit("Add Local Selection", |doc, _| {
        let depth = doc.depth;
        let surface = match &doc.selection {
            Some(sel) => {
                let f = sel.format();
                sel.try_convert(PixelFormat::new(f.mode, depth, f.alpha))?
            }
            None => return Err(bad(cmd, "make a selection first")),
        };
        add_local_selection(doc, id, &name, surface)
    })?;
    Ok(json!({"layer": id.0, "channel": channel}))
}

fn local_rec(s: &Session, id: LayerId) -> Result<LocalSelections> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    Ok(layer_meta::get(l, LOCAL_SELECTIONS).unwrap_or_default())
}

/// Activate a local selection: it becomes the active selection of the image.
fn local_activate(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.localSelection.activate";
    let id = layer_param(s, p)?;
    let rec = local_rec(s, id)?;
    let index = p.get("index").and_then(Value::as_u64).map(|i| i as usize).or(rec.active).ok_or_else(|| bad(cmd, "needs `index`"))?;
    let channel = rec.channels.get(index).cloned().ok_or_else(|| bad(cmd, format!("no local selection {index}")))?;
    s.edit("Activate Local Selection", |doc, _| {
        let ch = doc.channels.iter().find(|c| c.name == channel).ok_or_else(|| bad(cmd, format!("channel \"{channel}\" is gone")))?;
        doc.selection = Some(ch.surface.clone());
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let mut rec: LocalSelections = layer_meta::get(l, LOCAL_SELECTIONS).unwrap_or_default();
        rec.active = Some(index);
        layer_meta::set(l, LOCAL_SELECTIONS, &rec);
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "active": index, "channel": channel}))
}

/// Store the current selection back into the active local selection.
fn local_update(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.localSelection.update";
    let id = layer_param(s, p)?;
    let rec = local_rec(s, id)?;
    let index = p.get("index").and_then(Value::as_u64).map(|i| i as usize).or(rec.active).ok_or_else(|| bad(cmd, "no active local selection"))?;
    let channel = rec.channels.get(index).cloned().ok_or_else(|| bad(cmd, format!("no local selection {index}")))?;
    s.edit("Update Local Selection", |doc, _| {
        let sel = doc.selection.clone().ok_or_else(|| bad(cmd, "make a selection first"))?;
        let ch = doc.channels.iter_mut().find(|c| c.name == channel).ok_or_else(|| bad(cmd, format!("channel \"{channel}\" is gone")))?;
        ch.surface = sel;
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "channel": channel}))
}

fn local_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.localSelection.remove";
    let id = layer_param(s, p)?;
    let rec = local_rec(s, id)?;
    let index = p.get("index").and_then(Value::as_u64).map(|i| i as usize).or(rec.active).ok_or_else(|| bad(cmd, "needs `index`"))?;
    let channel = rec.channels.get(index).cloned().ok_or_else(|| bad(cmd, format!("no local selection {index}")))?;
    s.edit("Remove Local Selection", |doc, _| {
        doc.channels.retain(|c| c.name != channel);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let mut rec: LocalSelections = layer_meta::get(l, LOCAL_SELECTIONS).unwrap_or_default();
        rec.channels.remove(index);
        rec.active = rec.active.and_then(|a| {
            if a == index {
                None
            } else if a > index {
                Some(a - 1)
            } else {
                Some(a)
            }
        });
        if rec.channels.is_empty() {
            layer_meta::remove(l, LOCAL_SELECTIONS);
        } else {
            layer_meta::set(l, LOCAL_SELECTIONS, &rec);
        }
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "removed": channel}))
}

fn local_list(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let rec = local_rec(s, id)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let global = st.doc.selection.is_some();
    let list: Vec<Value> = rec
        .channels
        .iter()
        .enumerate()
        .map(|(i, c)| json!({"index": i, "channel": c, "active": rec.active == Some(i), "present": st.doc.channels.iter().any(|a| &a.name == c)}))
        .collect();
    Ok(json!({"layer": id.0, "selections": list, "globalSelection": global}))
}

// ---------- select layers by state ----------

fn select_by_state(s: &mut Session, state: &str) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = &d.doc;
    let ids: Vec<LayerId> = doc
        .walk()
        .into_iter()
        .filter(|(path, _, l)| match state {
            "visible" => doc.shown_at(path),
            "invisible" => !doc.shown_at(path),
            "locked" => l.locks.any() || doc.is_background(l.id),
            "unlocked" => !l.locks.any() && !doc.is_background(l.id),
            _ => !doc.is_background(l.id),
        })
        .map(|(_, _, l)| l.id)
        .collect();
    if ids.is_empty() {
        return Err(EngineError::Other(format!("there are no {state} layers")));
    }
    let active = d.active_layer.filter(|a| ids.contains(a)).or(ids.last().copied());
    let n = ids.len();
    crate::layer_multi_cmds::set_selection(s, ids, active, None)?;
    Ok(json!({"selected": n, "state": state}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $params:expr, $en:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $en, journal: true, run: $run }
        };
    }
    const CONVERT: &[&str] = &["Layer", "Convert"];
    const SPLIT: &[&str] = &["Layer", "Split Alpha"];
    const LOCAL: &[&str] = &["Select", "Local Selection"];
    const BY_STATE: &[&str] = &["Select", "Layers by State"];
    vec![
        spec!(
            "layer.new.fileLayer",
            "File Layer…",
            &["Layer", "New"],
            r##"{"path":file,"scaling":"none"|"toImageSize"|"toImageResolution"=none,"filter":"bicubic"|"bilinear"|"nearest"|"lanczos"?,"name":str?} → a linked image that reloads when its file changes"##,
            has_doc,
            new_file_layer
        ),
        spec!(
            "layer.fileLayer.set",
            "File Layer Properties…",
            &["Layer"],
            r##"{"layer":id?,"scaling":…?,"filter":…?,"path":file?}"##,
            has_file_layer,
            set_file_layer
        ),
        spec!("layer.fileLayer.reload", "Reload File Layer", &["Layer"], r##"{"layer":id?}"##, has_file_layer, reload_file_layer),
        spec!("layer.convert.toPaintLayer", "to Paint Layer", CONVERT, r##"{"layer":id?}"##, has_layer, to_paint_layer),
        spec!("layer.convert.toFileLayer", "to File Layer…", CONVERT, r##"{"layer":id?,"path":file.png?,"scaling":…?}"##, has_layer, to_file_layer),
        spec!(
            "layer.convert.toTransparencyMask",
            "to Transparency Mask",
            CONVERT,
            r##"{"layer":id?} (its alpha masks the layer below)"##,
            has_layer,
            to_transparency_mask
        ),
        spec!(
            "layer.convert.toFilterMask",
            "to Filter Mask…",
            CONVERT,
            r##"{"layer":id?,"filter":"filter.blur.gaussian"|…,"params":{…}?} (its alpha limits the filter over the layer below)"##,
            has_layer,
            to_filter_mask
        ),
        spec!(
            "layer.convert.toSelectionMask",
            "to Selection Mask",
            CONVERT,
            r##"{"layer":id?} (its alpha becomes a local selection of the layer below)"##,
            has_layer,
            to_selection_mask
        ),
        spec!(
            "layer.convert.colorSpace",
            "Layer Color Space…",
            CONVERT,
            r##"{"layer":id?,"mode":"rgb"|"gray"|"cmyk"|"lab"?,"depth":8|16|32?}"##,
            has_raster,
            to_color_space
        ),
        spec!(
            "layer.importAs",
            "Import Image as…",
            &["Layer", "Import"],
            r##"{"path":file,"as":"layer"|"transparencyMask"|"filterMask"|"selectionMask"=layer,"layer":id? (the masked layer),"filter":…? (filter mask)}"##,
            has_doc,
            import_as
        ),
        spec!("layer.splitAlpha.alphaIntoMask", "Alpha into Mask", SPLIT, r##"{"layer":id?}"##, has_raster, alpha_into_mask),
        spec!("layer.splitAlpha.writeAsAlpha", "Write as Alpha", SPLIT, r##"{"layer":id?}"##, has_raster_mask, mask_into_alpha),
        spec!("layer.splitAlpha.saveMerged", "Save Merged…", SPLIT, r##"{"layer":id?,"path":file} (colour kept under transparency)"##, has_raster, save_merged),
        spec!(
            "layer.localSelection.add",
            "Add Local Selection",
            LOCAL,
            r##"{"layer":id?,"name":str?} (stores the current selection on the layer)"##,
            has_layer,
            local_add
        ),
        spec!("layer.localSelection.activate", "Activate Local Selection", LOCAL, r##"{"layer":id?,"index":int?}"##, has_layer, local_activate),
        spec!("layer.localSelection.update", "Update Local Selection", LOCAL, r##"{"layer":id?,"index":int?}"##, has_layer, local_update),
        spec!("layer.localSelection.remove", "Remove Local Selection", LOCAL, r##"{"layer":id?,"index":int?}"##, has_layer, local_remove),
        spec!("layer.localSelection.list", "Local Selections", &[], r##"{"layer":id?} → {selections:[{index,channel,active}]}"##, has_layer, local_list),
        spec!("select.layers.all", "All", BY_STATE, "{}", has_doc, |s, _| select_by_state(s, "all")),
        spec!("select.layers.visible", "Visible", BY_STATE, "{}", has_doc, |s, _| select_by_state(s, "visible")),
        spec!("select.layers.invisible", "Invisible", BY_STATE, "{}", has_doc, |s, _| select_by_state(s, "invisible")),
        spec!("select.layers.locked", "Locked", BY_STATE, "{}", has_doc, |s, _| select_by_state(s, "locked")),
        spec!("select.layers.unlocked", "Unlocked", BY_STATE, "{}", has_doc, |s, _| select_by_state(s, "unlocked")),
    ]
}
