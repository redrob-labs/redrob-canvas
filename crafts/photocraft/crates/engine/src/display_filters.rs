//! View › Display Filters: a per-document stack of view-only filters applied after the display
//! transform (they change what the screen shows, never the pixels):
//!
//! * Colour deficiency simulation: protanopia, deuteranopia (Viénot, Brettel & Mollon 1999, see
//!   [`crate::proof_sim::dichromat`]) and tritanopia (the same construction with the S cone
//!   replaced by its projection onto the tritanope's plane, using the paper's LMS matrices).
//! * Gamma: the displayed values raised to `1/gamma` in linear light.
//! * Contrast: the displayed values stretched round mid grey by a factor, in linear light.
//! * Clip warning: canvas values at or below 0 in any channel (shadows) or at or above 1
//!   (highlights) are painted in warning colours; the test can be restricted to the shadows or
//!   the highlights, and the colours are settable.
//!
//! The stack lives in `Session::color.display_filters` keyed by document, so each view can be
//! set up separately, and `view.displayFilters` changes it. The canvas LUT builder
//! ([`crate::proof_sim::display_lut_with`]) applies [`DisplayFilters::apply`] to every entry.

use redrob_photo_doc::{DocId, Document};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::color_cmds::ColorState;
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Which dichromacy is simulated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Deficiency {
    Protanopia,
    Deuteranopia,
    Tritanopia,
}

impl Deficiency {
    pub fn id(self) -> &'static str {
        match self {
            Deficiency::Protanopia => "protanopia",
            Deficiency::Deuteranopia => "deuteranopia",
            Deficiency::Tritanopia => "tritanopia",
        }
    }
    pub fn from_id(s: &str) -> Option<Deficiency> {
        Some(match s {
            "protanopia" | "protan" => Deficiency::Protanopia,
            "deuteranopia" | "deutan" => Deficiency::Deuteranopia,
            "tritanopia" | "tritan" => Deficiency::Tritanopia,
            _ => return None,
        })
    }
}

/// The filter stack of one view.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DisplayFilters {
    pub deficiency: Option<Deficiency>,
    /// 0.1…10, 1 = off.
    pub gamma: f32,
    /// 0.1…10, 1 = off.
    pub contrast: f32,
    pub clip_warning: bool,
    /// Warn where a channel is at or below 0.
    pub clip_shadows: bool,
    /// Warn where a channel is at or above 1.
    pub clip_highlights: bool,
    /// Warning colour for the shadows (sRGB 0..1).
    pub shadows_color: [f32; 3],
    /// Warning colour for the highlights (sRGB 0..1).
    pub highlights_color: [f32; 3],
}

impl Default for DisplayFilters {
    fn default() -> Self {
        Self {
            deficiency: None,
            gamma: 1.0,
            contrast: 1.0,
            clip_warning: false,
            clip_shadows: true,
            clip_highlights: true,
            shadows_color: [0.0, 0.0, 1.0],
            highlights_color: [1.0, 0.0, 0.0],
        }
    }
}

