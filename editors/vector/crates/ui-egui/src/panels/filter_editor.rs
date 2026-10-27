//! Filter Editor panel: the selected object's SVG filter graph (the `filter.graph` effect on the
//! object) as a list of primitives drawn as a connection graph (a line from every `result` to the
//! primitives reading it), with the filter region, a parameter editor for the selected primitive,
//! and buttons to add, duplicate, move and remove primitives. Every change goes through
//! `effect.setParams` (one undo step each); Add Filter applies the effect through `effect.apply`.

use egui::{Pos2, Sense, Stroke, Ui, vec2};
use redrob_vector_effects::FILTER_GRAPH;
use redrob_vector_effects::svgfilter::{FilterGraph, Kind, Primitive, primitive_kinds};
use serde_json::{Value, json};

use super::{first_selected, pstate, selection_len, set_pstate};
use crate::VectorApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

pub const ID: &str = "filterEditor";
/// Panel state: the selected primitive's index.
const SELECTED: &str = "filter-editor-selected";
const LABEL_W: f32 = 78.0;

/// The first selected object's filter graph: its index in the object's effect list and the graph.
fn current(app: &VectorApp) -> Option<(usize, FilterGraph)> {
    let n = first_selected(app)?;
    let (i, e) = n.appearance.effects.iter().enumerate().find(|(_, e)| e.id == FILTER_GRAPH)?;
    Some((i, FilterGraph::from_params(&e.params)))
}

/// Write `g` back as the effect at `index` of the object's own list.
fn commit(app: &mut VectorApp, index: usize, g: &FilterGraph) {
    if let Err(e) = app.run("effect.setParams", json!({ "index": index, "item": Value::Null, "params": g.to_params() })) {
        app.ui.status = e;
    }
}

/// Does the first selected object carry a filter graph?
pub fn has_filter(app: &VectorApp) -> bool {
    current(app).is_some()
}

/// Remove the first selected object's filter graph.
pub fn remove_filter(app: &mut VectorApp) {
    if let Some((index, _)) = current(app)
        && let Err(e) = app.run("effect.remove", json!({ "index": index, "item": Value::Null }))
    {
        app.ui.status = e;
    }
}

/// Every filter graph in the document: (object id, object label, the graph), in paint order.
fn document_filters(app: &VectorApp) -> Vec<(u64, String, FilterGraph)> {
    let mut out = vec![];
    if let Some(st) = app.session.active() {
        st.doc.walk(|n| {
            for e in n.appearance.effects.iter().filter(|e| e.id == FILTER_GRAPH) {
                let g = FilterGraph::from_params(&e.params);
                let label = n.name.clone().unwrap_or_else(|| format!("{} {}", n.kind_label(), n.id.0));
                out.push((n.id.0, label, g));
            }
        });
    }
    out
}

/// Panel state: the document filter list is open.
const DOC_LIST_OPEN: &str = "filter-editor-doc-list";
/// Panel state: the primitive a connection is being dragged from.
const DRAG_FROM: &str = "filter-editor-drag-from";

