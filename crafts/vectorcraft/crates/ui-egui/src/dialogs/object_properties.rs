//! Object › Object Properties…: the SVG id, label, title, description and hyperlink of the
//! selected objects, and whether they are hidden or locked. OK applies the name and flags with
//! `layer.setProps` and the rest with `attributes.set`, as one undo step.
//!
//! Fields: `ids` (the objects), `id` (the XML id, [`redrob_vector_doc::xml_id`] of the name),
//! `label` (the name as shown), `title`, `description`, `url`, `urlTarget`, `urlTitle`,
//! `highlight` (a hex colour, empty: the layer colour), `hidden`, `locked`, for images `dpi` and
//! `preserveAspectRatio`, and the interactivity scripts `onclick`, `onmouseover`, `onmouseout`,
//! `onmousedown`, `onmouseup`, `onfocusin`, `onfocusout`, `onload`. With several objects the id
//! and label are left alone (only the first object's are shown).

use redrob_vector_doc::{Node, NodeId, xml_id};
use serde_json::{Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Object Properties.
pub const KIND: &str = "objectProperties";

/// The SVG event attributes the Interactivity section edits.
pub const EVENTS: [&str; 8] = ["onclick", "onmouseover", "onmouseout", "onmousedown", "onmouseup", "onfocusin", "onfocusout", "onload"];

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Object Properties").into(), body, confirm, min_width: 360.0, ..DialogSpec::FORM };

/// The fields for `n`.
fn fields_of(n: &Node) -> serde_json::Map<String, Value> {
    let attrs = n.extras().attrs.as_deref().cloned().unwrap_or_default();
    let name = n.name.clone().unwrap_or_default();
    let mut f = serde_json::Map::new();
    f.insert("id".into(), json!(if name.is_empty() { String::new() } else { xml_id::from_object_name(&name) }));
    f.insert("label".into(), json!(name));
    f.insert("title".into(), json!(attrs.title));
    f.insert("description".into(), json!(attrs.description));
    f.insert("url".into(), json!(attrs.url));
    f.insert("urlTarget".into(), json!(attrs.url_target));
    f.insert("urlTitle".into(), json!(attrs.url_title));
    f.insert("highlight".into(), json!(attrs.highlight.clone().unwrap_or_default()));
    f.insert("hidden".into(), json!(!n.visible));
    f.insert("locked".into(), json!(n.locked));
    let image = matches!(n.kind, redrob_vector_doc::NodeKind::Image(_));
    f.insert("image".into(), json!(image));
    if image {
        f.insert("dpi".into(), json!(attrs.dpi.map(|d| format!("{d}")).unwrap_or_default()));
        f.insert("preserveAspectRatio".into(), json!(attrs.preserve_aspect_ratio.clone().unwrap_or_default()));
    }
    for ev in EVENTS {
        let script = attrs.events.iter().find(|(k, _)| k == ev).map(|(_, v)| v.clone()).unwrap_or_default();
        f.insert(ev.into(), json!(script));
    }
    f
}

/// `ui.objectProperties {ids?|id?}`: open Object Properties for objects (default: the selection),
/// filled in from the first.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let object = |v: &Value| -> Result<NodeId, String> {
        let id = NodeId(v.as_u64().ok_or_else(|| format!("object ids are non-negative integers, not {v}"))?);
        st.doc.node(id).map(|_| id).ok_or_else(|| "no such object".to_string())
    };
    let ids: Vec<NodeId> = match (p.get("ids"), p.get("id")) {
        (Some(Value::Array(a)), _) => a.iter().map(object).collect::<Result<_, _>>()?,
        (Some(v), _) => return Err(format!("`ids` must be an array of object ids, not {v}")),
        (None, Some(v)) => vec![object(v)?],
        (None, None) => st.selection.objects.clone(),
    };
    let first = ids.first().and_then(|id| st.doc.node(*id)).ok_or("select an object")?;
    let mut f = fields_of(first);
    f.insert("ids".into(), json!(ids.iter().map(|i| i.0).collect::<Vec<_>>()));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields: f });
    Ok(Value::Null)
}

