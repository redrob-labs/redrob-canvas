// SPDX-License-Identifier: MIT OR Apache-2.0
//! Colour models and channel depths beyond the document pixel formats: XYZ and YCbCr models,
//! the HSV / HSL / HSI / HSY cylindrical models with configurable luma coefficients and gamma, a
//! set of wide RGB working spaces (sRGB, Display P3, Adobe-compatible, Rec. 2020, ACEScg, linear
//! sRGB) with proofing to the painting space, and 8/16-bit integer and 16/32-bit float channel
//! values with a real IEEE half-float encoding.
//!
//! Everything here works on straight sRGB-encoded floats (the tool colours) and is pure maths:
//! the colour selectors (advanced, specific, wide-gamut, artistic) and the colour-management
//! settings read it. Document pixel storage keeps [`crate::SampleType`]; a 16-bit-float value is a
//! presentation and interchange form here.

use serde::{Deserialize, Serialize};

use crate::convert::{D50, SRGB_TO_XYZ_D50, XYZ_D50_TO_SRGB, linear_to_srgb, mat3_mul, srgb_to_lab, srgb_to_linear};

/// A colour model the selectors can show channels in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorModel {
    #[default]
    Rgb,
    Gray,
    Cmyk,
    Lab,
    /// CIE XYZ (D50), channels 0..1 relative to the white point.
    Xyz,
    /// ITU-R BT.709 Y'CbCr, Y' 0..1 and Cb, Cr stored 0..1 (½ = neutral).
    YCbCr,
    Hsv,
    Hsl,
    Hsi,
    /// Hue, saturation and luma (the Y' of the luma coefficients).
    Hsy,
}

impl ColorModel {
    pub const ALL: [ColorModel; 10] = [
        ColorModel::Rgb,
        ColorModel::Gray,
        ColorModel::Cmyk,
        ColorModel::Lab,
        ColorModel::Xyz,
        ColorModel::YCbCr,
        ColorModel::Hsv,
        ColorModel::Hsl,
        ColorModel::Hsi,
        ColorModel::Hsy,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ColorModel::Rgb => "rgb",
            ColorModel::Gray => "gray",
            ColorModel::Cmyk => "cmyk",
            ColorModel::Lab => "lab",
            ColorModel::Xyz => "xyz",
            ColorModel::YCbCr => "ycbcr",
            ColorModel::Hsv => "hsv",
            ColorModel::Hsl => "hsl",
            ColorModel::Hsi => "hsi",
            ColorModel::Hsy => "hsy",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == s.to_ascii_lowercase())
    }

    pub fn label(self) -> &'static str {
        match self {
            ColorModel::Rgb => "RGB",
            ColorModel::Gray => "Gray",
            ColorModel::Cmyk => "CMYK",
            ColorModel::Lab => "Lab",
            ColorModel::Xyz => "XYZ",
            ColorModel::YCbCr => "YCbCr",
            ColorModel::Hsv => "HSV",
            ColorModel::Hsl => "HSL",
            ColorModel::Hsi => "HSI",
            ColorModel::Hsy => "HSY",
        }
    }

    /// Channel names, in order.
    pub fn channels(self) -> &'static [&'static str] {
        match self {
            ColorModel::Rgb => &["R", "G", "B"],
            ColorModel::Gray => &["K"],
            ColorModel::Cmyk => &["C", "M", "Y", "K"],
            ColorModel::Lab => &["L", "a", "b"],
            ColorModel::Xyz => &["X", "Y", "Z"],
            ColorModel::YCbCr => &["Y", "Cb", "Cr"],
            ColorModel::Hsv => &["H", "S", "V"],
            ColorModel::Hsl => &["H", "S", "L"],
            ColorModel::Hsi => &["H", "S", "I"],
            ColorModel::Hsy => &["H", "S", "Y"],
        }
    }

    /// Is this one of the hue-based cylindrical models?
    pub fn is_cylindrical(self) -> bool {
        matches!(self, ColorModel::Hsv | ColorModel::Hsl | ColorModel::Hsi | ColorModel::Hsy)
    }

    /// Nominal display range of channel `i` for a readout (`(min, max, unit)`); every model is
    /// stored as 0..1 fractions internally.
    pub fn channel_range(self, i: usize) -> (f32, f32, &'static str) {
        match (self, i) {
            (ColorModel::Lab, 0) => (0.0, 100.0, ""),
            (ColorModel::Lab, _) => (-128.0, 127.0, ""),
            (ColorModel::Hsv | ColorModel::Hsl | ColorModel::Hsi | ColorModel::Hsy, 0) => (0.0, 360.0, "°"),
            (ColorModel::Hsv | ColorModel::Hsl | ColorModel::Hsi | ColorModel::Hsy, _) => (0.0, 100.0, "%"),
            (ColorModel::Cmyk, _) => (0.0, 100.0, "%"),
            (ColorModel::YCbCr, 0) => (0.0, 1.0, ""),
            (ColorModel::YCbCr, _) => (-0.5, 0.5, ""),
            _ => (0.0, 1.0, ""),
        }
    }
}

