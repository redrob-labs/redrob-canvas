//! Painting assistants on the canvas: the Assistant tool places, moves and deletes rulers,
//! infinite rulers, parallel rulers, vanishing points, perspective grids, ellipses, splines,
//! 2-point and curvilinear perspectives (`paint.assistants.*`); the painting tools' strokes snap
//! to them ([`filter_event`]) within the brush's assistant magnetism. The assistants and their
//! handles are drawn over the document while the tool is active or View › Show Painting
//! Assistants is on, each in its own (or the global) colour and opacity, clipped to the area it
//! is limited to; the snapped stroke's preview line shows while painting when View › Show
//! Assistant Previews is on.

use egui::Stroke;
use redrob_photo_paint::assistants::{self, Assistant, AssistantStyle};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
    /// Dragging corner `corner` (0 or 1) of assistant `index`'s limit area.
    Limit { index: usize, corner: usize },
    /// Drawing a new assistant of the options bar's kind from `start`.
    New { start: [f64; 2], current: [f64; 2] },
}

/// The stroke being snapped: where it began and, once its direction is known, which assistant
/// it follows, plus the last snapped point (for the preview line).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeSnap {
    pub start: [f64; 2],
    pub chosen: Option<usize>,
    pub last: Option<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantUi {
    /// The kind the Assistant tool places next (`Assistant::KINDS`).
    pub kind: String,
    /// Draw the assistants with the painting tools too, not only with the Assistant tool
    /// (mirrors the document's View › Show Painting Assistants once one is open).
    pub show: bool,
    /// The assistant the options bar edits (the last one clicked with the Assistant tool).
    #[serde(skip)]
    pub selected: Option<usize>,
    #[serde(skip)]
    pub drag: Option<Drag>,
    #[serde(skip)]
    pub stroke: Option<StrokeSnap>,
}

impl Default for AssistantUi {
    fn default() -> Self {
        Self { kind: "ruler".into(), show: true, selected: None, drag: None, stroke: None }
    }
}

/// How far a stroke must travel before its direction picks an assistant (document pixels).
const DIRECTION_DISTANCE: f64 = 3.0;
/// The magnetism slider's range: how far (document pixels) a stroke may sit from an assistant
/// and still be pulled onto it; the top value snaps always.
pub const MAX_MAGNETISM: f32 = 1000.0;

fn tolerance(app: &PhotocraftApp) -> f64 {
    8.0 / f64::from(app.point_zoom().max(0.01))
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

pub(crate) fn kind_label(kind: &str) -> &'static str {
    match kind {
        "infiniteRuler" => tl!("Infinite Ruler"),
        "parallelRuler" => tl!("Parallel Ruler"),
        "vanishingPoint" => tl!("Vanishing Point"),
        "perspectiveGrid" => tl!("Perspective Grid"),
        "ellipse" => tl!("Ellipse"),
        "concentricEllipse" => tl!("Concentric Ellipse"),
        "perspectiveEllipse" => tl!("Perspective Ellipse"),
        "spline" => tl!("Spline"),
        "twoPointPerspective" => tl!("2-Point Perspective"),
        "fishEye" => tl!("Fish-Eye Point"),
        "curvilinear" => tl!("Curvilinear Perspective"),
        _ => tl!("Ruler"),
    }
}

fn assistants(app: &PhotocraftApp) -> Vec<Assistant> {
    app.session.active().map(|st| st.assistants.clone()).unwrap_or_default()
}

fn styles(app: &PhotocraftApp) -> Vec<AssistantStyle> {
    app.session.active().map(|st| (0..st.assistants.len()).map(|i| redrob_photo_engine::assistant_cmds::style_of(st, i)).collect()).unwrap_or_default()
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

/// The limit-area corner under `p`: (assistant, corner).
fn limit_corner_at(styles: &[AssistantStyle], p: [f64; 2], tol: f64) -> Option<(usize, usize)> {
    styles
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.limit.map(|l| (i, l)))
        .flat_map(|(i, [c0, c1])| [(i, 0usize, dist(p, c0)), (i, 1, dist(p, c1))])
        .filter(|(_, _, d)| *d <= tol)
        .min_by(|x, y| x.2.total_cmp(&y.2))
        .map(|(i, c, _)| (i, c))
}

/// The assistant whose centre (move handle) is under `p`.
fn center_at(list: &[Assistant], p: [f64; 2], tol: f64) -> Option<usize> {
    list.iter().enumerate().filter(|(_, a)| dist(p, a.center()) <= tol).min_by(|x, y| dist(p, x.1.center()).total_cmp(&dist(p, y.1.center()))).map(|(i, _)| i)
}

