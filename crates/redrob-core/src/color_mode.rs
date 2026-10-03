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

/// The ordered-dither threshold matrix: a Bayer matrix of order 2, DERIVED rather than written out.
///
/// It used to be sixteen literals here. `gegl:bayer-matrix` (K.6, cycle 79) needed the same object
/// for a different purpose — rendering it as a visible pattern rather than consuming it as a
/// threshold — and two copies of a defined object can drift apart. So both now come from
/// `filters::bayer_matrix`, which makes them equal by construction rather than by a test that only
/// catches drift once someone runs it.
///
/// This also gave the generator its check. The literals were written for the dither long before the
/// filter existed, and only the correct BLOCK recursion reproduces them: an interleaved variant
/// still yields a permutation of 0..15 and still tiles, so neither of those properties can tell the
/// two recursions apart.
fn ordered_matrix() -> [[f32; 4]; 4] {
    let matrix = crate::filters::bayer_matrix(2);
    let mut out = [[0.0f32; 4]; 4];
    for (y, row) in matrix.iter().enumerate() {
        for (x, &value) in row.iter().enumerate() {
            out[y][x] = value as f32;
        }
    }
    out
}

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
    // Computed once rather than per pixel: it is the same sixteen numbers every time.
    let ordered = ordered_matrix();

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
                    (ordered[y % 4][x % 4] / 16.0 - 0.5) * 32.0
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

/// The alpha above which a pixel counts as opaque when an indexed file is written.
///
/// 127, as upstream's PNG plug-in uses (`find_unused_ia_color`, `data[1] > 127`). Palette
/// transparency is per-ENTRY and all-or-nothing, so a soft edge cannot survive: every pixel must be
/// called opaque or not, and half way is the only threshold that is not a preference.
pub const INDEXED_ALPHA_THRESHOLD: u8 = 127;

/// Reserves one palette index for transparency, as an indexed file needs.
///
/// Returns the palette to write and which index is the transparent one, or `None` when the image
/// has nothing transparent in it (no entry is needed) or the palette is full and no entry can be
/// spared (the transparency cannot be expressed and the caller must say so).
///
/// Re-derived from upstream's PNG export path (`plug-ins/common/file-png.c`, `find_unused_ia_color`
/// and `respin_cmap`, GPL-3.0-or-later). Its rule, in order:
///
/// 1. Find an index that no OPAQUE pixel uses. Such an entry is free: reusing it costs no colour
///    anybody can see. This is the case that matters, because quantizing already assigns
///    transparent pixels somewhere.
/// 2. Otherwise append a new entry, if the palette is under 256.
/// 3. Otherwise give up. A 256-colour palette with every entry carrying visible pixels has nowhere
///    to put transparency, and silently dropping a colour would be worse than dropping the alpha.
///
/// Upstream then swaps the chosen index to 0 and remaps the palette, which is what lets `tRNS` be a
/// single byte instead of a full 256-byte table. Done here too — the swap is the only reason the
/// chunk is one byte, so skipping it would quietly write 250 pointless bytes into every file.
pub fn reserve_transparent_index(
    palette: &[Pixel],
    indices: &[u8],
    alphas: &[u8],
) -> Option<(Vec<Pixel>, u8)> {
    if !alphas.iter().any(|alpha| *alpha <= INDEXED_ALPHA_THRESHOLD) {
        return None;
    }
    let mut used_by_opaque = [false; MAX_PALETTE_COLORS];
    for (index, alpha) in indices.iter().zip(alphas.iter()) {
        if *alpha > INDEXED_ALPHA_THRESHOLD {
            used_by_opaque[*index as usize] = true;
        }
    }
    let mut palette = palette.to_vec();
    let transparent = match (0..palette.len()).find(|index| !used_by_opaque[*index]) {
        Some(index) => index,
        None if palette.len() < MAX_PALETTE_COLORS => {
            // The appended entry's colour is never seen -- every pixel pointing at it is
            // transparent -- so it is black rather than a guess that implies meaning.
            palette.push(Pixel::rgba(0, 0, 0, 0));
            palette.len() - 1
        }
        None => return None,
    };
    // Swap the transparent entry to index 0 so `tRNS` is one byte.
    palette.swap(0, transparent);
    // And mark entry 0 transparent. A REUSED entry still carries alpha 255 from whatever colour it
    // held, and the writer decides whether to emit `tRNS` by looking at the entries' alpha -- so
    // without this the chunk is omitted and the swap accomplishes nothing. Found by the round-trip
    // test, not by reading.
    palette[0].a = 0;
    Some((palette, transparent as u8))
}

/// Rewrites per-pixel indices for a palette whose transparent entry was swapped to 0.
///
/// Every pixel at or below the alpha threshold points at 0; the two swapped entries trade places
/// for everything else. Returns the rewritten indices.
pub fn remap_indices_for_transparency(indices: &[u8], alphas: &[u8], transparent: u8) -> Vec<u8> {
    indices
        .iter()
        .zip(alphas.iter())
        .map(|(index, alpha)| {
            if *alpha <= INDEXED_ALPHA_THRESHOLD {
                0
            } else if *index == 0 {
                transparent
            } else if *index == transparent {
                0
            } else {
                *index
            }
        })
        .collect()
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
