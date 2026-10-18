// SPDX-License-Identifier: MIT OR Apache-2.0
//! Frame animation on the document timeline: raster keyframes per layer (blank, duplicated,
//! moved, copied, cloned; hold frames between keys), the onion-skin overlay, scalar animation
//! curves (opacity and transform-mask channels with bezier handles), importing image sequences
//! and video, rendering through ffmpeg (codec, size, frame rate, audio, HDR metadata), an audio
//! track, the storyboard and the timelapse recorder.
//!
//! An animated layer is a raster layer whose [`Layer::video`] frame stack holds one surface per
//! timeline frame and whose `rKey` record ([`Keys`]) says which frames are keyframes: the frames
//! after a key up to the next one hold it (they share its pixels, copy-on-write). Painting on a
//! hold frame edits its key, as [`before_sync`] writes the painted frame back to the key before
//! the playhead moves. Document-wide settings live in the `redrob.animation` record.

use redrob_photo_color::PixelFormat;
use redrob_photo_doc::{Document, Layer, LayerContent, LayerId, Timeline, VideoData};
use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::layer_meta::{self, Key};
use crate::live_layers::{self, Live};
use crate::{EngineError, Result, Session};

/// Layer record: which frames of an animated layer are keyframes.
pub const KEYS: Key = *b"rKey";
/// Layer record: animation curves.
pub const CURVES: Key = *b"rCrv";
/// Document record name.
pub const RECORD: &str = "animation";

const MAX_FRAMES: usize = 100_000;

// ---------- records ----------

/// Keyframes of an animated layer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Keys {
    /// Keyframe indices, sorted.
    pub keys: Vec<usize>,
    /// Clone frames: `(frame, key)` — the frame shows that key's pixels.
    #[serde(default)]
    pub clones: Vec<(usize, usize)>,
}

impl Keys {
    /// The key a frame shows: its own, the clone's key, or the latest key before it.
    pub fn key_of(&self, frame: usize) -> Option<usize> {
        if self.keys.contains(&frame) {
            return Some(frame);
        }
        if let Some((_, k)) = self.clones.iter().find(|(f, _)| *f == frame) {
            return Some(*k);
        }
        self.keys.iter().rev().find(|k| **k < frame).copied()
    }

    /// What a frame is in the frame table.
    pub fn status(&self, frame: usize) -> &'static str {
        if self.keys.contains(&frame) {
            "key"
        } else if self.clones.iter().any(|(f, _)| *f == frame) {
            "clone"
        } else if self.key_of(frame).is_some() {
            "hold"
        } else {
            "empty"
        }
    }
}

/// Onion skin settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Onion {
    pub enabled: bool,
    /// Frames shown before / after the playhead.
    pub before: usize,
    pub after: usize,
    /// Opacity per offset (index 0 = one frame away), percent.
    pub opacities: Vec<f32>,
    pub tint_before: [f32; 3],
    pub tint_after: [f32; 3],
    /// How far the frames are pushed towards their tint, percent.
    pub tint_strength: f32,
    /// The onion-skin live layer, while enabled.
    pub layer: Option<u64>,
}

impl Default for Onion {
    fn default() -> Self {
        Onion {
            enabled: false,
            before: 2,
            after: 2,
            opacities: vec![50.0, 30.0, 20.0, 15.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0],
            tint_before: [1.0, 0.0, 0.0],
            tint_after: [0.0, 0.4, 1.0],
            tint_strength: 50.0,
            layer: None,
        }
    }
}

/// The audio track attached to the animation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Audio {
    pub path: String,
    pub muted: bool,
    /// Where frame 0 falls in the audio, in frames (negative: the audio starts later).
    pub offset_frames: i64,
    /// Volume, percent.
    pub volume: f32,
}

/// One storyboard scene.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Scene {
    pub name: String,
    pub frame: usize,
    pub duration: usize,
    pub comment: String,
}

/// Timelapse recorder settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Recorder {
    pub enabled: bool,
    /// Where the snapshots go (PNG, numbered).
    pub dir: String,
    /// Capture every Nth edit.
    pub every: u32,
    /// Snapshot scale, percent.
    pub scale: f32,
    /// Edits seen since the last capture, and snapshots taken.
    pub pending: u32,
    pub count: u32,
}

impl Default for Recorder {
    fn default() -> Self {
        Recorder { enabled: false, dir: String::new(), every: 1, scale: 50.0, pending: 0, count: 0 }
    }
}

/// The document's animation record.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Animation {
    pub onion: Onion,
    pub audio: Option<Audio>,
    pub storyboard: Vec<Scene>,
    pub recorder: Recorder,
}

pub fn animation(doc: &Document) -> Animation {
    layer_meta::doc_get(doc, RECORD).unwrap_or_default()
}

pub fn set_animation(doc: &mut Document, a: &Animation) {
    layer_meta::doc_set(doc, RECORD, a);
}

