//! The pop-up palette: a right-click (Preferences › Tools › Right-click with painting tools ›
//! Popup Palette) opens a radial palette under the pointer with the tagged brush presets around a
//! hue ring and a saturation/brightness field, the colour history between them, the foreground
//! and background chips, and the view's zoom, rotation and mirror controls beneath. A press
//! outside, Escape or picking a preset closes it.

use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use redrob_photo_engine::paint::BrushPreset;
use serde_json::json;

use crate::{PhotocraftApp, brush_preview, color_history, color_picker_ui, theme::Tokens, widgets};

/// Radius of the disk: the preset ring's outer edge.
const DISK: f32 = 124.0;
/// Preset slots around the ring, their centre radius and size.
pub const PRESET_SLOTS: usize = 10;
const PRESET_RADIUS: f32 = 102.0;
const PRESET_SIZE: f32 = 17.0;
/// Colour history patches between the presets and the hue ring.
pub const HISTORY_SLOTS: usize = 12;
const HISTORY_RADIUS: f32 = 74.0;
const HISTORY_SIZE: f32 = 9.0;
/// The hue ring (inner and outer radius) and the saturation/brightness square inside it.
const HUE_INNER: f32 = 48.0;
const HUE_OUTER: f32 = 62.0;
const SV_HALF: f32 = 32.0;
/// Room for the controls under the disk.
const CONTROLS_H: f32 = 60.0;

pub(crate) fn palette_id() -> egui::Id {
    egui::Id::new("popup-palette")
}

/// Open the palette centred on screen point `at`.
pub(crate) fn open(ui: &mut crate::state::UiState, ctx: &egui::Context, at: [f32; 2]) {
    ui.popup_palette = Some(at);
    ui.brush_picker = None;
    crate::canvas_popup::mark_open(ctx, palette_id());
}

pub fn close(ui: &mut crate::state::UiState) {
    ui.popup_palette = None;
}

/// The presets shown around the ring: those carrying the palette's tag (every preset when no tag
/// is chosen), the current preset always among them, at most [`PRESET_SLOTS`].
pub fn ring_presets(app: &PhotocraftApp) -> Vec<&BrushPreset> {
    let tag = app.ui.popup_tag.trim();
    let session = &app.session;
    let mut list: Vec<&BrushPreset> = session
        .tools
        .presets
        .iter()
        .filter(|p| tag.is_empty() || redrob_photo_engine::brush_preset_cmds::has_tag(session, &p.name, tag))
        .take(PRESET_SLOTS)
        .collect();
    if let Some(cur) = session.tools.current_preset.as_deref()
        && !list.iter().any(|p| p.name.eq_ignore_ascii_case(cur))
        && let Some(p) = session.tools.presets.iter().find(|p| p.name.eq_ignore_ascii_case(cur))
    {
        if list.len() == PRESET_SLOTS {
            list.pop();
        }
        list.push(p);
    }
    list
}

/// Where slot `i` of `n` sits on a ring of `radius` around `c`: the first slot at the top,
/// clockwise.
fn slot_center(c: Pos2, radius: f32, i: usize, n: usize) -> Pos2 {
    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / n.max(1) as f32;
    pos2(c.x + radius * a.cos(), c.y + radius * a.sin())
}

fn hsv(c: [f32; 4]) -> [f32; 3] {
    color_picker_ui::rgb_to_hsv([c[0], c[1], c[2]].map(|v| v.clamp(0.0, 1.0)))
}

