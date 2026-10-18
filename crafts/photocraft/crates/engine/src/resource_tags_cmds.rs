//! Tags on resources (patterns, gradients, palettes, dynamics, tool presets, styles, shapes and
//! brushes): free-form words typed on a resource, used to filter the panels that list them.
//!
//! Brush presets keep their tags where they always were (`ToolState::preset_tags`, the
//! `brush.presets.tag` family); every other kind is stored in the preferences
//! (`Preferences::resource_tags`, key `<kind>/<name>`), so they persist with the settings.
//!
//! * `resource.tag {"kind","name","tag"}` / `resource.untag`: add or remove one tag.
//! * `resource.tags {"kind"?,"name"?}`: the tags of one resource, of a kind, or of everything.
//! * `resource.filter {"kind","query"}`: the names of `kind` matching a filter: each word of the
//!   query must be the start of one of the resource's tags (or of its name), the autocomplete
//!   rule the filter fields use.
//! * `resource.tagComplete {"kind"?,"prefix"}`: the tags starting with `prefix`, for autocomplete.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The kinds of resource that take tags, as the `kind` param spells them.
pub const KINDS: [&str; 9] = ["brush", "pattern", "gradient", "palette", "dynamics", "toolPreset", "style", "shape", "mypaintBrush"];

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn kind_param(p: &Value, cmd: &str) -> Result<String> {
    let k = p.get("kind").and_then(Value::as_str).map(str::trim).filter(|k| !k.is_empty()).ok_or_else(|| bad(cmd, "missing `kind`"))?;
    KINDS.iter().find(|x| x.eq_ignore_ascii_case(k)).map(|x| x.to_string()).ok_or_else(|| bad(cmd, format!("unknown kind `{k}` (want {})", KINDS.join("|"))))
}

fn name_param(p: &Value, cmd: &str) -> Result<String> {
    p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).ok_or_else(|| bad(cmd, "missing `name`"))
}

fn tag_param(p: &Value, key: &str, cmd: &str) -> Result<String> {
    let t = p.get(key).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| bad(cmd, format!("missing `{key}`")))?;
    if t.len() > 64 {
        return Err(bad(cmd, "a tag is at most 64 characters"));
    }
    if t.contains([',', '\n', '\t']) {
        return Err(bad(cmd, "a tag holds no comma, tab or newline"));
    }
    Ok(t.to_string())
}

/// The resource names of `kind` known to the session (for the filter; empty when the kind's
/// library is not enumerable here, in which case tagged names stand in).
pub fn names(s: &Session, kind: &str) -> Vec<String> {
    match kind {
        "brush" => s.tools.presets.iter().map(|p| p.name.clone()).collect(),
        "gradient" => s.presets.gradients.iter().flat_map(|g| g.items.iter().map(|i| i.name.clone())).collect(),
        "pattern" => s.patterns.items.iter().map(|p| p.display_name().to_string()).collect(),
        "palette" => s.presets.swatches.iter().map(|g| g.name.clone()).collect(),
        "dynamics" => s.tools.dynamics_presets.iter().map(|d| d.name.clone()).collect(),
        "toolPreset" => s.presets.tool_presets.iter().map(|p| p.name.clone()).collect(),
        "style" => s.presets.styles.iter().flat_map(|g| g.items.iter().map(|i| i.name.clone())).collect(),
        "shape" => s.presets.shapes.iter().flat_map(|g| g.items.iter().map(|i| i.name.clone())).collect(),
        _ => Vec::new(),
    }
}

fn key(kind: &str, name: &str) -> String {
    format!("{kind}/{}", name.to_lowercase())
}

/// The tags of resource `name` of `kind`.
pub fn tags_of(s: &Session, kind: &str, name: &str) -> Vec<String> {
    if kind == "brush" {
        return crate::brush_preset_cmds::tags_of(s, name).to_vec();
    }
    s.prefs().resource_tags.get(&key(kind, name)).cloned().unwrap_or_default()
}

