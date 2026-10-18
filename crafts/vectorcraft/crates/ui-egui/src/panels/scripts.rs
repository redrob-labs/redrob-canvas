//! Scripts panel (Document Properties › Scripting): the document's SVG `<script>` elements, each
//! an external reference or embedded code, listed with add, remove and editing of the reference,
//! the MIME type and the code. The scripts travel with the SVG for interactive drawings; nothing
//! here runs them. Edits go through `doc.addScript`, `doc.setScript` and `doc.removeScript`.

use egui::Ui;
use serde_json::{Value, json};

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;

pub const ID: &str = "scripts";
/// Panel state: the selected script's index.
const SELECTED: &str = "scripts-selected";
const LABEL_W: f32 = 56.0;

fn run(app: &mut VectorcraftApp, cmd: &str, p: Value) {
    if let Err(e) = app.run(cmd, p) {
        app.ui.status = e;
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(scripts) = app.session.active().map(|st| st.doc.scripts.clone()) else {
        super::empty_state(ui, "dc-list-view", "No Document", "Open a document to edit its scripts.");
        return;
    };
    let selected: usize = pstate::<usize>(ui.ctx(), SELECTED).min(scripts.len().saturating_sub(1));
    if scripts.is_empty() {
        widgets::dim_label(ui, tl!("The document has no scripts."));
    }
    for (i, sc) in scripts.iter().enumerate() {
        let label = match &sc.href {
            Some(h) => format!("{} {h}", tl!("External:")),
            None => format!("{} ({})", tl!("Embedded script"), sc.content.lines().count()),
        };
        if ui.add(egui::Button::selectable(selected == i, egui::RichText::new(label).color(t.text).size(12.0)).truncate()).clicked() {
            set_pstate(ui.ctx(), SELECTED, i);
        }
    }
    ui.horizontal(|ui| {
        if widgets::icon_button(ui, "plus", tl!("Add Embedded Script"), false, 18.0).clicked() {
            run(app, "doc.addScript", json!({}));
            set_pstate(ui.ctx(), SELECTED, scripts.len());
        }
        if widgets::icon_button(ui, "dc-folder", tl!("Add External Script"), false, 18.0).clicked() {
            run(app, "doc.addScript", json!({ "href": "script.js" }));
            set_pstate(ui.ctx(), SELECTED, scripts.len());
        }
        if widgets::icon_button_enabled(ui, "x", tl!("Remove Script"), false, !scripts.is_empty(), 18.0).clicked() {
            run(app, "doc.removeScript", json!({ "index": selected }));
            set_pstate(ui.ctx(), SELECTED, selected.saturating_sub(1));
        }
    });
    let Some(sc) = scripts.get(selected) else { return };
    widgets::divider(ui);
    let mut changes = serde_json::Map::new();
    widgets::label_row(ui, tl!("Type:"), LABEL_W, |ui| {
        if let Some(s) = widgets::exact_text_field(ui, ("script-type", selected), &sc.mime, 160.0) {
            changes.insert("type".into(), json!(s));
        }
    });
    widgets::label_row(ui, tl!("Source:"), LABEL_W, |ui| {
        let external = sc.href.is_some();
        if widgets::check(ui, "External file", external, true) {
            changes.insert("href".into(), if external { Value::Null } else { json!("script.js") });
        }
    });
    match &sc.href {
        Some(href) => {
            widgets::label_row(ui, tl!("URL:"), LABEL_W, |ui| {
                if let Some(s) = widgets::exact_text_field(ui, ("script-href", selected), href, 200.0) {
                    changes.insert("href".into(), json!(s));
                }
            });
        }
        None => {
            widgets::dim_label(ui, tl!("Code:"));
            if let Some(code) = widgets::text_field(ui, ("script-code", selected), Some(&sc.content), ui.available_width().max(120.0), 8) {
                changes.insert("content".into(), json!(code));
            }
        }
    }
    if !changes.is_empty() {
        changes.insert("index".into(), json!(selected));
        run(app, "doc.setScript", Value::Object(changes));
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    if widgets::menu_item(ui, tl!("Add Embedded Script"), true, false) {
        run(app, "doc.addScript", json!({}));
    }
    if widgets::menu_item(ui, tl!("Add External Script"), true, false) {
        run(app, "doc.addScript", json!({ "href": "script.js" }));
    }
}
