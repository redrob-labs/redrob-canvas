//! Edit › Input Devices… and Edit › Input Controllers… dialogs, and the runtime that applies
//! them: the pen's configuration goes to the stylus each frame ([`apply_device`]), and wheel
//! notches or keyboard shortcuts bound in the controller mappings run their commands
//! ([`take_events`]) before the canvas sees those events.

use egui::{Event, Modifiers, MouseWheelUnit};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const DEVICES: &str = "__inputDevices";
const CONTROLLERS: &str = "__inputControllers";

pub fn handles(id: &str) -> bool {
    matches!(id, "edit.inputDevices" | "edit.inputControllers")
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(DEVICES) || fields.contains_key(CONTROLLERS)
}

pub fn dialog_width(fields: &Map<String, Value>) -> Option<f32> {
    owns(fields).then_some(600.0)
}

pub fn menu(app: &mut PhotocraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    match id {
        "edit.inputDevices" => Some(open(app, DEVICES, "edit.inputDevices", "Input Devices…")),
        "edit.inputControllers" => Some(open(app, CONTROLLERS, "edit.inputControllers", "Input Controllers…")),
        _ => None,
    }
}

fn open(app: &mut PhotocraftApp, mark: &str, command: &str, label: &str) -> Result<Value, String> {
    let list = app.run("input.list", json!({}))?;
    let mut f = Map::new();
    f.insert(mark.into(), json!(true));
    f.insert("__command".into(), json!(command));
    f.insert("__label".into(), json!(label));
    f.insert("devices".into(), list.get("devices").cloned().unwrap_or_else(|| json!([])));
    f.insert("controllers".into(), list.get("controllers").cloned().unwrap_or_else(|| json!([])));
    f.insert("new".into(), json!({"controller": "wheel", "event": "Alt+up", "action": "view.zoomIn"}));
    Ok(json!({ "dialog": app.ui.open_dialog(DialogKind::Command, f) }))
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if let Some(error) = f.get("__error").and_then(Value::as_str) {
        ui.colored_label(ui.visuals().error_fg_color, error);
        ui.add_space(4.0);
    }
    if f.contains_key(DEVICES) {
        devices_body(ui, f);
    } else {
        controllers_body(ui, f);
    }
}

fn devices_body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut devices: Vec<Value> = f.get("devices").and_then(Value::as_array).cloned().unwrap_or_default();
    ui.weak(tl!("The pen is the tablet stylus the platform reports. Disabled makes it act as a mouse."));
    ui.add_space(6.0);
    let modes = redrob_photo_engine::input_cmds::MODES;
    egui::Grid::new("input-devices").num_columns(8).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
        for h in ["Device", "Mode", "Tilt", "Rotation", "Invert tilt X", "Invert tilt Y", "Swap tilt", "Pressure scale"] {
            ui.strong(tl!(h));
        }
        ui.end_row();
        for (i, d) in devices.iter_mut().enumerate() {
            ui.label(d.get("name").and_then(Value::as_str).unwrap_or("?"));
            let mut mode = d.get("mode").and_then(Value::as_str).unwrap_or("screen").to_string();
            let opts: Vec<(String, String)> = modes.iter().map(|m| (m.to_string(), tl!(&crate::filter_dialog::label(m)).to_string())).collect();
            let refs: Vec<(String, &str)> = opts.iter().map(|(a, b)| (a.clone(), b.as_str())).collect();
            if crate::widgets::dropdown(ui, &format!("input-device-mode-{i}"), &mut mode, &refs, 110.0) {
                d["mode"] = json!(mode);
            }
            for k in ["tilt", "rotation", "invertTiltX", "invertTiltY", "swapTilt"] {
                let mut b = d.get(k).and_then(Value::as_bool).unwrap_or(k == "tilt" || k == "rotation");
                if ui.checkbox(&mut b, "").changed() {
                    d[k] = json!(b);
                }
            }
            let mut scale = d.get("pressureScale").and_then(Value::as_f64).unwrap_or(1.0);
            if ui.add(egui::DragValue::new(&mut scale).speed(0.01).range(0.1..=4.0).fixed_decimals(2)).changed() {
                d["pressureScale"] = json!(scale);
            }
            ui.end_row();
        }
    });
    f.insert("devices".into(), Value::Array(devices));
}

