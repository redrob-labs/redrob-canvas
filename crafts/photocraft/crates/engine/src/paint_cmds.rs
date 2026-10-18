//! Paint helpers: Paint Bucket and Gradient tool.

use redrob_photo_algo::paint::{GradientShape, bucket_fill_inks, bucket_fill_region_inks, bucket_fill_region_src, bucket_fill_src, paint_gradient};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, blend_from_str};
use crate::presets::gradients;
use crate::{EngineError, Result, Session};

fn f(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn color(v: Option<&Value>, d: [f32; 4]) -> [f32; 4] {
    match v {
        Some(Value::Array(a)) if a.len() >= 3 => {
            let c: Vec<f32> = a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            [c[0], c[1], c[2], c.get(3).copied().unwrap_or(1.0)]
        }
        Some(Value::String(h)) => {
            let h = h.trim_start_matches('#');
            let c = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| f32::from(v) / 255.0);
            match (c(0), c(2), c(4)) {
                (Some(r), Some(g), Some(b)) => [r, g, b, c(6).unwrap_or(1.0)],
                _ => d,
            }
        }
        _ => d,
    }
}
/// A colour param (`#rrggbb[aa]` or `[r, g, b, a?]`), or `None` when it is neither.
pub(crate) fn color_value(v: &Value) -> Option<[f32; 4]> {
    let c = color(Some(v), [f32::NAN; 4]);
    (!c[0].is_nan()).then_some(c)
}

fn point(p: &Value, k: &str) -> Option<(f32, f32)> {
    let a = p.get(k)?.as_array()?;
    Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
}

/// `opacity` (1..100 %) as a coverage in 0.01..=1, clamped like the Magic Eraser's: a value
/// above 1 would overshoot the blend (#4167).
fn opacity(p: &Value, cmd: &str) -> Result<f32> {
    Ok(crate::eraser_cmds::number(p, cmd, "opacity", 100.0, 1.0..=100.0)? as f32 / 100.0)
}

/// The layers the fill reads its boundaries from: `reference` = `current` (the target layer),
/// `all` (every visible layer) or `labels` (the visible layers carrying one of `labels`' colour
/// labels, see `redrob_photo_doc::LabelColor`). `sampleAllLayers` is the old spelling of `all`.
pub(crate) fn sample_reference(s: &Session, p: &Value, cmd: &str) -> Result<(redrob_photo_geom::Rect, Vec<[u8; 4]>)> {
    let reference = match p.get("reference").and_then(Value::as_str) {
        Some(r) => r,
        None if b(p, "sampleAllLayers", false) => "all",
        None => "current",
    };
    match reference {
        "current" => crate::selection_cmds::sample_rgba8(s, false),
        "all" => crate::selection_cmds::sample_rgba8(s, true),
        "labels" => {
            let labels: Vec<redrob_photo_doc::LabelColor> = match p.get("labels") {
                Some(Value::Array(a)) => a
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .and_then(|n| redrob_photo_doc::LabelColor::from_id(n).or_else(|| serde_json::from_value(v.clone()).ok()))
                            .ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: format!("unknown colour label {v}") })
                    })
                    .collect::<Result<_>>()?,
                _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "`reference: labels` needs `labels`: a list of colour label names".into() }),
            };
            let d = s.active().ok_or(EngineError::NoDocument)?;
            let area = d.doc.bounds();
            let mut doc = (*d.doc).clone();
            let hide: Vec<redrob_photo_doc::LayerId> = doc.walk().iter().filter(|(_, _, l)| !labels.contains(&l.label)).map(|(_, _, l)| l.id).collect();
            for id in hide {
                if let Some(l) = doc.layer_mut(id) {
                    l.visible = false;
                }
            }
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            Ok((area, redrob_photo_compose::render(&doc, area).px.iter().map(|p| p.map(q)).collect()))
        }
        other => Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("unknown reference `{other}` (want current|all|labels)") }),
    }
}

/// The fill's `contents` and colour: the foreground (default), the background, or a pattern.
fn bucket_source(s: &Session, p: &Value) -> (&'static str, [f32; 4]) {
    let source = match p.get("contents").or_else(|| p.get("source")).and_then(Value::as_str) {
        Some("background") => "background",
        Some("pattern") => "pattern",
        _ => "foreground",
    };
    let fg = if source == "background" { s.tools.background } else { s.tools.foreground };
    (source, color(p.get("color"), fg))
}

/// Grow, anti-alias and feather a fill region as the params ask (`spread` with its stops,
/// `antiAlias` for a hard-edged bounded region, `feather`).
#[allow(clippy::too_many_arguments)]
fn refine_region(
    region: redrob_photo_algo::selection::Region,
    area: redrob_photo_geom::Rect,
    img: &[[u8; 4]],
    spread: u32,
    stops: (bool, u8, bool, u8),
    antialias: bool,
    feather: f32,
) -> redrob_photo_algo::selection::Region {
    let mut r = region;
    if spread > 0 {
        r = r.grow_until(spread, area, img, stops.0, stops.1, stops.2, stops.3);
    }
    if antialias {
        r = r.antialiased(area);
    }
    if feather > 0.0 {
        r = r.feathered(feather, area);
    }
    r
}

