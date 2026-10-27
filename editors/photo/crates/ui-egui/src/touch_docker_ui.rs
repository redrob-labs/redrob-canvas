//! The Touch Docker (Window › Touch Docker): a panel of big buttons for tablets without a
//! keyboard: file open and save, undo and redo, brush opacity and size, view rotation and zoom,
//! and preset switching. Every button runs the command the keyboard or menu would.

use egui::vec2;
use serde_json::{Value, json};

use crate::{PhotoApp, theme::Tokens};

/// The button side in points (comfortable for a finger).
const BUTTON: f32 = 52.0;

/// Window › Touch Docker toggles the window.
pub fn menu(app: &mut PhotoApp, id: &str) -> Option<Result<Value, String>> {
    if id != "window.touchDocker" {
        return None;
    }
    app.ui.panels.touch_docker = !app.ui.panels.touch_docker;
    Some(Ok(json!({"visible": app.ui.panels.touch_docker})))
}

pub fn checked(app: &PhotoApp, id: &str) -> Option<bool> {
    (id == "window.touchDocker").then_some(app.ui.panels.touch_docker)
}

fn big(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    crate::icons::button(ui, icon, BUTTON * 0.5, false, label)
}

/// Step the brush size (`+`/`-` as the bracket keys) or opacity (10 % steps).
fn step_brush(app: &mut PhotoApp, size: bool, up: bool) {
    let cmd = match (size, up) {
        (true, true) => "tools.increaseBrushSize",
        (true, false) => "tools.decreaseBrushSize",
        _ => {
            let o = (app.session.tools.brush.opacity + if up { 0.1 } else { -0.1 }).clamp(0.0, 1.0);
            let _ = app.run("tools.setBrush", json!({"brush": {"opacity": o}}));
            return;
        }
    };
    let _ = app.run(cmd, json!({}));
}

/// The preset before or after the current one in the list.
fn step_preset(app: &mut PhotoApp, next: bool) {
    let names: Vec<String> = app.session.tools.presets.iter().map(|p| p.name.clone()).collect();
    if names.is_empty() {
        return;
    }
    let at = app.session.tools.current_preset.as_deref().and_then(|c| names.iter().position(|n| n == c));
    let i = match (at, next) {
        (Some(i), true) => (i + 1) % names.len(),
        (Some(i), false) => (i + names.len() - 1) % names.len(),
        (None, _) => 0,
    };
    let _ = app.run("tools.setBrush", json!({"preset": names[i]}));
}

pub fn window(app: &mut PhotoApp, ctx: &egui::Context) {
    if !app.ui.panels.touch_docker {
        return;
    }
    let mut open = true;
    egui::Window::new(tl!("Touch Docker")).id(egui::Id::new("touch-docker")).open(&mut open).resizable(false).default_width(4.0 * BUTTON + 40.0).show(
        ctx,
        |ui| {
            let t = Tokens::get(ui.ctx());
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            ui.spacing_mut().button_padding = vec2(10.0, 10.0);
            let rows: [[(&str, &str, &str); 4]; 4] = [
                [
                    ("folder-open", "file.open", tl!("Open…")),
                    ("save", "file.save", tl!("Save")),
                    ("undo", "edit.undo", tl!("Undo")),
                    ("redo", "edit.redo", tl!("Redo")),
                ],
                [
                    ("minus", "opacity-", tl!("Opacity -")),
                    ("plus", "opacity+", tl!("Opacity +")),
                    ("minus", "size-", tl!("Size -")),
                    ("plus", "size+", tl!("Size +")),
                ],
                [
                    ("rotate-ccw", "rotate-", tl!("Rotate Left")),
                    ("rotate-cw", "rotate+", tl!("Rotate Right")),
                    ("zoom-out", "view.zoomOut", tl!("Zoom Out")),
                    ("zoom-in", "view.zoomIn", tl!("Zoom In")),
                ],
                [
                    ("chevron-left", "preset-", tl!("Previous Preset")),
                    ("chevron-right", "preset+", tl!("Next Preset")),
                    ("compass", "view.resetView", tl!("Reset View")),
                    ("maximize", "view.fitOnScreen", tl!("Fit on Screen")),
                ],
            ];
            for row in rows {
                ui.horizontal(|ui| {
                    for (icon, action, label) in row {
                        let icon = if crate::icons::exists(icon) { icon } else { "circle-dot" };
                        if big(ui, icon, label).clicked() {
                            match action {
                                "opacity-" => step_brush(app, false, false),
                                "opacity+" => step_brush(app, false, true),
                                "size-" => step_brush(app, true, false),
                                "size+" => step_brush(app, true, true),
                                "rotate-" => {
                                    let _ = crate::menus::invoke(app, ui.ctx(), "view.rotateView", json!({"delta": -15.0}));
                                }
                                "rotate+" => {
                                    let _ = crate::menus::invoke(app, ui.ctx(), "view.rotateView", json!({"delta": 15.0}));
                                }
                                "preset-" => step_preset(app, false),
                                "preset+" => step_preset(app, true),
                                cmd => {
                                    let _ = crate::menus::invoke(app, ui.ctx(), cmd, json!({}));
                                }
                            }
                        }
                    }
                });
            }
            let b = &app.session.tools.brush;
            ui.label(egui::RichText::new(format!("{}: {:.0} px   {}: {:.0} %", tl!("Size"), b.size, tl!("Opacity"), b.opacity * 100.0)).color(t.text_dim));
            if let Some(p) = &app.session.tools.current_preset {
                ui.label(egui::RichText::new(p.clone()).color(t.text_dim));
            }
        },
    );
    if !open {
        app.ui.panels.touch_docker = false;
    }
}