/// The keyframe record of a layer, if it is an animated layer.
pub fn keys_of(l: &Layer) -> Option<Keys> {
    l.video.as_ref()?;
    layer_meta::get(l, KEYS)
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn current_frame(doc: &Document) -> usize {
    doc.timeline.as_ref().map_or(0, |t| t.current)
}

fn frame_param(doc: &Document, p: &Value, key: &str) -> usize {
    p.get(key).and_then(Value::as_u64).map_or(current_frame(doc), |f| (f as usize).min(MAX_FRAMES))
}

/// Make sure the document has a timeline of at least `frames` frames.
fn ensure_timeline(doc: &mut Document, frames: usize) -> f32 {
    if doc.timeline.is_none() {
        doc.timeline = Some(Timeline::new(frames.max(1), 24.0));
    }
    let Some(t) = doc.timeline.as_mut() else { return 24.0 };
    if t.duration < frames {
        let at_end = t.work_end >= t.duration;
        t.duration = frames;
        if at_end {
            t.work_end = frames;
        }
    }
    t.clamp();
    t.fps
}

// ---------- frame stacks ----------

/// Fill the hold and clone frames of an animated layer from their keys, and grow the stack to
/// `len` frames.
fn fill_holds(v: &mut VideoData, keys: &Keys, len: usize, fmt: PixelFormat) {
    while v.frames.len() < len {
        v.frames.push(Surface::new(fmt));
    }
    let n = v.frames.len();
    for f in 0..n {
        if keys.keys.contains(&f) {
            continue;
        }
        match keys.key_of(f) {
            Some(k) if k < n => v.frames[f] = v.frames[k].clone(),
            _ => v.frames[f] = Surface::new(fmt),
        }
    }
}

/// Before the playhead moves (after `video_cmds::store` wrote the displayed pixels into the
/// current frame): a painted hold or clone frame edits its key, then every hold follows its key.
pub fn before_sync(doc: &mut Document) {
    let cur = current_frame(doc);
    let fmt = doc.pixel_format();
    let len = doc.timeline.as_ref().map_or(1, |t| t.duration);
    let ids: Vec<(LayerId, Keys)> = layer_meta::layers_with(doc, KEYS);
    for (id, keys) in ids {
        let Some(l) = doc.layer_mut(id) else { continue };
        let Some(v) = l.video.as_mut() else { continue };
        if let Some(k) = keys.key_of(cur)
            && k != cur
            && k < v.frames.len()
            && cur < v.frames.len()
        {
            v.frames[k] = v.frames[cur].clone();
        }
        fill_holds(v, &keys, len, fmt);
    }
}

/// Turn a raster layer into an animated layer with one key at frame 0 holding its pixels.
fn animate_layer(doc: &mut Document, id: LayerId) -> Result<Keys> {
    let fmt = doc.pixel_format();
    let len = doc.timeline.as_ref().map_or(1, |t| t.duration);
    let fps = ensure_timeline(doc, len);
    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    if let Some(k) = layer_meta::get::<Keys>(l, KEYS)
        && l.video.is_some()
    {
        return Ok(k);
    }
    let LayerContent::Raster(s) = &l.content else {
        return Err(EngineError::Other(format!("layer \"{}\" is not a paint layer", l.name)));
    };
    let mut keys = Keys { keys: vec![0], clones: Vec::new() };
    let mut v = match l.video.take() {
        Some(v) => {
            // An existing video layer: every frame it has is a key.
            keys.keys = (0..v.frames.len()).collect();
            v
        }
        None => VideoData::new(vec![s.clone()], fps),
    };
    fill_holds(&mut v, &keys, len, fmt);
    l.video = Some(v);
    layer_meta::set(l, KEYS, &keys);
    Ok(keys)
}

/// Edit the keys and frames of the animated layer `p` names (default active), then re-sync.
fn edit_keys(s: &mut Session, p: &Value, label: &str, f: impl FnOnce(&mut Document, LayerId, &mut Keys, &mut VideoData, usize) -> Result<()>) -> Result<Value> {
    let id = layer_param(s, p)?;
    let keys = s.edit(label, |doc, _| {
        crate::video_cmds::store(doc);
        before_sync(doc);
        let mut keys = animate_layer(doc, id)?;
        let fmt = doc.pixel_format();
        let cur = current_frame(doc);
        let mut v = doc.layer_mut(id).and_then(|l| l.video.take()).ok_or(EngineError::NoLayer(id))?;
        let r = f(doc, id, &mut keys, &mut v, cur);
        keys.keys.sort_unstable();
        keys.keys.dedup();
        let len = keys.keys.iter().chain(keys.clones.iter().map(|(f, _)| f)).copied().max().map_or(1, |m| m + 1);
        let len = len.max(doc.timeline.as_ref().map_or(1, |t| t.duration));
        ensure_timeline(doc, len);
        fill_holds(&mut v, &keys, len, fmt);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        l.video = Some(v);
        layer_meta::set(l, KEYS, &keys);
        r?;
        crate::video_cmds::sync(doc);
        Ok(keys)
    })?;
    Ok(json!({"layer": id.0, "keys": keys.keys, "clones": keys.clones}))
}

/// Add a keyframe at `frame` (default playhead): blank, or a duplicate of the key shown there.
fn add_key(s: &mut Session, p: &Value, duplicate: bool) -> Result<Value> {
    let frame = s.active().map(|d| frame_param(&d.doc, p, "frame")).ok_or(EngineError::NoDocument)?;
    edit_keys(s, p, if duplicate { "Duplicate Keyframe" } else { "New Keyframe" }, |doc, _, keys, v, _| {
        let fmt = doc.pixel_format();
        let from = keys.key_of(frame).filter(|_| duplicate);
        let surface = from.and_then(|k| v.frames.get(k).cloned()).unwrap_or_else(|| Surface::new(fmt));
        while v.frames.len() <= frame {
            v.frames.push(Surface::new(fmt));
        }
        v.frames[frame] = surface;
        keys.clones.retain(|(f, _)| *f != frame);
        if !keys.keys.contains(&frame) {
            keys.keys.push(frame);
        }
        Ok(())
    })
}

fn remove_key(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "anim.keyframe.remove";
    let frame = s.active().map(|d| frame_param(&d.doc, p, "frame")).ok_or(EngineError::NoDocument)?;
    edit_keys(s, p, "Remove Keyframe", |_, _, keys, _, _| {
        let n = keys.keys.len() + keys.clones.len();
        keys.keys.retain(|k| *k != frame);
        keys.clones.retain(|(f, k)| *f != frame && *k != frame);
        if keys.keys.len() + keys.clones.len() == n {
            return Err(bad(cmd, format!("frame {frame} is not a keyframe")));
        }
        Ok(())
    })
}

/// Move a key (`from` → `to`), or copy it (a new independent key at `to`).
fn move_key(s: &mut Session, p: &Value, copy: bool) -> Result<Value> {
    let cmd = if copy { "anim.keyframe.copy" } else { "anim.keyframe.move" };
    let (from, to) = {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        (frame_param(&d.doc, p, "from"), p.get("to").and_then(Value::as_u64).map(|f| (f as usize).min(MAX_FRAMES)).ok_or_else(|| bad(cmd, "needs `to`"))?)
    };
    edit_keys(s, p, if copy { "Copy Keyframe" } else { "Move Keyframe" }, |doc, _, keys, v, _| {
        let k = keys.key_of(from).ok_or_else(|| bad(cmd, format!("frame {from} has no keyframe")))?;
        let surface = v.frames.get(k).cloned().unwrap_or_else(|| Surface::new(doc.pixel_format()));
        if !copy {
            keys.keys.retain(|x| *x != k);
            for (_, ck) in &mut keys.clones {
                if *ck == k {
                    *ck = to;
                }
            }
        }
        while v.frames.len() <= to {
            v.frames.push(Surface::new(doc.pixel_format()));
        }
        v.frames[to] = surface;
        keys.clones.retain(|(f, _)| *f != to);
        if !keys.keys.contains(&to) {
            keys.keys.push(to);
        }
        Ok(())
    })
}

/// A clone frame at `to` showing the key at `from` (dragging a frame with the clone modifier).
fn clone_key(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "anim.keyframe.clone";
    let (from, to) = {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        (frame_param(&d.doc, p, "from"), p.get("to").and_then(Value::as_u64).map(|f| (f as usize).min(MAX_FRAMES)).ok_or_else(|| bad(cmd, "needs `to`"))?)
    };
    edit_keys(s, p, "Clone Keyframe", |_, _, keys, _, _| {
        let k = keys.key_of(from).ok_or_else(|| bad(cmd, format!("frame {from} has no keyframe")))?;
        if k == to {
            return Err(bad(cmd, "a key cannot clone itself"));
        }
        keys.keys.retain(|x| *x != to);
        keys.clones.retain(|(f, _)| *f != to);
        keys.clones.push((to, k));
        Ok(())
    })
}

/// The frame table of the timeline docker: per animated layer, the status of every frame in the
/// range (key, hold, clone, empty) and which frames are cached (have pixels).
fn table(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = &st.doc;
    let t = doc.timeline.clone().unwrap_or_else(|| Timeline::new(1, 24.0));
    let start = p.get("start").and_then(Value::as_u64).map_or(0, |f| f as usize);
    let end = p.get("end").and_then(Value::as_u64).map_or(t.duration, |f| (f as usize).min(MAX_FRAMES)).max(start + 1);
    let mut layers = Vec::new();
    for (_, _, l) in doc.walk() {
        let Some(keys) = keys_of(l) else { continue };
        let frames: Vec<Value> = (start..end).map(|f| json!({"frame": f, "status": keys.status(f)})).collect();
        let cached: Vec<usize> =
            l.video.as_ref().map(|v| (0..v.frames.len()).filter(|f| !v.frames[*f].content_bounds().is_empty()).collect()).unwrap_or_default();
        layers.push(json!({"layer": l.id.0, "name": l.name, "keys": keys.keys, "clones": keys.clones, "frames": frames, "cached": cached}));
    }
    let a = animation(doc);
    Ok(json!({
        "fps": t.fps, "duration": t.duration, "current": t.current, "workStart": t.work_start, "workEnd": t.work_end,
        "layers": layers, "onionSkin": a.onion, "audio": a.audio, "storyboard": a.storyboard.len(),
    }))
}

/// Layer › Convert › Group to Animated Layer: each child, bottom to top, becomes one keyframe.
pub fn group_to_animated(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "anim.groupToAnimated";
    let id = layer_param(s, p)?;
    let (nid, n) = s.edit("Convert Group to Animated Layer", |doc, active| {
        let canvas = doc.bounds();
        let fmt = doc.pixel_format();
        let g = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let children = g.children().ok_or_else(|| bad(cmd, format!("layer \"{}\" is not a group", g.name)))?;
        if children.is_empty() {
            return Err(bad(cmd, "the group is empty"));
        }
        let frames: Vec<Surface> = children
            .iter()
            .map(|c| {
                let mut c = c.clone();
                c.visible = true;
                live_layers::render_layer(&c, canvas, fmt)
            })
            .collect();
        let n = frames.len();
        let name = g.name.clone();
        let fps = ensure_timeline(doc, n);
        let mut l = Layer::new(name, LayerContent::Raster(frames[0].clone()));
        l.video = Some(VideoData::new(frames, fps));
        layer_meta::set(&mut l, KEYS, &Keys { keys: (0..n).collect(), clones: Vec::new() });
        let nid = doc.insert_above(Some(id), l);
        doc.remove(id);
        *active = Some(nid);
        crate::video_cmds::sync(doc);
        Ok((nid, n))
    })?;
    Ok(json!({"layer": nid.0, "frames": n}))
}

// ---------- onion skin ----------

/// The onion-skin overlay: the key frames before and after the playhead of every animated layer,
/// tinted and faded by their distance.
pub fn render_onion(doc: &Document, me: LayerId, canvas: Rect, fmt: PixelFormat) -> Option<Surface> {
    let a = animation(doc);
    let o = &a.onion;
    if !o.enabled {
        return Some(Surface::new(fmt));
    }
    let cur = current_frame(doc);
    let (w, h) = (canvas.width() as usize, canvas.height() as usize);
    let mut acc = vec![[0.0f32; 4]; w * h];
    let mut tmp = vec![[0.0f32; 4]; w * h];
    let strength = (o.tint_strength / 100.0).clamp(0.0, 1.0);
    for (_, _, l) in doc.walk() {
        if l.id == me || !l.visible {
            continue;
        }
        let (Some(keys), Some(v)) = (keys_of(l), l.video.as_ref()) else { continue };
        let my_key = keys.key_of(cur);
        for off in 1..=o.before.max(o.after) {
            for dir in [-1i64, 1] {
                if (dir < 0 && off > o.before) || (dir > 0 && off > o.after) {
                    continue;
                }
                let f = cur as i64 + dir * off as i64;
                if f < 0 {
                    continue;
                }
                let Some(k) = keys.key_of(f as usize) else { continue };
                if Some(k) == my_key {
                    continue;
                }
                let Some(surf) = v.frames.get(k) else { continue };
                let opacity = o.opacities.get(off - 1).copied().unwrap_or(10.0) / 100.0;
                let tint = if dir < 0 { o.tint_before } else { o.tint_after };
                surf.read_rgba_into(canvas, &mut tmp);
                for (d, s) in acc.iter_mut().zip(&tmp) {
                    let a = s[3] * opacity;
                    if a <= 0.0 {
                        continue;
                    }
                    let c = [s[0] + (tint[0] - s[0]) * strength, s[1] + (tint[1] - s[1]) * strength, s[2] + (tint[2] - s[2]) * strength];
                    // Source over.
                    let out_a = a + d[3] * (1.0 - a);
                    for i in 0..3 {
                        d[i] = if out_a > 0.0 { (c[i] * a + d[i] * d[3] * (1.0 - a)) / out_a } else { 0.0 };
                    }
                    d[3] = out_a;
                }
            }
        }
    }
    Some(live_layers::buffer_to_surface(&redrob_photo_compose::Buffer { rect: canvas, px: acc }, fmt))
}

/// Onion skin settings; creates or removes the overlay layer as it is enabled or disabled.
fn onion_set(s: &mut Session, p: &Value) -> Result<Value> {
    let a = s.edit("Onion Skin", |doc, _| {
        let mut a = animation(doc);
        let o = &mut a.onion;
        if let Some(v) = p.get("enabled").and_then(Value::as_bool) {
            o.enabled = v;
        }
        if let Some(v) = p.get("before").and_then(Value::as_u64) {
            o.before = (v as usize).min(10);
        }
        if let Some(v) = p.get("after").and_then(Value::as_u64) {
            o.after = (v as usize).min(10);
        }
        if let Some(v) = p.get("opacities").and_then(Value::as_array) {
            o.opacities = v.iter().map(|x| x.as_f64().unwrap_or(10.0).clamp(0.0, 100.0) as f32).collect();
        }
        if let Some(v) = p.get("tintStrength").and_then(Value::as_f64) {
            o.tint_strength = v.clamp(0.0, 100.0) as f32;
        }
        for (key, slot) in [("tintBefore", &mut o.tint_before), ("tintAfter", &mut o.tint_after)] {
            if p.get(key).is_some() {
                let c = crate::commands::color_param(p, key, [slot[0], slot[1], slot[2], 1.0]);
                *slot = [c[0], c[1], c[2]];
            }
        }
        let existing = o.layer.map(LayerId).filter(|id| doc.layer(*id).is_some());
        match (o.enabled, existing) {
            (true, None) => {
                let mut l = Layer::raster("Onion Skin", doc.pixel_format());
                l.locks.pixels = true;
                layer_meta::set(&mut l, live_layers::KEY, &Live::OnionSkin);
                let top = doc.top_layer();
                o.layer = Some(doc.insert_above(top, l).0);
            }
            (false, Some(id)) => {
                doc.remove(id);
                o.layer = None;
            }
            (true, Some(id)) => o.layer = Some(id.0),
            (false, None) => o.layer = None,
        }
        set_animation(doc, &a);
        Ok(a)
    })?;
    Ok(json!(a.onion))
}

// ---------- animation curves ----------

/// A scalar key with optional bezier handles: `out` leaves this key, `in` arrives at the next,
/// each as `[frames, value]` offsets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScalarKey {
    pub frame: usize,
    pub value: f64,
    #[serde(default)]
    pub ease: live_layers::Ease,
    #[serde(default)]
    pub handle_out: Option<[f64; 2]>,
    #[serde(default)]
    pub handle_in: Option<[f64; 2]>,
}

