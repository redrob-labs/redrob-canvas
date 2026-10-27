//! Colour tools (Image › Adjustments, Image › Mode and Image › Analysis): component extraction,
//! Decompose / Compose / Recompose, Linear and Value Invert, Stretch Contrast HSV, Colour Enhance,
//! Alien Map, Rotate Colours, Palette Map, Sample Colorize, RGB Clip, Dither, Local Threshold,
//! Destripe, Retinex, Stress, Colour to Gray, the Fattal / Mantiuk / Reinhard tone-mapping
//! operators, Border Average, Smooth Palette, Export Histogram and the colormap tools (Rearrange
//! Colormap, Set Colormap from a swatch group).
//!
//! The pixel maths lives in `redrob_photo_algo::color_tools`. Like the other destructive
//! adjustments, the per-layer tools convert the target (layer pixels, or a targeted alpha channel
//! / Quick Mask) to straight RGBA, so every colour model and bit depth is handled, and blend the
//! result through the selection.

use redrob_photo_algo::color_tools as ct;
use redrob_photo_color::{ColorMode, PixelFormat};
use redrob_photo_doc::{ColorTable, Document, Layer, LayerContent, LayerId, Rect, Size};
use redrob_photo_raster::{Surface, from_rgba_into, to_rgba};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn num(p: &Value, key: &str, default: f32) -> f32 {
    p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(default, |v| v as f32)
}

fn int(p: &Value, key: &str, default: i64) -> i64 {
    p.get(key).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64))).unwrap_or(default)
}

