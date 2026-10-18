//! Brush engines in the options bar: which engine the Brush paints with (the tip-stamping Pixel
//! engine, or sketch, hatching, shape, grid, particle, curve, dyna, tangent normal, MyPaint-style,
//! spray, bristle, deform, or a filter through the brush), that engine's main settings, a text tip,
//! and a MyPaint-style brush file. The current engine lives in `ToolOptions::engine_settings` (the
//! `engine` object `paint.engineStroke` takes) and travels in every stroke's params.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::Tokens;
use crate::widgets;

/// A UI-only engine: a filter through the brush (`paint.filterBrush`).
pub const FILTER_KIND: &str = "filter";

/// The default engine object: the Pixel engine.
pub fn default_engine() -> Value {
    json!({ "kind": "pixel" })
}

/// The current engine's `kind`.
pub fn kind(app: &PhotocraftApp) -> String {
    app.ui.tool_options.engine_settings.get("kind").and_then(Value::as_str).unwrap_or("pixel").to_string()
}

/// Does a stroke with `tool` go to an alternative engine (so it is not drawn live and commits
/// through `paint.engineStroke` / `paint.filterBrush`)?
pub fn active(app: &PhotocraftApp, tool: Tool) -> bool {
    tool == Tool::Brush && kind(app) != "pixel"
}

