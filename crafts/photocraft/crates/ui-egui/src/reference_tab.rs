//! The Reference Images tab (Window › Brushes › Reference): browse a folder of images and show
//! one beside the canvas, with zoom, pan and previous/next. The engine keeps the state and
//! decodes the images (`reference.*`); this tab draws them.

use egui::{Color32, RichText, Sense, pos2, vec2};
use serde_json::json;

use crate::brush_panel::run_or_status;
use crate::theme::Tokens;
use crate::{PhotocraftApp, icons, widgets};

pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (folder, files, index, zoom, rev) = {
        let st = &app.session.reference_images;
        (st.folder.clone(), st.files.clone(), st.index, st.zoom, st.rev)
    };
    ui.horizontal(|ui| {
        if icons::button(ui, "folder", 24.0, false, tl!("Browse a folder of reference images")).clicked() {
            let _ = app.ask_folder(|app, path| app.run("reference.browse", json!({ "folder": path })));
        }
        let shown: String = if folder.is_empty() { tl!("No folder").to_owned() } else { crate::file_open::display_name(&folder) };
        ui.label(RichText::new(shown).color(t.text_dim)).on_hover_text(folder.as_str());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(ui, "chevron-right", 22.0, false, tl!("Next image")).clicked() {
                run_or_status(app, "reference.next", json!({}));
            }
            if icons::button(ui, "chevron-left", 22.0, false, tl!("Previous image")).clicked() {
                run_or_status(app, "reference.previous", json!({}));
            }
        });
    });
    if !files.is_empty() {
        let mut pick: Option<usize> = None;
        egui::ScrollArea::vertical().id_salt("reference-files").max_height(90.0).show(ui, |ui| {
            for (i, f) in files.iter().enumerate() {
                if ui.selectable_label(index == Some(i), f.as_str()).clicked() {
                    pick = Some(i);
                }
            }
        });
        if let Some(i) = pick {
            run_or_status(app, "reference.show", json!({ "index": i }));
        }
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Zoom")).color(t.text_dim).size(11.0));
        let mut z = if zoom <= 0.0 { 0.0 } else { zoom.max(0.01).ln() };
        let r = widgets::slider(ui, &mut z, -4.0..=4.0, None);
        if r.changed() {
            run_or_status(app, "reference.zoom", json!({ "zoom": z.exp() }));
        }
        if widgets::secondary_button(ui, tl!("Fit"), 0.0).clicked() {
            run_or_status(app, "reference.zoom", json!({ "zoom": 0.0, "pan": [0.0, 0.0] }));
        }
    });
    let Some((w, h, px)) = app.session.reference_images.image.clone() else {
        ui.label(RichText::new(tl!("Pick an image to show it here")).color(t.text_faint));
        return;
    };
    // The texture is keyed on the engine's revision, so a new image replaces it.
    let id = egui::Id::new("reference-image-texture");
    let cached: Option<(u64, egui::TextureHandle)> = ui.data(|d| d.get_temp(id));
    let texture = match cached {
        Some((r, tex)) if r == rev => tex,
        _ => {
            let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
            let tex = ui.ctx().load_texture("reference-image", img, egui::TextureOptions::LINEAR);
            ui.data_mut(|d| d.insert_temp(id, (rev, tex.clone())));
            tex
        }
    };
    let avail = ui.available_size_before_wrap();
    let area = vec2(avail.x.max(64.0), avail.y.max(120.0));
    let (rect, resp) = ui.allocate_exact_size(area, Sense::drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, t.field);
    let fit = (rect.width() / w as f32).min(rect.height() / h as f32);
    let scale = if zoom <= 0.0 { fit } else { zoom * fit };
    let size = vec2(w as f32 * scale, h as f32 * scale);
    let pan = app.session.reference_images.pan;
    let center = rect.center() + vec2(pan[0] * size.x * 0.5, pan[1] * size.y * 0.5);
    let img_rect = egui::Rect::from_center_size(center, size);
    painter.image(texture.id(), img_rect, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    if resp.dragged() && zoom > 0.0 && size.x > 0.0 && size.y > 0.0 {
        let d = resp.drag_delta();
        let new_pan = [(pan[0] + d.x / (size.x * 0.5)).clamp(-1.0, 1.0), (pan[1] + d.y / (size.y * 0.5)).clamp(-1.0, 1.0)];
        app.session.reference_images.pan = new_pan;
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.0 {
            let base = if zoom <= 0.0 { 1.0 } else { zoom };
            let z = (base * (1.0 + scroll / 300.0)).clamp(0.05, 64.0);
            run_or_status(app, "reference.zoom", json!({ "zoom": z }));
        }
    }
}
