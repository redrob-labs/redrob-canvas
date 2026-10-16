// SPDX-License-Identifier: MIT OR Apache-2.0

//! Image Trace: raster to vector paths, a thin wrapper over `redrob_vector_trace` so every mode can
//! trace a selection or a layer into paths without depending on the vector editor.

pub use redrob_vector_trace::{
    MAX_PIXELS, Method, Mode, PRESET_NAMES, Palette, TraceError, TraceParams, TraceResult,
    TracedPath, enhanced_preset, preset, presets,
};

use crate::{AiError, Result, pixels::Rgba8Image};

/// Traces `rgba` with the built-in preset `preset` (case-insensitive, see [`PRESET_NAMES`]).
/// An unknown preset name is an error; an empty image gives an empty result.
pub fn vectorize(rgba: &Rgba8Image, preset_name: &str) -> Result<TraceResult> {
    let params = preset(preset_name).ok_or_else(|| {
        AiError::Geometry(format!(
            "Image Trace: no preset named \"{preset_name}\" (one of: {})",
            PRESET_NAMES.join(", ")
        ))
    })?;
    vectorize_with(rgba, &params)
}

/// Traces `rgba` with explicit parameters, refusing an image over [`MAX_PIXELS`].
pub fn vectorize_with(rgba: &Rgba8Image, params: &TraceParams) -> Result<TraceResult> {
    let pixels = u64::from(rgba.width) * u64::from(rgba.height);
    if pixels > MAX_PIXELS {
        return Err(TraceError::TooLarge {
            width: rgba.width,
            height: rgba.height,
            max: MAX_PIXELS >> 20,
        }
        .into());
    }
    if rgba.is_empty() || rgba.data.len() != (pixels as usize) * 4 {
        return Ok(TraceResult::default());
    }
    let raster = redrob_vector_trace::Raster::new(rgba.width, rgba.height, rgba.data.clone());
    Ok(redrob_vector_trace::trace_within(
        &raster,
        params,
        usize::MAX,
    )?)
}

/// The preset names, in panel order.
pub fn preset_names() -> &'static [&'static str] {
    PRESET_NAMES
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn two_colour_8x8_gives_a_path() {
        // A black 4×4 square in the middle of a white 8×8 image.
        let mut data = Vec::new();
        for y in 0..8u32 {
            for x in 0..8u32 {
                let black = (2..6).contains(&x) && (2..6).contains(&y);
                data.extend_from_slice(if black {
                    &[0, 0, 0, 255]
                } else {
                    &[255, 255, 255, 255]
                });
            }
        }
        let img = Rgba8Image::new(8, 8, data).unwrap();
        let result = vectorize(&img, "Black and White Logo").unwrap();
        assert!(!result.paths.is_empty());
        assert!(result.anchor_count() >= 4);
        assert!(!vectorize(&img, "default").unwrap().paths.is_empty());
    }

    #[test]
    fn unknown_preset_and_empty_image() {
        let img = Rgba8Image::filled(2, 2, [0, 0, 0, 255]).unwrap();
        assert!(vectorize(&img, "no such preset").is_err());
        let empty = Rgba8Image::default();
        assert!(vectorize(&empty, "default").unwrap().paths.is_empty());
        assert!(preset_names().contains(&"Flat Logo"));
    }
}
