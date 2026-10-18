//! 3D Box (`box3d.create`, `box3d.set`): a box drawn in the document's perspective (the
//! perspective grid's vanishing points: X toward the right one, Z toward the left one, Y toward the
//! vertical one in 3-point perspective), each of its six faces a separately styled path inside a
//! group named "3D Box". The group's `vc-box3d` data keeps the corner points and axes, so the
//! box can be rebuilt (`box3d.set`) when its depth or axes change. Boxes share the document's
//! perspective; an axis can instead be infinite at an angle (parallel edges), per axis.

use redrob_vector_color::{Color, Paint};
use redrob_vector_doc::{Appearance, Node, NodeId, NodeKind};
use redrob_vector_geom::{FillRule, PathData, Point, SubPath, Vec2};
use redrob_vector_tools::distort::perspective::PerspectiveGrid;
use serde_json::{Value, json};

use super::create::add_node;
use super::distortcmds::grid_of;
use super::*;

pub const BOX_KEY: &str = "vc-box3d";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "box3d.create",
            "3D Box",
            [],
            None,
            "{x0, y0, x1, y1: the front face's dragged corners (document points), depth?: 0..1 (0.4; how far toward the Z vanishing point the back face lies, or the back face's offset as a share of the front's width for an infinite Z), x?, y?, z?: {finite?: bool (true: the document perspective grid's vanishing point: right for X, vertical for Y (3-point grids), left for Z; false: parallel edges), angle?: deg (an infinite axis's direction, counter-clockwise from +x)}} draw a box in perspective: six faces, each its own path (front, back, top, bottom, left, right) shaded from the current fill, in a group → {id, faces: [ids]}",
            has_doc,
            create
        ),
        cmd!(
            "box3d.set",
            "3D Box Options",
            [],
            None,
            "{id? (default: the selected box), depth?, x?, y?, z?: as box3d.create, x0?, y0?, x1?, y1?: new front corners} rebuild the box's faces with the new values (their styles stay) → {id}",
            has_selection,
            set
        ),
    ]
}

/// How one axis of the box recedes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    /// Toward a vanishing point of the document's perspective (true), or parallel at `angle`.
    pub finite: bool,
    /// Degrees, counter-clockwise from +x, for an infinite axis.
    pub angle: f64,
}

/// A box's record: the front corners, the depth and the axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box3d {
    pub a: Point,
    pub c: Point,
    pub depth: f64,
    pub x: Axis,
    pub y: Axis,
    pub z: Axis,
}

impl Box3d {
    fn data(&self) -> String {
        let ax = |a: Axis| format!("{}:{}", u8::from(a.finite), a.angle);
        format!("{},{},{},{},{},{},{},{}", self.a.x, self.a.y, self.c.x, self.c.y, self.depth, ax(self.x), ax(self.y), ax(self.z))
    }

    fn parse(s: &str) -> Option<Self> {
        let mut it = s.split(',');
        let mut num = || it.next()?.trim().parse::<f64>().ok();
        let a = Point::new(num()?, num()?);
        let c = Point::new(num()?, num()?);
        let depth = num()?;
        let mut axis = || -> Option<Axis> {
            let (f, ang) = it.next()?.split_once(':')?;
            Some(Axis { finite: f.trim() == "1", angle: ang.trim().parse().ok()? })
        };
        let x = axis()?;
        let y = axis()?;
        let z = axis()?;
        Some(Box3d { a, c, depth, x, y, z })
    }
}

/// The box record of `n`, if it is a 3D box group.
pub fn box_of(n: &Node) -> Option<Box3d> {
    n.extras().attrs.as_deref()?.data.iter().find(|(k, _)| k == BOX_KEY).and_then(|(_, v)| Box3d::parse(v))
}

/// Where the lines along axis `axis` from `p` go: a vanishing point, or a direction.
enum Toward {
    Point(Point),
    Dir(Vec2),
}

