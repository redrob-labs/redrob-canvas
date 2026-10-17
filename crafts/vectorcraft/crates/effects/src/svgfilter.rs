//! SVG filter graphs (Effect › SVG Filters › Filter Graph, the Filter Editor panel): a chain of
//! filter primitives as the SVG specification defines them (`feGaussianBlur`, `feOffset`,
//! `feFlood`, `feMerge`, `feComposite`, `feColorMatrix`, `feComponentTransfer`,
//! `feConvolveMatrix`, `feMorphology`, `feTurbulence`, `feDisplacementMap`, `feDiffuseLighting`,
//! `feSpecularLighting`, `feBlend`, `feDropShadow`, `feTile`), connected by their `in`/`in2`
//! inputs and `result` names into a graph, and evaluated here on the object's pixels.
//!
//! The graph is the effect's parameters (`filter.graph`): `{region: [x, y, w, h] (fractions of the
//! object's bounds, SVG's defaults -10 %, -10 %, 120 %, 120 %), primitives: [{type, result?, in?,
//! …}]}`, so the SVG writer puts it back as a `<filter>` and the reader keeps any `<filter>` it
//! meets. Lengths are in document units (points) and become pixels through the raster's
//! [`PixelSpace::px`]. Colours are filtered in sRGB (`color-interpolation-filters` is not
//! distinguished) and a primitive's own subregion is the filter region (not kept).

use std::collections::HashMap;

use redrob_vector_geom::{Point, Rect};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pixel::PixelSpace;

/// The effect id of a filter graph.
pub const FILTER_GRAPH: &str = "filter.graph";

/// SVG's default filter region: 10 % beyond the object's bounds on every side.
pub const DEFAULT_REGION: [f64; 4] = [-0.1, -0.1, 1.2, 1.2];

fn default_region() -> [f64; 4] {
    DEFAULT_REGION
}

/// A filter: its region and its primitives, in evaluation order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterGraph {
    /// The filter region as fractions of the object's bounds: x, y, width, height.
    #[serde(default = "default_region")]
    pub region: [f64; 4],
    /// The primitives, first evaluated first; the last one's result is the filter's.
    #[serde(default)]
    pub primitives: Vec<Primitive>,
}

impl Default for FilterGraph {
    fn default() -> Self {
        Self { region: DEFAULT_REGION, primitives: vec![] }
    }
}

/// One primitive: what it computes, and the `result` name later primitives refer to it by.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Primitive {
    /// `result`: the name later primitives refer to this one by (empty: unnamed).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result: String,
    /// What it does.
    #[serde(flatten)]
    pub kind: Kind,
}

/// A filter primitive's input: `SourceGraphic`, `SourceAlpha`, a `result` name, or empty for the
/// previous primitive's result (the source graphic for the first).
pub type Input = String;

/// Composite operators (`feComposite operator`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompositeOp {
    #[default]
    Over,
    In,
    Out,
    Atop,
    Xor,
    Arithmetic,
}

/// Colour matrix modes (`feColorMatrix type`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatrixMode {
    #[default]
    Matrix,
    Saturate,
    HueRotate,
    LuminanceToAlpha,
}

/// A component transfer function (`feFuncR`…).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Transfer {
    #[default]
    Identity,
    Table {
        #[serde(default)]
        values: Vec<f64>,
    },
    Discrete {
        #[serde(default)]
        values: Vec<f64>,
    },
    Linear {
        #[serde(default = "one")]
        slope: f64,
        #[serde(default)]
        intercept: f64,
    },
    Gamma {
        #[serde(default = "one")]
        amplitude: f64,
        #[serde(default = "one")]
        exponent: f64,
        #[serde(default)]
        offset: f64,
    },
}

fn one() -> f64 {
    1.0
}

/// `feConvolveMatrix edgeMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeMode {
    #[default]
    Duplicate,
    Wrap,
    None,
}

/// `feMorphology operator`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MorphologyOp {
    #[default]
    Erode,
    Dilate,
}

/// A colour channel (`feDisplacementMap xChannelSelector`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Channel {
    R,
    G,
    B,
    #[default]
    A,
}

/// `feBlend mode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlendOp {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
}

/// A light source of the lighting primitives.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Light {
    /// `feDistantLight`: from `azimuth`° around the page (clockwise from +x) at `elevation`°.
    Distant { azimuth: f64, elevation: f64 },
    /// `fePointLight` at (x, y, z), document units from the object's top-left corner.
    Point { x: f64, y: f64, z: f64 },
    /// `feSpotLight` at (x, y, z) pointing at (`px`, `py`, `pz`), focused by `exponent` within
    /// `cone`° (0: no cone).
    Spot { x: f64, y: f64, z: f64, px: f64, py: f64, pz: f64, exponent: f64, cone: f64 },
}

impl Default for Light {
    fn default() -> Self {
        Light::Distant { azimuth: 45.0, elevation: 45.0 }
    }
}