/// Channel depth a value is shown or stored at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Depth {
    #[default]
    U8,
    U16,
    /// IEEE 754 binary16.
    F16,
    F32,
}

impl Depth {
    pub const ALL: [Depth; 4] = [Depth::U8, Depth::U16, Depth::F16, Depth::F32];

    pub fn id(self) -> &'static str {
        match self {
            Depth::U8 => "u8",
            Depth::U16 => "u16",
            Depth::F16 => "f16",
            Depth::F32 => "f32",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "u8" | "8" | "8-bit" | "8bit" => Some(Depth::U8),
            "u16" | "16" | "16-bit" | "16bit" => Some(Depth::U16),
            "f16" | "16f" | "half" | "16-bit float" => Some(Depth::F16),
            "f32" | "32" | "32f" | "float" | "32-bit float" => Some(Depth::F32),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Depth::U8 => "8-bit integer",
            Depth::U16 => "16-bit integer",
            Depth::F16 => "16-bit float",
            Depth::F32 => "32-bit float",
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, Depth::F16 | Depth::F32)
    }

    /// The largest integer code value (`None` for float depths).
    pub fn max_code(self) -> Option<u32> {
        match self {
            Depth::U8 => Some(255),
            Depth::U16 => Some(65535),
            Depth::F16 | Depth::F32 => None,
        }
    }

    /// The nearest storable value to `v` (a 0..1 fraction, or any finite float for float depths):
    /// integers quantise, half floats round to binary16, F32 is as given.
    pub fn quantize(self, v: f32) -> f32 {
        match self {
            Depth::U8 => (v.clamp(0.0, 1.0) * 255.0).round() / 255.0,
            Depth::U16 => (v.clamp(0.0, 1.0) * 65535.0).round() / 65535.0,
            Depth::F16 => half_to_f32(f32_to_half(v)),
            Depth::F32 => v,
        }
    }

    /// `v` as the number a channel field shows at this depth: the integer code (0..255,
    /// 0..65535) or the float itself.
    pub fn display_value(self, v: f32) -> f32 {
        match self.max_code() {
            Some(m) => (v.clamp(0.0, 1.0) * m as f32).round(),
            None => self.quantize(v),
        }
    }

    /// Inverse of [`display_value`](Self::display_value).
    pub fn from_display(self, shown: f32) -> f32 {
        match self.max_code() {
            Some(m) => (shown / m as f32).clamp(0.0, 1.0),
            None => shown,
        }
    }

    /// The smallest step between two shown values (for field drag speed).
    pub fn step(self) -> f32 {
        match self {
            Depth::U8 | Depth::U16 => 1.0,
            Depth::F16 => 0.001,
            Depth::F32 => 0.0001,
        }
    }
}

/// Encode an `f32` as IEEE 754 binary16 bits (round to nearest even; overflow to infinity,
/// NaN kept as a quiet NaN).
pub fn f32_to_half(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;
    if exp == 0xff {
        // Inf or NaN.
        return sign | 0x7c00 | if mant != 0 { 0x0200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        // Subnormal half (or zero): shift the full mantissa (with the hidden bit) right.
        if e < -10 {
            return sign;
        }
        let m = mant | 0x0080_0000;
        let shift = (14 - e) as u32;
        let half_m = m >> shift;
        let rem = m & ((1u32 << shift) - 1);
        let halfway = 1u32 << (shift - 1);
        let rounded = if rem > halfway || (rem == halfway && (half_m & 1) == 1) { half_m + 1 } else { half_m };
        return sign | rounded as u16;
    }
    let half_m = mant >> 13;
    let rem = mant & 0x1fff;
    let mut out = ((e as u32) << 10) | half_m;
    if rem > 0x1000 || (rem == 0x1000 && (half_m & 1) == 1) {
        out += 1; // may carry into the exponent, which is the correct rounding
    }
    sign | out as u16
}

/// Decode IEEE 754 binary16 bits to `f32`.
pub fn half_to_f32(h: u16) -> f32 {
    let sign = u32::from(h & 0x8000) << 16;
    let exp = u32::from((h >> 10) & 0x1f);
    let mant = u32::from(h & 0x03ff);
    let bits = if exp == 0 {
        if mant == 0 {
            sign
        } else {
            // Subnormal: normalise.
            let mut e = 127 - 15 + 1;
            let mut m = mant;
            while m & 0x0400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x03ff) << 13)
        }
    } else if exp == 0x1f {
        sign | 0x7f80_0000 | (mant << 13)
    } else {
        sign | ((exp + 127 - 15) << 23) | (mant << 13)
    };
    f32::from_bits(bits)
}

