//! Foreground Select: the two-stage matting workflow on the Lasso tool (its Foreground Select
//! switch in the options bar).
//!
//! 1. The first closed lasso drag is the rough outline round the subject (nothing is selected
//!    yet; the outline is drawn on the canvas).
//! 2. Every later drag is a mark: a foreground stroke (the default) or a background stroke (the
//!    Background switch, or ⌥ while drawing), `fg_stroke_width` px wide, drawn tinted on the
//!    canvas.
//! 3. Preview runs the matting engine (`select.foreground`) and shows the mask as a Quick Mask
//!    overlay in the chosen colour (or as grayscale with the colour channels hidden); Select
//!    commits the mask to the selection, To Layer Mask puts it on the active layer's mask; Reset
//!    drops the outline and marks.
//!
//! Two engines are offered: the full graph cut, and a fast one that runs the same cut on a
//! smaller pixel budget (`select.foreground`'s `fast`).

use egui::{Color32, Pos2, Stroke};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::theme::Tokens;
use crate::widgets;

/// One mark stroke over the subject.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub points: Vec<[f64; 2]>,
    pub size: f32,
    pub background: bool,
}

/// The workflow's state: the outline once drawn, the marks since, and whether a preview is up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForegroundSelect {
    pub outline: Vec<[f64; 2]>,
    pub marks: Vec<Mark>,
    /// The preview Quick Mask is showing (`select.editInQuickMaskMode` on).
    pub previewing: bool,
    /// The colour channels were hidden for a grayscale preview.
    pub gray_hidden: bool,
}

impl ForegroundSelect {
    pub fn has_outline(&self) -> bool {
        self.outline.len() >= 3
    }
}

/// Whether the Lasso is in Foreground Select mode.
pub fn active(app: &PhotocraftApp) -> bool {
    app.ui.tool == crate::state::Tool::Lasso && app.ui.tool_options.foreground_select
}

/// Takes a finished lasso drag when Foreground Select is on: the outline the first time, a mark
/// afterwards. Returns false when the drag is an ordinary lasso selection.
pub fn take_lasso(app: &mut PhotocraftApp, pts: &[[f64; 2]], alt: bool) -> bool {
    if !active(app) {
        return false;
    }
    if app.fg_select.previewing {
        end_preview(app, false);
    }
    if !app.fg_select.has_outline() {
        if pts.len() >= 3 {
            app.fg_select.outline = pts.to_vec();
            app.ui.status = tl!("Outline placed: paint over the foreground, then Preview or Select").to_string();
            app.ui.status_error = false;
        }
        return true;
    }
    if !pts.is_empty() {
        let o = &app.ui.tool_options;
        app.fg_select.marks.push(Mark { points: pts.to_vec(), size: o.fg_stroke_width.max(1.0), background: o.fg_mark_background || alt });
    }
    true
}

fn colour_hex(name: &str) -> &'static str {
    match name {
        "green" => "#00c853",
        "blue" => "#2979ff",
        _ => "#ff1744",
    }
}

fn params(app: &PhotocraftApp) -> serde_json::Value {
    let f = &app.fg_select;
    let o = &app.ui.tool_options;
    let strokes = |bg: bool| -> Vec<serde_json::Value> { f.marks.iter().filter(|m| m.background == bg).map(|m| json!({"points": m.points, "size": m.size})).collect() };
    json!({
        "outline": f.outline,
        "foreground": strokes(false),
        "background": strokes(true),
        "sampleAllLayers": o.sample_all_layers,
        "fast": o.fg_engine == "fast",
        "mode": "replace",
    })
}

/// Runs the matting engine into the selection. Returns false (status set) on failure.
fn compute(app: &mut PhotocraftApp) -> bool {
    if !app.fg_select.has_outline() {
        app.ui.status = tl!("Draw a rough outline round the subject first").to_string();
        app.ui.status_error = true;
        return false;
    }
    let p = params(app);
    match app.run("select.foreground", p) {
        Ok(_) => true,
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            false
        }
    }
}