/// Commit a Brush drag with the current engine. Returns false when the Pixel engine paints it.
pub fn commit(app: &mut PhotocraftApp, d: &crate::canvas::Drag) -> bool {
    if !active(app, d.tool) {
        return false;
    }
    app.live_stroke = None;
    let pts = crate::stroke_constraint::straighten(&d.points, d.live.shift || d.modifiers.shift);
    let points = app.stylus.stroke_points(if crate::canvas::line_mode(app, d.tool) { &pts } else { &d.points });
    let engine = app.ui.tool_options.engine_settings.clone();
    let k = kind(app);
    let target = crate::canvas::paint_target(app);
    let sample_all = app.ui.tool_options.sample_all_layers;
    let (cmd, p) = if k == FILTER_KIND {
        let filter = engine.get("filter").and_then(Value::as_str).unwrap_or("filter.blur.gaussianBlur").to_string();
        let fp = engine.get("filterParams").cloned().unwrap_or_else(|| json!({}));
        ("paint.filterBrush", json!({ "points": points, "filter": filter, "filterParams": fp, "sampleAllLayers": sample_all, "target": target }))
    } else {
        ("paint.engineStroke", json!({ "points": points, "engine": engine, "sampleAllLayers": sample_all, "target": target }))
    };
    if let Err(e) = app.run(cmd, p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

/// The engines offered, by `kind` and label.
const ENGINES: &[(&str, &str)] = &[
    ("pixel", "Pixel"),
    ("sketch", "Sketch"),
    ("hatching", "Hatching"),
    ("shape", "Shape"),
    ("grid", "Grid"),
    ("particle", "Particle"),
    ("curve", "Curve"),
    ("dyna", "Dyna"),
    ("tangentNormal", "Tangent Normal"),
    ("myPaint", "MyPaint"),
    ("spray", "Spray"),
    ("bristle", "Bristle"),
    ("deform", "Deform"),
    (FILTER_KIND, "Filter"),
];

/// Numeric settings shown per engine: key, label, min, max, decimals.
fn numbers(kind: &str) -> &'static [(&'static str, &'static str, f32, f32, u32)] {
    match kind {
        "sketch" => &[
            ("lineWidth", "Line Width", 0.1, 50.0, 1),
            ("offsetScale", "Offset", 0.0, 1.0, 2),
            ("density", "Density", 0.0, 1.0, 2),
            ("randomRgb", "Random RGB", 0.0, 1.0, 2),
            ("randomOpacity", "Random Opacity", 0.0, 1.0, 2),
        ],
        "hatching" => &[("angle", "Angle", -180.0, 180.0, 0), ("separation", "Separation", 0.5, 200.0, 1), ("thickness", "Thickness", 0.1, 50.0, 1)],
        "shape" => &[("speed", "Speed", 0.0, 1.0, 2), ("smooth", "Smooth", 0.0, 8.0, 0), ("displace", "Displace", -200.0, 200.0, 1)],
        "grid" => &[
            ("gridWidth", "Grid Width", 1.0, 500.0, 0),
            ("gridHeight", "Grid Height", 1.0, 500.0, 0),
            ("division", "Division", 1.0, 64.0, 0),
            ("jitter", "Jitter", 0.0, 1.0, 2),
            ("scale", "Scale", 0.01, 4.0, 2),
        ],
        "particle" => &[
            ("particles", "Particles", 1.0, 500.0, 0),
            ("opacityWeight", "Weight", 0.0, 1.0, 2),
            ("gravity", "Gravity", 0.0, 1.0, 2),
            ("iterations", "Iterations", 1.0, 200.0, 0),
            ("dxScale", "X Scale", 0.01, 10.0, 2),
            ("dyScale", "Y Scale", 0.01, 10.0, 2),
        ],
        "curve" => &[
            ("lineWidth", "Line Width", 0.1, 50.0, 1),
            ("historySize", "History", 1.0, 1000.0, 0),
            ("curveOpacity", "Curve Opacity", 0.0, 1.0, 2),
            ("spacing", "Spacing", 0.5, 100.0, 1),
        ],
        "dyna" => &[
            ("mass", "Mass", 0.01, 2.0, 2),
            ("drag", "Drag", 0.0, 1.0, 2),
            ("width", "Width", 0.5, 200.0, 1),
            ("widthRange", "Width Range", 0.0, 1.0, 2),
            ("lineCount", "Lines", 1.0, 16.0, 0),
            ("lineSpacing", "Line Spacing", 0.0, 100.0, 1),
        ],
        "tangentNormal" => {
            &[("elevationSensitivity", "Elevation", 0.0, 1.0, 2), ("angleOffset", "Angle Offset", -180.0, 180.0, 0), ("mix", "Mix", 0.0, 1.0, 2)]
        }
        "spray" => &[
            ("diameter", "Diameter", 0.0, 1000.0, 0),
            ("aspect", "Aspect", 0.05, 20.0, 2),
            ("angle", "Angle", -180.0, 180.0, 0),
            ("count", "Count", 1.0, 4096.0, 0),
            ("shapeWidth", "Particle Width", 0.5, 200.0, 1),
            ("shapeHeight", "Particle Height", 0.5, 200.0, 1),
        ],
        "bristle" => &[
            ("scale", "Scale", 0.1, 4.0, 2),
            ("randomOffset", "Random Offset", 0.0, 50.0, 1),
            ("shear", "Shear", -2.0, 2.0, 2),
            ("density", "Density", 0.0, 1.0, 2),
            ("threshold", "Threshold", 0.0, 1.0, 2),
            ("inkAmount", "Ink Amount", 1.0, 100000.0, 0),
        ],
        "deform" => &[("amount", "Amount", 0.0, 1.0, 2), ("counter", "Counter", 1.0, 100.0, 0)],
        _ => &[],
    }
}

/// Toggles shown per engine: key, label.
fn toggles(kind: &str) -> &'static [(&'static str, &'static str)] {
    match kind {
        "sketch" => &[
            ("magnetify", "Magnetify"),
            ("distanceDensity", "Distance Density"),
            ("distanceOpacity", "Distance Opacity"),
            ("simpleMode", "Simple Mode"),
            ("antialias", "Anti-alias"),
        ],
        "hatching" => &[
            ("separationDynamics", "Separation by Pressure"),
            ("thicknessDynamics", "Thickness by Pressure"),
            ("coloredBackground", "Coloured Background"),
            ("antialias", "Anti-alias"),
        ],
        "shape" => &[("evenOdd", "Even-odd Fill"), ("hardEdge", "Hard Edge")],
        "grid" => &[
            ("divisionByPressure", "Division by Pressure"),
            ("colorPerParticle", "Colour per Particle"),
            ("sampleInput", "Sample Input Layer"),
            ("fillBackground", "Fill Background"),
        ],
        "particle" => &[("antialias", "Anti-alias")],
        "curve" => &[("widthByPressure", "Width by Pressure"), ("opacityByPressure", "Opacity by Pressure"), ("connectionLine", "Connection Line")],
        "dyna" => &[("fixedAngle", "Fixed Angle"), ("paintConnection", "Paint Connection")],
        "myPaint" => &[("useBrushColor", "Use Foreground Colour")],
        "spray" => &[
            ("useDensity", "Use Density"),
            ("proportional", "Proportional"),
            ("randomSize", "Random Size"),
            ("sampleInput", "Sample Input Layer"),
            ("colorPerParticle", "Colour per Particle"),
        ],
        "bristle" => &[
            ("connectHairs", "Connect Hairs"),
            ("inkDepletion", "Ink Depletion"),
            ("soakInk", "Soak Ink"),
            ("mousePressure", "Mouse Pressure"),
            ("compositeBristles", "Composite Bristles"),
        ],
        "deform" => &[("bilinear", "Bilinear"), ("useUndeformed", "Use Undeformed Image")],
        _ => &[],
    }
}

