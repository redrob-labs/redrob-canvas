// SPDX-License-Identifier: MIT OR Apache-2.0

//! The AI menu, the dock toggle and the dock, in the browser: the same places as on the desktop
//! (apps/redrob-canvas/src/ai/panel.rs), every tool disabled, and the dock saying where the AI
//! tools are.

use super::bridge::Capabilities;
use super::jobs::ToolKind;
use super::{Action, AiPanel, DESKTOP_ONLY, DESKTOP_URL};

pub const DEFAULT_WIDTH: f32 = 320.0;
pub const MIN_WIDTH: f32 = 240.0;
pub const MAX_WIDTH: f32 = 640.0;

/// The top bar's "AI" menu: the tools, each disabled with the reason, plus the dock toggle.
pub fn menu(ui: &mut egui::Ui, panel: &mut AiPanel, caps: &Capabilities) {
    ui.menu_button("AI", |ui| {
        for kind in ToolKind::ALL {
            ui.add_enabled(false, egui::Button::new(kind.label()))
                .on_disabled_hover_text(kind.unavailable_reason(caps).unwrap_or(DESKTOP_ONLY));
        }
        ui.separator();
        let label = if panel.open {
            "Hide AI panel"
        } else {
            "Show AI panel"
        };
        if ui.button(label).clicked() {
            panel.open = !panel.open;
            ui.close();
        }
    });
}

/// The top bar's right slot: the dock toggle.
pub fn toggle(ui: &mut egui::Ui, panel: &mut AiPanel) {
    if ui
        .selectable_label(panel.open, "AI")
        .on_hover_text(DESKTOP_ONLY)
        .clicked()
    {
        panel.open = !panel.open;
    }
}

/// The right dock: what the AI tools are and where to get them. Returns the actions its widgets
/// asked for (only the mode switch is possible here).
pub fn dock(ui: &mut egui::Ui, panel: &mut AiPanel, _caps: &Capabilities) -> Vec<Action> {
    let mut actions = Vec::new();
    ui.heading("AI");
    ui.add_space(4.0);
    ui.label(DESKTOP_ONLY);
    ui.label(
        egui::RichText::new(
            "Erase, inpaint, outpaint, background removal, upscaling, face restore, subject \
             selection and the agent run a local engine the browser cannot start. Image Trace \
             is still here: Mode > Send to Vector traces the photo in this tab.",
        )
        .weak(),
    );
    ui.add_space(8.0);
    ui.hyperlink_to("Get the desktop app", DESKTOP_URL);
    if let Some((line, error)) = &panel.status {
        ui.add_space(8.0);
        let color = if *error {
            ui.visuals().error_fg_color
        } else {
            ui.visuals().text_color()
        };
        ui.label(egui::RichText::new(line).color(color));
    }
    ui.add_space(8.0);
    if ui.small_button("Close").clicked() {
        panel.open = false;
    }
    // The shell drains `queued` before drawing the dock; nothing is queued from here.
    actions.append(&mut panel.queued);
    actions
}
