// SPDX-License-Identifier: GPL-3.0-or-later

//! Document colour modes: RGB, greyscale, and indexed with a palette.
//!
//! J.3. The product could already READ an indexed PSD or XCF by converting it on import; it could
//! not author one. A mode is what makes "this image has 16 colours and here they are" a property of
//! the document rather than an accident of its pixels.
//!
//! Re-derived from the behaviour of the reference implementation's indexed conversion
//! (`app/core/gimpimage-convert-indexed.c`, GPL-3.0-or-later), pinned and attributed in
//! `docs/upstream-sources.toml`. Its palette choices (generate / web / mono / custom) and dither
//! choices (none / Floyd–Steinberg / ordered) are the useful shape, and the two hard parts are the
//! palette search and where the dither error goes.
//!
//! # What a mode is here, and what it is not
//!
//! The mode is a declared CONSTRAINT plus a palette; the pixels stay RGBA, snapped to that palette.
//! Upstream stores one-byte indices instead, which saves memory and makes the constraint impossible
//! to violate.
//!
//! Storing indices was considered and rejected for now: it is a third storage layout beside the two
//! the precision work just introduced, it is meaningless in combination with 16-bit samples, and it
//! would have to be threaded through undo, every filter and the renderer. The cost of this choice is
//! real and is NOT hidden: a later edit can introduce an off-palette colour, so the constraint is
//! enforced at conversion and again at export rather than continuously. That gap is recorded in the
//! backlog as its own item rather than left for someone to discover.

use serde::{Deserialize, Serialize};

use crate::Pixel;

/// The largest palette an indexed document may carry.
///
/// 256, because that is what one byte of index addresses and what every indexed file format this
/// product reads or writes can express.
pub const MAX_PALETTE_COLORS: usize = 256;

/// How a document's colour is constrained.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorMode {
    #[default]
    Rgb,
    /// Every pixel is a neutral grey. Stored as RGBA with equal channels, so the renderer and every
    /// filter need no special case.
    Grayscale,
    /// Every pixel is one of the document's palette colours.
    Indexed,
}

/// Which palette an indexed conversion should use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaletteChoice {
    /// Build a palette from the image's own colours, up to `max_colors`.
    Generate { max_colors: usize },
    /// The 6x6x6 cube: a fixed 216-colour palette that every display could show.
    Web,
    /// Black and white only.
    Mono,
    /// Colours the caller supplies.
    Custom { colors: Vec<Pixel> },
}

/// How the error from snapping a colour to the palette is spread.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DitherMode {
    /// Snap each pixel to its nearest palette entry and discard the error. Flat areas stay flat and
    /// gradients band.
    #[default]
    None,
    /// Floyd–Steinberg: push the error into the neighbours not yet visited. Gradients survive as
    /// texture rather than as bands.
    FloydSteinberg,
    /// A fixed 4x4 threshold matrix. Deterministic per pixel and position-stable, so an animation
    /// does not shimmer between frames the way error diffusion can.
    Ordered,
}

/// Builds the palette a choice describes.
///
/// `Generate` uses median cut: repeatedly split the colour box with the longest side at its median
/// until there are enough boxes, then take each box's mean. The obvious alternative — take the N
/// most frequent colours — is what makes a photograph convert to twenty shades of sky and nothing
/// else, because frequency has no notion of coverage of the colour space.
pub fn build_palette(pixels: &[u8], choice: &PaletteChoice) -> Vec<Pixel> {
    match choice {
        PaletteChoice::Mono => vec![Pixel::rgba(0, 0, 0, 255), Pixel::rgba(255, 255, 255, 255)],
        PaletteChoice::Web => {
            let mut palette = Vec::with_capacity(216);
            for r in 0..6 {
                for g in 0..6 {
                    for b in 0..6 {
                        palette.push(Pixel::rgba(r * 51, g * 51, b * 51, 255));
                    }
                }
            }
            palette
        }
        PaletteChoice::Custom { colors } => colors.clone(),
        PaletteChoice::Generate { max_colors } => {
            let target = (*max_colors).clamp(1, MAX_PALETTE_COLORS);
            // Fully transparent pixels carry no colour worth a palette slot, and including them
            // drags every box toward black.
            let samples: Vec<[u8; 3]> = pixels
                .chunks_exact(4)
                .filter(|pixel| pixel[3] > 0)
                .map(|pixel| [pixel[0], pixel[1], pixel[2]])
                .collect();
            if samples.is_empty() {
                return vec![Pixel::rgba(0, 0, 0, 255)];
            }
            median_cut(samples, target)
        }
    }
}

