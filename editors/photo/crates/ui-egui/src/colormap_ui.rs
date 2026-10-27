//! Image › Mode › Color Table… for Indexed Color documents: the palette as a grid of swatches.
//! Clicking an entry sets the foreground colour, double-clicking edits it (hex field and
//! RGB values), `+` appends a new entry (the foreground colour), Delete removes the last entry,
//! and a dropdown picks the transparent entry. Changes apply through `image.mode.colorTable`
//! on OK (edited entries by index, so pixels keep their indices and follow the new colours).

use serde_json::{Map, Value, json};

use crate::PhotoApp;
use crate::state::DialogKind;

const MARK: &str = "__colormap";
const COMMAND: &str = "image.mode.colorTable";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

pub fn dialog_width(fields: &Map<String, Value>) -> Option<f32> {
    owns(fields).then_some(420.0)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Opens the dialog for the active Indexed Color document; `None` when it has no table.
pub fn open(app: &mut PhotoApp) -> Option<Result<Value, String>> {
    let d = app.session.active()?;
    let table = d.doc.color_table.clone()?;
    let colors: Vec<Value> = table.colors.iter().map(|c| json!(hex(*c))).collect();
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("Color Table…"));
    f.insert("colors".into(), Value::Array(colors.clone()));
    f.insert("original".into(), Value::Array(colors));
    f.insert("transparent".into(), table.transparent.map_or(Value::Null, |i| json!(i)));
    f.insert("selected".into(), json!(-1));
    f.insert("editHex".into(), json!(""));
    Some(Ok(json!({ "dialog": app.ui.open_dialog(DialogKind::Command, f) })))
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    redrob_photo_engine::prefs::parse_hex(s)
}

