//! Colour tools behind Image › Adjustments: component extraction and (de)composition, linear and
//! value inversion, HSV contrast stretch, colour enhance, alien map, colour rotation, palette map,
//! sample colorize, RGB clip, dither, local threshold, destripe, multi-scale retinex, the STRESS
//! envelope operators (Stress, Colour to Gray), border average, smooth palette and the Fattal,
//! Mantiuk and Reinhard tone-mapping operators.
//!
//! All work on straight (non-premultiplied) sRGB RGBA buffers in 0..=1, row-major `w × h`, like
//! [`crate::tone`]: callers convert any colour model / bit depth to RGBA first. Every formula here
//! is written from the published papers and the reference manuals' descriptions of the effects,
//! not from any other implementation.

use std::collections::HashMap;

use redrob_photo_color::convert::{lab_to_srgb, linear_to_srgb, rgb_to_cmyk_naive, srgb_to_lab, srgb_to_linear};

use crate::fxutil::{box_blur_n, gauss_blur_n};
use crate::other2::{hsb_to_rgb, hsl_to_rgb, rgb_to_hsb, rgb_to_hsl};
use crate::photo_util::{hash01, par_map, par_rows};
use crate::pyramid::{self, Level};

const EPS: f32 = 1e-6;

/// Rec. 601 luma of an encoded sRGB colour.
#[inline]
fn luma(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// Rec. 709 luminance of a linear-light colour.
#[inline]
fn luminance_linear(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

#[inline]
fn rgb(q: [f32; 4]) -> [f32; 3] {
    [q[0], q[1], q[2]]
}

#[inline]
fn clamp01(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

#[inline]
fn put(q: &mut [f32; 4], c: [f32; 3]) {
    q[0] = clamp01(c[0]);
    q[1] = clamp01(c[1]);
    q[2] = clamp01(c[2]);
}

// ---------------------------------------------------------------------------------------------
// Colour models and components

/// A colour model whose components Extract Component, Decompose and Compose work in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Rgb,
    Rgba,
    Hsv,
    Hsl,
    Cmy,
    Cmyk,
    Lab,
    Lch,
    /// ITU-R BT.470 (Rec. 601) Y'CbCr, studio range (Y' 16–235, Cb/Cr 16–240 of 255).
    YCbCr470,
    /// ITU-R BT.709 Y'CbCr, studio range.
    YCbCr709,
    /// BT.470 Y'CbCr, full range.
    YCbCr470F,
    /// BT.709 Y'CbCr, full range.
    YCbCr709F,
    Alpha,
}

/// Every model, in menu order.
pub const MODELS: [Model; 13] = [
    Model::Rgb,
    Model::Rgba,
    Model::Hsv,
    Model::Hsl,
    Model::Cmy,
    Model::Cmyk,
    Model::Lab,
    Model::Lch,
    Model::YCbCr470,
    Model::YCbCr709,
    Model::YCbCr470F,
    Model::YCbCr709F,
    Model::Alpha,
];

impl Model {
    /// The model for a parameter id (`rgb`, `rgba`, `hsv`, `hsl`, `cmy`, `cmyk`, `lab`, `lch`,
    /// `ycbcr470` / `ycbcr`, `ycbcr709`, `ycbcr470f`, `ycbcr709f`, `alpha`).
    pub fn from_id(id: &str) -> Option<Model> {
        Some(match id {
            "rgb" => Model::Rgb,
            "rgba" => Model::Rgba,
            "hsv" => Model::Hsv,
            "hsl" => Model::Hsl,
            "cmy" => Model::Cmy,
            "cmyk" => Model::Cmyk,
            "lab" => Model::Lab,
            "lch" => Model::Lch,
            "ycbcr470" | "ycbcr" => Model::YCbCr470,
            "ycbcr709" => Model::YCbCr709,
            "ycbcr470f" => Model::YCbCr470F,
            "ycbcr709f" => Model::YCbCr709F,
            "alpha" => Model::Alpha,
            _ => return None,
        })
    }

    /// The parameter id.
    pub fn id(self) -> &'static str {
        match self {
            Model::Rgb => "rgb",
            Model::Rgba => "rgba",
            Model::Hsv => "hsv",
            Model::Hsl => "hsl",
            Model::Cmy => "cmy",
            Model::Cmyk => "cmyk",
            Model::Lab => "lab",
            Model::Lch => "lch",
            Model::YCbCr470 => "ycbcr470",
            Model::YCbCr709 => "ycbcr709",
            Model::YCbCr470F => "ycbcr470f",
            Model::YCbCr709F => "ycbcr709f",
            Model::Alpha => "alpha",
        }
    }

    /// Component names, in order (also the layer names Decompose gives them).
    pub fn components(self) -> &'static [&'static str] {
        match self {
            Model::Rgb => &["red", "green", "blue"],
            Model::Rgba => &["red", "green", "blue", "alpha"],
            Model::Hsv => &["hue", "saturation", "value"],
            Model::Hsl => &["hue", "saturation", "lightness"],
            Model::Cmy => &["cyan", "magenta", "yellow"],
            Model::Cmyk => &["cyan", "magenta", "yellow", "black"],
            Model::Lab => &["L", "A", "B"],
            Model::Lch => &["L", "C", "H"],
            Model::YCbCr470 | Model::YCbCr709 | Model::YCbCr470F | Model::YCbCr709F => &["Y", "Cb", "Cr"],
            Model::Alpha => &["alpha"],
        }
    }

    /// Whether the model carries the alpha channel as a component.
    pub fn has_alpha(self) -> bool {
        matches!(self, Model::Rgba | Model::Alpha)
    }

    /// The colour a colourised decomposition layer of component `k` is tinted with (`None`: grey).
    pub fn tint(self, k: usize) -> Option<[f32; 3]> {
        match (self, k) {
            (Model::Rgb | Model::Rgba, 0) => Some([1.0, 0.0, 0.0]),
            (Model::Rgb | Model::Rgba, 1) => Some([0.0, 1.0, 0.0]),
            (Model::Rgb | Model::Rgba, 2) => Some([0.0, 0.0, 1.0]),
            (Model::Cmy | Model::Cmyk, 0) => Some([0.0, 1.0, 1.0]),
            (Model::Cmy | Model::Cmyk, 1) => Some([1.0, 0.0, 1.0]),
            (Model::Cmy | Model::Cmyk, 2) => Some([1.0, 1.0, 0.0]),
            _ => None,
        }
    }

    fn ycbcr_weights(self) -> (f32, f32) {
        match self {
            Model::YCbCr709 | Model::YCbCr709F => (0.2126, 0.0722),
            _ => (0.299, 0.114),
        }
    }

    fn ycbcr_studio(self) -> bool {
        matches!(self, Model::YCbCr470 | Model::YCbCr709)
    }

    /// Decomposes a straight sRGB RGBA pixel into the model's components, each in 0..=1 (only the
    /// first [`Self::components`]`.len()` entries are meaningful).
    pub fn decompose(self, q: [f32; 4]) -> [f32; 4] {
        let c = rgb(q);
        match self {
            Model::Rgb => [c[0], c[1], c[2], 0.0],
            Model::Rgba => q,
            Model::Hsv => {
                let h = rgb_to_hsb(c);
                [h[0], h[1], h[2], 0.0]
            }
            Model::Hsl => {
                let h = rgb_to_hsl(c);
                [h[0], h[1], h[2], 0.0]
            }
            Model::Cmy => [1.0 - c[0], 1.0 - c[1], 1.0 - c[2], 0.0],
            Model::Cmyk => rgb_to_cmyk_naive(c),
            Model::Lab => {
                let l = srgb_to_lab(c);
                [l[0] / 100.0, (l[1] + 128.0) / 256.0, (l[2] + 128.0) / 256.0, 0.0]
            }
            Model::Lch => {
                let l = srgb_to_lab(c);
                let chroma = l[1].hypot(l[2]);
                let hue = l[2].atan2(l[1]).to_degrees().rem_euclid(360.0);
                [l[0] / 100.0, chroma / 200.0, hue / 360.0, 0.0]
            }
            Model::YCbCr470 | Model::YCbCr709 | Model::YCbCr470F | Model::YCbCr709F => {
                let (kr, kb) = self.ycbcr_weights();
                let kg = 1.0 - kr - kb;
                let y = kr * c[0] + kg * c[1] + kb * c[2];
                let cb = 0.5 * (c[2] - y) / (1.0 - kb);
                let cr = 0.5 * (c[0] - y) / (1.0 - kr);
                if self.ycbcr_studio() {
                    [16.0 / 255.0 + y * 219.0 / 255.0, 128.0 / 255.0 + cb * 224.0 / 255.0, 128.0 / 255.0 + cr * 224.0 / 255.0, 0.0]
                } else {
                    [y, cb + 0.5, cr + 0.5, 0.0]
                }
            }
            Model::Alpha => [q[3], 0.0, 0.0, 0.0],
        }
    }

    /// Composes a straight sRGB RGBA pixel from the model's components (as [`Self::decompose`]
    /// produces them); `alpha` is used when the model has no alpha component.
    pub fn compose(self, v: [f32; 4], alpha: f32) -> [f32; 4] {
        let v = v.map(clamp01);
        let (c, a) = match self {
            Model::Rgb => ([v[0], v[1], v[2]], alpha),
            Model::Rgba => ([v[0], v[1], v[2]], v[3]),
            Model::Hsv => (hsb_to_rgb([v[0], v[1], v[2]]), alpha),
            Model::Hsl => (hsl_to_rgb([v[0], v[1], v[2]]), alpha),
            Model::Cmy => ([1.0 - v[0], 1.0 - v[1], 1.0 - v[2]], alpha),
            Model::Cmyk => {
                let k = 1.0 - v[3];
                ([(1.0 - v[0]) * k, (1.0 - v[1]) * k, (1.0 - v[2]) * k], alpha)
            }
            Model::Lab => (lab_to_srgb([v[0] * 100.0, v[1] * 256.0 - 128.0, v[2] * 256.0 - 128.0]), alpha),
            Model::Lch => {
                let (chroma, hue) = (v[1] * 200.0, (v[2] * 360.0).to_radians());
                (lab_to_srgb([v[0] * 100.0, chroma * hue.cos(), chroma * hue.sin()]), alpha)
            }
            Model::YCbCr470 | Model::YCbCr709 | Model::YCbCr470F | Model::YCbCr709F => {
                let (kr, kb) = self.ycbcr_weights();
                let kg = 1.0 - kr - kb;
                let (y, cb, cr) = if self.ycbcr_studio() {
                    ((v[0] - 16.0 / 255.0) * 255.0 / 219.0, (v[1] - 128.0 / 255.0) * 255.0 / 224.0, (v[2] - 128.0 / 255.0) * 255.0 / 224.0)
                } else {
                    (v[0], v[1] - 0.5, v[2] - 0.5)
                };
                let r = y + 2.0 * cr * (1.0 - kr);
                let b = y + 2.0 * cb * (1.0 - kb);
                let g = (y - kr * r - kb * b) / kg;
                ([r, g, b], alpha)
            }
            Model::Alpha => ([0.0, 0.0, 0.0], v[0]),
        };
        [clamp01(c[0]), clamp01(c[1]), clamp01(c[2]), clamp01(a)]
    }
}

