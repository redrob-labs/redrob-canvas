// SPDX-License-Identifier: GPL-3.0-or-later

//! Typed, bounded file-format boundaries for the UI-independent core.

use std::io::Cursor;

use image::{ColorType, ImageDecoder, ImageEncoder, ImageReader, Limits};

use crate::document::{MAX_DIMENSION, MAX_PIXELS};
use crate::precision::Precision;
use crate::{CoreError, Document, FrameId, NodeKind, Pixel, RenderSnapshot, Result};

pub const MAX_FORMAT_INPUT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_FORMAT_OUTPUT_BYTES: usize = 512 * 1024 * 1024;

/// The 18-byte TGA 2.0 footer signature, read verbatim from `plug-ins/common/file-tga.c`'s
/// `magic[18]` array — `TRUEVISION-XFILE.` with its terminating NUL.
///
/// One constant for both directions: the sniff looks for it at offset −18 and the export writes
/// it, so the two cannot disagree about what a TGA 2.0 file ends with. Two copies of a signature
/// are two places for one of them to be wrong.
pub const TGA_FOOTER_SIGNATURE: &[u8; 18] = b"TRUEVISION-XFILE.\0";

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
    Psd,
    Kra,
    Xcf,
    Tiff,
    Exr,
    Dds,
    /// Windows/OS2 BMP (M.1).
    ///
    /// Re-derived from `plug-ins/file-bmp/bmp-load.c` and `bmp-export.c`. Several of its facts are
    /// easy to assume wrongly and are pinned by test: rows are padded to a 4-byte stride
    /// (`((width * bits + 31) / 32) * 4`), a NEGATIVE `biHeight` means the rows are stored
    /// top-down, and the default 16-bit layout is **5-5-5** (masks `0x7c00 / 0x03e0 / 0x001f`) and
    /// not 5-6-5.
    Bmp,
    /// Truevision TGA (M.2).
    ///
    /// Re-derived from `plug-ins/common/file-tga.c`. **Its signature is at the END of the file**,
    /// not the start: upstream registers the magic as
    /// `-18&,string,TRUEVISION-XFILE.,-1,byte,0`, which is the 18-byte `TRUEVISION-XFILE.\0`
    /// footer signature at offset −18. A TGA 1.0 file has no footer and therefore **no signature at
    /// all**, so it cannot be detected by content — see `detect_format`.
    Tga,
    Heif,
    /// AVIF: the same ISO base media container as HEIF, but carrying AV1 instead of HEVC. A separate
    /// name because the codec is what a caller has to act on -- refusing an AVIF with a message about
    /// HEVC sends them looking for the wrong thing.
    Avif,
    JpegXl,
    Pdf,
    Raw,
    Gif,
    Apng,
    WebpAnim,
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
    FlattenedAlpha {
        matte: Pixel,
    },
    RasterizedSemanticNode {
        node: crate::NodeId,
    },
    BakedRasterMask {
        node: crate::NodeId,
    },
    OmittedDisabledMask {
        node: crate::NodeId,
    },
    OmittedFrames {
        exported: FrameId,
    },
    OmittedSelection,
    OmittedMetadata,
    EmbeddedRasterData {
        node: crate::NodeId,
    },
    /// The file carried deeper samples than this product's 8-bit rasters hold, so every channel was
    /// narrowed on the way in (H.3). Reported because the loss is real and silent otherwise: a 16-bit
    /// gradient reopened at 8-bit can band, and a 32-bit document's out-of-range values are clamped.
    NarrowedDepth {
        source_bits: u16,
    },
    /// The file was authored in a colour mode this product does not hold, so it was converted to RGB
    /// on the way in (H.4). Named rather than silent because a device space without its profile --
    /// CMYK above all -- converts approximately, and the caller may want to say so.
    ConvertedColorMode {
        source: &'static str,
    },
    /// The file carried a live adjustment layer (levels, curves, hue/saturation, ...) whose effect this
    /// product cannot reproduce as a node (H.5). The layer itself is kept; its effect is not applied,
    /// and this names which one so the difference is attributable instead of looking like a bug.
    UnappliedAdjustment {
        kind: String,
        name: String,
    },
    /// The export packed pixels into GPU blocks, which keep two endpoint colours and a few bits per
    /// pixel (H.11). Reported because the result is an approximation by construction, not because
    /// anything went wrong: a caller must not treat a block-compressed file as an archival copy.
    BlockCompressed {
        fourcc: &'static str,
    },
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

