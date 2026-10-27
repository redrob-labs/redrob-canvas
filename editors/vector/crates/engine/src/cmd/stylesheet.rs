//! Selectors and CSS (`css.rules`, `css.addRule`, `css.setRule`, `css.removeRule`, `css.assign`,
//! `css.unassign`, `css.applyRules`): the document's own stylesheet, rules objects share through
//! their classes. A class is kept on the object as its `vc-class` data (SVG `class`); applying a
//! rule gives every object it selects the fill, stroke, stroke options, opacity, blend and
//! visibility its declarations describe, through the ordinary paint, stroke and object commands.

use redrob_vector_doc::{CssRule, Node, NodeId, NodeKind};
use serde_json::{Value, json};

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "css.rules",
            "Stylesheet",
            [],
            None,
            "{} → {rules: [{index, selector, declarations: [[prop, value]…], css}]} the document's stylesheet (Selectors and CSS)",
            has_doc,
            |s, _| Ok(json!({ "rules": s.doc()?.doc.stylesheet.iter().enumerate().map(|(i, r)| rule_json(i, r)).collect::<Vec<_>>() }))
        ),
        cmd!(
            "css.addRule",
            "Add CSS Rule",
            [],
            None,
            "{selector?: \".name\"|element|\"*\" (default: a fresh .class1…), declarations?: {prop: value} | \"prop: value; …\" | [[prop, value]…], css?: \"selector { … }\" (a whole rule instead), assign?: bool (true: the selected objects get the rule's class)} add a rule to the stylesheet → {index, selector}",
            has_doc,
            add_rule
        ),
        cmd!(
            "css.setRule",
            "Edit CSS Rule",
            [],
            None,
            "{index: n, selector?, declarations?: {prop: value} | \"prop: value; …\" | [[prop, value]…], css?: \"selector { … }\", apply?: bool (true: the objects the rule selects take its declarations)} change a rule → {index}",
            has_doc,
            set_rule
        ),
        cmd!("css.removeRule", "Remove CSS Rule", [], None, "{index: n} remove a rule from the stylesheet → {index}", has_doc, remove_rule),
        cmd!(
            "css.assign",
            "Assign CSS Class",
            [],
            None,
            "{class: name (without the dot), ids?|id? (default: the selection), apply?: bool (true: the matching rules' declarations are applied to them)} give the objects a class → {ids}",
            has_selection,
            |s, p| assign(s, p, true)
        ),
        cmd!(
            "css.unassign",
            "Remove CSS Class",
            [],
            None,
            "{class: name, ids?|id? (default: the selection)} take a class off the objects → {ids}",
            has_selection,
            |s, p| assign(s, p, false)
        ),
        cmd!(
            "css.applyRules",
            "Apply Stylesheet",
            [],
            None,
            "{index?: n (one rule; default: every rule in order), ids?: [..] (default: every object)} give the objects each rule selects the fill, stroke, stroke-width, stroke-linecap, stroke-linejoin, stroke-dasharray, opacity, mix-blend-mode and visibility/display its declarations describe → {applied: n objects}",
            has_doc,
            apply_rules
        ),
    ]
}

fn rule_json(i: usize, r: &CssRule) -> Value {
    json!({ "index": i, "selector": r.selector, "declarations": r.declarations, "css": r.text() })
}

/// The declarations `p` gives: an object, a `prop: value;` string, or pairs.
fn declarations_param(p: &Value) -> Option<Vec<(String, String)>> {
    match p.get("declarations")? {
        Value::Object(m) => Some(m.iter().map(|(k, v)| (k.clone(), v.as_str().map_or_else(|| v.to_string(), str::to_string))).collect()),
        Value::String(s) => Some(CssRule::parse_declarations(s)),
        Value::Array(a) => Some(
            a.iter()
                .filter_map(|pair| {
                    let arr = pair.as_array()?;
                    Some((arr.first()?.as_str()?.to_string(), arr.get(1)?.as_str()?.to_string()))
                })
                .collect(),
        ),
        _ => None,
    }
}

/// Fill `r` from `css`, `selector` and `declarations` params (those given).
fn fill_rule(cmd: &str, r: &mut CssRule, p: &Value) -> Result<()> {
    if let Some(css) = str_param(p, "css") {
        *r = CssRule::parse_rule(css).ok_or_else(|| bad(cmd, "css must read `selector { prop: value; … }`"))?;
    }
    if let Some(sel) = str_param(p, "selector") {
        let sel = sel.trim();
        if sel.is_empty() || sel.contains('{') || sel.contains('}') {
            return Err(bad(cmd, "selector must be a class (.name), an element name or *"));
        }
        r.selector = sel.to_string();
    }
    if let Some(d) = declarations_param(p) {
        r.declarations = d;
    }
    Ok(())
}