/// A primitive's kind with its attributes (SVG's names, camel-cased; lengths in points).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Kind {
    /// `feGaussianBlur`.
    GaussianBlur {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default = "default_sd")]
        sd_x: f64,
        #[serde(default = "default_sd")]
        sd_y: f64,
    },
    /// `feOffset`.
    Offset {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
    /// `feFlood`: `color` as `#rrggbb`.
    Flood {
        #[serde(default = "black")]
        color: String,
        #[serde(default = "one")]
        opacity: f64,
    },
    /// `feMerge`: `inputs` layered bottom first.
    Merge {
        #[serde(default)]
        inputs: Vec<Input>,
    },
    /// `feComposite` of `in` over/in/out/atop/xor `in2`, or `k1 i1 i2 + k2 i1 + k3 i2 + k4`.
    Composite {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default)]
        in2: Input,
        #[serde(default)]
        operator: CompositeOp,
        #[serde(default)]
        k1: f64,
        #[serde(default)]
        k2: f64,
        #[serde(default)]
        k3: f64,
        #[serde(default)]
        k4: f64,
    },
    /// `feColorMatrix`: `values` are the 20 numbers of a matrix, the one number of saturate or
    /// hueRotate (degrees).
    ColorMatrix {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default)]
        mode: MatrixMode,
        #[serde(default)]
        values: Vec<f64>,
    },
    /// `feComponentTransfer`.
    ComponentTransfer {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default)]
        func_r: Transfer,
        #[serde(default)]
        func_g: Transfer,
        #[serde(default)]
        func_b: Transfer,
        #[serde(default)]
        func_a: Transfer,
    },
    /// `feConvolveMatrix`: a `columns` × `rows` `kernel`.
    ConvolveMatrix {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default = "three")]
        columns: u32,
        #[serde(default = "three")]
        rows: u32,
        #[serde(default)]
        kernel: Vec<f64>,
        /// 0: the kernel's sum (1 when that is 0).
        #[serde(default)]
        divisor: f64,
        #[serde(default)]
        bias: f64,
        #[serde(default)]
        target_x: Option<u32>,
        #[serde(default)]
        target_y: Option<u32>,
        #[serde(default)]
        edge_mode: EdgeMode,
        #[serde(default)]
        preserve_alpha: bool,
    },
    /// `feMorphology`.
    Morphology {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default)]
        operator: MorphologyOp,
        #[serde(default)]
        radius_x: f64,
        #[serde(default)]
        radius_y: f64,
    },
    /// `feTurbulence`: frequencies per point.
    Turbulence {
        #[serde(default)]
        base_frequency_x: f64,
        #[serde(default)]
        base_frequency_y: f64,
        #[serde(default = "one_u32")]
        num_octaves: u32,
        #[serde(default)]
        seed: i32,
        #[serde(default)]
        stitch_tiles: bool,
        /// `type="fractalNoise"` rather than `turbulence`.
        #[serde(default)]
        fractal_noise: bool,
    },
    /// `feDisplacementMap`: `in` moved by `scale` × (`in2`'s channels − ½).
    DisplacementMap {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default)]
        in2: Input,
        #[serde(default)]
        scale: f64,
        #[serde(default)]
        x_channel: Channel,
        #[serde(default)]
        y_channel: Channel,
    },
    /// `feDiffuseLighting` of `in`'s alpha as a height map.
    DiffuseLighting {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default = "one")]
        surface_scale: f64,
        #[serde(default = "one")]
        diffuse_constant: f64,
        #[serde(default = "white")]
        color: String,
        #[serde(default)]
        light: Light,
    },
    /// `feSpecularLighting` of `in`'s alpha as a height map.
    SpecularLighting {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default = "one")]
        surface_scale: f64,
        #[serde(default = "one")]
        specular_constant: f64,
        #[serde(default = "one")]
        specular_exponent: f64,
        #[serde(default = "white")]
        color: String,
        #[serde(default)]
        light: Light,
    },
    /// `feBlend` of `in` over `in2`.
    Blend {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default)]
        in2: Input,
        #[serde(default)]
        mode: BlendOp,
    },
    /// `feDropShadow`.
    DropShadow {
        #[serde(default, rename = "in")]
        input: Input,
        #[serde(default = "two")]
        dx: f64,
        #[serde(default = "two")]
        dy: f64,
        #[serde(default = "two")]
        sd: f64,
        #[serde(default = "black")]
        color: String,
        #[serde(default = "one")]
        opacity: f64,
    },
    /// `feTile`: the input repeated over the region (drawn as the input itself).
    Tile {
        #[serde(default, rename = "in")]
        input: Input,
    },
}

fn default_sd() -> f64 {
    2.0
}
fn two() -> f64 {
    2.0
}
fn three() -> u32 {
    3
}
fn one_u32() -> u32 {
    1
}
fn black() -> String {
    "#000000".into()
}
fn white() -> String {
    "#ffffff".into()
}

/// Every primitive kind the editor offers, with its SVG element name: (label, element, kind).
pub fn primitive_kinds() -> Vec<(&'static str, &'static str, Kind)> {
    vec![
        ("Gaussian Blur", "feGaussianBlur", Kind::GaussianBlur { input: String::new(), sd_x: 2.0, sd_y: 2.0 }),
        ("Offset", "feOffset", Kind::Offset { input: String::new(), dx: 2.0, dy: 2.0 }),
        ("Flood", "feFlood", Kind::Flood { color: black(), opacity: 1.0 }),
        ("Merge", "feMerge", Kind::Merge { inputs: vec![String::new(), "SourceGraphic".into()] }),
        (
            "Composite",
            "feComposite",
            Kind::Composite { input: String::new(), in2: "SourceGraphic".into(), operator: CompositeOp::Over, k1: 0.0, k2: 0.0, k3: 0.0, k4: 0.0 },
        ),
        ("Color Matrix", "feColorMatrix", Kind::ColorMatrix { input: String::new(), mode: MatrixMode::Saturate, values: vec![0.5] }),
        (
            "Component Transfer",
            "feComponentTransfer",
            Kind::ComponentTransfer {
                input: String::new(),
                func_r: Transfer::Identity,
                func_g: Transfer::Identity,
                func_b: Transfer::Identity,
                func_a: Transfer::Identity,
            },
        ),
        (
            "Convolve Matrix",
            "feConvolveMatrix",
            Kind::ConvolveMatrix {
                input: String::new(),
                columns: 3,
                rows: 3,
                kernel: vec![0.0, -1.0, 0.0, -1.0, 5.0, -1.0, 0.0, -1.0, 0.0],
                divisor: 0.0,
                bias: 0.0,
                target_x: None,
                target_y: None,
                edge_mode: EdgeMode::Duplicate,
                preserve_alpha: false,
            },
        ),
        ("Morphology", "feMorphology", Kind::Morphology { input: String::new(), operator: MorphologyOp::Dilate, radius_x: 1.0, radius_y: 1.0 }),
        (
            "Turbulence",
            "feTurbulence",
            Kind::Turbulence { base_frequency_x: 0.05, base_frequency_y: 0.05, num_octaves: 2, seed: 0, stitch_tiles: false, fractal_noise: false },
        ),
        (
            "Displacement Map",
            "feDisplacementMap",
            Kind::DisplacementMap { input: "SourceGraphic".into(), in2: String::new(), scale: 10.0, x_channel: Channel::R, y_channel: Channel::G },
        ),
        (
            "Diffuse Lighting",
            "feDiffuseLighting",
            Kind::DiffuseLighting { input: String::new(), surface_scale: 1.0, diffuse_constant: 1.0, color: white(), light: Light::default() },
        ),
        (
            "Specular Lighting",
            "feSpecularLighting",
            Kind::SpecularLighting {
                input: String::new(),
                surface_scale: 1.0,
                specular_constant: 1.0,
                specular_exponent: 20.0,
                color: white(),
                light: Light::default(),
            },
        ),
        ("Blend", "feBlend", Kind::Blend { input: String::new(), in2: "SourceGraphic".into(), mode: BlendOp::Multiply }),
        ("Drop Shadow", "feDropShadow", Kind::DropShadow { input: "SourceGraphic".into(), dx: 2.0, dy: 2.0, sd: 2.0, color: black(), opacity: 0.5 }),
        ("Tile", "feTile", Kind::Tile { input: String::new() }),
    ]
}

