//! The stock filter presets (Effect › Filters › <category>, the Filter Gallery panel): named
//! filter graphs built from the SVG primitives of [`crate::svgfilter`], grouped as the Filters menu
//! groups them (Bevels, Bumps, Color, Distort, Fill and Overlays, Materials, Morphology, Non-realistic
//! 3D shaders, Protrusions, Ridges, Scatter, Shadows and Glows, Textures, Transparency utilities).
//! Applying one puts a `filter.graph` effect on the object, so the Filter Editor edits it afterwards
//! like any other graph. Every preset is written here from the primitives' definitions (the SVG
//! specification), not copied from another program's stock filters.

use crate::svgfilter::{BlendOp, Channel, CompositeOp, EdgeMode, FilterGraph, Kind, Light, MatrixMode, MorphologyOp, Primitive, Transfer};

/// One stock preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterPreset {
    /// Stable id, `<category>.<name>`, e.g. `bevels.bloom`.
    pub id: &'static str,
    /// Menu label.
    pub label: &'static str,
    /// The category label (a Filters submenu).
    pub category: &'static str,
    /// Builds the graph.
    pub build: fn() -> FilterGraph,
}

/// The categories in menu order.
pub const CATEGORIES: [&str; 14] = [
    "Bevels",
    "Bumps",
    "Color",
    "Distort",
    "Fill and Overlays",
    "Materials",
    "Morphology",
    "Non-realistic 3D Shaders",
    "Protrusions",
    "Ridges",
    "Scatter",
    "Shadows and Glows",
    "Textures",
    "Transparency Utilities",
];

const fn p(id: &'static str, label: &'static str, category: &'static str, build: fn() -> FilterGraph) -> FilterPreset {
    FilterPreset { id, label, category, build }
}

/// Every preset, grouped by category in menu order.
pub const PRESETS: &[FilterPreset] = &[
    p("bevels.bloom", "Bloom", "Bevels", bloom),
    p("bevels.button", "Button", "Bevels", button),
    p("bevels.matteJelly", "Matte Jelly", "Bevels", matte_jelly),
    p("bevels.neon", "Neon", "Bevels", neon),
    p("bevels.specularLight", "Specular Light", "Bevels", specular_light),
    p("bevels.translucent", "Translucent", "Bevels", translucent),
    p("bumps.bump", "Bump", "Bumps", bump),
    p("bumps.canvasBumps", "Canvas Bumps", "Bumps", canvas_bumps),
    p("bumps.embossedLeather", "Embossed Leather", "Bumps", embossed_leather),
    p("bumps.paperBump", "Paper Bump", "Bumps", paper_bump),
    p("bumps.roughPaper", "Rough Paper", "Bumps", rough_paper),
    p("color.brilliance", "Brilliance", "Color", brilliance),
    p("color.colorBlindness", "Color Blindness", "Color", color_blindness),
    p("color.duochrome", "Duochrome", "Color", duochrome),
    p("color.extractChannel", "Extract Channel", "Color", extract_channel),
    p("color.fadeToBlack", "Fade to Black", "Color", fade_to_black),
    p("color.greyscale", "Greyscale", "Color", greyscale),
    p("color.invert", "Invert", "Color", invert),
    p("color.quadritone", "Quadritone", "Color", quadritone),
    p("color.solarize", "Solarize", "Color", solarize),
    p("color.tritone", "Tritone", "Color", tritone),
    p("distort.chalkAndSponge", "Chalk and Sponge", "Distort", chalk_and_sponge),
    p("distort.feltFeather", "Felt Feather", "Distort", felt_feather),
    p("distort.roughen", "Roughen", "Distort", roughen),
    p("distort.swirl", "Swirl", "Distort", swirl),
    p("distort.tornEdges", "Torn Edges", "Distort", torn_edges),
    p("distort.waves", "Waves", "Distort", waves),
    p("fill.noiseFill", "Noise Fill", "Fill and Overlays", noise_fill),
    p("fill.silhouette", "Silhouette", "Fill and Overlays", silhouette),
    p("fill.hslNoise", "HSL Noise", "Fill and Overlays", hsl_noise),
    p("fill.spotsOverlay", "Spots Overlay", "Fill and Overlays", spots_overlay),
    p("materials.marble", "3D Marble", "Materials", marble),
    p("materials.wood", "3D Wood", "Materials", wood),
    p("materials.metallizedPaint", "Metallized Paint", "Materials", metallized_paint),
    p("materials.gold", "Gold", "Materials", gold),
    p("materials.chrome", "Chrome", "Materials", chrome),
    p("morphology.outline", "Outline", "Morphology", outline),
    p("morphology.inkBlot", "Ink Blot", "Morphology", ink_blot),
    p("morphology.smoothEdges", "Smooth Edges", "Morphology", smooth_edges),
    p("morphology.blackHole", "Black Hole", "Morphology", black_hole),
    p("shaders.aluminium", "Aluminium", "Non-realistic 3D Shaders", aluminium),
    p("shaders.comicsCream", "Comics Cream", "Non-realistic 3D Shaders", comics_cream),
    p("shaders.darkAndGlow", "Dark and Glow", "Non-realistic 3D Shaders", dark_and_glow),
    p("protrusions.spikes", "Spikes", "Protrusions", spikes),
    p("protrusions.blobs", "Blobs", "Protrusions", blobs),
    p("ridges.dragee", "Dragee", "Ridges", dragee),
    p("ridges.glowingMetal", "Glowing Metal", "Ridges", glowing_metal),
    p("ridges.thinMembrane", "Thin Membrane", "Ridges", thin_membrane),
    p("scatter.airSpray", "Air Spray", "Scatter", air_spray),
    p("scatter.cubes", "Cubes", "Scatter", cubes),
    p("scatter.leopardFur", "Leopard Fur", "Scatter", leopard_fur),
    p("scatter.pointillism", "Pointillism", "Scatter", pointillism),
    p("scatter.stoneWall", "Stone Wall", "Scatter", stone_wall),
    p("shadows.dropShadow", "Drop Shadow", "Shadows and Glows", drop_shadow),
    p("shadows.innerShadow", "Inner Shadow", "Shadows and Glows", inner_shadow),
    p("shadows.glow", "Glow", "Shadows and Glows", glow),
    p("textures.crackedGlass", "Cracked Glass", "Textures", cracked_glass),
    p("textures.inkSplash", "Ink Splash", "Textures", ink_splash),
    p("textures.roughTransparency", "Rough Transparency", "Textures", rough_transparency),
    p("textures.watercolor", "Watercolor", "Textures", watercolor),
    p("transparency.channelToAlpha", "Channel to Alpha", "Transparency Utilities", channel_to_alpha),
    p("transparency.lightEraser", "Light Eraser", "Transparency Utilities", light_eraser),
    p("transparency.luminanceToAlpha", "Luminance to Alpha", "Transparency Utilities", luminance_to_alpha),
    p("transparency.saturationMap", "Saturation Map", "Transparency Utilities", saturation_map),
];