/// Luma coefficients of the HSY model and the YCbCr matrix.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Luma {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    /// Gamma the RGB values are linearised with before the luma is taken (1 = none, the sRGB
    /// encoded values themselves; 2.2 approximates linear light).
    pub gamma: f32,
}

impl Default for Luma {
    fn default() -> Self {
        Self::REC709
    }
}

impl Luma {
    /// ITU-R BT.709 (sRGB primaries).
    pub const REC709: Self = Self { r: 0.2126, g: 0.7152, b: 0.0722, gamma: 1.0 };
    /// ITU-R BT.601.
    pub const REC601: Self = Self { r: 0.299, g: 0.587, b: 0.114, gamma: 1.0 };
    /// ITU-R BT.2020.
    pub const REC2020: Self = Self { r: 0.2627, g: 0.6780, b: 0.0593, gamma: 1.0 };
    /// Equal weights (the I of HSI).
    pub const MEAN: Self = Self { r: 1.0 / 3.0, g: 1.0 / 3.0, b: 1.0 / 3.0, gamma: 1.0 };

    pub const PRESETS: [(&str, Self); 4] = [("Rec. 709", Self::REC709), ("Rec. 601", Self::REC601), ("Rec. 2020", Self::REC2020), ("Equal", Self::MEAN)];

    /// Coefficients normalised to sum to 1 (a hand-edited set stays a weighted mean).
    pub fn normalized(self) -> Self {
        let s = self.r + self.g + self.b;
        if s.is_finite() && s > 1e-6 { Self { r: self.r / s, g: self.g / s, b: self.b / s, gamma: self.gamma } } else { Self::REC709 }
    }

    fn gamma(&self) -> f32 {
        if self.gamma.is_finite() && self.gamma > 0.1 { self.gamma.min(4.0) } else { 1.0 }
    }

    /// Luma of an sRGB-encoded colour, 0..1.
    pub fn luma(&self, rgb: [f32; 3]) -> f32 {
        let k = self.normalized();
        let g = self.gamma();
        let lin = |v: f32| v.clamp(0.0, 1.0).powf(g);
        let y = k.r * lin(rgb[0]) + k.g * lin(rgb[1]) + k.b * lin(rgb[2]);
        y.max(0.0).powf(1.0 / g)
    }
}

/// RGB (sRGB-encoded 0..1) → HSV: hue in degrees 0..360, saturation and value 0..1.
pub fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let c = c.map(|v| v.clamp(0.0, 1.0));
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let d = mx - mn;
    [hue_of(c, mx, mn), if mx > 0.0 { d / mx } else { 0.0 }, mx]
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let h6 = (h.rem_euclid(360.0)) / 60.0;
    let i = h6.floor();
    let f = h6 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// RGB → HSL (hue 0..360, saturation and lightness 0..1).
pub fn rgb_to_hsl(c: [f32; 3]) -> [f32; 3] {
    let c = c.map(|v| v.clamp(0.0, 1.0));
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let l = (mx + mn) / 2.0;
    let d = mx - mn;
    let s = if d <= 0.0 || l <= 0.0 || l >= 1.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()) };
    [hue_of(c, mx, mn), s.clamp(0.0, 1.0), l]
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let m = l - c / 2.0;
    let [r, g, b] = hue_chroma(h, c);
    [r + m, g + m, b + m].map(|v| v.clamp(0.0, 1.0))
}

/// RGB → HSI (hue 0..360, saturation 0..1, intensity = mean 0..1).
pub fn rgb_to_hsi(c: [f32; 3]) -> [f32; 3] {
    let c = c.map(|v| v.clamp(0.0, 1.0));
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let i = (c[0] + c[1] + c[2]) / 3.0;
    let s = if i <= 0.0 { 0.0 } else { 1.0 - mn / i };
    [hue_of(c, mx, mn), s.clamp(0.0, 1.0), i]
}

