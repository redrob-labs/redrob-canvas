//! Layer boundaries, Threshold Alpha, per-layer composite options and Zealous Crop.
//!
//! A layer's own boundary ([`redrob_photo_doc::Layer::boundary`]) is the rectangle its content
//! is composited within; `None` is unbounded (the canvas clips it). These commands change it:
//!
//! * `layer.boundarySize {"width","height","offsetX","offsetY","fill"}`: a new boundary around the
//!   old one, the old content `offsetX`/`offsetY` inside it, the added area filled.
//! * `layer.toImageSize`: the boundary becomes the canvas (unbounded).
//! * `layer.resizeToSelection`: the boundary becomes the selection's bounds.
//! * `layer.cropToContent`: the boundary shrinks to the layer's opaque pixels.
//! * `layer.thresholdAlpha {"threshold"}`: alpha becomes 0 or 1 at a threshold.
//! * `layer.compositeOptions {"mode","blendSpace","compositeSpace","legacy"}`: how the layer's alpha
//!   combines with the backdrop's and which colour space the blend and the mix run in.
//! * `image.zealousCrop`: removes every whole row and column of one uniform colour, wherever it
//!   lies, from every layer, mask, channel and the selection.

use redrob_photo_algo::selection as sel;
use redrob_photo_doc::{BlendSpace, CompositeMode, Document, LayerComposite, LayerContent};
use redrob_photo_geom::{Rect, Size};
use redrob_photo_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn has_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.filter(|id| d.doc.layer(*id).is_some()).map(|_| ()).ok_or_else(|| "no active layer".into())
}
fn has_pixel_layer(s: &Session) -> std::result::Result<(), String> {
    let l = crate::active_layer_of(s)?;
    if matches!(l.content, LayerContent::Raster(_)) { Ok(()) } else { Err("the active layer has no pixels".into()) }
}
fn has_layer_and_selection(s: &Session) -> std::result::Result<(), String> {
    has_layer(s)?;
    s.active().filter(|d| d.doc.selection.is_some()).map(|_| ()).ok_or_else(|| "no selection".into())
}
fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
fn int(p: &Value, k: &str, d: i64) -> i64 {
    p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(d, |v| v.round() as i64)
}

/// The boundary a layer currently composites within: its own, or the canvas.
pub fn effective_boundary(doc: &Document, id: redrob_photo_doc::LayerId) -> Rect {
    doc.layer(id).and_then(|l| l.boundary).unwrap_or_else(|| doc.bounds())
}

fn set_boundary(s: &mut Session, label: &str, p: &Value, new: impl Fn(&mut Document, redrob_photo_doc::LayerId) -> Result<Option<Rect>>) -> Result<Value> {
    let id = crate::commands::layer_param(s, p)?;
    let r = s.edit(label, |doc, _| {
        let r = new(doc, id)?;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        l.boundary = r;
        Ok(r)
    })?;
    Ok(match r {
        Some(r) => json!({ "layer": id.0, "boundary": [r.x0, r.y0, r.width(), r.height()] }),
        None => json!({ "layer": id.0, "boundary": Value::Null }),
    })
}

// ---------------------------------------------------------------- Layer Boundary Size

fn boundary_size(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.boundarySize";
    let fill = match p.get("fill").and_then(Value::as_str).unwrap_or("transparent") {
        "transparent" => None,
        "foreground" => Some(s.tools.foreground),
        "background" => Some(s.tools.background),
        "white" => Some([1.0; 4]),
        "black" => Some([0.0, 0.0, 0.0, 1.0]),
        other => return Err(bad(CMD, format!("unknown fill `{other}` (want transparent|foreground|background|white|black)"))),
    };
    set_boundary(s, "Layer Boundary Size", p, move |doc, id| {
        let cur = effective_boundary(doc, id);
        let w = int(p, "width", i64::from(cur.width()));
        let h = int(p, "height", i64::from(cur.height()));
        if !(1..=300_000).contains(&w) || !(1..=300_000).contains(&h) {
            return Err(bad(CMD, "width and height must be 1..300000"));
        }
        let (ox, oy) = (int(p, "offsetX", 0), int(p, "offsetY", 0));
        let x0 = i64::from(cur.x0) - ox;
        let y0 = i64::from(cur.y0) - oy;
        let fits = |v: i64| i32::try_from(v).map_err(|_| bad(CMD, "the boundary leaves the 32-bit coordinate range"));
        let r = Rect::new(fits(x0)?, fits(y0)?, fits(x0 + w)?, fits(y0 + h)?);
        if let Some(c) = fill
            && let Some(l) = doc.layer_mut(id)
            && let LayerContent::Raster(surf) = &mut l.content
        {
            // The area the new boundary adds around the old one takes the fill colour.
            let mut m = sel::mask_from_surface(None, r);
            let rw = r.width() as usize;
            for (i, v) in m.iter_mut().enumerate() {
                let (x, y) = (r.x0 + (i % rw) as i32, r.y0 + (i / rw) as i32);
                if !cur.contains(x, y) {
                    *v = 1.0;
                }
            }
            if m.iter().any(|v| *v > 0.0) {
                let cover = sel::mask_to_surface(&m, r);
                crate::pixels::try_fill_surface(surf, r, c, Some(&cover), false)?;
            }
        }
        Ok(Some(r))
    })
}

