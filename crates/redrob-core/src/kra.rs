// SPDX-License-Identifier: GPL-3.0-or-later

//! Krita document (KRA) import/export, re-derived from the KRA container layout and the element
//! names in Krita's `plugins/impex/libkra` (behaviour studied, no code copied). A KRA is a ZIP:
//! a `mimetype` entry (`application/x-krita`), a `maindoc.xml` describing the image and its layer
//! stack, each layer's pixels, and a `mergedimage.png` composite.
//!
//! Krita's NATIVE tiled paint-layer data is both read and written (see `kra_tiles`), so a file authored
//! in Krita opens with its layer stack and a file we write opens in Krita as layers rather than as a
//! flat image. On import a layer's pixels are taken from the tiled device; a full-canvas PNG at the same
//! path is still accepted (that is what this writer used to produce), and `mergedimage.png` remains the
//! last resort for a layer shape neither path can read.

use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{
    Document, DocumentImportBuilder, ExportOptions, FileFormat, FormatError, FormatWarning,
    FrameId, ImportNode, ImportOptions, NodeKind, RasterCel, RenderSnapshot, Result,
};

const MIMETYPE: &[u8] = b"application/x-krita";
const DOC_NAME: &str = "redrob";

pub(crate) fn has_krita_mimetype(bytes: &[u8]) -> bool {
    // The uncompressed `mimetype` entry must be the archive's FIRST file, exactly like ORA — and this
    // reads just that entry, as ORA's own detection does, rather than running the full archive
    // hardening pass. Detection is asked about every file that starts with `PK`; expanding one to
    // decide it is not a Krita document is work for nothing.
    let Ok(mut archive) = ZipArchive::new(Cursor::new(bytes)) else {
        return false;
    };
    let Ok(mut first) = archive.by_index(0) else {
        return false;
    };
    if first.name() != "mimetype" || first.compression() != CompressionMethod::Stored {
        return false;
    }
    let mut value = Vec::new();
    first.read_to_end(&mut value).is_ok() && value == MIMETYPE
}