/// The preset with id `id`.
pub fn preset(id: &str) -> Option<&'static FilterPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// The presets of a category, in order.
pub fn presets_in(category: &str) -> impl Iterator<Item = &'static FilterPreset> {
    PRESETS.iter().filter(move |p| p.category == category)
}

/// The presets whose label or category contains `query` (case-insensitive; every preset for an
/// empty query).
pub fn search(query: &str) -> Vec<&'static FilterPreset> {
    let q = query.trim().to_lowercase();
    PRESETS.iter().filter(|p| q.is_empty() || p.label.to_lowercase().contains(&q) || p.category.to_lowercase().contains(&q)).collect()
}

// ---------- building blocks ----------

fn graph(name: &str, prims: Vec<Primitive>) -> FilterGraph {
    FilterGraph { name: name.into(), primitives: prims, ..FilterGraph::default() }
}

fn prim(result: &str, kind: Kind) -> Primitive {
    Primitive::new(result, kind)
}

fn blur(result: &str, input: &str, sd: f64) -> Primitive {
    prim(result, Kind::GaussianBlur { input: input.into(), sd_x: sd, sd_y: sd })
}

fn offset(result: &str, input: &str, dx: f64, dy: f64) -> Primitive {
    prim(result, Kind::Offset { input: input.into(), dx, dy })
}

fn flood(result: &str, color: &str, opacity: f64) -> Primitive {
    prim(result, Kind::Flood { color: color.into(), opacity })
}

fn composite(result: &str, input: &str, in2: &str, operator: CompositeOp) -> Primitive {
    prim(result, Kind::Composite { input: input.into(), in2: in2.into(), operator, k1: 0.0, k2: 0.0, k3: 0.0, k4: 0.0 })
}

fn arithmetic(result: &str, input: &str, in2: &str, k: [f64; 4]) -> Primitive {
    prim(result, Kind::Composite { input: input.into(), in2: in2.into(), operator: CompositeOp::Arithmetic, k1: k[0], k2: k[1], k3: k[2], k4: k[3] })
}

fn merge(result: &str, inputs: &[&str]) -> Primitive {
    prim(result, Kind::Merge { inputs: inputs.iter().map(|s| (*s).into()).collect() })
}

fn blend(result: &str, input: &str, in2: &str, mode: BlendOp) -> Primitive {
    prim(result, Kind::Blend { input: input.into(), in2: in2.into(), mode })
}

fn matrix(result: &str, input: &str, mode: MatrixMode, values: &[f64]) -> Primitive {
    prim(result, Kind::ColorMatrix { input: input.into(), mode, values: values.to_vec() })
}

fn transfer(result: &str, input: &str, r: Transfer, g: Transfer, b: Transfer, a: Transfer) -> Primitive {
    prim(result, Kind::ComponentTransfer { input: input.into(), func_r: r, func_g: g, func_b: b, func_a: a })
}

fn turbulence(result: &str, freq: f64, octaves: u32, seed: i32, fractal: bool) -> Primitive {
    prim(
        result,
        Kind::Turbulence { base_frequency_x: freq, base_frequency_y: freq, num_octaves: octaves, seed, stitch_tiles: false, fractal_noise: fractal },
    )
}

fn displace(result: &str, input: &str, map: &str, scale: f64) -> Primitive {
    prim(result, Kind::DisplacementMap { input: input.into(), in2: map.into(), scale, x_channel: Channel::R, y_channel: Channel::G })
}

