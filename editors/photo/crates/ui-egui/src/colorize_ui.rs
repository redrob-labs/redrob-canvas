//! The Colorize Mask tool: key strokes of colour drawn over a line-art layer, propagated into
//! its regions by `paint.colorize` onto a "Colorize" layer below it. The options bar edits the
//! key strokes (show them, clear them, a transparent key), updates the output, shows or hides it,
//! and sets edge detection, gap closing, clean-up and the layer-bounds limit.

use egui::Stroke;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{PhotoApp, canvas::ToolEvent, canvas::ViewXform, state::Tool, theme::Tokens, widgets};

/// A key stroke as the tool keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// sRGB 0..1; `None` is the transparent key.
    pub color: Option<[f32; 3]>,
    pub points: Vec<[f64; 2]>,
    pub width: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorizeState {
    pub strokes: Vec<Key>,
    /// The stroke being drawn.
    #[serde(skip)]
    pub current: Option<Key>,
    /// The next strokes are the transparent key.
    pub transparent_key: bool,
    /// Draw the key strokes over the canvas.
    pub edit_keys: bool,
    /// Show the Colorize layer (off hides it, to judge the line art alone).
    pub show_output: bool,
    pub edge_detection: bool,
    pub gap_close: f32,
    pub clean_up: f32,
    pub limit_to_layer: bool,
    /// Update the output after every stroke.
    pub auto_update: bool,
    /// The layer the last update wrote.
    pub output_layer: Option<u64>,
    #[serde(skip)]
    pub initialised: bool,
}

impl ColorizeState {
    fn init(&mut self) {
        if !self.initialised {
            self.initialised = true;
            if self.gap_close == 0.0 && self.clean_up == 0.0 && !self.edge_detection && !self.edit_keys && !self.show_output {
                // First use: the reference's defaults.
                self.edge_detection = true;
                self.gap_close = 8.0;
                self.edit_keys = true;
                self.show_output = true;
                self.auto_update = true;
            }
        }
    }
}

fn hex(c: [f32; 3]) -> String {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(c[0]), q(c[1]), q(c[2]))
}

