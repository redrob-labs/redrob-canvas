//! The Layers panel's filter row (#5048): a filter-type dropdown, as in Photoshop, then the
//! controls for that type. Kind toggles layer kinds, Name matches part of the layer name, Mode
//! picks one blend mode. The criteria live in `UiState` (`layer_filter*`), so the control channel
//! can read and set them.

use redrob_photo_color::BlendMode;
use redrob_photo_doc::{Layer, LayerContent};
use serde::{Deserialize, Serialize};

use crate::state::UiState;
use crate::{PhotoApp, icons, widgets};

/// What the Layers panel filters by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterBy {
    #[default]
    Kind,
    Name,
    Mode,
    /// Colour label (one or more; a layer with any of them shows).
    Label,
}

/// Thumbnail size of the Layers panel rows (display settings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThumbnailSize {
    None,
    Small,
    #[default]
    Medium,
    Large,
}

impl ThumbnailSize {
    /// The row's thumbnail size as a factor of the default.
    pub fn factor(self) -> f32 {
        match self {
            ThumbnailSize::None => 0.0,
            ThumbnailSize::Small => 0.7,
            ThumbnailSize::Medium => 1.0,
            ThumbnailSize::Large => 1.6,
        }
    }
}

/// How much blending detail a layer row shows under its name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlendDetail {
    /// Nothing but the name.
    #[default]
    Hidden,
    /// The blend mode when it is not Normal.
    Mode,
    /// Blend mode and opacity, always.
    ModeAndOpacity,
}

/// The Layers panel's display settings (the panel menu's Display Settings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LayerDisplay {
    pub thumbnail: ThumbnailSize,
    pub blend_detail: BlendDetail,
}

impl LayerDisplay {
    /// The detail line under a layer's name, if the settings ask for one.
    pub fn detail_line(&self, l: &Layer) -> Option<String> {
        match self.blend_detail {
            BlendDetail::Hidden => None,
            BlendDetail::Mode => (l.blend != BlendMode::Normal && l.blend != BlendMode::PassThrough).then(|| tl!(l.blend.label()).to_string()),
            BlendDetail::ModeAndOpacity => Some(format!("{} {:.0}%", tl!(l.blend.label()), l.opacity * 100.0)),
        }
    }
}

/// The filter criteria, copied out of `UiState` for one frame of the Layers panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub by: FilterBy,
    pub kinds: Vec<String>,
    /// As typed; matched trimmed and ignoring case.
    pub name: String,
    pub mode: BlendMode,
    /// Colour label ids (`LabelColor::id`).
    pub labels: Vec<String>,
}

impl Filter {
    pub fn of(ui: &UiState) -> Self {
        Self {
            by: ui.layer_filter_by,
            kinds: ui.layer_filter.clone(),
            name: ui.layer_filter_name.clone(),
            mode: ui.layer_filter_mode,
            labels: ui.layer_filter_labels.clone(),
        }
    }

    /// Whether the panel is filtered. Like Photoshop, Mode always filters (by Normal at first);
    /// Kind and Name only once a kind is toggled or some text is typed.
    pub fn active(&self) -> bool {
        match self.by {
            FilterBy::Kind => !self.kinds.is_empty(),
            FilterBy::Name => !self.name.trim().is_empty(),
            FilterBy::Mode => true,
            FilterBy::Label => !self.labels.is_empty(),
        }
    }

    /// Whether `l` is listed under this filter.
    pub fn shows(&self, l: &Layer) -> bool {
        if !self.active() {
            return true;
        }
        match self.by {
            FilterBy::Kind => self.kinds.iter().any(|k| k == kind(l)),
            FilterBy::Name => l.name.to_lowercase().contains(&self.name.trim().to_lowercase()),
            FilterBy::Mode => l.blend == self.mode,
            FilterBy::Label => self.labels.iter().any(|k| k == l.label.id()),
        }
    }
}