/// One animated scalar channel of a layer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    /// `opacity` (percent), or a transform-mask channel: `x`, `y` (pixels), `scaleX`, `scaleY`
    /// (factor), `rotation` (degrees).
    pub name: String,
    pub keys: Vec<ScalarKey>,
}

/// The channels keyed on a layer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Curves {
    pub channels: Vec<Channel>,
}

pub const CHANNELS: [&str; 6] = ["opacity", "x", "y", "scaleX", "scaleY", "rotation"];

/// A cubic bezier from `p0` to `p3` with control points `p1`, `p2`, evaluated at `x` (bisection
/// on the monotone x parameter).
fn bezier_at(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], x: f64) -> f64 {
    let ev = |t: f64| {
        let u = 1.0 - t;
        let b = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        [b[0] * p0[0] + b[1] * p1[0] + b[2] * p2[0] + b[3] * p3[0], b[0] * p0[1] + b[1] * p1[1] + b[2] * p2[1] + b[3] * p3[1]]
    };
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if ev(mid)[0] < x { lo = mid } else { hi = mid }
    }
    ev((lo + hi) / 2.0)[1]
}

/// The channel's value at `frame`.
pub fn channel_at(ch: &Channel, frame: usize) -> Option<f64> {
    let mut keys: Vec<&ScalarKey> = ch.keys.iter().collect();
    keys.sort_by_key(|k| k.frame);
    let a = keys.iter().rev().find(|k| k.frame <= frame);
    let b = keys.iter().find(|k| k.frame > frame);
    Some(match (a, b) {
        (None, None) => return None,
        (None, Some(k)) | (Some(k), None) => k.value,
        (Some(a), Some(b)) => {
            if a.ease == live_layers::Ease::Hold {
                return Some(a.value);
            }
            let (x0, x1) = (a.frame as f64, b.frame as f64);
            let x = frame as f64;
            match (a.handle_out, b.handle_in) {
                (None, None) => {
                    let mut t = (x - x0) / (x1 - x0);
                    if a.ease == live_layers::Ease::Smooth {
                        t = t * t * (3.0 - 2.0 * t);
                    }
                    a.value + (b.value - a.value) * t
                }
                (ho, hi) => {
                    let third = (x1 - x0) / 3.0;
                    let ho = ho.unwrap_or([third, 0.0]);
                    let hi = hi.unwrap_or([-third, 0.0]);
                    let p1 = [(x0 + ho[0]).clamp(x0, x1), a.value + ho[1]];
                    let p2 = [(x1 + hi[0]).clamp(x0, x1), b.value + hi[1]];
                    bezier_at([x0, a.value], p1, p2, [x1, b.value], x)
                }
            }
        }
    })
}

/// Apply every layer's curves at the playhead: opacity, and the transform of a transform mask
/// built from its x / y / scale / rotation channels. Runs before the live layers refresh.
pub fn apply_curves(doc: &mut Document) {
    let frame = current_frame(doc);
    let recs: Vec<(LayerId, Curves)> = layer_meta::layers_with(doc, CURVES);
    for (id, c) in recs {
        let Some(l) = doc.layer_mut(id) else { continue };
        let value = |name: &str| c.channels.iter().find(|ch| ch.name == name).and_then(|ch| channel_at(ch, frame));
        if let Some(o) = value("opacity") {
            l.opacity = (o / 100.0).clamp(0.0, 1.0) as f32;
        }
        let geo = [value("x"), value("y"), value("scaleX"), value("scaleY"), value("rotation")];
        if geo.iter().any(Option::is_some)
            && let Some(Live::Transform { source, transform, perspective, warp, filters, keys }) = live_layers::record(l)
        {
            let base = transform.unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
            let (sx, sy) = (geo[2].unwrap_or(1.0), geo[3].unwrap_or(1.0));
            let (sin, cos) = geo[4].unwrap_or(0.0).to_radians().sin_cos();
            let m = [cos * sx, sin * sx, -sin * sy, cos * sy, geo[0].unwrap_or(base[4]), geo[1].unwrap_or(base[5])];
            layer_meta::set(l, live_layers::KEY, &Live::Transform { source, transform: Some(m), perspective, warp, filters, keys });
        }
    }
}