/// Fill a region (or the plain bucket flood) with the params' contents, in one history step.
#[allow(clippy::too_many_arguments)]
fn fill_with(
    s: &mut Session,
    p: &Value,
    label: &str,
    region: Option<redrob_photo_algo::selection::Region>,
    needs_region: bool,
    flood: Option<((i32, i32), f32, bool, bool, redrob_photo_algo::selection::WandOptions)>,
) -> Result<Value> {
    let (source, c) = bucket_source(s, p);
    let opacity = opacity(p, label)?;
    let pattern = if source == "pattern" {
        let pat = crate::pattern_cmds::resolve_param(s, label, p)?;
        let tile = redrob_photo_compose::pattern::Tile::new(&pat)
            .ok_or_else(|| EngineError::BadParams { cmd: label.into(), msg: "the pattern is empty or too large to paint".into() })?;
        let (scale, angle, _, phase) = crate::pattern_cmds::placement(p);
        Some((tile, scale, angle, phase))
    } else {
        None
    };
    // A foreground picked as CMYK fills a CMYK document with the picked inks (#4196).
    let inks = s.tools.picked_cmyk([c[0], c[1], c[2]]).filter(|_| c[3] >= 1.0 && pattern.is_none());
    let history = match label {
        "paint.bucketDrag" => "Drag Fill",
        "paint.encloseFill" => "Enclose and Fill",
        _ => "Paint Bucket",
    };
    let filled = s.edit(history, |doc, active| {
        let area = doc.bounds();
        let sel = doc.selection.clone();
        let (surf, lock) = crate::channel_cmds::target_surface(doc, *active, p)?;
        let ok = if needs_region && region.is_none() {
            false
        } else if let Some((tile, scale, angle, phase)) = &pattern {
            // Render the pattern over the canvas once, then sample it at each filled pixel.
            let place = redrob_photo_compose::pattern::Placement::new(redrob_photo_geom::Rect::EMPTY, false, *phase, *scale, *angle);
            let rendered = redrob_photo_compose::pattern::render(tile, &place, area);
            let w = area.width() as usize;
            let src = |px: i32, py: i32| rendered[(py - area.y0) as usize * w + (px - area.x0) as usize];
            if let Some(region) = &region {
                bucket_fill_region_src(surf, region, opacity, sel.as_ref(), lock, src);
                true
            } else if let Some(((x, y), tol, contiguous, aa, opts)) = flood {
                bucket_fill_src(surf, area, (x, y), tol, contiguous, aa, opacity, sel.as_ref(), lock, src, opts)
            } else {
                false
            }
        } else if let Some(region) = &region {
            bucket_fill_region_inks(surf, region, opacity, sel.as_ref(), lock, |_, _| c, inks);
            true
        } else if let Some(((x, y), tol, contiguous, aa, opts)) = flood {
            bucket_fill_inks(surf, area, (x, y), tol, contiguous, aa, opacity, sel.as_ref(), lock, |_, _| c, inks, opts)
        } else {
            false
        };
        surf.prune();
        Ok(ok)
    })?;
    Ok(json!({ "filled": filled }))
}

/// The grow stops: `(stop at darkest, darkness 0..255, stop at most opaque, alpha 0..255)`.
fn grow_stops(p: &Value, cmd: &str) -> Result<(bool, u8, bool, u8)> {
    let darkness = crate::eraser_cmds::number(p, cmd, "growStopDarkness", 50.0, 0.0..=100.0)? as f32;
    let alpha = crate::eraser_cmds::number(p, cmd, "growStopAlpha", 50.0, 0.0..=100.0)? as f32;
    Ok((b(p, "growStopDarkest", false), (darkness / 100.0 * 255.0).round() as u8, b(p, "growStopOpaque", false), (alpha / 100.0 * 255.0).round() as u8))
}

fn bucket(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.bucket";
    let (x, y) = (f(p, "x", 0.0).floor() as i32, f(p, "y", 0.0).floor() as i32);
    // Fill mode: a contiguous region (the default), every similar colour, or the selection's
    // shape; the old `contiguous` flag still reads.
    let fill_mode = match p.get("fillMode").and_then(Value::as_str) {
        Some(m @ ("contiguous" | "similar" | "selection")) => m,
        Some(other) => return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("unknown fillMode `{other}` (want contiguous|similar|selection)") }),
        None if !b(p, "contiguous", true) => "similar",
        None => "contiguous",
    };
    let (tol, contiguous, aa) = (f(p, "tolerance", 32.0), fill_mode != "similar", b(p, "antiAlias", true));
    let opts = crate::selection_cmds::wand_options(p, CMD)?;
    let pixels = crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::Pixels;
    let reference = p.get("reference").and_then(Value::as_str).unwrap_or(if b(p, "sampleAllLayers", false) { "all" } else { "current" });
    let all = reference != "current" && pixels;
    let line_art = b(p, "lineArt", false);
    // Fill until a boundary colour instead of over similar colours; close gaps in the boundary;
    // spread the filled area outward (stopping at lines when asked); feather the edge.
    let boundary = p.get("boundaryColor").filter(|v| !v.is_null()).map(|v| color(Some(v), [0.0, 0.0, 0.0, 1.0]));
    let close_gap = crate::eraser_cmds::number(p, CMD, "closeGap", 0.0, 0.0..=128.0)? as u32;
    let spread = crate::eraser_cmds::number(p, CMD, "spread", 0.0, 0.0..=64.0)? as u32;
    let feather = crate::eraser_cmds::number(p, CMD, "feather", 0.0, 0.0..=64.0)? as f32;
    let stops = grow_stops(p, CMD)?;
    let bounded = boundary.is_some() || close_gap > 0;
    let selection_shape = fill_mode == "selection";
    let needs_region = all || line_art || bounded || spread > 0 || feather > 0.0 || selection_shape;
    // Other visible layers can define the region, but the fill still writes only to the target.
    // A line-art fill always samples up front: its boundary is the ink, not the target's colours.
    let region = if selection_shape {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        let area = d.doc.bounds();
        let bounds = d.doc.selection.as_ref().map_or(area, |m| m.content_bounds().intersect(&area));
        let region = redrob_photo_algo::selection::Region::full(bounds);
        if feather > 0.0 { region.map(|r| r.feathered(feather, area)) } else { region }
    } else if line_art {
        let threshold = crate::eraser_cmds::number(p, CMD, "lineArtThreshold", 0.92, 0.01..=1.0)? as f32;
        let gap = crate::eraser_cmds::number(p, CMD, "lineArtGap", 16.0, 0.0..=128.0)? as u32;
        let grow = crate::eraser_cmds::number(p, CMD, "lineArtGrow", 2.0, 0.0..=64.0)? as u32;
        let (area, img) = if all { sample_reference(s, p, CMD)? } else { crate::selection_cmds::sample_rgba8(s, false)? };
        let region = redrob_photo_algo::selection::line_art_region(&img, area, (x, y), threshold, gap, grow);
        let region = region.map(|r| refine_region(r, area, &img, 0, stops, false, feather));
        drop(img);
        region
    } else if needs_region {
        let (area, img) = if all { sample_reference(s, p, CMD)? } else { crate::selection_cmds::sample_rgba8(s, false)? };
        let region = if bounded {
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            let boundary8 = boundary.map(|b| b.map(q));
            redrob_photo_algo::selection::boundary_region(&img, area, (x, y), boundary8, tol, close_gap, opts)
        } else {
            redrob_photo_algo::selection::wand_region_with(&img, area, (x, y), tol, contiguous, aa, opts)
        };
        // A bounded region is hard-edged until asked to anti-alias.
        let region = region.map(|r| refine_region(r, area, &img, spread, stops, bounded && aa, feather));
        drop(img);
        region
    } else {
        None
    };
    fill_with(s, p, CMD, region, needs_region, Some(((x, y), tol, contiguous, aa, opts)))
}

