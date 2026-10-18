//! `paint.colorize`: the Colorize Mask tool's fill. The active layer is the line art; the key
//! strokes' colours propagate into its regions (`redrob_photo_algo::colorize`) and land on a
//! "Colorize" pixel layer directly below it (created the first time, rewritten after), as one
//! history step. The line art itself is never changed.

use redrob_photo_algo::colorize::{ColorizeOptions, KeyStroke};
use redrob_photo_doc::{Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "paint.colorize";
/// The output layer's name.
pub const OUTPUT_NAME: &str = "Colorize";
/// Most key strokes one fill takes.
const MAX_STROKES: usize = 256;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn color8(v: &Value) -> Option<[u8; 4]> {
    let c = crate::paint_cmds::color_value(v)?;
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    Some([q(c[0]), q(c[1]), q(c[2]), q(c[3])])
}

fn strokes(p: &Value) -> Result<Vec<KeyStroke>> {
    let arr = p.get("strokes").and_then(Value::as_array).ok_or_else(|| bad("missing `strokes`: a list of {color, points, width}"))?;
    if arr.len() > MAX_STROKES {
        return Err(bad(format!("at most {MAX_STROKES} key strokes")));
    }
    arr.iter()
        .map(|s| {
            let color = match s.get("color") {
                None | Some(Value::Null) => None,
                Some(v) => Some(color8(v).ok_or_else(|| bad(format!("a key stroke's `color` must be #rrggbb or [r, g, b], not {v}")))?),
            };
            let points: Vec<(f32, f32)> = s
                .get("points")
                .and_then(Value::as_array)
                .ok_or_else(|| bad("a key stroke needs `points`"))?
                .iter()
                .filter_map(|q| {
                    let a = q.as_array()?;
                    Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
                })
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .collect();
            if points.is_empty() {
                return Err(bad("a key stroke needs at least one finite point"));
            }
            let width = s.get("width").and_then(Value::as_f64).unwrap_or(4.0).clamp(1.0, 500.0) as f32;
            Ok(KeyStroke { color, points, width })
        })
        .collect()
}

/// The existing output layer right below `line_art`, if any.
fn output_below(doc: &redrob_photo_doc::Document, line_art: LayerId) -> Option<LayerId> {
    let walk = doc.walk();
    let i = walk.iter().position(|(_, _, l)| l.id == line_art)?;
    // `walk` is bottom-to-top: the layer below is the previous entry at the same depth.
    let (path, depth, _) = &walk[i];
    walk[..i]
        .iter()
        .rev()
        .find(|(p, d, _)| *d == *depth && p[..p.len() - 1] == path[..path.len() - 1])
        .filter(|(_, _, l)| l.name == OUTPUT_NAME && matches!(l.content, LayerContent::Raster(_)))
        .map(|(_, _, l)| l.id)
}

fn colorize(s: &mut Session, p: &Value) -> Result<Value> {
    let strokes = strokes(p)?;
    let opts = ColorizeOptions {
        threshold: crate::eraser_cmds::number(p, CMD, "threshold", 0.9, 0.01..=1.0)? as f32,
        edge_detection: p.get("edgeDetection").and_then(Value::as_bool).unwrap_or(true),
        gap_close: crate::eraser_cmds::number(p, CMD, "gapClose", 8.0, 0.0..=128.0)? as u32,
        clean_up: crate::eraser_cmds::number(p, CMD, "cleanUp", 0.0, 0.0..=100.0)? as f32,
    };
    let limit = p.get("limitToLayer").and_then(Value::as_bool).unwrap_or(false);
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let line_art = match p.get("layer").and_then(Value::as_u64) {
        Some(id) => LayerId(id),
        None => st.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?,
    };
    let surf =
        st.doc.layer(line_art).and_then(|l| l.surface()).ok_or_else(|| EngineError::Other("the Colorize Mask needs a pixel layer as line art".into()))?;
    let bounds = st.doc.bounds();
    let area = if limit { surf.content_bounds().intersect(&bounds) } else { bounds };
    if area.is_empty() {
        return Ok(json!({"filled": false}));
    }
    let img = redrob_photo_algo::selection::rgba8_image(surf, area);
    let out = redrob_photo_algo::colorize::colorize(&img, area, &strokes, &opts);
    drop(img);
    let existing = output_below(&st.doc, line_art);
    let id = s.edit("Colorize Mask", |doc, _| {
        let id = match existing {
            Some(id) if doc.layer(id).is_some() => id,
            _ => {
                let layer = Layer::raster(OUTPUT_NAME.to_string(), doc.pixel_format());
                doc.insert_below(Some(line_art), layer)
            }
        };
        let surf = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or_else(|| EngineError::Other("the Colorize layer has no pixels".into()))?;
        // Rewrite the whole output: clear, then the coloured pixels.
        let fmt = surf.format();
        surf.fill_rect(bounds, &redrob_photo_raster::from_rgba(&fmt, [0.0; 4]));
        let n = fmt.channels();
        let w = area.width() as usize;
        let mut data = vec![0.0f32; w * area.height() as usize * n];
        let mut enc = [0.0f32; 8];
        for (i, px) in out.iter().enumerate() {
            if px[3] == 0 {
                continue;
            }
            let rgba = [f32::from(px[0]) / 255.0, f32::from(px[1]) / 255.0, f32::from(px[2]) / 255.0, f32::from(px[3]) / 255.0];
            let k = redrob_photo_raster::from_rgba_into(&fmt, rgba, &mut enc);
            data[i * n..i * n + k.min(n)].copy_from_slice(&enc[..k.min(n)]);
        }
        surf.write_region(area, &data);
        surf.prune();
        Ok(id)
    })?;
    Ok(json!({"filled": true, "layer": id.0, "area": [area.x0, area.y0, area.x1, area.y1]}))
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    st.active_layer.and_then(|id| st.doc.layer(id)).and_then(|l| l.surface()).map(|_| ()).ok_or_else(|| "the Colorize Mask needs a pixel layer".into())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Colorize Mask",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id?=the active layer (the line art),"strokes":[{"color":"#rrggbb"|null (transparent key),"points":[[x,y],…],"width":px=4},…],"threshold":0.01..1=0.9 (luma under which a pixel is ink),"edgeDetection":bool=true,"gapClose":px=8,"cleanUp":0..100=0 (how far the colour stops short of running under the ink),"limitToLayer":bool=false (only inside the line art's pixels)} → {filled, layer (the Colorize layer below the line art), area}"##,
        enabled,
        run: colorize,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use redrob_photo_geom::Rect;

    #[test]
    fn colorize_writes_a_layer_below_the_line_art_and_rewrites_it() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 12, "height": 6, "background": "transparent"})).unwrap();
        let line_art = s.active().unwrap().active_layer.unwrap();
        s.edit("ink", |doc, _| {
            doc.layer_mut(line_art).unwrap().surface_mut().unwrap().fill_rect(Rect::new(6, 0, 7, 6), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let r = s.execute(CMD, json!({"strokes": [{"color": "#ff0000", "points": [[2, 3]], "width": 2}, {"color": "#0000ff", "points": [[9, 3]], "width": 2}], "gapClose": 0, "edgeDetection": false})).unwrap();
        assert_eq!(r["filled"], true);
        let id = LayerId(r["layer"].as_u64().unwrap());
        let st = s.active().unwrap();
        let walk = st.doc.walk();
        let (pos_out, pos_ink) = (walk.iter().position(|(_, _, l)| l.id == id).unwrap(), walk.iter().position(|(_, _, l)| l.id == line_art).unwrap());
        assert!(pos_out < pos_ink, "the Colorize layer sits below the line art");
        let surf = st.doc.layer(id).unwrap().surface().unwrap();
        assert_eq!(surf.pixel(1, 1), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(surf.pixel(10, 1), vec![0.0, 0.0, 1.0, 1.0]);
        assert_eq!(st.doc.layer(line_art).unwrap().surface().unwrap().pixel(1, 1)[3], 0.0, "the line art is untouched");
        // A second fill rewrites the same layer instead of adding one.
        let count = st.doc.layer_count();
        let r = s
            .execute(CMD, json!({"layer": line_art.0, "strokes": [{"color": null, "points": [[2, 3]], "width": 2}], "gapClose": 0, "edgeDetection": false}))
            .unwrap();
        assert_eq!(LayerId(r["layer"].as_u64().unwrap()), id);
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer_count(), count);
        assert_eq!(st.doc.layer(id).unwrap().surface().unwrap().pixel(1, 1)[3], 0.0, "the transparent key cleared the left side");
        assert!(s.execute(CMD, json!({"strokes": []})).is_ok());
        assert!(s.execute(CMD, json!({})).is_err());
        assert!(s.execute(CMD, json!({"strokes": [{"color": "#ff0000", "points": []}]})).is_err());
    }
}