pub fn show(app: &mut VectorApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // The document's filters: apply one to the selection, or a copy of it.
    let open: bool = pstate(ui.ctx(), DOC_LIST_OPEN);
    if widgets::section_toggle(ui, tl!("Document Filters"), open) {
        set_pstate(ui.ctx(), DOC_LIST_OPEN, !open);
    }
    if open {
        let filters = document_filters(app);
        let mut apply: Option<(FilterGraph, bool)> = None;
        if filters.is_empty() {
            widgets::dim_label(ui, tl!("The document has no filters yet."));
        }
        for (_, label, g) in &filters {
            ui.horizontal(|ui| {
                let name = if g.name.is_empty() { tl!("Filter").to_string() } else { g.name.clone() };
                ui.label(egui::RichText::new(format!("{name}  ({label}, {})", g.primitives.len())).color(t.text).size(12.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, "copy", tl!("Duplicate Filter"), false, 18.0).clicked() {
                        apply = Some((g.clone(), true));
                    }
                    if widgets::icon_button_enabled(ui, "plus", tl!("Apply to Selection"), false, selection_len(app) > 0, 18.0).clicked() {
                        apply = Some((g.clone(), false));
                    }
                });
            });
        }
        if let Some((mut g, duplicate)) = apply {
            if duplicate {
                g.name = if g.name.is_empty() { tl!("Filter copy").to_string() } else { format!("{} copy", g.name) };
            }
            let r = match current(app) {
                Some((index, _)) => app.run("effect.setParams", json!({ "index": index, "item": Value::Null, "params": g.to_params() })),
                None => app.run("effect.apply", json!({ "effect": FILTER_GRAPH, "item": Value::Null, "params": g.to_params() })),
            };
            if let Err(e) = r {
                app.ui.status = e;
            }
        }
        widgets::divider(ui);
    }
    let Some((index, mut g)) = current(app) else {
        if selection_len(app) == 0 {
            super::empty_state(ui, "dc-fx", "No Selection", "Select an object to edit its SVG filter.");
        } else {
            super::empty_state(ui, "dc-fx", "No Filter", "The object has no SVG filter graph yet.");
            ui.vertical_centered(|ui| {
                if widgets::primary_button(ui, tl!("Add Filter")).clicked() {
                    let p = json!({ "effect": FILTER_GRAPH, "item": Value::Null, "params": { "primitives": [] } });
                    if let Err(e) = app.run("effect.apply", p) {
                        app.ui.status = e;
                    }
                }
            });
        }
        return;
    };
    let mut changed = false;
    // The filter's name and colour space.
    widgets::label_row(ui, tl!("Name:"), LABEL_W, |ui| {
        if let Some(s) = widgets::exact_text_field(ui, "filter-name", &g.name, 140.0) {
            g.name = s.trim().to_string();
            changed = true;
        }
    });
    widgets::label_row(ui, tl!("Colour Space:"), LABEL_W, |ui| {
        let cur = if g.linear_rgb { "linearRGB" } else { "sRGB" };
        if let Some(k) = widgets::dropdown_names(ui, "filter-interp", cur, &["sRGB", "linearRGB"], 110.0) {
            g.linear_rgb = k == 1;
            changed = true;
        }
    });
    // The filter region, as percentages of the object's bounds.
    widgets::subheader(ui, tl!("Filter Region"));
    ui.horizontal(|ui| {
        for (k, label) in [(0, "X:"), (1, "Y:"), (2, "W:"), (3, "H:")] {
            widgets::dim_label(ui, tl!(label));
            if let Some(v) = widgets::plain_field(ui, ("filter-region", k), g.region[k] * 100.0, "%", 1, 52.0) {
                let v = v / 100.0;
                if k < 2 || v > 0.0 {
                    g.region[k] = v;
                    changed = true;
                }
            }
        }
    });
    widgets::divider(ui);
    // The primitives as rows, their connections drawn down the left margin.
    let selected: Option<usize> = pstate(ui.ctx(), SELECTED);
    let names: Vec<String> = g.primitives.iter().map(|p| p.result.clone()).collect();
    let mut rows: Vec<egui::Rect> = vec![];
    let margin = 16.0;
    let mut action: Option<Action> = None;
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("filter-rows").max_height(180.0).show(ui, |ui| {
            if g.primitives.is_empty() {
                widgets::dim_label(ui, tl!("No primitives. Add one below."));
            }
            for (i, p) in g.primitives.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
                if resp.clicked() {
                    set_pstate(ui.ctx(), SELECTED, Some(i));
                }
                // Dragging a row's result onto a later row connects them: that row reads this one.
                if resp.drag_started() {
                    set_pstate(ui.ctx(), DRAG_FROM, Some(i));
                }
                let dragging: Option<usize> = pstate(ui.ctx(), DRAG_FROM);
                if let Some(from) = dragging {
                    let over = ui.input(|inp| inp.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
                    if over && from != i {
                        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
                    }
                    if over && from < i && ui.input(|inp| inp.pointer.any_released()) {
                        action = Some(Action::Connect(from, i));
                    }
                }
                let is_sel = selected == Some(i);
                if is_sel {
                    ui.painter().rect_filled(rect, 2.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 2.0, t.hover);
                }
                let label = format!("{}  {}", tl!(p.kind.label()), p.kind.element());
                let text_pos = Pos2::new(rect.left() + margin + 6.0, rect.center().y);
                ui.painter().text(text_pos, egui::Align2::LEFT_CENTER, label, egui::TextStyle::Body.resolve(ui.style()), t.text);
                if !p.result.is_empty() {
                    let at = Pos2::new(rect.right() - 6.0, rect.center().y);
                    ui.painter().text(
                        at,
                        egui::Align2::RIGHT_CENTER,
                        format!("→ {}", p.result),
                        egui::TextStyle::Small.resolve(ui.style()),
                        t.text_dim,
                    );
                }
                rows.push(rect);
            }
        });
    });
    if ui.input(|inp| inp.pointer.any_released()) {
        set_pstate(ui.ctx(), DRAG_FROM, Option::<usize>::None);
    }
    // Connections: a result name to each later primitive reading it; the implicit previous-result
    // link as a short tick.
    let painter = ui.painter();
    for (i, p) in g.primitives.iter().enumerate() {
        let Some(to) = rows.get(i) else { continue };
        for (slot, input) in p.kind.inputs().iter().enumerate() {
            let from_row = match input.as_str() {
                "" => i.checked_sub(1),
                name if redrob_vector_effects::svgfilter::STANDARD_INPUTS.contains(&name) => None,
                name => names.iter().take(i).rposition(|n| n == name),
            };
            let x = to.left() + 4.0 + slot as f32 * 4.0;
            let end = Pos2::new(to.left() + margin, to.center().y);
            match from_row.and_then(|r| rows.get(r)) {
                Some(from) => {
                    let start = Pos2::new(from.left() + margin, from.center().y);
                    let mid1 = Pos2::new(x, start.y);
                    let mid2 = Pos2::new(x, end.y);
                    painter.line_segment([start, mid1], Stroke::new(1.0, t.accent));
                    painter.line_segment([mid1, mid2], Stroke::new(1.0, t.accent));
                    painter.line_segment([mid2, end], Stroke::new(1.0, t.accent));
                }
                None if !input.is_empty() => {
                    // From the source: a dot at the margin.
                    painter.circle_filled(Pos2::new(x, end.y), 2.0, t.text_dim);
                    painter.line_segment([Pos2::new(x, end.y), end], Stroke::new(1.0, t.text_dim));
                }
                None => {}
            }
        }
    }
    // Row buttons: add, duplicate, up, down, remove.
    ui.horizontal(|ui| {
        let resp = widgets::icon_button(ui, "plus", tl!("Add Primitive"), false, 22.0);
        egui::Popup::menu(&resp).show(|ui| {
            widgets::menu_scroll(ui, |ui| {
                ui.set_min_width(180.0);
                for (label, element, kind) in primitive_kinds() {
                    if menu_item(ui, &format!("{} ({element})", tl!(label)), true, false) {
                        action = Some(Action::Add(kind));
                    }
                }
            });
        });
        let has = selected.is_some_and(|i| i < g.primitives.len());
        if widgets::icon_button_enabled(ui, "copy", tl!("Duplicate Primitive"), false, has, 22.0).clicked() {
            action = Some(Action::Duplicate);
        }
        if widgets::icon_button_enabled(ui, "dc-arrow-up", tl!("Move Up"), false, has && selected > Some(0), 22.0).clicked() {
            action = Some(Action::Move(-1));
        }
        if widgets::icon_button_enabled(
            ui,
            "dc-arrow-down",
            tl!("Move Down"),
            false,
            has && selected.is_some_and(|i| i + 1 < g.primitives.len()),
            22.0,
        )
        .clicked()
        {
            action = Some(Action::Move(1));
        }
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Remove Primitive"), false, has, 22.0).clicked() {
            action = Some(Action::Remove);
        }
        ui.add_space(8.0);
        if widgets::icon_button(ui, "dc-clear", tl!("Remove Filter"), false, 22.0).clicked() {
            action = Some(Action::RemoveFilter);
        }
    });
    match action {
        Some(Action::Add(kind)) => {
            let at = selected.map_or(g.primitives.len(), |i| (i + 1).min(g.primitives.len()));
            g.primitives.insert(at, Primitive::new("", kind));
            set_pstate(ui.ctx(), SELECTED, Some(at));
            changed = true;
        }
        Some(Action::Duplicate) => {
            if let Some(p) = selected.and_then(|i| g.primitives.get(i).cloned()) {
                let at = selected.unwrap_or(0) + 1;
                let mut p = p;
                p.result.clear();
                g.primitives.insert(at, p);
                set_pstate(ui.ctx(), SELECTED, Some(at));
                changed = true;
            }
        }
        Some(Action::Move(d)) => {
            if let Some(i) = selected {
                let j = (i as i64 + d).clamp(0, g.primitives.len() as i64 - 1) as usize;
                if j != i && i < g.primitives.len() {
                    g.primitives.swap(i, j);
                    set_pstate(ui.ctx(), SELECTED, Some(j));
                    changed = true;
                }
            }
        }
        Some(Action::Remove) => {
            if let Some(i) = selected.filter(|i| *i < g.primitives.len()) {
                g.primitives.remove(i);
                set_pstate(ui.ctx(), SELECTED, i.checked_sub(1).or(if g.primitives.is_empty() { None } else { Some(0) }));
                changed = true;
            }
        }
        Some(Action::Connect(from, to)) => {
            // The source gets a name when it has none; the target's first input (a merge: a new
            // input) reads it.
            if g.primitives[from].result.is_empty() {
                g.primitives[from].result = format!("result{}", from + 1);
            }
            let name = g.primitives[from].result.clone();
            match &mut g.primitives[to].kind {
                Kind::Merge { inputs } => inputs.push(name),
                k => {
                    if let Some(first) = k.inputs_mut().into_iter().next() {
                        *first = name;
                    }
                }
            }
            set_pstate(ui.ctx(), SELECTED, Some(to));
            changed = true;
        }
        Some(Action::RemoveFilter) => {
            if let Err(e) = app.run("effect.remove", json!({ "index": index, "item": Value::Null })) {
                app.ui.status = e;
            }
            return;
        }
        None => {}
    }
    // The selected primitive's parameters.
    if let Some(i) = selected.filter(|i| *i < g.primitives.len()) {
        widgets::divider(ui);
        let earlier: Vec<String> = names.iter().take(i).filter(|n| !n.is_empty()).cloned().collect();
        if let Some(p) = g.primitives.get_mut(i) {
            widgets::subheader(ui, &format!("{} ({})", tl!(p.kind.label()), p.kind.element()));
            widgets::label_row(ui, tl!("Result:"), LABEL_W, |ui| {
                if let Some(s) = widgets::exact_text_field(ui, ("filter-result", i), &p.result, 120.0) {
                    p.result = s.trim().replace(' ', "_");
                    changed = true;
                }
            });
            // The primitive subregion: off (the filter region) or x, y, w, h as percentages.
            widgets::label_row(ui, tl!("Subregion:"), LABEL_W, |ui| {
                let on = p.subregion.is_some();
                if widgets::check(ui, "", on, true) {
                    p.subregion = if on { None } else { Some(g.region) };
                    changed = true;
                }
                if let Some(sub) = &mut p.subregion {
                    for (k, label) in [(0, "X:"), (1, "Y:"), (2, "W:"), (3, "H:")] {
                        widgets::dim_label(ui, tl!(label));
                        if let Some(v) = widgets::plain_field(ui, ("filter-sub-region", i, k), sub[k] * 100.0, "%", 0, 44.0) {
                            let v = v / 100.0;
                            if k < 2 || v > 0.0 {
                                sub[k] = v;
                                changed = true;
                            }
                        }
                    }
                }
            });
            // Inputs: the previous result, the sources, or an earlier primitive's result.
            let mut options: Vec<String> = vec![String::new()];
            options.extend(redrob_vector_effects::svgfilter::STANDARD_INPUTS.iter().map(|s| s.to_string()));
            options.extend(earlier);
            let labels: Vec<String> = options.iter().map(|o| if o.is_empty() { tl!("Previous Result").to_string() } else { o.clone() }).collect();
            let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let n_inputs = p.kind.inputs().len();
            let is_merge = matches!(p.kind, Kind::Merge { .. });
            for slot in 0..n_inputs {
                let current = p.kind.inputs().get(slot).map(|s| (*s).clone()).unwrap_or_default();
                let cur_label = if current.is_empty() { tl!("Previous Result").to_string() } else { current.clone() };
                let name = match (slot, is_merge) {
                    (_, true) => format!("{} {}:", tl!("Input"), slot + 1),
                    (0, false) => format!("{}:", tl!("Input")),
                    _ => format!("{} 2:", tl!("Input")),
                };
                let mut picked = None;
                widgets::label_row(ui, &name, LABEL_W, |ui| {
                    picked = widgets::dropdown_names(ui, ("filter-in", i, slot), &cur_label, &label_refs, 140.0);
                    if is_merge && widgets::icon_button_enabled(ui, "x", tl!("Remove Input"), false, n_inputs > 1, 18.0).clicked() {
                        picked = Some(usize::MAX);
                    }
                });
                match picked {
                    Some(usize::MAX) => {
                        if let Kind::Merge { inputs } = &mut p.kind
                            && slot < inputs.len()
                        {
                            inputs.remove(slot);
                            changed = true;
                        }
                    }
                    Some(k) => {
                        if let (Some(slot_ref), Some(v)) = (p.kind.inputs_mut().get_mut(slot), options.get(k)) {
                            **slot_ref = v.clone();
                            changed = true;
                        }
                    }
                    None => {}
                }
            }
            if let Kind::Merge { inputs } = &mut p.kind
                && widgets::flat_button(ui, tl!("Add Input"), 90.0).clicked()
            {
                inputs.push("SourceGraphic".into());
                changed = true;
            }
            changed |= param_fields(ui, i, &mut p.kind);
        }
    }
    if changed {
        commit(app, index, &g);
    }
}

