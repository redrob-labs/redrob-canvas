//! Brush preset tooling beyond the preset list: option locks that survive a preset switch
//! (`brush.locks.*`), Reload (`brush.presets.reload`), the engine switch (`brush.setEngine`),
//! the Brush Preset History (`brush.history.*`), the Ten Brushes shortcut slots
//! (`brush.tenBrushes.*`), the brush editor's paintable scratch pad (`brush.scratchPad.*`), and
//! preset thumbnails: edited directly or composed from the icon library
//! (`brush.presets.setThumbnail`, `brush.presets.composeThumbnail`).
//!
//! The history, the slots and the thumbnails are tool state, like the presets themselves; they
//! live for the session and in the preferences document where the shell persists them.

use redrob_photo_color::PixelFormat;
use redrob_photo_geom::Rect;
use redrob_photo_paint::engines::{EngineSettings, KINDS};
use redrob_photo_paint::tile::{b64_decode, b64_encode};
use redrob_photo_paint::{BrushSettings, ColorTile, GrayTile, SectionLocks, StrokePoint};
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn flag(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}

fn name_param(p: &Value, cmd: &str) -> Result<String> {
    let n = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "missing `name`"))?;
    Ok(n.to_string())
}

fn preset_brush(s: &Session, name: &str, cmd: &str) -> Result<BrushSettings> {
    let pr = redrob_photo_paint::presets::find(&s.tools.presets, name).ok_or_else(|| bad(cmd, format!("no brush preset named `{name}`")))?;
    Ok(pr.picked_over(&s.tools.brush))
}

// ---------------------------------------------------------------------------
// Locks
// ---------------------------------------------------------------------------

/// `brush.locks.get`: every lock and whether it is on.
fn locks_get(s: &mut Session, _: &Value) -> Result<Value> {
    let l = &s.tools.brush.locks;
    Ok(json!({ "locks": SectionLocks::NAMES.iter().map(|n| json!({"name": n, "locked": l.get(n).unwrap_or(false)})).collect::<Vec<_>>() }))
}

/// `brush.locks.set`: lock or unlock an option. Unlocking with `keep: false` drops the locked
/// value and takes the current preset's instead (the reference's unlock choice); `keep: true`
/// (the default) only removes the lock and leaves the value as it is.
fn locks_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.locks.set";
    let name = name_param(p, cmd)?;
    let on = p.get("locked").or_else(|| p.get("on")).and_then(Value::as_bool).unwrap_or(true);
    let keep = flag(p, "keep", true);
    if s.tools.brush.locks.get(&name).is_none() {
        return Err(bad(cmd, format!("unknown lock `{name}` (one of {})", SectionLocks::NAMES.join(", "))));
    }
    let was = s.tools.brush.locks.get(&name).unwrap_or(false);
    let mut b = s.tools.brush.clone();
    b.locks.set(&name, on);
    if was && !on && !keep {
        // Drop the locked value: the option as the current preset stores it, nothing locked.
        if let Some(preset) = s.tools.current_preset.clone() {
            let mut unlocked = b.clone();
            unlocked.locks = SectionLocks::default();
            let from = {
                let pr = redrob_photo_paint::presets::find(&s.tools.presets, &preset).ok_or_else(|| bad(cmd, format!("no brush preset named `{preset}`")))?;
                pr.picked_over(&unlocked)
            };
            // Only the unlocked option comes from the preset: lock everything else briefly so
            // `picked_over` keeps it, then restore the real locks.
            let mut keep_rest = b.clone();
            for n in SectionLocks::NAMES {
                keep_rest.locks.set(n, n != name);
            }
            let mut merged = redrob_photo_paint::BrushPreset {
                name: preset.clone(),
                brush: from,
                builtin: false,
                group: String::new(),
                folder: Vec::new(),
                captures: Default::default(),
            }
            .picked_over(&keep_rest);
            merged.locks = b.locks.clone();
            b = merged;
        }
    }
    s.tools.brush = b;
    s.tools.brush_changed();
    Ok(json!({ "name": name, "locked": on }))
}

/// `brush.locks.clear`: unlock everything (values stay).
fn locks_clear(s: &mut Session, _: &Value) -> Result<Value> {
    let n = SectionLocks::NAMES.iter().filter(|n| s.tools.brush.locks.get(n) == Some(true)).count();
    s.tools.brush.locks = SectionLocks::default();
    s.tools.brush_changed();
    Ok(json!({ "unlocked": n }))
}

// ---------------------------------------------------------------------------
// Reload, engine switch
// ---------------------------------------------------------------------------

/// `brush.presets.reload`: discard the session's tweaks and go back to the stored preset
/// (`name`, else the preset the brush was picked from). Locked options stay, as on any pick.
fn presets_reload(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.reload";
    let name = match p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => s.tools.current_preset.clone().ok_or_else(|| bad(cmd, "the current brush was not picked from a preset; pass `name`"))?,
    };
    let mut b = preset_brush(s, &name, cmd)?;
    s.load_brush_tips(&mut b).map_err(|e| bad(cmd, e))?;
    // Colours are tool state, not preset state.
    b.color = s.tools.brush.color;
    b.background = s.tools.brush.background;
    s.tools.brush = b;
    s.tools.current_preset = Some(name.clone());
    s.tools.brush_changed();
    Ok(json!({ "name": name }))
}

