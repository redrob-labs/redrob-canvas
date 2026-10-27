// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cross-mode hand-offs: documents flow between the three editors. [`vectorize_to_vector`] traces
//! pixels from any mode into paths in the Vector document; [`raster_to_layout`] and
//! [`vector_to_layout`] place the Photo pixels (as PNG) or the Vector artboard (as SVG) on the
//! Layout page; [`vector_to_photo`] and [`layout_page_to_photo`] rasterise the artboard or the page
//! into a Photo document (a new one, or a layer in the open one). [`Handoff`] is the matrix the
//! Mode menu is enabled from; [`send_to`] dispatches.

use std::sync::Arc;

use redrob_ai::Rgba8Image;
use redrob_ai::vectorize::{TraceResult, TracedPath};
use redrob_vector_doc::{Appearance, Document, Node, NodeId, color::Color, color::Paint};
use redrob_vector_geom::Affine;
use serde_json::json;

use crate::ai::bridge::{LayoutBridge, ModeBridge, PhotoBridge, VectorBridge};
use crate::shell::{Mode, ShellApp};

/// The name of the group the traced paths go into.
pub const GROUP_NAME: &str = "Vectorized";

/// The trace preset [`send_to`] uses for Photo / Layout -> Vector.
pub const DEFAULT_TRACE_PRESET: &str = "default";

/// One way a document moves from one mode to another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Handoff {
    /// Photo pixels -> PNG asset placed on the Layout page.
    RasterToLayout,
    /// Vector artboard -> SVG asset placed on the Layout page.
    VectorToLayout,
    /// Vector artboard rendered -> Photo document or layer.
    VectorToPhoto,
    /// Layout page rendered -> Photo document or layer.
    LayoutPageToPhoto,
    /// Any mode's pixels traced into paths in the Vector document.
    TraceToVector,
}

impl Handoff {
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the unit tests
    pub const ALL: [Handoff; 5] = [
        Handoff::RasterToLayout,
        Handoff::VectorToLayout,
        Handoff::VectorToPhoto,
        Handoff::LayoutPageToPhoto,
        Handoff::TraceToVector,
    ];

    /// The hand-off that takes the active `source` mode's document to `target`, if there is one.
    /// A mode never hands off to itself.
    pub fn between(source: Mode, target: Mode) -> Option<Handoff> {
        match (source, target) {
            (Mode::Photo, Mode::Layout) => Some(Handoff::RasterToLayout),
            (Mode::Vector, Mode::Layout) => Some(Handoff::VectorToLayout),
            (Mode::Vector, Mode::Photo) => Some(Handoff::VectorToPhoto),
            (Mode::Layout, Mode::Photo) => Some(Handoff::LayoutPageToPhoto),
            (Mode::Photo | Mode::Layout, Mode::Vector) => Some(Handoff::TraceToVector),
            (Mode::Photo, Mode::Photo)
            | (Mode::Vector, Mode::Vector)
            | (Mode::Layout, Mode::Layout) => None,
        }
    }

    /// The mode the hand-off ends in.
    #[cfg_attr(not(test), allow(dead_code))] // exercised by the unit tests
    pub fn target(self) -> Mode {
        match self {
            Handoff::RasterToLayout | Handoff::VectorToLayout => Mode::Layout,
            Handoff::VectorToPhoto | Handoff::LayoutPageToPhoto => Mode::Photo,
            Handoff::TraceToVector => Mode::Vector,
        }
    }

    /// What the menu item says about how the document crosses.
    pub fn description(self) -> &'static str {
        match self {
            Handoff::RasterToLayout => "the flattened picture, placed on the page as PNG",
            Handoff::VectorToLayout => "the artboard, placed on the page as SVG",
            Handoff::VectorToPhoto => "the artboard rasterised at document size",
            Handoff::LayoutPageToPhoto => "the current page rendered at 1 pt = 1 px",
            Handoff::TraceToVector => "the picture traced into paths (Vectorize)",
        }
    }
}

