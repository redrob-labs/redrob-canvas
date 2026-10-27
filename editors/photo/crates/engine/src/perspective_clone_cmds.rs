//! Perspective Clone as a canvas paint tool (`paint.perspectiveClone`): the Clone Stamp with a
//! perspective plane. The caller defines the plane's four corners (clicked on the canvas), sets a
//! source point and strokes; the cloned pixels are rescaled to the plane's perspective
//! ([`redrob_photo_algo::vanishing::clone_stroke`]), the same maths the Vanishing Point workspace
//! uses, but on the active layer as one stroke per history step.

use redrob_photo_algo::vanishing::{PlaneCheck, Scene, VpPlane, clone_stroke};
use redrob_photo_doc::{LayerContent, LayerId};
use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "paint.perspectiveClone";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn pt(v: &Value) -> Option<[f64; 2]> {
    let x = v.get(0)?.as_f64()?;
    let y = v.get(1)?.as_f64()?;
    (x.is_finite() && y.is_finite() && x.abs() < 1e7 && y.abs() < 1e7).then_some([x, y])
}

/// The plane from `"corners"`: four finite points forming a convex quad.
pub fn plane(p: &Value, canvas: Rect) -> Result<Scene> {
    let a =
        p.get("corners").and_then(Value::as_array).filter(|a| a.len() == 4).ok_or_else(|| bad("pass `corners`: four [x,y] points of the perspective plane"))?;
    let c: Vec<[f64; 2]> = a.iter().filter_map(pt).collect();
    if c.len() != 4 {
        return Err(bad("every corner must be a finite [x,y]"));
    }
    let center = [(canvas.x0 + canvas.x1) as f64 / 2.0, (canvas.y0 + canvas.y1) as f64 / 2.0];
    let pl = VpPlane { corners: [c[0], c[1], c[2], c[3]] };
    if pl.check(center) == PlaneCheck::Invalid {
        return Err(bad("the four corners must form a convex quad"));
    }
    let focal = p.get("focalLength").and_then(Value::as_f64).filter(|f| *f > 0.0);
    let sc = Scene::new(pl, center, focal, canvas.width().max(canvas.height()) as f64);
    if sc.planes.iter().any(|p| p.homography().is_none()) {
        return Err(bad("the plane is degenerate"));
    }
    Ok(sc)
}

fn has_pixel_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let l = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active layer")?;
    if matches!(l.content, LayerContent::Raster(_)) { Ok(()) } else { Err("the Perspective Clone needs a pixel layer".into()) }
}

fn perspective_clone(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = st.doc.bounds();
    let sc = plane(p, canvas)?;
    let points: Vec<[f64; 2]> = p.get("points").and_then(Value::as_array).map(|a| a.iter().filter_map(pt).collect()).unwrap_or_default();
    let Some(first) = points.first().copied() else {
        return Err(bad("`points` must hold at least one [x,y]"));
    };
    // The source: given outright, or as the aligned offset from the first point.
    let source = match (p.get("source").and_then(pt), p.get("offset").and_then(pt)) {
        (Some(src), _) => src,
        (None, Some(off)) => [first[0] + off[0], first[1] + off[1]],
        (None, None) => return Err(bad("pass `source` [x,y] or `offset` [dx,dy]")),
    };
    let size = p.get("size").and_then(Value::as_f64).unwrap_or_else(|| f64::from(s.tools.brush.size)).clamp(1.0, 2000.0);
    let hardness = p.get("hardness").and_then(Value::as_f64).unwrap_or_else(|| f64::from(s.tools.brush.hardness) * 100.0).clamp(0.0, 100.0) / 100.0;
    let opacity = (p.get("opacity").and_then(Value::as_f64).unwrap_or_else(|| f64::from(s.tools.brush.opacity) * 100.0).clamp(0.0, 100.0) / 100.0) as f32;
    let sample_all = p.get("sampleAllLayers").and_then(Value::as_bool).unwrap_or(false);
    let id: LayerId = st.active_layer.ok_or_else(|| bad("no active layer"))?;
    if sc.plane_at(source).is_none() {
        return Err(bad("the source point lies outside the perspective plane"));
    }
    let mut dabs = 0usize;
    let dmg = s.edit("Perspective Clone", |doc, _| {
        let fmt = doc.pixel_format();
        let base: Option<Surface> = sample_all.then(|| crate::file_cmds::flattened(doc, fmt));
        let locks = doc.effective_locks(id);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if locks.all || locks.pixels {
            return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
        }
        let surf = l.surface_mut().ok_or_else(|| bad("not a pixel layer"))?;
        // Sample from the layer (or the visible image), paint onto the layer.
        let mut work = match &base {
            Some(b) => {
                let mut w = b.clone();
                over(&mut w, surf);
                w
            }
            None => surf.clone(),
        };
        let before = work.clone();
        dabs = clone_stroke(&sc, &mut work, source, &points, size, hardness, opacity);
        let b = work.content_bounds().union(&before.content_bounds()).intersect(&canvas);
        if !b.is_empty() {
            let (nw, ow) = (work.read_region(b), before.read_region(b));
            let n = surf.format().channels();
            let mut cur = surf.read_region(b);
            for i in 0..cur.len() / n {
                if nw[i * n..(i + 1) * n] != ow[i * n..(i + 1) * n] {
                    cur[i * n..(i + 1) * n].copy_from_slice(&nw[i * n..(i + 1) * n]);
                }
            }
            surf.write_region(b, &cur);
        }
        surf.prune();
        Ok(b)
    })?;
    if let Some(st) = s.active_mut()
        && !dmg.is_empty()
    {
        st.last_damage = Some(dmg);
    }
    Ok(json!({ "dabs": dabs, "offset": [source[0] - first[0], source[1] - first[1]], "damage": [dmg.x0, dmg.y0, dmg.width(), dmg.height()] }))
}

/// `top` over `dst` (straight alpha), both in the same format.
fn over(dst: &mut Surface, top: &Surface) {
    let r = top.content_bounds();
    if r.is_empty() {
        return;
    }
    let fmt = dst.format();
    let n = fmt.channels();
    let (mut d, t) = (dst.read_region(r), top.read_region(r));
    for i in 0..d.len() / n {
        let (dp, tp) = (&mut d[i * n..(i + 1) * n], &t[i * n..(i + 1) * n]);
        let td = redrob_photo_raster::to_rgba(&fmt, tp);
        if td[3] <= 0.0 {
            continue;
        }
        let dd = redrob_photo_raster::to_rgba(&fmt, dp);
        let o = redrob_photo_color::blend::composite(redrob_photo_color::BlendMode::Normal, dd, td, 1.0);
        let mut enc = [0.0f32; 8];
        redrob_photo_raster::from_rgba_into(&fmt, o, &mut enc);
        dp.copy_from_slice(&enc[..n]);
    }
    dst.write_region(r, &d);
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Perspective Clone",
        menu: &[],
        shortcut: None,
        params: r##"{"corners":[[x,y]×4] (the perspective plane),"source":[x,y]? (where to clone from),"offset":[dx,dy]? (aligned: source = first point + offset),"points":[[x,y],…],"size":px=brush size,"hardness":0..100=brush hardness,"opacity":0..100=brush opacity,"sampleAllLayers":bool=false,"focalLength":px?} → {dabs, offset, damage}"##,
        enabled: has_pixel_layer,
        run: perspective_clone,
        journal: true,
    }]
}
