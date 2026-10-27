//! On-canvas knots of live effects with the node tool: a draggable knot sets the distance of an
//! Offset Path effect (dynamic offset), the distance of a linked offset object, or the radius of a
//! Gaussian Blur effect, live, through `effect.setParams` / `path.linkedOffset.set`. The canvas
//! draws the knots from the same geometry ([`EffectKnots::of`]).
//!
//! A linked offset object and a fill-between object keep their link in the object's own data
//! (the Attributes panel's `data-*` entries, [`LINKED_OFFSET`] and [`FILL_BETWEEN`]), so the
//! document format needs no new field and SVG export writes the link as `data-*` attributes.

use redrob_vector_doc::{Document, Node, NodeId, Selection};
use redrob_vector_geom::{Point, Rect};
use serde_json::{Value, json};

use crate::{Action, Mods, Overlay, PointerEvent, ToolContext};

/// Object data key of a linked offset object: `"<source id> <distance>"`.
pub const LINKED_OFFSET: &str = "vc-linked-offset";
/// Object data key of a fill-between object: the source ids, space-separated.
pub const FILL_BETWEEN: &str = "vc-fill-between";

/// The source object and distance of linked offset object `n`, if it is one.
pub fn linked_offset_of(n: &Node) -> Option<(NodeId, f64)> {
    let a = n.extras().attrs.as_deref()?;
    let v = a.data.iter().find(|(k, _)| k == LINKED_OFFSET).map(|(_, v)| v.as_str())?;
    let mut it = v.split_whitespace();
    let source = NodeId(it.next()?.parse().ok()?);
    let distance: f64 = it.next()?.parse().ok()?;
    distance.is_finite().then_some((source, distance))
}

/// The source objects of fill-between object `n`, if it is one.
pub fn fill_between_of(n: &Node) -> Option<Vec<NodeId>> {
    let a = n.extras().attrs.as_deref()?;
    let v = a.data.iter().find(|(k, _)| k == FILL_BETWEEN).map(|(_, v)| v.as_str())?;
    let ids: Vec<NodeId> = v.split_whitespace().filter_map(|s| s.parse().ok().map(NodeId)).collect();
    (!ids.is_empty()).then_some(ids)
}

/// Which value a knot stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnotKind {
    /// The `offset` of the Offset Path effect at this index of the object's effects.
    OffsetEffect(usize),
    /// The `radius` of the Gaussian Blur effect at this index of the object's effects.
    BlurEffect(usize),
    /// The distance of a linked offset object.
    LinkedOffset,
}

/// One knot on one object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knot {
    pub id: NodeId,
    pub kind: KnotKind,
    /// Its centre in the document.
    pub at: Point,
    /// The object's geometric bounds the knot is measured from.
    pub bounds: Rect,
    /// The value it shows.
    pub value: f64,
}

/// The effect knots of the selected objects.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectKnots {
    knots: Vec<Knot>,
}

/// An Offset Path knot sits on the right edge's middle, moved out by the offset; a blur knot on
/// the top-right corner, moved right by the radius; a linked offset's knot as an Offset Path's.
fn knots_of(n: &Node) -> Vec<Knot> {
    let Some(b) = n.geometric_bounds() else { return vec![] };
    let mut out = vec![];
    for (i, e) in n.appearance.effects.iter().enumerate() {
        if !e.visible {
            continue;
        }
        match e.id.as_str() {
            "path.offsetPath" => {
                let v = e.params.get("offset").and_then(Value::as_f64).unwrap_or(10.0);
                out.push(Knot { id: n.id, kind: KnotKind::OffsetEffect(i), at: Point::new(b.x1 + v, b.center().y), bounds: b, value: v });
            }
            "blur.gaussian" => {
                let v = e.params.get("radius").and_then(Value::as_f64).unwrap_or(5.0);
                out.push(Knot { id: n.id, kind: KnotKind::BlurEffect(i), at: Point::new(b.x1 + v, b.y0), bounds: b, value: v });
            }
            _ => {}
        }
    }
    if let Some((_, d)) = linked_offset_of(n) {
        // The linked object's own bounds already include the distance: the knot sits on its edge.
        out.push(Knot { id: n.id, kind: KnotKind::LinkedOffset, at: Point::new(b.x1, b.center().y), bounds: b, value: d });
    }
    out
}

impl EffectKnots {
    /// The knots of the selected editable objects, if any.
    pub fn of(doc: &Document, selection: &Selection) -> Option<Self> {
        let mut knots = vec![];
        for id in &selection.objects {
            if !doc.is_editable(*id) {
                continue;
            }
            if let Some(n) = doc.node(*id) {
                knots.extend(knots_of(n));
            }
        }
        (!knots.is_empty()).then_some(Self { knots })
    }

    /// Every knot, with the object it belongs to.
    pub fn visible_on(&self) -> impl Iterator<Item = &Knot> + '_ {
        self.knots.iter()
    }

    fn hit(&self, p: Point, tol: f64) -> Option<Knot> {
        self.knots.iter().filter(|k| k.at.distance(p) <= tol).min_by(|a, b| a.at.distance(p).total_cmp(&b.at.distance(p))).copied()
    }
}

/// Is `p` over an effect knot of the selection?
pub fn over_knot(cx: &ToolContext, p: Point) -> bool {
    EffectKnots::of(cx.doc, cx.selection).and_then(|k| k.hit(p, cx.tol(6.0))).is_some()
}

/// Dragging an effect knot: the value follows the pointer.
#[derive(Clone, Debug)]
pub struct KnotDrag {
    knot: Knot,
    start: Point,
    began: bool,
    at: Point,
    label: String,
}

impl KnotDrag {
    /// Start a drag when the press `ev` lands on a knot of the selection.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        let knot = EffectKnots::of(cx.doc, cx.selection)?.hit(ev.pos, cx.tol(6.0))?;
        Some(Self { knot, start: ev.pos, began: false, at: ev.pos, label: String::new() })
    }

    /// The command and params for the pointer at `p` (Shift: whole points).
    fn command(&mut self, cx: &ToolContext, p: Point, m: Mods) -> (String, Value) {
        let id = self.knot.id.0;
        let round = |v: f64| if m.shift { v.round() } else { (v * 100.0).round() / 100.0 };
        match self.knot.kind {
            KnotKind::OffsetEffect(i) => {
                let v = round(p.x - self.knot.bounds.x1);
                self.label = format!("Offset: {}", cx.len(v));
                ("effect.setParams".into(), json!({ "ids": [id], "index": i, "item": null, "params": { "offset": v } }))
            }
            KnotKind::BlurEffect(i) => {
                let v = round((p.x - self.knot.bounds.x1).max(0.0));
                self.label = format!("Radius: {}", cx.len(v));
                ("effect.setParams".into(), json!({ "ids": [id], "index": i, "item": null, "params": { "radius": v } }))
            }
            KnotKind::LinkedOffset => {
                // The knot sits on the offset object's edge: the pointer's move is the change.
                let v = round(self.knot.value + (p.x - self.knot.at.x));
                self.label = format!("Offset: {}", cx.len(v));
                ("path.linkedOffset.set".into(), json!({ "id": id, "distance": v }))
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
            out.push(Action::Begin("Effect Knot".into()));
        }
        self.at = p;
        let (cmd, params) = self.command(cx, p, m);
        out.push(Action::Preview(cmd, params));
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