enum Action {
    Add(Kind),
    /// Connect primitive `from`'s result to primitive `to`'s input (a dragged connection).
    Connect(usize, usize),
    Duplicate,
    Move(i64),
    Remove,
    RemoveFilter,
}

/// The choices of a primitive's text attribute (by element and key).
fn choices(element: &str, key: &str) -> Option<&'static [&'static str]> {
    Some(match (element, key) {
        ("feComposite", "operator") => &["over", "in", "out", "atop", "xor", "arithmetic"],
        ("feMorphology", "operator") => &["erode", "dilate"],
        ("feColorMatrix", "mode") => &["matrix", "saturate", "hueRotate", "luminanceToAlpha"],
        ("feBlend", "mode") => &[
            "normal",
            "multiply",
            "screen",
            "overlay",
            "darken",
            "lighten",
            "color-dodge",
            "color-burn",
            "hard-light",
            "soft-light",
            "difference",
            "exclusion",
            "hue",
            "saturation",
            "color",
            "luminosity",
        ],
        ("feConvolveMatrix", "edgeMode") => &["duplicate", "wrap", "none"],
        ("feDisplacementMap", "xChannel" | "yChannel") => &["R", "G", "B", "A"],
        (_, "lightType") => &["distant", "point", "spot"],
        (_, "funcType") => &["identity", "table", "discrete", "linear", "gamma"],
        _ => return None,
    })
}

