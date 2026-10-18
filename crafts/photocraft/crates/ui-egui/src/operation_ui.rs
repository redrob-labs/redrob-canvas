//! Filter › Operation… : a generic front end for every filter and adjustment operation, with or
//! without a menu entry of its own. The dialog lists the operations (searchable, grouped by
//! their menu path), shows the chosen one's parameters, and OK opens that operation's own dialog
//! with its live preview, or runs it straight away when it takes no parameters.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__operation";

pub fn handles(id: &str) -> bool {
    id == "filter.operation"
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

pub fn dialog_width(fields: &Map<String, Value>) -> Option<f32> {
    owns(fields).then_some(560.0)
}

/// Every operation the dialog offers: (id, label, menu path, parameter summary).
pub fn operations() -> Vec<(String, String, String, String)> {
    let mut out: Vec<(String, String, String, String)> = redrob_photo_engine::command_specs()
        .iter()
        .filter(|c| c.id.starts_with("filter.") || c.id.starts_with("image.adjustments.") || c.id.starts_with("select.modify."))
        .filter(|c| !matches!(c.id, "filter.lastFilter" | "filter.operation"))
        .map(|c| (c.id.to_string(), c.label.trim_end_matches('…').to_string(), c.menu.join(" › "), c.params.to_string()))
        .collect();
    out.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.1.cmp(&b.1)));
    out
}

pub fn menu(app: &mut PhotocraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    if id != "filter.operation" {
        return None;
    }
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!("filter.operation"));
    f.insert("__label".into(), json!("Operation…"));
    f.insert("search".into(), json!(""));
    f.insert("selected".into(), json!(app.ui.last_operation.clone().unwrap_or_default()));
    Some(Ok(json!({ "dialog": app.ui.open_dialog(DialogKind::Command, f) })))
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut search = f.get("search").and_then(Value::as_str).unwrap_or("").to_string();
    let mut selected = f.get("selected").and_then(Value::as_str).unwrap_or("").to_string();
    if let Some(error) = f.get("__error").and_then(Value::as_str) {
        ui.colored_label(ui.visuals().error_fg_color, error);
        ui.add_space(4.0);
    }
    ui.horizontal(|ui| {
        ui.label(tl!("Search"));
        ui.add(egui::TextEdit::singleline(&mut search).desired_width(300.0).hint_text(tl!("Operation name or id")));
    });
    ui.add_space(4.0);
    let needle = search.trim().to_lowercase();
    let ops = operations();
    let shown: Vec<&(String, String, String, String)> = ops
        .iter()
        .filter(|o| needle.is_empty() || o.0.to_lowercase().contains(&needle) || o.1.to_lowercase().contains(&needle) || o.2.to_lowercase().contains(&needle))
        .collect();
    ui.weak(crate::i18n::fmt(tl!("{n} operations"), &[("n", &shown.len().to_string())]));
    egui::ScrollArea::vertical().id_salt("operation-list").max_height(260.0).show(ui, |ui| {
        let mut group = String::new();
        for (id, label, path, _) in &shown {
            if *path != group {
                group = path.clone();
                ui.add_space(4.0);
                ui.strong(if group.is_empty() {
                    tl!("Other").to_string()
                } else {
                    group.split(" › ").map(|p| tl!(p).to_string()).collect::<Vec<_>>().join(" › ")
                });
            }
            let is_sel = *id == selected;
            let r = ui.selectable_label(is_sel, format!("{}  ", tl!(label)));
            if r.clicked() {
                selected = id.clone();
            }
            if r.double_clicked() {
                selected = id.clone();
                f.insert("__confirm".into(), json!(true));
            }
            r.on_hover_text(id);
        }
    });
    if let Some((id, label, _, params)) = ops.iter().find(|o| o.0 == selected) {
        ui.separator();
        ui.label(egui::RichText::new(tl!(label)).strong());
        ui.weak(id);
        ui.add(egui::Label::new(egui::RichText::new(params).small()).wrap());
    }
    f.insert("search".into(), json!(search));
    f.insert("selected".into(), json!(selected));
}

/// OK: the operation's own dialog (live preview) when it has parameters, else it runs at once.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let id = f.get("selected").and_then(Value::as_str).unwrap_or("").to_string();
    if id.is_empty() || redrob_photo_engine::commands::find(&id).is_none() {
        let mut again = f.clone();
        again.insert("__error".into(), json!(tl!("Choose an operation")));
        app.ui.open_dialog(DialogKind::Command, again);
        return Err(tl!("Choose an operation").to_string());
    }
    app.ui.last_operation = Some(id.clone());
    if crate::filter_dialog::has_dialog(&id) {
        return Ok(json!({ "dialog": crate::filter_dialog::open(app, &id) }));
    }
    app.run(&id, json!({}))
}

/// A double-clicked row confirms the dialog on the next frame.
pub fn tick(app: &mut PhotocraftApp) {
    let Some(d) = app.ui.dialogs.iter().find(|d| owns(&d.fields) && d.fields.contains_key("__confirm")) else { return };
    let id = d.id;
    if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) {
        d.fields.remove("__confirm");
    }
    let _ = crate::dialogs::confirm(app, id);
}
