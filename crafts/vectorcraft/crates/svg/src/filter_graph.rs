//! SVG `<filter>` ⇄ [`FilterGraph`]: the primitives of a filter graph written as their SVG
//! elements (export), and a parsed `<filter>` (usvg's) kept whole as a filter graph when its
//! chain isn't one of the effects the canvas draws natively (import, see [`super::import::fx`]).

use redrob_vector_effects::svgfilter::{
    BlendOp, Channel, CompositeOp, EdgeMode, FilterGraph, Kind, Light, MatrixMode, MorphologyOp, Primitive, Transfer,
};
use usvg::filter as uf;

fn n(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" { "0".to_string() } else { s.to_string() }
}

fn input_attr(name: &str, v: &str) -> String {
    if v.is_empty() { String::new() } else { format!(" {name}=\"{}\"", escape(v)) }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;")
}

fn transfer_el(name: &str, f: &Transfer) -> String {
    let list = |v: &[f64]| v.iter().map(|x| n(*x)).collect::<Vec<_>>().join(" ");
    match f {
        Transfer::Identity => String::new(),
        Transfer::Table { values } => format!("<{name} type=\"table\" tableValues=\"{}\"/>", list(values)),
        Transfer::Discrete { values } => format!("<{name} type=\"discrete\" tableValues=\"{}\"/>", list(values)),
        Transfer::Linear { slope, intercept } => format!("<{name} type=\"linear\" slope=\"{}\" intercept=\"{}\"/>", n(*slope), n(*intercept)),
        Transfer::Gamma { amplitude, exponent, offset } => {
            format!("<{name} type=\"gamma\" amplitude=\"{}\" exponent=\"{}\" offset=\"{}\"/>", n(*amplitude), n(*exponent), n(*offset))
        }
    }
}

fn light_el(l: &Light) -> String {
    match l {
        Light::Distant { azimuth, elevation } => format!("<feDistantLight azimuth=\"{}\" elevation=\"{}\"/>", n(*azimuth), n(*elevation)),
        Light::Point { x, y, z } => format!("<fePointLight x=\"{}\" y=\"{}\" z=\"{}\"/>", n(*x), n(*y), n(*z)),
        Light::Spot { x, y, z, px, py, pz, exponent, cone } => {
            let cone = if *cone > 0.0 { format!(" limitingConeAngle=\"{}\"", n(*cone)) } else { String::new() };
            format!(
                "<feSpotLight x=\"{}\" y=\"{}\" z=\"{}\" pointsAtX=\"{}\" pointsAtY=\"{}\" pointsAtZ=\"{}\" specularExponent=\"{}\"{cone}/>",
                n(*x),
                n(*y),
                n(*z),
                n(*px),
                n(*py),
                n(*pz),
                n(*exponent)
            )
        }
    }
}

