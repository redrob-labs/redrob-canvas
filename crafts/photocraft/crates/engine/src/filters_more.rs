//! Fourth filter batch (v0.11 parity): Variable Blur, Apply Lens, Mosaic (tiles), Newsprint,
//! Curve Bend, Bloom, Sobel, Cartoon, Clothify, Stencil Chrome, Bump Map, Panorama Projection,
//! Warp (map), Perlin Noise, Plasma and Solid Noise run through [`crate::filters::run_filter`];
//! this module decodes their parameters, resolves their auxiliary images (maps, environment,
//! brush and paper images, filmstrip frames) and lists their command specs.
//!
//! It also holds the script-like commands of the batch, which drive other commands and edit the
//! document directly: Add Border, Fog, Old Photo, Round Corners, Tile, Depth Merge, Geometric
//! Shapes (star / spiral / arc on a shape layer), and the Perspective Shadow runner that grows
//! the canvas when `allowResizing` is set.

use std::sync::Arc;

use redrob_photo_algo::{self as algo, BumpType, FilterParams, MaskSource, MosaicTile, NewsprintModel, PanoSampler, ScreenPattern, WarpEdge};
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
fn mask_source(p: &Value, k: &str, d: MaskSource) -> MaskSource {
    match p.get(k).and_then(Value::as_str) {
        Some("map") => MaskSource::Map,
        Some("layerMask") => MaskSource::LayerMask,
        Some("self") | Some("selfImage") | Some("alpha") | Some("luminance") => MaskSource::SelfImage,
        _ => d,
    }
}
fn pattern(v: &str) -> ScreenPattern {
    match v {
        "line" => ScreenPattern::Line,
        "diamond" => ScreenPattern::Diamond,
        "psSquare" => ScreenPattern::PsSquare,
        "cross" => ScreenPattern::Cross,
        _ => ScreenPattern::Circle,
    }
}
/// A curve as `[[x, y], …]` (0..1), or the straight edge when absent / malformed.
fn curve(p: &Value, k: &str) -> Vec<[f32; 2]> {
    let pts: Vec<[f32; 2]> = p
        .get(k)
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

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// Ids whose rendering uses the foreground/background colours (Warp's edge colour).
pub(crate) fn uses_colours(id: &str) -> bool {
    id == "filter.distort.warpMap"
}

/// Algorithm parameters for the ids this module owns (dialog units).
pub fn params_for(id: &str, p: &Value) -> Option<FilterParams> {
    Some(match id {
        "filter.blur.variableBlur" => FilterParams::VariableBlur {
            radius: f(p, "radius", 30.0).clamp(0.0, 1000.0),
            linear_mask: b(p, "linearMask", false),
            high_quality: b(p, "highQuality", false),
            invert: b(p, "invertMask", false),
            source: mask_source(p, "mask", MaskSource::Map),
            map: None,
        },
        "filter.distort.applyLens" => FilterParams::ApplyLens {
            refraction: f(p, "refractionIndex", 1.7).clamp(1.0, 100.0),
            keep_surroundings: b(p, "keepSurroundings", true),
            background: colour(p, "background", BLACK),
        },
        "filter.distort.mosaicTiles" => FilterParams::MosaicTiles {
            tile: match s(p, "tileType", "squares") {
                "hexagons" => MosaicTile::Hexagons,
                "octagons" => MosaicTile::Octagons,
                "triangles" => MosaicTile::Triangles,
                _ => MosaicTile::Squares,
            },
            size: f(p, "tileSize", 15.0).clamp(1.0, 1000.0),
            height: f(p, "tileHeight", 4.0).clamp(0.0, 1000.0),
            spacing: f(p, "tileSpacing", 1.0).clamp(0.0, 1000.0),
            neatness: f(p, "tileNeatness", 0.65).clamp(0.0, 1.0),
            allow_splitting: b(p, "allowSplitting", true),
            color_variation: f(p, "colorVariation", 0.2).clamp(0.0, 1.0),
            light_direction: f(p, "lightDirection", 135.0).clamp(0.0, 360.0),
            antialiasing: b(p, "antialiasing", true),
            color_averaging: b(p, "colorAveraging", true),
            joints_color: colour(p, "jointsColor", BLACK),
            light_color: colour(p, "lightColor", WHITE),
            seed: seed(p),
        },
        "filter.distort.newsprint" => FilterParams::Newsprint {
            model: match s(p, "colorModel", "blackOnWhite") {
                "whiteOnBlack" => NewsprintModel::WhiteOnBlack,
                "rgb" => NewsprintModel::Rgb,
                "cmyk" => NewsprintModel::Cmyk,
                _ => NewsprintModel::BlackOnWhite,
            },
            pattern: [pattern(s(p, "pattern", "circle")), pattern(s(p, "pattern2", "circle")), pattern(s(p, "pattern3", "circle")), pattern(s(p, "pattern4", "circle"))],
            period: [
                f(p, "period", 12.0).clamp(1.0, 1000.0),
                f(p, "period2", 12.0).clamp(1.0, 1000.0),
                f(p, "period3", 12.0).clamp(1.0, 1000.0),
                f(p, "period4", 12.0).clamp(1.0, 1000.0),
            ],
            angle: [f(p, "angle", 75.0), f(p, "angle2", 15.0), f(p, "angle3", 0.0), f(p, "angle4", 45.0)],
            black_pullout: f(p, "blackPullout", 1.0).clamp(0.0, 1.0),
            antialiasing: i(p, "antialiasing", 4).clamp(1, 16) as u32,
            turbulence: f(p, "turbulence", 0.0).clamp(0.0, 1.0),
            seed: seed(p),
        },
        "filter.distort.curveBend" => FilterParams::CurveBend {
            upper: curve(p, "upperCurve"),
            lower: curve(p, "lowerCurve"),
            rotation: f(p, "rotation", 0.0).clamp(0.0, 360.0),
            smoothing: b(p, "smoothing", true),
            antialias: b(p, "antialias", true),
        },
        "filter.lightShadow.bloom" => FilterParams::Bloom {
            threshold: f(p, "threshold", 50.0).clamp(0.0, 100.0),
            softness: f(p, "softness", 25.0).clamp(0.0, 100.0),
            radius: f(p, "radius", 10.0).clamp(0.0, 1500.0),
            strength: f(p, "strength", 50.0).clamp(0.0, 1000.0),
            limit_exposure: b(p, "limitExposure", false),
        },
        "filter.stylize.sobel" => FilterParams::Sobel { horizontal: b(p, "horizontal", true), vertical: b(p, "vertical", true), keep_sign: b(p, "keepSign", true) },
        "filter.artistic.cartoon" => FilterParams::Cartoon { mask_radius: f(p, "maskRadius", 7.0).clamp(1.0, 50.0), pct_black: f(p, "pctBlack", 0.2).clamp(0.0, 1.0) },
        "filter.artistic.clothify" => FilterParams::Clothify {
            blur_x: f(p, "blurX", 9.0).clamp(0.0, 100.0),
            blur_y: f(p, "blurY", 9.0).clamp(0.0, 100.0),
            azimuth: f(p, "azimuth", 135.0).clamp(0.0, 360.0),
            elevation: f(p, "elevation", 45.0).clamp(0.0, 90.0),
            depth: f(p, "depth", 3.0).clamp(0.0, 50.0),
            seed: seed(p),
        },
        "filter.decor.stencilChrome" => FilterParams::StencilChrome {
            saturation: f(p, "chromeSaturation", -80.0).clamp(-100.0, 100.0),
            lightness: f(p, "chromeLightness", -47.0).clamp(-100.0, 100.0),
            factor: f(p, "chromeFactor", 0.75).clamp(0.0, 1.0),
            highlight_balance: f(p, "highlightBalance", -30.0).clamp(-100.0, 100.0),
            chrome_balance: f(p, "chromeBalance", 0.0).clamp(-100.0, 100.0),
            map: None,
        },
        "filter.map.bumpMap" => FilterParams::BumpMap {
            kind: match s(p, "type", "linear") {
                "spherical" => BumpType::Spherical,
                "sinusoidal" => BumpType::Sinusoidal,
                _ => BumpType::Linear,
            },
            compensate: b(p, "compensate", true),
            invert: b(p, "invert", false),
            tiled: b(p, "tiled", false),
            azimuth: f(p, "azimuth", 135.0).clamp(0.0, 360.0),
            elevation: f(p, "elevation", 45.0).clamp(0.5, 90.0),
            depth: f(p, "depth", 3.0).clamp(1.0, 65.0),
            offset_x: i(p, "xOffset", 0).clamp(-20000, 20000) as i32,
            offset_y: i(p, "yOffset", 0).clamp(-20000, 20000) as i32,
            waterlevel: f(p, "waterlevel", 0.0).clamp(0.0, 1.0),
            ambient: f(p, "ambient", 0.0).clamp(0.0, 1.0),
            source: mask_source(p, "auxInput", MaskSource::Map),
            map: None,
        },
        "filter.map.panoramaProjection" => FilterParams::PanoramaProjection {
            pan: f(p, "pan", 0.0).clamp(-180.0, 180.0),
            tilt: f(p, "tilt", 0.0).clamp(-180.0, 180.0),
            spin: f(p, "spin", 0.0).clamp(-360.0, 360.0),
            zoom: f(p, "zoom", 100.0).clamp(1.0, 1000.0),
            inverse: b(p, "inverse", false),
            sampler: match s(p, "sampler", "linear") {
                "nearest" => PanoSampler::Nearest,
                "cubic" => PanoSampler::Cubic,
                _ => PanoSampler::Linear,
            },
        },
        "filter.distort.warpMap" => FilterParams::WarpMap {
            step_size: f(p, "stepSize", 10.0).clamp(-1000.0, 1000.0),
            iterations: i(p, "iterations", 5).clamp(1, 100) as u32,
            on_edges: match s(p, "onEdges", "wrap") {
                "smear" => WarpEdge::Smear,
                "black" => WarpEdge::Black,
                "foregroundColor" => WarpEdge::ForegroundColor,
                _ => WarpEdge::Wrap,
            },
            dither: f(p, "dither", 0.0).clamp(0.0, 100.0),
            rotation_angle: f(p, "rotationAngle", 90.0).clamp(0.0, 360.0),
            substeps: i(p, "substeps", 1).clamp(1, 16) as u32,
            edge_color: colour(p, "foreground", BLACK),
            seed: seed(p),
            map: None,
            magnitude: None,
        },
        "filter.render.perlinNoise" => FilterParams::PerlinNoise {
            alpha: f(p, "alpha", 1.2).clamp(0.0, 4.0),
            scale: f(p, "scale", 1.8).clamp(0.0, 20.0),
            z_offset: f(p, "zOffset", -1.0).clamp(-1.0, 8.0),
            iterations: i(p, "iterations", 3).clamp(1, 8) as u32,
        },
        "filter.render.plasma" => FilterParams::Plasma { turbulence: f(p, "turbulence", 1.0).clamp(0.1, 7.0), seed: seed(p) },
        "filter.render.solidNoise" => FilterParams::SolidNoise {
            x_size: f(p, "xSize", 4.0).clamp(0.1, 16.0),
            y_size: f(p, "ySize", 4.0).clamp(0.1, 16.0),
            detail: i(p, "detail", 1).clamp(0, 15) as u32,
            tileable: b(p, "tileable", false),
            turbulent: b(p, "turbulent", false),
            seed: seed(p),
        },
        _ => return None,
    })
}

// ---- auxiliary images ----

/// A path param that is set and not empty.
fn path_of<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str).filter(|v| !v.is_empty())
}

