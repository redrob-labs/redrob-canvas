//! Three on-canvas transform modes that share one preview: the layer's pixels on a textured grid
//! that follows the mapping, with handles drawn over it; ↩ commits, Esc cancels.
//!
//! * Cage Transform (`edit.cageTransform` from the menu): click round the part to deform to lay
//!   the cage's vertices, close it by clicking the first one (or Close Cage), then drag vertices;
//!   the enclosed pixels follow by mean value coordinates. Fill Plain Colour paints the vacated
//!   part of the original cage with the outline's plain colour.
//! * Handle Transform (`edit.handleTransform`): place one to four handles on the layer, then
//!   drag them: one handle moves, two rotate and scale, three shear (an affine map), four give a
//!   perspective. ⌥-click removes a handle.
//! * 3D Transform (`edit.transform3d`): Camera, Move and Rotate tabs. The layer is a plane in
//!   front of a pinhole camera with a focal length (px) looking at the vanishing point; Move
//!   drags it in X/Y (⇧ for Z), Rotate drags turn it about X (vertical drag) and Y (horizontal),
//!   ⌥ about Z; Camera drags move the vanishing point (⇧ changes the focal length). The pivot is
//!   the layer's centre (local) or the canvas centre (unified).
//!
//! Handle and 3D transforms commit through `edit.transform` with the layer's bounds as the frame
//! and the mapped corners as the quad; Cage commits through `edit.cageTransform`.

use std::sync::Arc;

use egui::{Color32, Pos2, Stroke, TextureHandle, pos2};
use redrob_photo_algo::cage::Cage;
use redrob_photo_algo::transform::Homography;
use redrob_photo_doc::{Document, LayerId};
use redrob_photo_geom::Rect;
use serde_json::{Value, json};

use crate::PhotoApp;
use crate::canvas::{ToolEvent, ViewXform};

pub const HANDLE: &str = "edit.handleTransform";
pub const THREE_D: &str = "edit.transform3d";
pub const CAGE: &str = "edit.cageTransform";

const GRID: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Cage,
    Handle,
    ThreeD,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Camera,
    Move,
    Rotate,
}

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Tab::Camera => "camera",
            Tab::Move => "move",
            Tab::Rotate => "rotate",
        }
    }
    fn parse(s: &str) -> Option<Tab> {
        Some(match s {
            "camera" => Tab::Camera,
            "move" => Tab::Move,
            "rotate" => Tab::Rotate,
            _ => return None,
        })
    }
}

/// The 3D Transform's parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreeD {
    /// Vanishing point (document px): where the camera looks.
    pub vanish: [f64; 2],
    /// Focal length (px).
    pub focal: f64,
    /// Translation in X, Y (document px) and Z (px, towards the camera when negative).
    pub offset: [f64; 3],
    /// Rotation about X, Y, Z in degrees.
    pub rotation: [f64; 3],
    /// Pivot: the layer's centre (false) or the canvas centre (true).
    pub unified_pivot: bool,
}

impl ThreeD {
    /// Where a plane point `(x, y)` (document px, on the untransformed layer) projects.
    pub fn project(&self, pivot: [f64; 2], x: f64, y: f64) -> [f64; 2] {
        let (px, py, pz) = (x - pivot[0], y - pivot[1], 0.0);
        let [rx, ry, rz] = self.rotation.map(f64::to_radians);
        // Rotate about X, then Y, then Z.
        let (cy, sy) = (rx.cos(), rx.sin());
        let (y1, z1) = (py * cy - pz * sy, py * sy + pz * cy);
        let (cx, sx) = (ry.cos(), ry.sin());
        let (x2, z2) = (px * cx + z1 * sx, -px * sx + z1 * cx);
        let (cz, sz) = (rz.cos(), rz.sin());
        let (x3, y3) = (x2 * cz - y1 * sz, x2 * sz + y1 * cz);
        let (wx, wy, wz) = (x3 + pivot[0] + self.offset[0], y3 + pivot[1] + self.offset[1], z2 + self.offset[2]);
        // Pinhole at distance `focal` in front of the plane, looking at the vanishing point.
        let depth = (self.focal + wz).max(1e-3);
        let k = self.focal / depth;
        [self.vanish[0] + (wx - self.vanish[0]) * k, self.vanish[1] + (wy - self.vanish[1]) * k]
    }

    pub fn is_identity(&self) -> bool {
        self.offset == [0.0; 3] && self.rotation == [0.0; 3]
    }
}