fn to_image_size(s: &mut Session, p: &Value) -> Result<Value> {
    set_boundary(s, "Layer to Image Size", p, |_, _| Ok(None))
}

fn resize_to_selection(s: &mut Session, p: &Value) -> Result<Value> {
    set_boundary(s, "Resize Layer to Selection", p, |doc, _| {
        let sel = doc.selection.as_ref().ok_or_else(|| EngineError::Other("no selection".into()))?;
        let r = sel.content_bounds();
        if r.is_empty() {
            return Err(EngineError::Other("the selection is empty".into()));
        }
        Ok(Some(r))
    })
}

fn crop_to_content(s: &mut Session, p: &Value) -> Result<Value> {
    set_boundary(s, "Crop Layer to Content", p, |doc, id| {
        let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let surf = l.surface().ok_or_else(|| EngineError::Other("the layer has no pixels to crop to".into()))?;
        let r = opaque_bounds(surf).intersect(&l.boundary.unwrap_or_else(|| doc.bounds()));
        if r.is_empty() {
            return Err(EngineError::Other("the layer has no opaque content".into()));
        }
        Ok(Some(r))
    })
}

/// The bounds of the pixels of `s` with any alpha (every pixel, for a surface without alpha).
pub fn opaque_bounds(s: &Surface) -> Rect {
    let tiles = s.content_bounds();
    let fmt = s.format();
    if tiles.is_empty() || !fmt.alpha {
        return tiles;
    }
    let n = fmt.channels();
    let w = tiles.width() as usize;
    let data = s.read_region(tiles);
    let mut out = Rect::EMPTY;
    for (i, px) in data.chunks_exact(n).enumerate() {
        if px[n - 1] > 0.0 {
            let (x, y) = (tiles.x0 + (i % w) as i32, tiles.y0 + (i / w) as i32);
            let one = Rect::new(x, y, x + 1, y + 1);
            out = if out.is_empty() { one } else { out.union(&one) };
        }
    }
    out
}

// ---------------------------------------------------------------- Threshold Alpha

fn threshold_alpha(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.thresholdAlpha";
    let t = p.get("threshold").and_then(Value::as_f64).unwrap_or(0.5);
    if !(0.0..=1.0).contains(&t) {
        return Err(bad(CMD, "threshold must be 0..1"));
    }
    let id = crate::commands::layer_param(s, p)?;
    s.edit("Threshold Alpha", |doc, _| {
        let area = doc.bounds();
        let sel = doc.selection.clone();
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("the layer has no pixels".into()))?;
        let fmt = surf.format();
        if !fmt.alpha {
            return Err(EngineError::Other("the layer has no alpha channel".into()));
        }
        let n = fmt.channels();
        let r = surf.content_bounds().intersect(&area);
        if r.is_empty() {
            return Ok(());
        }
        let w = r.width() as usize;
        let mut data = surf.read_region(r);
        for (i, px) in data.chunks_exact_mut(n).enumerate() {
            let k = sel.as_ref().map_or(1.0, |m| m.sample_channel(r.x0 + (i % w) as i32, r.y0 + (i / w) as i32, 0));
            if k <= 0.0 {
                continue;
            }
            let a = px[n - 1];
            let hard = if f64::from(a) >= t { 1.0 } else { 0.0 };
            px[n - 1] = a + (hard - a) * k;
        }
        surf.write_region(r, &data);
        surf.prune();
        Ok(())
    })?;
    Ok(json!({ "layer": id.0, "threshold": t }))
}

