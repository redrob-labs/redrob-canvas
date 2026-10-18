// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Color panel's selectors beyond the HSB field: the advanced selector (wheel / triangle /
//! square in HSV, HSL, HSI or HSY with luma coefficients and gamma, a zoom pop-up), the shade
//! strip, the artistic selector (stepped hue ring with value and saturation rings, continuous
//! mode, gamut masks), the wide-gamut selector (a wide RGB space with a static or dynamic hue
//! ring, patches, proofing to the painting space), the specific colour selector (channel sliders
//! of a chosen model and depth, hex field), the digital colour mixer and the common-colours
//! patches sampled from the image. Gamut masks are created, edited, duplicated, deleted and
//! rotated here and shared by the artistic and advanced selectors.
//!
//! All of it edits the foreground (Alt: the background) through `tools.setColors`, so Enforce
//! Palette Colours and the colour history apply. Settings persist in `UiState::color_selectors`.

use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use redrob_photo_color::models::{self, ColorModel, Depth, Luma, WideSpace};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::theme::Tokens;
use crate::widgets;
use crate::{PhotocraftApp, color_history};

// ------------------------------------------------------------------ settings

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectorKind {
    /// The classic HSB field and sliders only.
    #[default]
    Basic,
    Advanced,
    Artistic,
    WideGamut,
    Specific,
    Mixer,
}

