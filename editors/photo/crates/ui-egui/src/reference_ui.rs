//! Reference images (the Reference Images tool): pictures floating over the canvas, not part of
//! the document: add one from a file or paste one from the clipboard, drag it around or resize it
//! from its corners (keeping its aspect when asked), set its opacity and saturation, and save or
//! load the whole set as a `.rref` JSON file that embeds the pixels or links the files.

use egui::{Color32, Pos2, Rect, Stroke, pos2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{PhotoApp, canvas::ToolEvent, canvas::ViewXform, state::Tool, theme::Tokens, widgets};

/// Most reference images in a set.
pub const MAX_REFERENCES: usize = 32;
/// Largest reference image side kept (larger ones are scaled down on load).
pub const MAX_SIDE: u32 = 4096;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RefImage {
    pub name: String,
    /// The encoded file (PNG, JPEG, …), kept when embedded; empty for a linked image.
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
    /// The file it is linked to instead of embedding.
    pub link: Option<String>,
    /// Document-space placement: x, y, width, height.
    pub rect: [f64; 4],
    pub opacity: f32,
    /// 1 = as it is, 0 = greyscale.
    pub saturation: f32,
    pub keep_aspect: bool,
    #[serde(skip)]
    pub pixels: Option<egui::ColorImage>,
    #[serde(skip)]
    pub texture: Option<egui::TextureHandle>,
    /// The saturation the texture was built with.
    #[serde(skip)]
    pub texture_saturation: f32,
}

impl Default for RefImage {
    fn default() -> Self {
        Self {
            name: String::new(),
            bytes: Vec::new(),
            link: None,
            rect: [0.0, 0.0, 100.0, 100.0],
            opacity: 1.0,
            saturation: 1.0,
            keep_aspect: true,
            pixels: None,
            texture: None,
            texture_saturation: 1.0,
        }
    }
}

mod base64_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD.decode(text.as_bytes()).map_err(serde::de::Error::custom)
    }
}

/// What a drag with the tool does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    Move {
        index: usize,
        from: [f64; 2],
    },
    /// Resize from corner `corner` (0 tl, 1 tr, 2 br, 3 bl).
    Resize {
        index: usize,
        corner: usize,
        start: [f64; 4],
    },
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct References {
    pub items: Vec<RefImage>,
    pub selected: Option<usize>,
    /// Draw the references with every tool (off: only with the Reference Images tool).
    pub show: bool,
    #[serde(skip)]
    pub drag: Option<Drag>,
}

#[derive(Serialize, Deserialize)]
struct RefFile {
    format: String,
    version: u32,
    items: Vec<RefImage>,
}

const FORMAT: &str = "redrob-reference-images";

/// Decode an image file into egui pixels, scaled to at most [`MAX_SIDE`] a side.
pub fn decode(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    use redrob_photo_codecs::{ChannelLayout, SampleType};
    let img = redrob_photo_codecs::decode(bytes).map_err(|e| e.to_string())?.convert(ChannelLayout::Rgba, SampleType::U8);
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 || img.data().len() != w * h * 4 {
        return Err("the image is empty".into());
    }
    let full = egui::ColorImage::from_rgba_unmultiplied([w, h], img.data());
    let longest = w.max(h) as u32;
    if longest <= MAX_SIDE {
        return Ok(full);
    }
    // Nearest-neighbour reduction: a reference is looked at, not measured.
    let k = longest.div_ceil(MAX_SIDE) as usize;
    let (nw, nh) = ((w / k).max(1), (h / k).max(1));
    let mut px = Vec::with_capacity(nw * nh);
    for y in 0..nh {
        for x in 0..nw {
            px.push(full.pixels[(y * k).min(h - 1) * w + (x * k).min(w - 1)]);
        }
    }
    Ok(egui::ColorImage { size: [nw, nh], source_size: egui::vec2(nw as f32, nh as f32), pixels: px })
}