/// One primitive as its SVG element. `offset`: (x, y) added to point and spot light positions
/// (document → user space).
pub(crate) fn primitive_markup(p: &Primitive, offset: (f64, f64)) -> String {
    let result = input_attr("result", &p.result);
    let el = p.kind.element();
    match &p.kind {
        Kind::GaussianBlur { input, sd_x, sd_y } => {
            format!("<{el}{} stdDeviation=\"{} {}\"{result}/>", input_attr("in", input), n(*sd_x), n(*sd_y))
        }
        Kind::Offset { input, dx, dy } => format!("<{el}{} dx=\"{}\" dy=\"{}\"{result}/>", input_attr("in", input), n(*dx), n(*dy)),
        Kind::Flood { color, opacity } => format!("<{el} flood-color=\"{}\" flood-opacity=\"{}\"{result}/>", escape(color), n(*opacity)),
        Kind::Merge { inputs } => {
            let nodes: String = inputs.iter().map(|i| format!("<feMergeNode{}/>", input_attr("in", i))).collect();
            format!("<{el}{result}>{nodes}</{el}>")
        }
        Kind::Composite { input, in2, operator, k1, k2, k3, k4 } => {
            let op = match operator {
                CompositeOp::Over => "over",
                CompositeOp::In => "in",
                CompositeOp::Out => "out",
                CompositeOp::Atop => "atop",
                CompositeOp::Xor => "xor",
                CompositeOp::Arithmetic => "arithmetic",
            };
            let ks = if *operator == CompositeOp::Arithmetic {
                format!(" k1=\"{}\" k2=\"{}\" k3=\"{}\" k4=\"{}\"", n(*k1), n(*k2), n(*k3), n(*k4))
            } else {
                String::new()
            };
            format!("<{el}{}{} operator=\"{op}\"{ks}{result}/>", input_attr("in", input), input_attr("in2", in2))
        }
        Kind::ColorMatrix { input, mode, values } => {
            let (ty, vals) = match mode {
                MatrixMode::Matrix => ("matrix", values.iter().map(|v| n(*v)).collect::<Vec<_>>().join(" ")),
                MatrixMode::Saturate => ("saturate", n(values.first().copied().unwrap_or(1.0))),
                MatrixMode::HueRotate => ("hueRotate", n(values.first().copied().unwrap_or(0.0))),
                MatrixMode::LuminanceToAlpha => ("luminanceToAlpha", String::new()),
            };
            let vals = if vals.is_empty() { String::new() } else { format!(" values=\"{vals}\"") };
            format!("<{el}{} type=\"{ty}\"{vals}{result}/>", input_attr("in", input))
        }
        Kind::ComponentTransfer { input, func_r, func_g, func_b, func_a } => {
            let body = [("feFuncR", func_r), ("feFuncG", func_g), ("feFuncB", func_b), ("feFuncA", func_a)]
                .iter()
                .map(|(name, f)| transfer_el(name, f))
                .collect::<String>();
            format!("<{el}{}{result}>{body}</{el}>", input_attr("in", input))
        }
        Kind::ConvolveMatrix { input, columns, rows, kernel, divisor, bias, target_x, target_y, edge_mode, preserve_alpha } => {
            let div = if *divisor != 0.0 { format!(" divisor=\"{}\"", n(*divisor)) } else { String::new() };
            let tx = target_x.map(|t| format!(" targetX=\"{t}\"")).unwrap_or_default();
            let ty = target_y.map(|t| format!(" targetY=\"{t}\"")).unwrap_or_default();
            let edge = match edge_mode {
                EdgeMode::Duplicate => "duplicate",
                EdgeMode::Wrap => "wrap",
                EdgeMode::None => "none",
            };
            format!(
                "<{el}{} order=\"{columns} {rows}\" kernelMatrix=\"{}\"{div} bias=\"{}\"{tx}{ty} edgeMode=\"{edge}\" preserveAlpha=\"{preserve_alpha}\"{result}/>",
                input_attr("in", input),
                kernel.iter().map(|v| n(*v)).collect::<Vec<_>>().join(" "),
                n(*bias)
            )
        }
        Kind::Morphology { input, operator, radius_x, radius_y } => {
            let op = match operator {
                MorphologyOp::Erode => "erode",
                MorphologyOp::Dilate => "dilate",
            };
            format!("<{el}{} operator=\"{op}\" radius=\"{} {}\"{result}/>", input_attr("in", input), n(*radius_x), n(*radius_y))
        }
        Kind::Turbulence { base_frequency_x, base_frequency_y, num_octaves, seed, stitch_tiles, fractal_noise } => format!(
            "<{el} type=\"{}\" baseFrequency=\"{} {}\" numOctaves=\"{num_octaves}\" seed=\"{seed}\" stitchTiles=\"{}\"{result}/>",
            if *fractal_noise { "fractalNoise" } else { "turbulence" },
            n(*base_frequency_x),
            n(*base_frequency_y),
            if *stitch_tiles { "stitch" } else { "noStitch" }
        ),
        Kind::DisplacementMap { input, in2, scale, x_channel, y_channel } => {
            let ch = |c: &Channel| match c {
                Channel::R => "R",
                Channel::G => "G",
                Channel::B => "B",
                Channel::A => "A",
            };
            format!(
                "<{el}{}{} scale=\"{}\" xChannelSelector=\"{}\" yChannelSelector=\"{}\"{result}/>",
                input_attr("in", input),
                input_attr("in2", in2),
                n(*scale),
                ch(x_channel),
                ch(y_channel)
            )
        }
        Kind::DiffuseLighting { input, surface_scale, diffuse_constant, color, light } => format!(
            "<{el}{} surfaceScale=\"{}\" diffuseConstant=\"{}\" lighting-color=\"{}\"{result}>{}</{el}>",
            input_attr("in", input),
            n(*surface_scale),
            n(*diffuse_constant),
            escape(color),
            light_el(&shifted(light, offset))
        ),
        Kind::SpecularLighting { input, surface_scale, specular_constant, specular_exponent, color, light } => format!(
            "<{el}{} surfaceScale=\"{}\" specularConstant=\"{}\" specularExponent=\"{}\" lighting-color=\"{}\"{result}>{}</{el}>",
            input_attr("in", input),
            n(*surface_scale),
            n(*specular_constant),
            n(*specular_exponent),
            escape(color),
            light_el(&shifted(light, offset))
        ),
        Kind::Blend { input, in2, mode } => {
            let m = match mode {
                BlendOp::Normal => "normal",
                BlendOp::Multiply => "multiply",
                BlendOp::Screen => "screen",
                BlendOp::Overlay => "overlay",
                BlendOp::Darken => "darken",
                BlendOp::Lighten => "lighten",
                BlendOp::ColorDodge => "color-dodge",
                BlendOp::ColorBurn => "color-burn",
                BlendOp::HardLight => "hard-light",
                BlendOp::SoftLight => "soft-light",
                BlendOp::Difference => "difference",
                BlendOp::Exclusion => "exclusion",
            };
            format!("<{el}{}{} mode=\"{m}\"{result}/>", input_attr("in", input), input_attr("in2", in2))
        }
        Kind::DropShadow { input, dx, dy, sd, color, opacity } => format!(
            "<{el}{} dx=\"{}\" dy=\"{}\" stdDeviation=\"{}\" flood-color=\"{}\" flood-opacity=\"{}\"{result}/>",
            input_attr("in", input),
            n(*dx),
            n(*dy),
            n(*sd),
            escape(color),
            n(*opacity)
        ),
        Kind::Tile { input } => format!("<{el}{}{result}/>", input_attr("in", input)),
    }
}

