// SPDX-License-Identifier: MIT OR Apache-2.0
//! Live layers: pixel layers whose content is derived from other layers and re-derived after
//! every edit, so they follow their sources. Clone layers (a live copy of another layer with its
//! own position, masks and filters), filter layers (a filter stack over everything below, or over
//! one layer as a stackable filter mask), transform masks (a non-destructive free, perspective,
//! warp, cage or liquify transform of a layer, keyable over the timeline) and the clones array
//! (a grid of clone layers) all live here. The onion-skin overlay of the animation module is a
//! live layer too, rendered by `anim_cmds`.
//!
//! A live layer is an ordinary raster layer carrying a [`Live`] record ([`layer_meta`]), so every
//! command that works on pixel layers (masks, blend modes, effects, export) works on it; only its
//! pixels are owned by the engine. [`refresh`] runs at the end of every `Session::edit` and after
//! a timeline move. Freezing a live layer drops the record and keeps the pixels.

use redrob_photo_color::{BlendMode, PixelFormat};
use redrob_photo_doc::{Document, Layer, LayerContent, LayerId, LayerMask, SmartFilter};
use redrob_photo_geom::{Affine, Rect};
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::layer_meta::{self, Key};
use crate::{EngineError, Result, Session};

/// The layer record key of a live layer.
pub const KEY: Key = *b"rLiv";

/// Interpolation between two transform keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Ease {
    #[default]
    Linear,
    /// Keep the earlier key until the next one.
    Hold,
    /// Smooth-step (bezier ease in and out).
    Smooth,
}

/// One keyed transform of a transform mask (animated transform masks).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformKey {
    pub frame: usize,
    /// Affine `[a, b, c, d, e, f]` (source → document pixels).
    pub m: [f64; 6],
    #[serde(default)]
    pub ease: Ease,
}

/// What a live layer derives its pixels from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Live {
    /// A live copy of `source`, moved by `offset`, through `filters`.
    Clone {
        source: u64,
        #[serde(default)]
        offset: [i32; 2],
        #[serde(default)]
        filters: Vec<SmartFilter>,
    },
    /// `filters` over everything below this layer (`source` none) or over one layer (a filter
    /// mask: the layer is clipped to that source and masked by its own layer mask).
    Filter {
        #[serde(default)]
        filters: Vec<SmartFilter>,
        #[serde(default)]
        source: Option<u64>,
    },
    /// A non-destructive transform of `source`: an affine or a perspective map, an optional
    /// warp mesh, then distortion filters (cage, liquify, puppet). `keys` animate the affine.
    Transform {
        source: u64,
        #[serde(default)]
        transform: Option<[f64; 6]>,
        #[serde(default)]
        perspective: Option<[f64; 9]>,
        #[serde(default)]
        warp: Option<redrob_photo_geom::warp::Warp>,
        #[serde(default)]
        filters: Vec<SmartFilter>,
        #[serde(default)]
        keys: Vec<TransformKey>,
    },
    /// Onion skin overlay of the animated layers (see `anim_cmds`).
    OnionSkin,
}

impl Live {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Live::Clone { .. } => "clone",
            Live::Filter { source: None, .. } => "filter",
            Live::Filter { source: Some(_), .. } => "filterMask",
            Live::Transform { .. } => "transformMask",
            Live::OnionSkin => "onionSkin",
        }
    }
}

/// The live record of a layer, if it is a live layer.
pub fn record(l: &Layer) -> Option<Live> {
    layer_meta::get(l, KEY)
}

/// Whether `l` is a live layer of any kind.
pub fn is_live(l: &Layer) -> bool {
    layer_meta::has(l, KEY)
}

// ---------- rendering ----------

/// A composite buffer as a surface in `fmt`.
pub(crate) fn buffer_to_surface(buf: &redrob_photo_compose::Buffer, fmt: PixelFormat) -> Surface {
    let n = fmt.channels();
    let mut data = vec![0.0f32; buf.px.len() * n];
    for (p, out) in buf.px.iter().zip(data.chunks_exact_mut(n)) {
        redrob_photo_raster::from_rgba_into(&fmt, *p, out);
    }
    let mut s = Surface::new(fmt);
    if !buf.rect.is_empty() {
        s.write_region(buf.rect, &data);
    }
    s.prune();
    s
}

