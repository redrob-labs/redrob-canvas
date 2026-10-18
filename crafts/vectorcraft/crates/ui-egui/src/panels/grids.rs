//! Grids panel: the document's grids (any number), each rectangular, axonometric or modular with
//! its own spacing, subdivisions, angles, gaps, origin, colour and visibility. View › Show Grid
//! draws every visible grid and Snap to Grid snaps to the nearest crossing of any of them. Edits go
//! through `grid.add`, `grid.set` and `grid.remove` (one undo step each).

use egui::Ui;
use redrob_vector_doc::{GridKind, GridPrefs};
use serde_json::{Value, json};

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;

pub const ID: &str = "grids";
/// Panel state: the selected grid's index.
const SELECTED: &str = "grids-selected";
const LABEL_W: f32 = 84.0;

const KINDS: [(&str, GridKind); 3] = [("Rectangular", GridKind::Rectangular), ("Axonometric", GridKind::Axonometric), ("Modular", GridKind::Modular)];

fn run(app: &mut VectorcraftApp, cmd: &str, p: Value) {
    if let Err(e) = app.run(cmd, p) {
        app.ui.status = e;
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(grids) = app.session.active().map(|st| st.doc.grids().cloned().collect::<Vec<GridPrefs>>()) else {
        super::empty_state(ui, "layout-grid", "No Document", "Open a document to edit its grids.");
        return;
    };
    let selected: usize = pstate::<usize>(ui.ctx(), SELECTED).min(grids.len().saturating_sub(1));
    // The list: name (or kind), visibility, one row per grid.
    let mut set: Option<Value> = None;
    for (i, g) in grids.iter().enumerate() {
        ui.horizontal(|ui| {
            if widgets::check(ui, "", g.visible, true) {
                set = Some(json!({ "index": i, "visible": !g.visible }));
            }
            let label = if g.name.is_empty() {
                if i == 0 {
                    tl!("Main Grid").to_string()
                } else {
                    format!("{} {}", tl!(KINDS.iter().find(|(_, k)| *k == g.kind).map_or("Grid", |(l, _)| l)), i)
                }
            } else {
                g.name.clone()
            };
            let color = g.color.as_deref().and_then(redrob_vector_color::Color::from_hex).map_or(t.text, |c| super::c32(&c));
            if ui.add(egui::Button::selectable(selected == i, egui::RichText::new(label).color(color).size(12.0))).clicked() {
                set_pstate(ui.ctx(), SELECTED, i);
            }
        });
    }
    ui.horizontal(|ui| {
        if widgets::icon_button(ui, "plus", tl!("Add Grid"), false, 18.0).clicked() {
            run(app, "grid.add", json!({}));
            set_pstate(ui.ctx(), SELECTED, grids.len());
        }
        if widgets::icon_button_enabled(ui, "x", tl!("Remove Grid"), false, selected > 0, 18.0).clicked() {
            run(app, "grid.remove", json!({ "index": selected }));
            set_pstate(ui.ctx(), SELECTED, selected - 1);
        }
    });
    if let Some(p) = set.take() {
        run(app, "grid.set", p);
    }
    widgets::divider(ui);
    let Some(g) = grids.get(selected) else { return };
    let mut changes = serde_json::Map::new();
    widgets::label_row(ui, tl!("Name:"), LABEL_W, |ui| {
        if let Some(s) = widgets::exact_text_field(ui, ("grid-name", selected), &g.name, 140.0) {
            changes.insert("name".into(), json!(s));
        }
    });
    widgets::label_row(ui, tl!("Kind:"), LABEL_W, |ui| {
        let labels: Vec<&str> = KINDS.iter().map(|(l, _)| *l).collect();
        let cur = KINDS.iter().find(|(_, k)| *k == g.kind).map_or("Rectangular", |(l, _)| l);
        if let Some(i) = widgets::dropdown_names(ui, ("grid-kind", selected), cur, &labels, 120.0) {
            let id = match KINDS[i].1 {
                GridKind::Rectangular => "rectangular",
                GridKind::Axonometric => "axonometric",
                GridKind::Modular => "modular",
            };
            changes.insert("kind".into(), json!(id));
        }
    });
    let unit = app.session.general_unit();
    widgets::label_row(ui, tl!("Gridline every:"), LABEL_W, |ui| {
        if let Some(v) = widgets::plain_field(ui, ("grid-spacing", selected), unit.from_pt(g.spacing), unit.suffix(), 2, 72.0) {
            changes.insert("spacing".into(), json!(unit.to_pt(v)));
        }
    });
    widgets::label_row(ui, tl!("Subdivisions:"), LABEL_W, |ui| {
        if let Some(v) = widgets::plain_field(ui, ("grid-subdiv", selected), f64::from(g.subdivisions), "", 0, 56.0) {
            changes.insert("subdivisions".into(), json!(v.round().clamp(1.0, 1000.0) as u64));
        }
    });
    if g.kind == GridKind::Axonometric {
        widgets::label_row(ui, tl!("Angles:"), LABEL_W, |ui| {
            if let Some(v) = widgets::plain_field(ui, ("grid-ax", selected), g.angle_x, "°", 1, 56.0) {
                changes.insert("angleX".into(), json!(v));
            }
            if let Some(v) = widgets::plain_field(ui, ("grid-az", selected), g.angle_z, "°", 1, 56.0) {
                changes.insert("angleZ".into(), json!(v));
            }
        });
    }
    if g.kind == GridKind::Modular {
        widgets::label_row(ui, tl!("Gaps:"), LABEL_W, |ui| {
            if let Some(v) = widgets::plain_field(ui, ("grid-gx", selected), unit.from_pt(g.gap_x), unit.suffix(), 2, 64.0) {
                changes.insert("gapX".into(), json!(unit.to_pt(v)));
            }
            if let Some(v) = widgets::plain_field(ui, ("grid-gy", selected), unit.from_pt(g.gap_y), unit.suffix(), 2, 64.0) {
                changes.insert("gapY".into(), json!(unit.to_pt(v)));
            }
        });
    }
    widgets::label_row(ui, tl!("Origin:"), LABEL_W, |ui| {
        let mut o = [g.origin.x, g.origin.y];
        let mut moved = false;
        for (k, key) in ["grid-ox", "grid-oy"].into_iter().enumerate() {
            if let Some(v) = widgets::plain_field(ui, (key, selected), unit.from_pt(o[k]), unit.suffix(), 2, 64.0) {
                o[k] = unit.to_pt(v);
                moved = true;
            }
        }
        if moved {
            changes.insert("origin".into(), json!(o));
        }
    });
    widgets::label_row(ui, tl!("Color:"), LABEL_W, |ui| {
        let own = g.color.is_some();
        if widgets::check(ui, "Own color", own, true) {
            changes.insert("color".into(), if own { Value::Null } else { json!("#4d9de0") });
        }
        if let Some(hex) = &g.color
            && let Some(s) = widgets::exact_text_field(ui, ("grid-color", selected), hex, 80.0)
            && redrob_vector_color::Color::from_hex(&s).is_some()
        {
            changes.insert("color".into(), json!(s));
        }
    });
    if !changes.is_empty() {
        changes.insert("index".into(), json!(selected));
        run(app, "grid.set", Value::Object(changes));
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    if widgets::menu_item(ui, tl!("Add Grid"), true, false) {
        run(app, "grid.add", json!({}));
    }
    if widgets::menu_item(ui, tl!("Show Grid"), true, app.ui.view.grid) {
        app.ui.view.grid = !app.ui.view.grid;
    }
    if widgets::menu_item(ui, tl!("Snap to Grid"), true, app.ui.view.snap_to_grid) {
        app.ui.view.snap_to_grid = !app.ui.view.snap_to_grid;
    }
}
