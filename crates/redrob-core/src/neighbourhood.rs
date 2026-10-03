// SPDX-License-Identifier: GPL-3.0-or-later

//! Reading a pixel's neighbourhood, with one place that decides what lies outside the image (K.0).
//!
//! Every window filter needs a rule for samples off the edge, and before this there were **twelve
//! separate sites** in `filters.rs` spelling `x.clamp(0, w - 1)` inline. Twelve copies of a policy
//! is twelve chances for one of them to differ, and no way to change the rule for a filter that
//! needs a different one.
//!
//! Group K adds dozens more window filters. This exists so each of them states which policy it
//! wants instead of re-deriving it.
//!
//! # The policies, and the one that is not upstream's
//!
//! GEGL names three abyss policies, and `app/` uses two of them in practice:
//! `GEGL_ABYSS_NONE` (197 uses — outside is transparent black), `GEGL_ABYSS_CLAMP` (18 — the edge
//! pixel repeats), and `GEGL_ABYSS_LOOP` (3 — the image wraps). All three are here.
//!
//! [`EdgePolicy::Normalise`] is a FOURTH and is ours. It reports which samples were in bounds and
//! lets the caller divide by their weight instead of by the full window. For a blur that is the
//! only correct answer: with transparent black the edges fade out, and with clamp they are biased
//! toward whatever the border pixel happened to be. Our existing box blur already did exactly this
//! by hand — summing a shortened run and dividing by its real length — so naming it here makes an
//! unwritten rule explicit rather than inventing one.

/// What lies outside the image when a window overhangs the edge.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EdgePolicy {
    /// Outside is transparent black, `GEGL_ABYSS_NONE`. Upstream's overwhelming default.
    ///
    /// Right for anything measuring a difference — an edge detector should see the image's border
    /// as an edge, because it is one.
    #[default]
    TransparentBlack,
    /// The nearest edge pixel repeats, `GEGL_ABYSS_CLAMP`.
    ///
    /// Right for anything that must not invent contrast at the border. This is the policy the
    /// twelve hand-rolled sites were all using.
    Clamp,
    /// The image wraps, `GEGL_ABYSS_LOOP`.
    ///
    /// Right for a texture meant to tile, where the edge is not an edge.
    Wrap,
    /// Out-of-bounds samples are SKIPPED and reported, so the caller divides by what it actually
    /// read.
    ///
    /// Not one of upstream's. See the module note: it is the only policy that leaves a blurred
    /// image's border neither faded nor biased.
    Normalise,
}

/// A borrowed RGBA image that can be sampled outside its own bounds.
#[derive(Clone, Copy)]
pub struct Neighbourhood<'a> {
    pixels: &'a [u8],
    width: i64,
    height: i64,
    policy: EdgePolicy,
}

impl<'a> Neighbourhood<'a> {
    pub fn new(pixels: &'a [u8], width: u32, height: u32, policy: EdgePolicy) -> Self {
        Self {
            pixels,
            width: i64::from(width),
            height: i64::from(height),
            policy,
        }
    }

    /// One channel at `(x, y)`, resolved through the policy.
    ///
    /// `None` means the sample does not exist and the caller must not count it — which only
    /// [`EdgePolicy::Normalise`] ever returns. Every other policy answers for any coordinate, so a
    /// caller using one of them can `unwrap_or` with confidence rather than carrying a branch it
    /// can never take.
    pub fn channel(&self, x: i64, y: i64, channel: usize) -> Option<f64> {
        let (sx, sy) = match self.policy {
            EdgePolicy::Clamp => (x.clamp(0, self.width - 1), y.clamp(0, self.height - 1)),
            EdgePolicy::Wrap => (x.rem_euclid(self.width), y.rem_euclid(self.height)),
            EdgePolicy::TransparentBlack => {
                if x < 0 || y < 0 || x >= self.width || y >= self.height {
                    return Some(0.0);
                }
                (x, y)
            }
            EdgePolicy::Normalise => {
                if x < 0 || y < 0 || x >= self.width || y >= self.height {
                    return None;
                }
                (x, y)
            }
        };
        let index = (sy as usize * self.width as usize + sx as usize) * 4 + channel;
        self.pixels.get(index).map(|value| f64::from(*value))
    }

    /// One channel at `(x, y)` as a plain value, with the policy's own answer for outside.
    ///
    /// Convenience for the three total policies. Under [`EdgePolicy::Normalise`] an out-of-bounds
    /// sample reads as 0.0 here, which is why that policy's callers must use [`Self::channel`] —
    /// the point of it is knowing a sample was absent.
    pub fn channel_or_zero(&self, x: i64, y: i64, channel: usize) -> f64 {
        self.channel(x, y, channel).unwrap_or(0.0)
    }

    /// Sums one channel over a square window, returning the total and how many samples counted.
    ///
    /// The count is what [`EdgePolicy::Normalise`] is for. Under any other policy it is always the
    /// full window, because those policies answer for every coordinate.
    pub fn window_sum(&self, x: i64, y: i64, radius: i64, channel: usize) -> (f64, usize) {
        let mut total = 0.0;
        let mut counted = 0usize;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if let Some(value) = self.channel(x + dx, y + dy, channel) {
                    total += value;
                    counted += 1;
                }
            }
        }
        (total, counted)
    }

    /// Applies a square convolution kernel to one channel.
    ///
    /// The kernel is row-major and its side must be odd, so there is a centre to apply it at. An
    /// even kernel has no centre and the only ways to use one are to bias the result half a pixel
    /// or to silently pick a corner; refusing is better than either.
    ///
    /// Divides by the weight ACTUALLY applied, not by the kernel's total, so a window overhanging
    /// the edge under [`EdgePolicy::Normalise`] stays correctly scaled. Under the other policies
    /// the two are identical.
    pub fn convolve(&self, x: i64, y: i64, kernel: &[f64], side: usize, channel: usize) -> f64 {
        debug_assert!(side % 2 == 1, "a convolution kernel needs a centre");
        debug_assert_eq!(kernel.len(), side * side);
        let radius = (side / 2) as i64;
        let mut total = 0.0;
        let mut weight = 0.0;
        for (index, factor) in kernel.iter().enumerate() {
            let dy = (index / side) as i64 - radius;
            let dx = (index % side) as i64 - radius;
            if let Some(value) = self.channel(x + dx, y + dy, channel) {
                total += value * factor;
                weight += factor;
            }
        }
        if weight.abs() <= f64::EPSILON {
            // A kernel summing to zero is a difference operator -- a Laplacian, a Sobel -- where
            // rescaling by the weight is meaningless and dividing by it is a division by zero. The
            // raw response IS the answer for those.
            return total;
        }
        total / weight
    }
}
