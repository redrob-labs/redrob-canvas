//! Connector tool: drag from one object to another to join them with a connector
//! (`connector.create`), a stroked path that stays attached to both as they move and routes around
//! the objects marked as obstacles. Shift-click marks an object as an obstacle or unmarks it
//! (`connector.setObstacle`); a double-click on a connector reroutes it. The control bar sets the
//! routing (polyline or orthogonal), the corner curvature and the spacing kept from obstacles, the
//! tool's `routing`, `curvature` and `spacing` options.

use redrob_vector_doc::NodeId;
use redrob_vector_geom::Point;
use serde_json::{Value, json};

use crate::draw2::FEEDBACK;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub struct ConnectorTool {
    /// The object the drag started on, and where.
    from: Option<(NodeId, Point)>,
    at: Point,
    /// The object under the pointer while dragging.
    over: Option<NodeId>,
    pub orthogonal: bool,
    /// 0..100.
    pub curvature: f64,
    pub spacing: f64,
}

impl Default for ConnectorTool {
    fn default() -> Self {
        Self { from: None, at: Point::ZERO, over: None, orthogonal: true, curvature: 0.0, spacing: 10.0 }
    }
}

/// The top-level object under `p` that a connector can join (not a connector itself).
fn object_at(cx: &ToolContext, p: Point) -> Option<NodeId> {
    let h = redrob_vector_doc::hit::hit_test(cx.doc, p, cx.hit_options())?;
    let id = h.top_object(cx.isolation);
    let n = cx.doc.node(id)?;
    let is_connector = n.extras().attrs.as_deref().is_some_and(|a| a.data.iter().any(|(k, _)| k == "vc-connector"));
    (!is_connector).then_some(id)
}

/// The connector under `p`, if any.
fn connector_at(cx: &ToolContext, p: Point) -> Option<NodeId> {
    let h = redrob_vector_doc::hit::hit_test(cx.doc, p, cx.hit_options())?;
    let id = h.top_object(cx.isolation);
    let n = cx.doc.node(id)?;
    n.extras().attrs.as_deref().is_some_and(|a| a.data.iter().any(|(k, _)| k == "vc-connector")).then_some(id)
}

impl Tool for ConnectorTool {
    fn id(&self) -> &'static str {
        "connector"
    }

    fn busy(&self) -> bool {
        self.from.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        self.at = p;
        match ev.kind {
            PointerKind::Move => {
                self.over = object_at(cx, p);
                vec![]
            }
            PointerKind::Down => {
                if ev.mods.shift {
                    // Shift-click: the object becomes an obstacle, or stops being one.
                    if let Some(id) = object_at(cx, p) {
                        let on = cx.doc.node(id).is_some_and(|n| {
                            !n.extras().attrs.as_deref().is_some_and(|a| a.data.iter().any(|(k, v)| k == "vc-obstacle" && v != "0"))
                        });
                        return vec![Action::Exec("connector.setObstacle".into(), json!({ "ids": [id.0], "obstacle": on }))];
                    }
                    return vec![];
                }
                if ev.mods.alt
                    && let Some(id) = connector_at(cx, p)
                {
                    return vec![Action::Exec("connector.reroute".into(), json!({ "ids": [id.0] }))];
                }
                self.from = object_at(cx, p).map(|id| (id, p));
                vec![]
            }
            PointerKind::Drag => {
                self.over = object_at(cx, p);
                vec![]
            }
            PointerKind::Up => {
                let Some((from, _)) = self.from.take() else { return vec![] };
                let Some(to) = object_at(cx, p).filter(|to| *to != from) else { return vec![] };
                vec![Action::Exec(
                    "connector.create".into(),
                    json!({
                        "from": from.0,
                        "to": to.0,
                        "routing": if self.orthogonal { "orthogonal" } else { "polyline" },
                        "curvature": self.curvature,
                        "spacing": self.spacing,
                    }),
                )]
            }
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        if matches!(key, ToolKey::Escape) {
            self.from = None;
        }
        vec![]
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = vec![];
        // The object under the pointer (or the start) is outlined by its bounds.
        for id in self.over.into_iter().chain(self.from.map(|(id, _)| id)) {
            if let Some(b) = cx.doc.bounds_of(&[id], true) {
                o.push(Overlay::Highlight {
                    quad: [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)],
                    color: [FEEDBACK[0], FEEDBACK[1], FEEDBACK[2], 48],
                });
            }
        }
        if let Some((from, _)) = self.from
            && let Some(b) = cx.doc.bounds_of(&[from], true)
        {
            o.push(Overlay::Line { a: b.center(), b: self.at, color: FEEDBACK, dashed: true });
        }
        o
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }

    fn options(&self) -> Value {
        json!({ "routing": if self.orthogonal { "orthogonal" } else { "polyline" }, "curvature": self.curvature, "spacing": self.spacing })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "routing" => {
                if let Some(r) = v.as_str() {
                    self.orthogonal = r != "polyline";
                }
            }
            "curvature" => self.curvature = v.as_f64().unwrap_or(self.curvature).clamp(0.0, 100.0),
            "spacing" => self.spacing = v.as_f64().unwrap_or(self.spacing).clamp(0.0, 1000.0),
            _ => {}
        }
    }
}