pub fn hsi_to_rgb(h: f32, s: f32, i: f32) -> [f32; 3] {
    let (s, i) = (s.clamp(0.0, 1.0), i.clamp(0.0, 1.0));
    // The colour lies on the hue's chroma line `base × scale + m` with its minimum channel at
    // `m = i (1 − s)` and its mean at `i`.
    let [r0, g0, b0] = hue_chroma(h, 1.0);
    let mean0 = (r0 + g0 + b0) / 3.0;
    let m = i * (1.0 - s);
    let scale = (i - m) / mean0.max(1e-6);
    [r0 * scale + m, g0 * scale + m, b0 * scale + m].map(|v| v.clamp(0.0, 1.0))
}

/// RGB → HSY (hue 0..360, saturation 0..1, luma 0..1 by `luma`).
pub fn rgb_to_hsy(c: [f32; 3], luma: &Luma) -> [f32; 3] {
    let c = c.map(|v| v.clamp(0.0, 1.0));
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let y = luma.luma(c);
    let h = hue_of(c, mx, mn);
    // Saturation as the chroma relative to the largest chroma possible at this hue and luma.
    let chroma = mx - mn;
    let max_c = max_chroma_at(h, y, luma);
    let s = if max_c > 1e-6 { (chroma / max_c).clamp(0.0, 1.0) } else { 0.0 };
    [h, s, y]
}

pub fn hsy_to_rgb(h: f32, s: f32, y: f32, luma: &Luma) -> [f32; 3] {
    let (s, y) = (s.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
    let c = s * max_chroma_at(h, y, luma);
    let base = hue_chroma(h, c);
    // Offset so the luma equals y (with gamma 1 this is exact; otherwise iterate once).
    let k = luma.normalized();
    let mut m = y - (k.r * base[0] + k.g * base[1] + k.b * base[2]);
    let mut out = [base[0] + m, base[1] + m, base[2] + m];
    if (luma.gamma() - 1.0).abs() > 1e-3 {
        for _ in 0..4 {
            let yy = luma.luma(out.map(|v| v.clamp(0.0, 1.0)));
            m += (y - yy) * 0.9;
            out = [base[0] + m, base[1] + m, base[2] + m];
        }
    }
    out.map(|v| v.clamp(0.0, 1.0))
}

/// The largest chroma a colour of hue `h` and luma `y` can have within 0..1 RGB.
fn max_chroma_at(h: f32, y: f32, luma: &Luma) -> f32 {
    let base = hue_chroma(h, 1.0);
    let k = luma.normalized();
    let yb = k.r * base[0] + k.g * base[1] + k.b * base[2];
    // Colours on the hue's chroma line: rgb = base × c + m with luma yb × c + m = y.
    // Up from black: m = 0 → c = y / yb. Down from white: max channel = 1 → c + m = 1 → c = (1 − y) / (1 − yb).
    let up = if yb > 1e-6 { y / yb } else { 0.0 };
    let down = if yb < 1.0 - 1e-6 { (1.0 - y) / (1.0 - yb) } else { 0.0 };
    up.min(down).clamp(0.0, 1.0)
}

fn hue_of(c: [f32; 3], mx: f32, mn: f32) -> f32 {
    let d = mx - mn;
    if d <= 0.0 {
        return 0.0;
    }
    let h = if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    (h * 60.0).rem_euclid(360.0)
}

/// The RGB triple of hue `h` (degrees) at chroma `c` with its minimum channel at 0.
fn hue_chroma(h: f32, c: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h6.rem_euclid(2.0) - 1.0).abs());
    match h6.floor() as i32 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    }
}

/// sRGB → CIE XYZ (D50), each channel as a fraction of the white point (so white is 1, 1, 1).
pub fn srgb_to_xyz(rgb: [f32; 3]) -> [f32; 3] {
    let lin = rgb.map(|v| srgb_to_linear(v.clamp(0.0, 1.0)));
    let xyz = mat3_mul(&SRGB_TO_XYZ_D50, lin);
    [xyz[0] / D50[0], xyz[1] / D50[1], xyz[2] / D50[2]]
}

pub fn xyz_to_srgb(xyz: [f32; 3]) -> [f32; 3] {
    let abs = [xyz[0] * D50[0], xyz[1] * D50[1], xyz[2] * D50[2]];
    mat3_mul(&XYZ_D50_TO_SRGB, abs).map(|v| linear_to_srgb(v.clamp(0.0, 1.0)))
}