impl Kind {
    /// The SVG element the primitive is written as.
    pub fn element(&self) -> &'static str {
        match self {
            Kind::GaussianBlur { .. } => "feGaussianBlur",
            Kind::Offset { .. } => "feOffset",
            Kind::Flood { .. } => "feFlood",
            Kind::Merge { .. } => "feMerge",
            Kind::Composite { .. } => "feComposite",
            Kind::ColorMatrix { .. } => "feColorMatrix",
            Kind::ComponentTransfer { .. } => "feComponentTransfer",
            Kind::ConvolveMatrix { .. } => "feConvolveMatrix",
            Kind::Morphology { .. } => "feMorphology",
            Kind::Turbulence { .. } => "feTurbulence",
            Kind::DisplacementMap { .. } => "feDisplacementMap",
            Kind::DiffuseLighting { .. } => "feDiffuseLighting",
            Kind::SpecularLighting { .. } => "feSpecularLighting",
            Kind::Blend { .. } => "feBlend",
            Kind::DropShadow { .. } => "feDropShadow",
            Kind::Tile { .. } => "feTile",
        }
    }

    /// The editor's label of the primitive's kind.
    pub fn label(&self) -> &'static str {
        primitive_kinds().iter().find(|(_, el, _)| *el == self.element()).map_or("Primitive", |(l, _, _)| *l)
    }

    /// The primitive's inputs, in order (`in`, `in2`, a merge's nodes).
    pub fn inputs(&self) -> Vec<&Input> {
        match self {
            Kind::GaussianBlur { input, .. }
            | Kind::Offset { input, .. }
            | Kind::ColorMatrix { input, .. }
            | Kind::ComponentTransfer { input, .. }
            | Kind::ConvolveMatrix { input, .. }
            | Kind::Morphology { input, .. }
            | Kind::DiffuseLighting { input, .. }
            | Kind::SpecularLighting { input, .. }
            | Kind::DropShadow { input, .. }
            | Kind::Tile { input } => vec![input],
            Kind::Composite { input, in2, .. } | Kind::DisplacementMap { input, in2, .. } | Kind::Blend { input, in2, .. } => vec![input, in2],
            Kind::Merge { inputs } => inputs.iter().collect(),
            Kind::Flood { .. } | Kind::Turbulence { .. } => vec![],
        }
    }

    /// The primitive's inputs, mutable, in the order of [`Kind::inputs`].
    pub fn inputs_mut(&mut self) -> Vec<&mut Input> {
        match self {
            Kind::GaussianBlur { input, .. }
            | Kind::Offset { input, .. }
            | Kind::ColorMatrix { input, .. }
            | Kind::ComponentTransfer { input, .. }
            | Kind::ConvolveMatrix { input, .. }
            | Kind::Morphology { input, .. }
            | Kind::DiffuseLighting { input, .. }
            | Kind::SpecularLighting { input, .. }
            | Kind::DropShadow { input, .. }
            | Kind::Tile { input } => vec![input],
            Kind::Composite { input, in2, .. } | Kind::DisplacementMap { input, in2, .. } | Kind::Blend { input, in2, .. } => vec![input, in2],
            Kind::Merge { inputs } => inputs.iter_mut().collect(),
            Kind::Flood { .. } | Kind::Turbulence { .. } => vec![],
        }
    }

    /// How far (document units) the primitive moves or spreads its input.
    fn spread(&self) -> f64 {
        match self {
            Kind::GaussianBlur { sd_x, sd_y, .. } => 3.0 * sd_x.max(*sd_y).max(0.0),
            Kind::Offset { dx, dy, .. } => dx.abs().max(dy.abs()),
            Kind::Morphology { radius_x, radius_y, .. } => radius_x.max(*radius_y).max(0.0),
            Kind::DisplacementMap { scale, .. } => scale.abs() / 2.0,
            Kind::DropShadow { dx, dy, sd, .. } => dx.abs().max(dy.abs()) + 3.0 * sd.max(0.0),
            Kind::ConvolveMatrix { columns, rows, .. } => f64::from((*columns).max(*rows)),
            Kind::DiffuseLighting { .. } | Kind::SpecularLighting { .. } => 1.0,
            _ => 0.0,
        }
    }

    /// Scale the primitive's lengths by `s` (Scale Strokes & Effects).
    pub fn scale(&mut self, s: f64) {
        match self {
            Kind::GaussianBlur { sd_x, sd_y, .. } => {
                *sd_x *= s;
                *sd_y *= s;
            }
            Kind::Offset { dx, dy, .. } => {
                *dx *= s;
                *dy *= s;
            }
            Kind::Morphology { radius_x, radius_y, .. } => {
                *radius_x *= s;
                *radius_y *= s;
            }
            Kind::DisplacementMap { scale, .. } => *scale *= s,
            Kind::DropShadow { dx, dy, sd, .. } => {
                *dx *= s;
                *dy *= s;
                *sd *= s;
            }
            Kind::Turbulence { base_frequency_x, base_frequency_y, .. } if s > 0.0 => {
                *base_frequency_x /= s;
                *base_frequency_y /= s;
            }
            _ => {}
        }
    }
}

impl FilterGraph {
    /// The graph in effect parameters `p` (`{region?, primitives?}`); an empty graph for anything
    /// else.
    pub fn from_params(p: &Value) -> Self {
        let mut g: FilterGraph = serde_json::from_value(p.clone()).unwrap_or_default();
        if g.region.iter().any(|v| !v.is_finite()) || g.region[2] <= 0.0 || g.region[3] <= 0.0 {
            g.region = DEFAULT_REGION;
        }
        g
    }

