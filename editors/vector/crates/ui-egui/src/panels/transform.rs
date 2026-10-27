//! Transform panel: reference point, X/Y/W/H with the constrain link, rotate and shear, flips,
//! live-shape properties and the Scale Corners / Scale Strokes & Effects options. While the
//! Artboard tool is in use it shows and edits the active artboard's X/Y/W/H instead, and with only
//! ruler guides selected their X (vertical guides) and Y (horizontal ones).

use egui::Ui;
use redrob_vector_doc::{LiveShape, Node, NodeKind};
use redrob_vector_geom::Rect;
use redrob_vector_geom::shapes::{ArcKind, CornerKind};
use serde_json::json;

use super::{corner_radius_row, first_selected, pstate, set_pstate};
use crate::dialogs::corners::kind_label as corner_kind;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};
use crate::{VectorApp, icons};

pub const ANGLE_PRESETS: [f64; 9] = [-180.0, -135.0, -90.0, -45.0, 0.0, 45.0, 90.0, 135.0, 180.0];
/// The shear angle's presets: steep enough to be useful, short of the degenerate ±90°.
pub const SHEAR_PRESETS: [f64; 9] = [-60.0, -45.0, -30.0, -15.0, 0.0, 15.0, 30.0, 45.0, 60.0];

/// Proportional size: the other dimension when one changes with the link on.
pub fn constrained(w: f64, h: f64, new_w: Option<f64>, new_h: Option<f64>) -> (f64, f64) {
    match (new_w, new_h) {
        (Some(nw), _) if w.abs() > 1e-9 => (nw, h * nw / w),
        (_, Some(nh)) if h.abs() > 1e-9 => (w * nh / h, nh),
        (a, b) => (a.unwrap_or(w), b.unwrap_or(h)),
    }
}

/// The artboard the Artboard tool has active, while that tool is in use.
fn tool_artboard(app: &VectorApp) -> Option<(usize, Rect)> {
    if app.session.tool_id() != "artboard" {
        return None;
    }
    let i = usize::try_from(app.session.tool_options().get("active")?.as_u64()?).ok()?;
    app.session.active()?.doc.artboards.get(i).map(|a| (i, a.rect))
}

/// Where an artboard goes when the panel sets its X, Y, W or H: the reference point `refi` moves to
/// the new X/Y or stays put while the size changes (both sides together when `link` is on).
pub fn artboard_rect(r: Rect, refi: usize, x: Option<f64>, y: Option<f64>, w: Option<f64>, h: Option<f64>, link: bool) -> Rect {
    let rp = redrob_vector_geom::reference_point(r, refi);
    let (w0, h0) = (r.width(), r.height());
    let (nw, nh) = if link { constrained(w0, h0, w, h) } else { (w.unwrap_or(w0), h.unwrap_or(h0)) };
    // Where the reference point sits across the box (0, ½ or 1 of each side).
    let fx = if w0 > 1e-9 { (rp.x - r.x0) / w0 } else { 0.0 };
    let fy = if h0 > 1e-9 { (rp.y - r.y0) / h0 } else { 0.0 };
    let (x0, y0) = (x.unwrap_or(rp.x) - fx * nw, y.unwrap_or(rp.y) - fy * nh);
    Rect::new(x0, y0, x0 + nw, y0 + nh)
}