/// `brush.setEngine`: switch the brush's engine. The engine's own options reset to the
/// engine's defaults (or to `settings`); the tip, dynamics and stroke options are shared and stay.
fn set_engine(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.setEngine";
    let kind = p.get("kind").and_then(Value::as_str).ok_or_else(|| bad(cmd, format!("`kind` must be one of {}", KINDS.join(", "))))?;
    let mut engine = EngineSettings::by_kind(kind).ok_or_else(|| bad(cmd, format!("unknown engine `{kind}` (one of {})", KINDS.join(", "))))?;
    if let Some(settings) = p.get("settings").filter(|v| v.is_object()) {
        let mut v = serde_json::to_value(&engine).map_err(|e| bad(cmd, e.to_string()))?;
        if let (Some(d), Some(o)) = (v.as_object_mut(), settings.as_object()) {
            for (k, val) in o {
                if k != "kind" {
                    d.insert(k.clone(), val.clone());
                }
            }
        }
        engine = serde_json::from_value(v).map_err(|e| bad(cmd, format!("bad `settings`: {e}")))?;
    }
    let was = s.tools.brush.engine.kind();
    s.tools.brush.engine = engine;
    s.tools.brush_changed();
    Ok(json!({ "kind": kind, "previous": was, "engine": serde_json::to_value(&s.tools.brush.engine).unwrap_or(Value::Null) }))
}

// ---------------------------------------------------------------------------
// Preset history
// ---------------------------------------------------------------------------

/// Where a re-used preset goes in the history list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryMode {
    /// A preset keeps its place; new ones are added at the top.
    Static,
    /// A re-used preset jumps to the top.
    #[default]
    MoveToTop,
    /// A re-used preset moves up one place.
    BubbleUp,
}

impl HistoryMode {
    pub fn name(self) -> &'static str {
        match self {
            HistoryMode::Static => "static",
            HistoryMode::MoveToTop => "moveToTop",
            HistoryMode::BubbleUp => "bubbleUp",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "static" => HistoryMode::Static,
            "moveToTop" | "top" => HistoryMode::MoveToTop,
            "bubbleUp" | "bubble" => HistoryMode::BubbleUp,
            _ => return None,
        })
    }
}

/// Recently used brush presets, newest first (the Brush Preset History docker).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PresetHistory {
    pub items: Vec<String>,
    pub mode: HistoryMode,
    /// Most presets kept (1..100).
    pub count: usize,
}

impl Default for PresetHistory {
    fn default() -> Self {
        Self { items: Vec::new(), mode: HistoryMode::MoveToTop, count: 10 }
    }
}

impl PresetHistory {
    /// Note that `name` was picked.
    pub fn record(&mut self, name: &str) {
        let max = self.count.clamp(1, 100);
        match self.items.iter().position(|n| n.eq_ignore_ascii_case(name)) {
            Some(0) => {}
            Some(i) => match self.mode {
                HistoryMode::Static => {}
                HistoryMode::MoveToTop => {
                    let n = self.items.remove(i);
                    self.items.insert(0, n);
                }
                HistoryMode::BubbleUp => self.items.swap(i, i - 1),
            },
            None => self.items.insert(0, name.to_string()),
        }
        self.items.truncate(max);
    }
    /// Drop one preset from the list; false when it wasn't there.
    pub fn forget(&mut self, name: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|n| !n.eq_ignore_ascii_case(name));
        self.items.len() != before
    }
}

fn history_list(s: &mut Session, _: &Value) -> Result<Value> {
    let h = &s.tools.preset_history;
    Ok(json!({ "items": h.items, "mode": h.mode.name(), "count": h.count }))
}

fn history_forget(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.history.forget";
    let name = name_param(p, cmd)?;
    if !s.tools.preset_history.forget(&name) {
        return Err(bad(cmd, format!("`{name}` is not in the history")));
    }
    Ok(json!({ "name": name, "items": s.tools.preset_history.items }))
}

fn history_clear(s: &mut Session, _: &Value) -> Result<Value> {
    let n = s.tools.preset_history.items.len();
    s.tools.preset_history.items.clear();
    Ok(json!({ "cleared": n }))
}

/// `brush.history.configure`: the list's behaviour (`mode`) and length (`count`).
fn history_configure(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.history.configure";
    if let Some(m) = p.get("mode") {
        let m = m.as_str().and_then(HistoryMode::parse).ok_or_else(|| bad(cmd, "`mode` must be static, moveToTop or bubbleUp"))?;
        s.tools.preset_history.mode = m;
    }
    if let Some(c) = p.get("count") {
        let c = c.as_u64().filter(|c| (1..=100).contains(c)).ok_or_else(|| bad(cmd, "`count` must be 1..100"))?;
        s.tools.preset_history.count = c as usize;
        s.tools.preset_history.items.truncate(c as usize);
    }
    history_list(s, p)
}

/// `brush.history.pick`: make a history entry the current brush (a `tools.setBrush` with that
/// preset, so the history records it as used).
fn history_pick(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.history.pick";
    let name = match p.get("index").and_then(Value::as_u64) {
        Some(i) => s.tools.preset_history.items.get(i as usize).cloned().ok_or_else(|| bad(cmd, format!("no history entry {i}")))?,
        None => name_param(p, cmd)?,
    };
    pick_preset(s, &name)
}

/// Pick a preset as the shell's picker does and record it in the history.
pub fn pick_preset(s: &mut Session, name: &str) -> Result<Value> {
    let r = crate::brush_cmds::set_brush(s, &json!({ "preset": name }))?;
    s.tools.preset_history.record(name);
    Ok(r)
}

// ---------------------------------------------------------------------------
// Ten Brushes
// ---------------------------------------------------------------------------

fn slot_param(p: &Value, cmd: &str) -> Result<usize> {
    p.get("slot").and_then(Value::as_u64).filter(|i| (1..=10).contains(i)).map(|i| i as usize - 1).ok_or_else(|| bad(cmd, "`slot` must be 1..10"))
}

fn ten_list(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "slots": s.tools.ten_brushes.iter().enumerate().map(|(i, n)| json!({"slot": i + 1, "preset": n})).collect::<Vec<_>>() }))
}