/// The dialog body. The foreground colour is applied by [`tick`] (the body has no app access).
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut colors: Vec<String> =
        f.get("colors").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let mut selected = f.get("selected").and_then(Value::as_i64).unwrap_or(-1);
    let mut transparent = f.get("transparent").and_then(Value::as_u64).map(|v| v as i64).unwrap_or(-1);
    let mut edit_hex = f.get("editHex").and_then(Value::as_str).unwrap_or("").to_string();
    if let Some(error) = f.get("__error").and_then(Value::as_str) {
        ui.colored_label(ui.visuals().error_fg_color, error);
        ui.add_space(4.0);
    }
    ui.label(crate::i18n::fmt(tl!("{n} entries; click one to set the foreground colour, double-click to edit it"), &[("n", &colors.len().to_string())]));
    ui.add_space(4.0);
    // 16 swatches per row.
    let cell = 22.0;
    let cols = 16usize;
    let rows = colors.len().div_ceil(cols);
    let (grid, _) = ui.allocate_exact_size(egui::vec2(cell * cols as f32, cell * rows.max(1) as f32), egui::Sense::hover());
    let mut set_fg: Option<i64> = None;
    for (i, c) in colors.iter().enumerate() {
        let x = (i % cols) as f32;
        let y = (i / cols) as f32;
        let r = egui::Rect::from_min_size(egui::pos2(grid.left() + x * cell, grid.top() + y * cell), egui::vec2(cell, cell));
        let rgb = parse_hex(c).unwrap_or([0, 0, 0]);
        let resp = ui.interact(r, ui.id().with(("colormap-cell", i)), egui::Sense::click());
        ui.painter().rect_filled(r.shrink(1.0), 2.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
        if transparent == i as i64 {
            // A diagonal marks the transparent entry.
            ui.painter().line_segment([r.left_bottom(), r.right_top()], egui::Stroke::new(1.5, egui::Color32::WHITE));
        }
        if selected == i as i64 {
            ui.painter().rect_stroke(r.shrink(0.5), 2.0, egui::Stroke::new(2.0, ui.visuals().selection.stroke.color), egui::StrokeKind::Inside);
        }
        resp.clone().on_hover_text(format!("{i}: {c}"));
        if resp.clicked() {
            selected = i as i64;
            edit_hex = c.clone();
            set_fg = Some(i as i64);
        }
        if resp.double_clicked() {
            selected = i as i64;
            edit_hex = c.clone();
            f.insert("editing".into(), json!(true));
        }
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Entry"));
        let enabled = (0..colors.len() as i64).contains(&selected);
        let mut idx = selected.max(0);
        if ui.add_enabled(!colors.is_empty(), egui::DragValue::new(&mut idx).range(0..=(colors.len() as i64 - 1).max(0))).changed() {
            selected = idx;
            edit_hex = colors.get(idx as usize).cloned().unwrap_or_default();
        }
        ui.label(tl!("Color"));
        let r = ui.add_enabled(enabled, egui::TextEdit::singleline(&mut edit_hex).desired_width(80.0));
        if enabled
            && r.changed()
            && let Some(rgb) = parse_hex(&edit_hex)
            && let Some(slot) = colors.get_mut(selected as usize)
        {
            *slot = hex(rgb);
        }
        if enabled {
            let mut rgb = parse_hex(&edit_hex).unwrap_or([0, 0, 0]);
            let mut changed = false;
            for (k, label) in ["R", "G", "B"].iter().enumerate() {
                ui.label(*label);
                let mut v = i64::from(rgb[k]);
                if ui.add(egui::DragValue::new(&mut v).range(0..=255)).changed() {
                    rgb[k] = v as u8;
                    changed = true;
                }
            }
            if changed {
                edit_hex = hex(rgb);
                if let Some(slot) = colors.get_mut(selected as usize) {
                    *slot = edit_hex.clone();
                }
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.add_enabled(colors.len() < 256, egui::Button::new("+")).on_hover_text(tl!("Add an entry with the foreground colour")).clicked() {
            f.insert("addEntry".into(), json!(true));
        }
        if ui.add_enabled(colors.len() > 1, egui::Button::new("−")).on_hover_text(tl!("Delete the last entry")).clicked() {
            colors.pop();
            if transparent >= colors.len() as i64 {
                transparent = -1;
            }
            if selected >= colors.len() as i64 {
                selected = colors.len() as i64 - 1;
                edit_hex = colors.last().cloned().unwrap_or_default();
            }
        }
        ui.separator();
        ui.label(tl!("Transparent"));
        let mut opts: Vec<(i64, String)> = vec![(-1, tl!("None").to_string())];
        opts.extend((0..colors.len() as i64).map(|i| (i, i.to_string())));
        let refs: Vec<(i64, &str)> = opts.iter().map(|(i, s)| (*i, s.as_str())).collect();
        crate::widgets::dropdown(ui, "colormap-transparent", &mut transparent, &refs, 90.0);
        if (0..colors.len() as i64).contains(&selected) && ui.small_button(tl!("Use selected")).clicked() {
            transparent = selected;
        }
    });
    f.insert("colors".into(), Value::Array(colors.into_iter().map(Value::String).collect()));
    f.insert("selected".into(), json!(selected));
    f.insert("transparent".into(), if transparent < 0 { Value::Null } else { json!(transparent) });
    f.insert("editHex".into(), json!(edit_hex));
    if let Some(i) = set_fg {
        f.insert("setForeground".into(), json!(i));
    }
}

/// Per-frame follow-up with app access: a clicked entry becomes the foreground colour, `+` adds
/// an entry in the foreground colour. Called from the dialog loop for open Color Table dialogs.
pub fn tick(app: &mut PhotoApp) {
    let Some(idx) = app.ui.dialogs.iter().position(|d| owns(&d.fields)) else { return };
    let fields = &mut app.ui.dialogs[idx].fields;
    let mut set_fg: Option<String> = None;
    if let Some(i) = fields.remove("setForeground").and_then(|v| v.as_i64()) {
        set_fg = fields.get("colors").and_then(Value::as_array).and_then(|a| a.get(i.max(0) as usize)).and_then(Value::as_str).map(str::to_string);
    }
    if fields.remove("addEntry").is_some() {
        let fg = app.session.tools.foreground;
        let c =
            hex([(fg[0].clamp(0.0, 1.0) * 255.0).round() as u8, (fg[1].clamp(0.0, 1.0) * 255.0).round() as u8, (fg[2].clamp(0.0, 1.0) * 255.0).round() as u8]);
        if let Some(Value::Array(a)) = fields.get_mut("colors")
            && a.len() < 256
        {
            a.push(json!(c));
            let n = a.len() as i64 - 1;
            fields.insert("selected".into(), json!(n));
            fields.insert("editHex".into(), json!(c));
        }
    }
    if let Some(c) = set_fg
        && let Err(e) = app.run("tools.setColors", json!({"foreground": c}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

pub fn confirm(app: &mut PhotoApp, f: &Map<String, Value>) -> Result<Value, String> {
    let colors: Vec<&str> = f.get("colors").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let original: Vec<&str> = f.get("original").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut p = Map::new();
    if colors.len() != original.len() {
        p.insert("colors".into(), json!(colors));
    } else {
        let mut entries = Map::new();
        for (i, (c, o)) in colors.iter().zip(&original).enumerate() {
            if c != o {
                entries.insert(i.to_string(), json!(c));
            }
        }
        if !entries.is_empty() {
            p.insert("entries".into(), Value::Object(entries));
        }
    }
    p.insert("transparent".into(), f.get("transparent").cloned().unwrap_or(Value::Null));
    let result = app.run(COMMAND, Value::Object(p));
    if let Err(e) = &result {
        let mut again = f.clone();
        again.insert("__error".into(), json!(e));
        app.ui.open_dialog(DialogKind::Command, again);
    }
    result
}