fn morph(result: &str, input: &str, op: MorphologyOp, radius: f64) -> Primitive {
    prim(result, Kind::Morphology { input: input.into(), operator: op, radius_x: radius, radius_y: radius })
}

fn diffuse(result: &str, input: &str, surface: f64, constant: f64, color: &str, azimuth: f64, elevation: f64) -> Primitive {
    prim(
        result,
        Kind::DiffuseLighting {
            input: input.into(),
            surface_scale: surface,
            diffuse_constant: constant,
            color: color.into(),
            light: Light::Distant { azimuth, elevation },
        },
    )
}

fn specular(result: &str, input: &str, surface: f64, constant: f64, exponent: f64, color: &str, azimuth: f64, elevation: f64) -> Primitive {
    prim(
        result,
        Kind::SpecularLighting {
            input: input.into(),
            surface_scale: surface,
            specular_constant: constant,
            specular_exponent: exponent,
            color: color.into(),
            light: Light::Distant { azimuth, elevation },
        },
    )
}

fn convolve(result: &str, input: &str, kernel: [f64; 9], divisor: f64, bias: f64) -> Primitive {
    prim(
        result,
        Kind::ConvolveMatrix {
            input: input.into(),
            columns: 3,
            rows: 3,
            kernel: kernel.to_vec(),
            divisor,
            bias,
            target_x: None,
            target_y: None,
            edge_mode: EdgeMode::Duplicate,
            preserve_alpha: true,
        },
    )
}

/// A linear transfer `slope × c + intercept`.
fn lin(slope: f64, intercept: f64) -> Transfer {
    Transfer::Linear { slope, intercept }
}

fn table(values: &[f64]) -> Transfer {
    Transfer::Table { values: values.to_vec() }
}

fn gamma(amplitude: f64, exponent: f64, offset: f64) -> Transfer {
    Transfer::Gamma { amplitude, exponent, offset }
}

/// Lighting of the blurred alpha, mixed back into the source: the common bevel shape.
fn lit(name: &str, sd: f64, surface: f64, specular_k: f64, exponent: f64, color: &str, mode: BlendOp) -> FilterGraph {
    graph(
        name,
        vec![
            blur("soft", "SourceAlpha", sd),
            specular("light", "soft", surface, specular_k, exponent, color, 225.0, 45.0),
            composite("lightIn", "light", "SourceAlpha", CompositeOp::In),
            blend("", "lightIn", "SourceGraphic", mode),
        ],
    )
}

// ---------- Bevels ----------

fn bloom() -> FilterGraph {
    graph(
        "Bloom",
        vec![
            blur("soft", "SourceAlpha", 4.0),
            specular("light", "soft", 5.0, 1.0, 25.0, "#ffffff", 225.0, 45.0),
            composite("lightIn", "light", "SourceAlpha", CompositeOp::In),
            blend("lit", "lightIn", "SourceGraphic", BlendOp::Screen),
            blur("halo", "lit", 2.0),
            merge("", &["halo", "lit"]),
        ],
    )
}

fn button() -> FilterGraph {
    graph(
        "Button",
        vec![
            blur("soft", "SourceAlpha", 3.0),
            diffuse("shade", "soft", 6.0, 1.0, "#ffffff", 225.0, 55.0),
            composite("shadeIn", "shade", "SourceAlpha", CompositeOp::In),
            blend("shaded", "shadeIn", "SourceGraphic", BlendOp::Multiply),
            specular("gloss", "soft", 6.0, 0.8, 20.0, "#ffffff", 225.0, 55.0),
            composite("glossIn", "gloss", "SourceAlpha", CompositeOp::In),
            blend("", "glossIn", "shaded", BlendOp::Screen),
        ],
    )
}

fn matte_jelly() -> FilterGraph {
    lit("Matte Jelly", 6.0, 8.0, 0.6, 8.0, "#ffffff", BlendOp::Screen)
}

fn neon() -> FilterGraph {
    graph(
        "Neon",
        vec![
            morph("core", "SourceAlpha", MorphologyOp::Erode, 1.5),
            blur("softCore", "core", 1.5),
            specular("tube", "softCore", 4.0, 2.0, 12.0, "#ffffff", 225.0, 60.0),
            composite("tubeIn", "tube", "SourceAlpha", CompositeOp::In),
            blur("glow", "SourceGraphic", 5.0),
            merge("", &["glow", "SourceGraphic", "tubeIn"]),
        ],
    )
}

fn specular_light() -> FilterGraph {
    lit("Specular Light", 3.0, 5.0, 1.5, 30.0, "#ffffff", BlendOp::Screen)
}

fn translucent() -> FilterGraph {
    graph(
        "Translucent",
        vec![
            blur("soft", "SourceAlpha", 5.0),
            specular("light", "soft", 7.0, 0.9, 10.0, "#ffffff", 225.0, 50.0),
            composite("lightIn", "light", "SourceAlpha", CompositeOp::In),
            blend("lit", "lightIn", "SourceGraphic", BlendOp::Screen),
            arithmetic("", "lit", "SourceAlpha", [0.0, 0.75, 0.0, 0.0]),
        ],
    )
}

// ---------- Bumps ----------