/// Recognise camera-raw containers that are NOT plain baseline TIFF, by their own signatures. The
/// TIFF-based raws (Sony ARW, Nikon NEF, Adobe DNG) are deliberately NOT matched here: they are
/// valid TIFF and open through the TIFF path as their embedded preview rather than being rejected.
fn is_camera_raw(bytes: &[u8]) -> bool {
    // Canon CR2: a TIFF whose bytes 8..10 are "CR".
    if (bytes.starts_with(b"II*\x00") || bytes.starts_with(b"MM\x00*"))
        && bytes.len() >= 10
        && &bytes[8..10] == b"CR"
    {
        return true;
    }
    // Fujifilm RAF.
    if bytes.starts_with(b"FUJIFILMCCD-RAW") {
        return true;
    }
    // Panasonic RW2.
    if bytes.starts_with(b"IIU\x00") {
        return true;
    }
    // Sigma X3F.
    if bytes.starts_with(b"FOVb") {
        return true;
    }
    // Canon CR3: an ISOBMFF file whose major brand is "crx ".
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && &bytes[8..12] == b"crx " {
        return true;
    }
    false
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
    if bytes.starts_with(b"GIF8") {
        return Ok(FileFormat::Gif);
    }
    if bytes.starts_with(b"8BPS") {
        return Ok(FileFormat::Psd);
    }
    if bytes.starts_with(b"%PDF-") {
        return Ok(FileFormat::Pdf);
    }
    // Camera raw formats that are NOT plain TIFF, checked before the TIFF signature.
    if is_camera_raw(bytes) {
        return Ok(FileFormat::Raw);
    }
    if bytes.starts_with(b"II*\x00") || bytes.starts_with(b"MM\x00*") {
        return Ok(FileFormat::Tiff);
    }
    if bytes.starts_with(&[0x76, 0x2f, 0x31, 0x01]) {
        return Ok(FileFormat::Exr);
    }
    if bytes.starts_with(b"DDS ") {
        return Ok(FileFormat::Dds);
    }
    // BMP. Upstream accepts six two-byte signatures — `BM`, and the OS/2 `BA`, `IC`, `PT`, `CI`
    // and `CP` — but only `BM` is a standalone bitmap: the other five are OS/2 array, icon,
    // pointer and colour variants, and `BA` is a CONTAINER whose loader loops over the array until
    // it reaches a `BM`. Sniffing only `BM` is deliberate, so an OS/2 array is reported as an
    // unknown format rather than decoded as the wrong thing. The length check is what stops a
    // two-byte file being claimed: a header is at least 14 + 12 bytes.
    if bytes.len() >= 26 && bytes.starts_with(b"BM") {
        return Ok(FileFormat::Bmp);
    }
    // TGA, and it is the only format here whose signature is at the END of the file. Upstream
    // registers the magic as `-18&,string,TRUEVISION-XFILE.,-1,byte,0`: the 18-byte footer
    // signature at offset −18, which its loader checks as `memcmp (footer + 8, magic, 18)` after
    // reading 26 bytes from the end (4 bytes extension offset + 4 bytes developer offset + the
    // signature).
    //
    // **A TGA 1.0 file has no footer and so no signature at all.** That is a property of the
    // format, not a gap here: upstream's own magic rule cannot detect one either, and it reaches
    // the loader by file extension. Sniffing the 2.0 footer is therefore the whole of what content
    // detection can do, and a footerless TGA must be imported with an explicit expected format.
    if bytes.len() >= 18 && bytes[bytes.len() - 18..] == *TGA_FOOTER_SIGNATURE {
        return Ok(FileFormat::Tga);
    }
    // JPEG-XL: raw codestream (FF 0A) or the ISOBMFF container box.
    if bytes.starts_with(&[0xff, 0x0a])
        || bytes.starts_with(&[
            0x00, 0x00, 0x00, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a,
        ])
    {
        return Ok(FileFormat::JpegXl);
    }
    // HEIF and AVIF share the ISO base media container and are told apart by their BRANDS, including
    // the compatible-brand list -- a file whose major brand is the generic `mif1` can still declare
    // `avif`, and that is the brand that says which codec is inside.
    if let Some(brand) = crate::isobmff::classify(bytes) {
        return Ok(match brand {
            crate::isobmff::ContainerBrand::Avif => FileFormat::Avif,
            crate::isobmff::ContainerBrand::Heif => FileFormat::Heif,
        });
    }
    if bytes.starts_with(b"gimp xcf") {
        return Ok(FileFormat::Xcf);
    }
    if bytes.starts_with(b"PK\x03\x04") && crate::kra::has_krita_mimetype(bytes) {
        return Ok(FileFormat::Kra);
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
        // TIFF and EXR carry depth worth keeping (J.1c-b), and an EXR is NEVER 8-bit, so reading
        // one through the byte path discarded the whole point of the format. The precision comes
        // from the decoded colour type rather than from the format, because a TIFF may be 8-, 16-
        // or 32-bit and only the decoder knows which.
        // These carry depth worth keeping (J.1c-b, J.1c-c). An EXR is NEVER 8-bit, so reading one
        // through the byte path discarded the whole point of the format. The precision comes from
        // the decoded colour type rather than from the format, because a TIFF or a PNG may be 8- or
        // 16-bit and only the decoder knows which.
        FileFormat::Png | FileFormat::Tiff | FileFormat::Exr => {
            let (width, height, mut pixels, precision) = decode_rgba_deep(bytes, format)?;
            // A tagged file states its OWN colour space, and ignoring that tag is not a subtle loss:
            // an Adobe RGB photo opened as sRGB has visibly dull colour, and the file said so all
            // along (H.17). The transform runs AT the file's precision, so colour management no
            // longer costs the depth the import just preserved.
            //
            // Only PNG is read here because it is the only one of these whose profile this product
            // can reach without a second metadata parser.
            let mut warnings = Vec::new();
            if format == FileFormat::Png
                && let Some(profile) = crate::icc::embedded_png_profile(bytes)
            {
                profile.convert_rgba_at(precision, &mut pixels);
                warnings.push(FormatWarning::ConvertedColorMode { source: "icc" });
            }
            let mut builder = crate::DocumentImportBuilder::new(width, height)?;
            builder.precision(precision);
            builder.push_node(crate::ImportNode::raster(
                "Background",
                vec![crate::RasterCel::new(FrameId::DEFAULT, pixels)],
            ))?;
            (builder.build()?, warnings)
        }
        // No depth to keep: JPEG and GIF are 8-bit by their formats, WebP's lossless mode is 8-bit
        // RGBA, and DDS block compression decodes to bytes. They stay on the byte path by right,
        // not by omission. BMP joins them: every variant upstream reads resolves to 8 bits per
        // channel at most — the deepest case is 32-bit, which is 8 bits x 4 and not more depth per
        // sample.
        FileFormat::Jpeg
        | FileFormat::WebP
        | FileFormat::Dds
        | FileFormat::Gif
        | FileFormat::Bmp
        | FileFormat::Tga => {
            let (width, height, pixels) = decode_rgba(bytes, format)?;
            (
                Document::from_single_layer(width, height, pixels, String::new())?,
                Vec::new(),
            )
        }
        FileFormat::Ora => crate::ora::import_ora(bytes, options)?,
        FileFormat::Svg => crate::svg::import_svg(bytes, options)?,
        FileFormat::Psd => crate::psd::import_psd(bytes, options)?,
        FileFormat::Kra => crate::kra::import_kra(bytes, options)?,
        FileFormat::Xcf => crate::xcf::import_xcf(bytes, options)?,
        FileFormat::Heif | FileFormat::Avif => {
            // The container is READ before refusing, so a truncated or corrupt file fails as malformed
            // rather than as an unsupported codec. The distinction is the error's whole value: it says
            // whether the file or this product is the problem.
            crate::isobmff::primary_extent(bytes)?;
            return Err(
                FormatError::UnsupportedFeature(if format == FileFormat::Avif {
                    "AVIF carries AV1, whose only pure-Rust decoder exposes a C-shaped API"
                } else {
                    "HEIF carries HEVC, which has no pure-Rust decoder"
                })
                .into(),
            );
        }
        FileFormat::JpegXl => {
            let (width, height, pixels) = crate::jxl::decode_jxl(bytes)?;
            (
                Document::from_single_layer(width, height, pixels, String::new())?,
                Vec::new(),
            )
        }
        FileFormat::Pdf => {
            let (width, height, pixels) = crate::pdf::decode_pdf(bytes)?;
            (
                Document::from_single_layer(width, height, pixels, String::new())?,
                Vec::new(),
            )
        }
        FileFormat::Raw => {
            let (width, height, pixels) = crate::raw::decode_raw(bytes)?;
            (
                Document::from_single_layer(width, height, pixels, String::new())?,
                Vec::new(),
            )
        }
        FileFormat::Apng | FileFormat::WebpAnim => {
            return Err(
                FormatError::UnsupportedFeature("animation import reads the still format").into(),
            );
        }
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
        FileFormat::Tiff => Some(image::ImageFormat::Tiff),
        FileFormat::Exr => Some(image::ImageFormat::OpenExr),
        FileFormat::Dds => Some(image::ImageFormat::Dds),
        FileFormat::Bmp => Some(image::ImageFormat::Bmp),
        FileFormat::Tga => Some(image::ImageFormat::Tga),
        FileFormat::Gif => Some(image::ImageFormat::Gif),
        _ => None,
    }
}