/// `paint.bucketDrag`: the Paint Bucket dragged across the canvas fills every region it passes
/// over, each once (any colour, or only colours similar to the first), as one history step.
fn bucket_drag(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.bucketDrag";
    let pts: Vec<(i32, i32)> = match p.get("points") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| {
                let q = v.as_array()?;
                Some((q.first()?.as_f64()?.floor() as i32, q.get(1)?.as_f64()?.floor() as i32))
            })
            .collect(),
        _ => return Err(EngineError::BadParams { cmd: CMD.into(), msg: "missing `points`: a list of [x, y]".into() }),
    };
    if pts.is_empty() {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "`points` is empty".into() });
    }
    let (tol, aa) = (f(p, "tolerance", 32.0), b(p, "antiAlias", true));
    let opts = crate::selection_cmds::wand_options(p, CMD)?;
    let similar = b(p, "similarOnly", false);
    let spread = crate::eraser_cmds::number(p, CMD, "spread", 0.0, 0.0..=64.0)? as u32;
    let feather = crate::eraser_cmds::number(p, CMD, "feather", 0.0, 0.0..=64.0)? as f32;
    let stops = grow_stops(p, CMD)?;
    let (area, img) = sample_reference(s, p, CMD)?;
    let region = redrob_photo_algo::fill_regions::drag_regions(&img, area, &pts, tol, aa, similar, opts)
        .map(|r| refine_region(r, area, &img, spread, stops, false, feather));
    drop(img);
    fill_with(s, p, CMD, region, true, None)
}

/// `paint.encloseFill`: the regions an enclosing shape (a rectangle, an ellipse or a polygon)
/// fully surrounds, chosen by `rule`, filled as one history step.
fn enclose_fill(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.encloseFill";
    use redrob_photo_algo::fill_regions::{EncloseRule, ellipse_mask, enclosed_regions, polygon_mask};
    let (area, img) = sample_reference(s, p, CMD)?;
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let rgba8 = |v: &Value| color(Some(v), [0.0, 0.0, 0.0, 1.0]).map(q);
    let points: Vec<(f32, f32)> = match p.get("points") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| {
                let q = v.as_array()?;
                Some((q.first()?.as_f64()? as f32, q.get(1)?.as_f64()? as f32))
            })
            .collect(),
        _ => Vec::new(),
    };
    let shape = p.get("shape").and_then(Value::as_str).unwrap_or("rectangle");
    let enclosure = match shape {
        "polygon" | "lasso" | "bezier" => {
            polygon_mask(&points, area).ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "a polygon needs at least 3 `points`".into() })?
        }
        "rectangle" | "ellipse" => {
            let (x0, y0, x1, y1) = match (point(p, "from"), point(p, "to")) {
                (Some(a), Some(c)) => (a.0.min(c.0), a.1.min(c.1), a.0.max(c.0), a.1.max(c.1)),
                _ => return Err(EngineError::BadParams { cmd: CMD.into(), msg: "a rectangle or ellipse needs `from` and `to`".into() }),
            };
            if shape == "ellipse" {
                ellipse_mask(x0, y0, x1, y1, area)
            } else {
                polygon_mask(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], area)
                    .ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "an empty rectangle".into() })?
            }
        }
        other => return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("unknown shape `{other}` (want rectangle|ellipse|polygon)") }),
    };
    let rule = match p.get("rule").and_then(Value::as_str).unwrap_or("all") {
        "all" => EncloseRule::All,
        "transparent" => EncloseRule::Transparent,
        "color" => EncloseRule::Color(p.get("ruleColor").map(rgba8).unwrap_or_else(|| s.tools.background.map(q))),
        "surroundedBy" => EncloseRule::SurroundedBy(p.get("ruleColor").map(rgba8).unwrap_or_else(|| s.tools.foreground.map(q))),
        other => return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("unknown rule `{other}` (want all|color|transparent|surroundedBy)") }),
    };
    let tol = f(p, "tolerance", 32.0);
    let aa = b(p, "antiAlias", true);
    let exclude_transparent = b(p, "excludeTransparent", false);
    let exclude_color = p.get("excludeColor").filter(|v| !v.is_null()).map(rgba8);
    let spread = crate::eraser_cmds::number(p, CMD, "spread", 0.0, 0.0..=64.0)? as u32;
    let feather = crate::eraser_cmds::number(p, CMD, "feather", 0.0, 0.0..=64.0)? as f32;
    let stops = grow_stops(p, CMD)?;
    let region = enclosed_regions(&img, area, &enclosure, tol, rule, exclude_transparent, exclude_color, aa)
        .map(|r| refine_region(r, area, &img, spread, stops, false, feather));
    drop(img);
    fill_with(s, p, CMD, region, true, None)
}