fn bumps(name: &str, freq: f64, octaves: u32, surface: f64, azimuth: f64, mode: BlendOp) -> FilterGraph {
    graph(
        name,
        vec![
            turbulence("noise", freq, octaves, 1, true),
            diffuse("relief", "noise", surface, 1.0, "#ffffff", azimuth, 45.0),
            composite("reliefIn", "relief", "SourceAlpha", CompositeOp::In),
            blend("", "reliefIn", "SourceGraphic", mode),
        ],
    )
}

fn bump() -> FilterGraph {
    bumps("Bump", 0.04, 3, 4.0, 135.0, BlendOp::Multiply)
}

fn canvas_bumps() -> FilterGraph {
    bumps("Canvas Bumps", 0.3, 2, 2.0, 45.0, BlendOp::Multiply)
}

fn embossed_leather() -> FilterGraph {
    graph(
        "Embossed Leather",
        vec![
            turbulence("noise", 0.08, 4, 7, false),
            diffuse("relief", "noise", 6.0, 1.2, "#ffffff", 135.0, 40.0),
            composite("reliefIn", "relief", "SourceAlpha", CompositeOp::In),
            blend("", "reliefIn", "SourceGraphic", BlendOp::Multiply),
        ],
    )
}

fn paper_bump() -> FilterGraph {
    bumps("Paper Bump", 0.5, 1, 1.5, 225.0, BlendOp::Multiply)
}

fn rough_paper() -> FilterGraph {
    graph(
        "Rough Paper",
        vec![
            turbulence("noise", 0.4, 3, 3, true),
            diffuse("relief", "noise", 2.5, 1.0, "#ffffff", 225.0, 50.0),
            composite("reliefIn", "relief", "SourceAlpha", CompositeOp::In),
            blend("paper", "reliefIn", "SourceGraphic", BlendOp::Multiply),
            displace("", "paper", "noise", 1.5),
        ],
    )
}

// ---------- Color ----------

fn brilliance() -> FilterGraph {
    graph(
        "Brilliance",
        vec![
            matrix("", "SourceGraphic", MatrixMode::Saturate, &[1.6]),
            transfer("", "", lin(1.2, -0.05), lin(1.2, -0.05), lin(1.2, -0.05), Transfer::Identity),
        ],
    )
}