/// Extract Component ids: `(id, model, component index)`.
pub const COMPONENTS: [(&str, Model, usize); 31] = [
    ("rgbRed", Model::Rgb, 0),
    ("rgbGreen", Model::Rgb, 1),
    ("rgbBlue", Model::Rgb, 2),
    ("alpha", Model::Alpha, 0),
    ("hsvHue", Model::Hsv, 0),
    ("hsvSaturation", Model::Hsv, 1),
    ("hsvValue", Model::Hsv, 2),
    ("hslHue", Model::Hsl, 0),
    ("hslSaturation", Model::Hsl, 1),
    ("hslLightness", Model::Hsl, 2),
    ("cmykCyan", Model::Cmyk, 0),
    ("cmykMagenta", Model::Cmyk, 1),
    ("cmykYellow", Model::Cmyk, 2),
    ("cmykKey", Model::Cmyk, 3),
    ("labL", Model::Lab, 0),
    ("labA", Model::Lab, 1),
    ("labB", Model::Lab, 2),
    ("lchC", Model::Lch, 1),
    ("lchH", Model::Lch, 2),
    ("ycbcr470Y", Model::YCbCr470, 0),
    ("ycbcr470Cb", Model::YCbCr470, 1),
    ("ycbcr470Cr", Model::YCbCr470, 2),
    ("ycbcr709Y", Model::YCbCr709, 0),
    ("ycbcr709Cb", Model::YCbCr709, 1),
    ("ycbcr709Cr", Model::YCbCr709, 2),
    ("ycbcr470fY", Model::YCbCr470F, 0),
    ("ycbcr470fCb", Model::YCbCr470F, 1),
    ("ycbcr470fCr", Model::YCbCr470F, 2),
    ("ycbcr709fY", Model::YCbCr709F, 0),
    ("ycbcr709fCb", Model::YCbCr709F, 1),
    ("ycbcr709fCr", Model::YCbCr709F, 2),
];

/// The model and component index of an Extract Component id.
pub fn component(id: &str) -> Option<(Model, usize)> {
    COMPONENTS.iter().find(|(i, _, _)| *i == id).map(|(_, m, k)| (*m, *k))
}

/// Extract Component: every pixel becomes the grey of one component, optionally inverted;
/// `linear` reads the colour in linear light first. Alpha is kept.
pub fn extract_component(px: &mut [[f32; 4]], model: Model, k: usize, invert: bool, linear: bool) {
    for q in px.iter_mut() {
        let src = if linear { [srgb_to_linear(q[0]), srgb_to_linear(q[1]), srgb_to_linear(q[2]), q[3]] } else { *q };
        let v = model.decompose(src);
        let mut g = clamp01(v[k.min(3)]);
        if invert {
            g = 1.0 - g;
        }
        put(q, [g, g, g]);
    }
}

/// Decompose: one grey plane (0..=1 values) per component of `model`.
pub fn decompose(px: &[[f32; 4]], model: Model) -> Vec<Vec<f32>> {
    let n = model.components().len();
    let mut planes: Vec<Vec<f32>> = (0..n).map(|_| Vec::with_capacity(px.len())).collect();
    for q in px {
        let v = model.decompose(*q);
        for (k, plane) in planes.iter_mut().enumerate() {
            plane.push(clamp01(v[k]));
        }
    }
    planes
}

/// Compose: the colour image whose components of `model` are `planes` (missing planes read as
/// 0, alpha 1 unless the model carries alpha).
pub fn compose(planes: &[&[f32]], model: Model, n: usize) -> Vec<[f32; 4]> {
    par_map(n, |i| {
        let mut v = [0.0f32; 4];
        for (k, plane) in planes.iter().enumerate().take(4) {
            v[k] = plane.get(i).copied().unwrap_or(0.0);
        }
        model.compose(v, 1.0)
    })
}

// ---------------------------------------------------------------------------------------------
// Per-pixel colour tools

/// Linear Invert: inverts the colour in linear light (alpha kept).
pub fn linear_invert(px: &mut [[f32; 4]]) {
    for q in px.iter_mut() {
        put(q, [linear_to_srgb(1.0 - srgb_to_linear(q[0])), linear_to_srgb(1.0 - srgb_to_linear(q[1])), linear_to_srgb(1.0 - srgb_to_linear(q[2]))]);
    }
}

/// Value Invert: inverts the HSV value, keeping hue and saturation.
pub fn value_invert(px: &mut [[f32; 4]]) {
    for q in px.iter_mut() {
        let h = rgb_to_hsb(rgb(*q));
        put(q, hsb_to_rgb([h[0], h[1], 1.0 - h[2]]));
    }
}

/// Stretch Contrast HSV: stretches saturation and value to the full range, keeping hue.
pub fn stretch_contrast_hsv(px: &mut [[f32; 4]]) {
    let (mut smin, mut smax, mut vmin, mut vmax) = (1.0f32, 0.0f32, 1.0f32, 0.0f32);
    for q in px.iter().filter(|q| q[3] > 0.0) {
        let h = rgb_to_hsb(rgb(*q));
        smin = smin.min(h[1]);
        smax = smax.max(h[1]);
        vmin = vmin.min(h[2]);
        vmax = vmax.max(h[2]);
    }
    let stretch = |v: f32, lo: f32, hi: f32| if hi - lo > EPS { (v - lo) / (hi - lo) } else { v };
    for q in px.iter_mut() {
        let h = rgb_to_hsb(rgb(*q));
        put(q, hsb_to_rgb([h[0], stretch(h[1], smin, smax), stretch(h[2], vmin, vmax)]));
    }
}

/// The largest CIE LCh chroma at `l`, `hue` (radians) whose colour is inside the sRGB gamut.
fn gamut_chroma(l: f32, hue: f32) -> f32 {
    let inside = |c: f32| {
        let rgb = lab_to_srgb_unclamped([l, c * hue.cos(), c * hue.sin()]);
        rgb.iter().all(|v| (-0.002..=1.002).contains(v))
    };
    let (mut lo, mut hi) = (0.0f32, 160.0f32);
    for _ in 0..9 {
        let mid = 0.5 * (lo + hi);
        if inside(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// `lab_to_srgb` without the gamut clamp (for in-gamut tests).
fn lab_to_srgb_unclamped(lab: [f32; 3]) -> [f32; 3] {
    use redrob_photo_color::convert::{D50, XYZ_D50_TO_SRGB, mat3_mul};
    let [l, a, b] = lab;
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let finv = |t: f32| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f32 / 29.0).powi(2) * (t - 4.0 / 29.0) };
    let xyz = [finv(fx) * D50[0], finv(fy) * D50[1], finv(fz) * D50[2]];
    mat3_mul(&XYZ_D50_TO_SRGB, xyz)
}

/// Colour Enhance: stretches the chroma range (CIE LCh) so the dullest colour loses all its
/// chroma and the most colourful one reaches the gamut boundary, keeping lightness and hue.
pub fn color_enhance(px: &mut [[f32; 4]]) {
    let (mut cmin, mut cmax) = (f32::MAX, 0.0f32);
    for q in px.iter().filter(|q| q[3] > 0.0) {
        let l = srgb_to_lab(rgb(*q));
        let c = l[1].hypot(l[2]);
        cmin = cmin.min(c);
        cmax = cmax.max(c);
    }
    if cmax - cmin <= EPS {
        return;
    }
    par_rows(px, 1, 1, |_, row| {
        for q in row.iter_mut() {
            let l = srgb_to_lab(rgb(*q));
            let c = l[1].hypot(l[2]);
            let hue = l[2].atan2(l[1]);
            let t = ((c - cmin) / (cmax - cmin)).clamp(0.0, 1.0);
            let nc = t * gamut_chroma(l[0], hue);
            put(q, lab_to_srgb([l[0], nc * hue.cos(), nc * hue.sin()]));
        }
    });
}

/// Alien Map settings: per component a frequency (0–20) and a phase shift (degrees); `keep`
/// leaves the component untouched.
#[derive(Clone, Debug, PartialEq)]
pub struct AlienMap {
    pub hsl: bool,
    pub frequency: [f32; 3],
    pub phase: [f32; 3],
    pub keep: [bool; 3],
}

/// Alien Map: each RGB (or HSL) component becomes `(1 + sin((2·v − 1)·f·π + phase)) / 2`.
pub fn alien_map(px: &mut [[f32; 4]], o: &AlienMap) {
    let map = |v: f32, k: usize| {
        if o.keep[k] {
            return v;
        }
        let ph = o.phase[k].to_radians();
        0.5 * (1.0 + ((2.0 * v - 1.0) * o.frequency[k] * std::f32::consts::PI + ph).sin())
    };
    for q in px.iter_mut() {
        let c = rgb(*q);
        let c = if o.hsl { rgb_to_hsl(c) } else { c };
        let m = [map(c[0], 0), map(c[1], 1), map(c[2], 2)];
        put(q, if o.hsl { hsl_to_rgb([m[0].rem_euclid(1.0), m[1], m[2]]) } else { m });
    }
}

/// How Rotate Colours treats grey pixels (saturation at or below the threshold).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrayMode {
    /// Treat them as having the given hue and saturation, then rotate like any other colour.
    TreatAs,
    /// Change them to the given hue and saturation.
    ChangeTo,
}