    /// The graph as effect parameters.
    pub fn to_params(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// The filter region in document coordinates for an object whose bounds are `bounds`.
    pub fn region_rect(&self, bounds: Rect) -> Rect {
        let [x, y, w, h] = self.region;
        let (bw, bh) = (bounds.width(), bounds.height());
        Rect::new(bounds.x0 + x * bw, bounds.y0 + y * bh, bounds.x0 + (x + w) * bw, bounds.y0 + (y + h) * bh)
    }

    /// How far beyond `bounds` the filter can paint: the region's overhang (SVG clips a filter to
    /// its region).
    pub fn outset(&self, bounds: Rect) -> f64 {
        let r = self.region_rect(bounds);
        [bounds.x0 - r.x0, bounds.y0 - r.y0, r.x1 - bounds.x1, r.y1 - bounds.y1].into_iter().fold(0.0, f64::max)
    }

    /// How far from a pixel the content deciding it lies: every primitive's spread, added up
    /// (`None` when a turbulence or a lighting makes the whole region matter).
    pub fn reach(&self) -> Option<f64> {
        Some(self.primitives.iter().map(|p| p.kind.spread()).sum())
    }

    /// Scale every length in the graph by `s`.
    pub fn scale(&mut self, s: f64) {
        for p in &mut self.primitives {
            p.kind.scale(s);
        }
    }

    /// Run the filter on `data`, `w` × `h` premultiplied RGBA8 pixels lying in the document as
    /// `space` says, for an object whose bounds are `bounds`. Does nothing when the sizes don't
    /// match or the graph is empty.
    pub fn apply(&self, data: &mut [u8], w: usize, h: usize, space: &PixelSpace, bounds: Rect) {
        if self.primitives.is_empty() || w == 0 || h == 0 || data.len() != w * h * 4 {
            return;
        }
        let px = space.px.max(1e-9);
        let source: Buf =
            data.chunks_exact(4).map(|c| [f32::from(c[0]), f32::from(c[1]), f32::from(c[2]), f32::from(c[3])].map(|v| v / 255.0)).collect();
        let ctx = Ctx { w, h, px, space, bounds };
        let mut results: HashMap<&str, Buf> = HashMap::new();
        let mut last: Buf = source.clone();
        for prim in &self.primitives {
            let input = |name: &str, last: &Buf| -> Buf {
                match name {
                    "" => last.clone(),
                    "SourceGraphic" => source.clone(),
                    "SourceAlpha" => source.iter().map(|p| [0.0, 0.0, 0.0, p[3]]).collect(),
                    other => results.get(other).cloned().unwrap_or_else(|| vec![[0.0; 4]; w * h]),
                }
            };
            let out = eval(&prim.kind, &ctx, &|n| input(n, &last));
            if !prim.result.is_empty() {
                results.insert(prim.result.as_str(), out.clone());
            }
            last = out;
        }
        // The result, clipped to the filter region.
        let region = self.region_rect(bounds);
        let to_doc = space.to_doc;
        for (i, (o, p)) in data.chunks_exact_mut(4).zip(&last).enumerate() {
            let q = to_doc * Point::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
            let v = if region.contains(q) { *p } else { [0.0; 4] };
            let a = v[3].clamp(0.0, 1.0);
            let c = |k: usize| (v[k].clamp(0.0, a) * 255.0).round() as u8;
            o.copy_from_slice(&[c(0), c(1), c(2), (a * 255.0).round() as u8]);
        }
    }
}

/// Premultiplied RGBA pixels, 0..1.
type Buf = Vec<[f32; 4]>;

struct Ctx<'a> {
    w: usize,
    h: usize,
    px: f64,
    space: &'a PixelSpace,
    bounds: Rect,
}

fn parse_color(s: &str) -> [f32; 3] {
    redrob_vector_doc::color::Color::from_hex(s).map_or([0.0, 0.0, 0.0], |c| c.to_rgb())
}

fn eval(kind: &Kind, c: &Ctx, input: &dyn Fn(&str) -> Buf) -> Buf {
    let (w, h) = (c.w, c.h);
    let to_px = |len: f64| len / c.px;
    match kind {
        Kind::GaussianBlur { input: i, sd_x, sd_y } => {
            let mut b = input(i);
            blur(&mut b, w, h, to_px(*sd_x), to_px(*sd_y));
            b
        }
        Kind::Offset { input: i, dx, dy } => offset(&input(i), w, h, to_px(*dx), to_px(*dy)),
        Kind::Flood { color, opacity } => {
            let [r, g, b] = parse_color(color);
            let a = opacity.clamp(0.0, 1.0) as f32;
            vec![[r * a, g * a, b * a, a]; w * h]
        }
        Kind::Merge { inputs } => {
            let mut out = vec![[0.0; 4]; w * h];
            for i in inputs {
                let layer = input(i);
                for (o, p) in out.iter_mut().zip(&layer) {
                    *o = over(*p, *o);
                }
            }
            out
        }
        Kind::Composite { input: i, in2, operator, k1, k2, k3, k4 } => {
            let (a, b) = (input(i), input(in2));
            let (k1, k2, k3, k4) = (*k1 as f32, *k2 as f32, *k3 as f32, *k4 as f32);
            a.iter()
                .zip(&b)
                .map(|(p, q)| match operator {
                    CompositeOp::Over => over(*p, *q),
                    CompositeOp::In => p.map(|v| v * q[3]),
                    CompositeOp::Out => p.map(|v| v * (1.0 - q[3])),
                    CompositeOp::Atop => [0, 1, 2, 3].map(|k| p[k] * q[3] + q[k] * (1.0 - p[3])),
                    CompositeOp::Xor => [0, 1, 2, 3].map(|k| p[k] * (1.0 - q[3]) + q[k] * (1.0 - p[3])),
                    CompositeOp::Arithmetic => {
                        let a = (k1 * p[3] * q[3] + k2 * p[3] + k3 * q[3] + k4).clamp(0.0, 1.0);
                        let ch = |k: usize| (k1 * p[k] * q[k] + k2 * p[k] + k3 * q[k] + k4).clamp(0.0, a);
                        [ch(0), ch(1), ch(2), a]
                    }
                })
                .collect()
        }
        Kind::ColorMatrix { input: i, mode, values } => {
            let m = color_matrix(*mode, values);
            let mut b = input(i);
            apply_matrix(&mut b, &m);
            b
        }
        Kind::ComponentTransfer { input: i, func_r, func_g, func_b, func_a } => {
            let mut b = input(i);
            let funcs = [func_r, func_g, func_b, func_a];
            for p in &mut b {
                let mut s = unpremul(*p);
                for (k, f) in funcs.iter().enumerate() {
                    s[k] = transfer(f, s[k]).clamp(0.0, 1.0);
                }
                *p = premul(s);
            }
            b
        }
        Kind::ConvolveMatrix { input: i, columns, rows, kernel, divisor, bias, target_x, target_y, edge_mode, preserve_alpha } => {
            convolve(&input(i), w, h, *columns, *rows, kernel, *divisor, *bias, *target_x, *target_y, *edge_mode, *preserve_alpha)
        }
        Kind::Morphology { input: i, operator, radius_x, radius_y } => {
            let r = |v: f64| to_px(v).round().clamp(0.0, 256.0) as usize;
            morphology(&input(i), w, h, *operator, r(*radius_x), r(*radius_y))
        }
        Kind::Turbulence { base_frequency_x, base_frequency_y, num_octaves, seed, stitch_tiles: _, fractal_noise } => {
            turbulence(c, *base_frequency_x, *base_frequency_y, *num_octaves, *seed, *fractal_noise)
        }
        Kind::DisplacementMap { input: i, in2, scale, x_channel, y_channel } => {
            displace(&input(i), &input(in2), w, h, to_px(*scale), *x_channel, *y_channel)
        }
        Kind::DiffuseLighting { input: i, surface_scale, diffuse_constant, color, light } => {
            lighting(c, &input(i), *surface_scale, *diffuse_constant, 0.0, color, *light, false)
        }
        Kind::SpecularLighting { input: i, surface_scale, specular_constant, specular_exponent, color, light } => {
            lighting(c, &input(i), *surface_scale, *specular_constant, *specular_exponent, color, *light, true)
        }
        Kind::Blend { input: i, in2, mode } => {
            let (a, b) = (input(i), input(in2));
            a.iter().zip(&b).map(|(p, q)| blend(*mode, *p, *q)).collect()
        }
        Kind::DropShadow { input: i, dx, dy, sd, color, opacity } => {
            let src = input(i);
            let mut shadow: Buf = src.iter().map(|p| [0.0, 0.0, 0.0, p[3]]).collect();
            blur(&mut shadow, w, h, to_px(*sd), to_px(*sd));
            let shadow = offset(&shadow, w, h, to_px(*dx), to_px(*dy));
            let [r, g, b] = parse_color(color);
            let o = opacity.clamp(0.0, 1.0) as f32;
            shadow
                .iter()
                .zip(&src)
                .map(|(s, p)| {
                    let a = s[3] * o;
                    over(*p, [r * a, g * a, b * a, a])
                })
                .collect()
        }
        Kind::Tile { input: i } => input(i),
    }
}

