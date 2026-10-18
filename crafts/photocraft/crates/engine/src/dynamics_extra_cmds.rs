//! Paint-dynamics extras: Colour from gradient along the stroke (`brush.colorFromGradient`) and
//! the Clipboard Image brush (`brush.presets.clipboard`).
//!
//! Colour from gradient sets the current brush's [`BrushSettings::color_gradient`] from the
//! Gradients panel's current gradient (or a named preset), with a fade length in pixels and a
//! repeat mode; a stroke then runs through the gradient instead of painting one colour. The
//! clipboard brush turns the engine clipboard's pixels into a sampled tip preset named
//! "Clipboard Image" and makes it current, so the last copied image paints.

use redrob_photo_paint::{BrushPreset, BrushSettings};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// The name of the clipboard brush preset.
pub const CLIPBOARD_BRUSH: &str = "Clipboard Image";

/// `brush.colorFromGradient`: switch Colour from gradient on (with the current or a named
/// gradient) or off for the current brush.
fn color_from_gradient(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "brush.colorFromGradient";
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
    if !on {
        s.tools.brush.color_gradient.clear();
        s.tools.brush_changed();
        return Ok(json!({ "on": false }));
    }
    let q = match p.get("gradient").and_then(Value::as_str) {
        Some(name) => json!({ "gradient": name }),
        None => json!({}),
    };
    let stops = crate::presets::gradients::tool_stops(s, &q)?.ok_or_else(|| bad(CMD, "no gradient"))?;
    if stops.is_empty() {
        return Err(bad(CMD, "the gradient has no colour stops"));
    }
    let length = crate::eraser_cmds::number(p, CMD, "length", 100.0, 1.0..=100_000.0)? as f32;
    let repeat = match p.get("repeat").and_then(Value::as_str).unwrap_or("none") {
        "none" | "truncate" => String::new(),
        r @ ("sawtooth" | "triangular") => r.to_string(),
        _ => {
            return Err(bad(CMD, "`repeat` must be one of none|sawtooth|triangular|truncate"));
        }
    };
    let b: &mut BrushSettings = &mut s.tools.brush;
    b.color_gradient = stops;
    b.color_gradient_length = length;
    b.color_gradient_repeat = repeat;
    s.tools.brush_changed();
    Ok(json!({ "on": true, "stops": s.tools.brush.color_gradient.len(), "length": length }))
}

fn has_clipboard(s: &Session) -> std::result::Result<(), String> {
    s.clipboard.as_ref().map(|_| ()).ok_or_else(|| "the clipboard is empty".into())
}

/// `brush.presets.clipboard`: the clipboard's pixels as the "Clipboard Image" brush, selected.
fn clipboard_brush(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "brush.presets.clipboard";
    let clip = s.clipboard.clone().ok_or_else(|| bad(CMD, "the clipboard is empty"))?;
    let area = clip.bounds;
    let (w, h) = (area.width() as usize, area.height() as usize);
    if w == 0 || h == 0 {
        return Err(bad(CMD, "the clipboard image has no size"));
    }
    if w.max(h) > 4096 {
        return Err(bad(CMD, format!("the clipboard image is too large for a brush tip ({w}×{h}; max 4096 px)")));
    }
    // Paint amount = darkness × alpha (black = full paint), as Define Brush Preset does; a
    // colour clipboard image becomes a grayscale mask, like a colour bitmap brush.
    let mut v = vec![0.0f32; w * h];
    let mut px = vec![[0.0f32; 4]; w * h];
    clip.surface.read_rgba_into(area, &mut px);
    for (o, c) in v.iter_mut().zip(&px) {
        let l = redrob_photo_color::blend::lum([c[0], c[1], c[2]]);
        *o = ((1.0 - l) * c[3]).clamp(0.0, 1.0);
    }
    let (brush, tw, th) = crate::brush_cmds::sampled_brush(CMD, &v, w, h)?;
    let name = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).unwrap_or(CLIPBOARD_BRUSH).to_string();
    crate::brush_cmds::upsert(
        s,
        BrushPreset { name: name.clone(), brush: brush.clone(), builtin: false, group: String::new(), folder: Vec::new(), captures: Default::default() },
    );
    if p.get("select").and_then(Value::as_bool).unwrap_or(true) {
        s.tools.brush = brush;
        s.tools.current_preset = Some(name.clone());
        s.tools.brush_changed();
    }
    Ok(json!({ "name": name, "width": tw, "height": th }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "brush.colorFromGradient",
            label: "Color from Gradient",
            menu: &[],
            shortcut: None,
            params: r##"{"on":bool=true,"gradient":preset name?=the current gradient,"length":px=100 (stroke length of one run through the gradient),"repeat":"none|sawtooth|triangular"="none"} → {on, stops, length}"##,
            enabled: crate::commands::always,
            run: color_from_gradient,
            journal: true,
        },
        CommandSpec {
            id: "brush.presets.clipboard",
            label: "Clipboard Image Brush",
            menu: &[],
            shortcut: None,
            params: r##"{"name":string="Clipboard Image","select":bool=true} → {name, width, height}"##,
            enabled: has_clipboard,
            run: clipboard_brush,
            journal: true,
        },
    ]
}