fn boolean(p: &Value, key: &str, default: bool) -> bool {
    p.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn string<'a>(p: &'a Value, key: &str, default: &'a str) -> &'a str {
    p.get(key).and_then(Value::as_str).unwrap_or(default)
}

fn hex(c: [f32; 3]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

// ---------- targets ----------

/// Runs `f` on the target surface's content as straight RGBA (`w × h` = content bounds), blending
/// the result through the selection.
fn process_surface(surf: &mut Surface, selection: Option<&Surface>, f: &mut dyn FnMut(&mut Vec<[f32; 4]>, Rect)) {
    let r = surf.content_bounds();
    if r.is_empty() {
        return;
    }
    let fmt = surf.format();
    let n = fmt.channels();
    let raw = surf.read_region(r);
    let mut px: Vec<[f32; 4]> = raw.chunks_exact(n).map(|q| to_rgba(&fmt, q)).collect();
    let orig = px.clone();
    f(&mut px, r);
    surf.write_region(r, &crate::pixels::mix_through_selection(&fmt, r, &orig, &px, selection));
}

/// One history step running `f` on the active pixel layer (or targeted channel or layer mask).
fn rgba_edit(s: &mut Session, label: &str, p: &Value, mut f: impl FnMut(&mut Vec<[f32; 4]>, Rect)) -> Result<Value> {
    let id = crate::commands::layer_param(s, &Value::Null).ok();
    if crate::channel_cmds::target_of(p) != crate::channel_cmds::Target::Pixels {
        return s.edit(label, |doc, _| {
            let sel = doc.selection.clone();
            if let Some(surf) = crate::channel_cmds::channel_surface_for_filter(doc, id, p)? {
                process_surface(surf, sel.as_ref(), &mut f);
                surf.prune();
            }
            Ok(Value::Null)
        });
    }
    let id = id.ok_or_else(|| EngineError::Other("no active layer".into()))?;
    s.edit(label, |doc, _| {
        let sel = doc.selection.clone();
        let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
        process_surface(surf, sel.as_ref(), &mut f);
        Ok(Value::Null)
    })
}

fn has_pixels(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_cmds::edits_channel(s) {
        return Ok(());
    }
    let d = s.active().ok_or("no document open")?;
    let l = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active layer")?;
    if matches!(l.content, LayerContent::Raster(_)) {
        Ok(())
    } else {
        Err(format!("active layer is {} {} layer, not a pixel layer", l.content.article(), l.content.kind_name()))
    }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_color_table(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.color_table.is_some() { Ok(()) } else { Err("the document has no color table (Image › Mode › Indexed Color first)".into()) }
}

fn has_swatches(s: &Session) -> std::result::Result<(), String> {
    has_pixels(s)?;
    if s.presets.swatches.iter().any(|g| !g.items.is_empty()) { Ok(()) } else { Err("the Swatches panel is empty".into()) }
}

/// Straight RGBA of a document's merged image or one of its layers, over the canvas.
fn document_pixels(doc: &Document, layer: Option<u64>) -> Result<Vec<[f32; 4]>> {
    let area = doc.bounds();
    Ok(match layer {
        Some(id) => {
            let l = doc.layer(LayerId(id)).ok_or(EngineError::NoLayer(LayerId(id)))?;
            redrob_photo_compose::render_layer(l, area).px
        }
        None => redrob_photo_compose::flatten(doc).px,
    })
}

/// A raster layer holding `px` (straight RGBA) over `area` in `fmt`.
fn layer_from_rgba(name: &str, fmt: PixelFormat, area: Rect, px: &[[f32; 4]], background: bool) -> Layer {
    let mut l = Layer::raster(name, fmt);
    if background {
        l.locks.transparency = true;
        l.locks.position = true;
    }
    let n = fmt.channels();
    let mut data = Vec::with_capacity(px.len() * n);
    let mut tmp = vec![0.0f32; n.max(1)];
    for q in px {
        let m = from_rgba_into(&fmt, *q, &mut tmp);
        data.extend_from_slice(&tmp[..m]);
    }
    if let Some(sf) = l.surface_mut() {
        sf.write_region(area, &data);
        sf.prune();
    }
    l
}

/// A new document like `like` (size, resolution, depth) in `mode` holding one background layer.
fn document_like(name: &str, like: &Document, mode: ColorMode, px: &[[f32; 4]]) -> Document {
    let mut d = Document::new(name, like.size, mode, like.depth);
    d.resolution_dpi = like.resolution_dpi;
    let fmt = d.pixel_format();
    d.layers.push(layer_from_rgba("Background", fmt, d.bounds(), px, true));
    d
}

/// The RGB colours of a Swatches group (`group` by name, else the first non-empty group).
fn swatch_palette(s: &Session, group: Option<&str>) -> Option<Vec<[f32; 3]>> {
    let conv = crate::presets::swatches::SwatchConv::new(s);
    let g = match group {
        Some(name) => s.presets.swatches.iter().find(|g| g.name == name)?,
        None => s.presets.swatches.iter().find(|g| !g.items.is_empty())?,
    };
    Some(g.items.iter().map(|sw| conv.rgb(&sw.color)).collect())
}

// ---------- per-pixel tools ----------

fn linear_invert(s: &mut Session, p: &Value) -> Result<Value> {
    rgba_edit(s, "Linear Invert", p, |px, _| ct::linear_invert(px))
}

fn value_invert(s: &mut Session, p: &Value) -> Result<Value> {
    rgba_edit(s, "Value Invert", p, |px, _| ct::value_invert(px))
}

fn stretch_contrast_hsv(s: &mut Session, p: &Value) -> Result<Value> {
    rgba_edit(s, "Stretch Contrast HSV", p, |px, _| ct::stretch_contrast_hsv(px))
}

fn color_enhance(s: &mut Session, p: &Value) -> Result<Value> {
    rgba_edit(s, "Color Enhance", p, |px, _| ct::color_enhance(px))
}

fn extract_component(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.adjustments.extractComponent";
    let id = string(p, "component", "rgbRed");
    let (model, k) = ct::component(id).ok_or_else(|| bad(CMD, format!("unknown component `{id}` (see the command's params)")))?;
    let invert = boolean(p, "invert", false);
    let linear = boolean(p, "linear", false);
    rgba_edit(s, "Extract Component", p, move |px, _| ct::extract_component(px, model, k, invert, linear))
}

fn alien_map(s: &mut Session, p: &Value) -> Result<Value> {
    let o = ct::AlienMap {
        hsl: string(p, "colorModel", "rgb") == "hsl",
        frequency: [
            num(p, "redFrequency", 1.0).clamp(0.0, 20.0),
            num(p, "greenFrequency", 1.0).clamp(0.0, 20.0),
            num(p, "blueFrequency", 1.0).clamp(0.0, 20.0),
        ],
        phase: [num(p, "redPhase", 0.0).clamp(-180.0, 180.0), num(p, "greenPhase", 0.0).clamp(-180.0, 180.0), num(p, "bluePhase", 0.0).clamp(-180.0, 180.0)],
        keep: [boolean(p, "keepRed", false), boolean(p, "keepGreen", false), boolean(p, "keepBlue", false)],
    };
    rgba_edit(s, "Alien Map", p, move |px, _| ct::alien_map(px, &o))
}

fn rotate_colors(s: &mut Session, p: &Value) -> Result<Value> {
    let o = ct::RotateColors {
        src_from: num(p, "sourceFrom", 0.0).rem_euclid(360.0),
        src_to: num(p, "sourceTo", 90.0).rem_euclid(360.0),
        src_clockwise: boolean(p, "sourceClockwise", true),
        dst_from: num(p, "destinationFrom", 0.0).rem_euclid(360.0),
        dst_to: num(p, "destinationTo", 90.0).rem_euclid(360.0),
        dst_clockwise: boolean(p, "destinationClockwise", true),
        gray_threshold: num(p, "grayThreshold", 0.0).clamp(0.0, 1.0),
        gray_mode: if string(p, "grayMode", "treatAs") == "changeTo" { ct::GrayMode::ChangeTo } else { ct::GrayMode::TreatAs },
        gray_hue: num(p, "grayHue", 0.0).rem_euclid(360.0),
        gray_saturation: num(p, "graySaturation", 0.0).clamp(0.0, 1.0),
    };
    rgba_edit(s, "Rotate Colors", p, move |px, _| ct::rotate_colors(px, &o))
}

fn palette_map(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.adjustments.paletteMap";
    let group = p.get("group").and_then(Value::as_str).filter(|g| !g.is_empty());
    let palette = swatch_palette(s, group).ok_or_else(|| bad(CMD, "no swatch group with colours (see swatches.list)"))?;
    rgba_edit(s, "Palette Map", p, move |px, _| ct::palette_map(px, &palette))
}

fn rgb_clip(s: &mut Session, p: &Value) -> Result<Value> {
    let low = boolean(p, "clipLow", true).then(|| num(p, "lowLimit", 0.0).clamp(-10.0, 10.0));
    let high = boolean(p, "clipHigh", true).then(|| num(p, "highLimit", 1.0).clamp(-10.0, 10.0));
    rgba_edit(s, "RGB Clip", p, move |px, _| ct::rgb_clip(px, low, high))
}

fn sample_colorize(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.adjustments.sampleColorize";
    let o = ct::SampleColorize {
        hold_intensity: boolean(p, "holdIntensity", true),
        original_intensity: boolean(p, "originalIntensity", true),
        smooth_samples: boolean(p, "smoothSamples", true),
        in_low: num(p, "inLow", 0.0).clamp(0.0, 255.0),
        in_high: num(p, "inHigh", 255.0).clamp(0.0, 255.0),
        gamma: num(p, "gamma", 1.0).clamp(0.1, 10.0),
        out_low: num(p, "outLow", 0.0).clamp(0.0, 255.0),
        out_high: num(p, "outHigh", 255.0).clamp(0.0, 255.0),
    };
    let fg = s.tools.foreground;
    let bg = s.tools.background;
    let table = match p.get("source").and_then(Value::as_i64) {
        Some(i) if i >= 0 && !boolean(p, "useGradient", false) => {
            let d = s.documents().get(i as usize).ok_or_else(|| bad(CMD, format!("no document {i}")))?;
            let px = document_pixels(&d.doc, p.get("sourceLayer").and_then(Value::as_u64))?;
            ct::sample_table(&px, o.smooth_samples).ok_or_else(|| bad(CMD, "the sample has no opaque pixels"))?
        }
        _ => ct::gradient_table([fg[0], fg[1], fg[2]], [bg[0], bg[1], bg[2]]),
    };
    rgba_edit(s, "Sample Colorize", p, move |px, _| ct::sample_colorize(px, &table, &o))
}

fn dither(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.adjustments.dither";
    let id = string(p, "method", "floydSteinberg");
    let method = ct::DitherMethod::from_id(id).ok_or_else(|| bad(CMD, format!("unknown method `{id}`")))?;
    let levels = |key: &str, d: i64| int(p, key, d).clamp(2, 65536) as u32;
    let o = ct::Dither {
        levels: [levels("redLevels", 6), levels("greenLevels", 7), levels("blueLevels", 6), levels("alphaLevels", 256)],
        method,
        seed: int(p, "seed", 0).clamp(0, u32::MAX as i64) as u32,
    };
    rgba_edit(s, "Dither", p, move |px, r| ct::dither(px, r.width() as usize, &o))
}

fn local_threshold(s: &mut Session, p: &Value) -> Result<Value> {
    let radius = num(p, "radius", 25.0).clamp(0.0, 1000.0);
    let low = num(p, "low", 0.5).clamp(0.0, 1.0);
    let high = num(p, "high", 0.5).clamp(0.0, 1.0);
    let aa = int(p, "antialias", 1).clamp(0, 8) as u32;
    rgba_edit(s, "Local Threshold", p, move |px, r| ct::local_threshold(px, r.width() as usize, r.height() as usize, radius, low, high, aa))
}

fn destripe(s: &mut Session, p: &Value) -> Result<Value> {
    let width = int(p, "width", 36).clamp(2, 100) as u32;
    let histogram = boolean(p, "createHistogram", false);
    rgba_edit(s, "Destripe", p, move |px, r| ct::destripe(px, r.width() as usize, r.height() as usize, width, histogram))
}

fn retinex(s: &mut Session, p: &Value) -> Result<Value> {
    let o = ct::Retinex {
        level: match string(p, "level", "uniform") {
            "low" => ct::RetinexLevel::Low,
            "high" => ct::RetinexLevel::High,
            _ => ct::RetinexLevel::Uniform,
        },
        scale: num(p, "scale", 240.0).clamp(16.0, 250.0),
        scale_division: int(p, "scaleDivision", 3).clamp(1, 8) as u32,
        dynamic: num(p, "dynamic", 1.2).clamp(0.05, 4.0),
    };
    rgba_edit(s, "Retinex", p, move |px, r| ct::retinex(px, r.width() as usize, r.height() as usize, &o))
}

fn stress_params(p: &Value, samples: i64, iterations: i64) -> ct::Stress {
    ct::Stress {
        radius: num(p, "radius", 300.0).clamp(2.0, 6000.0),
        samples: int(p, "samples", samples).clamp(1, 1000) as u32,
        iterations: int(p, "iterations", iterations).clamp(1, 1000) as u32,
        enhance_shadows: boolean(p, "enhanceShadows", false),
        seed: int(p, "seed", 0).clamp(0, u32::MAX as i64) as u32,
    }
}

fn stress(s: &mut Session, p: &Value) -> Result<Value> {
    let o = stress_params(p, 5, 5);
    rgba_edit(s, "Stress", p, move |px, r| ct::stress(px, r.width() as usize, r.height() as usize, &o))
}

fn color_to_gray(s: &mut Session, p: &Value) -> Result<Value> {
    let o = stress_params(p, 4, 10);
    rgba_edit(s, "Color to Gray", p, move |px, r| ct::color_to_gray(px, r.width() as usize, r.height() as usize, &o))
}

fn tone_map_fattal(s: &mut Session, p: &Value) -> Result<Value> {
    let d = ct::Fattal::default();
    let o = ct::Fattal {
        alpha: num(p, "alpha", d.alpha).clamp(0.0, 2.0),
        beta: num(p, "beta", d.beta).clamp(0.1, 2.0),
        saturation: num(p, "saturation", d.saturation).clamp(0.0, 1.0),
        noise: num(p, "noise", d.noise).clamp(0.0, 1.0),
    };
    rgba_edit(s, "Fattal et al. 2002", p, move |px, r| ct::fattal(px, r.width() as usize, r.height() as usize, &o))
}

fn tone_map_mantiuk(s: &mut Session, p: &Value) -> Result<Value> {
    let d = ct::Mantiuk::default();
    let o = ct::Mantiuk {
        contrast: num(p, "contrast", d.contrast).clamp(0.0, 1.0),
        saturation: num(p, "saturation", d.saturation).clamp(0.0, 2.0),
        detail: num(p, "detail", d.detail).clamp(1.0, 99.0),
    };
    rgba_edit(s, "Mantiuk 2006", p, move |px, r| ct::mantiuk(px, r.width() as usize, r.height() as usize, &o))
}

fn tone_map_reinhard(s: &mut Session, p: &Value) -> Result<Value> {
    let d = ct::Reinhard::default();
    let o = ct::Reinhard {
        brightness: num(p, "brightness", d.brightness).clamp(-8.0, 8.0),
        chromatic: num(p, "chromatic", d.chromatic).clamp(0.0, 1.0),
        light: num(p, "light", d.light).clamp(0.0, 1.0),
    };
    rgba_edit(s, "Reinhard 2005", p, move |px, r| ct::reinhard(px, r.width() as usize, r.height() as usize, &o))
}

// ---------- Decompose / Compose / Recompose ----------

const DECOMPOSE_MODEL: &str = "redrob:decompose:model";
const DECOMPOSE_SOURCE: &str = "redrob:decompose:source";

fn model_param(cmd: &str, p: &Value, default: &str) -> Result<ct::Model> {
    let id = string(p, "model", default);
    ct::Model::from_id(id)
        .ok_or_else(|| bad(cmd, format!("unknown model `{id}` (rgb|rgba|hsv|hsl|cmy|cmyk|lab|lch|ycbcr470|ycbcr709|ycbcr470f|ycbcr709f|alpha)")))
}

/// Decompose: the active document's merged image split into one grey image per component of the
/// model, as the layers of a new document (`toLayers`, the default) or as separate documents.
/// `colorize` tints the RGB / CMY component layers with their colour.
fn decompose(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.mode.decompose";
    let model = model_param(CMD, p, "rgb")?;
    let to_layers = boolean(p, "toLayers", true);
    let colorize = boolean(p, "colorize", false);
    let doc = (*s.active().ok_or(EngineError::NoDocument)?.doc).clone();
    let px = redrob_photo_compose::flatten(&doc).px;
    let planes = ct::decompose(&px, model);
    let names = model.components();
    let area = doc.bounds();
    let plane_px = |plane: &[f32], tint: Option<[f32; 3]>| -> Vec<[f32; 4]> {
        plane
            .iter()
            .map(|&g| match tint {
                Some(t) => [t[0] * g, t[1] * g, t[2] * g, 1.0],
                None => [g, g, g, 1.0],
            })
            .collect()
    };
    let mut made = Vec::new();
    if to_layers {
        let mode = if colorize { ColorMode::Rgb } else { ColorMode::Grayscale };
        let mut out = Document::new(format!("{}-{}", doc.name, model.id()), doc.size, mode, doc.depth);
        out.resolution_dpi = doc.resolution_dpi;
        let fmt = out.pixel_format();
        // Layers are stored bottom-up: the first component ends up on top, as in a channels list.
        for (k, name) in names.iter().enumerate().rev() {
            let tint = if colorize { model.tint(k) } else { None };
            out.layers.push(layer_from_rgba(name, fmt, area, &plane_px(&planes[k], tint), false));
        }
        out.metadata.text.push((DECOMPOSE_MODEL.into(), model.id().into()));
        out.metadata.text.push((DECOMPOSE_SOURCE.into(), doc.name.clone()));
        made.push(s.add_document(out, None));
    } else {
        for (k, name) in names.iter().enumerate() {
            let tint = if colorize { model.tint(k) } else { None };
            let mode = if tint.is_some() { ColorMode::Rgb } else { ColorMode::Grayscale };
            let mut out = document_like(&format!("{}-{}", doc.name, name), &doc, mode, &plane_px(&planes[k], tint));
            out.metadata.text.push((DECOMPOSE_MODEL.into(), model.id().into()));
            out.metadata.text.push((DECOMPOSE_SOURCE.into(), doc.name.clone()));
            made.push(s.add_document(out, None));
        }
    }
    Ok(json!({ "documents": made, "components": names }))
}

/// Grey plane (luma) of a layer or merged document.
fn luma_plane(px: &[[f32; 4]]) -> Vec<f32> {
    px.iter().map(|q| 0.299 * q[0] + 0.587 * q[1] + 0.114 * q[2]).collect()
}

/// Compose: a new colour document whose components of `model` are the grey images given as open
/// documents (`documents`, by index) or, with `fromLayers`, the layers of the active document
/// (bottom to top, or matched by component name).
fn compose(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.mode.compose";
    let model = model_param(CMD, p, "rgb")?;
    let names = model.components();
    let active = s.active_index().ok_or(EngineError::NoDocument)?;
    let like = (*s.documents()[active].doc).clone();
    let mut planes: Vec<Vec<f32>> = Vec::new();
    if boolean(p, "fromLayers", false) {
        let area = like.bounds();
        let rasters: Vec<&Layer> = like.layers.iter().filter(|l| matches!(l.content, LayerContent::Raster(_))).collect();
        for name in names {
            let l =
                rasters.iter().find(|l| l.name.eq_ignore_ascii_case(name)).copied().or_else(|| rasters.get(planes.len()).copied()).ok_or_else(|| {
                    bad(CMD, format!("the document has no layer for the {name} component ({} layers, {} needed)", rasters.len(), names.len()))
                })?;
            planes.push(luma_plane(&redrob_photo_compose::render_layer(l, area).px));
        }
    } else {
        let idxs: Vec<usize> = match p.get("documents").and_then(Value::as_array) {
            Some(a) => a.iter().map(|v| v.as_u64().map(|i| i as usize).ok_or_else(|| bad(CMD, "bad document index"))).collect::<Result<_>>()?,
            None => (0..s.documents().len()).filter(|&i| s.documents()[i].doc.size == like.size).take(names.len()).collect(),
        };
        if idxs.len() < names.len() {
            return Err(bad(CMD, format!("{} needs {} documents of the same size, got {}", model.id(), names.len(), idxs.len())));
        }
        for &i in idxs.iter().take(names.len()) {
            let d = s.documents().get(i).ok_or_else(|| bad(CMD, format!("no document {i}")))?;
            if d.doc.size != like.size {
                return Err(bad(CMD, format!("\"{}\" is not the same size as the active document", d.doc.name)));
            }
            planes.push(luma_plane(&redrob_photo_compose::flatten(&d.doc).px));
        }
    }
    let n = like.bounds().width() as usize * like.bounds().height() as usize;
    let refs: Vec<&[f32]> = planes.iter().map(|v| v.as_slice()).collect();
    let px = ct::compose(&refs, model, n);
    let name = string(p, "name", "Composed").to_string();
    let out = document_like(&name, &like, ColorMode::Rgb, &px);
    let i = s.add_document(out, None);
    Ok(json!({ "document": i }))
}

fn can_recompose(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.metadata.text.iter().any(|(k, _)| k == DECOMPOSE_MODEL) { Ok(()) } else { Err("the active document was not made by Decompose".into()) }
}

/// Recompose: rebuilds the colour image from a Decompose-to-layers document, into the source
/// document when it is still open (one history step there), else as a new document.
fn recompose(s: &mut Session, _p: &Value) -> Result<Value> {
    const CMD: &str = "image.mode.recompose";
    let active = s.active_index().ok_or(EngineError::NoDocument)?;
    let doc = (*s.documents()[active].doc).clone();
    let model_id = doc
        .metadata
        .text
        .iter()
        .find(|(k, _)| k == DECOMPOSE_MODEL)
        .map(|(_, v)| v.clone())
        .ok_or_else(|| bad(CMD, "the active document was not made by Decompose"))?;
    let model = ct::Model::from_id(&model_id).ok_or_else(|| bad(CMD, format!("unknown decomposition model `{model_id}`")))?;
    let source = doc.metadata.text.iter().find(|(k, _)| k == DECOMPOSE_SOURCE).map(|(_, v)| v.clone());
    let names = model.components();
    let area = doc.bounds();
    let rasters: Vec<&Layer> = doc.layers.iter().filter(|l| matches!(l.content, LayerContent::Raster(_))).collect();
    let mut planes: Vec<Vec<f32>> = Vec::new();
    for (k, name) in names.iter().enumerate() {
        let l = rasters
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(name))
            .copied()
            .or_else(|| rasters.get(rasters.len().checked_sub(k + 1)?).copied())
            .ok_or_else(|| bad(CMD, format!("no layer for the {name} component")))?;
        let px = redrob_photo_compose::render_layer(l, area).px;
        let tint = model.tint(k);
        planes.push(match tint {
            // A colourised layer holds the component in its tinted channels: read the brightest.
            Some(_) => px.iter().map(|q| q[0].max(q[1]).max(q[2])).collect(),
            None => luma_plane(&px),
        });
    }
    let n = area.width() as usize * area.height() as usize;
    let refs: Vec<&[f32]> = planes.iter().map(|v| v.as_slice()).collect();
    let px = ct::compose(&refs, model, n);
    let target =
        source.as_deref().and_then(|name| s.documents().iter().position(|d| d.doc.name == name && d.doc.size == doc.size && d.doc.mode == ColorMode::Rgb));
    match target {
        Some(t) => {
            s.set_active(t);
            let r = s.edit("Recompose", |target_doc, active| {
                let fmt = target_doc.pixel_format();
                let bounds = target_doc.bounds();
                let id =
                    active.or_else(|| target_doc.layers.first().map(|l| l.id)).ok_or_else(|| EngineError::Other("the source document has no layer".into()))?;
                let l = target_doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
                let surf = l.surface_mut().ok_or_else(|| EngineError::Other("the active layer of the source document has no pixels".into()))?;
                let n = fmt.channels();
                let mut data = Vec::with_capacity(px.len() * n);
                let mut tmp = vec![0.0f32; n.max(1)];
                for q in &px {
                    let m = from_rgba_into(&fmt, *q, &mut tmp);
                    data.extend_from_slice(&tmp[..m]);
                }
                surf.write_region(bounds, &data);
                surf.prune();
                Ok(())
            });
            s.set_active(active);
            r?;
            Ok(json!({ "document": t }))
        }
        None => {
            let name = source.unwrap_or_else(|| "Recomposed".into());
            let out = document_like(&name, &doc, ColorMode::Rgb, &px);
            let i = s.add_document(out, None);
            Ok(json!({ "document": i }))
        }
    }
}

// ---------- colormap ----------

/// Rearrange Colormap: reorders the entries of the active indexed document's colour table (the
/// pixels keep their colours, so only indices change): an explicit `order`, a `from` → `to` move,
/// a `sort` key and/or `reverse`.
fn rearrange_colormap(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.mode.colorTable.rearrange";
    let cur = s.active().ok_or(EngineError::NoDocument)?.doc.color_table.clone().ok_or_else(|| EngineError::Other("the document has no color table".into()))?;
    let n = cur.colors.len();
    let mut perm: Vec<usize> = (0..n).collect();
    if let Some(order) = p.get("order").and_then(Value::as_array) {
        let o: Option<Vec<usize>> = order.iter().map(|v| v.as_u64().map(|i| i as usize).filter(|i| *i < n)).collect();
        let o = o.ok_or_else(|| bad(CMD, format!("order: indices from 0 to {}", n.saturating_sub(1))))?;
        let mut seen = vec![false; n];
        for &i in &o {
            if seen[i] {
                return Err(bad(CMD, format!("order lists entry {i} twice")));
            }
            seen[i] = true;
        }
        // Entries not listed keep their relative order after the listed ones.
        perm = o;
        perm.extend((0..n).filter(|i| !seen[*i]));
    }
    if let (Some(from), Some(to)) = (p.get("from").and_then(Value::as_u64), p.get("to").and_then(Value::as_u64)) {
        let (from, to) = (from as usize, to as usize);
        if from >= n || to >= n {
            return Err(bad(CMD, format!("from/to: indices from 0 to {}", n.saturating_sub(1))));
        }
        let e = perm.remove(from);
        perm.insert(to, e);
    }
    let key = string(p, "sort", "none");
    if key != "none" {
        let rgb = |i: usize| cur.rgb(i);
        let hsv = |i: usize| {
            let c = rgb(i);
            let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
            let d = mx - mn;
            let h = if d <= 0.0 {
                0.0
            } else if mx == c[0] {
                ((c[1] - c[2]) / d).rem_euclid(6.0)
            } else if mx == c[1] {
                (c[2] - c[0]) / d + 2.0
            } else {
                (c[0] - c[1]) / d + 4.0
            };
            (h / 6.0, if mx > 0.0 { d / mx } else { 0.0 }, mx)
        };
        let score: Box<dyn Fn(usize) -> f32> = match key {
            "hue" => Box::new(move |i| hsv(i).0),
            "saturation" => Box::new(move |i| hsv(i).1),
            "value" => Box::new(move |i| hsv(i).2),
            "luminance" => Box::new(move |i| {
                let c = rgb(i);
                0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
            }),
            "red" => Box::new(move |i| rgb(i)[0]),
            "green" => Box::new(move |i| rgb(i)[1]),
            "blue" => Box::new(move |i| rgb(i)[2]),
            other => return Err(bad(CMD, format!("sort `{other}` (none|hue|saturation|value|luminance|red|green|blue)"))),
        };
        perm.sort_by(|&a, &b| score(a).total_cmp(&score(b)));
    }
    if boolean(p, "reverse", false) {
        perm.reverse();
    }
    let table = ColorTable {
        colors: perm.iter().map(|&i| cur.colors[i]).collect(),
        transparent: cur.transparent.and_then(|t| perm.iter().position(|&i| i == usize::from(t))).and_then(|i| u8::try_from(i).ok()),
    };
    let report = json!({ "colors": table.colors.iter().map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).collect::<Vec<_>>(), "transparent": table.transparent, "order": perm });
    if table == cur {
        return Ok(report);
    }
    s.edit("Rearrange Colormap", |doc, _| {
        doc.color_table = Some(table.clone());
        Ok(())
    })?;
    Ok(report)
}

/// Set Colormap: replaces the active indexed document's colour table with the colours of a
/// Swatches group; pixels follow their index (`remap: "index"`, the default) or move to the
/// nearest new colour (`"nearest"`).
fn set_colormap(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.mode.colorTable.fromPalette";
    let cur = s.active().ok_or(EngineError::NoDocument)?.doc.color_table.clone().ok_or_else(|| EngineError::Other("the document has no color table".into()))?;
    let group = p.get("group").and_then(Value::as_str).filter(|g| !g.is_empty());
    let palette = swatch_palette(s, group).ok_or_else(|| bad(CMD, "no swatch group with colours (see swatches.list)"))?;
    if palette.is_empty() {
        return Err(bad(CMD, "the swatch group is empty"));
    }
    let colors: Vec<[u8; 3]> = palette.iter().take(256).map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    let nearest = string(p, "remap", "index") == "nearest";
    let table = ColorTable { colors, transparent: cur.transparent.filter(|&t| usize::from(t) < palette.len().min(256)) };
    let report = json!({ "colors": table.colors.iter().map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).collect::<Vec<_>>(), "transparent": table.transparent });
    s.edit("Set Colormap", |doc, _| {
        for surf in doc.layers.iter_mut().filter_map(Layer::surface_mut) {
            let r = surf.content_bounds();
            if r.is_empty() {
                continue;
            }
            let fmt = surf.format();
            let n = fmt.channels();
            let mut data = surf.read_region(r);
            for q in data.chunks_exact_mut(n) {
                let c = to_rgba(&fmt, q);
                if c[3] <= 0.0 {
                    continue;
                }
                let rgb = [c[0], c[1], c[2]];
                let i = if nearest { table.nearest(rgb) } else { cur.nearest(rgb).min(table.colors.len() - 1) };
                let nc = table.rgb(i);
                let alpha = if table.transparent == Some(i as u8) { 0.0 } else { c[3] };
                let mut enc = [0.0f32; 8];
                let m = from_rgba_into(&fmt, [nc[0], nc[1], nc[2], alpha], &mut enc);
                q.copy_from_slice(&enc[..m]);
            }
            surf.write_region(r, &data);
            surf.prune();
        }
        doc.color_table = Some(table.clone());
        Ok(())
    })?;
    Ok(report)
}

