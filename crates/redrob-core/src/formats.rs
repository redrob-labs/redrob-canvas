// SPDX-License-Identifier: GPL-3.0-or-later

//! Typed, bounded file-format boundaries for the UI-independent core.

use std::io::Cursor;

use image::{ColorType, ImageDecoder, ImageEncoder, ImageReader, Limits};

use crate::document::{MAX_DIMENSION, MAX_PIXELS};
use crate::{CoreError, Document, FrameId, NodeKind, Pixel, RenderSnapshot, Result};

pub const MAX_FORMAT_INPUT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_FORMAT_OUTPUT_BYTES: usize = 512 * 1024 * 1024;

/// Formats accepted by the generic core API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FileFormat {
    Rrg,
    Png,
    Jpeg,
    WebP,
    Ora,
    Svg,
}

/// Policy for formats that cannot represent straight alpha.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AlphaPolicy {
    #[default]
    RejectNonOpaque,
    Flatten {
        matte: Pixel,
    },
}

/// Policy for representational loss at a format boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LossPolicy {
    #[default]
    RejectLoss,
    AllowLoss,
}

/// Stable machine-readable warnings emitted only for explicitly allowed loss.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FormatWarning {
    FlattenedHierarchy,
    FlattenedAlpha { matte: Pixel },
    RasterizedSemanticNode { node: crate::NodeId },
    BakedRasterMask { node: crate::NodeId },
    OmittedDisabledMask { node: crate::NodeId },
    OmittedFrames { exported: FrameId },
    OmittedSelection,
    OmittedMetadata,
    EmbeddedRasterData { node: crate::NodeId },
}

/// Effective metadata for one completed import or export.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveFormatMetadata {
    pub format: FileFormat,
    pub width: u32,
    pub height: u32,
    pub frame: Option<FrameId>,
    pub jpeg_quality: Option<u8>,
    pub lossless: bool,
}

/// Bounded generic import controls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportOptions {
    expected_format: Option<FileFormat>,
    loss_policy: LossPolicy,
    max_input_bytes: usize,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            expected_format: None,
            loss_policy: LossPolicy::RejectLoss,
            max_input_bytes: MAX_FORMAT_INPUT_BYTES,
        }
    }
}

impl ImportOptions {
    pub fn with_expected_format(mut self, format: FileFormat) -> Self {
        self.expected_format = Some(format);
        self
    }

    pub fn with_loss_policy(mut self, policy: LossPolicy) -> Self {
        self.loss_policy = policy;
        self
    }

    pub fn with_max_input_bytes(mut self, bytes: usize) -> Self {
        self.max_input_bytes = bytes;
        self
    }

    pub const fn expected_format(&self) -> Option<FileFormat> {
        self.expected_format
    }

    pub const fn loss_policy(&self) -> LossPolicy {
        self.loss_policy
    }

    fn validate(&self) -> std::result::Result<(), FormatError> {
        if self.max_input_bytes == 0 || self.max_input_bytes > MAX_FORMAT_INPUT_BYTES {
            return Err(FormatError::InvalidOption("max input bytes"));
        }
        Ok(())
    }
}

/// Bounded generic export controls. Defaults reject all degradation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportOptions {
    frame: Option<FrameId>,
    alpha_policy: AlphaPolicy,
    loss_policy: LossPolicy,
    jpeg_quality: u8,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            frame: None,
            alpha_policy: AlphaPolicy::RejectNonOpaque,
            loss_policy: LossPolicy::RejectLoss,
            jpeg_quality: 90,
        }
    }
}

impl ExportOptions {
    pub fn with_frame(mut self, frame: FrameId) -> Self {
        self.frame = Some(frame);
        self
    }

    pub fn with_alpha_policy(mut self, policy: AlphaPolicy) -> Self {
        self.alpha_policy = policy;
        self
    }

    pub fn with_loss_policy(mut self, policy: LossPolicy) -> Self {
        self.loss_policy = policy;
        self
    }

    pub fn with_jpeg_quality(mut self, quality: u8) -> Self {
        self.jpeg_quality = quality;
        self
    }

    pub const fn frame(&self) -> Option<FrameId> {
        self.frame
    }

    pub const fn alpha_policy(&self) -> AlphaPolicy {
        self.alpha_policy
    }

    pub const fn loss_policy(&self) -> LossPolicy {
        self.loss_policy
    }

    pub const fn jpeg_quality(&self) -> u8 {
        self.jpeg_quality
    }