/// `brush.tenBrushes.assign`: put a preset (`name`, else the current one) on a slot; `name: null`
/// clears the slot.
fn ten_assign(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.tenBrushes.assign";
    let slot = slot_param(p, cmd)?;
    let name = match p.get("name") {
        Some(Value::Null) => None,
        Some(Value::String(n)) if !n.trim().is_empty() => {
            if !s.tools.presets.iter().any(|x| x.name.eq_ignore_ascii_case(n)) {
                return Err(bad(cmd, format!("no brush preset named `{n}`")));
            }
            Some(n.trim().to_string())
        }
        Some(_) => return Err(bad(cmd, "`name` must be a preset name or null")),
        None => Some(s.tools.current_preset.clone().ok_or_else(|| bad(cmd, "the current brush was not picked from a preset; pass `name`"))?),
    };
    s.tools.ten_brushes[slot] = name.clone();
    Ok(json!({ "slot": slot + 1, "preset": name }))
}

/// `brush.tenBrushes.activate`: pick the slot's preset; a second activation of the slot the
/// current preset sits on goes back to the preset used before (the reference's toggle).
fn ten_activate(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.tenBrushes.activate";
    let slot = slot_param(p, cmd)?;
    let Some(name) = s.tools.ten_brushes[slot].clone() else { return Err(bad(cmd, format!("slot {} is empty", slot + 1))) };
    let current = s.tools.current_preset.clone();
    if current.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&name)) {
        if let Some(prev) = s.tools.ten_brushes_previous.take() {
            let r = pick_preset(s, &prev)?;
            s.tools.ten_brushes_previous = Some(name);
            return Ok(r);
        }
        return Ok(json!({ "preset": name, "unchanged": true }));
    }
    let r = pick_preset(s, &name)?;
    s.tools.ten_brushes_previous = current;
    Ok(r)
}

/// `brush.presets.next` / `brush.presets.previous`: step through the presets in list order
/// (wrapping), or through the Ten Brushes slots with `slots: true`.
fn step_preset(s: &mut Session, p: &Value, forward: bool, cmd: &str) -> Result<Value> {
    let names: Vec<String> = if flag(p, "slots", false) {
        s.tools.ten_brushes.iter().flatten().cloned().collect()
    } else {
        s.tools.presets.iter().map(|x| x.name.clone()).collect()
    };
    if names.is_empty() {
        return Err(bad(cmd, "no presets to step through"));
    }
    let at = s.tools.current_preset.as_deref().and_then(|c| names.iter().position(|n| n.eq_ignore_ascii_case(c)));
    let next = match (at, forward) {
        (Some(i), true) => (i + 1) % names.len(),
        (Some(i), false) => (i + names.len() - 1) % names.len(),
        (None, true) => 0,
        (None, false) => names.len() - 1,
    };
    pick_preset(s, &names[next].clone())
}

// ---------------------------------------------------------------------------
// Scratch pad
// ---------------------------------------------------------------------------

/// The brush editor's scratch pad: a small RGBA surface strokes are painted on with the current
/// brush, filled from the preset icon, the current image, a gradient, the background colour or
/// white.
#[derive(Clone, Debug)]
pub struct ScratchPad {
    pub width: u32,
    pub height: u32,
    pub surface: Surface,
    /// Bumped on every fill and stroke, so a view re-uploads only when the pad changed.
    pub rev: u64,
}

/// Largest scratch pad side.
pub const MAX_PAD_SIDE: u32 = 1024;

impl ScratchPad {
    pub fn new(width: u32, height: u32) -> Self {
        let width = width.clamp(16, MAX_PAD_SIDE);
        let height = height.clamp(16, MAX_PAD_SIDE);
        let mut surface = Surface::new(PixelFormat::RGBA32F);
        surface.fill_rect(Rect::new(0, 0, width as i32, height as i32), &[1.0, 1.0, 1.0, 1.0]);
        Self { width, height, surface, rev: 1 }
    }
    pub fn rect(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }
    /// The pad's pixels as 8-bit RGBA.
    pub fn rgba8(&self) -> Vec<u8> {
        let r = self.rect();
        let mut px = vec![[0u8; 4]; (self.width * self.height) as usize];
        self.surface.read_rgba8_into(r, &mut px);
        px.into_iter().flatten().collect()
    }
}

fn pad(s: &mut Session) -> &mut ScratchPad {
    s.scratch_pad.get_or_insert_with(|| ScratchPad::new(256, 256))
}