/// Median cut: split the widest box at its median until `target` boxes exist.
fn median_cut(samples: Vec<[u8; 3]>, target: usize) -> Vec<Pixel> {
    let mut boxes = vec![samples];
    while boxes.len() < target {
        // Split the box with the longest channel range. Splitting the LARGEST box by count instead
        // would keep subdividing a big flat area that needs one colour.
        let Some((index, channel)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, box_samples)| box_samples.len() > 1)
            .map(|(index, box_samples)| {
                let (channel, range) = widest_channel(box_samples);
                (index, channel, range)
            })
            .max_by_key(|(_, _, range)| *range)
            .map(|(index, channel, _)| (index, channel))
        else {
            // Every box holds one colour; there is nothing left to split.
            break;
        };
        let mut box_samples = boxes.swap_remove(index);
        box_samples.sort_unstable_by_key(|sample| sample[channel]);
        let half = box_samples.len() / 2;
        let upper = box_samples.split_off(half);
        boxes.push(box_samples);
        boxes.push(upper);
    }
    boxes
        .into_iter()
        .filter(|box_samples| !box_samples.is_empty())
        .map(|box_samples| {
            let count = box_samples.len() as u32;
            let mut sum = [0u32; 3];
            for sample in &box_samples {
                for (slot, value) in sum.iter_mut().zip(sample.iter()) {
                    *slot += u32::from(*value);
                }
            }
            Pixel::rgba(
                (sum[0] / count) as u8,
                (sum[1] / count) as u8,
                (sum[2] / count) as u8,
                255,
            )
        })
        .collect()
}

fn widest_channel(samples: &[[u8; 3]]) -> (usize, u8) {
    let mut widest = (0usize, 0u8);
    for channel in 0..3 {
        let mut low = u8::MAX;
        let mut high = 0u8;
        for sample in samples {
            low = low.min(sample[channel]);
            high = high.max(sample[channel]);
        }
        let range = high - low;
        if range >= widest.1 {
            widest = (channel, range);
        }
    }
    widest
}

/// The palette index nearest to one colour, by squared distance in RGB.
///
/// Squared distance, not the sum of absolute differences: the latter treats a colour that is far
/// off in one channel as equal to one slightly off in three, which picks visibly wrong entries at
/// the edges of a small palette.
pub fn nearest_index(palette: &[Pixel], color: [i32; 3]) -> usize {
    let mut best = 0usize;
    let mut best_distance = i32::MAX;
    for (index, entry) in palette.iter().enumerate() {
        let dr = color[0] - i32::from(entry.r);
        let dg = color[1] - i32::from(entry.g);
        let db = color[2] - i32::from(entry.b);
        let distance = dr * dr + dg * dg + db * db;
        if distance < best_distance {
            best_distance = distance;
            best = index;
        }
    }
    best
}

/// A fixed 4x4 ordered-dither threshold matrix, scaled to -0.5..0.5 of a palette step.
const ORDERED_MATRIX: [[f32; 4]; 4] = [
    [0.0, 8.0, 2.0, 10.0],
    [12.0, 4.0, 14.0, 6.0],
    [3.0, 11.0, 1.0, 9.0],
    [15.0, 7.0, 13.0, 5.0],
];