fn over(top: [f32; 4], bottom: [f32; 4]) -> [f32; 4] {
    [0, 1, 2, 3].map(|k| top[k] + bottom[k] * (1.0 - top[3]))
}

fn unpremul(p: [f32; 4]) -> [f32; 4] {
    if p[3] <= 1e-6 { [0.0, 0.0, 0.0, 0.0] } else { [p[0] / p[3], p[1] / p[3], p[2] / p[3], p[3]] }
}

fn premul(s: [f32; 4]) -> [f32; 4] {
    let a = s[3].clamp(0.0, 1.0);
    [s[0].clamp(0.0, 1.0) * a, s[1].clamp(0.0, 1.0) * a, s[2].clamp(0.0, 1.0) * a, a]
}

/// Radii of three box blurs approximating a Gaussian of `sigma` pixels.
fn boxes(sigma: f64) -> [usize; 3] {
    crate::pixel::gauss_boxes(sigma)
}

/// One box blur of radius `r` along rows of `w` pixels (transparent past the ends).
fn box_rows(src: &[[f32; 4]], dst: &mut [[f32; 4]], w: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f32;
    for (row, out) in src.chunks_exact(w).zip(dst.chunks_exact_mut(w)) {
        let mut acc = [0.0f32; 4];
        for p in row.iter().take(r + 1) {
            acc.iter_mut().zip(p).for_each(|(a, v)| *a += v);
        }
        for (x, o) in out.iter_mut().enumerate() {
            *o = acc.map(|v| v * norm);
            if let Some(p) = row.get(x + r + 1) {
                acc.iter_mut().zip(p).for_each(|(a, v)| *a += v);
            }
            if let Some(p) = x.checked_sub(r).and_then(|i| row.get(i)) {
                acc.iter_mut().zip(p).for_each(|(a, v)| *a = (*a - v).max(0.0));
            }
        }
    }
}

fn transpose(src: &[[f32; 4]], dst: &mut [[f32; 4]], w: usize, h: usize) {
    for (y, row) in src.chunks_exact(w).enumerate().take(h) {
        for (x, v) in row.iter().enumerate() {
            if let Some(d) = dst.get_mut(x * h + y) {
                *d = *v;
            }
        }
    }
}

/// Gaussian blur with σ `sx` along x and `sy` along y (pixels).
fn blur(b: &mut Buf, w: usize, h: usize, sx: f64, sy: f64) {
    if b.len() != w * h {
        return;
    }
    let mut tmp = vec![[0.0f32; 4]; b.len()];
    for (len, other, sigma) in [(w, h, sx), (h, w, sy)] {
        if sigma.is_finite() && sigma >= 0.2 {
            for r in boxes(sigma.min(len as f64)) {
                if r > 0 {
                    box_rows(b, &mut tmp, len, r);
                    std::mem::swap(b, &mut tmp);
                }
            }
        }
        transpose(b, &mut tmp, len, other);
        std::mem::swap(b, &mut tmp);
    }
}

fn offset(src: &Buf, w: usize, h: usize, dx: f64, dy: f64) -> Buf {
    let (dx, dy) = (dx.round() as i64, dy.round() as i64);
    let mut out = vec![[0.0f32; 4]; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let (sx, sy) = (x - dx, y - dy);
            if sx >= 0
                && sy >= 0
                && sx < w as i64
                && sy < h as i64
                && let (Some(o), Some(s)) = (out.get_mut(y as usize * w + x as usize), src.get(sy as usize * w + sx as usize))
            {
                *o = *s;
            }
        }
    }
    out
}

