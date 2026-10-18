//! The Brush tool's engines: the brush tip (the default), the Ink nib (`paint.ink`) and the
//! MyPaint-style presets (`paint.mypaint`). The options bar shows an Engine dropdown and the
//! chosen engine's controls; the canvas asks [`stroke_command`] which command a stroke commits
//! and [`extend_params`] for the engine's parameters.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

const ENGINES: [(&str, &str); 3] = [("brush", "Brush"), ("ink", "Ink"), ("mypaint", "MyPaint Brush")];
const INK_SHAPES: [(&str, &str); 3] = [("circle", "Circle"), ("square", "Square"), ("diamond", "Diamond")];

/// The engine in use for `tool` (only the Brush tool has engines).
pub fn engine(app: &PhotocraftApp, tool: Tool) -> &str {
    if tool == Tool::Brush { app.ui.tool_options.brush_engine.as_str() } else { "brush" }
}

/// The command a Brush stroke commits under the chosen engine, or `None` for the brush tip.
pub fn stroke_command(app: &PhotocraftApp, tool: Tool) -> Option<&'static str> {
    match engine(app, tool) {
        "ink" => Some("paint.ink"),
        "mypaint" => Some("paint.mypaint"),
        _ => None,
    }
}

/// Adds the engine's parameters to a stroke's params.
pub fn extend_params(app: &PhotocraftApp, tool: Tool, p: &mut Value) {
    let o = &app.ui.tool_options;
    match engine(app, tool) {
        "ink" => {
            p["size"] = json!(app.session.tools.brush.size);
            p["shape"] = json!(o.ink_shape);
            p["angle"] = json!(o.ink_angle);
            p["aspect"] = json!(o.ink_aspect);
            p["sensitivity"] = json!({ "size": o.ink_sens_size, "tilt": o.ink_sens_tilt, "speed": o.ink_sens_speed });
            p["opacity"] = json!(app.session.tools.brush.opacity);
        }
        "mypaint" => {
            p["preset"] = json!(o.mypaint_preset);
            p["radius"] = json!(o.mypaint_radius);
            p["opaque"] = json!(o.mypaint_opaque);
            if o.mypaint_erase {
                p["erase"] = json!(true);
            }
        }
        _ => {}
    }
}

fn label(ui: &mut egui::Ui, s: &str) {
    ui.label(egui::RichText::new(s).color(crate::theme::Tokens::get(ui.ctx()).text_dim));
}

/// The Engine dropdown and the engine's own controls, in the Brush tool's options bar.
pub fn controls(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let engines: Vec<(String, String)> = ENGINES.iter().map(|(k, l)| (k.to_string(), tl!(l).to_string())).collect();
    let refs: Vec<(String, &str)> = engines.iter().map(|(k, l)| (k.clone(), l.as_str())).collect();
    label(ui, tl!("Engine"));
    crate::widgets::dropdown(ui, "brush-engine", &mut app.ui.tool_options.brush_engine, &refs, 120.0);
    match app.ui.tool_options.brush_engine.as_str() {
        "ink" => ink_controls(app, ui),
        "mypaint" => mypaint_controls(app, ui),
        _ => {}
    }
    crate::widgets::vline(ui, 22.0);
}

fn ink_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let o = &mut app.ui.tool_options;
    let shapes: Vec<(String, String)> = INK_SHAPES.iter().map(|(k, l)| (k.to_string(), tl!(l).to_string())).collect();
    let refs: Vec<(String, &str)> = shapes.iter().map(|(k, l)| (k.clone(), l.as_str())).collect();
    label(ui, tl!("Nib"));
    crate::widgets::dropdown(ui, "ink-shape", &mut o.ink_shape, &refs, 100.0);
    label(ui, tl!("Angle"));
    crate::widgets::value_field(ui, &mut o.ink_angle, 0.0..=360.0, "°", 60.0);
    label(ui, tl!("Aspect"));
    crate::widgets::value_field_prec(ui, &mut o.ink_aspect, 1.0..=10.0, "", 52.0, 1);
    label(ui, tl!("Pressure"));
    crate::widgets::value_field_prec(ui, &mut o.ink_sens_size, 0.0..=1.0, "", 48.0, 2).on_hover_text(tl!("How much pressure changes the nib size"));
    label(ui, tl!("Tilt"));
    crate::widgets::value_field_prec(ui, &mut o.ink_sens_tilt, 0.0..=1.0, "", 48.0, 2)
        .on_hover_text(tl!("How much the pen's tilt turns and stretches the nib"));
    label(ui, tl!("Speed"));
    crate::widgets::value_field_prec(ui, &mut o.ink_sens_speed, 0.0..=1.0, "", 48.0, 2).on_hover_text(tl!("How much a fast stroke thins the nib"));
}

fn mypaint_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let brushes = redrob_photo_engine::mypaint_engine_cmds::brushes(&app.session);
    let names: Vec<(String, String)> =
        brushes.iter().map(|b| (b.name.clone(), if b.group.is_empty() { b.name.clone() } else { format!("{} › {}", b.group, b.name) })).collect();
    let refs: Vec<(String, &str)> = names.iter().map(|(k, l)| (k.clone(), l.as_str())).collect();
    let o = &mut app.ui.tool_options;
    if !brushes.iter().any(|b| b.name == o.mypaint_preset)
        && let Some(first) = brushes.first()
    {
        o.mypaint_preset = first.name.clone();
    }
    label(ui, tl!("Preset"));
    crate::widgets::dropdown(ui, "mypaint-preset", &mut o.mypaint_preset, &refs, 170.0);
    label(ui, tl!("Radius"));
    crate::widgets::value_field_prec(ui, &mut o.mypaint_radius, -3.0..=3.0, "", 52.0, 2).on_hover_text(tl!("Added to the preset's logarithmic radius"));
    label(ui, tl!("Opaque"));
    crate::widgets::value_field_prec(ui, &mut o.mypaint_opaque, 0.0..=2.0, "", 48.0, 2);
    crate::widgets::checkbox(ui, &mut o.mypaint_erase, tl!("Erase with brush"));
    if ui.button(tl!("Import .myb…")).on_hover_text(tl!("Add a MyPaint brush file to the presets")).clicked() {
        import_dialog(app);
    }
}

/// Import .myb…: the chosen file becomes a preset and is selected.
pub fn import_dialog(app: &mut PhotocraftApp) {
    let result = app.pick_file_bytes_filtered(&["myb"], |app, name, bytes| {
        let stem = std::path::Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.clone());
        let r = app.run("mypaint.import", json!({"data": redrob_photo_paint::tile::b64_encode(&bytes), "name": stem}))?;
        if let Some(n) = r.get("name").and_then(Value::as_str) {
            app.ui.tool_options.mypaint_preset = n.to_string();
            app.ui.tool_options.brush_engine = "mypaint".into();
            app.ui.status_error = false;
            app.ui.status = crate::i18n::fmt(tl!("Imported MyPaint brush {name}"), &[("name", n)]);
        }
        Ok(r)
    });
    if let Err(e) = result
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}
