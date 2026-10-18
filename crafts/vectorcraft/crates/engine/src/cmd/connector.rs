//! Connectors (`connector.create`, `connector.reroute`, `connector.setObstacle`,
//! `connector.layout`): stroked open paths that stay attached to the two objects they join and
//! route around the objects marked as obstacles. A connector is a path whose `vc-connector` data
//! names its ends and options (`from,to,routing,curvature,spacing`); every transform of the
//! document reroutes the connectors whose ends moved ([`reroute_all`]), so they follow.
//!
//! Routing is from the edge of the start object's bounds facing the end object to the facing edge
//! of the end object's: a straight polyline, or an orthogonal route of three segments through a
//! channel midway, bent around the obstacles its segments would cross (a detour above or below,
//! left or right of the obstacle, `spacing` away). `curvature` rounds the corners.

use redrob_vector_doc::{Document, Node, NodeId, NodeKind};
use redrob_vector_geom::{Anchor, FillRule, PathData, Point, Rect, SubPath, Vec2};
use serde_json::{Value, json};

use super::create::add_node;
use super::*;

/// The data key a connector keeps its ends and options under.
pub const CONNECTOR_KEY: &str = "vc-connector";
/// The data key marking an object as an obstacle connectors route around.
pub const OBSTACLE_KEY: &str = "vc-obstacle";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "connector.create",
            "Connect Objects",
            [],
            None,
            "{from: id, to: id, routing?: \"polyline\"|\"orthogonal\" (orthogonal), curvature?: 0..100 (0; rounds the corners), spacing?: pt (10; how far the route keeps from obstacles)} draw a connector between two objects: a stroked path that follows them when they move and routes around the objects marked as obstacles → {id}",
            has_doc,
            create
        ),
        cmd!(
            "connector.reroute",
            "Reroute Connectors",
            [],
            None,
            "{ids?: [..] (default: every connector)} route the connectors again from their ends' current bounds (what every transform does on its own) → {rerouted: n}",
            has_doc,
            reroute
        ),
        cmd!(
            "connector.setObstacle",
            "Connector Obstacle",
            [],
            None,
            "{ids?|id? (default: the selection), obstacle?: bool (true)} mark the objects as obstacles connectors route around (or unmark them) → {ids}",
            has_selection,
            set_obstacle
        ),
        cmd!(
            "connector.layout",
            "Arrange Connected Objects",
            [],
            None,
            "{ids?: [..] (default: the selection, else every connected object), spacing?: pt (40), direction?: \"right\"|\"down\" (right)} lay the objects joined by connectors out in layers (the objects nothing points to first), evenly spaced, and reroute → {moved: n}",
            has_doc,
            layout
        ),
    ]
}

/// How a connector routes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Routing {
    pub orthogonal: bool,
    /// 0..1: how much of each corner is rounded.
    pub curvature: f64,
    pub spacing: f64,
}

/// A connector's record: its ends and routing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub from: NodeId,
    pub to: NodeId,
    pub routing: Routing,
}

impl Link {
    fn data(&self) -> String {
        format!(
            "{},{},{},{},{}",
            self.from.0,
            self.to.0,
            if self.routing.orthogonal { "orthogonal" } else { "polyline" },
            (self.routing.curvature * 100.0).round(),
            self.routing.spacing
        )
    }

    fn parse(s: &str) -> Option<Self> {
        let mut it = s.split(',');
        let from = NodeId(it.next()?.trim().parse().ok()?);
        let to = NodeId(it.next()?.trim().parse().ok()?);
        let orthogonal = it.next().is_none_or(|r| r.trim() != "polyline");
        let curvature = it.next().and_then(|c| c.trim().parse::<f64>().ok()).unwrap_or(0.0) / 100.0;
        let spacing = it.next().and_then(|c| c.trim().parse::<f64>().ok()).unwrap_or(10.0);
        Some(Link { from, to, routing: Routing { orthogonal, curvature: curvature.clamp(0.0, 1.0), spacing: spacing.max(0.0) } })
    }
}

