//! One lasso outline can alternate between freehand strokes and Alt-held straight segments:
//! with Alt held, each click adds a straight segment and a drag draws freehand. Releasing Alt
//! while the button is up closes the outline and makes the selection.
//! Selection intent is latched at the first press: Alt held then subtracts from an existing
//! selection; with no selection there is nothing to subtract, so it starts straight segments.
//! Alt pressed later changes drawing mode.
//!
//! The Freeform Pen draws the same way (Photoshop): a drag is freehand, Alt-clicks add straight
//! segments, releasing Alt with the button down goes on freehand and with it up finishes the
//! path (`vector_ui::freeform_finish`, which keeps the straight segments straight).
//!
//! Free select (the Lasso's options-bar toggle, `ToolOptions::lasso_free_select`): clicks and
//! drags build one outline without Alt. A click places a polygon vertex, a drag draws a freehand
//! stretch, and the outline stays open and editable until it is closed: a press on a vertex
//! handle drags it, a press on a segment inserts a vertex there (and drags it), Backspace removes
//! the last segment, Enter, a double-click or a click on the first point closes it, Esc cancels.
//! The vertices (segment ends, never the points inside a freehand stretch) are drawn as the Pen
//! tool's anchor squares.

use crate::{
    PhotocraftApp,
    canvas::{Drag, ToolEvent, ViewXform},
    state::Tool,
};
use egui::{Event, Modifiers, PointerButton};

#[derive(Clone, Debug)]
pub struct Lasso {
    pub cursor: [f64; 2],
    down: bool,
    polygonal: bool,
    document: Option<redrob_photo_doc::DocId>,
    /// Indices of the points an Alt-click added: the segment into each one is straight.
    straight: Vec<usize>,
    /// Free select: the outline stays open after a release and its vertices can be edited.
    free: bool,
    /// Free select: indices of the points that are vertices (handles): the start, each click,
    /// and the end of each freehand stretch. Sorted, no duplicates.
    vertices: Vec<usize>,
    /// Free select: the vertex a press grabbed; pointer moves move it until the release.
    grab: Option<usize>,
}

impl Lasso {
    /// Indices into the drag's points whose incoming segment is straight (an Alt-click).
    pub fn straight(&self) -> &[usize] {
        &self.straight
    }

    /// Free select: the outline is editable and shows its vertices as handles.
    pub fn free(&self) -> bool {
        self.free
    }

    /// Free select: indices of the points drawn as handles.
    pub fn vertices(&self) -> &[usize] {
        &self.vertices
    }

    /// The vertex being dragged, if any: the rubber band to the cursor is not drawn meanwhile.
    pub fn grabbed(&self) -> Option<usize> {
        self.grab
    }

    fn mark_vertex(&mut self, i: usize) {
        if let Err(at) = self.vertices.binary_search(&i) {
            self.vertices.insert(at, i);
        }
    }

    /// A point was inserted at `at`: every index from there on moves up by one.
    fn shift_from(&mut self, at: usize) {
        for i in self.straight.iter_mut().chain(self.vertices.iter_mut()) {
            if *i >= at {
                *i += 1;
            }
        }
    }
}

/// The tools this module draws with: the Lasso and the Freeform Pen.
pub fn owns(tool: Tool) -> bool {
    matches!(tool, Tool::Lasso | Tool::FreeformPen)
}

pub fn active(app: &PhotocraftApp) -> bool {
    app.drag.as_ref().is_some_and(|d| owns(d.tool) && d.lasso.is_some())
}

/// A free-select outline is open (and editable) on the active document.
pub fn free_select_active(app: &PhotocraftApp) -> bool {
    active(app) && app.drag.as_ref().is_some_and(|d| d.lasso.as_ref().is_some_and(|l| l.free))
}

/// Between polygonal clicks, Backspace/right-click retracts the last fixed vertex.
/// Do not affect a freehand stroke whose primary button is still held.
pub fn waiting_for_vertex(app: &PhotocraftApp) -> bool {
    owns(app.ui.tool)
        && app
            .drag
            .as_ref()
            .is_some_and(|d| d.tool == app.ui.tool && d.lasso.as_ref().is_some_and(|l| !l.down && l.document == app.session.active().map(|st| st.doc.id)))
}