/// Rotate Colours settings: hue arcs in degrees on the colour wheel.
#[derive(Clone, Debug, PartialEq)]
pub struct RotateColors {
    pub src_from: f32,
    pub src_to: f32,
    pub src_clockwise: bool,
    pub dst_from: f32,
    pub dst_to: f32,
    pub dst_clockwise: bool,
    pub gray_threshold: f32,
    pub gray_mode: GrayMode,
    pub gray_hue: f32,
    pub gray_saturation: f32,
}

/// Length of the arc from `from` to `to` in the given direction (a zero length is the whole wheel).
fn arc_len(from: f32, to: f32, clockwise: bool) -> f32 {
    let d = if clockwise { (to - from).rem_euclid(360.0) } else { (from - to).rem_euclid(360.0) };
    if d <= EPS { 360.0 } else { d }
}

/// Rotate Colours: hues inside the source arc are mapped linearly onto the destination arc.
pub fn rotate_colors(px: &mut [[f32; 4]], o: &RotateColors) {
    let slen = arc_len(o.src_from, o.src_to, o.src_clockwise);
    let dlen = arc_len(o.dst_from, o.dst_to, o.dst_clockwise);
    let map_hue = |h: f32| -> Option<f32> {
        let d = if o.src_clockwise { (h - o.src_from).rem_euclid(360.0) } else { (o.src_from - h).rem_euclid(360.0) };
        if d > slen + 1e-4 {
            return None;
        }
        let t = d / slen;
        let nh = if o.dst_clockwise { o.dst_from + t * dlen } else { o.dst_from - t * dlen };
        Some(nh.rem_euclid(360.0))
    };
    for q in px.iter_mut() {
        let mut h = rgb_to_hsb(rgb(*q));
        if h[1] <= o.gray_threshold {
            match o.gray_mode {
                GrayMode::ChangeTo => {
                    put(q, hsb_to_rgb([o.gray_hue / 360.0, o.gray_saturation, h[2]]));
                    continue;
                }
                GrayMode::TreatAs => {
                    h[0] = o.gray_hue / 360.0;
                    h[1] = o.gray_saturation;
                }
            }
        }
        if let Some(nh) = map_hue(h[0] * 360.0) {
            put(q, hsb_to_rgb([nh / 360.0, h[1], h[2]]));
        }
    }
}

/// Palette Map: the pixel's luminosity picks a palette colour, in palette order (alpha kept).
pub fn palette_map(px: &mut [[f32; 4]], palette: &[[f32; 3]]) {
    if palette.is_empty() {
        return;
    }
    let n = palette.len();
    for q in px.iter_mut() {
        let i = ((luma(rgb(*q)) * n as f32) as usize).min(n - 1);
        put(q, palette[i]);
    }
}

