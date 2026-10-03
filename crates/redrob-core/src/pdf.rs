// SPDX-License-Identifier: GPL-3.0-or-later

//! PDF import (H.15): the first page's embedded image.
//!
//! What this is and is NOT. A PDF page is a program — a content stream of drawing operators over
//! fonts, shadings and transparency groups — and running it is a graphics engine, not a file reader.
//! That engine is not attempted here, and saying so is the point: a half-written interpreter renders
//! *something* for every file, and a page that silently loses its text looks like a bug in this
//! product rather than a feature it never had.
//!
//! What IS read is the case this product is actually asked for: a page whose content is ONE embedded
//! image. A scan, a photo export, a flattened artboard. For those the page is a wrapper, the image is
//! the document, and reaching it needs object parsing rather than rasterisation.
//!
//! Anything else is refused by name, so the error distinguishes "this PDF is drawn, not scanned" from
//! "this PDF is broken".

use crate::{FormatError, Result};

/// Decodes the first page's embedded image to `(width, height, rgba)`.
pub(crate) fn decode_pdf(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let document =
        lopdf::Document::load_mem(bytes).map_err(|_| FormatError::Malformed("PDF structure"))?;
    // Pages are keyed by page NUMBER, so the lowest key is the first page -- not the first object in
    // the file, which is an arbitrary order that a linearised PDF deliberately scrambles.
    let (_, page_id) = document
        .get_pages()
        .into_iter()
        .next()
        .ok_or(FormatError::Malformed("PDF has no pages"))?;

    let images = document
        .get_page_images(page_id)
        .map_err(|_| FormatError::Malformed("PDF page resources"))?;
    // The LARGEST image, not the first: a scanned page often carries a thumbnail or a stamp beside the
    // scan itself, and the first one in the resource dictionary is in no particular order.
    let image = images
        .into_iter()
        .max_by_key(|image| image.width.saturating_mul(image.height))
        .ok_or(FormatError::UnsupportedFeature(
            "this PDF page is drawn rather than a single image; rendering it needs a graphics engine",
        ))?;

    let width =
        u32::try_from(image.width).map_err(|_| FormatError::Malformed("PDF image width"))?;
    let height =
        u32::try_from(image.height).map_err(|_| FormatError::Malformed("PDF image height"))?;
    if width == 0
        || height == 0
        || width > crate::document::MAX_DIMENSION
        || height > crate::document::MAX_DIMENSION
    {
        return Err(FormatError::Malformed("PDF image dimensions out of range").into());
    }

    // The filters still on the stream decide what `content` actually is. A PDF image is not a format,
    // it is samples plus a filter chain, so the chain is what must be matched -- an image whose filter
    // we ignore decodes as noise at exactly the right dimensions.
    let filters: Vec<String> = image.filters.clone().unwrap_or_default();
    let last = filters.last().map(String::as_str);
    match last {
        // DCTDecode is a complete JPEG bitstream, so it goes to the decoder this product already has.
        Some("DCTDecode") => {
            let decoded =
                image::load_from_memory_with_format(image.content, image::ImageFormat::Jpeg)
                    .map_err(|_| FormatError::Malformed("PDF JPEG image"))?;
            let rgba = decoded.to_rgba8();
            Ok((rgba.width(), rgba.height(), rgba.into_raw()))
        }
        // JPXDecode is JPEG 2000: a different codec again, and not one this product has anywhere.
        Some("JPXDecode") => Err(FormatError::UnsupportedFeature(
            "this PDF embeds a JPEG 2000 image, which needs a separate codec",
        )
        .into()),
        // No filter left means `content` is raw samples, and the colour space says how to read them.
        None => raw_samples(&image, width, height),
        Some(_) => Err(FormatError::UnsupportedFeature(
            "this PDF image uses a filter chain this product does not decode",
        )
        .into()),
    }
}

/// Expands raw 8-bit samples to RGBA using the image's declared colour space.
///
/// Only the two colour spaces a scan actually uses are handled. An indexed or CMYK PDF image would
/// need its palette or ink conversion, and guessing either produces a picture rather than an error.
fn raw_samples(
    image: &lopdf::xobject::PdfImage<'_>,
    width: u32,
    height: u32,
) -> Result<(u32, u32, Vec<u8>)> {
    if image.bits_per_component.unwrap_or(8) != 8 {
        return Err(
            FormatError::UnsupportedFeature("this PDF image is not 8 bits per component").into(),
        );
    }
    let count = width as usize * height as usize;
    let space = image.color_space.as_deref().unwrap_or("DeviceRGB");
    let channels = match space {
        "DeviceGray" | "CalGray" | "G" => 1,
        "DeviceRGB" | "CalRGB" | "RGB" => 3,
        _ => {
            return Err(FormatError::UnsupportedFeature(
                "this PDF image uses a colour space this product does not convert",
            )
            .into());
        }
    };
    if image.content.len() < count * channels {
        return Err(FormatError::Malformed("PDF image data truncated").into());
    }
    let mut rgba = Vec::with_capacity(count * 4);
    for index in 0..count {
        let at = index * channels;
        let (r, g, b) = if channels == 1 {
            let grey = image.content[at];
            (grey, grey, grey)
        } else {
            (
                image.content[at],
                image.content[at + 1],
                image.content[at + 2],
            )
        };
        rgba.extend_from_slice(&[r, g, b, 255]);
    }
    Ok((width, height, rgba))
}
