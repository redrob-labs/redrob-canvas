//! Paint Dynamics editor: the matrix of input sensors (columns) against brush outputs (rows),
//! with each row's minimum and the preset's fade length and random amount. Opened from the brush
//! panel's Dynamics row; Apply puts the table on the current brush (`brush.dynamics.apply`), Save
//! stores it as a named `.rdyn` preset (`brush.dynamics.save`).
//!
//! The editor's state lives in egui memory, keyed by [`ID`], so it needs no settings field.

use egui::RichText;
use redrob_photo_paint::{DynInput, DynOutput, DynamicsPreset};
use serde_json::json;

use crate::PhotoApp;
use crate::theme::Tokens;
use crate::widgets;

const ID: &str = "paint-dynamics-editor";

#[derive(Clone, Default)]
struct Editor {
    open: bool,
    preset: DynamicsPreset,
    name: String,
}

fn id() -> egui::Id {
    egui::Id::new(ID)
}

fn load(ctx: &egui::Context) -> Editor {
    ctx.data(|d| d.get_temp::<Editor>(id())).unwrap_or_default()
}

fn store(ctx: &egui::Context, e: Editor) {
    ctx.data_mut(|d| d.insert_temp(id(), e));
}

/// Open the editor on `preset` (a copy; the current preset's name is kept for Save).
pub fn open(ctx: &egui::Context, preset: DynamicsPreset) {
    let name = if preset.builtin { format!("{} copy", preset.name) } else { preset.name.clone() };
    store(ctx, Editor { open: true, preset, name });
}

/// Is the editor window showing?
pub fn is_open(ctx: &egui::Context) -> bool {
    load(ctx).open
}

/// Draw the window when it is open.
pub fn window(app: &mut PhotoApp, ctx: &egui::Context) {
    let mut e = load(ctx);
    if !e.open {
        return;
    }
    let mut open = true;
    let mut apply = false;
    let mut save = false;
    egui::Window::new(tl!("Paint Dynamics Editor"))
        .id(egui::Id::new("paint-dynamics-editor-window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .show(ctx, |ui| {
            let t = Tokens::get(ui.ctx());
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Name")).color(t.text_dim));
                ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(220.0));
            });
            ui.add_space(6.0);
            // The matrix: one row per output, one checkbox per input.
            egui::Grid::new("paint-dynamics-matrix").striped(true).spacing([6.0, 4.0]).show(ui, |ui| {
                ui.label("");
                for input in DynInput::ALL {
                    ui.label(RichText::new(tl!(input.label())).color(t.text_dim).small());
                }
                ui.label(RichText::new(tl!("Minimum")).color(t.text_dim).small());
                ui.end_row();
                for output in DynOutput::ALL {
                    ui.label(tl!(output.label()));
                    for input in DynInput::ALL {
                        let mut on = e.preset.maps(output, input);
                        if ui.checkbox(&mut on, "").changed() {
                            e.preset.set(output, input, on);
                        }
                    }
                    let mut min = e.preset.row(output).map_or(0.0, |r| r.minimum) * 100.0;
                    let has_row = e.preset.row(output).is_some();
                    ui.add_enabled_ui(has_row, |ui| {
                        if widgets::value_field(ui, &mut min, 0.0..=100.0, "%", 58.0).changed() && has_row {
                            e.preset.row_mut(output).minimum = (min / 100.0).clamp(0.0, 1.0);
                        }
                    });
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Fade length")).color(t.text_dim));
                let mut fade = e.preset.fade_steps as f32;
                if widgets::value_field(ui, &mut fade, 1.0..=10000.0, "", 70.0).changed() {
                    e.preset.fade_steps = fade.max(1.0) as u32;
                }
                ui.label(RichText::new(tl!("Random amount")).color(t.text_dim));
                let mut rnd = e.preset.random_amount * 100.0;
                if widgets::value_field(ui, &mut rnd, 0.0..=100.0, "%", 58.0).changed() {
                    e.preset.random_amount = (rnd / 100.0).clamp(0.0, 1.0);
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                apply = widgets::primary_button(ui, tl!("Apply"), 0.0).on_hover_text(tl!("Put this table on the current brush")).clicked();
                save = widgets::secondary_button(ui, tl!("Save Preset"), 0.0).on_hover_text(tl!("Save the table as a named dynamics preset")).clicked();
            });
        });
    if apply {
        let mut p = e.preset.clone();
        p.name = e.name.trim().to_string();
        p.builtin = false;
        let value = serde_json::to_value(&p).unwrap_or(serde_json::Value::Null);
        crate::brush_panel::run_or_status(app, "brush.dynamics.apply", json!({ "preset": value }));
    }
    if save {
        let name = e.name.trim().to_string();
        if name.is_empty() {
            app.ui.status = tl!("Give the dynamics preset a name first").to_string();
            app.ui.status_error = true;
        } else {
            let mut p = e.preset.clone();
            p.name = name.clone();
            p.builtin = false;
            let value = serde_json::to_value(&p).unwrap_or(serde_json::Value::Null);
            crate::brush_panel::run_or_status(app, "brush.dynamics.save", json!({ "name": name, "preset": value }));
        }
    }
    e.open = open;
    store(ctx, e);
}
