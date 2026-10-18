//! Fifth filter batch (v0.11 Paint-mode parity): Dodge and Burn filters, Index Colors, the
//! polygon Blur, Edge Detection, the directional Emboss family, Phong Bump Map, Palettize,
//! Cross-Channel Adjustment, Slope / Offset / Power, Raindrops, Maximize / Minimize Channel,
//! Propagate Colors, Height to Normal Map, Mean Removal, Wavelet Noise Reducer, Normalize, Reset
//! Transparent and Random Pick run through [`crate::filters::run_filter`]; this module decodes
//! their parameters (Palettize's palette comes from the Swatches library) and lists their specs.
//!
//! It also holds the external G'MIC filter runner, which hands the layer to a `gmic`
//! command-line executable the user installed (Wavelet Decompose lives in `extra_cmds`).

use redrob_photo_algo::{
    CrossChannel, CrossCurve, EdgeMethod, EdgeOutput, EmbossDirection, FilterParams, HeightChannel, IndexDither, NormalSwizzle, PaletteAlphaMode,
    PaletteColorMode, PaletteDither, PaletteSpace, PhongLight, PolyShape, ToneRange,
};
use redrob_photo_doc::{Layer, LayerContent};
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
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn s<'a>(p: &'a Value, k: &str, d: &'a str) -> &'a str {
    p.get(k).and_then(Value::as_str).unwrap_or(d)
}
fn seed(p: &Value) -> u32 {
    i(p, "seed", 0).clamp(0, u32::MAX as i64) as u32
}
/// Straight sRGB from `[r, g, b(, a)]` (0–1) or `#rrggbb`; `None` when absent or malformed.
fn rgb_of(v: &Value) -> Option<[f32; 3]> {
    match v {
        Value::Array(a) if a.len() >= 3 => {
            let c = |j: usize| a.get(j).and_then(Value::as_f64).map(|v| (v as f32).clamp(0.0, 1.0));
            Some([c(0)?, c(1)?, c(2)?])
        }
        Value::String(h) if h.len() == 7 && h.starts_with('#') => {
            let ch = |j: usize| h.get(j..j + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).map(|v| v as f32 / 255.0);
            Some([ch(1)?, ch(3)?, ch(5)?])
        }
        _ => None,
    }
}
fn rgb(p: &Value, k: &str, d: [f32; 3]) -> [f32; 3] {
    p.get(k).and_then(rgb_of).unwrap_or(d)
}
/// A list of colours (`colors`: hex strings or `[r, g, b]` triples).
fn colour_list(p: &Value, k: &str) -> Vec<[f32; 3]> {
    p.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(rgb_of).take(4096).collect()).unwrap_or_default()
}
fn edge_method(v: &str) -> EdgeMethod {
    match v {
        "prewitt" => EdgeMethod::Prewitt,
        "simple" => EdgeMethod::Simple,
        _ => EdgeMethod::Sobel,
    }
}
fn height_channel(v: &str) -> HeightChannel {
    match v {
        "red" => HeightChannel::Red,
        "green" => HeightChannel::Green,
        "blue" => HeightChannel::Blue,
        "alpha" => HeightChannel::Alpha,
        _ => HeightChannel::Luminance,
    }
}
fn cross_channel(v: &str) -> CrossChannel {
    match v {
        "red" => CrossChannel::Red,
        "green" => CrossChannel::Green,
        "blue" => CrossChannel::Blue,
        "alpha" => CrossChannel::Alpha,
        "saturation" => CrossChannel::Saturation,
        "lightness" => CrossChannel::Lightness,
        _ => CrossChannel::Hue,
    }
}
fn swizzle(v: &str, d: NormalSwizzle) -> NormalSwizzle {
    match v {
        "x+" | "xPlus" => NormalSwizzle::XPlus,
        "x-" | "xMinus" => NormalSwizzle::XMinus,
        "y+" | "yPlus" => NormalSwizzle::YPlus,
        "y-" | "yMinus" => NormalSwizzle::YMinus,
        "z+" | "zPlus" => NormalSwizzle::ZPlus,
        "z-" | "zMinus" => NormalSwizzle::ZMinus,
        _ => d,
    }
}
/// A curve as `[[x, y], …]` (0..1); the flat (no change) curve when absent / malformed.
fn curve(v: Option<&Value>) -> Vec<[f32; 2]> {
    let pts: Vec<[f32; 2]> = v
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let q = e.as_array()?;
                    let x = q.first()?.as_f64()? as f32;
                    let y = q.get(1)?.as_f64()? as f32;
                    (x.is_finite() && y.is_finite()).then_some([x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)])
                })
                .take(64)
                .collect()
        })
        .unwrap_or_default();
    if pts.len() < 2 { vec![[0.0, 0.5], [1.0, 0.5]] } else { pts }
}

