// SPDX-License-Identifier: MIT OR Apache-2.0
//! Colour-management settings beyond Edit › Color Settings' working spaces: the display profile
//! settings (a profile per screen, the canvas surface encoding and bit depth, unmanaged mode),
//! what profile pasted pixels from another application are assumed to be in, and Enforce
//! Palette Colours. All of it lives in [`ColorSettings`](crate::color_cmds::ColorSettings), so it
//! persists with the preferences, and `edit.colorManagement` reads or changes it.
//!
//! The HDR canvas surface: with `surface` set to Rec. 2020 PQ (or scRGB linear), the display LUT
//! ends with that encoding, so a window presented on an HDR surface of that format shows a float
//! document's values above 1.0 as brighter light instead of clipping. The shell's window surface
//! is what the platform gives it (see the parity note on PT-CMS-008); the transform and the
//! settings are here.

use redrob_photo_cms::Transform;
use redrob_photo_color::ColorMode;
use redrob_photo_color::models::{hlg_encode, pq_encode};
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::color_cmds::ColorState;
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The encoding of the canvas surface the display LUT ends with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplaySurface {
    /// The monitor profile's own encoding (sRGB-like), 8 or 10 bits.
    #[default]
    Srgb,
    /// Rec. 709 (the same primaries as sRGB, the BT.1886 / Rec. 709 curve).
    Rec709,
    /// Rec. 2020 primaries with the PQ (SMPTE ST 2084) curve: an HDR surface; 1.0 = `sdr_nits`.
    Rec2020Pq,
    /// Rec. 2020 primaries with the hybrid log-gamma curve.
    Rec2020Hlg,
    /// scRGB: linear light with sRGB primaries, half-float surfaces.
    ScrgbLinear,
}

impl DisplaySurface {
    pub const ALL: [DisplaySurface; 5] =
        [DisplaySurface::Srgb, DisplaySurface::Rec709, DisplaySurface::Rec2020Pq, DisplaySurface::Rec2020Hlg, DisplaySurface::ScrgbLinear];

    pub fn id(self) -> &'static str {
        match self {
            DisplaySurface::Srgb => "srgb",
            DisplaySurface::Rec709 => "rec709",
            DisplaySurface::Rec2020Pq => "rec2020-pq",
            DisplaySurface::Rec2020Hlg => "rec2020-hlg",
            DisplaySurface::ScrgbLinear => "scrgb-linear",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let k = s.trim().to_ascii_lowercase().replace('_', "-");
        Self::ALL.into_iter().find(|d| d.id() == k)
    }

    pub fn label(self) -> &'static str {
        match self {
            DisplaySurface::Srgb => "sRGB (monitor profile)",
            DisplaySurface::Rec709 => "Rec. 709",
            DisplaySurface::Rec2020Pq => "Rec. 2020 PQ (HDR)",
            DisplaySurface::Rec2020Hlg => "Rec. 2020 HLG (HDR)",
            DisplaySurface::ScrgbLinear => "scRGB linear (HDR)",
        }
    }

    pub fn is_default(self) -> bool {
        self == DisplaySurface::Srgb
    }

    pub fn is_hdr(self) -> bool {
        matches!(self, DisplaySurface::Rec2020Pq | DisplaySurface::Rec2020Hlg | DisplaySurface::ScrgbLinear)
    }

    /// The bit depths the surface supports.
    pub fn bit_depths(self) -> &'static [u8] {
        match self {
            DisplaySurface::Srgb | DisplaySurface::Rec709 => &[8, 10],
            DisplaySurface::Rec2020Pq | DisplaySurface::Rec2020Hlg => &[10, 12],
            DisplaySurface::ScrgbLinear => &[16],
        }
    }

    /// Re-encode a display-encoded (sRGB-like, 0..1) colour for this surface, with SDR white at
    /// `sdr_nits`.
    pub fn encode_with(self, c: [f32; 3], sdr_nits: f32) -> [f32; 3] {
        match self {
            DisplaySurface::Srgb => c,
            DisplaySurface::Rec709 => c.map(|v| {
                let lin = redrob_photo_color::convert::srgb_to_linear(v.clamp(0.0, 1.0));
                if lin < 0.018_053_97 { lin * 4.5 } else { 1.099_297 * lin.powf(0.45) - 0.099_297 }
            }),
            DisplaySurface::Rec2020Pq => {
                let lin = srgb_to_rec2020_linear(c);
                lin.map(|v| pq_encode(v.max(0.0) * sdr_nits.clamp(1.0, 10000.0)))
            }
            DisplaySurface::Rec2020Hlg => {
                let lin = srgb_to_rec2020_linear(c);
                // HLG reference white sits at 75 % of the signal; scale SDR white to it.
                lin.map(|v| hlg_encode(v.max(0.0) * (sdr_nits.clamp(1.0, 10000.0) / 1000.0) * 12.0 / 12.0))
            }
            DisplaySurface::ScrgbLinear => c.map(|v| redrob_photo_color::convert::srgb_to_linear(v.clamp(0.0, 1.0))),
        }
    }

    /// [`encode_with`](Self::encode_with) at the reference 80 nits for SDR white.
    pub fn encode(self, c: [f32; 3]) -> [f32; 3] {
        self.encode_with(c, 80.0)
    }
}