pub const SELECTORS: [(SelectorKind, &str); 6] = [
    (SelectorKind::Basic, "Basic"),
    (SelectorKind::Advanced, "Advanced"),
    (SelectorKind::Artistic, "Artistic"),
    (SelectorKind::WideGamut, "Wide Gamut"),
    (SelectorKind::Specific, "Specific"),
    (SelectorKind::Mixer, "Mixer"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Shape {
    /// Hue ring with a saturation/value triangle inside.
    #[default]
    Triangle,
    /// Hue ring with a square inside.
    Square,
    /// A full wheel: hue by angle, saturation by radius, a value slider beside it.
    Wheel,
}

pub const SHAPES: [(Shape, &str); 3] = [(Shape::Triangle, "Triangle"), (Shape::Square, "Square"), (Shape::Wheel, "Wheel")];

pub const MODELS: [(ColorModel, &str); 4] = [(ColorModel::Hsv, "HSV"), (ColorModel::Hsl, "HSL"), (ColorModel::Hsi, "HSI"), (ColorModel::Hsy, "HSY")];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AdvancedSettings {
    pub shape: Shape,
    pub model: ColorModel,
    pub luma: Luma,
    /// Side of the selector in points; 0 = fit the panel width (resize behaviour).
    pub size: f32,
    pub fit_width: bool,
    /// Side of the zoom pop-up (a larger copy of the selector shown while dragging).
    pub zoom_size: f32,
    pub zoom_popup: bool,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self { shape: Shape::Triangle, model: ColorModel::Hsv, luma: Luma::REC709, size: 180.0, fit_width: true, zoom_size: 320.0, zoom_popup: true }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShadeStyle {
    /// Rows of hue, saturation and lightness steps around the colour.
    #[default]
    Rows,
    /// One row of lightness steps.
    Simple,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShadeUpdate {
    /// The strip follows every change of the foreground colour.
    #[default]
    Always,
    /// The strip updates when a colour is picked from it (or the button is pressed).
    OnPick,
    /// Only the update button refreshes it.
    Manual,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShadeSettings {
    pub enabled: bool,
    pub style: ShadeStyle,
    pub update: ShadeUpdate,
    /// Patches per row.
    pub count: u32,
    /// Patch side in points.
    pub size: f32,
    /// Spread of the shades, 0..1.
    pub spread: f32,
    /// The colour the strip was built around (kept for OnPick / Manual).
    pub base: [f32; 3],
}

impl Default for ShadeSettings {
    fn default() -> Self {
        Self { enabled: false, style: ShadeStyle::Rows, update: ShadeUpdate::Always, count: 9, size: 16.0, spread: 0.3, base: [0.5, 0.5, 0.5] }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtisticSettings {
    /// Hue steps round the ring (continuous mode: 0).
    pub hue_steps: u32,
    /// Saturation rings.
    pub sat_steps: u32,
    /// Value steps shown in the inner block.
    pub value_steps: u32,
    pub continuous: bool,
    /// Swap the value and saturation rings' roles (value on the ring, saturation inside).
    pub value_ring: bool,
    pub size: f32,
    /// The hue and saturation steps are shown for the current value (off: full value).
    pub shade_by_value: bool,
}

impl Default for ArtisticSettings {
    fn default() -> Self {
        Self { hue_steps: 24, sat_steps: 5, value_steps: 11, continuous: false, value_ring: false, size: 180.0, shade_by_value: true }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatchLayout {
    #[default]
    Row,
    Column,
    Grid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WideSettings {
    pub space: WideSpace,
    /// Dynamic ring: the ring shows the hues at the current saturation and value (static: the
    /// pure hues).
    pub dynamic_ring: bool,
    pub size: f32,
    pub popup_size: f32,
    pub patches: PatchLayout,
    pub patch_count: u32,
    pub patch_size: f32,
    pub shade_strip: bool,
    /// Proof the picked colour to the painting (sRGB) space and mark out-of-gamut colours.
    pub proof: bool,
    /// The colour as picked in `space` (encoded 0..1 in that space).
    pub picked: [f32; 3],
}

impl Default for WideSettings {
    fn default() -> Self {
        Self {
            space: WideSpace::Rec2020,
            dynamic_ring: false,
            size: 180.0,
            popup_size: 320.0,
            patches: PatchLayout::Row,
            patch_count: 8,
            patch_size: 16.0,
            shade_strip: true,
            proof: true,
            picked: [0.5, 0.5, 0.5],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SpecificSettings {
    pub model: ColorModel,
    pub depth: Depth,
    pub luma: Luma,
    /// Show the channels as 0..1 fractions whatever the depth (the "percent/float" toggle).
    pub normalized: bool,
    /// The value of each channel as typed, so an unbounded float value is kept while the colour
    /// is clipped for the tool.
    pub channels: [f32; 4],
}

impl Default for SpecificSettings {
    fn default() -> Self {
        Self { model: ColorModel::Rgb, depth: Depth::U8, luma: Luma::REC709, normalized: false, channels: [0.0; 4] }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MixerSettings {
    /// The six column colours mixed with the current colour.
    pub colors: [[f32; 3]; 6],
    /// Mix amount per column, 0 (the current colour) .. 1 (the column colour).
    pub amounts: [f32; 6],
    /// The gradient slider's two ends.
    pub gradient: [[f32; 3]; 2],
    pub gradient_pos: f32,
}

impl Default for MixerSettings {
    fn default() -> Self {
        Self {
            colors: [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 1.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0]],
            amounts: [0.5; 6],
            gradient: [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
            gradient_pos: 0.5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommonSettings {
    pub enabled: bool,
    pub count: u32,
    pub size: f32,
    pub layout: PatchLayout,
    /// Patches beyond the panel scroll (off: they wrap).
    pub scroll: bool,
    pub all_layers: bool,
    /// Recent colours shown (0 = all of the history).
    pub history_count: u32,
    pub history_size: f32,
    pub history_layout: PatchLayout,
}

impl Default for CommonSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            count: 12,
            size: 14.0,
            layout: PatchLayout::Row,
            scroll: false,
            all_layers: true,
            history_count: 16,
            history_size: 14.0,
            history_layout: PatchLayout::Row,
        }
    }
}

/// One shape of a gamut mask: a polygon in the hue/saturation plane, vertices as (hue degrees,
/// saturation 0..1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct MaskShape {
    pub points: Vec<[f32; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct GamutMask {
    pub name: String,
    pub shapes: Vec<MaskShape>,
    pub builtin: bool,
}

impl GamutMask {
    fn poly(name: &str, pts: &[(f32, f32)]) -> MaskShape {
        let _ = name;
        MaskShape { points: pts.iter().map(|(h, s)| [*h, *s]).collect() }
    }

    /// The built-in masks: classic colour-scheme shapes.
    pub fn builtins() -> Vec<GamutMask> {
        let tri = |a: f32, b: f32, c: f32| Self::poly("", &[(a, 1.0), (b, 1.0), (c, 1.0), (0.0, 0.0)]);
        let wedge = |h: f32, w: f32, s_min: f32| Self::poly("", &[(h - w, s_min), (h - w, 1.0), (h, 1.0), (h + w, 1.0), (h + w, s_min)]);
        vec![
            GamutMask { name: "Atmospheric Triad".into(), shapes: vec![tri(0.0, 120.0, 240.0)], builtin: true },
            GamutMask { name: "Complementary".into(), shapes: vec![wedge(0.0, 20.0, 0.0), wedge(180.0, 20.0, 0.0)], builtin: true },
            GamutMask {
                name: "Split Complementary".into(),
                shapes: vec![wedge(0.0, 15.0, 0.0), wedge(150.0, 15.0, 0.0), wedge(210.0, 15.0, 0.0)],
                builtin: true,
            },
            GamutMask { name: "Analogous".into(), shapes: vec![wedge(0.0, 45.0, 0.0)], builtin: true },
            GamutMask {
                name: "Tetradic".into(),
                shapes: vec![wedge(0.0, 12.0, 0.0), wedge(90.0, 12.0, 0.0), wedge(180.0, 12.0, 0.0), wedge(270.0, 12.0, 0.0)],
                builtin: true,
            },
            GamutMask { name: "Dominant Hue with Accent".into(), shapes: vec![wedge(0.0, 40.0, 0.0), wedge(180.0, 8.0, 0.5)], builtin: true },
        ]
    }

    /// Is the colour at hue `h` (degrees) and saturation `s` inside the mask turned by `rotation`
    /// degrees? Masks with no shapes allow everything.
    pub fn contains(&self, h: f32, s: f32, rotation: f32) -> bool {
        if self.shapes.is_empty() {
            return true;
        }
        let (x, y) = polar_xy(h - rotation, s);
        self.shapes.iter().any(|sh| {
            let pts: Vec<(f32, f32)> = sh.points.iter().map(|p| polar_xy(p[0], p[1])).collect();
            point_in_polygon(x, y, &pts)
        })
    }

    /// The nearest hue/saturation inside the mask (the picked colour is pulled into it).
    pub fn clamp(&self, h: f32, s: f32, rotation: f32) -> (f32, f32) {
        if self.contains(h, s, rotation) {
            return (h, s);
        }
        let (x, y) = polar_xy(h - rotation, s);
        let mut best: Option<(f32, (f32, f32))> = None;
        for sh in &self.shapes {
            let pts: Vec<(f32, f32)> = sh.points.iter().map(|p| polar_xy(p[0], p[1])).collect();
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                let p = nearest_on_segment((x, y), a, b);
                let d = (p.0 - x).powi(2) + (p.1 - y).powi(2);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, p));
                }
            }
        }
        match best {
            Some((_, (px, py))) => {
                let hh = py.atan2(px).to_degrees().rem_euclid(360.0) + rotation;
                (hh.rem_euclid(360.0), (px * px + py * py).sqrt().clamp(0.0, 1.0))
            }
            None => (h, s),
        }
    }
}

fn polar_xy(h: f32, s: f32) -> (f32, f32) {
    let a = h.to_radians();
    (a.cos() * s, a.sin() * s)
}

fn point_in_polygon(x: f32, y: f32, pts: &[(f32, f32)]) -> bool {
    let n = pts.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi + 1e-12) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn nearest_on_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= 1e-12 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    (a.0 + dx * t, a.1 + dy * t)
}

/// Everything the selectors remember (`UiState::color_selectors`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ColorSelectorSettings {
    pub kind: SelectorKind,
    pub advanced: AdvancedSettings,
    pub shade: ShadeSettings,
    pub artistic: ArtisticSettings,
    pub wide: WideSettings,
    pub specific: SpecificSettings,
    pub mixer: MixerSettings,
    pub common: CommonSettings,
    pub gamut_masks: Vec<GamutMask>,
    /// Index into `gamut_masks` of the active mask.
    pub active_mask: Option<usize>,
    pub mask_enabled: bool,
    /// Degrees the active mask is turned by.
    pub mask_rotation: f32,
    /// Show the mask's shapes over the selector (off: only limit the picks).
    pub mask_show: bool,
    /// The settings rows are open.
    pub show_settings: bool,
    /// Common colours of the image, cached per document revision (never saved).
    #[serde(skip)]
    pub common_cache: Option<((u64, u64), Vec<[f32; 3]>)>,
    /// A new shape being drawn on the mask editor (hue, saturation points).
    #[serde(skip)]
    pub mask_draft: Vec<[f32; 2]>,
}

impl Default for ColorSelectorSettings {
    fn default() -> Self {
        Self {
            kind: SelectorKind::Basic,
            advanced: AdvancedSettings::default(),
            shade: ShadeSettings::default(),
            artistic: ArtisticSettings::default(),
            wide: WideSettings::default(),
            specific: SpecificSettings::default(),
            mixer: MixerSettings::default(),
            common: CommonSettings::default(),
            gamut_masks: GamutMask::builtins(),
            active_mask: None,
            mask_enabled: false,
            mask_rotation: 0.0,
            mask_show: true,
            show_settings: false,
            common_cache: None,
            mask_draft: Vec::new(),
        }
    }
}

impl ColorSelectorSettings {
    /// The active gamut mask when masking is on.
    pub fn mask(&self) -> Option<&GamutMask> {
        if !self.mask_enabled {
            return None;
        }
        self.active_mask.and_then(|i| self.gamut_masks.get(i))
    }

    /// Hue and saturation limited to the active mask.
    pub fn masked(&self, h: f32, s: f32) -> (f32, f32) {
        match self.mask() {
            Some(m) => m.clamp(h, s, self.mask_rotation),
            None => (h, s),
        }
    }
}

// ------------------------------------------------------------------ helpers

fn fg(app: &PhotocraftApp) -> [f32; 3] {
    let c = app.session.tools.foreground;
    [c[0], c[1], c[2]].map(|v| v.clamp(0.0, 1.0))
}

fn set_color(app: &mut PhotocraftApp, rgb: [f32; 3], background: bool) {
    let c = rgb.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    color_history::pick(app, [c[0], c[1], c[2], 1.0], background);
    if !background {
        crate::type_tool::foreground_changed(app);
    }
}

fn c32(c: [f32; 3]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

fn dim(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(s).color(t.text_dim));
}

/// A cylindrical model's channels of `rgb`: (hue degrees, saturation, third).
fn to_cyl(model: ColorModel, luma: &Luma, rgb: [f32; 3]) -> [f32; 3] {
    match model {
        ColorModel::Hsl => models::rgb_to_hsl(rgb),
        ColorModel::Hsi => models::rgb_to_hsi(rgb),
        ColorModel::Hsy => models::rgb_to_hsy(rgb, luma),
        _ => models::rgb_to_hsv(rgb),
    }
}

fn from_cyl(model: ColorModel, luma: &Luma, h: f32, s: f32, v: f32) -> [f32; 3] {
    match model {
        ColorModel::Hsl => models::hsl_to_rgb(h, s, v),
        ColorModel::Hsi => models::hsi_to_rgb(h, s, v),
        ColorModel::Hsy => models::hsy_to_rgb(h, s, v, luma),
        _ => models::hsv_to_rgb(h, s, v),
    }
}

/// A patch; returns `Some(alt)` when clicked.
fn patch(ui: &mut egui::Ui, side: f32, c: [f32; 3], label: &str, marked: bool) -> Option<bool> {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    let hex = crate::color_picker_ui::hex(c);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, true, format!("{label} {hex}")));
    ui.painter().rect_filled(rect, 2.0, c32(c));
    if marked {
        ui.painter().rect_stroke(rect.shrink(1.0), 2.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    let resp = resp.on_hover_text(hex);
    resp.clicked().then(|| ui.input(|i| i.modifiers.alt))
}

fn patches(ui: &mut egui::Ui, colors: &[[f32; 3]], side: f32, layout: PatchLayout, label: &str, scroll: bool) -> Option<([f32; 3], bool)> {
    let mut picked = None;
    let mut body = |ui: &mut egui::Ui| {
        let mut row = |ui: &mut egui::Ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            for (i, c) in colors.iter().enumerate() {
                if let Some(alt) = patch(ui, side, *c, &format!("{label} {}", i + 1), false) {
                    picked = Some((*c, alt));
                }
            }
        };
        match layout {
            PatchLayout::Row if scroll => {
                egui::ScrollArea::horizontal().id_salt(("color-patches", label)).show(ui, |ui| {
                    ui.horizontal(|ui| row(ui));
                });
            }
            PatchLayout::Row => {
                ui.horizontal_wrapped(|ui| row(ui));
            }
            PatchLayout::Column => {
                ui.vertical(|ui| row(ui));
            }
            PatchLayout::Grid => {
                ui.horizontal_wrapped(|ui| row(ui));
            }
        }
    };
    body(ui);
    picked
}

/// Cached texture of a `n × n` image computed by `f(u, v)` (0..1 each), keyed by `sig`.
fn texture(ui: &egui::Ui, slot: &str, sig: u64, n: usize, f: impl Fn(f32, f32) -> [f32; 3]) -> egui::TextureHandle {
    crate::brush_preview::with_cache(ui.ctx(), |c| {
        c.get(slot, sig, || {
            let mut px = Vec::with_capacity(n * n * 4);
            for y in 0..n {
                for x in 0..n {
                    let c = f((x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32);
                    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    px.extend_from_slice(&[q(c[0]), q(c[1]), q(c[2]), 255]);
                }
            }
            let img = egui::ColorImage::from_rgba_unmultiplied([n, n], &px);
            ui.ctx().load_texture(slot, img, egui::TextureOptions::LINEAR)
        })
    })
}

fn hash<T: std::hash::Hash>(v: T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

fn bits(v: f32) -> u32 {
    v.to_bits()
}

// ------------------------------------------------------------------ the panel

/// The selector rows of the Color panel (after the HSB field): the selector picker, the chosen
/// selector, the shade strip and the common-colours patches.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let mut settings = std::mem::take(&mut app.ui.color_selectors);
    ui.horizontal(|ui| {
        dim(ui, tl!("Selector:"));
        widgets::dropdown(ui, "color-selector-kind", &mut settings.kind, &SELECTORS, 110.0);
        let label = if settings.show_settings { "−" } else { "⚙" };
        if widgets::compact_button(ui, label, 20.0).on_hover_text(tl!("Selector settings")).clicked() {
            settings.show_settings = !settings.show_settings;
        }
    });
    match settings.kind {
        SelectorKind::Basic => {}
        SelectorKind::Advanced => advanced(app, ui, &mut settings),
        SelectorKind::Artistic => artistic(app, ui, &mut settings),
        SelectorKind::WideGamut => wide_gamut(app, ui, &mut settings),
        SelectorKind::Specific => specific(app, ui, &mut settings),
        SelectorKind::Mixer => mixer(app, ui, &mut settings),
    }
    if settings.shade.enabled {
        shade_strip(app, ui, &mut settings);
    }
    if settings.common.enabled {
        common_colors(app, ui, &mut settings);
    }
    if settings.show_settings {
        settings_rows(ui, &mut settings);
        ocio_rows(app, ui);
    }
    ocio_proof(app, ui);
    app.ui.color_selectors = settings;
}

/// LUT Management: the OpenColorIO display transform's switch, exposure (also the Y-key drag on
/// the canvas, `view.ocioExposure`), gamma, components and the locked colour.
fn ocio_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    widgets::hairline(ui);
    ui.add_space(2.0);
    let st = app.session.color.settings.ocio.clone();
    let mut enabled = st.enabled;
    let mut lock = st.lock_color;
    let mut exposure = st.exposure;
    let mut gamma = st.gamma;
    let mut components = st.components;
    let mut changed = false;
    changed |= widgets::checkbox(ui, &mut enabled, tl!("LUT Management (OCIO)")).changed();
    if enabled {
        changed |= widgets::slider_row_prec(
            ui,
            tl!("Exposure"),
            &mut exposure,
            -10.0..=10.0,
            "",
            None,
            widgets::RowGestures { reset: Some(0.0), ..Default::default() },
            Some(2),
        )
        .changed();
        changed |= widgets::slider_row_prec(
            ui,
            tl!("Gamma"),
            &mut gamma,
            0.1..=5.0,
            "",
            None,
            widgets::RowGestures { reset: Some(1.0), ..Default::default() },
            Some(2),
        )
        .changed();
        ui.horizontal(|ui| {
            dim(ui, tl!("Components:"));
            let opts: Vec<(redrob_photo_engine::ocio::Components, &str)> = redrob_photo_engine::ocio::Components::ALL.iter().map(|c| (*c, c.id())).collect();
            changed |= widgets::dropdown_literal(ui, "color-ocio-components", &mut components, &opts, 100.0);
        });
        changed |=
            widgets::checkbox(ui, &mut lock, tl!("Lock Color")).on_hover_text(tl!("Panel colours are not shown through the display transform")).changed();
        if !st.config.is_empty() {
            dim(ui, &st.config);
        }
    }
    if changed {
        let _ = app.run("view.ocio", json!({ "enabled": enabled, "lockColor": lock, "exposure": exposure, "gamma": gamma, "components": components.id() }));
    }
}

/// The foreground colour as the OCIO display transform shows it (unless the colour is locked):
/// a patch beside the plain colour, so what the canvas will show is visible while picking.
fn ocio_proof(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let st = &app.session.color.settings.ocio;
    if !st.enabled || st.lock_color {
        return;
    }
    let Some(p) = redrob_photo_engine::ocio::active(&app.session.color) else { return };
    let c = fg(app);
    let shown = p.apply(c);
    ui.horizontal(|ui| {
        dim(ui, tl!("Display:"));
        let _ = patch(ui, 16.0, c, "Foreground", false);
        let _ = patch(ui, 16.0, shown, "Displayed", true);
        dim(ui, &format!("{:+.2} EV", st.exposure));
    });
}

fn settings_rows(ui: &mut egui::Ui, st: &mut ColorSelectorSettings) {
    ui.add_space(4.0);
    widgets::hairline(ui);
    ui.add_space(2.0);
    match st.kind {
        SelectorKind::Advanced => {
            let a = &mut st.advanced;
            ui.horizontal(|ui| {
                dim(ui, tl!("Shape:"));
                widgets::dropdown(ui, "color-adv-shape", &mut a.shape, &SHAPES, 100.0);
                dim(ui, tl!("Model:"));
                widgets::dropdown(ui, "color-adv-model", &mut a.model, &MODELS, 70.0);
            });
            if a.model == ColorModel::Hsy {
                luma_rows(ui, "color-adv-luma", &mut a.luma);
            }
            widgets::checkbox(ui, &mut a.fit_width, tl!("Fit Panel Width"));
            if !a.fit_width {
                widgets::slider_row(ui, tl!("Size"), &mut a.size, 80.0..=600.0, "px", None);
            }
            widgets::checkbox(ui, &mut a.zoom_popup, tl!("Zoom Pop-up"));
            if a.zoom_popup {
                widgets::slider_row(ui, tl!("Pop-up Size"), &mut a.zoom_size, 120.0..=800.0, "px", None);
            }
            gamut_mask_rows(ui, st);
        }
        SelectorKind::Artistic => {
            let a = &mut st.artistic;
            widgets::checkbox(ui, &mut a.continuous, tl!("Continuous Mode"));
            if !a.continuous {
                let mut n = a.hue_steps as f32;
                if widgets::slider_row(ui, tl!("Hue Steps"), &mut n, 3.0..=72.0, "", None).changed() {
                    a.hue_steps = n.round() as u32;
                }
            }
            let mut n = a.sat_steps as f32;
            if widgets::slider_row(ui, tl!("Saturation Steps"), &mut n, 1.0..=12.0, "", None).changed() {
                a.sat_steps = n.round() as u32;
            }
            let mut n = a.value_steps as f32;
            if widgets::slider_row(ui, tl!("Value Steps"), &mut n, 2.0..=24.0, "", None).changed() {
                a.value_steps = n.round() as u32;
            }
            widgets::checkbox(ui, &mut a.value_ring, tl!("Value on the Ring"));
            widgets::checkbox(ui, &mut a.shade_by_value, tl!("Shade Ring by Value"));
            widgets::slider_row(ui, tl!("Size"), &mut a.size, 80.0..=600.0, "px", None);
            gamut_mask_rows(ui, st);
        }
        SelectorKind::WideGamut => {
            let w = &mut st.wide;
            ui.horizontal(|ui| {
                dim(ui, tl!("Color Space:"));
                let opts: Vec<(WideSpace, &str)> = WideSpace::ALL.iter().map(|s| (*s, s.label())).collect();
                widgets::dropdown_literal(ui, "color-wide-space", &mut w.space, &opts, 150.0);
            });
            widgets::checkbox(ui, &mut w.dynamic_ring, tl!("Dynamic Hue Ring"));
            widgets::checkbox(ui, &mut w.proof, tl!("Proof to Painting Space"));
            widgets::checkbox(ui, &mut w.shade_strip, tl!("Shade Strip"));
            widgets::slider_row(ui, tl!("Size"), &mut w.size, 80.0..=600.0, "px", None);
            widgets::slider_row(ui, tl!("Pop-up Size"), &mut w.popup_size, 120.0..=800.0, "px", None);
            ui.horizontal(|ui| {
                dim(ui, tl!("Patches:"));
                widgets::dropdown(
                    ui,
                    "color-wide-patches",
                    &mut w.patches,
                    &[(PatchLayout::Row, "Row"), (PatchLayout::Column, "Column"), (PatchLayout::Grid, "Grid")],
                    90.0,
                );
            });
            let mut n = w.patch_count as f32;
            if widgets::slider_row(ui, tl!("Patch Count"), &mut n, 0.0..=32.0, "", None).changed() {
                w.patch_count = n.round() as u32;
            }
            widgets::slider_row(ui, tl!("Patch Size"), &mut w.patch_size, 8.0..=40.0, "px", None);
        }
        SelectorKind::Specific => {
            let sp = &mut st.specific;
            ui.horizontal(|ui| {
                dim(ui, tl!("Model:"));
                let opts: Vec<(ColorModel, &str)> = ColorModel::ALL.iter().map(|m| (*m, m.label())).collect();
                widgets::dropdown_literal(ui, "color-spec-model", &mut sp.model, &opts, 80.0);
                dim(ui, tl!("Depth:"));
                let depths: Vec<(Depth, &str)> = Depth::ALL.iter().map(|d| (*d, d.label())).collect();
                widgets::dropdown(ui, "color-spec-depth", &mut sp.depth, &depths, 120.0);
            });
            if matches!(sp.model, ColorModel::Hsy | ColorModel::YCbCr) {
                luma_rows(ui, "color-spec-luma", &mut sp.luma);
            }
            widgets::checkbox(ui, &mut sp.normalized, tl!("Show as Fractions"));
        }
        SelectorKind::Mixer | SelectorKind::Basic => {}
    }
    ui.add_space(2.0);
    let sh = &mut st.shade;
    widgets::checkbox(ui, &mut sh.enabled, tl!("Shade Selector"));
    if sh.enabled {
        ui.horizontal(|ui| {
            dim(ui, tl!("Style:"));
            widgets::dropdown(ui, "color-shade-style", &mut sh.style, &[(ShadeStyle::Rows, "Rows"), (ShadeStyle::Simple, "Simple")], 90.0);
            dim(ui, tl!("Update:"));
            widgets::dropdown(
                ui,
                "color-shade-update",
                &mut sh.update,
                &[(ShadeUpdate::Always, "Always"), (ShadeUpdate::OnPick, "On Pick"), (ShadeUpdate::Manual, "Manually")],
                90.0,
            );
        });
        let mut n = sh.count as f32;
        if widgets::slider_row(ui, tl!("Shade Count"), &mut n, 3.0..=21.0, "", None).changed() {
            sh.count = n.round() as u32;
        }
        widgets::slider_row(ui, tl!("Shade Size"), &mut sh.size, 8.0..=40.0, "px", None);
        widgets::slider_row(ui, tl!("Shade Spread"), &mut sh.spread, 0.05..=1.0, "", None);
    }
    let co = &mut st.common;
    widgets::checkbox(ui, &mut co.enabled, tl!("Common Colors"));
    if co.enabled {
        let mut n = co.count as f32;
        if widgets::slider_row(ui, tl!("Patch Count"), &mut n, 1.0..=64.0, "", None).changed() {
            co.count = n.round() as u32;
            st.common_cache = None;
        }
        widgets::slider_row(ui, tl!("Patch Size"), &mut co.size, 8.0..=40.0, "px", None);
        ui.horizontal(|ui| {
            dim(ui, tl!("Direction:"));
            widgets::dropdown(
                ui,
                "color-common-layout",
                &mut co.layout,
                &[(PatchLayout::Row, "Row"), (PatchLayout::Column, "Column"), (PatchLayout::Grid, "Grid")],
                90.0,
            );
            widgets::checkbox(ui, &mut co.scroll, tl!("Scroll"));
        });
        if widgets::checkbox(ui, &mut co.all_layers, tl!("Sample All Layers")).changed() {
            st.common_cache = None;
        }
    }
    let mut n = co.history_count as f32;
    if widgets::slider_row(ui, tl!("Recent Colors"), &mut n, 0.0..=24.0, "", None).changed() {
        co.history_count = n.round() as u32;
    }
}

fn luma_rows(ui: &mut egui::Ui, id: &str, luma: &mut Luma) {
    ui.horizontal(|ui| {
        dim(ui, tl!("Luma:"));
        let mut cur: Option<usize> =
            Luma::PRESETS.iter().position(|(_, l)| (l.r - luma.r).abs() < 1e-4 && (l.g - luma.g).abs() < 1e-4 && (l.b - luma.b).abs() < 1e-4);
        let mut opts: Vec<(Option<usize>, &str)> = Luma::PRESETS.iter().enumerate().map(|(i, (n, _))| (Some(i), *n)).collect();
        opts.push((None, "Custom"));
        if widgets::dropdown_literal(ui, id, &mut cur, &opts, 100.0)
            && let Some(i) = cur
        {
            let g = luma.gamma;
            *luma = Luma::PRESETS[i].1;
            luma.gamma = g;
        }
    });
    ui.horizontal(|ui| {
        for (k, v) in [("R", &mut luma.r), ("G", &mut luma.g), ("B", &mut luma.b)] {
            dim(ui, k);
            let mut x = *v;
            if widgets::value_field_prec(ui, &mut x, 0.0..=1.0, "", 54.0, 4).changed() {
                *v = x.clamp(0.0, 1.0);
            }
        }
    });
    widgets::slider_row_prec(ui, tl!("Gamma"), &mut luma.gamma, 0.1..=4.0, "", None, widgets::RowGestures::default(), Some(2));
}

fn gamut_mask_rows(ui: &mut egui::Ui, st: &mut ColorSelectorSettings) {
    ui.add_space(2.0);
    widgets::checkbox(ui, &mut st.mask_enabled, tl!("Gamut Mask"));
    if !st.mask_enabled {
        return;
    }
    ui.horizontal(|ui| {
        let names: Vec<(Option<usize>, String)> =
            std::iter::once((None, tl!("None").to_string())).chain(st.gamut_masks.iter().enumerate().map(|(i, m)| (Some(i), m.name.clone()))).collect();
        let opts: Vec<(Option<usize>, &str)> = names.iter().map(|(i, n)| (*i, n.as_str())).collect();
        widgets::dropdown_literal(ui, "color-gamut-mask", &mut st.active_mask, &opts, 150.0);
        widgets::checkbox(ui, &mut st.mask_show, tl!("Show"));
    });
    widgets::slider_row(ui, tl!("Rotate Mask"), &mut st.mask_rotation, 0.0..=360.0, "°", None);
    ui.horizontal(|ui| {
        if widgets::compact_button(ui, tl!("New"), 20.0).clicked() {
            let n = st.gamut_masks.len() + 1;
            st.gamut_masks.push(GamutMask { name: format!("Mask {n}"), shapes: Vec::new(), builtin: false });
            st.active_mask = Some(st.gamut_masks.len() - 1);
            st.mask_draft.clear();
        }
        if let Some(i) = st.active_mask.filter(|i| *i < st.gamut_masks.len()) {
            if widgets::compact_button(ui, tl!("Duplicate"), 20.0).clicked() {
                let mut m = st.gamut_masks[i].clone();
                m.name = format!("{} copy", m.name);
                m.builtin = false;
                st.gamut_masks.push(m);
                st.active_mask = Some(st.gamut_masks.len() - 1);
            }
            if !st.gamut_masks[i].builtin && widgets::compact_button(ui, tl!("Delete"), 20.0).clicked() {
                st.gamut_masks.remove(i);
                st.active_mask = None;
            }
        }
    });
    if let Some(i) = st.active_mask.filter(|i| *i < st.gamut_masks.len()) {
        let m = &mut st.gamut_masks[i];
        if !m.builtin {
            ui.horizontal(|ui| {
                dim(ui, tl!("Name:"));
                ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(120.0));
            });
            // Edit: click points on the hue/saturation disc to draw a shape, Close adds it.
            dim(ui, tl!("Click the disc to add shape points"));
            let side = 120.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
            let center = rect.center();
            let r = side / 2.0 - 2.0;
            let tex = texture(ui, "gamut-mask-editor", 1, 48, |u, v| {
                let (x, y) = ((u - 0.5) * 2.0, (v - 0.5) * 2.0);
                let s = (x * x + y * y).sqrt();
                if s > 1.0 {
                    return [0.2, 0.2, 0.2];
                }
                models::hsv_to_rgb(y.atan2(x).to_degrees().rem_euclid(360.0), s, 1.0)
            });
            ui.painter().image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            let t = Tokens::get(ui.ctx());
            for sh in &m.shapes {
                draw_mask_shape(ui.painter(), center, r, &sh.points, 0.0, t.accent);
            }
            draw_mask_shape(ui.painter(), center, r, &st.mask_draft, 0.0, t.warning);
            if resp.clicked()
                && let Some(p) = resp.interact_pointer_pos()
            {
                let d = p - center;
                let s = (d.length() / r).clamp(0.0, 1.0);
                let h = d.y.atan2(d.x).to_degrees().rem_euclid(360.0);
                st.mask_draft.push([h, s]);
            }
            ui.horizontal(|ui| {
                if st.mask_draft.len() >= 3 && widgets::compact_button(ui, tl!("Close Shape"), 20.0).clicked() {
                    m.shapes.push(MaskShape { points: std::mem::take(&mut st.mask_draft) });
                }
                if !st.mask_draft.is_empty() && widgets::compact_button(ui, tl!("Clear"), 20.0).clicked() {
                    st.mask_draft.clear();
                }
                if !m.shapes.is_empty() && widgets::compact_button(ui, tl!("Remove Last Shape"), 20.0).clicked() {
                    m.shapes.pop();
                }
            });
        }
    }
}

fn draw_mask_shape(p: &egui::Painter, center: Pos2, r: f32, pts: &[[f32; 2]], rotation: f32, color: Color32) {
    if pts.is_empty() {
        return;
    }
    let to = |q: &[f32; 2]| {
        let (x, y) = polar_xy(q[0] + rotation, q[1]);
        pos2(center.x + x * r, center.y + y * r)
    };
    let poly: Vec<Pos2> = pts.iter().map(to).collect();
    if poly.len() >= 3 {
        p.add(egui::Shape::closed_line(poly.clone(), Stroke::new(1.5, color)));
        let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 40);
        p.add(egui::Shape::convex_polygon(poly, fill, Stroke::NONE));
    } else {
        for q in &poly {
            p.circle_filled(*q, 2.5, color);
        }
    }
}

// ------------------------------------------------------------------ advanced selector

fn advanced(app: &mut PhotocraftApp, ui: &mut egui::Ui, st: &mut ColorSelectorSettings) {
    let a = st.advanced.clone();
    let side = if a.fit_width { ui.available_width().clamp(80.0, 600.0) } else { a.size.clamp(80.0, 600.0) };
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    let rgb = fg(app);
    let cur = to_cyl(a.model, &a.luma, rgb);
    let key = egui::Id::new("color-advanced-hs");
    // Keep the hue and saturation of greys, so the marker doesn't jump.
    let (mut h, mut s) = ui.data(|d| d.get_temp::<(f32, f32)>(key)).unwrap_or((cur[0], cur[1]));
    if cur[1] > 0.01 && cur[2] > 0.01 {
        h = cur[0];
        s = cur[1];
    }
    let v = cur[2];
    let picked = draw_advanced(ui, rect, &resp, &a, st, h, s, v, "color-advanced");
    if let Some((nh, ns, nv)) = picked {
        let (nh, ns) = st.masked(nh, ns);
        ui.data_mut(|d| d.insert_temp(key, (nh, ns)));
        let out = from_cyl(a.model, &a.luma, nh, ns, nv);
        set_color(app, out, ui.input(|i| i.modifiers.alt));
    }
    // Zoom pop-up: a larger copy while dragging.
    if a.zoom_popup && resp.dragged() {
        let size = a.zoom_size.clamp(120.0, 800.0);
        let screen = ui.ctx().content_rect();
        let pos =
            pos2((rect.center().x - size / 2.0).clamp(screen.left(), (screen.right() - size).max(screen.left())), (rect.top() - size - 8.0).max(screen.top()));
        egui::Area::new(egui::Id::new("color-advanced-zoom")).order(egui::Order::Tooltip).fixed_pos(pos).show(ui.ctx(), |ui| {
            let (zr, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
            let zresp = ui.interact(zr, egui::Id::new("color-advanced-zoom-i"), Sense::hover());
            let _ = draw_advanced(ui, zr, &zresp, &a, st, h, s, v, "color-advanced-zoom");
        });
    }
}

/// Draw the advanced selector into `rect`; returns a picked (hue, saturation, third) when the
/// pointer is down in it.
#[allow(clippy::too_many_arguments)]
fn draw_advanced(
    ui: &mut egui::Ui,
    rect: Rect,
    resp: &egui::Response,
    a: &AdvancedSettings,
    st: &ColorSelectorSettings,
    h: f32,
    s: f32,
    v: f32,
    slot: &str,
) -> Option<(f32, f32, f32)> {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    let center = rect.center();
    let outer = rect.width().min(rect.height()) / 2.0;
    let ring_w = (outer * 0.16).max(8.0);
    let inner = outer - ring_w;
    let model = a.model;
    let luma = a.luma;
    let shade_v = if a.shape == Shape::Wheel { v } else { 1.0 };
    // Hue ring (or the whole wheel).
    let n = 96;
    let mut mesh = egui::Mesh::default();
    for i in 0..n {
        let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        let c0 = c32(from_cyl(model, &luma, a0.to_degrees(), 1.0, if model == ColorModel::Hsv { 1.0 } else { 0.5 }));
        let c1 = c32(from_cyl(model, &luma, a1.to_degrees(), 1.0, if model == ColorModel::Hsv { 1.0 } else { 0.5 }));
        let (r_in, c_in0, c_in1) = if a.shape == Shape::Wheel {
            (0.0, c32(from_cyl(model, &luma, a0.to_degrees(), 0.0, shade_v)), c32(from_cyl(model, &luma, a1.to_degrees(), 0.0, shade_v)))
        } else {
            (inner, c0, c1)
        };
        let (c0, c1) = if a.shape == Shape::Wheel {
            (c32(from_cyl(model, &luma, a0.to_degrees(), 1.0, shade_v)), c32(from_cyl(model, &luma, a1.to_degrees(), 1.0, shade_v)))
        } else {
            (c0, c1)
        };
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(center + vec2(a0.cos(), a0.sin()) * outer, c0);
        mesh.colored_vertex(center + vec2(a1.cos(), a1.sin()) * outer, c1);
        mesh.colored_vertex(center + vec2(a1.cos(), a1.sin()) * r_in, c_in1);
        mesh.colored_vertex(center + vec2(a0.cos(), a0.sin()) * r_in, c_in0);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    p.add(egui::Shape::mesh(mesh));
    let hr = h.to_radians();
    // Inner shape.
    let mut picked = None;
    let pointer = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked() || resp.drag_started());
    match a.shape {
        Shape::Wheel => {
            // Marker at (hue, saturation); the value slider is a bar under the wheel is not
            // available in a square allocation, so the value follows the vertical wheel drag
            // with Shift held.
            let m = center + vec2(hr.cos(), hr.sin()) * (s * outer);
            p.circle_stroke(m, 5.0, Stroke::new(1.5, Color32::WHITE));
            p.circle_stroke(m, 6.5, Stroke::new(1.0, Color32::BLACK));
            if let Some(pp) = pointer {
                let d = pp - center;
                let dist = d.length();
                if resp.drag_started() || resp.clicked() {
                    ui.data_mut(|dm| dm.insert_temp(egui::Id::new((slot, "wheel-in")), dist <= outer + 4.0));
                }
                let inside: bool = ui.data(|dm| dm.get_temp(egui::Id::new((slot, "wheel-in")))).unwrap_or(true);
                if inside {
                    let nh = d.y.atan2(d.x).to_degrees().rem_euclid(360.0);
                    let ns = (dist / outer).clamp(0.0, 1.0);
                    let nv = if ui.input(|i| i.modifiers.shift) { (1.0 - (pp.y - rect.top()) / rect.height()).clamp(0.0, 1.0) } else { v };
                    picked = Some((nh, ns, nv));
                }
            }
        }
        Shape::Triangle => {
            // Vertices: the hue at the ring's angle, white and black at ±120°.
            let r = inner - 2.0;
            let pv = |ang: f32| center + vec2(ang.cos(), ang.sin()) * r;
            let (p_h, p_w, p_k) = (pv(hr), pv(hr + 2.0 * std::f32::consts::FRAC_PI_3), pv(hr - 2.0 * std::f32::consts::FRAC_PI_3));
            let sig = hash((slot, a.shape as u8, model as u8, bits(h), bits(luma.r), bits(luma.g), bits(luma.b), bits(luma.gamma)));
            // A texture of the triangle in barycentric coordinates: u → saturation, v → value.
            let tex = texture(ui, &format!("{slot}-tri"), sig, 64, |u, w| {
                // u along white→hue (saturation), w from the hue/white edge down to black (value).
                let sat = u;
                let val = 1.0 - w;
                from_cyl(model, &luma, h, sat, val)
            });
            // Draw the triangle as a textured mesh: map the (sat, val) square's three corners.
            let mut mesh = egui::Mesh::with_texture(tex.id());
            let uv = |sat: f32, val: f32| pos2(sat, 1.0 - val);
            mesh.vertices.push(egui::epaint::Vertex { pos: p_h, uv: uv(1.0, 1.0), color: Color32::WHITE });
            mesh.vertices.push(egui::epaint::Vertex { pos: p_w, uv: uv(0.0, 1.0), color: Color32::WHITE });
            mesh.vertices.push(egui::epaint::Vertex { pos: p_k, uv: uv(0.0, 0.0), color: Color32::WHITE });
            mesh.add_triangle(0, 1, 2);
            p.add(egui::Shape::mesh(mesh));
            // The marker: sat/val → barycentric position.
            let bary = |sat: f32, val: f32| {
                // p = k + val × (w − k) + sat × val × (h − w)
                let pw = p_k + (p_w - p_k) * val;
                pw + (p_h - p_w) * (sat * val)
            };
            let m = bary(s, v);
            p.circle_stroke(m, 5.0, Stroke::new(1.5, Color32::WHITE));
            p.circle_stroke(m, 6.5, Stroke::new(1.0, Color32::BLACK));
            if let Some(pp) = pointer {
                let d = pp - center;
                let dist = d.length();
                let on_ring_now = dist > inner && dist <= outer + 4.0;
                if resp.drag_started() || resp.clicked() {
                    ui.data_mut(|dm| dm.insert_temp(egui::Id::new((slot, "ring")), on_ring_now));
                }
                let on_ring: bool = ui.data(|dm| dm.get_temp(egui::Id::new((slot, "ring")))).unwrap_or(on_ring_now);
                if on_ring {
                    picked = Some((d.y.atan2(d.x).to_degrees().rem_euclid(360.0), s, v));
                } else {
                    // Invert the barycentric mapping: solve for val along k→w/h, then sat.
                    let (ns, nv) = tri_inverse(pp, p_h, p_w, p_k);
                    picked = Some((h, ns, nv));
                }
            }
        }
        Shape::Square => {
            let half = inner / std::f32::consts::SQRT_2 - 2.0;
            let sq = Rect::from_center_size(center, vec2(half * 2.0, half * 2.0));
            let sig = hash((slot, a.shape as u8, model as u8, bits(h), bits(luma.r), bits(luma.g), bits(luma.b), bits(luma.gamma)));
            let tex = texture(ui, &format!("{slot}-sq"), sig, 64, |u, w| from_cyl(model, &luma, h, u, 1.0 - w));
            p.image(tex.id(), sq, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            let m = pos2(sq.left() + s * sq.width(), sq.top() + (1.0 - v) * sq.height());
            p.circle_stroke(m, 5.0, Stroke::new(1.5, Color32::WHITE));
            p.circle_stroke(m, 6.5, Stroke::new(1.0, Color32::BLACK));
            if let Some(pp) = pointer {
                let d = pp - center;
                let dist = d.length();
                let on_ring_now = dist > inner && dist <= outer + 4.0;
                if resp.drag_started() || resp.clicked() {
                    ui.data_mut(|dm| dm.insert_temp(egui::Id::new((slot, "ring")), on_ring_now));
                }
                let on_ring: bool = ui.data(|dm| dm.get_temp(egui::Id::new((slot, "ring")))).unwrap_or(on_ring_now);
                if on_ring {
                    picked = Some((d.y.atan2(d.x).to_degrees().rem_euclid(360.0), s, v));
                } else {
                    let ns = ((pp.x - sq.left()) / sq.width()).clamp(0.0, 1.0);
                    let nv = (1.0 - (pp.y - sq.top()) / sq.height()).clamp(0.0, 1.0);
                    picked = Some((h, ns, nv));
                }
            }
        }
    }
    // Hue marker on the ring.
    let hm = center + vec2(hr.cos(), hr.sin()) * (outer - ring_w / 2.0);
    p.circle_stroke(hm, ring_w / 2.0 - 1.0, Stroke::new(1.5, Color32::WHITE));
    p.circle_stroke(hm, ring_w / 2.0, Stroke::new(1.0, Color32::BLACK));
    // Gamut mask over the wheel (hue by angle, saturation by radius).
    if st.mask_show
        && let Some(m) = st.mask()
    {
        for sh in &m.shapes {
            draw_mask_shape(p, center, if a.shape == Shape::Wheel { outer } else { inner }, &sh.points, st.mask_rotation, t.accent);
        }
    }
    let _ = t;
    picked
}

/// Saturation and value of point `p` inside the triangle (hue vertex `ph`, white `pw`, black
/// `pk`), clamped to the triangle.
fn tri_inverse(p: Pos2, ph: Pos2, pw: Pos2, pk: Pos2) -> (f32, f32) {
    // Barycentric coordinates.
    let v0 = pw - pk;
    let v1 = ph - pk;
    let v2 = p - pk;
    let d00 = v0.dot(v0);
    let d01 = v0.dot(v1);
    let d11 = v1.dot(v1);
    let d20 = v2.dot(v0);
    let d21 = v2.dot(v1);
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-6 {
        return (0.0, 0.0);
    }
    let bw = ((d11 * d20 - d01 * d21) / den).clamp(0.0, 1.0); // weight of white
    let bh = ((d00 * d21 - d01 * d20) / den).clamp(0.0, 1.0); // weight of hue
    let (bw, bh) = if bw + bh > 1.0 {
        let sum = bw + bh;
        (bw / sum, bh / sum)
    } else {
        (bw, bh)
    };
    // p = k + val(w − k) + sat·val(h − w): val = bw + bh, sat = bh / val.
    let val = (bw + bh).clamp(0.0, 1.0);
    let sat = if val > 1e-4 { (bh / val).clamp(0.0, 1.0) } else { 0.0 };
    (sat, val)
}

// ------------------------------------------------------------------ shade strip

/// Shades of `base`: MyPaint-style rows (hue ±, saturation ±, lightness ±) or one lightness row.
pub fn shades(base: [f32; 3], style: ShadeStyle, count: u32, spread: f32) -> Vec<Vec<[f32; 3]>> {
    let n = count.clamp(3, 21) as i32;
    let half = n / 2;
    let [h, s, l] = models::rgb_to_hsl(base);
    let k = spread.clamp(0.05, 1.0);
    let step = |i: i32| i as f32 / half.max(1) as f32;
    let light = |i: i32| models::hsl_to_rgb(h, s, (l + step(i) * k * 0.5).clamp(0.0, 1.0));
    match style {
        ShadeStyle::Simple => vec![(-half..=half).map(light).collect()],
        ShadeStyle::Rows => vec![
            (-half..=half).map(|i| models::hsl_to_rgb(h + step(i) * k * 60.0, s, l)).collect(),
            (-half..=half).map(|i| models::hsl_to_rgb(h, (s + step(i) * k).clamp(0.0, 1.0), l)).collect(),
            (-half..=half).map(light).collect(),
            (-half..=half)
                .map(|i| models::hsl_to_rgb(h + step(i) * k * 30.0, (s - step(i) * k * 0.5).clamp(0.0, 1.0), (l + step(i) * k * 0.3).clamp(0.0, 1.0)))
                .collect(),
        ],
    }
}

fn shade_strip(app: &mut PhotocraftApp, ui: &mut egui::Ui, st: &mut ColorSelectorSettings) {
    let cur = fg(app);
    let sh = &mut st.shade;
    if sh.update == ShadeUpdate::Always {
        sh.base = cur;
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        dim(ui, tl!("Shades"));
        if sh.update != ShadeUpdate::Always && widgets::compact_button(ui, tl!("Update"), 18.0).clicked() {
            sh.base = cur;
        }
    });
    let rows = shades(sh.base, sh.style, sh.count, sh.spread);
    let (side, update) = (sh.size, sh.update);
    let mut picked = None;
    for (ri, row) in rows.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
            for (i, c) in row.iter().enumerate() {
                if let Some(alt) = patch(ui, side, *c, &format!("Shade {} {}", ri + 1, i + 1), false) {
                    picked = Some((*c, alt));
                }
            }
        });
    }
    if let Some((c, alt)) = picked {
        if update == ShadeUpdate::OnPick {
            st.shade.base = c;
        }
        set_color(app, c, alt);
    }
}

// ------------------------------------------------------------------ artistic selector

fn artistic(app: &mut PhotocraftApp, ui: &mut egui::Ui, st: &mut ColorSelectorSettings) {
    let a = st.artistic.clone();
    let side = a.size.clamp(80.0, 600.0).min(ui.available_width().max(80.0));
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    let rgb = fg(app);
    let [h, s, v] = models::rgb_to_hsv(rgb);
    let key = egui::Id::new("color-artistic-hs");
    let (mut hh, mut ss) = ui.data(|d| d.get_temp::<(f32, f32)>(key)).unwrap_or((h, s));
    if s > 0.01 && v > 0.01 {
        hh = h;
        ss = s;
    }
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    let center = rect.center();
    let outer = side / 2.0;
    let ring_w = outer * 0.22;
    let inner = outer - ring_w;
    let hue_steps = if a.continuous { 0 } else { a.hue_steps.clamp(3, 72) };
    let sat_steps = a.sat_steps.clamp(1, 12);
    let value_steps = a.value_steps.clamp(2, 24);
    let ring_v = if a.shade_by_value { v.max(0.05) } else { 1.0 };
    // The ring: hue steps (or continuous) × saturation rings (or value rings).
    let segs = if hue_steps == 0 { 96 } else { hue_steps };
    let rings = if a.value_ring { value_steps } else { sat_steps };
    let mut mesh = egui::Mesh::default();
    for i in 0..segs {
        let a0 = i as f32 / segs as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / segs as f32 * std::f32::consts::TAU;
        let hue = if hue_steps == 0 { ((a0 + a1) / 2.0).to_degrees() } else { (i as f32 + 0.5) / segs as f32 * 360.0 };
        for k in 0..rings {
            let r0 = inner + ring_w * k as f32 / rings as f32;
            let r1 = inner + ring_w * (k + 1) as f32 / rings as f32;
            let f = (k as f32 + 0.5) / rings as f32;
            let c = if a.value_ring { models::hsv_to_rgb(hue, ss.max(0.05), f) } else { models::hsv_to_rgb(hue, f, ring_v) };
            let masked = st.mask().is_some_and(|m| !m.contains(hue, if a.value_ring { ss } else { f }, st.mask_rotation));
            let col = if masked && st.mask_show {
                Color32::from_rgba_unmultiplied((c[0] * 60.0) as u8, (c[1] * 60.0) as u8, (c[2] * 60.0) as u8, 255)
            } else {
                c32(c)
            };
            let base = mesh.vertices.len() as u32;
            let gap = if hue_steps == 0 { 0.0 } else { 0.012 };
            let (b0, b1) = (a0 + gap, a1 - gap);
            mesh.colored_vertex(center + vec2(b0.cos(), b0.sin()) * r1, col);
            mesh.colored_vertex(center + vec2(b1.cos(), b1.sin()) * r1, col);
            mesh.colored_vertex(center + vec2(b1.cos(), b1.sin()) * r0, col);
            mesh.colored_vertex(center + vec2(b0.cos(), b0.sin()) * r0, col);
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
    }
    p.add(egui::Shape::mesh(mesh));
    // Inner block: value steps (or saturation steps) of the current hue as bars.
    let block = Rect::from_center_size(center, vec2(inner * 1.3, inner * 1.3));
    let steps = if a.value_ring { sat_steps } else { value_steps };
    let bw = block.width() / steps as f32;
    for k in 0..steps {
        let f = (k as f32 + 0.5) / steps as f32;
        let c = if a.value_ring { models::hsv_to_rgb(hh, f, v.max(0.05)) } else { models::hsv_to_rgb(hh, ss, f) };
        let r = Rect::from_min_size(pos2(block.left() + bw * k as f32, block.top()), vec2(bw, block.height()));
        p.rect_filled(r.shrink(0.5), 0.0, c32(c));
    }
    // Markers.
    let hr = hh.to_radians();
    let ring_f = if a.value_ring { v } else { ss };
    let m = center + vec2(hr.cos(), hr.sin()) * (inner + ring_w * ring_f.clamp(0.0, 1.0));
    p.circle_stroke(m, 5.0, Stroke::new(1.5, Color32::WHITE));
    p.circle_stroke(m, 6.5, Stroke::new(1.0, Color32::BLACK));
    let bf = if a.value_ring { ss } else { v };
    let bx = block.left() + bf.clamp(0.0, 1.0) * block.width();
    p.line_segment([pos2(bx, block.top()), pos2(bx, block.bottom())], Stroke::new(2.0, Color32::WHITE));
    p.line_segment([pos2(bx, block.top()), pos2(bx, block.bottom())], Stroke::new(0.8, Color32::BLACK));
    if st.mask_show
        && let Some(mk) = st.mask()
    {
        for sh in &mk.shapes {
            draw_mask_shape(p, center, outer, &sh.points, st.mask_rotation, t.accent);
        }
    }
    // Picking.
    if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked() || resp.drag_started()) {
        let d = pp - center;
        let dist = d.length();
        let in_block_now = block.contains(pp) && dist < inner;
        if resp.drag_started() || resp.clicked() {
            ui.data_mut(|dm| dm.insert_temp(egui::Id::new("color-artistic-block"), in_block_now));
        }
        let in_block: bool = ui.data(|dm| dm.get_temp(egui::Id::new("color-artistic-block"))).unwrap_or(in_block_now);
        let (mut nh, mut ns, mut nv) = (hh, ss, v);
        if in_block {
            let f = ((pp.x - block.left()) / block.width()).clamp(0.0, 1.0);
            let f = if hue_steps == 0 { f } else { ((f * steps as f32).floor() + 0.5) / steps as f32 };
            if a.value_ring {
                ns = f;
            } else {
                nv = f;
            }
        } else {
            let ang = d.y.atan2(d.x).to_degrees().rem_euclid(360.0);
            nh = if hue_steps == 0 { ang } else { ((ang / 360.0 * segs as f32).floor() + 0.5) / segs as f32 * 360.0 };
            let f = ((dist - inner) / ring_w).clamp(0.0, 1.0);
            let f = if hue_steps == 0 { f } else { ((f * rings as f32).floor().min(rings as f32 - 1.0) + 0.5) / rings as f32 };
            if a.value_ring {
                nv = f;
            } else {
                ns = f;
            }
        }
        let (nh, ns) = st.masked(nh, ns);
        ui.data_mut(|d| d.insert_temp(key, (nh, ns)));
        set_color(app, models::hsv_to_rgb(nh, ns, nv), ui.input(|i| i.modifiers.alt));
    }
}

// ------------------------------------------------------------------ wide-gamut selector

fn wide_gamut(app: &mut PhotocraftApp, ui: &mut egui::Ui, st: &mut ColorSelectorSettings) {
    let w = st.wide.clone();
    let side = w.size.clamp(80.0, 600.0).min(ui.available_width().max(80.0));
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    // The picked colour lives in the wide space; the foreground is its sRGB proof. When the
    // foreground changed elsewhere, pick it up.
    let cur_srgb = fg(app);
    let picked_srgb = w.space.to_srgb(w.picked);
    let mut picked = if picked_srgb.iter().zip(cur_srgb).all(|(a, b)| (a - b).abs() < 2.0 / 255.0) { w.picked } else { w.space.from_srgb(cur_srgb) };
    let [h, s, v] = models::rgb_to_hsv(picked);
    let key = egui::Id::new("color-wide-hs");
    let (mut hh, mut ss) = ui.data(|d| d.get_temp::<(f32, f32)>(key)).unwrap_or((h, s));
    if s > 0.01 && v > 0.01 {
        hh = h;
        ss = s;
    }
    let p = ui.painter();
    let center = rect.center();
    let outer = side / 2.0;
    let ring_w = outer * 0.16;
    let inner = outer - ring_w;
    let space = w.space;
    let n = 96;
    let mut mesh = egui::Mesh::default();
    for i in 0..n {
        let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        let (rs, rv) = if w.dynamic_ring { (ss.max(0.05), v.max(0.05)) } else { (1.0, 1.0) };
        let c0 = c32(space.to_srgb(models::hsv_to_rgb(a0.to_degrees(), rs, rv)));
        let c1 = c32(space.to_srgb(models::hsv_to_rgb(a1.to_degrees(), rs, rv)));
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(center + vec2(a0.cos(), a0.sin()) * outer, c0);
        mesh.colored_vertex(center + vec2(a1.cos(), a1.sin()) * outer, c1);
        mesh.colored_vertex(center + vec2(a1.cos(), a1.sin()) * inner, c1);
        mesh.colored_vertex(center + vec2(a0.cos(), a0.sin()) * inner, c0);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    p.add(egui::Shape::mesh(mesh));
    let half = inner / std::f32::consts::SQRT_2 - 2.0;
    let sq = Rect::from_center_size(center, vec2(half * 2.0, half * 2.0));
    let sig = hash(("color-wide", space as u8, bits(hh), w.proof));
    let proof = w.proof;
    let tex = texture(ui, "color-wide-sq", sig, 64, |u, ww| {
        let c = models::hsv_to_rgb(hh, u, 1.0 - ww);
        if proof && space.out_of_srgb(c) {
            // Out of the painting gamut: shown dimmed with a hatch-like checker.
            let s = space.to_srgb(c);
            let k = if ((u * 16.0).floor() as i32 + (ww * 16.0).floor() as i32) % 2 == 0 { 0.75 } else { 0.6 };
            [s[0] * k, s[1] * k, s[2] * k]
        } else {
            space.to_srgb(c)
        }
    });
    p.image(tex.id(), sq, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    let m = pos2(sq.left() + ss * sq.width(), sq.top() + (1.0 - v) * sq.height());
    p.circle_stroke(m, 5.0, Stroke::new(1.5, Color32::WHITE));
    p.circle_stroke(m, 6.5, Stroke::new(1.0, Color32::BLACK));
    let hr = hh.to_radians();
    let hm = center + vec2(hr.cos(), hr.sin()) * (outer - ring_w / 2.0);
    p.circle_stroke(hm, ring_w / 2.0 - 1.0, Stroke::new(1.5, Color32::WHITE));
    p.circle_stroke(hm, ring_w / 2.0, Stroke::new(1.0, Color32::BLACK));
    let mut changed = false;
    if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked() || resp.drag_started()) {
        let d = pp - center;
        let dist = d.length();
        let on_ring_now = dist > inner && dist <= outer + 4.0;
        if resp.drag_started() || resp.clicked() {
            ui.data_mut(|dm| dm.insert_temp(egui::Id::new("color-wide-ring"), on_ring_now));
        }
        let on_ring: bool = ui.data(|dm| dm.get_temp(egui::Id::new("color-wide-ring"))).unwrap_or(on_ring_now);
        let (nh, ns, nv) = if on_ring {
            (d.y.atan2(d.x).to_degrees().rem_euclid(360.0), ss, v)
        } else {
            (hh, ((pp.x - sq.left()) / sq.width()).clamp(0.0, 1.0), (1.0 - (pp.y - sq.top()) / sq.height()).clamp(0.0, 1.0))
        };
        ui.data_mut(|d| d.insert_temp(key, (nh, ns)));
        picked = models::hsv_to_rgb(nh, ns, nv);
        changed = true;
    }
    // Readout: the wide-space values, the proof and the gamut flag.
    let (srgb, out) = models::proof_to_srgb(space, picked);
    ui.horizontal(|ui| {
        dim(ui, space.label());
        ui.label(format!("{:.3} {:.3} {:.3}", picked[0], picked[1], picked[2]));
        if out {
            let t = Tokens::get(ui.ctx());
            ui.label(egui::RichText::new(tl!("Out of gamut")).color(t.warning));
        }
    });
    if w.shade_strip {
        let rows = shades(srgb, ShadeStyle::Simple, 9, 0.4);
        let mut pick = None;
        for row in &rows {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
                for (i, c) in row.iter().enumerate() {
                    if let Some(alt) = patch(ui, w.patch_size.clamp(8.0, 40.0), *c, &format!("Wide shade {}", i + 1), false) {
                        pick = Some((*c, alt));
                    }
                }
            });
        }
        if let Some((c, alt)) = pick {
            set_color(app, c, alt);
        }
    }
    if w.patch_count > 0 {
        let history: Vec<[f32; 3]> = app.ui.color_history.iter().take(w.patch_count as usize).map(|c| [c[0], c[1], c[2]]).collect();
        if let Some((c, alt)) = patches(ui, &history, w.patch_size.clamp(8.0, 40.0), w.patches, "Wide patch", false) {
            set_color(app, c, alt);
        }
    }
    st.wide.picked = picked;
    if changed {
        set_color(app, srgb, ui.input(|i| i.modifiers.alt));
    }
}

// ------------------------------------------------------------------ specific selector

fn specific(app: &mut PhotocraftApp, ui: &mut egui::Ui, st: &mut ColorSelectorSettings) {
    let sp = &mut st.specific;
    let rgb = fg(app);
    let model = sp.model;
    let depth = sp.depth;
    let luma = sp.luma;
    let shown = models::to_model(rgb, model, &luma);
    // Keep typed float values that round-trip to the same tool colour (so an HDR value above 1
    // or a precise 16-bit value survives the sRGB clamp).
    let round_trip = models::from_model(sp.channels, model, &luma);
    let same = round_trip.iter().zip(rgb).all(|(a, b)| (a - b).abs() < 1.5 / 255.0);
    let mut ch = if same { sp.channels } else { shown };
    let mut changed = false;
    for (i, name) in model.channels().iter().enumerate() {
        let (lo, hi, unit) = model.channel_range(i);
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(vec2(28.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| dim(ui, name));
            let (mut shown_v, range, suffix, decimals) = if sp.normalized || model.is_cylindrical() || matches!(model, ColorModel::Lab) {
                (lo + ch[i] * (hi - lo), lo..=hi, unit, if (hi - lo) > 2.0 { 0 } else { 3 })
            } else {
                match depth.max_code() {
                    Some(m) => (depth.display_value(ch[i]), 0.0..=m as f32, "", 0),
                    None => (depth.quantize(ch[i]), -16.0..=16.0, "", if depth == Depth::F16 { 3 } else { 4 }),
                }
            };
            let mut slider = shown_v;
            let r = widgets::value_field_prec(ui, &mut shown_v, range.clone(), suffix, 78.0, decimals);
            let sr = ui.add(egui::Slider::new(&mut slider, range.clone()).show_value(false));
            if sr.changed() {
                shown_v = slider;
            }
            if r.changed() || sr.changed() {
                ch[i] = if sp.normalized || model.is_cylindrical() || matches!(model, ColorModel::Lab) {
                    ((shown_v - lo) / (hi - lo)).clamp(0.0, 1.0)
                } else {
                    match depth.max_code() {
                        Some(_) => depth.from_display(shown_v),
                        None => shown_v,
                    }
                };
                changed = true;
            }
        });
    }
    // Hex field.
    ui.horizontal(|ui| {
        dim(ui, "#");
        let key = egui::Id::new("color-specific-hex");
        let mut text: String = ui.data(|d| d.get_temp(key)).unwrap_or_else(|| crate::color_picker_ui::hex(rgb).trim_start_matches('#').to_string());
        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(70.0));
        if r.changed()
            && let Some(c) = crate::color_picker_ui::parse_hex(&text)
        {
            ch = models::to_model(c, model, &luma);
            changed = true;
        }
        if r.has_focus() {
            ui.data_mut(|d| d.insert_temp(key, text));
        } else {
            ui.data_mut(|d| d.remove::<String>(key));
        }
        dim(ui, depth.label());
    });
    if changed {
        sp.channels = ch;
        let out = models::from_model(ch, model, &luma);
        set_color(app, out, ui.input(|i| i.modifiers.alt));
    } else {
        sp.channels = ch;
    }
}

// ------------------------------------------------------------------ digital colour mixer

fn mixer(app: &mut PhotocraftApp, ui: &mut egui::Ui, st: &mut ColorSelectorSettings) {
    let m = &mut st.mixer;
    let cur = fg(app);
    let mut pick: Option<([f32; 3], bool)> = None;
    let mut set_column: Option<usize> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
        for i in 0..6 {
            ui.vertical(|ui| {
                let col = m.colors[i];
                let mixed = [0, 1, 2].map(|k| cur[k] + (col[k] - cur[k]) * m.amounts[i]);
                if let Some(alt) = patch(ui, 26.0, mixed, &format!("Mix {}", i + 1), false) {
                    pick = Some((mixed, alt));
                }
                let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 70.0), Sense::click_and_drag());
                let p = ui.painter();
                let mut mesh = egui::Mesh::default();
                let base = 0;
                mesh.colored_vertex(rect.left_top(), c32(col));
                mesh.colored_vertex(rect.right_top(), c32(col));
                mesh.colored_vertex(rect.right_bottom(), c32(cur));
                mesh.colored_vertex(rect.left_bottom(), c32(cur));
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base, base + 2, base + 3);
                p.add(egui::Shape::mesh(mesh));
                let y = rect.bottom() - m.amounts[i].clamp(0.0, 1.0) * rect.height();
                p.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(2.0, Color32::WHITE));
                p.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(0.8, Color32::BLACK));
                if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
                    m.amounts[i] = (1.0 - (pp.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                }
                // The column's own colour: click to set it from the foreground.
                if let Some(alt) = patch(ui, 26.0, col, &format!("Mixer color {}", i + 1), false) {
                    if alt { set_column = Some(i) } else { pick = Some((col, false)) }
                }
            });
        }
    });
    dim(ui, tl!("Alt-click a bottom patch to set it to the foreground colour"));
    // Gradient slider.
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().clamp(60.0, 400.0), 18.0), Sense::click_and_drag());
    let p = ui.painter();
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), c32(m.gradient[0]));
    mesh.colored_vertex(rect.right_top(), c32(m.gradient[1]));
    mesh.colored_vertex(rect.right_bottom(), c32(m.gradient[1]));
    mesh.colored_vertex(rect.left_bottom(), c32(m.gradient[0]));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(egui::Shape::mesh(mesh));
    let x = rect.left() + m.gradient_pos.clamp(0.0, 1.0) * rect.width();
    p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(2.0, Color32::WHITE));
    p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(0.8, Color32::BLACK));
    if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
        m.gradient_pos = ((pp.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let g = [0, 1, 2].map(|k| m.gradient[0][k] + (m.gradient[1][k] - m.gradient[0][k]) * m.gradient_pos);
        pick = Some((g, ui.input(|i| i.modifiers.alt)));
    }
    ui.horizontal(|ui| {
        if let Some(alt) = patch(ui, 16.0, m.gradient[0], "Gradient start", false) {
            if alt { m.gradient[0] = cur } else { pick = Some((m.gradient[0], false)) }
        }
        dim(ui, tl!("Gradient ends: Alt-click to set from the foreground"));
        if let Some(alt) = patch(ui, 16.0, m.gradient[1], "Gradient end", false) {
            if alt { m.gradient[1] = cur } else { pick = Some((m.gradient[1], false)) }
        }
    });
    if let Some(i) = set_column {
        m.colors[i] = cur;
    }
    if let Some((c, alt)) = pick {
        set_color(app, c, alt);
    }
}