fn curve_key(s: &mut Session, p: &Value, remove: bool) -> Result<Value> {
    let cmd = if remove { "anim.curves.removeKey" } else { "anim.curves.addKey" };
    let name = p.get("channel").and_then(Value::as_str).ok_or_else(|| bad(cmd, "needs `channel`"))?.to_string();
    if !CHANNELS.contains(&name.as_str()) {
        return Err(bad(cmd, format!("unknown channel `{name}` ({})", CHANNELS.join(", "))));
    }
    let frame = s.active().map(|d| frame_param(&d.doc, p, "frame")).ok_or(EngineError::NoDocument)?;
    let value = p.get("value").and_then(Value::as_f64);
    let handle = |k: &str| -> Result<Option<[f64; 2]>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => {
                let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad(cmd, format!("`{k}` is [frames, value]")))?;
                Ok(Some([a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)]))
            }
        }
    };
    let (handle_out, handle_in) = (handle("handleOut")?, handle("handleIn")?);
    let ease = match p.get("ease").and_then(Value::as_str) {
        None | Some("linear") => live_layers::Ease::Linear,
        Some("hold") => live_layers::Ease::Hold,
        Some("smooth") => live_layers::Ease::Smooth,
        Some(o) => return Err(bad(cmd, format!("unknown ease `{o}`"))),
    };
    let id = layer_param(s, p)?;
    let curves = s.edit(if remove { "Remove Curve Key" } else { "Add Curve Key" }, |doc, _| {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let mut c: Curves = layer_meta::get(l, CURVES).unwrap_or_default();
        let value = match value {
            Some(v) => v,
            None if remove => 0.0,
            None if name == "opacity" => f64::from(l.opacity) * 100.0,
            None => return Err(bad(cmd, "needs `value`")),
        };
        let ch = match c.channels.iter().position(|ch| ch.name == name) {
            Some(i) => &mut c.channels[i],
            None => {
                c.channels.push(Channel { name: name.clone(), keys: Vec::new() });
                c.channels.last_mut().expect("just pushed")
            }
        };
        let n = ch.keys.len();
        ch.keys.retain(|k| k.frame != frame);
        if remove {
            if ch.keys.len() == n {
                return Err(bad(cmd, format!("no key at frame {frame}")));
            }
        } else {
            ch.keys.push(ScalarKey { frame, value, ease, handle_out, handle_in });
            ch.keys.sort_by_key(|k| k.frame);
        }
        c.channels.retain(|ch| !ch.keys.is_empty());
        if c.channels.is_empty() {
            layer_meta::remove(l, CURVES);
        } else {
            layer_meta::set(l, CURVES, &c);
        }
        apply_curves(doc);
        Ok(c)
    })?;
    Ok(json!({"layer": id.0, "curves": curves}))
}

fn curves_list(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let frame = current_frame(&st.doc);
    let samples = p.get("samples").and_then(Value::as_bool).unwrap_or(false);
    let t = st.doc.timeline.clone().unwrap_or_else(|| Timeline::new(1, 24.0));
    let list: Vec<Value> = st
        .doc
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| layer_meta::get::<Curves>(l, CURVES).map(|c| (l, c)))
        .map(|(l, c)| {
            let channels: Vec<Value> = c
                .channels
                .iter()
                .map(|ch| {
                    let values: Vec<Value> = if samples { (0..t.duration).map(|f| json!(channel_at(ch, f))).collect() } else { Vec::new() };
                    json!({"name": ch.name, "keys": ch.keys, "now": channel_at(ch, frame), "values": values})
                })
                .collect();
            json!({"layer": l.id.0, "name": l.name, "channels": channels})
        })
        .collect();
    Ok(json!({"frame": frame, "layers": list}))
}

// ---------- ffmpeg ----------

#[cfg(not(target_arch = "wasm32"))]
fn tool(name: &str) -> String {
    std::env::var(format!("REDROB_{}", name.to_ascii_uppercase())).unwrap_or_else(|_| name.to_string())
}

/// Run an external tool, with its stderr tail in the error.
#[cfg(not(target_arch = "wasm32"))]
fn run_tool(name: &str, args: &[String]) -> Result<()> {
    let out = std::process::Command::new(tool(name))
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| EngineError::Other(format!("{name} could not be started ({e}); install it or point REDROB_{} at it", name.to_ascii_uppercase())))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let mut tail: Vec<&str> = err.lines().rev().take(6).collect();
    tail.reverse();
    Err(EngineError::Other(format!("{name} failed ({}): {}", out.status, tail.join(" | "))))
}

#[cfg(target_arch = "wasm32")]
fn run_tool(name: &str, _args: &[String]) -> Result<()> {
    Err(EngineError::Other(format!("{name} is not available on the web")))
}

/// Start an external tool without waiting (audio playback).
#[cfg(not(target_arch = "wasm32"))]
fn spawn_tool(name: &str, args: &[String]) -> Result<()> {
    std::process::Command::new(tool(name))
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| EngineError::Other(format!("{name} could not be started ({e})")))
}

#[cfg(target_arch = "wasm32")]
fn spawn_tool(name: &str, _args: &[String]) -> Result<()> {
    Err(EngineError::Other(format!("{name} is not available on the web")))
}

/// A fresh scratch directory for frame files.
fn scratch_dir(tag: &str) -> Result<std::path::PathBuf> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("redrob-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| EngineError::Other(format!("{}: {e}", dir.display())))?;
    Ok(dir)
}

fn list_dir_sorted(dir: &std::path::Path, ext: &str) -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext))).collect())
        .unwrap_or_default();
    v.sort();
    v
}

// ---------- import ----------

/// Decode one image file into a surface in `fmt`, placed at the origin.
fn decode_frame(path: &str, fmt: PixelFormat) -> Result<Surface> {
    let bytes = crate::file_cmds::read_file(path)?;
    let imported = redrob_photo_io::import(path, &bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(crate::file_cmds::flattened(&imported.document, fmt))
}

/// Import an image sequence as an animated layer: one key per image, `step` frames apart,
/// starting at `start`; ordered by name, by modification time or as given.
fn import_frames(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "anim.importFrames";
    let paths: Vec<String> = match p.get("paths") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(dir)) => list_dir_sorted(std::path::Path::new(dir), p.get("ext").and_then(Value::as_str).unwrap_or("png"))
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        _ => return Err(bad(cmd, "needs `paths` (a list of files, or a directory)")),
    };
    if paths.is_empty() {
        return Err(bad(cmd, "no frames to import"));
    }
    import_paths(s, p, &paths, "Import Animation Frames")
}

fn import_paths(s: &mut Session, p: &Value, paths: &[String], label: &str) -> Result<Value> {
    let cmd = "anim.importFrames";
    let mut paths: Vec<String> = paths.to_vec();
    match p.get("order").and_then(Value::as_str).unwrap_or("name") {
        "given" => {}
        "name" => paths.sort(),
        "mtime" => paths.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()),
        o => return Err(bad(cmd, format!("unknown order `{o}` (name, mtime, given)"))),
    }
    if p.get("reverse").and_then(Value::as_bool).unwrap_or(false) {
        paths.reverse();
    }
    let step = p.get("step").and_then(Value::as_u64).unwrap_or(1).clamp(1, 1000) as usize;
    let start = p.get("start").and_then(Value::as_u64).unwrap_or(0) as usize;
    let fps = p.get("fps").and_then(Value::as_f64).map(|f| f as f32);
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let mut frames = Vec::with_capacity(paths.len());
    for path in &paths {
        frames.push(decode_frame(path, fmt)?);
    }
    let (id, n) = s.edit(label, |doc, active| {
        let len = start + (frames.len() - 1) * step + 1;
        let cur_fps = ensure_timeline(doc, len);
        if let (Some(f), Some(t)) = (fps, doc.timeline.as_mut()) {
            t.fps = f;
            t.clamp();
        }
        let fps = fps.unwrap_or(cur_fps);
        let name = name
            .clone()
            .unwrap_or_else(|| paths.first().map(|p| crate::file_cmds::stem(p)).filter(|n| !n.is_empty()).unwrap_or_else(|| doc.next_layer_name("Animation")));
        let keys = Keys { keys: (0..frames.len()).map(|i| start + i * step).collect(), clones: Vec::new() };
        let mut stack = vec![Surface::new(fmt); len];
        for (i, f) in frames.iter().enumerate() {
            stack[start + i * step] = f.clone();
        }
        let mut v = VideoData::new(stack, fps);
        v.source = redrob_photo_doc::VideoSource::File { path: paths.first().cloned().unwrap_or_default() };
        fill_holds(&mut v, &keys, len, fmt);
        let mut l = Layer::new(name, LayerContent::Raster(Surface::new(fmt)));
        l.video = Some(v);
        layer_meta::set(&mut l, KEYS, &keys);
        let id = doc.insert_above(*active, l);
        *active = Some(id);
        crate::video_cmds::sync(doc);
        Ok((id, frames.len()))
    })?;
    Ok(json!({"layer": id.0, "frames": n}))
}