fn controllers_body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut mappings: Vec<Value> = f.get("controllers").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut new = f.get("new").cloned().unwrap_or_else(|| json!({"controller": "wheel", "event": "Alt+up", "action": "view.zoomIn"}));
    ui.weak(tl!("Wheel events are up, down, left or right with optional Shift+, Ctrl+ and Alt+; keyboard events are shortcuts such as Ctrl+Alt+Up. The action is a command id, optionally followed by JSON parameters."));
    ui.add_space(6.0);
    let mut removed: Option<usize> = None;
    let row = |ui: &mut egui::Ui, m: &mut Value, i: usize| {
        let mut controller = m.get("controller").and_then(Value::as_str).unwrap_or("wheel").to_string();
        let opts = [("wheel".to_string(), tl!("Wheel")), ("keyboard".to_string(), tl!("Keyboard"))];
        let refs: Vec<(String, &str)> = opts.iter().map(|(a, b)| (a.clone(), *b)).collect();
        if crate::widgets::dropdown(ui, &format!("input-controller-{i}"), &mut controller, &refs, 100.0) {
            m["controller"] = json!(controller);
        }
        for (k, w) in [("event", 120.0), ("action", 220.0)] {
            let mut s = m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            if ui.add(egui::TextEdit::singleline(&mut s).desired_width(w)).changed() {
                m[k] = json!(s);
            }
        }
    };
    egui::ScrollArea::vertical().id_salt("input-controllers").max_height(280.0).show(ui, |ui| {
        egui::Grid::new("input-controllers-grid").num_columns(4).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
            for h in ["Controller", "Event", "Action"] {
                ui.strong(tl!(h));
            }
            ui.label("");
            ui.end_row();
            for (i, m) in mappings.iter_mut().enumerate() {
                row(ui, m, i);
                if crate::icons::button(ui, "trash", 18.0, false, tl!("Remove mapping")).clicked() {
                    removed = Some(i);
                }
                ui.end_row();
            }
        });
    });
    if let Some(i) = removed {
        mappings.remove(i);
    }
    ui.add_space(8.0);
    ui.label(tl!("New mapping"));
    egui::Grid::new("input-controllers-new").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
        row(ui, &mut new, usize::MAX);
        if ui.button(tl!("Add Mapping")).clicked() {
            mappings.push(new.clone());
        }
        ui.end_row();
    });
    f.insert("controllers".into(), Value::Array(mappings));
    f.insert("new".into(), new);
}

pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let result = if f.contains_key(DEVICES) {
        app.run("input.devices.set", json!({"devices": f.get("devices").cloned().unwrap_or_else(|| json!([]))}))
    } else {
        app.run("input.controllers.set", json!({"mappings": f.get("controllers").cloned().unwrap_or_else(|| json!([]))}))
    };
    if let Err(e) = &result {
        let mut again = f.clone();
        again.insert("__error".into(), json!(e));
        app.ui.open_dialog(DialogKind::Command, again);
    }
    result
}

/// Each frame: the pen's configuration into the stylus. `use_pressure` was set from Preferences ›
/// Tools just before; a disabled pen acts as a mouse.
pub fn apply_device(app: &mut PhotocraftApp) {
    let pen = redrob_photo_engine::input_cmds::pen_device(&app.session);
    if pen.mode == "disabled" {
        app.stylus.use_pressure = false;
    }
    app.stylus.device = crate::stylus::DeviceAdjust {
        tilt: pen.tilt,
        rotation: pen.rotation,
        invert_tilt_x: pen.invert_tilt_x,
        invert_tilt_y: pen.invert_tilt_y,
        swap_tilt: pen.swap_tilt,
        pressure_scale: pen.pressure_scale,
    };
}

fn modifiers_of(event: &str) -> (Modifiers, &str) {
    let mut m = Modifiers::NONE;
    let mut key = event;
    while let Some((prefix, rest)) = key.split_once('+') {
        match prefix.trim() {
            "Shift" => m.shift = true,
            "Ctrl" => m.ctrl = true,
            "Alt" => m.alt = true,
            "Cmd" => m.command = true,
            _ => break,
        }
        key = rest;
    }
    (m, key.trim())
}

/// Consumes the wheel notches (while the canvas is hovered) and keyboard shortcuts bound by the
/// controller mappings and runs their commands. Returns how many ran.
pub fn take_events(app: &mut PhotocraftApp, ctx: &egui::Context, canvas_hovered: bool) -> usize {
    let mappings = app.session.prefs().tools.input_controllers.clone();
    if mappings.is_empty() {
        return 0;
    }
    let mut actions: Vec<String> = Vec::new();
    // Keyboard shortcuts first (consumed whatever is hovered, as the command's own shortcut would be).
    for m in mappings.iter().filter(|m| m.controller == "keyboard") {
        if let Some(sc) = crate::shortcuts::parse(&m.event)
            && ctx.input_mut(|i| i.consume_shortcut(&sc))
        {
            actions.push(m.action.clone());
        }
    }
    if canvas_hovered && mappings.iter().any(|m| m.controller == "wheel") {
        let notch = ctx.options(|o| o.input_options.line_scroll_speed).max(1.0);
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                let Event::MouseWheel { unit, delta, modifiers, .. } = e else { return true };
                let d = if *unit == MouseWheelUnit::Point { *delta / notch } else { *delta };
                let dir = if d.y.abs() >= d.x.abs() {
                    if d.y > 0.0 { "up" } else { "down" }
                } else if d.x > 0.0 {
                    "right"
                } else {
                    "left"
                };
                if d.x == 0.0 && d.y == 0.0 {
                    return true;
                }
                let hit = mappings.iter().filter(|m| m.controller == "wheel").find(|m| {
                    let (mods, key) = modifiers_of(&m.event);
                    key == dir && modifiers.shift == mods.shift && modifiers.ctrl == mods.ctrl && modifiers.alt == mods.alt && modifiers.command == mods.command
                });
                match hit {
                    Some(m) => {
                        // One run per notch, several notches per event at most 8.
                        let n = (d.x.abs().max(d.y.abs()).round() as usize).clamp(1, 8);
                        actions.extend(std::iter::repeat_n(m.action.clone(), n));
                        false
                    }
                    None => true,
                }
            });
        });
    }
    let ran = actions.len();
    for a in actions {
        if let Some((id, params)) = redrob_photo_engine::input_cmds::split_action(&a)
            && let Err(e) = crate::menus::invoke(app, ctx, &id, params)
        {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    ran
}
