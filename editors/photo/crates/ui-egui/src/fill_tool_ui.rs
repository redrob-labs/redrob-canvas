//! The Paint Bucket's fill modes, reference layers, grow stops, feather and drag fill, and the
//! Enclose and Fill tool: options-bar controls, the click/drag gestures and the enclosing shape's
//! preview (`paint.bucket`, `paint.bucketDrag`, `paint.encloseFill`).

use egui::Stroke;
use serde_json::{Map, Value, json};

use crate::{
    PhotoApp,
    canvas::{Drag, ViewXform},
    state::{Tool, ToolOptions},
    theme::Tokens,
    widgets,
};

/// Every fill mode: (`fillMode` value, label).
pub const FILL_MODES: [(&str, &str); 3] = [("contiguous", "Contiguous"), ("similar", "Similar Colours"), ("selection", "Selection")];
/// Every reference: (`reference` value, label).
pub const REFERENCES: [(&str, &str); 3] = [("current", "Current Layer"), ("all", "All Layers"), ("labels", "Colour Labels")];
/// Every enclosing shape.
pub const ENCLOSE_SHAPES: [(&str, &str); 3] = [("rectangle", "Rectangle"), ("ellipse", "Ellipse"), ("lasso", "Lasso")];
/// Every Enclose and Fill rule.
pub const ENCLOSE_RULES: [(&str, &str); 4] =
    [("all", "All Regions"), ("color", "Specific Colour"), ("transparent", "Transparent Regions"), ("surroundedBy", "Surrounded By")];

fn opt(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).color(t.text_dim).size(12.0));
}

fn hex(c: [f32; 3]) -> String {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(c[0]), q(c[1]), q(c[2]))
}

fn color_button(ui: &mut egui::Ui, c: &mut [f32; 3]) -> bool {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut col = egui::Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]));
    if widgets::color_edit_button_srgba(ui, &mut col).changed() {
        *c = [f32::from(col.r()) / 255.0, f32::from(col.g()) / 255.0, f32::from(col.b()) / 255.0];
        return true;
    }
    false
}

/// The params the bucket, drag fill and enclose fill share: tolerance, anti-alias, the reference
/// layers, growing and feathering, the contents and the target.
pub fn common_params(app: &PhotoApp) -> Map<String, Value> {
    let o = &app.ui.tool_options;
    let contents = if o.bucket_fill_pattern {
        "pattern"
    } else if o.bucket_fill_background {
        "background"
    } else {
        "foreground"
    };
    let mut m = Map::new();
    m.insert("tolerance".into(), json!(o.tolerance));
    m.insert("antiAlias".into(), json!(o.anti_alias));
    m.insert("diagonal".into(), json!(o.wand_diagonal));
    m.insert("criterion".into(), json!(o.wand_criterion));
    m.insert("reference".into(), json!(if o.bucket_reference.is_empty() { "current" } else { o.bucket_reference.as_str() }));
    if o.bucket_reference == "labels" {
        m.insert("labels".into(), json!(o.bucket_labels));
    }
    m.insert("opacity".into(), json!(o.fill_opacity));
    m.insert("contents".into(), json!(contents));
    m.insert("scale".into(), json!(o.bucket_pattern_scale));
    m.insert("angle".into(), json!(o.bucket_pattern_angle));
    m.insert("spread".into(), json!(o.bucket_spread));
    m.insert("growStopDarkest".into(), json!(o.bucket_grow_stop_darkest));
    m.insert("growStopOpaque".into(), json!(o.bucket_grow_stop_opaque));
    m.insert("feather".into(), json!(o.bucket_feather));
    m.insert("target".into(), crate::canvas::paint_target(app));
    m
}

/// The Paint Bucket's click: a plain fill, or the start of a drag fill when Drag Fill is on
/// (the release fills every region the drag passed over). Returns true when a drag began.
pub fn bucket_down(app: &mut PhotoApp, x: f64, y: f64, mods: egui::Modifiers) -> bool {
    let o = app.ui.tool_options.clone();
    if o.bucket_drag_fill {
        app.drag = Some(Drag::new(Tool::PaintBucket, [x, y], vec![[x, y, 1.0]], mods, false));
        return true;
    }
    let mut m = common_params(app);
    m.insert("x".into(), json!(x.floor()));
    m.insert("y".into(), json!(y.floor()));
    m.insert(
        "fillMode".into(),
        json!(if o.bucket_fill_mode.is_empty() { if o.contiguous { "contiguous" } else { "similar" } } else { o.bucket_fill_mode.as_str() }),
    );
    m.insert("lineArt".into(), json!(o.bucket_line_art));
    m.insert("lineArtThreshold".into(), json!(o.line_art_threshold));
    m.insert("lineArtGap".into(), json!(o.line_art_gap));
    m.insert("lineArtGrow".into(), json!(o.line_art_grow));
    m.insert("closeGap".into(), json!(o.bucket_close_gap));
    m.insert("boundaryColor".into(), if o.bucket_use_boundary { json!(o.bucket_boundary_color.to_vec()) } else { Value::Null });
    let _ = crate::mirror_tools_ui::run_mirrored(app, "paint.bucket", Value::Object(m));
    false
}