/// `img` with its saturation scaled by `s` (0 = greyscale).
pub fn desaturated(img: &egui::ColorImage, s: f32) -> egui::ColorImage {
    let s = s.clamp(0.0, 1.0);
    if s >= 1.0 {
        return img.clone();
    }
    let pixels = img
        .pixels
        .iter()
        .map(|c| {
            let (r, g, b) = (f32::from(c.r()), f32::from(c.g()), f32::from(c.b()));
            let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let mix = |v: f32| (l + (v - l) * s).round().clamp(0.0, 255.0) as u8;
            Color32::from_rgba_premultiplied(mix(r), mix(g), mix(b), c.a())
        })
        .collect();
    egui::ColorImage { size: img.size, pixels, ..img.clone() }
}

impl RefImage {
    /// Build a reference from an encoded file, placed at `at` with its own size (scaled to fit a
    /// `canvas` when larger than it).
    pub fn new(name: &str, bytes: Vec<u8>, link: Option<String>, at: [f64; 2], canvas: [f64; 2]) -> Result<Self, String> {
        let pixels = decode(&bytes)?;
        let (w, h) = (pixels.size[0] as f64, pixels.size[1] as f64);
        let fit = (canvas[0] / w).min(canvas[1] / h).clamp(1e-3, 1.0);
        Ok(Self {
            name: name.into(),
            bytes: if link.is_some() { Vec::new() } else { bytes },
            link,
            rect: [at[0], at[1], w * fit, h * fit],
            pixels: Some(pixels),
            ..Default::default()
        })
    }

    pub fn contains(&self, p: [f64; 2]) -> bool {
        let [x, y, w, h] = self.rect;
        p[0] >= x && p[0] <= x + w && p[1] >= y && p[1] <= y + h
    }