/// The appearance a traced path paints with: its colour as a fill, or as a stroke of the traced
/// width when the preset made strokes (`TracedPath::stroke`, editors/vector/crates/trace/
/// src/lib.rs L452). `Appearance::basic(fill, stroke, width)` is
/// editors/vector/crates/doc/src/appearance.rs L751.
pub fn appearance_for(path: &TracedPath) -> Appearance {
    let [r, g, b] = path.color;
    let color = Paint::solid(Color::rgb8(r, g, b));
    match path.stroke {
        Some(width) => Appearance::basic(Paint::None, color, width),
        None => Appearance::basic(color, Paint::None, 0.0),
    }
}

/// Builds one path node per traced path (`Node::path(id, PathData, Appearance)`,
/// editors/vector/crates/doc/src/node.rs L864), moved by `origin`, inside a group named
/// [`GROUP_NAME`], and inserts the group at the top of `layer` (`Document::insert`,
/// editors/vector/crates/doc/src/lib.rs L1002). Returns the group's id and the path ids,
/// bottom-most first, in the trace's paint order.
pub fn trace_to_nodes(
    doc: &mut Document,
    result: &TraceResult,
    origin: (f64, f64),
    layer: Option<NodeId>,
) -> Result<(NodeId, Vec<NodeId>), redrob_vector_doc::DocError> {
    let shift = Affine::translate(origin);
    let mut ids = Vec::with_capacity(result.paths.len());
    let mut children = Vec::with_capacity(result.paths.len());
    for traced in &result.paths {
        let id = doc.alloc_id();
        let path = traced.path.transformed(shift);
        children.push(Arc::new(Node::path(id, path, appearance_for(traced))));
        ids.push(id);
    }
    let group_id = doc.alloc_id();
    let mut group = Node::group(group_id, children);
    group.name = Some(GROUP_NAME.to_owned());
    doc.insert(layer, usize::MAX, group)?;
    Ok((group_id, ids))
}