/// The drag fill's release: every region the path passed over, once.
pub fn finish_bucket_drag(app: &mut PhotoApp, d: &Drag) {
    let pts: Vec<[f64; 2]> = std::iter::once([d.start[0], d.start[1]]).chain(d.points.iter().map(|p| [p[0], p[1]])).collect();
    let mut m = common_params(app);
    m.insert("points".into(), json!(pts));
    m.insert("similarOnly".into(), json!(app.ui.tool_options.bucket_drag_similar));
    let _ = crate::mirror_tools_ui::run_mirrored(app, "paint.bucketDrag", Value::Object(m));
}

/// The Enclose and Fill tool's release: the drawn shape (a rectangle or ellipse from the drag's
/// corners, or the lasso path) and its rule.
pub fn finish_enclose(app: &mut PhotoApp, d: &Drag) {
    let o = app.ui.tool_options.clone();
    let end = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
    let mut m = common_params(app);
    match o.enclose_shape.as_str() {
        "lasso" => {
            let pts: Vec<[f64; 2]> = std::iter::once(d.start).chain(d.points.iter().map(|p| [p[0], p[1]])).collect();
            if pts.len() < 3 {
                return;
            }
            m.insert("shape".into(), json!("polygon"));
            m.insert("points".into(), json!(pts));
        }
        shape => {
            if (end[0] - d.start[0]).abs() < 1.0 || (end[1] - d.start[1]).abs() < 1.0 {
                return;
            }
            m.insert("shape".into(), json!(if shape == "ellipse" { "ellipse" } else { "rectangle" }));
            m.insert("from".into(), json!(d.start));
            m.insert("to".into(), json!(end));
        }
    }
    m.insert("rule".into(), json!(if o.enclose_rule.is_empty() { "all" } else { o.enclose_rule.as_str() }));
    if matches!(o.enclose_rule.as_str(), "color" | "surroundedBy") {
        m.insert("ruleColor".into(), json!(hex(o.enclose_rule_color)));
    }
    m.insert("excludeTransparent".into(), json!(o.enclose_exclude_transparent));
    if o.enclose_exclude_color_on {
        m.insert("excludeColor".into(), json!(hex(o.enclose_exclude_color)));
    }
    let r = crate::mirror_tools_ui::run_mirrored(app, "paint.encloseFill", Value::Object(m));
    if let Ok(v) = &r
        && v.get("filled") == Some(&Value::Bool(false))
    {
        app.ui.status = tl!("No region is fully enclosed by the shape").into();
        app.ui.status_error = false;
    }
}

/// The Paint Bucket's extra options: fill mode, reference layers, feather, grow stops, drag fill.
pub fn bucket_options(ui: &mut egui::Ui, o: &mut ToolOptions) {
    widgets::vline(ui, 22.0);
    opt(ui, tl!("Fill mode"));
    if o.bucket_fill_mode.is_empty() {
        o.bucket_fill_mode = if o.contiguous { "contiguous" } else { "similar" }.into();
    }
    let modes: Vec<(String, &str)> = FILL_MODES.iter().map(|(k, l)| (k.to_string(), tl!(*l))).collect();
    if widgets::dropdown_literal(ui, "bucket-fill-mode", &mut o.bucket_fill_mode, &modes, 130.0) {
        o.contiguous = o.bucket_fill_mode != "similar";
    }
    reference_options(ui, o);
    grow_options(ui, o);
    widgets::checkbox(ui, &mut o.mirror_all_tools, tl!("Mirror")).on_hover_text(tl!("The painting symmetry mirrors this tool's output too"));
    widgets::checkbox(ui, &mut o.bucket_drag_fill, tl!("Drag fill")).on_hover_text(tl!("Dragging fills every region passed over"));
    if o.bucket_drag_fill {
        widgets::checkbox(ui, &mut o.bucket_drag_similar, tl!("Similar only")).on_hover_text(tl!("Only regions coloured like the first one"));
    }
}

fn reference_options(ui: &mut egui::Ui, o: &mut ToolOptions) {
    opt(ui, tl!("Reference"));
    if o.bucket_reference.is_empty() {
        o.bucket_reference = if o.sample_all_layers { "all" } else { "current" }.into();
    }
    let refs: Vec<(String, &str)> = REFERENCES.iter().map(|(k, l)| (k.to_string(), tl!(*l))).collect();
    if widgets::dropdown_literal(ui, "bucket-reference", &mut o.bucket_reference, &refs, 120.0) {
        o.sample_all_layers = o.bucket_reference != "current";
    }
    if o.bucket_reference == "labels" {
        for color in redrob_photo_doc::LabelColor::ALL {
            if color == redrob_photo_doc::LabelColor::None {
                continue;
            }
            let id = color.id().to_string();
            let mut on = o.bucket_labels.contains(&id);
            if widgets::checkbox(ui, &mut on, tl!(color.label())).changed() {
                if on {
                    o.bucket_labels.push(id);
                } else {
                    o.bucket_labels.retain(|l| *l != id);
                }
            }
        }
    }
}

