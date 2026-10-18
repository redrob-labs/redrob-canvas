//! XML Editor panel: the document's objects as an element tree (`<Path id="12" name="…">` per
//! node, layers at the root, groups nested) with an attribute list for the selected node. The
//! tree follows the canvas selection and clicking a row selects its object; the attribute list
//! edits the node's own values (name, visible, locked, opacity, URL, note, data-* attributes:
//! `object.setProps`, `attributes.set`) and shows everything else the node is made of as
//! read-only JSON (the SVG the writer makes of it starts from these). Rows can be created,
//! duplicated, deleted, raised, lowered, indented, outdented and dragged to a new place; a path's
//! `d` is edited as SVG path data and rounded; data attributes added and removed. Tags, attribute
//! names and values are coloured apart (syntax highlighting).

use std::collections::BTreeSet;

use egui::{Sense, Ui, vec2};
use redrob_vector_doc::{Node, NodeId};
use serde_json::{Value, json};

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

pub const ID: &str = "xmlEditor";
/// Panel state: the collapsed nodes.
const COLLAPSED: &str = "xml-collapsed";
/// Panel state: the raw JSON section is open.
const RAW_OPEN: &str = "xml-raw-open";
/// Panel state: the name of a data attribute being added.
const NEW_KEY: &str = "xml-new-key";
/// Panel state: the row being dragged to a new place.
const DRAGGING: &str = "xml-dragging";
/// Panel state: the decimals Round Path Data rounds to.
const ROUND: &str = "xml-round";
const LABEL_W: f32 = 70.0;

/// One row of the tree: the node, its depth and whether it has children.
struct Row {
    id: NodeId,
    depth: usize,
    label: String,
    parent: bool,
    selected: bool,
}