/// The editable attributes of `kind` (everything but its type, inputs and result), edited in
/// place through its JSON form. Returns whether a value changed.
fn param_fields(ui: &mut Ui, i: usize, kind: &mut Kind) -> bool {
    let element = kind.element();
    let Ok(Value::Object(mut m)) = serde_json::to_value(&*kind) else { return false };
    let mut changed = false;
    let skip = |k: &str| matches!(k, "type" | "in" | "in2" | "inputs");
    let keys: Vec<String> = m.keys().filter(|k| !skip(k)).cloned().collect();
    for key in keys {
        let Some(v) = m.get(&key).cloned() else { continue };
        match v {
            Value::Object(inner) => {
                // A light or a transfer function: its type, then its own fields.
                let (type_key, label) = if key == "light" { ("lightType", tl!("Light")) } else { ("funcType", key.as_str()) };
                let mut inner = inner;
                let ty = inner.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
                let mut new_inner = None;
                widgets::label_row(ui, &format!("{label}:"), LABEL_W, |ui| {
                    let opts = choices(element, type_key).unwrap_or(&[]);
                    if let Some(k) = widgets::dropdown_names(ui, ("filter-sub", i, &key), &ty, opts, 120.0)
                        && let Some(t) = opts.get(k)
                    {
                        new_inner = Some(sub_default(&key, t));
                    }
                });
                if let Some(n) = new_inner {
                    m.insert(key.clone(), n);
                    changed = true;
                    continue;
                }
                let sub_keys: Vec<String> = inner.keys().filter(|k| *k != "type").cloned().collect();
                for sk in sub_keys {
                    let Some(sv) = inner.get(&sk).cloned() else { continue };
                    if let Some(nv) = scalar_field(ui, (i, &key, &sk), &format!("  {}:", humanize(&sk)), element, &sk, sv) {
                        inner.insert(sk, nv);
                        changed = true;
                    }
                }
                m.insert(key.clone(), Value::Object(inner));
            }
            other => {
                if let Some(nv) = scalar_field(ui, (i, &key, ""), &format!("{}:", humanize(&key)), element, &key, other) {
                    m.insert(key.clone(), nv);
                    changed = true;
                }
            }
        }
    }
    if changed && let Ok(k) = serde_json::from_value::<Kind>(Value::Object(m)) {
        *kind = k;
        return true;
    }
    false
}