/// Import a video through ffmpeg: frames extracted at the document's (or given) frame rate,
/// scaled to `scale` percent or `width`×`height`, between `from` and `to` seconds.
fn import_video(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "anim.importVideo";
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad(cmd, "needs `path`"))?.to_string();
    let fps = p.get("fps").and_then(Value::as_f64).or_else(|| s.active().and_then(|d| d.doc.timeline.as_ref()).map(|t| f64::from(t.fps))).unwrap_or(24.0);
    let dir = scratch_dir("import")?;
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into(), "-y".into()];
    if let Some(from) = p.get("from").and_then(Value::as_f64) {
        args.extend(["-ss".to_string(), format!("{from}")]);
    }
    args.extend(["-i".to_string(), path.clone()]);
    if let Some(to) = p.get("to").and_then(Value::as_f64) {
        args.extend(["-to".to_string(), format!("{to}")]);
    }
    let mut vf = vec![format!("fps={fps}")];
    match (p.get("width").and_then(Value::as_u64), p.get("height").and_then(Value::as_u64), p.get("scale").and_then(Value::as_f64)) {
        (Some(w), Some(h), _) => vf.push(format!("scale={w}:{h}")),
        (Some(w), None, _) => vf.push(format!("scale={w}:-2")),
        (None, Some(h), _) => vf.push(format!("scale=-2:{h}")),
        (None, None, Some(k)) if (k - 100.0).abs() > 1e-6 => vf.push(format!("scale=iw*{}:ih*{}", k / 100.0, k / 100.0)),
        _ => {}
    }
    args.extend(["-vf".to_string(), vf.join(","), dir.join("f%06d.png").to_string_lossy().into_owned()]);
    run_tool("ffmpeg", &args)?;
    let files: Vec<String> = list_dir_sorted(&dir, "png").into_iter().map(|p| p.to_string_lossy().into_owned()).collect();
    if files.is_empty() {
        return Err(EngineError::Other(format!("{path}: ffmpeg produced no frames")));
    }
    let mut q = p.clone();
    q["order"] = json!("name");
    q["fps"] = json!(fps);
    if q.get("name").is_none() {
        q["name"] = json!(crate::file_cmds::stem(&path));
    }
    let r = import_paths(s, &q, &files, "Import Video");
    let _ = std::fs::remove_dir_all(&dir);
    r
}

// ---------- render ----------

/// The composite of frame `f` as PNG bytes, with the live layers and curves of that frame,
/// resized to `width` × `height` when given (one of them keeps the aspect ratio).
fn render_frame_png(base: &Document, f: usize, width: Option<u32>, height: Option<u32>) -> Result<Vec<u8>> {
    let mut doc = base.clone();
    if let Some(t) = &mut doc.timeline {
        t.current = f;
    }
    crate::video_cmds::sync(&mut doc);
    apply_curves(&mut doc);
    live_layers::refresh(&mut doc);
    let fmt = doc.pixel_format();
    let mut surf = crate::file_cmds::flattened(&doc, fmt);
    let bounds = doc.bounds();
    let (bw, bh) = (bounds.width().max(1), bounds.height().max(1));
    let (w, h) = match (width, height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, (f64::from(w) * f64::from(bh) / f64::from(bw)).round() as u32),
        (None, Some(h)) => ((f64::from(h) * f64::from(bw) / f64::from(bh)).round() as u32, h),
        (None, None) => (bw, bh),
    };
    let (w, h) = (w.max(1), h.max(1));
    let mut out = Document::new(doc.name.clone(), redrob_photo_doc::Size::new(w, h), doc.mode, doc.depth);
    if (w, h) != (bw, bh) {
        let (sx, sy) = (f64::from(w) / f64::from(bw), f64::from(h) / f64::from(bh));
        surf = redrob_photo_algo::resample::resize_surface(&surf, sx, sy, redrob_photo_algo::resample::Resample::Bicubic);
    }
    out.layers.push(Layer::new("Frame", LayerContent::Raster(surf)));
    redrob_photo_io::export(&out, "png", &redrob_photo_io::ExportOptions::default()).map(|r| r.bytes).map_err(|e| EngineError::Other(format!("frame {f}: {e}")))
}

/// Render Animation: an image sequence, or a video through ffmpeg with codec, size, frame rate,
/// the audio track and HDR metadata.
fn render_animation(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "anim.render";
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad(cmd, "needs `path` (a video file, or a directory for an image sequence)"))?.to_string();
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let mut base = (*st.doc).clone();
    crate::video_cmds::store(&mut base);
    before_sync(&mut base);
    let t = base.timeline.clone().unwrap_or_else(|| Timeline::new(1, 24.0));
    let start = p.get("start").and_then(Value::as_u64).map_or(t.work_start, |f| f as usize);
    let end = p.get("end").and_then(Value::as_u64).map_or(t.work_end, |f| (f as usize).min(MAX_FRAMES)).max(start + 1).min(t.duration.max(start + 1));
    let fps = p.get("fps").and_then(Value::as_f64).unwrap_or(f64::from(t.fps));
    let width = p.get("width").and_then(Value::as_u64).map(|w| w as u32);
    let height = p.get("height").and_then(Value::as_u64).map(|h| h as u32);
    let format = p.get("format").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| {
        std::path::Path::new(&path).extension().map(|e| e.to_string_lossy().to_lowercase()).filter(|e| !e.is_empty()).unwrap_or_else(|| "png".into())
    });
    let sequence = matches!(format.as_str(), "png" | "jpg" | "jpeg" | "tif" | "tiff" | "webp");
    let stem = base.name.rsplit_once('.').map_or(base.name.as_str(), |(a, _)| a).to_string();
    let a = animation(&base);
    let frames_dir = if sequence { std::path::PathBuf::from(&path) } else { scratch_dir("render")? };
    std::fs::create_dir_all(&frames_dir).map_err(|e| EngineError::Other(format!("{}: {e}", frames_dir.display())))?;
    let mut files = Vec::new();
    for f in start..end {
        let png = render_frame_png(&base, f, width, height)?;
        let bytes = if sequence && format != "png" {
            // Other sequence formats go through the export codecs from the rendered PNG.
            let imported = redrob_photo_io::import("frame.png", &png).map_err(|e| EngineError::Other(e.to_string()))?;
            redrob_photo_io::export(&imported.document, &format, &redrob_photo_io::ExportOptions::default())
                .map(|r| r.bytes)
                .map_err(|e| EngineError::Other(e.to_string()))?
        } else {
            png
        };
        let file = frames_dir.join(format!("{stem}_{f:05}.{}", if sequence { format.as_str() } else { "png" }));
        crate::file_cmds::write_file(&file.to_string_lossy(), &bytes)?;
        files.push(file.to_string_lossy().into_owned());
    }
    if sequence {
        return Ok(json!({"frames": files.len(), "dir": path, "files": files}));
    }
    // Video through ffmpeg.
    let codec = p.get("codec").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| {
        match format.as_str() {
            "webm" => "libvpx-vp9",
            "mov" => "prores_ks",
            "gif" => "gif",
            _ => "libx264",
        }
        .into()
    });
    let hdr = p.get("hdr").and_then(Value::as_bool).unwrap_or(false);
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-framerate".into(),
        format!("{fps}"),
        "-start_number".into(),
        format!("{start}"),
    ];
    args.extend(["-i".to_string(), frames_dir.join(format!("{stem}_%05d.png")).to_string_lossy().into_owned()]);
    let with_audio = p.get("audio").and_then(Value::as_bool).unwrap_or(true);
    let audio = a.audio.as_ref().filter(|au| with_audio && !au.muted && !au.path.is_empty());
    if let Some(au) = audio {
        let offset = (au.offset_frames as f64 + start as f64) / fps;
        if offset >= 0.0 {
            args.extend(["-ss".to_string(), format!("{offset}")]);
        } else {
            args.extend(["-itsoffset".to_string(), format!("{}", -offset)]);
        }
        args.extend(["-i".to_string(), au.path.clone(), "-shortest".into(), "-filter:a".into(), format!("volume={}", au.volume.max(0.0) / 100.0)]);
    }
    args.extend(["-c:v".to_string(), codec.clone(), "-r".into(), format!("{fps}")]);
    if let Some(crf) = p.get("crf").and_then(Value::as_u64) {
        args.extend(["-crf".to_string(), crf.to_string()]);
    }
    if let Some(b) = p.get("bitrate").and_then(Value::as_str) {
        args.extend(["-b:v".to_string(), b.to_string()]);
    }
    if hdr {
        args.extend([
            "-pix_fmt".to_string(),
            "yuv420p10le".into(),
            "-color_primaries".into(),
            "bt2020".into(),
            "-color_trc".into(),
            "smpte2084".into(),
            "-colorspace".into(),
            "bt2020nc".into(),
        ]);
    } else if codec.starts_with("libx26") || codec == "libvpx-vp9" {
        args.extend(["-pix_fmt".to_string(), "yuv420p".into()]);
    }
    if let Some(extra) = p.get("ffmpegArgs").and_then(Value::as_array) {
        args.extend(extra.iter().filter_map(Value::as_str).map(str::to_string));
    }
    args.push(path.clone());
    let r = run_tool("ffmpeg", &args);
    let _ = std::fs::remove_dir_all(&frames_dir);
    r?;
    Ok(json!({"file": path, "frames": files.len(), "codec": codec, "fps": fps, "audio": audio.is_some(), "hdr": hdr}))
}