/// A class name no rule uses yet: `class1`, `class2`…
fn fresh_class(rules: &[CssRule]) -> String {
    (1..).map(|i| format!("class{i}")).find(|c| !rules.iter().any(|r| r.selector.trim_start_matches('.') == c)).unwrap_or_default()
}

fn add_rule(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "css.addRule";
    if s.doc()?.doc.stylesheet.len() >= 256 {
        return Err(bad(C, "a stylesheet holds at most 256 rules"));
    }
    let mut r = CssRule { selector: format!(".{}", fresh_class(&s.doc()?.doc.stylesheet)), declarations: vec![] };
    fill_rule(C, &mut r, p)?;
    let selector = r.selector.clone();
    let index = s.edit("Add CSS Rule", |d, _| {
        d.stylesheet.push(r);
        Ok(d.stylesheet.len() - 1)
    })?;
    if bool_or(p, "assign", false)
        && let Some(class) = selector.strip_prefix('.')
    {
        assign(s, &json!({ "class": class }), true)?;
    }
    Ok(json!({ "index": index, "selector": selector }))
}

fn rule_index(s: &Session, p: &Value, cmd: &str) -> Result<usize> {
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "missing `index`"))? as usize;
    if index >= s.doc()?.doc.stylesheet.len() {
        return Err(bad(cmd, format!("no rule {index}")));
    }
    Ok(index)
}

fn set_rule(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "css.setRule";
    let index = rule_index(s, p, C)?;
    let mut r = s.doc()?.doc.stylesheet[index].clone();
    fill_rule(C, &mut r, p)?;
    s.edit("Edit CSS Rule", |d, _| {
        if let Some(slot) = d.stylesheet.get_mut(index) {
            *slot = r;
        }
        Ok(())
    })?;
    if bool_or(p, "apply", false) {
        apply_rules(s, &json!({ "index": index }))?;
    }
    Ok(json!({ "index": index }))
}

fn remove_rule(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "css.removeRule";
    let index = rule_index(s, p, C)?;
    s.edit("Remove CSS Rule", |d, _| {
        d.stylesheet.remove(index);
        Ok(())
    })?;
    Ok(json!({ "index": index }))
}

/// The classes of `n`, from its `vc-class` data.
fn classes_of(n: &Node) -> Vec<String> {
    n.extras()
        .attrs
        .as_deref()
        .and_then(|a| a.data.iter().find(|(k, _)| k == CssRule::CLASS_KEY))
        .map(|(_, v)| v.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

/// The SVG element name an object answers to in a selector.
fn element_of(n: &Node) -> &'static str {
    match &n.kind {
        NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) => "g",
        NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Mesh(_) => "path",
        NodeKind::Text(_) => "text",
        NodeKind::Image(_) | NodeKind::PlacedDocument(_) => "image",
        NodeKind::SymbolInstance { .. } => "use",
    }
}

fn assign(s: &mut Session, p: &Value, add: bool) -> Result<Value> {
    let cmd = if add { "css.assign" } else { "css.unassign" };
    let class =
        str_param(p, "class").map(|c| c.trim().trim_start_matches('.').to_string()).filter(|c| !c.is_empty() && !c.contains(char::is_whitespace));
    let class = class.ok_or_else(|| bad(cmd, "missing `class` (a name without spaces)"))?;
    let ids = checked_ids_or_selection(s, p, cmd)?;
    if ids.is_empty() {
        return Err(bad(cmd, "nothing selected"));
    }
    s.edit(if add { "Assign CSS Class" } else { "Remove CSS Class" }, |d, _| {
        for id in &ids {
            let Some(n) = d.node_mut(*id) else { continue };
            let mut classes = classes_of(n);
            classes.retain(|c| *c != class);
            if add {
                classes.push(class.clone());
            }
            n.edit_attrs(|a| {
                a.data.retain(|(k, _)| k != CssRule::CLASS_KEY);
                if !classes.is_empty() {
                    a.data.push((CssRule::CLASS_KEY.to_string(), classes.join(" ")));
                }
            });
        }
        Ok(())
    })?;
    if add && bool_or(p, "apply", false) {
        apply_rules(s, &json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))?;
    }
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn apply_rules(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "css.applyRules";
    let only = p.get("index").and_then(Value::as_u64).map(|i| i as usize);
    let rules: Vec<CssRule> = s.doc()?.doc.stylesheet.clone();
    if let Some(i) = only
        && i >= rules.len()
    {
        return Err(bad(C, format!("no rule {i}")));
    }
    // The objects: the ones given, else every object of every layer.
    let ids: Vec<NodeId> = if p.get("ids").is_some() {
        checked_ids_param(s, p, "ids", C)?
    } else {
        let mut all = vec![];
        s.doc()?.doc.walk(|n| {
            if !n.is_layer() {
                all.push(n.id);
            }
        });
        all
    };
    let mut applied = 0usize;
    for (ri, r) in rules.iter().enumerate() {
        if only.is_some_and(|i| i != ri) || r.declarations.is_empty() {
            continue;
        }
        let targets: Vec<u64> = {
            let d = &s.doc()?.doc;
            ids.iter().filter(|id| d.node(**id).is_some_and(|n| r.matches(&classes_of(n), element_of(n)))).map(|id| id.0).collect()
        };
        if targets.is_empty() {
            continue;
        }
        applied += targets.len();
        apply_declarations(s, r, &targets)?;
    }
    Ok(json!({ "applied": applied }))
}