/// Every tag of `kind` (all kinds when `None`), sorted, de-duplicated case-insensitively.
pub fn all_tags(s: &Session, kind: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |t: &str| {
        if !out.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            out.push(t.to_string());
        }
    };
    if kind.is_none_or(|k| k == "brush") {
        for t in crate::brush_preset_cmds::all_tags(s) {
            push(&t);
        }
    }
    for (k, tags) in &s.prefs().resource_tags {
        if kind.is_none_or(|kind| k.starts_with(&format!("{kind}/"))) {
            for t in tags {
                push(t);
            }
        }
    }
    out.sort_by_key(|t| t.to_lowercase());
    out
}

/// Does resource `name` of `kind` match the filter `query`? Each whitespace-separated word must
/// start one of its tags or its name (case-insensitive); an empty query matches everything.
pub fn matches(s: &Session, kind: &str, name: &str, query: &str) -> bool {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let tags = tags_of(s, kind, name);
    let lname = name.to_lowercase();
    words.iter().all(|w| lname.starts_with(w.as_str()) || lname.contains(&format!(" {w}")) || tags.iter().any(|t| t.to_lowercase().starts_with(w.as_str())))
}

fn set_tags(s: &mut Session, kind: &str, name: &str, tags: Vec<String>) {
    if kind == "brush" {
        let key = s.tools.preset_tags.keys().find(|n| n.eq_ignore_ascii_case(name)).cloned().unwrap_or_else(|| name.to_string());
        if tags.is_empty() {
            s.tools.preset_tags.remove(&key);
        } else {
            s.tools.preset_tags.insert(key, tags);
        }
        s.brush_presets_changed();
        return;
    }
    let k = key(kind, name);
    s.edit_prefs(|pr| {
        if tags.is_empty() {
            pr.resource_tags.remove(&k);
        } else {
            pr.resource_tags.insert(k, tags);
        }
    });
}

fn tag(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "resource.tag";
    let kind = kind_param(p, cmd)?;
    let name = name_param(p, cmd)?;
    let tag = tag_param(p, "tag", cmd)?;
    let mut tags = tags_of(s, &kind, &name);
    if !tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
        tags.push(tag);
        tags.sort_by_key(|t| t.to_lowercase());
        set_tags(s, &kind, &name, tags);
    }
    Ok(json!({"kind": kind, "name": name, "tags": tags_of(s, &kind, &name)}))
}

fn untag(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "resource.untag";
    let kind = kind_param(p, cmd)?;
    let name = name_param(p, cmd)?;
    let tag = tag_param(p, "tag", cmd)?;
    let mut tags = tags_of(s, &kind, &name);
    let before = tags.len();
    tags.retain(|t| !t.eq_ignore_ascii_case(&tag));
    if tags.len() != before {
        set_tags(s, &kind, &name, tags);
    }
    Ok(json!({"kind": kind, "name": name, "tags": tags_of(s, &kind, &name)}))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "resource.tags";
    let kind = match p.get("kind") {
        Some(_) => Some(kind_param(p, cmd)?),
        None => None,
    };
    if let (Some(kind), Some(name)) = (&kind, p.get("name").and_then(Value::as_str)) {
        return Ok(json!({"kind": kind, "name": name, "tags": tags_of(s, kind, name)}));
    }
    let tags = all_tags(s, kind.as_deref());
    let mut by_tag: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for t in &tags {
        by_tag.insert(t.clone(), Vec::new());
    }
    let kinds: Vec<&str> = match &kind {
        Some(k) => vec![k.as_str()],
        None => KINDS.to_vec(),
    };
    for k in kinds {
        for n in names(s, k) {
            for t in tags_of(s, k, &n) {
                if let Some(list) = by_tag.iter_mut().find(|(x, _)| x.eq_ignore_ascii_case(&t)).map(|(_, l)| l) {
                    list.push(format!("{k}/{n}"));
                }
            }
        }
    }
    Ok(json!({"kind": kind, "tags": tags, "resources": by_tag}))
}