/// The kinds the Kind filter toggles.
pub const KINDS: [&str; 5] = ["pixel", "adjustment", "type", "shape", "smart"];

/// `ui.set {layerFilter: {by?, kinds?, name?, mode?}}`: `current` with the given fields replaced,
/// or an error naming the bad field (nothing changes then). `by` is `kind`, `name` or `mode`;
/// `kinds` a list of [`KINDS`]; `mode` a blend mode name such as `Multiply` or `Pass Through`.
pub fn patched(current: &UiState, v: &serde_json::Value) -> Result<Filter, String> {
    let Some(o) = v.as_object() else { return Err("layerFilter must be an object".into()) };
    if let Some(k) = o.keys().find(|k| !["by", "kinds", "name", "mode", "labels"].contains(&k.as_str())) {
        return Err(format!("unknown layerFilter field `{k}` (fields: by, kinds, name, mode, labels)"));
    }
    let mut f = Filter::of(current);
    if let Some(by) = o.get("by") {
        f.by = serde_json::from_value(by.clone()).map_err(|_| "layerFilter.by must be kind, name, mode or label".to_string())?;
    }
    if let Some(labels) = o.get("labels") {
        let list = labels.as_array().ok_or("layerFilter.labels must be a list")?;
        f.labels = Vec::new();
        for k in list {
            let Some(k) = k.as_str().filter(|k| redrob_photo_doc::LabelColor::ALL.iter().any(|c| c.id() == *k)) else {
                return Err(format!("layerFilter.labels: unknown label {k}"));
            };
            if !f.labels.iter().any(|have| have == k) {
                f.labels.push(k.to_string());
            }
        }
    }
    if let Some(kinds) = o.get("kinds") {
        let list = kinds.as_array().ok_or("layerFilter.kinds must be a list")?;
        f.kinds = Vec::new();
        for k in list {
            let Some(k) = k.as_str().filter(|k| KINDS.contains(k)) else {
                return Err(format!("layerFilter.kinds: unknown kind {k} (kinds: {})", KINDS.join(", ")));
            };
            if !f.kinds.iter().any(|have| have == k) {
                f.kinds.push(k.to_string());
            }
        }
    }
    if let Some(name) = o.get("name") {
        f.name = name.as_str().ok_or("layerFilter.name must be a string")?.to_string();
    }
    if let Some(mode) = o.get("mode") {
        let name = mode.as_str().ok_or("layerFilter.mode must be a blend mode name")?;
        f.mode = redrob_photo_engine::commands::blend_from_str(name)
            .filter(|m| *m == BlendMode::PassThrough || BlendMode::LAYER_MODES.contains(m))
            .ok_or_else(|| format!("layerFilter.mode: unknown blend mode `{name}`"))?;
    }
    Ok(f)
}

/// Store `f` as the Layers panel's filter.
pub fn apply(ui: &mut UiState, f: Filter) {
    ui.layer_filter_by = f.by;
    ui.layer_filter = f.kinds;
    ui.layer_filter_name = f.name;
    ui.layer_filter_mode = f.mode;
    ui.layer_filter_labels = f.labels;
}

/// The kind filter's name for `l`'s content ("" for groups, which no kind matches).
fn kind(l: &Layer) -> &'static str {
    match &l.content {
        LayerContent::Raster(_) => "pixel",
        LayerContent::Adjustment(_) | LayerContent::Fill(_) => "adjustment",
        LayerContent::Text(_) => "type",
        LayerContent::Shape(_) => "shape",
        LayerContent::Smart(_) => "smart",
        LayerContent::Group(_) => "",
    }
}