// ---------------------------------------------------------------- Composite options

fn composite_options(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.compositeOptions";
    let id = crate::commands::layer_param(s, p)?;
    let mode = match p.get("mode").and_then(Value::as_str) {
        None => None,
        Some(v) => Some(
            CompositeMode::from_name(v)
                .ok_or_else(|| bad(CMD, format!("unknown composite mode `{v}` (want union|clipToBackdrop|clipToLayer|intersection)")))?,
        ),
    };
    let space = |k: &str| -> Result<Option<BlendSpace>> {
        match p.get(k).and_then(Value::as_str) {
            None => Ok(None),
            Some(v) => Ok(Some(BlendSpace::from_name(v).ok_or_else(|| bad(CMD, format!("unknown {k} `{v}` (want auto|rgbLinear|rgbPerceptual|lab)")))?)),
        }
    };
    let (blend_space, composite_space) = (space("blendSpace")?, space("compositeSpace")?);
    let legacy = p.get("legacy").and_then(Value::as_bool);
    let reset = p.get("reset").and_then(Value::as_bool).unwrap_or(false);
    let out = s.edit("Composite Options", |doc, _| {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if reset {
            l.composite = LayerComposite::default();
        }
        if let Some(m) = mode {
            l.composite.mode = m;
        }
        if let Some(b) = blend_space {
            l.composite.blend_space = b;
        }
        if let Some(c) = composite_space {
            l.composite.composite_space = c;
        }
        if let Some(g) = legacy {
            l.composite.legacy = g;
        }
        Ok(l.composite)
    })?;
    Ok(json!({ "layer": id.0, "composite": serde_json::to_value(out).unwrap_or(Value::Null) }))
}

// ---------------------------------------------------------------- Zealous Crop

/// Rows and columns of the composite `buf` (over `area`) that are one uniform colour: the ones
/// Zealous Crop removes. Returns (keep_rows, keep_cols) as index lists into `area`.
pub fn zealous_keep(px: &[[f32; 4]], w: usize, h: usize) -> (Vec<usize>, Vec<usize>) {
    let same = |a: [f32; 4], b: [f32; 4]| (a[3] <= 0.0 && b[3] <= 0.0) || (0..4).all(|c| (a[c] - b[c]).abs() <= 0.5 / 255.0);
    let rows: Vec<usize> = (0..h).filter(|&y| !(1..w).all(|x| same(px[y * w + x], px[y * w]))).collect();
    let cols: Vec<usize> = (0..w).filter(|&x| !(1..h).all(|y| same(px[y * w + x], px[x]))).collect();
    (rows, cols)
}

/// `s` over `area` with only `rows` and `cols` kept, packed from (0, 0).
fn compact_surface(s: &Surface, area: Rect, rows: &[usize], cols: &[usize]) -> Surface {
    let fmt = s.format();
    let n = fmt.channels();
    let mut out = Surface::with_default(fmt, &s.default_pixel());
    if rows.is_empty() || cols.is_empty() {
        return out;
    }
    let src = s.read_region(area);
    let w = area.width() as usize;
    let mut data = Vec::with_capacity(rows.len() * cols.len() * n);
    for &y in rows {
        for &x in cols {
            let i = (y * w + x) * n;
            data.extend_from_slice(&src[i..i + n]);
        }
    }
    out.write_region(Rect::new(0, 0, cols.len() as i32, rows.len() as i32), &data);
    out.prune();
    out
}

