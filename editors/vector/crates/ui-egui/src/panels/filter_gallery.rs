//! Filter Gallery panel: every stock filter preset (Effect › Filters) as a rendered thumbnail,
//! grouped by category, with a search field; a click applies the preset to the selection through
//! `filter.applyPreset` (replacing the object's filter graph, so the Filter Editor edits it next).
//! Thumbnails are the preset's graph run on a sample swatch (a rounded square in the accent colour
//! over a transparent background) and cached as textures.

use std::cell::RefCell;
use std::collections::HashMap;

use egui::{Color32, Rect, Sense, Ui, vec2};
use redrob_vector_effects::PixelSpace;
use redrob_vector_effects::filter_presets::{self, FilterPreset};
use redrob_vector_geom::{Affine, Point};
use serde_json::json;

use super::selection_len;
use crate::VectorApp;
use crate::theme::Tokens;
use crate::widgets;

pub const ID: &str = "filterGallery";
/// Thumbnail side in pixels.
const THUMB: usize = 56;
const CELL: f32 = 72.0;

thread_local! {
    static CACHE: RefCell<HashMap<(&'static str, [u8; 3]), egui::TextureHandle>> = RefCell::new(HashMap::new());
}

/// The sample swatch: a rounded square of `rgb` in the middle of a transparent `THUMB` square,
/// premultiplied RGBA8.
fn swatch(rgb: [u8; 3]) -> Vec<u8> {
    let mut px = vec![0u8; THUMB * THUMB * 4];
    let (lo, hi, r) = (THUMB as f64 * 0.22, THUMB as f64 * 0.78, THUMB as f64 * 0.12);
    for y in 0..THUMB {
        for x in 0..THUMB {
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            // Signed distance to the rounded square.
            let dx = (cx - (lo + hi) / 2.0).abs() - ((hi - lo) / 2.0 - r);
            let dy = (cy - (lo + hi) / 2.0).abs() - ((hi - lo) / 2.0 - r);
            let d = (dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0)) - r;
            let a = (0.5 - d).clamp(0.0, 1.0);
            let i = (y * THUMB + x) * 4;
            px[i] = (f64::from(rgb[0]) * a) as u8;
            px[i + 1] = (f64::from(rgb[1]) * a) as u8;
            px[i + 2] = (f64::from(rgb[2]) * a) as u8;
            px[i + 3] = (a * 255.0) as u8;
        }
    }
    px
}

/// The preset's thumbnail: the swatch filtered by its graph (one pixel = one point).
fn thumbnail(ui: &Ui, p: &FilterPreset, rgb: [u8; 3]) -> egui::TextureHandle {
    if let Some(t) = CACHE.with(|c| c.borrow().get(&(p.id, rgb)).cloned()) {
        return t;
    }
    let mut px = swatch(rgb);
    let g = (p.build)();
    let side = THUMB as f64;
    let bounds = redrob_vector_geom::Rect::new(side * 0.22, side * 0.22, side * 0.78, side * 0.78);
    let space = PixelSpace {
        to_doc: Affine::IDENTITY,
        px: 1.0,
        center: Point::new(side / 2.0, side / 2.0),
        channels: Default::default(),
        line: 1.0,
        extent: bounds.width() * std::f64::consts::FRAC_1_SQRT_2,
    };
    g.apply(&mut px, THUMB, THUMB, &space, bounds);
    let image = egui::ColorImage::from_rgba_premultiplied([THUMB, THUMB], &px);
    let tex = ui.ctx().load_texture(format!("filter-gallery-{}", p.id), image, egui::TextureOptions::LINEAR);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 256 {
            c.clear();
        }
        c.insert((p.id, rgb), tex.clone());
    });
    tex
}

pub fn show(app: &mut VectorApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let query = widgets::search_field(ui, ui.id().with("filter-gallery-search"), "Search filters");
    let has_sel = selection_len(app) > 0;
    let accent = t.accent;
    let rgb = [accent.r(), accent.g(), accent.b()];
    let mut apply: Option<&'static str> = None;
    let hits = filter_presets::search(&query);
    egui::ScrollArea::vertical().id_salt("filter-gallery").auto_shrink([false, false]).show(ui, |ui| {
        for category in filter_presets::CATEGORIES {
            let presets: Vec<&FilterPreset> = hits.iter().copied().filter(|p| p.category == category).collect();
            if presets.is_empty() {
                continue;
            }
            widgets::subheader(ui, tl!(category));
            let cols = ((ui.available_width() / CELL).floor() as usize).max(1);
            for row in presets.chunks(cols) {
                ui.horizontal(|ui| {
                    for p in row {
                        let (rect, resp) = ui.allocate_exact_size(vec2(CELL, CELL + 16.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, 3.0, t.hover);
                        }
                        let tex = thumbnail(ui, p, rgb);
                        let img = Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + CELL / 2.0), vec2(THUMB as f32, THUMB as f32));
                        ui.painter().image(tex.id(), img, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                        let label = tl!(p.label);
                        ui.painter().text(
                            egui::pos2(rect.center().x, rect.bottom() - 8.0),
                            egui::Align2::CENTER_CENTER,
                            label,
                            egui::TextStyle::Small.resolve(ui.style()),
                            t.text,
                        );
                        let resp = resp.on_hover_text(format!("{} › {}", tl!(p.category), tl!(p.label)));
                        if resp.clicked() && has_sel {
                            apply = Some(p.id);
                        }
                    }
                });
            }
        }
        if hits.is_empty() {
            widgets::dim_label(ui, tl!("No filter matches the search."));
        }
    });
    if !has_sel {
        widgets::dim_label(ui, tl!("Select an object to apply a filter."));
    }
    if let Some(id) = apply
        && let Err(e) = app.run("filter.applyPreset", json!({ "id": id }))
    {
        app.ui.status = e;
    }
}

pub fn menu(app: &mut VectorApp, ui: &mut Ui) {
    if widgets::menu_item(ui, tl!("Open Filter Editor"), true, false)
        && let Err(e) = app.run("window.panel", json!({ "panel": super::filter_editor::ID }))
    {
        app.ui.status = e;
    }
    if widgets::menu_item(ui, tl!("Remove Filter"), super::filter_editor::has_filter(app), false) {
        super::filter_editor::remove_filter(app);
    }
}
