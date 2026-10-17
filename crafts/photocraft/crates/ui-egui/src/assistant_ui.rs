//! Painting assistants on the canvas: the Assistant tool places, moves and deletes rulers,
//! parallel rulers, vanishing points and perspective grids (`paint.assistants.*`); the painting
//! tools' strokes snap to them ([`filter_event`]). The assistants and their handles are drawn over
//! the document while the tool is active or Show Assistants is on.

use egui::Stroke;
use redrob_photo_paint::assistants::{self, Assistant};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    PhotocraftApp,
    canvas::{ToolEvent, ViewXform},
    state::Tool,
    theme::Tokens,
    widgets,
};

/// What the Assistant tool is doing with the pointer down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    /// Dragging handle `handle` of assistant `index`.
    Handle { index: usize, handle: usize },
    /// Dragging assistant `index` as a whole, from `from`.
    Whole { index: usize, from: [f64; 2] },
    /// Drawing a new assistant of the options bar's kind from `start`.
    New { start: [f64; 2], current: [f64; 2] },
}

/// The stroke being snapped: where it began and, once its direction is known, which assistant
/// it follows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeSnap {
    pub start: [f64; 2],
    pub chosen: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantUi {
    /// The kind the Assistant tool places next (`Assistant::KINDS`).
    pub kind: String,
    /// Draw the assistants with the painting tools too, not only with the Assistant tool.
    pub show: bool,
    #[serde(skip)]
    pub drag: Option<Drag>,
    #[serde(skip)]
    pub stroke: Option<StrokeSnap>,
}

impl Default for AssistantUi {
    fn default() -> Self {
        Self { kind: "ruler".into(), show: true, drag: None, stroke: None }
    }
}

/// How far a stroke must travel before its direction picks an assistant (document pixels).
const DIRECTION_DISTANCE: f64 = 3.0;

fn tolerance(app: &PhotocraftApp) -> f64 {
    8.0 / f64::from(app.point_zoom().max(0.01))
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "parallelRuler" => tl!("Parallel Ruler"),
        "vanishingPoint" => tl!("Vanishing Point"),
        "perspectiveGrid" => tl!("Perspective Grid"),
        _ => tl!("Ruler"),
    }
}

fn assistants(app: &PhotocraftApp) -> Vec<Assistant> {
    app.session.active().map(|st| st.assistants.clone()).unwrap_or_default()
}

/// The handle under `p`: (assistant, handle), the nearest within tolerance.
fn handle_at(list: &[Assistant], p: [f64; 2], tol: f64) -> Option<(usize, usize)> {
    list.iter()
        .enumerate()
        .flat_map(|(i, a)| a.handles().into_iter().enumerate().map(move |(h, q)| (i, h, dist(p, q))))
        .filter(|(_, _, d)| *d <= tol)
        .min_by(|x, y| x.2.total_cmp(&y.2))
        .map(|(i, h, _)| (i, h))
}

/// The assistant whose centre (move handle) is under `p`.
fn center_at(list: &[Assistant], p: [f64; 2], tol: f64) -> Option<usize> {
    list.iter().enumerate().filter(|(_, a)| dist(p, a.center()) <= tol).min_by(|x, y| dist(p, x.1.center()).total_cmp(&dist(p, y.1.center()))).map(|(i, _)| i)
}

/// The points a new assistant of `kind` takes from a drag `start → current`.
fn new_points(kind: &str, start: [f64; 2], current: [f64; 2]) -> Vec<[f64; 2]> {
    match kind {
        "vanishingPoint" => vec![current],
        "perspectiveGrid" => {
            let (x0, x1) = (start[0].min(current[0]), start[0].max(current[0]));
            let (y0, y1) = (start[1].min(current[1]), start[1].max(current[1]));
            vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
        }
        _ => vec![start, current],
    }
}