pub struct Session {
    pub key: u64,
    pub kind: Kind,
    pub layer: LayerId,
    bounds: Rect,
    texture: TextureHandle,
    opacity: f32,
    pub preview_doc: Arc<Document>,
    /// The textured grid's vertices after the mapping (row-major, (GRID+1)²).
    grid: Vec<[f64; 2]>,
    // Cage
    pub cage: Vec<[f64; 2]>,
    pub deformed: Vec<[f64; 2]>,
    pub closed: bool,
    pub fill_plain: bool,
    // Handle
    pub handles: Vec<([f64; 2], [f64; 2])>,
    // 3D
    pub three_d: ThreeD,
    pub tab: Tab,
    drag: Option<Drag>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    Vertex(usize),
    Handle(usize),
    ThreeD { last: [f64; 2], shift: bool, alt: bool },
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The homography `n` handle pairs stand for: a move, a similarity, an affine map or a full
/// perspective (one to four handles); identity when they are degenerate.
pub fn handle_homography(handles: &[([f64; 2], [f64; 2])]) -> Homography {
    match handles.len() {
        0 => Homography::IDENTITY,
        1 => {
            let (s, d) = handles[0];
            Homography([1.0, 0.0, d[0] - s[0], 0.0, 1.0, d[1] - s[1], 0.0, 0.0, 1.0])
        }
        2 => {
            let ((s0, d0), (s1, d1)) = (handles[0], handles[1]);
            let (sv, dv) = ([s1[0] - s0[0], s1[1] - s0[1]], [d1[0] - d0[0], d1[1] - d0[1]]);
            let ls = sv[0].hypot(sv[1]);
            if ls < 1e-9 {
                return Homography::IDENTITY;
            }
            // Complex division dv / sv gives the rotation and scale.
            let (a, b) = ((dv[0] * sv[0] + dv[1] * sv[1]) / (ls * ls), (dv[1] * sv[0] - dv[0] * sv[1]) / (ls * ls));
            // x' = a x - b y + tx ; y' = b x + a y + ty, through s0 → d0.
            let tx = d0[0] - (a * s0[0] - b * s0[1]);
            let ty = d0[1] - (b * s0[0] + a * s0[1]);
            Homography([a, -b, tx, b, a, ty, 0.0, 0.0, 1.0])
        }
        3 => {
            let [(s0, d0), (s1, d1), (s2, d2)] = [handles[0], handles[1], handles[2]];
            // Solve the affine map from three point pairs.
            let det = (s1[0] - s0[0]) * (s2[1] - s0[1]) - (s2[0] - s0[0]) * (s1[1] - s0[1]);
            if det.abs() < 1e-9 {
                return handle_homography(&handles[..2]);
            }
            let solve = |v0: f64, v1: f64, v2: f64| -> [f64; 3] {
                let (u1, u2) = (v1 - v0, v2 - v0);
                let a = (u1 * (s2[1] - s0[1]) - u2 * (s1[1] - s0[1])) / det;
                let b = (u2 * (s1[0] - s0[0]) - u1 * (s2[0] - s0[0])) / det;
                [a, b, v0 - a * s0[0] - b * s0[1]]
            };
            let [a, b, c] = solve(d0[0], d1[0], d2[0]);
            let [d, e, f] = solve(d0[1], d1[1], d2[1]);
            Homography([a, b, c, d, e, f, 0.0, 0.0, 1.0])
        }
        _ => {
            let src = [handles[0].0, handles[1].0, handles[2].0, handles[3].0];
            let dst = [handles[0].1, handles[1].1, handles[2].1, handles[3].1];
            let unit = [0.0, 0.0, 1.0, 1.0];
            match (Homography::rect_to_quad(unit, src).and_then(|h| h.inverse()), Homography::rect_to_quad(unit, dst)) {
                (Some(src_inv), Some(to_dst)) => to_dst.mul(&src_inv),
                _ => handle_homography(&handles[..3]),
            }
        }
    }
}

impl Session {
    fn pivot(&self, doc: &Document) -> [f64; 2] {
        if self.three_d.unified_pivot {
            [f64::from(doc.size.width) / 2.0, f64::from(doc.size.height) / 2.0]
        } else {
            [f64::from(self.bounds.x0 + self.bounds.x1) / 2.0, f64::from(self.bounds.y0 + self.bounds.y1) / 2.0]
        }
    }