/// Minimal attribute reader: find `name="value"` within an element string.
fn attr<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let start = element.find(&key)? + key.len();
    let rest = &element[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

pub(crate) fn import_kra(
    bytes: &[u8],
    _options: &ImportOptions,
) -> Result<(Document, Vec<FormatWarning>)> {
    let files = crate::ora::read_archive(bytes)?;
    if files.get("mimetype").map(Vec::as_slice) != Some(MIMETYPE) {
        return Err(FormatError::Malformed("not a KRA (bad mimetype)").into());
    }
    let xml = files
        .get("maindoc.xml")
        .ok_or(FormatError::Malformed("missing maindoc.xml"))?;
    let xml =
        std::str::from_utf8(xml).map_err(|_| FormatError::Malformed("maindoc.xml not UTF-8"))?;

    // Image dimensions from the <IMAGE ...> element.
    let image_el =
        slice_element(xml, "IMAGE").ok_or(FormatError::Malformed("maindoc.xml missing IMAGE"))?;
    let width: u32 = attr(image_el, "width")
        .and_then(|v| v.parse().ok())
        .ok_or(FormatError::Malformed("IMAGE width"))?;
    let height: u32 = attr(image_el, "height")
        .and_then(|v| v.parse().ok())
        .ok_or(FormatError::Malformed("IMAGE height"))?;
    if width == 0
        || height == 0
        || width > crate::document::MAX_DIMENSION
        || height > crate::document::MAX_DIMENSION
    {
        return Err(FormatError::Malformed("KRA dimensions out of range").into());
    }

    let mut builder = DocumentImportBuilder::new(width, height)?;
    let mut warnings = Vec::new();

    // Each <layer .../> element in the stack. Krita writes them top-first; our siblings are
    // bottom-first, so collect then reverse.
    let mut layer_elements: Vec<&str> = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<layer ") {
        let after = &rest[start..];
        let end = after
            .find("/>")
            .or_else(|| after.find('>'))
            .unwrap_or(after.len());
        layer_elements.push(&after[..end]);
        rest = &after[end..];
    }

    let mut pushed = 0;
    for el in layer_elements.into_iter().rev() {
        let name = attr(el, "name").unwrap_or("Layer").to_string();
        let opacity = attr(el, "opacity")
            .and_then(|v| v.parse::<f32>().ok())
            .map(|o| (o / 255.0).clamp(0.0, 1.0))
            .unwrap_or(1.0);
        let visible = attr(el, "visible").map(|v| v != "0").unwrap_or(true);
        let filename = attr(el, "filename");
        // Three shapes of layer data, tried in order. Our own writer stores a full-canvas PNG; a REAL
        // Krita file stores the native tiled paint device under a directory named after the image, which
        // is not our `DOC_NAME` -- so the tiled lookup matches on the path's tail rather than assuming
        // the document name.
        let pixels = match filename {
            None => continue,
            Some(f) => {
                let png = files
                    .get(&format!("{DOC_NAME}/layers/{f}.png"))
                    .or_else(|| files.get(&format!("{DOC_NAME}/layers/{f}")))
                    .filter(|data| data.starts_with(&[0x89, b'P', b'N', b'G']));
                match png {
                    Some(png) => {
                        let (lw, lh, src) = crate::formats::decode_rgba(png, FileFormat::Png)?;
                        place(&src, lw, lh, width, height)
                    }
                    None => {
                        let tail = format!("/layers/{f}");
                        let tiled = files
                            .iter()
                            .find(|(path, _)| path.ends_with(&tail))
                            .map(|(_, data)| data);
                        match tiled {
                            Some(data) => {
                                // The default pixel is a sidecar, and it is not always transparent: a
                                // layer flood-filled white keeps one white default and no tiles for the
                                // untouched area, so ignoring it opens that layer empty.
                                let default_tail = format!("/layers/{f}.defaultpixel");
                                let default = files
                                    .iter()
                                    .find(|(path, _)| path.ends_with(&default_tail))
                                    .and_then(|(_, data)| {
                                        crate::kra_tiles::decode_default_pixel(data)
                                    });
                                crate::kra_tiles::decode_tiled_layer(data, width, height, default)?
                            }
                            None => continue,
                        }
                    }
                }
            }
        };
        builder.push_node(
            ImportNode::raster(name, vec![RasterCel::new(FrameId::DEFAULT, pixels)])
                .with_visibility(visible)
                .with_opacity(opacity),
        )?;
        pushed += 1;
    }

    if pushed == 0 {
        // Foreign KRA (native tiled layers we do not decode yet): open the merged composite.
        warnings.push(FormatWarning::FlattenedHierarchy);
        let merged = files
            .get("mergedimage.png")
            .ok_or(FormatError::UnsupportedFeature(
                "KRA with only native tiled layers",
            ))?;
        let (lw, lh, src) = crate::formats::decode_rgba(merged, FileFormat::Png)?;
        let pixels = place(&src, lw, lh, width, height);
        builder.push_node(ImportNode::raster(
            "Background",
            vec![RasterCel::new(FrameId::DEFAULT, pixels)],
        ))?;
    }

    Ok((builder.build()?, warnings))
}

/// Clip/pad a decoded buffer to the canvas (KRA layers are canvas-sized in our writer, but a foreign
/// merged image could differ).
fn place(src: &[u8], sw: u32, sh: u32, cw: u32, ch: u32) -> Vec<u8> {
    if sw == cw && sh == ch {
        return src.to_vec();
    }
    let mut out = vec![0u8; cw as usize * ch as usize * 4];
    for y in 0..sh.min(ch) as usize {
        for x in 0..sw.min(cw) as usize {
            let so = (y * sw as usize + x) * 4;
            let d = (y * cw as usize + x) * 4;
            out[d..d + 4].copy_from_slice(&src[so..so + 4]);
        }
    }
    out
}

/// Return the full `<TAG ...>` opening element (without children) as a slice.
fn slice_element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{tag}"))?;
    let after = &xml[start..];
    let end = after.find('>')? + 1;
    Some(&after[..end])
}

