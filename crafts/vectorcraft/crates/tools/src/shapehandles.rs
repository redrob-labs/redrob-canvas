//! Live shape handles with Direct Selection: the parameters a live shape keeps show as handles
//! on the shape itself, which a drag edits through `object.setLiveShape` (the shape stays live).
//! An ellipse shows where its arc starts and ends (both at 3 o'clock on a whole ellipse): dragging
//! one sets that angle, inside the ellipse as a pie, outside as an open arc; Shift keeps to 15°
//! steps. The canvas draws the handles from the same geometry.

use redrob_vector_doc::{Document, LiveShape, NodeId, NodeKind, Selection};
use redrob_vector_geom::Point;
use redrob_vector_geom::shapes::ArcKind;
use serde_json::{Value, json};

use crate::{Action, Mods, Overlay, PointerEvent, ToolContext};

/// Which parameter a handle stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleKind {
    /// Where an ellipse's arc starts.
    ArcStart,
    /// Where an ellipse's arc ends.
    ArcEnd,
}

/// One handle on one live shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeHandle {
    pub id: NodeId,
    pub kind: HandleKind,
    /// Its centre in the document.
    pub at: Point,
}

/// The live shape handles of the selected shapes.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeHandles {
    handles: Vec<ShapeHandle>,
}

/// The handles of live shape `live` (object `id`), in hit order: on a whole ellipse, whose two
/// handles coincide, the end handle is picked first so a drag opens the arc from 3 o'clock.
fn handles_of(id: NodeId, live: &LiveShape) -> Vec<ShapeHandle> {
    match live {
        LiveShape::Ellipse { .. } => match live.arc_handles() {
            Some((start, end)) => {
                vec![ShapeHandle { id, kind: HandleKind::ArcEnd, at: end }, ShapeHandle { id, kind: HandleKind::ArcStart, at: start }]
            }
            None => vec![],
        },
        _ => vec![],
    }
}

impl ShapeHandles {
    /// The handles of the selected live ellipses that can be edited, if any.
    pub fn of(doc: &Document, selection: &Selection) -> Option<Self> {
        let mut handles = vec![];
        for id in &selection.objects {
            if !doc.is_editable(*id) {
                continue;
            }
            let Some(NodeKind::Path { live: Some(live), .. }) = doc.node(*id).map(|n| &n.kind) else { continue };
            handles.extend(handles_of(*id, live));
        }
        (!handles.is_empty()).then_some(Self { handles })
    }

    /// Every handle, with the object it belongs to.
    pub fn visible_on(&self) -> impl Iterator<Item = &ShapeHandle> + '_ {
        self.handles.iter()
    }

    /// The handle nearest to `p` within `tol` (document units).
    fn hit(&self, p: Point, tol: f64) -> Option<ShapeHandle> {
        self.handles.iter().filter(|h| h.at.distance(p) <= tol).min_by(|a, b| a.at.distance(p).total_cmp(&b.at.distance(p))).copied()
    }
}

/// Is `p` over a live shape handle of the selection?
pub fn over_handle(cx: &ToolContext, p: Point) -> bool {
    ShapeHandles::of(cx.doc, cx.selection).and_then(|h| h.hit(p, cx.tol(6.0))).is_some()
}

/// Round `a` (degrees) to the nearest `step`.
fn snap_angle(a: f64, step: f64) -> f64 {
    ((a / step).round() * step).rem_euclid(360.0)
}

/// Dragging a live shape handle: the shape's parameter follows the pointer.
#[derive(Clone, Debug)]
pub struct ShapeHandleDrag {
    handle: ShapeHandle,
    /// The shape as it was when the press landed.
    live: LiveShape,
    start: Point,
    began: bool,
    at: Point,
    /// The readout beside the pointer.
    label: String,
}

impl ShapeHandleDrag {
    /// Start a drag when the press `ev` lands on a handle of the selection.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        let handle = ShapeHandles::of(cx.doc, cx.selection)?.hit(ev.pos, cx.tol(6.0))?;
        let NodeKind::Path { live: Some(live), .. } = &cx.doc.node(handle.id)?.kind else { return None };
        Some(Self { handle, live: (**live).clone(), start: ev.pos, began: false, at: ev.pos, label: String::new() })
    }

    /// The drag has left the click: a parameter is changing.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    /// The `object.setLiveShape` params for the pointer at `p` with `m` held, and the readout.
    fn params(&mut self, _cx: &ToolContext, p: Point, m: Mods) -> Option<Value> {
        let id = self.handle.id.0;
        match self.handle.kind {
            HandleKind::ArcStart | HandleKind::ArcEnd => {
                let (mut a, inside) = self.live.arc_angle_toward(p)?;
                if m.shift {
                    a = snap_angle(a, 15.0);
                }
                let LiveShape::Ellipse { arc, .. } = &self.live else { return None };
                // Inside the ellipse the arc closes as a pie (a chord stays a chord), outside it is open.
                let kind = match (inside, arc) {
                    (true, ArcKind::Arc) => ArcKind::Pie,
                    (true, k) => *k,
                    (false, _) => ArcKind::Arc,
                };
                let key = if self.handle.kind == HandleKind::ArcStart { "pieStart" } else { "pieEnd" };
                self.label = format!("{}: {a:.1}°", if key == "pieStart" { "Start" } else { "End" });
                Some(json!({ "id": id, key: a, "arcKind": kind.name() }))
            }
        }
    }

    pub fn drag(&mut self, cx: &ToolContext, p: Point, m: Mods) -> Vec<Action> {
        let mut out = vec![];
        if !self.began {
            if p.distance(self.start) < cx.tol(3.0) {
                return out;
            }
            self.began = true;
            out.push(Action::Begin("Live Shape".into()));
        }
        self.at = p;
        if let Some(params) = self.params(cx, p, m) {
            out.push(Action::Preview("object.setLiveShape".into(), params));
        }
        out
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began { vec![Action::Commit] } else { vec![] }
    }

    /// The readout next to the pointer.
    pub fn overlays(&self) -> Vec<Overlay> {
        if !self.began || self.label.is_empty() {
            return vec![];
        }
        vec![Overlay::Measure { p: self.at, text: self.label.clone() }]
    }
}