/// The 5 × 4 matrix of a colour matrix primitive (rows R, G, B, A over R, G, B, A, 1).
pub fn color_matrix(mode: MatrixMode, values: &[f64]) -> [f32; 20] {
    let identity: [f32; 20] = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    match mode {
        MatrixMode::Matrix => {
            let mut m = identity;
            if values.len() >= 20 {
                for (d, v) in m.iter_mut().zip(values) {
                    *d = *v as f32;
                }
            }
            m
        }
        MatrixMode::Saturate => {
            let s = values.first().copied().unwrap_or(1.0).max(0.0) as f32;
            [
                0.213 + 0.787 * s,
                0.715 - 0.715 * s,
                0.072 - 0.072 * s,
                0.0,
                0.0,
                0.213 - 0.213 * s,
                0.715 + 0.285 * s,
                0.072 - 0.072 * s,
                0.0,
                0.0,
                0.213 - 0.213 * s,
                0.715 - 0.715 * s,
                0.072 + 0.928 * s,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
            ]
        }
        MatrixMode::HueRotate => {
            let a = values.first().copied().unwrap_or(0.0).to_radians();
            let (c, s) = (a.cos() as f32, a.sin() as f32);
            [
                0.213 + c * 0.787 - s * 0.213,
                0.715 - c * 0.715 - s * 0.715,
                0.072 - c * 0.072 + s * 0.928,
                0.0,
                0.0,
                0.213 - c * 0.213 + s * 0.143,
                0.715 + c * 0.285 + s * 0.140,
                0.072 - c * 0.072 - s * 0.283,
                0.0,
                0.0,
                0.213 - c * 0.213 - s * 0.787,
                0.715 - c * 0.715 + s * 0.715,
                0.072 + c * 0.928 + s * 0.072,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
            ]
        }
        MatrixMode::LuminanceToAlpha => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2125, 0.7154, 0.0721, 0.0, 0.0],
    }
}

/// Apply a 5 × 4 colour matrix to premultiplied pixels (on their straight colours).
pub fn apply_matrix(b: &mut [[f32; 4]], m: &[f32; 20]) {
    for p in b.iter_mut() {
        let s = unpremul(*p);
        let row = |r: usize| m[r * 5] * s[0] + m[r * 5 + 1] * s[1] + m[r * 5 + 2] * s[2] + m[r * 5 + 3] * s[3] + m[r * 5 + 4];
        *p = premul([row(0), row(1), row(2), row(3)]);
    }
}

fn transfer(f: &Transfer, c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    match f {
        Transfer::Identity => c,
        Transfer::Table { values } => {
            let n = values.len();
            if n < 2 {
                return c;
            }
            let x = c * (n - 1) as f32;
            let k = (x.floor() as usize).min(n - 2);
            let (v0, v1) = (values[k] as f32, values[k + 1] as f32);
            v0 + (x - k as f32) * (v1 - v0)
        }
        Transfer::Discrete { values } => {
            let n = values.len();
            if n == 0 {
                return c;
            }
            values[((c * n as f32).floor() as usize).min(n - 1)] as f32
        }
        Transfer::Linear { slope, intercept } => *slope as f32 * c + *intercept as f32,
        Transfer::Gamma { amplitude, exponent, offset } => *amplitude as f32 * c.powf(*exponent as f32) + *offset as f32,
    }
}

#[allow(clippy::too_many_arguments)]
fn convolve(
    src: &Buf,
    w: usize,
    h: usize,
    columns: u32,
    rows: u32,
    kernel: &[f64],
    divisor: f64,
    bias: f64,
    target_x: Option<u32>,
    target_y: Option<u32>,
    edge: EdgeMode,
    preserve_alpha: bool,
) -> Buf {
    let (cols, rows) = (columns.clamp(1, 64) as usize, rows.clamp(1, 64) as usize);
    if kernel.len() != cols * rows {
        return src.clone();
    }
    let sum: f64 = kernel.iter().sum();
    let div = if divisor != 0.0 {
        divisor
    } else if sum != 0.0 {
        sum
    } else {
        1.0
    };
    let (tx, ty) = (target_x.map_or(cols / 2, |t| (t as usize).min(cols - 1)), target_y.map_or(rows / 2, |t| (t as usize).min(rows - 1)));
    let straight: Buf = if preserve_alpha { src.iter().map(|p| unpremul(*p)).collect() } else { src.clone() };
    let fetch = |x: i64, y: i64| -> Option<[f32; 4]> {
        let (wi, hi) = (w as i64, h as i64);
        let (x, y) = match edge {
            EdgeMode::Duplicate => (x.clamp(0, wi - 1), y.clamp(0, hi - 1)),
            EdgeMode::Wrap => (x.rem_euclid(wi), y.rem_euclid(hi)),
            EdgeMode::None => {
                if x < 0 || y < 0 || x >= wi || y >= hi {
                    return Some([0.0; 4]);
                }
                (x, y)
            }
        };
        straight.get(y as usize * w + x as usize).copied()
    };
    let mut out = vec![[0.0f32; 4]; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f64; 4];
            for j in 0..rows {
                for i in 0..cols {
                    // The kernel is applied rotated by 180° (the SVG definition).
                    let k = kernel[(rows - 1 - j) * cols + (cols - 1 - i)];
                    let (sx, sy) = (x as i64 + i as i64 - tx as i64, y as i64 + j as i64 - ty as i64);
                    if let Some(p) = fetch(sx, sy) {
                        acc.iter_mut().zip(p).for_each(|(a, v)| *a += k * f64::from(v));
                    }
                }
            }
            let v = acc.map(|a| (a / div + bias) as f32);
            let p = if preserve_alpha {
                let a = src.get(y * w + x).map_or(0.0, |p| p[3]);
                premul([v[0], v[1], v[2], a])
            } else {
                let a = v[3].clamp(0.0, 1.0);
                [v[0].clamp(0.0, a), v[1].clamp(0.0, a), v[2].clamp(0.0, a), a]
            };
            if let Some(o) = out.get_mut(y * w + x) {
                *o = p;
            }
        }
    }
    out
}

fn morphology(src: &Buf, w: usize, h: usize, op: MorphologyOp, rx: usize, ry: usize) -> Buf {
    if rx == 0 && ry == 0 {
        return src.clone();
    }
    let pick = |a: f32, b: f32| match op {
        MorphologyOp::Erode => a.min(b),
        MorphologyOp::Dilate => a.max(b),
    };
    let start = match op {
        MorphologyOp::Erode => [1.0f32; 4],
        MorphologyOp::Dilate => [0.0f32; 4],
    };
    // Rows, then columns (the operators are separable).
    let mut tmp = vec![start; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = start;
            for sx in x.saturating_sub(rx)..=(x + rx).min(w - 1) {
                if let Some(p) = src.get(y * w + sx) {
                    acc.iter_mut().zip(p).for_each(|(a, v)| *a = pick(*a, *v));
                }
            }
            if let Some(o) = tmp.get_mut(y * w + x) {
                *o = acc;
            }
        }
    }
    let mut out = vec![start; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = start;
            for sy in y.saturating_sub(ry)..=(y + ry).min(h - 1) {
                if let Some(p) = tmp.get(sy * w + x) {
                    acc.iter_mut().zip(p).for_each(|(a, v)| *a = pick(*a, *v));
                }
            }
            if let Some(o) = out.get_mut(y * w + x) {
                *o = acc;
            }
        }
    }
    out
}