/// One layer rendered alone over `canvas`, in `fmt`.
pub(crate) fn render_layer(l: &Layer, canvas: Rect, fmt: PixelFormat) -> Surface {
    let rect = match redrob_photo_compose::composite_bounds(l, canvas) {
        Some(r) => r.union(&canvas),
        None => canvas,
    };
    buffer_to_surface(&redrob_photo_compose::render_layer(l, rect), fmt)
}

/// Everything below layer `id` in its own sibling list, composited over `canvas`.
fn render_below(doc: &Document, id: LayerId, canvas: Rect, fmt: PixelFormat) -> Option<Surface> {
    let path = doc.path_of(id)?;
    let (&idx, parent) = path.split_last()?;
    let list: &[Layer] = if parent.is_empty() { &doc.layers } else { doc.layer_at(parent)?.children()? };
    let mut d = doc.clone();
    d.layers = list.get(..idx)?.to_vec();
    Some(buffer_to_surface(&redrob_photo_compose::render(&d, canvas), fmt))
}

/// `a` blended towards `b` by `t` (0 = a, 1 = b), per channel, over their joint bounds.
fn lerp_surfaces(a: &Surface, b: &Surface, t: f32) -> Surface {
    let area = a.content_bounds().union(&b.content_bounds());
    if area.is_empty() {
        return b.clone();
    }
    let pa = a.read_region(area);
    let pb = b.read_region(area);
    let px: Vec<f32> = pa.iter().zip(&pb).map(|(x, y)| x + (y - x) * t).collect();
    let mut out = Surface::new(a.format());
    out.write_region(area, &px);
    out.prune();
    out
}

/// The filter stack over `cur`: each visible filter in turn, faded in by its opacity.
pub(crate) fn apply_filters(mut cur: Surface, filters: &[SmartFilter], canvas: Rect) -> Surface {
    for f in filters.iter().filter(|f| f.visible) {
        let extent = canvas.union(&cur.content_bounds());
        let Some(out) = crate::filters::apply_filter_to_surface(&f.command, &f.params, &cur, canvas, None, extent) else { continue };
        let t = f.opacity.clamp(0.0, 1.0);
        cur = if f.blend == BlendMode::Normal && t >= 1.0 { out } else { lerp_surfaces(&cur, &out, t) };
    }
    cur
}

/// Smooth-step easing.
fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// The affine of a transform mask at `frame`: the stored transform, overridden by the keys
/// around `frame` when there are any.
pub fn transform_at(transform: Option<[f64; 6]>, keys: &[TransformKey], frame: usize) -> Affine {
    let base = Affine { m: transform.unwrap_or(Affine::IDENTITY.m) };
    if keys.is_empty() {
        return base;
    }
    let mut keys: Vec<&TransformKey> = keys.iter().collect();
    keys.sort_by_key(|k| k.frame);
    let before = keys.iter().rev().find(|k| k.frame <= frame);
    let after = keys.iter().find(|k| k.frame > frame);
    match (before, after) {
        (None, Some(k)) => Affine { m: k.m },
        (Some(k), None) => Affine { m: k.m },
        (Some(a), Some(b)) => {
            if a.ease == Ease::Hold || b.frame == a.frame {
                return Affine { m: a.m };
            }
            let mut t = (frame - a.frame) as f64 / (b.frame - a.frame) as f64;
            if a.ease == Ease::Smooth {
                t = smooth(t);
            }
            let mut m = [0.0; 6];
            for i in 0..6 {
                m[i] = a.m[i] + (b.m[i] - a.m[i]) * t;
            }
            Affine { m }
        }
        (None, None) => base,
    }
}

/// `src` transformed by the mask's map: the warp mesh and the affine or perspective in one pass.
fn transform_surface(src: &Surface, transform: &Affine, perspective: Option<[f64; 9]>, warp: Option<&redrob_photo_geom::warp::Warp>) -> Option<Surface> {
    let bounds = src.content_bounds();
    if bounds.is_empty() {
        return Some(src.clone());
    }
    match perspective {
        Some(h) => redrob_photo_algo::warp::place_source_projective(src, bounds, &redrob_photo_algo::transform::Homography(h), warp).ok(),
        None => redrob_photo_algo::warp::place_source(src, bounds, transform, warp).ok(),
    }
}