pub fn undo_last_vertex(app: &mut PhotocraftApp) -> bool {
    cancel_stale(app);
    if !waiting_for_vertex(app) {
        return false;
    }
    if app.drag.as_ref().is_some_and(|d| d.points.len() <= 1) {
        app.drag = None;
    } else if let Some(d) = app.drag.as_mut() {
        let free = d.lasso.as_ref().is_some_and(|l| l.free);
        if free {
            // Free select: back to the previous vertex, dropping the whole last segment (a
            // freehand stretch is many points).
            let keep = d.lasso.as_ref().and_then(|l| l.vertices.iter().rev().find(|&&v| v + 1 < d.points.len()).copied()).unwrap_or(0);
            d.points.truncate(keep + 1);
        } else {
            d.points.pop();
        }
        let n = d.points.len();
        if let Some(lasso) = &mut d.lasso {
            lasso.straight.retain(|&i| i < n);
            lasso.vertices.retain(|&i| i < n);
            lasso.mark_vertex(n - 1);
        }
    }
    true
}

/// A lasso drag that moves the selection (started inside it, `canvas::selection_drag_kind`):
/// the canvas's own selection-move path handles it, this module only feeds it the events.
fn moving_selection(app: &PhotocraftApp) -> bool {
    app.drag.as_ref().is_some_and(|d| d.tool == Tool::Lasso && d.sel_move.is_some())
}

/// A drag the canvas handles rather than this module, which only feeds it the events: moving the
/// selection, a guide that ⌘ took (`canvas::command_guide_at`, #2690), the layer that ⌘ moves
/// (`canvas::command_moves_layer`, refused on a locked layer: `move_blocked`, #2725), or a path
/// that the Freeform Pen's ⌘ (Direct Selection) edits.
fn handed_over(app: &PhotocraftApp) -> bool {
    moving_selection(app)
        || app.guide_drag.is_some_and(|d| d.index.is_some())
        || app.drag.as_ref().is_some_and(|d| d.tool == Tool::Move)
        || app.move_blocked
        || app.ui.direct_selection.drag.is_some()
}

pub fn cancel_stale(app: &mut PhotocraftApp) {
    let document = app.session.active().map(|st| st.doc.id);
    if app.drag.as_ref().is_some_and(|d| d.lasso.as_ref().is_some_and(|lasso| app.ui.tool != d.tool || lasso.document != document)) {
        app.drag = None;
    }
}

fn append(d: &mut Drag, p: [f64; 2]) {
    if d.points.last().is_none_or(|last| last[0] != p[0] || last[1] != p[1]) {
        d.points.push([p[0], p[1], 1.0]);
    }
}

