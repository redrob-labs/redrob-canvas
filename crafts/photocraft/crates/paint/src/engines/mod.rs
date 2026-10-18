//! Alternative brush engines: ways of turning a stroke into pixels other than stamping a tip.
//!
//! The Pixel engine is the dab machinery of [`crate::render`]; every other engine here takes the
//! same [`Stroke`] (brush settings for size, colour, opacity, spacing and seed, plus the points) and
//! its own [`EngineSettings`] variant, and paints onto a surface with the selection and
//! lock-transparency honoured. Engines are deterministic from the stroke seed, like the dabs.

use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use crate::Stroke;
use crate::brush::{HoseDimension, HoseSelection, ImageHose, TipShape};
use crate::tile::GrayTile;

pub mod bristle;
pub mod buffer;
pub mod curve;
pub mod deform;
pub mod dyna;
pub mod grid;
pub mod hatching;
pub mod mypaint;
pub mod particle;
pub mod shape;
pub mod sketch;
pub mod spray;
pub mod tangent;

pub use bristle::BristleSettings;
pub use buffer::PaintBuf;
pub use curve::CurveSettings;
pub use deform::{DeformMode, DeformSettings};
pub use dyna::{DynaSettings, DynaShape};
pub use grid::{GridParticle, GridSettings};
pub use hatching::{Crosshatch, HatchingSettings};
pub use mypaint::{MpCurve, MpInput, MpSetting, MyPaintSettings};
pub use particle::ParticleSettings;
pub use shape::ShapeSettings;
pub use sketch::SketchSettings;
pub use spray::{Distribution, ParticleRotation, ParticleShape, SpraySettings};
pub use tangent::{Axis, NormalSource, TangentSettings};

/// Which engine paints, with its settings. In JSON the variant is the `kind` field
/// (`{"kind": "sketch", "lineWidth": 2}`); `{"kind": "pixel"}` is the tip-stamping default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EngineSettings {
    #[default]
    Pixel,
    Sketch(SketchSettings),
    Hatching(HatchingSettings),
    Shape(ShapeSettings),
    Grid(GridSettings),
    Particle(ParticleSettings),
    Curve(CurveSettings),
    Dyna(DynaSettings),
    TangentNormal(TangentSettings),
    #[serde(rename = "myPaint")]
    MyPaint(MyPaintSettings),
    Spray(SpraySettings),
    Bristle(BristleSettings),
    Deform(DeformSettings),
}

/// The engine kinds, by their JSON `kind` names.
pub const KINDS: [&str; 13] =
    ["pixel", "sketch", "hatching", "shape", "grid", "particle", "curve", "dyna", "tangentNormal", "myPaint", "spray", "bristle", "deform"];

impl EngineSettings {
    /// The JSON `kind` name.
    pub fn kind(&self) -> &'static str {
        match self {
            EngineSettings::Pixel => "pixel",
            EngineSettings::Sketch(_) => "sketch",
            EngineSettings::Hatching(_) => "hatching",
            EngineSettings::Shape(_) => "shape",
            EngineSettings::Grid(_) => "grid",
            EngineSettings::Particle(_) => "particle",
            EngineSettings::Curve(_) => "curve",
            EngineSettings::Dyna(_) => "dyna",
            EngineSettings::TangentNormal(_) => "tangentNormal",
            EngineSettings::MyPaint(_) => "myPaint",
            EngineSettings::Spray(_) => "spray",
            EngineSettings::Bristle(_) => "bristle",
            EngineSettings::Deform(_) => "deform",
        }
    }

    /// Default settings of an engine by its `kind` name.
    pub fn by_kind(kind: &str) -> Option<Self> {
        Some(match kind {
            "pixel" => EngineSettings::Pixel,
            "sketch" => EngineSettings::Sketch(SketchSettings::default()),
            "hatching" => EngineSettings::Hatching(HatchingSettings::default()),
            "shape" => EngineSettings::Shape(ShapeSettings::default()),
            "grid" => EngineSettings::Grid(GridSettings::default()),
            "particle" => EngineSettings::Particle(ParticleSettings::default()),
            "curve" => EngineSettings::Curve(CurveSettings::default()),
            "dyna" => EngineSettings::Dyna(DynaSettings::default()),
            "tangentNormal" => EngineSettings::TangentNormal(TangentSettings::default()),
            "myPaint" => EngineSettings::MyPaint(MyPaintSettings::default()),
            "spray" => EngineSettings::Spray(SpraySettings::default()),
            "bristle" => EngineSettings::Bristle(BristleSettings::default()),
            "deform" => EngineSettings::Deform(DeformSettings::default()),
            _ => return None,
        })
    }

    /// Does the engine read the canvas (sample-all-layers can apply)?
    pub fn samples_canvas(&self) -> bool {
        match self {
            EngineSettings::Grid(g) => g.sample_input,
            EngineSettings::Spray(s) => s.sample_input,
            EngineSettings::Bristle(b) => b.soak_ink,
            EngineSettings::MyPaint(m) => !m.smudge.is_constant() || m.smudge.base_value > 0.0,
            EngineSettings::TangentNormal(t) => t.mix > 0.0,
            _ => false,
        }
    }
}