fn srgb_decode(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn srgb_encode(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Viénot et al. (1999): linear RGB → LMS (shared with the proof simulations).
const RGB_LMS: [[f32; 3]; 3] = [[17.8824, 43.5161, 4.11935], [3.45565, 27.1554, 3.86714], [0.0299566, 0.184309, 1.46709]];
#[allow(clippy::excessive_precision)]
const LMS_RGB: [[f32; 3]; 3] =
    [[0.080_944_45, -0.130_504_41, 0.116_721_07], [-0.010_248_533, 0.054_019_327, -0.113_614_71], [-0.000_365_297, -0.004_121_614_7, 0.693_511_4]];

fn mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// A tritanope's view of a linear-RGB colour: the S response is replaced by its projection onto
/// the plane through the origin, the display white and the long-wavelength anchor (display red,
/// standing for the 660 nm anchor of Brettel, Viénot & Mollon 1997), expressed in the paper's
/// LMS space. Red and cyan are kept, blue and green collapse towards teal, neutrals stay neutral.
pub fn tritanope(rgb_lin: [f32; 3]) -> [f32; 3] {
    let v = rgb_lin.map(|c| 0.992_052 * c + 0.003_974);
    let [l, m, _s] = mul(&RGB_LMS, v);
    let lms = [l, m, -0.012_245 * l + 0.072_035 * m];
    mul(&LMS_RGB, lms).map(|c| c.clamp(0.0, 1.0))
}

impl DisplayFilters {
    /// Whether the stack changes anything.
    pub fn is_active(&self) -> bool {
        self.deficiency.is_some() || self.gamma != 1.0 || self.contrast != 1.0 || (self.clip_warning && (self.clip_shadows || self.clip_highlights))
    }

    /// Whether `canvas` (the document's canvas value before display) trips the clip warning.
    pub fn clipped(&self, canvas: [f32; 3]) -> Option<[f32; 3]> {
        if !self.clip_warning {
            return None;
        }
        if self.clip_shadows && canvas.iter().any(|c| *c <= 0.0) {
            return Some(self.shadows_color);
        }
        if self.clip_highlights && canvas.iter().any(|c| *c >= 1.0) {
            return Some(self.highlights_color);
        }
        None
    }

    /// Applies the stack to a displayed sRGB-encoded colour `shown` whose canvas value was
    /// `canvas`.
    pub fn apply(&self, canvas: [f32; 3], shown: [f32; 3]) -> [f32; 3] {
        if !self.is_active() {
            return shown;
        }
        if let Some(c) = self.clipped(canvas) {
            return c;
        }
        let mut lin = shown.map(srgb_decode);
        if let Some(d) = self.deficiency {
            lin = match d {
                Deficiency::Protanopia => crate::proof_sim::dichromat(lin, true),
                Deficiency::Deuteranopia => crate::proof_sim::dichromat(lin, false),
                Deficiency::Tritanopia => tritanope(lin),
            };
        }
        if self.gamma != 1.0 {
            lin = lin.map(|v| v.max(0.0).powf(1.0 / self.gamma));
        }
        if self.contrast != 1.0 {
            // Mid grey (sRGB 0.5 → 0.214 linear) stays put.
            let mid = srgb_decode(0.5);
            lin = lin.map(|v| mid + (v - mid) * self.contrast);
        }
        lin.map(srgb_encode)
    }

    /// Hashable summary for display caches.
    pub fn key(&self) -> (Option<Deficiency>, u32, u32, bool, bool, bool, [u32; 3], [u32; 3]) {
        (
            self.deficiency,
            self.gamma.to_bits(),
            self.contrast.to_bits(),
            self.clip_warning,
            self.clip_shadows,
            self.clip_highlights,
            self.shadows_color.map(f32::to_bits),
            self.highlights_color.map(f32::to_bits),
        )
    }

    pub fn to_json(&self) -> Value {
        let hex = |c: [f32; 3]| format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8);
        json!({
            "deficiency": self.deficiency.map(Deficiency::id),
            "gamma": self.gamma,
            "contrast": self.contrast,
            "clipWarning": self.clip_warning,
            "clipShadows": self.clip_shadows,
            "clipHighlights": self.clip_highlights,
            "shadowsColor": hex(self.shadows_color),
            "highlightsColor": hex(self.highlights_color),
            "active": self.is_active(),
        })
    }
}

/// The active filter stack of `doc` (None when nothing is on).
pub fn active(c: &ColorState, doc: &Document) -> Option<DisplayFilters> {
    c.display_filters.get(&doc.id).copied().filter(DisplayFilters::is_active)
}

/// Changes whenever the filters of `doc` change (for display caches).
pub fn key(c: &ColorState, id: DocId) -> Option<(Option<Deficiency>, u32, u32, bool, bool, bool, [u32; 3], [u32; 3])> {
    c.display_filters.get(&id).filter(|f| f.is_active()).map(DisplayFilters::key)
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "view.displayFilters".into(), msg: msg.into() }
}

