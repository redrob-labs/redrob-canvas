//! The painting tools' extra options-bar controls, after the Line toggle: assistant snapping and
//! magnetism, mirroring every tool's output, the Pencil's pixel smoothing, the Multibrush's
//! modes and the Dynamic Brush's physics.

use crate::{PhotocraftApp, state::Tool, widgets};

/// Appended to the Brush, Pencil and Eraser options bars after the Line toggle.
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let tool = app.ui.tool;
    crate::assistant_ui::brush_options(app, ui, tool);
    if app.session.active().is_some_and(|st| st.symmetry.is_some()) {
        widgets::checkbox(ui, &mut app.ui.tool_options.mirror_all_tools, tl!("Mirror all tools"))
            .on_hover_text(tl!("The painting symmetry mirrors fills, gradients and retouching too"));
    }
    if tool == Tool::Pencil {
        widgets::checkbox(ui, &mut app.ui.tool_options.pixel_smoothing, tl!("Pixel smoothing"))
            .on_hover_text(tl!("Keep one-pixel lines single width without L corners"));
    }
    crate::multibrush_ui::options(app, ui);
    crate::dyna_ui::options(app, ui);
}