/// Reads the image a `<key>Path` / `<key>Document` (+ `<key>Layer`) pair names: `prefix` is
/// `"map"`, `"magnitude"`, `"paper"`, `"brush"`. `None` when neither is given.
fn aux_image(s: Option<&Session>, p: &Value, prefix: &str, what: &str) -> Result<Option<Arc<algo::Image>>> {
    let layer = p.get(format!("{prefix}Layer")).and_then(Value::as_u64);
    if let Some(path) = path_of(p, &format!("{prefix}Path")) {
        let img = crate::filters_ext::map_from_path(path, layer)?;
        return Ok(img.map(Arc::new));
    }
    let Some(d) = p.get(format!("{prefix}Document")) else { return Ok(None) };
    if d.as_i64() == Some(-1) || d.is_null() {
        return Ok(None);
    }
    let Some(s) = s else { return Ok(None) };
    let docs = s.documents();
    let found = match d {
        Value::String(name) => docs.iter().find(|ds| ds.doc.name == *name || ds.path.as_deref() == Some(name.as_str())),
        other => other.as_u64().and_then(|k| docs.get(k as usize)),
    };
    let ds = found.ok_or_else(|| EngineError::Other(format!("{what} document not found")))?;
    Ok(crate::filters_ext::map_from_document(&ds.doc, layer).map(Arc::new))
}

/// Resolves the auxiliary images of this batch that need the session (open documents) or a
/// file: Variable Blur's / Bump Map's map, Stencil Chrome's environment, Warp's displacement
/// and magnitude maps, Impressionist's paper and brush images, Filmstrip's pictures.
pub(crate) fn resolve(s: &Session, fp: &mut FilterParams, p: &Value) -> Result<()> {
    resolve_with(Some(s), fp, p)
}

/// The session-free part of [`resolve`] (paths only), for smart-filter re-renders.
pub(crate) fn resolve_paths(fp: &mut FilterParams, p: &Value) -> Result<()> {
    resolve_with(None, fp, p)
}