    /// Where a layer point goes under the current mapping.
    pub fn map(&self, x: f64, y: f64) -> [f64; 2] {
        match self.kind {
            Kind::Cage => match Cage::new(self.cage.clone(), self.deformed.clone()) {
                Some(c) if self.closed => {
                    let (u, v) = c.map(x, y);
                    [u, v]
                }
                _ => [x, y],
            },
            Kind::Handle => {
                let (u, v) = handle_homography(&self.handles).apply(x, y);
                [u, v]
            }
            Kind::ThreeD => self.three_d.project(self.pivot(&self.preview_doc), x, y),
        }
    }

    /// The layer bounds' corners under the mapping (clockwise from top-left).
    pub fn quad(&self) -> [[f64; 2]; 4] {
        let b = self.bounds;
        let r = [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)];
        [self.map(r[0], r[1]), self.map(r[2], r[1]), self.map(r[2], r[3]), self.map(r[0], r[3])]
    }

    fn update_grid(&mut self) {
        let b = self.bounds;
        let (w, h) = (f64::from(b.width()), f64::from(b.height()));
        let mut g = Vec::with_capacity((GRID + 1) * (GRID + 1));
        for j in 0..=GRID {
            for i in 0..=GRID {
                let x = f64::from(b.x0) + w * i as f64 / GRID as f64;
                let y = f64::from(b.y0) + h * j as f64 / GRID as f64;
                g.push(self.map(x, y));
            }
        }
        self.grid = g;
    }

    pub fn is_identity(&self) -> bool {
        match self.kind {
            Kind::Cage => !self.closed || self.cage.iter().zip(&self.deformed).all(|(a, b)| dist(*a, *b) < 1e-9),
            Kind::Handle => self.handles.iter().all(|(s, d)| dist(*s, *d) < 1e-9),
            Kind::ThreeD => self.three_d.is_identity(),
        }
    }

    pub fn describe(&self) -> Value {
        json!({
            "kind": match self.kind { Kind::Cage => "cage", Kind::Handle => "handle", Kind::ThreeD => "3d" },
            "layer": self.layer.0,
            "cage": self.cage,
            "deformed": self.deformed,
            "closed": self.closed,
            "fillPlain": self.fill_plain,
            "handles": self.handles.iter().map(|(s, d)| json!({"src": s, "dst": d})).collect::<Vec<_>>(),
            "vanishingPoint": self.three_d.vanish,
            "focalLength": self.three_d.focal,
            "offset": self.three_d.offset,
            "rotation": self.three_d.rotation,
            "unifiedPivot": self.three_d.unified_pivot,
            "tab": self.tab.name(),
            "quad": self.quad(),
        })
    }
}