// ---------- analysis ----------

/// Border Average: the most common colour of the strip just inside the selection (or the canvas),
/// set as the foreground colour.
fn border_average(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.analysis.borderAverage";
    let thickness = int(p, "thickness", 3).clamp(1, 256) as usize;
    let bucket = int(p, "bucketSize", 16).clamp(1, 256) as u32;
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let canvas = doc.bounds();
    let area = doc.selection.as_ref().map(|sel| sel.content_bounds().intersect(&canvas)).filter(|r| !r.is_empty()).unwrap_or(canvas);
    let px = redrob_photo_compose::flatten(&doc).px;
    let w = canvas.width() as usize;
    let rel = ((area.x0 - canvas.x0) as usize, (area.y0 - canvas.y0) as usize, (area.x1 - canvas.x0) as usize, (area.y1 - canvas.y0) as usize);
    let c = ct::border_average(&px, w, rel, thickness, bucket).ok_or_else(|| bad(CMD, "the border has no opaque pixels"))?;
    s.tools.foreground = [c[0], c[1], c[2], 1.0];
    Ok(json!({ "color": hex(c), "rgb": [c[0], c[1], c[2]] }))
}

/// Smooth Palette: a new image whose columns are colours of the active document ordered so that
/// neighbours are similar.
fn smooth_palette(s: &mut Session, p: &Value) -> Result<Value> {
    let width = int(p, "width", 256).clamp(2, 1024) as usize;
    let height = int(p, "height", 64).clamp(1, 256) as usize;
    let depth = int(p, "searchDepth", 50).clamp(1, 1024) as u32;
    let seed = int(p, "seed", 0).clamp(0, u32::MAX as i64) as u32;
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let px = redrob_photo_compose::flatten(&doc).px;
    let out_px = ct::smooth_palette(&px, width, height, depth, seed);
    let mut out = Document::new(format!("{} Palette", doc.name), Size::new(width as u32, height as u32), ColorMode::Rgb, doc.depth);
    out.resolution_dpi = doc.resolution_dpi;
    let fmt = out.pixel_format();
    out.layers.push(layer_from_rgba("Background", fmt, out.bounds(), &out_px, true));
    let i = s.add_document(out, None);
    Ok(json!({ "document": i, "width": width, "height": height }))
}