/// The points a new assistant of `kind` takes from a drag `start → current`.
fn new_points(kind: &str, start: [f64; 2], current: [f64; 2]) -> Vec<[f64; 2]> {
    let d = [current[0] - start[0], current[1] - start[1]];
    let len = d[0].hypot(d[1]).max(1e-9);
    // Perpendicular to the drag, a quarter of its length: the third handle of an ellipse-like
    // assistant, and the spline's control handles.
    let n = [-d[1] / len * len / 4.0, d[0] / len * len / 4.0];
    let m = [(start[0] + current[0]) / 2.0, (start[1] + current[1]) / 2.0];
    match kind {
        "vanishingPoint" => vec![current],
        "perspectiveGrid" | "perspectiveEllipse" => {
            let (x0, x1) = (start[0].min(current[0]), start[0].max(current[0]));
            let (y0, y1) = (start[1].min(current[1]), start[1].max(current[1]));
            vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
        }
        "ellipse" | "concentricEllipse" | "fishEye" | "twoPointPerspective" => vec![start, current, [m[0] + n[0], m[1] + n[1]]],
        "spline" => {
            let p1 = [start[0] + d[0] / 3.0 + n[0], start[1] + d[1] / 3.0 + n[1]];
            let p2 = [start[0] + d[0] * 2.0 / 3.0 + n[0], start[1] + d[1] * 2.0 / 3.0 + n[1]];
            vec![start, p1, p2, current]
        }
        _ => vec![start, current],
    }
}

fn report(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
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
    let st = styles(app);
    let tol = tolerance(app);
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let p = [x, y];
            if let Some((index, corner)) = limit_corner_at(&st, p, tol) {
                app.ui.assistants.selected = Some(index);
                app.ui.assistants.drag = Some(Drag::Limit { index, corner });
            } else if let Some((index, handle)) = handle_at(&list, p, tol) {
                if mods.alt && !matches!(list[index], Assistant::VanishingPoint { .. }) {
                    let _ = app.run("paint.assistants.remove", json!({"index": index}));
                    app.ui.assistants.selected = None;
                } else {
                    app.ui.assistants.selected = Some(index);
                    app.ui.assistants.drag = Some(Drag::Handle { index, handle });
                }
            } else if let Some(index) = center_at(&list, p, tol) {
                if mods.alt {
                    let _ = app.run("paint.assistants.remove", json!({"index": index}));
                    app.ui.assistants.selected = None;
                } else {
                    app.ui.assistants.selected = Some(index);
                    app.ui.assistants.drag = Some(Drag::Whole { index, from: p });
                }
            } else if app.ui.assistants.kind == "vanishingPoint" {
                let r = app.run("paint.assistants.add", json!({"kind": "vanishingPoint", "points": [p]}));
                if let Ok(v) = &r {
                    app.ui.assistants.selected = v.get("index").and_then(Value::as_u64).map(|i| i as usize);
                }
            } else {
                app.ui.assistants.drag = Some(Drag::New { start: p, current: p });
            }
        }
        ToolEvent::Move { x, y, .. } => {
            let mut p = [x, y];
            match app.ui.assistants.drag {
                Some(Drag::Handle { index, handle }) => {
                    // Alt keeps a dragged vanishing point on the horizon the other points define.
                    if mods.alt
                        && matches!(list.get(index), Some(Assistant::VanishingPoint { .. }))
                        && let Some(h) = assistants::horizon_for(&list, index, p)
                    {
                        p = h;
                    }
                    // The engine refuses a degenerate move; the handle then stays where it was.
                    let _ = app.run("paint.assistants.move", json!({"index": index, "handle": handle, "to": p}));
                }
                Some(Drag::Whole { index, from }) => {
                    if app.run("paint.assistants.move", json!({"index": index, "delta": [p[0] - from[0], p[1] - from[1]]})).is_ok() {
                        app.ui.assistants.drag = Some(Drag::Whole { index, from: p });
                    }
                }
                Some(Drag::Limit { index, corner }) => {
                    if let Some(mut limit) = st.get(index).and_then(|s| s.limit) {
                        limit[corner] = p;
                        let _ = app.run("paint.assistants.style", json!({"index": index, "limit": limit}));
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
                    let r = app.run("paint.assistants.add", json!({"kind": kind, "points": new_points(&kind, start, end)}));
                    if let Ok(v) = &r {
                        app.ui.assistants.selected = v.get("index").and_then(Value::as_u64).map(|i| i as usize);
                    }
                }
            }
            app.ui.assistants.drag = None;
        }
    }
    true
}