fn filter(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "resource.filter";
    let kind = kind_param(p, cmd)?;
    let query = p.get("query").and_then(Value::as_str).unwrap_or("");
    let names: Vec<String> = names(s, &kind).into_iter().filter(|n| matches(s, &kind, n, query)).collect();
    Ok(json!({"kind": kind, "query": query, "names": names}))
}

fn complete(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "resource.tagComplete";
    let kind = match p.get("kind") {
        Some(_) => Some(kind_param(p, cmd)?),
        None => None,
    };
    let prefix = p.get("prefix").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    let tags: Vec<String> = all_tags(s, kind.as_deref()).into_iter().filter(|t| t.to_lowercase().starts_with(&prefix)).collect();
    Ok(json!({"prefix": prefix, "tags": tags}))
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $run:expr, $journal:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: crate::commands::always, run: $run, journal: $journal }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "resource.tag",
            "Tag Resource",
            r##"{"kind":"brush|pattern|gradient|palette|dynamics|toolPreset|style|shape|mypaintBrush","name":string,"tag":string} → {kind, name, tags}"##,
            tag,
            true
        ),
        spec!("resource.untag", "Untag Resource", r##"{"kind":string,"name":string,"tag":string} → {kind, name, tags}"##, untag, true),
        spec!("resource.tags", "Resource Tags", r##"{"kind":string?,"name":string?} → {tags, resources: tag → [kind/name]}"##, list, false),
        spec!(
            "resource.filter",
            "Filter Resources by Tag",
            r##"{"kind":string,"query":string} → {names} (each word of the query starts a tag or the name)"##,
            filter,
            false
        ),
        spec!("resource.tagComplete", "Complete Resource Tag", r##"{"kind":string?,"prefix":string} → {tags}"##, complete, false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_persist_in_preferences_and_filter_by_prefix() {
        let mut s = Session::new();
        let g = s.presets.gradients.iter().flat_map(|g| g.items.iter()).map(|i| i.name.clone()).next().expect("a gradient preset");
        let r = s.execute("resource.tag", json!({"kind": "gradient", "name": g, "tag": "Warm"})).unwrap();
        assert_eq!(r["tags"], json!(["Warm"]));
        s.execute("resource.tag", json!({"kind": "gradient", "name": g, "tag": "sunset"})).unwrap();
        assert_eq!(tags_of(&s, "gradient", &g), vec!["sunset", "Warm"]);
        assert!(s.prefs().resource_tags.contains_key(&format!("gradient/{}", g.to_lowercase())));
        let f = s.execute("resource.filter", json!({"kind": "gradient", "query": "wa"})).unwrap();
        assert_eq!(f["names"], json!([g]));
        let f = s.execute("resource.filter", json!({"kind": "gradient", "query": "wa sun"})).unwrap();
        assert_eq!(f["names"].as_array().unwrap().len(), 1);
        let f = s.execute("resource.filter", json!({"kind": "gradient", "query": "nothing"})).unwrap();
        assert!(f["names"].as_array().unwrap().is_empty());
        let c = s.execute("resource.tagComplete", json!({"prefix": "s"})).unwrap();
        assert!(c["tags"].as_array().unwrap().contains(&json!("sunset")), "{c}");
        s.execute("resource.untag", json!({"kind": "gradient", "name": g, "tag": "WARM"})).unwrap();
        assert_eq!(tags_of(&s, "gradient", &g), vec!["sunset"]);
        assert!(s.execute("resource.tag", json!({"kind": "font", "name": "x", "tag": "y"})).is_err());
    }

    #[test]
    fn brush_tags_go_through_the_brush_store() {
        let mut s = Session::new();
        let b = s.tools.presets[0].name.clone();
        s.execute("resource.tag", json!({"kind": "brush", "name": b, "tag": "Ink"})).unwrap();
        assert!(crate::brush_preset_cmds::has_tag(&s, &b, "ink"));
        let all = s.execute("resource.tags", json!({})).unwrap();
        assert!(all["tags"].as_array().unwrap().contains(&json!("Ink")), "{all}");
        assert!(matches(&s, "brush", &b, "in"));
    }
}