fn resolve_with(s: Option<&Session>, fp: &mut FilterParams, p: &Value) -> Result<()> {
    match fp {
        FilterParams::VariableBlur { source: MaskSource::Map, map, .. } if map.is_none() => {
            *map = aux_image(s, p, "map", "the blur mask")?;
            if map.is_none() && s.is_some() {
                return Err(EngineError::Other("Variable Blur needs a mask: pass `mapPath` or `mapDocument`, or set `mask` to layerMask / alpha".into()));
            }
        }
        FilterParams::BumpMap { source: MaskSource::Map, map, .. } if map.is_none() => {
            *map = aux_image(s, p, "map", "the bump map")?;
            if map.is_none() && s.is_some() {
                return Err(EngineError::Other("Bump Map needs a height map: pass `mapPath` or `mapDocument`, or set `auxInput` to layerMask / self".into()));
            }
        }
        FilterParams::StencilChrome { map, .. } if map.is_none() => *map = aux_image(s, p, "map", "the environment map")?,
        FilterParams::WarpMap { map, magnitude, .. } => {
            if map.is_none() {
                *map = aux_image(s, p, "map", "the displacement map")?;
                if map.is_none() && s.is_some() {
                    return Err(EngineError::Other("Warp needs a displacement map: pass `mapPath` or `mapDocument`".into()));
                }
            }
            if magnitude.is_none() {
                *magnitude = aux_image(s, p, "magnitude", "the magnitude map")?;
            }
        }
        FilterParams::Impressionist { paper_map, brush_map, .. } => {
            if paper_map.is_none() {
                *paper_map = aux_image(s, p, "paper", "the paper")?;
            }
            if brush_map.is_none() {
                *brush_map = aux_image(s, p, "brush", "the brush")?;
            }
        }
        FilterParams::Filmstrip { frames, .. } if frames.is_empty() => {
            // `images`: a list of layer ids (of the active document), document indices / names,
            // or file paths, in strip order.
            let Some(list) = p.get("images").and_then(Value::as_array) else { return Ok(()) };
            for item in list.iter().take(64) {
                let img = match item {
                    Value::Object(o) if o.contains_key("layer") => {
                        let Some(s) = s else { continue };
                        let d = s.active().ok_or(EngineError::NoDocument)?;
                        let id = o.get("layer").and_then(Value::as_u64).ok_or_else(|| EngineError::Other("`images[].layer` must be a layer id".into()))?;
                        crate::filters_ext::map_from_document(&d.doc, Some(id))
                    }
                    Value::Object(o) if o.contains_key("document") => {
                        let Some(s) = s else { continue };
                        let docs = s.documents();
                        let ds = match o.get("document") {
                            Some(Value::String(name)) => docs.iter().find(|ds| ds.doc.name == *name || ds.path.as_deref() == Some(name.as_str())),
                            Some(other) => other.as_u64().and_then(|k| docs.get(k as usize)),
                            None => None,
                        };
                        let ds = ds.ok_or_else(|| EngineError::Other("filmstrip image document not found".into()))?;
                        crate::filters_ext::map_from_document(&ds.doc, o.get("layer").and_then(Value::as_u64))
                    }
                    Value::String(path) => crate::filters_ext::map_from_path(path, None)?,
                    Value::Number(n) => {
                        let Some(s) = s else { continue };
                        let ds = n.as_u64().and_then(|k| s.documents().get(k as usize)).ok_or_else(|| EngineError::Other("filmstrip image document not found".into()))?;
                        crate::filters_ext::map_from_document(&ds.doc, None)
                    }
                    _ => None,
                };
                if let Some(img) = img {
                    frames.push(Arc::new(img));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Resolves inputs read from the target layer itself: the layer mask as Variable Blur's radius
/// mask or Bump Map's height map.
pub(crate) fn resolve_in_layer(fp: &mut FilterParams, layer: &Layer, bounds: Rect) {
    match fp {
        FilterParams::VariableBlur { source: MaskSource::LayerMask, map, .. } | FilterParams::BumpMap { source: MaskSource::LayerMask, map, .. } if map.is_none() => {
            if let Some(m) = &layer.mask {
                *map = Some(Arc::new(algo::Image::read(&m.surface, bounds)));
            }
        }
        _ => {}
    }
}

// ---- script-like commands ----

fn has_pixels(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    match d.doc.layer(id).ok_or("no active layer")?.content {
        LayerContent::Raster(_) => Ok(()),
        ref c => Err(format!("the active layer is {} {} layer, not a pixel layer", c.article(), c.kind_name())),
    }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Straight RGBA → the layer's native pixel.
fn native(fmt: &redrob_photo_color::PixelFormat, c: [f32; 4]) -> Vec<f32> {
    redrob_photo_raster::from_rgba(fmt, c)
}

/// The surface of the active (or `layer`) pixel layer, after growing the canvas by `grow`
/// (left, top, right, bottom) with a transparent extension when any side is non-zero.
fn grow_canvas(s: &mut Session, grow: [i32; 4]) -> Result<()> {
    let [l, t, r, bt] = grow;
    if l <= 0 && t <= 0 && r <= 0 && bt <= 0 {
        return Ok(());
    }
    let (l, t, r, bt) = (l.max(0), t.max(0), r.max(0), bt.max(0));
    // Canvas Size anchors at a corner or edge: grow in two moves when the sides differ.
    let anchor = |dx: i32, dy: i32| match (dx.signum(), dy.signum()) {
        (1, 1) => "topLeft",
        (1, 0) => "left",
        (1, -1) => "bottomLeft",
        (0, 1) => "top",
        (0, -1) => "bottom",
        (-1, 1) => "topRight",
        (-1, 0) => "right",
        (-1, -1) => "bottomRight",
        _ => "center",
    };
    // Right/bottom growth: anchor top-left. Left/top growth: anchor bottom-right.
    if r > 0 || bt > 0 {
        s.execute("image.canvasSize", json!({"width": r, "height": bt, "relative": true, "anchor": anchor(1, 1), "extensionColor": "transparent"}))?;
    }
    if l > 0 || t > 0 {
        s.execute("image.canvasSize", json!({"width": l, "height": t, "relative": true, "anchor": anchor(-1, -1), "extensionColor": "transparent"}))?;
    }
    Ok(())
}

/// Perspective Shadow with `allowResizing`: the canvas grows to hold the shadow before the
/// filter runs, so nothing is clipped (the filter never grows a layer by itself).
fn run_perspective_shadow(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "filter.lightShadow.perspectiveShadow";
    if b(p, "allowResizing", false) {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        let bounds = d.doc.bounds();
        let layer = crate::commands::layer_param(s, p)?;
        let content = d.doc.layer(layer).and_then(|l| l.surface()).map(|sf| sf.content_bounds()).unwrap_or(bounds);
        let height = content.height().max(1) as f32;
        let angle = f(p, "angle", 45.0).clamp(0.0, 180.0).to_radians();
        let len = f(p, "shadowLength", 1.0).clamp(0.1, 24.0);
        let dist = f(p, "relativeDistance", 5.0).clamp(0.1, 24.0) * height;
        let blur = f(p, "blurRadius", 3.0).clamp(0.0, 1024.0);
        // The shadow's far end: object height · length along the ground direction, foreshortened
        // by the viewer distance; plus the blur margin.
        let z = height * len * angle.sin().max(0.05);
        let down = (z * dist / (dist + z)).ceil() as i32 + blur.ceil() as i32 + 2;
        let side = (height * len * angle.cos()).abs().ceil() as i32 + blur.ceil() as i32 + 2;
        let bottom = (content.y1 + down - bounds.y1).max(0);
        let (left, right) = if angle.cos() >= 0.0 { ((bounds.x0 - (content.x0 - side)).max(0), 0) } else { (0, (content.x1 + side - bounds.x1).max(0)) };
        grow_canvas(s, [left, 0, right, bottom])?;
    }
    let mut params = p.clone();
    if let Value::Object(m) = &mut params {
        m.remove("allowResizing");
    }
    crate::filters::run_filter(s, ID, &params)
}

/// Add Border: the canvas grows by `borderX` / `borderY` on each side and a new layer draws a
/// bevelled frame there in `color`, each side shaded by `delta` levels (top lightest, bottom
/// darkest) with mitred corners.
fn add_border(s: &mut Session, p: &Value) -> Result<Value> {
    let bx = i(p, "borderX", 12).clamp(0, 5000) as i32;
    let by = i(p, "borderY", 12).clamp(0, 5000) as i32;
    let color = colour(p, "color", [0.15, 0.2, 0.6, 1.0]);
    let delta = f(p, "delta", 25.0).clamp(0.0, 255.0) / 255.0;
    if bx == 0 && by == 0 {
        return Err(EngineError::Other("Add Border needs a border size".into()));
    }
    grow_canvas(s, [bx, by, bx, by])?;
    let id = s.edit("Add Border", |doc, active| {
        let fmt = doc.pixel_format();
        let bounds = doc.bounds();
        let (w, h) = (bounds.width() as i32, bounds.height() as i32);
        let mut layer = Layer::raster(doc.next_layer_name("Border Layer"), fmt);
        let shade = |k: f32| -> [f32; 4] {
            [(color[0] + k).clamp(0.0, 1.0), (color[1] + k).clamp(0.0, 1.0), (color[2] + k).clamp(0.0, 1.0), 1.0]
        };
        let (top, left, right, bottom) = (shade(delta), shade(delta * 0.5), shade(-delta * 0.5), shade(-delta));
        if let Some(surf) = layer.surface_mut() {
            let n = fmt.channels();
            let mut data = vec![0.0f32; (w * h) as usize * n];
            let (tn, ln, rn, bn) = (native(&fmt, top), native(&fmt, left), native(&fmt, right), native(&fmt, bottom));
            for y in 0..h {
                for x in 0..w {
                    let in_border = x < bx || y < by || x >= w - bx || y >= h - by;
                    if !in_border {
                        continue;
                    }
                    // Which trapezoid: compare the distance to each edge in border units.
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let dl = fx / bx.max(1) as f32;
                    let dr = (w as f32 - fx) / bx.max(1) as f32;
                    let dt = fy / by.max(1) as f32;
                    let db = (h as f32 - fy) / by.max(1) as f32;
                    let m = dl.min(dr).min(dt).min(db);
                    let px = if m == dt {
                        &tn
                    } else if m == db {
                        &bn
                    } else if m == dl {
                        &ln
                    } else {
                        &rn
                    };
                    let at = (y * w + x) as usize * n;
                    data[at..at + n].copy_from_slice(&px[..n]);
                }
            }
            surf.write_region(bounds, &data);
            surf.prune();
        }
        let id = doc.insert_above(doc.top_layer(), layer);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({ "layer": id.0, "borderX": bx, "borderY": by }))
}

/// Fog: a new layer of coloured cloud noise above the active layer, its alpha the noise, at
/// `opacity`; `turbulence` scales the noise detail.
fn fog(sess: &mut Session, p: &Value) -> Result<Value> {
    let name = s(p, "layerName", "Clouds").to_string();
    let color = colour(p, "color", [0.94, 0.94, 0.94, 1.0]);
    let turbulence = f(p, "turbulence", 1.0).clamp(0.0, 10.0);
    let opacity = f(p, "opacity", 100.0).clamp(0.0, 100.0) / 100.0;
    let sd = seed(p);
    let id = sess.edit("Fog", |doc, active| {
        let fmt = doc.pixel_format();
        let bounds = doc.bounds();
        let (w, h) = (bounds.width() as i32, bounds.height() as i32);
        let base = (w.max(h) as f32 / (2.0 + turbulence * 2.0)).clamp(4.0, 1024.0);
        let mut layer = Layer::raster(doc.next_layer_name(&name), fmt);
        layer.opacity = opacity;
        if let Some(surf) = layer.surface_mut() {
            let n = fmt.channels();
            let mut data = vec![0.0f32; (w * h) as usize * n];
            for y in 0..h {
                for x in 0..w {
                    let t = crate::extra_cmds::clouds_value(x as f32, y as f32, base, sd);
                    // More turbulence: a harder, more contrasted cloud edge.
                    let a = ((t - 0.5) * (1.0 + turbulence * 0.5) + 0.5).clamp(0.0, 1.0);
                    let px = native(&fmt, [color[0], color[1], color[2], a]);
                    let at = (y * w + x) as usize * n;
                    data[at..at + n].copy_from_slice(&px[..n]);
                }
            }
            surf.write_region(bounds, &data);
            surf.prune();
        }
        let id = doc.insert_above(*active, layer);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({ "layer": id.0 }))
}

/// Old Photo: defocus (a slight blur), a white fuzzy border, a sepia tone and mottling (grain),
/// each optional, on the layer or on a copy of the document.
fn old_photo(s: &mut Session, p: &Value) -> Result<Value> {
    if b(p, "workOnCopy", true) {
        s.execute("layer.duplicate", json!({"document": "new", "documentName": "Old Photo"}))?;
    }
    let layer = crate::commands::layer_param(s, &Value::Null)?;
    let target = json!(layer.0);
    if b(p, "defocus", true) {
        s.execute("filter.blur.gaussianBlur", json!({"layer": target, "radius": 1.5}))?;
    }
    let border = f(p, "borderSize", 20.0).clamp(0.0, 300.0);
    if border >= 1.0 {
        s.execute("filter.decor.fuzzyBorder", json!({"layer": target, "color": "#ffffff", "borderSize": border, "blurBorder": true, "granularity": 4, "addShadow": false}))?;
    }
    if b(p, "sepia", true) {
        s.execute("filter.adjust.desaturate", json!({"layer": target, "mode": "luminance"}))?;
        s.execute("image.adjustments.photoFilter", json!({"filter": "sepia", "density": 60, "preserveLuminosity": true}))?;
    }
    if b(p, "mottle", false) {
        s.execute("filter.noise.addNoise", json!({"layer": target, "amount": 8, "distribution": "gaussian", "monochromatic": true, "seed": seed(p)}))?;
    }
    Ok(json!({ "layer": layer.0 }))
}

/// Coverage (0..1) of a rounded rectangle `r` with corner radius `radius` at pixel `(x, y)`.
fn rounded_cover(r: Rect, radius: f32, x: i32, y: i32) -> f32 {
    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
    if !r.contains(x, y) {
        return 0.0;
    }
    let rad = radius.clamp(0.0, (r.width().min(r.height()) as f32) / 2.0);
    if rad < 0.5 {
        return 1.0;
    }
    let cx = fx.clamp(r.x0 as f32 + rad, r.x1 as f32 - rad);
    let cy = fy.clamp(r.y0 as f32 + rad, r.y1 as f32 - rad);
    let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
    (rad + 0.5 - d).clamp(0.0, 1.0)
}

/// Round Corners: the image's corners are cut to `edgeRadius`; optionally a drop shadow
/// (offset, blur) under it and a background layer behind, on the document or a copy.
fn round_corners(s: &mut Session, p: &Value) -> Result<Value> {
    let radius = f(p, "edgeRadius", 15.0).clamp(0.0, 4096.0);
    let shadow = b(p, "addShadow", true);
    let (sx, sy) = (i(p, "shadowX", 8).clamp(-4096, 4096) as i32, i(p, "shadowY", 8).clamp(-4096, 4096) as i32);
    let blur = f(p, "blurRadius", 15.0).clamp(0.0, 1024.0);
    let background = b(p, "addBackground", true);
    if b(p, "workOnCopy", true) {
        s.execute("layer.duplicate", json!({"document": "new", "documentName": "Round Corners"}))?;
    }
    s.execute("layer.new.layerFromBackground", json!({"ifPresent": true}))?;
    let image = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    if shadow {
        let m = blur.ceil() as i32 + 1;
        grow_canvas(s, [(m - sx).max(0), (m - sy).max(0), (m + sx).max(0), (m + sy).max(0)])?;
    }
    let bg = s.tools.background;
    let id = s.edit("Round Corners", |doc, active| {
        let layer = active.ok_or_else(|| EngineError::Other("no active layer".into()))?;
        let bounds = doc.bounds();
        let fmt = doc.pixel_format();
        let n = fmt.channels();
        // The image now sits where the (possibly grown) canvas put it: its content bounds.
        let img_rect = doc.layer(layer).and_then(|l| l.surface()).map(|sf| sf.content_bounds()).filter(|r| !r.is_empty()).unwrap_or(image);
        // Cut the corners.
        {
            let l = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?;
            crate::extra_cmds::unlock_background(l);
            let surf = l.surface_mut().ok_or_else(|| EngineError::Other("the active layer has no pixels".into()))?;
            if !surf.format().alpha {
                return Err(EngineError::Other("Round Corners needs a layer that can hold transparency".into()));
            }
            let mut data = surf.read_region(img_rect);
            let w = img_rect.width() as i32;
            for (k, px) in data.chunks_exact_mut(n).enumerate() {
                let (x, y) = (img_rect.x0 + k as i32 % w, img_rect.y0 + k as i32 / w);
                px[n - 1] *= rounded_cover(img_rect, radius, x, y);
            }
            surf.write_region(img_rect, &data);
            surf.prune();
        }
        let mut below = layer;
        if shadow {
            let mut sh = Layer::raster(doc.next_layer_name("Shadow"), fmt);
            if let Some(surf) = sh.surface_mut() {
                let r = img_rect.translate(sx, sy);
                let mut data = vec![0.0f32; (r.width() * r.height()) as usize * n];
                let w = r.width() as i32;
                for (k, px) in data.chunks_exact_mut(n).enumerate() {
                    let (x, y) = (r.x0 + k as i32 % w, r.y0 + k as i32 / w);
                    let a = rounded_cover(r, radius, x, y);
                    px.copy_from_slice(&native(&fmt, [0.0, 0.0, 0.0, a])[..n]);
                }
                surf.write_region(r, &data);
                if blur >= 0.5
                    && let Some(blurred) =
                        crate::filters::apply_filter_to_surface("filter.blur.gaussianBlur", &json!({"radius": blur / 2.0}), surf, bounds, None, bounds)
                {
                    *surf = blurred;
                }
                surf.prune();
            }
            sh.opacity = 0.8;
            below = doc.insert_below(Some(layer), sh);
        }
        if background {
            let mut bgl = Layer::raster(doc.next_layer_name("Background"), fmt);
            if let Some(surf) = bgl.surface_mut() {
                surf.fill_rect(bounds, &native(&fmt, [bg[0], bg[1], bg[2], 1.0]));
            }
            doc.insert_below(Some(below), bgl);
        }
        *active = Some(layer);
        Ok(layer)
    })?;
    Ok(json!({ "layer": id.0 }))
}

/// Tile: the canvas becomes `width` × `height` and the active layer's pixels repeat across it,
/// on this document or on a new one holding a copy.
fn tile(s: &mut Session, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let (ow, oh) = (d.doc.size.width, d.doc.size.height);
    let w = i(p, "width", (ow * 2) as i64).clamp(1, 300_000) as u32;
    let h = i(p, "height", (oh * 2) as i64).clamp(1, 300_000) as u32;
    if b(p, "newImage", true) {
        s.execute("layer.duplicate", json!({"document": "new", "documentName": "Tiled"}))?;
    }
    let layer = crate::commands::layer_param(s, &Value::Null)?;
    let old = Rect::from_xywh(0, 0, ow, oh);
    s.execute("image.canvasSize", json!({"width": w, "height": h, "anchor": "topLeft", "extensionColor": "transparent"}))?;
    s.edit("Tile", |doc, active| {
        let l = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?;
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("the active layer has no pixels".into()))?;
        let src = surf.read_region(old);
        let bounds = doc.bounds();
        let (tw, th) = (old.width() as i32, old.height() as i32);
        let mut y = 0;
        while y < bounds.y1 {
            let mut x = 0;
            while x < bounds.x1 {
                if x != 0 || y != 0 {
                    let dst = Rect::from_xywh(x, y, old.width(), old.height()).intersect(&bounds);
                    if dst.is_empty() {
                        x += tw;
                        continue;
                    }
                    if dst.width() == old.width() && dst.height() == old.height() {
                        surf.write_region(dst, &src);
                    } else {
                        // A partial tile at the right/bottom edge: copy the visible part row by row.
                        let n = surf.channels();
                        let mut part = Vec::with_capacity((dst.width() * dst.height()) as usize * n);
                        for yy in 0..dst.height() as i32 {
                            let row = (yy as usize * tw as usize) * n;
                            part.extend_from_slice(&src[row..row + dst.width() as usize * n]);
                        }
                        surf.write_region(dst, &part);
                    }
                }
                x += tw;
            }
            y += th;
        }
        surf.prune();
        *active = Some(layer);
        Ok(())
    })?;
    Ok(json!({ "layer": layer.0, "width": w, "height": h }))
}

/// An image (a layer of the active document, another document, or a file) named by a param
/// object `{layer}`, `{document, layer?}` or a path string, stretched over `bounds` as RGBA.
fn source_image(s: &Session, v: Option<&Value>, what: &str) -> Result<Option<algo::Image>> {
    let Some(v) = v else { return Ok(None) };
    let img = match v {
        Value::Null => return Ok(None),
        Value::String(path) if !path.is_empty() => crate::filters_ext::map_from_path(path, None)?,
        Value::Object(o) if o.contains_key("layer") && !o.contains_key("document") => {
            let d = s.active().ok_or(EngineError::NoDocument)?;
            crate::filters_ext::map_from_document(&d.doc, o.get("layer").and_then(Value::as_u64))
        }
        Value::Object(o) => {
            let docs = s.documents();
            let ds = match o.get("document") {
                Some(Value::String(name)) => docs.iter().find(|ds| ds.doc.name == *name || ds.path.as_deref() == Some(name.as_str())),
                Some(other) => other.as_u64().and_then(|k| docs.get(k as usize)),
                None => None,
            };
            let ds = ds.ok_or_else(|| EngineError::Other(format!("{what}: document not found")))?;
            crate::filters_ext::map_from_document(&ds.doc, o.get("layer").and_then(Value::as_u64))
        }
        Value::Number(n) => {
            let ds = n.as_u64().and_then(|k| s.documents().get(k as usize)).ok_or_else(|| EngineError::Other(format!("{what}: document not found")))?;
            crate::filters_ext::map_from_document(&ds.doc, None)
        }
        _ => return Err(EngineError::Other(format!("{what} must be {{\"layer\":id}}, {{\"document\":index|name}} or a file path"))),
    };
    Ok(img)
}

/// Bilinear RGBA sample of `img` stretched over `b` at document pixel `(x, y)`.
fn stretched(img: &algo::Image, b: Rect, x: i32, y: i32) -> [f32; 4] {
    let r = img.rect;
    let u = (x - b.x0) as f32 / b.width().max(1) as f32 * r.width() as f32 - 0.5;
    let v = (y - b.y0) as f32 / b.height().max(1) as f32 * r.height() as f32 - 0.5;
    let (x0, y0) = (u.floor(), v.floor());
    let (ax, ay) = (u - x0, v - y0);
    let g = |ix: i32, iy: i32, c: usize| img.get(r.x0 + ix.clamp(0, r.width() as i32 - 1), r.y0 + iy.clamp(0, r.height() as i32 - 1), c.min(img.ch - 1));
    let (x0, y0) = (x0 as i32, y0 as i32);
    let mut out = [0.0f32; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let cc = if img.ch >= 3 { c } else if c == 3 { if img.ch == 2 { 1 } else { usize::MAX } } else { 0 };
        if cc == usize::MAX {
            *o = 1.0;
            continue;
        }
        let top = g(x0, y0, cc) + (g(x0 + 1, y0, cc) - g(x0, y0, cc)) * ax;
        let bot = g(x0, y0 + 1, cc) + (g(x0 + 1, y0 + 1, cc) - g(x0, y0 + 1, cc)) * ax;
        *o = top + (bot - top) * ay;
    }
    if img.ch == 3 {
        out[3] = 1.0;
    }
    out
}

/// Depth Merge: the active layer (source 1) and a second image are combined by their depth
/// maps: where source 1's depth (times `scale1`) is nearer than source 2's (times `scale2`,
/// plus `offset`) it shows, with a soft transition `overlap` wide. Brighter depth is nearer.
fn depth_merge(s: &mut Session, p: &Value) -> Result<Value> {
    let layer = crate::commands::layer_param(s, p)?;
    let bounds = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    let src2 = source_image(s, p.get("source2"), "`source2`")?.ok_or_else(|| EngineError::Other("Depth Merge needs `source2`".into()))?;
    let depth1 = source_image(s, p.get("depthMap1"), "`depthMap1`")?;
    let depth2 = source_image(s, p.get("depthMap2"), "`depthMap2`")?;
    let overlap = f(p, "overlap", 0.0).clamp(0.0, 2.0);
    let offset = f(p, "offset", 0.0).clamp(-1.0, 1.0);
    let scale1 = f(p, "scale1", 1.0).clamp(-1.0, 1.0);
    let scale2 = f(p, "scale2", 1.0).clamp(-1.0, 1.0);
    s.edit("Depth Merge", |doc, active| {
        let fmt = doc.pixel_format();
        let n = fmt.channels();
        let l = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?;
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("the active layer has no pixels".into()))?;
        let mut data = surf.read_region(bounds);
        let w = bounds.width() as i32;
        let luma = |c: [f32; 4]| (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]) * c[3];
        for (k, px) in data.chunks_exact_mut(n).enumerate() {
            let (x, y) = (bounds.x0 + k as i32 % w, bounds.y0 + k as i32 / w);
            let a = redrob_photo_raster::to_rgba(&fmt, px);
            let bb = stretched(&src2, bounds, x, y);
            let z1 = depth1.as_ref().map_or(0.5, |m| luma(stretched(m, bounds, x, y))) * scale1;
            let z2 = depth2.as_ref().map_or(0.5, |m| luma(stretched(m, bounds, x, y))) * scale2 + offset;
            let diff = z1 - z2;
            let t = if overlap <= 1e-4 { if diff >= 0.0 { 1.0 } else { 0.0 } } else { ((diff / overlap) + 0.5).clamp(0.0, 1.0) };
            let t = t * t * (3.0 - 2.0 * t);
            let mix = [bb[0] + (a[0] - bb[0]) * t, bb[1] + (a[1] - bb[1]) * t, bb[2] + (a[2] - bb[2]) * t, bb[3] + (a[3] - bb[3]) * t];
            px.copy_from_slice(&native(&fmt, mix)[..n]);
        }
        surf.write_region(bounds, &data);
        surf.prune();
        *active = Some(layer);
        Ok(())
    })?;
    Ok(json!({ "layer": layer.0 }))
}

/// Geometric Shapes: a line, rectangle, circle, ellipse, arc, polygon, star, spiral or Bézier
/// curve drawn as a shape layer (stroke and/or fill), optionally rasterized onto the layer below.
fn geometric_shape(sess: &mut Session, p: &Value) -> Result<Value> {
    let kind = s(p, "shape", "star").to_string();
    let x = f(p, "x", 50.0) as f64;
    let y = f(p, "y", 50.0) as f64;
    let w = f(p, "width", 200.0).max(1.0) as f64;
    let h = f(p, "height", 200.0).max(1.0) as f64;
    let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
    let fill_on = b(p, "fill", true);
    let stroke_on = b(p, "stroke", true);
    let stroke_w = f(p, "strokeWidth", 2.0).clamp(0.0, 1000.0);
    let fill_c = colour(p, "fillColor", [0.2, 0.5, 0.9, 1.0]);
    let stroke_c = colour(p, "strokeColor", BLACK);
    let hex = |c: [f32; 4]| format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8);
    let mut params = json!({ "name": format!("{} Shape", { let mut c = kind.clone(); if let Some(f) = c.get_mut(0..1) { f.make_ascii_uppercase(); } c }) });
    let obj = params.as_object_mut().expect("object");
    let polyline = |pts: Vec<[f64; 2]>, closed: bool| json!({ "subpaths": [{ "closed": closed, "knots": pts }] });
    match kind.as_str() {
        "line" => {
            obj.insert("kind".into(), json!("line"));
            obj.insert("from".into(), json!([x, y]));
            obj.insert("to".into(), json!([x + w, y + h]));
            obj.insert("weight".into(), json!(stroke_w.max(1.0)));
        }
        "rectangle" => {
            obj.insert("kind".into(), json!("rect"));
            obj.insert("rect".into(), json!([x, y, w, h]));
        }
        "circle" => {
            let r = w.min(h);
            obj.insert("kind".into(), json!("ellipse"));
            obj.insert("rect".into(), json!([cx - r / 2.0, cy - r / 2.0, r, r]));
        }
        "ellipse" => {
            obj.insert("kind".into(), json!("ellipse"));
            obj.insert("rect".into(), json!([x, y, w, h]));
        }
        "polygon" | "star" => {
            obj.insert("kind".into(), json!(kind));
            obj.insert("rect".into(), json!([x, y, w, h]));
            obj.insert("sides".into(), json!(i(p, "sides", 5).clamp(3, 100)));
            if kind == "star" {
                obj.insert("starRatio".into(), json!(f(p, "starRatio", 0.5).clamp(0.01, 1.0)));
            }
        }
        "arc" => {
            // Points along the ellipse from `startAngle` through `sweepAngle` (degrees, clockwise
            // on screen); a pie closes through the centre.
            let a0 = f(p, "startAngle", 0.0) as f64;
            let sweep = f(p, "sweepAngle", 90.0).clamp(-360.0, 360.0) as f64;
            let steps = ((sweep.abs() / 4.0).ceil() as usize).clamp(2, 180);
            let mut pts: Vec<[f64; 2]> = (0..=steps)
                .map(|k| {
                    let a = (a0 + sweep * k as f64 / steps as f64).to_radians();
                    [cx + rx * a.cos(), cy + ry * a.sin()]
                })
                .collect();
            let pie = b(p, "pie", false);
            if pie {
                pts.push([cx, cy]);
            }
            obj.insert("kind".into(), json!("path"));
            obj.insert("path".into(), polyline(pts, pie));
        }
        "spiral" => {
            let turns = f(p, "turns", 3.0).clamp(0.25, 50.0) as f64;
            let ccw = s(p, "direction", "clockwise") == "counterClockwise";
            let inner = f(p, "innerRatio", 0.0).clamp(0.0, 0.95) as f64;
            let steps = ((turns * 36.0).ceil() as usize).clamp(8, 2000);
            let pts: Vec<[f64; 2]> = (0..=steps)
                .map(|k| {
                    let t = k as f64 / steps as f64;
                    let a = t * turns * std::f64::consts::TAU * if ccw { -1.0 } else { 1.0 };
                    let r = inner + (1.0 - inner) * t;
                    [cx + rx * r * a.cos(), cy + ry * r * a.sin()]
                })
                .collect();
            obj.insert("kind".into(), json!("path"));
            obj.insert("path".into(), polyline(pts, false));
        }
        "bezier" => {
            // A smooth S-curve through the box: anchors at the corners with handles a third in.
            let knots = json!([
                { "anchor": [x, y + h], "in": [x, y + h], "out": [x + w / 3.0, y + h], "smooth": true },
                { "anchor": [x + w, y], "in": [x + w - w / 3.0, y], "out": [x + w, y], "smooth": true }
            ]);
            obj.insert("kind".into(), json!("path"));
            obj.insert("path".into(), json!({ "subpaths": [{ "closed": false, "knots": knots }] }));
        }
        other => return Err(EngineError::BadParams { cmd: "filter.render.geometricShapes".into(), msg: format!("unknown shape `{other}`") }),
    }
    let open = matches!(kind.as_str(), "line" | "arc" | "spiral" | "bezier") && !b(p, "pie", false);
    obj.insert("fill".into(), if fill_on && !open { json!(hex(fill_c)) } else { Value::Null });
    obj.insert("stroke".into(), if stroke_on || open { json!({ "width": stroke_w.max(0.5), "color": hex(stroke_c) }) } else { Value::Null });
    let info = sess.execute("shape.create", params)?;
    if b(p, "rasterize", false) {
        sess.execute("layer.rasterize.shape", json!({}))?;
        if sess.is_enabled("layer.mergeDown") {
            sess.execute("layer.mergeDown", json!({}))?;
        }
    }
    Ok(info)
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
        // Blur
        cmd!(
            "filter.blur.variableBlur",
            "Variable Blur…",
            ["Filter", "Blur"],
            r##"{"radius":0..1000=30,"mask":"map|layerMask|alpha"="map","mapDocument":doc,"mapPath":text,"mapLayer":json,"invertMask":bool,"linearMask":bool,"highQuality":bool}"##
        ),
        // Distort
        cmd!(
            "filter.distort.applyLens",
            "Apply Lens…",
            ["Filter", "Distort"],
            r##"{"refractionIndex":1..100=1.7,"keepSurroundings":bool=true,"background":color=#000000}"##
        ),
        cmd!(
            "filter.distort.mosaicTiles",
            "Mosaic Tiles…",
            ["Filter", "Distort"],
            r##"{"tileType":"squares|hexagons|octagons|triangles","tileSize":1..1000=15,"tileHeight":0..1000=4,"tileSpacing":0..1000=1,"tileNeatness":0..1=0.65,"allowSplitting":bool=true,"colorVariation":0..1=0.2,"lightDirection":0..360=135,"antialiasing":bool=true,"colorAveraging":bool=true,"jointsColor":color=#000000,"lightColor":color=#ffffff,"seed":u32=0}"##
        ),
        cmd!(
            "filter.distort.newsprint",
            "Newsprint…",
            ["Filter", "Distort"],
            r##"{"colorModel":"blackOnWhite|whiteOnBlack|rgb|cmyk","pattern":"circle|line|diamond|psSquare|cross","period":1..1000=12,"angle":-180..180=75,"pattern2":"circle|line|diamond|psSquare|cross","period2":1..1000=12,"angle2":-180..180=15,"pattern3":"circle|line|diamond|psSquare|cross","period3":1..1000=12,"angle3":-180..180=0,"pattern4":"circle|line|diamond|psSquare|cross","period4":1..1000=12,"angle4":-180..180=45,"blackPullout":0..1=1,"antialiasing":1..16=4,"turbulence":0..1=0,"seed":u32=0}"##
        ),
        cmd!(
            "filter.distort.curveBend",
            "Curve Bend…",
            ["Filter", "Distort"],
            r##"{"rotation":0..360=0,"smoothing":bool=true,"antialias":bool=true,"upperCurve":json,"lowerCurve":json}"##
        ),
        cmd!(
            "filter.distort.warpMap",
            "Warp (Map)…",
            ["Filter", "Distort"],
            r##"{"stepSize":-1000..1000=10,"iterations":1..100=5,"onEdges":"wrap|smear|black|foregroundColor","dither":0..100=0,"rotationAngle":0..360=90,"substeps":1..16=1,"mapDocument":doc,"mapPath":text,"mapLayer":json,"magnitudeDocument":doc,"magnitudePath":text,"magnitudeLayer":json,"seed":u32=0,"foreground":json}"##
        ),
        // Light and Shadow
        cmd!(
            "filter.lightShadow.bloom",
            "Bloom…",
            ["Filter", "Light and Shadow"],
            r##"{"threshold":0..100=50,"softness":0..100=25,"radius":0..1500=10,"strength":0..1000=50,"limitExposure":bool}"##
        ),
        // Stylize
        cmd!("filter.stylize.sobel", "Sobel…", ["Filter", "Stylize"], r##"{"horizontal":bool=true,"vertical":bool=true,"keepSign":bool=true}"##),
        // Artistic
        cmd!("filter.artistic.cartoon", "Cartoon…", ["Filter", "Artistic"], r##"{"maskRadius":1..50=7,"pctBlack":0..1=0.2}"##),
        cmd!(
            "filter.artistic.clothify",
            "Clothify…",
            ["Filter", "Artistic"],
            r##"{"blurX":0..100=9,"blurY":0..100=9,"azimuth":0..360=135,"elevation":0..90=45,"depth":0..50=3,"seed":u32=0}"##
        ),
        // Decor
        cmd!(
            "filter.decor.stencilChrome",
            "Stencil Chrome…",
            ["Filter", "Decor"],
            r##"{"chromeSaturation":-100..100=-80,"chromeLightness":-100..100=-47,"chromeFactor":0..1=0.75,"highlightBalance":-100..100=-30,"chromeBalance":-100..100=0,"mapDocument":doc,"mapPath":text,"mapLayer":json}"##
        ),
        // Map
        cmd!(
            "filter.map.bumpMap",
            "Bump Map…",
            ["Filter", "Map"],
            r##"{"auxInput":"map|layerMask|self"="map","mapDocument":doc,"mapPath":text,"mapLayer":json,"type":"linear|spherical|sinusoidal","compensate":bool=true,"invert":bool,"tiled":bool,"azimuth":0..360=135,"elevation":0.5..90=45,"depth":1..65=3,"xOffset":px=0,"yOffset":px=0,"waterlevel":0..1=0,"ambient":0..1=0}"##
        ),
        cmd!(
            "filter.map.panoramaProjection",
            "Panorama Projection…",
            ["Filter", "Map"],
            r##"{"pan":-180..180=0,"tilt":-180..180=0,"spin":-360..360=0,"zoom":1..1000=100,"inverse":bool,"sampler":"nearest|linear|cubic"="linear"}"##
        ),
        // Render › Noise
        cmd!(
            "filter.render.perlinNoise",
            "Perlin Noise…",
            ["Filter", "Render", "Noise"],
            r##"{"alpha":0..4=1.2,"scale":0..20=1.8,"zOffset":-1..8=-1,"iterations":1..8=3}"##
        ),
        cmd!("filter.render.plasma", "Plasma…", ["Filter", "Render", "Noise"], r##"{"turbulence":0.1..7=1,"seed":u32=0}"##),
        cmd!(
            "filter.render.solidNoise",
            "Solid Noise…",
            ["Filter", "Render", "Noise"],
            r##"{"xSize":0.1..16=4,"ySize":0.1..16=4,"detail":0..15=1,"tileable":bool,"turbulent":bool,"seed":u32=0}"##
        ),
        // Scripts
        script!(
            "filter.decor.addBorder",
            "Add Border…",
            ["Filter", "Decor"],
            r##"{"borderX":0..5000=12,"borderY":0..5000=12,"color":color=#26339a,"delta":0..255=25}"##,
            has_doc,
            add_border
        ),
        script!(
            "filter.render.fog",
            "Fog…",
            ["Filter", "Render"],
            r##"{"layerName":text,"color":color=#f0f0f0,"turbulence":0..10=1,"opacity":0..100=100,"seed":u32=0}"##,
            has_doc,
            fog
        ),
        script!(
            "filter.decor.oldPhoto",
            "Old Photo…",
            ["Filter", "Decor"],
            r##"{"defocus":bool=true,"borderSize":0..300=20,"sepia":bool=true,"mottle":bool,"workOnCopy":bool=true,"seed":u32=0}"##,
            has_pixels,
            old_photo
        ),
        script!(
            "filter.decor.roundCorners",
            "Round Corners…",
            ["Filter", "Decor"],
            r##"{"edgeRadius":0..4096=15,"addShadow":bool=true,"shadowX":-4096..4096=8,"shadowY":-4096..4096=8,"blurRadius":0..1024=15,"addBackground":bool=true,"workOnCopy":bool=true}"##,
            has_pixels,
            round_corners
        ),
        script!(
            "filter.map.tile",
            "Tile…",
            ["Filter", "Map"],
            r##"{"width":px,"height":px,"newImage":bool=true} (the new canvas size; the layer's pixels repeat across it)"##,
            has_pixels,
            tile
        ),
        script!(
            "filter.combine.depthMerge",
            "Depth Merge…",
            ["Filter", "Combine"],
            r##"{"source2":{"layer":id}|{"document":index|name,"layer":id?}|path,"depthMap1":{"layer":id}|{"document":index|name}|path,"depthMap2":{"layer":id}|{"document":index|name}|path,"overlap":0..2=0,"offset":-1..1=0,"scale1":-1..1=1,"scale2":-1..1=1} (brighter depth is nearer; the result replaces the active layer's pixels)"##,
            has_pixels,
            depth_merge
        ),
        script!(
            "filter.render.geometricShapes",
            "Geometric Shapes…",
            ["Filter", "Render"],
            r##"{"shape":"star|polygon|circle|ellipse|rectangle|line|arc|spiral|bezier","x":px=50,"y":px=50,"width":1..30000=200,"height":1..30000=200,"sides":3..100=5,"starRatio":0.01..1=0.5,"startAngle":-360..360=0,"sweepAngle":-360..360=90,"pie":bool,"turns":0.25..50=3,"direction":"clockwise|counterClockwise","innerRatio":0..0.95=0,"fill":bool=true,"fillColor":color=#3380e6,"stroke":bool=true,"strokeColor":color=#000000,"strokeWidth":0..1000=2,"rasterize":bool}"##,
            has_doc,
            geometric_shape
        ),
    ]
}

/// Perspective Shadow's spec (replaces the plain runner's: `allowResizing` grows the canvas).
pub(crate) fn perspective_shadow_spec() -> CommandSpec {
    CommandSpec {
        id: "filter.lightShadow.perspectiveShadow",
        label: "Perspective Shadow…",
        menu: &["Filter", "Light and Shadow"],
        shortcut: None,
        params: r##"{"angle":0..180=45,"relativeDistance":0.1..24=5,"shadowLength":0.1..24=1,"blurRadius":0..1024=3,"color":color=#000000,"opacity":0..100=80,"interpolation":"none|linear|cubic"="linear","allowResizing":bool}"##,
        enabled: crate::filters::has_filterable_layer,
        run: run_perspective_shadow,
        journal: true,
    }
}
