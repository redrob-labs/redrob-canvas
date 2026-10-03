// SPDX-License-Identifier: GPL-3.0-or-later

//! A scene-referred working buffer: linear-light f32 RGBA whose values may sit OUTSIDE 0..=1 (H.19).
//!
//! Re-derived from how GEGL and Krita hold a working space — linear float, not display-encoded bytes —
//! rather than from any one function.
//!
//! ## What this is for, and what it honestly is not
//!
//! This product's rasters are 8-bit sRGB, and that is a decision in the document model, the undo
//! history, the renderer and the FFI layout. Changing it is not a module, it is a different product.
//! So this type does NOT make the document wide-gamut. What it does is give the places that COMPOSE
//! pixels a space where composition is correct, and quantise once at the boundary instead of after
//! every step.
//!
//! Two concrete wrongs it fixes where it is used:
//!
//! 1. **Blending display-encoded bytes is not blending light.** Half of black and half of white is a
//!    mid GREY in light, which sRGB encodes as about 188 — not 128. Averaging the bytes gives 128,
//!    which is visibly darker than the real mixture, and it is the reason a half-strength effect looks
//!    heavier than it should.
//! 2. **Every 8-bit hop rounds.** A chain of operations that stores bytes between nodes quantises at
//!    each one, so four half-strength passes drift from four exact ones. Holding the chain in f32 and
//!    rounding once removes that drift entirely.
//!
//! Values above 1.0 are kept rather than clamped while in this form, because an intermediate result
//! that overshoots and comes back — a blur over a bright highlight, a gain followed by a reduction --
//! should survive the trip. The clamp happens at the one place the buffer becomes bytes again.
//!
//! ## Still in byte space, deliberately
//!
//! Named so the boundary is visible rather than discovered: the renderer's layer compositing, the
//! individual filters, layer styles, and every file format. Each of those is a signature change across
//! the crate, and the ones that touch the renderer change every pixel this product has ever produced —
//! that is a pass with a build and a golden-image comparison behind it, not a module.

/// Linear-light RGBA, one f32 per channel, alpha NOT premultiplied.
///
/// Alpha is kept as-is rather than linearised: it is coverage, not light, and a transfer function
/// applied to it darkens edges (the same mistake the ICC path refuses to make).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneBuffer {
    pub width: u32,
    pub height: u32,
    /// Interleaved RGBA, `width * height * 4` long.
    pub samples: Vec<f32>,
}

impl SceneBuffer {
    /// Decodes 8-bit sRGB bytes into linear light.
    pub fn from_srgb8(width: u32, height: u32, pixels: &[u8]) -> Self {
        let mut samples = Vec::with_capacity(pixels.len());
        for pixel in pixels.chunks_exact(4) {
            for channel in &pixel[..3] {
                samples.push(crate::color::srgb_to_linear(f64::from(*channel) / 255.0) as f32);
            }
            samples.push(f32::from(pixel[3]) / 255.0);
        }
        Self {
            width,
            height,
            samples,
        }
    }

    /// Encodes back to 8-bit sRGB, clamping to the displayable range.
    ///
    /// This is the ONE place values are narrowed, which is the point of the type: a pipeline that
    /// encodes between every step throws away the headroom it was given.
    pub fn to_srgb8(&self) -> Vec<u8> {
        let mut pixels = Vec::with_capacity(self.samples.len());
        for pixel in self.samples.chunks_exact(4) {
            for channel in &pixel[..3] {
                let encoded = crate::color::linear_to_srgb(f64::from(*channel).clamp(0.0, 1.0));
                pixels.push((encoded * 255.0).round().clamp(0.0, 255.0) as u8);
            }
            pixels.push((pixel[3] * 255.0).round().clamp(0.0, 255.0) as u8);
        }
        pixels
    }

    /// Mixes `other` into this buffer by `amount`, in LINEAR light.
    ///
    /// The whole reason this method exists rather than a byte-wise lerp at the call site: mixing in
    /// light is what makes a half-strength effect look half as strong.
    pub fn mix_from(&mut self, other: &Self, amount: f32) {
        let amount = amount.clamp(0.0, 1.0);
        for (out, source) in self.samples.iter_mut().zip(other.samples.iter()) {
            *out = *out * (1.0 - amount) + *source * amount;
        }
    }

    /// True when any channel carries light outside the displayable range -- the headroom this type
    /// exists to preserve. Reported rather than silently clamped so a caller can say the result was
    /// narrowed on the way out.
    pub fn has_out_of_range(&self) -> bool {
        self.samples
            .chunks_exact(4)
            .any(|pixel| pixel[..3].iter().any(|value| *value < 0.0 || *value > 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fix in one assertion: half of black and half of white is a mid GREY in light, which sRGB
    /// encodes near 188. A byte-wise average gives 128, visibly darker, and that is why a half-strength
    /// effect used to look heavier than it should.
    #[test]
    fn mixing_in_linear_light_is_not_the_byte_average() {
        let black = SceneBuffer::from_srgb8(1, 1, &[0, 0, 0, 255]);
        let white = SceneBuffer::from_srgb8(1, 1, &[255, 255, 255, 255]);
        let mut mixed = black;
        mixed.mix_from(&white, 0.5);
        let out = mixed.to_srgb8();
        assert!(
            (185..=191).contains(&out[0]),
            "a linear half-mix should encode near 188, got {out:?}"
        );
        assert_eq!(out[3], 255, "alpha is coverage and must not be transferred");
    }

    /// A round trip through the buffer must not move a pixel: if it did, every use of the type would
    /// shift colours on its own.
    #[test]
    fn a_round_trip_through_linear_is_lossless_at_eight_bits() {
        let pixels: Vec<u8> = (0..=255u8)
            .flat_map(|value| [value, 255 - value, value / 2, 255])
            .collect();
        let buffer = SceneBuffer::from_srgb8(256, 1, &pixels);
        assert_eq!(buffer.to_srgb8(), pixels);
    }

    /// Out-of-range light survives while the buffer is in this form, and is reported. Clamping it on
    /// arrival would defeat the headroom the type exists for.
    #[test]
    fn values_above_one_survive_until_the_buffer_becomes_bytes() {
        let mut buffer = SceneBuffer::from_srgb8(1, 1, &[255, 255, 255, 255]);
        for channel in 0..3 {
            buffer.samples[channel] *= 4.0;
        }
        assert!(buffer.has_out_of_range());
        // Scaling back down recovers the original, which a clamp on arrival would have made impossible.
        for channel in 0..3 {
            buffer.samples[channel] /= 4.0;
        }
        assert_eq!(buffer.to_srgb8(), vec![255, 255, 255, 255]);
    }
}
