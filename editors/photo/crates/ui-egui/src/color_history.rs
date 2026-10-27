//! Colour history: the foreground colours painted with, most recent first, shown as patches in
//! the Color panel and the pop-up palette. A colour enters the history when a painting stroke
//! begins with it (not while a slider is still being dragged), and a repeat moves to the front.

use egui::{Color32, Sense, Stroke, vec2};
use serde_json::json;

use crate::{PhotoApp, theme::Tokens};

/// Most colours kept.
pub const MAX: usize = 24;

/// Put `c` at the front of `history`, dropping an earlier copy and anything past [`MAX`].
pub fn push(history: &mut Vec<[f32; 4]>, c: [f32; 4]) {
    let same = |a: &[f32; 4]| a.iter().zip(&c).all(|(x, y)| (x - y).abs() < 0.5 / 255.0);
    history.retain(|a| !same(a));
    history.insert(0, c);
    history.truncate(MAX);
}

/// Record the foreground colour once per painting stroke. Run every frame.
pub fn track(app: &mut PhotoApp) {
    let painting = app.drag.as_ref().is_some_and(|d| crate::canvas::strokes_live(d.tool) && !d.erase);
    if painting && !app.color_history_recorded {
        let fg = app.session.tools.foreground;
        push(&mut app.ui.color_history, fg);
        app.color_history_recorded = true;
    } else if !painting {
        app.color_history_recorded = false;
    }
}

/// Make `c` the foreground (or, with Alt held, the background) colour.
pub fn pick(app: &mut PhotoApp, c: [f32; 4], background: bool) {
    let rgba = [c[0], c[1], c[2], c[3]];
    let params = if background { json!({"background": rgba}) } else { json!({"foreground": rgba}) };
    let _ = app.run("tools.setColors", params);
}

pub fn color32(c: [f32; 4]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

/// A row of the most recent `count` colours as `side`-point patches; a click picks one (Alt: as
/// the background). Draws nothing when the history is empty.
pub fn patches(app: &mut PhotoApp, ui: &mut egui::Ui, side: f32, count: usize) {
    let colors: Vec<[f32; 4]> = app.ui.color_history.iter().take(count).copied().collect();
    if colors.is_empty() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        for (i, c) in colors.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
            let hex = crate::color_picker_ui::hex([c[0], c[1], c[2]]);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, true, format!("Recent color {} {hex}", i + 1)));
            ui.painter().rect_filled(rect, 2.0, color32(*c));
            if resp.hovered() {
                ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            }
            let resp = resp.on_hover_text(hex);
            if resp.clicked() {
                picked = Some((*c, ui.input(|i| i.modifiers.alt)));
            }
        }
    });
    if let Some((c, bg)) = picked {
        pick(app, c, bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_moves_repeats_to_the_front_and_caps_the_list() {
        let mut h = Vec::new();
        push(&mut h, [1.0, 0.0, 0.0, 1.0]);
        push(&mut h, [0.0, 1.0, 0.0, 1.0]);
        push(&mut h, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(h, vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]]);
        for i in 0..40 {
            push(&mut h, [i as f32 / 40.0, 0.5, 0.5, 1.0]);
        }
        assert_eq!(h.len(), MAX);
        assert_eq!(h[0], [39.0 / 40.0, 0.5, 0.5, 1.0]);
    }
}