fn rows(app: &VectorcraftApp, collapsed: &BTreeSet<u64>) -> Vec<Row> {
    let Some(st) = app.session.active() else { return vec![] };
    let mut out = vec![];
    fn walk(n: &Node, depth: usize, collapsed: &BTreeSet<u64>, sel: &[NodeId], out: &mut Vec<Row>) {
        let children = n.children().filter(|c| !c.is_empty());
        let name = n.name.as_deref().map(|s| format!(" name=\"{s}\"")).unwrap_or_default();
        out.push(Row {
            id: n.id,
            depth,
            label: format!("<{} id=\"{}\"{name}>", n.kind_label().replace(' ', ""), n.id.0),
            parent: children.is_some(),
            selected: sel.contains(&n.id),
        });
        if collapsed.contains(&n.id.0) {
            return;
        }
        for c in children.into_iter().flatten() {
            walk(c, depth + 1, collapsed, sel, out);
        }
    }
    for l in &st.doc.layers {
        walk(l, 0, collapsed, &st.selection.objects, &mut out);
    }
    out
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    if app.session.active().is_none() {
        super::empty_state(ui, "dc-list-view", "No Document", "Open a document to see its element tree.");
        return;
    }
    let mut collapsed: BTreeSet<u64> = pstate(ui.ctx(), COLLAPSED);
    let rows = rows(app, &collapsed);
    let mut clicked: Option<(NodeId, bool)> = None;
    let mut toggled: Option<u64> = None;
    // Drag reorder: the row pressed, and where it is dropped (before that row, or into it when it
    // is dropped on the right half of a group's row).
    let dragging: Option<u64> = pstate(ui.ctx(), DRAGGING);
    let mut drop: Option<(NodeId, bool)> = None;
    let released = ui.input(|i| i.pointer.any_released());
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("xml-tree").max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
            for row in &rows {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click_and_drag());
                if row.selected {
                    ui.painter().rect_filled(rect, 2.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 2.0, t.hover);
                }
                if resp.drag_started() {
                    set_pstate(ui.ctx(), DRAGGING, Some(row.id.0));
                }
                if let Some(from) = dragging
                    && from != row.id.0
                    && let Some(p) = ui.input(|i| i.pointer.hover_pos())
                    && rect.contains(p)
                {
                    let into = row.parent && p.x > rect.center().x;
                    if into {
                        ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
                    } else {
                        ui.painter().line_segment([rect.left_top(), rect.right_top()], egui::Stroke::new(2.0, t.accent));
                    }
                    if released {
                        drop = Some((row.id, into));
                    }
                }
                let x = rect.left() + 4.0 + row.depth as f32 * 12.0;
                if row.parent {
                    let tri = egui::Rect::from_center_size(egui::pos2(x + 5.0, rect.center().y), vec2(10.0, 10.0));
                    let open = !collapsed.contains(&row.id.0);
                    crate::icons::paint(ui, if open { "chevron-down" } else { "chevron-right" }, tri, t.text_dim);
                    if ui.interact(tri.expand(2.0), ui.id().with(("xml-tri", row.id.0)), Sense::click()).clicked() {
                        toggled = Some(row.id.0);
                    }
                }
                let galley = highlighted(ui, &row.label, &t);
                let top = rect.center().y - galley.size().y / 2.0;
                ui.painter().galley(egui::pos2(x + 14.0, top), galley, t.text);
                if resp.clicked() && toggled.is_none() {
                    clicked = Some((row.id, ui.input(|i| i.modifiers.shift)));
                }
            }
            if rows.is_empty() {
                widgets::dim_label(ui, tl!("The document has no layers."));
            }
        });
    });
    if let Some(id) = toggled {
        if !collapsed.remove(&id) {
            collapsed.insert(id);
        }
        set_pstate(ui.ctx(), COLLAPSED, collapsed);
    }
    if let Some((id, add)) = clicked {
        let cmd = if add { "select.toggle" } else { "select.set" };
        if let Err(e) = app.run(cmd, json!({ "ids": [id.0] })) {
            app.status(e);
        }
    }
    // A dragged row dropped: before the row under the pointer, or into a group.
    if let (Some(from), Some((to, into))) = (dragging, drop) {
        let p = if into { json!({ "id": from, "into": to.0 }) } else { json!({ "id": from, "before": to.0 }) };
        if let Err(e) = app.run("xml.move", p) {
            app.status(e);
        }
    }
    if released {
        set_pstate(ui.ctx(), DRAGGING, Option::<u64>::None);
    }
    // Node buttons: new, duplicate, delete, raise, lower, indent, outdent.
    let has = app.session.active().is_some_and(|d| !d.selection.is_empty());
    ui.horizontal(|ui| {
        if widgets::icon_button(ui, "plus", tl!("New Node"), false, 22.0).clicked()
            && let Err(e) = app.run("xml.newNode", json!({}))
        {
            app.status(e);
        }
        let buttons = [
            ("copy", tl!("Duplicate Node"), "edit.duplicate"),
            ("trash-2", tl!("Delete Node"), "edit.clear"),
            ("dc-arrow-up", tl!("Raise Node"), "object.arrange.bringForward"),
            ("dc-arrow-down", tl!("Lower Node"), "object.arrange.sendBackward"),
            ("chevron-right", tl!("Indent Node"), "xml.indent"),
            ("chevron-left", tl!("Outdent Node"), "xml.outdent"),
        ];
        for (icon, tip, cmd) in buttons {
            if widgets::icon_button_enabled(ui, icon, tip, false, has, 22.0).clicked()
                && let Err(e) = app.run(cmd, json!({}))
            {
                app.status(e);
            }
        }
    });
    let Some(n) = super::first_selected(app) else {
        widgets::dim_label(ui, tl!("Select a node to edit its attributes."));
        return;
    };
    widgets::divider(ui);
    attributes(app, ui, &n);
}

