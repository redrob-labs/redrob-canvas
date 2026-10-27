//! Edit › Modules… and Edit › Units… dialogs. Modules lists the loadable modules (installed
//! plug-ins) with an enable checkbox each; Units lists the custom measurement units with their
//! identifier, factor (units per inch), digits, symbol and names, and lets one be added, changed
//! or removed. Both apply through the engine's `module.*` and `units.*` commands on OK.

use serde_json::{Map, Value, json};

use crate::PhotoApp;
use crate::state::DialogKind;

const MODULES: &str = "__modules";
const UNITS: &str = "__units";

pub fn handles(id: &str) -> bool {
    matches!(id, "edit.modules" | "edit.units")
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MODULES) || fields.contains_key(UNITS)
}

pub fn dialog_width(fields: &Map<String, Value>) -> Option<f32> {
    owns(fields).then_some(560.0)
}

/// Edit › Modules… / Edit › Units… open their dialog.
pub fn menu(app: &mut PhotoApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    match id {
        "edit.modules" => Some(open_modules(app)),
        "edit.units" => Some(open_units(app)),
        _ => None,
    }
}

fn open_modules(app: &mut PhotoApp) -> Result<Value, String> {
    let list = app.run("module.list", json!({}))?;
    let mut f = Map::new();
    f.insert(MODULES.into(), json!(true));
    f.insert("__command".into(), json!("edit.modules"));
    f.insert("__label".into(), json!("Modules…"));
    f.insert("modules".into(), list.get("modules").cloned().unwrap_or_else(|| json!([])));
    Ok(json!({ "dialog": app.ui.open_dialog(DialogKind::Command, f) }))
}

fn open_units(app: &mut PhotoApp) -> Result<Value, String> {
    let list = app.run("units.list", json!({}))?;
    let mut f = Map::new();
    f.insert(UNITS.into(), json!(true));
    f.insert("__command".into(), json!("edit.units"));
    f.insert("__label".into(), json!("Units…"));
    f.insert("custom".into(), list.get("custom").cloned().unwrap_or_else(|| json!([])));
    f.insert("original".into(), list.get("custom").cloned().unwrap_or_else(|| json!([])));
    f.insert("new".into(), json!({"identifier": "", "factor": 1.0, "digits": 2, "symbol": "", "abbreviation": "", "singular": "", "plural": ""}));
    Ok(json!({ "dialog": app.ui.open_dialog(DialogKind::Command, f) }))
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if f.contains_key(MODULES) {
        modules_body(ui, f);
    } else {
        units_body(ui, f);
    }
}

fn modules_body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut modules: Vec<Value> = f.get("modules").and_then(Value::as_array).cloned().unwrap_or_default();
    if let Some(error) = f.get("__error").and_then(Value::as_str) {
        ui.colored_label(ui.visuals().error_fg_color, error);
        ui.add_space(4.0);
    }
    if modules.is_empty() {
        ui.weak(tl!("No loadable modules are installed. Filter > Plug-ins > Install Plug-in… adds one."));
        return;
    }
    ui.weak(tl!("A module that is switched off stays installed but leaves the menus until it is switched on again."));
    ui.add_space(6.0);
    egui::ScrollArea::vertical().id_salt("modules-list").max_height(320.0).show(ui, |ui| {
        egui::Grid::new("modules-grid").num_columns(3).spacing([12.0, 6.0]).striped(true).show(ui, |ui| {
            ui.strong(tl!("Enabled"));
            ui.strong(tl!("Module"));
            ui.strong(tl!("Version"));
            ui.end_row();
            for m in &mut modules {
                let mut enabled = m.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                let loaded = m.get("loaded").and_then(Value::as_bool).unwrap_or(true);
                if ui.checkbox(&mut enabled, "").changed() {
                    m["enabled"] = json!(enabled);
                }
                let name = m.get("name").and_then(Value::as_str).unwrap_or("?");
                let id = m.get("id").and_then(Value::as_str).unwrap_or("?");
                ui.vertical(|ui| {
                    ui.label(name);
                    ui.weak(id);
                    if !loaded {
                        ui.weak(tl!("Not installed"));
                    }
                });
                ui.label(m.get("version").and_then(Value::as_str).unwrap_or(""));
                ui.end_row();
            }
        });
    });
    f.insert("modules".into(), Value::Array(modules));
}

fn unit_row(ui: &mut egui::Ui, u: &mut Value, editable_id: bool) -> bool {
    let mut remove = false;
    let text = |u: &mut Value, k: &str, ui: &mut egui::Ui, w: f32, enabled: bool| {
        let mut s = u.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        if ui.add_enabled(enabled, egui::TextEdit::singleline(&mut s).desired_width(w)).changed() {
            u[k] = json!(s);
        }
    };
    text(u, "identifier", ui, 80.0, editable_id);
    let mut factor = u.get("factor").and_then(Value::as_f64).unwrap_or(1.0);
    if ui.add(egui::DragValue::new(&mut factor).speed(0.1).range(1e-9..=1e9).custom_parser(crate::widgets::parse_num)).changed() {
        u["factor"] = json!(factor);
    }
    let mut digits = u.get("digits").and_then(Value::as_u64).unwrap_or(2) as i64;
    if ui.add(egui::DragValue::new(&mut digits).range(0..=8)).changed() {
        u["digits"] = json!(digits);
    }
    text(u, "symbol", ui, 50.0, true);
    text(u, "abbreviation", ui, 60.0, true);
    text(u, "singular", ui, 80.0, true);
    text(u, "plural", ui, 80.0, true);
    if editable_id {
        ui.label("");
    } else if crate::icons::button(ui, "trash", 18.0, false, tl!("Delete Unit")).clicked() {
        remove = true;
    }
    ui.end_row();
    remove
}