/// Free select: the vertex handle within `reach` of `p`, the nearest when several.
fn vertex_at(d: &Drag, p: [f64; 2], reach: f64) -> Option<usize> {
    let lasso = d.lasso.as_ref()?;
    lasso
        .vertices
        .iter()
        .copied()
        .filter(|&i| i < d.points.len())
        .map(|i| (i, (d.points[i][0] - p[0]).hypot(d.points[i][1] - p[1])))
        .filter(|&(_, dist)| dist < reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Free select: the outline segment (from point `i` to `i + 1`) within `reach` of `p`, with the
/// point of it nearest to `p`.
fn segment_at(d: &Drag, p: [f64; 2], reach: f64) -> Option<(usize, [f64; 2])> {
    let mut best: Option<(usize, [f64; 2], f64)> = None;
    for (i, w) in d.points.windows(2).enumerate() {
        let (a, b) = ([w[0][0], w[0][1]], [w[1][0], w[1][1]]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
        let q = [a[0] + dx * t, a[1] + dy * t];
        let dist = (q[0] - p[0]).hypot(q[1] - p[1]);
        if dist < reach && best.is_none_or(|(_, _, d0)| dist < d0) {
            best = Some((i, q, dist));
        }
    }
    best.map(|(i, q, _)| (i, q))
}

/// Free select: a press on the open outline grabs a vertex handle, or inserts one on a segment
/// and grabs it. False when the press is elsewhere (a new vertex, `pointer`).
fn grab_at(d: &mut Drag, p: [f64; 2], reach: f64) -> bool {
    if let Some(i) = vertex_at(d, p, reach) {
        if let Some(lasso) = &mut d.lasso {
            lasso.grab = Some(i);
            lasso.down = true;
            lasso.cursor = p;
        }
        return true;
    }
    if let Some((i, q)) = segment_at(d, p, reach) {
        d.points.insert(i + 1, [q[0], q[1], 1.0]);
        if let Some(lasso) = &mut d.lasso {
            lasso.shift_from(i + 1);
            lasso.mark_vertex(i + 1);
            lasso.grab = Some(i + 1);
            lasso.down = true;
            lasso.cursor = p;
        }
        return true;
    }
    false
}

fn modifiers(app: &mut PhotocraftApp, mods: Modifiers) {
    let Some(d) = app.drag.as_mut().filter(|d| owns(d.tool)) else { return };
    d.track(mods);
    // Free select: clicks are straight segments already, Alt only sets the intent at the press.
    if d.lasso.as_ref().is_some_and(|l| l.free) {
        return;
    }
    // Alt held before the initial press still means subtract, until released and pressed again.
    let polygonal = mods.alt && (!d.modifiers.alt || d.released.alt);
    // Alt released between straight segments (button up): the outline is done.
    let close = d.lasso.as_ref().is_some_and(|l| l.polygonal && !polygonal && !l.down);
    let anchor = d.lasso.as_ref().and_then(|l| (l.polygonal && !polygonal && l.down).then_some(l.cursor));
    if let Some(p) = anchor {
        append(d, p);
    }
    if let Some(lasso) = &mut d.lasso {
        lasso.polygonal = polygonal;
    }
    if close {
        commit(app);
    }
}

pub fn commit(app: &mut PhotocraftApp) {
    cancel_stale(app);
    if active(app)
        && let Some(d) = app.drag.take()
    {
        crate::canvas::finish_gesture(app, d);
    }
}

pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    cancel_stale(app);
    let tool = app.ui.tool;
    if !owns(tool) || handed_over(app) {
        return false;
    }
    let p = match ev {
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => [x, y],
    };
    // A press inside the selection (no outline in progress) drags the selection instead; a ⌘
    // press outside it moves the layer, as the Move tool (#2725).
    if matches!(ev, ToolEvent::Down { .. })
        && tool == Tool::Lasso
        && !active(app)
        && p.iter().all(|n| n.is_finite())
        && (crate::canvas::selection_drag_kind(app, Tool::Lasso, p, mods).is_some() || crate::canvas::command_moves_layer(app, Tool::Lasso, p, mods))
    {
        return false;
    }
    if !p.iter().all(|n| n.is_finite()) {
        return true;
    }
    modifiers(app, mods);
    match ev {
        ToolEvent::Down { .. } => {
            if active(app) {
                let free = app.drag.as_ref().is_some_and(|d| d.lasso.as_ref().is_some_and(|l| l.free));
                // A click on the first point, or on the last one again, closes the outline. In
                // free select only the first point closes: the last one is a handle to drag.
                let reach = 8.0 / app.point_zoom().max(0.01) as f64;
                let near = |q: [f64; 2]| (q[0] - p[0]).hypot(q[1] - p[1]) < reach;
                let close =
                    app.drag.as_ref().is_some_and(|d| d.points.len() >= 3 && (near(d.start) || (!free && d.points.last().is_some_and(|l| near([l[0], l[1]])))));
                if close {
                    // A path closes on its start point (a selection outline always closes).
                    if let Some(d) = app.drag.as_mut().filter(|d| d.tool == Tool::FreeformPen && near(d.start)) {
                        let (start, n) = (d.start, d.points.len());
                        append(d, start);
                        if let Some(lasso) = d.lasso.as_mut().filter(|_| d.points.len() > n) {
                            lasso.straight.push(n);
                        }
                    }
                    commit(app);
                    return true;
                }
                // Free select: a press on a vertex drags it, on a segment adds a vertex there.
                if free && app.drag.as_mut().is_some_and(|d| grab_at(d, p, reach)) {
                    return true;
                }
                if let Some(d) = &mut app.drag {
                    let n = d.points.len();
                    append(d, p);
                    let added = d.points.len() > n;
                    if let Some(lasso) = &mut d.lasso {
                        lasso.cursor = p;
                        lasso.down = true;
                        lasso.grab = None;
                        if added {
                            lasso.straight.push(n);
                            lasso.mark_vertex(n);
                        }
                    }
                }
            } else {
                // Alt with no selection has nothing to subtract from: the outline starts with
                // straight segments, as a new selection.
                // A path has nothing to subtract from either.
                let nothing_selected = tool == Tool::FreeformPen || app.session.active().is_none_or(|st| st.doc.selection.is_none());
                let free = tool == Tool::Lasso && app.ui.tool_options.lasso_free_select;
                let polygonal = mods.alt && nothing_selected && !free;
                let intent = if mods.alt && nothing_selected { Modifiers { alt: false, ..mods } } else { mods };
                let mut d = Drag::new(tool, p, vec![[p[0], p[1], 1.0]], intent, false);
                let document = app.session.active().map(|st| st.doc.id);
                d.lasso = Some(Lasso { cursor: p, down: true, polygonal, document, straight: Vec::new(), free, vertices: vec![0], grab: None });
                app.drag = Some(d);
            }
        }
        ToolEvent::Move { .. } | ToolEvent::Up { .. } => {
            let up = matches!(ev, ToolEvent::Up { .. });
            let Some(d) = app.drag.as_mut().filter(|d| d.tool == tool) else { return true };
            let Some(lasso) = &d.lasso else { return true };
            let (down, polygonal, previous, free, grab) = (lasso.down, lasso.polygonal, lasso.cursor, lasso.free, lasso.grab);
            if d.reposition {
                // Move relative to the rubber-band endpoint, not the last fixed vertex.
                let last = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
                d.shift_to([last[0] + p[0] - previous[0], last[1] + p[1] - previous[1]]);
            } else if let Some(i) = grab.filter(|_| down) {
                // Free select: the grabbed vertex follows the pointer.
                if let Some(q) = d.points.get_mut(i) {
                    q[0] = p[0];
                    q[1] = p[1];
                }
                if i == 0 {
                    d.start = p;
                }
            } else if down {
                // While the button is down the outline follows the pointer (freehand), Alt or
                // not; between Alt clicks only the rubber band moves.
                append(d, p);
            }
            let n = d.points.len();
            if let Some(lasso) = &mut d.lasso {
                lasso.cursor = p;
                if up {
                    lasso.down = false;
                    lasso.grab = None;
                    // Free select: the end of a freehand stretch (or the click) is a vertex.
                    if free && down && n > 0 {
                        lasso.mark_vertex(n - 1);
                    }
                }
            }
            if up && down && !polygonal && !free {
                commit(app);
            }
        }
    }
    true
}

/// Read actual presses/releases, including clicks below egui's drag threshold. Hover moves
/// update only the rubber band; they must never become freehand vertices between Alt clicks.
pub fn canvas_input(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, response: &egui::Response) {
    let events = ctx.input(|i| i.events.clone());
    let mut mods = app.drag.as_ref().map_or(ctx.input(|i| i.modifiers), |d| d.live);
    for ev in events {
        let tool_ev = match ev {
            Event::ModifiersChanged(m) => {
                mods = m;
                modifiers(app, crate::workspace_ui::sticky_mods(app, m));
                None
            }
            Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: m } => {
                mods = m;
                let p = xf.to_doc(pos);
                if pressed && response.contains_pointer() && response.rect.contains(pos) {
                    Some(ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 })
                } else if !pressed && (active(app) || handed_over(app)) {
                    Some(ToolEvent::Up { x: p[0], y: p[1] })
                } else {
                    None
                }
            }
            Event::PointerMoved(pos) if active(app) || handed_over(app) => {
                let p = xf.to_doc(pos);
                Some(ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 })
            }
            Event::WindowFocused(false) => {
                if active(app) || moving_selection(app) {
                    app.drag = None;
                }
                None
            }
            _ => None,
        };
        if let Some(ev) = tool_ev {
            crate::canvas::tool_event(app, ev, mods);
        }
    }
    if response.double_clicked() {
        commit(app);
    }
}

#[cfg(test)]
mod tests;
