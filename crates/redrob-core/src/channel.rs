// SPDX-License-Identifier: GPL-3.0-or-later

//! Named greyscale channels: stored selections the user can keep, see and reuse.
//!
//! J.2a. A channel is a full-canvas coverage mask with a name, plus how it is DISPLAYED — a colour,
//! an opacity and which side of the mask the overlay paints. It is not a layer: it carries no colour
//! of its own, and compositing it does not blend with the image, it tints it.
//!
//! Re-derived from the behaviour of the reference implementation's channel drawable
//! (`app/core/gimpchannel.c`, GPL-3.0-or-later), which this repository pins and attributes in
//! `docs/upstream-sources.toml`. Two things from its design are load-bearing and easy to get wrong:
//!
//! - The display colour **carries the opacity**. There, a channel's colour is a `GeglColor` whose
//!   alpha IS the overlay strength. Splitting them into two unrelated fields invites a UI that sets
//!   one and leaves the other, so they are stored apart here but the renderer multiplies both and
//!   the test pins that it does.
//! - `show_masked` decides whether the overlay marks the SELECTED area or the masked-out one. The
//!   same channel with the flag flipped is the photographic negative of itself on screen, and a
//!   reader who assumes one convention sees a correct implementation as inverted.
//!
//! # One byte per pixel, at every document precision
//!
//! Coverage is not colour. Sixteen bits of "how selected is this pixel" buys nothing a user can see
//! or act on, and the document's masks are already byte-per-pixel, so a channel matches them rather
//! than the colour samples next to it.

use serde::{Deserialize, Serialize};

use crate::{Pixel, RasterBytes};

/// How many channels one document may hold.
///
/// A bound rather than no bound because each is a full-canvas buffer: at 8000x8000 a single channel
/// is 64 MB, so an unbounded list is an out-of-memory the user cannot see coming.
pub const MAX_CHANNELS: usize = 32;

/// A named coverage mask stored with the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    id: ChannelId,
    name: String,
    /// One byte of coverage per pixel. 0 is outside, 255 is fully inside.
    pixels: RasterBytes,
    visible: bool,
    /// Overlay strength, 0..=1, multiplied with the display colour's own alpha.
    opacity: f32,
    /// The colour the overlay is painted in. Its alpha participates: see the module note.
    color: Pixel,
    /// `true` paints the masked-out area, `false` paints the selected area.
    ///
    /// Defaults to `true`, which is the convention a user expects from a stored selection: the
    /// overlay shows what is being HELD BACK, so the subject stays visible underneath.
    show_masked: bool,
}

/// Stable identity for a channel, independent of its position in the list.
pub type ChannelId = uuid::Uuid;

/// The name the quick-mask channel is given (J.2b).
///
/// Named rather than anonymous because the channel is REAL while the mode is on — it is in the
/// list, it can be hidden, recoloured and renamed like any other. The name is what tells a user
/// which of their channels the mode is currently editing.
pub const QUICK_MASK_NAME: &str = "Quick Mask";

/// A colour's perceptual brightness, as the coverage value painting it means (J.2b).
///
/// Rec. 709 weights, the same ones the filters use for luma: a channel has nowhere to put a hue, so
/// a stroke's colour has to be read for how BRIGHT it is. White paints full coverage, black paints
/// none. Using the plain mean of the channels instead would make a saturated blue paint roughly as
/// much mask as a mid grey, which is not what a user picking blue intends either way.
pub fn luminance_of(color: Pixel) -> u8 {
    (0.2126 * f32::from(color.r) + 0.7152 * f32::from(color.g) + 0.0722 * f32::from(color.b))
        .round()
        .clamp(0.0, 255.0) as u8
}

impl Channel {
    /// A channel with the given coverage.
    pub(crate) fn new(id: ChannelId, name: String, pixels: Vec<u8>) -> Self {
        Self {
            id,
            name,
            pixels: RasterBytes::new(pixels),
            visible: true,
            opacity: 1.0,
            // Red at full alpha, the long-standing convention for a selection overlay.
            color: Pixel::rgba(255, 0, 0, 255),
            show_masked: true,
        }
    }

    pub fn id(&self) -> ChannelId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn pixels(&self) -> &[u8] {
        self.pixels.as_slice()
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    pub fn color(&self) -> Pixel {
        self.color
    }

    pub fn shows_masked(&self) -> bool {
        self.show_masked
    }

    /// Coverage at one pixel, 0..=255. Out of bounds reads as 0 rather than panicking: the renderer
    /// addresses by absolute index and a clipped region must not be able to crash a frame.
    pub fn coverage(&self, width: u32, x: u32, y: u32) -> u8 {
        let index = y as usize * width as usize + x as usize;
        self.pixels.as_slice().get(index).copied().unwrap_or(0)
    }

    pub(crate) fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub(crate) fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity;
    }

    pub(crate) fn set_color(&mut self, color: Pixel) {
        self.color = color;
    }

    pub(crate) fn set_show_masked(&mut self, show_masked: bool) {
        self.show_masked = show_masked;
    }

    pub(crate) fn set_name(&mut self, name: String) {
        self.name = name;
    }

    /// Replaces the channel's coverage wholesale.
    ///
    /// Takes the buffer by value and does NOT check its length: the document's validator measures
    /// every channel against the pixel count, so a wrong length is caught there rather than in two
    /// places that could disagree.
    pub(crate) fn replace_pixels(&mut self, pixels: Vec<u8>) {
        self.pixels = RasterBytes::new(pixels);
    }

    /// The effective overlay coverage at one pixel, 0..=1.
    ///
    /// Folds together the three things that can hide a channel — the stored coverage, the channel's
    /// opacity and the display colour's alpha — and applies `show_masked`. Kept here rather than in
    /// the renderer so there is one answer to "how much does this channel show", and the test can
    /// ask for it without rendering a frame.
    pub fn overlay_coverage(&self, width: u32, x: u32, y: u32) -> f32 {
        let stored = f32::from(self.coverage(width, x, y)) / 255.0;
        // Inverted for the masked side: the overlay marks what is held back, so a fully selected
        // pixel shows nothing.
        let side = if self.show_masked {
            1.0 - stored
        } else {
            stored
        };
        side * self.opacity.clamp(0.0, 1.0) * (f32::from(self.color.a) / 255.0)
    }
}
