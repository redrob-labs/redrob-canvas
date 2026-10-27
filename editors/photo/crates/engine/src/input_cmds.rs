//! Edit › Input Devices… and Edit › Input Controllers… (Photo mode, v0.11 parity): the per-device
//! configuration (mode, tilt, rotation, pressure scale) and the controller mappings (wheel
//! notches and keyboard shortcuts bound to commands) that persist in Preferences › Tools. The UI
//! applies the device settings to the stylus and runs the mapped commands when the events arrive.

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::prefs::{ControllerMapping, InputDevice};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Device modes.
pub const MODES: &[&str] = &["disabled", "screen", "window"];
/// Wheel directions a controller event names.
pub const WHEEL_DIRECTIONS: &[&str] = &["up", "down", "left", "right"];

fn device_json(d: &InputDevice) -> Value {
    json!({
        "name": d.name, "mode": d.mode, "tilt": d.tilt, "rotation": d.rotation,
        "invertTiltX": d.invert_tilt_x, "invertTiltY": d.invert_tilt_y, "swapTilt": d.swap_tilt, "pressureScale": d.pressure_scale,
    })
}

fn mapping_json(m: &ControllerMapping) -> Value {
    json!({ "controller": m.controller, "event": m.event, "action": m.action })
}

/// The pen's configuration (the `pen` entry, or the defaults).
pub fn pen_device(s: &Session) -> InputDevice {
    s.prefs().tools.input_devices.iter().find(|d| d.name == "pen").cloned().unwrap_or_default()
}

fn parse_device(v: &Value, cmd: &str) -> Result<InputDevice> {
    let d = InputDevice::default();
    let name = v.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "a device needs a `name`"))?;
    let mode = v.get("mode").and_then(Value::as_str).unwrap_or(&d.mode);
    if !MODES.contains(&mode) {
        return Err(bad(cmd, format!("device `{name}`: `mode` must be disabled, screen or window")));
    }
    let b = |k: &str, dflt: bool| v.get(k).and_then(Value::as_bool).unwrap_or(dflt);
    let scale = v.get("pressureScale").and_then(Value::as_f64).unwrap_or(f64::from(d.pressure_scale));
    if !scale.is_finite() || !(0.1..=4.0).contains(&scale) {
        return Err(bad(cmd, format!("device `{name}`: `pressureScale` is 0.1 to 4")));
    }
    Ok(InputDevice {
        name: name.chars().take(64).collect(),
        mode: mode.to_string(),
        tilt: b("tilt", d.tilt),
        rotation: b("rotation", d.rotation),
        invert_tilt_x: b("invertTiltX", d.invert_tilt_x),
        invert_tilt_y: b("invertTiltY", d.invert_tilt_y),
        swap_tilt: b("swapTilt", d.swap_tilt),
        pressure_scale: scale as f32,
    })
}

/// Checks a controller event string: wheel `[Shift+][Ctrl+][Alt+](up|down|left|right)`, keyboard
/// a `Cmd+Shift+N`-style shortcut (one key name, optional modifiers).
pub fn valid_event(controller: &str, event: &str) -> bool {
    let parts: Vec<&str> = event.split('+').map(str::trim).collect();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return false;
    }
    let (mods, key) = parts.split_at(parts.len() - 1);
    let mods_ok = mods.iter().all(|m| matches!(*m, "Shift" | "Ctrl" | "Alt" | "Cmd"));
    match controller {
        "wheel" => mods_ok && WHEEL_DIRECTIONS.contains(&key[0]),
        "keyboard" => mods_ok && !key[0].is_empty() && key[0].len() <= 16,
        _ => false,
    }
}

/// Splits an action into (command id, params): `view.zoomIn` or `brush.size {"delta": 5}`.
pub fn split_action(action: &str) -> Option<(String, Value)> {
    let action = action.trim();
    let (id, rest) = action.split_once(' ').map_or((action, ""), |(a, b)| (a, b.trim()));
    if id.is_empty() {
        return None;
    }
    let params = if rest.is_empty() { json!({}) } else { serde_json::from_str(rest).ok()? };
    Some((id.to_string(), params))
}

