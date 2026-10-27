//! Effect › Filters: the stock filter presets (`filter.applyPreset`, `filter.presets`) and the
//! destructive raster edit of an embedded image's own pixels (`image.applyEffects`: Extensions ›
//! Raster-style operations). A preset puts a `filter.graph` effect built from
//! [`redrob_vector_effects::filter_presets`] on the selected objects, so the Filter Editor edits it
//! afterwards; the Filter Gallery panel and the Effect › Filters submenus both run it.

use redrob_vector_doc::{Node, NodeKind};
use redrob_vector_geom::{Affine, Rect};
use redrob_vector_render::AntiAlias;
use redrob_vector_render::effects::{self, filter_presets};
use redrob_vector_render::encode::{RasterExportOptions, RasterFormat};
use serde_json::{Value, json};

use super::edit::selected_roots;
use super::menucmds::{isolated_doc, render_png, unique_key};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "filter.applyPreset",
            "Apply Filter Preset",
            [],
            None,
            "{id: preset id (see filter.presets, e.g. \"bevels.bloom\"), ids?: [..] (default: the selection), replace?: bool (true: an existing filter graph on the object is replaced; false: the preset is added as another filter)} put the stock filter's graph on the objects as a filter.graph effect → {ids, index}",
            has_selection,
            apply_preset
        ),
        cmd!(
            query "filter.presets",
            "Filter Presets",
            [],
            None,
            "{query?: text (matches labels and categories), category?: name} → {categories: [..], presets: [{id, label, category, primitives}]} the stock filter presets",
            always,
            list_presets
        ),
        cmd!(
            "image.applyEffects",
            "Apply Effects to Image Pixels",
            ["Object"],
            None,
            "{ids?: [..] (default: the selected images), effect?: id (a raster effect to apply as well, e.g. \"pixel.unsharpMask\", \"filter.graph\"), params?: {…} (its parameters), ppi?: pixels per inch (default: the image's own resolution)} render each selected embedded image with its live effects (and `effect`) into its own pixels and drop the effects: a destructive raster edit, as Extensions › Raster operations are → {ids}",
            has_image_selected,
            apply_to_pixels
        ),
    ]
}

fn has_image_selected(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    let st = s.active().ok_or("no document open")?;
    if st.selection.objects.iter().any(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Image(_)))) {
        Ok(())
    } else {
        Err("select an embedded image".into())
    }
}

fn apply_preset(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "filter.applyPreset";
    let id = str_param(p, "id").ok_or_else(|| bad(C, "missing preset `id`"))?;
    let preset = filter_presets::preset(id).ok_or_else(|| bad(C, format!("unknown filter preset {id}; see filter.presets")))?;
    let graph = (preset.build)();
    let replace = bool_or(p, "replace", true);
    let ids = checked_ids_or_selection(s, p, C)?;
    if ids.is_empty() {
        return Err(bad(C, "nothing selected"));
    }
    let params = graph.to_params();
    let mut index = 0usize;
    s.edit(preset.label, |d, sel| {
        for id in &ids {
            let Some(n) = d.node_mut(*id) else { continue };
            let fx = &mut n.appearance.effects;
            match fx.iter().position(|e| e.id == effects::FILTER_GRAPH).filter(|_| replace) {
                Some(i) => {
                    fx[i].params = params.clone();
                    fx[i].visible = true;
                    index = i;
                }
                None => {
                    fx.push(redrob_vector_doc::Effect { id: effects::FILTER_GRAPH.into(), params: params.clone(), visible: true });
                    index = fx.len() - 1;
                }
            }
        }
        sel.set(ids.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "index": index }))
}

fn list_presets(_s: &mut Session, p: &Value) -> Result<Value> {
    let query = str_param(p, "query").unwrap_or("");
    let category = str_param(p, "category");
    let presets: Vec<Value> = filter_presets::search(query)
        .into_iter()
        .filter(|pr| category.is_none_or(|c| pr.category == c))
        .map(|pr| json!({ "id": pr.id, "label": pr.label, "category": pr.category, "primitives": (pr.build)().primitives.len() }))
        .collect();
    Ok(json!({ "categories": filter_presets::CATEGORIES, "presets": presets }))
}

fn apply_to_pixels(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "image.applyEffects";
    let roots = match p.get("ids") {
        Some(_) => checked_ids_param(s, p, "ids", C)?,
        None => selected_roots(s)?,
    };
    let extra = match str_param(p, "effect") {
        Some(id) => Some(
            effects::new_effect(id, p.get("params").unwrap_or(&Value::Null))
                .ok_or_else(|| bad(C, format!("unknown effect {id}; see effect.list")))?,
        ),
        None => None,
    };
    let st = s.doc()?;
    let look = st.doc.raster_effects.clone();
    let anti_alias = if look.anti_alias { AntiAlias::Art } else { AntiAlias::None };
    // Each image rendered alone, with its effects, at its own resolution over its visual bounds.
    let mut renders: Vec<(NodeId, Vec<u8>, u32, u32, Rect, f64)> = vec![];
    for id in &roots {
        let Some(n) = st.doc.node(*id) else { continue };
        let NodeKind::Image(im) = &n.kind else { continue };
        let mut node: Node = n.clone();
        node.visible = true;
        if let Some(e) = &extra {
            node.appearance.effects.push(e.clone());
        }
        let own_ppi = {
            let k = im.xf.determinant().abs().sqrt().max(1e-9);
            72.0 / k
        };
        let ppi = f64_or(p, "ppi", own_ppi).clamp(1.0, 2400.0);
        let scale = ppi / 72.0;
        let tmp = isolated_doc(&st.doc, vec![node.clone()]);
        let b = tmp.bounds_of(&[*id], true).or_else(|| n.visual_bounds()).ok_or_else(|| bad(C, "the image has no bounds"))?;
        let region = Rect::new(b.x0, b.y0, b.x0 + (b.width() * scale).ceil().max(1.0) / scale, b.y0 + (b.height() * scale).ceil().max(1.0) / scale);
        let raster = RasterExportOptions { ppi, background: None, anti_alias, ..Default::default() };
        let (png, w, h) = render_png(&tmp, region, scale, &raster.render_options(RasterFormat::Png), look.color_model)?;
        renders.push((*id, png, w, h, region, scale));
    }
    if renders.is_empty() {
        return Err(bad(C, "select an embedded image"));
    }
    let done: Vec<u64> = renders.iter().map(|r| r.0.0).collect();
    s.edit("Apply Effects to Image Pixels", |d, sel| {
        for (id, png, w, h, region, scale) in renders {
            let key = unique_key(d, "raster");
            d.images.insert(key.clone(), redrob_vector_doc::ImageBlob::new("image/png", png));
            if let Some(n) = d.node_mut(id) {
                n.appearance.effects.clear();
                if let NodeKind::Image(im) = &mut n.kind {
                    im.key = key;
                    im.width = w;
                    im.height = h;
                    im.xf = Affine::translate(region.origin().to_vec2()) * Affine::scale(1.0 / scale);
                    im.link = None;
                }
            }
        }
        sel.set(done.iter().map(|i| NodeId(*i)));
        Ok(())
    })?;
    Ok(json!({ "ids": done }))
}
