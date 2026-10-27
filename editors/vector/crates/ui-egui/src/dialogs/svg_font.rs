//! Type › SVG Font Editor…: the document's SVG fonts (`svgFont.*`): pick or make a font, list its
//! glyphs (unicode, name, advance), add and remove glyphs, take a glyph's curves from the
//! selection or put them back as objects, set kerning pairs, set a preview string, and turn
//! glyph layers into a font or a font into glyph layers.
//!
//! Fields: `font` (the id shown), `fonts` (svgFont.list's fonts), `glyph` (the selected
//! unicode), `newUnicode`, `newName`, `advance`, `kernLeft`, `kernRight`, `kernK`, `preview`,
//! `emSize`.

use serde_json::{Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorApp, widgets};

/// The dialog kind of the SVG Font Editor.
pub const KIND: &str = "svgFontEditor";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("SVG Font Editor").into(), body, confirm, ok: None, min_width: 460.0, ..DialogSpec::FORM };

/// `ui.svgFontEditor {font?}`: open the editor on a font (default: the first, made when none).
pub fn open(app: &mut VectorApp, p: &Value) -> Result<Value, String> {
    let mut fields = json!({ "newUnicode": "", "newName": "", "advance": "", "kernLeft": "", "kernRight": "", "kernK": "0", "preview": "Abc", "emSize": "72", "glyph": "" });
    if let Some(f) = p.get("font").and_then(Value::as_str) {
        fields["font"] = json!(f);
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    refresh(app);
    Ok(Value::Null)
}

/// Fetch the fonts again into the dialog (after any change).
fn refresh(app: &mut VectorApp) {
    let Ok(r) = app.run("svgFont.list", json!({})) else { return };
    let fonts = r.get("fonts").cloned().unwrap_or(json!([]));
    if let Some(d) = app.ui.dialog.as_mut().filter(|d| d.kind == KIND) {
        let cur = d.str("font");
        let ids: Vec<String> =
            fonts.as_array().map(|a| a.iter().filter_map(|f| f.get("id").and_then(Value::as_str)).map(str::to_string).collect()).unwrap_or_default();
        if !ids.contains(&cur) {
            d.fields.insert("font".into(), json!(ids.first().cloned().unwrap_or_default()));
        }
        d.fields.insert("fonts".into(), fonts);
    }
}

/// The font shown.
fn current(d: &Dialog) -> Option<Value> {
    let id = d.str("font");
    d.fields.get("fonts").and_then(Value::as_array).and_then(|a| a.iter().find(|f| f.get("id").and_then(Value::as_str) == Some(id.as_str()))).cloned()
}

fn body(app: &mut VectorApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut actions: Vec<(&str, Value)> = vec![];
    // The font row.
    let fonts: Vec<(String, String)> = d
        .fields
        .get("fonts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|f| Some((f.get("id")?.as_str()?.to_string(), f.get("family").and_then(Value::as_str).unwrap_or("").to_string())))
                .collect()
        })
        .unwrap_or_default();
    ui.horizontal(|ui| {
        label(ui, tl!("Font:"));
        let cur = d.str("font");
        let shown = fonts.iter().find(|(id, _)| *id == cur).map(|(_, fam)| fam.as_str()).unwrap_or("");
        let names: Vec<&str> = fonts.iter().map(|(_, f)| f.as_str()).collect();
        if let Some(i) = widgets::dropdown_names(ui, "svgfont-pick", shown, &names, 160.0)
            && let Some((id, _)) = fonts.get(i)
        {
            d.fields.insert("font".into(), json!(id));
            d.fields.insert("glyph".into(), json!(""));
        }
        if widgets::secondary_button(ui, tl!("New Font")).clicked() {
            actions.push(("svgFont.create", json!({})));
        }
        if !fonts.is_empty() && widgets::secondary_button(ui, tl!("Delete Font")).clicked() {
            actions.push(("svgFont.delete", json!({ "font": cur })));
        }
    });
    let Some(font) = current(d) else {
        ui.label(egui::RichText::new(tl!("No SVG font yet: make one, or turn glyph layers into one.")).color(t.text_dim));
        if widgets::secondary_button(ui, tl!("Glyph Layers to Font")).clicked() {
            actions.push(("svgFont.fromLayers", json!({})));
        }
        run(app, d, actions);
        return false;
    };
    let font_id = d.str("font");
    ui.add_space(6.0);
    // Glyphs.
    let glyphs: Vec<Value> = font.get("glyphs").and_then(Value::as_array).cloned().unwrap_or_default();
    let selected = d.str("glyph");
    egui::ScrollArea::vertical().id_salt("svgfont-glyphs").max_height(180.0).show(ui, |ui| {
        egui::Grid::new("svgfont-glyph-grid").num_columns(4).spacing([12.0, 4.0]).striped(true).show(ui, |ui| {
            for h in [tl!("Glyph"), tl!("Name"), tl!("Advance"), tl!("Curves")] {
                ui.label(egui::RichText::new(h).size(11.0).color(t.text_dim));
            }
            ui.end_row();
            for g in &glyphs {
                let u = g.get("unicode").and_then(Value::as_str).unwrap_or("");
                let is_sel = u == selected;
                if ui.selectable_label(is_sel, egui::RichText::new(u).strong()).clicked() {
                    d.fields.insert("glyph".into(), json!(u));
                    d.fields.insert("newUnicode".into(), json!(u));
                    d.fields.insert("newName".into(), json!(g.get("name").and_then(Value::as_str).unwrap_or("")));
                    d.fields.insert("advance".into(), json!(g.get("advance").and_then(Value::as_f64).map(|a| format!("{a}")).unwrap_or_default()));
                }
                ui.label(egui::RichText::new(g.get("name").and_then(Value::as_str).unwrap_or("")).color(t.text));
                ui.label(
                    egui::RichText::new(g.get("advance").and_then(Value::as_f64).map(|a| format!("{a}")).unwrap_or_else(|| "—".into())).color(t.text),
                );
                ui.label(egui::RichText::new(if g.get("hasPath").and_then(Value::as_bool).unwrap_or(false) { "●" } else { "○" }).color(t.text_dim));
                ui.end_row();
            }
        });
    });
    ui.add_space(6.0);
    grid(ui, |ui| {
        label(ui, tl!("Unicode:"));
        form::text(ui, d, "newUnicode", 120.0);
        ui.end_row();
        label(ui, tl!("Glyph name:"));
        form::text(ui, d, "newName", 120.0);
        ui.end_row();
        label(ui, tl!("Advance:"));
        form::text(ui, d, "advance", 120.0);
        ui.end_row();
    });
    ui.horizontal_wrapped(|ui| {
        let unicode = d.str("newUnicode");
        let has = !unicode.is_empty();
        if ui.add_enabled(has, egui::Button::new(tl!("Add Glyph"))).clicked() {
            let adv = d.str("advance").trim().parse::<f64>().ok();
            let mut q = json!({ "font": font_id, "unicode": unicode, "name": d.str("newName") });
            if let Some(a) = adv {
                q["advance"] = json!(a);
            }
            actions.push(("svgFont.setGlyph", q));
        }
        let sel_has = !selected.is_empty();
        if ui.add_enabled(sel_has && has, egui::Button::new(tl!("Apply"))).clicked() {
            let adv = d.str("advance");
            let mut q = json!({ "font": font_id, "unicode": selected, "name": d.str("newName"), "newUnicode": unicode });
            q["advance"] = adv.trim().parse::<f64>().ok().map_or(Value::Null, |a| json!(a));
            actions.push(("svgFont.setGlyph", q));
        }
        if ui.add_enabled(sel_has, egui::Button::new(tl!("Remove Glyph"))).clicked() {
            actions.push(("svgFont.removeGlyph", json!({ "font": font_id, "unicode": selected })));
        }
        let has_sel = app.session.active().is_some_and(|s| !s.selection.is_empty());
        if ui.add_enabled(has && has_sel, egui::Button::new(tl!("Get Curves from Selection"))).clicked() {
            actions.push(("svgFont.glyphFromSelection", json!({ "font": font_id, "unicode": unicode })));
        }
        if ui.add_enabled(sel_has, egui::Button::new(tl!("Glyph to Objects"))).clicked() {
            actions.push((
                "svgFont.glyphToSelection",
                json!({ "font": font_id, "unicode": selected, "emSize": d.str("emSize").trim().parse::<f64>().unwrap_or(100.0) }),
            ));
        }
    });
    ui.add_space(6.0);
    // Kerning.
    ui.horizontal(|ui| {
        label(ui, tl!("Kerning:"));
        form::text(ui, d, "kernLeft", 40.0);
        form::text(ui, d, "kernRight", 40.0);
        form::text(ui, d, "kernK", 60.0);
        let ok = !d.str("kernLeft").is_empty() && !d.str("kernRight").is_empty();
        if ui.add_enabled(ok, egui::Button::new(tl!("Set Pair"))).clicked() {
            let k = d.str("kernK").trim().parse::<f64>().unwrap_or(0.0);
            actions.push(("svgFont.setKerning", json!({ "font": font_id, "left": d.str("kernLeft"), "right": d.str("kernRight"), "k": k })));
        }
    });
    let pairs: Vec<Value> = font.get("kerning").and_then(Value::as_array).cloned().unwrap_or_default();
    if !pairs.is_empty() {
        let shown: Vec<String> = pairs
            .iter()
            .take(12)
            .map(|k| {
                format!(
                    "{}{} {}",
                    k.get("left").and_then(Value::as_str).unwrap_or(""),
                    k.get("right").and_then(Value::as_str).unwrap_or(""),
                    k.get("k").and_then(Value::as_f64).unwrap_or(0.0)
                )
            })
            .collect();
        ui.label(egui::RichText::new(shown.join("   ")).size(11.0).color(t.text_dim));
    }
    ui.add_space(6.0);
    // Preview and the layer round trip.
    ui.horizontal(|ui| {
        label(ui, tl!("Preview:"));
        form::text(ui, d, "preview", 160.0);
        label(ui, tl!("Size:"));
        form::text(ui, d, "emSize", 50.0);
        if widgets::secondary_button(ui, tl!("Preview")).clicked() {
            actions.push((
                "svgFont.preview",
                json!({ "font": font_id, "text": d.str("preview"), "emSize": d.str("emSize").trim().parse::<f64>().unwrap_or(72.0) }),
            ));
        }
    });
    ui.horizontal(|ui| {
        if widgets::secondary_button(ui, tl!("Glyph Layers to Font")).clicked() {
            actions.push(("svgFont.fromLayers", json!({ "font": font_id })));
        }
        if widgets::secondary_button(ui, tl!("Font to Glyph Layers")).clicked() {
            actions.push(("svgFont.toLayers", json!({ "font": font_id, "emSize": d.str("emSize").trim().parse::<f64>().unwrap_or(100.0) })));
        }
    });
    run(app, d, actions);
    false
}