// ------------------------------------------------------------------ common colours

fn common_colors(app: &mut PhotocraftApp, ui: &mut egui::Ui, st: &mut ColorSelectorSettings) {
    let co = st.common.clone();
    let key = app.session.active().map(|d| (d.doc.id.0, d.revision));
    let colors: Vec<[f32; 3]> = match key {
        None => Vec::new(),
        Some(k) => {
            if st.common_cache.as_ref().is_none_or(|(ck, _)| *ck != k) {
                let got = app
                    .session
                    .execute("colors.common", json!({ "count": co.count, "allLayers": co.all_layers }))
                    .ok()
                    .and_then(|v| serde_json::from_value::<Vec<[f32; 3]>>(v["colors"].clone()).ok())
                    .unwrap_or_default();
                st.common_cache = Some((k, got));
            }
            st.common_cache.as_ref().map(|(_, c)| c.clone()).unwrap_or_default()
        }
    };
    if colors.is_empty() {
        return;
    }
    ui.add_space(4.0);
    dim(ui, tl!("Common Colors"));
    if let Some((c, alt)) = patches(ui, &colors, co.size.clamp(8.0, 40.0), co.layout, "Common color", co.scroll) {
        set_color(app, c, alt);
    }
}

/// The recent-colours row with the settings' count, size and direction (the Color panel's
/// history row; the pop-up palette has its own ring).
pub fn history_patches(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let co = app.ui.color_selectors.common.clone();
    let n = if co.history_count == 0 { color_history::MAX } else { co.history_count as usize };
    let colors: Vec<[f32; 3]> = app.ui.color_history.iter().take(n).map(|c| [c[0], c[1], c[2]]).collect();
    if colors.is_empty() {
        return;
    }
    ui.add_space(4.0);
    if let Some((c, alt)) = patches(ui, &colors, co.history_size.clamp(8.0, 40.0), co.history_layout, "Recent color", false) {
        set_color(app, c, alt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamut_masks_contain_and_clamp() {
        let masks = GamutMask::builtins();
        let comp = masks.iter().find(|m| m.name == "Complementary").unwrap();
        assert!(comp.contains(5.0, 0.8, 0.0));
        assert!(comp.contains(185.0, 0.5, 0.0));
        assert!(!comp.contains(90.0, 0.8, 0.0));
        assert!(comp.contains(95.0, 0.8, 90.0), "rotated by 90°");
        let (h, s) = comp.clamp(90.0, 0.8, 0.0);
        assert!(comp.contains(h, s + 0.0, 0.0) || (comp.clamp(h, s, 0.0).0 - h).abs() < 1.0, "{h} {s}");
        let empty = GamutMask::default();
        assert!(empty.contains(123.0, 0.5, 0.0));
    }

    #[test]
    fn shades_keep_the_base_in_the_middle() {
        let base = [0.3, 0.5, 0.7];
        for style in [ShadeStyle::Rows, ShadeStyle::Simple] {
            let rows = shades(base, style, 9, 0.3);
            for row in rows {
                assert_eq!(row.len(), 9);
                let mid = row[4];
                assert!(mid.iter().zip(base).all(|(a, b)| (a - b).abs() < 0.02), "{style:?}: {mid:?}");
            }
        }
    }

    #[test]
    fn triangle_inverse_recovers_corners() {
        let (ph, pw, pk) = (pos2(100.0, 0.0), pos2(0.0, 100.0), pos2(0.0, 0.0));
        assert_eq!(tri_inverse(ph, ph, pw, pk), (1.0, 1.0));
        assert_eq!(tri_inverse(pw, ph, pw, pk), (0.0, 1.0));
        let (s, v) = tri_inverse(pk, ph, pw, pk);
        assert!(v < 1e-4 && s < 1e-4);
    }
}