/// The Artboard tool's view of the panel: the active artboard's position and size.
fn artboard_fields(app: &mut VectorApp, ui: &mut Ui, index: usize, r: Rect) {
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    let refi: usize = ui.data(|d| d.get_temp(egui::Id::new("refpt"))).unwrap_or(4);
    let link = app.session.prefs.constrain_proportions;
    let rp = redrob_vector_geom::reference_point(r, refi);
    let set = |app: &mut VectorApp, x, y, w, h| {
        let n = artboard_rect(r, refi, x, y, w, h, link);
        let scale_art = crate::panels::artboards::scale_art(app);
        let move_art = crate::panels::artboards::move_art(app);
        let p = json!({"index": index, "x": n.x0, "y": n.y0, "width": n.width(), "height": n.height(), "scaleArt": scale_art, "moveArt": move_art});
        if let Err(e) = app.run("artboard.setProps", p) {
            app.status(e);
        }
    };
    ui.horizontal(|ui| {
        if let Some(i) = widgets::reference_point(ui, refi) {
            ui.data_mut(|d| d.insert_temp(egui::Id::new("refpt"), i));
        }
        ui.add_space(4.0);
        egui::Grid::new("xfp-ab-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
            widgets::dim_label(ui, "X:");
            if let Some(v) = widgets::num_field(ui, "xfp-ab-x", Some(rp.x - origin.x), units, 80.0) {
                set(app, Some(v + origin.x), None, None, None);
            }
            widgets::dim_label(ui, "W:");
            if let Some(v) = widgets::num_field(ui, "xfp-ab-w", Some(r.width()), units, 80.0) {
                set(app, None, None, Some(v), None);
            }
            ui.end_row();
            widgets::dim_label(ui, "Y:");
            if let Some(v) = widgets::num_field(ui, "xfp-ab-y", Some(rp.y - origin.y), units, 80.0) {
                set(app, None, Some(v + origin.y), None, None);
            }
            widgets::dim_label(ui, "H:");
            if let Some(v) = widgets::num_field(ui, "xfp-ab-h", Some(r.height()), units, 80.0) {
                set(app, None, None, None, Some(v));
            }
            ui.end_row();
        });
        constrain_link(app, ui);
    });
}

/// With ruler guides selected and no art: the X of the vertical ones and the Y of the horizontal
/// ones (the least of each; none where no such guide is selected).
fn selected_guides(app: &VectorApp) -> Option<(Option<f64>, Option<f64>)> {
    let st = app.session.active()?;
    if !st.selection.objects.is_empty() || st.selection.guides.is_empty() {
        return None;
    }
    let least = |vertical: bool| {
        st.selection.guides.iter().filter_map(|i| st.doc.guides.get(*i)).filter(|g| g.vertical == vertical).map(|g| g.pos).min_by(f64::total_cmp)
    };
    Some((least(true), least(false)))
}

/// The selected ruler guides' view of the panel (#1597): X moves the vertical ones and Y the
/// horizontal ones, together.
fn guide_fields(app: &mut VectorApp, ui: &mut Ui, (x, y): (Option<f64>, Option<f64>)) {
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    egui::Grid::new("xfp-guide-grid").num_columns(2).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
        for (label, id, pos, origin, by) in [("X:", "xfp-guide-x", x, origin.x, "dx"), ("Y:", "xfp-guide-y", y, origin.y, "dy")] {
            widgets::dim_label(ui, label);
            ui.add_enabled_ui(pos.is_some(), |ui| {
                if let (Some(v), Some(pos)) = (widgets::num_field(ui, id, pos.map(|p| p - origin), units, 80.0), pos)
                    && let Err(e) = app.run("guide.move", json!({ by: v + origin - pos }))
                {
                    app.status(e);
                }
            });
            ui.end_row();
        }
    });
}