/// Paint a stroke with an engine. `sample` is the surface the engines that read colour from the
/// canvas read instead of the target (a flattened composite for Sample All Layers). Returns the
/// damaged rectangle; `None` for the Pixel engine, which the caller renders with
/// [`crate::render_stroke`].
pub fn paint(
    target: &mut Surface,
    sample: Option<&Surface>,
    stroke: &Stroke,
    engine: &EngineSettings,
    selection: Option<&Surface>,
    lock: bool,
) -> Option<Rect> {
    let stroke = Stroke { brush: stroke.brush.bounded_for_render(), points: stroke.points.clone() };
    Some(match engine {
        EngineSettings::Pixel => return None,
        EngineSettings::Sketch(s) => sketch::paint(target, &stroke, s, selection, lock),
        EngineSettings::Hatching(s) => hatching::paint(target, &stroke, s, selection, lock),
        EngineSettings::Shape(s) => shape::paint(target, &stroke, s, selection, lock),
        EngineSettings::Grid(s) => grid::paint(target, sample, &stroke, s, selection, lock),
        EngineSettings::Particle(s) => particle::paint(target, &stroke, s, selection, lock),
        EngineSettings::Curve(s) => curve::paint(target, &stroke, s, selection, lock),
        EngineSettings::Dyna(s) => dyna::paint(target, &stroke, s, selection, lock),
        EngineSettings::TangentNormal(s) => tangent::paint(target, &stroke, s, selection, lock),
        EngineSettings::MyPaint(s) => mypaint::paint(target, &stroke, s, selection, lock),
        EngineSettings::Spray(s) => spray::paint(target, sample, &stroke, s, selection, lock),
        EngineSettings::Bristle(s) => bristle::paint(target, &stroke, s, selection, lock),
        EngineSettings::Deform(s) => deform::paint(target, &stroke, s, selection, lock),
    })
}

/// A text tip from rendered glyph cells (one grayscale tile per character, in order): the tip is
/// the whole word; with `pipe` each dab paints the next character instead (an image hose over
/// the cells, incremental). An empty list gives a round tip.
pub fn text_tip(cells: Vec<GrayTile>, pipe: bool) -> (TipShape, Option<ImageHose>) {
    let cells: Vec<GrayTile> = cells.into_iter().filter(GrayTile::is_valid).collect();
    let Some(first) = cells.first().cloned() else { return (TipShape::Round, None) };
    if !pipe || cells.len() == 1 {
        // The word as one tip: the cells side by side on a common baseline.
        let w: u32 = cells.iter().map(|c| c.width).sum();
        let h = cells.iter().map(|c| c.height).max().unwrap_or(1);
        if w == 0 || h == 0 {
            return (TipShape::Round, None);
        }
        let mut data = vec![0.0f32; w as usize * h as usize];
        let mut x0 = 0u32;
        for c in &cells {
            let yo = h - c.height;
            for y in 0..c.height {
                for x in 0..c.width {
                    data[((yo + y) * w + x0 + x) as usize] = c.get(x, y);
                }
            }
            x0 += c.width;
        }
        return (TipShape::Sampled(GrayTile::from_f32(w, h, &data)), None);
    }
    let n = cells.len() as u32;
    (TipShape::Sampled(first), Some(ImageHose { cells, dims: vec![HoseDimension { rank: n, selection: HoseSelection::Incremental }] }))
}