/// sRGB-encoded → Rec. 2020 linear (both D65).
fn srgb_to_rec2020_linear(c: [f32; 3]) -> [f32; 3] {
    let lin = c.map(|v| redrob_photo_color::convert::srgb_to_linear(v.clamp(0.0, 1.0)));
    // sRGB → XYZ → Rec. 2020, combined.
    const M: [[f32; 3]; 3] = [[0.627_404, 0.329_283, 0.043_313], [0.069_097, 0.919_540, 0.011_362], [0.016_391, 0.088_013, 0.895_595]];
    redrob_photo_color::convert::mat3_mul(&M, lin)
}

/// Display profile settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DisplaySettings {
    /// Profile per screen (display id → built-in profile id or `.icc` path); a screen not listed
    /// uses Color Settings › Monitor Profile.
    pub screen_profiles: BTreeMap<String, String>,
    /// The canvas surface the display LUT ends with.
    pub surface: DisplaySurface,
    /// Bits per channel of the surface (8, 10, 12 or 16, as the surface allows).
    pub bit_depth: u8,
    /// The luminance SDR white (1.0) is shown at on an HDR surface, in nits.
    pub sdr_nits: f32,
    /// Unmanaged: the composite's numbers reach the screen as they are (no monitor transform).
    pub unmanaged: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self { screen_profiles: BTreeMap::new(), surface: DisplaySurface::Srgb, bit_depth: 8, sdr_nits: 80.0, unmanaged: false }
    }
}

impl DisplaySettings {
    /// The profile chosen for `display`, if any.
    pub fn screen_profile(&self, display: Option<u32>) -> Option<&str> {
        let id = display?;
        self.screen_profiles.get(&id.to_string()).map(String::as_str).filter(|s| !s.is_empty() && *s != "auto")
    }
}

/// What a pasted image from another application is assumed to be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PasteAssume {
    /// sRGB (what most applications put on the clipboard).
    #[default]
    Srgb,
    /// The monitor profile (the pixels looked right on this screen).
    Monitor,
    /// The working RGB space.
    Working,
    /// Ask every time (the shell shows a prompt; the engine pastes as sRGB meanwhile).
    Ask,
}

impl PasteAssume {
    pub const ALL: [PasteAssume; 4] = [PasteAssume::Srgb, PasteAssume::Monitor, PasteAssume::Working, PasteAssume::Ask];
    pub fn id(self) -> &'static str {
        match self {
            PasteAssume::Srgb => "srgb",
            PasteAssume::Monitor => "monitor",
            PasteAssume::Working => "working",
            PasteAssume::Ask => "ask",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == s.trim().to_ascii_lowercase())
    }
}

/// Is a stage of this module changing the display LUT (so the plain profile LUT can't be used)?
pub fn display_stage_active(c: &ColorState) -> bool {
    !c.settings.display.surface.is_default() || c.settings.display.unmanaged || crate::ocio::active(c).is_some()
}

