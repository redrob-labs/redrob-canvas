// SPDX-License-Identifier: MIT OR Apache-2.0

//! How the AI layer reads and writes an editor's document without the AI crate depending on any
//! editor: [`ModeBridge`] is implemented once per mode over the upstream document APIs. Pixels
//! cross as [`redrob_ai::Rgba8Image`] / [`redrob_ai::Mask`].
//!
//! A bridge runs on the UI thread only; the jobs (`super::jobs`) never see one. Where an editor
//! has no concept for a method (a layout has no selection mask, a vector artboard is not a pixel
//! canvas to pad), the bridge answers [`BridgeError::Unsupported`] and the panel shows "not
//! available in this mode"; nothing is faked.

use redrob_ai::{Expand, Mask, Rgba8Image};
use serde_json::json;

use crate::shell::Mode;

/// Why a bridge method could not do its work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BridgeError {
    /// The mode has no way to do this at all.
    Unsupported(&'static str),
    /// No document is open in the mode.
    NoDocument,
    /// The editor refused or failed (its own message).
    Failed(String),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Unsupported(what) => write!(f, "{what} is not available in this mode"),
            BridgeError::NoDocument => write!(f, "open a document first"),
            BridgeError::Failed(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for BridgeError {}

pub type BridgeResult<T> = Result<T, BridgeError>;

/// What one mode lets the AI layer do with its document. The `supports_*` methods are the static
/// side (what the mode can do at all), used to enable menu items and cards; the data methods
/// answer for the document that is open right now.
pub trait ModeBridge {
    fn mode(&self) -> Mode;

    /// The flattened document pixels (the current page / artboard for Layout and Vector).
    fn snapshot(&mut self) -> Option<Rgba8Image>;

    /// The active selection as a coverage mask of the document's size.
    fn selection_mask(&self) -> Option<Mask>;

    /// Add `img` as a new layer (or image object) named `name`, its top-left at `offset` pixels.
    fn place_layer(&mut self, name: &str, img: Rgba8Image, offset: (i32, i32)) -> BridgeResult<()>;

    /// Replace the selection with `mask` (document-sized).
    fn set_selection(&mut self, mask: Mask) -> BridgeResult<()>;

    /// Grow the canvas by `expand` pixels on each side, keeping the picture in place.
    fn expand_canvas(&mut self, expand: Expand) -> BridgeResult<()>;

    /// The document (page / artboard) size in pixels, when one is open.
    fn document_size(&self) -> Option<(u32, u32)>;

    /// Whether a selection exists right now. Cheap (polled every frame); the default reads the
    /// mask, modes override it.
    fn has_selection(&self) -> bool {
        self.supports_selection() && self.selection_mask().is_some()
    }

    /// Whether [`Self::selection_mask`] / [`Self::set_selection`] can ever answer in this mode.
    fn supports_selection(&self) -> bool;

    /// Whether [`Self::place_layer`] can ever succeed in this mode.
    fn supports_place_layer(&self) -> bool;

    /// Whether [`Self::expand_canvas`] can ever succeed in this mode.
    fn supports_expand(&self) -> bool;

    /// The static capabilities, for the menu and the cards.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            mode: self.mode(),
            has_document: self.document_size().is_some(),
            has_selection: self.has_selection(),
            selection: self.supports_selection(),
            place_layer: self.supports_place_layer(),
            expand: self.supports_expand(),
        }
    }
}

/// A snapshot of what a bridge can do, cheap to copy into the panel each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub mode: Mode,
    pub has_document: bool,
    /// A selection exists right now.
    pub has_selection: bool,
    /// The mode has selection masks at all.
    pub selection: bool,
    pub place_layer: bool,
    pub expand: bool,
}

impl Capabilities {
    /// A mode with nothing open: every static capability as the mode has it, no document.
    pub fn none(mode: Mode) -> Self {
        Self {
            mode,
            has_document: false,
            has_selection: false,
            selection: false,
            place_layer: false,
            expand: false,
        }
    }

    /// What the Photo mode offers.
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the unit tests
    pub fn photo(has_document: bool, has_selection: bool) -> Self {
        Self {
            mode: Mode::Photo,
            has_document,
            has_selection,
            selection: true,
            place_layer: true,
            expand: true,
        }
    }