/// Does a stroke with `tool` snap to the assistants? The Eraser only when its own toggle says so.
fn snaps(app: &PhotocraftApp, tool: Tool) -> bool {
    match tool {
        Tool::Eraser => app.ui.tool_options.eraser_snaps_assistants,
        Tool::Brush | Tool::Pencil | Tool::MixerBrush | Tool::ColorReplacement | Tool::CloneStamp | Tool::PatternStamp | Tool::HistoryBrush => true,
        _ => false,
    }
}

/// Do the assistants draw with `tool` (besides the Assistant tool itself)?
fn draws_with(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Brush | Tool::Pencil | Tool::Eraser | Tool::MixerBrush | Tool::ColorReplacement | Tool::CloneStamp | Tool::PatternStamp | Tool::HistoryBrush
    )
}

/// Within magnetism: a snap that moves the point no farther than the slider allows (the top value
/// means always).
fn within_magnetism(app: &PhotocraftApp, moved: f64) -> bool {
    let m = app.ui.tool_options.assistant_magnetism;
    m >= MAX_MAGNETISM || moved <= f64::from(m.max(0.0))
}

/// Snap a painting tool's pointer event to the document's assistants. The press records the
/// stroke's start (and snaps it onto a ruler when that is the only assistant); once the stroke has
/// a direction, the assistant it fits best (among those whose limit area holds the start) takes
/// every later point, as long as the pull stays within the brush's assistant magnetism.
pub fn filter_event(app: &mut PhotocraftApp, ev: ToolEvent) -> ToolEvent {
    let Some(st) = app.session.active() else { return ev };
    if !st.assistants_snap || st.assistants.is_empty() || !snaps(app, app.active_tool()) {
        app.ui.assistants.stroke = None;
        return ev;
    }
    let list = &st.assistants;
    let admits = |i: usize, p: [f64; 2]| redrob_photo_engine::assistant_cmds::style_of(st, i).admits(p);
    match ev {
        ToolEvent::Down { x, y, pressure } => {
            let mut p = [x, y];
            let chosen = (list.len() == 1 && admits(0, p)).then_some(0);
            if let Some(Assistant::Ruler { .. } | Assistant::InfiniteRuler { .. }) = chosen.map(|i| &list[i])
                && let Some(s) = list[0].snap(p, p)
                && within_magnetism(app, s.distance)
            {
                p = s.point;
            }
            app.ui.assistants.stroke = Some(StrokeSnap { start: p, chosen, last: Some(p) });
            ToolEvent::Down { x: p[0], y: p[1], pressure }
        }
        ToolEvent::Move { x, y, pressure } => {
            let Some(mut s) = app.ui.assistants.stroke else { return ev };
            let p = [x, y];
            if s.chosen.is_none() {
                if dist(s.start, p) < DIRECTION_DISTANCE {
                    return ev;
                }
                let start = s.start;
                s.chosen = assistants::choose_where(list, start, p, |i| admits(i, start));
                app.ui.assistants.stroke = Some(s);
            }
            match s.chosen.and_then(|i| list.get(i)).and_then(|a| a.snap(s.start, p)).filter(|q| within_magnetism(app, q.distance)) {
                Some(q) => {
                    s.last = Some(q.point);
                    app.ui.assistants.stroke = Some(s);
                    ToolEvent::Move { x: q.point[0], y: q.point[1], pressure }
                }
                None => ev,
            }
        }
        ToolEvent::Up { x, y } => {
            let Some(s) = app.ui.assistants.stroke.take() else { return ev };
            match s.chosen.and_then(|i| list.get(i)).and_then(|a| a.snap(s.start, [x, y])).filter(|q| within_magnetism(app, q.distance)) {
                Some(q) => ToolEvent::Up { x: q.point[0], y: q.point[1] },
                None => ev,
            }
        }
    }
}