/// `brush.scratchPad.fill`: `with` = white (reset), background, gradient (the Gradient tool's
/// current gradient, left to right), image (the active document, fitted) or icon (the current
/// preset's thumbnail or a stroke preview of the brush).
fn pad_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.scratchPad.fill";
    let with = p.get("with").and_then(Value::as_str).unwrap_or("white");
    if let (Some(w), Some(h)) = (p.get("width").and_then(Value::as_u64), p.get("height").and_then(Value::as_u64)) {
        if !(16..=u64::from(MAX_PAD_SIDE)).contains(&w) || !(16..=u64::from(MAX_PAD_SIDE)).contains(&h) {
            return Err(bad(cmd, format!("`width`/`height` must be 16..{MAX_PAD_SIDE}")));
        }
        s.scratch_pad = Some(ScratchPad::new(w as u32, h as u32));
    }
    let (fg, bg) = (s.tools.foreground, s.tools.background);
    let gradient: Vec<(f32, [f32; 4])> = s.presets.gradient.resolve(fg, bg);
    let image: Option<(Rect, Vec<[f32; 4]>)> = s.active().map(|d| {
        let r = Rect::new(0, 0, d.doc.size.width as i32, d.doc.size.height as i32);
        (r, redrob_photo_compose::render(&d.doc, r).px)
    });
    let icon: Option<ColorTile> = s.tools.current_preset.as_ref().and_then(|n| s.tools.preset_thumbnails.get(n).cloned());
    let brush = s.tools.brush.clone();
    let pad = pad(s);
    let (w, h) = (pad.width, pad.height);
    let rect = pad.rect();
    let mut px = vec![[1.0f32; 4]; (w * h) as usize];
    match with {
        "white" | "reset" => {}
        "background" => px.iter_mut().for_each(|p| *p = bg),
        "gradient" => {
            let stops: Vec<redrob_photo_paint::GradientStop> =
                gradient.iter().map(|(t, c)| redrob_photo_paint::GradientStop { position: *t, color: *c }).collect();
            for y in 0..h {
                for x in 0..w {
                    let t = x as f32 / (w - 1).max(1) as f32;
                    px[(y * w + x) as usize] = redrob_photo_paint::brush::gradient_at(&stops, fg, bg, t);
                }
            }
        }
        "image" => {
            let (ir, ipx) = image.ok_or(EngineError::NoDocument)?;
            let (iw, ih) = (ir.width().max(1) as f32, ir.height().max(1) as f32);
            let scale = (iw / w as f32).max(ih / h as f32).max(1e-6);
            for y in 0..h {
                for x in 0..w {
                    let (sx, sy) = (((x as f32 + 0.5) * scale) as i32, ((y as f32 + 0.5) * scale) as i32);
                    let c = if sx < ir.width() && sy < ir.height() { ipx[(sy * ir.width() + sx) as usize] } else { [1.0; 4] };
                    // Over white, as a flattened preview.
                    px[(y * w + x) as usize] = [c[0] * c[3] + (1.0 - c[3]), c[1] * c[3] + (1.0 - c[3]), c[2] * c[3] + (1.0 - c[3]), 1.0];
                }
            }
        }
        "icon" => match icon {
            Some(t) => {
                let scale = (t.width as f32 / w as f32).max(t.height as f32 / h as f32).max(1e-6);
                for y in 0..h {
                    for x in 0..w {
                        let (sx, sy) = (((x as f32 + 0.5) * scale) as u32, ((y as f32 + 0.5) * scale) as u32);
                        let c = if sx < t.width && sy < t.height { t.get(sx, sy) } else { [1.0; 3] };
                        px[(y * w + x) as usize] = [c[0], c[1], c[2], 1.0];
                    }
                }
            }
            None => {
                // No custom thumbnail: a stroke preview of the brush across the pad.
                pad.surface.fill_rect(rect, &[1.0, 1.0, 1.0, 1.0]);
                let pts = preview_points(w as f64, h as f64, 24);
                let mut b = brush;
                b.erase = false;
                b.resize_with_dual_brush((h as f32 / 3.0).clamp(1.0, redrob_photo_paint::MAX_BRUSH_SIZE));
                redrob_photo_paint::render_stroke(&mut pad.surface, &b, &pts, None, false, 1.0);
                pad.rev += 1;
                return Ok(json!({ "with": with, "width": w, "height": h }));
            }
        },
        other => return Err(bad(cmd, format!("unknown fill `{other}` (white, background, gradient, image, icon)"))),
    }
    let flat: Vec<f32> = px.iter().flatten().copied().collect();
    pad.surface.write_region(rect, &flat);
    pad.rev += 1;
    Ok(json!({ "with": with, "width": w, "height": h }))
}

/// An S-curve across a `w × h` area with rising then falling pressure: the stroke preview.
fn preview_points(w: f64, h: f64, n: usize) -> Vec<StrokePoint> {
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let x = w * 0.1 + t * w * 0.8;
            let y = h * 0.5 + (t * std::f64::consts::TAU).sin() * h * 0.25;
            StrokePoint { x, y, pressure: (1.0 - (2.0 * t as f32 - 1.0).abs()).max(0.1), time: t * 500.0, ..Default::default() }
        })
        .collect()
}

/// `brush.scratchPad.stroke`: paint on the pad with the current brush (or `brush`/`preset`).
fn pad_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.scratchPad.stroke";
    let pts = crate::brush_cmds::parse_points(p, cmd)?;
    let brush = crate::brush_cmds::resolve_brush(s, p, cmd)?;
    let pad = pad(s);
    let dmg = redrob_photo_paint::render_stroke(&mut pad.surface, &brush, &pts, None, false, 1.0).intersect(&pad.rect());
    pad.rev += 1;
    Ok(json!({ "damage": [dmg.x0, dmg.y0, dmg.width(), dmg.height()] }))
}

/// `brush.scratchPad.get`: the pad's size and its RGBA pixels (base64).
fn pad_get(s: &mut Session, p: &Value) -> Result<Value> {
    let pad = pad(s);
    let mut v = json!({ "width": pad.width, "height": pad.height });
    if flag(p, "pixels", true) {
        v["rgba"] = Value::String(b64_encode(&pad.rgba8()));
    }
    Ok(v)
}

// ---------------------------------------------------------------------------
// Thumbnails and the icon library
// ---------------------------------------------------------------------------

/// Side of a composed thumbnail.
pub const THUMB_SIDE: u32 = 128;

/// Stock thumbnail backgrounds of the icon library.
pub const ICON_BACKGROUNDS: [&str; 6] = ["white", "paper", "dark", "gradient", "canvas", "transparent"];
/// Tool emblems of the icon library.
pub const ICON_EMBLEMS: [&str; 7] = ["none", "brush", "pencil", "eraser", "airbrush", "smudge", "fill"];

fn rgb_param(p: &Value, k: &str, d: [f32; 3]) -> [f32; 3] {
    match p.get(k) {
        Some(Value::String(hex)) => {
            let h = hex.trim_start_matches('#');
            if h.len() == 6
                && let (Ok(r), Ok(g), Ok(b)) = (u8::from_str_radix(&h[0..2], 16), u8::from_str_radix(&h[2..4], 16), u8::from_str_radix(&h[4..6], 16))
            {
                return [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0];
            }
            d
        }
        Some(Value::Array(a)) if a.len() >= 3 => {
            let c = |i: usize| a[i].as_f64().map_or(0.0, |v| v.clamp(0.0, 1.0) as f32);
            [c(0), c(1), c(2)]
        }
        _ => d,
    }
}