/// Perlin turbulence as the SVG specification defines `feTurbulence` (its lattice, gradients
/// and octave sum), seeded as it says.
struct Perlin {
    lattice: Vec<usize>,
    gradient: Vec<[[f64; 2]; 4]>,
}

const B_SIZE: usize = 0x100;
const B_MASK: usize = 0xff;
const PERLIN_N: f64 = 0x1000 as f64;

impl Perlin {
    fn new(seed: i32) -> Self {
        // The specification's linear congruential generator.
        const RAND_M: i64 = 2147483647;
        const RAND_A: i64 = 16807;
        const RAND_Q: i64 = 127773;
        const RAND_R: i64 = 2836;
        let mut s = i64::from(seed);
        if s <= 0 {
            s = -(s % (RAND_M - 1)) + 1;
        }
        if s > RAND_M - 1 {
            s = RAND_M - 1;
        }
        let mut random = move || {
            let mut r = RAND_A * (s % RAND_Q) - RAND_R * (s / RAND_Q);
            if r <= 0 {
                r += RAND_M;
            }
            s = r;
            r
        };
        let mut lattice = vec![0usize; B_SIZE + B_SIZE + 2];
        let mut gradient = vec![[[0.0f64; 2]; 4]; B_SIZE + B_SIZE + 2];
        for k in 0..4 {
            for (i, (l, grad)) in lattice.iter_mut().zip(gradient.iter_mut()).enumerate().take(B_SIZE) {
                *l = i;
                let mut g = [0.0f64; 2];
                for v in &mut g {
                    *v = ((random() % (2 * B_SIZE as i64)) - B_SIZE as i64) as f64 / B_SIZE as f64;
                }
                let len = (g[0] * g[0] + g[1] * g[1]).sqrt();
                if len > 0.0 {
                    g = [g[0] / len, g[1] / len];
                }
                grad[k] = g;
            }
        }
        for i in (1..B_SIZE).rev() {
            let j = (random() % B_SIZE as i64) as usize;
            lattice.swap(i, j);
        }
        for i in 0..B_SIZE + 2 {
            lattice[B_SIZE + i] = lattice[i];
            gradient[B_SIZE + i] = gradient[i];
        }
        Self { lattice, gradient }
    }

    fn noise2(&self, channel: usize, x: f64, y: f64) -> f64 {
        let s_curve = |t: f64| t * t * (3.0 - 2.0 * t);
        let lerp = |t: f64, a: f64, b: f64| a + t * (b - a);
        let t = x + PERLIN_N;
        let bx0 = (t as usize) & B_MASK;
        let bx1 = (bx0 + 1) & B_MASK;
        let rx0 = t - t.floor();
        let rx1 = rx0 - 1.0;
        let t = y + PERLIN_N;
        let by0 = (t as usize) & B_MASK;
        let by1 = (by0 + 1) & B_MASK;
        let ry0 = t - t.floor();
        let ry1 = ry0 - 1.0;
        let i = self.lattice[bx0];
        let j = self.lattice[bx1];
        let b00 = self.lattice[i + by0];
        let b10 = self.lattice[j + by0];
        let b01 = self.lattice[i + by1];
        let b11 = self.lattice[j + by1];
        let sx = s_curve(rx0);
        let sy = s_curve(ry0);
        let dot = |g: [f64; 2], rx: f64, ry: f64| rx * g[0] + ry * g[1];
        let u = dot(self.gradient[b00][channel], rx0, ry0);
        let v = dot(self.gradient[b10][channel], rx1, ry0);
        let a = lerp(sx, u, v);
        let u = dot(self.gradient[b01][channel], rx0, ry1);
        let v = dot(self.gradient[b11][channel], rx1, ry1);
        let b = lerp(sx, u, v);
        lerp(sy, a, b)
    }

    fn turbulence(&self, channel: usize, x: f64, y: f64, fx: f64, fy: f64, octaves: u32, fractal: bool) -> f64 {
        let mut sum = 0.0;
        let (mut vx, mut vy) = (x * fx, y * fy);
        let mut ratio = 1.0;
        for _ in 0..octaves.clamp(1, 16) {
            let n = self.noise2(channel, vx, vy);
            sum += if fractal { n / ratio } else { n.abs() / ratio };
            vx *= 2.0;
            vy *= 2.0;
            ratio *= 2.0;
        }
        if fractal { (sum + 1.0) / 2.0 } else { sum }
    }
}

fn turbulence(c: &Ctx, fx: f64, fy: f64, octaves: u32, seed: i32, fractal: bool) -> Buf {
    let perlin = Perlin::new(seed);
    let (w, h) = (c.w, c.h);
    let mut out = vec![[0.0f32; 4]; w * h];
    // Noise coordinates are document coordinates from the object's origin, so the texture moves
    // with the object.
    let origin = c.bounds.origin();
    for (i, o) in out.iter_mut().enumerate() {
        let q = c.space.to_doc * Point::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
        let (x, y) = (q.x - origin.x, q.y - origin.y);
        let v = [0, 1, 2, 3].map(|ch| perlin.turbulence(ch, x, y, fx.max(0.0), fy.max(0.0), octaves, fractal).clamp(0.0, 1.0) as f32);
        *o = premul(v);
    }
    out
}

fn channel(p: [f32; 4], ch: Channel) -> f32 {
    let s = unpremul(p);
    match ch {
        Channel::R => s[0],
        Channel::G => s[1],
        Channel::B => s[2],
        Channel::A => s[3],
    }
}