/// sRGB → Y'CbCr (full range): Y' 0..1, Cb and Cr −0.5..0.5, with `luma`'s coefficients
/// (Rec. 709 by default).
pub fn srgb_to_ycbcr(rgb: [f32; 3], luma: &Luma) -> [f32; 3] {
    let k = luma.normalized();
    let c = rgb.map(|v| v.clamp(0.0, 1.0));
    let y = k.r * c[0] + k.g * c[1] + k.b * c[2];
    let cb = 0.5 * (c[2] - y) / (1.0 - k.b).max(1e-6);
    let cr = 0.5 * (c[0] - y) / (1.0 - k.r).max(1e-6);
    [y, cb, cr]
}

pub fn ycbcr_to_srgb(ycc: [f32; 3], luma: &Luma) -> [f32; 3] {
    let k = luma.normalized();
    let (y, cb, cr) = (ycc[0], ycc[1], ycc[2]);
    let r = y + cr * 2.0 * (1.0 - k.r);
    let b = y + cb * 2.0 * (1.0 - k.b);
    let g = if k.g > 1e-6 { (y - k.r * r - k.b * b) / k.g } else { y };
    [r, g, b].map(|v| v.clamp(0.0, 1.0))
}

/// A colour as its channels in `model`, every channel as a 0..1 fraction (hue / 360, Lab
/// `(L / 100, (a + 128) / 255, (b + 128) / 255)`, YCbCr chroma + ½), so a channel slider works
/// the same for every model. Unused channels are 0.
pub fn to_model(rgb: [f32; 3], model: ColorModel, luma: &Luma) -> [f32; 4] {
    let c = rgb.map(|v| v.clamp(0.0, 1.0));
    match model {
        ColorModel::Rgb => [c[0], c[1], c[2], 0.0],
        ColorModel::Gray => [crate::convert::rgb_to_gray(c), 0.0, 0.0, 0.0],
        ColorModel::Cmyk => crate::convert::rgb_to_cmyk(c),
        ColorModel::Lab => {
            let l = srgb_to_lab(c);
            [(l[0] / 100.0).clamp(0.0, 1.0), ((l[1] + 128.0) / 255.0).clamp(0.0, 1.0), ((l[2] + 128.0) / 255.0).clamp(0.0, 1.0), 0.0]
        }
        ColorModel::Xyz => {
            let x = srgb_to_xyz(c);
            [x[0].clamp(0.0, 1.0), x[1].clamp(0.0, 1.0), x[2].clamp(0.0, 1.0), 0.0]
        }
        ColorModel::YCbCr => {
            let y = srgb_to_ycbcr(c, luma);
            [y[0], (y[1] + 0.5).clamp(0.0, 1.0), (y[2] + 0.5).clamp(0.0, 1.0), 0.0]
        }
        ColorModel::Hsv => {
            let h = rgb_to_hsv(c);
            [h[0] / 360.0, h[1], h[2], 0.0]
        }
        ColorModel::Hsl => {
            let h = rgb_to_hsl(c);
            [h[0] / 360.0, h[1], h[2], 0.0]
        }
        ColorModel::Hsi => {
            let h = rgb_to_hsi(c);
            [h[0] / 360.0, h[1], h[2], 0.0]
        }
        ColorModel::Hsy => {
            let h = rgb_to_hsy(c, luma);
            [h[0] / 360.0, h[1], h[2], 0.0]
        }
    }
}

/// Inverse of [`to_model`]: channels (0..1 fractions) in `model` → sRGB-encoded RGB.
pub fn from_model(ch: [f32; 4], model: ColorModel, luma: &Luma) -> [f32; 3] {
    let f = ch.map(|v| if v.is_finite() { v } else { 0.0 });
    match model {
        ColorModel::Rgb => [f[0], f[1], f[2]].map(|v| v.clamp(0.0, 1.0)),
        ColorModel::Gray => [f[0].clamp(0.0, 1.0); 3],
        ColorModel::Cmyk => crate::convert::cmyk_to_rgb(f.map(|v| v.clamp(0.0, 1.0))),
        ColorModel::Lab => {
            crate::convert::lab_to_srgb([f[0].clamp(0.0, 1.0) * 100.0, f[1].clamp(0.0, 1.0) * 255.0 - 128.0, f[2].clamp(0.0, 1.0) * 255.0 - 128.0])
                .map(|v| v.clamp(0.0, 1.0))
        }
        ColorModel::Xyz => xyz_to_srgb([f[0], f[1], f[2]].map(|v| v.clamp(0.0, 1.0))),
        ColorModel::YCbCr => ycbcr_to_srgb([f[0].clamp(0.0, 1.0), f[1].clamp(0.0, 1.0) - 0.5, f[2].clamp(0.0, 1.0) - 0.5], luma),
        ColorModel::Hsv => hsv_to_rgb(f[0] * 360.0, f[1], f[2]),
        ColorModel::Hsl => hsl_to_rgb(f[0] * 360.0, f[1], f[2]),
        ColorModel::Hsi => hsi_to_rgb(f[0] * 360.0, f[1], f[2]),
        ColorModel::Hsy => hsy_to_rgb(f[0] * 360.0, f[1], f[2], luma),
    }
}