fn display_filters(s: &mut Session, p: &Value) -> Result<Value> {
    let id = s.active().ok_or(EngineError::NoDocument)?.doc.id;
    let mut f = s.color.display_filters.get(&id).copied().unwrap_or_default();
    if p.get("reset").and_then(Value::as_bool) == Some(true) {
        f = DisplayFilters::default();
    }
    match p.get("deficiency") {
        None => {}
        Some(Value::Null) => f.deficiency = None,
        Some(Value::String(d)) if d == "none" || d.is_empty() => f.deficiency = None,
        Some(Value::String(d)) => {
            f.deficiency = Some(Deficiency::from_id(d).ok_or_else(|| bad(format!("deficiency `{d}` (none|protanopia|deuteranopia|tritanopia)")))?)
        }
        Some(_) => return Err(bad("deficiency must be a string")),
    }
    if let Some(g) = p.get("gamma").and_then(Value::as_f64) {
        if !g.is_finite() {
            return Err(bad("gamma must be a number"));
        }
        f.gamma = (g as f32).clamp(0.1, 10.0);
    }
    if let Some(c) = p.get("contrast").and_then(Value::as_f64) {
        if !c.is_finite() {
            return Err(bad("contrast must be a number"));
        }
        f.contrast = (c as f32).clamp(0.1, 10.0);
    }
    if let Some(b) = p.get("clipWarning").and_then(Value::as_bool) {
        f.clip_warning = b;
    }
    if let Some(b) = p.get("clipShadows").and_then(Value::as_bool) {
        f.clip_shadows = b;
    }
    if let Some(b) = p.get("clipHighlights").and_then(Value::as_bool) {
        f.clip_highlights = b;
    }
    if p.get("shadowsColor").is_some() {
        let c = crate::commands::color_param(p, "shadowsColor", [0.0, 0.0, 1.0, 1.0]);
        f.shadows_color = [c[0], c[1], c[2]];
    }
    if p.get("highlightsColor").is_some() {
        let c = crate::commands::color_param(p, "highlightsColor", [1.0, 0.0, 0.0, 1.0]);
        f.highlights_color = [c[0], c[1], c[2]];
    }
    if f == DisplayFilters::default() {
        s.color.display_filters.remove(&id);
    } else {
        s.color.display_filters.insert(id, f);
    }
    Ok(f.to_json())
}

fn deficiency_cmd(s: &mut Session, d: Option<Deficiency>) -> Result<Value> {
    let id = s.active().ok_or(EngineError::NoDocument)?.doc.id;
    let f = s.color.display_filters.entry(id).or_default();
    f.deficiency = d;
    Ok(f.to_json())
}