/// Decodes an image-crate format, KEEPING its sample depth where the document can hold it (J.1c-b).
///
/// Returns the pixels in our own storage encoding together with the precision they are at, so the
/// caller declares that on the import rather than guessing.
///
/// Before this, every one of these formats went through `to_rgba8`. For EXR that discarded the whole
/// point of the format — an EXR is never 8-bit — and unlike the PSD reader it reported nothing at
/// all, so the loss was invisible from both ends.
///
/// The precision is chosen from the DECODED colour type, not from the file extension or the format
/// enum: a TIFF may be 8-, 16- or 32-bit, and the only thing that knows which is the decoder.
pub(crate) fn decode_rgba_deep(
    bytes: &[u8],
    format: FileFormat,
) -> Result<(u32, u32, Vec<u8>, Precision)> {
    let decoded = decode_dynamic(bytes, format)?;
    let (width, height) = (decoded.width(), decoded.height());
    match decoded.color() {
        // Half-float is decoded as f32 by the image crate, so both float types land here. Values
        // outside 0..=1 are KEPT: an EXR carrying highlight headroom is the main reason to read one
        // at float rather than clamping it into an integer.
        image::ColorType::Rgb32F | image::ColorType::Rgba32F => {
            let image = decoded.to_rgba32f();
            let mut out = vec![0u8; Precision::F32.buffer_len((width * height) as usize)];
            for (index, sample) in image.into_raw().into_iter().enumerate() {
                Precision::F32.write_sample(&mut out, index, sample);
            }
            Ok((width, height, out, Precision::F32))
        }
        image::ColorType::Rgb16
        | image::ColorType::Rgba16
        | image::ColorType::L16
        | image::ColorType::La16 => {
            let image = decoded.to_rgba16();
            // The crate's 16-bit samples are already full-scale against 65535, which is our own
            // U16 encoding, so this is a byte re-order and not a rescale. Running them through
            // `write_sample` anyway keeps one encoder for the whole file and costs a multiply.
            let mut out = vec![0u8; Precision::U16.buffer_len((width * height) as usize)];
            for (index, sample) in image.into_raw().into_iter().enumerate() {
                Precision::U16.write_sample(&mut out, index, f32::from(sample) / 65535.0);
            }
            Ok((width, height, out, Precision::U16))
        }
        _ => Ok((width, height, decoded.to_rgba8().into_raw(), Precision::U8)),
    }
}