fn zealous_crop(s: &mut Session, _p: &Value) -> Result<Value> {
    const CMD: &str = "image.zealousCrop";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let area = d.doc.bounds();
    if area.size().area() > (1 << 28) {
        return Err(bad(CMD, "the canvas is too large to crop this way"));
    }
    let (w, h) = (area.width() as usize, area.height() as usize);
    let buf = redrob_photo_compose::flatten(&d.doc);
    let (rows, cols) = zealous_keep(&buf.px, w, h);
    drop(buf);
    if rows.len() == h && cols.len() == w {
        return Ok(json!({ "changed": false, "width": w, "height": h }));
    }
    if rows.is_empty() || cols.is_empty() {
        return Err(EngineError::Other("the whole image is one colour; nothing would remain".into()));
    }
    let (nw, nh) = (cols.len() as u32, rows.len() as u32);
    s.edit("Zealous Crop", |doc, _| {
        crate::image_cmds::for_each_surface(&mut doc.layers, true, &mut |surf, _| *surf = compact_surface(surf, area, &rows, &cols));
        for c in &mut doc.channels {
            c.surface = compact_surface(&c.surface, area, &rows, &cols);
        }
        if let Some(q) = &mut doc.quick_mask {
            q.surface = compact_surface(&q.surface, area, &rows, &cols);
        }
        if let Some(sel) = &doc.selection {
            let s2 = compact_surface(sel, area, &rows, &cols);
            doc.selection = (s2.tile_count() > 0 && !s2.content_bounds().is_empty()).then_some(s2);
        }
        let ids: Vec<redrob_photo_doc::LayerId> = doc.walk().into_iter().map(|(_, _, l)| l.id).collect();
        for id in ids {
            if let Some(l) = doc.layer_mut(id) {
                l.boundary = None;
            }
        }
        doc.size = Size::new(nw, nh);
        crate::canvas_geom::refresh(doc, crate::canvas_geom::Refresh::Shapes);
        Ok(())
    })?;
    Ok(json!({ "changed": true, "width": nw, "height": nh, "removedRows": h - rows.len(), "removedColumns": w - cols.len() }))
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "layer.boundarySize",
            "Layer Boundary Size…",
            ["Layer"],
            r##"{"layer":id?,"width":px=current,"height":px=current,"offsetX":px=0,"offsetY":px=0 (where the old content sits in the new boundary),"fill":"transparent|foreground|background|white|black"="transparent"} → {layer, boundary [x,y,w,h]}"##,
            has_layer,
            boundary_size
        ),
        spec!(
            "layer.toImageSize",
            "Layer to Image Size",
            ["Layer"],
            r##"{"layer":id?} (the layer's boundary becomes the canvas) → {layer, boundary}"##,
            has_layer,
            to_image_size
        ),
        spec!(
            "layer.resizeToSelection",
            "Resize Layer to Selection",
            ["Layer"],
            r##"{"layer":id?} (the layer's boundary becomes the selection's bounds) → {layer, boundary}"##,
            has_layer_and_selection,
            resize_to_selection
        ),
        spec!(
            "layer.cropToContent",
            "Crop Layer to Content",
            ["Layer"],
            r##"{"layer":id?} (the layer's boundary shrinks to its opaque pixels) → {layer, boundary}"##,
            has_layer,
            crop_to_content
        ),
        spec!(
            "layer.thresholdAlpha",
            "Threshold Alpha…",
            ["Layer", "Transparency"],
            r##"{"layer":id?,"threshold":0..1=0.5} (alpha at or above the threshold becomes opaque, below it transparent; within the selection) → {layer, threshold}"##,
            has_pixel_layer,
            threshold_alpha
        ),
        spec!(
            "layer.compositeOptions",
            "Composite Options…",
            ["Layer"],
            r##"{"layer":id?,"mode":"union|clipToBackdrop|clipToLayer|intersection"?,"blendSpace":"auto|rgbLinear|rgbPerceptual|lab"?,"compositeSpace":"auto|rgbLinear|rgbPerceptual|lab"?,"legacy":bool? (the pre-linear variant of the mode: perceptual spaces, alpha union),"reset":bool=false} → {layer, composite}"##,
            has_layer,
            composite_options
        ),
        spec!(
            "image.zealousCrop",
            "Zealous Crop",
            ["Image"],
            r##"{} (removes every row and column of one uniform colour, wherever it lies) → {changed, width, height, removedRows, removedColumns}"##,
            has_doc,
            zealous_crop
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with_layer() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            l.surface_mut().unwrap().fill_rect(Rect::new(10, 5, 30, 20), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s
    }

    fn active(s: &Session) -> &redrob_photo_doc::Layer {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap()
    }

    #[test]
    fn boundary_commands_set_the_layer_boundary() {
        let mut s = doc_with_layer();
        assert_eq!(active(&s).boundary, None);
        let r = s.execute("layer.cropToContent", json!({})).unwrap();
        assert_eq!(r["boundary"], json!([10, 5, 20, 15]));
        assert_eq!(active(&s).boundary, Some(Rect::new(10, 5, 30, 20)));
        s.execute("layer.boundarySize", json!({"width": 30, "height": 25, "offsetX": 5, "offsetY": 5, "fill": "white"})).unwrap();
        assert_eq!(active(&s).boundary, Some(Rect::new(5, 0, 35, 25)));
        // The added band took the fill; the old content is untouched.
        let surf = active(&s).surface().unwrap();
        assert_eq!(surf.rgba(6, 1), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(surf.rgba(15, 10), [1.0, 0.0, 0.0, 1.0]);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
        s.execute("layer.resizeToSelection", json!({})).unwrap();
        assert_eq!(active(&s).boundary, Some(Rect::new(0, 0, 8, 8)));
        s.execute("layer.toImageSize", json!({})).unwrap();
        assert_eq!(active(&s).boundary, None);
        // The composite honours the boundary: pixels outside it vanish.
        s.execute("layer.resizeToSelection", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 40, 30), &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let out = redrob_photo_compose::flatten(&s.active().unwrap().doc);
        let at = |x: usize, y: usize| out.px[y * 40 + x];
        assert_eq!(at(2, 2), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(at(20, 20), [1.0, 1.0, 1.0, 1.0], "outside the boundary the white Background shows");
    }

    #[test]
    fn threshold_alpha_makes_alpha_binary() {
        let mut s = doc_with_layer();
        s.edit("soft", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 5, 5), &[0.0, 0.0, 1.0, 0.3]);
            l.surface_mut().unwrap().fill_rect(Rect::new(5, 0, 10, 5), &[0.0, 0.0, 1.0, 0.7]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.thresholdAlpha", json!({"threshold": 0.5})).unwrap();
        let surf = active(&s).surface().unwrap();
        assert_eq!(surf.rgba(2, 2)[3], 0.0);
        assert_eq!(surf.rgba(7, 2)[3], 1.0);
        assert!(s.execute("layer.thresholdAlpha", json!({"threshold": 2})).is_err());
    }

    #[test]
    fn composite_options_round_trip_and_clip() {
        let mut s = doc_with_layer();
        let r = s.execute("layer.compositeOptions", json!({"mode": "clipToBackdrop", "blendSpace": "linear", "legacy": false})).unwrap();
        assert_eq!(r["composite"]["mode"], json!("clipToBackdrop"));
        assert_eq!(active(&s).composite.blend_space, BlendSpace::RgbLinear);
        // Clip to backdrop over a transparent backdrop: the layer disappears there.
        s.edit("clear bg", |doc, _| {
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 40, 30), &[0.0, 0.0, 0.0, 0.0]);
            Ok(())
        })
        .unwrap();
        let out = redrob_photo_compose::flatten(&s.active().unwrap().doc);
        assert_eq!(out.px[10 * 40 + 15][3], 0.0);
        s.execute("layer.compositeOptions", json!({"mode": "union"})).unwrap();
        let out = redrob_photo_compose::flatten(&s.active().unwrap().doc);
        assert_eq!(out.px[10 * 40 + 15][3], 1.0);
        assert!(s.execute("layer.compositeOptions", json!({"mode": "sideways"})).is_err());
        s.execute("layer.compositeOptions", json!({"reset": true})).unwrap();
        assert!(active(&s).composite.is_default());
    }

    #[test]
    fn zealous_crop_removes_uniform_rows_and_columns_anywhere() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 10, "height": 6, "background": "white"})).unwrap();
        s.edit("paint", |doc, _| {
            let surf = doc.layers[0].surface_mut().unwrap();
            // Two red columns with a white gap between them and white margins; one red row band.
            surf.fill_rect(Rect::new(2, 1, 4, 5), &[1.0, 0.0, 0.0, 1.0]);
            surf.fill_rect(Rect::new(6, 1, 8, 5), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let r = s.execute("image.zealousCrop", json!({})).unwrap();
        assert_eq!(r["changed"], json!(true));
        assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(4), Some(4)));
        let d = &s.active().unwrap().doc;
        assert_eq!(d.size, Size::new(4, 4));
        let surf = d.layers[0].surface().unwrap();
        assert_eq!(surf.rgba(0, 0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(surf.rgba(3, 3), [1.0, 0.0, 0.0, 1.0]);
        let r = s.execute("image.zealousCrop", json!({})).unwrap();
        assert_eq!(r["changed"], json!(false));
    }
}