// ---------- audio ----------

fn edit_animation(s: &mut Session, label: &str, f: impl FnOnce(&mut Animation) -> Result<()>) -> Result<Animation> {
    s.edit(label, |doc, _| {
        let mut a = animation(doc);
        f(&mut a)?;
        set_animation(doc, &a);
        Ok(a)
    })
}

fn audio_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "anim.audio.set";
    let path = p.get("path").and_then(Value::as_str).map(str::to_string);
    let a = edit_animation(s, "Attach Audio", |a| {
        let au = a.audio.get_or_insert_with(|| Audio { volume: 100.0, ..Default::default() });
        if let Some(path) = path {
            if !std::path::Path::new(&path).exists() {
                return Err(bad(cmd, format!("{path}: no such file")));
            }
            au.path = path;
        }
        if au.path.is_empty() {
            return Err(bad(cmd, "needs `path`"));
        }
        if let Some(m) = p.get("muted").and_then(Value::as_bool) {
            au.muted = m;
        }
        if let Some(o) = p.get("offsetFrames").and_then(Value::as_i64) {
            au.offset_frames = o;
        }
        if let Some(v) = p.get("volume").and_then(Value::as_f64) {
            au.volume = v.clamp(0.0, 400.0) as f32;
        }
        Ok(())
    })?;
    Ok(json!(a.audio))
}

fn audio_mute(s: &mut Session, p: &Value) -> Result<Value> {
    let a = edit_animation(s, "Mute Audio", |a| {
        let au = a.audio.as_mut().ok_or_else(|| EngineError::Other("no audio track".into()))?;
        au.muted = p.get("muted").and_then(Value::as_bool).unwrap_or(!au.muted);
        Ok(())
    })?;
    Ok(json!(a.audio))
}

fn audio_remove(s: &mut Session, _p: &Value) -> Result<Value> {
    edit_animation(s, "Remove Audio", |a| {
        a.audio = None;
        Ok(())
    })?;
    Ok(json!({"audio": Value::Null}))
}

/// Play the audio under a frame range (scrubbing: one frame; playback: the work area) through
/// ffplay, without blocking.
fn audio_scrub(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let a = animation(&st.doc);
    let au = a.audio.filter(|au| !au.muted).ok_or_else(|| EngineError::Other("no audible audio track".into()))?;
    let t = st.doc.timeline.clone().unwrap_or_else(|| Timeline::new(1, 24.0));
    let frame = frame_param(&st.doc, p, "frame");
    let frames = p.get("frames").and_then(Value::as_u64).unwrap_or(1).max(1) as f64;
    let fps = f64::from(t.fps.max(1.0));
    let from = (frame as f64 + au.offset_frames as f64) / fps;
    if from < 0.0 {
        return Ok(json!({"played": false, "reason": "before the audio starts"}));
    }
    let args: Vec<String> = vec![
        "-nodisp".into(),
        "-autoexit".into(),
        "-loglevel".into(),
        "quiet".into(),
        "-ss".into(),
        format!("{from}"),
        "-t".into(),
        format!("{}", frames / fps),
        "-volume".into(),
        format!("{}", au.volume.clamp(0.0, 100.0).round() as i64),
        au.path.clone(),
    ];
    spawn_tool("ffplay", &args)?;
    Ok(json!({"played": true, "from": from, "seconds": frames / fps}))
}

// ---------- storyboard ----------

fn storyboard_add(s: &mut Session, p: &Value) -> Result<Value> {
    let (frame, count) = s.active().map(|d| (frame_param(&d.doc, p, "frame"), animation(&d.doc).storyboard.len())).ok_or(EngineError::NoDocument)?;
    let scene = Scene {
        name: p.get("name").and_then(Value::as_str).map_or_else(|| format!("Scene {}", count + 1), str::to_string),
        frame,
        duration: p.get("duration").and_then(Value::as_u64).unwrap_or(24).clamp(1, MAX_FRAMES as u64) as usize,
        comment: p.get("comment").and_then(Value::as_str).unwrap_or("").to_string(),
    };
    let a = edit_animation(s, "Add Scene", |a| {
        a.storyboard.push(scene);
        a.storyboard.sort_by_key(|sc| sc.frame);
        Ok(())
    })?;
    Ok(json!({"storyboard": a.storyboard}))
}

fn storyboard_update(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "anim.storyboard.update";
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "needs `index`"))? as usize;
    let a = edit_animation(s, "Edit Scene", |a| {
        let sc = a.storyboard.get_mut(index).ok_or_else(|| bad(cmd, format!("no scene {index}")))?;
        if let Some(n) = p.get("name").and_then(Value::as_str) {
            sc.name = n.to_string();
        }
        if let Some(f) = p.get("frame").and_then(Value::as_u64) {
            sc.frame = (f as usize).min(MAX_FRAMES);
        }
        if let Some(d) = p.get("duration").and_then(Value::as_u64) {
            sc.duration = d.clamp(1, MAX_FRAMES as u64) as usize;
        }
        if let Some(c) = p.get("comment").and_then(Value::as_str) {
            sc.comment = c.to_string();
        }
        a.storyboard.sort_by_key(|sc| sc.frame);
        Ok(())
    })?;
    Ok(json!({"storyboard": a.storyboard}))
}

fn storyboard_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "anim.storyboard.remove";
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "needs `index`"))? as usize;
    let a = edit_animation(s, "Remove Scene", |a| {
        if index >= a.storyboard.len() {
            return Err(bad(cmd, format!("no scene {index}")));
        }
        a.storyboard.remove(index);
        Ok(())
    })?;
    Ok(json!({"storyboard": a.storyboard}))
}

fn storyboard_list(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let a = animation(&st.doc);
    let fps = st.doc.timeline.as_ref().map_or(24.0, |t| t.fps);
    let scenes: Vec<Value> = a
        .storyboard
        .iter()
        .enumerate()
        .map(|(i, sc)| json!({"index": i, "name": sc.name, "frame": sc.frame, "duration": sc.duration, "seconds": sc.duration as f32 / fps.max(1e-3), "comment": sc.comment}))
        .collect();
    Ok(json!({"scenes": scenes}))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Standard base64 (RFC 4648) with padding, for data URIs.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn pdf_escape(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii() && !c.is_control()).collect::<String>().replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)")
}