/// Run the paint, stroke and object commands a rule's declarations stand for, on `ids`.
fn apply_declarations(s: &mut Session, r: &CssRule, ids: &[u64]) -> Result<()> {
    let paint = |v: &str| -> Value {
        if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("transparent") {
            json!({ "none": true, "ids": ids })
        } else {
            json!({ "color": v, "ids": ids })
        }
    };
    let mut stroke = serde_json::Map::new();
    let mut props = serde_json::Map::new();
    let mut transparency = serde_json::Map::new();
    for (k, v) in &r.declarations {
        let v = v.trim();
        match k.to_ascii_lowercase().as_str() {
            "fill" => {
                s.execute("paint.setFill", &paint(v))?;
            }
            "stroke" => {
                s.execute("paint.setStroke", &paint(v))?;
            }
            "stroke-width" => {
                if let Some(w) = length(v) {
                    stroke.insert("weight".into(), json!(w));
                }
            }
            "stroke-linecap" => {
                stroke.insert("cap".into(), json!(v.to_ascii_lowercase()));
            }
            "stroke-linejoin" => {
                stroke.insert("join".into(), json!(v.to_ascii_lowercase()));
            }
            "stroke-dasharray" => {
                let dashes: Vec<f64> = v.split(|c: char| c == ',' || c.is_whitespace()).filter_map(length).collect();
                stroke.insert("dash".into(), if dashes.is_empty() || v.eq_ignore_ascii_case("none") { Value::Null } else { json!(dashes) });
            }
            "opacity" => {
                if let Some(o) = v.strip_suffix('%').map_or_else(|| v.parse::<f64>().ok().map(|o| o * 100.0), |pct| pct.trim().parse::<f64>().ok()) {
                    transparency.insert("opacity".into(), json!(o.clamp(0.0, 100.0)));
                }
            }
            "mix-blend-mode" => {
                // transparency.set reads the CSS keyword as it reads a label.
                transparency.insert("blend".into(), json!(v));
            }
            "visibility" => {
                props.insert("visible".into(), json!(!v.eq_ignore_ascii_case("hidden")));
            }
            "display" => {
                props.insert("visible".into(), json!(!v.eq_ignore_ascii_case("none")));
            }
            _ => {}
        }
    }
    if !stroke.is_empty() {
        stroke.insert("ids".into(), json!(ids));
        s.execute("stroke.set", &Value::Object(stroke))?;
    }
    if !transparency.is_empty() {
        transparency.insert("ids".into(), json!(ids));
        s.execute("transparency.set", &Value::Object(transparency))?;
    }
    if !props.is_empty() {
        props.insert("ids".into(), json!(ids));
        s.execute("object.setProps", &Value::Object(props))?;
    }
    Ok(())
}

/// A CSS length in points (`px` and unitless are points, as in SVG export).
fn length(v: &str) -> Option<f64> {
    let v = v.trim();
    let (num, unit) = v.find(|c: char| c.is_ascii_alphabetic() || c == '%').map_or((v, ""), |i| v.split_at(i));
    let n: f64 = num.trim().parse().ok()?;
    let k = match unit.trim() {
        "" | "px" | "pt" => 1.0,
        "mm" => 72.0 / 25.4,
        "cm" => 72.0 / 2.54,
        "in" => 72.0,
        "pc" => 12.0,
        _ => return None,
    };
    Some(n * k)
}