fn toggle(ui: &mut egui::Ui, d: &mut Dialog, key: &str, text: &str) {
    let v = d.bool(key);
    if widgets::check(ui, text, v, true) {
        d.fields.insert(key.into(), json!(!v));
    }
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let many = d.fields.get("ids").and_then(Value::as_array).is_some_and(|a| a.len() > 1);
    grid(ui, |ui| {
        if !many {
            label(ui, tl!("ID:"));
            // The id and the label are one name: editing either writes the other.
            if form::text(ui, d, "id", 220.0) {
                let id = d.str("id");
                d.fields.insert("label".into(), json!(xml_id::to_object_name(&id)));
            }
            ui.end_row();
            label(ui, tl!("Label:"));
            if form::text(ui, d, "label", 220.0) {
                let name = d.str("label");
                d.fields.insert("id".into(), json!(if name.is_empty() { String::new() } else { xml_id::from_object_name(&name) }));
            }
            ui.end_row();
        }
        label(ui, tl!("Title:"));
        form::text(ui, d, "title", 220.0);
        ui.end_row();
        label(ui, tl!("Description:"));
        form::text_area(ui, d, "description", 220.0, 3);
        ui.end_row();
        label(ui, tl!("Hyperlink:"));
        form::text(ui, d, "url", 220.0);
        ui.end_row();
        label(ui, tl!("Link target:"));
        form::text(ui, d, "urlTarget", 220.0);
        ui.end_row();
        label(ui, tl!("Link title:"));
        form::text(ui, d, "urlTitle", 220.0);
        ui.end_row();
        label(ui, tl!("Highlight color:"));
        form::text(ui, d, "highlight", 220.0);
        ui.end_row();
        if d.bool("image") {
            label(ui, tl!("DPI:"));
            form::text(ui, d, "dpi", 220.0);
            ui.end_row();
            label(ui, tl!("Aspect ratio:"));
            form::text(ui, d, "preserveAspectRatio", 220.0);
            ui.end_row();
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        toggle(ui, d, "hidden", tl!("Hide"));
        ui.add_space(16.0);
        toggle(ui, d, "locked", tl!("Lock"));
    });
    ui.add_space(8.0);
    ui.collapsing(tl!("Interactivity"), |ui| {
        grid(ui, |ui| {
            for ev in EVENTS {
                // Attribute names are SVG's own: shown as they are written.
                label(ui, ev);
                form::text(ui, d, ev, 220.0);
                ui.end_row();
            }
        });
    });
    false
}

/// The commands OK runs: (command, params) for the name and flags, then the attributes. `xml_id`:
/// Preferences › Units › Identify Objects By › XML ID, which `layer.setProps` then takes the
/// name as.
pub(crate) fn params(d: &Dialog, xml_id: bool) -> [(&'static str, Value); 2] {
    let ids = d.fields.get("ids").cloned().unwrap_or(json!([]));
    let many = ids.as_array().is_some_and(|a| a.len() > 1);
    let mut props = json!({ "ids": ids.clone(), "visible": !d.bool("hidden"), "locked": d.bool("locked") });
    if !many && let Some(o) = props.as_object_mut() {
        let name = if xml_id { d.str("id") } else { d.str("label") };
        o.insert("name".into(), json!(name));
    }
    let events: serde_json::Map<String, Value> = EVENTS.iter().map(|ev| (ev.to_string(), json!(d.str(ev)))).collect();
    let highlight = d.str("highlight");
    let mut attrs = json!({
        "ids": ids,
        "title": d.str("title"),
        "description": d.str("description"),
        "url": d.str("url"),
        "urlTarget": d.str("urlTarget"),
        "urlTitle": d.str("urlTitle"),
        "events": events,
        "highlight": if highlight.trim().is_empty() { Value::Null } else { json!(highlight.trim()) },
    });
    if d.bool("image") && let Some(o) = attrs.as_object_mut() {
        let dpi = d.str("dpi");
        o.insert("dpi".into(), dpi.trim().parse::<f64>().ok().filter(|v| *v > 0.0).map_or(Value::Null, |v| json!(v)));
        let par = d.str("preserveAspectRatio");
        o.insert("preserveAspectRatio".into(), if par.trim().is_empty() { Value::Null } else { json!(par.trim()) });
    }
    [("layer.setProps", props), ("attributes.set", attrs)]
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let xml_id = app.session.prefs.identify_objects_by == "xmlId";
    let [(c1, p1), (c2, p2)] = params(d, xml_id);
    app.session.begin_named_undo_group(Some("Object Properties".into()));
    let r = app.run(c1, p1).and_then(|_| app.run(c2, p2));
    app.session.end_undo_group(r.is_err());
    app.ui.dialog = None;
    r
}