/// Starts one of the modes on the active layer.
pub fn begin(app: &mut PhotoApp, ctx: &egui::Context, kind: Kind) -> Result<(), String> {
    if app.ui.transform.is_some() {
        return Err("finish Free Transform first".into());
    }
    let (layer, surface, _) = crate::distort_ui::active_pixels(app)?;
    let st = app.session.active().ok_or("no document")?;
    let l = st.doc.layer(layer).ok_or("no layer")?;
    let bounds = surface.content_bounds();
    if bounds.is_empty() {
        return Err("the layer is empty".into());
    }
    let opacity = l.opacity * l.fill_opacity;
    let preview_doc = crate::distort_ui::without_layer(&st.doc, layer);
    let (cw, ch) = (f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    let key = app.ui.alloc_id();
    let texture = ctx.load_texture(format!("shape-xf-{key}"), crate::distort_ui::surface_image(&surface, bounds, 2048), egui::TextureOptions::LINEAR);
    let mut s = Session {
        key,
        kind,
        layer,
        bounds,
        texture,
        opacity,
        preview_doc,
        grid: Vec::new(),
        cage: Vec::new(),
        deformed: Vec::new(),
        closed: false,
        fill_plain: false,
        handles: Vec::new(),
        three_d: ThreeD { vanish: [cw / 2.0, ch / 2.0], focal: cw.max(ch).max(1.0), offset: [0.0; 3], rotation: [0.0; 3], unified_pivot: false },
        tab: Tab::Rotate,
        drag: None,
    };
    s.update_grid();
    app.distort.shape = Some(s);
    app.ui.status = match kind {
        Kind::Cage => tl!("Cage Transform: click round the part to deform, click the first point to close the cage, then drag its points"),
        Kind::Handle => tl!("Handle Transform: place one to four handles and drag them (one moves, two rotate and scale, three shear, four give perspective)"),
        Kind::ThreeD => tl!("3D Transform: drag to rotate about X and Y, Alt-drag about Z; the Move and Camera tabs move the layer and the camera"),
    }
    .into();
    app.ui.status_error = false;
    Ok(())
}

pub fn commit(app: &mut PhotoApp) {
    let Some(s) = app.distort.shape.take() else { return };
    if s.is_identity() {
        return;
    }
    let b = s.bounds;
    let r = [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)];
    let result = match s.kind {
        Kind::Cage => app.run(CAGE, json!({"cage": s.cage, "deformed": s.deformed, "fillPlain": s.fill_plain, "layer": s.layer.0})),
        Kind::Handle | Kind::ThreeD => {
            let o = app.ui.transform_options;
            let mut p = json!({"layer": s.layer.0, "rect": r, "quad": s.quad(), "interpolation": "bicubic"});
            if o.clipping != crate::state::TransformClipping::Adjust {
                p["clipping"] = json!(o.clipping.param());
            }
            app.run("edit.transform", p)
        }
    };
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Control channel: `{"ui": {...}}` while a mode is active.
pub fn control(app: &mut PhotoApp, ui: &Value) -> Result<Value, String> {
    if ui.get("commit").and_then(Value::as_bool) == Some(true) {
        commit(app);
        return Ok(json!({"committed": true}));
    }
    if ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        app.distort.shape = None;
        return Ok(json!({"cancelled": true}));
    }
    let s = app.distort.shape.as_mut().ok_or("no on-canvas transform is active")?;
    let pts = |k: &str| -> Result<Option<Vec<[f64; 2]>>, String> {
        match ui.get(k) {
            Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("{k}: {e}")),
            None => Ok(None),
        }
    };
    if let Some(c) = pts("cage")? {
        s.cage = c.clone();
        s.deformed = c;
        s.closed = s.cage.len() >= 3;
    }
    if let Some(d) = pts("deformed")? {
        if d.len() != s.cage.len() {
            return Err("deformed must have as many vertices as the cage".into());
        }
        s.deformed = d;
    }
    if let Some(b) = ui.get("fillPlain").and_then(Value::as_bool) {
        s.fill_plain = b;
    }
    if let Some(h) = ui.get("handles").and_then(Value::as_array) {
        let mut v = Vec::new();
        for item in h.iter().take(4) {
            let src: [f64; 2] = serde_json::from_value(item["src"].clone()).map_err(|e| format!("handles.src: {e}"))?;
            let dst: [f64; 2] = item.get("dst").map(|d| serde_json::from_value(d.clone())).transpose().map_err(|e| format!("handles.dst: {e}"))?.unwrap_or(src);
            v.push((src, dst));
        }
        s.handles = v;
    }
    if let Some(v) = ui.get("vanishingPoint").map(|v| serde_json::from_value::<[f64; 2]>(v.clone())).transpose().map_err(|e| format!("vanishingPoint: {e}"))? {
        s.three_d.vanish = v;
    }
    if let Some(f) = ui.get("focalLength").and_then(Value::as_f64) {
        s.three_d.focal = f.clamp(1.0, 1e6);
    }
    if let Some(v) = ui.get("offset").map(|v| serde_json::from_value::<[f64; 3]>(v.clone())).transpose().map_err(|e| format!("offset: {e}"))? {
        s.three_d.offset = v;
    }
    if let Some(v) = ui.get("rotation").map(|v| serde_json::from_value::<[f64; 3]>(v.clone())).transpose().map_err(|e| format!("rotation: {e}"))? {
        s.three_d.rotation = v;
    }
    if let Some(b) = ui.get("unifiedPivot").and_then(Value::as_bool) {
        s.three_d.unified_pivot = b;
    }
    if let Some(t) = ui.get("tab").and_then(Value::as_str) {
        s.tab = Tab::parse(t).ok_or("tab: camera|move|rotate")?;
    }
    s.update_grid();
    Ok(s.describe())
}