/// The library background `name` as a tile.
fn icon_background(name: &str, tint: [f32; 3]) -> Option<ColorTile> {
    let n = THUMB_SIDE;
    let hash = |x: u32, y: u32| redrob_photo_paint::rng::hash2(x as i32, y as i32, 0x6963_6F6E);
    Some(match name {
        "white" => ColorTile::from_fn(n, n, |_, _| [1.0, 1.0, 1.0]),
        "transparent" => ColorTile::from_fn(n, n, |x, y| if ((x / 8) + (y / 8)) % 2 == 0 { [0.8, 0.8, 0.8] } else { [1.0, 1.0, 1.0] }),
        "dark" => ColorTile::from_fn(n, n, |_, _| [0.16, 0.16, 0.17]),
        "paper" => ColorTile::from_fn(n, n, |x, y| {
            let g = 0.93 + 0.05 * (hash(x, y) - 0.5);
            [g * 0.98 + 0.02, g * 0.97, g * 0.92]
        }),
        "canvas" => ColorTile::from_fn(n, n, |x, y| {
            let weave = if (x / 2 + y / 2) % 2 == 0 { 0.02 } else { -0.02 };
            let g = 0.88 + weave + 0.03 * (hash(x, y) - 0.5);
            [g, g * 0.98, g * 0.93]
        }),
        "gradient" => ColorTile::from_fn(n, n, |x, y| {
            let t = (x as f32 + y as f32) / (2.0 * (n - 1) as f32);
            [tint[0] * (1.0 - t) + t, tint[1] * (1.0 - t) + t, tint[2] * (1.0 - t) + t]
        }),
        _ => return None,
    })
}

/// Coverage (0..1) of the tool emblem `name` at a thumbnail pixel: simple geometric glyphs in
/// the lower right quarter.
fn emblem_coverage(name: &str, x: u32, y: u32) -> f32 {
    let n = THUMB_SIDE as f32;
    // Emblem box: the lower-right 40 % of the thumbnail.
    let (bx, by, bs) = (n * 0.58, n * 0.58, n * 0.36);
    let (u, v) = ((x as f32 + 0.5 - bx) / bs, (y as f32 + 0.5 - by) / bs);
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return 0.0;
    }
    let edge = |d: f32| (0.5 - d * bs).clamp(0.0, 1.0);
    let seg = |ax: f32, ay: f32, bx2: f32, by2: f32, px: f32, py: f32| {
        let (dx, dy) = (bx2 - ax, by2 - ay);
        let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
        ((px - (ax + t * dx)).powi(2) + (py - (ay + t * dy)).powi(2)).sqrt()
    };
    match name {
        "brush" => {
            // A diagonal handle with a round head at the lower left.
            let handle = edge(seg(0.9, 0.1, 0.4, 0.6, u, v) - 0.06);
            let head = edge(((u - 0.3).powi(2) + (v - 0.7).powi(2)).sqrt() - 0.18);
            handle.max(head)
        }
        "pencil" => {
            let body = edge(seg(0.85, 0.15, 0.3, 0.7, u, v) - 0.09);
            let tip = edge(seg(0.3, 0.7, 0.15, 0.85, u, v) - 0.03);
            body.max(tip)
        }
        "eraser" => {
            // A tilted block.
            let (cx, cy) = (u - 0.5, v - 0.5);
            let (rx, ry) = (cx * 0.7071 + cy * 0.7071, -cx * 0.7071 + cy * 0.7071);
            edge((rx.abs() - 0.32).max(ry.abs() - 0.18))
        }
        "airbrush" => {
            let can = edge((u - 0.5).abs().max((v - 0.55).abs() - 0.1) - 0.2);
            let mist = (0..5).fold(0.0f32, |m, i| {
                let a = i as f32 * 0.5 - 1.0;
                m.max(edge(((u - 0.5 - a.sin() * 0.3).powi(2) + (v - 0.15 + a.cos() * 0.05).powi(2)).sqrt() - 0.04))
            });
            can.max(mist)
        }
        // A finger: a rounded bar.
        "smudge" => edge(seg(0.5, 0.2, 0.5, 0.75, u, v) - 0.14),
        "fill" => {
            // A tilted bucket and a drop.
            let bucket = edge((u - 0.45).abs().max((v - 0.5).abs()) - 0.25);
            let drop = edge(((u - 0.85).powi(2) + (v - 0.8).powi(2)).sqrt() - 0.08);
            bucket.max(drop)
        }
        _ => 0.0,
    }
}

fn blend_px(dst: &mut [f32; 3], src: [f32; 3], a: f32) {
    for i in 0..3 {
        dst[i] = dst[i] * (1.0 - a) + src[i] * a;
    }
}