/// Snaps an RGBA buffer onto `palette`, spreading the error as `dither` says.
///
/// Returns the per-pixel palette indices alongside the snapped pixels, because the indices are what
/// an indexed FILE needs and recomputing them afterwards would be a second nearest-colour search
/// that could disagree with this one.
///
/// Alpha is carried through untouched. A palette entry's own alpha is not consulted: coverage is not
/// one of the colours being chosen between, and snapping it would make a soft edge jump to opaque.
pub fn quantize(
    pixels: &[u8],
    width: usize,
    palette: &[Pixel],
    dither: DitherMode,
) -> (Vec<u8>, Vec<u8>) {
    let count = pixels.len() / 4;
    let mut out = vec![0u8; pixels.len()];
    let mut indices = vec![0u8; count];
    if palette.is_empty() {
        return (out, indices);
    }
    // Error diffusion needs somewhere to accumulate fractional error per channel. Held as f32 for
    // the whole image rather than one row, because the error travels DOWN as well as right.
    let mut error = vec![0.0f32; count * 3];

    for index in 0..count {
        // A fully transparent pixel has no colour to choose between, and the enforcement path
        // (`Document::enforce_palette`) skips it for that reason. Writing a palette colour under
        // zero alpha here as well would make conversion and later edits disagree about the same
        // pixel — found by the test that pins the edit-time behaviour.
        //
        // Its INDEX is left at 0. That is not a transparent colour: PNG colour type 3 has no alpha
        // channel, only a per-entry `tRNS`, so an indexed export cannot currently express
        // transparency at all. Recorded in the backlog rather than papered over here.
        if pixels[index * 4 + 3] == 0 {
            out[index * 4 + 3] = 0;
            continue;
        }
        let x = index % width;
        let y = index / width;
        let mut wanted = [0i32; 3];
        for channel in 0..3 {
            let base = f32::from(pixels[index * 4 + channel]);
            let nudge = match dither {
                DitherMode::None => 0.0,
                DitherMode::FloydSteinberg => error[index * 3 + channel],
                DitherMode::Ordered => {
                    // The matrix shifts the value before the search, so neighbouring pixels of the
                    // same colour land on different entries and read as a blend.
                    (ORDERED_MATRIX[y % 4][x % 4] / 16.0 - 0.5) * 32.0
                }
            };
            wanted[channel] = (base + nudge).round().clamp(0.0, 255.0) as i32;
        }
        let chosen = nearest_index(palette, wanted);
        let entry = palette[chosen];
        indices[index] = chosen as u8;
        out[index * 4] = entry.r;
        out[index * 4 + 1] = entry.g;
        out[index * 4 + 2] = entry.b;
        out[index * 4 + 3] = pixels[index * 4 + 3];

        if dither == DitherMode::FloydSteinberg {
            let entry_channels = [entry.r, entry.g, entry.b];
            for channel in 0..3 {
                let residual = wanted[channel] as f32 - f32::from(entry_channels[channel]);
                // The classic 7/16, 3/16, 5/16, 1/16 split, pushed only into pixels NOT yet visited.
                // Pushing error backwards would feed it into a decision already made and make the
                // pass depend on its own output in an order nobody can reproduce.
                let mut spread = |tx: usize, ty: usize, weight: f32| {
                    if tx >= width || ty * width + tx >= count {
                        return;
                    }
                    error[(ty * width + tx) * 3 + channel] += residual * weight;
                };
                spread(x + 1, y, 7.0 / 16.0);
                if x > 0 {
                    spread(x - 1, y + 1, 3.0 / 16.0);
                }
                spread(x, y + 1, 5.0 / 16.0);
                spread(x + 1, y + 1, 1.0 / 16.0);
            }
        }
    }
    (out, indices)
}

/// Converts an RGBA buffer to neutral grey, in place.
///
/// Rec. 709 luma, the same weights the filters use. Alpha untouched.
pub fn to_grayscale(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let grey = (0.2126 * f32::from(pixel[0])
            + 0.7152 * f32::from(pixel[1])
            + 0.0722 * f32::from(pixel[2]))
        .round()
        .clamp(0.0, 255.0) as u8;
        pixel[0] = grey;
        pixel[1] = grey;
        pixel[2] = grey;
    }
}