    fn validate(&self) -> std::result::Result<(), FormatError> {
        if !(1..=100).contains(&self.jpeg_quality) {
            return Err(FormatError::InvalidOption(
                "JPEG quality must be in 1..=100",
            ));
        }
        if let AlphaPolicy::Flatten { matte } = self.alpha_policy
            && matte.a != 255
        {
            return Err(FormatError::InvalidOption("JPEG matte must be opaque"));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FormatError {
    #[error("file format could not be detected")]
    UnknownFormat,
    #[error("detected format {detected:?} does not match expected format {expected:?}")]
    FormatMismatch {
        expected: FileFormat,
        detected: FileFormat,
    },
    #[error("format input exceeds its byte limit")]
    InputTooLarge,
    #[error("format output exceeds its byte limit")]
    OutputTooLarge,
    #[error("invalid format option: {0}")]
    InvalidOption(&'static str),
    #[error("unsupported format feature: {0}")]
    UnsupportedFeature(&'static str),
    #[error("operation would lose information: {0}")]
    LossRequired(&'static str),
    #[error("malformed format content: {0}")]
    Malformed(&'static str),
    #[error("format content exceeds the {0} limit")]
    LimitExceeded(&'static str),
}

#[derive(Debug)]
pub struct ImportOutcome {
    document: Document,
    warnings: Vec<FormatWarning>,
    metadata: EffectiveFormatMetadata,
}

impl ImportOutcome {
    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn into_document(self) -> Document {
        self.document
    }

    pub fn warnings(&self) -> &[FormatWarning] {
        &self.warnings
    }

    pub fn metadata(&self) -> &EffectiveFormatMetadata {
        &self.metadata
    }
}

#[derive(Debug)]
pub struct ExportOutcome {
    bytes: Vec<u8>,
    warnings: Vec<FormatWarning>,
    metadata: EffectiveFormatMetadata,
}

impl ExportOutcome {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn warnings(&self) -> &[FormatWarning] {
        &self.warnings
    }

    pub fn metadata(&self) -> &EffectiveFormatMetadata {
        &self.metadata
    }
}

fn format_metadata(
    format: FileFormat,
    document: &Document,
    frame: Option<FrameId>,
    quality: Option<u8>,
    lossless: bool,
) -> EffectiveFormatMetadata {
    EffectiveFormatMetadata {
        format,
        width: document.width(),
        height: document.height(),
        frame,
        jpeg_quality: quality,
        lossless,
    }
}

/// Detects a format from strict content signatures. Extension guessing is never used.
pub fn detect_format(bytes: &[u8]) -> std::result::Result<FileFormat, FormatError> {
    if bytes.len() > MAX_FORMAT_INPUT_BYTES {
        return Err(FormatError::InputTooLarge);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(FileFormat::Png);
    }
    if bytes.len() >= 3 && bytes[..3] == [0xff, 0xd8, 0xff] {
        return Ok(FileFormat::Jpeg);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Ok(FileFormat::WebP);
    }
    if bytes.starts_with(b"PK\x03\x04") && crate::ora::has_canonical_mimetype(bytes) {
        return Ok(FileFormat::Ora);
    }
    if bytes
        .first()
        .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'<')
        && crate::svg::has_svg_root(bytes)
    {
        return Ok(FileFormat::Svg);
    }
    if bytes.first() == Some(&b'{') {
        let header = serde_json::from_slice::<serde_json::Value>(bytes)
            .map_err(|_| FormatError::UnknownFormat)?;
        if header.get("magic").and_then(serde_json::Value::as_str) == Some("REDROB_CANVAS_PROJECT")
        {
            return Ok(FileFormat::Rrg);
        }
    }
    Err(FormatError::UnknownFormat)
}

fn validate_detected_format(bytes: &[u8], options: &ImportOptions) -> Result<FileFormat> {
    options.validate()?;
    if bytes.len() > options.max_input_bytes {
        return Err(FormatError::InputTooLarge.into());
    }
    let detected = detect_format(bytes)?;
    if let Some(expected) = options.expected_format
        && detected != expected
    {
        return Err(FormatError::FormatMismatch { expected, detected }.into());
    }
    Ok(detected)
}

/// Imports a strictly detected format through a bounded adapter.
pub fn import_document(bytes: &[u8], options: &ImportOptions) -> Result<ImportOutcome> {
    let format = validate_detected_format(bytes, options)?;
    let (document, warnings) = match format {
        FileFormat::Rrg => (crate::codec::load_project(bytes)?, Vec::new()),
        FileFormat::Png | FileFormat::Jpeg | FileFormat::WebP => {
            (decode_raster(bytes, format)?, Vec::new())
        }
        FileFormat::Ora => crate::ora::import_ora(bytes, options)?,
        FileFormat::Svg => crate::svg::import_svg(bytes, options)?,
    };
    let metadata = format_metadata(format, &document, None, None, format != FileFormat::Jpeg);
    Ok(ImportOutcome {
        document,
        warnings,
        metadata,
    })
}

fn image_format(format: FileFormat) -> Option<image::ImageFormat> {
    match format {
        FileFormat::Png => Some(image::ImageFormat::Png),
        FileFormat::Jpeg => Some(image::ImageFormat::Jpeg),
        FileFormat::WebP => Some(image::ImageFormat::WebP),
        _ => None,
    }
}

pub(crate) fn decode_rgba(bytes: &[u8], format: FileFormat) -> Result<(u32, u32, Vec<u8>)> {
    let expected =
        image_format(format).ok_or(FormatError::UnsupportedFeature("not a raster codec"))?;
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    if reader.format() != Some(expected) {
        return Err(FormatError::FormatMismatch {
            expected: format,
            detected: reader
                .format()
                .and_then(|value| match value {
                    image::ImageFormat::Png => Some(FileFormat::Png),
                    image::ImageFormat::Jpeg => Some(FileFormat::Jpeg),
                    image::ImageFormat::WebP => Some(FileFormat::WebP),
                    _ => None,
                })
                .ok_or(FormatError::UnknownFormat)?,
        }
        .into());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_PIXELS.saturating_mul(8));
    reader.limits(limits);
    let decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    crate::document::pixel_count(width, height)?;
    let image = image::DynamicImage::from_decoder(decoder)?.to_rgba8();
    Ok((width, height, image.into_raw()))
}

fn decode_raster(bytes: &[u8], format: FileFormat) -> Result<Document> {
    let (width, height, pixels) = decode_rgba(bytes, format)?;
    Document::from_single_layer(width, height, pixels, String::new())
}

pub(crate) fn encode_png(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes).write_image(
        pixels,
        width,
        height,
        ColorType::Rgba8.into(),
    )?;
    if bytes.len() > MAX_FORMAT_OUTPUT_BYTES {
        return Err(FormatError::OutputTooLarge.into());
    }
    Ok(bytes)
}

pub(crate) fn check_metadata_loss(
    document: &Document,
    policy: LossPolicy,
    warnings: &mut Vec<FormatWarning>,
) -> Result<()> {
    if document.metadata() != &crate::DocumentMetadata::default() {
        if policy == LossPolicy::RejectLoss {
            return Err(FormatError::LossRequired(
                "target format cannot preserve document metadata",
            )
            .into());
        }
        warnings.push(FormatWarning::OmittedMetadata);
    }
    Ok(())
}

fn direct_raster_pixels(document: &Document, frame: FrameId) -> Option<&[u8]> {
    let [node] = document.nodes() else {
        return None;
    };
    (node.kind() == NodeKind::Raster
        && node.is_visible()
        && node.opacity() == 1.0
        && node.blend_mode() == crate::BlendMode::Normal
        && node.mask().is_none())
    .then(|| node.raster_pixels(frame).ok())
    .flatten()
}

fn raster_loss_warnings(
    document: &Document,
    frame: FrameId,
    policy: LossPolicy,
) -> Result<Vec<FormatWarning>> {
    let mut warnings = Vec::new();
    check_metadata_loss(document, policy, &mut warnings)?;
    if document.timeline().frames().len() > 1 {
        if policy == LossPolicy::RejectLoss {
            return Err(FormatError::LossRequired("raster export omits other frames").into());
        }
        warnings.push(FormatWarning::OmittedFrames { exported: frame });
    }
    if document.is_selection_active() {
        if policy == LossPolicy::RejectLoss {
            return Err(FormatError::LossRequired("raster export omits selection").into());
        }
        warnings.push(FormatWarning::OmittedSelection);
    }
    let flattened = document.nodes().len() != 1
        || document.nodes()[0].kind() != NodeKind::Raster
        || !document.nodes()[0].is_visible()
        || document.nodes()[0].mask().is_some()
        || document.nodes()[0].opacity() != 1.0
        || document.nodes()[0].blend_mode() != crate::BlendMode::Normal;
    if flattened {
        if policy == LossPolicy::RejectLoss {
            return Err(
                FormatError::LossRequired("raster export flattens document structure").into(),
            );
        }
        warnings.push(FormatWarning::FlattenedHierarchy);
    }
    for node in document.nodes() {
        if matches!(node.kind(), NodeKind::Text | NodeKind::Vector) {
            if policy == LossPolicy::RejectLoss {
                return Err(
                    FormatError::LossRequired("raster export rasterizes semantic nodes").into(),
                );
            }
            warnings.push(FormatWarning::RasterizedSemanticNode { node: node.id() });
        }
    }
    Ok(warnings)
}

/// Exports a document through a typed bounded adapter.
pub fn export_document(
    document: &Document,
    format: FileFormat,
    options: &ExportOptions,
) -> Result<ExportOutcome> {
    options.validate()?;
    document.validate()?;
    let frame = options.frame.unwrap_or(document.current_frame_id());
    if !document.timeline().contains(frame) {
        return Err(CoreError::FrameNotFound(frame));
    }

    let (bytes, warnings, quality, lossless) = match format {
        FileFormat::Rrg => {
            if options.frame.is_some() {
                return Err(
                    FormatError::InvalidOption("RRG export does not select a frame").into(),
                );
            }
            (
                crate::codec::save_project(document)?,
                Vec::new(),
                None,
                true,
            )
        }
        FileFormat::Png | FileFormat::WebP | FileFormat::Jpeg => {
            let mut warnings = raster_loss_warnings(document, frame, options.loss_policy)?;
            let rendered = direct_raster_pixels(document, frame)
                .is_none()
                .then(|| RenderSnapshot::try_render_frame(document, 0, frame))
                .transpose()?;
            let pixels = direct_raster_pixels(document, frame)
                .or_else(|| rendered.as_ref().map(RenderSnapshot::pixels))
                .expect("direct or rendered raster pixels");
            if format == FileFormat::Jpeg
                && let AlphaPolicy::Flatten { matte } = options.alpha_policy
                && pixels.chunks_exact(4).any(|pixel| pixel[3] != 255)
            {
                warnings.push(FormatWarning::FlattenedAlpha { matte });
            }
            let bytes = match format {
                FileFormat::Png => encode_png(document.width(), document.height(), pixels)?,
                FileFormat::WebP => {
                    let mut bytes = Vec::new();
                    image::codecs::webp::WebPEncoder::new_lossless(&mut bytes).write_image(
                        pixels,
                        document.width(),
                        document.height(),
                        ColorType::Rgba8.into(),
                    )?;
                    bytes
                }
                FileFormat::Jpeg => {
                    encode_jpeg(document.width(), document.height(), pixels, options)?
                }
                _ => unreachable!(),
            };
            if bytes.len() > MAX_FORMAT_OUTPUT_BYTES {
                return Err(FormatError::OutputTooLarge.into());
            }
            (
                bytes,
                warnings,
                (format == FileFormat::Jpeg).then_some(options.jpeg_quality),
                format != FileFormat::Jpeg,
            )
        }
        FileFormat::Ora => {
            let (bytes, warnings) = crate::ora::export_ora(document, frame, options)?;
            (bytes, warnings, None, true)
        }
        FileFormat::Svg => {
            let (bytes, warnings) = crate::svg::export_svg(document, frame, options)?;
            (bytes, warnings, None, true)
        }
    };
    let metadata = format_metadata(format, document, Some(frame), quality, lossless);
    Ok(ExportOutcome {
        bytes,
        warnings,
        metadata,
    })
}

fn encode_jpeg(width: u32, height: u32, pixels: &[u8], options: &ExportOptions) -> Result<Vec<u8>> {
    let mut rgb = Vec::with_capacity(pixels.len() / 4 * 3);
    for pixel in pixels.chunks_exact(4) {
        let alpha = pixel[3];
        let [r, g, b] = match options.alpha_policy {
            AlphaPolicy::RejectNonOpaque => {
                if alpha != 255 {
                    return Err(FormatError::LossRequired(
                        "JPEG requires RejectNonOpaque input or an explicit opaque matte",
                    )
                    .into());
                }
                [pixel[0], pixel[1], pixel[2]]
            }
            AlphaPolicy::Flatten { matte } => {
                let inverse = u16::from(255 - alpha);
                let alpha = u16::from(alpha);
                [
                    ((u16::from(pixel[0]) * alpha + u16::from(matte.r) * inverse + 127) / 255)
                        as u8,
                    ((u16::from(pixel[1]) * alpha + u16::from(matte.g) * inverse + 127) / 255)
                        as u8,
                    ((u16::from(pixel[2]) * alpha + u16::from(matte.b) * inverse + 127) / 255)
                        as u8,
                ]
            }
        };
        rgb.extend_from_slice(&[r, g, b]);
    }
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, options.jpeg_quality)
        .write_image(&rgb, width, height, ColorType::Rgb8.into())?;
    Ok(bytes)
}

/// Compatibility helper used by the existing PNG wrapper.
pub(crate) fn export_png_compatible(document: &Document) -> Result<Vec<u8>> {
    export_document(
        document,
        FileFormat::Png,
        &ExportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
    )
    .map(ExportOutcome::into_bytes)
}