pub(crate) fn decode_rgba(bytes: &[u8], format: FileFormat) -> Result<(u32, u32, Vec<u8>)> {
    let decoded = decode_dynamic(bytes, format)?;
    let (width, height) = (decoded.width(), decoded.height());
    Ok((width, height, decoded.to_rgba8().into_raw()))
}

/// Decodes to the image crate's own representation, with this product's format check and allocation
/// limits applied.
///
/// Shared by the 8-bit and the depth-preserving paths so the format-mismatch refusal and the
/// dimension and allocation limits cannot differ between them -- a second copy of a limit is a
/// second place for it to be forgotten.
fn decode_dynamic(bytes: &[u8], format: FileFormat) -> Result<image::DynamicImage> {
    let expected =
        image_format(format).ok_or(FormatError::UnsupportedFeature("not a raster codec"))?;
    // TGA has no signature the decoder can guess from: its only magic is the optional TGA 2.0
    // FOOTER, which `detect_format` reads at offset −18 and a content guesser looking at the head
    // of the stream cannot see. So the guess cross-check below is not merely unhelpful for TGA, it
    // always fails — found by probing our own exported file, which detected as TGA and then
    // refused to import. For that format the already-detected value is set directly.
    //
    // This is narrow on purpose. The guess is a SECOND opinion on our own sniff, and dropping it
    // wholesale would let a mislabelled file reach the wrong decoder; it is dropped only where it
    // cannot exist.
    if format == FileFormat::Tga {
        let mut reader = ImageReader::new(Cursor::new(bytes));
        reader.set_format(expected);
        return finish_dynamic_decode(reader);
    }
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    if reader.format() != Some(expected) {
        return Err(FormatError::FormatMismatch {
            expected: format,
            detected: reader
                .format()
                .and_then(|value| match value {
                    image::ImageFormat::Png => Some(FileFormat::Png),
                    image::ImageFormat::Jpeg => Some(FileFormat::Jpeg),
                    image::ImageFormat::WebP => Some(FileFormat::WebP),
                    image::ImageFormat::Tiff => Some(FileFormat::Tiff),
                    image::ImageFormat::OpenExr => Some(FileFormat::Exr),
                    image::ImageFormat::Dds => Some(FileFormat::Dds),
                    image::ImageFormat::Bmp => Some(FileFormat::Bmp),
                    image::ImageFormat::Tga => Some(FileFormat::Tga),
                    image::ImageFormat::Gif => Some(FileFormat::Gif),
                    _ => None,
                })
                .ok_or(FormatError::UnknownFormat)?,
        }
        .into());
    }
    finish_dynamic_decode(reader)
}