/// Deuteranopia simulation (a published linear approximation of the cone response).
fn color_blindness() -> FilterGraph {
    graph(
        "Color Blindness",
        vec![matrix(
            "",
            "SourceGraphic",
            MatrixMode::Matrix,
            &[0.625, 0.375, 0.0, 0.0, 0.0, 0.7, 0.3, 0.0, 0.0, 0.0, 0.0, 0.3, 0.7, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        )],
    )
}

fn duochrome() -> FilterGraph {
    graph(
        "Duochrome",
        vec![
            matrix("grey", "SourceGraphic", MatrixMode::Saturate, &[0.0]),
            transfer("", "grey", table(&[0.1, 1.0]), table(&[0.15, 0.9]), table(&[0.45, 0.3]), Transfer::Identity),
        ],
    )
}

fn extract_channel() -> FilterGraph {
    graph(
        "Extract Channel",
        vec![matrix(
            "",
            "SourceGraphic",
            MatrixMode::Matrix,
            &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        )],
    )
}

fn fade_to_black() -> FilterGraph {
    graph("Fade to Black", vec![transfer("", "SourceGraphic", lin(0.5, 0.0), lin(0.5, 0.0), lin(0.5, 0.0), Transfer::Identity)])
}

fn greyscale() -> FilterGraph {
    graph("Greyscale", vec![matrix("", "SourceGraphic", MatrixMode::Saturate, &[0.0])])
}

fn invert() -> FilterGraph {
    graph("Invert", vec![transfer("", "SourceGraphic", lin(-1.0, 1.0), lin(-1.0, 1.0), lin(-1.0, 1.0), Transfer::Identity)])
}

fn quadritone() -> FilterGraph {
    graph(
        "Quadritone",
        vec![
            matrix("grey", "SourceGraphic", MatrixMode::Saturate, &[0.0]),
            transfer("", "grey", table(&[0.05, 0.6, 0.95, 1.0]), table(&[0.0, 0.2, 0.8, 1.0]), table(&[0.3, 0.1, 0.4, 0.9]), Transfer::Identity),
        ],
    )
}

fn solarize() -> FilterGraph {
    graph(
        "Solarize",
        vec![transfer("", "SourceGraphic", table(&[0.0, 1.0, 0.0]), table(&[0.0, 1.0, 0.0]), table(&[0.0, 1.0, 0.0]), Transfer::Identity)],
    )
}

fn tritone() -> FilterGraph {
    graph(
        "Tritone",
        vec![
            matrix("grey", "SourceGraphic", MatrixMode::Saturate, &[0.0]),
            transfer("", "grey", table(&[0.1, 0.8, 1.0]), table(&[0.05, 0.4, 0.95]), table(&[0.3, 0.2, 0.8]), Transfer::Identity),
        ],
    )
}

// ---------- Distort ----------

fn chalk_and_sponge() -> FilterGraph {
    graph(
        "Chalk and Sponge",
        vec![
            turbulence("noise", 0.2, 3, 2, false),
            displace("moved", "SourceGraphic", "noise", 6.0),
            composite("", "moved", "SourceAlpha", CompositeOp::In),
        ],
    )
}

fn felt_feather() -> FilterGraph {
    graph("Felt Feather", vec![turbulence("noise", 0.1, 4, 5, false), blur("soft", "SourceGraphic", 2.0), displace("", "soft", "noise", 12.0)])
}

fn roughen() -> FilterGraph {
    graph("Roughen", vec![turbulence("noise", 0.05, 2, 0, true), displace("", "SourceGraphic", "noise", 8.0)])
}

fn swirl() -> FilterGraph {
    graph("Swirl", vec![turbulence("noise", 0.015, 1, 3, true), blur("smooth", "noise", 2.0), displace("", "SourceGraphic", "smooth", 40.0)])
}

fn torn_edges() -> FilterGraph {
    graph(
        "Torn Edges",
        vec![
            turbulence("noise", 0.06, 3, 9, false),
            displace("tornAlpha", "SourceAlpha", "noise", 10.0),
            morph("shrunk", "tornAlpha", MorphologyOp::Erode, 1.0),
            composite("", "SourceGraphic", "shrunk", CompositeOp::In),
        ],
    )
}

fn waves() -> FilterGraph {
    graph("Waves", vec![turbulence("noise", 0.01, 1, 1, true), blur("smooth", "noise", 3.0), displace("", "SourceGraphic", "smooth", 25.0)])
}

// ---------- Fill and Overlays ----------

fn noise_fill() -> FilterGraph {
    graph(
        "Noise Fill",
        vec![
            turbulence("noise", 0.3, 4, 4, false),
            matrix("grey", "noise", MatrixMode::Saturate, &[0.0]),
            composite("", "grey", "SourceAlpha", CompositeOp::In),
        ],
    )
}

fn silhouette() -> FilterGraph {
    graph("Silhouette", vec![flood("ink", "#000000", 1.0), composite("", "ink", "SourceAlpha", CompositeOp::In)])
}

fn hsl_noise() -> FilterGraph {
    graph(
        "HSL Noise",
        vec![
            turbulence("noise", 0.15, 3, 11, true),
            composite("noiseIn", "noise", "SourceAlpha", CompositeOp::In),
            blend("", "noiseIn", "SourceGraphic", BlendOp::Hue),
        ],
    )
}

fn spots_overlay() -> FilterGraph {
    graph(
        "Spots Overlay",
        vec![
            turbulence("noise", 0.08, 1, 6, true),
            matrix("grey", "noise", MatrixMode::Saturate, &[0.0]),
            transfer("spots", "grey", Transfer::Identity, Transfer::Identity, Transfer::Identity, Transfer::Discrete { values: vec![0.0, 0.0, 1.0] }),
            composite("spotsIn", "spots", "SourceAlpha", CompositeOp::In),
            blend("", "spotsIn", "SourceGraphic", BlendOp::Multiply),
        ],
    )
}

// ---------- Materials ----------

fn material(name: &str, freq: f64, octaves: u32, seed: i32, tint: [Transfer; 3], surface: f64) -> FilterGraph {
    graph(
        name,
        vec![
            turbulence("noise", freq, octaves, seed, true),
            matrix("grey", "noise", MatrixMode::Saturate, &[0.0]),
            transfer("tinted", "grey", tint[0].clone(), tint[1].clone(), tint[2].clone(), Transfer::Identity),
            specular("light", "noise", surface, 1.0, 20.0, "#ffffff", 225.0, 45.0),
            blend("lit", "light", "tinted", BlendOp::Screen),
            composite("", "lit", "SourceAlpha", CompositeOp::In),
        ],
    )
}

fn marble() -> FilterGraph {
    material("3D Marble", 0.02, 4, 13, [table(&[0.95, 0.75, 1.0]), table(&[0.95, 0.75, 1.0]), table(&[0.95, 0.8, 1.0])], 2.0)
}

fn wood() -> FilterGraph {
    graph(
        "3D Wood",
        vec![
            prim(
                "grain",
                Kind::Turbulence {
                    base_frequency_x: 0.01,
                    base_frequency_y: 0.25,
                    num_octaves: 3,
                    seed: 21,
                    stitch_tiles: true,
                    fractal_noise: true,
                },
            ),
            matrix("grey", "grain", MatrixMode::Saturate, &[0.0]),
            transfer("tinted", "grey", table(&[0.35, 0.75]), table(&[0.18, 0.5]), table(&[0.05, 0.25]), Transfer::Identity),
            diffuse("light", "grain", 2.0, 1.0, "#ffffff", 225.0, 60.0),
            blend("lit", "light", "tinted", BlendOp::Multiply),
            composite("", "lit", "SourceAlpha", CompositeOp::In),
        ],
    )
}

fn metallized_paint() -> FilterGraph {
    graph(
        "Metallized Paint",
        vec![
            turbulence("noise", 0.35, 3, 17, false),
            specular("light", "noise", 3.0, 1.2, 15.0, "#ffffff", 225.0, 45.0),
            composite("lightIn", "light", "SourceAlpha", CompositeOp::In),
            blend("", "lightIn", "SourceGraphic", BlendOp::Overlay),
        ],
    )
}

fn gold() -> FilterGraph {
    material("Gold", 0.05, 3, 31, [table(&[0.5, 1.0]), table(&[0.35, 0.85]), table(&[0.05, 0.3])], 4.0)
}

fn chrome() -> FilterGraph {
    material("Chrome", 0.03, 2, 41, [table(&[0.2, 0.9, 0.4, 1.0]), table(&[0.2, 0.9, 0.4, 1.0]), table(&[0.25, 0.95, 0.45, 1.0])], 5.0)
}

// ---------- Morphology ----------

fn outline() -> FilterGraph {
    graph(
        "Outline",
        vec![
            morph("fat", "SourceAlpha", MorphologyOp::Dilate, 2.0),
            morph("thin", "SourceAlpha", MorphologyOp::Erode, 1.0),
            composite("ring", "fat", "thin", CompositeOp::Out),
            flood("ink", "#000000", 1.0),
            composite("", "ink", "ring", CompositeOp::In),
        ],
    )
}

fn ink_blot() -> FilterGraph {
    graph(
        "Ink Blot",
        vec![
            blur("soft", "SourceAlpha", 4.0),
            transfer("hard", "soft", Transfer::Identity, Transfer::Identity, Transfer::Identity, table(&[0.0, 0.0, 1.0, 1.0])),
            turbulence("noise", 0.1, 2, 5, false),
            displace("blot", "hard", "noise", 6.0),
            flood("ink", "#000000", 1.0),
            composite("", "ink", "blot", CompositeOp::In),
        ],
    )
}

fn smooth_edges() -> FilterGraph {
    graph(
        "Smooth Edges",
        vec![
            blur("soft", "SourceGraphic", 3.0),
            transfer("", "soft", Transfer::Identity, Transfer::Identity, Transfer::Identity, table(&[0.0, 0.0, 1.0, 1.0])),
        ],
    )
}

fn black_hole() -> FilterGraph {
    graph(
        "Black Hole",
        vec![
            morph("core", "SourceAlpha", MorphologyOp::Erode, 4.0),
            blur("soft", "core", 3.0),
            flood("ink", "#000000", 1.0),
            composite("hole", "ink", "soft", CompositeOp::In),
            merge("", &["SourceGraphic", "hole"]),
        ],
    )
}

// ---------- Non-realistic 3D shaders ----------

fn aluminium() -> FilterGraph {
    graph(
        "Aluminium",
        vec![
            blur("soft", "SourceAlpha", 3.0),
            specular("light", "soft", 4.0, 1.0, 8.0, "#ffffff", 225.0, 45.0),
            matrix("grey", "SourceGraphic", MatrixMode::Saturate, &[0.1]),
            blend("lit", "light", "grey", BlendOp::Screen),
            composite("", "lit", "SourceAlpha", CompositeOp::In),
        ],
    )
}

fn comics_cream() -> FilterGraph {
    graph(
        "Comics Cream",
        vec![
            blur("soft", "SourceAlpha", 4.0),
            diffuse("shade", "soft", 6.0, 1.0, "#ffffff", 225.0, 45.0),
            transfer(
                "bands",
                "shade",
                Transfer::Discrete { values: vec![0.4, 0.7, 1.0] },
                Transfer::Discrete { values: vec![0.4, 0.7, 1.0] },
                Transfer::Discrete { values: vec![0.4, 0.7, 1.0] },
                Transfer::Identity,
            ),
            blend("lit", "bands", "SourceGraphic", BlendOp::Multiply),
            composite("", "lit", "SourceAlpha", CompositeOp::In),
        ],
    )
}

fn dark_and_glow() -> FilterGraph {
    graph(
        "Dark and Glow",
        vec![
            transfer("dark", "SourceGraphic", lin(0.4, 0.0), lin(0.4, 0.0), lin(0.4, 0.0), Transfer::Identity),
            blur("glow", "SourceGraphic", 6.0),
            blend("lit", "glow", "dark", BlendOp::Screen),
            composite("", "lit", "SourceAlpha", CompositeOp::In),
        ],
    )
}

// ---------- Protrusions ----------

fn spikes() -> FilterGraph {
    graph(
        "Spikes",
        vec![
            turbulence("noise", 0.15, 1, 8, false),
            displace("moved", "SourceGraphic", "noise", 20.0),
            morph("", "moved", MorphologyOp::Dilate, 0.5),
        ],
    )
}

fn blobs() -> FilterGraph {
    graph(
        "Blobs",
        vec![
            turbulence("noise", 0.04, 2, 12, true),
            displace("moved", "SourceAlpha", "noise", 30.0),
            morph("fat", "moved", MorphologyOp::Dilate, 3.0),
            blur("soft", "fat", 3.0),
            transfer("hard", "soft", Transfer::Identity, Transfer::Identity, Transfer::Identity, table(&[0.0, 0.0, 1.0, 1.0])),
            flood("paint", "#000000", 1.0),
            composite("blob", "paint", "hard", CompositeOp::In),
            blend("", "SourceGraphic", "blob", BlendOp::Normal),
        ],
    )
}

// ---------- Ridges ----------

fn ridge(name: &str, erode: f64, sd: f64, exponent: f64, mode: BlendOp) -> FilterGraph {
    graph(
        name,
        vec![
            morph("inner", "SourceAlpha", MorphologyOp::Erode, erode),
            composite("ring", "SourceAlpha", "inner", CompositeOp::Out),
            blur("soft", "ring", sd),
            specular("light", "soft", 5.0, 1.0, exponent, "#ffffff", 225.0, 45.0),
            composite("lightIn", "light", "SourceAlpha", CompositeOp::In),
            blend("", "lightIn", "SourceGraphic", mode),
        ],
    )
}

fn dragee() -> FilterGraph {
    ridge("Dragee", 4.0, 3.0, 12.0, BlendOp::Screen)
}

fn glowing_metal() -> FilterGraph {
    ridge("Glowing Metal", 3.0, 2.0, 30.0, BlendOp::Overlay)
}

fn thin_membrane() -> FilterGraph {
    graph(
        "Thin Membrane",
        vec![
            morph("inner", "SourceAlpha", MorphologyOp::Erode, 2.0),
            composite("ring", "SourceAlpha", "inner", CompositeOp::Out),
            blur("soft", "ring", 1.5),
            specular("light", "soft", 3.0, 1.0, 15.0, "#ffffff", 225.0, 45.0),
            arithmetic("thin", "SourceGraphic", "SourceAlpha", [0.0, 0.5, 0.0, 0.0]),
            merge("", &["thin", "light"]),
        ],
    )
}

// ---------- Scatter ----------

fn air_spray() -> FilterGraph {
    graph(
        "Air Spray",
        vec![
            turbulence("noise", 0.6, 1, 2, false),
            transfer(
                "dots",
                "noise",
                Transfer::Identity,
                Transfer::Identity,
                Transfer::Identity,
                Transfer::Discrete { values: vec![0.0, 0.0, 0.0, 1.0] },
            ),
            composite("dotsIn", "SourceGraphic", "dots", CompositeOp::In),
            blur("soft", "SourceGraphic", 4.0),
            merge("", &["soft", "dotsIn"]),
        ],
    )
}

fn cubes() -> FilterGraph {
    graph(
        "Cubes",
        vec![
            turbulence("noise", 0.07, 1, 4, false),
            transfer(
                "steps",
                "noise",
                Transfer::Discrete { values: vec![0.0, 0.25, 0.5, 0.75, 1.0] },
                Transfer::Discrete { values: vec![0.0, 0.25, 0.5, 0.75, 1.0] },
                Transfer::Identity,
                Transfer::Identity,
            ),
            displace("moved", "SourceGraphic", "steps", 14.0),
            diffuse("faces", "steps", 3.0, 1.0, "#ffffff", 225.0, 45.0),
            blend("lit", "faces", "moved", BlendOp::Multiply),
            composite("", "lit", "moved", CompositeOp::In),
        ],
    )
}

fn leopard_fur() -> FilterGraph {
    graph(
        "Leopard Fur",
        vec![
            turbulence("noise", 0.06, 2, 19, false),
            matrix("grey", "noise", MatrixMode::Saturate, &[0.0]),
            transfer("spots", "grey", table(&[0.95, 0.75, 0.1]), table(&[0.75, 0.5, 0.05]), table(&[0.3, 0.1, 0.0]), Transfer::Identity),
            transfer(
                "hard",
                "spots",
                Transfer::Discrete { values: vec![0.1, 0.6, 0.95] },
                Transfer::Discrete { values: vec![0.05, 0.4, 0.75] },
                Transfer::Discrete { values: vec![0.0, 0.1, 0.3] },
                Transfer::Identity,
            ),
            composite("", "hard", "SourceAlpha", CompositeOp::In),
        ],
    )
}

fn pointillism() -> FilterGraph {
    graph(
        "Pointillism",
        vec![
            turbulence("noise", 0.5, 1, 7, false),
            transfer("dots", "noise", Transfer::Identity, Transfer::Identity, Transfer::Identity, Transfer::Discrete { values: vec![0.0, 1.0] }),
            composite("", "SourceGraphic", "dots", CompositeOp::In),
        ],
    )
}

fn stone_wall() -> FilterGraph {
    graph(
        "Stone Wall",
        vec![
            turbulence("noise", 0.05, 3, 23, false),
            transfer(
                "stones",
                "noise",
                Transfer::Discrete { values: vec![0.0, 0.5, 1.0] },
                Transfer::Discrete { values: vec![0.0, 0.5, 1.0] },
                Transfer::Identity,
                Transfer::Identity,
            ),
            diffuse("relief", "stones", 4.0, 1.0, "#ffffff", 135.0, 45.0),
            composite("reliefIn", "relief", "SourceAlpha", CompositeOp::In),
            blend("", "reliefIn", "SourceGraphic", BlendOp::Multiply),
        ],
    )
}

// ---------- Shadows and Glows ----------

fn drop_shadow() -> FilterGraph {
    graph(
        "Drop Shadow",
        vec![
            blur("blur", "SourceAlpha", 3.0),
            offset("shadow", "blur", 4.0, 4.0),
            flood("flood", "#000000", 0.6),
            composite("paint", "flood", "shadow", CompositeOp::In),
            merge("", &["paint", "SourceGraphic"]),
        ],
    )
}

fn inner_shadow() -> FilterGraph {
    graph(
        "Inner Shadow",
        vec![
            offset("moved", "SourceAlpha", 3.0, 3.0),
            blur("soft", "moved", 3.0),
            composite("inside", "SourceAlpha", "soft", CompositeOp::Out),
            flood("ink", "#000000", 0.7),
            composite("shadow", "ink", "inside", CompositeOp::In),
            merge("", &["SourceGraphic", "shadow"]),
        ],
    )
}

fn glow() -> FilterGraph {
    graph(
        "Glow",
        vec![
            morph("fat", "SourceAlpha", MorphologyOp::Dilate, 2.0),
            blur("soft", "fat", 6.0),
            flood("light", "#ffd040", 0.9),
            composite("halo", "light", "soft", CompositeOp::In),
            merge("", &["halo", "SourceGraphic"]),
        ],
    )
}

// ---------- Textures ----------

fn cracked_glass() -> FilterGraph {
    graph(
        "Cracked Glass",
        vec![
            turbulence("noise", 0.08, 2, 29, false),
            convolve("edges", "noise", [-1.0, -1.0, -1.0, -1.0, 8.0, -1.0, -1.0, -1.0, -1.0], 1.0, 0.0),
            matrix("grey", "edges", MatrixMode::Saturate, &[0.0]),
            composite("cracksIn", "grey", "SourceAlpha", CompositeOp::In),
            blend("", "cracksIn", "SourceGraphic", BlendOp::Screen),
        ],
    )
}

fn ink_splash() -> FilterGraph {
    graph(
        "Ink Splash",
        vec![
            turbulence("noise", 0.12, 3, 37, false),
            displace("splash", "SourceAlpha", "noise", 18.0),
            blur("soft", "splash", 1.0),
            transfer("hard", "soft", Transfer::Identity, Transfer::Identity, Transfer::Identity, table(&[0.0, 0.0, 1.0, 1.0])),
            composite("", "SourceGraphic", "hard", CompositeOp::Atop),
        ],
    )
}

fn rough_transparency() -> FilterGraph {
    graph(
        "Rough Transparency",
        vec![
            turbulence("noise", 0.1, 3, 43, false),
            matrix("alphaMap", "noise", MatrixMode::LuminanceToAlpha, &[]),
            composite("", "SourceGraphic", "alphaMap", CompositeOp::In),
        ],
    )
}

fn watercolor() -> FilterGraph {
    graph(
        "Watercolor",
        vec![
            turbulence("noise", 0.05, 3, 47, true),
            displace("moved", "SourceGraphic", "noise", 10.0),
            blur("soft", "moved", 1.5),
            turbulence("grain", 0.4, 2, 48, false),
            matrix("grainGrey", "grain", MatrixMode::Saturate, &[0.0]),
            composite("grainIn", "grainGrey", "soft", CompositeOp::In),
            blend("", "grainIn", "soft", BlendOp::Multiply),
        ],
    )
}

// ---------- Transparency utilities ----------

fn channel_to_alpha() -> FilterGraph {
    graph(
        "Channel to Alpha",
        vec![matrix(
            "",
            "SourceGraphic",
            MatrixMode::Matrix,
            &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        )],
    )
}

fn light_eraser() -> FilterGraph {
    graph(
        "Light Eraser",
        vec![
            matrix("lum", "SourceGraphic", MatrixMode::LuminanceToAlpha, &[]),
            transfer("dark", "lum", Transfer::Identity, Transfer::Identity, Transfer::Identity, lin(-1.0, 1.0)),
            composite("", "SourceGraphic", "dark", CompositeOp::In),
        ],
    )
}

fn luminance_to_alpha() -> FilterGraph {
    graph(
        "Luminance to Alpha",
        vec![matrix("lum", "SourceGraphic", MatrixMode::LuminanceToAlpha, &[]), composite("", "SourceGraphic", "lum", CompositeOp::In)],
    )
}

fn saturation_map() -> FilterGraph {
    graph(
        "Saturation Map",
        vec![
            matrix("grey", "SourceGraphic", MatrixMode::Saturate, &[0.0]),
            arithmetic("diff", "SourceGraphic", "grey", [0.0, 1.0, -1.0, 0.0]),
            transfer("boost", "diff", gamma(3.0, 0.5, 0.0), gamma(3.0, 0.5, 0.0), gamma(3.0, 0.5, 0.0), Transfer::Identity),
            matrix("alphaMap", "boost", MatrixMode::LuminanceToAlpha, &[]),
            composite("", "SourceGraphic", "alphaMap", CompositeOp::In),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_unique_and_build() {
        let mut ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len(), "duplicate preset ids");
        for p in PRESETS {
            assert!(CATEGORIES.contains(&p.category), "{}: unknown category {}", p.id, p.category);
            let g = (p.build)();
            assert!(!g.primitives.is_empty(), "{}: empty graph", p.id);
            // Every named input is a standard input or an earlier result.
            let mut seen: Vec<&str> = vec![];
            for prim in &g.primitives {
                for i in prim.kind.inputs() {
                    assert!(
                        i.is_empty() || crate::svgfilter::STANDARD_INPUTS.contains(&i.as_str()) || seen.contains(&i.as_str()),
                        "{}: input {i} is not an earlier result",
                        p.id
                    );
                }
                seen.push(prim.result.as_str());
            }
        }
        assert!(search("glow").iter().any(|p| p.id == "shadows.glow"));
    }
}