fn toggle_clip(s: &mut Session, _: &Value) -> Result<Value> {
    let id = s.active().ok_or(EngineError::NoDocument)?.doc.id;
    let f = s.color.display_filters.entry(id).or_default();
    f.clip_warning = !f.clip_warning;
    Ok(f.to_json())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "view.displayFilters",
            label: "Display Filters…",
            menu: &["View"],
            shortcut: None,
            params: r##"{"deficiency":"none|protanopia|deuteranopia|tritanopia"?,"gamma":0.1..10=1,"contrast":0.1..10=1,"clipWarning":bool?,"clipShadows":bool=true,"clipHighlights":bool=true,"shadowsColor":"#rrggbb"="#0000ff","highlightsColor":"#rrggbb"="#ff0000","reset":bool=false} (view-only filters of this document's view; omitted keys keep their value) → the stack"##,
            enabled: has_doc,
            run: display_filters,
            journal: false,
        },
        CommandSpec {
            id: "view.displayFilters.none",
            label: "No Color Deficiency Simulation",
            menu: &["View", "Display Filters"],
            shortcut: None,
            params: "{} → the stack",
            enabled: has_doc,
            run: |s, _| deficiency_cmd(s, None),
            journal: false,
        },
        CommandSpec {
            id: "view.displayFilters.protanopia",
            label: "Simulate Protanopia",
            menu: &["View", "Display Filters"],
            shortcut: None,
            params: "{} → the stack",
            enabled: has_doc,
            run: |s, _| deficiency_cmd(s, Some(Deficiency::Protanopia)),
            journal: false,
        },
        CommandSpec {
            id: "view.displayFilters.deuteranopia",
            label: "Simulate Deuteranopia",
            menu: &["View", "Display Filters"],
            shortcut: None,
            params: "{} → the stack",
            enabled: has_doc,
            run: |s, _| deficiency_cmd(s, Some(Deficiency::Deuteranopia)),
            journal: false,
        },
        CommandSpec {
            id: "view.displayFilters.tritanopia",
            label: "Simulate Tritanopia",
            menu: &["View", "Display Filters"],
            shortcut: None,
            params: "{} → the stack",
            enabled: has_doc,
            run: |s, _| deficiency_cmd(s, Some(Deficiency::Tritanopia)),
            journal: false,
        },
        CommandSpec {
            id: "view.displayFilters.clipWarning",
            label: "Clip Warning",
            menu: &["View", "Display Filters"],
            shortcut: None,
            params: "{} (toggles the over/under exposure warning) → the stack",
            enabled: has_doc,
            run: toggle_clip,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s
    }

    #[test]
    fn tritanope_keeps_neutrals_and_merges_blue_green() {
        let w = tritanope([1.0, 1.0, 1.0]);
        assert!(w.iter().all(|c| (c - 1.0).abs() < 0.03), "{w:?}");
        let g = tritanope([0.2, 0.2, 0.2]);
        assert!(g.iter().all(|c| (c - 0.2).abs() < 0.03), "{g:?}");
        // Blue loses its blueness (dark teal), green turns cyan, red stays red.
        let b = tritanope([0.0, 0.0, 1.0]);
        assert!(b[2] < 0.5 && b[1] > 0.1 && (b[1] - b[2]).abs() < 0.05, "{b:?}");
        let g = tritanope([0.0, 1.0, 0.0]);
        assert!(g[1] > 0.7 && g[2] > 0.7, "{g:?}");
        let r = tritanope([1.0, 0.0, 0.0]);
        assert!(r[0] > 0.8, "{r:?}");
    }

    #[test]
    fn stack_applies_gamma_contrast_and_clip() {
        let mut f = DisplayFilters::default();
        assert!(!f.is_active());
        assert_eq!(f.apply([0.3; 3], [0.3; 3]), [0.3; 3]);
        f.gamma = 2.0;
        let o = f.apply([0.5; 3], [0.5; 3]);
        assert!(o[0] > 0.5, "gamma 2 brightens: {o:?}");
        f.gamma = 1.0;
        f.contrast = 2.0;
        let lo = f.apply([0.3; 3], [0.3; 3]);
        let hi = f.apply([0.7; 3], [0.7; 3]);
        assert!(lo[0] < 0.3 && hi[0] > 0.7, "{lo:?} {hi:?}");
        let mid = f.apply([0.5; 3], [0.5; 3]);
        assert!((mid[0] - 0.5).abs() < 0.01, "{mid:?}");
        f.contrast = 1.0;
        f.clip_warning = true;
        assert_eq!(f.apply([1.0, 0.5, 0.5], [0.9; 3]), [1.0, 0.0, 0.0]);
        assert_eq!(f.apply([0.0, 0.5, 0.5], [0.1; 3]), [0.0, 0.0, 1.0]);
        f.clip_shadows = false;
        assert_eq!(f.apply([0.0, 0.5, 0.5], [0.1; 3]), [0.1; 3]);
    }

    #[test]
    fn command_edits_the_stack_and_the_canvas_lut_follows() {
        let mut s = session();
        let d = s.active().unwrap().doc.clone();
        assert!(s.color.canvas_lut(&d, 5).unwrap().is_none());
        let r = s.execute("view.displayFilters", json!({"deficiency": "tritanopia", "gamma": 1.5})).unwrap();
        assert_eq!(r["deficiency"], "tritanopia");
        assert_eq!(r["active"], true);
        assert!(s.color.canvas_lut(&d, 5).unwrap().is_some());
        let before = s.color.display_signature(&d);
        s.execute("view.displayFilters.clipWarning", json!({})).unwrap();
        assert_ne!(before, s.color.display_signature(&d));
        let lut = s.color.display_lut(&d, 5).unwrap();
        // White clips: shown in the highlights colour.
        let w = lut.data[4 + 4 * 5 + 4 * 25];
        assert!(w[0] > 0.9 && w[1] < 0.1 && w[2] < 0.1, "{w:?}");
        assert!(s.execute("view.displayFilters", json!({"deficiency": "nope"})).is_err());
        let r = s.execute("view.displayFilters", json!({"reset": true})).unwrap();
        assert_eq!(r["active"], false);
        assert!(s.color.canvas_lut(&d, 5).unwrap().is_none());
        assert!(!s.color.display_filters.contains_key(&d.id));
    }
}