/// The connector record of `n`, if it is one.
pub fn link_of(n: &Node) -> Option<Link> {
    n.extras().attrs.as_deref()?.data.iter().find(|(k, _)| k == CONNECTOR_KEY).and_then(|(_, v)| Link::parse(v))
}

fn is_obstacle(n: &Node) -> bool {
    n.extras().attrs.as_deref().is_some_and(|a| a.data.iter().any(|(k, v)| k == OBSTACLE_KEY && v != "0" && v != "false"))
}

/// The side of `a` that faces `b` and where the route leaves it: the midpoint of that edge, pushed
/// out a little, and the direction out of it.
fn port(a: Rect, b: Rect) -> (Point, Vec2) {
    let d = b.center() - a.center();
    if d.x.abs() * a.height() >= d.y.abs() * a.width() {
        if d.x >= 0.0 { (Point::new(a.x1, a.center().y), Vec2::new(1.0, 0.0)) } else { (Point::new(a.x0, a.center().y), Vec2::new(-1.0, 0.0)) }
    } else if d.y >= 0.0 {
        (Point::new(a.center().x, a.y1), Vec2::new(0.0, 1.0))
    } else {
        (Point::new(a.center().x, a.y0), Vec2::new(0.0, -1.0))
    }
}

/// Does the segment `a`–`b` cross `r`? (Liang–Barsky clipping.)
fn crosses(a: Point, b: Point, r: Rect) -> bool {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let d = b - a;
    let edges = [(-d.x, a.x - r.x0), (d.x, r.x1 - a.x), (-d.y, a.y - r.y0), (d.y, r.y1 - a.y)];
    for (p, q) in edges {
        if p.abs() < 1e-12 {
            if q < 0.0 {
                return false;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            if t > t1 {
                return false;
            }
            t0 = t0.max(t);
        } else {
            if t < t0 {
                return false;
            }
            t1 = t1.min(t);
        }
    }
    t0 <= t1
}

fn blocked(poly: &[Point], obstacles: &[Rect]) -> bool {
    poly.windows(2).any(|w| obstacles.iter().any(|o| crosses(w[0], w[1], *o)))
}

/// The route from `a` to `b` as a polyline, around `obstacles` (already inflated by the spacing).
pub fn route(a: Rect, b: Rect, obstacles: &[Rect], r: Routing) -> Vec<Point> {
    let (pa, da) = port(a, b);
    let (pb, db) = port(b, a);
    let gap = r.spacing.max(2.0);
    let start = pa + da * gap.min(a.width().min(a.height()) * 0.25).max(1.0);
    let end = pb + db * gap.min(b.width().min(b.height()) * 0.25).max(1.0);
    if !r.orthogonal {
        let direct = vec![pa, pb];
        if !blocked(&direct, obstacles) {
            return direct;
        }
        // A polyline bends once around the first obstacle in the way, above or below it.
        for o in obstacles {
            if !crosses(pa, pb, *o) {
                continue;
            }
            for corner in [Point::new(o.x0, o.y0), Point::new(o.x1, o.y0), Point::new(o.x1, o.y1), Point::new(o.x0, o.y1)] {
                let p = vec![pa, corner, pb];
                if !blocked(&p, obstacles) {
                    return p;
                }
            }
        }
        return direct;
    }
    // Orthogonal: start → channel → end, the channel midway between the ports.
    let horizontal = da.x != 0.0;
    let base = if horizontal {
        if db.x != 0.0 {
            let xm = (start.x + end.x) / 2.0;
            vec![pa, start, Point::new(xm, start.y), Point::new(xm, end.y), end, pb]
        } else {
            vec![pa, start, Point::new(end.x, start.y), end, pb]
        }
    } else if db.y != 0.0 {
        let ym = (start.y + end.y) / 2.0;
        vec![pa, start, Point::new(start.x, ym), Point::new(end.x, ym), end, pb]
    } else {
        vec![pa, start, Point::new(start.x, end.y), end, pb]
    };
    if !blocked(&base, obstacles) {
        return dedup(base);
    }
    // Detours: a channel beside each obstacle in the way, on either side.
    let mut candidates = vec![];
    for o in obstacles {
        if horizontal {
            for y in [o.y0 - 1.0, o.y1 + 1.0] {
                candidates.push(vec![pa, start, Point::new(start.x, y), Point::new(end.x, y), end, pb]);
            }
            for x in [o.x0 - 1.0, o.x1 + 1.0] {
                candidates.push(vec![pa, start, Point::new(x, start.y), Point::new(x, end.y), end, pb]);
            }
        } else {
            for x in [o.x0 - 1.0, o.x1 + 1.0] {
                candidates.push(vec![pa, start, Point::new(x, start.y), Point::new(x, end.y), end, pb]);
            }
            for y in [o.y0 - 1.0, o.y1 + 1.0] {
                candidates.push(vec![pa, start, Point::new(start.x, y), Point::new(end.x, y), end, pb]);
            }
        }
    }
    let len = |p: &Vec<Point>| p.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>();
    candidates.into_iter().filter(|p| !blocked(p, obstacles)).min_by(|p, q| len(p).total_cmp(&len(q))).map(dedup).unwrap_or_else(|| dedup(base))
}

fn dedup(p: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(p.len());
    for q in p {
        if out.last().is_none_or(|l| l.distance(q) > 1e-6) {
            out.push(q);
        }
    }
    // Collinear middle points add nothing.
    let mut i = 1;
    while i + 1 < out.len() {
        let (a, b, c) = (out[i - 1], out[i], out[i + 1]);
        if (b - a).cross(c - b).abs() < 1e-6 {
            out.remove(i);
        } else {
            i += 1;
        }
    }
    out
}

/// The polyline as a path, its corners rounded by `curvature` (0: sharp; 1: each corner cut back
/// halfway along its shorter side and bent through a cubic).
pub fn polyline_path(pts: &[Point], curvature: f64) -> PathData {
    if pts.len() < 2 {
        return PathData::new(vec![]);
    }
    let k = curvature.clamp(0.0, 1.0);
    if k <= 1e-6 || pts.len() == 2 {
        return PathData::single(SubPath::polyline(pts, false));
    }
    let mut anchors = vec![Anchor::corner(pts[0])];
    for i in 1..pts.len() - 1 {
        let (p0, p, p1) = (pts[i - 1], pts[i], pts[i + 1]);
        let (din, dout) = (p - p0, p1 - p);
        let (lin, lout) = (din.hypot(), dout.hypot());
        if lin < 1e-9 || lout < 1e-9 {
            continue;
        }
        let cut = (k * 0.5 * lin.min(lout)).min(lin / 2.0).min(lout / 2.0);
        let a = p - din / lin * cut;
        let b = p + dout / lout * cut;
        // A smooth bend through the corner: handles toward the corner point.
        anchors.push(Anchor::with_handles(a, a, a + (p - a) * 0.55));
        anchors.push(Anchor::with_handles(b, b + (p - b) * 0.55, b));
    }
    anchors.push(Anchor::corner(pts[pts.len() - 1]));
    PathData::single(SubPath::new(anchors, false))
}

/// The obstacles a connector between `from` and `to` routes around: the marked objects' bounds
/// (not the ends, not connectors), inflated by the spacing.
fn obstacles(d: &Document, from: NodeId, to: NodeId, spacing: f64) -> Vec<Rect> {
    let mut out = vec![];
    d.walk(|n| {
        if n.id != from
            && n.id != to
            && is_obstacle(n)
            && link_of(n).is_none()
            && n.visible
            && let Some(b) = n.visual_bounds()
        {
            out.push(b.inflate(spacing, spacing));
        }
    });
    out
}

/// The connector's path for the ends' current bounds; `None` when an end is gone.
fn routed(d: &Document, link: &Link) -> Option<PathData> {
    let a = d.bounds_of(&[link.from], true)?;
    let b = d.bounds_of(&[link.to], true)?;
    let obs = obstacles(d, link.from, link.to, link.routing.spacing);
    Some(polyline_path(&route(a, b, &obs, link.routing), link.routing.curvature))
}

/// Route every connector of `d` again (what a transform does when it is done): the connectors
/// whose ends are gone keep their path.
pub(crate) fn reroute_all(d: &mut Document) -> usize {
    let mut links: Vec<(NodeId, Link)> = vec![];
    d.walk(|n| {
        if let Some(l) = link_of(n) {
            links.push((n.id, l));
        }
    });
    let mut n = 0;
    for (id, link) in links {
        let Some(pd) = routed(d, &link) else { continue };
        if let Some(node) = d.node_mut(id)
            && let NodeKind::Path { path, live, .. } = &mut node.kind
            && *path != pd
        {
            *path = pd;
            *live = None;
            n += 1;
        }
    }
    n
}

fn routing_param(p: &Value, cmd: &str, base: Routing) -> Result<Routing> {
    let orthogonal = match str_param(p, "routing") {
        None => base.orthogonal,
        Some("orthogonal") => true,
        Some("polyline") => false,
        Some(other) => return Err(bad(cmd, format!("routing must be polyline or orthogonal, not `{other}`"))),
    };
    Ok(Routing {
        orthogonal,
        curvature: (f64_or(p, "curvature", base.curvature * 100.0) / 100.0).clamp(0.0, 1.0),
        spacing: f64_or(p, "spacing", base.spacing).clamp(0.0, 1000.0),
    })
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "connector.create";
    let from = checked_id(s, p.get("from").ok_or_else(|| bad(C, "missing `from`"))?, C)?;
    let to = checked_id(s, p.get("to").ok_or_else(|| bad(C, "missing `to`"))?, C)?;
    if from == to {
        return Err(bad(C, "a connector joins two different objects"));
    }
    let routing = routing_param(p, C, Routing { orthogonal: true, curvature: 0.0, spacing: 10.0 })?;
    let link = Link { from, to, routing };
    let pd = routed(&s.doc()?.doc, &link).ok_or_else(|| bad(C, "an end has no bounds"))?;
    let kind = NodeKind::Path { path: pd, rule: FillRule::NonZero, live: None, clipping: false, guide: false };
    let stroke = s.paint.stroke.clone();
    let stroke = if stroke.is_none() { redrob_vector_color::Paint::solid(redrob_vector_color::Color::BLACK) } else { stroke };
    let appearance = redrob_vector_doc::Appearance::basic(redrob_vector_color::Paint::None, stroke, s.paint.stroke_width.max(0.5));
    let out = add_node(s, "Connector", kind, appearance, Some("Connector".into()))?;
    let id = out.get("id").and_then(Value::as_u64).map(NodeId).ok_or_else(|| bad(C, "no connector made"))?;
    s.edit("Connector", |d, _| {
        if let Some(n) = d.node_mut(id) {
            n.edit_attrs(|a| a.data.push((CONNECTOR_KEY.to_string(), link.data())));
        }
        Ok(())
    })?;
    Ok(json!({ "id": id.0 }))
}

fn reroute(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "connector.reroute";
    let only: Option<Vec<NodeId>> = if p.get("ids").is_some() { Some(checked_ids_param(s, p, "ids", C)?) } else { None };
    let n = s.edit("Reroute Connectors", |d, _| {
        Ok(match &only {
            None => reroute_all(d),
            Some(ids) => {
                let mut n = 0;
                for id in ids {
                    let Some(link) = d.node(*id).and_then(link_of) else { continue };
                    let Some(pd) = routed(d, &link) else { continue };
                    if let Some(node) = d.node_mut(*id)
                        && let NodeKind::Path { path, live, .. } = &mut node.kind
                    {
                        *path = pd;
                        *live = None;
                        n += 1;
                    }
                }
                n
            }
        })
    })?;
    Ok(json!({ "rerouted": n }))
}

fn set_obstacle(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "connector.setObstacle";
    let ids = checked_ids_or_selection(s, p, C)?;
    let on = bool_or(p, "obstacle", true);
    s.edit(if on { "Connector Obstacle" } else { "Not an Obstacle" }, |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.edit_attrs(|a| {
                    a.data.retain(|(k, _)| k != OBSTACLE_KEY);
                    if on {
                        a.data.push((OBSTACLE_KEY.to_string(), "1".into()));
                    }
                });
            }
        }
        reroute_all(d);
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn layout(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "connector.layout";
    let spacing = f64_or(p, "spacing", 40.0).clamp(0.0, 10_000.0);
    let down = match str_param(p, "direction") {
        None | Some("right") => false,
        Some("down") => true,
        Some(other) => return Err(bad(C, format!("direction must be right or down, not `{other}`"))),
    };
    let d = &s.doc()?.doc;
    let mut links = vec![];
    d.walk(|n| {
        if let Some(l) = link_of(n) {
            links.push(l);
        }
    });
    // The objects laid out: the ones given or selected, else every connected object.
    let given: Vec<NodeId> = if p.get("ids").is_some() { checked_ids_param(s, p, "ids", C)? } else { s.doc()?.selection.objects.clone() };
    let d = &s.doc()?.doc;
    let mut nodes: Vec<NodeId> = if given.is_empty() {
        let mut v: Vec<NodeId> = links.iter().flat_map(|l| [l.from, l.to]).collect();
        v.sort_by_key(|i| i.0);
        v.dedup();
        v
    } else {
        given
    };
    nodes.retain(|id| d.node(*id).is_some_and(|n| link_of(n).is_none()) && d.bounds_of(&[*id], true).is_some());
    if nodes.is_empty() {
        return Err(bad(C, "no connected objects to arrange"));
    }
    // Layers: the objects nothing in the set points to come first; each next layer holds what the
    // previous one points to (a cycle breaks at its first member).
    let in_set = |id: NodeId| nodes.contains(&id);
    let mut layer_of: std::collections::HashMap<NodeId, usize> = std::collections::HashMap::new();
    let mut frontier: Vec<NodeId> = nodes.iter().copied().filter(|n| !links.iter().any(|l| l.to == *n && in_set(l.from) && l.from != *n)).collect();
    if frontier.is_empty() {
        frontier.push(nodes[0]);
    }
    let mut level = 0;
    while !frontier.is_empty() && level < nodes.len() {
        let mut next = vec![];
        for id in frontier {
            if layer_of.contains_key(&id) {
                continue;
            }
            layer_of.insert(id, level);
            for l in links.iter().filter(|l| l.from == id && in_set(l.to) && !layer_of.contains_key(&l.to)) {
                next.push(l.to);
            }
        }
        frontier = next;
        level += 1;
    }
    for id in &nodes {
        layer_of.entry(*id).or_insert(level);
    }
    // Columns (or rows) of layers, each spaced by `spacing`, starting at the set's top-left.
    let all = d.bounds_of(&nodes, true).ok_or_else(|| bad(C, "no bounds"))?;
    let mut by_layer: Vec<Vec<NodeId>> = vec![vec![]; level + 1];
    for id in &nodes {
        by_layer[layer_of[id]].push(*id);
    }
    let bounds: std::collections::HashMap<NodeId, Rect> = nodes.iter().filter_map(|id| d.bounds_of(&[*id], true).map(|b| (*id, b))).collect();
    let mut moves: Vec<(NodeId, Vec2)> = vec![];
    let mut along = if down { all.y0 } else { all.x0 };
    for layer in by_layer.iter().filter(|l| !l.is_empty()) {
        let thick = layer.iter().map(|id| if down { bounds[id].height() } else { bounds[id].width() }).fold(0.0, f64::max);
        let mut across = if down { all.x0 } else { all.y0 };
        for id in layer {
            let b = bounds[id];
            let target = if down { Point::new(across, along) } else { Point::new(along, across) };
            moves.push((*id, target - Point::new(b.x0, b.y0)));
            across += if down { b.width() } else { b.height() } + spacing;
        }
        along += thick + spacing;
    }
    let moved = moves.iter().filter(|(_, v)| v.hypot() > 1e-9).count();
    s.edit("Arrange Connected Objects", |d, _| {
        for (id, v) in &moves {
            if let Some(n) = d.node_mut(*id) {
                n.transform(redrob_vector_geom::Affine::translate(*v), false);
            }
        }
        reroute_all(d);
        Ok(())
    })?;
    Ok(json!({ "moved": moved }))
}
