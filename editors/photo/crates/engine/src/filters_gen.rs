//! The third filter batch: Map (Fractal Trace, Illusion, Little Planet, Recursive Transform,
//! Tile Seamless, Map Object, Small Tiles), Render (fractals, noise and pattern generators,
//! Circuit, Lava, Line Nova, Sphere Designer, Spirograph), Decor (Slide, Coffee Stain, Stencil
//! Carve), Web (Semi-Flatten) and the Animation commands, which render N frames as layers.
//!
//! The pixel filters run through [`crate::filters::run_filter`]; this module supplies their
//! parameter decoding and command specs. The animation commands build layers themselves.

use redrob_photo_algo::{
    self as algo, AnimEdge, CmlFunction, ExplorerFractal, FilterParams, FractalAbyss, FractalKind, IfsPreset, IllusionType, JigsawStyle, LavaGradient,
    MapShape, MazeAlgorithm, QuarterTurn, SinusBlend, SphereTexture, SpinDirection, SpiralType, SpyroCurve, SpyroShape, TileFlip, TileSelection,
};
use redrob_photo_doc::{Layer, LayerContent, LayerId};
use redrob_photo_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn f(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn i(p: &Value, k: &str, d: i64) -> i64 {
    p.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64))).unwrap_or(d)
}
fn u(p: &Value, k: &str, d: u32, lo: u32, hi: u32) -> u32 {
    i(p, k, i64::from(d)).clamp(i64::from(lo), i64::from(hi)) as u32
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn s<'a>(p: &'a Value, k: &str, d: &'a str) -> &'a str {
    p.get(k).and_then(Value::as_str).unwrap_or(d)
}
fn seed(p: &Value, k: &str, d: u32) -> u32 {
    p.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or_else(|| i(p, k, i64::from(d)) as u32)
}
/// Straight RGBA colour from `[r, g, b(, a)]` (0–1) or `#rrggbb`.
fn colour(p: &Value, k: &str, d: [f32; 4]) -> [f32; 4] {
    match p.get(k) {
        Some(Value::Array(a)) if a.len() >= 3 => {
            let c = |j: usize, dv: f32| a.get(j).and_then(Value::as_f64).map_or(dv, |v| v as f32);
            [c(0, 0.0), c(1, 0.0), c(2, 0.0), c(3, 1.0)]
        }
        Some(Value::String(h)) if h.len() == 7 && h.starts_with('#') => {
            let ch = |j: usize| h.get(j..j + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).map_or(0.0, |v| v as f32 / 255.0);
            [ch(1), ch(3), ch(5), 1.0]
        }
        _ => d,
    }
}

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

fn cml(p: &Value, k: &str, d: CmlFunction) -> CmlFunction {
    match p.get(k).and_then(Value::as_str) {
        Some("keepImage") => CmlFunction::KeepImage,
        Some("keepFirstValue") => CmlFunction::KeepFirstValue,
        Some("fillWithK") => CmlFunction::FillWithK,
        Some("logistic") => CmlFunction::Logistic,
        Some("deltaSinCurve") => CmlFunction::DeltaSinCurve,
        Some("sinCurve") => CmlFunction::SinCurve,
        Some("sinCurveStep") => CmlFunction::SinCurveStep,
        Some("random") => CmlFunction::Random,
        _ => d,
    }
}

/// Algorithm parameters for the ids this module owns (dialog units).
pub fn params_for(id: &str, p: &Value) -> Option<FilterParams> {
    Some(match id {
        // ---- Map ----
        "filter.map.fractalTrace" => FilterParams::FractalTrace {
            kind: if s(p, "fractal", "mandelbrot") == "julia" { FractalKind::Julia } else { FractalKind::Mandelbrot },
            x1: f(p, "x1", -1.0).clamp(-50.0, 50.0),
            x2: f(p, "x2", 0.5).clamp(-50.0, 50.0),
            y1: f(p, "y1", -1.0).clamp(-50.0, 50.0),
            y2: f(p, "y2", 1.0).clamp(-50.0, 50.0),
            jx: f(p, "jx", 0.5).clamp(-50.0, 50.0),
            jy: f(p, "jy", 0.5).clamp(-50.0, 50.0),
            depth: u(p, "depth", 3, 1, 64),
            bailout: f(p, "bailout", 10000.0).clamp(1.0, 100_000.0),
            abyss: match s(p, "abyss", "loop") {
                "clamp" => FractalAbyss::Clamp,
                "black" => FractalAbyss::Black,
                "white" => FractalAbyss::White,
                "transparent" => FractalAbyss::Transparent,
                _ => FractalAbyss::Loop,
            },
        },
        "filter.map.illusion" => FilterParams::Illusion {
            division: u(p, "division", 8, 1, 64),
            kind: if s(p, "type", "type1") == "type2" { IllusionType::Type2 } else { IllusionType::Type1 },
        },
        "filter.map.littlePlanet" => FilterParams::LittlePlanet {
            pan: f(p, "pan", 0.0).clamp(-180.0, 180.0),
            tilt: f(p, "tilt", 90.0).clamp(-180.0, 180.0),
            spin: f(p, "spin", 0.0).clamp(-360.0, 360.0),
            zoom: f(p, "zoom", 100.0).clamp(1.0, 1000.0),
            inverse: b(p, "inverse", false),
        },
        "filter.map.recursiveTransform" => {
            let fade = colour(p, "fadeColor", BLACK);
            FilterParams::RecursiveTransform {
                scale: f(p, "scale", 80.0).clamp(1.0, 1000.0),
                angle: f(p, "angle", 10.0).clamp(-360.0, 360.0),
                offset_x: f(p, "offsetX", 0.0).clamp(-1.0, 1.0),
                offset_y: f(p, "offsetY", 0.0).clamp(-1.0, 1.0),
                first: u(p, "firstIteration", 0, 0, 32),
                last: u(p, "lastIteration", 3, 1, 32),
                fade: [fade[0], fade[1], fade[2], f(p, "fadeOpacity", 0.0).clamp(0.0, 100.0) / 100.0],
                paste_below: b(p, "pasteBelow", false),
            }
        }
        "filter.map.tileSeamless" => FilterParams::TileSeamless,
        "filter.map.mapObject" => FilterParams::MapObject {
            shape: match s(p, "mapTo", "plane") {
                "sphere" => MapShape::Sphere,
                "box" => MapShape::Box,
                "cylinder" => MapShape::Cylinder,
                _ => MapShape::Plane,
            },
            rot_x: f(p, "rotationX", 0.0).clamp(-180.0, 180.0),
            rot_y: f(p, "rotationY", 0.0).clamp(-180.0, 180.0),
            rot_z: f(p, "rotationZ", 0.0).clamp(-180.0, 180.0),
            light_x: f(p, "lightX", -0.5).clamp(-5.0, 5.0),
            light_y: f(p, "lightY", -0.5).clamp(-5.0, 5.0),
            light_z: f(p, "lightZ", 2.0).clamp(-5.0, 5.0),
            ambient: f(p, "ambient", 0.3).clamp(0.0, 1.0),
            diffuse: f(p, "diffuse", 1.0).clamp(0.0, 1.0),
            specular: f(p, "specular", 0.5).clamp(0.0, 1.0),
            highlight: f(p, "highlight", 27.0).clamp(1.0, 100.0),
            radius: f(p, "radius", 0.25).clamp(0.05, 2.0),
            transparent_background: b(p, "transparentBackground", true),
        },
        "filter.map.smallTiles" => FilterParams::SmallTiles {
            segments: u(p, "segments", 2, 1, 32),
            flip: match s(p, "flip", "none") {
                "horizontal" => TileFlip::Horizontal,
                "vertical" => TileFlip::Vertical,
                "both" => TileFlip::Both,
                _ => TileFlip::None,
            },
            which: match s(p, "flipWhich", "all") {
                "alternate" => TileSelection::Alternate,
                "explicit" => TileSelection::Explicit,
                _ => TileSelection::All,
            },
            opacity: f(p, "opacity", 100.0).clamp(0.0, 100.0),
        },
        // ---- Render: fractals ----
        "filter.render.fractalExplorer" => FilterParams::FractalExplorer {
            kind: match s(p, "fractalType", "mandelbrot") {
                "julia" => ExplorerFractal::Julia,
                "barnsley1" => ExplorerFractal::Barnsley1,
                "barnsley2" => ExplorerFractal::Barnsley2,
                "barnsley3" => ExplorerFractal::Barnsley3,
                "spider" => ExplorerFractal::Spider,
                "manOWar" => ExplorerFractal::ManOWar,
                "lambda" => ExplorerFractal::Lambda,
                "sierpinski" => ExplorerFractal::Sierpinski,
                _ => ExplorerFractal::Mandelbrot,
            },
            x1: f(p, "x1", -2.0).clamp(-3.0, 3.0),
            x2: f(p, "x2", 2.0).clamp(-3.0, 3.0),
            y1: f(p, "y1", -1.5).clamp(-3.0, 3.0),
            y2: f(p, "y2", 1.5).clamp(-3.0, 3.0),
            iterations: u(p, "iterations", 50, 1, 1000),
            cx: f(p, "cx", -0.75).clamp(-2.5, 2.5),
            cy: f(p, "cy", 0.2).clamp(-2.5, 2.5),
            colors: u(p, "colors", 256, 2, 8192),
            red: f(p, "red", 1.0).clamp(0.0, 1.0),
            green: f(p, "green", 1.0).clamp(0.0, 1.0),
            blue: f(p, "blue", 1.0).clamp(0.0, 1.0),
            invert_red: b(p, "redInversion", false),
            invert_green: b(p, "greenInversion", false),
            invert_blue: b(p, "blueInversion", false),
        },
        "filter.render.ifsFractal" => FilterParams::IfsFractal {
            preset: match s(p, "preset", "fern") {
                "sierpinski" => IfsPreset::Sierpinski,
                "dragon" => IfsPreset::Dragon,
                "spiral" => IfsPreset::Spiral,
                "tree" => IfsPreset::Tree,
                "snowflake" => IfsPreset::Snowflake,
                _ => IfsPreset::Fern,
            },
            iterations: u(p, "iterations", 50, 1, 2000),
            spot_radius: f(p, "spotRadius", 0.75).clamp(0.3, 20.0),
            x: f(p, "x", 0.0).clamp(-1.0, 1.0),
            y: f(p, "y", 0.0).clamp(-1.0, 1.0),
            scale: f(p, "scale", 1.0).clamp(0.1, 5.0),
            angle: f(p, "angle", 0.0).clamp(-180.0, 180.0),
            color: colour(p, "color", [0.165, 0.49, 0.18, 1.0]),
            seed: seed(p, "seed", 0),
        },
        "filter.render.sphereDesigner" => FilterParams::SphereDesigner {
            texture: match s(p, "texture", "solid") {
                "checker" => SphereTexture::Checker,
                "marble" => SphereTexture::Marble,
                "lizard" => SphereTexture::Lizard,
                "phong" => SphereTexture::Phong,
                "noise" => SphereTexture::Noise,
                "wood" => SphereTexture::Wood,
                "spiral" => SphereTexture::Spiral,
                "spots" => SphereTexture::Spots,
                _ => SphereTexture::Solid,
            },
            color1: colour(p, "color1", [0.8, 0.2, 0.2, 1.0]),
            color2: colour(p, "color2", WHITE),
            scale: f(p, "scale", 1.0).clamp(0.05, 10.0),
            turbulence: f(p, "turbulence", 1.0).clamp(0.0, 10.0),
            amount: f(p, "amount", 1.0).clamp(0.0, 1.0),
            exponent: f(p, "exponent", 20.0).clamp(1.0, 100.0),
            light_x: f(p, "lightX", -1.0).clamp(-5.0, 5.0),
            light_y: f(p, "lightY", 1.0).clamp(-5.0, 5.0),
            light_z: f(p, "lightZ", 1.0).clamp(-5.0, 5.0),
            ambient: f(p, "ambient", 0.2).clamp(0.0, 1.0),
            bump: f(p, "bump", 0.0).clamp(0.0, 1.0),
            radius: f(p, "radius", 0.8).clamp(0.05, 2.0),
            transparent_background: b(p, "transparentBackground", false),
        },
        // ---- Render: noise ----
        "filter.render.cellNoise" => FilterParams::CellNoise {
            scale: f(p, "scale", 100.0).clamp(1.0, 1000.0),
            shape: f(p, "shape", 2.0).clamp(1.0, 2.0),
            rank: u(p, "rank", 1, 1, 3),
            iterations: u(p, "iterations", 1, 1, 8),
            palettize: b(p, "palettize", false),
            seed: seed(p, "seed", 0),
        },
        "filter.render.simplexNoise" => {
            FilterParams::SimplexNoise { scale: f(p, "scale", 100.0).clamp(1.0, 1000.0), iterations: u(p, "iterations", 1, 1, 8), seed: seed(p, "seed", 0) }
        }
        "filter.render.lava" => FilterParams::Lava {
            seed: seed(p, "seed", 10),
            size: f(p, "size", 10.0).clamp(2.0, 100.0),
            roughness: f(p, "roughness", 7.0).clamp(1.0, 10.0),
            gradient: match s(p, "gradient", "incandescent") {
                "germanFlag" => LavaGradient::GermanFlag,
                "caribbeanBlues" => LavaGradient::CaribbeanBlues,
                "golden" => LavaGradient::Golden,
                "fullSpectrum" => LavaGradient::FullSpectrum,
                _ => LavaGradient::Incandescent,
            },
        },
        "filter.render.circuit" => FilterParams::Circuit {
            mask_size: f(p, "oilifyMaskSize", 17.0).clamp(3.0, 50.0),
            seed: seed(p, "circuitSeed", 3),
            no_background: b(p, "noBackground", false),
        },
        "filter.render.qbist" => {
            FilterParams::Qbist { seed: seed(p, "seed", 0), antialias: b(p, "antialiasing", true), oversample: u(p, "oversample", 2, 1, 4) }
        }
        "filter.render.cmlExplorer" => FilterParams::CmlExplorer {
            hue: cml(p, "hueFunction", CmlFunction::Logistic),
            saturation: cml(p, "saturationFunction", CmlFunction::FillWithK),
            value: cml(p, "valueFunction", CmlFunction::Logistic),
            k: f(p, "parameterK", 3.6).clamp(0.0, 10.0),
            coupling: f(p, "coupling", 0.5).clamp(0.0, 1.0),
            zoom: f(p, "zoom", 1.0).clamp(1.0, 64.0),
            start_offset: u(p, "startOffset", 0, 0, 100),
            seed: seed(p, "seed", 0),
        },
        // ---- Render: pattern ----
        "filter.render.bayerMatrix" => FilterParams::BayerMatrix {
            subdivisions: u(p, "subdivisions", 1, 0, 15),
            x_scale: u(p, "xScale", 1, 1, 100),
            y_scale: u(p, "yScale", 1, 1, 100),
            rotation: match s(p, "rotation", "none") {
                "quarter" => QuarterTurn::Quarter,
                "half" => QuarterTurn::Half,
                "threeQuarters" => QuarterTurn::ThreeQuarters,
                _ => QuarterTurn::None,
            },
            reflect: b(p, "reflect", false),
            amplitude: f(p, "amplitude", 1.0).clamp(-1000.0, 1000.0),
            offset: f(p, "offset", 0.0).clamp(-2.0, 2.0),
            exponent: f(p, "exponent", 1.0).clamp(0.01, 10.0),
            x_offset: i(p, "xOffset", 0).clamp(-100_000, 100_000) as i32,
            y_offset: i(p, "yOffset", 0).clamp(-100_000, 100_000) as i32,
        },
        "filter.render.checkerboard" => FilterParams::Checkerboard {
            width: f(p, "width", 16.0).clamp(1.0, 512.0),
            height: f(p, "height", 16.0).clamp(1.0, 512.0),
            x_offset: f(p, "xOffset", 0.0),
            y_offset: f(p, "yOffset", 0.0),
            color1: colour(p, "color1", BLACK),
            color2: colour(p, "color2", WHITE),
        },
        "filter.render.diffractionPatterns" => FilterParams::DiffractionPatterns {
            red_frequency: f(p, "redFrequency", 0.815).clamp(0.0, 20.0),
            green_frequency: f(p, "greenFrequency", 1.221).clamp(0.0, 20.0),
            blue_frequency: f(p, "blueFrequency", 1.123).clamp(0.0, 20.0),
            red_contours: f(p, "redContours", 0.821).clamp(0.0, 10.0),
            green_contours: f(p, "greenContours", 0.821).clamp(0.0, 10.0),
            blue_contours: f(p, "blueContours", 0.974).clamp(0.0, 10.0),
            red_edges: f(p, "redSharpEdges", 0.61).clamp(0.0, 1.0),
            green_edges: f(p, "greenSharpEdges", 0.677).clamp(0.0, 1.0),
            blue_edges: f(p, "blueSharpEdges", 0.636).clamp(0.0, 1.0),
            brightness: f(p, "brightness", 1.0).clamp(0.0, 2.0),
            scattering: f(p, "scattering", 37.0).clamp(0.0, 100.0),
            polarization: f(p, "polarization", -85.0).clamp(-180.0, 180.0),
        },
        "filter.render.grid" => FilterParams::Grid {
            x_spacing: f(p, "xSpacing", 32.0).clamp(1.0, 512.0),
            y_spacing: f(p, "ySpacing", 32.0).clamp(1.0, 512.0),
            x_offset: f(p, "xOffset", 0.0),
            y_offset: f(p, "yOffset", 0.0),
            line_width: f(p, "lineWidth", 4.0).clamp(0.0, 64.0),
            line_height: f(p, "lineHeight", 4.0).clamp(0.0, 64.0),
            color: colour(p, "color", BLACK),
        },
        "filter.render.linearSinusoid" => FilterParams::LinearSinusoid {
            x_period: f(p, "xPeriod", 128.0).clamp(0.1, 1000.0),
            y_period: f(p, "yPeriod", 128.0).clamp(0.1, 1000.0),
            x_amplitude: f(p, "xAmplitude", 1.0).clamp(0.0, 10.0),
            y_amplitude: f(p, "yAmplitude", 1.0).clamp(0.0, 10.0),
            x_phase: f(p, "xPhase", 0.0).clamp(-360.0, 360.0),
            y_phase: f(p, "yPhase", 0.0).clamp(-360.0, 360.0),
            angle: f(p, "angle", 90.0).clamp(-180.0, 180.0),
            offset: f(p, "offset", 0.0).clamp(-1.0, 1.0),
            exponent: f(p, "exponent", 1.0).clamp(0.01, 10.0),
            x_offset: f(p, "xOffset", 0.0),
            y_offset: f(p, "yOffset", 0.0),
            rotation: f(p, "rotation", 0.0).clamp(-180.0, 180.0),
            supersampling: u(p, "supersampling", 1, 1, 8),
        },
        "filter.render.maze" => FilterParams::Maze {
            cell_width: f(p, "cellWidth", 16.0).clamp(1.0, 64.0),
            cell_height: f(p, "cellHeight", 16.0).clamp(1.0, 64.0),
            algorithm: if s(p, "algorithm", "depthFirst") == "prim" { MazeAlgorithm::Prim } else { MazeAlgorithm::DepthFirst },
            tileable: b(p, "tileable", false),
            seed: seed(p, "seed", 0),
            foreground: colour(p, "foreground", BLACK),
            background: colour(p, "background", WHITE),
        },
        "filter.render.sinus" => FilterParams::Sinus {
            x_scale: f(p, "xScale", 15.0).clamp(0.1, 100.0),
            y_scale: f(p, "yScale", 15.0).clamp(0.1, 100.0),
            complexity: f(p, "complexity", 1.0).clamp(0.0, 15.0),
            seed: seed(p, "seed", 42),
            tileable: b(p, "tileable", false),
            perturbation: b(p, "perturbation", true),
            blend: match s(p, "blendMode", "sinusoidal") {
                "linear" => SinusBlend::Linear,
                "bilinear" => SinusBlend::Bilinear,
                _ => SinusBlend::Sinusoidal,
            },
            exponent: f(p, "exponent", 1.0).clamp(0.01, 10.0),
            color1: colour(p, "color1", [1.0, 1.0, 0.0, 1.0]),
            color2: colour(p, "color2", [0.0, 0.0, 1.0, 1.0]),
        },
        "filter.render.spiral" => FilterParams::Spiral {
            kind: if s(p, "type", "linear") == "logarithmic" { SpiralType::Logarithmic } else { SpiralType::Linear },
            x: f(p, "x", 0.5).clamp(0.0, 1.0),
            y: f(p, "y", 0.5).clamp(0.0, 1.0),
            radius: f(p, "radius", 100.0).clamp(1.0, 1000.0),
            base: f(p, "base", 2.0).clamp(1.01, 20.0),
            balance: f(p, "balance", 0.0).clamp(-1.0, 1.0),
            rotation: f(p, "rotation", 0.0).clamp(0.0, 360.0),
            direction: if s(p, "direction", "clockwise") == "counterClockwise" { SpinDirection::CounterClockwise } else { SpinDirection::Clockwise },
            color1: colour(p, "color1", BLACK),
            color2: colour(p, "color2", WHITE),
        },
        "filter.render.jigsaw" => FilterParams::Jigsaw {
            x_tiles: u(p, "xTiles", 5, 1, 64),
            y_tiles: u(p, "yTiles", 5, 1, 64),
            bevel_width: f(p, "bevelWidth", 3.0).clamp(0.0, 100.0),
            highlight: f(p, "highlight", 0.5).clamp(0.0, 1.0),
            style: if s(p, "style", "square") == "curved" { JigsawStyle::Curved } else { JigsawStyle::Square },
        },
        "filter.render.lineNova" => FilterParams::LineNova {
            lines: u(p, "lines", 200, 1, 1000),
            sharpness: f(p, "sharpness", 1.0).clamp(1.0, 10.0),
            offset_radius: f(p, "offsetRadius", 100.0).clamp(0.0, 2000.0),
            randomness: f(p, "randomness", 0.3).clamp(0.0, 1.0),
            seed: seed(p, "seed", 0),
            color: colour(p, "foreground", BLACK),
        },
        "filter.render.spirograph" => FilterParams::Spirograph {
            curve: match s(p, "curveType", "spirograph") {
                "epitrochoid" => SpyroCurve::Epitrochoid,
                "sine" => SpyroCurve::Sine,
                "lissajous" => SpyroCurve::Lissajous,
                _ => SpyroCurve::Spirograph,
            },
            shape: match s(p, "shape", "circle") {
                "polygon" => SpyroShape::Polygon,
                "star" => SpyroShape::Star,
                "sine" => SpyroShape::Sine,
                "bumps" => SpyroShape::Bumps,
                "rack" => SpyroShape::Rack,
                "frame" => SpyroShape::Frame,
                _ => SpyroShape::Circle,
            },
            fixed_teeth: u(p, "fixedGearTeeth", 96, 1, 1000),
            moving_teeth: u(p, "movingGearTeeth", 36, 1, 1000),
            hole: f(p, "holePercent", 100.0).clamp(0.0, 100.0),
            sides: u(p, "sides", 5, 3, 32),
            morph: f(p, "morph", 0.5).clamp(0.0, 1.0),
            rotation: f(p, "rotation", 0.0).clamp(-360.0, 360.0),
            line_width: f(p, "lineWidth", 1.0).clamp(0.5, 50.0),
            color: colour(p, "foreground", BLACK),
        },
        // ---- Decor / Web ----
        "filter.decor.slide" => FilterParams::Slide {
            text: s(p, "text", "REDROB").to_string(),
            number: s(p, "number", "32").to_string(),
            font_size: f(p, "fontSize", 24.0).clamp(4.0, 200.0),
            color: colour(p, "fontColor", [1.0, 1.0, 0.0, 1.0]),
        },
        "filter.decor.coffeeStain" => {
            FilterParams::CoffeeStain { stains: u(p, "stains", 3, 1, 50), darken_only: b(p, "darkenOnly", true), seed: seed(p, "seed", 0) }
        }
        "filter.decor.stencilCarve" => FilterParams::StencilCarve { carve_white: b(p, "carveWhite", false), map: None },
        "filter.web.semiFlatten" => FilterParams::SemiFlatten { background: colour(p, "background", WHITE) },
        _ => return crate::gallery_cmds::params_for(id, p),
    })
}

/// Ids whose rendering uses the foreground/background colours (filled in by
/// `filters_ext::prepare`).
pub(crate) fn uses_colours(id: &str) -> bool {
    matches!(id, "filter.render.lineNova" | "filter.render.spirograph" | "filter.web.semiFlatten")
}

/// Lava and Circuit: `separateLayer` renders onto a new layer above the active one.
fn run_on_own_layer(s: &mut Session, id: &str, p: &Value) -> Result<Value> {
    if b(p, "separateLayer", false) {
        let name = if id == "filter.render.lava" { "Lava" } else { "Circuit" };
        s.execute("layer.new.layer", json!({"name": name}))?;
    }
    let mut params = p.clone();
    if let Value::Object(m) = &mut params {
        m.remove("separateLayer");
        m.remove("keepSelection");
    }
    crate::filters::run_filter(s, id, &params)
}

fn has_pixels(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    match d.doc.layer(id).ok_or("no active layer")?.content {
        LayerContent::Raster(_) => Ok(()),
        ref c => Err(format!("the active layer is {} {} layer, not a pixel layer", c.article(), c.kind_name())),
    }
}

// ---- Animation ----

/// The visible top-level pixel layers, bottom to top, as `(id, name, pixels over the canvas)`
/// in the document's alpha format.
fn frames_of(doc: &redrob_photo_doc::Document) -> Result<(redrob_photo_color::PixelFormat, Rect, Vec<(LayerId, String, Vec<f32>)>)> {
    let fmt = doc.pixel_format();
    let bounds = doc.bounds();
    let mut out = Vec::new();
    for l in &doc.layers {
        if !l.visible {
            continue;
        }
        let Some(surf) = l.surface() else { continue };
        let surf = if surf.format() == fmt { surf.clone() } else { surf.try_convert(fmt)? };
        out.push((l.id, l.name.clone(), surf.read_region(bounds)));
    }
    if out.is_empty() {
        return Err(EngineError::Other("the animation needs at least one visible pixel layer".into()));
    }
    Ok((fmt, bounds, out))
}

/// A new pixel layer over `bounds` from samples in `fmt`.
fn layer_from(name: String, fmt: redrob_photo_color::PixelFormat, bounds: Rect, data: &[f32]) -> Layer {
    let mut layer = Layer::raster(name, fmt);
    if let Some(surf) = layer.surface_mut() {
        surf.write_region(bounds, data);
        surf.prune();
    }
    layer
}

/// Straight-alpha `top` over `under`, in place (alpha last).
fn over_px(under: &mut [f32], top: &[f32]) {
    let n = under.len();
    let a = top[n - 1].clamp(0.0, 1.0);
    let ua = under[n - 1];
    let out_a = a + ua * (1.0 - a);
    if out_a <= 0.0 {
        under.iter_mut().for_each(|v| *v = 0.0);
        return;
    }
    for c in 0..n - 1 {
        under[c] = (top[c] * a + under[c] * ua * (1.0 - a)) / out_a;
    }
    under[n - 1] = out_a;
}

/// Adds `layers` (bottom to top) above the document's top layer and makes the last one active.
fn add_frames(s: &mut Session, label: &str, layers: Vec<Layer>) -> Result<Value> {
    let ids = s.edit(label, |doc, active| {
        let mut above = doc.top_layer();
        let mut ids = Vec::with_capacity(layers.len());
        for l in layers {
            above = Some(doc.insert_above(above, l));
            ids.push(above.map_or(0, |id| id.0));
        }
        *active = above;
        Ok(ids)
    })?;
    Ok(json!({ "frames": ids }))
}

/// Blend: `intermediateFrames` layers between each pair of consecutive visible layers (and from
/// the last back to the first when `looped`), optionally blurred up to `maxBlurRadius` px at the
/// midpoint of each transition.
fn blend_anim(s: &mut Session, p: &Value) -> Result<Value> {
    let steps = u(p, "intermediateFrames", 3, 1, 100) as usize;
    let blur = f(p, "maxBlurRadius", 0.0).clamp(0.0, 100.0);
    let looped = b(p, "looped", true);
    let (fmt, bounds, frames) = frames_of(&s.active().ok_or(EngineError::NoDocument)?.doc)?;
    if frames.len() < 2 {
        return Err(EngineError::Other("Blend needs at least two visible pixel layers".into()));
    }
    let n = fmt.channels();
    let mut layers = Vec::new();
    let pairs = if looped { frames.len() } else { frames.len() - 1 };
    for k in 0..pairs {
        let a = &frames[k].2;
        let bb = &frames[(k + 1) % frames.len()].2;
        layers.push(layer_from(format!("Frame {} ({})", layers.len() + 1, frames[k].1), fmt, bounds, a));
        for step in 1..=steps {
            let t = step as f32 / (steps + 1) as f32;
            let mut mixed = a.clone();
            for (m, (x, y)) in mixed.chunks_exact_mut(n).zip(a.chunks_exact(n).zip(bb.chunks_exact(n))) {
                for c in 0..n {
                    m[c] = x[c] + (y[c] - x[c]) * t;
                }
            }
            let mut layer = layer_from(format!("Frame {} (blend {step})", layers.len() + 1), fmt, bounds, &mixed);
            let radius = blur * (std::f32::consts::PI * t).sin();
            let blurred = if radius >= 0.1 {
                layer.surface().and_then(|surf| {
                    crate::filters::apply_filter_to_surface("filter.blur.gaussianBlur", &json!({"radius": radius}), surf, bounds, None, bounds)
                })
            } else {
                None
            };
            if let (Some(blurred), Some(dst)) = (blurred, layer.surface_mut()) {
                *dst = blurred;
            }
            layers.push(layer);
        }
    }
    if !looped {
        let last = frames.len() - 1;
        layers.push(layer_from(format!("Frame {} ({})", layers.len() + 1, frames[last].1), fmt, bounds, &frames[last].2));
    }
    add_frames(s, "Blend", layers)
}

/// Burn-In: the top visible layer (the text or shape, with transparency) is revealed from left
/// to right over the layer below it by a glow band, one layer per frame.
fn burn_in(s: &mut Session, p: &Value) -> Result<Value> {
    let glow = colour(p, "glowColor", WHITE);
    let fadeout = b(p, "fadeout", false);
    let fade_w = f(p, "fadeoutWidth", 100.0).clamp(1.0, 1000.0);
    let corner = f(p, "cornerRadius", 0.0).clamp(0.0, 100.0);
    let after = f(p, "afterGlow", 50.0).clamp(0.0, 100.0) / 100.0;
    let add_glow = b(p, "addGlowing", true);
    let speed = f(p, "speed", 50.0).clamp(1.0, 100.0);
    let (fmt, bounds, frames) = frames_of(&s.active().ok_or(EngineError::NoDocument)?.doc)?;
    if frames.len() < 2 {
        return Err(EngineError::Other("Burn-In needs a text (or shape) layer above a background layer".into()));
    }
    let n = fmt.channels();
    let text = &frames[frames.len() - 1].2;
    let background = &frames[frames.len() - 2].2;
    let w = bounds.width() as f32;
    let count = ((w + fade_w) / speed).ceil().clamp(1.0, 500.0) as usize;
    let glow_n = redrob_photo_raster::convert_pixel(&redrob_photo_color::PixelFormat::new(redrob_photo_color::ColorMode::Rgb, fmt.sample, true), &fmt, &glow);
    let mut layers = Vec::with_capacity(count);
    let wpx = bounds.width() as usize;
    for k in 0..count {
        let edge = (k as f32 + 1.0) * speed;
        let mut frame = background.clone();
        for (idx, (dst, t)) in frame.chunks_exact_mut(n).zip(text.chunks_exact(n)).enumerate() {
            let x = (idx % wpx) as f32 + 0.5;
            let y = (idx / wpx) as f32;
            // Softened band edge: the corner radius rounds the leading edge toward the top/bottom.
            let lead = edge - corner * (1.0 - (2.0 * y / bounds.height().max(1) as f32 - 1.0).abs());
            let d = lead - x;
            let ta = t[n - 1];
            if ta <= 0.0 {
                continue;
            }
            // Revealed text: full past the band, fading behind it when `fadeout`.
            let reveal = if d <= 0.0 {
                0.0
            } else if fadeout {
                (1.0 - (d / fade_w)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let mut px = t.to_vec();
            px[n - 1] = ta * reveal;
            // Glow: strongest at the leading edge, lingering with the after-glow behind it.
            let band = if d < 0.0 { (1.0 + d / (fade_w * 0.3)).clamp(0.0, 1.0) } else { (after + (1.0 - after) * (1.0 - d / fade_w).clamp(0.0, 1.0)).min(1.0) };
            let g = if add_glow { band * ta } else { 0.0 };
            if reveal > 0.0 {
                over_px(dst, &px);
            }
            if g > 0.0 {
                let mut gp = glow_n.clone();
                gp[n - 1] = g;
                over_px(dst, &gp);
            }
        }
        layers.push(layer_from(format!("Burn-In {}", k + 1), fmt, bounds, &frame));
    }
    add_frames(s, "Burn-In", layers)
}

/// Optimize (for GIF): each frame keeps only the pixels that differ from the frames before it;
/// Unoptimize rebuilds full frames from such a stack.
fn optimize(s: &mut Session, _p: &Value, unoptimize: bool) -> Result<Value> {
    let (fmt, bounds, frames) = frames_of(&s.active().ok_or(EngineError::NoDocument)?.doc)?;
    let n = fmt.channels();
    let mut composite = vec![0.0f32; frames[0].2.len()];
    let mut layers = Vec::with_capacity(frames.len());
    for (k, (_, name, data)) in frames.iter().enumerate() {
        let before = composite.clone();
        for (c, t) in composite.chunks_exact_mut(n).zip(data.chunks_exact(n)) {
            over_px(c, t);
        }
        let out = if unoptimize {
            composite.clone()
        } else {
            let mut out = composite.clone();
            if k > 0 {
                for (o, bf) in out.chunks_exact_mut(n).zip(before.chunks_exact(n)) {
                    let same = o.iter().zip(bf).all(|(a, b)| (a - b).abs() < 1.0 / 255.0);
                    if same {
                        o.iter_mut().for_each(|v| *v = 0.0);
                    }
                }
            }
            out
        };
        layers.push(layer_from(format!("{name} ({})", if unoptimize { "unoptimized" } else { "optimized" }), fmt, bounds, &out));
    }
    add_frames(s, if unoptimize { "Unoptimize" } else { "Optimize (for GIF)" }, layers)
}

fn anim_edge(p: &Value) -> AnimEdge {
    match s(p, "edgeBehavior", "wrap") {
        "smear" => AnimEdge::Smear,
        "black" => AnimEdge::Black,
        _ => AnimEdge::Wrap,
    }
}

/// The active layer's pixels over the canvas as an algo image plus its context.
fn active_image(s: &Session) -> Result<(redrob_photo_color::PixelFormat, Rect, algo::Image, algo::Ctx)> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = d.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let surf = l.surface().ok_or_else(|| EngineError::Other("the animation needs a pixel layer".into()))?;
    let fmt = d.doc.pixel_format();
    let surf = if surf.format() == fmt { surf.clone() } else { surf.try_convert(fmt)? };
    let bounds = d.doc.bounds();
    let img = algo::Image::read(&surf, bounds);
    Ok((fmt, bounds, img, algo::Ctx { bounds, mode: fmt.mode, alpha: fmt.alpha }))
}

/// Rippling: `frames` layers of a travelling ripple over the active layer.
fn rippling(s: &mut Session, p: &Value) -> Result<Value> {
    let frames = u(p, "frames", 15, 1, 100) as usize;
    let strength = f(p, "strength", 3.0).clamp(0.0, 100.0);
    let edge = anim_edge(p);
    let (fmt, bounds, img, ctx) = active_image(s)?;
    let layers = (0..frames)
        .map(|k| layer_from(format!("Ripple {}", k + 1), fmt, bounds, &algo::anim::ripple_frame(&img, &ctx, k as f32 / frames as f32, strength, edge)))
        .collect();
    add_frames(s, "Rippling", layers)
}

/// Spinning Globe: `frames` layers of the active layer wrapped round a turning globe.
fn spinning_globe(s: &mut Session, p: &Value) -> Result<Value> {
    let frames = u(p, "frames", 10, 1, 100) as usize;
    let left = b(p, "turnLeft", false);
    let transparent = b(p, "transparentBackground", true);
    let (fmt, bounds, img, ctx) = active_image(s)?;
    let n = fmt.channels();
    let white = redrob_photo_raster::convert_pixel(&redrob_photo_color::PixelFormat::new(redrob_photo_color::ColorMode::Rgb, fmt.sample, true), &fmt, &WHITE);
    let layers = (0..frames)
        .map(|k| {
            let mut data = algo::anim::globe_frame(&img, &ctx, k as f32 / frames as f32, left);
            if !transparent && fmt.alpha {
                // Opaque white behind the globe.
                for px in data.chunks_exact_mut(n) {
                    let a = px[n - 1];
                    for c in 0..n - 1 {
                        px[c] = px[c] * a + white[c] * (1.0 - a);
                    }
                    px[n - 1] = 1.0;
                }
            }
            layer_from(format!("Globe {}", k + 1), fmt, bounds, &data)
        })
        .collect();
    add_frames(s, "Spinning Globe", layers)
}

/// Waves: `frames` layers of concentric waves over the active layer.
fn waves_anim(s: &mut Session, p: &Value) -> Result<Value> {
    let amplitude = f(p, "amplitude", 10.0).clamp(0.0, 100.0);
    let wavelength = f(p, "wavelength", 10.0).clamp(1.0, 1000.0);
    let frames = u(p, "frames", 6, 1, 100) as usize;
    let invert = b(p, "invertDirection", false);
    let (fmt, bounds, img, ctx) = active_image(s)?;
    let layers = (0..frames)
        .map(|k| {
            layer_from(
                format!("Wave {}", k + 1),
                fmt,
                bounds,
                &algo::anim::waves_frame(&img, &ctx, k as f32 / frames as f32, amplitude, wavelength, invert, AnimEdge::Smear),
            )
        })
        .collect();
    add_frames(s, "Waves", layers)
}

macro_rules! cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal) => {
        CommandSpec {
            id: $id,
            label: $label,
            menu: &[$($m),*],
            shortcut: None,
            params: $params,
            enabled: crate::filters::has_filterable_layer,
            run: |s, p| crate::filters::run_filter(s, $id, p),
            journal: true,
        }
    };
}