pub fn show(app: &mut VectorApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    if app.session.active().is_none() {
        widgets::dim_label(ui, tl!("No document"));
        return;
    }
    if let Some((i, r)) = tool_artboard(app) {
        artboard_fields(app, ui, i, r);
        return;
    }
    if let Some(guides) = selected_guides(app) {
        guide_fields(app, ui, guides);
        return;
    }
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    // The bounding box, rotated with rotated objects: W/H are its own sides, X/Y its reference
    // point on the page.
    let bx = app.selection_box();
    let bounds = bx.map(|b| b.rect);
    let refi: usize = ui.data(|d| d.get_temp(egui::Id::new("refpt"))).unwrap_or(4);
    let link = app.session.prefs.constrain_proportions;
    let has = bounds.is_some();
    let rp = bx.map(|b| b.reference_point(refi));
    ui.horizontal(|ui| {
        if let Some(i) = widgets::reference_point(ui, refi) {
            ui.data_mut(|d| d.insert_temp(egui::Id::new("refpt"), i));
        }
        ui.add_space(4.0);
        ui.add_enabled_ui(has, |ui| {
            egui::Grid::new("xfp-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
                widgets::dim_label(ui, "X:");
                if let Some(v) = widgets::num_field(ui, "xfp-x", rp.map(|p| p.x - origin.x), units, 80.0) {
                    app.run("object.setBounds", json!({"x": v + origin.x, "reference": refi})).ok();
                }
                widgets::dim_label(ui, "W:");
                if let Some(v) = widgets::num_field(ui, "xfp-w", bounds.map(|b| b.width()), units, 80.0) {
                    app.run("object.setBounds", json!({"width": v, "reference": refi, "proportional": link})).ok();
                }
                ui.end_row();
                widgets::dim_label(ui, "Y:");
                if let Some(v) = widgets::num_field(ui, "xfp-y", rp.map(|p| p.y - origin.y), units, 80.0) {
                    app.run("object.setBounds", json!({"y": v + origin.y, "reference": refi})).ok();
                }
                widgets::dim_label(ui, "H:");
                if let Some(v) = widgets::num_field(ui, "xfp-h", bounds.map(|b| b.height()), units, 80.0) {
                    app.run("object.setBounds", json!({"height": v, "reference": refi, "proportional": link})).ok();
                }
                ui.end_row();
            });
        });
        constrain_link(app, ui);
    });
    ui.add_space(4.0);
    let origin = rp.map(|p| json!([p.x, p.y]));
    ui.horizontal(|ui| {
        ui.add_enabled_ui(has, |ui| {
            let icon = icons::icon(ui, "rotate-ccw", 16.0, t.icon).on_hover_text(tl!("Rotate"));
            crate::scrub::note_label(ui, icon.rect);
            // The bounding box's angle: a new value turns the selection to it.
            let angle = bx.map_or(0.0, |b| b.angle);
            if let Some(a) = widgets::spin_plain(ui, "xfp-rot", angle, "°", 2, 96.0, 15.0, -360.0, &ANGLE_PRESETS)
                && a != angle
            {
                app.run("object.rotate", json!({"angle": a, "absolute": true, "origin": origin})).ok();
            }
            ui.add_space(4.0);
            let icon = icons::icon(ui, "dc-shear", 16.0, t.icon).on_hover_text(tl!("Shear"));
            crate::scrub::note_label(ui, icon.rect);
            // Shear is relative: the field rests at 0° and a new value shears by it.
            if let Some(a) = widgets::spin_plain(ui, "xfp-shear", 0.0, "°", 2, 96.0, 15.0, -89.0, &SHEAR_PRESETS)
                && a != 0.0
                && a.abs() < 90.0
            {
                app.run("object.shear", json!({"angle": a, "axis": "horizontal", "origin": origin})).ok();
            }
        });
    });
    ui.horizontal(|ui| {
        // Lucide's flip icons are named for their mirror line: flip-vertical-2 draws a vertical
        // line (the left-right flip), flip-horizontal-2 a horizontal one (top-bottom).
        if widgets::icon_button_enabled(ui, "flip-vertical-2", tl!("Flip Horizontal"), false, has, 24.0).clicked() {
            app.run("object.reflect", json!({"axis": "vertical", "origin": origin})).ok();
        }
        if widgets::icon_button_enabled(ui, "flip-horizontal-2", tl!("Flip Vertical"), false, has, 24.0).clicked() {
            app.run("object.reflect", json!({"axis": "horizontal", "origin": origin})).ok();
        }
    });
    // Live shape properties (a path with live corners is no shape).
    if let Some(n) = first_selected(app)
        && let NodeKind::Path { live: Some(live), .. } = &n.kind
        && !matches!(**live, redrob_vector_doc::LiveShape::Path { .. })
    {
        widgets::divider(ui);
        match &**live {
            redrob_vector_doc::LiveShape::Rectangle { .. } => {
                widgets::subheader(ui, tl!("Rectangle Properties:"));
                corner_radius_row(app, ui, &n, "xfp-radius");
            }
            redrob_vector_doc::LiveShape::Polygon { .. } => {
                widgets::subheader(ui, tl!("Polygon Properties:"));
                polygon_rows(app, ui, &n, live, "xfp-polygon");
            }
            redrob_vector_doc::LiveShape::Ellipse { pie, arc, .. } => {
                widgets::subheader(ui, tl!("Ellipse Properties:"));
                pie_rows(app, ui, (*pie, *arc), "xfp-pie");
            }
            redrob_vector_doc::LiveShape::Star { .. } => {
                widgets::subheader(ui, tl!("Star Properties:"));
                ui.horizontal_wrapped(|ui| star_fields(app, ui, live, "xfp-star"));
            }
            redrob_vector_doc::LiveShape::Spiral { .. } => {
                widgets::subheader(ui, tl!("Spiral Properties:"));
                ui.horizontal_wrapped(|ui| spiral_fields(app, ui, live, "xfp-spiral"));
            }
            _ => {
                widgets::subheader(ui, tl!("Shape Properties:"));
                widgets::dim_label(ui, tl!(n.kind_label()));
            }
        }
    }
    if pstate::<bool>(ui.ctx(), "xf-hide-options") {
        return;
    }
    widgets::divider(ui);
    let (sc, ss) = (app.session.prefs.scale_corners, app.session.prefs.scale_strokes);
    if widgets::check(ui, tl!("Scale Corners"), sc, true) {
        set_pref(app, "scaleCorners", !sc);
    }
    if widgets::check(ui, tl!("Scale Strokes & Effects"), ss, true) {
        set_pref(app, "scaleStrokes", !ss);
    }
}

