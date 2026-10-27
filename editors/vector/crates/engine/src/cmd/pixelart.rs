//! Object › Trace Pixel Art…: a low-resolution image traced so that each pixel region becomes a
//! clean shape (`redrob_vector_trace::pixelart`): Voronoi cells, polygons or B-spline curves,
//! with the curves, islands and sparse-pixels heuristics weighted as asked.

use std::sync::Arc;

use redrob_vector_color::{Color, Paint};
use redrob_vector_doc::{Appearance, Node, NodeKind};
use redrob_vector_geom::PathData;
use redrob_vector_trace::{PixelArtOutput, PixelArtParams, trace_pixel_art};
use serde_json::{Value, json};

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "object.tracePixelArt",
        "Trace Pixel Art…",
        ["Object"],
        None,
        "{output?: bspline (default: each region's outline as smooth curves) | polygon (its outline as straight segments) | voronoi (one cell per pixel, cut and bridged at diagonals), curves?: 1, islands?: 1, sparse?: 1 (the weights of the heuristics that pick which diagonal of a 2×2 block stays: the longer curve, the pixel that would be left alone, the rarer colour; 0 turns one off), similarity?: 48 (0–255: how different two pixels may be in brightness and still be one colour), alphaThreshold?: 128 (pixels less opaque are background), deleteRaster?: false} trace the selected image as pixel art into a group of filled paths, one per region (voronoi: a group per region of its cells) → {id, shapes}",
        has_selection,
        trace
    )]
}

fn trace(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.tracePixelArt";
    let st = s.doc()?;
    let id = st
        .selection
        .objects
        .iter()
        .copied()
        .find(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Image(_))))
        .ok_or_else(|| bad(C, "select an image"))?;
    let node = st.doc.node(id).cloned().ok_or(EngineError::NoNode(id))?;
    let NodeKind::Image(im) = &node.kind else { return Err(bad(C, "select an image")) };
    let blob = st.doc.images.get(&im.key).filter(|b| !b.bytes.is_empty()).ok_or_else(|| bad(C, "the image has no embedded pixels"))?;
    let raster = redrob_vector_trace::Raster::decode(&blob.bytes).map_err(|e| bad(C, e.to_string()))?;
    if raster.width as u64 * raster.height as u64 > 1_000_000 {
        return Err(bad(C, "pixel art tracing is for small images (at most a million pixels): scale the image down, or use Image Trace"));
    }
    let output = match str_param(p, "output") {
        None => PixelArtOutput::BSpline,
        Some(o) => PixelArtOutput::parse(o).ok_or_else(|| bad(C, "output must be bspline, polygon or voronoi"))?,
    };
    let weight = |k: &str| f64_or(p, k, 1.0).clamp(0.0, 10.0);
    let params = PixelArtParams {
        output,
        curves: weight("curves"),
        islands: weight("islands"),
        sparse: weight("sparse"),
        similarity: f64_or(p, "similarity", 48.0).clamp(0.0, 255.0),
        alpha_threshold: p.get("alphaThreshold").and_then(Value::as_u64).unwrap_or(128).min(255) as u8,
    };
    let shapes = trace_pixel_art(&raster, &params);
    if shapes.is_empty() {
        return Err(bad(C, "the image has no opaque pixels to trace"));
    }
    let xf = im.xf;
    let delete = bool_or(p, "deleteRaster", false);
    let count = shapes.len();
    let gid = s.edit("Trace Pixel Art", |d, sel| {
        // Shapes by region, back to front as the regions were found.
        let mut groups: Vec<(usize, Vec<Arc<Node>>)> = vec![];
        for sh in &shapes {
            let [r, g, b, a] = sh.rgba;
            let color = Color::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0);
            let bp = xf * sh.path.clone();
            let mut n = Node::path(d.alloc_id(), PathData::from_bezpath(&bp), Appearance::basic(Paint::solid(color), Paint::None, 0.0));
            if a < 255 {
                n.opacity = f32::from(a) / 255.0;
            }
            match groups.iter_mut().find(|(rg, _)| *rg == sh.region) {
                Some((_, v)) => v.push(Arc::new(n)),
                None => groups.push((sh.region, vec![Arc::new(n)])),
            }
        }
        let children: Vec<Arc<Node>> =
            groups.into_iter().map(|(_, v)| if v.len() == 1 { v[0].clone() } else { Arc::new(Node::group(d.alloc_id(), v)) }).collect();
        let (par, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.name = Some("Pixel Art".into());
        d.insert(par, idx + 1, g)?;
        if delete {
            d.remove(id)?;
        }
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": gid.0, "shapes": count }))
}
