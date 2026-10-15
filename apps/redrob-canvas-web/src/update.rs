// SPDX-License-Identifier: MIT OR Apache-2.0

//! The update check as the web shell has it: none. A page is always the version its host
//! serves, so the Help menu says that where the desktop offers "Check for updates…"
//! (apps/redrob-canvas/src/update.rs), and the top bar's right slot shows nothing.

/// Help menu: the version this page is, and that the host updates it.
pub fn menu_items(ui: &mut egui::Ui) {
    ui.separator();
    ui.add_enabled(false, egui::Button::new("Check for updates…"))
        .on_disabled_hover_text("The web app is updated by the site that serves it");
    ui.label(
        egui::RichText::new(format!(
            "Redrob Canvas v{} (web)",
            env!("CARGO_PKG_VERSION")
        ))
        .weak(),
    );
}

/// Top bar right slot: nothing to offer.
pub fn top_bar(_ui: &mut egui::Ui) {}
