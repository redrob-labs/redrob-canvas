//! The brush editor's scratch pad: the stroke strip at the bottom of Brush Settings is a
//! paintable surface (`brush.scratchPad.*`). Drag on it to paint with the settings being edited;
//! the buttons fill it with white, the background colour, the current gradient, the current image
//! or the preset's icon, and Reload puts the stored preset back.

use egui::{Color32, Sense, vec2};
use redrob_photo_engine::BrushSettings;
use serde_json::{Value, json};

use crate::brush_panel::run_or_status;
use crate::theme::Tokens;
use crate::{PhotoApp, icons, widgets};

/// Draw the pad at `w × h` points and let the pointer paint on it with `b`.
pub fn strip(app: &mut PhotoApp, ui: &mut egui::Ui, b: &BrushSettings, w: u32, h: u32) {
    let t = Tokens::get(ui.ctx());
    if app.session.scratch_pad.as_ref().is_none_or(|p| p.width != w || p.height != h) {
        run_or_status(app, "brush.scratchPad.fill", json!({ "with": "white", "width": w, "height": h }));
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(w as f32, h as f32), Sense::click_and_drag());
    ui.painter().rect_filled(rect, t.radius_sm, t.field);
    let Some(pad) = app.session.scratch_pad.as_ref() else { return };
    // The texture is re-uploaded when the pad changes (its revision).
    let id = egui::Id::new("brush-scratch-pad-texture");
    let cached: Option<(u64, egui::TextureHandle)> = ui.data(|d| d.get_temp(id));
    let texture = match cached {
        Some((r, tex)) if r == pad.rev => tex,
        _ => {
            let img = egui::ColorImage::from_rgba_unmultiplied([pad.width as usize, pad.height as usize], &pad.rgba8());
            let tex = ui.ctx().load_texture("brush-scratch-pad", img, egui::TextureOptions::LINEAR);
            ui.data_mut(|d| d.insert_temp(id, (pad.rev, tex.clone())));
            tex
        }
    };
    ui.painter().image(texture.id(), rect, crate::brush_panel::full_uv(), Color32::WHITE);
    // Painting: the pointer's positions this frame, in pad pixels, go to the engine as one stroke
    // piece; the stroke restarts at each press.
    let pts_id = id.with("points");
    if resp.drag_started() || resp.clicked() {
        ui.data_mut(|d| d.insert_temp(pts_id, Vec::<[f64; 2]>::new()));
    }
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let x = f64::from((p.x - rect.left()) / rect.width()) * f64::from(w);
        let y = f64::from((p.y - rect.top()) / rect.height()) * f64::from(h);
        let mut pts: Vec<[f64; 2]> = ui.data(|d| d.get_temp(pts_id)).unwrap_or_default();
        pts.push([x, y]);
        let time = ui.input(|i| i.time) * 1000.0;
        // Overlap by the last point so the pieces join.
        let piece: Vec<Value> = pts.iter().rev().take(2).rev().map(|q| json!([q[0], q[1], 1.0, 0.0, 0.0, 0.0, time])).collect();
        let brush = serde_json::to_value(b).unwrap_or(Value::Null);
        run_or_status(app, "brush.scratchPad.stroke", json!({ "points": piece, "brush": brush, "seed": pts.len() as u64 }));
        ui.data_mut(|d| d.insert_temp(pts_id, pts));
    }
}

/// The fill buttons and Reload, drawn beside the pad.
pub fn buttons(app: &mut PhotoApp, ui: &mut egui::Ui) {
    for (label, with, tip) in [
        ("White", "white", tl!("Clear the scratch pad to white")),
        ("BG", "background", tl!("Fill the scratch pad with the background colour")),
        ("Gradient", "gradient", tl!("Fill the scratch pad with the current gradient")),
        ("Image", "image", tl!("Fill the scratch pad with the current image")),
        ("Icon", "icon", tl!("Fill the scratch pad with the preset's icon")),
    ] {
        if widgets::secondary_button(ui, tl!(label), 0.0).on_hover_text(tip).clicked() {
            run_or_status(app, "brush.scratchPad.fill", json!({ "with": with }));
        }
    }
    let has_preset = app.session.tools.current_preset.is_some();
    if ui.add_enabled_ui(has_preset, |ui| icons::button(ui, "undo-2", 24.0, false, tl!("Reload the stored preset, discarding the tweaks"))).inner.clicked() {
        run_or_status(app, "brush.presets.reload", json!({}));
    }
}
