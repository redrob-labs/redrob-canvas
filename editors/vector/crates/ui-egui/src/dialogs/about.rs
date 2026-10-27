//! Help → About Redrob Canvas.

use crate::VectorApp;
use crate::theme;

/// The About window, while `UiState::about` is set.
pub(super) fn show(app: &mut VectorApp, ctx: &egui::Context) {
    if !app.ui.about {
        return;
    }
    let mut open = true;
    egui::Window::new(tl!("About Redrob Canvas"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .frame(egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(18)))
        .show(ctx, |ui| {
            // Redrob Canvas shows only the About tab: no tab row for Contributors or Models.
            let tab_id = egui::Id::new("about_tab");
            let tab = ui.data_mut(|d| d.get_temp::<u8>(tab_id)).unwrap_or(0);
            match tab {
                1 => {
                    ui.set_width(660.0);
                    crate::credits::contributors_ui(app, ui);
                }
                2 => {
                    ui.set_width(660.0);
                    crate::credits::models_ui(ui);
                }
                _ => about_tab(app, ui),
            }
        });
    app.ui.about = open;
}

/// The About tab: mark, version, community links, licences and the GPU in use.
fn about_tab(app: &mut VectorApp, ui: &mut egui::Ui) {
    ui.set_width(380.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
        crate::brand::paint_mark(ui, r);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("Redrob Canvas").font(theme::semibold(22.0)));
            ui.label(crate::i18n::fmt(
                tl!("Version {version} — open-source vector illustration in pure Rust."),
                &[("version", env!("CARGO_PKG_VERSION"))],
            ));
        });
    });
    ui.add_space(12.0);
    crate::community::links(app, ui);
    ui.add_space(12.0);
    ui.label(
        egui::RichText::new(tl!(
            "Part of Redrob. MIT OR Apache-2.0. Fonts: Source Sans 3, Inter, JetBrains Mono (OFL). Icons: Lucide (ISC) + Redrob Canvas."
        ))
        .size(11.0),
    );
    // The GPU the window renders with, for bug reports (Preferences › Performance picks it).
    if let Some(adapter) = &app.graphics_adapter {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Graphics: {adapter}"), &[("adapter", adapter.as_str())])).size(11.0));
    }
}