fn color32(c: [f32; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(
        (c[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn rgb3(c: egui::Color32) -> [f32; 3] {
    [f32::from(c.r()) / 255.0, f32::from(c.g()) / 255.0, f32::from(c.b()) / 255.0]
}

/// The Assistant tool's options bar: kind, Snap, Show, colour and opacity (global, and the
/// selected assistant's own), the selected ruler's subdivisions or 2-point settings, its limit
/// area, Delete All, Save and Load.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::Assistant {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Assistant")).color(t.text_dim));
    let opts: Vec<(String, &str)> = Assistant::KINDS.iter().map(|k| (k.to_string(), kind_label(k))).collect();
    widgets::dropdown_literal(ui, "assistant-kind", &mut app.ui.assistants.kind, &opts, 170.0);
    widgets::vline(ui, 22.0);
    let mut snap = app.session.active().is_some_and(|st| st.assistants_snap);
    if widgets::checkbox(ui, &mut snap, tl!("Snap to Assistants")).changed() {
        let _ = app.run("paint.assistants.snap", json!({"enabled": snap}));
    }
    let display = app.session.active().map(|st| st.assistant_display).unwrap_or_default();
    let mut show = display.show;
    if widgets::checkbox(ui, &mut show, tl!("Show Assistants")).changed() {
        app.ui.assistants.show = show;
        let _ = app.run("paint.assistants.display", json!({"show": show}));
    }
    let mut previews = display.show_previews;
    if widgets::checkbox(ui, &mut previews, tl!("Previews")).on_hover_text(tl!("Show the snapped stroke's line while painting")).changed() {
        let _ = app.run("paint.assistants.display", json!({"showPreviews": previews}));
    }
    widgets::vline(ui, 22.0);
    // Global colour and opacity.
    let mut c = color32(display.color);
    if widgets::color_edit_button_srgba(ui, &mut c).on_hover_text(tl!("Assistant colour")).changed() {
        let _ = app.run("paint.assistants.display", json!({"color": rgb3(c)}));
    }
    let mut op = display.opacity * 100.0;
    if widgets::value_field(ui, &mut op, 0.0..=100.0, "%", 58.0).on_hover_text(tl!("Assistant opacity")).changed() {
        let _ = app.run("paint.assistants.display", json!({"opacity": op / 100.0}));
    }
    // The selected assistant's own settings.
    let selected = app.ui.assistants.selected.filter(|i| app.session.active().is_some_and(|st| *i < st.assistants.len()));
    if let Some(i) = selected {
        widgets::vline(ui, 22.0);
        let (a, style) = {
            let st = app.session.active().expect("checked above");
            (st.assistants[i].clone(), redrob_photo_engine::assistant_cmds::style_of(st, i))
        };
        ui.label(egui::RichText::new(kind_label(a.kind())).color(t.text_dim));
        let mut own = style.color.is_some();
        if widgets::checkbox(ui, &mut own, tl!("Own colour")).changed() {
            let v = if own { json!(display.color) } else { Value::Null };
            let _ = app.run("paint.assistants.style", json!({"index": i, "color": v}));
        }
        if let Some(col) = style.color {
            let mut c = color32(col);
            if widgets::color_edit_button_srgba(ui, &mut c).changed() {
                let _ = app.run("paint.assistants.style", json!({"index": i, "color": rgb3(c)}));
            }
            let mut op = style.opacity.unwrap_or(display.opacity) * 100.0;
            if widgets::value_field(ui, &mut op, 0.0..=100.0, "%", 58.0).changed() {
                let _ = app.run("paint.assistants.style", json!({"index": i, "opacity": op / 100.0}));
            }
        }
        let mut handles = style.show_handles;
        if widgets::checkbox(ui, &mut handles, tl!("Handles")).on_hover_text(tl!("Show this assistant's control handles")).changed() {
            let _ = app.run("paint.assistants.style", json!({"index": i, "showHandles": handles}));
        }
        let mut limited = style.limit.is_some();
        if widgets::checkbox(ui, &mut limited, tl!("Limit to area")).on_hover_text(tl!("Two extra handles clip where the assistant draws and snaps")).changed()
        {
            let v = if limited {
                // Start from the handles' bounding box, grown by a fifth.
                let h = a.handles();
                let (x0, x1) = (h.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min), h.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max));
                let (y0, y1) = (h.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min), h.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max));
                let (mx, my) = (((x1 - x0) * 0.2).max(8.0), ((y1 - y0) * 0.2).max(8.0));
                json!([[x0 - mx, y0 - my], [x1 + mx, y1 + my]])
            } else {
                Value::Null
            };
            let _ = app.run("paint.assistants.style", json!({"index": i, "limit": v}));
        }
        if let Some((major, minor)) = a.subdivisions() {
            let mut maj = major as f32;
            if widgets::value_field(ui, &mut maj, 0.0..=64.0, "", 48.0).on_hover_text(tl!("Subdivisions")).changed() {
                let _ = app.run("paint.assistants.subdivisions", json!({"index": i, "subdivisions": maj.round() as u32}));
            }
            let mut min = minor as f32;
            if widgets::value_field(ui, &mut min, 0.0..=64.0, "", 48.0).on_hover_text(tl!("Minor subdivisions")).changed() {
                let _ = app.run("paint.assistants.subdivisions", json!({"index": i, "minor": min.round() as u32}));
            }
        }
        if let Assistant::TwoPointPerspective { density, vertical, .. } = a {
            let mut d = density as f32;
            if widgets::value_field(ui, &mut d, 0.25..=4.0, "", 52.0).on_hover_text(tl!("Density")).changed() {
                let _ = app.run("paint.assistants.twoPoint", json!({"index": i, "density": d}));
            }
            let mut v = vertical;
            if widgets::checkbox(ui, &mut v, tl!("Vertical ruler")).changed() {
                let _ = app.run("paint.assistants.twoPoint", json!({"index": i, "vertical": v}));
            }
        }
        if widgets::secondary_button(ui, tl!("Delete"), 0.0).clicked() {
            let _ = app.run("paint.assistants.remove", json!({"index": i}));
            app.ui.assistants.selected = None;
        }
    }
    widgets::vline(ui, 22.0);
    let has = app.session.active().is_some_and(|st| !st.assistants.is_empty());
    if ui.add_enabled_ui(has, |ui| widgets::secondary_button(ui, tl!("Delete All"), 0.0)).inner.clicked() {
        let _ = app.run("paint.assistants.clear", json!({}));
        app.ui.assistants.selected = None;
    }
    if ui.add_enabled_ui(has, |ui| widgets::secondary_button(ui, tl!("Save…"), 0.0)).inner.clicked() {
        let r = app.pick_save("assistants.rassist", |app, path| app.run("paint.assistants.save", json!({"path": path})));
        report(app, r);
    }
    if widgets::secondary_button(ui, tl!("Load…"), 0.0).clicked() {
        let r = app.pick_file_bytes_filtered(&["rassist"], |app, name, bytes| {
            let text = String::from_utf8(bytes).map_err(|_| format!("{name}: not a text file"))?;
            app.run("paint.assistants.load", json!({"path": name, "data": text}))
        });
        report(app, r);
    }
    true
}