/// The default Index Colors ramps: a grey ramp and warm, cool and green ramps (dark to light).
fn default_ramps() -> Vec<Vec<[f32; 3]>> {
    let hex = |h: &str| rgb_of(&Value::String(h.into())).unwrap_or([0.0, 0.0, 0.0]);
    vec![
        vec![hex("#000000"), hex("#555555"), hex("#aaaaaa"), hex("#ffffff")],
        vec![hex("#3a1a0a"), hex("#8a3c14"), hex("#d98a3c"), hex("#ffe0a0")],
        vec![hex("#0a1a3a"), hex("#143c8a"), hex("#3c8ad9"), hex("#a0e0ff")],
        vec![hex("#0a3a1a"), hex("#148a3c"), hex("#3cd98a"), hex("#a0ffe0")],
    ]
}

/// Index Colors ramps from `ramps`: a list of up to 16 ramps, each a list of up to 16 colours.
fn ramps(p: &Value) -> Vec<Vec<[f32; 3]>> {
    let out: Vec<Vec<[f32; 3]>> = p
        .get("ramps")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter().take(16).filter_map(|r| r.as_array().map(|r| r.iter().filter_map(rgb_of).take(16).collect::<Vec<_>>())).filter(|r| !r.is_empty()).collect()
        })
        .unwrap_or_default();
    if out.is_empty() { default_ramps() } else { out }
}

fn phong_light(p: &Value, k: u32, enabled: bool, azimuth: f32) -> PhongLight {
    PhongLight {
        enabled: b(p, &format!("light{k}Enabled"), enabled),
        azimuth: f(p, &format!("light{k}Azimuth"), azimuth).rem_euclid(360.0),
        inclination: f(p, &format!("light{k}Inclination"), 45.0).clamp(0.0, 90.0),
        color: rgb(p, &format!("light{k}Color"), [1.0, 1.0, 1.0]),
    }
}

/// Ids whose rendering uses the foreground/background colours (none in this batch).
pub(crate) fn uses_colours(_id: &str) -> bool {
    false
}

