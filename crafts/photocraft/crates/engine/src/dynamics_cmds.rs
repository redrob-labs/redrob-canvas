//! Paint dynamics presets (`brush.dynamics.*`): a named table mapping input sensors onto brush
//! properties ([`DynamicsPreset`]), applied to the current brush, saved as `.rdyn` files in the
//! preset store and listed for the Brush Settings panel's Dynamics pop-up.
//!
//! - `brush.dynamics.list` lists the presets (built-ins first, then the user's).
//! - `brush.dynamics.apply` rewrites the current brush's dynamics sections from a preset.
//! - `brush.dynamics.save` saves a preset given in full, or the current brush's dynamics as one.
//! - `brush.dynamics.delete` removes a user preset (and its file).

use redrob_photo_paint::DynamicsPreset;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Most user dynamics presets kept.
pub const MAX_PRESETS: usize = 1000;

fn name_param(p: &Value, cmd: &str) -> Result<String> {
    p.get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty() && n.chars().count() <= 100)
        .map(str::to_string)
        .ok_or_else(|| bad(cmd, "`name` must be a non-empty string of at most 100 characters"))
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let items: Vec<Value> = s.tools.dynamics_presets.iter().map(|d| serde_json::to_value(d).unwrap_or(Value::Null)).collect();
    Ok(json!({ "presets": items, "current": s.tools.current_dynamics }))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.dynamics.apply";
    let preset = match p.get("preset") {
        Some(v) => serde_json::from_value::<DynamicsPreset>(v.clone()).map_err(|e| bad(cmd, format!("`preset`: {e}")))?,
        None => {
            let name = name_param(p, cmd)?;
            redrob_photo_paint::dynamics_preset::find(&s.tools.dynamics_presets, &name)
                .cloned()
                .ok_or_else(|| bad(cmd, format!("no dynamics preset named `{name}`")))?
        }
    };
    let mut b = s.tools.brush.clone();
    let un = preset.apply(&mut b);
    s.tools.brush = b;
    s.tools.current_dynamics = Some(preset.name.clone());
    s.tools.brush_changed();
    let mut warnings = Vec::new();
    if !un.is_empty() {
        warnings.push(un.note());
    }
    Ok(json!({ "name": preset.name, "warnings": warnings }))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.dynamics.save";
    let mut preset = match p.get("preset") {
        Some(v) => serde_json::from_value::<DynamicsPreset>(v.clone()).map_err(|e| bad(cmd, format!("`preset`: {e}")))?,
        None => DynamicsPreset::from_brush(&name_param(p, cmd)?, &s.tools.brush),
    };
    if let Some(n) = p.get("name").filter(|_| p.get("preset").is_some()) {
        preset.name = n.as_str().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "`name` must be a non-empty string"))?.to_string();
    }
    preset.name = preset.name.trim().to_string();
    if preset.name.is_empty() || preset.name.chars().count() > 100 {
        return Err(bad(cmd, "the preset needs a name of at most 100 characters"));
    }
    if redrob_photo_paint::dynamics_preset::find(&s.tools.dynamics_presets, &preset.name).is_some_and(|d| d.builtin) {
        return Err(bad(cmd, format!("`{}` is a built-in dynamics preset; choose another name", preset.name)));
    }
    preset.builtin = false;
    preset.format = redrob_photo_paint::dynamics_preset::FORMAT.into();
    preset.version = 1;
    preset.fade_steps = preset.fade_steps.clamp(1, 9999);
    preset.random_amount = if preset.random_amount.is_finite() { preset.random_amount.clamp(0.0, 1.0) } else { 0.5 };
    for r in &mut preset.rows {
        r.minimum = if r.minimum.is_finite() { r.minimum.clamp(0.0, 1.0) } else { 0.0 };
        r.inputs.dedup();
    }
    let replaced = match s.tools.dynamics_presets.iter_mut().find(|d| d.name.eq_ignore_ascii_case(&preset.name)) {
        Some(d) => {
            *d = preset.clone();
            true
        }
        None => {
            if s.tools.dynamics_presets.len() >= MAX_PRESETS {
                return Err(bad(cmd, format!("at most {MAX_PRESETS} dynamics presets")));
            }
            s.tools.dynamics_presets.push(preset.clone());
            false
        }
    };
    let mut warnings = Vec::new();
    if let Some(st) = s.preset_store.as_ref()
        && let Err(e) = st.write_dynamics(&preset)
    {
        warnings.push(format!("Couldn't save the dynamics preset file: {e}"));
    }
    Ok(json!({ "name": preset.name, "replaced": replaced, "preset": serde_json::to_value(&preset).unwrap_or(Value::Null), "warnings": warnings }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.dynamics.delete";
    let name = name_param(p, cmd)?;
    let Some(i) = s.tools.dynamics_presets.iter().position(|d| d.name.eq_ignore_ascii_case(&name)) else {
        return Err(bad(cmd, format!("no dynamics preset named `{name}`")));
    };
    if s.tools.dynamics_presets[i].builtin {
        return Err(bad(cmd, format!("`{name}` is a built-in dynamics preset and can't be deleted")));
    }
    let removed = s.tools.dynamics_presets.remove(i);
    if s.tools.current_dynamics.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&removed.name)) {
        s.tools.current_dynamics = None;
    }
    let mut warnings = Vec::new();
    if let Some(st) = s.preset_store.as_ref()
        && let Err(e) = st.remove_dynamics(&removed.name)
    {
        warnings.push(format!("Couldn't delete the dynamics preset file: {e}"));
    }
    Ok(json!({ "name": removed.name, "warnings": warnings }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "brush.dynamics.list",
            label: "Dynamics Presets",
            menu: &[],
            shortcut: None,
            params: r##"{} → {presets: [{name, builtin, rows: [{output, inputs, curve, minimum}], fadeSteps, randomAmount}], current}"##,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "brush.dynamics.apply",
            label: "Apply Dynamics Preset",
            menu: &[],
            shortcut: None,
            params: r##"{"name":string? (a listed preset),"preset":object? (a full preset instead)} → {name, warnings (outputs the brush has no dynamic for)}"##,
            enabled: always,
            run: apply,
            journal: true,
        },
        CommandSpec {
            id: "brush.dynamics.save",
            label: "Save Dynamics Preset",
            menu: &[],
            shortcut: None,
            params: r##"{"name":string? (without `preset`: save the current brush's dynamics under it),"preset":object? (a full preset; `name` overrides its name)} → {name, replaced, preset, warnings}"##,
            enabled: always,
            run: save,
            journal: true,
        },
        CommandSpec {
            id: "brush.dynamics.delete",
            label: "Delete Dynamics Preset",
            menu: &[],
            shortcut: None,
            params: r##"{"name":string} → {name, warnings}"##,
            enabled: always,
            run: delete,
            journal: true,
        },
    ]
}