/// A fresh light or transfer function of type `ty`.
fn sub_default(key: &str, ty: &str) -> Value {
    if key == "light" {
        match ty {
            "point" => json!({ "type": "point", "x": 0.0, "y": 0.0, "z": 50.0 }),
            "spot" => json!({ "type": "spot", "x": 0.0, "y": 0.0, "z": 50.0, "px": 50.0, "py": 50.0, "pz": 0.0, "exponent": 1.0, "cone": 0.0 }),
            _ => json!({ "type": "distant", "azimuth": 45.0, "elevation": 45.0 }),
        }
    } else {
        match ty {
            "table" => json!({ "type": "table", "values": [0.0, 1.0] }),
            "discrete" => json!({ "type": "discrete", "values": [0.0, 0.5, 1.0] }),
            "linear" => json!({ "type": "linear", "slope": 1.0, "intercept": 0.0 }),
            "gamma" => json!({ "type": "gamma", "amplitude": 1.0, "exponent": 1.0, "offset": 0.0 }),
            _ => json!({ "type": "identity" }),
        }
    }
}

/// One scalar attribute: a number field, a checkbox, a choice, a colour or a list of numbers;
/// returns the new value when it changed.
fn scalar_field(ui: &mut Ui, id: (usize, &str, &str), label: &str, element: &str, key: &str, v: Value) -> Option<Value> {
    let mut out = None;
    widgets::label_row(ui, label, LABEL_W, |ui| {
        match v {
            Value::Number(n) => {
                let is_int = n.is_i64() || n.is_u64();
                if let Some(x) = widgets::plain_field(ui, ("filter-num", id), n.as_f64().unwrap_or(0.0), "", if is_int { 0 } else { 2 }, 90.0) {
                    out = Some(if is_int { json!(x.round() as i64) } else { json!(x) });
                }
            }
            Value::Bool(b) => {
                if widgets::check(ui, "", b, true) {
                    out = Some(json!(!b));
                }
            }
            Value::Null => {
                // An optional whole number (convolve targets): blank is the centre.
                if let Some(x) = widgets::plain_field(ui, ("filter-opt", id), -1.0, "", 0, 90.0)
                    && x >= 0.0
                {
                    out = Some(json!(x.round() as u64));
                }
            }
            Value::String(s) => {
                if let Some(opts) = choices(element, key) {
                    if let Some(k) = widgets::dropdown_names(ui, ("filter-choice", id), &s, opts, 120.0)
                        && let Some(o) = opts.get(k)
                    {
                        out = Some(json!(o));
                    }
                } else if key == "color" {
                    if let Some(h) = widgets::hex_field(ui, ("filter-color", id), &s) {
                        out = Some(json!(h));
                    }
                } else if let Some(t) = widgets::exact_text_field(ui, ("filter-text", id), &s, 140.0) {
                    out = Some(json!(t));
                }
            }
            Value::Array(a) => {
                let text = a.iter().filter_map(Value::as_f64).map(|x| format!("{x}")).collect::<Vec<_>>().join(" ");
                let rows = if a.len() > 8 { 3 } else { 1 };
                if let Some(t) = widgets::text_field(ui, ("filter-list", id), Some(&text), 160.0, rows) {
                    let nums: Vec<f64> = t.split([' ', ',', '\n']).filter_map(|x| x.trim().parse().ok()).collect();
                    out = Some(json!(nums));
                }
            }
            Value::Object(_) => {}
        }
    });
    out
}