/// Algorithm parameters for the ids this module owns (dialog units).
pub fn params_for(id: &str, p: &Value) -> Option<FilterParams> {
    let range = || match s(p, "range", "midtones") {
        "shadows" => ToneRange::Shadows,
        "highlights" => ToneRange::Highlights,
        _ => ToneRange::Midtones,
    };
    Some(match id {
        "filter.adjust.dodge" => FilterParams::DodgeBurn { burn: false, range: range(), exposure: f(p, "exposure", 50.0).clamp(0.0, 100.0) / 100.0 },
        "filter.adjust.burn" => FilterParams::DodgeBurn { burn: true, range: range(), exposure: f(p, "exposure", 50.0).clamp(0.0, 100.0) / 100.0 },
        "filter.artistic.indexColors" => FilterParams::IndexColors {
            ramps: ramps(p),
            in_between: i(p, "inBetween", 1).clamp(0, 64) as u32,
            between_ramps: i(p, "betweenRamps", 0).clamp(0, 16) as u32,
            dither: match s(p, "dither", "ordered") {
                "none" => IndexDither::None,
                "noise" => IndexDither::Noise,
                _ => IndexDither::Ordered,
            },
            amount: f(p, "ditherAmount", 50.0).clamp(0.0, 100.0) / 100.0,
            alpha_steps: i(p, "alphaSteps", 256).clamp(1, 256) as u32,
            luma_weight: f(p, "lumaWeight", 50.0).clamp(0.0, 100.0) / 100.0,
        },
        "filter.blur.polygonBlur" => FilterParams::PolyBlur {
            width: f(p, "width", 5.0).clamp(0.0, 1000.0),
            height: f(p, "height", 5.0).clamp(0.0, 1000.0),
            strength: f(p, "strength", 100.0).clamp(0.0, 100.0) / 100.0,
            angle: f(p, "angle", 0.0).rem_euclid(360.0),
            shape: match s(p, "shape", "circle") {
                "triangle" => PolyShape::Triangle,
                "quadrilateral" => PolyShape::Quadrilateral,
                "pentagon" => PolyShape::Pentagon,
                "hexagon" => PolyShape::Hexagon,
                "octagon" => PolyShape::Octagon,
                _ => PolyShape::Circle,
            },
        },
        "filter.stylize.edgeDetection" => FilterParams::EdgeDetection {
            method: edge_method(s(p, "method", "sobel")),
            h_radius: i(p, "horizontalRadius", 1).clamp(1, 100) as u32,
            v_radius: i(p, "verticalRadius", 1).clamp(1, 100) as u32,
            output: match s(p, "output", "pythagorean") {
                "xOnly" => EdgeOutput::XOnly,
                "yOnly" => EdgeOutput::YOnly,
                "radians" => EdgeOutput::Radians,
                _ => EdgeOutput::Pythagorean,
            },
            to_alpha: b(p, "writeToAlpha", false),
        },
        "filter.stylize.embossDirectional" => FilterParams::EmbossFamily {
            direction: match s(p, "direction", "allDirections") {
                "horizontal" => EmbossDirection::Horizontal,
                "vertical" => EmbossDirection::Vertical,
                "both" => EmbossDirection::Both,
                "laplacian" => EmbossDirection::Laplacian,
                "variableDepth" => EmbossDirection::VariableDepth,
                _ => EmbossDirection::AllDirections,
            },
            depth: f(p, "depth", 3.0).clamp(1.0, 10.0),
        },
        "filter.map.phongBumpMap" => FilterParams::PhongBumpmap {
            lights: [phong_light(p, 1, true, 135.0), phong_light(p, 2, false, 45.0), phong_light(p, 3, false, 225.0), phong_light(p, 4, false, 315.0)],
            ambient: f(p, "ambient", 20.0).clamp(0.0, 100.0) / 100.0,
            diffuse: f(p, "diffuse", 50.0).clamp(0.0, 100.0) / 100.0,
            specular: f(p, "specular", 50.0).clamp(0.0, 100.0) / 100.0,
            shininess: f(p, "shininess", 40.0).clamp(1.0, 200.0),
            use_normal_map: b(p, "useNormalMap", false),
            height_channel: height_channel(s(p, "heightChannel", "luminance")),
        },
        "filter.map.palettize" => FilterParams::Palettize {
            colors: colour_list(p, "colors"),
            space: if s(p, "colorSpaceModel", "rgb") == "lab" { PaletteSpace::Lab } else { PaletteSpace::Rgb },
            dither: match s(p, "dither", "none") {
                "pattern" => PaletteDither::Pattern,
                "diffusion" => PaletteDither::Diffusion,
                "noise" => PaletteDither::Noise,
                _ => PaletteDither::None,
            },
            threshold: f(p, "threshold", 50.0).clamp(0.0, 100.0) / 100.0,
            color_mode: if s(p, "colorMode", "perChannelOffset") == "nearestColors" {
                PaletteColorMode::NearestColors
            } else {
                PaletteColorMode::PerChannelOffset
            },
            alpha_mode: match s(p, "alphaMode", "clip") {
                "index" => PaletteAlphaMode::Index,
                "dither" => PaletteAlphaMode::Dither,
                "unchanged" => PaletteAlphaMode::Unchanged,
                _ => PaletteAlphaMode::Clip,
            },
            alpha_index: i(p, "alphaIndex", 0).clamp(0, 4095) as u32,
            alpha_threshold: f(p, "alphaThreshold", 50.0).clamp(0.0, 100.0) / 100.0,
        },
        "filter.adjust.crossChannel" => FilterParams::CrossChannelAdjust {
            curves: match p.get("curves").and_then(Value::as_array) {
                Some(list) => list
                    .iter()
                    .take(16)
                    .map(|c| CrossCurve {
                        driven: cross_channel(s(c, "driven", "saturation")),
                        driver: cross_channel(s(c, "driver", "hue")),
                        points: curve(c.get("points")),
                    })
                    .collect(),
                // A single curve given flat: `driven`, `driver`, `points`.
                None => vec![CrossCurve {
                    driven: cross_channel(s(p, "driven", "saturation")),
                    driver: cross_channel(s(p, "driver", "hue")),
                    points: curve(p.get("points")),
                }],
            },
        },
        "filter.adjust.slopeOffsetPower" => FilterParams::SlopeOffsetPower {
            slope: [f(p, "slopeRed", 1.0).clamp(0.0, 10.0), f(p, "slopeGreen", 1.0).clamp(0.0, 10.0), f(p, "slopeBlue", 1.0).clamp(0.0, 10.0)],
            offset: [f(p, "offsetRed", 0.0).clamp(-1.0, 1.0), f(p, "offsetGreen", 0.0).clamp(-1.0, 1.0), f(p, "offsetBlue", 0.0).clamp(-1.0, 1.0)],
            power: [f(p, "powerRed", 1.0).clamp(0.01, 10.0), f(p, "powerGreen", 1.0).clamp(0.01, 10.0), f(p, "powerBlue", 1.0).clamp(0.01, 10.0)],
            saturation: f(p, "saturation", 1.0).clamp(0.0, 4.0),
        },
        "filter.artistic.raindrops" => FilterParams::Raindrops {
            drop_size: f(p, "dropSize", 20.0).clamp(1.0, 200.0),
            number: i(p, "number", 40).clamp(1, 500) as u32,
            fish_eye: f(p, "fishEye", 30.0).clamp(1.0, 100.0),
            seed: seed(p),
        },
        "filter.other.maximizeChannel" => FilterParams::MaximizeChannel,
        "filter.other.minimizeChannel" => FilterParams::MinimizeChannel,
        "filter.other.propagateColors" => FilterParams::PropagateColors { radius: i(p, "radius", 8).clamp(1, 256) as u32 },
        "filter.map.heightToNormalMap" => FilterParams::HeightToNormal {
            method: edge_method(s(p, "method", "sobel")),
            h_radius: i(p, "horizontalRadius", 1).clamp(1, 100) as u32,
            v_radius: i(p, "verticalRadius", 1).clamp(1, 100) as u32,
            channel: height_channel(s(p, "heightChannel", "luminance")),
            swizzle: [
                swizzle(s(p, "redChannel", "x+"), NormalSwizzle::XPlus),
                swizzle(s(p, "greenChannel", "y+"), NormalSwizzle::YPlus),
                swizzle(s(p, "blueChannel", "z+"), NormalSwizzle::ZPlus),
            ],
        },
        "filter.sharpen.meanRemoval" => FilterParams::MeanRemoval,
        "filter.noise.waveletNoiseReducer" => FilterParams::WaveletNoiseReducer { threshold: f(p, "threshold", 25.0).clamp(0.0, 100.0) / 100.0 },
        "filter.map.normalizeNormals" => FilterParams::NormalizeNormals,
        "filter.other.resetTransparent" => FilterParams::ResetTransparent,
        "filter.noise.randomPick" => FilterParams::RandomPick {
            level: f(p, "level", 50.0).clamp(0.0, 100.0) / 100.0,
            window: i(p, "windowSize", 2).clamp(1, 100) as u32,
            opacity: f(p, "opacity", 100.0).clamp(0.0, 100.0) / 100.0,
            seed: seed(p),
        },
        _ => return None,
    })
}