/// The painting tools' assistant options: Snap to Assistants with its magnetism, and whether the
/// Eraser snaps too (shown with the Eraser).
pub fn brush_options(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) {
    if !draws_with(tool) || app.session.active().is_none() {
        return;
    }
    let mut snap = app.session.active().is_some_and(|st| st.assistants_snap);
    if widgets::checkbox(ui, &mut snap, tl!("Snap to Assistants")).on_hover_text(tl!("Strokes follow the painting assistants (Ctrl+Shift+L)")).changed() {
        let _ = app.run("paint.assistants.snap", json!({"enabled": snap}));
    }
    if snap {
        let o = &mut app.ui.tool_options;
        widgets::value_field(ui, &mut o.assistant_magnetism, 0.0..=MAX_MAGNETISM, "", 58.0)
            .on_hover_text(tl!("Assistant magnetism: how far a stroke may stray from an assistant and still be pulled onto it (1000 = always)"));
        if tool == Tool::Eraser {
            widgets::checkbox(ui, &mut o.eraser_snaps_assistants, tl!("Eraser snaps"));
        }
    }
}

/// Menu checkmarks for View › Show Painting Assistants / Assistant Previews.
pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let d = app.session.active().map(|st| st.assistant_display).unwrap_or_default();
    match id {
        "view.show.paintingAssistants" => Some(d.show),
        "view.show.assistantPreviews" => Some(d.show_previews),
        _ => None,
    }
}

