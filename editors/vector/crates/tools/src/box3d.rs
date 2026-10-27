//! 3D Box tool: drag out the front face of a box drawn in the document's perspective
//! (`box3d.create`): six faces, each its own path, in a group. The three vanishing points of the
//! perspective grid show as draggable targets while the tool is active (X toward the right one, Z
//! toward the left one, Y toward the vertical one of a 3-point grid): dragging one moves it
//! (`perspective.grid.set`), and every box in the document shares the perspective. The control bar
//! sets each axis to the vanishing point (finite) or to parallel edges at an angle (infinite), and
//! the depth; the selected box takes those changes (`box3d.set`).

use redrob_vector_geom::{Point, Rect};
use serde_json::{Value, json};

use crate::distort::perspective::PerspectiveGrid;
use crate::draw2::FEEDBACK;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// What the tool is dragging.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    /// The front face, from the press.
    Face(Point),
    /// A vanishing point: 'x' (right), 'z' (left) or 'y' (vertical).
    Vp(char),
}

pub struct Box3dTool {
    drag: Option<Drag>,
    at: Point,
    began: bool,
    pub depth: f64,
    /// (finite, angle) per axis.
    pub x: (bool, f64),
    pub y: (bool, f64),
    pub z: (bool, f64),
}

impl Default for Box3dTool {
    fn default() -> Self {
        Self { drag: None, at: Point::ZERO, began: false, depth: 0.4, x: (true, 0.0), y: (false, 90.0), z: (true, 30.0) }
    }
}

impl Box3dTool {
    /// The grid's vanishing points as (axis, point).
    fn vps(g: &PerspectiveGrid) -> Vec<(char, Point)> {
        let mut v = vec![('x', Point::new(g.vp_right, g.horizon)), ('z', Point::new(g.vp_left, g.horizon))];
        if g.kind >= 3 {
            v.push(('y', Point::new(g.vp_vertical[0], g.vp_vertical[1])));
        }
        v
    }

    fn axes_json(&self) -> Value {
        let ax = |(finite, angle): (bool, f64)| json!({ "finite": finite, "angle": angle });
        json!({ "x": ax(self.x), "y": ax(self.y), "z": ax(self.z) })
    }

    fn create_params(&self, a: Point, c: Point) -> Value {
        let mut v = json!({ "x0": a.x, "y0": a.y, "x1": c.x, "y1": c.y, "depth": self.depth });
        for (k, val) in self.axes_json().as_object().into_iter().flatten() {
            v[k.as_str()] = val.clone();
        }
        v
    }
}

impl Tool for Box3dTool {
    fn id(&self) -> &'static str {
        "box3d"
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        self.at = p;
        match ev.kind {
            PointerKind::Down => {
                let g = PerspectiveGrid::effective(cx.doc);
                let tol = cx.tol(8.0);
                if let Some((axis, _)) = Self::vps(&g).into_iter().find(|(_, q)| q.distance(p) <= tol) {
                    self.drag = Some(Drag::Vp(axis));
                    return vec![Action::Begin("Move Vanishing Point".into())];
                }
                self.drag = Some(Drag::Face(p));
                self.began = false;
                vec![]
            }
            PointerKind::Drag => match self.drag {
                Some(Drag::Vp(axis)) => {
                    let params = match axis {
                        'x' => json!({ "vpRight": p.x, "horizon": p.y }),
                        'z' => json!({ "vpLeft": p.x, "horizon": p.y }),
                        _ => json!({ "vpVertical": [p.x, p.y] }),
                    };
                    vec![Action::Preview("perspective.grid.set".into(), params)]
                }
                Some(Drag::Face(start)) => {
                    if !self.began && start.distance(p) >= cx.tol(3.0) {
                        self.began = true;
                        return vec![Action::Begin("3D Box".into()), Action::Preview("box3d.create".into(), self.create_params(start, p))];
                    }
                    if self.began { vec![Action::Preview("box3d.create".into(), self.create_params(start, p))] } else { vec![] }
                }
                None => vec![],
            },
            PointerKind::Up => {
                let drag = self.drag.take();
                let began = std::mem::take(&mut self.began);
                match drag {
                    Some(Drag::Vp(_)) => vec![Action::Commit],
                    Some(Drag::Face(start)) if began => {
                        vec![Action::Preview("box3d.create".into(), self.create_params(start, p)), Action::Commit]
                    }
                    Some(Drag::Face(_)) => {
                        // A click: a box of a default size at the point (the options dialog's job
                        // elsewhere; here the box is drawn square).
                        let side = 100.0;
                        vec![Action::Exec("box3d.create".into(), self.create_params(p, Point::new(p.x + side, p.y + side)))]
                    }
                    None => vec![],
                }
            }
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        if matches!(key, ToolKey::Escape) && self.drag.take().is_some() {
            let began = std::mem::take(&mut self.began);
            return if began { vec![Action::Cancel] } else { vec![] };
        }
        vec![]
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let g = PerspectiveGrid::effective(cx.doc);
        let mut o = vec![];
        // The vanishing points as targets, labelled by axis.
        for (axis, q) in Self::vps(&g) {
            o.push(Overlay::Anchor { p: q, color: FEEDBACK, filled: true, size: 7.0 });
            o.push(Overlay::Label {
                p: q + redrob_vector_geom::Vec2::new(8.0 / cx.zoom.max(1e-6), -8.0 / cx.zoom.max(1e-6)),
                text: axis.to_uppercase().to_string(),
                color: FEEDBACK,
            });
        }
        if let Some(Drag::Face(start)) = self.drag
            && self.began
        {
            let r = Rect::from_points(start, self.at);
            o.push(Overlay::Marquee(r));
        }
        o
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }

    fn options(&self) -> Value {
        json!({
            "depth": self.depth,
            "xFinite": self.x.0, "xAngle": self.x.1,
            "yFinite": self.y.0, "yAngle": self.y.1,
            "zFinite": self.z.0, "zAngle": self.z.1,
        })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        let b = |cur: bool| v.as_bool().unwrap_or(cur);
        let f = |cur: f64| v.as_f64().unwrap_or(cur);
        match key {
            "depth" => self.depth = f(self.depth).clamp(0.0, 1.0),
            "xFinite" => self.x.0 = b(self.x.0),
            "xAngle" => self.x.1 = f(self.x.1),
            "yFinite" => self.y.0 = b(self.y.0),
            "yAngle" => self.y.1 = f(self.y.1),
            "zFinite" => self.z.0 = b(self.z.0),
            "zAngle" => self.z.1 = f(self.z.1),
            _ => {}
        }
    }
}