/// Channels of `rgb` in `model` as the numbers a readout shows (degrees, percent, Lab units,
/// XYZ fractions…), see [`ColorModel::channel_range`].
pub fn readout(rgb: [f32; 3], model: ColorModel, luma: &Luma) -> Vec<f32> {
    let ch = to_model(rgb, model, luma);
    (0..model.channels().len())
        .map(|i| {
            let (lo, hi, _) = model.channel_range(i);
            lo + ch[i] * (hi - lo)
        })
        .collect()
}

/// A wide RGB working space the wide-gamut selector can pick colours in. Primaries are given as
/// a linear-RGB → XYZ (D65, except as noted) matrix; the transfer is sRGB's, a pure gamma, or
/// linear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WideSpace {
    #[default]
    Srgb,
    LinearSrgb,
    DisplayP3,
    /// Adobe-compatible primaries, gamma 2.2.
    AdobeCompat,
    Rec2020,
    /// ACEScg (AP1 primaries), linear.
    AcesCg,
}

impl WideSpace {
    pub const ALL: [WideSpace; 6] =
        [WideSpace::Srgb, WideSpace::LinearSrgb, WideSpace::DisplayP3, WideSpace::AdobeCompat, WideSpace::Rec2020, WideSpace::AcesCg];

    pub fn id(self) -> &'static str {
        match self {
            WideSpace::Srgb => "srgb",
            WideSpace::LinearSrgb => "linear-srgb",
            WideSpace::DisplayP3 => "display-p3",
            WideSpace::AdobeCompat => "adobe-rgb-compat",
            WideSpace::Rec2020 => "rec2020",
            WideSpace::AcesCg => "acescg",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.id() == s.to_ascii_lowercase())
    }

    pub fn label(self) -> &'static str {
        match self {
            WideSpace::Srgb => "sRGB",
            WideSpace::LinearSrgb => "Linear sRGB",
            WideSpace::DisplayP3 => "Display P3",
            WideSpace::AdobeCompat => "Adobe RGB (compatible)",
            WideSpace::Rec2020 => "Rec. 2020",
            WideSpace::AcesCg => "ACEScg",
        }
    }

    /// Linear RGB → XYZ (D65) matrix.
    fn to_xyz(self) -> [[f32; 3]; 3] {
        match self {
            WideSpace::Srgb | WideSpace::LinearSrgb => {
                [[0.412_391, 0.357_584, 0.180_481], [0.212_639, 0.715_169, 0.072_192], [0.019_331, 0.119_195, 0.950_532]]
            }
            WideSpace::DisplayP3 => [[0.486_571, 0.265_668, 0.198_217], [0.228_975, 0.691_739, 0.079_287], [0.0, 0.045_113, 1.043_944]],
            WideSpace::AdobeCompat => [[0.576_669, 0.185_558, 0.188_229], [0.297_345, 0.627_364, 0.075_291], [0.027_031, 0.070_689, 0.991_338]],
            WideSpace::Rec2020 => [[0.636_958, 0.144_617, 0.168_881], [0.262_700, 0.677_998, 0.059_302], [0.0, 0.028_073, 1.060_985]],
            // AP1 primaries with a D60 white, adapted here to the D65 reference of the others
            // (Bradford), so a neutral stays neutral when proofed.
            WideSpace::AcesCg => [[0.662_454, 0.134_004, 0.156_188], [0.272_229, 0.674_082, 0.053_690], [-0.005_575, 0.004_061, 1.010_339]],
        }
    }

    fn matrix_from_xyz(self) -> [[f32; 3]; 3] {
        invert3(&self.to_xyz())
    }

    /// Encoded component → linear.
    fn decode(self, v: f32) -> f32 {
        match self {
            WideSpace::Srgb | WideSpace::DisplayP3 => srgb_to_linear(v.clamp(0.0, 1.0)),
            WideSpace::LinearSrgb | WideSpace::AcesCg => v.max(0.0),
            WideSpace::AdobeCompat => v.clamp(0.0, 1.0).powf(2.2),
            WideSpace::Rec2020 => {
                let v = v.clamp(0.0, 1.0);
                if v < 0.081_242_86 { v / 4.5 } else { ((v + 0.099_297) / 1.099_297).powf(1.0 / 0.45) }
            }
        }
    }

    fn encode(self, v: f32) -> f32 {
        match self {
            WideSpace::Srgb | WideSpace::DisplayP3 => linear_to_srgb(v.clamp(0.0, 1.0)),
            WideSpace::LinearSrgb | WideSpace::AcesCg => v.max(0.0),
            WideSpace::AdobeCompat => v.clamp(0.0, 1.0).powf(1.0 / 2.2),
            WideSpace::Rec2020 => {
                let v = v.clamp(0.0, 1.0);
                if v < 0.018_053_97 { v * 4.5 } else { 1.099_297 * v.powf(0.45) - 0.099_297 }
            }
        }
    }

    /// Encoded RGB in this space → sRGB-encoded RGB, unclipped (a value outside 0..1 is out of
    /// the sRGB gamut).
    pub fn to_srgb_unclipped(self, c: [f32; 3]) -> [f32; 3] {
        let lin = c.map(|v| self.decode(v));
        let xyz = mat3_mul(&self.to_xyz(), lin);
        let s = mat3_mul(&WideSpace::Srgb.matrix_from_xyz(), xyz);
        s.map(|v| {
            if v <= 0.0 {
                -linear_to_srgb((-v).min(1.0))
            } else if v >= 1.0 {
                1.0 + (v - 1.0).min(1.0)
            } else {
                linear_to_srgb(v)
            }
        })
    }

    /// Encoded RGB in this space → sRGB, clipped to the display.
    pub fn to_srgb(self, c: [f32; 3]) -> [f32; 3] {
        self.to_srgb_unclipped(c).map(|v| v.clamp(0.0, 1.0))
    }

    /// sRGB-encoded RGB → encoded RGB in this space (clipped to 0..1 for the bounded encodings).
    pub fn from_srgb(self, rgb: [f32; 3]) -> [f32; 3] {
        let lin = rgb.map(|v| srgb_to_linear(v.clamp(0.0, 1.0)));
        let xyz = mat3_mul(&WideSpace::Srgb.to_xyz(), lin);
        let own = mat3_mul(&self.matrix_from_xyz(), xyz);
        own.map(|v| self.encode(v.max(0.0)))
    }

    /// Is the colour `c` (encoded in this space) outside the sRGB painting gamut?
    pub fn out_of_srgb(self, c: [f32; 3]) -> bool {
        self.to_srgb_unclipped(c).iter().any(|v| *v < -0.002 || *v > 1.002)
    }
}