/// The profile pasted pixels from another application are assumed to be in (`None`: ask, or
/// the document is not RGB).
pub fn assumed_paste_profile(s: &Session, doc_mode: ColorMode) -> Option<std::sync::Arc<redrob_photo_cms::Profile>> {
    if doc_mode != ColorMode::Rgb {
        return None;
    }
    match s.color.settings.paste_assume {
        PasteAssume::Srgb => Some(std::sync::Arc::new(redrob_photo_cms::Builtin::Srgb.profile().clone())),
        PasteAssume::Monitor => Some(s.color.monitor()),
        PasteAssume::Working => Some(s.color.working(ColorMode::Rgb)),
        PasteAssume::Ask => None,
    }
}

/// Convert pasted pixels from another application into the document's RGB profile, from the
/// assumed profile (`pasteAssume`). Pixels copied inside the app (a clip with a source document
/// or a resolution) are already in a known space and are returned as they are; so is a clip when
/// the assumed and document profiles show the same colours.
pub fn prepare_pasted(s: &Session, clip: &crate::edit_cmds::Clip, surf: Surface, doc: &redrob_photo_doc::Document) -> Surface {
    if clip.layers.is_some() || clip.resolution.is_some() || doc.mode != ColorMode::Rgb || surf.format().mode != ColorMode::Rgb {
        return surf;
    }
    let Some(src) = assumed_paste_profile(s, doc.mode) else { return surf };
    let dst = crate::color_cmds::document_profile(doc);
    if src.same_colors(&dst) {
        return surf;
    }
    let Ok(t) = Transform::new(&src, &dst, s.color.settings.intent(), s.color.settings.bpc) else { return surf };
    crate::color_cmds::convert_surface(&surf, ColorMode::Rgb, surf.format(), &t)
}

/// The nearest library swatch to `rgb` (sRGB), as the RGB it shows, when Enforce Palette
/// Colours is on; `None` otherwise or with an empty library.
pub fn nearest_palette_color(s: &Session, rgb: [f32; 3]) -> Option<[f32; 3]> {
    if !s.color.settings.enforce_palette {
        return None;
    }
    let conv = crate::presets::swatches::SwatchConv::new(s);
    let lab = redrob_photo_color::convert::srgb_to_lab(rgb);
    let mut best: Option<(f32, [f32; 3])> = None;
    for g in &s.presets.swatches {
        for sw in &g.items {
            let c = conv.rgb(&sw.color);
            let l = redrob_photo_color::convert::srgb_to_lab(c);
            let d = (l[0] - lab[0]).powi(2) + (l[1] - lab[1]).powi(2) + (l[2] - lab[2]).powi(2);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, c));
            }
        }
    }
    best.map(|(_, c)| c)
}