/// Export the storyboard as an SVG or PDF layout: a grid of scene thumbnails with name, frame
/// range, duration and comment under each.
fn storyboard_export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "anim.storyboard.export";
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad(cmd, "needs `path` (.svg or .pdf)"))?.to_string();
    let format = p
        .get("format")
        .and_then(Value::as_str)
        .map(str::to_lowercase)
        .unwrap_or_else(|| std::path::Path::new(&path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "svg".into()));
    if format != "svg" && format != "pdf" {
        return Err(bad(cmd, "format is svg or pdf"));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let mut base = (*st.doc).clone();
    crate::video_cmds::store(&mut base);
    let a = animation(&base);
    if a.storyboard.is_empty() {
        return Err(bad(cmd, "the storyboard has no scenes"));
    }
    let fps = base.timeline.as_ref().map_or(24.0, |t| t.fps).max(1e-3);
    let columns = p.get("columns").and_then(Value::as_u64).unwrap_or(3).clamp(1, 8) as usize;
    let thumb_w = p.get("thumbnailWidth").and_then(Value::as_u64).unwrap_or(320).clamp(64, 1024) as u32;
    let bounds = base.bounds();
    let thumb_h = ((f64::from(thumb_w) * f64::from(bounds.height().max(1))) / f64::from(bounds.width().max(1))).round().max(1.0) as u32;
    let (cell_w, cell_h, margin, text_h) = (f64::from(thumb_w) + 24.0, f64::from(thumb_h) + 96.0, 24.0, 14.0);
    let rows = a.storyboard.len().div_ceil(columns);
    let (page_w, page_h) = (margin * 2.0 + cell_w * columns as f64, margin * 2.0 + cell_h * rows as f64);
    let mut thumbs = Vec::new();
    for sc in &a.storyboard {
        let f = sc.frame.min(base.timeline.as_ref().map_or(0, |t| t.duration.saturating_sub(1)));
        thumbs.push(render_frame_png(&base, f, Some(thumb_w), Some(thumb_h))?);
    }
    let caption = |sc: &Scene| {
        let end = sc.frame + sc.duration;
        (format!("{} — frames {}–{} ({:.2}s)", sc.name, sc.frame, end.saturating_sub(1), sc.duration as f32 / fps), sc.comment.clone())
    };
    let bytes = if format == "svg" {
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{page_w}\" height=\"{page_h}\" viewBox=\"0 0 {page_w} {page_h}\">\n<rect width=\"100%\" height=\"100%\" fill=\"white\"/>\n"
        );
        for (i, (sc, png)) in a.storyboard.iter().zip(&thumbs).enumerate() {
            let (x, y) = (margin + (i % columns) as f64 * cell_w, margin + (i / columns) as f64 * cell_h);
            let data = base64(png);
            let (title, comment) = caption(sc);
            svg.push_str(&format!(
                "<image x=\"{x}\" y=\"{y}\" width=\"{thumb_w}\" height=\"{thumb_h}\" xlink:href=\"data:image/png;base64,{data}\"/>\n<rect x=\"{x}\" y=\"{y}\" width=\"{thumb_w}\" height=\"{thumb_h}\" fill=\"none\" stroke=\"#444\"/>\n"
            ));
            let ty = y + f64::from(thumb_h) + text_h + 6.0;
            svg.push_str(&format!(
                "<text x=\"{x}\" y=\"{ty}\" font-family=\"sans-serif\" font-size=\"12\" font-weight=\"bold\">{}</text>\n",
                xml_escape(&title)
            ));
            for (k, line) in comment.lines().take(4).enumerate() {
                svg.push_str(&format!(
                    "<text x=\"{x}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"11\" fill=\"#333\">{}</text>\n",
                    ty + text_h * (k as f64 + 1.0),
                    xml_escape(line)
                ));
            }
        }
        svg.push_str("</svg>\n");
        svg.into_bytes()
    } else {
        // One PDF page: JPEG thumbnails and Helvetica captions.
        let mut jpegs = Vec::new();
        for png in &thumbs {
            let imported = redrob_photo_io::import("thumb.png", png).map_err(|e| EngineError::Other(e.to_string()))?;
            let mut d = imported.document;
            // JPEG has no alpha: flatten over white.
            let fmt = d.pixel_format();
            let white = crate::file_cmds::flattened(&d, fmt);
            d.layers = vec![Layer::new("t", LayerContent::Raster(white))];
            jpegs.push(
                redrob_photo_io::export(&d, "jpg", &redrob_photo_io::ExportOptions::default())
                    .map(|r| r.bytes)
                    .map_err(|e| EngineError::Other(e.to_string()))?,
            );
        }
        let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets: Vec<usize> = Vec::new();
        let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        };
        let n = jpegs.len();
        // 1 catalog, 2 pages, 3 page, 4 contents, 5 font, 6.. images.
        obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
        obj(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        let xobjects: Vec<String> = (0..n).map(|i| format!("/Im{i} {} 0 R", 6 + i)).collect();
        obj(
            &mut out,
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_w:.1} {page_h:.1}] /Resources << /Font << /F1 5 0 R >> /XObject << {} >> >> /Contents 4 0 R >>", xobjects.join(" ")).as_bytes(),
        );
        let mut stream = String::new();
        for (i, sc) in a.storyboard.iter().enumerate() {
            let (x, y_top) = (margin + (i % columns) as f64 * cell_w, margin + (i / columns) as f64 * cell_h);
            let y = page_h - y_top - f64::from(thumb_h);
            stream.push_str(&format!("q {thumb_w} 0 0 {thumb_h} {x:.1} {y:.1} cm /Im{i} Do Q\n"));
            let (title, comment) = caption(sc);
            let mut ty = y - text_h - 4.0;
            stream.push_str(&format!("BT /F1 11 Tf {x:.1} {ty:.1} Td ({}) Tj ET\n", pdf_escape(&title)));
            for line in comment.lines().take(4) {
                ty -= text_h;
                stream.push_str(&format!("BT /F1 10 Tf {x:.1} {ty:.1} Td ({}) Tj ET\n", pdf_escape(line)));
            }
        }
        obj(&mut out, format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()).as_bytes());
        obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
        for jpeg in &jpegs {
            let mut img = format!(
                "<< /Type /XObject /Subtype /Image /Width {thumb_w} /Height {thumb_h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
                jpeg.len()
            )
            .into_bytes();
            img.extend_from_slice(jpeg);
            img.extend_from_slice(b"\nendstream");
            obj(&mut out, &img);
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
        for o in &offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
        out
    };
    crate::file_cmds::write_file(&path, &bytes)?;
    Ok(json!({"file": path, "scenes": a.storyboard.len(), "format": format}))
}

// ---------- recorder (timelapse) ----------

/// One snapshot of the composite into the recorder's directory.
fn capture_snapshot(doc: &Document, rec: &mut Recorder) -> Result<String> {
    let scale = (rec.scale / 100.0).clamp(0.05, 1.0);
    let b = doc.bounds();
    let w = ((f64::from(b.width()) * f64::from(scale)).round() as u32).max(1);
    let h = ((f64::from(b.height()) * f64::from(scale)).round() as u32).max(1);
    let f = current_frame(doc);
    let png = render_frame_png(doc, f, Some(w), Some(h))?;
    std::fs::create_dir_all(&rec.dir).map_err(|e| EngineError::Other(format!("{}: {e}", rec.dir)))?;
    let file = std::path::Path::new(&rec.dir).join(format!("snap_{:06}.png", rec.count));
    crate::file_cmds::write_file(&file.to_string_lossy(), &png)?;
    rec.count += 1;
    rec.pending = 0;
    Ok(file.to_string_lossy().into_owned())
}

/// Called after every edit: while recording, take a snapshot every Nth edit.
pub fn on_edit(doc: &mut Document) {
    let mut a = animation(doc);
    if !a.recorder.enabled {
        return;
    }
    a.recorder.pending += 1;
    if a.recorder.pending >= a.recorder.every.max(1) {
        let _ = capture_snapshot(doc, &mut a.recorder);
    }
    set_animation(doc, &a);
}

fn recorder_start(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = p.get("dir").and_then(Value::as_str).map(str::to_string);
    let doc_name = s.active().map(|d| d.doc.name.clone()).ok_or(EngineError::NoDocument)?;
    let a = edit_animation(s, "Start Recording", |a| {
        let r = &mut a.recorder;
        r.dir = dir.unwrap_or_else(|| {
            if r.dir.is_empty() {
                std::env::temp_dir().join(format!("redrob-recorder-{}", crate::file_cmds::stem(&doc_name))).to_string_lossy().into_owned()
            } else {
                r.dir.clone()
            }
        });
        if let Some(e) = p.get("every").and_then(Value::as_u64) {
            r.every = e.clamp(1, 1000) as u32;
        }
        if let Some(k) = p.get("scale").and_then(Value::as_f64) {
            r.scale = k.clamp(5.0, 100.0) as f32;
        }
        r.enabled = true;
        r.pending = 0;
        Ok(())
    })?;
    Ok(json!(a.recorder))
}

fn recorder_stop(s: &mut Session, _p: &Value) -> Result<Value> {
    let a = edit_animation(s, "Stop Recording", |a| {
        a.recorder.enabled = false;
        Ok(())
    })?;
    Ok(json!(a.recorder))
}

fn recorder_capture(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = (*st.doc).clone();
    let mut rec = animation(&doc).recorder;
    if rec.dir.is_empty() {
        rec.dir = std::env::temp_dir().join(format!("redrob-recorder-{}", crate::file_cmds::stem(&doc.name))).to_string_lossy().into_owned();
    }
    let file = capture_snapshot(&doc, &mut rec)?;
    let a = edit_animation(s, "Capture Snapshot", |a| {
        a.recorder = rec;
        Ok(())
    })?;
    Ok(json!({"file": file, "count": a.recorder.count}))
}

