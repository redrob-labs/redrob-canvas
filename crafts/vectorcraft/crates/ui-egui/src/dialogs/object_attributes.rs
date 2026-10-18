//! Object › Object Attributes…: the selected element's raw SVG attributes (`object.xmlAttributes`)
//! for numeric editing; OK sets the changed ones with `object.setXmlAttribute` as one undo step.
//!
//! Fields: one per attribute (its value as text), `__id`, `__element`, `__editable` (the names
//! that take a change), `__orig` (the values as opened).

use serde_json::{Map, Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form, run_and_close};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of Object Attributes.
pub const KIND: &str = "objectAttributes";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Object Attributes").into(), body, confirm, min_width: 380.0, ..DialogSpec::FORM };

/// `ui.objectAttributes {id?}`: open the dialog on the selected element.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut q = json!({});
    if let Some(id) = p.get("id") {
        q["id"] = id.clone();
    }
    let r = app.run("object.xmlAttributes", q)?;
    let attrs = r.get("attributes").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut fields: Map<String, Value> = attrs.clone();
    fields.insert("__id".into(), r.get("id").cloned().unwrap_or(Value::Null));
    fields.insert("__element".into(), r.get("element").cloned().unwrap_or(json!("")));
    fields.insert("__editable".into(), r.get("editable").cloned().unwrap_or(json!([])));
    fields.insert("__orig".into(), Value::Object(attrs));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(Value::Null)
}

/// The attributes in a stable order: geometry first, then paint, then the rest.
const ORDER: [&str; 24] = [
    "id",
    "x",
    "y",
    "width",
    "height",
    "rx",
    "ry",
    "cx",
    "cy",
    "r",
    "x1",
    "y1",
    "x2",
    "y2",
    "points",
    "d",
    "href",
    "preserveAspectRatio",
    "transform",
    "fill",
    "stroke",
    "stroke-width",
    "opacity",
    "title",
];

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    let element = d.str("__element");
    let editable: Vec<String> = d
        .fields
        .get("__editable")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    ui.label(egui::RichText::new(format!("<{element}>")).monospace().color(t.text_dim));
    ui.add_space(6.0);
    let mut keys: Vec<String> = d.fields.keys().filter(|k| !k.starts_with("__")).cloned().collect();
    keys.sort_by_key(|k| ORDER.iter().position(|o| o == k).unwrap_or(ORDER.len()));
    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
        grid(ui, |ui| {
            for k in keys {
                // Attribute names are SVG's own: shown as they are written.
                label(ui, &k);
                if editable.contains(&k) {
                    let long = matches!(k.as_str(), "d" | "points");
                    if long {
                        form::text_area(ui, d, &k, 240.0, 3);
                    } else {
                        form::text(ui, d, &k, 240.0);
                    }
                } else {
                    let v = d.str(&k);
                    let shown = if v.chars().count() > 80 { format!("{}…", v.chars().take(80).collect::<String>()) } else { v };
                    ui.label(egui::RichText::new(shown).monospace().color(t.text));
                }
                ui.end_row();
            }
        });
    });
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let orig = d.fields.get("__orig").and_then(Value::as_object).cloned().unwrap_or_default();
    let editable: Vec<String> = d
        .fields
        .get("__editable")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let mut values = Map::new();
    for k in &editable {
        let now = d.str(k);
        let before = orig.get(k).and_then(Value::as_str).unwrap_or("");
        if now.trim() != before.trim() {
            values.insert(k.clone(), json!(now));
        }
    }
    if values.is_empty() {
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    run_and_close(app, "object.setXmlAttribute", json!({ "id": d.fields.get("__id"), "values": values }))
}