/// The pixels a live layer should show right now, or `None` when its source is gone.
fn render(doc: &Document, id: LayerId, live: &Live, canvas: Rect, fmt: PixelFormat) -> Option<Surface> {
    let frame = doc.timeline.as_ref().map_or(0, |t| t.current);
    match live {
        Live::Clone { source, offset, filters } => {
            let src = doc.layer(LayerId(*source)).filter(|l| l.id != id)?;
            let mut s = render_layer(src, canvas, fmt);
            if *offset != [0, 0] {
                s = redrob_photo_algo::resample::translate_surface(&s, offset[0], offset[1]);
            }
            Some(apply_filters(s, filters, canvas))
        }
        Live::Filter { filters, source: None } => Some(apply_filters(render_below(doc, id, canvas, fmt)?, filters, canvas)),
        Live::Filter { filters, source: Some(src) } => {
            let src = doc.layer(LayerId(*src)).filter(|l| l.id != id)?;
            Some(apply_filters(render_layer(src, canvas, fmt), filters, canvas))
        }
        Live::Transform { source, transform, perspective, warp, filters, keys } => {
            let src = doc.layer(LayerId(*source)).filter(|l| l.id != id)?;
            // The source is hidden while its transform mask shows it: render it as if visible.
            let mut shown = src.clone();
            shown.visible = true;
            let s = render_layer(&shown, canvas, fmt);
            let affine = transform_at(*transform, keys, frame);
            let s = transform_surface(&s, &affine, *perspective, warp.as_ref())?;
            Some(apply_filters(s, filters, canvas))
        }
        Live::OnionSkin => crate::anim_cmds::render_onion(doc, id, canvas, fmt),
    }
}

/// Re-derive every visible live layer from the document as it is now. Called at the end of each
/// edit and after the playhead moves; cheap when the document has no live layer.
pub fn refresh(doc: &mut Document) {
    let records: Vec<(LayerId, Live)> = layer_meta::layers_with(doc, KEY);
    if records.is_empty() {
        return;
    }
    let canvas = doc.bounds();
    let fmt = doc.pixel_format();
    for (id, live) in records {
        let Some(l) = doc.layer(id) else { continue };
        if !l.visible || !matches!(l.content, LayerContent::Raster(_)) {
            continue;
        }
        let Some(surface) = render(doc, id, &live, canvas, fmt) else { continue };
        if let Some(l) = doc.layer_mut(id)
            && let LayerContent::Raster(s) = &mut l.content
        {
            *s = surface;
        }
    }
}

// ---------- commands ----------

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    crate::active_layer_of(s).map(|_| ())
}

fn has_live(s: &Session) -> std::result::Result<(), String> {
    let l = crate::active_layer_of(s)?;
    if is_live(l) { Ok(()) } else { Err("the active layer is not a live layer".into()) }
}

fn has_transform_mask(s: &Session) -> std::result::Result<(), String> {
    let l = crate::active_layer_of(s)?;
    match record(l) {
        Some(Live::Transform { .. }) => Ok(()),
        _ => Err("the active layer is not a transform mask".into()),
    }
}

/// The `source` param (a layer id or name), else the active layer.
fn source_param(s: &Session, p: &Value, cmd: &str) -> Result<LayerId> {
    match p.get("source") {
        None | Some(Value::Null) => layer_param(s, &json!({})),
        Some(v) => {
            let q = json!({"layer": v});
            crate::commands::explicit_layer(s, &q)?.ok_or_else(|| bad(cmd, "`source` must be a layer id or name"))
        }
    }
}