/// Choice settings shown per engine: key, label, options (value, label).
fn choices(kind: &str) -> &'static [(&'static str, &'static str, &'static [(&'static str, &'static str)])] {
    match kind {
        "hatching" => &[(
            "crosshatch",
            "Crosshatch",
            &[
                ("none", "None"),
                ("perpendicular", "Perpendicular"),
                ("minusThenPlus", "-45° then +45°"),
                ("plusThenMinus", "+45° then -45°"),
                ("moire", "Moiré"),
            ],
        )],
        "grid" => &[(
            "particle",
            "Particle",
            &[("ellipse", "Ellipse"), ("rectangle", "Rectangle"), ("line", "Line"), ("pixel", "Pixel"), ("antialiasedPixel", "Anti-aliased Pixel")],
        )],
        "dyna" => &[("shape", "Shape", &[("circle", "Circle"), ("two", "Two"), ("lines", "Lines"), ("polygon", "Polygon"), ("wire", "Wire")])],
        "tangentNormal" => &[("source", "Source", &[("tilt", "Tilt"), ("direction", "Direction"), ("rotation", "Rotation")])],
        "spray" => &[
            ("radial", "Radial", &[("uniform", "Uniform"), ("gaussian", "Gaussian"), ("cluster", "Cluster"), ("curve", "Curve")]),
            ("angular", "Angular", &[("uniform", "Uniform"), ("gaussian", "Gaussian"), ("cluster", "Cluster"), ("curve", "Curve")]),
            (
                "shape",
                "Particle",
                &[
                    ("ellipse", "Ellipse"),
                    ("rectangle", "Rectangle"),
                    ("antialiasedPixel", "Anti-aliased Pixel"),
                    ("pixel", "Pixel"),
                    ("line", "Line"),
                    ("tip", "Tip"),
                ],
            ),
            ("rotation", "Rotation", &[("fixed", "Fixed"), ("random", "Random"), ("followCursor", "Follow Cursor"), ("weighted", "Weighted")]),
        ],
        "deform" => &[(
            "mode",
            "Mode",
            &[
                ("grow", "Grow"),
                ("shrink", "Shrink"),
                ("swirlCw", "Swirl CW"),
                ("swirlCcw", "Swirl CCW"),
                ("move", "Move"),
                ("lensZoomIn", "Lens Zoom In"),
                ("lensZoomOut", "Lens Zoom Out"),
                ("colorDeform", "Colour Deform"),
            ],
        )],
        _ => &[],
    }
}

