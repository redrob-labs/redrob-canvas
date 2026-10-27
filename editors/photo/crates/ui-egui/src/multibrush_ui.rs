//! The Multibrush tool: paints like the Brush with every stroke repeated by the document's
//! multibrush symmetry (`paint.setMultibrush`): mirror / rotation (the rotational mode),
//! snowflake (rotation plus mirroring), translate (copies on a circle), copy translate (copies at
//! offsets placed with Ctrl-click) and interval (copies on a grid, up to 50). The options bar
//! edits the mode and its settings; the centre is the document's symmetry centre.

use serde_json::{Value, json};

use crate::{PhotoApp, canvas::ToolEvent, state::Tool, theme::Tokens, widgets};

/// Every multibrush mode: (`mode` value, label).
pub const MODES: [(&str, &str); 5] = [
    ("rotational", "Mirror / Rotation"),
    ("snowflake", "Snowflake"),
    ("translate", "Translate"),
    ("copyTranslate", "Copy Translate"),
    ("interval", "Interval"),
];

fn opt(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).color(t.text_dim).size(12.0));
}

/// The symmetry the document holds, as `paint.setMultibrush` reports it.
fn current(app: &PhotoApp) -> Option<Value> {
    app.session.active().and_then(|st| st.symmetry.as_ref()).map(|s| s.inspect())
}

/// Send the options bar's settings to the engine.
pub fn apply(app: &mut PhotoApp) {
    let o = app.ui.tool_options.clone();
    let mode = if o.multibrush_mode.is_empty() { "rotational" } else { o.multibrush_mode.as_str() };
    let center = app.ui.multibrush_center.map(|c| json!(c)).unwrap_or(Value::Null);
    let p = match mode {
        "translate" => json!({"mode": mode, "center": center, "count": o.multibrush_count.round().max(1.0), "radius": o.multibrush_radius.max(1.0)}),
        "copyTranslate" => json!({"mode": mode, "offsets": app.ui.multibrush_offsets}),
        "interval" => {
            json!({"mode": mode, "x": o.multibrush_x, "y": o.multibrush_y, "columns": o.multibrush_columns.round().max(1.0), "rows": o.multibrush_rows.round().max(1.0)})
        }
        _ => json!({"mode": mode, "center": center, "count": o.multibrush_count.round().clamp(2.0, 64.0), "mirror": mode == "snowflake"}),
    };
    if let Err(e) = app.run("paint.setMultibrush", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Ctrl-click with the Multibrush tool in copy-translate mode places a copy (its offset from the
/// centre, or from the first click when there is no centre); Ctrl-Alt-click removes the nearest.
/// Returns true when the press was consumed.
pub fn pointer(app: &mut PhotoApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if app.ui.tool != Tool::Multibrush || !mods.command {
        return false;
    }
    let ToolEvent::Down { x, y, .. } = ev else { return true };
    if app.ui.tool_options.multibrush_mode != "copyTranslate" {
        // Ctrl-click sets the centre of the other modes.
        app.ui.multibrush_center = Some([x, y]);
        apply(app);
        return true;
    }
    let center = *app.ui.multibrush_center.get_or_insert([x, y]);
    let offset = [x - center[0], y - center[1]];
    if mods.alt {
        let tol = 8.0 / f64::from(app.point_zoom().max(0.01));
        if let Some(i) = app
            .ui
            .multibrush_offsets
            .iter()
            .enumerate()
            .map(|(i, o)| (i, (o[0] - offset[0]).hypot(o[1] - offset[1])))
            .filter(|(_, d)| *d <= tol)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
        {
            app.ui.multibrush_offsets.remove(i);
        }
    } else if app.ui.multibrush_offsets.len() < redrob_photo_paint::symmetry::MAX_TRANSLATE_COPIES && offset[0].hypot(offset[1]) >= 1.0 {
        app.ui.multibrush_offsets.push(offset);
    }
    if !app.ui.multibrush_offsets.is_empty() {
        apply(app);
    }
    true
}

/// The Multibrush tool's options: mode and its settings (shown after the brush options).
pub fn options(app: &mut PhotoApp, ui: &mut egui::Ui) {
    if app.ui.tool != Tool::Multibrush {
        return;
    }
    widgets::vline(ui, 22.0);
    opt(ui, tl!("Multibrush"));
    let mut changed = false;
    {
        let o = &mut app.ui.tool_options;
        if o.multibrush_mode.is_empty() {
            o.multibrush_mode = "rotational".into();
        }
        let modes: Vec<(String, &str)> = MODES.iter().map(|(k, l)| (k.to_string(), tl!(*l))).collect();
        changed |= widgets::dropdown_literal(ui, "multibrush-mode", &mut o.multibrush_mode, &modes, 140.0);
        match o.multibrush_mode.as_str() {
            "translate" => {
                opt(ui, tl!("Copies"));
                changed |= widgets::value_field(ui, &mut o.multibrush_count, 1.0..=50.0, "", 48.0).changed();
                opt(ui, tl!("Radius"));
                changed |= widgets::value_field(ui, &mut o.multibrush_radius, 1.0..=10000.0, "px", 62.0).changed();
            }
            "copyTranslate" => {
                ui.label(egui::RichText::new(tl!("Ctrl-click places a copy, Ctrl-Alt-click removes one")).color(Tokens::get(ui.ctx()).text_dim).size(12.0));
            }
            "interval" => {
                opt(ui, tl!("X interval"));
                changed |= widgets::value_field(ui, &mut o.multibrush_x, -10000.0..=10000.0, "px", 62.0).changed();
                opt(ui, tl!("Y interval"));
                changed |= widgets::value_field(ui, &mut o.multibrush_y, -10000.0..=10000.0, "px", 62.0).changed();
                opt(ui, tl!("Columns"));
                changed |= widgets::value_field(ui, &mut o.multibrush_columns, 1.0..=50.0, "", 48.0).changed();
                opt(ui, tl!("Rows"));
                changed |= widgets::value_field(ui, &mut o.multibrush_rows, 1.0..=50.0, "", 48.0).changed();
            }
            _ => {
                opt(ui, tl!("Copies"));
                changed |= widgets::value_field(ui, &mut o.multibrush_count, 2.0..=64.0, "", 48.0).changed();
            }
        }
    }
    if app.ui.tool_options.multibrush_mode == "copyTranslate" {
        let n = app.ui.multibrush_offsets.len();
        if ui.add_enabled_ui(n > 0, |ui| widgets::secondary_button(ui, tl!("Clear copies"), 0.0)).inner.clicked() {
            app.ui.multibrush_offsets.clear();
            let _ = app.run("paint.symmetryDisable", json!({}));
        }
    }
    let on = current(app).is_some();
    if changed || (!on && app.ui.tool_options.multibrush_mode != "copyTranslate") {
        apply(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_are_the_engine_names() {
        for (k, _) in MODES {
            assert!(["rotational", "snowflake", "translate", "copyTranslate", "interval"].contains(&k));
        }
    }
}