fn displace(src: &Buf, map: &Buf, w: usize, h: usize, scale: f64, xc: Channel, yc: Channel) -> Buf {
    let mut out = vec![[0.0f32; 4]; w * h];
    for y in 0..h {
        for x in 0..w {
            let Some(m) = map.get(y * w + x) else { continue };
            let sx = x as f64 + scale * (f64::from(channel(*m, xc)) - 0.5);
            let sy = y as f64 + scale * (f64::from(channel(*m, yc)) - 0.5);
            let (ix, iy) = (sx.round(), sy.round());
            if ix >= 0.0
                && iy >= 0.0
                && ix < w as f64
                && iy < h as f64
                && let (Some(o), Some(p)) = (out.get_mut(y * w + x), src.get(iy as usize * w + ix as usize))
            {
                *o = *p;
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn lighting(c: &Ctx, src: &Buf, surface_scale: f64, constant: f64, exponent: f64, color: &str, light: Light, specular: bool) -> Buf {
    let (w, h) = (c.w, c.h);
    let [lr, lg, lb] = parse_color(color);
    let alpha = |x: i64, y: i64| -> f64 {
        let (x, y) = (x.clamp(0, w as i64 - 1) as usize, y.clamp(0, h as i64 - 1) as usize);
        src.get(y * w + x).map_or(0.0, |p| f64::from(p[3]))
    };
    let px = c.px;
    let origin = c.bounds.origin();
    let mut out = vec![[0.0f32; 4]; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            // Sobel normal of the height map (alpha × surfaceScale), in pixels.
            let nx = -surface_scale / 4.0
                * ((alpha(x + 1, y - 1) + 2.0 * alpha(x + 1, y) + alpha(x + 1, y + 1))
                    - (alpha(x - 1, y - 1) + 2.0 * alpha(x - 1, y) + alpha(x - 1, y + 1)));
            let ny = -surface_scale / 4.0
                * ((alpha(x - 1, y + 1) + 2.0 * alpha(x, y + 1) + alpha(x + 1, y + 1))
                    - (alpha(x - 1, y - 1) + 2.0 * alpha(x, y - 1) + alpha(x + 1, y - 1)));
            let nlen = (nx * nx + ny * ny + 1.0).sqrt();
            let n = [nx / nlen, ny / nlen, 1.0 / nlen];
            let z = surface_scale * alpha(x, y);
            // The surface point in document units from the object's origin.
            let q = c.space.to_doc * Point::new(x as f64 + 0.5, y as f64 + 0.5);
            let (sx, sy) = (q.x - origin.x, q.y - origin.y);
            let (l, light_color) = match light {
                Light::Distant { azimuth, elevation } => {
                    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
                    ([a.cos() * e.cos(), a.sin() * e.cos(), e.sin()], [lr, lg, lb])
                }
                Light::Point { x: lx, y: ly, z: lz } => {
                    let d = [lx - sx, ly - sy, lz - z * px];
                    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-9);
                    ([d[0] / len, d[1] / len, d[2] / len], [lr, lg, lb])
                }
                Light::Spot { x: lx, y: ly, z: lz, px: tx, py: ty, pz: tz, exponent: se, cone } => {
                    let d = [lx - sx, ly - sy, lz - z * px];
                    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-9);
                    let l = [d[0] / len, d[1] / len, d[2] / len];
                    let s = [tx - lx, ty - ly, tz - lz];
                    let slen = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt().max(1e-9);
                    let s = [s[0] / slen, s[1] / slen, s[2] / slen];
                    let minus_l_dot_s = -(l[0] * s[0] + l[1] * s[1] + l[2] * s[2]);
                    let mut k = if minus_l_dot_s <= 0.0 { 0.0 } else { minus_l_dot_s.powf(se.max(0.0)) };
                    if cone > 0.0 && minus_l_dot_s < cone.to_radians().cos() {
                        k = 0.0;
                    }
                    (l, [lr * k as f32, lg * k as f32, lb * k as f32])
                }
            };
            let n_dot_l = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0);
            let v = if specular {
                let hv = [l[0], l[1], l[2] + 1.0];
                let hlen = (hv[0] * hv[0] + hv[1] * hv[1] + hv[2] * hv[2]).sqrt().max(1e-9);
                let n_dot_h = ((n[0] * hv[0] + n[1] * hv[1] + n[2] * hv[2]) / hlen).max(0.0);
                let k = constant * n_dot_h.powf(exponent.clamp(1.0, 128.0));
                let rgb = light_color.map(|lc| (f64::from(lc) * k).clamp(0.0, 1.0) as f32);
                let a = rgb[0].max(rgb[1]).max(rgb[2]);
                [rgb[0], rgb[1], rgb[2], a]
            } else {
                let k = constant * n_dot_l;
                let rgb = light_color.map(|lc| (f64::from(lc) * k).clamp(0.0, 1.0) as f32);
                [rgb[0], rgb[1], rgb[2], 1.0]
            };
            if let Some(o) = out.get_mut(y as usize * w + x as usize) {
                *o = v;
            }
        }
    }
    out
}

fn blend(mode: BlendOp, top: [f32; 4], bottom: [f32; 4]) -> [f32; 4] {
    let (ta, ba) = (top[3], bottom[3]);
    let a = ta + ba - ta * ba;
    let (t, b) = (unpremul(top), unpremul(bottom));
    let f = |cs: f32, cb: f32| -> f32 {
        match mode {
            BlendOp::Normal => cs,
            BlendOp::Multiply => cs * cb,
            BlendOp::Screen => cs + cb - cs * cb,
            BlendOp::Overlay => {
                if cb <= 0.5 {
                    2.0 * cs * cb
                } else {
                    1.0 - 2.0 * (1.0 - cs) * (1.0 - cb)
                }
            }
            BlendOp::Darken => cs.min(cb),
            BlendOp::Lighten => cs.max(cb),
            BlendOp::ColorDodge => {
                if cb <= 0.0 {
                    0.0
                } else if cs >= 1.0 {
                    1.0
                } else {
                    (cb / (1.0 - cs)).min(1.0)
                }
            }
            BlendOp::ColorBurn => {
                if cb >= 1.0 {
                    1.0
                } else if cs <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - cb) / cs).min(1.0)
                }
            }
            BlendOp::HardLight => {
                if cs <= 0.5 {
                    2.0 * cs * cb
                } else {
                    1.0 - 2.0 * (1.0 - cs) * (1.0 - cb)
                }
            }
            BlendOp::SoftLight => {
                if cs <= 0.5 {
                    cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
                } else {
                    let d = if cb <= 0.25 { ((16.0 * cb - 12.0) * cb + 4.0) * cb } else { cb.sqrt() };
                    cb + (2.0 * cs - 1.0) * (d - cb)
                }
            }
            BlendOp::Difference => (cs - cb).abs(),
            BlendOp::Exclusion => cs + cb - 2.0 * cs * cb,
        }
    };
    // The separable blend of the SVG compositing specification, premultiplied.
    let ch = |k: usize| (1.0 - ba) * top[k] + (1.0 - ta) * bottom[k] + ta * ba * f(t[k], b[k]);
    [ch(0), ch(1), ch(2), a]
}
