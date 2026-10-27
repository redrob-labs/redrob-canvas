//! The Brush Preset History tab (Window › Brushes › Brush History): the presets used recently,
//! newest first, with Forget and Clear, the list's behaviour (static, move to top, bubble up)
//! and length, and the Ten Brushes slots (assign the current preset, activate a slot). Every
//! change is a `brush.history.*` or `brush.tenBrushes.*` command.

use egui::{RichText, vec2};
use serde_json::json;

use crate::brush_panel::run_or_status;
use crate::brush_preview;
use crate::theme::Tokens;
use crate::{PhotoApp, icons, widgets};

const MODES: [(&str, &str); 3] = [("static", "Static"), ("moveToTop", "Move to Top"), ("bubbleUp", "Bubble Up")];

pub fn show(app: &mut PhotoApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let items: Vec<String> = app.session.tools.preset_history.items.clone();
    let current = app.session.tools.current_preset.clone();
    ui.horizontal(|ui| {
        let mut mode = app.session.tools.preset_history.mode.name().to_string();
        let opts: Vec<(String, &str)> = MODES.iter().map(|(k, l)| (k.to_string(), *l)).collect();
        if widgets::dropdown(ui, "brush-history-mode", &mut mode, &opts, 120.0) {
            run_or_status(app, "brush.history.configure", json!({ "mode": mode }));
        }
        let mut count = app.session.tools.preset_history.count as f32;
        if widgets::value_field(ui, &mut count, 1.0..=100.0, "", 52.0).on_hover_text(tl!("Presets kept in the history")).changed() {
            run_or_status(app, "brush.history.configure", json!({ "count": count.round().clamp(1.0, 100.0) as u32 }));
        }
        if icons::button(ui, "trash", 24.0, false, tl!("Clear the history")).clicked() {
            run_or_status(app, "brush.history.clear", json!({}));
        }
    });
    ui.add_space(4.0);
    if items.is_empty() {
        ui.label(RichText::new(tl!("No presets used yet")).color(t.text_faint));
    }
    let mut pick: Option<String> = None;
    let mut forget: Option<String> = None;
    egui::ScrollArea::vertical().id_salt("brush-history-list").max_height(220.0).show(ui, |ui| {
        for name in &items {
            let is_current = current.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(name));
            ui.horizontal(|ui| {
                if let Some(p) = redrob_photo_engine::paint::presets::find(&app.session.tools.presets, name) {
                    let b = &p.brush;
                    let tip = brush_preview::tip_texture(ui.ctx(), &format!("history-tip:{name}"), &b.tip, (b.hardness, b.angle, b.roundness), 28, t.text);
                    let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::hover());
                    ui.painter().image(tip.id(), r, crate::brush_panel::full_uv(), egui::Color32::WHITE);
                }
                let label = ui.selectable_label(is_current, RichText::new(name.as_str()).color(if is_current { t.accent } else { t.text }));
                if label.clicked() {
                    pick = Some(name.clone());
                }
                if icons::button(ui, "minus", 20.0, false, tl!("Forget this preset")).clicked() {
                    forget = Some(name.clone());
                }
            });
        }
    });
    if let Some(n) = pick {
        run_or_status(app, "brush.history.pick", json!({ "name": n }));
    }
    if let Some(n) = forget {
        run_or_status(app, "brush.history.forget", json!({ "name": n }));
    }
    ui.add_space(8.0);
    ui.label(RichText::new(tl!("Ten Brushes")).font(crate::theme::semibold(12.0)).color(t.text));
    ui.label(RichText::new(tl!("Assign presets to ten slots; activating a slot again returns to the previous preset")).color(t.text_faint).size(11.0));
    let slots: Vec<Option<String>> = app.session.tools.ten_brushes.to_vec();
    let mut assign: Option<usize> = None;
    let mut clear: Option<usize> = None;
    let mut activate: Option<usize> = None;
    for (i, slot) in slots.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{}", (i + 1) % 10)).color(t.text_dim).monospace());
            match slot {
                Some(name) => {
                    let is_current = current.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(name));
                    if ui.selectable_label(is_current, name.as_str()).clicked() {
                        activate = Some(i + 1);
                    }
                    if icons::button(ui, "minus", 20.0, false, tl!("Clear this slot")).clicked() {
                        clear = Some(i + 1);
                    }
                }
                None => {
                    if widgets::secondary_button(ui, tl!("Assign current"), 0.0).clicked() {
                        assign = Some(i + 1);
                    }
                }
            }
        });
    }
    if let Some(slot) = assign {
        run_or_status(app, "brush.tenBrushes.assign", json!({ "slot": slot }));
    }
    if let Some(slot) = clear {
        run_or_status(app, "brush.tenBrushes.assign", json!({ "slot": slot, "name": null }));
    }
    if let Some(slot) = activate {
        run_or_status(app, "brush.tenBrushes.activate", json!({ "slot": slot }));
    }
}
