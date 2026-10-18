//! Per-layer compositing options beyond the blend mode: how the layer's alpha combines with
//! the backdrop's (the composite mode), the colour space the blend formula runs in (the blend
//! space), the space the alpha mix runs in (the composite space), and the legacy switch that
//! selects the pre-linear, gamma-encoded variant of a mode for files from older editors.
//!
//! Defaults compose as every layer did before these options existed: alpha union, both spaces
//! following the document's transfer, no legacy variant. [`LayerComposite::is_default`] lets
//! writers omit the block.

use serde::{Deserialize, Serialize};

/// How the layer's coverage shapes the result's alpha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompositeMode {
    /// `αo = αs + αb·(1 − αs)`: the layer adds its coverage to the backdrop's (the default).
    #[default]
    Union,
    /// `αo = αb`: the layer only recolours the backdrop; it never extends it.
    ClipToBackdrop,
    /// `αo = αs`: the result exists only where the layer has coverage.
    ClipToLayer,
    /// `αo = αs·αb`: the result exists only where both have coverage.
    Intersection,
}

impl CompositeMode {
    pub const ALL: [CompositeMode; 4] = [Self::Union, Self::ClipToBackdrop, Self::ClipToLayer, Self::Intersection];

    /// The parameter name (`union`, `clipToBackdrop`, `clipToLayer`, `intersection`).
    pub fn name(self) -> &'static str {
        match self {
            CompositeMode::Union => "union",
            CompositeMode::ClipToBackdrop => "clipToBackdrop",
            CompositeMode::ClipToLayer => "clipToLayer",
            CompositeMode::Intersection => "intersection",
        }
    }

    /// The menu label.
    pub fn label(self) -> &'static str {
        match self {
            CompositeMode::Union => "Union",
            CompositeMode::ClipToBackdrop => "Clip to Backdrop",
            CompositeMode::ClipToLayer => "Clip to Layer",
            CompositeMode::Intersection => "Intersection",
        }
    }

    /// Inverse of [`CompositeMode::name`] (case-insensitive, also accepts the label).
    pub fn from_name(s: &str) -> Option<Self> {
        let k: String = s.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '_').collect::<String>().to_ascii_lowercase();
        Self::ALL.into_iter().find(|m| m.name().to_ascii_lowercase() == k || m.label().to_ascii_lowercase().replace(' ', "") == k)
    }
}

/// The colour space a blend formula or alpha mix runs in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlendSpace {
    /// Follow the document (its transfer setting; the default).
    #[default]
    Auto,
    /// Linear-light RGB.
    RgbLinear,
    /// Gamma-encoded (perceptual) RGB.
    RgbPerceptual,
    /// CIE L*a*b*, components scaled into 0..1.
    Lab,
}

impl BlendSpace {
    pub const ALL: [BlendSpace; 4] = [Self::Auto, Self::RgbLinear, Self::RgbPerceptual, Self::Lab];

    /// The parameter name (`auto`, `rgbLinear`, `rgbPerceptual`, `lab`).
    pub fn name(self) -> &'static str {
        match self {
            BlendSpace::Auto => "auto",
            BlendSpace::RgbLinear => "rgbLinear",
            BlendSpace::RgbPerceptual => "rgbPerceptual",
            BlendSpace::Lab => "lab",
        }
    }

    /// The menu label.
    pub fn label(self) -> &'static str {
        match self {
            BlendSpace::Auto => "Auto",
            BlendSpace::RgbLinear => "RGB (linear)",
            BlendSpace::RgbPerceptual => "RGB (perceptual)",
            BlendSpace::Lab => "LAB",
        }
    }

    /// Inverse of [`BlendSpace::name`] (case-insensitive; `linear` / `perceptual` also accepted).
    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().replace([' ', '-', '_', '(', ')'], "").as_str() {
            "auto" | "" => Some(BlendSpace::Auto),
            "rgblinear" | "linear" => Some(BlendSpace::RgbLinear),
            "rgbperceptual" | "perceptual" => Some(BlendSpace::RgbPerceptual),
            "lab" | "cielab" => Some(BlendSpace::Lab),
            _ => None,
        }
    }
}

/// The per-layer compositing options (see the module docs). `Default` composes as before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LayerComposite {
    /// How alpha combines.
    pub mode: CompositeMode,
    /// The space the blend formula runs in.
    pub blend_space: BlendSpace,
    /// The space the alpha mix runs in.
    pub composite_space: BlendSpace,
    /// Legacy variant of the blend mode: gamma-encoded RGB for both spaces and alpha union, as
    /// older files composited. Overrides the three fields above while set.
    pub legacy: bool,
}

impl LayerComposite {
    /// Whether every option is at its default (alpha union, spaces following the document).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The options in force: the legacy switch pins both spaces to perceptual RGB and the alpha
    /// to union.
    pub fn effective(&self) -> LayerComposite {
        if self.legacy {
            LayerComposite { mode: CompositeMode::Union, blend_space: BlendSpace::RgbPerceptual, composite_space: BlendSpace::RgbPerceptual, legacy: true }
        } else {
            *self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_legacy_pins_the_spaces() {
        for m in CompositeMode::ALL {
            assert_eq!(CompositeMode::from_name(m.name()), Some(m));
            assert_eq!(CompositeMode::from_name(m.label()), Some(m));
        }
        for s in BlendSpace::ALL {
            assert_eq!(BlendSpace::from_name(s.name()), Some(s));
        }
        assert_eq!(BlendSpace::from_name("linear"), Some(BlendSpace::RgbLinear));
        assert!(LayerComposite::default().is_default());
        let l = LayerComposite { mode: CompositeMode::Intersection, blend_space: BlendSpace::Lab, composite_space: BlendSpace::Auto, legacy: true };
        let e = l.effective();
        assert_eq!((e.mode, e.blend_space, e.composite_space), (CompositeMode::Union, BlendSpace::RgbPerceptual, BlendSpace::RgbPerceptual));
        let j = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<LayerComposite>(&j).unwrap(), l);
        assert_eq!(serde_json::from_str::<LayerComposite>("{}").unwrap(), LayerComposite::default());
    }
}