/// RGB Clip: clamps the colour channels to `low..=high` (either side optional).
pub fn rgb_clip(px: &mut [[f32; 4]], low: Option<f32>, high: Option<f32>) {
    for q in px.iter_mut() {
        for v in q.iter_mut().take(3) {
            if let Some(lo) = low {
                *v = v.max(lo);
            }
            if let Some(hi) = high {
                *v = v.min(hi);
            }
            *v = clamp01(*v);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Sample Colorize

/// Sample Colorize settings (levels 0–255 as in the dialog).
#[derive(Clone, Debug, PartialEq)]
pub struct SampleColorize {
    pub hold_intensity: bool,
    pub original_intensity: bool,
    pub smooth_samples: bool,
    pub in_low: f32,
    pub in_high: f32,
    pub gamma: f32,
    pub out_low: f32,
    pub out_high: f32,
}

impl Default for SampleColorize {
    fn default() -> Self {
        SampleColorize {
            hold_intensity: true,
            original_intensity: true,
            smooth_samples: true,
            in_low: 0.0,
            in_high: 255.0,
            gamma: 1.0,
            out_low: 0.0,
            out_high: 255.0,
        }
    }
}

/// The 256-entry luminosity → colour table of a sample image: each level gets the mean colour of
/// the sample pixels of that luminosity; levels without samples take the nearest filled level
/// (or, with `smooth`, a linear blend of the filled levels either side).
pub fn sample_table(sample: &[[f32; 4]], smooth: bool) -> Option<Vec<[f32; 3]>> {
    let mut sum = vec![[0.0f64; 3]; 256];
    let mut count = vec![0u32; 256];
    for q in sample.iter().filter(|q| q[3] > 0.0) {
        let i = (luma(rgb(*q)) * 255.0).round().clamp(0.0, 255.0) as usize;
        for (s, v) in sum[i].iter_mut().zip(q.iter()) {
            *s += f64::from(*v);
        }
        count[i] += 1;
    }
    let filled: Vec<usize> = (0..256).filter(|&i| count[i] > 0).collect();
    if filled.is_empty() {
        return None;
    }
    let mean = |i: usize| -> [f32; 3] {
        [(sum[i][0] / f64::from(count[i])) as f32, (sum[i][1] / f64::from(count[i])) as f32, (sum[i][2] / f64::from(count[i])) as f32]
    };
    let mut table = vec![[0.0f32; 3]; 256];
    for i in 0..256 {
        if count[i] > 0 {
            table[i] = mean(i);
            continue;
        }
        let below = filled.iter().rev().find(|&&f| f < i).copied();
        let above = filled.iter().find(|&&f| f > i).copied();
        table[i] = match (below, above) {
            (Some(b), Some(a)) if smooth => {
                let t = (i - b) as f32 / (a - b) as f32;
                let (cb, ca) = (mean(b), mean(a));
                [cb[0] + (ca[0] - cb[0]) * t, cb[1] + (ca[1] - cb[1]) * t, cb[2] + (ca[2] - cb[2]) * t]
            }
            (Some(b), Some(a)) => mean(if i - b <= a - i { b } else { a }),
            (Some(b), None) => mean(b),
            (None, Some(a)) => mean(a),
            (None, None) => [0.0; 3],
        };
    }
    Some(table)
}

/// A 256-entry table running from `from` to `to` (the gradient sample).
pub fn gradient_table(from: [f32; 3], to: [f32; 3]) -> Vec<[f32; 3]> {
    (0..256)
        .map(|i| {
            let t = i as f32 / 255.0;
            [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t, from[2] + (to[2] - from[2]) * t]
        })
        .collect()
}

/// Sample Colorize: each pixel's luminosity, mapped through the level controls, picks a colour of
/// `table`; `hold_intensity` then rescales it to the pixel's (original or mapped) luminosity.
pub fn sample_colorize(px: &mut [[f32; 4]], table: &[[f32; 3]], o: &SampleColorize) {
    if table.len() < 2 {
        return;
    }
    let n = table.len();
    let (in_low, in_high) = (o.in_low.min(o.in_high), o.in_high.max(o.in_low + 1.0));
    let gamma = o.gamma.clamp(0.1, 10.0);
    for q in px.iter_mut() {
        let lum = luma(rgb(*q));
        let t = ((lum * 255.0 - in_low) / (in_high - in_low)).clamp(0.0, 1.0).powf(1.0 / gamma);
        let level = (o.out_low + t * (o.out_high - o.out_low)).clamp(0.0, 255.0);
        let i = ((level / 255.0) * (n - 1) as f32).round() as usize;
        let mut c = table[i.min(n - 1)];
        if o.hold_intensity {
            let target = if o.original_intensity { lum } else { level / 255.0 };
            let have = luma(c);
            if have > EPS {
                let k = target / have;
                c = c.map(|v| v * k);
                // Keep hue when the scale runs out of range: clip towards white proportionally.
                let mx = c[0].max(c[1]).max(c[2]);
                if mx > 1.0 {
                    let over = (luma(c) - 1.0).max(0.0);
                    c = c.map(|v| (v / mx).max(over));
                }
            } else {
                c = [target; 3];
            }
        }
        put(q, c);
    }
}

// ---------------------------------------------------------------------------------------------
// Dither

/// Dithering method of the Dither tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DitherMethod {
    None,
    FloydSteinberg,
    Bayer,
    Random,
    RandomCovariant,
    ArithmeticAdd,
    ArithmeticAddCovariant,
    ArithmeticXor,
    ArithmeticXorCovariant,
    BlueNoise,
}

impl DitherMethod {
    /// The method for a parameter id.
    pub fn from_id(id: &str) -> Option<DitherMethod> {
        Some(match id {
            "none" => DitherMethod::None,
            "floydSteinberg" => DitherMethod::FloydSteinberg,
            "bayer" => DitherMethod::Bayer,
            "random" => DitherMethod::Random,
            "randomCovariant" => DitherMethod::RandomCovariant,
            "arithmeticAdd" => DitherMethod::ArithmeticAdd,
            "arithmeticAddCovariant" => DitherMethod::ArithmeticAddCovariant,
            "arithmeticXor" => DitherMethod::ArithmeticXor,
            "arithmeticXorCovariant" => DitherMethod::ArithmeticXorCovariant,
            "blueNoise" => DitherMethod::BlueNoise,
            _ => return None,
        })
    }
}

/// Dither settings: levels per channel (2 and up) and the method.
#[derive(Clone, Debug, PartialEq)]
pub struct Dither {
    pub levels: [u32; 4],
    pub method: DitherMethod,
    pub seed: u32,
}

const BAYER8: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

const BLUE_NOISE_SIZE: usize = 64;

/// A tileable blue-noise threshold matrix (void-and-cluster, R. Ulichney 1993), built once.
fn blue_noise() -> &'static [f32] {
    static TILE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    TILE.get_or_init(|| {
        let n = BLUE_NOISE_SIZE;
        let total = n * n;
        // Gaussian energy lookup for toroidal offsets.
        let sigma = 1.9f32;
        let energy: Vec<f32> = (0..total)
            .map(|i| {
                let (dx, dy) = ((i % n) as i32, (i / n) as i32);
                let dx = dx.min(n as i32 - dx) as f32;
                let dy = dy.min(n as i32 - dy) as f32;
                (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp()
            })
            .collect();
        let mut field = vec![0.0f32; total];
        let mut set = vec![false; total];
        let mut rank = vec![0u32; total];
        let add = |field: &mut Vec<f32>, i: usize, sign: f32| {
            let (x, y) = (i % n, i / n);
            for j in 0..total {
                let (jx, jy) = (j % n, j / n);
                let dx = (jx + n - x) % n;
                let dy = (jy + n - y) % n;
                field[j] += sign * energy[dy * n + dx];
            }
        };
        // Initial pattern: a tenth of the points, placed randomly then relaxed.
        let initial = total / 10;
        let mut seedv = 0x5EED_1234u64;
        let mut placed = 0;
        while placed < initial {
            seedv = seedv.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let i = ((seedv >> 33) as usize) % total;
            if !set[i] {
                set[i] = true;
                add(&mut field, i, 1.0);
                placed += 1;
            }
        }
        for _ in 0..initial * 2 {
            // Move the tightest cluster point into the largest void.
            let cluster = (0..total).filter(|&i| set[i]).max_by(|&a, &b| field[a].total_cmp(&field[b])).unwrap_or(0);
            set[cluster] = false;
            add(&mut field, cluster, -1.0);
            let void = (0..total).filter(|&i| !set[i]).min_by(|&a, &b| field[a].total_cmp(&field[b])).unwrap_or(0);
            if void == cluster {
                set[cluster] = true;
                add(&mut field, cluster, 1.0);
                break;
            }
            set[void] = true;
            add(&mut field, void, 1.0);
        }
        // Phase 1: remove the tightest cluster points, ranking them downwards.
        let mut work_set = set.clone();
        let mut work_field = field.clone();
        let mut r = initial;
        while r > 0 {
            let cluster = (0..total).filter(|&i| work_set[i]).max_by(|&a, &b| work_field[a].total_cmp(&work_field[b])).unwrap_or(0);
            work_set[cluster] = false;
            add(&mut work_field, cluster, -1.0);
            r -= 1;
            rank[cluster] = r as u32;
        }
        // Phase 2: fill the largest voids, ranking upwards.
        let mut r = initial;
        while r < total {
            let void = (0..total).filter(|&i| !set[i]).min_by(|&a, &b| field[a].total_cmp(&field[b])).unwrap_or(0);
            set[void] = true;
            add(&mut field, void, 1.0);
            rank[void] = r as u32;
            r += 1;
        }
        rank.iter().map(|&r| (r as f32 + 0.5) / total as f32).collect()
    })
}

/// Quantises `v` to `levels` steps with threshold `mask` in 0..1 (0.5 = plain rounding).
#[inline]
fn quantize(v: f32, levels: u32, mask: f32) -> f32 {
    let steps = (levels.max(2) - 1) as f32;
    ((v * steps + mask).floor() / steps).clamp(0.0, 1.0)
}

/// Dither: reduces each channel to `levels` steps with the chosen method (alpha included).
pub fn dither(px: &mut [[f32; 4]], w: usize, o: &Dither) {
    if px.is_empty() || w == 0 {
        return;
    }
    let h = px.len() / w;
    let seed = o.seed;
    if o.method == DitherMethod::FloydSteinberg {
        // Error diffusion, raster order: 7/16 right, 3/16 down-left, 5/16 down, 1/16 down-right.
        let mut cur = vec![[0.0f32; 4]; w + 2];
        let mut next = vec![[0.0f32; 4]; w + 2];
        for y in 0..h {
            for x in 0..w {
                let q = &mut px[y * w + x];
                for (c, qc) in q.iter_mut().enumerate() {
                    let v = *qc + cur[x + 1][c];
                    let out = quantize(v, o.levels[c], 0.5);
                    let err = v - out;
                    *qc = out;
                    cur[x + 2][c] += err * 7.0 / 16.0;
                    next[x][c] += err * 3.0 / 16.0;
                    next[x + 1][c] += err * 5.0 / 16.0;
                    next[x + 2][c] += err * 1.0 / 16.0;
                }
            }
            std::mem::swap(&mut cur, &mut next);
            next.iter_mut().for_each(|e| *e = [0.0; 4]);
        }
        return;
    }
    let noise = blue_noise();
    let levels = o.levels;
    let method = o.method;
    par_rows(px, w, 1, move |y, row| {
        for (x, q) in row.iter_mut().enumerate() {
            let (xi, yi) = (x as i64, y as i64);
            for (c, qc) in q.iter_mut().enumerate() {
                let mask = match method {
                    DitherMethod::None | DitherMethod::FloydSteinberg => 0.5,
                    DitherMethod::Bayer => (f32::from(BAYER8[y % 8][x % 8]) + 0.5) / 64.0,
                    DitherMethod::Random => hash01(xi, yi, u64::from(seed) * 4 + c as u64 + 1),
                    DitherMethod::RandomCovariant => hash01(xi, yi, u64::from(seed) * 4 + 1),
                    DitherMethod::ArithmeticAdd | DitherMethod::ArithmeticAddCovariant => {
                        let cc = if method == DitherMethod::ArithmeticAdd { c as i64 } else { 0 };
                        ((((xi + cc * 67) + yi * 236) * 119) & 255) as f32 / 256.0
                    }
                    DitherMethod::ArithmeticXor | DitherMethod::ArithmeticXorCovariant => {
                        let cc = if method == DitherMethod::ArithmeticXor { c as i64 } else { 0 };
                        ((((xi + cc * 17) ^ (yi * 149)) * 1234) & 511) as f32 / 512.0
                    }
                    DitherMethod::BlueNoise => {
                        let (ox, oy) = (c * 21 + seed as usize % BLUE_NOISE_SIZE, c * 37);
                        noise[((y + oy) % BLUE_NOISE_SIZE) * BLUE_NOISE_SIZE + (x + ox) % BLUE_NOISE_SIZE]
                    }
                };
                *qc = quantize(*qc, levels[c], mask);
            }
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Local Threshold, Destripe, Retinex

/// Local Threshold: pixels brighter than their neighbourhood mean (radius `radius`) by more
/// than `high − 0.5` become white, darker by more than `0.5 − low` black, in between a ramp;
/// `antialias` 3×3 smoothing passes soften the result.
pub fn local_threshold(px: &mut [[f32; 4]], w: usize, h: usize, radius: f32, low: f32, high: f32, antialias: u32) {
    if px.is_empty() || w == 0 || h == 0 {
        return;
    }
    let grey: Vec<f32> = px.iter().map(|q| luma(rgb(*q))).collect();
    let mut mean = grey.clone();
    box_blur_n(&mut mean, w, h, 1, radius.max(0.0).round() as usize);
    let (lo, hi) = (low.min(high) - 0.5, high.max(low) - 0.5);
    let mut out: Vec<f32> = grey
        .iter()
        .zip(&mean)
        .map(|(g, m)| {
            let d = g - m;
            if hi - lo <= EPS { if d > hi { 1.0 } else { 0.0 } } else { ((d - lo) / (hi - lo)).clamp(0.0, 1.0) }
        })
        .collect();
    for _ in 0..antialias.min(16) {
        let src = out.clone();
        par_rows(&mut out, w, 1, |y, row| {
            for (x, v) in row.iter_mut().enumerate() {
                let mut acc = 0.0;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
                        let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
                        acc += src[sy * w + sx];
                    }
                }
                *v = acc / 9.0;
            }
        });
    }
    for (q, v) in px.iter_mut().zip(out) {
        put(q, [v, v, v]);
    }
}

/// Destripe: removes vertical stripes (scanner banding) by subtracting, per column, the
/// deviation of its mean colour from the running mean over `width` columns. `histogram` renders
/// the correction pattern instead of applying it.
pub fn destripe(px: &mut [[f32; 4]], w: usize, h: usize, width: u32, histogram: bool) {
    if px.is_empty() || w == 0 || h == 0 {
        return;
    }
    // Column means (opaque pixels).
    let mut mean = vec![[0.0f64; 3]; w];
    let mut count = vec![0u32; w];
    for (i, q) in px.iter().enumerate() {
        if q[3] <= 0.0 {
            continue;
        }
        let x = i % w;
        for (m, v) in mean[x].iter_mut().zip(q.iter()) {
            *m += f64::from(*v);
        }
        count[x] += 1;
    }
    let col: Vec<[f32; 3]> = mean.iter().zip(&count).map(|(m, &n)| if n > 0 { m.map(|v| (v / f64::from(n)) as f32) } else { [0.0; 3] }).collect();
    // Running mean over `width` columns (edges clamped).
    let r = (width.max(2) / 2) as i64;
    let smooth: Vec<[f32; 3]> = (0..w)
        .map(|x| {
            let mut acc = [0.0f32; 3];
            let mut n = 0.0;
            for d in -r..=r {
                let sx = (x as i64 + d).clamp(0, w as i64 - 1) as usize;
                if count[sx] == 0 {
                    continue;
                }
                for (a, v) in acc.iter_mut().zip(&col[sx]) {
                    *a += v;
                }
                n += 1.0;
            }
            if n > 0.0 { acc.map(|v| v / n) } else { col[x] }
        })
        .collect();
    let correction: Vec<[f32; 3]> = col.iter().zip(&smooth).map(|(c, s)| [s[0] - c[0], s[1] - c[1], s[2] - c[2]]).collect();
    par_rows(px, w, 1, |_, row| {
        for (x, q) in row.iter_mut().enumerate() {
            let k = correction[x];
            if histogram {
                put(q, [0.5 + k[0], 0.5 + k[1], 0.5 + k[2]]);
            } else {
                put(q, [q[0] + k[0], q[1] + k[1], q[2] + k[2]]);
            }
        }
    });
}

/// How Retinex spreads its scales between the smallest and `scale`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetinexLevel {
    Uniform,
    Low,
    High,
}

/// Retinex settings (Multi-Scale Retinex with Colour Restoration, D. J. Jobson, Z. Rahman,
/// G. A. Woodell, IEEE Trans. Image Processing 6(7), 1997).
#[derive(Clone, Debug, PartialEq)]
pub struct Retinex {
    pub level: RetinexLevel,
    pub scale: f32,
    pub scale_division: u32,
    pub dynamic: f32,
}

/// Multi-scale retinex: each channel's log ratio to its Gaussian surround, averaged over the
/// scales, with the colour restoration term, then stretched around its mean by `dynamic`
/// standard deviations.
pub fn retinex(px: &mut [[f32; 4]], w: usize, h: usize, o: &Retinex) {
    if px.is_empty() || w == 0 || h == 0 {
        return;
    }
    let n = o.scale_division.clamp(1, 8) as usize;
    let scales: Vec<f32> = (0..n)
        .map(|i| {
            let t = (i + 1) as f32 / n as f32;
            let t = match o.level {
                RetinexLevel::Uniform => t,
                RetinexLevel::Low => t * t,
                RetinexLevel::High => t.sqrt(),
            };
            (o.scale.max(1.0) * t).max(1.0)
        })
        .collect();
    let npx = px.len();
    // Channels as separate planes (1-based so the log is finite), 0..255 scale as the paper.
    let planes: Vec<Vec<f32>> = (0..3).map(|c| px.iter().map(|q| q[c] * 255.0 + 1.0).collect()).collect();
    let mut out = vec![[0.0f32; 3]; npx];
    for (c, plane) in planes.iter().enumerate() {
        let mut acc = vec![0.0f32; npx];
        for &s in &scales {
            let mut blurred = plane.clone();
            gauss_blur_n(&mut blurred, w, h, 1, s / 3.0);
            for ((a, p), b) in acc.iter_mut().zip(plane).zip(&blurred) {
                *a += (p.ln() - b.max(1.0).ln()) / n as f32;
            }
        }
        for (o3, a) in out.iter_mut().zip(&acc) {
            o3[c] = *a;
        }
    }
    // Colour restoration: C = β · (ln(α·I_c) − ln(Σ I)).
    const ALPHA: f32 = 125.0;
    const BETA: f32 = 46.0;
    for (i, o3) in out.iter_mut().enumerate() {
        let sum = planes[0][i] + planes[1][i] + planes[2][i];
        for (c, v) in o3.iter_mut().enumerate() {
            *v *= BETA * ((ALPHA * planes[c][i]).ln() - sum.ln());
        }
    }
    // Stretch around the mean.
    let count = out.len() as f64;
    let mean: f64 = out.iter().flat_map(|o3| o3.iter()).map(|&v| f64::from(v)).sum::<f64>() / (count * 3.0);
    let var: f64 = out.iter().flat_map(|o3| o3.iter()).map(|&v| (f64::from(v) - mean).powi(2)).sum::<f64>() / (count * 3.0);
    let std = var.sqrt().max(1e-9) as f32;
    let mean = mean as f32;
    let dyn_ = o.dynamic.max(0.01);
    let (lo, hi) = (mean - dyn_ * std, mean + dyn_ * std);
    for (q, o3) in px.iter_mut().zip(&out) {
        put(q, [(o3[0] - lo) / (hi - lo), (o3[1] - lo) / (hi - lo), (o3[2] - lo) / (hi - lo)]);
    }
}

// ---------------------------------------------------------------------------------------------
// STRESS and Colour to Gray

/// Settings of the envelope operators (Ø. Kolås, I. Farup, A. Rizzi, *Spatio-Temporal Retinex-
/// Inspired Envelope with Stochastic Sampling*, J. Imaging Sci. Technol. 55(4), 2011).
#[derive(Clone, Debug, PartialEq)]
pub struct Stress {
    /// Neighbourhood radius in pixels.
    pub radius: f32,
    /// Samples per iteration.
    pub samples: u32,
    /// Iterations (envelope estimates averaged).
    pub iterations: u32,
    /// Compute the envelopes on the encoded values (boosting shadow detail) instead of in linear light.
    pub enhance_shadows: bool,
    pub seed: u32,
}

/// Linear-light copy of the colours (or the encoded values when `encoded`).
fn working_colors(px: &[[f32; 4]], encoded: bool) -> Vec<[f32; 3]> {
    par_map(px.len(), |i| {
        let c = rgb(px[i]);
        if encoded { c } else { c.map(srgb_to_linear) }
    })
}

/// The minimum / maximum envelope of the neighbourhood of `(x, y)` for one iteration.
#[inline]
fn envelope(src: &[[f32; 3]], alpha: &[f32], size: (usize, usize), at: (usize, usize), o: &Stress, it: u32) -> ([f32; 3], [f32; 3]) {
    let (w, h) = size;
    let (x, y) = at;
    let p = src[y * w + x];
    let (mut mn, mut mx) = (p, p);
    for s in 0..o.samples.max(1) {
        let key = u64::from(o.seed) * 1_000_003 + u64::from(it) * 1009 + u64::from(s);
        let u = hash01(x as i64, y as i64, key * 2 + 1);
        let v = hash01(x as i64, y as i64, key * 2 + 2);
        // More samples near the pixel: a quadratic radius distribution.
        let r = o.radius.max(1.0) * u * u;
        let a = v * std::f32::consts::TAU;
        let sx = (x as f32 + 0.5 + r * a.cos()).floor() as i64;
        let sy = (y as f32 + 0.5 + r * a.sin()).floor() as i64;
        if !(0..w as i64).contains(&sx) || !(0..h as i64).contains(&sy) {
            continue;
        }
        let i = sy as usize * w + sx as usize;
        if alpha[i] <= 0.0 {
            continue;
        }
        let q = src[i];
        for ((lo, hi), v) in mn.iter_mut().zip(mx.iter_mut()).zip(q) {
            *lo = lo.min(v);
            *hi = hi.max(v);
        }
    }
    (mn, mx)
}

/// Stress: each channel is stretched between the minimum and maximum of its neighbourhood
/// envelope, averaged over the iterations (a local contrast and colour enhancer).
pub fn stress(px: &mut [[f32; 4]], w: usize, h: usize, o: &Stress) {
    if px.is_empty() || w == 0 || h == 0 {
        return;
    }
    let src = working_colors(px, o.enhance_shadows);
    let alpha: Vec<f32> = px.iter().map(|q| q[3]).collect();
    let iters = o.iterations.max(1);
    let encoded = o.enhance_shadows;
    par_rows(px, w, 1, |y, row| {
        for (x, q) in row.iter_mut().enumerate() {
            if q[3] <= 0.0 {
                continue;
            }
            let p = src[y * w + x];
            let mut acc = [0.0f32; 3];
            for it in 0..iters {
                let (mn, mx) = envelope(&src, &alpha, (w, h), (x, y), o, it);
                for (c, a) in acc.iter_mut().enumerate() {
                    let range = mx[c] - mn[c];
                    *a += if range > EPS { (p[c] - mn[c]) / range } else { 0.5 };
                }
            }
            let v = acc.map(|a| a / iters as f32);
            put(q, if encoded { v } else { v.map(linear_to_srgb) });
        }
    });
}

/// Colour to Gray: the grey of each pixel is its position along the axis from the darkest to
/// the brightest colour of its neighbourhood envelope, so colour differences become luminance
/// differences (local contrast is preserved).
pub fn color_to_gray(px: &mut [[f32; 4]], w: usize, h: usize, o: &Stress) {
    if px.is_empty() || w == 0 || h == 0 {
        return;
    }
    let src = working_colors(px, o.enhance_shadows);
    let alpha: Vec<f32> = px.iter().map(|q| q[3]).collect();
    let iters = o.iterations.max(1);
    let encoded = o.enhance_shadows;
    par_rows(px, w, 1, |y, row| {
        for (x, q) in row.iter_mut().enumerate() {
            if q[3] <= 0.0 {
                continue;
            }
            let p = src[y * w + x];
            let mut acc = 0.0f32;
            for it in 0..iters {
                let (mn, mx) = envelope(&src, &alpha, (w, h), (x, y), o, it);
                let d = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
                let len2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                acc += if len2 > EPS {
                    (((p[0] - mn[0]) * d[0] + (p[1] - mn[1]) * d[1] + (p[2] - mn[2]) * d[2]) / len2).clamp(0.0, 1.0)
                } else if encoded {
                    luma(p)
                } else {
                    luminance_linear(p)
                };
            }
            let g = acc / iters as f32;
            let g = if encoded { g } else { linear_to_srgb(g) };
            put(q, [g, g, g]);
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Border Average, Smooth Palette

/// Border Average: the most common colour (channels bucketed by `bucket_size` levels of 255) in
/// the strip `thickness` pixels wide just inside `(x0, y0)..(x1, y1)` of a `w`-wide image, as the
/// mean of the pixels in that bucket. `None` when the strip has no opaque pixel.
pub fn border_average(px: &[[f32; 4]], w: usize, area: (usize, usize, usize, usize), thickness: usize, bucket_size: u32) -> Option<[f32; 3]> {
    let (x0, y0, x1, y1) = area;
    if x1 <= x0 || y1 <= y0 || w == 0 {
        return None;
    }
    let t = thickness.max(1);
    let bucket = bucket_size.clamp(1, 256) as f32;
    let mut buckets: HashMap<[u16; 3], (u32, [f64; 3])> = HashMap::new();
    for y in y0..y1 {
        for x in x0..x1 {
            let inner = x >= x0 + t && x < x1.saturating_sub(t) && y >= y0 + t && y < y1.saturating_sub(t);
            if inner {
                continue;
            }
            let Some(q) = px.get(y * w + x) else { continue };
            if q[3] <= 0.0 {
                continue;
            }
            let key = [(q[0] * 255.0 / bucket) as u16, (q[1] * 255.0 / bucket) as u16, (q[2] * 255.0 / bucket) as u16];
            let e = buckets.entry(key).or_insert((0, [0.0; 3]));
            e.0 += 1;
            for (s, v) in e.1.iter_mut().zip(q.iter()) {
                *s += f64::from(*v);
            }
        }
    }
    let (count, sum) = buckets.values().max_by_key(|(n, _)| *n)?;
    Some(sum.map(|v| (v / f64::from(*count)) as f32))
}

/// Smooth Palette: `width` colours sampled from the image and ordered so neighbours are similar
/// (a nearest-neighbour chain improved by `search_depth` passes of local reversal), as the
/// columns of a `width × height` image.
pub fn smooth_palette(px: &[[f32; 4]], width: usize, height: usize, search_depth: u32, seed: u32) -> Vec<[f32; 4]> {
    let width = width.max(1);
    let height = height.max(1);
    let opaque: Vec<[f32; 3]> = px.iter().filter(|q| q[3] > 0.0).map(|q| rgb(*q)).collect();
    if opaque.is_empty() {
        return vec![[0.0, 0.0, 0.0, 1.0]; width * height];
    }
    let mut colors: Vec<[f32; 3]> = (0..width)
        .map(|i| {
            let u = hash01(i as i64, 0, u64::from(seed) + 7);
            opaque[((u * opaque.len() as f32) as usize).min(opaque.len() - 1)]
        })
        .collect();
    let dist = |a: [f32; 3], b: [f32; 3]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
    // Nearest-neighbour chain from the darkest colour.
    let mut order: Vec<[f32; 3]> = Vec::with_capacity(width);
    let start = (0..colors.len()).min_by(|&a, &b| luma(colors[a]).total_cmp(&luma(colors[b]))).unwrap_or(0);
    order.push(colors.swap_remove(start));
    while !colors.is_empty() {
        let last = order[order.len() - 1];
        let i = (0..colors.len()).min_by(|&a, &b| dist(last, colors[a]).total_cmp(&dist(last, colors[b]))).unwrap_or(0);
        order.push(colors.swap_remove(i));
    }
    // 2-opt passes: reverse a segment when that shortens the chain.
    let n = order.len();
    for _ in 0..search_depth {
        let mut improved = false;
        for i in 0..n.saturating_sub(2) {
            for j in i + 2..n {
                let before = dist(order[i], order[i + 1]) + if j + 1 < n { dist(order[j], order[j + 1]) } else { 0.0 };
                let after = dist(order[i], order[j]) + if j + 1 < n { dist(order[i + 1], order[j + 1]) } else { 0.0 };
                if after + 1e-7 < before {
                    order[i + 1..=j].reverse();
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
    let mut out = Vec::with_capacity(width * height);
    for _ in 0..height {
        for c in &order {
            out.push([c[0], c[1], c[2], 1.0]);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Tone mapping operators

/// Neumann-boundary Poisson solver on the pixel grid: finds `u` with `Σ_neighbours (u_n − u) = f`
/// (the 5-point Laplacian, missing neighbours at the border dropped) up to a constant, by a
/// geometric multigrid with red-black Gauss–Seidel smoothing.
fn poisson_neumann(w: usize, h: usize, f: &[f32]) -> Vec<f32> {
    let mean = f.iter().map(|&v| f64::from(v)).sum::<f64>() / f.len().max(1) as f64;
    let f: Vec<f32> = f.iter().map(|&v| v - mean as f32).collect();
    let mut u = vec![0.0f32; w * h];
    let cycles = 8;
    for _ in 0..cycles {
        vcycle(&mut u, &f, w, h, 0);
        let m = u.iter().map(|&v| f64::from(v)).sum::<f64>() / u.len().max(1) as f64;
        u.iter_mut().for_each(|v| *v -= m as f32);
    }
    u
}

/// One Gauss–Seidel sweep over a colour of the red-black ordering.
fn gs_sweep(u: &mut [f32], f: &[f32], w: usize, h: usize, parity: usize) {
    let snapshot = u.to_vec();
    par_rows(u, w, 1, |y, row| {
        for x in 0..w {
            if (x + y) % 2 != parity {
                continue;
            }
            let mut sum = 0.0f32;
            let mut deg = 0.0f32;
            if x > 0 {
                sum += snapshot[y * w + x - 1];
                deg += 1.0;
            }
            if x + 1 < w {
                sum += snapshot[y * w + x + 1];
                deg += 1.0;
            }
            if y > 0 {
                sum += snapshot[(y - 1) * w + x];
                deg += 1.0;
            }
            if y + 1 < h {
                sum += snapshot[(y + 1) * w + x];
                deg += 1.0;
            }
            if deg > 0.0 {
                row[x] = (sum - f[y * w + x]) / deg;
            }
        }
    });
}

fn laplacian_residual(u: &[f32], f: &[f32], w: usize, h: usize) -> Vec<f32> {
    par_map(w * h, |i| {
        let (x, y) = (i % w, i / w);
        let c = u[i];
        let mut l = 0.0f32;
        if x > 0 {
            l += u[i - 1] - c;
        }
        if x + 1 < w {
            l += u[i + 1] - c;
        }
        if y > 0 {
            l += u[i - w] - c;
        }
        if y + 1 < h {
            l += u[i + w] - c;
        }
        f[i] - l
    })
}

fn vcycle(u: &mut [f32], f: &[f32], w: usize, h: usize, depth: usize) {
    let smooth = |u: &mut [f32], n: usize| {
        for _ in 0..n {
            gs_sweep(u, f, w, h, 0);
            gs_sweep(u, f, w, h, 1);
        }
    };
    if w <= 4 || h <= 4 || depth >= 12 {
        smooth(u, 40);
        return;
    }
    smooth(u, 3);
    let r = laplacian_residual(u, f, w, h);
    let (w2, h2) = (w.div_ceil(2), h.div_ceil(2));
    // Full-weighting restriction (2×2 block mean); the coarse stencil spans twice the distance,
    // so the coarse right-hand side is four times the restricted residual.
    let r2: Vec<f32> = par_map(w2 * h2, |i| {
        let (x, y) = (i % w2, i / w2);
        let mut acc = 0.0;
        let mut n = 0.0;
        for dy in 0..2 {
            for dx in 0..2 {
                let (sx, sy) = (2 * x + dx, 2 * y + dy);
                if sx < w && sy < h {
                    acc += r[sy * w + sx];
                    n += 1.0;
                }
            }
        }
        4.0 * acc / n.max(1.0)
    });
    let mut e2 = vec![0.0f32; w2 * h2];
    vcycle(&mut e2, &r2, w2, h2, depth + 1);
    // Bilinear prolongation and correction.
    par_rows(u, w, 1, |y, row| {
        let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (h2 - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy - fy.floor());
        let y1 = (y0 + 1).min(h2 - 1);
        for (x, v) in row.iter_mut().enumerate() {
            let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (w2 - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx - fx.floor());
            let x1 = (x0 + 1).min(w2 - 1);
            let top = e2[y0 * w2 + x0] * (1.0 - tx) + e2[y0 * w2 + x1] * tx;
            let bot = e2[y1 * w2 + x0] * (1.0 - tx) + e2[y1 * w2 + x1] * tx;
            *v += top * (1.0 - ty) + bot * ty;
        }
    });
    smooth(u, 3);
}

/// Forward-difference gradients of a `w × h` field (zero at the far edges).
fn gradients(a: &[f32], w: usize, h: usize) -> (Vec<f32>, Vec<f32>) {
    let gx = par_map(w * h, |i| {
        let x = i % w;
        if x + 1 < w { a[i + 1] - a[i] } else { 0.0 }
    });
    let gy = par_map(w * h, |i| {
        let y = i / w;
        if y + 1 < h { a[i + w] - a[i] } else { 0.0 }
    });
    (gx, gy)
}

/// Divergence of a forward-difference field, consistent with [`gradients`] and the solver's
/// Neumann Laplacian.
fn divergence(gx: &[f32], gy: &[f32], w: usize, h: usize) -> Vec<f32> {
    par_map(w * h, |i| {
        let (x, y) = (i % w, i / w);
        let mut d = gx[i] + gy[i];
        if x > 0 {
            d -= gx[i - 1];
        }
        if y > 0 {
            d -= gy[i - w];
        }
        d
    })
}

/// Central-difference gradient magnitude of a pyramid level `k` (spacing `2^(k+1)`).
fn level_gradient_magnitude(l: &Level, k: usize) -> Vec<f32> {
    let (w, h) = (l.w, l.h);
    let scale = 1.0 / (1u32 << (k + 1)) as f32;
    par_map(w * h, |i| {
        let (x, y) = (i % w, i / w);
        let xm = l.px[y * w + x.saturating_sub(1)];
        let xp = l.px[y * w + (x + 1).min(w - 1)];
        let ym = l.px[y.saturating_sub(1) * w + x];
        let yp = l.px[(y + 1).min(h - 1) * w + x];
        ((xp - xm).powi(2) + (yp - ym).powi(2)).sqrt() * scale
    })
}

/// Linear-light luminance of the pixels (with a floor), log luminance and per-pixel colour ratios.
fn log_luminance(px: &[[f32; 4]]) -> (Vec<f32>, Vec<[f32; 3]>) {
    let lin: Vec<[f32; 3]> = par_map(px.len(), |i| rgb(px[i]).map(srgb_to_linear));
    let lum: Vec<f32> = lin.iter().map(|c| luminance_linear(*c).max(1e-5)).collect();
    (lum, lin)
}

/// Writes the new linear luminance back, keeping the colour ratios raised to `saturation`
/// (C_out = (C_in / L_in)^s · L_out), then encodes to sRGB.
fn recolor(px: &mut [[f32; 4]], lin: &[[f32; 3]], lum_in: &[f32], lum_out: &[f32], saturation: f32) {
    par_rows(px, 1, 1, |i, row| {
        let q = &mut row[0];
        if q[3] <= 0.0 {
            return;
        }
        let c = lin[i];
        let (li, lo) = (lum_in[i], lum_out[i].max(0.0));
        let out = c.map(|v| ((v / li).max(0.0)).powf(saturation) * lo);
        put(q, out.map(|v| linear_to_srgb(v.clamp(0.0, 1.0))));
    });
}

/// The `lo..hi` percentile range of `v` (0..1 fractions), from a 4096-bin histogram.
fn percentile_range(v: &[f32], lo: f32, hi: f32) -> (f32, f32) {
    let (mut mn, mut mx) = (f32::MAX, f32::MIN);
    for &x in v {
        if x.is_finite() {
            mn = mn.min(x);
            mx = mx.max(x);
        }
    }
    if mx <= mn {
        return (mn.min(mx), mx.max(mn) + 1e-6);
    }
    const BINS: usize = 4096;
    let mut hist = vec![0u32; BINS];
    for &x in v.iter().filter(|x| x.is_finite()) {
        let b = (((x - mn) / (mx - mn)) * (BINS - 1) as f32) as usize;
        hist[b.min(BINS - 1)] += 1;
    }
    let total: u64 = hist.iter().map(|&c| u64::from(c)).sum();
    let at = |p: f32| {
        let target = (f64::from(p) * total as f64) as u64;
        let mut acc = 0u64;
        for (i, &c) in hist.iter().enumerate() {
            acc += u64::from(c);
            if acc >= target {
                return mn + (i as f32 + 0.5) / BINS as f32 * (mx - mn);
            }
        }
        mx
    };
    let (a, b) = (at(lo), at(hi));
    if b > a { (a, b) } else { (mn, mx) }
}

/// Fattal et al. 2002 settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Fattal {
    /// Gradient threshold scale (α is this times a tenth of the mean gradient magnitude).
    pub alpha: f32,
    /// Attenuation exponent (β < 1 compresses large gradients).
    pub beta: f32,
    /// Colour saturation exponent.
    pub saturation: f32,
    /// Gradients below this magnitude are left alone.
    pub noise: f32,
}

impl Default for Fattal {
    fn default() -> Self {
        Fattal { alpha: 1.0, beta: 0.9, saturation: 0.8, noise: 0.0 }
    }
}

/// Gradient-domain high-dynamic-range compression (R. Fattal, D. Lischinski, M. Werman,
/// *Gradient Domain High Dynamic Range Compression*, SIGGRAPH 2002): the log-luminance gradients
/// are attenuated by `φ = (α/|∇H|)·(|∇H|/α)^β`, built over a Gaussian pyramid so large
/// gradients shrink at every scale, and the result is recovered by solving a Poisson equation.
pub fn fattal(px: &mut [[f32; 4]], w: usize, h: usize, o: &Fattal) {
    if px.is_empty() || w < 2 || h < 2 {
        return;
    }
    let (lum, lin) = log_luminance(px);
    let logl: Vec<f32> = lum.iter().map(|l| l.ln()).collect();
    let levels = pyramid::auto_levels(w, h).max(1);
    let pyr = pyramid::gaussian(w, h, 1, &logl, levels);
    let beta = o.beta.clamp(0.1, 2.0);
    // Attenuation from the coarsest level down.
    let mut phi: Option<Vec<f32>> = None;
    for k in (0..pyr.len()).rev() {
        let l = &pyr[k];
        let mag = level_gradient_magnitude(l, k);
        let mean = mag.iter().map(|&v| f64::from(v)).sum::<f64>() / mag.len().max(1) as f64;
        let alpha = (0.1 * o.alpha.max(0.0) * mean as f32).max(1e-4);
        let own: Vec<f32> = mag.iter().map(|&g| if g < o.noise || g <= EPS { 1.0 } else { ((alpha / g) * (g / alpha).powf(beta)).clamp(0.0, 1.0) }).collect();
        let combined = match phi.take() {
            None => own,
            Some(coarse) => {
                let up = pyramid::expand(&Level { w: pyr[k + 1].w, h: pyr[k + 1].h, px: coarse }, 1, l.w, l.h);
                own.iter().zip(&up).map(|(a, b)| a * b.clamp(0.0, 1.0)).collect()
            }
        };
        phi = Some(combined);
    }
    let phi = phi.unwrap_or_else(|| vec![1.0; w * h]);
    let (mut gx, mut gy) = gradients(&logl, w, h);
    for ((x, y), p) in gx.iter_mut().zip(gy.iter_mut()).zip(&phi) {
        *x *= p;
        *y *= p;
    }
    let div = divergence(&gx, &gy, w, h);
    let u = poisson_neumann(w, h, &div);
    let out: Vec<f32> = u.iter().map(|v| v.exp()).collect();
    let (lo, hi) = percentile_range(&out, 0.001, 0.999);
    let lum_out: Vec<f32> = out.iter().map(|v| ((v - lo) / (hi - lo)).clamp(0.0, 1.0)).collect();
    recolor(px, &lin, &lum, &lum_out, o.saturation.clamp(0.0, 2.0));
}

/// Mantiuk 2006 settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Mantiuk {
    /// Contrast compression (0 none, 1 strongest).
    pub contrast: f32,
    /// Colour saturation exponent.
    pub saturation: f32,
    /// Emphasis on the finest-scale gradients (1–99).
    pub detail: f32,
}

impl Default for Mantiuk {
    fn default() -> Self {
        Mantiuk { contrast: 0.1, saturation: 0.8, detail: 1.0 }
    }
}

/// Contrast-domain tone mapping after R. Mantiuk, K. Myszkowski, H.-P. Seidel, *A Perceptual
/// Framework for Contrast Processing of High Dynamic Range Images*, ACM TAP 3(3), 2006: log-
/// luminance contrasts are measured at every pyramid scale, the coarse (large-area) contrasts are
/// compressed by the response transducer `R = 54.09·|G|^0.4185` scaled by the contrast setting,
/// the finest contrasts are amplified by the detail setting, and the image is rebuilt by solving
/// a Poisson equation on the modified gradients.
pub fn mantiuk(px: &mut [[f32; 4]], w: usize, h: usize, o: &Mantiuk) {
    if px.is_empty() || w < 2 || h < 2 {
        return;
    }
    let (lum, lin) = log_luminance(px);
    let logl: Vec<f32> = lum.iter().map(|l| l.ln() / std::f32::consts::LN_10).collect();
    let levels = pyramid::auto_levels(w, h).max(1);
    let pyr = pyramid::gaussian(w, h, 1, &logl, levels);
    let contrast = o.contrast.clamp(0.0, 1.0);
    let detail = 1.0 + (o.detail.clamp(1.0, 99.0) - 1.0) / 98.0;
    // Per-scale gain: the transducer compresses large contrasts more than small ones; coarser
    // scales (where the dynamic range lives) are compressed harder.
    let mut phi: Option<Vec<f32>> = None;
    let n = pyr.len() as f32;
    for k in (0..pyr.len()).rev() {
        let l = &pyr[k];
        let mag = level_gradient_magnitude(l, k);
        let strength = contrast * (k as f32 + 1.0) / n;
        let own: Vec<f32> = mag
            .iter()
            .map(|&g| {
                if g <= EPS {
                    return 1.0;
                }
                let response = 54.09288 * g.powf(0.41850);
                let compressed = response * (1.0 - strength);
                let g2 = (compressed / 54.09288).powf(1.0 / 0.41850);
                (g2 / g).clamp(0.0, 1.0)
            })
            .collect();
        let combined = match phi.take() {
            None => own,
            Some(coarse) => {
                let up = pyramid::expand(&Level { w: pyr[k + 1].w, h: pyr[k + 1].h, px: coarse }, 1, l.w, l.h);
                own.iter().zip(&up).map(|(a, b)| a * b.clamp(0.0, 1.0)).collect()
            }
        };
        phi = Some(combined);
    }
    let phi = phi.unwrap_or_else(|| vec![1.0; w * h]);
    let (mut gx, mut gy) = gradients(&logl, w, h);
    // Fine detail: the part of the gradient the first smoothing removes is amplified.
    let base = if pyr.len() > 1 { pyramid::expand(&pyr[1], 1, w, h) } else { logl.clone() };
    let (bx, by) = gradients(&base, w, h);
    for (i, p) in phi.iter().enumerate() {
        let (x, y) = (gx[i], gy[i]);
        gx[i] = (bx[i] + (x - bx[i]) * detail) * p;
        gy[i] = (by[i] + (y - by[i]) * detail) * p;
    }
    let div = divergence(&gx, &gy, w, h);
    let u = poisson_neumann(w, h, &div);
    let out: Vec<f32> = u.iter().map(|v| 10f32.powf(*v)).collect();
    let (lo, hi) = percentile_range(&out, 0.001, 0.999);
    let lum_out: Vec<f32> = out.iter().map(|v| ((v - lo) / (hi - lo)).clamp(0.0, 1.0)).collect();
    recolor(px, &lin, &lum, &lum_out, o.saturation.clamp(0.0, 2.0));
}

/// Reinhard 2005 settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Reinhard {
    /// Overall brightness (−8..8, exposure-like).
    pub brightness: f32,
    /// Chromatic adaptation: 0 adapts to luminance, 1 to each channel independently.
    pub chromatic: f32,
    /// Light adaptation: 0 global, 1 local.
    pub light: f32,
}

impl Default for Reinhard {
    fn default() -> Self {
        Reinhard { brightness: 0.0, chromatic: 0.0, light: 1.0 }
    }
}

/// Photoreceptor tone mapping (E. Reinhard, K. Devlin, *Dynamic Range Reduction Inspired by
/// Photoreceptor Physiology*, IEEE TVCG 11(1), 2005): `V = I / (I + σ(I_a))` with
/// `σ(I_a) = (f·I_a)^m`, the adaptation level `I_a` interpolating between the pixel and the global
/// mean (light adaptation) and between the channel and luminance (chromatic adaptation).
pub fn reinhard(px: &mut [[f32; 4]], w: usize, h: usize, o: &Reinhard) {
    if px.is_empty() || w == 0 || h == 0 {
        return;
    }
    let (lum, lin) = log_luminance(px);
    let n = lum.len() as f64;
    let (mut lmin, mut lmax, mut lsum, mut csum) = (f64::MAX, f64::MIN, 0.0f64, [0.0f64; 3]);
    for (l, c) in lum.iter().zip(&lin) {
        let ll = f64::from(*l).ln();
        lmin = lmin.min(ll);
        lmax = lmax.max(ll);
        lsum += ll;
        for (s, v) in csum.iter_mut().zip(c.iter()) {
            *s += f64::from(*v);
        }
    }
    let lav = lsum / n;
    let key = if lmax - lmin > 1e-9 { ((lmax - lav) / (lmax - lmin)).clamp(0.0, 1.0) } else { 0.5 };
    let m = (0.3 + 0.7 * key.powf(1.4)) as f32;
    let f = (-o.brightness).exp();
    let lmean = (lav.exp()) as f32;
    let cmean = csum.map(|v| (v / n) as f32);
    // Local adaptation: a wide Gaussian of the luminance and of each channel.
    let sigma = (w.min(h) as f32 * 0.02).max(1.0);
    let mut local_l = lum.clone();
    gauss_blur_n(&mut local_l, w, h, 1, sigma);
    let mut local_c: Vec<f32> = lin.iter().flat_map(|c| *c).collect();
    gauss_blur_n(&mut local_c, w, h, 3, sigma);
    let (chromatic, light) = (o.chromatic.clamp(0.0, 1.0), o.light.clamp(0.0, 1.0));
    let out: Vec<[f32; 3]> = par_map(px.len(), |i| {
        let c = lin[i];
        let mut v = [0.0f32; 3];
        for (k, vk) in v.iter_mut().enumerate() {
            let ia_local = chromatic * local_c[i * 3 + k] + (1.0 - chromatic) * local_l[i];
            let ia_global = chromatic * cmean[k] + (1.0 - chromatic) * lmean;
            let ia = light * ia_local + (1.0 - light) * ia_global;
            let sigma_a = (f * ia.max(1e-6)).powf(m);
            *vk = c[k] / (c[k] + sigma_a);
        }
        v
    });
    let flat: Vec<f32> = out.iter().flat_map(|c| *c).collect();
    let (lo, hi) = percentile_range(&flat, 0.001, 0.999);
    for (q, c) in px.iter_mut().zip(&out) {
        if q[3] <= 0.0 {
            continue;
        }
        put(q, c.map(|v| linear_to_srgb(((v - lo) / (hi - lo)).clamp(0.0, 1.0))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: usize, h: usize) -> Vec<[f32; 4]> {
        (0..w * h)
            .map(|i| {
                let x = (i % w) as f32 / (w - 1) as f32;
                let y = (i / w) as f32 / (h - 1).max(1) as f32;
                [x, 0.5 + 0.5 * y * x, 1.0 - x, 1.0]
            })
            .collect()
    }

    #[test]
    fn models_round_trip() {
        for m in MODELS {
            for c in [[0.2, 0.7, 0.4, 1.0], [0.9, 0.1, 0.3, 0.5], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]] {
                let v = m.decompose(c);
                let back = m.compose(v, c[3]);
                if m == Model::Alpha {
                    assert!((back[3] - c[3]).abs() < 1e-3, "{m:?}");
                    continue;
                }
                for (b, a) in back.iter().zip(c.iter()).take(3) {
                    assert!((b - a).abs() < 0.02, "{m:?}: {c:?} -> {v:?} -> {back:?}");
                }
            }
        }
    }

    #[test]
    fn inversions() {
        let mut px = vec![[0.0, 0.5, 1.0, 1.0]];
        linear_invert(&mut px);
        assert!((px[0][0] - 1.0).abs() < 1e-5 && px[0][2].abs() < 1e-5);
        let mut px = vec![[1.0, 0.0, 0.0, 1.0]];
        value_invert(&mut px);
        assert_eq!(px[0], [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn stretch_fills_the_range() {
        let mut px = vec![[0.4, 0.3, 0.3, 1.0], [0.6, 0.5, 0.5, 1.0]];
        stretch_contrast_hsv(&mut px);
        let a = rgb_to_hsb(rgb(px[0]));
        let b = rgb_to_hsb(rgb(px[1]));
        assert!(a[2] < 0.01 && b[2] > 0.99, "{px:?}");
    }

    #[test]
    fn dither_methods_quantise() {
        for id in ["none", "floydSteinberg", "bayer", "random", "randomCovariant", "arithmeticAdd", "arithmeticXor", "blueNoise"] {
            let mut px = ramp(16, 8);
            dither(&mut px, 16, &Dither { levels: [2, 2, 2, 256], method: DitherMethod::from_id(id).unwrap(), seed: 1 });
            assert!(px.iter().all(|q| (0..3).all(|c| q[c] == 0.0 || q[c] == 1.0)), "{id}");
        }
    }

    #[test]
    fn poisson_recovers_a_field() {
        let (w, h) = (24, 17);
        let field: Vec<f32> = (0..w * h).map(|i| ((i % w) as f32 * 0.3).sin() + ((i / w) as f32 * 0.2).cos()).collect();
        let (gx, gy) = gradients(&field, w, h);
        let div = divergence(&gx, &gy, w, h);
        let u = poisson_neumann(w, h, &div);
        let fm = field.iter().sum::<f32>() / field.len() as f32;
        let um = u.iter().sum::<f32>() / u.len() as f32;
        for (i, (uv, fv)) in u.iter().zip(&field).enumerate() {
            assert!(((uv - um) - (fv - fm)).abs() < 0.05, "{i}: {} vs {}", uv - um, fv - fm);
        }
    }

    #[test]
    fn tone_mappers_keep_range() {
        for op in 0..3 {
            let mut px = ramp(32, 20);
            match op {
                0 => fattal(&mut px, 32, 20, &Fattal::default()),
                1 => mantiuk(&mut px, 32, 20, &Mantiuk::default()),
                _ => reinhard(&mut px, 32, 20, &Reinhard::default()),
            }
            assert!(px.iter().all(|q| q.iter().all(|v| (0.0..=1.0).contains(v))), "{op}");
        }
    }

    #[test]
    fn border_average_finds_the_frame_colour() {
        let w = 10;
        let mut px = vec![[0.0, 0.0, 1.0, 1.0]; w * w];
        for y in 2..8 {
            for x in 2..8 {
                px[y * w + x] = [1.0, 0.0, 0.0, 1.0];
            }
        }
        let c = border_average(&px, w, (0, 0, w, w), 2, 16).unwrap();
        assert!(c[2] > 0.99 && c[0] < 0.01, "{c:?}");
    }
}