fn gradient(s: &mut Session, p: &Value) -> Result<Value> {
    let from = point(p, "from").ok_or_else(|| EngineError::BadParams { cmd: "paint.gradient".into(), msg: "missing from".into() })?;
    let to = point(p, "to").ok_or_else(|| EngineError::BadParams { cmd: "paint.gradient".into(), msg: "missing to".into() })?;
    // `shaped` follows the selection's outline instead of the drag (`None` shape below).
    let style = p.get("style").and_then(Value::as_str).unwrap_or("linear");
    let shape = match style {
        "radial" => Some(GradientShape::Radial),
        "angle" => Some(GradientShape::Angle),
        "reflected" => Some(GradientShape::Reflected),
        "diamond" => Some(GradientShape::Diamond),
        "square" => Some(GradientShape::Square),
        "conicalSymmetric" => Some(GradientShape::ConicalSymmetric),
        "spiralClockwise" => Some(GradientShape::SpiralClockwise),
        "spiralCounterClockwise" => Some(GradientShape::SpiralCounterClockwise),
        "shaped" => None,
        _ => Some(GradientShape::Linear),
    };
    let (fg, bg) = (s.tools.foreground, s.tools.background);
    // A preset (`gradient`), explicit `stops`, or the current gradient (Gradients panel); the
    // legacy `colors` list spaces its colours evenly. Explicit `transparency` applies to all.
    let stops: Vec<(f32, [f32; 4])> = match gradients::tool_stops(s, p)? {
        Some(st) => st,
        None => {
            let colors: Vec<[f32; 4]> =
                p.get("colors").and_then(Value::as_array).map(|a| a.iter().map(|v| color(Some(v), fg)).collect()).unwrap_or_else(|| vec![fg, bg]);
            let n = colors.len();
            let cs = colors.into_iter().enumerate().map(|(i, c)| (if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 }, c)).collect();
            gradients::apply_opacity(cs, &gradients::transparency_param(p, "paint.gradient")?.unwrap_or_default())
        }
    };
    // Photoshop's interpolation (`method`, with `smoothness` 0..=100, default 100) baked into dense
    // stops, as for Gradient Overlay: Photoshop 27.11's Classic black → white is 0.037 at t = 1/16
    // where straight stops give 0.0625.
    let stops = match p.get("method").and_then(Value::as_str).and_then(redrob_photo_io::GradientMethod::from_name) {
        Some(m) if stops.len() >= 2 => {
            let colors = stops.iter().map(|(t, c)| (*t, redrob_photo_color::Color::rgba(c[0], c[1], c[2], c[3]))).collect();
            let mids = vec![0.5; stops.len() - 1];
            redrob_photo_io::bake_gradient(colors, &mids, (f(p, "smoothness", 100.0) / 100.0).clamp(0.0, 1.0), m)
                .into_iter()
                .map(|(t, c)| {
                    let rgb = c.to_rgb();
                    (t, [rgb[0], rgb[1], rgb[2], c.alpha])
                })
                .collect()
        }
        _ => stops,
    };
    let reverse = b(p, "reverse", false);
    let opacity = opacity(p, "paint.gradient")?;
    let blend = p.get("mode").and_then(Value::as_str).and_then(blend_from_str).unwrap_or(redrob_photo_color::BlendMode::Normal);
    let dither = b(p, "dither", true); // Photoshop dithers gradients by default (reduces banding).
    // Repeat (sawtooth / triangular / truncate), a start offset and the shaped styles that
    // follow the selection outline go through the extended painter; the plain drag shapes
    // keep the original path.
    let style = p.get("style").and_then(Value::as_str).unwrap_or("linear");
    let shaped = redrob_photo_algo::paint_shaped::ShapedKind::from_style(style);
    let repeat = match p.get("repeat").and_then(Value::as_str) {
        None => redrob_photo_algo::paint_shaped::GradientRepeat::None,
        Some(r) => redrob_photo_algo::paint_shaped::GradientRepeat::from_name(r)
            .ok_or_else(|| EngineError::BadParams { cmd: "paint.gradient".into(), msg: "`repeat` must be one of none|sawtooth|triangular|truncate".into() })?,
    };
    let offset = crate::eraser_cmds::number(p, "paint.gradient", "offset", 0.0, 0.0..=100.0)? as f32 / 100.0;
    let extended = shaped.is_some() || repeat != redrob_photo_algo::paint_shaped::GradientRepeat::None || offset > 0.0;
    s.edit("Gradient", |doc, active| {
        let sel = doc.selection.clone();
        let area = sel.as_ref().map(|m| m.content_bounds()).filter(|r| !r.is_empty()).unwrap_or_else(|| doc.bounds()).intersect(&doc.bounds());
        let (surf, lock) = crate::channel_cmds::target_surface(doc, *active, p)?;
        let shape = shape.unwrap_or(GradientShape::Linear);
        if extended {
            redrob_photo_algo::paint_shaped::paint_gradient_ext(
                surf,
                area,
                from,
                to,
                shape,
                shaped,
                repeat,
                offset,
                &stops,
                reverse,
                opacity,
                blend,
                dither,
                sel.as_ref(),
                lock,
            );
        } else {
            paint_gradient(surf, area, from, to, shape, &stops, reverse, opacity, blend, dither, sel.as_ref(), lock);
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Paint helper command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "paint.bucket",
            label: "Paint Bucket",
            menu: &[],
            shortcut: None,
            params: r##"{"x":px,"y":px,"tolerance":0..255=32,"fillMode":"contiguous|similar|selection"="contiguous" (selection = fill the selection's shape),"contiguous":bool=true (old spelling of fillMode),"reference":"current|all|labels"="current" (which layers define the region),"labels":["Red","Blue",…]? (with reference=labels),"feather":px=0 (soften the filled edge),"growStopDarkest":bool=false (spread stops at pixels darker than growStopDarkness %=50),"growStopDarkness":%=50,"growStopOpaque":bool=false (spread stops at pixels more opaque than growStopAlpha %=50),"growStopAlpha":%=50,"antiAlias":bool=true,"sampleAllLayers":bool=false,"diagonal":bool=false,"criterion":"composite|red|green|blue|hue|saturation|value|alpha"="composite","lineArt":bool=false (fill the area the ink encloses instead of similar colours),"lineArtThreshold":0.01..1=0.92 (luma under which a pixel is ink),"lineArtGap":px=16 (gaps in the ink up to this are closed),"lineArtGrow":px=2 (the fill goes this far under the ink),"contents":"foreground|background|pattern"="foreground","color":"#rrggbb"=foreground,"pattern":id|name (contents=pattern),"scale":%=100,"angle":deg,"spread":px=0 (grow the filled area outward),"closeGap":px=0 (gaps in the boundary up to this are closed),"boundaryColor":"#rrggbb"? (fill until this colour instead of over similar colours),"opacity":1..100=100,"target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target}"##,
            enabled: crate::commands::has_paintable,
            run: bucket,
            journal: true,
        },
        CommandSpec {
            id: "paint.bucketDrag",
            label: "Drag Fill",
            menu: &[],
            shortcut: None,
            params: r##"{"points":[[x,y],…] (the drag's path),"similarOnly":bool=false (only regions coloured like the first),"tolerance":0..255=32,"antiAlias":bool=true,"reference":"current|all|labels"="current","labels":[…]?,"diagonal":bool=false,"criterion":"composite|…"="composite","spread":px=0,"growStopDarkest":bool=false,"growStopOpaque":bool=false,"feather":px=0,"contents":"foreground|background|pattern"="foreground","color":"#rrggbb"?,"pattern":id|name?,"opacity":1..100=100,"target":"pixels"|"mask"|"quickMask"|{"channel":i}}"##,
            enabled: crate::commands::has_paintable,
            run: bucket_drag,
            journal: true,
        },
        CommandSpec {
            id: "paint.encloseFill",
            label: "Enclose and Fill",
            menu: &[],
            shortcut: None,
            params: r##"{"shape":"rectangle|ellipse|polygon"="rectangle","from":[x,y],"to":[x,y] (rectangle/ellipse),"points":[[x,y],…] (polygon),"rule":"all|color|transparent|surroundedBy"="all","ruleColor":"#rrggbb"? (color: the regions' colour, default background; surroundedBy: the surrounding colour, default foreground),"excludeTransparent":bool=false,"excludeColor":"#rrggbb"?,"tolerance":0..255=32,"antiAlias":bool=true,"reference":"current|all|labels"="current","labels":[…]?,"spread":px=0,"growStopDarkest":bool=false,"growStopOpaque":bool=false,"feather":px=0,"contents":"foreground|background|pattern"="foreground","color":"#rrggbb"?,"pattern":id|name?,"opacity":1..100=100,"target":"pixels"|"mask"|"quickMask"|{"channel":i}}"##,
            enabled: crate::commands::has_paintable,
            run: enclose_fill,
            journal: true,
        },
        CommandSpec {
            id: "paint.gradient",
            label: "Gradient",
            menu: &[],
            shortcut: None,
            params: r##"{"from":[x,y],"to":[x,y],"style":"linear|radial|angle|reflected|diamond|square|conicalSymmetric|spiralClockwise|spiralCounterClockwise|shapedAngular|shapedSpherical|shapedDimpled"="linear" (the shaped styles follow the selection outline),"repeat":"none|sawtooth|triangular|truncate"="none","offset":0..100=0 (% of the drag held at the start colour),"colors":["#rrggbb",…]? (evenly spaced),"gradient":preset name?,"stops":[[t,"#rrggbb"|"foreground"|"background"],…]?,"transparency":[[t,0..100],…]? (default: the current gradient, see gradient.presets.select),"method":"classic|perceptual|linear|smooth"? (Photoshop's interpolation; omitted: straight stops),"smoothness":0..100=100,"reverse":bool=false,"dither":bool=true,"opacity":1..100=100,"mode":"normal|multiply|…"="normal","target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target}"##,
            enabled: crate::commands::has_paintable,
            run: gradient,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use redrob_photo_geom::Rect;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
        s
    }

    fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
    }

    /// A visible wall on the Background with an empty, active layer above it.
    fn bucket_layers(depth: u32) -> (Session, redrob_photo_doc::LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10, "depth": depth, "background": "white"})).unwrap();
        let bottom = s.active().unwrap().active_layer.unwrap();
        s.edit("wall", |doc, _| {
            doc.layer_mut(bottom).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 0, 11, 10), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        (s, bottom)
    }

    #[test]
    fn bucket_sample_all_layers_uses_visible_boundaries_at_all_depths() {
        for depth in [8, 16, 32] {
            for all in [None, Some(false), Some(true)] {
                let (mut s, bottom) = bucket_layers(depth);
                let before = s.active().unwrap().doc.layer(bottom).unwrap().clone();
                let mut p = json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false, "tolerance": 0});
                if let Some(all) = all {
                    p["sampleAllLayers"] = json!(all);
                }
                assert_eq!(s.execute("paint.bucket", p.clone()).unwrap()["filled"], true);
                assert_eq!(px(&s, 5, 5), vec![1.0, 0.0, 0.0, 1.0], "depth {depth}, all {all:?}");
                let beyond = if all == Some(true) { vec![0.0; 4] } else { vec![1.0, 0.0, 0.0, 1.0] };
                assert_eq!(px(&s, 15, 5), beyond, "depth {depth}, all {all:?}: sample flag controls the boundary");
                assert_eq!(s.active().unwrap().doc.layer(bottom).unwrap(), &before, "only the active layer is written");
                if all == Some(true) {
                    assert_eq!(px(&s, 10, 5), vec![0.0; 4], "wall itself is untouched");
                    assert!(s.undo());
                    p["contiguous"] = json!(false);
                    s.execute("paint.bucket", p).unwrap();
                    assert_eq!(px(&s, 15, 5), vec![1.0, 0.0, 0.0, 1.0], "noncontiguous finds both white regions");
                    assert_eq!(px(&s, 10, 5), vec![0.0; 4]);
                    assert!(s.undo());
                    assert_eq!(s.execute("paint.bucket", json!({"x": -1, "y": 2, "sampleAllLayers": true})).unwrap()["filled"], false);
                    assert_eq!(px(&s, 5, 5), vec![0.0; 4], "an off-canvas sample does not fill the layer");
                }
            }
        }
    }

    #[test]
    fn bucket_sample_all_layers_ignores_hidden_layers() {
        let (mut s, _) = bucket_layers(8);
        s.edit("hidden wall", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            l.surface_mut().unwrap().fill_rect(Rect::new(4, 0, 5, 10), &[0.0, 0.0, 0.0, 1.0]);
            l.visible = false;
            Ok(())
        })
        .unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false, "tolerance": 0, "sampleAllLayers": true})).unwrap();
        assert_eq!(px(&s, 4, 5), vec![1.0, 0.0, 0.0, 1.0], "hidden wall does not bound the fill");
        assert_eq!(px(&s, 8, 5), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(px(&s, 15, 5), vec![0.0; 4], "visible wall still bounds the fill");
    }

    #[test]
    fn bucket_sample_all_layers_keeps_selection_opacity_and_history() {
        for depth in [8, 16, 32] {
            let (mut s, _) = bucket_layers(depth);
            s.execute("select.rect", json!({"x": 0, "y": 2, "width": 20, "height": 6})).unwrap();
            s.edit("feather", |doc, _| {
                doc.selection.as_mut().unwrap().fill_rect(Rect::new(0, 2, 4, 8), &[0.5]);
                Ok(())
            })
            .unwrap();
            let selection = s.active().unwrap().doc.selection.clone();
            s.execute("paint.bucket", json!({"x": 2, "y": 5, "color": "#ff0000", "antiAlias": false, "tolerance": 0, "opacity": 50, "sampleAllLayers": true}))
                .unwrap();
            let (feathered, full) = (px(&s, 2, 5), px(&s, 6, 5));
            assert!((feathered[3] - 0.25).abs() < 0.005 && (full[3] - 0.5).abs() < 0.005, "depth {depth}: selection coverage multiplies opacity");
            assert_eq!(full[..3], [1.0, 0.0, 0.0]);
            assert_eq!(px(&s, 6, 1), vec![0.0; 4], "outside selection");
            assert_eq!(px(&s, 15, 5), vec![0.0; 4], "outside sampled region");
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Paint Bucket"));
            assert!(s.undo());
            assert_eq!(px(&s, 2, 5), vec![0.0; 4]);
            assert_eq!(s.active().unwrap().doc.selection, selection);
            assert!(s.redo());
            assert_eq!(px(&s, 2, 5), feathered);
            assert_eq!(px(&s, 6, 5), full);
            assert_eq!(s.active().unwrap().doc.selection, selection);
        }
    }

    #[test]
    fn bucket_sample_all_layers_pattern_uses_the_same_region() {
        let (mut s, _) = bucket_layers(8);
        s.execute(
            "paint.bucket",
            json!({"x": 2, "y": 2, "contents": "pattern", "pattern": "Diagonal Lines", "antiAlias": false, "tolerance": 0, "sampleAllLayers": true}),
        )
        .unwrap();
        let left: Vec<f32> = (0..10).map(|x| px(&s, x, 0)[0]).collect();
        assert!(left.iter().any(|&r| r < 0.3) && left.iter().any(|&r| r > 0.9), "pattern varies inside the region: {left:?}");
        assert_eq!(px(&s, 10, 5), vec![0.0; 4], "wall untouched");
        assert_eq!(px(&s, 15, 5), vec![0.0; 4], "region past the wall untouched");
    }

    #[test]
    fn bucket_sample_all_layers_mask_and_channel_targets_keep_their_own_boundaries() {
        for target in [json!("mask"), json!({"channel": 0}), json!("quickMask")] {
            for all in [false, true] {
                let (mut s, _) = bucket_layers(8);
                match crate::channel_cmds::target_of(&json!({"target": target})) {
                    crate::channel_cmds::Target::Mask => {
                        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                    }
                    crate::channel_cmds::Target::Alpha(_) => {
                        s.execute("channel.new", json!({"fill": "white"})).unwrap();
                    }
                    crate::channel_cmds::Target::QuickMask => {
                        s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
                    }
                    _ => panic!("expected a grayscale target"),
                }
                let p = json!({"x": 2, "y": 2, "color": "#000000", "antiAlias": false, "tolerance": 0, "target": target, "sampleAllLayers": all});
                s.edit("target wall", |doc, active| {
                    let area = doc.bounds();
                    let (surf, _) = crate::channel_cmds::target_surface(doc, *active, &p).unwrap();
                    surf.fill_rect(area, &[1.0]);
                    surf.fill_rect(Rect::new(5, 0, 6, 10), &[0.0]);
                    Ok(())
                })
                .unwrap();
                s.execute("paint.bucket", p.clone()).unwrap();
                let d = &s.active().unwrap().doc;
                let surf = match crate::channel_cmds::target_of(&p) {
                    crate::channel_cmds::Target::Mask => &d.layer(s.active().unwrap().active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface,
                    crate::channel_cmds::Target::Alpha(i) => &d.channels[i].surface,
                    crate::channel_cmds::Target::QuickMask => &d.quick_mask.as_ref().unwrap().surface,
                    _ => panic!("expected a grayscale target"),
                };
                assert_eq!(surf.sample_channel(2, 5, 0), 0.0, "{target}, all {all}: filled target");
                assert_eq!(surf.sample_channel(7, 5, 0), 1.0, "{target}, all {all}: stopped at target wall");
                assert_eq!(px(&s, 2, 5), vec![0.0; 4], "layer pixels remain untouched");
            }
        }
    }

    #[test]
    fn bucket_fill_contiguous() {
        let mut s = session();
        s.edit("wall", |doc, _| {
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(10, 0, 11, 10), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false})).unwrap();
        assert_eq!(px(&s, 5, 5), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(px(&s, 15, 5), vec![1.0, 1.0, 1.0, 1.0]);
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#0000ff", "contiguous": false, "antiAlias": false, "tolerance": 0})).unwrap();
        assert_eq!(px(&s, 5, 5), vec![0.0, 0.0, 1.0, 1.0]);
    }

    /// #4196: the Paint Bucket and Edit > Stroke write a CMYK pick as its inks into a CMYK document,
    /// not as its sRGB stand-in converted back (100/0/0/0 cyan would come out as about 100/28/0/17).
    #[test]
    fn bucket_and_stroke_write_a_cmyk_pick_as_its_inks() {
        let cyan = [1.0f32, 0.0, 0.0, 0.0];
        let mut s = session();
        s.execute("image.mode.cmyk", json!({})).unwrap();
        let rgb = redrob_photo_color::convert::cmyk_to_rgb(cyan).map(|c| c.clamp(0.0, 1.0));
        s.execute("tools.setColors", json!({"foreground": rgb, "foregroundCmyk": cyan})).unwrap();
        let inks_at = |s: &Session, x: i32, y: i32| -> Vec<f32> {
            let st = s.active().unwrap();
            let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
            surf.read_region(Rect::new(x, y, x + 1, y + 1)).into_iter().take(4).collect()
        };
        let is_cyan = |got: &[f32]| got.iter().zip(cyan).all(|(a, b)| (a - b).abs() < 0.02);
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "antiAlias": false})).unwrap();
        let got = inks_at(&s, 5, 5);
        assert!(is_cyan(&got), "bucket: {got:?}");
        s.execute("select.all", json!({})).unwrap();
        s.execute("edit.stroke", json!({"width": 2, "location": "inside", "color": rgb})).unwrap();
        let got = inks_at(&s, 0, 5);
        assert!(is_cyan(&got), "stroke: {got:?}");
    }

    #[test]
    fn bucket_fills_with_a_pattern() {
        // Paint Bucket with contents="pattern" flood-fills the region with a pattern, not a colour.
        let mut s = session();
        s.edit("wall", |doc, _| {
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(10, 0, 11, 10), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "contents": "pattern", "pattern": "Diagonal Lines", "antiAlias": false, "tolerance": 0})).unwrap();
        // The filled region (left of the wall) now carries the pattern's light/dark variation,
        // while the wall and the region past it are untouched.
        let left: Vec<[f32; 4]> = (0..10)
            .map(|x| {
                let p = px(&s, x, 0);
                [p[0], p[1], p[2], p[3]]
            })
            .collect();
        assert!(left.iter().any(|p| p[0] < 0.3) && left.iter().any(|p| p[0] > 0.9), "pattern varies in the fill: {left:?}");
        assert_eq!(px(&s, 15, 5), vec![1.0, 1.0, 1.0, 1.0], "region past the wall untouched");
    }

    #[test]
    fn gradient_tool_with_selection_and_reverse() {
        let mut s = session();
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [20, 0], "colors": ["#000000", "#ffffff"]})).unwrap();
        assert!(px(&s, 0, 5)[0] < 0.05 && px(&s, 19, 5)[0] > 0.95);
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [20, 0], "colors": ["#000000", "#ffffff"], "reverse": true})).unwrap();
        assert!(px(&s, 0, 5)[0] > 0.95);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 5, "height": 10})).unwrap();
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [0, 10], "colors": ["#ff0000"]})).unwrap();
        assert_eq!(px(&s, 2, 5)[..3], [1.0, 0.0, 0.0]);
        assert!(px(&s, 10, 5)[1] > 0.4, "outside the selection untouched");
        assert!(s.execute("paint.gradient", json!({"from": [0, 0]})).is_err());
    }

    #[test]
    fn gradient_dither_breaks_banding() {
        // A flat gradient (same colour both ends) is perfectly smooth with dither off, and gains
        // per-pixel noise with dither on (which breaks 8-bit banding).
        let run = |dither: bool| {
            let mut s = session();
            s.execute("paint.gradient", json!({"from": [0, 0], "to": [19, 0], "colors": [[0.5, 0.5, 0.5, 1.0], [0.5, 0.5, 0.5, 1.0]], "dither": dither}))
                .unwrap();
            (0..19).map(|x| px(&s, x, 5)[0]).collect::<Vec<f32>>()
        };
        let smooth = run(false);
        assert!(smooth.iter().all(|&v| v == smooth[0]), "no dither: perfectly flat");
        let noisy = run(true);
        assert!(noisy.windows(2).any(|w| w[0] != w[1]), "dither: per-pixel variation");
    }

    /// The two commands under test, each painting #808080 over a white document.
    fn grey_fill(cmd: &str, opacity: Value) -> Value {
        match cmd {
            "paint.bucket" => json!({"x": 2, "y": 2, "color": "#808080", "opacity": opacity}),
            _ => {
                json!({"from": [0, 0], "to": [19, 0], "colors": ["#808080", "#808080"], "dither": false, "opacity": opacity})
            }
        }
    }

    fn white(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10, "depth": depth, "background": "white"})).unwrap();
        s
    }

    /// Issue #4167: an opacity above 100 overshot the blend (200 painted near-black on white).
    #[test]
    fn bucket_and_gradient_clamp_opacity_like_the_magic_eraser() {
        for cmd in ["paint.bucket", "paint.gradient"] {
            for depth in [8, 16, 32] {
                let mut full = white(depth);
                full.execute(cmd, grey_fill(cmd, json!(100))).unwrap();
                let expected = px(&full, 5, 5);
                assert!((expected[0] - 128.0 / 255.0).abs() < 0.01, "{cmd} depth {depth}: 100% paints #808080, got {expected:?}");
                for over in [200.0, 1e9] {
                    let mut s = white(depth);
                    s.execute(cmd, grey_fill(cmd, json!(over))).unwrap();
                    assert_eq!(px(&s, 5, 5), expected, "{cmd} depth {depth}: opacity {over} clamps to 100");
                }
                let (mut low, mut one) = (white(depth), white(depth));
                low.execute(cmd, grey_fill(cmd, json!(-50))).unwrap();
                one.execute(cmd, grey_fill(cmd, json!(1))).unwrap();
                assert_eq!(px(&low, 5, 5), px(&one, 5, 5), "{cmd} depth {depth}: opacity below 1 clamps to 1");
                assert!(px(&low, 5, 5).iter().all(|v| (0.0..=1.0).contains(v)), "{cmd} depth {depth}: no overshoot");
            }
        }
    }

    #[test]
    fn bucket_and_gradient_reject_non_numeric_opacity_without_editing() {
        for cmd in ["paint.bucket", "paint.gradient"] {
            for bad in [json!("50"), json!(true), json!([50]), json!({"v": 50})] {
                let mut s = white(8);
                let (before, steps) = (px(&s, 5, 5), s.active().unwrap().history.past_len());
                let err = s.execute(cmd, grey_fill(cmd, bad.clone())).unwrap_err();
                assert!(matches!(err, EngineError::BadParams { .. }), "{cmd} {bad}: {err:?}");
                assert_eq!(px(&s, 5, 5), before, "{cmd} {bad}: pixels unchanged");
                assert_eq!(s.active().unwrap().history.past_len(), steps, "{cmd} {bad}: no history step");
            }
            // An explicit null keeps the default (100%), like the Magic Eraser.
            let mut s = white(8);
            s.execute(cmd, grey_fill(cmd, Value::Null)).unwrap();
            assert!((px(&s, 5, 5)[0] - 128.0 / 255.0).abs() < 0.01, "{cmd}: null opacity is 100%");
        }
    }

    #[test]
    fn gradient_repeat_modes_and_shaped_gradient() {
        let mut s = session();
        // Forward repeat over a 5 px drag: the colour at x = 7.5 equals the one at x = 2.5.
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [5, 0], "colors": ["#000000", "#ffffff"], "repeat": "sawtooth", "dither": false})).unwrap();
        let (a, b) = (px(&s, 2, 0), px(&s, 7, 0));
        assert!((a[0] - b[0]).abs() < 0.02, "{a:?} {b:?}");
        assert!(px(&s, 4, 0)[0] > 0.8 && px(&s, 5, 0)[0] < 0.2, "the sawtooth restarts");
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [5, 0], "colors": ["#000000", "#ffffff"], "repeat": "triangular", "dither": false})).unwrap();
        assert!(px(&s, 9, 0)[0] < 0.2 && px(&s, 5, 0)[0] > 0.8, "the triangle wave turns back");
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.execute("paint.gradient", json!({"from": [0, 0], "to": [5, 0], "repeat": "bounce"})).is_err());
        // Shaped: inside a rectangular selection the edge is the first colour, the middle the last.
        s.execute("select.rect", json!({"x": 2, "y": 1, "width": 11, "height": 7})).unwrap();
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [5, 0], "style": "shapedSpherical", "colors": ["#ff0000", "#0000ff"], "dither": false}))
            .unwrap();
        let (edge, middle, outside) = (px(&s, 2, 4), px(&s, 7, 4), px(&s, 0, 4));
        assert!(edge[0] > 0.9 && edge[2] < 0.1, "{edge:?}");
        assert!(middle[2] > 0.5 && middle[0] < 0.5 && middle[2] > edge[2], "{middle:?}");
        assert_eq!(outside[3], 0.0);
    }

    #[test]
    fn selection_fill_mode_feather_and_grow_stops() {
        let (mut s, _) = bucket_layers(8);
        // The selection's shape: fills only inside the marquee, ignoring the wall's regions.
        s.execute("select.rect", json!({"x": 2, "y": 2, "width": 12, "height": 5})).unwrap();
        s.execute("paint.bucket", json!({"x": 0, "y": 0, "fillMode": "selection", "color": "#ff0000", "antiAlias": false})).unwrap();
        assert_eq!(px(&s, 3, 3), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(px(&s, 12, 3), vec![1.0, 0.0, 0.0, 1.0], "the selection crosses the wall");
        assert_eq!(px(&s, 0, 0)[3], 0.0);
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        // A feathered contiguous fill softens its edge at the wall.
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false, "tolerance": 0, "sampleAllLayers": true, "feather": 1.5}))
            .unwrap();
        let (inside, edge) = (px(&s, 5, 5), px(&s, 9, 5));
        assert!(inside[3] > 0.9, "{inside:?}");
        assert!(edge[3] > 0.0 && edge[3] < inside[3], "{edge:?}");
        s.execute("edit.undo", json!({})).unwrap();
        // Spread stopping at the darkest pixels does not cross the black wall; a plain spread does.
        s.execute(
            "paint.bucket",
            json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false, "tolerance": 0, "sampleAllLayers": true, "spread": 3, "growStopDarkest": true}),
        )
        .unwrap();
        assert_eq!(px(&s, 10, 5)[3], 0.0, "the wall is darker than the stop");
        assert_eq!(px(&s, 12, 5)[3], 0.0);
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false, "tolerance": 0, "sampleAllLayers": true, "spread": 3}))
            .unwrap();
        assert_eq!(px(&s, 12, 5)[3], 1.0);
        assert!(s.execute("paint.bucket", json!({"x": 2, "y": 2, "fillMode": "everything"})).is_err());
    }

    #[test]
    fn colour_label_reference_drag_fill_and_enclose_fill() {
        let (mut s, bottom) = bucket_layers(8);
        // Labelled red, the wall layer is a reference; without the label it is not.
        s.execute("layer.setLabelColor", json!({"layer": bottom.0, "color": "red"})).unwrap();
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#0000ff", "antiAlias": false, "tolerance": 0, "reference": "labels", "labels": ["red"]}))
            .unwrap();
        assert_eq!(px(&s, 15, 5)[3], 0.0, "the labelled wall bounds the fill");
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#0000ff", "antiAlias": false, "tolerance": 0, "reference": "labels", "labels": ["blue"]}))
            .unwrap();
        assert_eq!(px(&s, 15, 5)[3], 1.0, "no referenced layer: the empty layer is one region");
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.execute("paint.bucket", json!({"x": 2, "y": 2, "reference": "labels"})).is_err(), "labels required");
        // Drag fill over both cells fills both in one history step.
        let steps = s.active().unwrap().history.past_len();
        s.execute("paint.bucketDrag", json!({"points": [[2, 5], [5, 5], [15, 5]], "color": "#00ff00", "antiAlias": false, "tolerance": 0, "reference": "all"}))
            .unwrap();
        assert_eq!(px(&s, 2, 5), vec![0.0, 1.0, 0.0, 1.0]);
        assert_eq!(px(&s, 15, 5), vec![0.0, 1.0, 0.0, 1.0]);
        assert_eq!(px(&s, 10, 5)[3], 0.0, "the wall column stays");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        s.execute("edit.undo", json!({})).unwrap();
        // Enclose and fill: a rectangle around the left cell only encloses nothing (the cell
        // reaches the shape's edge); one around a closed box does.
        let _ = s.execute("select.deselect", json!({}));
        let r = s
            .execute(
                "paint.encloseFill",
                json!({"shape": "rectangle", "from": [1, 1], "to": [8, 8], "color": "#ff00ff", "antiAlias": false, "tolerance": 0, "reference": "all"}),
            )
            .unwrap();
        assert_eq!(r["filled"], false);
        // Draw a closed ring on the wall layer and enclose it.
        s.edit("ring", |doc, _| {
            let surf = doc.layer_mut(bottom).unwrap().surface_mut().unwrap();
            surf.fill_rect(Rect::new(2, 2, 8, 3), &[0.0, 0.0, 0.0, 1.0]);
            surf.fill_rect(Rect::new(2, 7, 8, 8), &[0.0, 0.0, 0.0, 1.0]);
            surf.fill_rect(Rect::new(2, 2, 3, 8), &[0.0, 0.0, 0.0, 1.0]);
            surf.fill_rect(Rect::new(7, 2, 8, 8), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let r = s
            .execute(
                "paint.encloseFill",
                json!({"shape": "rectangle", "from": [1, 1], "to": [9, 9], "rule": "surroundedBy", "ruleColor": "#000000", "color": "#ff00ff", "antiAlias": false, "tolerance": 0, "reference": "all"}),
            )
            .unwrap();
        assert_eq!(r["filled"], true);
        assert_eq!(px(&s, 5, 5), vec![1.0, 0.0, 1.0, 1.0], "the box's inside is surrounded by black");
        assert_eq!(px(&s, 2, 2)[3], 0.0, "the ring itself is not");
        assert_eq!(px(&s, 15, 5)[3], 0.0);
        assert!(s.execute("paint.encloseFill", json!({"shape": "polygon", "points": [[0, 0], [1, 1]]})).is_err());
        assert!(s.execute("paint.encloseFill", json!({"shape": "rectangle", "from": [0, 0], "to": [5, 5], "rule": "nothing"})).is_err());
    }
}
