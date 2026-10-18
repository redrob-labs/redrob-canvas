//! The painting tools' extra options-bar controls: assistant snapping and magnetism.

use crate::PhotocraftApp;

/// Appended to the Brush, Pencil and Eraser options bars after the Line toggle.
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let tool = app.ui.tool;
    crate::assistant_ui::brush_options(app, ui, tool);
}