/// Fills in session state the batch reads: Palettize's `palette` (a Swatches group name; the
/// whole library when neither `palette` nor `colors` is given) becomes an explicit `colors` list
/// so the recorded params (journal, smart filters) reproduce the result.
pub(crate) fn prepare(sess: &Session, id: &str, out: &mut Value) {
    if id != "filter.map.palettize" {
        return;
    }
    let Value::Object(m) = out else { return };
    if m.get("colors").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
        return;
    }
    let group = m.get("palette").and_then(Value::as_str).map(str::trim).filter(|g| !g.is_empty());
    let conv = crate::presets::swatches::SwatchConv::new(sess);
    let colors: Vec<Value> = sess
        .presets
        .swatches
        .iter()
        .filter(|g| group.is_none_or(|name| g.name.eq_ignore_ascii_case(name)))
        .flat_map(|g| g.items.iter())
        .map(|sw| json!(crate::presets::swatches::hex(conv.rgb(&sw.color))))
        .take(4096)
        .collect();
    m.insert("colors".into(), Value::Array(colors));
}

// ---- helpers for the document-editing commands ----

fn has_pixels(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    match d.doc.layer(id).ok_or("no active layer")?.content {
        LayerContent::Raster(_) => Ok(()),
        ref c => Err(format!("the active layer is {} {} layer, not a pixel layer", c.article(), c.kind_name())),
    }
}