/// Puts `result` into the Vector editor's active document (creating a `size` document when none
/// is open) and selects the new group. The shell switches to Vector mode.
pub fn deliver_trace(
    shell: &mut ShellApp,
    result: &TraceResult,
    size: (u32, u32),
) -> Result<usize, String> {
    let count = result.paths.len();
    let app = shell.vector_app();
    if app.session.active().is_none() {
        // `file.new {width, height}` (editors/vector/crates/engine/src/cmd/docinfo.rs L388
        // uses it this way); `sync_views` aligns the views with the documents
        // (editors/vector/crates/ui-egui/src/lib.rs L570).
        app.run(
            "file.new",
            serde_json::json!({ "width": f64::from(size.0), "height": f64::from(size.1) }),
        )?;
        app.sync_views();
    }
    let origin = app
        .session
        .active()
        .and_then(|st| {
            let index = app.view().map(|v| v.artboard).unwrap_or(0);
            st.doc
                .artboards
                .get(index)
                .or_else(|| st.doc.artboards.first())
                .map(|a| (a.rect.x0, a.rect.y0))
        })
        .unwrap_or((0.0, 0.0));
    app.session
        .edit("Vectorize", |doc, sel| {
            let layer = doc.default_layer();
            let (group, _) = trace_to_nodes(doc, result, origin, layer)?;
            sel.set([group]);
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    shell.set_mode(Mode::Vector);
    Ok(count)
}

/// Snapshot the current mode's pixels, trace them with `preset` and hand the paths to Vector
/// mode. Synchronous (the panel's card runs the trace on the runtime instead and calls
/// [`deliver_trace`] with the result); the agent's `vectorize` tool and tests use this one.
pub fn vectorize_to_vector(shell: &mut ShellApp, preset: &str) -> Result<usize, String> {
    let image = shell
        .with_bridge(|bridge| bridge.snapshot())
        .ok_or_else(|| "open a document first".to_string())?;
    let size = (image.width, image.height);
    let result = redrob_ai::vectorize::vectorize(&image, preset).map_err(|e| e.to_string())?;
    deliver_trace(shell, &result, size)
}

/// Runs the hand-off from the active mode to `target` ([`Handoff::between`]) and switches the
/// shell to `target`. Returns a one-line summary for the status line.
pub fn send_to(shell: &mut ShellApp, target: Mode) -> Result<String, String> {
    let source = shell.mode();
    let Some(handoff) = Handoff::between(source, target) else {
        return Err(format!(
            "nothing to send from {} to {}",
            source.label(),
            target.label()
        ));
    };
    match handoff {
        Handoff::RasterToLayout => raster_to_layout(shell),
        Handoff::VectorToLayout => vector_to_layout(shell),
        Handoff::VectorToPhoto => vector_to_photo(shell),
        Handoff::LayoutPageToPhoto => layout_page_to_photo(shell),
        Handoff::TraceToVector => {
            let paths = vectorize_to_vector(shell, DEFAULT_TRACE_PRESET)?;
            Ok(format!("Traced {paths} paths into Vector"))
        }
    }
}

/// A file name for a hand-off asset: the source document's name with `ext`, the source name's own
/// extension dropped (`poster.psd` -> `poster.png`).
pub fn asset_name(source: &str, ext: &str) -> String {
    let stem = std::path::Path::new(source)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Untitled".to_owned());
    format!("{stem}.{ext}")
}

/// Photo: the active document's flattened pixels and its name.
fn photo_pixels(shell: &mut ShellApp) -> Result<(String, Rgba8Image), String> {
    let app = shell.photo_app();
    let name = app
        .session
        .active()
        .map(|st| st.doc.name.clone())
        .ok_or_else(|| "open a Photo document first".to_owned())?;
    let image = PhotoBridge::new(&mut app.session)
        .snapshot()
        .ok_or_else(|| "the Photo document has no pixels".to_owned())?;
    Ok((name, image))
}

/// Photo flattened pixels -> PNG -> Layout asset, placed at the current page's content box.
pub fn raster_to_layout(shell: &mut ShellApp) -> Result<String, String> {
    let (name, image) = photo_pixels(shell)?;
    let png = image.to_png().map_err(|e| e.to_string())?;
    let file = asset_name(&name, "png");
    let natural = (f64::from(image.width), f64::from(image.height));
    place_in_layout(shell, &file, &png, natural)?;
    shell.set_mode(Mode::Layout);
    Ok(format!("Placed {file} on the Layout page"))
}

/// The Vector document's active artboard as an SVG string (`redrob_vector_svg::export`,
/// editors/vector/crates/svg/src/lib.rs L321; `ExportOptions::artboard` sets the viewBox,
/// L186), with the document's title and the artboard's size in points. `None` with no document.
pub fn vector_svg(
    app: &redrob_vector_ui_egui::VectorApp,
) -> Option<(String, String, (f64, f64))> {
    let st = app.session.active()?;
    let index = app
        .view()
        .map(|v| v.artboard)
        .filter(|i| *i < st.doc.artboards.len())
        .unwrap_or(0);
    let board = st.doc.artboards.get(index)?;
    let size = (board.rect.width(), board.rect.height());
    let opts = redrob_vector_svg::ExportOptions {
        artboard: Some(index),
        ..redrob_vector_svg::ExportOptions::default()
    };
    let svg = redrob_vector_svg::export(&st.doc, &opts);
    Some((st.title(), svg, size))
}

/// Vector artboard -> SVG -> Layout asset on the current page. SVG assets are kept as SVG and
/// rasterised by resvg when drawn (`redrob_layout_images::is_svg`,
/// editors/layout/crates/images/src/lib.rs L27; `redrob_layout_render::natural_size` reads the
/// SVG's own size, crates/render/src/lib.rs L1149).
pub fn vector_to_layout(shell: &mut ShellApp) -> Result<String, String> {
    let (title, svg, natural) =
        vector_svg(shell.vector_app()).ok_or_else(|| "open a Vector document first".to_owned())?;
    if svg.trim().is_empty() {
        return Err("the Vector document exported as an empty SVG".to_owned());
    }
    let file = asset_name(&title, "svg");
    place_in_layout(shell, &file, svg.as_bytes(), natural)?;
    shell.set_mode(Mode::Layout);
    Ok(format!("Placed {file} on the Layout page"))
}

/// Puts `bytes` (a PNG or an SVG named `file`) on the Layout page as a graphic frame at the
/// page's content box, `natural` points wide unless the box is narrower (the height follows).
/// With no Layout document open, a page of the graphic's size and no margins is created first
/// (`file.new {width, height, margins, title}`, editors/layout/crates/engine/src/cmd/file.rs
/// L120). The placement is `file.place {base64, name, spread, x, y, width}` (L292): the asset
/// goes in with `Document::add_asset`, the frame holds `Content::Graphic` and lands with
/// `insert_item` (L381, L414, L422).
fn place_in_layout(
    shell: &mut ShellApp,
    file: &str,
    bytes: &[u8],
    natural: (f64, f64),
) -> Result<(), String> {
    let app = shell.layout_app();
    if app.session.active().is_none() {
        let title = std::path::Path::new(file)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_owned());
        app.run(
            "file.new",
            json!({
                "width": natural.0.max(1.0),
                "height": natural.1.max(1.0),
                "margins": 0.0,
                "title": title,
            }),
        )?;
    }
    let (content, spread) = LayoutBridge::new(app)
        .content_box()
        .ok_or_else(|| "the Layout document has no page".to_owned())?;
    let width = natural.0.min(content.width()).max(1.0);
    // `redrob_vector_svg::base64_encode` (editors/vector/crates/svg/src/lib.rs L546): the
    // standard alphabet with padding, which `file.place`'s decoder reads.
    let params = json!({
        "base64": redrob_vector_svg::base64_encode(bytes),
        "name": file,
        "spread": spread,
        "x": content.x0,
        "y": content.y0,
        "width": width,
    });
    app.run("file.place", params).map(|_| ())
}

/// A new Photo document holding `image` as its one raster layer named `name`
/// (`Document::new(name, size, mode, depth)`, editors/photo/crates/doc/src/lib.rs L947;
/// `Layer::new` L672; `Document::insert_above(None, layer)` L1168 appends at the top).
pub fn photo_document_from(name: &str, image: &Rgba8Image) -> redrob_photo_doc::Document {
    let size = redrob_photo_geom::Size::new(image.width, image.height);
    let mut doc = redrob_photo_doc::Document::new(
        name,
        size,
        redrob_photo_color::ColorMode::Rgb,
        redrob_photo_color::SampleType::U8,
    );
    let rect = redrob_photo_geom::Rect::new(0, 0, image.width as i32, image.height as i32);
    let rgba8 = redrob_photo_color::PixelFormat::RGBA8;
    let mut surface = redrob_photo_raster::Surface::from_interleaved(rgba8, rect, &image.data);
    let want = doc.pixel_format();
    if want != rgba8 {
        surface = surface.convert(want);
    }
    let layer = redrob_photo_doc::Layer::new(name, redrob_photo_doc::LayerContent::Raster(surface));
    doc.insert_above(None, layer);
    doc
}

/// Puts `image` into Photo mode: as a layer named `name` at the origin of the open document, or
/// as a new document named `name` when none is open (`Session::add_document`,
/// editors/photo/crates/engine/src/lib.rs L556, then `PhotoApp::sync_views`,
/// editors/photo/crates/ui-egui/src/lib.rs L1182, as the editor's own imports do, L1348).
pub fn deliver_pixels_to_photo(
    shell: &mut ShellApp,
    name: &str,
    image: Rgba8Image,
) -> Result<String, String> {
    let (w, h) = (image.width, image.height);
    let app = shell.photo_app();
    let summary = if app.session.active().is_some() {
        PhotoBridge::new(&mut app.session)
            .place_layer(name, image, (0, 0))
            .map_err(|e| e.to_string())?;
        format!("Added layer {name} ({w} × {h}) to the Photo document")
    } else {
        let doc = photo_document_from(name, &image);
        app.session.add_document(doc, None);
        app.sync_views();
        format!("Opened {name} ({w} × {h}) in Photo")
    };
    shell.set_mode(Mode::Photo);
    Ok(summary)
}

/// Vector artboard rasterised at document size (1 pt = 1 px, `VectorBridge::snapshot`) -> Photo.
pub fn vector_to_photo(shell: &mut ShellApp) -> Result<String, String> {
    let app = shell.vector_app();
    let title = app
        .session
        .active()
        .map(|st| st.title())
        .ok_or_else(|| "open a Vector document first".to_owned())?;
    let image = VectorBridge::new(app)
        .snapshot()
        .ok_or_else(|| "the Vector document has no artboard to render".to_owned())?;
    let name = asset_name(&title, "png");
    deliver_pixels_to_photo(shell, &name, image)
}

/// The current Layout page rendered at 1 pt = 1 px (`LayoutBridge::snapshot`) -> Photo.
pub fn layout_page_to_photo(shell: &mut ShellApp) -> Result<String, String> {
    let app = shell.layout_app();
    let title = app
        .session
        .active()
        .map(|st| st.title())
        .ok_or_else(|| "open a Layout document first".to_owned())?;
    let image = LayoutBridge::new(app)
        .snapshot()
        .ok_or_else(|| "the Layout document has no page to render".to_owned())?;
    let name = asset_name(&title, "png");
    deliver_pixels_to_photo(shell, &name, image)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use redrob_vector_geom::{PathData, Point, Rect, shapes};

    fn traced(x: f64, color: [u8; 3], stroke: Option<f64>) -> TracedPath {
        TracedPath {
            path: shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)),
            color,
            pixels: 100,
            stroke,
            shape: None,
            gradient: None,
            square_caps: false,
        }
    }

    #[test]
    fn handoff_builds_one_node_per_traced_path_in_a_named_group() {
        let result = TraceResult {
            paths: vec![
                traced(0.0, [255, 0, 0], None),
                traced(20.0, [0, 255, 0], None),
                traced(40.0, [0, 0, 255], Some(2.0)),
            ],
            palette: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]],
            parents: Vec::new(),
        };
        let mut doc = Document::new(100.0, 100.0);
        let layer = doc.default_layer();
        let (group, ids) = trace_to_nodes(&mut doc, &result, (5.0, 7.0), layer).unwrap();
        assert_eq!(ids.len(), 3);
        let group_node = doc.node(group).unwrap();
        assert_eq!(group_node.name.as_deref(), Some(GROUP_NAME));
        let children = group_node.children().unwrap();
        assert_eq!(children.len(), 3);
        for (child, id) in children.iter().zip(&ids) {
            assert_eq!(child.id, *id);
        }
        // Coordinates are shifted by the origin, 1 px = 1 unit.
        let first: &PathData = children[0].path_data().unwrap();
        let bounds = first.control_bounds().unwrap();
        assert_eq!(bounds.origin(), Point::new(5.0, 7.0));
        assert_eq!(bounds.width(), 10.0);
        // Fills carry the colour; a stroked path has no fill.
        assert!(children[0].appearance.fill().is_some());
        let stroked = &children[2].appearance;
        assert!(matches!(stroked.fill_paint(), Paint::None));
    }

    #[test]
    fn an_empty_trace_still_makes_the_group() {
        let mut doc = Document::new(10.0, 10.0);
        let (group, ids) =
            trace_to_nodes(&mut doc, &TraceResult::default(), (0.0, 0.0), None).unwrap();
        assert!(ids.is_empty());
        assert!(doc.node(group).is_some());
    }

    #[test]
    fn appearance_follows_stroke_or_fill() {
        let filled = appearance_for(&traced(0.0, [1, 2, 3], None));
        assert!(filled.fill().is_some());
        let stroked = appearance_for(&traced(0.0, [1, 2, 3], Some(3.0)));
        assert!(matches!(stroked.fill_paint(), Paint::None));
    }

    #[test]
    fn handoff_matrix_has_no_self_target_and_covers_every_pair() {
        for source in Mode::ALL {
            assert_eq!(Handoff::between(source, source), None, "{source:?}");
            for target in Mode::ALL.into_iter().filter(|t| *t != source) {
                let h = Handoff::between(source, target)
                    .unwrap_or_else(|| panic!("{source:?} -> {target:?} has no hand-off"));
                assert_eq!(h.target(), target);
                assert!(!h.description().is_empty());
            }
        }
        assert_eq!(
            Handoff::between(Mode::Photo, Mode::Layout),
            Some(Handoff::RasterToLayout)
        );
        assert_eq!(
            Handoff::between(Mode::Vector, Mode::Layout),
            Some(Handoff::VectorToLayout)
        );
        assert_eq!(
            Handoff::between(Mode::Vector, Mode::Photo),
            Some(Handoff::VectorToPhoto)
        );
        assert_eq!(
            Handoff::between(Mode::Layout, Mode::Photo),
            Some(Handoff::LayoutPageToPhoto)
        );
        assert_eq!(
            Handoff::between(Mode::Photo, Mode::Vector),
            Some(Handoff::TraceToVector)
        );
        assert_eq!(
            Handoff::between(Mode::Layout, Mode::Vector),
            Some(Handoff::TraceToVector)
        );
        // Every variant is reachable from some pair.
        for h in Handoff::ALL {
            assert!(
                Mode::ALL.iter().any(|s| Mode::ALL
                    .iter()
                    .any(|t| Handoff::between(*s, *t) == Some(h))),
                "{h:?} unreachable"
            );
        }
    }

    #[test]
    fn asset_names_swap_the_extension() {
        assert_eq!(asset_name("poster.psd", "png"), "poster.png");
        assert_eq!(asset_name("Untitled-1", "svg"), "Untitled-1.svg");
        assert_eq!(asset_name("", "png"), "Untitled.png");
        assert_eq!(asset_name("a.b.c", "svg"), "a.b.svg");
    }

    #[test]
    fn a_one_path_vector_document_exports_a_non_empty_svg() {
        let mut doc = Document::new(100.0, 80.0);
        let layer = doc.default_layer();
        let id = doc.alloc_id();
        let path = shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 50.0));
        let appearance =
            Appearance::basic(Paint::solid(Color::rgb8(200, 30, 30)), Paint::None, 0.0);
        doc.insert(layer, usize::MAX, Node::path(id, path, appearance))
            .unwrap();
        let opts = redrob_vector_svg::ExportOptions {
            artboard: Some(0),
            ..redrob_vector_svg::ExportOptions::default()
        };
        let svg = redrob_vector_svg::export(&doc, &opts);
        assert!(!svg.trim().is_empty());
        assert!(svg.contains("<svg"), "{svg}");
        assert!(svg.contains("<path"), "{svg}");
        // The SVG is what Layout stores as the asset: layout recognises it as SVG.
        assert!(redrob_layout_images::is_svg(svg.as_bytes()));
    }

    #[test]
    fn a_photo_document_is_built_around_the_handed_off_pixels() {
        let red = Rgba8Image::filled(3, 2, [255, 0, 0, 255]).unwrap();
        let doc = photo_document_from("art.png", &red);
        assert_eq!(doc.name, "art.png");
        assert_eq!((doc.size.width, doc.size.height), (3, 2));
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].name, "art.png");
        let flat = redrob_photo_compose::flatten(&doc).to_rgba8();
        assert_eq!((flat.width, flat.height), (3, 2));
        assert_eq!(&flat.pixels[0..4], &[255, 0, 0, 255]);
    }
}