/// Run `paint.colorize` with the current strokes and settings.
pub fn update(app: &mut PhotoApp) {
    let c = app.colorize.clone();
    let strokes: Vec<serde_json::Value> = c.strokes.iter().map(|k| json!({"color": k.color.map(hex), "points": k.points, "width": k.width})).collect();
    if strokes.is_empty() {
        return;
    }
    let r = app.run(
        "paint.colorize",
        json!({"strokes": strokes, "edgeDetection": c.edge_detection, "gapClose": c.gap_close, "cleanUp": c.clean_up, "limitToLayer": c.limit_to_layer}),
    );
    match r {
        Ok(v) => {
            app.colorize.output_layer = v.get("layer").and_then(serde_json::Value::as_u64);
            if !app.colorize.show_output
                && let Some(id) = app.colorize.output_layer
            {
                let _ = app.run("layer.setProps", json!({"layer": id, "visible": false}));
            }
        }
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Pointer input for the Colorize Mask tool: a drag draws a key stroke in the foreground colour
/// (or the transparent key); Alt-click deletes the key stroke under the pointer.
pub fn pointer(app: &mut PhotoApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if app.active_tool() != Tool::ColorizeMask {
        app.colorize.current = None;
        return false;
    }
    app.colorize.init();
    match ev {
        ToolEvent::Down { x, y, .. } => {
            if mods.alt {
                let tol = 8.0 / f64::from(app.point_zoom().max(0.01));
                if let Some(i) =
                    app.colorize.strokes.iter().rposition(|k| k.points.iter().any(|p| (p[0] - x).hypot(p[1] - y) <= tol + f64::from(k.width) / 2.0))
                {
                    app.colorize.strokes.remove(i);
                    if app.colorize.auto_update {
                        update(app);
                    }
                }
                return true;
            }
            let fg = app.session.tools.foreground;
            let color = (!app.colorize.transparent_key).then_some([fg[0], fg[1], fg[2]]);
            let width = app.session.tools.brush.size.clamp(1.0, 500.0);
            app.colorize.current = Some(Key { color, points: vec![[x, y]], width });
        }
        ToolEvent::Move { x, y, .. } => {
            if let Some(k) = app.colorize.current.as_mut()
                && k.points.last().is_none_or(|l| (l[0] - x).hypot(l[1] - y) >= 0.5)
            {
                k.points.push([x, y]);
            }
        }
        ToolEvent::Up { x, y } => {
            if let Some(mut k) = app.colorize.current.take() {
                if k.points.last().is_none_or(|l| l[0] != x || l[1] != y) {
                    k.points.push([x, y]);
                }
                if app.colorize.strokes.len() < 256 {
                    app.colorize.strokes.push(k);
                }
                if app.colorize.auto_update {
                    update(app);
                }
            }
        }
    }
    true
}

/// The tool's options bar.
pub fn options_bar(app: &mut PhotoApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::ColorizeMask {
        return false;
    }
    app.colorize.init();
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Colorize Mask")).color(t.text_dim));
    if widgets::secondary_button(ui, tl!("Update"), 0.0).clicked() {
        update(app);
    }
    let c = &mut app.colorize;
    widgets::checkbox(ui, &mut c.auto_update, tl!("Auto update"));
    widgets::checkbox(ui, &mut c.edit_keys, tl!("Edit key strokes")).on_hover_text(tl!("Show the key strokes over the canvas"));
    let mut show = c.show_output;
    if widgets::checkbox(ui, &mut show, tl!("Show output")).changed() {
        c.show_output = show;
        if let Some(id) = c.output_layer {
            let _ = app.run("layer.setProps", json!({"layer": id, "visible": show}));
        }
    }
    let c = &mut app.colorize;
    widgets::checkbox(ui, &mut c.transparent_key, tl!("Transparent key")).on_hover_text(tl!("The next key strokes keep their regions clear"));
    widgets::vline(ui, 22.0);
    widgets::checkbox(ui, &mut c.limit_to_layer, tl!("Limit to layer")).on_hover_text(tl!("Fill only inside the line art's pixels"));
    widgets::checkbox(ui, &mut c.edge_detection, tl!("Edge detection"));
    ui.label(egui::RichText::new(tl!("Gap close")).color(t.text_dim).size(12.0));
    widgets::value_field(ui, &mut c.gap_close, 0.0..=128.0, "px", 58.0);
    ui.label(egui::RichText::new(tl!("Clean up")).color(t.text_dim).size(12.0));
    widgets::value_field(ui, &mut c.clean_up, 0.0..=100.0, "%", 58.0);
    widgets::vline(ui, 22.0);
    let has = !c.strokes.is_empty();
    if ui.add_enabled_ui(has, |ui| widgets::secondary_button(ui, tl!("Clear key strokes"), 0.0)).inner.clicked() {
        app.colorize.strokes.clear();
    }
    true
}

/// The key strokes over the canvas (with the tool and Edit key strokes on).
pub fn draw(app: &PhotoApp, painter: &egui::Painter, xf: &ViewXform) {
    if app.ui.tool != Tool::ColorizeMask || !app.colorize.edit_keys {
        return;
    }
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
    let t = Tokens::get(painter.ctx());
    let zoom = xf.zoom.max(1e-3);
    for k in app.colorize.strokes.iter().chain(app.colorize.current.iter()) {
        let color = match k.color {
            Some(c) => egui::Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8),
            None => t.accent,
        };
        let pts: Vec<egui::Pos2> = k.points.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
        let width = (k.width * zoom).max(2.0);
        if pts.len() == 1 {
            painter.circle_filled(pts[0], width / 2.0, color);
        } else if pts.len() >= 2 {
            painter.add(egui::Shape::line(pts.clone(), Stroke::new(width, color)));
        }
        if k.color.is_none() {
            // The transparent key: a dashed outline instead of a fill.
            if pts.len() >= 2 {
                painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, egui::Color32::WHITE), 4.0, 4.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_defaults() {
        assert_eq!(hex([1.0, 0.5, 0.0]), "#ff8000");
        let mut c = ColorizeState::default();
        c.init();
        assert!(c.edge_detection && c.edit_keys && c.show_output && c.auto_update);
        assert_eq!(c.gap_close, 8.0);
    }
}
