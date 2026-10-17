//! Live shape handles with Direct Selection: the parameters a live shape keeps show as handles
//! on the shape itself, which a drag edits through `object.setLiveShape` (the shape stays live).
//! An ellipse shows where its arc starts and ends (both at 3 o'clock on a whole ellipse): dragging
//! one sets that angle, inside the ellipse as a pie, outside as an open arc; Shift keeps to 15°
//! steps. A star shows its first tip and the notch after it: the tip sets the outer radius and
//! turns the star, the notch the inner radius (the spoke ratio); with Shift the drag rounds the
//! corners, with Alt it randomises them. A spiral shows its outer end (radius and turn) and its
//! inner end (how fast it winds in). The canvas draws the handles from the same geometry.

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
    /// A star's first tip: its outer radius and its turn.
    StarTip,
    /// The notch after a star's first tip: its inner radius.
    StarNotch,
    /// A spiral's outer end: its radius and its turn.
    SpiralOuter,
    /// A spiral's inner end: its decay.
    SpiralInner,
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
        LiveShape::Star { .. } => match live.star_handles() {
            Some((tip, notch)) => {
                vec![ShapeHandle { id, kind: HandleKind::StarTip, at: tip }, ShapeHandle { id, kind: HandleKind::StarNotch, at: notch }]
            }
            None => vec![],
        },
        LiveShape::Spiral { .. } => match live.spiral_handles() {
            Some((outer, inner)) => {
                vec![ShapeHandle { id, kind: HandleKind::SpiralOuter, at: outer }, ShapeHandle { id, kind: HandleKind::SpiralInner, at: inner }]
            }
            None => vec![],
        },
        _ => vec![],
    }
}

impl ShapeHandles {
    /// The handles of the selected live ellipses, stars and spirals that can be edited, if any.
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
    fn params(&mut self, cx: &ToolContext, p: Point, m: Mods) -> Option<Value> {
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
            HandleKind::StarTip | HandleKind::StarNotch => {
                let (r, turn) = self.live.turned_polar(p)?;
                let radius = self.live.star_radius()?.max(1e-9);
                // Shift rounds the corners, Alt randomises them: how far the pointer went, as a
                // share of the radius.
                let share = ((p - self.start).hypot() / radius).clamp(0.0, 1.0);
                if m.shift {
                    self.label = format!("Rounded: {:.0} %", share * 100.0);
                    return Some(json!({ "id": id, "rounding": share }));
                }
                if m.alt {
                    self.label = format!("Randomized: {:.0} %", share * 100.0);
                    return Some(json!({ "id": id, "randomized": share }));
                }
                if self.handle.kind == HandleKind::StarTip {
                    let turn = if m.cmd { snap_angle(turn, 15.0) } else { turn };
                    self.label = format!("Radius: {}\nAngle: {turn:.1}°", cx.len(r));
                    Some(json!({ "id": id, "starRadius": r, "starAngle": turn }))
                } else {
                    let ratio = (r / radius).clamp(0.01, 1.0);
                    self.label = format!("Spoke Ratio: {:.0} %", ratio * 100.0);
                    Some(json!({ "id": id, "spokeRatio": ratio }))
                }
            }
            HandleKind::SpiralOuter | HandleKind::SpiralInner => {
                let (r, turn) = self.live.turned_polar(p)?;
                if self.handle.kind == HandleKind::SpiralOuter {
                    let turn = if m.cmd { snap_angle(turn, 15.0) } else { turn };
                    self.label = format!("Radius: {}\nAngle: {turn:.1}°", cx.len(r));
                    Some(json!({ "id": id, "spiralRadius": r, "spiralAngle": turn }))
                } else {
                    // The inner end's distance from the centre, as a share of the radius, sets how
                    // fast the spiral winds in.
                    let outer = self.live.spiral_radius()?.max(1e-9);
                    let decay = self.live.spiral_decay_for_inner((r / outer).clamp(0.001, 0.999))?;
                    self.label = format!("Decay: {decay:.1} %");
                    Some(json!({ "id": id, "spiralDecay": decay }))
                }
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