/// Export the snapshots as a timelapse video through ffmpeg.
fn recorder_export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "anim.recorder.export";
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad(cmd, "needs `path`"))?.to_string();
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let rec = animation(&st.doc).recorder;
    if rec.dir.is_empty() || rec.count == 0 {
        return Err(bad(cmd, "no snapshots recorded yet"));
    }
    let fps = p.get("fps").and_then(Value::as_f64).unwrap_or(30.0);
    let codec = p.get("codec").and_then(Value::as_str).unwrap_or("libx264").to_string();
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-framerate".into(),
        format!("{fps}"),
        "-i".into(),
        std::path::Path::new(&rec.dir).join("snap_%06d.png").to_string_lossy().into_owned(),
        "-c:v".into(),
        codec.clone(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-vf".into(),
        "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(),
    ];
    if let Some(extra) = p.get("ffmpegArgs").and_then(Value::as_array) {
        args.extend(extra.iter().filter_map(Value::as_str).map(str::to_string));
    }
    args.push(path.clone());
    run_tool("ffmpeg", &args)?;
    Ok(json!({"file": path, "snapshots": rec.count, "fps": fps, "codec": codec}))
}

// ---------- registry ----------

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    crate::active_layer_of(s).map(|_| ())
}

fn has_group(s: &Session) -> std::result::Result<(), String> {
    let l = crate::active_layer_of(s)?;
    if l.is_group() { Ok(()) } else { Err("select a group".into()) }
}

fn has_audio(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if animation(&d.doc).audio.is_some() { Ok(()) } else { Err("no audio track".into()) }
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $params:expr, $en:expr, $journal:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $en, journal: $journal, run: $run }
        };
    }
    const ANIM: &[&str] = &["Layer", "Animation"];
    vec![
        spec!(
            "anim.keyframe.add",
            "New Keyframe",
            ANIM,
            r##"{"layer":id?,"frame":int? (default playhead)} → a blank keyframe; the frames after it hold it"##,
            has_layer,
            true,
            |s, p| add_key(s, p, false)
        ),
        spec!(
            "anim.keyframe.duplicate",
            "Duplicate Keyframe",
            ANIM,
            r##"{"layer":id?,"frame":int?} → a keyframe copying the one shown there"##,
            has_layer,
            true,
            |s, p| add_key(s, p, true)
        ),
        spec!("anim.keyframe.remove", "Remove Keyframe", ANIM, r##"{"layer":id?,"frame":int?}"##, has_layer, true, remove_key),
        spec!("anim.keyframe.move", "Move Keyframe", ANIM, r##"{"layer":id?,"from":int? (default playhead),"to":int}"##, has_layer, true, |s, p| move_key(
            s, p, false
        )),
        spec!("anim.keyframe.copy", "Copy Keyframe", ANIM, r##"{"layer":id?,"from":int?,"to":int} → an independent copy"##, has_layer, true, |s, p| move_key(
            s, p, true
        )),
        spec!(
            "anim.keyframe.clone",
            "Clone Keyframe",
            ANIM,
            r##"{"layer":id?,"from":int?,"to":int} → `to` shows the same key (clone frame)"##,
            has_layer,
            true,
            clone_key
        ),
        spec!(
            "anim.table",
            "Animation Frame Table",
            &[],
            r##"{"start":int?,"end":int?} → {fps,duration,current,layers:[{keys,clones,frames:[{frame,status}],cached}],onionSkin,audio}"##,
            has_doc,
            false,
            table
        ),
        spec!(
            "anim.groupToAnimated",
            "Convert Group to Animated Layer",
            &["Layer", "Convert"],
            r##"{"layer":id?} (each child becomes one frame)"##,
            has_group,
            true,
            group_to_animated
        ),
        spec!(
            "anim.onionSkin.set",
            "Onion Skin…",
            ANIM,
            r##"{"enabled":bool?,"before":0..10?,"after":0..10?,"opacities":[percent…]?,"tintBefore":[r,g,b]|"#hex"?,"tintAfter":…?,"tintStrength":0..100?}"##,
            has_doc,
            true,
            onion_set
        ),
        spec!(
            "anim.curves.addKey",
            "Add Curve Key",
            ANIM,
            r##"{"layer":id?,"channel":"opacity"|"x"|"y"|"scaleX"|"scaleY"|"rotation","frame":int?,"value":number? (opacity: the layer's),"ease":"linear"|"hold"|"smooth"?,"handleOut":[frames,value]?,"handleIn":[frames,value]?}"##,
            has_layer,
            true,
            |s, p| curve_key(s, p, false)
        ),
        spec!("anim.curves.removeKey", "Remove Curve Key", ANIM, r##"{"layer":id?,"channel":str,"frame":int?}"##, has_layer, true, |s, p| curve_key(
            s, p, true
        )),
        spec!(
            "anim.curves.list",
            "Animation Curves",
            &[],
            r##"{"samples":bool?} → {frame,layers:[{channels:[{name,keys,now,values}]}]}"##,
            has_doc,
            false,
            curves_list
        ),
        spec!(
            "anim.importFrames",
            "Import Animation Frames…",
            &["File", "Import"],
            r##"{"paths":[file…]|directory,"ext":"png"?,"order":"name"|"mtime"|"given"=name,"reverse":bool?,"step":frames per image=1,"start":frame=0,"fps":number?,"name":str?}"##,
            has_doc,
            true,
            import_frames
        ),
        spec!(
            "anim.importVideo",
            "Import Video Animation…",
            &["File", "Import"],
            r##"{"path":video,"fps":number? (default timeline),"scale":percent?|"width":px?,"height":px?,"from":seconds?,"to":seconds?,"step":int?,"start":frame?,"name":str?} (through ffmpeg)"##,
            has_doc,
            true,
            import_video
        ),
        spec!(
            "anim.render",
            "Render Animation…",
            &["File", "Export"],
            r##"{"path":file.mp4|.webm|.mov|.gif or a directory,"format":"png"|"jpg"|"mp4"|…? (default from path),"codec":"libx264"|…?,"width":px?,"height":px?,"fps":number?,"start":frame?,"end":frame? (default work area),"crf":int?,"bitrate":"8M"?,"audio":bool=true,"hdr":bool=false (BT.2020 PQ metadata),"ffmpegArgs":[…]?}"##,
            has_doc,
            false,
            render_animation
        ),
        spec!("anim.audio.set", "Attach Audio…", ANIM, r##"{"path":file,"muted":bool?,"offsetFrames":int?,"volume":percent?}"##, has_doc, true, audio_set),
        spec!("anim.audio.mute", "Mute Audio", ANIM, r##"{"muted":bool? (default toggle)}"##, has_audio, true, audio_mute),
        spec!("anim.audio.remove", "Remove Audio", ANIM, "{}", has_audio, true, audio_remove),
        spec!(
            "anim.audio.scrub",
            "Scrub Audio",
            &[],
            r##"{"frame":int? (default playhead),"frames":int=1} (plays that span through ffplay)"##,
            has_audio,
            false,
            audio_scrub
        ),
        spec!(
            "anim.storyboard.add",
            "Add Scene",
            &["Window", "Storyboard"],
            r##"{"name":str?,"frame":int? (default playhead),"duration":frames=24,"comment":str?}"##,
            has_doc,
            true,
            storyboard_add
        ),
        spec!(
            "anim.storyboard.update",
            "Edit Scene",
            &[],
            r##"{"index":int,"name":str?,"frame":int?,"duration":int?,"comment":str?}"##,
            has_doc,
            true,
            storyboard_update
        ),
        spec!("anim.storyboard.remove", "Remove Scene", &[], r##"{"index":int}"##, has_doc, true, storyboard_remove),
        spec!("anim.storyboard.list", "Storyboard", &[], "{} → {scenes:[{index,name,frame,duration,seconds,comment}]}", has_doc, false, storyboard_list),
        spec!(
            "anim.storyboard.export",
            "Export Storyboard…",
            &["Window", "Storyboard"],
            r##"{"path":file.svg|.pdf,"format":"svg"|"pdf"?,"columns":1..8=3,"thumbnailWidth":px=320}"##,
            has_doc,
            false,
            storyboard_export
        ),
        spec!(
            "anim.recorder.start",
            "Start Recording",
            &["Window", "Recorder"],
            r##"{"dir":path?,"every":edits=1,"scale":percent=50}"##,
            has_doc,
            true,
            recorder_start
        ),
        spec!("anim.recorder.stop", "Stop Recording", &["Window", "Recorder"], "{}", has_doc, true, recorder_stop),
        spec!("anim.recorder.capture", "Capture Snapshot", &["Window", "Recorder"], "{}", has_doc, true, recorder_capture),
        spec!(
            "anim.recorder.export",
            "Export Timelapse…",
            &["Window", "Recorder"],
            r##"{"path":file.mp4,"fps":30?,"codec":"libx264"?,"ffmpegArgs":[…]?}"##,
            has_doc,
            false,
            recorder_export
        ),
    ]
}
