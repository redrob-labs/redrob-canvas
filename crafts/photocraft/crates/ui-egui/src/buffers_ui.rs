//! Window › Buffers: a floating panel listing the named buffers (Edit › Buffer › Cut / Copy /
//! Copy Visible Named…) with a thumbnail each, and Paste, Paste as New, Rename and Delete. The
//! Buffer menu entries ask for the name through a small form. Everything runs the engine's
//! `edit.buffer.*` commands.

use egui::{RichText, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Buffers panel view state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BuffersUi {
    pub open: bool,
    /// Selected buffer name.
    #[serde(default)]
    pub selected: Option<String>,
    /// A rename in progress: (buffer name, text being typed).
    #[serde(default, skip)]
    pub renaming: Option<(String, String)>,
}

pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    (id == "window.panel.buffers").then_some(app.ui.buffers.open)
}

pub fn handles(id: &str) -> bool {
    id == "window.panel.buffers"
}

/// Window › Buffers toggles the panel; the Edit › Buffer entries open their name form first.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "window.panel.buffers" {
        app.ui.buffers.open = !app.ui.buffers.open;
        return Some(Ok(json!({ "open": app.ui.buffers.open })));
    }
    let empty = params.as_object().is_none_or(|o| o.is_empty());
    if !empty {
        return None;
    }
    match id {
        "edit.buffer.cut" | "edit.buffer.copy" | "edit.buffer.copyVisible" => {
            let n = app.session.buffers.len() + 1;
            let mut f = serde_json::Map::new();
            f.insert("__command".into(), json!(id));
            f.insert("__label".into(), json!(label(id)));
            f.insert("__form".into(), json!(true));
            f.insert("__choices".into(), json!({}));
            f.insert("name".into(), json!(format!("Buffer {n}")));
            // Pasting back is where the panel is useful: show it after the first buffer.
            app.ui.buffers.open = true;
            Some(Ok(json!({ "dialog": app.ui.open_dialog(crate::state::DialogKind::Command, f) })))
        }
        "edit.buffer.paste" => {
            let names: Vec<String> = app.session.buffers.iter().map(|b| b.name.clone()).collect();
            if names.is_empty() {
                return Some(Err(tl!("There are no named buffers; use Edit > Buffer > Copy Named first").to_string()));
            }
            let mut f = serde_json::Map::new();
            f.insert("__command".into(), json!(id));
            f.insert("__label".into(), json!("Paste Named"));
            f.insert("__form".into(), json!(true));
            f.insert("__choices".into(), json!({ "name": names, "as": ["layer", "inPlace", "into", "new"] }));
            f.insert(
                "name".into(),
                json!(
                    app.ui.buffers.selected.clone().filter(|s| app.session.buffers.iter().any(|b| &b.name == s)).unwrap_or_else(|| app
                        .session
                        .buffers
                        .last()
                        .map(|b| b.name.clone())
                        .unwrap_or_default())
                ),
            );
            f.insert("as".into(), json!("layer"));
            Some(Ok(json!({ "dialog": app.ui.open_dialog(crate::state::DialogKind::Command, f) })))
        }
        _ => None,
    }
}

fn label(id: &str) -> &'static str {
    match id {
        "edit.buffer.cut" => "Cut Named",
        "edit.buffer.copy" => "Copy Named",
        _ => "Copy Visible Named",
    }
}