/// Draw the filter row: the type dropdown, then that type's controls.
pub fn bar(app: &mut PhotoApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let by = [(FilterBy::Kind, tl!("Kind")), (FilterBy::Name, tl!("Name")), (FilterBy::Mode, tl!("Mode")), (FilterBy::Label, tl!("Label"))];
        widgets::dropdown(ui, "layer-filter-by", &mut app.ui.layer_filter_by, &by, 64.0);
        ui.add_space(4.0);
        match app.ui.layer_filter_by {
            FilterBy::Kind => {
                for (kind, icon, tip) in [
                    ("pixel", "image", tl!("Filter for pixel layers")),
                    ("adjustment", "adjustment-layer", tl!("Filter for adjustment layers")),
                    ("type", "type", tl!("Filter for type layers")),
                    ("shape", "square", tl!("Filter for shape layers")),
                    ("smart", "app-window", tl!("Filter for smart objects")),
                ] {
                    let on = app.ui.layer_filter.iter().any(|k| k == kind);
                    if icons::button(ui, icon, 22.0, on, tip).clicked() {
                        if on {
                            app.ui.layer_filter.retain(|k| k != kind);
                        } else {
                            app.ui.layer_filter.push(kind.to_string());
                        }
                    }
                }
            }
            FilterBy::Name => {
                let width = ui.available_width().max(48.0);
                widgets::text_field(ui, ui.id().with("layer-filter-name"), &mut app.ui.layer_filter_name, tl!("Search"), width, false);
            }
            FilterBy::Mode => {
                let modes: Vec<(BlendMode, &str)> = std::iter::once(BlendMode::PassThrough).chain(BlendMode::LAYER_MODES).map(|m| (m, m.label())).collect();
                let width = ui.available_width().max(96.0) - 26.0;
                widgets::dropdown(ui, "layer-filter-mode", &mut app.ui.layer_filter_mode, &modes, width);
            }
            FilterBy::Label => {
                let t = crate::theme::Tokens::get(ui.ctx());
                for label in redrob_photo_doc::LabelColor::ALL.iter().filter(|c| **c != redrob_photo_doc::LabelColor::None) {
                    let id = label.id();
                    let on = app.ui.layer_filter_labels.iter().any(|k| k == id);
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
                    let swatch = t.layer_label_colors(*label).map_or(t.text_dim, |(s, _)| s);
                    let r = rect.shrink(if on { 1.0 } else { 3.0 });
                    ui.painter().rect_filled(r, 3.0, swatch);
                    if on {
                        ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.5, t.text), egui::StrokeKind::Inside);
                    }
                    if resp.on_hover_text(tl!(label.id())).clicked() {
                        if on {
                            app.ui.layer_filter_labels.retain(|k| k != id);
                        } else {
                            app.ui.layer_filter_labels.push(id.to_string());
                        }
                    }
                }
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| display_settings(app, ui));
    });
}

/// The panel's Display Settings popover: thumbnail size and how much blend detail rows show.
fn display_settings(app: &mut PhotoApp, ui: &mut egui::Ui) {
    let resp = icons::button(ui, "settings", 22.0, false, tl!("Display settings"));
    egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(200.0);
        let d = &mut app.ui.layer_display;
        ui.label(tl!("Thumbnail size"));
        let sizes = [
            (ThumbnailSize::None, tl!("None")),
            (ThumbnailSize::Small, tl!("Small")),
            (ThumbnailSize::Medium, tl!("Medium")),
            (ThumbnailSize::Large, tl!("Large")),
        ];
        widgets::dropdown(ui, "layer-display-thumb", &mut d.thumbnail, &sizes, 160.0);
        ui.add_space(4.0);
        ui.label(tl!("Blend details"));
        let details =
            [(BlendDetail::Hidden, tl!("Hidden")), (BlendDetail::Mode, tl!("Blend mode")), (BlendDetail::ModeAndOpacity, tl!("Blend mode and opacity"))];
        widgets::dropdown(ui, "layer-display-detail", &mut d.blend_detail, &details, 160.0);
    });
}

#[cfg(test)]
#[path = "layer_filter_ui_tests.rs"]
mod tests;