/// A light with its position moved by `offset` (lights are kept from the object's top-left corner;
/// SVG places them in user space).
fn shifted(l: &Light, (ox, oy): (f64, f64)) -> Light {
    match *l {
        Light::Point { x, y, z } => Light::Point { x: x + ox, y: y + oy, z },
        Light::Spot { x, y, z, px, py, pz, exponent, cone } => Light::Spot { x: x + ox, y: y + oy, z, px: px + ox, py: py + oy, pz, exponent, cone },
        d => d,
    }
}

/// The primitives of `g` as SVG markup, lights offset by `offset`.
pub(crate) fn graph_markup(g: &FilterGraph, offset: (f64, f64)) -> String {
    g.primitives.iter().map(|p| primitive_markup(p, offset)).collect()
}

fn input(i: &uf::Input) -> String {
    match i {
        uf::Input::SourceGraphic => "SourceGraphic".into(),
        uf::Input::SourceAlpha => "SourceAlpha".into(),
        uf::Input::Reference(r) => r.clone(),
    }
}

fn hex(c: usvg::Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)
}

fn transfer(f: &uf::TransferFunction) -> Transfer {
    match f {
        uf::TransferFunction::Identity => Transfer::Identity,
        uf::TransferFunction::Table(v) => Transfer::Table { values: v.iter().map(|x| f64::from(*x)).collect() },
        uf::TransferFunction::Discrete(v) => Transfer::Discrete { values: v.iter().map(|x| f64::from(*x)).collect() },
        uf::TransferFunction::Linear { slope, intercept } => Transfer::Linear { slope: f64::from(*slope), intercept: f64::from(*intercept) },
        uf::TransferFunction::Gamma { amplitude, exponent, offset } => {
            Transfer::Gamma { amplitude: f64::from(*amplitude), exponent: f64::from(*exponent), offset: f64::from(*offset) }
        }
    }
}

fn light(l: uf::LightSource, k: f64, origin: (f64, f64)) -> Light {
    let at = |x: f32, y: f32| ((f64::from(x) - origin.0) * k, (f64::from(y) - origin.1) * k);
    match l {
        uf::LightSource::DistantLight(d) => Light::Distant { azimuth: f64::from(d.azimuth), elevation: f64::from(d.elevation) },
        uf::LightSource::PointLight(p) => {
            let (x, y) = at(p.x, p.y);
            Light::Point { x, y, z: f64::from(p.z) * k }
        }
        uf::LightSource::SpotLight(s) => {
            let (x, y) = at(s.x, s.y);
            let (px, py) = at(s.points_at_x, s.points_at_y);
            Light::Spot {
                x,
                y,
                z: f64::from(s.z) * k,
                px,
                py,
                pz: f64::from(s.points_at_z) * k,
                exponent: f64::from(s.specular_exponent.get()),
                cone: s.limiting_cone_angle.map_or(0.0, f64::from),
            }
        }
    }
}