/// Export Histogram: the histogram of the active layer (or the merged image), within the
/// selection, with `buckets` bins per channel, as CSV (written to `path` when given).
fn export_histogram(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.analysis.exportHistogram";
    let buckets = int(p, "buckets", 256).clamp(2, 65536) as usize;
    let average = boolean(p, "sampleAverage", false);
    let format = string(p, "outputFormat", "pixelCount");
    if !matches!(format, "pixelCount" | "normalized" | "percent") {
        return Err(bad(CMD, "outputFormat: pixelCount|normalized|percent"));
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = d.doc.clone();
    let layer = match p.get("layer") {
        Some(Value::Bool(false)) => None,
        Some(v) if v.is_u64() => v.as_u64(),
        _ => d.active_layer.map(|id| id.0),
    };
    let px = document_pixels(&doc, layer)?;
    let area = doc.bounds();
    let mask: Option<Vec<f32>> = doc.selection.as_ref().map(|sel| redrob_photo_algo::selection::mask_from_surface(Some(sel), area));
    let mut hist = vec![[0u64; 4]; buckets];
    let mut total = 0u64;
    for (i, q) in px.iter().enumerate() {
        if q[3] <= 0.0 || mask.as_ref().is_some_and(|m| m[i] < 0.5) {
            continue;
        }
        total += 1;
        let bin = |v: f32| ((v.clamp(0.0, 1.0) * buckets as f32) as usize).min(buckets - 1);
        let value = (q[0] + q[1] + q[2]) / 3.0;
        hist[bin(value)][0] += 1;
        hist[bin(q[0])][1] += 1;
        hist[bin(q[1])][2] += 1;
        hist[bin(q[2])][3] += 1;
    }
    let scale = |n: u64| -> String {
        match format {
            "normalized" => format!("{:.6}", if total > 0 { n as f64 / total as f64 } else { 0.0 }),
            "percent" => format!("{:.4}", if total > 0 { 100.0 * n as f64 / total as f64 } else { 0.0 }),
            _ => n.to_string(),
        }
    };
    let mut csv = String::new();
    csv.push_str(if average { "Range start,Value\n" } else { "Range start,Value,Red,Green,Blue\n" });
    for (b, row) in hist.iter().enumerate() {
        let start = (b as f64 * 255.0 / buckets as f64).floor();
        if average {
            csv.push_str(&format!("{start},{}\n", scale(row[0])));
        } else {
            csv.push_str(&format!("{start},{},{},{},{}\n", scale(row[0]), scale(row[1]), scale(row[2]), scale(row[3])));
        }
    }
    match p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(path) => {
            crate::file_cmds::write_file(path, csv.as_bytes())?;
            Ok(json!({ "path": path, "buckets": buckets, "pixels": total }))
        }
        None => Ok(json!({ "csv": csv, "buckets": buckets, "pixels": total })),
    }
}