/// Pointer input for the Assistant tool. Returns true when consumed.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if app.active_tool() != Tool::Assistant {
        app.ui.assistants.drag = None;
        return false;
    }
    if app.session.active().is_none() {
        if matches!(ev, ToolEvent::Down { .. }) {
            app.ui.status = tl!("No document open").into();
            app.ui.status_error = false;
        }
        return true;
    }
    let list = assistants(app);
    let tol = tolerance(app);
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let p = [x, y];
            if let Some((index, handle)) = handle_at(&list, p, tol) {
                if mods.alt {
                    let _ = app.run("paint.assistants.remove", json!({"index": index}));
                } else {
                    app.ui.assistants.drag = Some(Drag::Handle { index, handle });
                }
            } else if let Some(index) = center_at(&list, p, tol) {
                if mods.alt {
                    let _ = app.run("paint.assistants.remove", json!({"index": index}));
                } else {
                    app.ui.assistants.drag = Some(Drag::Whole { index, from: p });
                }
            } else if app.ui.assistants.kind == "vanishingPoint" {
                let _ = app.run("paint.assistants.add", json!({"kind": "vanishingPoint", "points": [p]}));
            } else {
                app.ui.assistants.drag = Some(Drag::New { start: p, current: p });
            }
        }
        ToolEvent::Move { x, y, .. } => {
            let p = [x, y];
            match app.ui.assistants.drag {
                Some(Drag::Handle { index, handle }) => {
                    // The engine refuses a degenerate move; the handle then stays where it was.
                    let _ = app.run("paint.assistants.move", json!({"index": index, "handle": handle, "to": p}));
                }
                Some(Drag::Whole { index, from }) => {
                    if app.run("paint.assistants.move", json!({"index": index, "delta": [p[0] - from[0], p[1] - from[1]]})).is_ok() {
                        app.ui.assistants.drag = Some(Drag::Whole { index, from: p });
                    }
                }
                Some(Drag::New { start, .. }) => app.ui.assistants.drag = Some(Drag::New { start, current: p }),
                None => {}
            }
        }
        ToolEvent::Up { x, y } => {
            if let Some(Drag::New { start, .. }) = app.ui.assistants.drag.take() {
                let kind = app.ui.assistants.kind.clone();
                let end = [x, y];
                if dist(start, end) >= 1.0 {
                    let _ = app.run("paint.assistants.add", json!({"kind": kind, "points": new_points(&kind, start, end)}));
                }
            }
            app.ui.assistants.drag = None;
        }
    }
    true
}

/// Does a stroke with `tool` snap to the assistants?
fn snaps(tool: Tool) -> bool {
    matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser | Tool::MixerBrush | Tool::ColorReplacement | Tool::CloneStamp | Tool::PatternStamp | Tool::HistoryBrush)
}

/// Snap a painting tool's pointer event to the document's assistants. The press records the
/// stroke's start (and snaps it onto a ruler when that is the only assistant); once the stroke has
/// a direction, the assistant it fits best takes every later point.
pub fn filter_event(app: &mut PhotocraftApp, ev: ToolEvent) -> ToolEvent {
    let Some(st) = app.session.active() else { return ev };
    if !st.assistants_snap || st.assistants.is_empty() || !snaps(app.active_tool()) {
        app.ui.assistants.stroke = None;
        return ev;
    }
    let list = &st.assistants;
    match ev {
        ToolEvent::Down { x, y, pressure } => {
            let mut p = [x, y];
            let chosen = (list.len() == 1).then_some(0);
            if let Some(Assistant::Ruler { .. }) = chosen.map(|i| &list[i])
                && let Some(s) = list[0].snap(p, p)
            {
                p = s.point;
            }
            app.ui.assistants.stroke = Some(StrokeSnap { start: p, chosen });
            ToolEvent::Down { x: p[0], y: p[1], pressure }
        }
        ToolEvent::Move { x, y, pressure } => {
            let Some(mut s) = app.ui.assistants.stroke else { return ev };
            let p = [x, y];
            if s.chosen.is_none() {
                if dist(s.start, p) < DIRECTION_DISTANCE {
                    return ev;
                }
                s.chosen = assistants::choose(list, s.start, p);
                app.ui.assistants.stroke = Some(s);
            }
            match s.chosen.and_then(|i| list.get(i)).and_then(|a| a.snap(s.start, p)) {
                Some(q) => ToolEvent::Move { x: q.point[0], y: q.point[1], pressure },
                None => ev,
            }
        }
        ToolEvent::Up { x, y } => {
            let Some(s) = app.ui.assistants.stroke.take() else { return ev };
            match s.chosen.and_then(|i| list.get(i)).and_then(|a| a.snap(s.start, [x, y])) {
                Some(q) => ToolEvent::Up { x: q.point[0], y: q.point[1] },
                None => ev,
            }
        }
    }
}