fn parse_mapping(v: &Value, cmd: &str) -> Result<ControllerMapping> {
    let controller = v.get("controller").and_then(Value::as_str).unwrap_or("wheel");
    if !matches!(controller, "wheel" | "keyboard") {
        return Err(bad(cmd, "`controller` must be wheel or keyboard"));
    }
    let event = v.get("event").and_then(Value::as_str).map(str::trim).filter(|e| !e.is_empty()).ok_or_else(|| bad(cmd, "a mapping needs an `event`"))?;
    if !valid_event(controller, event) {
        return Err(bad(cmd, format!("`{event}` is not a {controller} event (wheel: [Shift+][Ctrl+][Alt+]up|down|left|right; keyboard: Cmd+Shift+N)")));
    }
    let action = v.get("action").and_then(Value::as_str).map(str::trim).filter(|a| !a.is_empty()).ok_or_else(|| bad(cmd, "a mapping needs an `action`"))?;
    let (id, _) = split_action(action).ok_or_else(|| bad(cmd, format!("`{action}` is not a command id with optional JSON params")))?;
    if crate::commands::find(&id).is_none() && !id.starts_with("ui.") && !id.starts_with("view.") && !id.starts_with("window.") {
        return Err(bad(cmd, format!("`{id}` is not a known command")));
    }
    Ok(ControllerMapping { controller: controller.to_string(), event: event.to_string(), action: action.to_string() })
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let t = &s.prefs().tools;
    let mut devices: Vec<Value> = t.input_devices.iter().map(device_json).collect();
    if !t.input_devices.iter().any(|d| d.name == "pen") {
        devices.insert(0, device_json(&InputDevice::default()));
    }
    Ok(json!({
        "devices": devices,
        "controllers": t.input_controllers.iter().map(mapping_json).collect::<Vec<_>>(),
        "modes": MODES,
    }))
}

fn set_devices(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "input.devices.set";
    let list = p
        .get("devices")
        .and_then(Value::as_array)
        .ok_or_else(|| bad(CMD, "pass `devices`: [{name, mode, tilt, rotation, invertTiltX, invertTiltY, swapTilt, pressureScale}]"))?;
    if list.len() > 32 {
        return Err(bad(CMD, "at most 32 devices"));
    }
    let devices: Vec<InputDevice> = list.iter().map(|v| parse_device(v, CMD)).collect::<Result<_>>()?;
    let n = devices.len();
    s.edit_prefs(|prefs| prefs.tools.input_devices = devices);
    Ok(json!({ "devices": n }))
}

fn set_controllers(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "input.controllers.set";
    let list = p.get("mappings").and_then(Value::as_array).ok_or_else(|| bad(CMD, "pass `mappings`: [{controller, event, action}]"))?;
    if list.len() > 128 {
        return Err(bad(CMD, "at most 128 mappings"));
    }
    let mappings: Vec<ControllerMapping> = list.iter().map(|v| parse_mapping(v, CMD)).collect::<Result<_>>()?;
    let n = mappings.len();
    s.edit_prefs(|prefs| prefs.tools.input_controllers = mappings);
    Ok(json!({ "mappings": n }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "input.list",
            label: "List Input Devices and Controllers",
            menu: &[],
            shortcut: None,
            params: r##"{} → {devices: [...], controllers: [...], modes}"##,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "input.devices.set",
            label: "Configure Input Devices",
            menu: &[],
            shortcut: None,
            params: r##"{"devices":[{"name":"pen","mode":"disabled|screen|window","tilt":bool,"rotation":bool,"invertTiltX":bool,"invertTiltY":bool,"swapTilt":bool,"pressureScale":0.1-4}]} → {devices}"##,
            enabled: always,
            run: set_devices,
            journal: false,
        },
        CommandSpec {
            id: "input.controllers.set",
            label: "Configure Input Controllers",
            menu: &[],
            shortcut: None,
            params: r##"{"mappings":[{"controller":"wheel|keyboard","event":"[Shift+][Ctrl+][Alt+]up|down|left|right or Cmd+Shift+N","action":"command id [JSON params]"}]} → {mappings}"##,
            enabled: always,
            run: set_controllers,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mappings_and_devices_validate_and_persist() {
        let mut s = Session::new();
        s.execute("input.controllers.set", json!({"mappings": [{"controller": "wheel", "event": "Alt+up", "action": "view.zoomIn"}]}))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(s.execute("input.controllers.set", json!({"mappings": [{"controller": "wheel", "event": "sideways", "action": "view.zoomIn"}]})).is_err());
        assert!(s.execute("input.controllers.set", json!({"mappings": [{"controller": "wheel", "event": "up", "action": "no.such.command"}]})).is_err());
        s.execute("input.devices.set", json!({"devices": [{"name": "pen", "mode": "screen", "tilt": false, "pressureScale": 1.5}]}))
            .unwrap_or_else(|e| panic!("{e}"));
        let pen = pen_device(&s);
        assert!(!pen.tilt);
        assert!((pen.pressure_scale - 1.5).abs() < 1e-6);
        let l = s.execute("input.list", json!({})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(l["controllers"].as_array().map(Vec::len), Some(1));
        assert_eq!(split_action("brush.size {\"delta\": 5}").map(|(id, p)| (id, p["delta"].as_i64())), Some(("brush.size".to_string(), Some(5))));
    }
}