/// Compose a thumbnail from the icon library: `background`, an optional stroke preview of
/// `brush`, a tool `emblem` and optional `text` (initials in the top left).
pub fn compose_thumbnail(
    background: &str,
    tint: [f32; 3],
    brush: Option<&BrushSettings>,
    emblem: &str,
    emblem_color: [f32; 3],
    text: &str,
    text_color: [f32; 3],
) -> Option<ColorTile> {
    let n = THUMB_SIDE;
    let bg = icon_background(background, tint)?;
    let mut px: Vec<[f32; 3]> = (0..n * n).map(|i| bg.get(i % n, i / n)).collect();
    if let Some(b) = brush {
        let mut surf = Surface::new(PixelFormat::RGBA32F);
        let mut b = b.clone();
        b.erase = false;
        b.color = [0.1, 0.1, 0.1, 1.0];
        b.resize_with_dual_brush((n as f32 / 4.0).clamp(1.0, redrob_photo_paint::MAX_BRUSH_SIZE));
        let pts = preview_points(f64::from(n), f64::from(n) * 0.9, 24);
        redrob_photo_paint::render_stroke(&mut surf, &b, &pts, None, false, 1.0);
        let mut out = vec![[0.0f32; 4]; (n * n) as usize];
        surf.read_rgba_into(Rect::new(0, 0, n as i32, n as i32), &mut out);
        for (d, s) in px.iter_mut().zip(&out) {
            blend_px(d, [s[0], s[1], s[2]], s[3].clamp(0.0, 1.0));
        }
    }
    if emblem != "none" {
        for y in 0..n {
            for x in 0..n {
                let a = emblem_coverage(emblem, x, y);
                if a > 0.0 {
                    blend_px(&mut px[(y * n + x) as usize], emblem_color, a);
                }
            }
        }
    }
    let text = text.trim();
    if !text.is_empty() {
        let shown: String = text.chars().take(3).collect();
        if let Some(g) = crate::brush_engine_cmds::text_cell(&shown, "", n as f32 * 0.3) {
            let (gw, gh) = (g.width.min(n - 4), g.height.min(n - 4));
            for y in 0..gh {
                for x in 0..gw {
                    let a = g.get(x, y);
                    if a > 0.0 {
                        blend_px(&mut px[((y + 4) * n + x + 4) as usize], text_color, a);
                    }
                }
            }
        }
    }
    Some(ColorTile::from_fn(n, n, |x, y| px[(y * n + x) as usize]))
}

/// `brush.presets.composeThumbnail`: build a preset's thumbnail from the icon library and keep it.
fn compose_thumbnail_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.composeThumbnail";
    let name = match p.get("name").and_then(Value::as_str) {
        Some(n) => n.to_string(),
        None => s.tools.current_preset.clone().ok_or_else(|| bad(cmd, "missing `name`"))?,
    };
    let brush = redrob_photo_paint::presets::find(&s.tools.presets, &name)
        .map(|pr| pr.brush.clone())
        .ok_or_else(|| bad(cmd, format!("no brush preset named `{name}`")))?;
    let background = p.get("background").and_then(Value::as_str).unwrap_or("white");
    if !ICON_BACKGROUNDS.contains(&background) {
        return Err(bad(cmd, format!("`background` must be one of {}", ICON_BACKGROUNDS.join(", "))));
    }
    let emblem = p.get("emblem").and_then(Value::as_str).unwrap_or("none");
    if !ICON_EMBLEMS.contains(&emblem) {
        return Err(bad(cmd, format!("`emblem` must be one of {}", ICON_EMBLEMS.join(", "))));
    }
    let text = p.get("text").and_then(Value::as_str).unwrap_or("");
    let tint = rgb_param(p, "tint", [0.35, 0.55, 0.9]);
    let emblem_color = rgb_param(p, "emblemColor", [0.2, 0.2, 0.22]);
    let text_color = rgb_param(p, "textColor", [0.1, 0.1, 0.1]);
    let with_stroke = flag(p, "stroke", true);
    let mut b = brush;
    let _ = s.load_brush_tips(&mut b);
    let tile = compose_thumbnail(background, tint, with_stroke.then_some(&b), emblem, emblem_color, text, text_color)
        .ok_or_else(|| bad(cmd, "could not compose the thumbnail"))?;
    s.tools.preset_thumbnails.insert(name.clone(), tile);
    s.brush_presets_changed();
    Ok(json!({ "name": name, "width": THUMB_SIDE, "height": THUMB_SIDE }))
}

/// `brush.presets.setThumbnail`: a preset's thumbnail from RGB bytes (`rgb` base64, `width`,
/// `height`), from the scratch pad (`fromScratchPad: true`), or none (`clear: true`).
fn set_thumbnail(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.setThumbnail";
    let name = match p.get("name").and_then(Value::as_str) {
        Some(n) => n.to_string(),
        None => s.tools.current_preset.clone().ok_or_else(|| bad(cmd, "missing `name`"))?,
    };
    if !s.tools.presets.iter().any(|x| x.name.eq_ignore_ascii_case(&name)) {
        return Err(bad(cmd, format!("no brush preset named `{name}`")));
    }
    if flag(p, "clear", false) {
        let had = s.tools.preset_thumbnails.remove(&name).is_some();
        s.brush_presets_changed();
        return Ok(json!({ "name": name, "cleared": had }));
    }
    let tile = if flag(p, "fromScratchPad", false) {
        let pad = pad(s);
        let rgba = pad.rgba8();
        let (w, h) = (pad.width, pad.height);
        // Fit into the thumbnail side, nearest neighbour.
        let scale = (w as f32 / THUMB_SIDE as f32).max(h as f32 / THUMB_SIDE as f32).max(1e-6);
        let (tw, th) = (((w as f32 / scale).round() as u32).clamp(1, THUMB_SIDE), ((h as f32 / scale).round() as u32).clamp(1, THUMB_SIDE));
        ColorTile::from_fn(tw, th, |x, y| {
            let (sx, sy) = ((((x as f32 + 0.5) * scale) as u32).min(w - 1), (((y as f32 + 0.5) * scale) as u32).min(h - 1));
            let i = ((sy * w + sx) * 4) as usize;
            [f32::from(rgba[i]) / 255.0, f32::from(rgba[i + 1]) / 255.0, f32::from(rgba[i + 2]) / 255.0]
        })
    } else {
        let w = p.get("width").and_then(Value::as_u64).filter(|w| (1..=512).contains(w)).ok_or_else(|| bad(cmd, "`width` must be 1..512"))? as u32;
        let h = p.get("height").and_then(Value::as_u64).filter(|h| (1..=512).contains(h)).ok_or_else(|| bad(cmd, "`height` must be 1..512"))? as u32;
        let data = p.get("rgb").and_then(Value::as_str).and_then(b64_decode).ok_or_else(|| bad(cmd, "`rgb` must be base64 RGB bytes"))?;
        if data.len() != (w * h * 3) as usize {
            return Err(bad(cmd, format!("`rgb` has {} bytes, expected {w}×{h}×3", data.len())));
        }
        ColorTile::from_rgb8(w, h, &data)
    };
    s.tools.preset_thumbnails.insert(name.clone(), tile);
    s.brush_presets_changed();
    Ok(json!({ "name": name }))
}