/// The Assistant tool's options bar: kind, Snap, Show, Delete All, Save and Load.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::Assistant {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Assistant")).color(t.text_dim));
    let opts: Vec<(String, &str)> = Assistant::KINDS.iter().map(|k| (k.to_string(), kind_label(k))).collect();
    widgets::dropdown_literal(ui, "assistant-kind", &mut app.ui.assistants.kind, &opts, 150.0);
    widgets::vline(ui, 22.0);
    let mut snap = app.session.active().is_some_and(|st| st.assistants_snap);
    if widgets::checkbox(ui, &mut snap, tl!("Snap to Assistants")).changed() {
        let _ = app.run("paint.assistants.snap", json!({"enabled": snap}));
    }
    widgets::checkbox(ui, &mut app.ui.assistants.show, tl!("Show Assistants"));
    widgets::vline(ui, 22.0);
    let has = app.session.active().is_some_and(|st| !st.assistants.is_empty());
    if ui.add_enabled_ui(has, |ui| widgets::secondary_button(ui, tl!("Delete All"), 0.0)).inner.clicked() {
        let _ = app.run("paint.assistants.clear", json!({}));
    }
    if ui.add_enabled_ui(has, |ui| widgets::secondary_button(ui, tl!("Save…"), 0.0)).inner.clicked() {
        let r = app.pick_save("assistants.rassist", |app, path| app.run("paint.assistants.save", json!({"path": path})));
        if let Err(e) = r {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    if widgets::secondary_button(ui, tl!("Load…"), 0.0).clicked() {
        let r = app.pick_file_bytes_filtered(&["rassist"], |app, name, bytes| {
            let text = String::from_utf8(bytes).map_err(|_| format!("{name}: not a text file"))?;
            app.run("paint.assistants.load", json!({"path": name, "data": text}))
        });
        if let Err(e) = r {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    true
}

/// Draw the assistants: with the Assistant tool always (with handles), with the painting tools
/// when Show Assistants is on.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(st) = app.session.active() else { return };
    let tool = app.ui.tool;
    let editing = tool == Tool::Assistant;
    if !editing && !(app.ui.assistants.show && snaps(tool)) {
        return;
    }
    if st.assistants.is_empty() && app.ui.assistants.drag.is_none() {
        return;
    }
    let size = [f64::from(st.doc.size.width), f64::from(st.doc.size.height)];
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
    let screen = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let t = Tokens::get(painter.ctx());
    let faint = t.accent.gamma_multiply(if st.assistants_snap { 0.7 } else { 0.35 });
    for a in &st.assistants {
        for [p, q] in a.guides(size) {
            painter.line_segment([screen(p), screen(q)], Stroke::new(1.0, faint));
        }
        if editing {
            for h in a.handles() {
                crate::vector_ui::draw_anchor(&painter, screen(h), false, t.accent);
            }
            painter.circle_stroke(screen(a.center()), 5.0, Stroke::new(1.0, t.accent));
        }
    }
    if let Some(Drag::New { start, current }) = app.ui.assistants.drag
        && let Some(a) = Assistant::from_points(&app.ui.assistants.kind, &new_points(&app.ui.assistants.kind, start, current))
    {
        for [p, q] in a.guides(size) {
            painter.line_segment([screen(p), screen(q)], Stroke::new(1.0, t.accent));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_points_follow_the_kind() {
        assert_eq!(new_points("ruler", [1.0, 2.0], [3.0, 4.0]), vec![[1.0, 2.0], [3.0, 4.0]]);
        assert_eq!(new_points("vanishingPoint", [1.0, 2.0], [3.0, 4.0]), vec![[3.0, 4.0]]);
        assert_eq!(new_points("perspectiveGrid", [3.0, 4.0], [1.0, 2.0]), vec![[1.0, 2.0], [3.0, 2.0], [3.0, 4.0], [1.0, 4.0]]);
        let list = vec![Assistant::from_points("ruler", &[[0.0, 0.0], [10.0, 0.0]]).unwrap()];
        assert_eq!(handle_at(&list, [10.5, 0.2], 1.0), Some((0, 1)));
        assert_eq!(handle_at(&list, [50.0, 0.0], 1.0), None);
        assert_eq!(center_at(&list, [5.0, 0.5], 1.0), Some(0));
    }
}