pub fn pointer(app: &mut PhotoApp, ev: ToolEvent, mods: egui::Modifiers) {
    let tol = crate::distort_ui::tolerance(app);
    let Some(s) = app.distort.shape.as_mut() else { return };
    let p = match ev {
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => [x, y],
    };
    match (s.kind, ev) {
        (Kind::Cage, ToolEvent::Down { .. }) => {
            if !s.closed {
                if s.cage.len() >= 3 && dist(s.cage[0], p) <= tol {
                    s.closed = true;
                    s.deformed = s.cage.clone();
                } else {
                    s.cage.push(p);
                    s.deformed.push(p);
                }
            } else if let Some(i) =
                s.deformed.iter().enumerate().filter(|(_, v)| dist(**v, p) <= tol).min_by(|a, b| dist(*a.1, p).total_cmp(&dist(*b.1, p))).map(|(i, _)| i)
            {
                s.drag = Some(Drag::Vertex(i));
            }
        }
        (Kind::Handle, ToolEvent::Down { .. }) => {
            let near =
                s.handles.iter().enumerate().filter(|(_, (_, d))| dist(*d, p) <= tol).min_by(|a, b| dist(a.1.1, p).total_cmp(&dist(b.1.1, p))).map(|(i, _)| i);
            match near {
                Some(i) if mods.alt => {
                    s.handles.remove(i);
                }
                Some(i) => s.drag = Some(Drag::Handle(i)),
                None if s.handles.len() < 4 && !mods.alt => {
                    // A new handle attaches to the pixels now under the pointer (the inverse map).
                    let h = handle_homography(&s.handles);
                    let src = h.inverse().map(|inv| inv.apply(p[0], p[1])).map_or(p, |(x, y)| [x, y]);
                    s.handles.push((src, p));
                    s.drag = Some(Drag::Handle(s.handles.len() - 1));
                }
                None => {}
            }
        }
        (Kind::ThreeD, ToolEvent::Down { .. }) => s.drag = Some(Drag::ThreeD { last: p, shift: mods.shift, alt: mods.alt }),
        (_, ToolEvent::Move { .. }) => match s.drag {
            Some(Drag::Vertex(i)) => {
                if let Some(v) = s.deformed.get_mut(i) {
                    *v = p;
                }
            }
            Some(Drag::Handle(i)) => {
                if let Some(h) = s.handles.get_mut(i) {
                    h.1 = p;
                }
            }
            Some(Drag::ThreeD { last, shift, alt }) => {
                let (dx, dy) = (p[0] - last[0], p[1] - last[1]);
                let t = &mut s.three_d;
                match s.tab {
                    Tab::Move => {
                        if shift {
                            t.offset[2] += dy;
                        } else {
                            t.offset[0] += dx;
                            t.offset[1] += dy;
                        }
                    }
                    Tab::Rotate => {
                        if alt {
                            t.rotation[2] += dx * 0.5;
                        } else {
                            t.rotation[1] += dx * 0.5;
                            t.rotation[0] -= dy * 0.5;
                        }
                    }
                    Tab::Camera => {
                        if shift {
                            t.focal = (t.focal + dy * 4.0).clamp(1.0, 1e6);
                        } else {
                            t.vanish[0] += dx;
                            t.vanish[1] += dy;
                        }
                    }
                }
                s.drag = Some(Drag::ThreeD { last: p, shift, alt });
            }
            None => return,
        },
        (_, ToolEvent::Up { .. }) => {
            if let Some(Drag::Vertex(i)) = s.drag
                && let Some(v) = s.deformed.get_mut(i)
            {
                *v = p;
            }
            if let Some(Drag::Handle(i)) = s.drag
                && let Some(h) = s.handles.get_mut(i)
            {
                h.1 = p;
            }
            s.drag = None;
        }
    }
    s.update_grid();
}

