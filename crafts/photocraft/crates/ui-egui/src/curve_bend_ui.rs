//! Curve Bend's curve editors (PH-FLT-042): the filter dialog's `upperCurve` and `lowerCurve`
//! are drawn here rather than typed. Each graph is the edge of the layer: the horizontal axis
//! runs along the edge, the vertical axis moves it up to half the layer's height either way
//! (the middle line is the straight edge). Click to add a point, drag to move, right-click or
//! drag off the graph to delete; arrows nudge the selected point.

use egui::{Pos2, Sense, Stroke, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::state::PointCurveState;
use crate::theme::Tokens;

const UPPER: &str = "upperCurve";
const LOWER: &str = "lowerCurve";

/// The curve stored in `f[key]` as `[[x, y], …]` in 0..1, as editor points in 0..255 (sorted,
/// endpoints present).
fn points_of(f: &Map<String, Value>, key: &str) -> Vec<[f32; 2]> {
    let mut pts: Vec<[f32; 2]> = f
        .get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let q = e.as_array()?;
                    let x = q.first()?.as_f64()? as f32;
                    let y = q.get(1)?.as_f64()? as f32;
                    (x.is_finite() && y.is_finite()).then_some([(x * 255.0).clamp(0.0, 255.0), (y * 255.0).clamp(0.0, 255.0)])
                })
                .collect()
        })
        .unwrap_or_default();
    if pts.len() < 2 {
        pts = vec![[0.0, 127.5], [255.0, 127.5]];
    }
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
    pts
}

fn store(f: &mut Map<String, Value>, key: &str, pts: &[[f32; 2]]) {
    let v: Vec<Value> = pts.iter().map(|p| json!([(p[0] / 255.0 * 1000.0).round() / 1000.0, (p[1] / 255.0 * 1000.0).round() / 1000.0])).collect();
    f.insert(key.into(), Value::Array(v));
}

/// Catmull-Rom through the points (what the filter draws with smoothing on), sampled at `t`
/// (0..255 input), as a 0..255 output.
fn curve_at(pts: &[[f32; 2]], t: f32) -> f32 {
    if pts.len() < 2 {
        return 127.5;
    }
    if t <= pts[0][0] {
        return pts[0][1];
    }
    let last = pts.len() - 1;
    if t >= pts[last][0] {
        return pts[last][1];
    }
    let i = pts.windows(2).position(|w| t >= w[0][0] && t <= w[1][0]).unwrap_or(0);
    let (p1, p2) = (pts[i], pts[i + 1]);
    let span = (p2[0] - p1[0]).max(1e-6);
    let s = ((t - p1[0]) / span).clamp(0.0, 1.0);
    let p0 = if i > 0 { pts[i - 1] } else { p1 };
    let p3 = if i + 2 <= last { pts[i + 2] } else { p2 };
    let m1 = if i > 0 { (p2[1] - p0[1]) / (p2[0] - p0[0]).max(1e-6) * span } else { p2[1] - p1[1] };
    let m2 = if i + 2 <= last { (p3[1] - p1[1]) / (p3[0] - p1[0]).max(1e-6) * span } else { p2[1] - p1[1] };
    let (s2, s3) = (s * s, s * s * s);
    ((2.0 * s3 - 3.0 * s2 + 1.0) * p1[1] + (s3 - 2.0 * s2 + s) * m1 + (-2.0 * s3 + 3.0 * s2) * p2[1] + (s3 - s2) * m2).clamp(0.0, 255.0)
}

/// One edge's graph. Returns whether the points changed.
fn graph(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, title: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Reset")).clicked() {
                f.remove(key);
            }
        });
    });
    let width = ui.available_width().min(260.0);
    let height = (width * 0.5).max(60.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.radius_sm, t.field);
    // Quarter grid and the straight-edge midline.
    for i in 1..4 {
        let fx = rect.left() + i as f32 / 4.0 * width;
        painter.line_segment([pos2(fx, rect.top()), pos2(fx, rect.bottom())], Stroke::new(1.0, t.separator));
    }
    for i in 1..4 {
        let fy = rect.top() + i as f32 / 4.0 * height;
        let stroke = if i == 2 { Stroke::new(1.0, t.text_dim) } else { Stroke::new(1.0, t.separator) };
        painter.line_segment([pos2(rect.left(), fy), pos2(rect.right(), fy)], stroke);
    }
    let mut pts = points_of(f, key);
    let id = egui::Id::new(("curve-bend-state", key));
    let mut state: PointCurveState = ui.ctx().data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let changed = crate::point_curve::interact(ui, &resp, rect, &mut pts, &mut state).changed;
    ui.ctx().data_mut(|d| d.insert_temp(id, state));
    let to_screen = |x: f32, y: f32| pos2(rect.left() + x / 255.0 * width, rect.bottom() - y / 255.0 * height);
    let smooth = f.get("smoothing").and_then(Value::as_bool).unwrap_or(true);
    let line: Vec<Pos2> = if smooth {
        (0..=128).map(|k| {
            let x = k as f32 * 2.0;
            to_screen(x, curve_at(&pts, x.min(255.0)))
        })
        .collect()
    } else {
        pts.iter().map(|p| to_screen(p[0], p[1])).collect()
    };
    painter.add(egui::Shape::line(line, Stroke::new(1.5, t.text)));
    for (i, p) in pts.iter().enumerate() {
        let c = to_screen(p[0], p[1]);
        if state.selected == Some(i) {
            painter.circle_filled(c, 3.0, t.accent);
        }
        painter.circle_stroke(c, 4.0, Stroke::new(1.5, t.accent));
    }
    if changed || !f.contains_key(key) {
        store(f, key, &pts);
    }
    changed
}

/// The two curve editors plus copy / swap buttons, below the dialog's generated fields.
pub(crate) fn controls(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.add_space(6.0);
    crate::widgets::section_label(ui, tl!("Curves"));
    graph(ui, f, UPPER, tl!("Upper"));
    ui.add_space(4.0);
    graph(ui, f, LOWER, tl!("Lower"));
    ui.horizontal(|ui| {
        if ui.button(tl!("Copy upper to lower")).clicked() {
            let upper = points_of(f, UPPER);
            store(f, LOWER, &upper);
        }
        if ui.button(tl!("Swap curves")).clicked() {
            let (u, l) = (points_of(f, UPPER), points_of(f, LOWER));
            store(f, UPPER, &l);
            store(f, LOWER, &u);
        }
        if ui.button(tl!("Mirror")).clicked() {
            let upper = points_of(f, UPPER);
            let mirrored: Vec<[f32; 2]> = upper.iter().map(|p| [p[0], 255.0 - p[1]]).collect();
            store(f, LOWER, &mirrored);
        }
    });
}