fn toward(axis: Axis, which: char, g: &PerspectiveGrid) -> Toward {
    if axis.finite {
        match which {
            'x' if g.kind >= 1 => return Toward::Point(Point::new(g.vp_right, g.horizon)),
            'z' if g.kind >= 1 => return Toward::Point(Point::new(g.vp_left, g.horizon)),
            'y' if g.kind >= 3 => return Toward::Point(Point::new(g.vp_vertical[0], g.vp_vertical[1])),
            _ => {}
        }
    }
    let default = match which {
        'x' => 0.0,
        'y' => 90.0,
        _ => 30.0,
    };
    let a = if axis.finite { default } else { axis.angle }.to_radians();
    // Document y grows down: a counter-clockwise angle on the page goes to −y.
    Toward::Dir(Vec2::new(a.cos(), -a.sin()))
}

/// The point on the line from `p` along `t` where it crosses `x = at` (`vertical`) or `y = at`.
fn along_to(p: Point, t: &Toward, at: f64, vertical: bool) -> Point {
    let d = match t {
        Toward::Point(v) => *v - p,
        Toward::Dir(d) => *d,
    };
    let k = if vertical { (at - p.x) / d.x } else { (at - p.y) / d.y };
    if !k.is_finite() {
        return if vertical { Point::new(at, p.y) } else { Point::new(p.x, at) };
    }
    p + d * k
}

/// The crossing of the line from `p` along `tp` with the line from `q` along `tq`.
fn cross(p: Point, tp: &Toward, q: Point, tq: &Toward) -> Point {
    let dp = match tp {
        Toward::Point(v) => *v - p,
        Toward::Dir(d) => *d,
    };
    let dq = match tq {
        Toward::Point(v) => *v - q,
        Toward::Dir(d) => *d,
    };
    let den = dp.cross(dq);
    if den.abs() < 1e-12 {
        return Point::new(q.x, p.y);
    }
    let k = (q - p).cross(dq) / den;
    p + dp * k
}

/// A point moved toward the Z target by the depth.
fn recede(p: Point, t: &Toward, depth: f64, width: f64) -> Point {
    match t {
        Toward::Point(v) => p + (*v - p) * depth.clamp(0.0, 0.95),
        Toward::Dir(d) => p + *d * (depth * width),
    }
}

/// The eight corners: front A B C D (clockwise from the top-left), back A' B' C' D'.
pub fn corners(b: &Box3d, g: &PerspectiveGrid) -> [Point; 8] {
    let (tx, ty, tz) = (toward(b.x, 'x', g), toward(b.y, 'y', g), toward(b.z, 'z', g));
    let a = b.a;
    // B lies along X from A at the far corner's x; D along Y at its y; C where their lines cross.
    let bp = along_to(a, &tx, b.c.x, true);
    let dp = along_to(a, &ty, b.c.y, false);
    let cp = cross(bp, &ty, dp, &tx);
    let width = (bp - a).hypot().max(1.0);
    let back = |p: Point| recede(p, &tz, b.depth, width);
    [a, bp, cp, dp, back(a), back(bp), back(cp), back(dp)]
}