pub fn draw(s: &Session, painter: &egui::Painter, xf: &ViewXform) {
    let scr = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let mut mesh = egui::Mesh::with_texture(s.texture.id());
    let tint = Color32::from_white_alpha((s.opacity.clamp(0.0, 1.0) * 255.0) as u8);
    for j in 0..=GRID {
        for i in 0..=GRID {
            let p = s.grid[j * (GRID + 1) + i];
            mesh.vertices.push(egui::epaint::Vertex { pos: scr(p), uv: pos2(i as f32 / GRID as f32, j as f32 / GRID as f32), color: tint });
        }
    }
    for j in 0..GRID {
        for i in 0..GRID {
            let a = (j * (GRID + 1) + i) as u32;
            mesh.add_triangle(a, a + 1, a + GRID as u32 + 2);
            mesh.add_triangle(a, a + GRID as u32 + 2, a + GRID as u32 + 1);
        }
    }
    painter.add(mesh);
    let t = crate::theme::Tokens::get(painter.ctx());
    let accent = t.accent;
    let handle = |p: Pos2, filled: bool| {
        painter.circle_filled(p, 4.5, if filled { Color32::WHITE } else { Color32::from_white_alpha(120) });
        painter.circle_stroke(p, 4.5, Stroke::new(1.25, accent));
    };
    match s.kind {
        Kind::Cage => {
            let ring = if s.closed { &s.deformed } else { &s.cage };
            let pts: Vec<Pos2> = ring.iter().map(|c| scr(*c)).collect();
            if s.closed {
                // The original cage, faint, under the moved one.
                let orig: Vec<Pos2> = s.cage.iter().map(|c| scr(*c)).collect();
                painter.add(egui::Shape::closed_line(orig, Stroke::new(1.0, Color32::from_white_alpha(90))));
                painter.add(egui::Shape::closed_line(pts.clone(), Stroke::new(1.5, accent)));
            } else if pts.len() > 1 {
                painter.add(egui::Shape::line(pts.clone(), Stroke::new(1.5, accent)));
            }
            for (i, p) in pts.iter().enumerate() {
                handle(*p, s.closed || i == 0);
            }
        }
        Kind::Handle => {
            let q: Vec<Pos2> = s.quad().iter().map(|c| scr(*c)).collect();
            painter.add(egui::Shape::closed_line(q, Stroke::new(1.0, accent)));
            for (k, (_, d)) in s.handles.iter().enumerate() {
                let p = scr(*d);
                handle(p, true);
                painter.text(p + egui::vec2(7.0, -7.0), egui::Align2::LEFT_BOTTOM, (k + 1).to_string(), egui::FontId::proportional(10.0), t.text);
            }
        }
        Kind::ThreeD => {
            let q: Vec<Pos2> = s.quad().iter().map(|c| scr(*c)).collect();
            painter.add(egui::Shape::closed_line(q.clone(), Stroke::new(1.5, accent)));
            // The pivot and the vanishing point.
            let piv = scr(s.pivot(&s.preview_doc));
            painter.circle_stroke(piv, 5.0, Stroke::new(1.25, Color32::WHITE));
            painter.line_segment([piv + egui::vec2(-8.0, 0.0), piv + egui::vec2(8.0, 0.0)], Stroke::new(1.0, Color32::WHITE));
            painter.line_segment([piv + egui::vec2(0.0, -8.0), piv + egui::vec2(0.0, 8.0)], Stroke::new(1.0, Color32::WHITE));
            if s.tab == Tab::Camera {
                let v = scr(s.three_d.vanish);
                handle(v, true);
                for c in &q {
                    painter.line_segment([*c, v], Stroke::new(0.75, Color32::from_white_alpha(80)));
                }
            }
        }
    }
}

fn number(ui: &mut egui::Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f32>, suffix: &str) -> bool {
    let tk = crate::theme::Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(label).color(tk.text_dim).size(12.0));
    let mut f = *v as f32;
    if crate::widgets::value_field(ui, &mut f, range, suffix, 62.0).changed() {
        *v = f64::from(f);
        return true;
    }
    false
}