    pub fn corners(&self) -> [[f64; 2]; 4] {
        let [x, y, w, h] = self.rect;
        [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
    }

    /// The aspect of the pixels (width / height), 1 without pixels.
    pub fn aspect(&self) -> f64 {
        self.pixels.as_ref().map_or(1.0, |p| p.size[0] as f64 / p.size[1].max(1) as f64)
    }

    /// The rect after dragging `corner` of `start` to `to`, keeping the aspect when asked; never
    /// smaller than 4 px.
    pub fn resized(start: [f64; 4], corner: usize, to: [f64; 2], keep_aspect: bool, aspect: f64) -> [f64; 4] {
        let [x, y, w, h] = start;
        // The corner opposite the dragged one stays put.
        let anchor = match corner {
            0 => [x + w, y + h],
            1 => [x, y + h],
            2 => [x, y],
            _ => [x + w, y],
        };
        let mut nw = (to[0] - anchor[0]).abs().max(4.0);
        let mut nh = (to[1] - anchor[1]).abs().max(4.0);
        if keep_aspect && aspect > 0.0 {
            // Follow the larger relative change.
            if nw / w.max(1e-9) >= nh / h.max(1e-9) {
                nh = nw / aspect;
            } else {
                nw = nh * aspect;
            }
        }
        let nx = if to[0] < anchor[0] { anchor[0] - nw } else { anchor[0] };
        let ny = if to[1] < anchor[1] { anchor[1] - nh } else { anchor[1] };
        [nx, ny, nw, nh]
    }
}

fn tolerance(app: &PhotoApp) -> f64 {
    8.0 / f64::from(app.point_zoom().max(0.01))
}

fn status(app: &mut PhotoApp, msg: String, error: bool) {
    app.ui.status = msg;
    app.ui.status_error = error;
}

fn canvas_size(app: &PhotoApp) -> [f64; 2] {
    app.session.active().map_or([1000.0, 1000.0], |st| [f64::from(st.doc.size.width), f64::from(st.doc.size.height)])
}

/// Add a reference from file bytes at the canvas centre.
pub fn add(app: &mut PhotoApp, name: &str, bytes: Vec<u8>, link: Option<String>) -> Result<(), String> {
    if app.references.items.len() >= MAX_REFERENCES {
        return Err(format!("at most {MAX_REFERENCES} reference images"));
    }
    let size = canvas_size(app);
    let mut img = RefImage::new(name, bytes, link, [0.0, 0.0], size)?;
    img.rect[0] = (size[0] - img.rect[2]) / 2.0;
    img.rect[1] = (size[1] - img.rect[3]) / 2.0;
    app.references.items.push(img);
    app.references.selected = Some(app.references.items.len() - 1);
    Ok(())
}

/// Paste the OS clipboard's image as a reference.
pub fn paste(app: &mut PhotoApp) -> Result<(), String> {
    let Some(get) = app.services.clipboard_get_image.as_mut() else { return Err(tl!("The clipboard is not available").into()) };
    let Some((w, h, rgba)) = get() else { return Err(tl!("Nothing to paste: the clipboard holds no image").into()) };
    if w == 0 || h == 0 || rgba.len() != (w as usize) * (h as usize) * 4 {
        return Err("the clipboard image is malformed".into());
    }
    let mut img =
        redrob_photo_codecs::Image::new(w, h, redrob_photo_codecs::ChannelLayout::Rgba, redrob_photo_codecs::SampleType::U8).map_err(|e| e.to_string())?;
    img.data_mut().copy_from_slice(&rgba);
    let bytes =
        redrob_photo_codecs::encode(&img, redrob_photo_codecs::Format::Png, &redrob_photo_codecs::EncodeOptions::default()).map_err(|e| e.to_string())?;
    add(app, tl!("Pasted image"), bytes, None)
}

/// Pointer input for the Reference Images tool. Returns true when consumed.
pub fn pointer(app: &mut PhotoApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if app.active_tool() != Tool::ReferenceImages {
        app.references.drag = None;
        return false;
    }
    let tol = tolerance(app);
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let p = [x, y];
            // Corners first (topmost image wins), then bodies.
            let corner = app
                .references
                .items
                .iter()
                .enumerate()
                .rev()
                .find_map(|(i, r)| r.corners().iter().position(|c| (c[0] - p[0]).hypot(c[1] - p[1]) <= tol).map(|k| (i, k)));
            if let Some((index, corner)) = corner {
                app.references.selected = Some(index);
                let start = app.references.items[index].rect;
                app.references.drag = Some(Drag::Resize { index, corner, start });
            } else if let Some(index) = app.references.items.iter().rposition(|r| r.contains(p)) {
                if mods.alt {
                    app.references.items.remove(index);
                    app.references.selected = None;
                } else {
                    app.references.selected = Some(index);
                    app.references.drag = Some(Drag::Move { index, from: p });
                }
            } else {
                app.references.selected = None;
            }
        }
        ToolEvent::Move { x, y, .. } => match app.references.drag {
            Some(Drag::Move { index, from }) => {
                if let Some(r) = app.references.items.get_mut(index) {
                    r.rect[0] += x - from[0];
                    r.rect[1] += y - from[1];
                }
                app.references.drag = Some(Drag::Move { index, from: [x, y] });
            }
            Some(Drag::Resize { index, corner, start }) => {
                if let Some(r) = app.references.items.get_mut(index) {
                    let (keep, aspect) = (r.keep_aspect, r.aspect());
                    r.rect = RefImage::resized(start, corner, [x, y], keep, aspect);
                }
            }
            None => {}
        },
        ToolEvent::Up { .. } => app.references.drag = None,
    }
    true
}