/// `camelCase` → `Camel Case`.
fn humanize(k: &str) -> String {
    let mut s = String::new();
    for (i, c) in k.chars().enumerate() {
        if i == 0 {
            s.extend(c.to_uppercase());
        } else if c.is_ascii_uppercase() {
            s.push(' ');
            s.push(c);
        } else {
            s.push(c);
        }
    }
    s
}

pub fn menu(app: &mut VectorApp, ui: &mut Ui) {
    let has = current(app).is_some();
    if menu_item(ui, tl!("Add Filter"), !has && selection_len(app) > 0, false) {
        let p = json!({ "effect": FILTER_GRAPH, "item": Value::Null, "params": { "primitives": [] } });
        if let Err(e) = app.run("effect.apply", p) {
            app.ui.status = e;
        }
    }
    if menu_item(ui, tl!("Drop Shadow Preset"), has, false)
        && let Some((index, mut g)) = current(app)
    {
        g.primitives = vec![
            Primitive::new("blur", Kind::GaussianBlur { input: "SourceAlpha".into(), sd_x: 3.0, sd_y: 3.0 }),
            Primitive::new("shadow", Kind::Offset { input: "blur".into(), dx: 4.0, dy: 4.0 }),
            Primitive::new("flood", Kind::Flood { color: "#000000".into(), opacity: 0.6 }),
            Primitive::new(
                "paint",
                Kind::Composite {
                    input: "flood".into(),
                    in2: "shadow".into(),
                    operator: redrob_vector_effects::svgfilter::CompositeOp::In,
                    k1: 0.0,
                    k2: 0.0,
                    k3: 0.0,
                    k4: 0.0,
                },
            ),
            Primitive::new("", Kind::Merge { inputs: vec!["paint".into(), "SourceGraphic".into()] }),
        ];
        commit(app, index, &g);
    }
    if menu_item(ui, tl!("Remove Filter"), has, false)
        && let Some((index, _)) = current(app)
        && let Err(e) = app.run("effect.remove", json!({ "index": index, "item": Value::Null }))
    {
        app.ui.status = e;
    }
}