pub fn options_bar(app: &mut PhotoApp, ui: &mut egui::Ui) {
    let Some(s) = app.distort.shape.as_mut() else { return };
    let mut changed = false;
    let mut cancel = false;
    let mut commit_now = false;
    match s.kind {
        Kind::Cage => {
            if !s.closed {
                ui.label(crate::i18n::fmt(tl!("{n} points"), &[("n", &s.cage.len().to_string())]));
                if ui.add_enabled(s.cage.len() >= 3, egui::Button::new(tl!("Close Cage"))).clicked() {
                    s.closed = true;
                    s.deformed = s.cage.clone();
                    changed = true;
                }
            } else if ui.button(tl!("Reset Cage")).clicked() {
                s.deformed = s.cage.clone();
                changed = true;
            }
            crate::widgets::checkbox(ui, &mut s.fill_plain, tl!("Fill Plain Colour"))
                .on_hover_text(tl!("Fill the original position of the cage with the plain colour of its outline"));
        }
        Kind::Handle => {
            let n = s.handles.len();
            let what = match n {
                0 => tl!("Click to place a handle"),
                1 => tl!("Move"),
                2 => tl!("Rotate and Scale"),
                3 => tl!("Shear"),
                _ => tl!("Perspective"),
            };
            ui.label(crate::i18n::fmt(tl!("{n} handles: {what}"), &[("n", &n.to_string()), ("what", what)]));
            if ui.add_enabled(n > 0, egui::Button::new(tl!("Reset Handles"))).clicked() {
                for h in &mut s.handles {
                    h.1 = h.0;
                }
                changed = true;
            }
            if ui.add_enabled(n > 0, egui::Button::new(tl!("Remove Handles"))).clicked() {
                s.handles.clear();
                changed = true;
            }
        }
        Kind::ThreeD => {
            for (tab, label) in [(Tab::Camera, tl!("Camera")), (Tab::Move, tl!("Move")), (Tab::Rotate, tl!("Rotate"))] {
                if crate::widgets::pill_tab(ui, label, s.tab == tab).clicked() {
                    s.tab = tab;
                }
            }
            crate::widgets::vline(ui, 22.0);
            let t = &mut s.three_d;
            match s.tab {
                Tab::Camera => {
                    changed |= number(ui, tl!("Vanishing X:"), &mut t.vanish[0], -100000.0..=100000.0, "px");
                    changed |= number(ui, tl!("Y:"), &mut t.vanish[1], -100000.0..=100000.0, "px");
                    changed |= number(ui, tl!("Focal length:"), &mut t.focal, 1.0..=100000.0, "px");
                }
                Tab::Move => {
                    changed |= number(ui, "X:", &mut t.offset[0], -100000.0..=100000.0, "px");
                    changed |= number(ui, "Y:", &mut t.offset[1], -100000.0..=100000.0, "px");
                    changed |= number(ui, "Z:", &mut t.offset[2], -100000.0..=100000.0, "px");
                }
                Tab::Rotate => {
                    changed |= number(ui, "X:", &mut t.rotation[0], -180.0..=180.0, "°");
                    changed |= number(ui, "Y:", &mut t.rotation[1], -180.0..=180.0, "°");
                    changed |= number(ui, "Z:", &mut t.rotation[2], -180.0..=180.0, "°");
                }
            }
            let pivots = [(false, tl!("Local pivot")), (true, tl!("Unified pivot"))];
            changed |= crate::widgets::dropdown(ui, "3d-pivot", &mut t.unified_pivot, &pivots, 110.0);
            if ui.button(tl!("Reset")).clicked() {
                t.offset = [0.0; 3];
                t.rotation = [0.0; 3];
                changed = true;
            }
        }
    }
    if changed {
        s.update_grid();
    }
    let kind = s.kind;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let tip = match kind {
            Kind::Cage => tl!("Commit Cage Transform (Enter)"),
            Kind::Handle => tl!("Commit Handle Transform (Enter)"),
            Kind::ThreeD => tl!("Commit 3D Transform (Enter)"),
        };
        if crate::widgets::primary_button(ui, "✓", 32.0).on_hover_text(tip).clicked() {
            commit_now = true;
        } else if crate::widgets::secondary_button(ui, "⊘", 32.0).on_hover_text(tl!("Cancel (Esc)")).clicked() {
            cancel = true;
        }
    });
    if commit_now {
        commit(app);
    } else if cancel {
        app.distort.shape = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotoApp {
        let mut app = PhotoApp::new(redrob_photo_engine::Session::new(), crate::Services::default());
        // A transparent layer: the transform's frame is the painted square's bounds, not the
        // whole of an opaque Background.
        app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
        app.session
            .edit("fill", |doc, a| {
                let surf = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
                surf.fill_rect(Rect::new(16, 16, 48, 48), &[0.2, 0.4, 0.8, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        app
    }

    #[test]
    fn handle_homographies_follow_the_handle_count() {
        let one = handle_homography(&[([0.0, 0.0], [5.0, 3.0])]);
        assert_eq!(one.apply(1.0, 1.0), (6.0, 4.0));
        // Two handles: a 90° turn with doubling about the first.
        let two = handle_homography(&[([0.0, 0.0], [0.0, 0.0]), ([1.0, 0.0], [0.0, 2.0])]);
        let (x, y) = two.apply(1.0, 0.0);
        assert!((x - 0.0).abs() < 1e-9 && (y - 2.0).abs() < 1e-9, "{x} {y}");
        let (x, y) = two.apply(0.0, 1.0);
        assert!((x + 2.0).abs() < 1e-9 && y.abs() < 1e-9, "{x} {y}");
        // Three: a shear keeping two points.
        let three = handle_homography(&[([0.0, 0.0], [0.0, 0.0]), ([1.0, 0.0], [1.0, 0.0]), ([0.0, 1.0], [1.0, 1.0])]);
        let (x, y) = three.apply(0.0, 2.0);
        assert!((x - 2.0).abs() < 1e-9 && (y - 2.0).abs() < 1e-9, "{x} {y}");
        // Four: a perspective where the top edge narrows.
        let four = handle_homography(&[([0.0, 0.0], [2.0, 0.0]), ([10.0, 0.0], [8.0, 0.0]), ([10.0, 10.0], [10.0, 10.0]), ([0.0, 10.0], [0.0, 10.0])]);
        let (x, _) = four.apply(5.0, 0.0);
        assert!((x - 5.0).abs() < 1e-6);
        // The source's middle row maps to the trapezoid's diagonals' crossing, which lies towards
        // the narrow top (equal steps along a receding plane shrink towards the far edge).
        let (_, y) = four.apply(5.0, 5.0);
        assert!(y < 5.0, "the middle row rises towards the narrow top: {y}");
    }

    #[test]
    fn three_d_projection_is_identity_at_rest_and_turns_about_y() {
        let t = ThreeD { vanish: [32.0, 32.0], focal: 100.0, offset: [0.0; 3], rotation: [0.0; 3], unified_pivot: false };
        let p = t.project([32.0, 32.0], 10.0, 20.0);
        assert!((p[0] - 10.0).abs() < 1e-9 && (p[1] - 20.0).abs() < 1e-9);
        let turned = ThreeD { rotation: [0.0, 45.0, 0.0], ..t };
        let l = turned.project([32.0, 32.0], 0.0, 32.0);
        let r = turned.project([32.0, 32.0], 64.0, 32.0);
        assert!(r[0] - l[0] < 64.0, "a turned plane is narrower on screen: {l:?} {r:?}");
        assert!((turned.project([32.0, 32.0], 32.0, 32.0)[0] - 32.0).abs() < 1e-9, "the pivot stays");
    }

    #[test]
    fn cage_mode_closes_and_commits_a_deformation() {
        let mut app = app();
        let ctx = egui::Context::default();
        begin(&mut app, &ctx, Kind::Cage).unwrap();
        let m = egui::Modifiers::NONE;
        for p in [[10.0, 10.0], [54.0, 10.0], [54.0, 54.0], [10.0, 54.0]] {
            pointer(&mut app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, m);
            pointer(&mut app, ToolEvent::Up { x: p[0], y: p[1] }, m);
        }
        pointer(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, m);
        pointer(&mut app, ToolEvent::Up { x: 10.0, y: 10.0 }, m);
        assert!(app.distort.shape.as_ref().unwrap().closed);
        pointer(&mut app, ToolEvent::Down { x: 54.0, y: 54.0, pressure: 1.0 }, m);
        pointer(&mut app, ToolEvent::Move { x: 60.0, y: 60.0, pressure: 1.0 }, m);
        pointer(&mut app, ToolEvent::Up { x: 60.0, y: 60.0 }, m);
        let s = app.distort.shape.as_ref().unwrap();
        assert_eq!(s.deformed[2], [60.0, 60.0]);
        assert!(!s.is_identity());
        let before = app.session.active().unwrap().history.past_len();
        commit(&mut app);
        assert!(app.distort.shape.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    }

    #[test]
    fn handle_mode_moves_the_layer_with_one_handle() {
        let mut app = app();
        let ctx = egui::Context::default();
        begin(&mut app, &ctx, Kind::Handle).unwrap();
        let m = egui::Modifiers::NONE;
        pointer(&mut app, ToolEvent::Down { x: 32.0, y: 32.0, pressure: 1.0 }, m);
        pointer(&mut app, ToolEvent::Move { x: 40.0, y: 32.0, pressure: 1.0 }, m);
        pointer(&mut app, ToolEvent::Up { x: 40.0, y: 32.0 }, m);
        let q = app.distort.shape.as_ref().unwrap().quad();
        assert!((q[0][0] - 24.0).abs() < 1e-9 && (q[0][1] - 16.0).abs() < 1e-9, "{q:?}");
        commit(&mut app);
        let doc = app.session.active().unwrap().doc.clone();
        let surf = doc.layers[0].surface().unwrap();
        assert!(surf.pixel(50, 30)[3] > 0.5 && surf.pixel(18, 30)[3] < 0.5);
        // The control channel drives 3D.
        begin(&mut app, &ctx, Kind::ThreeD).unwrap();
        let d = control(&mut app, &json!({"rotation": [0.0, 30.0, 0.0], "tab": "move"})).unwrap();
        assert_eq!(d["tab"], "move");
        assert!(!app.distort.shape.as_ref().unwrap().is_identity());
        control(&mut app, &json!({"cancel": true})).unwrap();
        assert!(app.distort.shape.is_none());
    }
}