/// The limits and the decode itself, shared by the guessed and the format-set paths.
///
/// Factored rather than duplicated: these are this product's allocation and dimension ceilings, and
/// a second copy of a limit is a second place for it to be forgotten — the same reason
/// `decode_dynamic` is itself shared between the 8-bit and depth-preserving callers.
fn finish_dynamic_decode(mut reader: ImageReader<Cursor<&[u8]>>) -> Result<image::DynamicImage> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_PIXELS.saturating_mul(8));
    reader.limits(limits);
    let decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    crate::document::pixel_count(width, height)?;
    Ok(image::DynamicImage::from_decoder(decoder)?)
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

/// Encode RGBA8 pixels through the generic `image` writer for a given format (TIFF, EXR, …). EXR
/// stores float internally; the 8-bit RGBA round-trips through image's conversion.
fn encode_via_image(
    pixels: &[u8],
    width: u32,
    height: u32,
    format: image::ImageFormat,
) -> Result<Vec<u8>> {
    let buffer: image::RgbaImage = image::ImageBuffer::from_raw(width, height, pixels.to_vec())
        .ok_or(FormatError::OutputTooLarge)?;
    let mut bytes = Vec::new();
    buffer
        .write_to(&mut Cursor::new(&mut bytes), format)
        // NOT OutputTooLarge: an encoder that refuses the pixel format, or a codec that is simply not
        // compiled in, reported as "output too large" sends whoever debugs it looking for a size
        // limit that was never reached. That mapping hid the EXR bug this function sits above for a
        // whole porting group.
        .map_err(|_| FormatError::UnsupportedFeature("the image encoder refused this buffer"))?;
    if bytes.len() > MAX_FORMAT_OUTPUT_BYTES {
        return Err(FormatError::OutputTooLarge.into());
    }
    Ok(bytes)
}