/// The tool's options bar: Add, Paste, Load and Save the set, the selected image's opacity,
/// saturation, keep-aspect and embed/link, Delete, Show with all tools.
pub fn options_bar(app: &mut PhotoApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::ReferenceImages {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Reference Images")).color(t.text_dim));
    if widgets::secondary_button(ui, tl!("Add…"), 0.0).clicked() {
        let r = app.pick_file_bytes_filtered(&["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff"], |app, name, bytes| {
            add(app, &name, bytes, None).map(|()| Value::Null)
        });
        if let Err(e) = r {
            status(app, e, true);
        }
    }
    if widgets::secondary_button(ui, tl!("Paste"), 0.0).clicked()
        && let Err(e) = paste(app)
    {
        status(app, e, true);
    }
    if widgets::secondary_button(ui, tl!("Load…"), 0.0).clicked() {
        let r = app.pick_file_bytes_filtered(&["rref"], |app, name, bytes| load(app, &name, &bytes).map(|n| json!({"loaded": n})));
        if let Err(e) = r {
            status(app, e, true);
        }
    }
    let has = !app.references.items.is_empty();
    if ui.add_enabled_ui(has, |ui| widgets::secondary_button(ui, tl!("Save…"), 0.0)).inner.clicked() {
        let text = match save_text(app) {
            Ok(t) => t,
            Err(e) => {
                status(app, e, true);
                return true;
            }
        };
        let r = app.pick_save("references.rref", move |_app, path| {
            std::fs::write(&path, text.as_bytes()).map(|_| json!({"path": path})).map_err(|e| format!("{path}: {e}"))
        });
        if let Err(e) = r {
            status(app, e, true);
        }
    }
    widgets::vline(ui, 22.0);
    widgets::checkbox(ui, &mut app.references.show, tl!("Show with all tools"));
    if let Some(i) = app.references.selected.filter(|i| *i < app.references.items.len()) {
        widgets::vline(ui, 22.0);
        let r = &mut app.references.items[i];
        ui.label(egui::RichText::new(r.name.clone()).color(t.text_dim));
        ui.label(egui::RichText::new(tl!("Opacity")).color(t.text_dim));
        let mut op = r.opacity * 100.0;
        if widgets::value_field(ui, &mut op, 0.0..=100.0, "%", 58.0).changed() {
            r.opacity = op / 100.0;
        }
        ui.label(egui::RichText::new(tl!("Saturation")).color(t.text_dim));
        let mut sat = r.saturation * 100.0;
        if widgets::value_field(ui, &mut sat, 0.0..=100.0, "%", 58.0).changed() {
            r.saturation = sat / 100.0;
        }
        widgets::checkbox(ui, &mut r.keep_aspect, tl!("Keep aspect"));
        let mut embed = r.link.is_none();
        if r.link.is_some() || !r.bytes.is_empty() {
            let linkable = r.link.is_some() || std::path::Path::new(&r.name).is_absolute();
            ui.add_enabled_ui(linkable || embed, |ui| {
                if widgets::checkbox(ui, &mut embed, tl!("Embed")).on_hover_text(tl!("Store the pixels in the set; off, link to the file")).changed() {
                    if embed {
                        if let Some(path) = r.link.take()
                            && let Ok(b) = std::fs::read(&path)
                        {
                            r.bytes = b;
                        }
                    } else if r.link.is_none() && std::path::Path::new(&r.name).is_absolute() {
                        r.link = Some(r.name.clone());
                        r.bytes.clear();
                    }
                }
            });
        }
        if widgets::secondary_button(ui, tl!("Delete"), 0.0).clicked() {
            app.references.items.remove(i);
            app.references.selected = None;
        }
    }
    true
}

/// The set as `.rref` JSON text.
pub fn save_text(app: &PhotoApp) -> Result<String, String> {
    let file = RefFile { format: FORMAT.into(), version: 1, items: app.references.items.clone() };
    serde_json::to_string_pretty(&file).map_err(|e| e.to_string())
}

/// Load a `.rref` set (replacing the current one); linked images are read from their files.
pub fn load(app: &mut PhotoApp, name: &str, bytes: &[u8]) -> Result<usize, String> {
    let file: RefFile = serde_json::from_slice(bytes).map_err(|e| format!("{name}: not a reference set: {e}"))?;
    if file.format != FORMAT {
        return Err(format!("{name}: not a reference set (format `{}`)", file.format));
    }
    let mut items = Vec::new();
    for mut r in file.items.into_iter().take(MAX_REFERENCES) {
        let data = match (&r.link, r.bytes.is_empty()) {
            (Some(path), _) => std::fs::read(path).map_err(|e| format!("{path}: {e}"))?,
            (None, false) => r.bytes.clone(),
            (None, true) => continue,
        };
        r.pixels = Some(decode(&data)?);
        r.texture = None;
        items.push(r);
    }
    let n = items.len();
    app.references.items = items;
    app.references.selected = None;
    Ok(n)
}

/// Draw the references over the canvas (with the tool, or everywhere when Show is on).
pub fn draw(app: &mut PhotoApp, painter: &egui::Painter, xf: &ViewXform) {
    let editing = app.ui.tool == Tool::ReferenceImages;
    if !editing && !app.references.show {
        return;
    }
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
    let t = Tokens::get(painter.ctx());
    let selected = app.references.selected;
    for (i, r) in app.references.items.iter_mut().enumerate() {
        let Some(px) = r.pixels.as_ref() else { continue };
        if r.texture.is_none() || (r.texture_saturation - r.saturation).abs() > 1e-3 {
            let img = desaturated(px, r.saturation);
            r.texture = Some(painter.ctx().load_texture(format!("reference-{i}"), img, egui::TextureOptions::LINEAR));
            r.texture_saturation = r.saturation;
        }
        let Some(tex) = &r.texture else { continue };
        let [x, y, w, h] = r.rect;
        let a: Pos2 = xf.to_screen(x as f32, y as f32);
        let b: Pos2 = xf.to_screen((x + w) as f32, (y + h) as f32);
        let rect = Rect::from_two_pos(a, b);
        let tint = Color32::WHITE.gamma_multiply(r.opacity.clamp(0.0, 1.0));
        painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        if editing {
            let color = if selected == Some(i) { t.accent } else { t.accent.gamma_multiply(0.5) };
            painter.rect_stroke(rect, 0.0, Stroke::new(1.0, color), egui::StrokeKind::Outside);
            if selected == Some(i) {
                for c in r.corners() {
                    crate::vector_ui::draw_anchor(&painter, xf.to_screen(c[0] as f32, c[1] as f32), true, t.accent);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resizing_keeps_the_opposite_corner_and_the_aspect() {
        let start = [10.0, 10.0, 100.0, 50.0];
        let r = RefImage::resized(start, 2, [210.0, 110.0], false, 2.0);
        assert_eq!(r, [10.0, 10.0, 200.0, 100.0]);
        let k = RefImage::resized(start, 2, [210.0, 20.0], true, 2.0);
        assert_eq!(k, [10.0, 10.0, 200.0, 100.0], "the width change wins and the height follows");
        let tl = RefImage::resized(start, 0, [60.0, 35.0], false, 2.0);
        assert_eq!(tl, [60.0, 35.0, 50.0, 25.0]);
        let tiny = RefImage::resized(start, 2, [10.0, 10.0], false, 2.0);
        assert_eq!(tiny[2], 4.0);
        let img = RefImage { rect: [0.0, 0.0, 10.0, 10.0], ..Default::default() };
        assert!(img.contains([5.0, 5.0]) && !img.contains([15.0, 5.0]));
        assert_eq!(img.corners()[2], [10.0, 10.0]);
    }

    #[test]
    fn desaturation_greys_pixels_and_sets_round_trip() {
        let img = egui::ColorImage { size: [1, 1], pixels: vec![Color32::from_rgb(200, 100, 0)], ..Default::default() };
        let g = desaturated(&img, 0.0);
        let c = g.pixels[0];
        assert_eq!(c.r(), c.g());
        assert_eq!(c.g(), c.b());
        assert_eq!(desaturated(&img, 1.0).pixels[0], img.pixels[0]);
        let file = RefFile { format: FORMAT.into(), version: 1, items: vec![RefImage { name: "a".into(), bytes: vec![1, 2, 3], ..Default::default() }] };
        let text = serde_json::to_string(&file).unwrap();
        assert!(text.contains("\"bytes\":\"AQID\""));
        let back: RefFile = serde_json::from_str(&text).unwrap();
        assert_eq!(back.items[0].bytes, vec![1, 2, 3]);
        assert!(decode(&[0, 1, 2]).is_err());
    }
}
