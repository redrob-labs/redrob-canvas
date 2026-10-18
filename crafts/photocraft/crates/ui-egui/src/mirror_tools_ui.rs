//! Mirror and multibrush for every tool, not only the painting strokes: a command whose params
//! carry canvas geometry (`x`/`y`, `points`, `from`/`to`) runs once per symmetry pass of the
//! document's painting symmetry, the copies sharing one history step. The brush strokes mirror in
//! the engine already (`paint.stroke`'s passes); this covers the Paint Bucket, Enclose and Fill,
//! the Gradient, the Magic Eraser and the retouching tools.

use redrob_photo_paint::StrokePoint;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Should `cmd` be mirrored? Off when the option is, when no symmetry is set, or for the brush
/// strokes the engine mirrors itself.
fn mirrors(app: &PhotocraftApp, cmd: &str) -> bool {
    if !app.ui.tool_options.mirror_all_tools {
        return false;
    }
    if matches!(cmd, "paint.stroke" | "paint.pencil" | "paint.engineStroke") {
        return false;
    }
    app.session.active().is_some_and(|st| st.symmetry.is_some() || st.wrap_around)
}

/// The geometry a command's params carry, as stroke points, and a way to write transformed
/// points back in the same order.
fn geometry(p: &Value) -> Vec<StrokePoint> {
    let mut out = Vec::new();
    if let (Some(x), Some(y)) = (p.get("x").and_then(Value::as_f64), p.get("y").and_then(Value::as_f64)) {
        out.push(StrokePoint::new(x, y, 1.0));
    }
    for k in ["from", "to"] {
        if let Some(a) = p.get(k).and_then(Value::as_array)
            && a.len() >= 2
            && let (Some(x), Some(y)) = (a[0].as_f64(), a[1].as_f64())
        {
            out.push(StrokePoint::new(x, y, 1.0));
        }
    }
    if let Some(pts) = p.get("points").and_then(Value::as_array) {
        for q in pts {
            if let Some(a) = q.as_array()
                && a.len() >= 2
                && let (Some(x), Some(y)) = (a[0].as_f64(), a[1].as_f64())
            {
                out.push(StrokePoint::new(x, y, 1.0));
            }
        }
    }
    out
}

/// `p` with its geometry replaced by `pts` (in [`geometry`]'s order). The Paint Bucket's seed is
/// re-floored so the mirrored seed lands on a pixel.
fn with_geometry(p: &Value, pts: &[StrokePoint]) -> Value {
    let mut q = p.clone();
    let mut i = 0;
    if q.get("x").and_then(Value::as_f64).is_some() && q.get("y").and_then(Value::as_f64).is_some() {
        if let Some(s) = pts.get(i) {
            q["x"] = json!(s.x.floor());
            q["y"] = json!(s.y.floor());
        }
        i += 1;
    }
    for k in ["from", "to"] {
        if q.get(k).and_then(Value::as_array).is_some_and(|a| a.len() >= 2 && a[0].as_f64().is_some() && a[1].as_f64().is_some()) {
            if let Some(s) = pts.get(i) {
                q[k] = json!([s.x, s.y]);
            }
            i += 1;
        }
    }
    if let Some(Value::Array(arr)) = q.get_mut("points") {
        for v in arr.iter_mut() {
            let Some(a) = v.as_array_mut() else { continue };
            if a.len() >= 2 && a[0].as_f64().is_some() && a[1].as_f64().is_some() {
                if let Some(s) = pts.get(i) {
                    a[0] = json!(s.x);
                    a[1] = json!(s.y);
                }
                i += 1;
            }
        }
    }
    q
}

/// Run `cmd` with `params`, then once more per symmetry pass with the geometry mirrored, all as
/// one history step. The first run's result is returned; a failing mirrored copy is ignored (a
/// seed landing on the boundary, say), as a mirrored brush dab off the canvas is.
pub fn run_mirrored(app: &mut PhotocraftApp, cmd: &str, params: Value) -> Result<Value, String> {
    if !mirrors(app, cmd) {
        return app.run(cmd, params);
    }
    let pts = geometry(&params);
    if pts.is_empty() {
        return app.run(cmd, params);
    }
    let passes = app.session.active().and_then(redrob_photo_engine::symmetry_cmds::stroke_passes).map(|s| s.reflected_passes(&pts)).unwrap_or_default();
    if passes.is_empty() {
        return app.run(cmd, params);
    }
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let key = format!("mirror:{cmd}:{}", SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let mut first = params.clone();
    first["coalesce"] = json!(key);
    let r = app.run(cmd, first)?;
    for copy in passes {
        let mut q = with_geometry(&params, &copy);
        q["coalesce"] = json!(key);
        let _ = app.run(cmd, q);
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_round_trips_through_params() {
        let p = json!({"x": 3.0, "y": 4.0, "from": [1, 2], "to": [5, 6], "points": [[7, 8, 0.5], [9, 10]], "other": true});
        let g = geometry(&p);
        assert_eq!(g.len(), 5);
        assert_eq!((g[0].x, g[0].y), (3.0, 4.0));
        assert_eq!((g[4].x, g[4].y), (9.0, 10.0));
        let moved: Vec<StrokePoint> = g.iter().map(|s| StrokePoint::new(-s.x, s.y, 1.0)).collect();
        let q = with_geometry(&p, &moved);
        assert_eq!(q["x"], -3.0);
        assert_eq!(q["from"], json!([-1.0, 2.0]));
        assert_eq!(q["points"][0], json!([-7.0, 8.0, 0.5]));
        assert_eq!(q["other"], true);
        assert!(geometry(&json!({"size": 3})).is_empty());
    }
}