/// `brush.presets.thumbnail`: a preset's custom thumbnail (RGB base64), if it has one.
fn get_thumbnail(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.thumbnail";
    let name = name_param(p, cmd)?;
    Ok(match s.tools.preset_thumbnails.get(&name) {
        Some(t) => json!({ "name": name, "width": t.width, "height": t.height, "rgb": b64_encode(&t.data) }),
        None => json!({ "name": name, "custom": false }),
    })
}

/// `brush.presets.setTipDefaults`: a predefined tip's own initial scale and rotation (the tip
/// chooser applies them when the tip is picked), and the Color Stamp / Lightness / Gradient Map
/// tip mode with its adjustments, on the current brush.
fn set_tip_defaults(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.setTipDefaults";
    let mut b = s.tools.brush.clone();
    if let Some(v) = p.get("scale").and_then(Value::as_f64) {
        if !v.is_finite() || !(0.01..=100.0).contains(&v) {
            return Err(bad(cmd, "`scale` must be 0.01..100"));
        }
        b.tip_scale = v as f32;
    }
    if let Some(v) = p.get("rotation").and_then(Value::as_f64) {
        if !v.is_finite() {
            return Err(bad(cmd, "`rotation` must be finite"));
        }
        b.tip_rotation = (v as f32).rem_euclid(360.0);
    }
    if let Some(v) = p.get("spacing").and_then(Value::as_f64) {
        if !v.is_finite() || !(0.01..=10.0).contains(&v) {
            return Err(bad(cmd, "`spacing` must be 0.01..10"));
        }
        b.spacing = v as f32;
    }
    if flag(p, "apply", false) {
        b.apply_tip_defaults();
    }
    s.tools.brush = b;
    s.tools.brush_changed();
    Ok(json!({ "scale": s.tools.brush.tip_scale, "rotation": s.tools.brush.tip_rotation, "spacing": s.tools.brush.spacing, "size": s.tools.brush.size }))
}

/// `brush.tips.list`: every predefined tip across the presets, in one chooser list: name,
/// size, scale, rotation, spacing, cells (an image hose), colour (a colour bitmap).
fn tips_list(s: &mut Session, _: &Value) -> Result<Value> {
    let mut seen = std::collections::HashSet::new();
    let mut tips = Vec::new();
    for pr in &s.tools.presets {
        let Some((w, h)) = pr.brush.tip.bitmap_size() else { continue };
        let key = match &pr.brush.tip {
            redrob_photo_paint::TipShape::Stored(r) => r.key.clone(),
            _ => format!("{}:{w}x{h}", pr.name),
        };
        if !seen.insert(key.clone()) {
            continue;
        }
        tips.push(json!({
            "preset": pr.name,
            "key": key,
            "width": w,
            "height": h,
            "scale": pr.brush.tip_scale,
            "rotation": pr.brush.tip_rotation,
            "spacing": pr.brush.spacing,
            "cells": pr.brush.hose.as_ref().map_or(1, |h| h.cells.len()),
            "color": pr.brush.tip_color.is_some(),
            "mode": serde_json::to_value(pr.brush.tip_mode).unwrap_or(Value::Null),
        }));
    }
    Ok(json!({ "tips": tips }))
}

/// `brush.pickTip`: take the predefined tip of preset `preset` into the current brush (its
/// bitmap, hose, colour bitmap, scale, rotation and spacing), applying the tip's own initial
/// size and rotation. Everything else about the brush stays.
fn pick_tip(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.pickTip";
    let name = p.get("preset").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `preset`"))?.to_string();
    let pr = redrob_photo_paint::presets::find(&s.tools.presets, &name).ok_or_else(|| bad(cmd, format!("no brush preset named `{name}`")))?;
    let mut from = pr.brush.clone();
    s.load_brush_tips(&mut from).map_err(|e| bad(cmd, e))?;
    let mut b = s.tools.brush.clone();
    b.tip = from.tip.clone();
    b.hose = from.hose.clone();
    b.tip_color = from.tip_color.clone();
    b.tip_mode = from.tip_mode;
    b.tip_adjust = from.tip_adjust;
    b.tip_scale = from.tip_scale;
    b.tip_rotation = from.tip_rotation;
    b.spacing = from.spacing;
    b.apply_tip_defaults();
    s.tools.brush = b;
    s.tools.brush_changed();
    Ok(json!({ "preset": name, "size": s.tools.brush.size, "angle": s.tools.brush.angle }))
}