/// A parsed filter as a graph: `k` scales user lengths to document units; `bounds` is the
/// filtered element's bounding box in user space (the region and lights are kept relative to it).
/// `None` when a primitive has no counterpart (`feImage`).
pub(crate) fn graph_of(f: &uf::Filter, k: f64, bounds: Option<usvg::tiny_skia_path::Rect>) -> Option<FilterGraph> {
    let mut g = FilterGraph::default();
    if let Some(b) = bounds.filter(|b| b.width() > 0.0 && b.height() > 0.0) {
        let r = f.rect();
        let (bw, bh) = (f64::from(b.width()), f64::from(b.height()));
        g.region = [
            (f64::from(r.x()) - f64::from(b.x())) / bw,
            (f64::from(r.y()) - f64::from(b.y())) / bh,
            f64::from(r.width()) / bw,
            f64::from(r.height()) / bh,
        ];
    }
    let origin = bounds.map_or((0.0, 0.0), |b| (f64::from(b.x()), f64::from(b.y())));
    for p in f.primitives() {
        let kind = match p.kind() {
            uf::Kind::GaussianBlur(b) => {
                Kind::GaussianBlur { input: input(b.input()), sd_x: f64::from(b.std_dev_x().get()) * k, sd_y: f64::from(b.std_dev_y().get()) * k }
            }
            uf::Kind::Offset(o) => Kind::Offset { input: input(o.input()), dx: f64::from(o.dx()) * k, dy: f64::from(o.dy()) * k },
            uf::Kind::Flood(fl) => Kind::Flood { color: hex(fl.color()), opacity: f64::from(fl.opacity().get()) },
            uf::Kind::Merge(m) => Kind::Merge { inputs: m.inputs().iter().map(input).collect() },
            uf::Kind::Composite(c) => {
                let (operator, ks) = match c.operator() {
                    uf::CompositeOperator::Over => (CompositeOp::Over, [0.0; 4]),
                    uf::CompositeOperator::In => (CompositeOp::In, [0.0; 4]),
                    uf::CompositeOperator::Out => (CompositeOp::Out, [0.0; 4]),
                    uf::CompositeOperator::Atop => (CompositeOp::Atop, [0.0; 4]),
                    uf::CompositeOperator::Xor => (CompositeOp::Xor, [0.0; 4]),
                    uf::CompositeOperator::Arithmetic { k1, k2, k3, k4 } => {
                        (CompositeOp::Arithmetic, [f64::from(k1), f64::from(k2), f64::from(k3), f64::from(k4)])
                    }
                };
                Kind::Composite { input: input(c.input1()), in2: input(c.input2()), operator, k1: ks[0], k2: ks[1], k3: ks[2], k4: ks[3] }
            }
            uf::Kind::ColorMatrix(c) => {
                let (mode, values) = match c.kind() {
                    uf::ColorMatrixKind::Matrix(m) => (MatrixMode::Matrix, m.iter().map(|v| f64::from(*v)).collect()),
                    uf::ColorMatrixKind::Saturate(s) => (MatrixMode::Saturate, vec![f64::from(s.get())]),
                    uf::ColorMatrixKind::HueRotate(a) => (MatrixMode::HueRotate, vec![f64::from(*a)]),
                    uf::ColorMatrixKind::LuminanceToAlpha => (MatrixMode::LuminanceToAlpha, vec![]),
                };
                Kind::ColorMatrix { input: input(c.input()), mode, values }
            }
            uf::Kind::ComponentTransfer(c) => Kind::ComponentTransfer {
                input: input(c.input()),
                func_r: transfer(c.func_r()),
                func_g: transfer(c.func_g()),
                func_b: transfer(c.func_b()),
                func_a: transfer(c.func_a()),
            },
            uf::Kind::ConvolveMatrix(c) => {
                let m = c.matrix();
                Kind::ConvolveMatrix {
                    input: input(c.input()),
                    columns: m.columns(),
                    rows: m.rows(),
                    kernel: m.data().iter().map(|v| f64::from(*v)).collect(),
                    divisor: f64::from(c.divisor().get()),
                    bias: f64::from(c.bias()),
                    target_x: Some(m.target_x()),
                    target_y: Some(m.target_y()),
                    edge_mode: match c.edge_mode() {
                        uf::EdgeMode::None => EdgeMode::None,
                        uf::EdgeMode::Duplicate => EdgeMode::Duplicate,
                        uf::EdgeMode::Wrap => EdgeMode::Wrap,
                    },
                    preserve_alpha: c.preserve_alpha(),
                }
            }
            uf::Kind::Morphology(m) => Kind::Morphology {
                input: input(m.input()),
                operator: match m.operator() {
                    uf::MorphologyOperator::Erode => MorphologyOp::Erode,
                    uf::MorphologyOperator::Dilate => MorphologyOp::Dilate,
                },
                radius_x: f64::from(m.radius_x().get()) * k,
                radius_y: f64::from(m.radius_y().get()) * k,
            },
            uf::Kind::Turbulence(t) => Kind::Turbulence {
                base_frequency_x: f64::from(t.base_frequency_x().get()) / k.max(1e-9),
                base_frequency_y: f64::from(t.base_frequency_y().get()) / k.max(1e-9),
                num_octaves: t.num_octaves(),
                seed: t.seed(),
                stitch_tiles: t.stitch_tiles(),
                fractal_noise: t.kind() == uf::TurbulenceKind::FractalNoise,
            },
            uf::Kind::DisplacementMap(d) => {
                let ch = |c: uf::ColorChannel| match c {
                    uf::ColorChannel::R => Channel::R,
                    uf::ColorChannel::G => Channel::G,
                    uf::ColorChannel::B => Channel::B,
                    uf::ColorChannel::A => Channel::A,
                };
                Kind::DisplacementMap {
                    input: input(d.input1()),
                    in2: input(d.input2()),
                    scale: f64::from(d.scale()) * k,
                    x_channel: ch(d.x_channel_selector()),
                    y_channel: ch(d.y_channel_selector()),
                }
            }
            uf::Kind::DiffuseLighting(d) => Kind::DiffuseLighting {
                input: input(d.input()),
                surface_scale: f64::from(d.surface_scale()),
                diffuse_constant: f64::from(d.diffuse_constant()),
                color: hex(d.lighting_color()),
                light: light(d.light_source(), k, origin),
            },
            uf::Kind::SpecularLighting(s) => Kind::SpecularLighting {
                input: input(s.input()),
                surface_scale: f64::from(s.surface_scale()),
                specular_constant: f64::from(s.specular_constant()),
                specular_exponent: f64::from(s.specular_exponent()),
                color: hex(s.lighting_color()),
                light: light(s.light_source(), k, origin),
            },
            uf::Kind::Blend(b) => Kind::Blend {
                input: input(b.input1()),
                in2: input(b.input2()),
                mode: match b.mode() {
                    usvg::BlendMode::Multiply => BlendOp::Multiply,
                    usvg::BlendMode::Screen => BlendOp::Screen,
                    usvg::BlendMode::Overlay => BlendOp::Overlay,
                    usvg::BlendMode::Darken => BlendOp::Darken,
                    usvg::BlendMode::Lighten => BlendOp::Lighten,
                    usvg::BlendMode::ColorDodge => BlendOp::ColorDodge,
                    usvg::BlendMode::ColorBurn => BlendOp::ColorBurn,
                    usvg::BlendMode::HardLight => BlendOp::HardLight,
                    usvg::BlendMode::SoftLight => BlendOp::SoftLight,
                    usvg::BlendMode::Difference => BlendOp::Difference,
                    usvg::BlendMode::Exclusion => BlendOp::Exclusion,
                    _ => BlendOp::Normal,
                },
            },
            uf::Kind::DropShadow(d) => Kind::DropShadow {
                input: input(d.input()),
                dx: f64::from(d.dx()) * k,
                dy: f64::from(d.dy()) * k,
                sd: f64::from((d.std_dev_x().get() + d.std_dev_y().get()) / 2.0) * k,
                color: hex(d.color()),
                opacity: f64::from(d.opacity().get()),
            },
            uf::Kind::Tile(t) => Kind::Tile { input: input(t.input()) },
            uf::Kind::Image(_) => return None,
        };
        g.primitives.push(Primitive { result: p.result().to_string(), kind });
    }
    // usvg names every result; a name no later primitive reads is noise in the editor.
    let read: Vec<String> = g.primitives.iter().flat_map(|p| p.kind.inputs().into_iter().cloned()).collect();
    for p in &mut g.primitives {
        if !read.contains(&p.result) {
            p.result.clear();
        }
    }
    Some(g)
}
