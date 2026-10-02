// SPDX-License-Identifier: GPL-3.0-or-later

//! JPEG XL import (H.13), decoded by the pure-Rust `jxl-oxide`.
//!
//! The codec itself is NOT re-written here, and that is a deliberate line. JPEG XL is a modern
//! transform codec whose decoder is tens of thousands of lines; hand-porting one would be a worse
//! decoder facing untrusted input, which is the one place a weaker implementation costs more than it
//! saves. What this module owns is the part that was actually missing: the bridge from a decoded frame
//! to this product's 8-bit RGBA rasters.
//!
//! Three conversions happen here, and each is a decision:
//!
//! 1. The decoder is asked for sRGB output. A JPEG XL file may be authored in any colour encoding,
//!    including wide-gamut and HDR; taking the file's own channels and calling them sRGB would open a
//!    wide-gamut image with its colours visibly wrong rather than merely clipped.
//! 2. Samples arrive as f32 in 0..=1 (after the sRGB request) and are quantised to 8 bits. Values
//!    outside that range exist in HDR files and are CLAMPED, since this product's rasters have no room
//!    for them.
//! 3. Channel counts vary: grey, grey+alpha, RGB, RGBA. They are expanded rather than assumed, because
//!    a greyscale JPEG XL read as RGB would take three successive pixels for one.
//!
//! Animation is not read: only the first keyframe is decoded. A JPEG XL animation would need the
//! timeline, which is a separate pass.

use crate::{FormatError, Result};

/// Decodes a JPEG XL file to `(width, height, rgba)`.
pub(crate) fn decode_jxl(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let mut image = jxl_oxide::JxlImage::read_with_defaults(bytes)
        .map_err(|_| FormatError::Malformed("JPEG XL bitstream"))?;
    // Ask for sRGB rather than converting afterwards: the decoder owns the colour transform, and it
    // knows the file's own encoding, which we would otherwise have to re-derive from its ICC profile.
    image.request_color_encoding(jxl_oxide::EnumColourEncoding::srgb(
        jxl_oxide::RenderingIntent::Perceptual,
    ));

    let width = image.width();
    let height = image.height();
    if width == 0
        || height == 0
        || width > crate::document::MAX_DIMENSION
        || height > crate::document::MAX_DIMENSION
    {
        return Err(FormatError::Malformed("JPEG XL dimensions out of range").into());
    }
    if image.num_loaded_keyframes() == 0 {
        return Err(FormatError::Malformed("JPEG XL has no frames").into());
    }

    let render = image
        .render_frame(0)
        .map_err(|_| FormatError::Malformed("JPEG XL frame render"))?;
    // `stream` carries colour plus black and alpha, with the file's orientation already applied --
    // which matters because an orientation left unapplied rotates the image without any error.
    let stream = render.stream();
    let channels = stream.channels() as usize;
    let stream_width = stream.width() as usize;
    let stream_height = stream.height() as usize;
    let mut samples = vec![0f32; stream_width * stream_height * channels];
    let mut stream = stream;
    stream.write_to_buffer(&mut samples);

    let mut rgba = Vec::with_capacity(stream_width * stream_height * 4);
    let quantise = |value: f32| -> u8 {
        if value.is_finite() {
            (value.clamp(0.0, 1.0) * 255.0).round() as u8
        } else {
            0
        }
    };
    for pixel in samples.chunks_exact(channels) {
        let (r, g, b, a) = match channels {
            1 => (pixel[0], pixel[0], pixel[0], 1.0),
            2 => (pixel[0], pixel[0], pixel[0], pixel[1]),
            3 => (pixel[0], pixel[1], pixel[2], 1.0),
            _ => (pixel[0], pixel[1], pixel[2], pixel[3]),
        };
        rgba.push(quantise(r));
        rgba.push(quantise(g));
        rgba.push(quantise(b));
        rgba.push(quantise(a));
    }
    Ok((stream_width as u32, stream_height as u32, rgba))
}