/// Draw the assistants: with the Assistant tool always (with handles), with the painting tools
/// when Show Painting Assistants is on. Each in its own or the global colour and opacity, clipped
/// to its limit area; the snapped stroke's preview line when Show Assistant Previews is on.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(st) = app.session.active() else { return };
    let tool = app.ui.tool;
    let editing = tool == Tool::Assistant;
    let display = st.assistant_display;
    if !editing && !(display.show && draws_with(tool)) {
        return;
    }
    if st.assistants.is_empty() && app.ui.assistants.drag.is_none() {
        return;
    }
    let size = [f64::from(st.doc.size.width), f64::from(st.doc.size.height)];
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
    let screen = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let t = Tokens::get(painter.ctx());
    let dim = if st.assistants_snap { 1.0 } else { 0.5 };
    for (i, a) in st.assistants.iter().enumerate() {
        let style = redrob_photo_engine::assistant_cmds::style_of(st, i);
        let color = color32(style.color.unwrap_or(display.color)).gamma_multiply(style.opacity.unwrap_or(display.opacity) * dim);
        for seg in a.guides(size) {
            if let Some([p, q]) = style.clip(seg) {
                painter.line_segment([screen(p), screen(q)], Stroke::new(1.0, color));
            }
        }
        if let Some([x0, y0, x1, y1]) = style.limit_rect() {
            let r = egui::Rect::from_two_pos(screen([x0, y0]), screen([x1, y1]));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, color.gamma_multiply(0.6)), egui::StrokeKind::Outside);
        }
        if editing {
            let selected = app.ui.assistants.selected == Some(i);
            if style.show_handles || selected {
                for h in a.handles() {
                    crate::vector_ui::draw_anchor(&painter, screen(h), selected, t.accent);
                }
                painter.circle_stroke(screen(a.center()), 5.0, Stroke::new(1.0, t.accent));
            }
            if let Some([c0, c1]) = style.limit {
                for c in [c0, c1] {
                    let s = screen(c);
                    painter.rect_stroke(egui::Rect::from_center_size(s, egui::vec2(8.0, 8.0)), 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Outside);
                }
            }
        }
    }
    if let Some(Drag::New { start, current }) = app.ui.assistants.drag
        && let Some(a) = Assistant::from_points(&app.ui.assistants.kind, &new_points(&app.ui.assistants.kind, start, current))
    {
        for [p, q] in a.guides(size) {
            painter.line_segment([screen(p), screen(q)], Stroke::new(1.0, t.accent));
        }
    }
    // The stroke preview: the start to the last snapped point along the chosen assistant.
    if display.show_previews
        && !editing
        && let Some(s) = app.ui.assistants.stroke
        && let Some(last) = s.last
        && let Some(a) = s.chosen.and_then(|i| st.assistants.get(i))
    {
        let color = t.accent.gamma_multiply(0.9);
        if a.is_curved() {
            // Sample the curve between the two points.
            const N: usize = 24;
            let mut prev = s.start;
            for k in 1..=N {
                let f = k as f64 / N as f64;
                let raw = [s.start[0] + (last[0] - s.start[0]) * f, s.start[1] + (last[1] - s.start[1]) * f];
                let q = a.snap(s.start, raw).map_or(raw, |q| q.point);
                painter.line_segment([screen(prev), screen(q)], Stroke::new(1.5, color));
                prev = q;
            }
        } else {
            painter.line_segment([screen(s.start), screen(last)], Stroke::new(1.5, color));
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
        assert_eq!(new_points("perspectiveEllipse", [3.0, 4.0], [1.0, 2.0]), vec![[1.0, 2.0], [3.0, 2.0], [3.0, 4.0], [1.0, 4.0]]);
        let e = new_points("ellipse", [0.0, 0.0], [100.0, 0.0]);
        assert_eq!(e.len(), 3);
        assert!((e[2][0] - 50.0).abs() < 1e-9 && (e[2][1].abs() - 25.0).abs() < 1e-9, "{e:?}");
        assert!(Assistant::from_points("ellipse", &e).is_some());
        let sp = new_points("spline", [0.0, 0.0], [90.0, 0.0]);
        assert_eq!(sp.len(), 4);
        assert!(Assistant::from_points("spline", &sp).is_some());
        for kind in Assistant::KINDS {
            if kind == "vanishingPoint" {
                continue;
            }
            assert!(Assistant::from_points(kind, &new_points(kind, [10.0, 10.0], [110.0, 60.0])).is_some(), "{kind}");
        }
        let list = vec![Assistant::from_points("ruler", &[[0.0, 0.0], [10.0, 0.0]]).unwrap()];
        assert_eq!(handle_at(&list, [10.5, 0.2], 1.0), Some((0, 1)));
        assert_eq!(handle_at(&list, [50.0, 0.0], 1.0), None);
        assert_eq!(center_at(&list, [5.0, 0.5], 1.0), Some(0));
        let st = vec![AssistantStyle { limit: Some([[0.0, 0.0], [20.0, 20.0]]), ..Default::default() }];
        assert_eq!(limit_corner_at(&st, [19.5, 20.2], 1.0), Some((0, 1)));
        assert_eq!(limit_corner_at(&st, [10.0, 10.0], 1.0), None);
    }
}
