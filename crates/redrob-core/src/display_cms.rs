// SPDX-License-Identifier: GPL-3.0-or-later

//! Display colour management: monitor profile, soft-proofing, rendering intent (J.5).
//!
//! Re-derived from the reference implementation's colour configuration
//! (`libgimpconfig/gimpcolorconfig.h` and `libgimpconfig/gimpconfigenums.h`, GPL-3.0-or-later),
//! pinned and attributed in `docs/upstream-sources.toml`. Its three management modes, four
//! rendering intents, per-direction black-point compensation, and the gamut-check colour are the
//! shape worth copying.
//!
//! # This is a VIEW transform and nothing else
//!
//! Every function here runs at the display boundary and never touches a document pixel. That is not
//! a detail: the whole purpose of soft-proofing is to show what the image would look like somewhere
//! else WITHOUT changing it, and a colour-managed display that altered the stored pixels would
//! destroy the image it was meant to describe. Upstream keeps this on the display shell as a
//! filter for the same reason.
//!
//! The settings therefore live beside the renderer, not on `Document`. They are a property of the
//! person looking at the image — their monitor, what they are proofing for — so they must not be
//! saved into a file that someone else opens on a different screen.

use serde::{Deserialize, Serialize};

use crate::icc::IccProfile;

/// How much colour management the display applies.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorManagementMode {
    /// Pixels go to the screen as they are stored.
    ///
    /// The default, and deliberately so: without a monitor profile there is nothing to convert TO,
    /// and inventing one would shift every colour on a correctly calibrated screen.
    #[default]
    Off,
    /// Document space → monitor profile.
    Display,
    /// Document space → simulated device → back → monitor profile.
    SoftProof,
}

/// How a colour outside the destination's gamut is handled.
///
/// Upstream's four, from `GimpColorRenderingIntent`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderingIntent {
    /// Compress the whole range so relationships between colours survive.
    Perceptual,
    /// Keep in-gamut colours exactly and clip the rest. The default, as upstream's is.
    #[default]
    RelativeColorimetric,
    Saturation,
    /// Keep in-gamut colours exactly AND keep the source white point, so paper white shows as the
    /// paper's own tint rather than as the monitor's white.
    AbsoluteColorimetric,
}

/// Everything the display transform needs. Not document state — see the module note.
#[derive(Clone, Debug, Default)]
pub struct DisplaySettings {
    pub mode: ColorManagementMode,
    /// The monitor's profile. `None` with a mode other than `Off` means there is nothing to convert
    /// to, and the transform becomes a no-op rather than a guess.
    pub display_profile: Option<IccProfile>,
    /// The device being simulated in `SoftProof`.
    pub simulation_profile: Option<IccProfile>,
    pub display_intent: RenderingIntent,
    pub simulation_intent: RenderingIntent,
    /// Scale black to the destination's black instead of clipping to it.
    pub display_bpc: bool,
    pub simulation_bpc: bool,
    /// Paint colours the simulated device cannot reproduce in `out_of_gamut_color`.
    pub simulation_gamut_check: bool,
    pub out_of_gamut_color: crate::Pixel,
}

impl DisplaySettings {
    /// Whether this configuration would change any pixel.
    ///
    /// Checked before touching the buffer: `Display` with no monitor profile, and `SoftProof` with
    /// no simulation profile, are configurations a UI can easily be in half-way through being set
    /// up, and converting against a profile that is not there is worse than not converting.
    pub fn is_active(&self) -> bool {
        match self.mode {
            ColorManagementMode::Off => false,
            ColorManagementMode::Display => self.display_profile.is_some(),
            ColorManagementMode::SoftProof => self.simulation_profile.is_some(),
        }
    }

