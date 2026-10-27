//! The palette strip: a row of colour chips along the bottom of the window (Window › Palette
//! Strip). A click paints the fill, Shift-click the stroke, and a chip dragged onto an object on
//! the canvas paints that object ([`crate::widgets::PanelDrag::Paint`]). The ≡ menu at its left
//! end picks the palette shown (the document's swatches or any swatch library, user-added ones
//! included) and the chip size.

use egui::{Sense, Stroke, Ui, vec2};
use redrob_vector_color::Paint;
use redrob_vector_engine::cmd::swatchlib;
use serde_json::json;

use crate::VectorApp;
use crate::theme::Tokens;
use crate::widgets::{self, PanelDrag, menu_item};

/// The chip sizes the menu offers: (label, side in points).
const SIZES: [(&str, f32); 3] = [("Small Chips", 12.0), ("Medium Chips", 16.0), ("Large Chips", 22.0)];

/// The strip's height for chips `side` tall.
fn height(side: f32) -> f32 {
    side + 8.0
}

/// The paints of the palette shown: the document's swatches, or library `key`'s.
fn paints(app: &VectorApp, key: &str) -> Vec<(String, Paint)> {
    if key == swatchlib::DOCUMENT_SWATCHES {
        return app.session.active().map(|st| st.doc.swatches_iter().map(|s| (s.name.clone(), s.paint.clone())).collect()).unwrap_or_default();
    }
    swatchlib::library(&app.session, key).map(|(_, lib)| lib.iter().map(|s| (s.name.clone(), s.paint.clone())).collect()).unwrap_or_default()
}

/// Command params painting with a strip chip: a document swatch by name (it stays linked), a
/// library colour by value.
fn params(key: &str, paint: &Paint) -> serde_json::Value {
    match paint {
        Paint::Solid { color, .. } if key != swatchlib::DOCUMENT_SWATCHES => json!({ "color": crate::panels::color_json(color) }),
        other => crate::panels::paint_params(other),
    }
}

pub fn show(app: &mut VectorApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let side = app.ui.palette_strip_size.clamp(8.0, 40.0);
    egui::Panel::bottom("palette_strip")
        .exact_size(height(side))
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(6, 0)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let menu = widgets::icon_button(ui, "menu", tl!("Palette Strip Menu"), false, 18.0);
                egui::Popup::menu(&menu).show(|ui| strip_menu(app, ui));
                ui.add_space(4.0);
                let key = app.ui.palette_strip_library.clone();
                let chips = paints(app, &key);
                if chips.is_empty() {
                    widgets::dim_label(ui, tl!("No colours in this palette"));
                    return;
                }
                widgets::strip_scroll(ui, "palette-strip-scroll", |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (name, paint) in &chips {
                            let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
                            widgets::swatch_tile(ui, rect, paint, false, resp.hovered());
                            widgets::drag_source(ui, &resp, || PanelDrag::paint(paint.clone()));
                            let resp = resp.on_hover_text(name);
                            if resp.clicked() {
                                let stroke = ui.input(|i| i.modifiers.shift);
                                let mut p = params(&key, paint);
                                p["focus"] = json!(false);
                                if let Err(e) = app.run(if stroke { "paint.setStroke" } else { "paint.setFill" }, p) {
                                    app.status(e);
                                }
                            }
                        }
                    });
                });
            });
        });
}

/// The ≡ menu: the palettes (the document's swatches and every swatch library), the chip size,
/// and Hide.
fn strip_menu(app: &mut VectorApp, ui: &mut Ui) {
    widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(220.0);
        let current = app.ui.palette_strip_library.clone();
        if menu_item(ui, tl!("Document Swatches"), true, current == swatchlib::DOCUMENT_SWATCHES) {
            app.ui.palette_strip_library = swatchlib::DOCUMENT_SWATCHES.into();
        }
        ui.separator();
        for lib in swatchlib::libraries(&app.session).into_iter().filter(|l| l.category != "gradients") {
            if widgets::menu_item_name(ui, &lib.name, true, current == lib.id) {
                app.ui.palette_strip_library = lib.id.clone();
            }
        }
        ui.separator();
        for (label, side) in SIZES {
            if menu_item(ui, tl!(label), true, (app.ui.palette_strip_size - side).abs() < 0.5) {
                app.ui.palette_strip_size = side;
            }
        }
        ui.separator();
        if menu_item(ui, tl!("Hide Palette Strip"), true, false) {
            app.ui.palette_strip = false;
        }
    });
}