/// The label of an arc kind (how a cut ellipse closes).
pub(crate) fn arc_kind_label(k: ArcKind) -> &'static str {
    match k {
        ArcKind::Pie => tl!("Pie"),
        ArcKind::Chord => tl!("Chord"),
        ArcKind::Arc => tl!("Arc"),
    }
}

/// The arc kind dropdown (pie, chord or open arc) of a live ellipse cut as `arc`.
fn arc_kind_dropdown(app: &mut VectorApp, ui: &mut Ui, arc: ArcKind, id: &str) {
    let labels: Vec<&str> = ArcKind::ALL.iter().map(|k| arc_kind_label(*k)).collect();
    if let Some(k) = widgets::dropdown(ui, (id, "arc-kind"), arc_kind_label(arc), &labels, 80.0).and_then(|i| ArcKind::ALL.get(i)) {
        app.run("object.setLiveShape", json!({"arcKind": k.name()})).ok();
    }
}

/// An ellipse's Pie Start and End Angle (degrees, counterclockwise from 3 o'clock; 0 to 360 is the
/// whole ellipse), how the cut closes (pie, chord or open arc), Invert Pie, which shows the other
/// part of it, and Whole Ellipse (Transform and Properties panels).
pub(crate) fn pie_rows(app: &mut VectorApp, ui: &mut Ui, (pie, arc): ((f64, f64), ArcKind), id: &str) {
    for (label, key, v) in [(tl!("Pie Start Angle:"), "pieStart", pie.0), (tl!("Pie End Angle:"), "pieEnd", pie.1)] {
        ui.horizontal(|ui| {
            widgets::dim_label(ui, label);
            if let Some(a) = widgets::plain_field(ui, (id, key), v, "°", 1, 60.0) {
                app.run("object.setLiveShape", json!({ key: a })).ok();
            }
        });
    }
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Close As:"));
        arc_kind_dropdown(app, ui, arc, id);
    });
    ui.horizontal(|ui| {
        if widgets::flat_button(ui, tl!("Invert Pie"), 90.0).clicked() {
            app.run("object.setLiveShape", json!({"invertPie": true})).ok();
        }
        if widgets::flat_button(ui, tl!("Whole Ellipse"), 100.0).clicked() {
            app.run("object.setLiveShape", json!({"wholeEllipse": true})).ok();
        }
    });
}

/// The Control bar's live shape fields for one selected live shape: an ellipse's arc start and end
/// angles, how the cut closes and Whole Ellipse.
pub(crate) fn control_bar(app: &mut VectorApp, ui: &mut Ui) {
    // The Star tool's own options: what the next star drawn gets.
    if app.session.tool_id() == "star" && app.session.active().is_some_and(|st| st.selection.objects.is_empty()) {
        star_tool_options(app, ui);
        return;
    }
    let Some(n) = first_selected(app) else { return };
    if app.session.active().is_none_or(|st| st.selection.objects.len() != 1) {
        return;
    }
    let NodeKind::Path { live: Some(live), .. } = &n.kind else { return };
    let t = Tokens::get(ui.ctx());
    let label = |ui: &mut Ui, s: &str| widgets::field_label(ui, egui::RichText::new(s).size(12.0).color(t.text_dim));
    match &**live {
        LiveShape::Ellipse { pie, arc, .. } => {
            for (lbl, key, v) in [(tl!("Start:"), "pieStart", pie.0), (tl!("End:"), "pieEnd", pie.1)] {
                label(ui, lbl);
                if let Some(a) = widgets::plain_field(ui, ("cb-arc", key), v, "°", 1, 64.0) {
                    app.run("object.setLiveShape", json!({ key: a })).ok();
                }
            }
            arc_kind_dropdown(app, ui, *arc, "cb-arc");
            if widgets::flat_button(ui, tl!("Whole Ellipse"), 100.0).clicked() {
                app.run("object.setLiveShape", json!({"wholeEllipse": true})).ok();
            }
            ui.separator();
        }
        LiveShape::Star { .. } => {
            star_fields(app, ui, live, "cb-star");
            ui.separator();
        }
        LiveShape::Spiral { .. } => {
            spiral_fields(app, ui, live, "cb-spiral");
            ui.separator();
        }
        _ => {}
    }
}