/// Run the actions clicked this frame; the font list is fetched again after them.
fn run(app: &mut VectorApp, d: &mut Dialog, actions: Vec<(&str, Value)>) {
    if actions.is_empty() {
        return;
    }
    for (cmd, p) in actions {
        match app.run(cmd, p) {
            Ok(r) => {
                if (cmd == "svgFont.create" || cmd == "svgFont.fromLayers")
                    && let Some(id) = r.get("id").and_then(Value::as_str)
                {
                    d.fields.insert("font".into(), json!(id));
                }
                if cmd == "svgFont.removeGlyph" {
                    d.fields.insert("glyph".into(), json!(""));
                }
            }
            Err(e) => app.status(e),
        }
    }
    // The dialog's copy of the fonts: fetched into `d`, which the frame writes back.
    if let Ok(r) = app.run("svgFont.list", json!({})) {
        let fonts = r.get("fonts").cloned().unwrap_or(json!([]));
        let cur = d.str("font");
        let ids: Vec<String> =
            fonts.as_array().map(|a| a.iter().filter_map(|f| f.get("id").and_then(Value::as_str)).map(str::to_string).collect()).unwrap_or_default();
        if !ids.contains(&cur) {
            d.fields.insert("font".into(), json!(ids.first().cloned().unwrap_or_default()));
        }
        d.fields.insert("fonts".into(), fonts);
    }
}

fn confirm(app: &mut VectorApp, _: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    Ok(Value::Null)
}