/// Filters offered through the Filter engine: id, label, and the one parameter shown (key, label,
/// min, max) when the filter has one.
const FILTERS: &[(&str, &str, Option<(&str, &str, f32, f32)>)] = &[
    ("filter.blur.gaussianBlur", "Gaussian Blur", Some(("radius", "Radius", 0.1, 250.0))),
    ("filter.blur.boxBlur", "Box Blur", Some(("radius", "Radius", 1.0, 250.0))),
    ("filter.blur.motionBlur", "Motion Blur", Some(("distance", "Distance", 1.0, 500.0))),
    ("filter.sharpen.unsharpMask", "Unsharp Mask", Some(("amount", "Amount", 1.0, 500.0))),
    ("filter.sharpen.sharpen", "Sharpen", None),
    ("filter.noise.addNoise", "Add Noise", Some(("amount", "Amount", 0.1, 400.0))),
    ("filter.noise.median", "Median", Some(("radius", "Radius", 1.0, 100.0))),
    ("filter.other.highPass", "High Pass", Some(("radius", "Radius", 0.1, 250.0))),
    ("filter.other.minimum", "Minimum", Some(("radius", "Radius", 0.2, 100.0))),
    ("filter.other.maximum", "Maximum", Some(("radius", "Radius", 0.2, 100.0))),
    ("filter.pixelate.mosaic", "Mosaic", Some(("cellSize", "Cell Size", 2.0, 200.0))),
    ("filter.stylize.emboss", "Emboss", Some(("height", "Height", 1.0, 100.0))),
    ("filter.stylize.findEdges", "Find Edges", None),
    ("filter.stylize.solarize", "Solarize", None),
    ("filter.adjust.desaturate", "Desaturate", None),
    ("filter.distort.twirl", "Twirl", Some(("angle", "Angle", -999.0, 999.0))),
    ("filter.distort.pinch", "Pinch", Some(("amount", "Amount", -100.0, 100.0))),
    ("filter.distort.spherize", "Spherize", Some(("amount", "Amount", -100.0, 100.0))),
];

fn label(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    let text = if t.pro && !s.ends_with(':') { format!("{}:", tl!(s)) } else { tl!(s).to_string() };
    ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_dim)).selectable(false));
}

/// Set the engine object to `kind`'s defaults (asking the engine for them).
fn select_kind(app: &mut PhotocraftApp, kind: &str) {
    let engine = if kind == FILTER_KIND {
        json!({ "kind": FILTER_KIND, "filter": "filter.blur.gaussianBlur", "filterParams": { "radius": 5.0 } })
    } else {
        match app.run("brush.engines.list", json!({})) {
            Ok(r) => r
                .get("engines")
                .and_then(Value::as_array)
                .and_then(|a| a.iter().find(|e| e.get("kind").and_then(Value::as_str) == Some(kind)))
                .and_then(|e| e.get("defaults").cloned())
                .unwrap_or_else(|| json!({ "kind": kind })),
            Err(_) => json!({ "kind": kind }),
        }
    };
    app.ui.tool_options.engine_settings = engine;
}