/// A grey tile from a colour tile: its luminance (for defining a mask from an image stamp).
pub fn luminance_tile(c: &ColorTile) -> GrayTile {
    GrayTile::from_fn(c.width, c.height, |x, y| c.lightness(x, y))
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $run:expr, $journal:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: always, run: $run, journal: $journal }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("brush.locks.get", "Brush Option Locks", "{} → {locks:[{name, locked}]}", locks_get, false),
        spec!(
            "brush.locks.set",
            "Lock Brush Option",
            r##"{"name":"tip|size|spacing|opacityFlow|hardness|mode|shapeDynamics|scattering|texture|dualBrush|colorDynamics|transfer|pose|noise|wetEdges|buildUp|smoothing|protectTexture","locked":bool=true,"keep":bool=true (false: unlocking takes the current preset's value back)}"##,
            locks_set,
            true
        ),
        spec!("brush.locks.clear", "Unlock All Brush Options", "{}", locks_clear, true),
        spec!("brush.presets.reload", "Reload Brush Preset", r##"{"name":string?=the preset the brush was picked from}"##, presets_reload, true),
        spec!(
            "brush.setEngine",
            "Switch Brush Engine",
            r##"{"kind":"pixel|sketch|hatching|shape|grid|particle|curve|dyna|tangentNormal|myPaint|spray|bristle|deform","settings":{…}?=the engine's defaults}"##,
            set_engine,
            true
        ),
        spec!("brush.history.list", "Brush Preset History", "{} → {items, mode, count}", history_list, false),
        spec!("brush.history.forget", "Forget Brush Preset", r##"{"name":string}"##, history_forget, true),
        spec!("brush.history.clear", "Clear Brush Preset History", "{}", history_clear, true),
        spec!(
            "brush.history.configure",
            "Configure Brush Preset History",
            r##"{"mode":"static|moveToTop|bubbleUp"?,"count":1..100?}"##,
            history_configure,
            true
        ),
        spec!("brush.history.pick", "Pick Brush From History", r##"{"index":0..?|"name":string}"##, history_pick, true),
        spec!("brush.tenBrushes.list", "Ten Brushes", "{} → {slots:[{slot, preset}]}", ten_list, false),
        spec!("brush.tenBrushes.assign", "Assign Ten Brushes Slot", r##"{"slot":1..10,"name":string?=current preset (null clears)}"##, ten_assign, true),
        spec!("brush.tenBrushes.activate", "Activate Ten Brushes Slot", r##"{"slot":1..10}"##, ten_activate, true),
        spec!("brush.tenBrushes.slot1", "Ten Brushes Slot 1", "{}", |s, _| ten_activate(s, &json!({"slot": 1})), true),
        spec!("brush.tenBrushes.slot2", "Ten Brushes Slot 2", "{}", |s, _| ten_activate(s, &json!({"slot": 2})), true),
        spec!("brush.tenBrushes.slot3", "Ten Brushes Slot 3", "{}", |s, _| ten_activate(s, &json!({"slot": 3})), true),
        spec!("brush.tenBrushes.slot4", "Ten Brushes Slot 4", "{}", |s, _| ten_activate(s, &json!({"slot": 4})), true),
        spec!("brush.tenBrushes.slot5", "Ten Brushes Slot 5", "{}", |s, _| ten_activate(s, &json!({"slot": 5})), true),
        spec!("brush.tenBrushes.slot6", "Ten Brushes Slot 6", "{}", |s, _| ten_activate(s, &json!({"slot": 6})), true),
        spec!("brush.tenBrushes.slot7", "Ten Brushes Slot 7", "{}", |s, _| ten_activate(s, &json!({"slot": 7})), true),
        spec!("brush.tenBrushes.slot8", "Ten Brushes Slot 8", "{}", |s, _| ten_activate(s, &json!({"slot": 8})), true),
        spec!("brush.tenBrushes.slot9", "Ten Brushes Slot 9", "{}", |s, _| ten_activate(s, &json!({"slot": 9})), true),
        spec!("brush.tenBrushes.slot10", "Ten Brushes Slot 10", "{}", |s, _| ten_activate(s, &json!({"slot": 10})), true),
        spec!(
            "brush.presets.next",
            "Next Brush Preset",
            r##"{"slots":bool=false (step through the Ten Brushes slots)}"##,
            |s, p| step_preset(s, p, true, "brush.presets.next"),
            true
        ),
        spec!("brush.presets.previous", "Previous Brush Preset", r##"{"slots":bool=false}"##, |s, p| step_preset(s, p, false, "brush.presets.previous"), true),
        spec!(
            "brush.scratchPad.fill",
            "Fill Scratch Pad",
            r##"{"with":"white|background|gradient|image|icon"="white","width":16..1024?,"height":16..1024?}"##,
            pad_fill,
            false
        ),
        spec!("brush.scratchPad.stroke", "Stroke Scratch Pad", r##"{"points":[[x,y,pressure?,…],…],"brush":{…}?,"preset":name?}"##, pad_stroke, false),
        spec!("brush.scratchPad.get", "Scratch Pad Pixels", r##"{"pixels":bool=true} → {width, height, rgba (base64)}"##, pad_get, false),
        spec!(
            "brush.presets.composeThumbnail",
            "Compose Brush Thumbnail",
            r##"{"name":string?=current preset,"background":"white|paper|dark|gradient|canvas|transparent"="white","tint":"#rrggbb"?,"stroke":bool=true,"emblem":"none|brush|pencil|eraser|airbrush|smudge|fill"="none","emblemColor":"#rrggbb"?,"text":string? (up to 3 characters),"textColor":"#rrggbb"?}"##,
            compose_thumbnail_cmd,
            true
        ),
        spec!(
            "brush.presets.setThumbnail",
            "Set Brush Thumbnail",
            r##"{"name":string?=current preset,"rgb":base64?,"width":1..512?,"height":1..512?,"fromScratchPad":bool=false,"clear":bool=false}"##,
            set_thumbnail,
            true
        ),
        spec!("brush.presets.thumbnail", "Brush Thumbnail", r##"{"name":string} → {width, height, rgb (base64)} | {custom:false}"##, get_thumbnail, false),
        spec!(
            "brush.setTipDefaults",
            "Set Tip Defaults",
            r##"{"scale":0.01..100? (initial size as a multiple of the bitmap),"rotation":degrees?,"spacing":0.01..10?,"apply":bool=false (resize and turn the brush now)}"##,
            set_tip_defaults,
            true
        ),
        spec!(
            "brush.tips.list",
            "List Predefined Tips",
            "{} → {tips:[{preset, key, width, height, scale, rotation, spacing, cells, color, mode}]}",
            tips_list,
            false
        ),
        spec!("brush.pickTip", "Pick Predefined Tip", r##"{"preset":string}"##, pick_tip, true),
    ]
}