fn grow_options(ui: &mut egui::Ui, o: &mut ToolOptions) {
    opt(ui, tl!("Feather"));
    widgets::value_field(ui, &mut o.bucket_feather, 0.0..=64.0, "px", 52.0).on_hover_text(tl!("Soften the filled edge"));
    if o.bucket_spread > 0.0 {
        widgets::checkbox(ui, &mut o.bucket_grow_stop_darkest, tl!("Stop at darkest")).on_hover_text(tl!("Spread stops at the darkest pixels (lines)"));
        widgets::checkbox(ui, &mut o.bucket_grow_stop_opaque, tl!("Stop at opaque")).on_hover_text(tl!("Spread stops at the most opaque pixels"));
    }
}

/// The Enclose and Fill tool's options bar. Returns false for other tools.
pub fn enclose_options_bar(app: &mut PhotoApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::EncloseFill {
        return false;
    }
    let o = &mut app.ui.tool_options;
    opt(ui, tl!("Shape"));
    if o.enclose_shape.is_empty() {
        o.enclose_shape = "rectangle".into();
    }
    let shapes: Vec<(String, &str)> = ENCLOSE_SHAPES.iter().map(|(k, l)| (k.to_string(), tl!(*l))).collect();
    widgets::dropdown_literal(ui, "enclose-shape", &mut o.enclose_shape, &shapes, 110.0);
    opt(ui, tl!("Fill"));
    if o.enclose_rule.is_empty() {
        o.enclose_rule = "all".into();
    }
    let rules: Vec<(String, &str)> = ENCLOSE_RULES.iter().map(|(k, l)| (k.to_string(), tl!(*l))).collect();
    widgets::dropdown_literal(ui, "enclose-rule", &mut o.enclose_rule, &rules, 150.0);
    if matches!(o.enclose_rule.as_str(), "color" | "surroundedBy") {
        color_button(ui, &mut o.enclose_rule_color);
    }
    widgets::vline(ui, 22.0);
    widgets::checkbox(ui, &mut o.enclose_exclude_transparent, tl!("Exclude transparent"));
    widgets::checkbox(ui, &mut o.enclose_exclude_color_on, tl!("Exclude colour"));
    if o.enclose_exclude_color_on {
        color_button(ui, &mut o.enclose_exclude_color);
    }
    widgets::vline(ui, 22.0);
    opt(ui, tl!("Tolerance"));
    widgets::value_field(ui, &mut o.tolerance, 0.0..=255.0, "", 58.0);
    widgets::checkbox(ui, &mut o.anti_alias, tl!("Anti-alias"));
    reference_options(ui, o);
    opt(ui, tl!("Spread"));
    widgets::value_field(ui, &mut o.bucket_spread, 0.0..=64.0, "px", 52.0);
    grow_options(ui, o);
    opt(ui, tl!("Opacity"));
    widgets::value_field(ui, &mut o.fill_opacity, 1.0..=100.0, "%", 62.0);
    true
}

/// The enclosing shape while it is dragged, and the drag fill's path.
pub fn draw_overlay(app: &PhotoApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(d) = app.drag.as_ref() else { return };
    let t = Tokens::get(painter.ctx());
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
    let screen = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let stroke = Stroke::new(1.0, t.accent);
    match d.tool {
        Tool::EncloseFill => {
            let end = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
            match app.ui.tool_options.enclose_shape.as_str() {
                "lasso" => {
                    let pts: Vec<egui::Pos2> = std::iter::once(screen(d.start)).chain(d.points.iter().map(|p| screen([p[0], p[1]]))).collect();
                    if pts.len() >= 2 {
                        painter.add(egui::Shape::closed_line(pts, stroke));
                    }
                }
                "ellipse" => {
                    let r = egui::Rect::from_two_pos(screen(d.start), screen(end));
                    painter.add(egui::Shape::ellipse_stroke(r.center(), r.size() / 2.0, stroke));
                }
                _ => {
                    painter.rect_stroke(egui::Rect::from_two_pos(screen(d.start), screen(end)), 0.0, stroke, egui::StrokeKind::Outside);
                }
            }
        }
        Tool::PaintBucket if app.ui.tool_options.bucket_drag_fill => {
            let pts: Vec<egui::Pos2> = std::iter::once(screen(d.start)).chain(d.points.iter().map(|p| screen([p[0], p[1]]))).collect();
            if pts.len() >= 2 {
                painter.add(egui::Shape::line(pts, stroke));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_and_hex() {
        assert_eq!(hex([1.0, 0.0, 0.5]), "#ff0080");
        assert!(FILL_MODES.iter().any(|(k, _)| *k == "selection"));
        assert!(ENCLOSE_RULES.iter().any(|(k, _)| *k == "surroundedBy"));
    }
}