/// Straight RGBA → the layer's native pixel.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn native(fmt: &redrob_photo_color::PixelFormat, c: [f32; 4]) -> Vec<f32> {
    redrob_photo_raster::from_rgba(fmt, c)
}

/// A new pixel layer named `name` holding the straight RGBA buffer `rgba` at `rect`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn layer_from_rgba(doc: &redrob_photo_doc::Document, name: &str, rect: Rect, rgba: &[f32]) -> Layer {
    let fmt = doc.pixel_format();
    let n = fmt.channels();
    let mut layer = Layer::raster(name, fmt);
    if let Some(surf) = layer.surface_mut() {
        let mut data = vec![0.0f32; (rect.width() * rect.height()) as usize * n];
        for (dst, px) in data.chunks_exact_mut(n).zip(rgba.chunks_exact(4)) {
            dst.copy_from_slice(&native(&fmt, [px[0], px[1], px[2], px[3]])[..n]);
        }
        surf.write_region(rect, &data);
        surf.prune();
    }
    layer
}

// ---- G'MIC ----

#[cfg(not(target_arch = "wasm32"))]
/// Splits a G'MIC command line into arguments: whitespace separates, double or single quotes
/// group, a backslash escapes the next character.
fn split_args(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() || any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if !cur.is_empty() || any {
        out.push(cur);
    }
    out
}

/// The `gmic` executable: the command's `executable`, else Preferences › Plug-Ins › G'MIC
/// executable, else `gmic` on the PATH.
fn gmic_executable(sess: &Session, p: &Value) -> String {
    let own = s(p, "executable", "").trim();
    if !own.is_empty() {
        return own.to_string();
    }
    let pref = sess.prefs().plug_ins.gmic_executable.trim();
    if pref.is_empty() { "gmic".to_string() } else { pref.to_string() }
}

#[cfg(not(target_arch = "wasm32"))]
fn gmic_command(sess: &Session, p: &Value) -> std::process::Command {
    let mut cmd = std::process::Command::new(gmic_executable(sess, p));
    let folder = sess.prefs().plug_ins.gmic_update_folder.trim();
    if !folder.is_empty() {
        // Where the executable looks for its update (`.gmic`) files.
        cmd.env("GMIC_PATH", folder);
    }
    cmd
}

/// `filter.gmic.status`: whether the executable runs, and its version line.
#[cfg(not(target_arch = "wasm32"))]
fn gmic_status(sess: &mut Session, p: &Value) -> Result<Value> {
    let exe = gmic_executable(sess, p);
    let out = gmic_command(sess, p).arg("-version").output();
    Ok(match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let version = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
            json!({ "available": true, "executable": exe, "version": version, "updateFolder": sess.prefs().plug_ins.gmic_update_folder })
        }
        Ok(o) => json!({ "available": false, "executable": exe, "error": String::from_utf8_lossy(&o.stderr).trim() }),
        Err(e) => json!({ "available": false, "executable": exe, "error": e.to_string() }),
    })
}