/// Proof a colour picked in `space` to the painting space: the sRGB colour it maps to and
/// whether it had to be clipped (out of gamut).
pub fn proof_to_srgb(space: WideSpace, c: [f32; 3]) -> ([f32; 3], bool) {
    (space.to_srgb(c), space.out_of_srgb(c))
}

fn invert3(m: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let a = m;
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if det.abs() < 1e-12 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let d = 1.0 / det;
    [
        [(a[1][1] * a[2][2] - a[1][2] * a[2][1]) * d, (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * d, (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * d],
        [(a[1][2] * a[2][0] - a[1][0] * a[2][2]) * d, (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * d, (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * d],
        [(a[1][0] * a[2][1] - a[1][1] * a[2][0]) * d, (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * d, (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * d],
    ]
}

/// Perceptual quantiser (SMPTE ST 2084) encoding of an absolute luminance `nits` (0..10000) to
/// a 0..1 signal, for an HDR (Rec. 2020 PQ) canvas surface.
pub fn pq_encode(nits: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let y = (nits.max(0.0) / 10000.0).powf(M1);
    ((C1 + C2 * y) / (1.0 + C3 * y)).powf(M2).clamp(0.0, 1.0)
}

/// Inverse of [`pq_encode`]: a 0..1 PQ signal to nits.
pub fn pq_decode(v: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let p = v.clamp(0.0, 1.0).powf(1.0 / M2);
    let n = (p - C1).max(0.0) / (C2 - C3 * p).max(1e-6);
    10000.0 * n.powf(1.0 / M1)
}

/// Hybrid log-gamma (ARIB STD-B67) encoding of scene-linear `e` (1 = reference white) to 0..1.
pub fn hlg_encode(e: f32) -> f32 {
    const A: f32 = 0.178_832_77;
    const B: f32 = 0.284_668_92;
    const C: f32 = 0.559_910_7;
    let e = e.max(0.0) / 12.0;
    if e <= 1.0 / 12.0 { (3.0 * e).sqrt() } else { A * (12.0 * e - B).max(1e-6).ln() + C }.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn half_floats_round_trip_and_round_to_nearest() {
        for v in [0.0f32, 1.0, 0.5, 0.25, 0.1, 65504.0, -2.0, 1e-5, 3.14159] {
            let h = half_to_f32(f32_to_half(v));
            assert!((h - v).abs() <= v.abs() * 1e-3 + 1e-7, "{v} -> {h}");
        }
        assert_eq!(f32_to_half(1.0), 0x3c00);
        assert_eq!(f32_to_half(-2.0), 0xc000);
        assert!(half_to_f32(f32_to_half(1e6)).is_infinite());
        assert!(half_to_f32(f32_to_half(f32::NAN)).is_nan());
        assert_eq!(half_to_f32(f32_to_half(1e-9)), 0.0, "below the smallest subnormal");
    }

    #[test]
    fn every_model_round_trips_srgb() {
        let luma = Luma::default();
        for model in ColorModel::ALL {
            for rgb in [[0.2, 0.4, 0.6], [1.0, 0.0, 0.0], [0.5, 0.5, 0.5], [0.9, 0.8, 0.1], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]] {
                let back = from_model(to_model(rgb, model, &luma), model, &luma);
                let tol = match model {
                    ColorModel::Gray => 1.0, // lossy by nature
                    ColorModel::Cmyk => 0.2, // through the CMYK profile
                    ColorModel::Lab | ColorModel::Xyz => 0.02,
                    _ => 0.01,
                };
                assert!(close(back, rgb, tol), "{model:?}: {rgb:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn hsy_keeps_the_luma_and_hue() {
        let luma = Luma::REC709;
        let rgb = [0.3, 0.6, 0.2];
        let [h, s, y] = rgb_to_hsy(rgb, &luma);
        assert!((y - luma.luma(rgb)).abs() < 1e-5);
        let back = hsy_to_rgb(h, s, y, &luma);
        assert!(close(back, rgb, 0.01), "{back:?}");
        assert!((luma.luma(hsy_to_rgb(200.0, 1.0, 0.3, &luma)) - 0.3).abs() < 0.01);
    }

    #[test]
    fn depths_quantize_and_display() {
        assert_eq!(Depth::U8.display_value(0.5), 128.0);
        assert_eq!(Depth::U16.display_value(1.0), 65535.0);
        assert_eq!(Depth::U8.from_display(255.0), 1.0);
        assert_eq!(Depth::F32.quantize(2.5), 2.5);
        assert!((Depth::F16.quantize(0.1) - 0.1).abs() < 1e-4);
        assert!(Depth::F16.is_float() && !Depth::U16.is_float());
    }

    #[test]
    fn wide_spaces_contain_srgb_and_flag_out_of_gamut() {
        for sp in WideSpace::ALL {
            for rgb in [[0.2, 0.4, 0.6], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], [0.8, 0.1, 0.3]] {
                let back = sp.to_srgb(sp.from_srgb(rgb));
                assert!(close(back, rgb, 0.02), "{sp:?}: {rgb:?} -> {back:?}");
                assert!(!sp.out_of_srgb(sp.from_srgb(rgb)) || sp == WideSpace::AcesCg, "{sp:?} {rgb:?}");
            }
        }
        // Pure Rec. 2020 green is far outside sRGB.
        assert!(WideSpace::Rec2020.out_of_srgb([0.0, 1.0, 0.0]));
        assert!(!WideSpace::Srgb.out_of_srgb([0.0, 1.0, 0.0]));
    }

    #[test]
    fn pq_and_hlg_encode_reference_points() {
        assert!((pq_encode(100.0) - 0.508).abs() < 0.01, "{}", pq_encode(100.0));
        assert!((pq_decode(pq_encode(1000.0)) - 1000.0).abs() < 1.0);
        assert!((hlg_encode(1.0) - 0.5).abs() < 0.01);
        assert_eq!(pq_encode(0.0), 0.0);
    }

    #[test]
    fn xyz_and_ycbcr_match_known_values() {
        let w = srgb_to_xyz([1.0, 1.0, 1.0]);
        assert!(close(w, [1.0, 1.0, 1.0], 0.01), "{w:?}");
        let y = srgb_to_ycbcr([1.0, 1.0, 1.0], &Luma::REC709);
        assert!(close(y, [1.0, 0.0, 0.0], 1e-4), "{y:?}");
        let red = srgb_to_ycbcr([1.0, 0.0, 0.0], &Luma::REC709);
        assert!((red[0] - 0.2126).abs() < 1e-4 && (red[2] - 0.5).abs() < 1e-4);
    }
}