/// A live star's fields (Control bar and panels), each an `object.setLiveShape` param: corners,
/// spoke ratio, rounding and randomisation.
pub(crate) fn star_fields(app: &mut VectorApp, ui: &mut Ui, live: &LiveShape, id: &str) {
    let LiveShape::Star { r1, r2, points, rounding, randomized, .. } = live else { return };
    let t = Tokens::get(ui.ctx());
    let label = |ui: &mut Ui, s: &str| widgets::field_label(ui, egui::RichText::new(s).size(12.0).color(t.text_dim));
    label(ui, tl!("Corners:"));
    if let Some(v) = widgets::plain_field(ui, (id, "points"), f64::from(*points), "", 0, 48.0) {
        app.run("object.setLiveShape", json!({"points": v.round().clamp(2.0, 1000.0) as u64})).ok();
    }
    let ratio = if *r1 > 1e-12 { r2 / r1 } else { 0.5 };
    for (lbl, key, v) in
        [(tl!("Spoke Ratio:"), "spokeRatio", ratio), (tl!("Rounded:"), "rounding", *rounding), (tl!("Randomized:"), "randomized", *randomized)]
    {
        label(ui, lbl);
        if let Some(v) = widgets::plain_field(ui, (id, key), v * 100.0, "%", 0, 56.0) {
            app.run("object.setLiveShape", json!({ key: (v / 100.0).clamp(0.0, 1.0) })).ok();
        }
    }
}

/// A live spiral's fields (Control bar and panels): decay and segments.
pub(crate) fn spiral_fields(app: &mut VectorApp, ui: &mut Ui, live: &LiveShape, id: &str) {
    let LiveShape::Spiral { decay, segments, .. } = live else { return };
    let t = Tokens::get(ui.ctx());
    let label = |ui: &mut Ui, s: &str| widgets::field_label(ui, egui::RichText::new(s).size(12.0).color(t.text_dim));
    label(ui, tl!("Decay:"));
    if let Some(v) = widgets::plain_field(ui, (id, "decay"), *decay, "%", 1, 56.0) {
        app.run("object.setLiveShape", json!({"spiralDecay": v.clamp(5.0, 99.99)})).ok();
    }
    label(ui, tl!("Segments:"));
    if let Some(v) = widgets::plain_field(ui, (id, "segments"), f64::from(*segments), "", 0, 48.0) {
        app.run("object.setLiveShape", json!({"segments": v.round().clamp(2.0, 1000.0) as u64})).ok();
    }
}

/// The Star tool's options in the Control bar (`tool.setOption`): corners, spoke ratio, rounding
/// and randomisation of the next star drawn.
fn star_tool_options(app: &mut VectorApp, ui: &mut Ui) {
    let opts = app.session.tool_options();
    let t = Tokens::get(ui.ctx());
    let label = |ui: &mut Ui, s: &str| widgets::field_label(ui, egui::RichText::new(s).size(12.0).color(t.text_dim));
    label(ui, tl!("Corners:"));
    if let Some(v) = widgets::plain_field(ui, ("cb-startool", "points"), opts["points"].as_f64().unwrap_or(5.0), "", 0, 48.0) {
        app.run("tool.setOption", json!({"key": "points", "value": v.round().clamp(3.0, 1000.0) as u64})).ok();
    }
    for (lbl, key, default) in [(tl!("Spoke Ratio:"), "starRatio", 0.5), (tl!("Rounded:"), "rounding", 0.0), (tl!("Randomized:"), "randomized", 0.0)]
    {
        label(ui, lbl);
        let v = opts[key].as_f64().unwrap_or(default);
        if let Some(v) = widgets::plain_field(ui, ("cb-startool", key), v * 100.0, "%", 0, 56.0) {
            app.run("tool.setOption", json!({"key": key, "value": (v / 100.0).clamp(0.0, 1.0)})).ok();
        }
    }
    ui.separator();
}