macro_rules! anim {
    ($id:literal, $label:literal, $params:literal, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &["Filter", "Animation"], shortcut: None, params: $params, enabled: has_pixels, run: $run, journal: true }
    };
}

/// The command specs of this batch.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        // Map
        cmd!(
            "filter.map.fractalTrace",
            "Fractal Trace…",
            ["Filter", "Map"],
            r##"{"fractal":"mandelbrot|julia","x1":-50..50=-1,"x2":-50..50=0.5,"y1":-50..50=-1,"y2":-50..50=1,"jx":-50..50=0.5,"jy":-50..50=0.5,"depth":1..64=3,"bailout":1..100000=10000,"abyss":"loop|clamp|black|white|transparent"}"##
        ),
        cmd!("filter.map.illusion", "Illusion…", ["Filter", "Map"], r##"{"division":1..64=8,"type":"type1|type2"}"##),
        cmd!(
            "filter.map.littlePlanet",
            "Little Planet…",
            ["Filter", "Map"],
            r##"{"pan":-180..180=0,"tilt":-180..180=90,"spin":-360..360=0,"zoom":1..1000=100,"inverse":bool}"##
        ),
        cmd!(
            "filter.map.recursiveTransform",
            "Recursive Transform…",
            ["Filter", "Map"],
            r##"{"scale":1..1000=80,"angle":-360..360=10,"offsetX":-1..1=0,"offsetY":-1..1=0,"firstIteration":0..32=0,"lastIteration":1..32=3,"fadeColor":color=#000000,"fadeOpacity":0..100=0,"pasteBelow":bool}"##
        ),
        cmd!("filter.map.tileSeamless", "Tile Seamless", ["Filter", "Map"], "{}"),
        cmd!(
            "filter.map.mapObject",
            "Map Object…",
            ["Filter", "Map"],
            r##"{"mapTo":"plane|sphere|box|cylinder","rotationX":-180..180=0,"rotationY":-180..180=0,"rotationZ":-180..180=0,"lightX":-5..5=-0.5,"lightY":-5..5=-0.5,"lightZ":-5..5=2,"ambient":0..1=0.3,"diffuse":0..1=1,"specular":0..1=0.5,"highlight":1..100=27,"radius":0.05..2=0.25,"transparentBackground":bool=true}"##
        ),
        cmd!(
            "filter.map.smallTiles",
            "Small Tiles…",
            ["Filter", "Map"],
            r##"{"segments":1..32=2,"flip":"none|horizontal|vertical|both","flipWhich":"all|alternate|explicit","opacity":0..100=100}"##
        ),
        // Render › Fractals
        cmd!(
            "filter.render.fractalExplorer",
            "Fractal Explorer…",
            ["Filter", "Render", "Fractals"],
            r##"{"fractalType":"mandelbrot|julia|barnsley1|barnsley2|barnsley3|spider|manOWar|lambda|sierpinski","x1":-3..3=-2,"x2":-3..3=2,"y1":-3..3=-1.5,"y2":-3..3=1.5,"iterations":1..1000=50,"cx":-2.5..2.5=-0.75,"cy":-2.5..2.5=0.2,"colors":2..8192=256,"red":0..1=1,"green":0..1=1,"blue":0..1=1,"redInversion":bool,"greenInversion":bool,"blueInversion":bool}"##
        ),
        cmd!(
            "filter.render.ifsFractal",
            "IFS Fractal…",
            ["Filter", "Render", "Fractals"],
            r##"{"preset":"fern|sierpinski|dragon|spiral|tree|snowflake","iterations":1..2000=50,"spotRadius":0.3..20=0.75,"x":-1..1=0,"y":-1..1=0,"scale":0.1..5=1,"angle":-180..180=0,"color":color=#2a7d2e,"seed":u32=0}"##
        ),
        cmd!(
            "filter.render.sphereDesigner",
            "Sphere Designer…",
            ["Filter", "Render"],
            r##"{"texture":"solid|checker|marble|lizard|phong|noise|wood|spiral|spots","color1":color=#cc3333,"color2":color=#ffffff,"scale":0.05..10=1,"turbulence":0..10=1,"amount":0..1=1,"exponent":1..100=20,"lightX":-5..5=-1,"lightY":-5..5=1,"lightZ":-5..5=1,"ambient":0..1=0.2,"bump":0..1=0,"radius":0.05..2=0.8,"transparentBackground":bool}"##
        ),
        // Render › Noise
        cmd!(
            "filter.render.cellNoise",
            "Cell Noise…",
            ["Filter", "Render", "Noise"],
            r##"{"scale":1..1000=100,"shape":1..2=2,"rank":1..3=1,"iterations":1..8=1,"palettize":bool,"seed":u32=0}"##
        ),
        cmd!("filter.render.simplexNoise", "Simplex Noise…", ["Filter", "Render", "Noise"], r##"{"scale":1..1000=100,"iterations":1..8=1,"seed":u32=0}"##),
        // Render
        CommandSpec {
            id: "filter.render.lava",
            label: "Lava…",
            menu: &["Filter", "Render"],
            shortcut: None,
            params: r##"{"seed":u32=10,"size":2..100=10,"roughness":1..10=7,"gradient":"incandescent|germanFlag|caribbeanBlues|golden|fullSpectrum","separateLayer":bool}"##,
            enabled: crate::filters::has_filterable_layer,
            run: |s, p| run_on_own_layer(s, "filter.render.lava", p),
            journal: true,
        },
        CommandSpec {
            id: "filter.render.circuit",
            label: "Circuit…",
            menu: &["Filter", "Render"],
            shortcut: None,
            params: r##"{"oilifyMaskSize":3..50=17,"circuitSeed":u32=3,"noBackground":bool,"separateLayer":bool}"##,
            enabled: crate::filters::has_filterable_layer,
            run: |s, p| run_on_own_layer(s, "filter.render.circuit", p),
            journal: true,
        },
        cmd!("filter.render.qbist", "Qbist…", ["Filter", "Render", "Pattern"], r##"{"seed":u32=0,"antialiasing":bool=true,"oversample":1..4=2}"##),
        cmd!(
            "filter.render.cmlExplorer",
            "CML Explorer…",
            ["Filter", "Render", "Pattern"],
            r##"{"hueFunction":"keepImage|keepFirstValue|fillWithK|logistic|deltaSinCurve|sinCurve|sinCurveStep|random"="logistic","saturationFunction":"keepImage|keepFirstValue|fillWithK|logistic|deltaSinCurve|sinCurve|sinCurveStep|random"="fillWithK","valueFunction":"keepImage|keepFirstValue|fillWithK|logistic|deltaSinCurve|sinCurve|sinCurveStep|random"="logistic","parameterK":0..10=3.6,"coupling":0..1=0.5,"zoom":1..64=1,"startOffset":0..100=0,"seed":u32=0}"##
        ),
        // Render › Pattern
        cmd!(
            "filter.render.bayerMatrix",
            "Bayer Matrix…",
            ["Filter", "Render", "Pattern"],
            r##"{"subdivisions":0..15=1,"xScale":1..100=1,"yScale":1..100=1,"rotation":"none|quarter|half|threeQuarters","reflect":bool,"amplitude":-1000..1000=1,"offset":-2..2=0,"exponent":0.01..10=1,"xOffset":px=0,"yOffset":px=0}"##
        ),
        cmd!(
            "filter.render.checkerboard",
            "Checkerboard…",
            ["Filter", "Render", "Pattern"],
            r##"{"width":1..512=16,"height":1..512=16,"xOffset":px=0,"yOffset":px=0,"color1":color=#000000,"color2":color=#ffffff}"##
        ),
        cmd!(
            "filter.render.diffractionPatterns",
            "Diffraction Patterns…",
            ["Filter", "Render", "Pattern"],
            r##"{"redFrequency":0..20=0.815,"greenFrequency":0..20=1.221,"blueFrequency":0..20=1.123,"redContours":0..10=0.821,"greenContours":0..10=0.821,"blueContours":0..10=0.974,"redSharpEdges":0..1=0.61,"greenSharpEdges":0..1=0.677,"blueSharpEdges":0..1=0.636,"brightness":0..2=1,"scattering":0..100=37,"polarization":-180..180=-85}"##
        ),
        cmd!(
            "filter.render.grid",
            "Grid…",
            ["Filter", "Render", "Pattern"],
            r##"{"xSpacing":1..512=32,"ySpacing":1..512=32,"xOffset":px=0,"yOffset":px=0,"lineWidth":0..64=4,"lineHeight":0..64=4,"color":color=#000000}"##
        ),
        cmd!(
            "filter.render.linearSinusoid",
            "Linear Sinusoid…",
            ["Filter", "Render", "Pattern"],
            r##"{"xPeriod":0.1..1000=128,"yPeriod":0.1..1000=128,"xAmplitude":0..10=1,"yAmplitude":0..10=1,"xPhase":-360..360=0,"yPhase":-360..360=0,"angle":-180..180=90,"offset":-1..1=0,"exponent":0.01..10=1,"xOffset":px=0,"yOffset":px=0,"rotation":-180..180=0,"supersampling":1..8=1}"##
        ),
        cmd!(
            "filter.render.maze",
            "Maze…",
            ["Filter", "Render", "Pattern"],
            r##"{"cellWidth":1..64=16,"cellHeight":1..64=16,"algorithm":"depthFirst|prim","tileable":bool,"seed":u32=0,"foreground":color=#000000,"background":color=#ffffff}"##
        ),
        cmd!(
            "filter.render.sinus",
            "Sinus…",
            ["Filter", "Render", "Pattern"],
            r##"{"xScale":0.1..100=15,"yScale":0.1..100=15,"complexity":0..15=1,"seed":u32=42,"tileable":bool,"perturbation":bool=true,"blendMode":"linear|bilinear|sinusoidal"="sinusoidal","exponent":0.01..10=1,"color1":color=#ffff00,"color2":color=#0000ff}"##
        ),
        cmd!(
            "filter.render.spiral",
            "Spiral…",
            ["Filter", "Render", "Pattern"],
            r##"{"type":"linear|logarithmic","x":0..1=0.5,"y":0..1=0.5,"radius":1..1000=100,"base":1.01..20=2,"balance":-1..1=0,"rotation":0..360=0,"direction":"clockwise|counterClockwise","color1":color=#000000,"color2":color=#ffffff}"##
        ),
        cmd!(
            "filter.render.jigsaw",
            "Jigsaw…",
            ["Filter", "Render", "Pattern"],
            r##"{"xTiles":1..64=5,"yTiles":1..64=5,"bevelWidth":0..100=3,"highlight":0..1=0.5,"style":"square|curved"}"##
        ),
        cmd!(
            "filter.render.lineNova",
            "Line Nova…",
            ["Filter", "Render"],
            r##"{"lines":1..1000=200,"sharpness":1..10=1,"offsetRadius":0..2000=100,"randomness":0..1=0.3,"seed":u32=0,"foreground":json}"##
        ),
        cmd!(
            "filter.render.spirograph",
            "Spirograph…",
            ["Filter", "Render"],
            r##"{"curveType":"spirograph|epitrochoid|sine|lissajous","shape":"circle|polygon|star|sine|bumps|rack|frame","fixedGearTeeth":1..1000=96,"movingGearTeeth":1..1000=36,"holePercent":0..100=100,"sides":3..32=5,"morph":0..1=0.5,"rotation":-360..360=0,"lineWidth":0.5..50=1,"foreground":json}"##
        ),
        // Decor / Web
        cmd!("filter.decor.slide", "Slide…", ["Filter", "Decor"], r##"{"text":text,"number":text,"fontSize":4..200=24,"fontColor":color=#ffff00}"##),
        cmd!("filter.decor.coffeeStain", "Coffee Stain…", ["Filter", "Decor"], r##"{"stains":1..50=3,"darkenOnly":bool=true,"seed":u32=0}"##),
        cmd!("filter.decor.stencilCarve", "Stencil Carve…", ["Filter", "Decor"], r##"{"carveWhite":bool,"mapDocument":doc,"mapPath":text,"mapLayer":json}"##),
        cmd!("filter.web.semiFlatten", "Semi-Flatten", ["Filter", "Web"], r##"{"background":json}"##),
        // Animation
        anim!("filter.animation.blend", "Blend…", r##"{"intermediateFrames":1..100=3,"maxBlurRadius":0..100=0,"looped":bool=true}"##, blend_anim),
        anim!(
            "filter.animation.burnIn",
            "Burn-In…",
            r##"{"glowColor":color=#ffffff,"fadeout":bool,"fadeoutWidth":1..1000=100,"cornerRadius":0..100=0,"afterGlow":0..100=50,"addGlowing":bool=true,"speed":1..100=50}"##,
            burn_in
        ),
        anim!("filter.animation.optimize", "Optimize (for GIF)", "{}", |s, p| optimize(s, p, false)),
        anim!("filter.animation.unoptimize", "Unoptimize", "{}", |s, p| optimize(s, p, true)),
        anim!("filter.animation.rippling", "Rippling…", r##"{"frames":1..100=15,"strength":0..100=3,"edgeBehavior":"wrap|smear|black"}"##, rippling),
        anim!(
            "filter.animation.spinningGlobe",
            "Spinning Globe…",
            r##"{"frames":1..100=10,"turnLeft":bool,"transparentBackground":bool=true}"##,
            spinning_globe
        ),
        anim!("filter.animation.waves", "Waves…", r##"{"amplitude":0..100=10,"wavelength":1..1000=10,"frames":1..100=6,"invertDirection":bool}"##, waves_anim),
    ]
}