/// The attribute list of node `n`: its own values as editable rows, then its data attributes,
/// then the rest as JSON.
fn attributes(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node) {
    let t = Tokens::get(ui.ctx());
    widgets::subheader(ui, &format!("<{}>", n.kind_label()));
    let id = n.id.0;
    let mut props = serde_json::Map::new();
    widgets::label_row(ui, "id", LABEL_W, |ui| {
        ui.label(egui::RichText::new(id.to_string()).color(t.text_dim).monospace());
    });
    widgets::label_row(ui, "name", LABEL_W, |ui| {
        if let Some(s) = widgets::exact_text_field(ui, ("xml-name", id), n.name.as_deref().unwrap_or(""), 150.0) {
            props.insert("name".into(), json!(s));
        }
    });
    widgets::label_row(ui, "visible", LABEL_W, |ui| {
        if widgets::check(ui, "", n.visible, true) {
            props.insert("visible".into(), json!(!n.visible));
        }
    });
    widgets::label_row(ui, "locked", LABEL_W, |ui| {
        if widgets::check(ui, "", n.locked, true) {
            props.insert("locked".into(), json!(!n.locked));
        }
    });
    widgets::label_row(ui, "opacity", LABEL_W, |ui| {
        if let Some(v) = widgets::plain_field(ui, ("xml-opacity", id), f64::from(n.opacity) * 100.0, "%", 0, 70.0) {
            props.insert("opacity".into(), json!(v.clamp(0.0, 100.0)));
        }
    });
    widgets::label_row(ui, "blend", LABEL_W, |ui| {
        if let Some(m) = widgets::blend_dropdown(ui, ("xml-blend", id), Some(n.blend), 120.0) {
            props.insert("blend".into(), json!(m.label()));
        }
    });
    let attrs = n.extras().attrs.as_deref();
    let mut attr_set = serde_json::Map::new();
    widgets::label_row(ui, "href", LABEL_W, |ui| {
        if let Some(s) = widgets::exact_text_field(ui, ("xml-url", id), attrs.map_or("", |a| a.url.as_str()), 150.0) {
            attr_set.insert("url".into(), json!(s));
        }
    });
    widgets::label_row(ui, "desc", LABEL_W, |ui| {
        if let Some(s) = widgets::exact_text_field(ui, ("xml-note", id), attrs.map_or("", |a| a.note.as_str()), 150.0) {
            attr_set.insert("note".into(), json!(s));
        }
    });
    // A path's geometry as SVG path data, editable, with rounding.
    if let redrob_vector_doc::NodeKind::Path { path, .. } = &n.kind {
        let d = path.to_bezpath().to_svg();
        let shown: String = if d.len() > 20_000 { d.chars().take(20_000).collect() } else { d.clone() };
        widgets::dim_label(ui, "d");
        if let Some(edited) = widgets::text_field(ui, ("xml-d", id), Some(&shown), ui.available_width().max(120.0), 4)
            && edited != d
            && let Err(e) = app.run("path.setSvgData", json!({ "id": id, "d": edited }))
        {
            app.status(e);
        }
        ui.horizontal(|ui| {
            let decimals: f64 = pstate::<Option<f64>>(ui.ctx(), ROUND).unwrap_or(2.0);
            widgets::dim_label(ui, tl!("Round to:"));
            if let Some(v) = widgets::plain_field(ui, ("xml-round", id), decimals, "", 0, 44.0) {
                set_pstate(ui.ctx(), ROUND, Some(v.clamp(0.0, 6.0)));
            }
            if widgets::secondary_button(ui, tl!("Round Path Data")).clicked()
                && let Err(e) = app.run("path.setSvgData", json!({ "id": id, "round": decimals.clamp(0.0, 6.0) as u64 }))
            {
                app.status(e);
            }
        });
    }
    // data-* attributes: edit, remove, add.
    let mut data = serde_json::Map::new();
    for (k, v) in attrs.map(|a| a.data.clone()).unwrap_or_default() {
        widgets::label_row(ui, &format!("data-{k}"), LABEL_W, |ui| {
            if let Some(s) = widgets::exact_text_field(ui, ("xml-data", id, &k), &v, 124.0) {
                data.insert(k.clone(), json!(s));
            }
            if widgets::icon_button(ui, "x", tl!("Remove Attribute"), false, 18.0).clicked() {
                data.insert(k.clone(), Value::Null);
            }
        });
    }
    ui.horizontal(|ui| {
        let mut key: String = pstate(ui.ctx(), NEW_KEY);
        widgets::dim_label(ui, "data-");
        if ui.add(egui::TextEdit::singleline(&mut key).desired_width(90.0).hint_text(tl!("attribute"))).changed() {
            set_pstate(ui.ctx(), NEW_KEY, key.clone());
        }
        let name: String = key.trim().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        if widgets::icon_button_enabled(ui, "plus", tl!("Add Attribute"), false, !name.is_empty(), 18.0).clicked() {
            data.insert(name, json!(""));
            set_pstate(ui.ctx(), NEW_KEY, String::new());
        }
    });
    if !data.is_empty() {
        props.insert("data".into(), Value::Object(data));
    }
    if !props.is_empty() {
        props.insert("ids".into(), json!([id]));
        if let Err(e) = app.run("object.setProps", Value::Object(props)) {
            app.status(e);
        }
    }
    if !attr_set.is_empty() {
        attr_set.insert("ids".into(), json!([id]));
        if let Err(e) = app.run("attributes.set", Value::Object(attr_set)) {
            app.status(e);
        }
    }
    // Everything else, read-only: the node's kind with its geometry and its appearance.
    let open: bool = pstate(ui.ctx(), RAW_OPEN);
    if widgets::section_toggle(ui, tl!("Node Data"), open) {
        set_pstate(ui.ctx(), RAW_OPEN, !open);
    }
    if open {
        let raw = serde_json::to_value(n).unwrap_or(Value::Null);
        let kind = raw.get("kind").cloned().unwrap_or(Value::Null);
        let appearance = raw.get("appearance").cloned().unwrap_or(Value::Null);
        let text = serde_json::to_string_pretty(&json!({ "kind": kind, "appearance": appearance })).unwrap_or_default();
        let shown: String = if text.len() > 20_000 { text.chars().take(20_000).chain(std::iter::once('…')).collect() } else { text };
        egui::ScrollArea::vertical().id_salt("xml-raw").max_height(200.0).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut shown.as_str()).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).interactive(false));
        });
    }
}