/// Render the Buffers panel.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.buffers.open {
        return;
    }
    let t = Tokens::get(ctx);
    let names: Vec<(String, u32, u32)> = app.session.buffers.iter().map(|b| (b.name.clone(), b.clip.bounds.width(), b.clip.bounds.height())).collect();
    let mut act: Option<(&str, Value)> = None;
    let mut close = false;
    let mut select: Option<String> = None;
    let mut rename_start: Option<String> = None;
    let mut rename_commit = false;
    let mut rename_cancel = false;
    let mut renaming = app.ui.buffers.renaming.clone();
    let selected = app.ui.buffers.selected.clone();
    let have_doc = app.session.active().is_some();
    // Thumbnails are small and rebuilt each frame the panel is open (at most 256 buffers).
    let thumbs: Vec<Option<(u32, u32, Vec<u8>)>> = (0..names.len()).map(|i| redrob_photo_engine::buffers_cmds::thumbnail(&app.session, i, 40)).collect();
    crate::analysis_ui::panel_window(app, ctx, "buffers", tl!("Buffers"), vec2(0.0, 300.0), 320.0, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Buffers")).strong().color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::icons::button(ui, "x", 20.0, false, tl!("Close")).clicked() {
                    close = true;
                }
            });
        });
        ui.separator();
        if names.is_empty() {
            ui.weak(tl!("No named buffers. Use Edit > Buffer > Copy Named… to keep a copy here."));
            return;
        }
        egui::ScrollArea::vertical().id_salt("buffers-rows").max_height(260.0).show(ui, |ui| {
            for (i, (name, w, h)) in names.iter().enumerate() {
                let is_sel = selected.as_deref() == Some(name.as_str());
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), egui::Sense::click());
                if is_sel {
                    ui.painter().rect_filled(rect, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
                }
                if let Some((tw, th, px)) = thumbs.get(i).and_then(|o| o.as_ref()) {
                    let img = egui::ColorImage::from_rgba_unmultiplied([*tw as usize, *th as usize], px);
                    let tex = ui.ctx().load_texture(format!("buffer-thumb-{i}"), img, egui::TextureOptions::LINEAR);
                    let size = vec2(*tw as f32, *th as f32);
                    let at = egui::Rect::from_min_size(egui::pos2(rect.left() + 6.0 + (40.0 - size.x) / 2.0, rect.top() + 3.0 + (40.0 - size.y) / 2.0), size);
                    ui.painter().image(tex.id(), at, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
                match &mut renaming {
                    Some((target, text)) if target == name => {
                        let edit = ui.put(
                            egui::Rect::from_min_size(egui::pos2(rect.left() + 52.0, rect.top() + 12.0), vec2(rect.width() - 60.0, 22.0)),
                            egui::TextEdit::singleline(text),
                        );
                        edit.request_focus();
                        if edit.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                rename_commit = true;
                            } else {
                                rename_cancel = true;
                            }
                        }
                    }
                    _ => {
                        ui.painter().text(
                            egui::pos2(rect.left() + 52.0, rect.top() + 14.0),
                            egui::Align2::LEFT_TOP,
                            name,
                            egui::FontId::proportional(12.5),
                            t.text,
                        );
                        ui.painter().text(
                            egui::pos2(rect.left() + 52.0, rect.top() + 30.0),
                            egui::Align2::LEFT_TOP,
                            format!("{w} × {h} px"),
                            egui::FontId::proportional(10.5),
                            t.text_dim,
                        );
                    }
                }
                if resp.clicked() {
                    select = Some(name.clone());
                }
                if resp.double_clicked() {
                    rename_start = Some(name.clone());
                }
                resp.context_menu(|ui| {
                    if ui.button(tl!("Paste Buffer")).clicked() {
                        act = Some(("edit.buffer.paste", json!({"name": name, "as": "layer"})));
                        ui.close();
                    }
                    if ui.button(tl!("Paste Buffer in Place")).clicked() {
                        act = Some(("edit.buffer.paste", json!({"name": name, "as": "inPlace"})));
                        ui.close();
                    }
                    if ui.button(tl!("Paste Buffer as New Image")).clicked() {
                        act = Some(("edit.buffer.paste", json!({"name": name, "as": "new"})));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(tl!("Rename Buffer…")).clicked() {
                        rename_start = Some(name.clone());
                        ui.close();
                    }
                    if ui.button(tl!("Delete Buffer")).clicked() {
                        act = Some(("edit.buffer.delete", json!({"name": name})));
                        ui.close();
                    }
                });
                ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(1.0, t.separator));
            }
        });
        ui.add_space(4.0);
        let sel = selected.clone().filter(|s| names.iter().any(|(n, _, _)| n == s)).or_else(|| names.last().map(|(n, _, _)| n.clone()));
        ui.horizontal(|ui| {
            let can_paste = have_doc && sel.is_some();
            if ui.add_enabled(can_paste, egui::Button::new(tl!("Paste"))).on_hover_text(tl!("Paste the buffer as a new layer")).clicked()
                && let Some(n) = &sel
            {
                act = Some(("edit.buffer.paste", json!({"name": n, "as": "layer"})));
            }
            if ui.add_enabled(sel.is_some(), egui::Button::new(tl!("Paste as New"))).on_hover_text(tl!("Paste the buffer as a new document")).clicked()
                && let Some(n) = &sel
            {
                act = Some(("edit.buffer.paste", json!({"name": n, "as": "new"})));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::icons::button(ui, "trash", 22.0, false, tl!("Delete Buffer")).clicked()
                    && let Some(n) = &sel
                {
                    act = Some(("edit.buffer.delete", json!({"name": n})));
                }
            });
        });
    });
    if close {
        app.ui.buffers.open = false;
    }
    if let Some(n) = select {
        app.ui.buffers.selected = Some(n);
    }
    if let Some(n) = rename_start {
        app.ui.buffers.renaming = Some((n.clone(), n));
    } else if rename_cancel {
        app.ui.buffers.renaming = None;
    } else if rename_commit {
        if let Some((from, to)) = app.ui.buffers.renaming.take()
            && from != to
            && !to.trim().is_empty()
        {
            act = Some(("edit.buffer.rename", json!({"name": from, "to": to.trim()})));
        }
    } else {
        app.ui.buffers.renaming = renaming;
    }
    if let Some((cmd, p)) = act {
        match app.run(cmd, p) {
            Ok(r) => {
                if let Some(to) = r.get("to").and_then(Value::as_str) {
                    app.ui.buffers.selected = Some(to.to_string());
                }
            }
            Err(e) => {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
    }
}