/// A filter from `filter` (a command id) and `params`, as a smart filter record.
fn filter_param(p: &Value, cmd: &str) -> Result<Option<SmartFilter>> {
    let Some(f) = p.get("filter").and_then(Value::as_str).filter(|f| !f.is_empty()) else { return Ok(None) };
    if !f.starts_with("filter.") && !f.starts_with("image.adjustments.") && !f.starts_with("edit.") {
        return Err(bad(cmd, format!("`filter` must be a filter command id, not `{f}`")));
    }
    let params = p.get("params").cloned().unwrap_or_else(|| json!({}));
    let opacity = p.get("opacity").and_then(Value::as_f64).map_or(1.0, |o| (o / 100.0).clamp(0.0, 1.0) as f32);
    Ok(Some(SmartFilter { command: f.to_string(), params, blend: BlendMode::Normal, opacity, visible: true }))
}

fn affine_param(p: &Value, key: &str, cmd: &str) -> Result<Option<[f64; 6]>> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let a = v.as_array().filter(|a| a.len() == 6).ok_or_else(|| bad(cmd, format!("`{key}` is [a, b, c, d, e, f]")))?;
    let mut m = [0.0; 6];
    for (i, x) in a.iter().enumerate() {
        m[i] = x.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}` needs finite numbers")))?;
    }
    Ok(Some(m))
}

fn perspective_param(p: &Value, cmd: &str) -> Result<Option<[f64; 9]>> {
    let Some(v) = p.get("perspective").filter(|v| !v.is_null()) else { return Ok(None) };
    let a = v.as_array().filter(|a| a.len() == 9).ok_or_else(|| bad(cmd, "`perspective` is a row-major 3×3 matrix (9 numbers)"))?;
    let mut m = [0.0; 9];
    for (i, x) in a.iter().enumerate() {
        m[i] = x.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad(cmd, "`perspective` needs finite numbers"))?;
    }
    Ok(Some(m))
}

/// A new live layer above `above`, with the document's pixel format and a revealing mask.
fn insert_live(doc: &mut Document, above: Option<LayerId>, name: String, live: &Live, clipped: bool) -> LayerId {
    let mut l = Layer::raster(name, doc.pixel_format());
    l.clipped = clipped;
    l.mask = Some(LayerMask::reveal_all());
    layer_meta::set(&mut l, KEY, live);
    doc.insert_above(above, l)
}

/// Layer › New › Clone Layer: a live copy of the source (default: the active layer).
fn new_clone(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.new.cloneLayer";
    let src = source_param(s, p, cmd)?;
    let dx = crate::commands::int_i32(cmd, p, "dx")?.unwrap_or(0);
    let dy = crate::commands::int_i32(cmd, p, "dy")?.unwrap_or(0);
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let id = s.edit("New Clone Layer", |doc, active| {
        let base = doc.layer(src).ok_or(EngineError::NoLayer(src))?;
        let name = name.clone().unwrap_or_else(|| format!("{} clone", base.name));
        let live = Live::Clone { source: src.0, offset: [dx, dy], filters: Vec::new() };
        let id = insert_live(doc, Some(src), name, &live, false);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "source": src.0}))
}

/// Edit the live record of the target layer (default active) in one history step.
fn edit_live(s: &mut Session, p: &Value, label: &str, f: impl FnOnce(&mut Live) -> Result<()>) -> Result<Value> {
    let id = layer_param(s, p)?;
    let live = s.edit(label, |doc, _| {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let mut live: Live = layer_meta::get(l, KEY).ok_or_else(|| EngineError::Other(format!("layer \"{}\" is not a live layer", l.name)))?;
        f(&mut live)?;
        layer_meta::set(l, KEY, &live);
        Ok(live)
    })?;
    Ok(json!({"layer": id.0, "live": live}))
}

fn set_clone_source(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.cloneLayer.setSource";
    let new_src = source_param(s, p, cmd)?;
    let target = layer_param(s, p)?;
    if new_src == target {
        return Err(bad(cmd, "a clone layer cannot be its own source"));
    }
    edit_live(s, p, "Change Clone Source", |live| match live {
        Live::Clone { source, .. } | Live::Transform { source, .. } => {
            *source = new_src.0;
            Ok(())
        }
        Live::Filter { source: Some(source), .. } => {
            *source = new_src.0;
            Ok(())
        }
        _ => Err(bad(cmd, "the layer has no source to change")),
    })
}

fn set_clone_offset(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.cloneLayer.setOffset";
    let dx = crate::commands::int_i32(cmd, p, "dx")?;
    let dy = crate::commands::int_i32(cmd, p, "dy")?;
    edit_live(s, p, "Move Clone", |live| match live {
        Live::Clone { offset, .. } => {
            if let Some(dx) = dx {
                offset[0] = dx;
            }
            if let Some(dy) = dy {
                offset[1] = dy;
            }
            Ok(())
        }
        _ => Err(bad(cmd, "the layer is not a clone layer")),
    })
}

/// Layer › New › Filter Layer: `filter` applied non-destructively to everything below.
fn new_filter_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.new.filterLayer";
    let filter = filter_param(p, cmd)?.ok_or_else(|| bad(cmd, "needs `filter` (a filter command id)"))?;
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let id = s.edit("New Filter Layer", |doc, active| {
        let name = name.clone().unwrap_or_else(|| doc.next_layer_name("Filter Layer"));
        let live = Live::Filter { filters: vec![filter.clone()], source: None };
        let id = insert_live(doc, *active, name, &live, false);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0}))
}

/// Layer › New › Filter Mask: `filter` over one layer, clipped to it, limited by the new layer's
/// own (paintable) mask. Several stack.
fn new_filter_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.new.filterMask";
    let filter = filter_param(p, cmd)?.ok_or_else(|| bad(cmd, "needs `filter` (a filter command id)"))?;
    let src = source_param(s, p, cmd)?;
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let id = s.edit("New Filter Mask", |doc, active| {
        let base = doc.layer(src).ok_or(EngineError::NoLayer(src))?;
        if base.is_group() {
            return Err(bad(cmd, "a filter mask goes on a paint layer; use a filter layer inside the group"));
        }
        let name = name.clone().unwrap_or_else(|| format!("{} filter mask", base.name));
        // Stack above any filter masks the layer already has.
        let mut above = src;
        if let Some(path) = doc.path_of(src)
            && let (Some(&idx), parent) = (path.last(), &path[..path.len() - 1])
        {
            let list: &[Layer] = if parent.is_empty() { &doc.layers } else { doc.layer_at(parent).and_then(Layer::children).unwrap_or(&[]) };
            for l in list.iter().skip(idx + 1) {
                match record(l) {
                    Some(Live::Filter { source: Some(sid), .. }) if sid == src.0 => above = l.id,
                    _ => break,
                }
            }
        }
        let live = Live::Filter { filters: vec![filter.clone()], source: Some(src.0) };
        let id = insert_live(doc, Some(above), name, &live, true);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "source": src.0}))
}

/// Replace, add to or remove from a filter layer's / filter mask's / clone's filter stack.
fn edit_filters(s: &mut Session, p: &Value, op: &str) -> Result<Value> {
    let cmd = "layer.filterLayer.editFilters";
    let filter = filter_param(p, cmd)?;
    let index = p.get("index").and_then(Value::as_u64).map(|i| i as usize);
    edit_live(s, p, "Edit Filter Layer", |live| {
        let stack = match live {
            Live::Clone { filters, .. } | Live::Filter { filters, .. } | Live::Transform { filters, .. } => filters,
            Live::OnionSkin => return Err(bad(cmd, "the onion skin has no filters")),
        };
        match op {
            "set" => {
                let f = filter.ok_or_else(|| bad(cmd, "needs `filter`"))?;
                match index {
                    Some(i) if i < stack.len() => stack[i] = f,
                    Some(i) => return Err(bad(cmd, format!("no filter at index {i}"))),
                    None => *stack = vec![f],
                }
            }
            "add" => stack.push(filter.ok_or_else(|| bad(cmd, "needs `filter`"))?),
            "remove" => {
                let i = index.ok_or_else(|| bad(cmd, "needs `index`"))?;
                if i >= stack.len() {
                    return Err(bad(cmd, format!("no filter at index {i}")));
                }
                stack.remove(i);
            }
            _ => return Err(bad(cmd, "unknown operation")),
        }
        Ok(())
    })
}

/// Layer › New › Transform Mask: a non-destructive transform of the source, which is hidden
/// while the mask shows it transformed.
fn new_transform_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.new.transformMask";
    let src = source_param(s, p, cmd)?;
    let transform = affine_param(p, "transform", cmd)?;
    let perspective = perspective_param(p, cmd)?;
    let warp = match p.get("warp") {
        None | Some(Value::Null) => None,
        Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("`warp`: {e}")))?),
    };
    let filter = filter_param(p, cmd)?;
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let id = s.edit("New Transform Mask", |doc, active| {
        let base = doc.layer_mut(src).ok_or(EngineError::NoLayer(src))?;
        let name = name.clone().unwrap_or_else(|| format!("{} transform mask", base.name));
        base.visible = false;
        let live =
            Live::Transform { source: src.0, transform, perspective, warp: warp.clone(), filters: filter.clone().into_iter().collect(), keys: Vec::new() };
        let id = insert_live(doc, Some(src), name, &live, false);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "source": src.0}))
}

fn set_transform(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.transformMask.set";
    let transform = affine_param(p, "transform", cmd)?;
    let perspective = perspective_param(p, cmd)?;
    let warp_v = p.get("warp").cloned();
    let clear_perspective = p.get("perspective").is_some_and(Value::is_null);
    let clear_warp = p.get("warp").is_some_and(Value::is_null);
    edit_live(s, p, "Edit Transform Mask", |live| match live {
        Live::Transform { transform: t, perspective: h, warp: w, .. } => {
            if transform.is_some() {
                *t = transform;
            }
            if perspective.is_some() || clear_perspective {
                *h = perspective;
            }
            if clear_warp {
                *w = None;
            } else if let Some(v) = warp_v {
                *w = Some(serde_json::from_value(v).map_err(|e| bad(cmd, format!("`warp`: {e}")))?);
            }
            Ok(())
        }
        _ => Err(bad(cmd, "the layer is not a transform mask")),
    })
}

/// Key the transform mask's affine at a frame (default: the playhead).
fn add_key(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.transformMask.addKey";
    let frame = match p.get("frame").and_then(Value::as_u64) {
        Some(f) => f as usize,
        None => s.active().and_then(|d| d.doc.timeline.as_ref()).map_or(0, |t| t.current),
    };
    let m = affine_param(p, "transform", cmd)?;
    let ease = match p.get("ease").and_then(Value::as_str) {
        None | Some("linear") => Ease::Linear,
        Some("hold") => Ease::Hold,
        Some("smooth") => Ease::Smooth,
        Some(o) => return Err(bad(cmd, format!("unknown ease `{o}` (linear, hold, smooth)"))),
    };
    edit_live(s, p, "Add Transform Key", |live| match live {
        Live::Transform { transform, keys, .. } => {
            let m = m.or(*transform).unwrap_or(Affine::IDENTITY.m);
            keys.retain(|k| k.frame != frame);
            keys.push(TransformKey { frame, m, ease });
            keys.sort_by_key(|k| k.frame);
            Ok(())
        }
        _ => Err(bad(cmd, "the layer is not a transform mask")),
    })
}

fn remove_key(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.transformMask.removeKey";
    let frame = match p.get("frame").and_then(Value::as_u64) {
        Some(f) => f as usize,
        None => s.active().and_then(|d| d.doc.timeline.as_ref()).map_or(0, |t| t.current),
    };
    edit_live(s, p, "Remove Transform Key", |live| match live {
        Live::Transform { keys, .. } => {
            let n = keys.len();
            keys.retain(|k| k.frame != frame);
            if keys.len() == n {
                return Err(bad(cmd, format!("no key at frame {frame}")));
            }
            Ok(())
        }
        _ => Err(bad(cmd, "the layer is not a transform mask")),
    })
}

/// Drop a transform mask: its source shows again as it was.
fn remove_transform_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    s.edit("Remove Transform Mask", |doc, active| {
        let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let Some(Live::Transform { source, .. }) = record(l) else {
            return Err(EngineError::Other(format!("layer \"{}\" is not a transform mask", l.name)));
        };
        doc.remove(id);
        let src = LayerId(source);
        if let Some(sl) = doc.layer_mut(src) {
            sl.visible = true;
            *active = Some(src);
        }
        Ok(())
    })?;
    Ok(json!({"removed": id.0}))
}

/// Freeze a live layer: keep its pixels, drop the link to its source. A frozen transform mask
/// keeps its source hidden (it has become the layer's rasterized transform).
fn freeze(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = crate::layer_multi_cmds::targets(s, p)?;
    let mut frozen = Vec::new();
    s.edit("Freeze Live Layer", |doc, _| {
        for id in &ids {
            if let Some(l) = doc.layer_mut(*id)
                && layer_meta::remove(l, KEY)
            {
                frozen.push(id.0);
            }
        }
        Ok(())
    })?;
    Ok(json!({"frozen": frozen}))
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let list: Vec<Value> = st
        .doc
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| record(l).map(|r| json!({"layer": l.id.0, "name": l.name, "kind": r.kind_name(), "live": r})))
        .collect();
    match p.get("layer") {
        Some(_) => {
            let id = layer_param(s, p)?;
            let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            Ok(json!({"layer": id.0, "live": record(l)}))
        }
        None => Ok(json!({"liveLayers": list})),
    }
}

/// Layer › Clones Array: a grid of clone layers of the source in a group, each offset by its
/// column and row times the step, scaled by `distance` and turned by `angle` (degrees).
fn clones_array(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.clonesArray";
    let src = source_param(s, p, cmd)?;
    let num = |k: &str, d: i64| -> Result<i64> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(d),
            Some(v) => v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)).ok_or_else(|| bad(cmd, format!("`{k}` must be a number"))),
        }
    };
    let columns = num("columns", 2)?.clamp(1, 64) as i64;
    let rows = num("rows", 2)?.clamp(1, 64) as i64;
    let (dx, dy) = (num("dx", 0)?, num("dy", 0)?);
    let distance = p.get("distance").and_then(Value::as_f64).unwrap_or(1.0);
    let angle = p.get("angle").and_then(Value::as_f64).unwrap_or(0.0).to_radians();
    let include_source = p.get("includeSource").and_then(Value::as_bool).unwrap_or(false);
    if columns * rows > 1024 {
        return Err(bad(cmd, "at most 1024 clones"));
    }
    let (sin, cos) = angle.sin_cos();
    let (gid, n) = s.edit("Clones Array", |doc, active| {
        let base = doc.layer(src).ok_or(EngineError::NoLayer(src))?;
        let (dx, dy) = if dx == 0 && dy == 0 {
            // Default step: the source's own size, so clones tile without gaps.
            let r = redrob_photo_compose::composite_bounds(base, doc.bounds()).unwrap_or(doc.bounds());
            (r.width() as i64, r.height() as i64)
        } else {
            (dx, dy)
        };
        let name = base.name.clone();
        let mut children = Vec::new();
        for i in 0..rows {
            for j in 0..columns {
                if !include_source && i == 0 && j == 0 {
                    continue;
                }
                let (ox, oy) = ((j * dx) as f64 * distance, (i * dy) as f64 * distance);
                let (rx, ry) = (ox * cos - oy * sin, ox * sin + oy * cos);
                let mut l = Layer::raster(format!("{name} clone {}×{}", j + 1, i + 1), doc.pixel_format());
                layer_meta::set(&mut l, KEY, &Live::Clone { source: src.0, offset: [rx.round() as i32, ry.round() as i32], filters: Vec::new() });
                children.push(l);
            }
        }
        let n = children.len();
        let group = Layer::group(format!("{name} clones"), children);
        let gid = doc.insert_above(Some(src), group);
        *active = Some(gid);
        Ok((gid, n))
    })?;
    Ok(json!({"group": gid.0, "clones": n, "source": src.0}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    vec![
        spec!(
            "layer.new.cloneLayer",
            "Clone Layer",
            &["Layer", "New"],
            r##"{"source":id|name? (default active),"dx":int?,"dy":int?,"name":str?} → a live copy that follows its source"##,
            has_layer,
            new_clone
        ),
        spec!("layer.cloneLayer.setSource", "Change Clone Source…", &["Layer"], r##"{"layer":id?,"source":id|name}"##, has_live, set_clone_source),
        spec!(
            "layer.cloneLayer.setOffset",
            "Move Clone…",
            &["Layer"],
            r##"{"layer":id?,"dx":int?,"dy":int?} (absolute offset from the source)"##,
            has_live,
            set_clone_offset
        ),
        spec!(
            "layer.new.filterLayer",
            "Filter Layer…",
            &["Layer", "New"],
            r##"{"filter":"filter.blur.gaussian"|…,"params":{…}?,"opacity":0..100?,"name":str?} → the filter over everything below, with its own mask"##,
            has_doc,
            new_filter_layer
        ),
        spec!(
            "layer.new.filterMask",
            "Filter Mask…",
            &["Layer", "New"],
            r##"{"source":id|name? (the paint layer, default active),"filter":…,"params":{…}?,"opacity":0..100?,"name":str?} → the filter over that layer only, limited by its own paintable mask; stackable"##,
            has_layer,
            new_filter_mask
        ),
        spec!(
            "layer.filterLayer.setFilter",
            "Set Filter…",
            &["Layer"],
            r##"{"layer":id?,"filter":"filter.blur.gaussian"|…,"params":{…}?,"opacity":0..100?,"index":int? (default: replace the whole stack)}"##,
            has_live,
            |s, p| edit_filters(s, p, "set")
        ),
        spec!(
            "layer.filterLayer.addFilter",
            "Add Filter…",
            &["Layer"],
            r##"{"layer":id?,"filter":"filter.blur.gaussian"|…,"params":{…}?,"opacity":0..100?}"##,
            has_live,
            |s, p| edit_filters(s, p, "add")
        ),
        spec!("layer.filterLayer.removeFilter", "Remove Filter", &["Layer"], r##"{"layer":id?,"index":int}"##, has_live, |s, p| edit_filters(s, p, "remove")),
        spec!(
            "layer.new.transformMask",
            "Transform Mask…",
            &["Layer", "New"],
            r##"{"source":id|name? (default active),"transform":[a,b,c,d,e,f]?,"perspective":[9]?,"warp":{…}?,"filter":"filter.distort.liquify"|cage|puppet?,"params":{…}?,"name":str?} → the source shown transformed, editable"##,
            has_layer,
            new_transform_mask
        ),
        spec!(
            "layer.transformMask.set",
            "Edit Transform Mask…",
            &["Layer"],
            r##"{"layer":id?,"transform":[6]?,"perspective":[9]|null?,"warp":{…}|null?}"##,
            has_transform_mask,
            set_transform
        ),
        spec!(
            "layer.transformMask.addKey",
            "Add Transform Key",
            &["Layer"],
            r##"{"layer":id?,"frame":int? (default playhead),"transform":[6]? (default the mask's),"ease":"linear"|"hold"|"smooth"?} → tweened between keys"##,
            has_transform_mask,
            add_key
        ),
        spec!(
            "layer.transformMask.removeKey",
            "Remove Transform Key",
            &["Layer"],
            r##"{"layer":id?,"frame":int? (default playhead)}"##,
            has_transform_mask,
            remove_key
        ),
        spec!(
            "layer.transformMask.remove",
            "Remove Transform Mask",
            &["Layer"],
            r##"{"layer":id?} (the source shows again)"##,
            has_transform_mask,
            remove_transform_mask
        ),
        spec!("layer.liveLayer.freeze", "Freeze Live Layer", &["Layer"], r##"{"layer":id?} (keeps the pixels, drops the link)"##, has_live, freeze),
        spec!("layer.liveLayer.info", "Live Layer Info", &[], r##"{"layer":id?} → {live} or {liveLayers:[…]}"##, has_doc, info),
        spec!(
            "layer.clonesArray",
            "Clones Array…",
            &["Layer"],
            r##"{"source":id|name?,"columns":1..64=2,"rows":1..64=2,"dx":int? (default source width),"dy":int?,"distance":number=1,"angle":degrees=0,"includeSource":bool=false} → a group of clone layers"##,
            has_layer,
            clones_array
        ),
    ]
}