/// A tree row's label with its syntax coloured: the tag in the accent colour, attribute names
/// dimmed, quoted values in the text colour.
fn highlighted(ui: &Ui, label: &str, t: &Tokens) -> std::sync::Arc<egui::Galley> {
    use egui::text::{LayoutJob, TextFormat};
    let font = egui::FontId::monospace(11.0);
    let mut job = LayoutJob::default();
    let mut push = |text: &str, color: egui::Color32| {
        if !text.is_empty() {
            job.append(text, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
        }
    };
    // `<Tag attr="value" attr="value">`
    let body = label.trim_start_matches('<').trim_end_matches('>');
    let (tag, rest) = body.split_once(' ').unwrap_or((body, ""));
    push("<", t.text_dim);
    push(tag, t.accent);
    let mut rest = rest;
    while !rest.trim().is_empty() {
        let Some(eq) = rest.find('=') else {
            push(rest, t.text_dim);
            break;
        };
        push(&rest[..eq], t.text_dim);
        push("=", t.text_dim);
        let after = &rest[eq + 1..];
        // The quoted value, to its closing quote.
        let end = after.strip_prefix('"').and_then(|v| v.find('"')).map_or(after.len(), |i| i + 2);
        push(&after[..end], t.text);
        rest = &after[end..];
    }
    push(">", t.text_dim);
    ui.painter().layout_job(job)
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let collapsed: BTreeSet<u64> = pstate(ui.ctx(), COLLAPSED);
    if menu_item(ui, tl!("Expand All"), !collapsed.is_empty(), false) {
        set_pstate(ui.ctx(), COLLAPSED, BTreeSet::<u64>::new());
    }
    if menu_item(ui, tl!("Collapse All"), true, false) {
        let mut all = BTreeSet::new();
        if let Some(st) = app.session.active() {
            st.doc.walk(|n| {
                if n.children().is_some_and(|c| !c.is_empty()) {
                    all.insert(n.id.0);
                }
            });
        }
        set_pstate(ui.ctx(), COLLAPSED, all);
    }
}