/// Preview: the mask as a Quick Mask overlay in the preview colour (or grayscale).
pub fn preview(app: &mut PhotocraftApp) {
    if app.fg_select.previewing {
        end_preview(app, false);
    }
    if !compute(app) {
        return;
    }
    let colour = colour_hex(&app.ui.tool_options.fg_preview_color);
    let gray = app.ui.tool_options.fg_preview_gray;
    if app.run("select.editInQuickMaskMode", json!({"on": true})).is_err() {
        return;
    }
    let _ = app.run("channel.options", json!({"channel": "quickMask", "color": colour, "opacity": 60, "indicates": "selected"}));
    if gray {
        let _ = app.run("channel.setVisible", json!({"channel": "composite", "visible": false}));
    }
    app.fg_select.previewing = true;
    app.fg_select.gray_hidden = gray;
}

/// Leaves the preview: back to the selection (kept when `keep`, else the mask is dropped and the
/// selection restored to what it was by undoing the preview's steps).
fn end_preview(app: &mut PhotocraftApp, keep: bool) {
    if !app.fg_select.previewing {
        return;
    }
    if app.fg_select.gray_hidden {
        let _ = app.run("channel.setVisible", json!({"channel": "composite", "visible": true}));
    }
    let _ = app.run("select.editInQuickMaskMode", json!({"on": false}));
    if !keep {
        // The Quick Mask round trip left the engine's mask as the selection: drop it.
        let _ = app.run("select.deselect", json!({}));
    }
    app.fg_select.previewing = false;
    app.fg_select.gray_hidden = false;
}

/// Select: commits the mask to the selection and ends the workflow.
pub fn commit(app: &mut PhotocraftApp) {
    if app.fg_select.previewing {
        end_preview(app, true);
    } else if !compute(app) {
        return;
    }
    app.fg_select = ForegroundSelect::default();
}

/// To Layer Mask: commits the mask and puts it on the active layer as a layer mask.
pub fn to_layer_mask(app: &mut PhotocraftApp) {
    commit(app);
    if app.session.is_enabled("layer.layerMask.revealSelection") {
        let _ = app.run("layer.layerMask.revealSelection", json!({}));
    }
}

/// Reset: drops the outline, the marks and any preview.
pub fn reset(app: &mut PhotocraftApp) {
    if app.fg_select.previewing {
        end_preview(app, false);
    }
    app.fg_select = ForegroundSelect::default();
}

fn opt(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).color(t.text_dim).size(12.0));
}

/// The Foreground Select part of the Lasso options bar.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    {
        let o = &mut app.ui.tool_options;
        opt(ui, tl!("Stroke width"));
        widgets::value_field(ui, &mut o.fg_stroke_width, 1.0..=400.0, "px", 58.0);
        let marks = [(false, tl!("Foreground")), (true, tl!("Background"))];
        widgets::dropdown(ui, "fg-mark", &mut o.fg_mark_background, &marks, 104.0);
        opt(ui, tl!("Engine"));
        let engines = [("graphCut".to_string(), tl!("Graph cut")), ("fast".to_string(), tl!("Fast"))];
        widgets::dropdown(ui, "fg-engine", &mut o.fg_engine, &engines, 96.0);
        opt(ui, tl!("Preview"));
        let colours = [("red".to_string(), tl!("Red")), ("green".to_string(), tl!("Green")), ("blue".to_string(), tl!("Blue"))];
        widgets::dropdown(ui, "fg-color", &mut o.fg_preview_color, &colours, 80.0);
        widgets::checkbox(ui, &mut o.fg_preview_gray, tl!("Grayscale"));
    }
    widgets::vline(ui, 22.0);
    let has_outline = app.fg_select.has_outline();
    if ui.add_enabled_ui(has_outline, |ui| widgets::secondary_button(ui, tl!("Preview"), 0.0)).inner.clicked() {
        preview(app);
    }
    if ui.add_enabled_ui(has_outline, |ui| widgets::primary_button(ui, tl!("Select"), 0.0)).inner.clicked() {
        commit(app);
    }
    if ui.add_enabled_ui(has_outline, |ui| widgets::secondary_button(ui, tl!("To Layer Mask"), 0.0)).inner.clicked() {
        to_layer_mask(app);
    }
    if ui.add_enabled_ui(has_outline || !app.fg_select.marks.is_empty(), |ui| widgets::secondary_button(ui, tl!("Reset"), 0.0)).inner.clicked() {
        reset(app);
    }
    let t = Tokens::get(ui.ctx());
    let hint = if !has_outline {
        tl!("Draw a rough outline round the subject")
    } else {
        tl!("Paint over the foreground (hold Alt for background), then Preview or Select")
    };
    ui.label(egui::RichText::new(hint).color(t.text_faint));
}