    /// What the Vector mode offers: images can be placed, there is no pixel selection and the
    /// artboard is not padded.
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the unit tests
    pub fn vector(has_document: bool) -> Self {
        Self {
            mode: Mode::Vector,
            has_document,
            has_selection: false,
            selection: false,
            place_layer: true,
            expand: false,
        }
    }

    /// What the Layout mode offers: graphics can be placed on the page, nothing else.
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the unit tests
    pub fn layout(has_document: bool) -> Self {
        Self {
            mode: Mode::Layout,
            has_document,
            has_selection: false,
            selection: false,
            place_layer: true,
            expand: false,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Photo (photocraft)
// ---------------------------------------------------------------------------------------------

/// The Photo editor's document through `redrob_photo_engine::Session`.
pub struct PhotoBridge<'a> {
    pub session: &'a mut redrob_photo_engine::Session,
}

impl<'a> PhotoBridge<'a> {
    pub fn new(session: &'a mut redrob_photo_engine::Session) -> Self {
        Self { session }
    }

    fn failed(e: impl std::fmt::Display) -> BridgeError {
        BridgeError::Failed(e.to_string())
    }
}

/// `redrob_ai::Mask` from photocraft's selection surface (a grayscale coverage surface, 255 =
/// selected) read over the document bounds. `Surface::convert` is
/// crafts/photocraft/crates/raster/src/lib.rs L930, `to_interleaved` L728.
pub fn mask_from_selection(
    selection: &redrob_photo_raster::Surface,
    bounds: redrob_photo_geom::Rect,
) -> Option<Mask> {
    let gray = redrob_photo_color::PixelFormat::GRAY8;
    let bytes = if selection.format() == gray {
        selection.to_interleaved(bounds)
    } else {
        selection.convert(gray).to_interleaved(bounds)
    };
    Mask::new(bounds.width(), bounds.height(), bytes).ok()
}

/// photocraft's selection surface from a document-sized `redrob_ai::Mask`
/// (`Surface::from_interleaved`, crafts/photocraft/crates/raster/src/lib.rs L704).
pub fn selection_from_mask(mask: &Mask) -> redrob_photo_raster::Surface {
    let rect = redrob_photo_geom::Rect::new(0, 0, mask.width as i32, mask.height as i32);
    redrob_photo_raster::Surface::from_interleaved(
        redrob_photo_color::PixelFormat::GRAY8,
        rect,
        &mask.data,
    )
}

impl ModeBridge for PhotoBridge<'_> {
    fn mode(&self) -> Mode {
        Mode::Photo
    }

    fn snapshot(&mut self) -> Option<Rgba8Image> {
        let st = self.session.active()?;
        // `flatten(&Document) -> Buffer` (crafts/photocraft/crates/compose/src/lib.rs L323),
        // `Buffer::to_rgba8()` (L61) gives redrob_photo_raster::Rgba8Image { width, height, pixels }.
        let img = redrob_photo_compose::flatten(&st.doc).to_rgba8();
        Rgba8Image::new(img.width, img.height, img.pixels).ok()
    }

    fn selection_mask(&self) -> Option<Mask> {
        let st = self.session.active()?;
        // `Document::selection: Option<Surface>` (crafts/photocraft/crates/doc/src/lib.rs L873).
        let selection = st.doc.selection.as_ref()?;
        mask_from_selection(selection, st.doc.bounds())
    }

    fn place_layer(&mut self, name: &str, img: Rgba8Image, offset: (i32, i32)) -> BridgeResult<()> {
        if self.session.active().is_none() {
            return Err(BridgeError::NoDocument);
        }
        let rect = redrob_photo_geom::Rect::new(
            offset.0,
            offset.1,
            offset.0.saturating_add(img.width as i32),
            offset.1.saturating_add(img.height as i32),
        );
        let name = name.to_owned();
        self.session
            .edit("AI layer", move |doc, active| {
                let rgba8 = redrob_photo_color::PixelFormat::RGBA8;
                let mut surface =
                    redrob_photo_raster::Surface::from_interleaved(rgba8, rect, &img.data);
                // New raster layers take the document's format
                // (`Document::pixel_format`, crafts/photocraft/crates/doc/src/lib.rs ~L1000).
                let want = doc.pixel_format();
                if want != rgba8 {
                    surface = surface.convert(want);
                }
                // `Layer::new(name, LayerContent::Raster(surface))` (doc/src/lib.rs L672),
                // `Document::insert_above(above, layer) -> LayerId` (L1168).
                let layer = redrob_photo_doc::Layer::new(
                    name,
                    redrob_photo_doc::LayerContent::Raster(surface),
                );
                let id = doc.insert_above(*active, layer);
                *active = Some(id);
                Ok(())
            })
            .map_err(Self::failed)
    }

    fn set_selection(&mut self, mask: Mask) -> BridgeResult<()> {
        let Some(st) = self.session.active() else {
            return Err(BridgeError::NoDocument);
        };
        let size = st.doc.size;
        if (mask.width, mask.height) != (size.width, size.height) {
            return Err(BridgeError::Failed(format!(
                "the mask is {} × {}, the document {} × {}",
                mask.width, mask.height, size.width, size.height
            )));
        }
        let surface = selection_from_mask(&mask);
        self.session
            .edit("AI selection", move |doc, _| {
                doc.selection = if mask.coverage() == 0 {
                    None
                } else {
                    Some(surface)
                };
                Ok(())
            })
            .map_err(Self::failed)
    }

    fn expand_canvas(&mut self, expand: Expand) -> BridgeResult<()> {
        let Some(st) = self.session.active() else {
            return Err(BridgeError::NoDocument);
        };
        if !expand.is_valid() {
            return Err(BridgeError::Failed("nothing to expand".into()));
        }
        let (w, h) = (st.doc.size.width, st.doc.size.height);
        // `image.canvasSize` takes one of nine anchors (crafts/photocraft/crates/engine/src/
        // image_cmds.rs L346): grow right/bottom anchored top-left, then left/top anchored
        // bottom-right. Transparent extension, so the AI result layer covers the new border.
        if expand.right + expand.bottom > 0 {
            self.session
                .execute(
                    "image.canvasSize",
                    json!({
                        "width": w + expand.right,
                        "height": h + expand.bottom,
                        "anchor": "topLeft",
                        "extensionColor": "transparent",
                    }),
                )
                .map_err(Self::failed)?;
        }
        if expand.left + expand.top > 0 {
            self.session
                .execute(
                    "image.canvasSize",
                    json!({
                        "width": w + expand.right + expand.left,
                        "height": h + expand.bottom + expand.top,
                        "anchor": "bottomRight",
                        "extensionColor": "transparent",
                    }),
                )
                .map_err(Self::failed)?;
        }
        Ok(())
    }

    fn document_size(&self) -> Option<(u32, u32)> {
        let st = self.session.active()?;
        Some((st.doc.size.width, st.doc.size.height))
    }

    fn has_selection(&self) -> bool {
        self.session
            .active()
            .is_some_and(|st| st.doc.selection.is_some())
    }

    fn supports_selection(&self) -> bool {
        true
    }

    fn supports_place_layer(&self) -> bool {
        true
    }

    fn supports_expand(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------------------------
// Vector (vectorcraft)
// ---------------------------------------------------------------------------------------------

/// The Vector editor's document through `VectorcraftApp` (its session and the active view's
/// artboard).
pub struct VectorBridge<'a> {
    pub app: &'a mut redrob_vector_ui_egui::VectorcraftApp,
}

impl<'a> VectorBridge<'a> {
    pub fn new(app: &'a mut redrob_vector_ui_egui::VectorcraftApp) -> Self {
        Self { app }
    }

    /// The active view's artboard rectangle in document points.
    fn artboard_rect(&self) -> Option<redrob_vector_geom::Rect> {
        let st = self.app.session.active()?;
        let index = self.app.view().map(|v| v.artboard).unwrap_or(0);
        let board = st
            .doc
            .artboards
            .get(index)
            .or_else(|| st.doc.artboards.first())?;
        Some(board.rect)
    }
}

impl ModeBridge for VectorBridge<'_> {
    fn mode(&self) -> Mode {
        Mode::Vector
    }

    fn snapshot(&mut self) -> Option<Rgba8Image> {
        let rect = self.artboard_rect()?;
        let st = self.app.session.active()?;
        // `Renderer::render_region(doc, region, scale, white) -> Rendered` (premultiplied), then
        // `Rendered::to_straight()` (crafts/vectorcraft/crates/render/src/lib.rs L782, L272).
        let mut renderer = redrob_vector_render::Renderer::new();
        let rendered = renderer.render_region(&st.doc, rect, 1.0, false);
        Rgba8Image::new(rendered.width, rendered.height, rendered.to_straight()).ok()
    }

    fn selection_mask(&self) -> Option<Mask> {
        None
    }

    fn place_layer(&mut self, name: &str, img: Rgba8Image, offset: (i32, i32)) -> BridgeResult<()> {
        let Some(rect) = self.artboard_rect() else {
            return Err(BridgeError::NoDocument);
        };
        let png = img
            .to_png()
            .map_err(|e| BridgeError::Failed(e.to_string()))?;
        let (w, h) = (img.width, img.height);
        let key = format!("ai-{}-{}", name.to_lowercase().replace(' ', "-"), png.len());
        let x = rect.x0 + f64::from(offset.0);
        let y = rect.y0 + f64::from(offset.1);
        let name = name.to_owned();
        self.app
            .session
            .edit("AI image", move |doc, sel| {
                // `Document::images: BTreeMap<String, ImageBlob>` + `ImageBlob::new(mime, bytes)`
                // (crafts/vectorcraft/crates/doc/src/{lib.rs L607, links.rs L104}); an image node
                // is `NodeKind::Image(Box<ImageObject>)` (node.rs L382), as the engine's own
                // tests build it (crafts/vectorcraft/crates/engine/src/tests_build.rs L660).
                doc.images.insert(
                    key.clone(),
                    redrob_vector_doc::ImageBlob::new("image/png", png),
                );
                let id = doc.alloc_id();
                let image = redrob_vector_doc::ImageObject {
                    key,
                    width: w,
                    height: h,
                    xf: redrob_vector_geom::Affine::translate((x, y)),
                    link: None,
                    placement: Default::default(),
                };
                let kind = redrob_vector_doc::NodeKind::Image(Box::new(image));
                let mut node = redrob_vector_doc::Node::new(id, kind);
                node.name = Some(name);
                let layer = doc.default_layer();
                doc.insert(layer, usize::MAX, node)?;
                sel.set([id]);
                Ok(())
            })
            .map_err(|e| BridgeError::Failed(e.to_string()))
    }

    fn set_selection(&mut self, _mask: Mask) -> BridgeResult<()> {
        Err(BridgeError::Unsupported("A pixel selection"))
    }

    fn expand_canvas(&mut self, _expand: Expand) -> BridgeResult<()> {
        Err(BridgeError::Unsupported("Expanding the canvas"))
    }

    fn document_size(&self) -> Option<(u32, u32)> {
        let rect = self.artboard_rect()?;
        let (w, h) = redrob_vector_render::region_pixels(rect, 1.0);
        Some((w, h))
    }

    fn supports_selection(&self) -> bool {
        false
    }

    fn supports_place_layer(&self) -> bool {
        true
    }

    fn supports_expand(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------------------------
// Layout (designcraft)
// ---------------------------------------------------------------------------------------------

/// The Layout editor's document through `DesignApp`: the page under the view centre.
pub struct LayoutBridge<'a> {
    pub app: &'a mut redrob_layout_ui_egui::DesignApp,
}

impl<'a> LayoutBridge<'a> {
    pub fn new(app: &'a mut redrob_layout_ui_egui::DesignApp) -> Self {
        Self { app }
    }

    /// The absolute index of the page under the view centre (`canvas::current_page`,
    /// crafts/designcraft/crates/ui-egui/src/canvas.rs L168), else the first page.
    fn current_page(&self) -> Option<usize> {
        let st = self.app.session.active()?;
        if st.doc.page_count() == 0 {
            return None;
        }
        Some(redrob_layout_ui_egui::canvas::current_page(self.app).unwrap_or(0))
    }

    /// The page's bounds in points (`Page::bounds`, crafts/designcraft/crates/doc/src/page.rs
    /// L229) and the index of its spread.
    fn page_rect(&self) -> Option<(redrob_layout_geom::Rect, usize)> {
        let abs = self.current_page()?;
        let st = self.app.session.active()?;
        let (si, _) = st.doc.page_loc(abs)?;
        let page = st.doc.page(abs)?;
        Some((page.bounds(), si))
    }

    /// The current page's content box (inside its margins; `Page::margin_rect`,
    /// crafts/designcraft/crates/doc/src/page.rs L240) in points and the index of its spread:
    /// where a hand-off places its graphic.
    pub fn content_box(&self) -> Option<(redrob_layout_geom::Rect, usize)> {
        let abs = self.current_page()?;
        let st = self.app.session.active()?;
        let (si, _) = st.doc.page_loc(abs)?;
        let page = st.doc.page(abs)?;
        Some((page.margin_rect(), si))
    }
}

impl ModeBridge for LayoutBridge<'_> {
    fn mode(&self) -> Mode {
        Mode::Layout
    }

    fn snapshot(&mut self) -> Option<Rgba8Image> {
        let abs = self.current_page()?;
        let st = self.app.session.active()?;
        // `Renderer::render_page(doc, cache, abs, scale, bleed, opts) -> Option<Rendered>`
        // (crafts/designcraft/crates/render/src/lib.rs L404); `session.cache: Arc<Cache>`
        // (crafts/designcraft/crates/engine/src/lib.rs L276).
        let opts = redrob_layout_render::RenderOptions {
            paper: true,
            printing_only: true,
            ..redrob_layout_render::RenderOptions::default()
        };
        let mut renderer = redrob_layout_render::Renderer::new();
        let cache = &self.app.session.cache;
        let rendered = renderer.render_page(&st.doc, cache, abs, 1.0, false, &opts)?;
        Rgba8Image::new(rendered.width, rendered.height, rendered.to_straight()).ok()
    }

    fn selection_mask(&self) -> Option<Mask> {
        None
    }

    fn place_layer(&mut self, name: &str, img: Rgba8Image, offset: (i32, i32)) -> BridgeResult<()> {
        let Some((rect, spread)) = self.page_rect() else {
            return Err(BridgeError::NoDocument);
        };
        let png = img
            .to_png()
            .map_err(|e| BridgeError::Failed(e.to_string()))?;
        // `file.place` takes `base64` + `name` + `spread` + `x`/`y`/`width` in points
        // (crafts/designcraft/crates/engine/src/cmd/file.rs L296, L366-L420): the asset is added
        // with `Document::add_asset`, the frame holds `Content::Graphic` and goes in with
        // `insert_item`. At 1 px = 1 pt the snapshot and the page line up.
        let base64 = redrob_ai::pixels::data_url(&png)
            .trim_start_matches("data:image/png;base64,")
            .to_owned();
        let params = json!({
            "base64": base64,
            "name": format!("{name}.png"),
            "spread": spread,
            "x": rect.x0 + f64::from(offset.0),
            "y": rect.y0 + f64::from(offset.1),
            "width": f64::from(img.width),
        });
        self.app
            .run("file.place", params)
            .map(|_| ())
            .map_err(BridgeError::Failed)
    }

    fn set_selection(&mut self, _mask: Mask) -> BridgeResult<()> {
        Err(BridgeError::Unsupported("A pixel selection"))
    }

    fn expand_canvas(&mut self, _expand: Expand) -> BridgeResult<()> {
        Err(BridgeError::Unsupported("Expanding the page"))
    }

    fn document_size(&self) -> Option<(u32, u32)> {
        let (rect, _) = self.page_rect()?;
        let w = rect.width().round().max(1.0) as u32;
        let h = rect.height().round().max(1.0) as u32;
        Some((w, h))
    }

    fn supports_selection(&self) -> bool {
        false
    }

    fn supports_place_layer(&self) -> bool {
        true
    }

    fn supports_expand(&self) -> bool {
        false
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn session_with_4x4() -> redrob_photo_engine::Session {
        let mut session = redrob_photo_engine::Session::new();
        let doc = redrob_photo_doc::Document::new(
            "t",
            redrob_photo_geom::Size::new(4, 4),
            redrob_photo_color::ColorMode::Rgb,
            redrob_photo_color::SampleType::U8,
        );
        session.add_document(doc, None);
        session
    }

    #[test]
    fn photo_selection_round_trips_through_mask_on_a_4x4() {
        let mut session = session_with_4x4();
        let mut bridge = PhotoBridge::new(&mut session);
        assert_eq!(bridge.document_size(), Some((4, 4)));
        assert!(bridge.selection_mask().is_none());
        // A 2 × 2 block in the lower right, one half-selected pixel.
        let mut data = vec![0u8; 16];
        for y in 2..4 {
            for x in 2..4 {
                data[y * 4 + x] = 255;
            }
        }
        data[0] = 128;
        let mask = Mask::new(4, 4, data).unwrap();
        bridge.set_selection(mask.clone()).unwrap();
        let back = bridge.selection_mask().unwrap();
        assert_eq!(back, mask);
        let caps = bridge.capabilities();
        assert!(caps.has_document && caps.has_selection && caps.selection && caps.expand);
        // An empty mask clears the selection.
        bridge
            .set_selection(Mask::filled(4, 4, 0).unwrap())
            .unwrap();
        assert!(bridge.selection_mask().is_none());
        // A mask of the wrong size is refused, not silently resized.
        assert!(
            bridge
                .set_selection(Mask::filled(3, 4, 255).unwrap())
                .is_err()
        );
    }

    #[test]
    fn photo_snapshot_and_layer_placement() {
        let mut session = session_with_4x4();
        let mut bridge = PhotoBridge::new(&mut session);
        let snap = bridge.snapshot().unwrap();
        assert_eq!((snap.width, snap.height), (4, 4));
        let red = Rgba8Image::filled(2, 2, [255, 0, 0, 255]).unwrap();
        let layers_before = session.active().unwrap().doc.layers.len();
        let mut bridge = PhotoBridge::new(&mut session);
        bridge.place_layer("AI test", red, (1, 1)).unwrap();
        let st = session.active().unwrap();
        assert_eq!(st.doc.layers.len(), layers_before + 1);
        assert_eq!(st.doc.layers.last().unwrap().name, "AI test");
        let mut bridge = PhotoBridge::new(&mut session);
        let snap = bridge.snapshot().unwrap();
        assert_eq!(snap.pixel(1, 1)[0], 255);
        assert_eq!(snap.pixel(1, 1)[2], 0);
    }

    #[test]
    fn photo_canvas_expands_asymmetrically() {
        let mut session = session_with_4x4();
        let mut bridge = PhotoBridge::new(&mut session);
        bridge
            .expand_canvas(Expand {
                left: 1,
                top: 2,
                right: 3,
                bottom: 4,
            })
            .unwrap();
        assert_eq!(bridge.document_size(), Some((8, 10)));
        assert!(bridge.expand_canvas(Expand::default()).is_err());
    }

    #[test]
    fn mask_surface_conversion_is_lossless() {
        let mask = Mask::new(3, 2, vec![0, 255, 7, 9, 0, 255]).unwrap();
        let surface = selection_from_mask(&mask);
        let back = mask_from_selection(&surface, redrob_photo_geom::Rect::new(0, 0, 3, 2)).unwrap();
        assert_eq!(back, mask);
    }

    #[test]
    fn capabilities_per_mode() {
        let photo = Capabilities::photo(true, true);
        let vector = Capabilities::vector(true);
        let layout = Capabilities::layout(true);
        assert!(photo.selection && photo.expand && photo.place_layer);
        assert!(!vector.selection && !vector.expand && vector.place_layer);
        assert!(!layout.selection && !layout.expand && layout.place_layer);
        assert!(!Capabilities::none(Mode::Photo).has_document);
        assert_eq!(
            BridgeError::Unsupported("A pixel selection").to_string(),
            "A pixel selection is not available in this mode"
        );
    }
}