/// The engine section of the Brush options bar: the engine dropdown and the engine's settings.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let mut k = kind(app);
    label(ui, "Engine");
    let options: Vec<(String, &str)> = ENGINES.iter().map(|(id, l)| (id.to_string(), *l)).collect();
    // Not "brush-engine": that is the tip / Ink / MyPaint dropdown's id (paint_engines_ui), on the
    // same bar.
    if widgets::dropdown(ui, "brush-engine-kind", &mut k, &options, 110.0) {
        select_kind(app, &k);
    }
    if k == "pixel" {
        return;
    }
    widgets::vline(ui, 22.0);
    let mut engine = app.ui.tool_options.engine_settings.clone();
    let mut changed = false;
    if k == FILTER_KIND {
        let mut filter = engine.get("filter").and_then(Value::as_str).unwrap_or("filter.blur.gaussianBlur").to_string();
        let opts: Vec<(String, &str)> = FILTERS.iter().map(|(id, l, _)| (id.to_string(), *l)).collect();
        label(ui, "Filter");
        if widgets::dropdown(ui, "brush-engine-filter", &mut filter, &opts, 130.0) {
            engine["filter"] = json!(filter);
            engine["filterParams"] = json!({});
            changed = true;
        }
        if let Some((key, l, min, max)) = FILTERS.iter().find(|(id, _, _)| *id == filter).and_then(|(_, _, p)| *p) {
            let mut v = engine.get("filterParams").and_then(|fp| fp.get(key)).and_then(Value::as_f64).map_or((min + max) / 2.0, |x| x as f32);
            label(ui, l);
            if widgets::value_field_prec(ui, &mut v, min..=max, "", 64.0, 1).changed() {
                engine["filterParams"][key] = json!(v);
                changed = true;
            }
        }
    } else {
        for (key, l, min, max, decimals) in numbers(&k) {
            let mut v = engine.get(*key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            label(ui, l);
            if widgets::value_field_prec(ui, &mut v, *min..=*max, "", 56.0, *decimals).changed() {
                engine[*key] = if *decimals == 0 { json!(v.round() as i64) } else { json!(v) };
                changed = true;
            }
        }
        for (key, l, opts) in choices(&k) {
            let mut v = engine.get(*key).and_then(Value::as_str).unwrap_or(opts.first().map_or("", |o| o.0)).to_string();
            let options: Vec<(String, &str)> = opts.iter().map(|(id, ol)| (id.to_string(), *ol)).collect();
            label(ui, l);
            if widgets::dropdown(ui, &format!("brush-engine-{key}"), &mut v, &options, 110.0) {
                engine[*key] = json!(v);
                changed = true;
            }
        }
        for (key, l) in toggles(&k) {
            let mut on = engine.get(*key).and_then(Value::as_bool).unwrap_or(false);
            if widgets::checkbox(ui, &mut on, tl!(l)).changed() {
                engine[*key] = json!(on);
                changed = true;
            }
        }
        if k == "myPaint" {
            // A MyPaint-style brush file by path.
            let id = ui.make_persistent_id("brush-engine-myb-path");
            let mut path: String = ui.ctx().data_mut(|d| d.get_temp(id)).unwrap_or_default();
            if widgets::text_field(ui, id, &mut path, tl!("Brush file (.myb)"), 180.0, false).changed() {
                ui.ctx().data_mut(|d| d.insert_temp(id, path.clone()));
            }
            if widgets::secondary_button(ui, tl!("Load"), 48.0).clicked() && !path.trim().is_empty() {
                let use_fg = engine.get("useBrushColor").and_then(Value::as_bool).unwrap_or(true);
                match app.run("brush.mypaint.parse", json!({ "path": path.trim(), "useBrushColor": use_fg })) {
                    Ok(r) => {
                        if let Some(e) = r.get("engine") {
                            engine = e.clone();
                            changed = true;
                        }
                    }
                    Err(e) => {
                        app.ui.status = e;
                        app.ui.status_error = true;
                    }
                }
            }
            if let Some(name) = engine.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) {
                let t = Tokens::get(ui.ctx());
                ui.add(egui::Label::new(egui::RichText::new(name).color(t.text_dim)).selectable(false));
            }
        }
    }
    if changed {
        app.ui.tool_options.engine_settings = engine;
    }
}

/// The text-tip row of the Brush options bar: a string, pipe mode and Apply (`brush.textTip`).
pub fn text_tip_row(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let id = ui.make_persistent_id("brush-text-tip");
    let mut text: String = ui.ctx().data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let pipe_id = id.with("pipe");
    let mut pipe: bool = ui.ctx().data_mut(|d| d.get_temp(pipe_id)).unwrap_or(false);
    label(ui, "Text Tip");
    if widgets::text_field(ui, id, &mut text, tl!("Text as brush tip"), 120.0, false).changed() {
        ui.ctx().data_mut(|d| d.insert_temp(id, text.clone()));
    }
    if widgets::checkbox(ui, &mut pipe, tl!("Pipe")).on_hover_text(tl!("One character per dab along the stroke")).changed() {
        ui.ctx().data_mut(|d| d.insert_temp(pipe_id, pipe));
    }
    if widgets::secondary_button(ui, tl!("Apply"), 48.0).clicked() && !text.trim().is_empty() {
        let size = app.session.tools.brush.size.max(4.0);
        if let Err(e) = app.run("brush.textTip", json!({ "text": text, "size": size, "pipe": pipe })) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}
