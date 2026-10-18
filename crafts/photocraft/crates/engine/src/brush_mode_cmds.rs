// SPDX-License-Identifier: MIT OR Apache-2.0
//! Brush mode and preset-session commands:
//!
//! - `tools.toggleErase`: switch the current brush between painting and erasing. With the
//!   brush's eraser switch on, erase mode keeps its own size and opacity on the same preset.
//! - `tools.setSensorsEnabled`: the "enable pen settings" box for the whole brush: every
//!   dynamic's sensors off (or on) while their curves and settings are kept.
//! - `tools.instantPreview`: the global Instant Preview switch; the per-preset part
//!   is `brush.instantPreview` on the brush.
//! - `brush.presets.tweaks` / `brush.presets.reload` / `brush.presets.discardTweaks`: changes made
//!   to a preset's brush in the session are kept while other presets are picked (the tweaked
//!   brush comes back with the preset, the file is not written); `reload` reverts the current
//!   preset to its saved brush, `discardTweaks` drops every kept tweak.

use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn toggle_erase(s: &mut Session, p: &Value) -> Result<Value> {
    let mut b = s.tools.brush.clone();
    match p.get("erase").and_then(Value::as_bool) {
        Some(e) => {
            b.set_erase(e);
        }
        None => b.toggle_erase(),
    }
    s.tools.brush = b;
    s.tools.brush_changed();
    let b = &s.tools.brush;
    Ok(json!({ "erase": b.erase, "size": b.size, "opacity": b.opacity }))
}

fn set_sensors_enabled(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "tools.setSensorsEnabled";
    let on = p.get("enabled").and_then(Value::as_bool).ok_or_else(|| bad(cmd, "`enabled` must be a bool"))?;
    let mut b = s.tools.brush.clone();
    b.set_sensors_enabled(on);
    s.tools.brush = b;
    s.tools.brush_changed();
    Ok(json!({ "enabled": on }))
}

fn instant_preview(s: &mut Session, p: &Value) -> Result<Value> {
    let now = s.prefs().tools.instant_preview;
    let next = p.get("enabled").and_then(Value::as_bool).unwrap_or(!now);
    if next != now {
        s.edit_prefs(|pr| pr.tools.instant_preview = next);
    }
    let b = &s.tools.brush;
    Ok(
        json!({ "enabled": next, "threshold": b.instant_preview.threshold, "presetEnabled": b.instant_preview.enabled, "active": b.preview_brush(next).is_some() }),
    )
}

/// Load the colour pattern a Pattern / Locked Pattern colour source paints (`patternKey`, else
/// the Patterns panel's current pattern) into the brush, so the stroke renderer can read its
/// pixels. Brushes with another source are left alone. A missing pattern is an error naming it.
pub fn load_color_pattern(s: &Session, b: &mut redrob_photo_paint::BrushSettings, cmd: &str) -> Result<()> {
    let cd = &mut b.color_dynamics;
    if !(cd.enabled && cd.source.uses_pattern()) {
        return Ok(());
    }
    let key = cd.pattern_key.clone();
    let pat =
        crate::pattern_cmds::resolve(s, &key).ok_or_else(|| bad(cmd, format!("no pattern \"{key}\" for the brush's colour source (see pattern.list)")))?;
    let (w, h) = (pat.width.clamp(1, 4096), pat.height.clamp(1, 4096));
    let mut data = Vec::with_capacity((w * h) as usize * 4);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            data.extend_from_slice(&pat.surface.rgba(x, y));
        }
    }
    cd.pattern = Some(std::sync::Arc::new(redrob_photo_paint::brush::ColorTile { width: w, height: h, data }));
    Ok(())
}

/// Keep the current preset's tweaked brush aside when another preset is about to be picked
/// (called by `tools.setBrush`), so picking it again in this session brings the tweaks back.
pub(crate) fn park_tweaks(s: &mut Session, next: &str) {
    let Some(cur) = s.tools.current_preset.clone() else { return };
    if cur.eq_ignore_ascii_case(next) {
        return;
    }
    let Some(pr) = redrob_photo_paint::presets::find(&s.tools.presets, &cur) else { return };
    if is_dirty(&s.tools.brush, &pr.brush) {
        s.tools.preset_tweaks.insert(cur.to_ascii_lowercase(), s.tools.brush.clone());
    } else {
        s.tools.preset_tweaks.remove(&cur.to_ascii_lowercase());
    }
}

/// The tweaked brush kept for preset `name`, if any.
pub(crate) fn tweak_of(s: &Session, name: &str) -> Option<&redrob_photo_paint::BrushSettings> {
    s.tools.preset_tweaks.get(&name.to_ascii_lowercase())
}