fn units_body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut custom: Vec<Value> = f.get("custom").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut new = f.get("new").cloned().unwrap_or_else(|| json!({}));
    if let Some(error) = f.get("__error").and_then(Value::as_str) {
        ui.colored_label(ui.visuals().error_fg_color, error);
        ui.add_space(4.0);
    }
    ui.weak(tl!("The factor is how many of the unit make one inch (2.54 for centimetres, 72 for points)."));
    ui.add_space(6.0);
    let mut removed: Option<usize> = None;
    egui::ScrollArea::vertical().id_salt("units-list").max_height(300.0).show(ui, |ui| {
        egui::Grid::new("units-grid").num_columns(8).spacing([8.0, 6.0]).striped(true).show(ui, |ui| {
            for h in ["Identifier", "Factor", "Digits", "Symbol", "Abbreviation", "Singular", "Plural"] {
                ui.strong(tl!(h));
            }
            ui.label("");
            ui.end_row();
            for (i, u) in custom.iter_mut().enumerate() {
                if unit_row(ui, u, false) {
                    removed = Some(i);
                }
            }
        });
    });
    if let Some(i) = removed {
        custom.remove(i);
    }
    ui.add_space(8.0);
    ui.label(tl!("New unit"));
    egui::Grid::new("units-new").num_columns(8).spacing([8.0, 6.0]).show(ui, |ui| {
        unit_row(ui, &mut new, true);
    });
    ui.horizontal(|ui| {
        let id = new.get("identifier").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let taken = custom.iter().any(|u| u.get("identifier").and_then(Value::as_str) == Some(id.as_str()));
        let ok = !id.is_empty() && !taken && new.get("factor").and_then(Value::as_f64).is_some_and(|v| v > 0.0);
        if ui.add_enabled(ok, egui::Button::new(tl!("Add Unit"))).clicked() {
            let mut u = new.clone();
            for k in ["symbol", "abbreviation", "singular", "plural"] {
                if u.get(k).and_then(Value::as_str).is_none_or(|s| s.trim().is_empty()) {
                    u[k] = json!(id);
                }
            }
            custom.push(u);
            new = json!({"identifier": "", "factor": 1.0, "digits": 2, "symbol": "", "abbreviation": "", "singular": "", "plural": ""});
        }
        if taken {
            ui.weak(tl!("That identifier is already a unit"));
        }
    });
    f.insert("custom".into(), Value::Array(custom));
    f.insert("new".into(), new);
}

pub fn confirm(app: &mut PhotoApp, f: &Map<String, Value>) -> Result<Value, String> {
    let result = if f.contains_key(MODULES) { confirm_modules(app, f) } else { confirm_units(app, f) };
    if let Err(e) = &result {
        let mut again = f.clone();
        again.insert("__error".into(), json!(e));
        app.ui.open_dialog(DialogKind::Command, again);
    }
    result
}

fn confirm_modules(app: &mut PhotoApp, f: &Map<String, Value>) -> Result<Value, String> {
    let modules = f.get("modules").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut changed = 0;
    for m in &modules {
        let (Some(id), Some(enabled)) = (m.get("id").and_then(Value::as_str), m.get("enabled").and_then(Value::as_bool)) else { continue };
        let was = redrob_photo_engine::modules_cmds::module_enabled(&app.session, id);
        if was != enabled {
            app.run("module.setEnabled", json!({"id": id, "enabled": enabled}))?;
            changed += 1;
        }
    }
    Ok(json!({ "changed": changed }))
}

fn confirm_units(app: &mut PhotoApp, f: &Map<String, Value>) -> Result<Value, String> {
    let custom = f.get("custom").and_then(Value::as_array).cloned().unwrap_or_default();
    let original = f.get("original").and_then(Value::as_array).cloned().unwrap_or_default();
    let id_of = |u: &Value| u.get("identifier").and_then(Value::as_str).unwrap_or("").trim().to_string();
    // Deleted units first, then every unit as the dialog shows it.
    for o in &original {
        let id = id_of(o);
        if !id.is_empty() && !custom.iter().any(|u| id_of(u) == id) {
            app.run("units.delete", json!({"identifier": id}))?;
        }
    }
    let mut set = 0;
    for u in &custom {
        let id = id_of(u);
        if id.is_empty() {
            continue;
        }
        if original.iter().any(|o| o == u) {
            continue;
        }
        let mut p = u.clone();
        p["identifier"] = json!(id);
        app.run("units.set", p)?;
        set += 1;
    }
    Ok(json!({ "set": set }))
}