pub(crate) fn export_kra(
    document: &Document,
    frame: FrameId,
    options: &ExportOptions,
) -> Result<(Vec<u8>, Vec<FormatWarning>)> {
    let mut warnings = Vec::new();
    let width = document.width();
    let height = document.height();

    // Build maindoc.xml. Krita writes layers top-first, so iterate our nodes in reverse.
    let mut layers_xml = String::new();
    // Each entry is the layer's native tiled paint device plus its default-pixel sidecar, which is what
    // Krita itself reads. Writing our own PNG convention instead made a file only THIS product could
    // open, and left the tile writer untested by our own round-trip.
    let mut layer_files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut index = 0;
    let nodes: Vec<&crate::Layer> = document.nodes().iter().collect();
    for node in nodes.iter().rev() {
        if matches!(node.kind(), NodeKind::Group) {
            warnings.push(FormatWarning::FlattenedHierarchy);
            continue;
        }
        let filename = format!("layer{index}");
        index += 1;
        let pixels = source_pixels(document, node, frame)?;
        let device = crate::kra_tiles::encode_tiled_layer(&pixels, width, height);
        layer_files.push((format!("{DOC_NAME}/layers/{filename}"), device));
        // The default pixel is transparent here: our tiles cover the whole canvas, so nothing outside
        // them is ever consulted. Written anyway rather than left out, because an absent sidecar makes a
        // reader guess at a value this one states.
        layer_files.push((
            format!("{DOC_NAME}/layers/{filename}.defaultpixel"),
            vec![0, 0, 0, 0],
        ));
        let opacity = (node.opacity() * 255.0).round().clamp(0.0, 255.0) as u32;
        let visible = if node.is_visible() { 1 } else { 0 };
        let name = xml_escape(node.name());
        layers_xml.push_str(&format!(
            "   <layer name=\"{name}\" nodetype=\"paintlayer\" opacity=\"{opacity}\" visible=\"{visible}\" filename=\"{filename}\" colorspacename=\"RGBA\" compositeop=\"normal\"/>\n"
        ));
    }

    let maindoc = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE DOC>\n\
         <DOC syntaxVersion=\"2\" editor=\"redrob-canvas\">\n\
         <IMAGE width=\"{width}\" height=\"{height}\" colorspacename=\"RGBA\" name=\"{DOC_NAME}\">\n\
         <layers>\n{layers_xml}</layers>\n\
         </IMAGE>\n</DOC>\n"
    );

    let cursor = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(cursor);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer
        .start_file("mimetype", stored)
        .map_err(map_zip_error)?;
    writer.write_all(MIMETYPE).map_err(map_write_error)?;
    writer
        .start_file("maindoc.xml", deflated)
        .map_err(map_zip_error)?;
    writer
        .write_all(maindoc.as_bytes())
        .map_err(map_write_error)?;
    for (path, data) in &layer_files {
        writer.start_file(path, deflated).map_err(map_zip_error)?;
        writer.write_all(data).map_err(map_write_error)?;
    }
    let merged = RenderSnapshot::try_render_frame(document, 0, frame)?;
    let merged_png = crate::formats::encode_png(merged.width(), merged.height(), merged.pixels())?;
    writer
        .start_file("mergedimage.png", deflated)
        .map_err(map_zip_error)?;
    writer.write_all(&merged_png).map_err(map_write_error)?;
    let bytes = writer.finish().map_err(map_zip_error)?.into_inner();
    if bytes.len() > crate::MAX_FORMAT_OUTPUT_BYTES {
        return Err(FormatError::OutputTooLarge.into());
    }

    let _ = options;
    Ok((bytes, warnings))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn map_zip_error(_error: zip::result::ZipError) -> crate::CoreError {
    FormatError::OutputTooLarge.into()
}
fn map_write_error(_error: std::io::Error) -> crate::CoreError {
    FormatError::OutputTooLarge.into()
}

/// Full-canvas RGBA for one node (shared shape with the ORA/PSD exporters).
fn source_pixels(document: &Document, node: &crate::Layer, frame: FrameId) -> Result<Vec<u8>> {
    let mut pixels = match node.kind() {
        NodeKind::Raster => node.raster_pixels(frame).map_or_else(
            |_| vec![0; document.width() as usize * document.height() as usize * 4],
            <[u8]>::to_vec,
        ),
        NodeKind::Text | NodeKind::Vector => {
            crate::semantic::rasterize(node.content(), document.width(), document.height())?
        }
        NodeKind::Group => unreachable!(),
    };
    if let Some(mask) = node.mask().filter(|mask| mask.is_enabled()) {
        for (pixel, coverage) in pixels.chunks_exact_mut(4).zip(mask.pixels()) {
            pixel[3] = ((u16::from(pixel[3]) * u16::from(*coverage) + 127) / 255) as u8;
        }
    }
    Ok(pixels)
}