#[cfg(target_arch = "wasm32")]
fn gmic_status(sess: &mut Session, p: &Value) -> Result<Value> {
    Ok(json!({ "available": false, "executable": gmic_executable(sess, p), "error": "external filters are not available on the web" }))
}

/// `filter.gmic.run`: the active layer (or the flattened document, `input` = allLayers) is
/// written to a PNG, the `command` is run on it by the G'MIC executable, and the result replaces
/// the layer's pixels or lands on a new layer above it (`output` = newLayer).
#[cfg(not(target_arch = "wasm32"))]
fn gmic_run(sess: &mut Session, p: &Value) -> Result<Value> {
    use redrob_photo_color::{ColorMode, SampleType};
    use redrob_photo_doc::Document;
    use redrob_photo_geom::Size;

    let command = s(p, "command", "").trim().to_string();
    if command.is_empty() {
        return Err(EngineError::BadParams { cmd: "filter.gmic.run".into(), msg: "`command` is the G'MIC command line to run, e.g. `fx_sharpen_1 3`".into() });
    }
    let layer = crate::commands::layer_param(sess, p)?;
    let all = s(p, "input", "activeLayer") == "allLayers";
    let new_layer = s(p, "output", "replace") == "newLayer";
    let img = {
        let d = sess.active().ok_or(EngineError::NoDocument)?;
        crate::filters_ext::map_from_document(&d.doc, (!all).then_some(layer.0)).ok_or_else(|| EngineError::Other("the layer has no pixels".into()))?
    };
    // The input as an 8-bit RGBA PNG in a private temporary directory.
    let mut input = Document::new("gmic-input", Size::new(img.rect.width(), img.rect.height()), ColorMode::Rgb, SampleType::U8);
    let origin = img.rect;
    let local = Rect::from_xywh(0, 0, origin.width(), origin.height());
    let in_layer = layer_from_rgba(&input, "Layer", local, &img.data);
    input.layers.push(in_layer);
    let png = redrob_photo_io::export(&input, "input.png", &redrob_photo_io::ExportOptions::default())
        .map_err(|e| EngineError::Other(format!("cannot encode the input: {e}")))?;
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("redrob-gmic-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| EngineError::Other(format!("cannot create a temporary folder: {e}")))?;
    let in_path = dir.join("input.png");
    let out_path = dir.join("output.png");
    let cleanup = |dir: &std::path::Path| {
        let _ = std::fs::remove_dir_all(dir);
    };
    if let Err(e) = std::fs::write(&in_path, &png.bytes) {
        cleanup(&dir);
        return Err(EngineError::Other(format!("cannot write the input: {e}")));
    }
    let mut cmd = gmic_command(sess, p);
    cmd.arg("-v").arg("-1").arg("-input").arg(&in_path);
    for a in split_args(&command) {
        cmd.arg(a);
    }
    cmd.arg("-output").arg(&out_path);
    let exe = gmic_executable(sess, p);
    let run = cmd.output();
    let out = match run {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            cleanup(&dir);
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            return Err(EngineError::Other(format!("G'MIC failed ({}): {}", o.status, if err.is_empty() { "no message".to_string() } else { err })));
        }
        Err(e) => {
            cleanup(&dir);
            return Err(EngineError::Other(format!(
                "cannot run the G'MIC executable `{exe}`: {e}. Install G'MIC and set its path under Preferences › Plug-Ins › G'MIC executable"
            )));
        }
    };
    let bytes = std::fs::read(&out_path);
    cleanup(&dir);
    let bytes = bytes.map_err(|e| EngineError::Other(format!("G'MIC wrote no output: {e}")))?;
    let result = crate::file_cmds::import("output.png", &bytes)?;
    let res = crate::filters_ext::map_from_document(&result, None).ok_or_else(|| EngineError::Other("G'MIC returned an empty image".into()))?;
    let (rw, rh) = (res.rect.width(), res.rect.height());
    let target = Rect::from_xywh(origin.x0, origin.y0, rw, rh);
    let warnings = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let placed = sess.edit("G'MIC", |doc, active| {
        if new_layer {
            let l = layer_from_rgba(doc, &doc.next_layer_name("G'MIC"), target, &res.data);
            let id = doc.insert_above(Some(layer), l);
            *active = Some(id);
            return Ok(id);
        }
        let fmt = doc.pixel_format();
        let l = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?;
        crate::extra_cmds::unlock_background(l);
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("the active layer has no pixels".into()))?;
        let n = fmt.channels();
        let mut fresh = redrob_photo_raster::Surface::new(surf.format());
        let mut data = vec![0.0f32; (rw * rh) as usize * n];
        for (dst, px) in data.chunks_exact_mut(n).zip(res.data.chunks_exact(4)) {
            dst.copy_from_slice(&native(&fmt, [px[0], px[1], px[2], px[3]])[..n]);
        }
        fresh.write_region(target, &data);
        fresh.prune();
        *surf = fresh;
        *active = Some(layer);
        Ok(layer)
    })?;
    Ok(json!({ "layer": placed.0, "width": rw, "height": rh, "command": command, "messages": warnings }))
}

