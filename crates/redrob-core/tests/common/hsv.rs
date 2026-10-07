// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared helpers for the filter tests.
//!
//! # Why `hue` returns an `Option`
//!
//! Cycles 21 and 24 each independently wrote a saturation-stretch test that asserted hue
//! preservation on every pixel, and each failed on the LEAST-SATURATED one — because a stretch
//! maps the minimum to zero, zero saturation is grey, and grey has no hue. The second time
//! happened one cycle after the first was written down in the progress log, which is the evidence
//! that a note does not prevent a repeat and a type does.
//!
//! So hue is `Option<f64>` here. A test cannot accidentally compare the hue of a grey pixel
//! against a number; it has to say what it expects of an achromatic result.

#![allow(dead_code)]

/// Hue in degrees, saturation and value, all from an 8-bit RGB triple.
///
/// Computed here rather than by calling the implementation's own conversion: a test that reused
/// that function would agree with it even if it were wrong.
pub fn hsv(r: u8, g: u8, b: u8) -> Hsv {
    let (rf, gf, bf) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let delta = max - min;
    let hue = if delta < 1e-9 {
        None
    } else if max == rf {
        Some((60.0 * (((gf - bf) / delta) % 6.0)).rem_euclid(360.0))
    } else if max == gf {
        Some((60.0 * ((bf - rf) / delta + 2.0)).rem_euclid(360.0))
    } else {
        Some((60.0 * ((rf - gf) / delta + 4.0)).rem_euclid(360.0))
    };
    Hsv {
        hue,
        saturation: if max <= 0.0 { 0.0 } else { delta / max },
        value: max,
    }
}

pub struct Hsv {
    /// `None` when the colour is achromatic — grey has no hue, and pretending it is 0 is the trap
    /// this type exists to close.
    pub hue: Option<f64>,
    pub saturation: f64,
    pub value: f64,
}

impl Hsv {
    /// The hue, or a test failure naming which pixel was unexpectedly grey.
    pub fn chromatic_hue(&self, label: &str) -> f64 {
        self.hue
            .unwrap_or_else(|| panic!("{label} is achromatic, so it has no hue to compare"))
    }
}
