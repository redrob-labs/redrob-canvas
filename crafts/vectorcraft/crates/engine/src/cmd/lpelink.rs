//! Live path effects that link to other objects (Effect › Path Effects › Clone Original, Attach
//! Path, Bounding Box, Fill Between Many, Fill Between Strokes), kept following their linked
//! objects: after every edit [`refresh`] (run by `Session::edit`, like clones) copies each linked
//! object's geometry into the effect's parameters (`redrob_vector_effects::LINK_SOURCES`), where the
//! effect reads it. The commands here select an effect's linked objects (Select Linked Object),
//! turn a clip group into a Power Clip, give an opacity mask a background (Power Mask), and make
//! and fuse live slices.

use std::sync::Arc;

use redrob_vector_doc::{Appearance, Document, Effect, Node, NodeId, NodeKind};
use redrob_vector_geom::{PathData, shapes};
use redrob_vector_render::effects;
use serde_json::{Value, json};

use super::appearance::{appearance_targets, index_param, item_target};
use super::edit::copy_above;
use super::*;

/// The name of the rectangle Power Mask puts behind a mask's art.
pub(crate) const MASK_BACKGROUND: &str = "Mask Background";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "effect.selectLinked",
            "Select Linked Object",
            [],
            None,
            "{index: int (position in the effect list), item?: appearance item index|null (that fill/stroke's effects; omitted: the active item, else the object's), id?: object id (default: the first selected object)} select the objects the effect links to (Clone Original's original, Attach Path's path, Bounding Box's object, Fill Between's paths) → {ids}",
            has_doc,
            select_linked
        ),
        cmd!(
            "object.powerClip",
            "Power Clip",
            ["Object", "Clipping Mask"],
            None,
            "{inverse?: bool (false; show everything but the clipping path), hideClip?: bool (false; hide the clipping path's own fill and stroke), remove?: bool (false; take Power Clip off), ids?|id?: clip groups (or clipping paths; default: the selection)} keeps a clip editable as a live effect on its clipping path, with inverse and hide options; Release Clipping Mask keeps the path → {ids: the clipping paths}",
            has_selection,
            power_clip
        ),
        cmd!(
            "transparency.maskBackground",
            "Mask Background",
            [],
            None,
            "{color: \"#rrggbb\"|null (a colour fills the masked object's bounds behind the mask art, white shows it all and black hides it where the art isn't; null takes the background away), ids?|id?: masked objects (default: the selection, or the mask being edited)} Power Mask: an opacity mask with a background behind its art → {ids}",
            has_doc,
            mask_background
        ),
        cmd!(
            "object.liveSlice",
            "Live Slice",
            ["Object", "Path"],
            None,
            "{angle?: degrees (90; the cutting line's direction), point?: \"x,y\" (\"0.5,0.5\"; a point on it in box coordinates of each object's bounds), gap?: pt (0), ids?|id?} splits each selected path along a line into two live halves (the Slice effect, left on the object and right on a copy above it) that keep their own styles and stay editable; Fuse Slice joins them back → {ids: [left, right, …]}",
            has_selection,
            live_slice
        ),
        cmd!(
            "object.fuseSlice",
            "Fuse Slice",
            ["Object", "Path"],
            None,
            "{ids?|id?} takes the Slice effect off each selected half and removes the other half of its pair, so the object is whole again → {ids}",
            has_selection,
            fuse_slice
        ),
    ]
}

/// The geometry of object `id` as a linked effect reads it: a path's own, a compound path's
/// members together, any other object's bounds as a rectangle; with its geometric and visual
/// bounds.
fn linked_of(d: &Document, id: NodeId) -> Option<effects::Linked> {
    let n = d.node(id)?;
    let bounds = n.geometric_bounds();
    let path = match &n.kind {
        NodeKind::Path { path, .. } => path.clone(),
        NodeKind::Compound { children, .. } => {
            PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect())
        }
        _ => bounds.map(shapes::rectangle).unwrap_or_default(),
    };
    Some(effects::Linked { id: id.0, path, bounds, visual: n.visual_bounds() })
}