/// A live polygon's Polygon Properties (Transform and Properties panels): its side count, angle
/// (counterclockwise), corner type and radius, radius (centre to vertex) and side length, and Make
/// Sides Equal, which an uneven scale enables.
pub(crate) fn polygon_rows(app: &mut VectorApp, ui: &mut Ui, n: &Node, live: &LiveShape, id: &str) {
    let LiveShape::Polygon { sides, .. } = live else { return };
    let (units, fw) = (app.session.general_unit(), super::field_width(ui));
    let radius = live.polygon_radius();
    let side = radius.map(|r| 2.0 * r * (std::f64::consts::PI / f64::from((*sides).max(3))).sin());
    let kind = super::corner_style(app, n).1;
    let kinds: Vec<&str> = CornerKind::ALL.iter().map(|k| corner_kind(*k)).collect();
    // The `object.setLiveShape` param an edited field sets.
    let mut set = None;
    egui::Grid::new((id, "grid")).num_columns(2).spacing([6.0, 6.0]).show(ui, |ui| {
        widgets::dim_label(ui, tl!("Sides:"));
        if let Some(s) = widgets::plain_field(ui, (id, "sides"), f64::from(*sides), "", 0, fw) {
            set = Some(("sides", json!(s.round().max(3.0) as u64)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Angle:"));
        if let Some(a) = widgets::plain_field(ui, (id, "angle"), live.polygon_angle().unwrap_or(0.0), "°", 1, fw) {
            set = Some(("polygonAngle", json!(a)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Corner:"));
        if let Some(k) = widgets::dropdown_names(ui, (id, "kind"), kind.map_or("", corner_kind), &kinds, fw).and_then(|i| CornerKind::ALL.get(i)) {
            set = Some(("kind", json!(k)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Corner Radius:"));
        super::corner_radius_field(app, ui, n, (id, "cornerRadius"), fw);
        ui.end_row();
        widgets::dim_label(ui, tl!("Radius:"));
        if let Some(r) = widgets::num_field(ui, (id, "radius"), radius, units, fw) {
            set = Some(("polygonRadius", json!(r)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Side Length:"));
        if let Some(l) = widgets::num_field(ui, (id, "side"), side, units, fw) {
            set = Some(("sideLength", json!(l)));
        }
        ui.end_row();
    });
    if ui.add_enabled_ui(!live.polygon_sides_equal(), |ui| widgets::flat_button(ui, tl!("Make Sides Equal"), 130.0)).inner.clicked() {
        set = Some(("makeSidesEqual", json!(true)));
    }
    if let Some((key, v)) = set {
        app.run("object.setLiveShape", json!({ key: v })).ok();
    }
}

/// The Control bar's underlined "Transform" link: a click toggles the Transform panel in a
/// popover under it, as the Stroke link does with the Stroke panel.
pub fn link(app: &mut VectorApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let resp = ui.link(egui::RichText::new(tl!("Transform")).size(12.0).color(t.text).underline()).on_hover_text(tl!("Transform options"));
    widgets::popover(&resp, resp.clicked(), |ui| {
        ui.set_width(300.0);
        show(app, ui);
    });
}

/// The link between W and H (Transform panel, Properties panel, Control bar): one toggle, the
/// `constrainProportions` preference, which the size fields pass on as `proportional`.
pub fn constrain_link(app: &mut VectorApp, ui: &mut Ui) {
    let on = app.session.prefs.constrain_proportions;
    if widgets::icon_button(ui, if on { "link" } else { "link-2-off" }, tl!("Constrain Width and Height Proportions"), on, 22.0).clicked() {
        set_pref(app, "constrainProportions", !on);
    }
}

/// Toggle a boolean preference (through `prefs.set`, as agents do).
pub fn set_pref(app: &mut VectorApp, key: &str, on: bool) {
    if let Err(e) = app.run("prefs.set", json!({ "key": key, "value": on })) {
        app.status(e);
    }
}

pub fn menu(app: &mut VectorApp, ui: &mut Ui) {
    let hidden: bool = pstate(ui.ctx(), "xf-hide-options");
    let has = super::selection_len(app) > 0;
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), "xf-hide-options", !hidden);
    }
    ui.separator();
    if menu_item(ui, tl!("Flip Horizontal"), has, false) {
        app.run("object.reflect", json!({"axis": "vertical"})).ok();
    }
    if menu_item(ui, tl!("Flip Vertical"), has, false) {
        app.run("object.reflect", json!({"axis": "horizontal"})).ok();
    }
    ui.separator();
    let ss = app.session.prefs.scale_strokes;
    if menu_item(ui, tl!("Scale Strokes & Effects"), true, ss) {
        set_pref(app, "scaleStrokes", !ss);
    }
    ui.separator();
    menu_item(ui, tl!("Transform Object Only"), false, true);
    menu_item(ui, tl!("Transform Pattern Only"), false, false);
    menu_item(ui, tl!("Transform Both"), false, false);
    ui.separator();
    menu_item(ui, tl!("Use Registration Point for Symbol"), false, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One headless frame of the panel (or the Align panel's menu): the texts drawn.
    fn texts(app: &mut VectorApp, draw: fn(&mut VectorApp, &mut Ui)) -> Vec<String> {
        fn walk(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| draw(app, ui));
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    }

    #[test]
    fn use_preview_bounds_shows_the_visual_size_and_checks_the_align_flyout() {
        let mut app = VectorApp::new(redrob_vector_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
        app.run("stroke.set", json!({"weight": 10})).unwrap();
        assert!(texts(&mut app, show).iter().any(|t| t == "100 pt"));
        assert!(texts(&mut app, crate::panels::align::menu).iter().any(|t| t == "   Use Preview Bounds"));
        set_pref(&mut app, "usePreviewBounds", true);
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "110 pt") && shown.iter().any(|t| t == "60 pt"), "{shown:?}");
        assert!(texts(&mut app, crate::panels::align::menu).iter().any(|t| t == "✓ Use Preview Bounds"));
    }

    #[test]
    fn rotation_field_shows_the_angle_the_box_keeps() {
        let mut app = VectorApp::new(redrob_vector_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
        app.run("object.rotate", json!({"angle": 45, "absolute": true})).unwrap();
        let shown = texts(&mut app, show);
        // The angle stays, and W/H are the rectangle's own sides.
        assert!(shown.iter().any(|t| t == "45°"), "{shown:?}");
        assert!(shown.iter().any(|t| t == "100 pt") && shown.iter().any(|t| t == "50 pt"), "{shown:?}");
        app.run("object.resetBoundingBox", json!({})).unwrap();
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "0°") && !shown.iter().any(|t| t == "100 pt"), "{shown:?}");
    }

    /// #812: a selected live polygon shows its Polygon Properties in the Transform panel and the
    /// Properties panel's Transform section, and Make Sides Equal evens it out after a stretch.
    #[test]
    fn a_polygon_shows_its_polygon_properties() {
        let mut app = VectorApp::new(redrob_vector_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.polygon", json!({"cx": 150, "cy": 150, "radius": 50, "sides": 6})).unwrap();
        for draw in [show as fn(&mut VectorApp, &mut Ui), crate::panels::properties::transform_section] {
            let shown = texts(&mut app, draw);
            // A hexagon's side is as long as its radius.
            for t in ["Sides:", "6", "Angle:", "0°", "Corner:", "Round", "Corner Radius:", "Radius:", "Side Length:", "50 pt", "Make Sides Equal"] {
                assert!(shown.iter().any(|s| s == t), "{t} in {shown:?}");
            }
        }
        assert!(texts(&mut app, show).iter().any(|s| s == "Polygon Properties:"));
        app.run("object.scale", json!({"sx": 200, "sy": 100})).unwrap();
        let live = |app: &VectorApp| match &first_selected(app).unwrap().kind {
            NodeKind::Path { live: Some(l), .. } => l.clone(),
            k => panic!("{k:?}"),
        };
        assert!(!live(&app).polygon_sides_equal());
        app.run("object.setLiveShape", json!({"makeSidesEqual": true})).unwrap();
        assert!(live(&app).polygon_sides_equal());
    }

    #[test]
    fn the_artboard_tool_shows_the_active_artboard() {
        let mut app = VectorApp::new(redrob_vector_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
        assert!(texts(&mut app, show).iter().any(|t| t == "40 pt"), "the selection first");
        app.run("tool.select", json!({"tool": "artboard"})).unwrap();
        let shown = texts(&mut app, show);
        // Centre reference point: X 150, Y 100, W 300, H 200; no rotate/shear for artboards.
        for v in ["150 pt", "100 pt", "300 pt", "200 pt"] {
            assert!(shown.iter().any(|t| t == v), "{v} in {shown:?}");
        }
        assert!(!shown.iter().any(|t| t == "40 pt" || t == "0°"), "{shown:?}");
    }

    /// #1597: with only ruler guides selected the panel shows their X or Y, and nothing of the art.
    #[test]
    fn selected_guides_show_their_position() {
        let mut app = VectorApp::new(redrob_vector_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("guide.add", json!({"vertical": true, "pos": 120})).unwrap();
        app.run("guide.add", json!({"vertical": false, "pos": 45})).unwrap();
        app.run("guide.select", json!({"indexes": [0]})).unwrap();
        assert_eq!(selected_guides(&app), Some((Some(120.0), None)));
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "120 pt") && !shown.iter().any(|t| t == "W:" || t == "45 pt"), "{shown:?}");
        app.run("guide.select", json!({"indexes": [0, 1]})).unwrap();
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "120 pt") && shown.iter().any(|t| t == "45 pt"), "{shown:?}");
        // The field's edit: the vertical guide moves, the horizontal one stays.
        app.run("guide.move", json!({"dx": 30})).unwrap();
        assert_eq!(selected_guides(&app), Some((Some(150.0), Some(45.0))));
        // Art selected with them: the art's box, as before.
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
        assert_eq!(selected_guides(&app), None);
    }

    /// #1824: with an anchor selected with Direct Selection the panel shows that anchor's position,
    /// and a new X moves it alone.
    #[test]
    fn an_anchor_selection_shows_and_moves_its_own_box() {
        let mut app = VectorApp::new(redrob_vector_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let square = json!({"anchors": [{"x": 20, "y": 30}, {"x": 120, "y": 30}, {"x": 120, "y": 130}, {"x": 20, "y": 130}], "closed": true});
        let id = app.run("path.create", square).unwrap()["id"].as_u64().unwrap();
        let shown = texts(&mut app, show);
        assert!(shown.iter().any(|t| t == "70 pt") && shown.iter().any(|t| t == "100 pt"), "the whole path: {shown:?}");
        app.run("select.anchors", json!({"id": id, "anchors": [[0, 1]]})).unwrap();
        let shown = texts(&mut app, show);
        for v in ["120 pt", "30 pt", "0 pt"] {
            assert!(shown.iter().any(|t| t == v), "{v} in {shown:?}");
        }
        assert!(!shown.iter().any(|t| t == "70 pt" || t == "100 pt"), "{shown:?}");
        app.run("object.setBounds", json!({"x": 150, "reference": 4})).unwrap();
        let n = first_selected(&app).unwrap();
        let Some(path) = n.path_data() else { panic!("a path") };
        let xs: Vec<f64> = path.subpaths[0].anchors.iter().map(|a| a.p.x).collect();
        assert_eq!(xs, [20.0, 150.0, 120.0, 20.0]);
    }

    #[test]
    fn artboard_fields_keep_the_reference_point() {
        let r = Rect::new(0.0, 0.0, 300.0, 200.0);
        // Centre: a new X centres it there, a new W grows it both ways.
        assert_eq!(artboard_rect(r, 4, Some(200.0), None, None, None, false), Rect::new(50.0, 0.0, 350.0, 200.0));
        assert_eq!(artboard_rect(r, 4, None, None, Some(400.0), None, false), Rect::new(-50.0, 0.0, 350.0, 200.0));
        // Top-left: a new W keeps the left edge; linked, H follows.
        assert_eq!(artboard_rect(r, 0, None, None, Some(600.0), None, true), Rect::new(0.0, 0.0, 600.0, 400.0));
        // Bottom-right: a new Y puts the bottom edge there.
        assert_eq!(artboard_rect(r, 8, None, Some(500.0), None, None, false), Rect::new(0.0, 300.0, 300.0, 500.0));
    }

    #[test]
    fn constrain_proportions() {
        assert_eq!(constrained(100.0, 50.0, Some(200.0), None), (200.0, 100.0));
        assert_eq!(constrained(100.0, 50.0, None, Some(25.0)), (50.0, 25.0));
        assert_eq!(constrained(0.0, 50.0, Some(10.0), None), (10.0, 50.0));
    }
}