/// Draws the outline and the marks on the canvas.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if !active(app) {
        return;
    }
    let f = &app.fg_select;
    let t = Tokens::get(painter.ctx());
    if f.has_outline() {
        let mut pts: Vec<Pos2> = f.outline.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
        if let Some(first) = pts.first().copied() {
            pts.push(first);
        }
        painter.add(egui::Shape::line(pts, Stroke::new(1.5, t.accent)));
    }
    let zoom = app.point_zoom().max(0.01);
    for m in &f.marks {
        let pts: Vec<Pos2> = m.points.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
        let colour = if m.background { Color32::from_rgba_unmultiplied(0x29, 0x79, 0xff, 140) } else { Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 140) };
        let width = (m.size * zoom).max(1.0);
        if pts.len() == 1 {
            painter.circle_filled(pts[0], width / 2.0, colour);
        } else {
            painter.add(egui::Shape::line(pts, Stroke::new(width, colour)));
        }
    }
}

#[cfg(test)]
mod tests {
    use redrob_photo_geom::Rect;
    use serde_json::json;

    use super::*;
    use crate::canvas::{ToolEvent, tool_event};
    use crate::state::Tool;

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(redrob_photo_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.session
            .edit("subject", |doc, a| {
                let surf = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
                surf.fill_rect(Rect::new(0, 0, 64, 64), &[0.1, 0.2, 0.9, 1.0]);
                surf.fill_rect(Rect::new(20, 20, 44, 44), &[0.9, 0.3, 0.1, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        app.ui.tool = Tool::Lasso;
        app.ui.tool_options.foreground_select = true;
        app
    }

    fn drag(app: &mut PhotocraftApp, pts: &[[f64; 2]], mods: egui::Modifiers) {
        tool_event(app, ToolEvent::Down { x: pts[0][0], y: pts[0][1], pressure: 1.0 }, mods);
        for p in &pts[1..] {
            tool_event(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, mods);
        }
        let last = pts[pts.len() - 1];
        tool_event(app, ToolEvent::Up { x: last[0], y: last[1] }, mods);
    }

    #[test]
    fn outline_then_marks_then_select() {
        let mut app = app();
        drag(&mut app, &[[12.0, 12.0], [52.0, 12.0], [52.0, 52.0], [12.0, 52.0], [12.0, 14.0]], egui::Modifiers::NONE);
        assert!(app.fg_select.has_outline(), "the first drag is the outline");
        assert!(app.session.active().unwrap().doc.selection.is_none(), "nothing is selected yet");
        drag(&mut app, &[[24.0, 32.0], [40.0, 32.0]], egui::Modifiers::NONE);
        drag(&mut app, &[[4.0, 4.0], [60.0, 4.0]], egui::Modifiers::ALT);
        assert_eq!(app.fg_select.marks.len(), 2);
        assert!(!app.fg_select.marks[0].background && app.fg_select.marks[1].background);
        commit(&mut app);
        let sel = app.session.active().unwrap().doc.selection.clone().expect("the subject is selected");
        assert!(sel.sample_channel(32, 32, 0) > 0.5);
        assert!(sel.sample_channel(2, 2, 0) < 0.5);
        assert!(!app.fg_select.has_outline(), "Select ends the workflow");
    }

    #[test]
    fn preview_shows_a_quick_mask_and_reset_clears_it() {
        let mut app = app();
        drag(&mut app, &[[12.0, 12.0], [52.0, 12.0], [52.0, 52.0], [12.0, 52.0], [12.0, 14.0]], egui::Modifiers::NONE);
        drag(&mut app, &[[24.0, 32.0], [40.0, 32.0]], egui::Modifiers::NONE);
        app.ui.tool_options.fg_preview_color = "green".into();
        preview(&mut app);
        assert!(app.fg_select.previewing);
        let st = app.session.active().unwrap();
        let q = st.doc.quick_mask.as_ref().expect("a Quick Mask previews the mask");
        let rgb = q.color.to_rgba8();
        assert_eq!([rgb[0], rgb[1], rgb[2]], [0x00, 0xc8, 0x53]);
        reset(&mut app);
        assert!(!app.fg_select.previewing && app.fg_select.marks.is_empty());
        assert!(app.session.active().unwrap().doc.quick_mask.is_none());
    }
}