/// The six faces as closed quads: (name, the four corners, shade).
pub fn faces(c: &[Point; 8]) -> [(&'static str, [Point; 4], f32); 6] {
    let [a, b, cc, d, a2, b2, c2, d2] = *c;
    [
        ("back", [a2, b2, c2, d2], 0.75),
        ("bottom", [d, cc, c2, d2], 0.65),
        ("left", [a, d, d2, a2], 0.8),
        ("right", [b, cc, c2, b2], 0.9),
        ("top", [a, b, b2, a2], 1.15),
        ("front", [a, b, cc, d], 1.0),
    ]
}

fn quad_path(q: [Point; 4]) -> PathData {
    PathData::single(SubPath::polyline(&q, true))
}

fn shaded(base: &Paint, shade: f32) -> Paint {
    match base.color() {
        Some(c) => {
            let [r, g, b] = c.to_rgb();
            Paint::solid(Color::rgb((r * shade).clamp(0.0, 1.0), (g * shade).clamp(0.0, 1.0), (b * shade).clamp(0.0, 1.0)))
        }
        None => Paint::solid(Color::rgb(0.6 * shade, 0.6 * shade, 0.6 * shade)),
    }
}

fn axis_param(p: &Value, key: &str, base: Axis) -> Axis {
    match p.get(key) {
        Some(v) if v.is_object() => {
            Axis { finite: v.get("finite").and_then(Value::as_bool).unwrap_or(base.finite), angle: f64_or(v, "angle", base.angle) }
        }
        _ => base,
    }
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "box3d.create";
    let a = Point::new(f64_req(p, "x0", C)?, f64_req(p, "y0", C)?);
    let c = Point::new(f64_req(p, "x1", C)?, f64_req(p, "y1", C)?);
    if (c.x - a.x).abs() < 1e-6 || (c.y - a.y).abs() < 1e-6 {
        return Err(bad(C, "the front face needs a width and a height"));
    }
    let b = Box3d {
        a,
        c,
        depth: f64_or(p, "depth", 0.4).clamp(0.0, 1.0),
        x: axis_param(p, "x", Axis { finite: true, angle: 0.0 }),
        y: axis_param(p, "y", Axis { finite: false, angle: 90.0 }),
        z: axis_param(p, "z", Axis { finite: true, angle: 30.0 }),
    };
    let g = grid_of(&s.doc()?.doc);
    let fill = s.paint.fill.clone();
    let stroke = s.paint.stroke.clone();
    let width = s.paint.stroke_width.max(0.0);
    let corners = corners(&b, &g);
    // The group, then its faces back to front.
    let out = add_node(s, "3D Box", NodeKind::Group { children: vec![], clip: false }, Appearance::default(), Some("3D Box".into()))?;
    let gid = out.get("id").and_then(Value::as_u64).map(NodeId).ok_or_else(|| bad(C, "no group made"))?;
    let faces_made = s.edit("3D Box", |d, sel| {
        let mut ids = vec![];
        for (name, quad, shade) in faces(&corners) {
            let id = d.alloc_id();
            let mut n = Node::new(id, NodeKind::Path { path: quad_path(quad), rule: FillRule::NonZero, live: None, clipping: false, guide: false });
            n.appearance = Appearance::basic(shaded(&fill, shade), stroke.clone(), width).into();
            n.name = Some(name.to_string());
            d.insert(Some(gid), usize::MAX, n)?;
            ids.push(id.0);
        }
        if let Some(n) = d.node_mut(gid) {
            n.edit_attrs(|at| at.data.push((BOX_KEY.to_string(), b.data())));
        }
        sel.set([gid]);
        Ok(ids)
    })?;
    Ok(json!({ "id": gid.0, "faces": faces_made }))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "box3d.set";
    let id = match p.get("id") {
        Some(v) if !v.is_null() => checked_id(s, v, C)?,
        _ => {
            let d = &s.doc()?.doc;
            s.doc()?
                .selection
                .objects
                .iter()
                .copied()
                .find(|id| d.node(*id).is_some_and(|n| box_of(n).is_some()))
                .ok_or_else(|| bad(C, "no 3D box selected"))?
        }
    };
    let d = &s.doc()?.doc;
    let mut b = d.node(id).and_then(box_of).ok_or_else(|| bad(C, "the object is not a 3D box"))?;
    b.a = Point::new(f64_or(p, "x0", b.a.x), f64_or(p, "y0", b.a.y));
    b.c = Point::new(f64_or(p, "x1", b.c.x), f64_or(p, "y1", b.c.y));
    b.depth = f64_or(p, "depth", b.depth).clamp(0.0, 1.0);
    b.x = axis_param(p, "x", b.x);
    b.y = axis_param(p, "y", b.y);
    b.z = axis_param(p, "z", b.z);
    let g = grid_of(d);
    let corners = corners(&b, &g);
    let new_faces = faces(&corners);
    s.edit("3D Box Options", |d, _| {
        let children: Vec<NodeId> = d.node(id).and_then(|n| n.children()).map(|c| c.iter().map(|n| n.id).collect()).unwrap_or_default();
        for cid in children {
            let Some(n) = d.node_mut(cid) else { continue };
            let Some((_, quad, _)) = new_faces.iter().find(|(name, ..)| n.name.as_deref() == Some(name)) else { continue };
            if let NodeKind::Path { path, live, .. } = &mut n.kind {
                *path = quad_path(*quad);
                *live = None;
            }
        }
        if let Some(n) = d.node_mut(id) {
            n.edit_attrs(|at| {
                at.data.retain(|(k, _)| k != BOX_KEY);
                at.data.push((BOX_KEY.to_string(), b.data()));
            });
        }
        Ok(())
    })?;
    Ok(json!({ "id": id.0 }))
}