/// Encodes OpenEXR, which is a FLOAT format: its encoder accepts only `Rgba32F` / `Rgb32F`, so handing
/// it the 8-bit buffer every other raster export uses fails outright.
///
/// EXR exists to carry values outside 0..=1 — that is what high dynamic range means — and this product
/// stores 8-bit sRGB, so what is written here is the honest conversion of what we have: each channel
/// divided by 255 into the unit range. The file is a valid EXR and round-trips through any reader; what
/// it cannot do is invent the headroom the format allows and our canvas never held.
fn encode_exr(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let floats: Vec<f32> = pixels
        .iter()
        .map(|value| f32::from(*value) / 255.0)
        .collect();
    let buffer: image::Rgba32FImage =
        image::ImageBuffer::from_raw(width, height, floats).ok_or(FormatError::OutputTooLarge)?;
    let mut bytes = Vec::new();
    buffer
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::OpenExr)
        .map_err(|_| FormatError::UnsupportedFeature("the EXR encoder refused this buffer"))?;
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
        FileFormat::Png
        | FileFormat::WebP
        | FileFormat::Jpeg
        | FileFormat::Tiff
        | FileFormat::Exr
        | FileFormat::Dds
        | FileFormat::Bmp
        | FileFormat::Tga => {
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
                // An indexed document writes a PALETTE png (J.3). Exporting it as RGBA would carry
                // the palette nowhere, which is most of what the mode is for. The indices come from
                // the same nearest-colour search the pixels did, not a second one that could
                // disagree with what is on screen.
                FileFormat::Png if document.color_mode() == crate::ColorMode::Indexed => {
                    let (_, raw_indices) = crate::color_mode::quantize(
                        pixels,
                        document.width() as usize,
                        document.palette(),
                        crate::DitherMode::None,
                    );
                    let alphas: Vec<u8> = pixels.chunks_exact(4).map(|pixel| pixel[3]).collect();
                    // An indexed PNG has NO alpha channel -- colour type 3 carries only a per-entry
                    // `tRNS` -- so transparency survives only if one palette index is dedicated to
                    // it (J.3-c). Without this an indexed export silently turned every transparent
                    // pixel opaque.
                    let (palette, indices) = match crate::color_mode::reserve_transparent_index(
                        document.palette(),
                        &raw_indices,
                        &alphas,
                    ) {
                        Some((palette, transparent)) => (
                            palette,
                            crate::color_mode::remap_indices_for_transparency(
                                &raw_indices,
                                &alphas,
                                transparent,
                            ),
                        ),
                        None => {
                            // Either nothing is transparent, or the palette is full and the alpha
                            // cannot be expressed. The second case is a real loss and is reported;
                            // dropping a visible colour to make room would be worse.
                            if alphas
                                .iter()
                                .any(|alpha| *alpha <= crate::color_mode::INDEXED_ALPHA_THRESHOLD)
                            {
                                warnings.push(FormatWarning::FlattenedAlpha {
                                    matte: Pixel::rgba(0, 0, 0, 255),
                                });
                            }
                            (document.palette().to_vec(), raw_indices)
                        }
                    };
                    crate::anim::export_indexed_png(
                        document.width(),
                        document.height(),
                        &indices,
                        &palette,
                    )?
                }
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
                FileFormat::Tiff => encode_via_image(
                    pixels,
                    document.width(),
                    document.height(),
                    image::ImageFormat::Tiff,
                )?,
                FileFormat::Exr => encode_exr(pixels, document.width(), document.height())?,
                FileFormat::Dds => {
                    // Block compression is lossy: a 4x4 block keeps two endpoints and two bits per
                    // pixel, so anything but a flat block is approximated. Reported rather than implied.
                    if crate::dds::is_lossy_for(document.width(), document.height(), pixels) {
                        warnings.push(FormatWarning::BlockCompressed {
                            fourcc: crate::dds::fourcc_for(pixels),
                        });
                    }
                    crate::dds::encode_dds(document.width(), document.height(), pixels)?
                }
                // Upstream writes 32 bpp for an RGBA image and 24 for RGB
                // (`bmp-export.c`, `GIMP_RGBA_IMAGE` -> `BitsPerPixel = 32`), so BMP carries alpha
                // and is not a lossy container.
                FileFormat::Bmp => encode_via_image(
                    pixels,
                    document.width(),
                    document.height(),
                    image::ImageFormat::Bmp,
                )?,
                // The encoder writes a TGA 1.0 stream, which has NO signature — and our own
                // detector then could not read it back, so export produced a file this product
                // refuses to import. Found by probing the round trip, not by reading.
                //
                // Fixed the faithful way rather than by loosening import: the 26-byte TGA 2.0
                // footer is the format's own mechanism for being identifiable, and upstream's magic
                // rule (`-18&,string,TRUEVISION-XFILE.`) expects exactly it. Both offsets are zero
                // because we write neither an extension area nor a developer directory, which is
                // what upstream's loader treats as "nothing further to read" (`if (offset != 0)`).
                FileFormat::Tga => {
                    let mut bytes = encode_via_image(
                        pixels,
                        document.width(),
                        document.height(),
                        image::ImageFormat::Tga,
                    )?;
                    bytes.extend_from_slice(&0u32.to_le_bytes());
                    bytes.extend_from_slice(&0u32.to_le_bytes());
                    bytes.extend_from_slice(TGA_FOOTER_SIGNATURE);
                    bytes
                }
                // Reached only if a format is added to the arm list ABOVE without an encoder here.
                // This was `unreachable!()` and M.1 reached it: the outer arm listed BMP before
                // this match did, the wildcard swallowed the mismatch, and the export PANICKED at
                // run time instead of failing to compile. A `debug_assert` plus an error is the
                // same trade the caret arms take — loud in tests, survivable in release.
                other => {
                    debug_assert!(
                        false,
                        "no encoder for {other:?} despite being dispatched here"
                    );
                    return Err(
                        FormatError::UnsupportedFeature("no encoder for this format").into(),
                    );
                }
            };
            if bytes.len() > MAX_FORMAT_OUTPUT_BYTES {
                return Err(FormatError::OutputTooLarge.into());
            }
            (
                bytes,
                warnings,
                (format == FileFormat::Jpeg).then_some(options.jpeg_quality),
                // DDS joins JPEG as a lossy container: saying otherwise would invite a caller to treat
                // a block-compressed export as an archival copy.
                !matches!(format, FileFormat::Jpeg | FileFormat::Dds),
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
        FileFormat::Psd => {
            let (bytes, warnings) = crate::psd::export_psd(document, frame, options)?;
            (bytes, warnings, None, true)
        }
        FileFormat::Kra => {
            let (bytes, warnings) = crate::kra::export_kra(document, frame, options)?;
            (bytes, warnings, None, true)
        }
        FileFormat::Xcf => {
            let (bytes, warnings) = crate::xcf::export_xcf(document, frame, options)?;
            (bytes, warnings, None, true)
        }
        FileFormat::Heif | FileFormat::Avif => {
            return Err(FormatError::UnsupportedFeature(
                "HEIF and AVIF export need an HEVC or AV1 encoder",
            )
            .into());
        }
        FileFormat::JpegXl => {
            return Err(FormatError::UnsupportedFeature("JPEG-XL needs an external codec").into());
        }
        FileFormat::Pdf => {
            return Err(FormatError::UnsupportedFeature("PDF export (read-only format)").into());
        }
        FileFormat::Raw => {
            return Err(
                FormatError::UnsupportedFeature("camera raw export (read-only format)").into(),
            );
        }
        FileFormat::Gif => {
            let (bytes, warnings) = crate::anim::export_animated_gif(document)?;
            (bytes, warnings, None, false)
        }
        FileFormat::Apng => {
            let (bytes, warnings) = crate::anim::export_apng(document)?;
            (bytes, warnings, None, true)
        }
        FileFormat::WebpAnim => {
            let (bytes, warnings) = crate::anim::export_animated_webp(document)?;
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