/// Every effect list of `n` (the object's, then each item's) as (item index, effects).
fn effect_lists(n: &Node) -> Vec<(Option<usize>, Vec<Effect>)> {
    let mut out = vec![(None, n.appearance.effects.clone())];
    out.extend(n.appearance.items.iter().enumerate().map(|(i, item)| (Some(i), item.effects().clone())));
    out
}

/// Linked effects follow their objects: after an edit, every effect that links to other objects
/// takes those objects' current geometry into its parameters (when it changed), and Clone
/// Original with `copyStyle` takes the original's fills and strokes.
pub(crate) fn refresh(d: &mut Document) {
    let mut carriers: Vec<NodeId> = vec![];
    d.walk(|n| {
        let linked = |fx: &[Effect]| fx.iter().any(|e| effects::is_linked(&e.id));
        if linked(&n.appearance.effects) || n.appearance.items.iter().any(|i| linked(i.effects())) {
            carriers.push(n.id);
        }
    });
    for id in carriers {
        let Some(n) = d.node(id) else { continue };
        // What each linked effect should hold now.
        let mut updates: Vec<(Option<usize>, usize, Value, Option<Appearance>)> = vec![];
        for (item, fx) in effect_lists(n) {
            for (k, e) in fx.iter().enumerate() {
                if !effects::is_linked(&e.id) {
                    continue;
                }
                let ids = effects::link_ids(&e.id, &e.params);
                let kept: Vec<effects::Linked> = ids.iter().filter(|s| **s != id.0).filter_map(|s| linked_of(d, NodeId(*s))).collect();
                let value = Value::Array(kept.iter().map(effects::Linked::to_value).collect());
                let current = e.params.get(effects::LINK_SOURCES).cloned().unwrap_or(Value::Null);
                let style = (e.id == effects::CLONE_ORIGINAL && e.params.get("copyStyle").and_then(Value::as_bool).unwrap_or(false))
                    .then(|| ids.first().and_then(|s| d.node(NodeId(*s))).map(|o| (*o.appearance).clone()))
                    .flatten();
                if current != value || style.as_ref().is_some_and(|a| a.items != n.appearance.items) {
                    updates.push((item, k, value, style));
                }
            }
        }
        if updates.is_empty() {
            continue;
        }
        let Some(n) = d.node_mut(id) else { continue };
        for (item, k, value, style) in updates {
            if let Some(fx) = n.appearance.effects_mut(item)
                && let Some(e) = fx.get_mut(k)
            {
                if !e.params.is_object() {
                    e.params = json!({});
                }
                e.params[effects::LINK_SOURCES] = value;
            }
            if let Some(a) = style {
                n.appearance.items = a.items;
            }
        }
    }
}

/// Applying a linked effect to a selection of several objects links them instead of applying
/// to each: the last selected object (the first, for Fill Between Many) carries the effect and
/// the others are its sources. `None` when the effect isn't linked, names its source already, or
/// one object is selected (then it applies as any effect, its source to be set in its options).
pub(crate) fn apply_linked(s: &mut Session, p: &Value, id: &str, effect: &Effect, label: &str) -> Option<Result<Value>> {
    if !effects::is_linked(id) || !effects::link_ids(id, &effect.params).is_empty() || p.get("item").is_some() {
        return None;
    }
    let roots = appearance_targets(s, p).ok()?;
    if roots.len() < 2 {
        return None;
    }
    let (carrier, sources): (NodeId, Vec<NodeId>) =
        if id == effects::FILL_BETWEEN_MANY { (roots[0], roots[1..].to_vec()) } else { (roots[roots.len() - 1], roots[..roots.len() - 1].to_vec()) };
    let mut e = effect.clone();
    if !e.params.is_object() {
        e.params = json!({});
    }
    match id {
        effects::FILL_BETWEEN_MANY => e.params["sources"] = json!(sources.iter().map(|i| i.0).collect::<Vec<_>>()),
        effects::FILL_BETWEEN_STROKES if sources.len() >= 2 => {
            e.params["source"] = json!(sources[0].0);
            e.params["source2"] = json!(sources[1].0);
        }
        _ => e.params["source"] = json!(sources[0].0),
    }
    let r = s.edit(label, |d, sel| {
        let n = d.node_mut(carrier).ok_or(EngineError::NoNode(carrier))?;
        n.appearance.effects.push(e);
        let index = n.appearance.effects.len() - 1;
        sel.set([carrier]);
        Ok(json!({ "ids": [carrier.0], "index": index, "item": Value::Null, "linked": sources.iter().map(|i| i.0).collect::<Vec<_>>() }))
    });
    Some(r)
}