/// The components Extract Component offers (id, model, component name), for UIs and agents.
fn list_components(_: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!(ct::COMPONENTS.iter().map(|(id, m, k)| json!({ "id": id, "model": m.id(), "component": m.components()[*k] })).collect::<Vec<_>>()))
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("image.adjustments.linearInvert", "Linear Invert", ["Image", "Adjustments"], "{}", has_pixels, linear_invert),
        spec!("image.adjustments.valueInvert", "Value Invert", ["Image", "Adjustments"], "{}", has_pixels, value_invert),
        spec!("image.adjustments.stretchContrastHsv", "Stretch Contrast HSV", ["Image", "Adjustments"], "{}", has_pixels, stretch_contrast_hsv),
        spec!("image.adjustments.colorEnhance", "Color Enhance", ["Image", "Adjustments"], "{}", has_pixels, color_enhance),
        spec!(
            "image.adjustments.extractComponent",
            "Extract Component…",
            ["Image", "Adjustments"],
            r##"{"component":"rgbRed|rgbGreen|rgbBlue|alpha|hsvHue|hsvSaturation|hsvValue|hslHue|hslSaturation|hslLightness|cmykCyan|cmykMagenta|cmykYellow|cmykKey|labL|labA|labB|lchC|lchH|ycbcr470Y|ycbcr470Cb|ycbcr470Cr|ycbcr709Y|ycbcr709Cb|ycbcr709Cr|ycbcr470fY|ycbcr470fCb|ycbcr470fCr|ycbcr709fY|ycbcr709fCb|ycbcr709fCr"="rgbRed","invert":bool=false,"linear":bool=false}"##,
            has_pixels,
            extract_component
        ),
        spec!(
            "image.adjustments.alienMap",
            "Alien Map…",
            ["Image", "Adjustments"],
            r##"{"colorModel":"rgb|hsl"="rgb","redFrequency":0..20=1,"redPhase":-180..180=0,"greenFrequency":0..20=1,"greenPhase":-180..180=0,"blueFrequency":0..20=1,"bluePhase":-180..180=0,"keepRed":bool=false,"keepGreen":bool=false,"keepBlue":bool=false} (in HSL the red/green/blue controls act on hue/saturation/lightness)"##,
            has_pixels,
            alien_map
        ),
        spec!(
            "image.adjustments.rotateColors",
            "Rotate Colors…",
            ["Image", "Adjustments"],
            r##"{"sourceFrom":0..360=0,"sourceTo":0..360=90,"sourceClockwise":bool=true,"destinationFrom":0..360=0,"destinationTo":0..360=90,"destinationClockwise":bool=true,"grayThreshold":0..1=0,"grayMode":"treatAs|changeTo"="treatAs","grayHue":0..360=0,"graySaturation":0..1=0}"##,
            has_pixels,
            rotate_colors
        ),
        spec!(
            "image.adjustments.paletteMap",
            "Palette Map…",
            ["Image", "Adjustments"],
            r##"{"group":text} (group: a Swatches group name, default the first group with colours; luminosity picks the colours in order)"##,
            has_swatches,
            palette_map
        ),
        spec!(
            "image.adjustments.sampleColorize",
            "Sample Colorize…",
            ["Image", "Adjustments"],
            r##"{"source":doc,"sourceLayer":json,"useGradient":bool=false,"holdIntensity":bool=true,"originalIntensity":bool=true,"smoothSamples":bool=true,"inLow":0..255=0,"inHigh":0..255=255,"gamma":0.1..10=1,"outLow":0..255=0,"outHigh":0..255=255} (source: document index; sourceLayer: layer id, default the merged image; no source or useGradient: foreground → background)"##,
            has_pixels,
            sample_colorize
        ),
        spec!(
            "image.adjustments.rgbClip",
            "RGB Clip…",
            ["Image", "Adjustments"],
            r##"{"clipLow":bool=true,"lowLimit":-10..10=0,"clipHigh":bool=true,"highLimit":-10..10=1}"##,
            has_pixels,
            rgb_clip
        ),
        spec!(
            "image.adjustments.dither",
            "Dither…",
            ["Image", "Adjustments"],
            r##"{"redLevels":2..65536=6,"greenLevels":2..65536=7,"blueLevels":2..65536=6,"alphaLevels":2..65536=256,"method":"none|floydSteinberg|bayer|random|randomCovariant|arithmeticAdd|arithmeticAddCovariant|arithmeticXor|arithmeticXorCovariant|blueNoise"="floydSteinberg","seed":u32=0}"##,
            has_pixels,
            dither
        ),
        spec!(
            "image.adjustments.localThreshold",
            "Local Threshold…",
            ["Image", "Adjustments"],
            r##"{"radius":0..1000=25,"low":0..1=0.5,"high":0..1=0.5,"antialias":0..8=1}"##,
            has_pixels,
            local_threshold
        ),
        spec!(
            "image.adjustments.destripe",
            "Destripe…",
            ["Image", "Adjustments"],
            r##"{"width":2..100=36,"createHistogram":bool=false}"##,
            has_pixels,
            destripe
        ),
        spec!(
            "image.adjustments.retinex",
            "Retinex…",
            ["Image", "Adjustments"],
            r##"{"level":"uniform|low|high"="uniform","scale":16..250=240,"scaleDivision":1..8=3,"dynamic":0.05..4=1.2}"##,
            has_pixels,
            retinex
        ),
        spec!(
            "image.adjustments.stress",
            "Stress…",
            ["Image", "Adjustments"],
            r##"{"radius":2..6000=300,"samples":2..200=5,"iterations":1..200=5,"enhanceShadows":bool=false,"seed":u32=0}"##,
            has_pixels,
            stress
        ),
        spec!(
            "image.adjustments.colorToGray",
            "Color to Gray…",
            ["Image", "Adjustments"],
            r##"{"radius":2..6000=300,"samples":1..1000=4,"iterations":1..1000=10,"enhanceShadows":bool=false,"seed":u32=0}"##,
            has_pixels,
            color_to_gray
        ),
        spec!(
            "image.adjustments.toneMapFattal",
            "Fattal et al. 2002…",
            ["Image", "Adjustments"],
            r##"{"alpha":0..2=1,"beta":0.1..2=0.9,"saturation":0..1=0.8,"noise":0..1=0}"##,
            has_pixels,
            tone_map_fattal
        ),
        spec!(
            "image.adjustments.toneMapMantiuk",
            "Mantiuk 2006…",
            ["Image", "Adjustments"],
            r##"{"contrast":0..1=0.1,"saturation":0..2=0.8,"detail":1..99=1}"##,
            has_pixels,
            tone_map_mantiuk
        ),
        spec!(
            "image.adjustments.toneMapReinhard",
            "Reinhard 2005…",
            ["Image", "Adjustments"],
            r##"{"brightness":-8..8=0,"chromatic":0..1=0,"light":0..1=1}"##,
            has_pixels,
            tone_map_reinhard
        ),
        spec!(
            "image.adjustments.smoothPalette",
            "Smooth Palette…",
            ["Image", "Adjustments"],
            r##"{"width":2..1024=256,"height":1..256=64,"searchDepth":1..1024=50,"seed":u32=0} (makes a new document)"##,
            has_doc,
            smooth_palette
        ),
        spec!(
            "image.mode.decompose",
            "Decompose…",
            ["Image", "Mode"],
            r##"{"model":"rgb|rgba|hsv|hsl|cmy|cmyk|lab|lch|ycbcr470|ycbcr709|ycbcr470f|ycbcr709f|alpha"="rgb","toLayers":bool=true,"colorize":bool=false} (toLayers: one new document with a layer per component, else a document per component)"##,
            has_doc,
            decompose
        ),
        spec!(
            "image.mode.compose",
            "Compose…",
            ["Image", "Mode"],
            r##"{"model":"rgb|rgba|hsv|hsl|cmy|cmyk|lab|lch|ycbcr470|ycbcr709|ycbcr470f|ycbcr709f"="rgb","fromLayers":bool=false,"documents":[index,…]?,"name":text} (fromLayers: the active document's layers are the components; else open documents of the same size, by index or the first ones)"##,
            has_doc,
            compose
        ),
        spec!("image.mode.recompose", "Recompose", ["Image", "Mode"], "{}", can_recompose, recompose),
        spec!(
            "image.mode.colorTable.rearrange",
            "Rearrange Colormap…",
            ["Image", "Mode"],
            r##"{"order":[index,…]?,"from":int,"to":int,"sort":"none|hue|saturation|value|luminance|red|green|blue"="none","reverse":bool=false} (pixels keep their colours; only the indices change)"##,
            has_color_table,
            rearrange_colormap
        ),
        spec!(
            "image.mode.colorTable.fromPalette",
            "Set Colormap…",
            ["Image", "Mode"],
            r##"{"group":text,"remap":"index|nearest"="index"} (group: a Swatches group name, default the first with colours)"##,
            has_color_table,
            set_colormap
        ),
        spec!(
            "image.analysis.borderAverage",
            "Border Average…",
            ["Image", "Analysis"],
            r##"{"thickness":1..256=3,"bucketSize":1..256=16} (sets the foreground colour)"##,
            has_doc,
            border_average
        ),
        spec!(
            "image.analysis.exportHistogram",
            "Export Histogram…",
            ["Image", "Analysis"],
            r##"{"path":text,"buckets":2..65536=256,"sampleAverage":bool=false,"outputFormat":"pixelCount|normalized|percent"="pixelCount","layer":json} (layer: id, false for the merged image, default the active layer; no path: the CSV is returned)"##,
            has_doc,
            export_histogram
        ),
        CommandSpec {
            id: "image.adjustments.extractComponent.list",
            label: "List Extractable Components",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: |_| Ok(()),
            run: list_components,
            journal: false,
        },
    ]
}