/// Does `brush` differ from `saved` in what a preset stores? Tool state (smoothing, locks, the
/// colour, the seed) is not a tweak.
pub fn is_dirty(brush: &redrob_photo_paint::BrushSettings, saved: &redrob_photo_paint::BrushSettings) -> bool {
    let mut a = brush.clone();
    let mut b = saved.clone();
    for x in [&mut a, &mut b] {
        x.smoothing = Default::default();
        x.locks = Default::default();
        x.color = [0.0; 4];
        x.color_cmyk = None;
        x.background = [0.0; 4];
        x.seed = 0;
        x.erase = false;
        x.anti_erase = false;
    }
    a != b
}

fn tweaks(s: &mut Session, _p: &Value) -> Result<Value> {
    let current = s.tools.current_preset.clone();
    let current_dirty =
        current.as_deref().and_then(|n| redrob_photo_paint::presets::find(&s.tools.presets, n)).is_some_and(|pr| is_dirty(&s.tools.brush, &pr.brush));
    let mut names: Vec<String> =
        s.tools.preset_tweaks.keys().filter_map(|k| redrob_photo_paint::presets::find(&s.tools.presets, k).map(|pr| pr.name.clone())).collect();
    if let (true, Some(c)) = (current_dirty, current.as_ref())
        && !names.iter().any(|n| n.eq_ignore_ascii_case(c))
    {
        names.push(c.clone());
    }
    names.sort();
    Ok(json!({ "current": current, "currentDirty": current_dirty, "tweaked": names }))
}

fn reload(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.reload";
    let name = match p.get("name").and_then(Value::as_str) {
        Some(n) => n.to_string(),
        None => s.tools.current_preset.clone().ok_or_else(|| bad(cmd, "no current preset to reload"))?,
    };
    let pr = redrob_photo_paint::presets::find(&s.tools.presets, &name).ok_or_else(|| bad(cmd, format!("no brush preset named `{name}`")))?.clone();
    s.tools.preset_tweaks.remove(&name.to_ascii_lowercase());
    let is_current = s.tools.current_preset.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&name));
    if is_current {
        let mut b = pr.picked_over(&s.tools.brush);
        s.load_brush_tips(&mut b).map_err(|e| bad(cmd, e))?;
        s.tools.brush = b;
        s.tools.brush_changed();
    }
    Ok(json!({ "name": pr.name, "reloaded": is_current }))
}

fn discard_tweaks(s: &mut Session, _p: &Value) -> Result<Value> {
    let n = s.tools.preset_tweaks.len();
    s.tools.preset_tweaks.clear();
    Ok(json!({ "discarded": n }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "tools.toggleErase",
            label: "Toggle Erase Mode",
            menu: &[],
            shortcut: None,
            params: r##"{"erase":bool? (omit: toggle)} → {erase, size, opacity} (with the brush's eraserSwitch, erase mode keeps its own size and opacity)"##,
            enabled: always,
            run: toggle_erase,
            journal: true,
        },
        CommandSpec {
            id: "tools.setSensorsEnabled",
            label: "Enable Pen Settings",
            menu: &[],
            shortcut: None,
            params: r##"{"enabled":bool} (every dynamic's sensors on or off; curves and settings are kept)"##,
            enabled: always,
            run: set_sensors_enabled,
            journal: true,
        },
        CommandSpec {
            id: "tools.instantPreview",
            label: "Instant Preview",
            menu: &[],
            shortcut: None,
            params: r##"{"enabled":bool? (omit: toggle)} → {enabled, threshold, presetEnabled, active}"##,
            enabled: always,
            run: instant_preview,
            journal: true,
        },
        CommandSpec {
            id: "brush.presets.tweaks",
            label: "Tweaked Brush Presets",
            menu: &[],
            shortcut: None,
            params: r##"{} → {current, currentDirty, tweaked: [name]}"##,
            enabled: always,
            run: tweaks,
            journal: false,
        },
        CommandSpec {
            id: "brush.presets.reload",
            label: "Reload Brush Preset",
            menu: &[],
            shortcut: None,
            params: r##"{"name":string?=the current preset} → {name, reloaded} (drops the session's tweaks of the preset; the current preset's brush reverts to the saved one)"##,
            enabled: always,
            run: reload,
            journal: true,
        },
        CommandSpec {
            id: "brush.presets.discardTweaks",
            label: "Discard Preset Tweaks",
            menu: &[],
            shortcut: None,
            params: r##"{} → {discarded}"##,
            enabled: always,
            run: discard_tweaks,
            journal: true,
        },
    ]
}