fn select_linked(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "effect.selectLinked";
    let index = index_param(p, "index", C)?;
    let item = item_target(s, p, C)?;
    let target = match id_param(p, "id") {
        Some(id) => id,
        None => *appearance_targets(s, p)?.first().ok_or_else(|| bad(C, "select an object"))?,
    };
    let ids: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        let n = d.node(target).ok_or(EngineError::NoNode(target))?;
        let item = item.effects_item(&n.appearance, C)?;
        let e = n.appearance.effects_at(item).and_then(|fx| fx.get(index)).ok_or_else(|| bad(C, format!("no effect at index {index}")))?;
        effects::link_ids(&e.id, &e.params).into_iter().map(NodeId).filter(|id| d.node(*id).is_some()).collect()
    };
    if ids.is_empty() {
        return Err(EngineError::Other("the effect links to no object in this document".into()));
    }
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// The clipping path of each target: a clip group's first child, or the target itself when it is
/// a clipping path.
fn clipping_paths(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    ids.iter()
        .filter_map(|id| {
            let n = d.node(*id)?;
            if n.clips() {
                return n.children()?.first().map(|c| c.id);
            }
            matches!(n.kind, NodeKind::Path { clipping: true, .. }).then_some(*id)
        })
        .collect()
}

fn power_clip(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.powerClip";
    let ids = {
        let d = &s.doc()?.doc;
        clipping_paths(d, &targets(s, p)?)
    };
    if ids.is_empty() {
        return Err(bad(C, "select a clip group (Object › Clipping Mask › Make)"));
    }
    let remove = bool_or(p, "remove", false);
    let mut params = json!({});
    for key in ["inverse", "hideClip"] {
        if let Some(v) = p.get(key).and_then(Value::as_bool) {
            params[key] = json!(v);
        }
    }
    s.edit("Power Clip", |d, _| {
        for id in &ids {
            let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
            let fx = &mut n.appearance.effects;
            let at = fx.iter().position(|e| e.id == effects::POWER_CLIP);
            match at {
                Some(_) if remove => fx.retain(|e| e.id != effects::POWER_CLIP),
                Some(k) => {
                    let e = &mut fx[k];
                    if !e.params.is_object() {
                        e.params = json!({});
                    }
                    if let Value::Object(m) = &params {
                        for (k, v) in m {
                            e.params[k] = v.clone();
                        }
                    }
                }
                None if remove => {}
                None => fx.push(effects::new_effect(effects::POWER_CLIP, &params).ok_or_else(|| bad(C, "no Power Clip effect"))?),
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// The mask art of a masked object with (`color` given) or without a background rectangle.
fn with_background(
    d: &mut Document,
    art: &Arc<Node>,
    bounds: Option<redrob_vector_geom::Rect>,
    color: Option<redrob_vector_color::Color>,
) -> Arc<Node> {
    let is_bg = |n: &Node| n.name.as_deref() == Some(MASK_BACKGROUND);
    // The art without any background it had.
    let (mut members, grouped): (Vec<Arc<Node>>, bool) = match &art.kind {
        NodeKind::Group { children, clip: false } if children.first().is_some_and(|c| is_bg(c)) => (children[1..].to_vec(), true),
        _ => (vec![art.clone()], false),
    };
    let Some(color) = color else {
        return match (members.len(), grouped) {
            (1, true) => members.remove(0),
            (_, true) => Arc::new(Node::group(art.id, members)),
            _ => art.clone(),
        };
    };
    let Some(b) = bounds.or_else(|| art.geometric_bounds()) else { return art.clone() };
    let b = b.inflate(1.0, 1.0);
    let mut bg = Node::path(
        d.alloc_id(),
        shapes::rectangle(b),
        Appearance::basic(redrob_vector_color::Paint::solid(color), redrob_vector_color::Paint::None, 0.0),
    );
    bg.name = Some(MASK_BACKGROUND.into());
    let mut children = vec![Arc::new(bg)];
    children.append(&mut members);
    let gid = if grouped { art.id } else { d.alloc_id() };
    Arc::new(Node::group(gid, children))
}

fn mask_background(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "transparency.maskBackground";
    let color = match p.get("color") {
        None | Some(Value::Null) => None,
        Some(v) => Some(color_value(v).ok_or_else(|| bad(C, "`color` must be a colour (\"#rrggbb\") or null"))?),
    };
    let ids: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        super::opacitymask::transparency_targets(s, p)?.into_iter().filter(|id| d.node(*id).is_some_and(|n| n.extras().mask.is_some())).collect()
    };
    if ids.is_empty() {
        return Err(EngineError::Other("no target object has an opacity mask: select one or give `id`".into()));
    }
    s.edit("Mask Background", |d, _| {
        for id in &ids {
            let Some(n) = d.node(*id) else { continue };
            let bounds = n.visual_bounds();
            let Some(art) = n.extras().mask.as_deref().map(|m| m.art.clone()) else { continue };
            let art = with_background(d, &art, bounds, color);
            if let Some(m) = d.node_mut(*id).and_then(|n| n.extras_if_any().and_then(|e| e.mask.as_mut())) {
                m.art = art;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn live_slice(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.liveSlice";
    let roots: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        targets(s, p)?
            .into_iter()
            .filter(|id| d.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })))
            .collect()
    };
    if roots.is_empty() {
        return Err(bad(C, "select a path"));
    }
    let mut base = json!({});
    for key in ["angle", "point", "gap"] {
        if let Some(v) = p.get(key) {
            base[key] = v.clone();
        }
    }
    let effect = |side: &str, pair: NodeId| -> Option<Effect> {
        let mut params = base.clone();
        params["side"] = json!(side);
        params["__pair"] = json!(pair.0);
        effects::new_effect(effects::lpe_draw::SLICE, &params)
    };
    let ids = s.edit("Live Slice", |d, sel| {
        let copies = copy_above(d, &roots, |_, _, c| c.name = None)?;
        let mut out = vec![];
        for (left, right) in roots.iter().zip(&copies) {
            let l = d.node_mut(*left).ok_or(EngineError::NoNode(*left))?;
            l.appearance.effects.push(effect("left", *right).ok_or_else(|| bad(C, "no Slice effect"))?);
            let r = d.node_mut(*right).ok_or(EngineError::NoNode(*right))?;
            r.appearance.effects.push(effect("right", *left).ok_or_else(|| bad(C, "no Slice effect"))?);
            out.push(*left);
            out.push(*right);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn fuse_slice(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.fuseSlice";
    let halves: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        targets(s, p)?
            .into_iter()
            .filter(|id| d.node(*id).is_some_and(|n| n.appearance.effects.iter().any(|e| e.id == effects::lpe_draw::SLICE)))
            .collect()
    };
    if halves.is_empty() {
        return Err(bad(C, "select a live slice (Object › Path › Live Slice)"));
    }
    let ids = s.edit("Fuse Slice", |d, sel| {
        let mut kept = vec![];
        let mut gone: Vec<NodeId> = vec![];
        for id in &halves {
            if gone.contains(id) || d.node(*id).is_none() {
                continue;
            }
            let pairs: Vec<NodeId> = d
                .node(*id)
                .map(|n| {
                    n.appearance
                        .effects
                        .iter()
                        .filter(|e| e.id == effects::lpe_draw::SLICE)
                        .filter_map(|e| e.params.get("__pair").and_then(Value::as_u64).map(NodeId))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(n) = d.node_mut(*id) {
                n.appearance.effects.retain(|e| e.id != effects::lpe_draw::SLICE);
            }
            for pair in pairs {
                if pair != *id && d.node(pair).is_some() {
                    d.remove(pair)?;
                    gone.push(pair);
                }
            }
            kept.push(*id);
        }
        sel.set(kept.iter().copied());
        Ok(kept)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with_two_paths() -> (Session, NodeId, NodeId) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
        let b = s.execute("shape.rectangle", &json!({"x": 200, "y": 0, "width": 40, "height": 40})).unwrap();
        let id = |v: &Value| NodeId(v["id"].as_u64().unwrap());
        (s, id(&a), id(&b))
    }

    #[test]
    fn clone_original_follows_its_source() {
        let (mut s, a, b) = session_with_two_paths();
        s.execute("effect.apply", &json!({"effect": effects::CLONE_ORIGINAL, "ids": [b.0], "item": null, "params": {"source": a.0}})).unwrap();
        let bounds = |s: &Session, id: NodeId| {
            let d = &s.active().unwrap().doc;
            let n = d.node(id).unwrap();
            let e = &n.appearance.effects[0];
            let out = effects::apply_geometry(&n.appearance.effects, n.path_data().unwrap(), n.geometric_bounds().unwrap());
            (out.bounds().unwrap(), e.params.get(effects::LINK_SOURCES).cloned())
        };
        let (bb, kept) = bounds(&s, b);
        assert!(kept.is_some());
        assert!((bb.width() - 100.0).abs() < 1e-6, "{bb:?}");
        // Moving the original moves the clone.
        s.select(|_, sel| sel.set([a])).unwrap();
        s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
        let (bb2, _) = bounds(&s, b);
        assert!((bb2.x0 - 10.0).abs() < 1e-6, "{bb2:?}");
        // Select Linked Object selects the original.
        s.select(|_, sel| sel.set([b])).unwrap();
        let r = s.execute("effect.selectLinked", &json!({"index": 0, "item": null})).unwrap();
        assert_eq!(r["ids"], json!([a.0]));
    }

    #[test]
    fn two_selected_objects_link_on_apply() {
        let (mut s, a, b) = session_with_two_paths();
        s.select(|_, sel| sel.set([a, b])).unwrap();
        let r = s.execute("effect.apply", &json!({"effect": effects::BOUNDING_BOX})).unwrap();
        assert_eq!(r["ids"], json!([b.0]));
        assert_eq!(r["linked"], json!([a.0]));
    }

    #[test]
    fn live_slice_makes_two_halves_and_fuses_them() {
        let (mut s, a, _) = session_with_two_paths();
        let r = s.execute("object.liveSlice", &json!({"ids": [a.0]})).unwrap();
        let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
        assert_eq!(ids.len(), 2);
        {
            let d = &s.active().unwrap().doc;
            let n = d.node(NodeId(ids[0])).unwrap();
            let out = effects::apply_geometry(&n.appearance.effects, n.path_data().unwrap(), n.geometric_bounds().unwrap());
            assert!((out.bounds().unwrap().width() - 50.0).abs() < 1e-3);
        }
        s.execute("object.fuseSlice", &json!({"ids": [ids[0]]})).unwrap();
        let d = &s.active().unwrap().doc;
        assert!(d.node(NodeId(ids[1])).is_none());
        assert!(d.node(a).unwrap().appearance.effects.is_empty());
    }
}