#[cfg(target_arch = "wasm32")]
fn gmic_run(_sess: &mut Session, _p: &Value) -> Result<Value> {
    Err(EngineError::Other("external G'MIC filters are not available on the web".into()))
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
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

macro_rules! script {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

/// The command specs of this batch.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        // Adjust
        cmd!("filter.adjust.dodge", "Dodge…", ["Image", "Adjustments"], r##"{"range":"shadows|midtones|highlights"="midtones","exposure":0..100=50}"##),
        cmd!("filter.adjust.burn", "Burn…", ["Image", "Adjustments"], r##"{"range":"shadows|midtones|highlights"="midtones","exposure":0..100=50}"##),
        cmd!(
            "filter.adjust.crossChannel",
            "Cross-Channel Adjustment…",
            ["Image", "Adjustments"],
            r##"{"driven":"red|green|blue|alpha|hue|saturation|lightness"="saturation","driver":"red|green|blue|alpha|hue|saturation|lightness"="hue","points":json,"curves":json} (curves: [{driven, driver, points:[[x,y],…]}, …]; y 0.5 = no change)"##
        ),
        cmd!(
            "filter.adjust.slopeOffsetPower",
            "Slope, Offset, Power…",
            ["Image", "Adjustments"],
            r##"{"slopeRed":0..10=1,"slopeGreen":0..10=1,"slopeBlue":0..10=1,"offsetRed":-1..1=0,"offsetGreen":-1..1=0,"offsetBlue":-1..1=0,"powerRed":0.01..10=1,"powerGreen":0.01..10=1,"powerBlue":0.01..10=1,"saturation":0..4=1}"##
        ),
        // Artistic
        cmd!(
            "filter.artistic.indexColors",
            "Index Colors…",
            ["Filter", "Artistic"],
            r##"{"ramps":json,"inBetween":0..64=1,"betweenRamps":0..16=0,"dither":"none|ordered|noise"="ordered","ditherAmount":0..100=50,"alphaSteps":1..256=256,"lumaWeight":0..100=50} (ramps: [[dark…light colours], …], up to 16 × 16)"##
        ),
        cmd!(
            "filter.artistic.raindrops",
            "Raindrops…",
            ["Filter", "Artistic"],
            r##"{"dropSize":1..200=20,"number":1..500=40,"fishEye":1..100=30,"seed":u32=0}"##
        ),
        // Blur
        cmd!(
            "filter.blur.polygonBlur",
            "Polygon Blur…",
            ["Filter", "Blur"],
            r##"{"width":0..1000=5,"height":0..1000=5,"strength":0..100=100,"angle":0..360=0,"shape":"circle|triangle|quadrilateral|pentagon|hexagon|octagon"="circle"}"##
        ),
        // Stylize
        cmd!(
            "filter.stylize.edgeDetection",
            "Edge Detection…",
            ["Filter", "Stylize"],
            r##"{"method":"prewitt|sobel|simple"="sobel","horizontalRadius":1..100=1,"verticalRadius":1..100=1,"output":"pythagorean|xOnly|yOnly|radians"="pythagorean","writeToAlpha":bool}"##
        ),
        cmd!(
            "filter.stylize.embossDirectional",
            "Emboss (Directional)…",
            ["Filter", "Stylize"],
            r##"{"direction":"horizontal|vertical|both|allDirections|laplacian|variableDepth"="allDirections","depth":1..10=3}"##
        ),
        // Map
        cmd!(
            "filter.map.phongBumpMap",
            "Phong Bump Map…",
            ["Filter", "Map"],
            r##"{"heightChannel":"red|green|blue|alpha|luminance"="luminance","useNormalMap":bool,"ambient":0..100=20,"diffuse":0..100=50,"specular":0..100=50,"shininess":1..200=40,"light1Enabled":bool=true,"light1Azimuth":0..360=135,"light1Inclination":0..90=45,"light1Color":color=#ffffff,"light2Enabled":bool,"light2Azimuth":0..360=45,"light2Inclination":0..90=45,"light2Color":color=#ffffff,"light3Enabled":bool,"light3Azimuth":0..360=225,"light3Inclination":0..90=45,"light3Color":color=#ffffff,"light4Enabled":bool,"light4Azimuth":0..360=315,"light4Inclination":0..90=45,"light4Color":color=#ffffff}"##
        ),
        cmd!(
            "filter.map.palettize",
            "Palettize…",
            ["Filter", "Map"],
            r##"{"palette":text,"colors":json,"colorSpaceModel":"rgb|lab"="rgb","dither":"none|pattern|diffusion|noise"="none","threshold":0..100=50,"colorMode":"perChannelOffset|nearestColors"="perChannelOffset","alphaMode":"clip|index|dither|unchanged"="clip","alphaIndex":0..4095=0,"alphaThreshold":0..100=50} (palette: a Swatches group name; colors: explicit list; neither = the whole Swatches library)"##
        ),
        cmd!(
            "filter.map.heightToNormalMap",
            "Height to Normal Map…",
            ["Filter", "Map"],
            r##"{"method":"prewitt|sobel|simple"="sobel","horizontalRadius":1..100=1,"verticalRadius":1..100=1,"heightChannel":"red|green|blue|alpha|luminance"="luminance","redChannel":"x+|x-|y+|y-|z+|z-"="x+","greenChannel":"x+|x-|y+|y-|z+|z-"="y+","blueChannel":"x+|x-|y+|y-|z+|z-"="z+"}"##
        ),
        cmd!("filter.map.normalizeNormals", "Normalize Normal Map", ["Filter", "Map"], "{}"),
        // Other
        cmd!("filter.other.maximizeChannel", "Maximize Channel", ["Filter", "Other"], "{}"),
        cmd!("filter.other.minimizeChannel", "Minimize Channel", ["Filter", "Other"], "{}"),
        cmd!("filter.other.propagateColors", "Propagate Colors…", ["Filter", "Other"], r##"{"radius":1..256=8}"##),
        cmd!("filter.other.resetTransparent", "Reset Transparent", ["Filter", "Other"], "{}"),
        // Sharpen / Noise
        cmd!("filter.sharpen.meanRemoval", "Mean Removal", ["Filter", "Sharpen"], "{}"),
        cmd!("filter.noise.waveletNoiseReducer", "Wavelet Noise Reducer…", ["Filter", "Noise"], r##"{"threshold":0..100=25}"##),
        cmd!(
            "filter.noise.randomPick",
            "Random Pick…",
            ["Filter", "Noise"],
            r##"{"level":0..100=50,"windowSize":1..100=2,"opacity":0..100=100,"seed":u32=0}"##
        ),
        // Scripts
        script!(
            "filter.gmic.run",
            "G'MIC Filter…",
            ["Filter", "Other"],
            r##"{"command":text,"input":"activeLayer|allLayers"="activeLayer","output":"replace|newLayer"="replace","executable":text} (the G'MIC command line, e.g. `fx_sharpen_1 3`; the executable defaults to Preferences › Plug-Ins)"##,
            has_pixels,
            gmic_run
        ),
        script!("filter.gmic.status", "G'MIC Status", [], r##"{"executable":text}"##, always, gmic_status),
    ]
}