    /// Applies the display transform to an 8-bit RGBA buffer in place.
    ///
    /// Alpha is never touched. Coverage is not a colour and has no profile; scaling it by a
    /// transform meant for colour would make a soft edge change opacity when someone picked a
    /// different monitor.
    pub fn apply(&self, pixels: &mut [u8]) {
        if !self.is_active() {
            return;
        }
        for pixel in pixels.chunks_exact_mut(4) {
            // A fully transparent pixel has no visible colour to convert, and its stored RGB is
            // usually zero -- which would come back as the destination's black and then show up the
            // moment anything raised that alpha.
            if pixel[3] == 0 {
                continue;
            }
            let source = [
                f64::from(pixel[0]) / 255.0,
                f64::from(pixel[1]) / 255.0,
                f64::from(pixel[2]) / 255.0,
            ];
            let converted = match self.mode {
                ColorManagementMode::Off => source,
                ColorManagementMode::Display => {
                    let profile = self
                        .display_profile
                        .as_ref()
                        .expect("is_active checked the profile is present");
                    to_device(profile, source, self.display_intent, self.display_bpc)
                }
                ColorManagementMode::SoftProof => {
                    let simulation = self
                        .simulation_profile
                        .as_ref()
                        .expect("is_active checked the profile is present");
                    // Document → simulated device, then BACK to sRGB. The round trip is the whole
                    // point: what returns is what that device could actually reproduce, and the
                    // difference from what went in is the loss being previewed.
                    let device = to_device(
                        simulation,
                        source,
                        self.simulation_intent,
                        self.simulation_bpc,
                    );
                    let back = simulation.to_srgb_unit(device);
                    if self.simulation_gamut_check && out_of_gamut(simulation, source) {
                        [
                            f64::from(self.out_of_gamut_color.r) / 255.0,
                            f64::from(self.out_of_gamut_color.g) / 255.0,
                            f64::from(self.out_of_gamut_color.b) / 255.0,
                        ]
                    } else if let Some(display) = self.display_profile.as_ref() {
                        // The proof is then shown on the actual monitor, which is a second
                        // transform. Skipping it would display the simulation's numbers as if the
                        // screen were sRGB -- the soft proof would be wrong on exactly the screens
                        // that need it.
                        to_device(display, back, self.display_intent, self.display_bpc)
                    } else {
                        back
                    }
                }
            };
            pixel[0] = quantize(converted[0]);
            pixel[1] = quantize(converted[1]);
            pixel[2] = quantize(converted[2]);
        }
    }
}

/// sRGB unit triple → a profile's device space, under one intent.
///
/// # What the intents do here, and what they cannot
///
/// `RelativeColorimetric` keeps in-gamut colours exactly and clips the rest — the matrix transform
/// with its white-point adaptation, which is what `IccProfile` already performs in the other
/// direction. `AbsoluteColorimetric` skips the white-point adaptation, so the source white survives
/// as the destination's tinted white instead of being mapped onto it.
///
/// `Perceptual` and `Saturation` are relative colorimetric with black-point compensation forced on.
/// This is stated plainly rather than dressed up: both intents are defined by LOOKUP TABLES that a
/// profile may carry, and `IccProfile` here is a matrix-and-curve ("matrix-shaper") profile, which
/// has no such tables. A colour library given a matrix-shaper profile does the same fallback for
/// the same reason — there is nothing else it could do. Honouring them properly needs table-based
/// profile support, which is a different piece of work and is filed as one.
fn to_device(
    profile: &IccProfile,
    srgb: [f64; 3],
    intent: RenderingIntent,
    black_point_compensation: bool,
) -> [f64; 3] {
    let adapt_white = intent != RenderingIntent::AbsoluteColorimetric;
    let mut device = profile.from_srgb_unit(srgb, adapt_white);
    let compensate = black_point_compensation
        || matches!(
            intent,
            RenderingIntent::Perceptual | RenderingIntent::Saturation
        );
    if compensate {
        let black = profile.from_srgb_unit([0.0, 0.0, 0.0], adapt_white);
        for channel in 0..3 {
            // Scale the range so the source's black lands on the destination's black rather than
            // being clipped to it. Clipping is what turns shadow detail into one flat patch.
            let floor = black[channel].clamp(0.0, 0.999);
            device[channel] = floor + device[channel] * (1.0 - floor);
        }
    }
    [
        device[0].clamp(0.0, 1.0),
        device[1].clamp(0.0, 1.0),
        device[2].clamp(0.0, 1.0),
    ]
}

/// Whether a colour falls outside what `profile` can reproduce.
///
/// Measured by the round trip rather than by a gamut boundary: convert in, convert back, and see
/// whether the colour survived. A colour the device cannot hold gets clamped on the way in, so it
/// comes back different — and the size of that difference is exactly the error being flagged.
///
/// The threshold is 1/255: smaller than one step of the 8-bit buffer this is shown in, so the check
/// cannot flag a colour that would have displayed identically anyway.
fn out_of_gamut(profile: &IccProfile, srgb: [f64; 3]) -> bool {
    let device = profile.from_srgb_unit(srgb, true);
    let clamped = [
        device[0].clamp(0.0, 1.0),
        device[1].clamp(0.0, 1.0),
        device[2].clamp(0.0, 1.0),
    ];
    let back = profile.to_srgb_unit(clamped);
    (0..3).any(|channel| (back[channel] - srgb[channel]).abs() > 1.0 / 255.0)
}

fn quantize(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