fn set_foreground(app: &mut PhotocraftApp, h: f32, s: f32, v: f32) {
    let [r, g, b] = color_picker_ui::hsv_to_rgb(h, s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    color_history::pick(app, [r, g, b, 1.0], false);
}

/// The hue ring: `n` filled quads around the circle.
fn draw_hue_ring(painter: &egui::Painter, c: Pos2) {
    let n = 72;
    for i in 0..n {
        let (a0, a1) = (std::f32::consts::TAU * i as f32 / n as f32, std::f32::consts::TAU * (i + 1) as f32 / n as f32 + 0.004);
        let p = |r: f32, a: f32| pos2(c.x + r * a.cos(), c.y + r * a.sin());
        let [red, green, blue] = color_picker_ui::hsv_to_rgb(360.0 * (i as f32 + 0.5) / n as f32, 1.0, 1.0);
        let color = color_history::color32([red, green, blue, 1.0]);
        painter.add(egui::Shape::convex_polygon(vec![p(HUE_INNER, a0), p(HUE_OUTER, a0), p(HUE_OUTER, a1), p(HUE_INNER, a1)], color, Stroke::NONE));
    }
}

/// The saturation/brightness square for hue `h`: saturation left to right, brightness bottom to
/// top, as a shaded mesh.
fn draw_sv_square(painter: &egui::Painter, square: Rect, h: f32) {
    let mut mesh = egui::Mesh::default();
    let n = 12;
    for j in 0..=n {
        for i in 0..=n {
            let (s, v) = (i as f32 / n as f32, 1.0 - j as f32 / n as f32);
            let [r, g, b] = color_picker_ui::hsv_to_rgb(h, s, v);
            let p = pos2(square.left() + square.width() * i as f32 / n as f32, square.top() + square.height() * j as f32 / n as f32);
            mesh.colored_vertex(p, color_history::color32([r, g, b, 1.0]));
        }
    }
    let w = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * w + i;
            mesh.add_triangle(a, a + 1, a + w);
            mesh.add_triangle(a + 1, a + w + 1, a + w);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Show the open palette. Run once a frame from the document area.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some([x, y]) = app.ui.popup_palette else { return };
    if !crate::paint_mouse::has_brush_picker(app.ui.tool) || app.session.active().is_none() {
        close(&mut app.ui);
        return;
    }
    let key_close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let screen = ctx.content_rect();
    let size = vec2(2.0 * DISK + 16.0, 2.0 * DISK + 16.0 + CONTROLS_H);
    let pos = pos2(
        (x - size.x / 2.0).clamp(screen.left(), (screen.right() - size.x).max(screen.left())),
        (y - DISK - 8.0).clamp(screen.top(), (screen.bottom() - size.y).max(screen.top())),
    );
    let id = palette_id();
    let mut closes = false;
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        let t = Tokens::get(ui.ctx());
        ui.set_min_size(size);
        let (disk_rect, resp) = ui.allocate_exact_size(vec2(size.x, 2.0 * DISK + 16.0), Sense::click_and_drag());
        let c = disk_rect.center();
        let painter = ui.painter();
        painter.circle_filled(c, DISK + 6.0, t.dock.gamma_multiply(0.96));
        painter.circle_stroke(c, DISK + 6.0, Stroke::new(1.0, t.field_border));
        // Presets around the ring.
        let presets: Vec<(String, BrushPreset)> = ring_presets(app).into_iter().map(|p| (p.name.clone(), p.clone())).collect();
        let current = app.session.tools.current_preset.clone();
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let n = presets.len().max(1);
        let mut hovered_preset = None;
        for (i, (name, p)) in presets.iter().enumerate() {
            let slot = slot_center(c, PRESET_RADIUS, i, n.max(6));
            let is_current = current.as_deref().is_some_and(|cur| cur.eq_ignore_ascii_case(name));
            let over = pointer.is_some_and(|q| q.distance(slot) <= PRESET_SIZE);
            if over {
                hovered_preset = Some(i);
            }
            painter.circle_filled(slot, PRESET_SIZE, if is_current { t.accent_soft } else { t.field });
            if is_current || over {
                painter.circle_stroke(slot, PRESET_SIZE, Stroke::new(1.5, t.accent));
            }
            let b = &p.brush;
            let tip = brush_preview::tip_texture(ui.ctx(), &format!("popup-tip:{name}"), &b.tip, (b.hardness, b.angle, b.roundness), 36, t.text);
            let r = Rect::from_center_size(slot, vec2(2.0, 2.0) * (PRESET_SIZE - 3.0) * crate::brush_sections::thumb_scale(b.size).max(0.35));
            painter.image(tip.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        // Colour history between the presets and the hue ring.
        let history: Vec<[f32; 4]> = app.ui.color_history.iter().take(HISTORY_SLOTS).copied().collect();
        let mut hovered_history = None;
        for (i, col) in history.iter().enumerate() {
            let slot = slot_center(c, HISTORY_RADIUS, i, HISTORY_SLOTS);
            let r = Rect::from_center_size(slot, vec2(HISTORY_SIZE, HISTORY_SIZE));
            painter.rect_filled(r, 1.5, color_history::color32(*col));
            if pointer.is_some_and(|q| r.expand(2.0).contains(q)) {
                hovered_history = Some(i);
                painter.rect_stroke(r.expand(1.0), 1.5, Stroke::new(1.0, t.accent), egui::StrokeKind::Outside);
            }
        }
        // Hue ring and the saturation/brightness square, marked with the foreground colour.
        let fg = app.session.tools.foreground;
        let [h, s, v] = hsv(fg);
        let remembered: Option<f32> = ui.data(|d| d.get_temp(id.with("hue")));
        let hue = if s < 0.01 || v < 0.01 { remembered.unwrap_or(h) } else { h };
        draw_hue_ring(painter, c);
        let square = Rect::from_center_size(c, vec2(2.0 * SV_HALF, 2.0 * SV_HALF));
        draw_sv_square(painter, square, hue);
        painter.rect_stroke(square, 0.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
        let a = hue.to_radians();
        let mid = (HUE_INNER + HUE_OUTER) / 2.0;
        painter.circle_stroke(pos2(c.x + mid * a.cos(), c.y + mid * a.sin()), 5.0, Stroke::new(2.0, Color32::WHITE));
        let marker = pos2(square.left() + square.width() * s, square.top() + square.height() * (1.0 - v));
        painter.circle_stroke(marker, 4.0, Stroke::new(1.5, if v > 0.5 { Color32::BLACK } else { Color32::WHITE }));
        // Foreground / background chips in the top-left corner of the disk's box.
        let chips = Rect::from_min_size(disk_rect.left_top() + vec2(6.0, 6.0), vec2(30.0, 30.0));
        let bg = app.session.tools.background;
        painter.rect_filled(Rect::from_min_size(chips.min + vec2(10.0, 10.0), vec2(20.0, 20.0)), 2.0, color_history::color32(bg));
        painter.rect_filled(Rect::from_min_size(chips.min, vec2(20.0, 20.0)), 2.0, color_history::color32(fg));
        painter.rect_stroke(Rect::from_min_size(chips.min, vec2(20.0, 20.0)), 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
        // Interaction on the disk.
        let pressed = resp.clicked() || resp.drag_started() || resp.dragged();
        if let Some(q) = pointer.filter(|_| pressed) {
            let d = q.distance(c);
            if resp.clicked() && chips.contains(q) {
                let _ = app.run("tools.swapColors", json!({}));
            } else if square.expand(2.0).contains(q) && d < HUE_INNER {
                let s = ((q.x - square.left()) / square.width()).clamp(0.0, 1.0);
                let v = (1.0 - (q.y - square.top()) / square.height()).clamp(0.0, 1.0);
                set_foreground(app, hue, s, v);
                ui.data_mut(|d| d.insert_temp(id.with("hue"), hue));
            } else if (HUE_INNER - 3.0..=HUE_OUTER + 3.0).contains(&d) {
                let new_hue = (q.y - c.y).atan2(q.x - c.x).to_degrees().rem_euclid(360.0);
                // A grey or black foreground takes the ring's full colour; otherwise only the hue moves.
                let (s2, v2) = (if s < 0.01 { 1.0 } else { s }, if v < 0.01 { 1.0 } else { v });
                set_foreground(app, new_hue, s2, v2);
                ui.data_mut(|d| d.insert_temp(id.with("hue"), new_hue));
            } else if resp.clicked() {
                if let Some(i) = hovered_history {
                    color_history::pick(app, history[i], ui.input(|i| i.modifiers.alt));
                } else if let Some(i) = hovered_preset {
                    let _ = app.run("tools.setBrush", json!({"preset": presets[i].0}));
                    closes = true;
                }
            }
        }
        // Controls under the disk: tag, zoom, rotation, mirror.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let all = tl!("All Tags");
            let tags = redrob_photo_engine::brush_preset_cmds::all_tags(&app.session);
            let mut opts: Vec<(String, &str)> = vec![(String::new(), all)];
            opts.extend(tags.iter().map(|x| (x.clone(), x.as_str())));
            widgets::dropdown_literal(ui, "popup-palette-tag", &mut app.ui.popup_tag, &opts, 110.0);
            let mirrored = app
                .session
                .active()
                .and_then(|st| st.symmetry.as_ref())
                .and_then(|s| s.preset())
                .is_some_and(|p| p.mode == redrob_photo_paint::symmetry::SymmetryMode::Vertical);
            if crate::icons::button(ui, "symmetry-vertical", 24.0, mirrored, tl!("Mirror painting across the vertical axis")).clicked() {
                let _ = app
                    .run(if mirrored { "paint.symmetryDisable" } else { "paint.setSymmetry" }, if mirrored { json!({}) } else { json!({"mode": "vertical"}) });
            }
            if widgets::secondary_button(ui, "100%", 0.0).clicked()
                && let Some(i) = app.session.active_index()
                && let Some(view) = app.ui.views.get_mut(i)
            {
                view.fit_pending = false;
                view.zoom = 1.0;
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if let Some(i) = app.session.active_index()
                && let Some(view) = app.ui.views.get_mut(i)
            {
                let mut lz = view.zoom.max(0.01).ln();
                ui.label(egui::RichText::new(tl!("Zoom")).color(t.text_dim).size(11.0));
                ui.add_sized(vec2(80.0, 18.0), |ui: &mut egui::Ui| {
                    let r = widgets::slider(ui, &mut lz, 0.01f32.ln()..=32.0f32.ln(), None);
                    if r.changed() {
                        view.fit_pending = false;
                        view.zoom = lz.exp();
                    }
                    r
                });
                let mut rot = view.rotation;
                ui.label(egui::RichText::new(tl!("Rotation")).color(t.text_dim).size(11.0));
                ui.add_sized(vec2(80.0, 18.0), |ui: &mut egui::Ui| {
                    let r = widgets::slider(ui, &mut rot, -180.0..=180.0, None);
                    if r.changed() {
                        view.rotation = rot;
                    }
                    r
                });
            }
        });
    });
    let outside = crate::canvas_popup::outside_pressed(ctx, id, area.response.rect);
    if outside || key_close || closes {
        close(&mut app.ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_start_at_the_top_and_go_clockwise() {
        let c = pos2(0.0, 0.0);
        let top = slot_center(c, 10.0, 0, 4);
        assert!((top.x).abs() < 1e-4 && (top.y + 10.0).abs() < 1e-4);
        let right = slot_center(c, 10.0, 1, 4);
        assert!((right.x - 10.0).abs() < 1e-4 && right.y.abs() < 1e-4);
    }
}