/// Snap the tool colours to the palette when Enforce Palette Colours is on (called after a
/// colour is set). Returns whether either colour moved.
pub fn enforce_palette(s: &mut Session) -> bool {
    if !s.color.settings.enforce_palette {
        return false;
    }
    let mut moved = false;
    for which in 0..2 {
        let cur = if which == 0 { s.tools.foreground } else { s.tools.background };
        if let Some(n) = nearest_palette_color(s, [cur[0], cur[1], cur[2]]) {
            let next = [n[0], n[1], n[2], cur[3]];
            if next != cur {
                if which == 0 {
                    s.tools.foreground = next;
                } else {
                    s.tools.background = next;
                }
                moved = true;
            }
        }
    }
    moved
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn report(s: &Session) -> Value {
    let c = &s.color.settings;
    json!({
        "display": c.display,
        "surfaces": DisplaySurface::ALL.iter().map(|d| json!({"id": d.id(), "label": d.label(), "hdr": d.is_hdr(), "bitDepths": d.bit_depths()})).collect::<Vec<_>>(),
        "pasteAssume": c.paste_assume.id(),
        "enforcePalette": c.enforce_palette,
        "monitorStatus": s.color.monitor_status(),
        "displays": s.color.display_statuses(),
        "intent": c.intent,
    })
}

/// `edit.colorManagement`: read or change the display, paste and palette settings.
fn color_management(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "edit.colorManagement";
    let mut next = s.color.settings.clone();
    if p.get("reset").and_then(Value::as_bool) == Some(true) {
        next.display = DisplaySettings::default();
        next.paste_assume = PasteAssume::Srgb;
        next.enforce_palette = false;
    }
    if let Some(v) = p.get("surface").and_then(Value::as_str) {
        next.display.surface = DisplaySurface::parse(v).ok_or_else(|| bad(cmd, "`surface` must be srgb|rec709|rec2020-pq|rec2020-hlg|scrgb-linear"))?;
        if !next.display.surface.bit_depths().contains(&next.display.bit_depth) {
            next.display.bit_depth = next.display.surface.bit_depths()[0];
        }
    }
    if let Some(v) = p.get("bitDepth") {
        let d = v.as_u64().filter(|d| [8, 10, 12, 16].contains(d)).ok_or_else(|| bad(cmd, "`bitDepth` must be 8, 10, 12 or 16"))? as u8;
        if !next.display.surface.bit_depths().contains(&d) {
            return Err(bad(cmd, format!("the {} surface supports {:?} bits", next.display.surface.label(), next.display.surface.bit_depths())));
        }
        next.display.bit_depth = d;
    }
    if let Some(v) = p.get("sdrNits") {
        next.display.sdr_nits =
            v.as_f64().filter(|n| n.is_finite() && (1.0..=10000.0).contains(n)).ok_or_else(|| bad(cmd, "`sdrNits` must be 1..10000"))? as f32;
    }
    if let Some(v) = p.get("unmanaged").and_then(Value::as_bool) {
        next.display.unmanaged = v;
    }
    if let Some(v) = p.get("enforcePalette").and_then(Value::as_bool) {
        next.enforce_palette = v;
    }
    if let Some(v) = p.get("pasteAssume").and_then(Value::as_str) {
        next.paste_assume = PasteAssume::parse(v).ok_or_else(|| bad(cmd, "`pasteAssume` must be srgb|monitor|working|ask"))?;
    }
    if let Some(sp) = p.get("screenProfiles").and_then(Value::as_object) {
        for (k, v) in sp {
            match v {
                Value::Null => {
                    next.display.screen_profiles.remove(k);
                }
                Value::String(spec) if spec.is_empty() || spec == "auto" => {
                    next.display.screen_profiles.remove(k);
                }
                Value::String(spec) => {
                    let prof = crate::color_cmds::resolve_profile(spec, None, Some(ColorMode::Rgb))?;
                    if prof.color_space != redrob_photo_cms::ColorSpace::Rgb {
                        return Err(bad(cmd, format!("screen profile `{spec}` is not RGB")));
                    }
                    next.display.screen_profiles.insert(k.clone(), spec.clone());
                }
                _ => return Err(bad(cmd, "`screenProfiles` values must be profile ids, paths, \"auto\" or null")),
            }
        }
    }
    if next != s.color.settings {
        s.color.settings = next;
        s.prefs.edit(|_| ());
        let moved = enforce_palette(s);
        let _ = moved;
    }
    Ok(report(s))
}

/// `colors.common`: the colours the image is made of (the colour selectors' "common colours"
/// patches): a palette of `count` colours built from the composite (or the active layer),
/// sampled down to at most 16k pixels first (it runs per document revision while the Color
/// panel shows). Fully transparent pixels say nothing.
fn common_colors(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "colors.common";
    use redrob_photo_algo::quantize::{Forced, PaletteKind, build_palette};
    let count = p.get("count").and_then(Value::as_u64).unwrap_or(12).clamp(1, 256) as usize;
    let all = p.get("allLayers").and_then(Value::as_bool).unwrap_or(true);
    let kind = match p.get("method").and_then(Value::as_str).unwrap_or("perceptual") {
        "adaptive" => PaletteKind::Adaptive,
        "selective" => PaletteKind::Selective,
        "perceptual" => PaletteKind::Perceptual,
        m => return Err(bad(CMD, format!("unknown method `{m}` (perceptual|selective|adaptive)"))),
    };
    let (area, img) = crate::selection_cmds::sample_rgba8(s, all)?;
    let (w, h) = (area.width() as usize, area.height() as usize);
    if w == 0 || h == 0 || img.is_empty() {
        return Ok(json!({ "colors": [] }));
    }
    // Stride so that at most ~16k pixels are considered.
    let step = (((w * h) as f32 / 16_384.0).sqrt().ceil() as usize).max(1);
    let mut px: Vec<[f32; 4]> = Vec::with_capacity(w * h / (step * step) + 1);
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let c = img[y * w + x];
            if c[3] > 0 {
                px.push([f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, f32::from(c[3]) / 255.0]);
            }
        }
    }
    if px.is_empty() {
        return Ok(json!({ "colors": [] }));
    }
    let pal = build_palette(&px, kind, count, Forced::None).map_err(|e| bad(CMD, e))?;
    let colors: Vec<[f32; 3]> = pal.iter().map(|c| c.map(|v| f32::from(v) / 255.0)).collect();
    Ok(json!({ "colors": colors, "sampled": px.len() }))
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "colors.common",
            label: "Common Colours",
            menu: &[],
            shortcut: None,
            params: r##"{"count":1..256=12,"allLayers":bool=true,"method":"perceptual|selective|adaptive"="perceptual"} → {colors: [[r,g,b]], sampled}"##,
            enabled: has_doc,
            run: common_colors,
            journal: false,
        },
        CommandSpec {
            id: "edit.colorManagement",
            label: "Color Management Settings",
            menu: &[],
            shortcut: None,
            params: r##"{"surface":"srgb|rec709|rec2020-pq|rec2020-hlg|scrgb-linear"?,"bitDepth":8|10|12|16?,"sdrNits":1..10000?,"unmanaged":bool?,"screenProfiles":{"<display id>": profile|path|"auto"|null}?,"pasteAssume":"srgb|monitor|working|ask"?,"enforcePalette":bool?,"reset":bool?} → {display, surfaces, pasteAssume, enforcePalette, monitorStatus, displays, intent}"##,
            enabled: always,
            run: color_management,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_encode_sdr_white_sensibly() {
        let w = [1.0, 1.0, 1.0];
        assert_eq!(DisplaySurface::Srgb.encode(w), w);
        let pq = DisplaySurface::Rec2020Pq.encode_with(w, 100.0);
        assert!((pq[0] - 0.508).abs() < 0.02, "{pq:?}");
        assert!(pq.iter().all(|v| (v - pq[0]).abs() < 0.01), "white stays neutral: {pq:?}");
        let lin = DisplaySurface::ScrgbLinear.encode([0.5, 0.5, 0.5]);
        assert!((lin[0] - 0.214).abs() < 0.01);
        assert!(DisplaySurface::parse("rec2020_pq") == Some(DisplaySurface::Rec2020Pq));
    }

    #[test]
    fn color_management_command_round_trips() {
        let mut s = Session::new();
        let r =
            s.execute("edit.colorManagement", json!({"surface": "rec2020-pq", "unmanaged": true, "pasteAssume": "monitor", "enforcePalette": true})).unwrap();
        assert_eq!(r["display"]["surface"], "rec2020Pq");
        assert_eq!(r["display"]["bitDepth"], 10);
        assert_eq!(r["pasteAssume"], "monitor");
        assert!(s.color.settings.enforce_palette);
        assert!(s.execute("edit.colorManagement", json!({"bitDepth": 8})).is_err(), "8 bits is not a PQ depth");
        let r = s.execute("edit.colorManagement", json!({"reset": true})).unwrap();
        assert_eq!(r["display"]["surface"], "srgb");
        assert!(!s.color.settings.display.unmanaged);
    }

    #[test]
    fn enforce_palette_snaps_the_foreground() {
        let mut s = Session::new();
        s.execute("edit.colorManagement", json!({"enforcePalette": true})).unwrap();
        s.execute("tools.setColors", json!({"foreground": [0.98, 0.01, 0.02, 1.0]})).unwrap();
        let fg = s.tools.foreground;
        let has = s.presets.swatches.iter().flat_map(|g| &g.items).any(|sw| {
            let c = crate::presets::swatches::SwatchConv::new(&s).rgb(&sw.color);
            (c[0] - fg[0]).abs() < 1e-3 && (c[1] - fg[1]).abs() < 1e-3 && (c[2] - fg[2]).abs() < 1e-3
        });
        assert!(has, "the foreground {fg:?} is a swatch colour");
    }
}
