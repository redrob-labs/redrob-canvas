//! Painting assistants per document (`paint.assistants.*`): rulers, parallel rulers, vanishing
//! points and perspective grids that the canvas snaps brush strokes to. Tool state like the
//! painting symmetry: not document pixels, not saved in the document, not a history step. A set
//! can be saved to and loaded from a JSON `.rassist` file.

use redrob_photo_paint::assistants::{Assistant, AssistantDisplay, AssistantStyle, MAX_ASSISTANTS, MAX_SUBDIVISIONS};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session, commands::CommandSpec};

/// Largest assistant file read.
const MAX_FILE_BYTES: u64 = 1 << 20;
/// The file format written by `paint.assistants.save`.
const FORMAT: &str = "redrob-painting-assistants";

#[derive(Serialize, Deserialize)]
struct AssistantFile {
    format: String,
    version: u32,
    assistants: Vec<Assistant>,
    /// Per-assistant looks and areas (version 2; absent in version 1 files).
    #[serde(default)]
    styles: Vec<AssistantStyle>,
    #[serde(default)]
    display: Option<AssistantDisplay>,
}

/// Keep `assistant_styles` as long as `assistants` (a style per assistant).
fn align_styles(st: &mut crate::DocState) {
    st.assistant_styles.resize(st.assistants.len(), AssistantStyle::default());
}

/// The style of assistant `i` (the default when none was set).
pub fn style_of(st: &crate::DocState, i: usize) -> AssistantStyle {
    st.assistant_styles.get(i).copied().unwrap_or_default()
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn point(v: &Value, cmd: &str, what: &str) -> Result<[f64; 2]> {
    let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad(cmd, format!("`{what}` must be [x, y]")))?;
    let x = a[0].as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, format!("`{what}` must be finite numbers")))?;
    let y = a[1].as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, format!("`{what}` must be finite numbers")))?;
    Ok([x, y])
}

fn index(s: &Session, p: &Value, cmd: &str) -> Result<usize> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "missing `index`"))? as usize;
    if i >= st.assistants.len() {
        return Err(bad(cmd, format!("no assistant at index {i} ({} in the document)", st.assistants.len())));
    }
    Ok(i)
}

pub fn inspect(st: &crate::DocState) -> Value {
    json!({
        "snap": st.assistants_snap,
        "display": st.assistant_display,
        "items": st
            .assistants
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let mut v = json!({"kind": a.kind(), "handles": a.handles(), "style": style_of(st, i)});
                if let Some((major, minor)) = a.subdivisions() {
                    v["subdivisions"] = json!(major);
                    v["minor"] = json!(minor);
                }
                v
            })
            .collect::<Vec<_>>(),
    })
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.assistants.add";
    let kind = p.get("kind").and_then(Value::as_str).ok_or_else(|| bad(cmd, format!("`kind` must be one of {}", Assistant::KINDS.join(", "))))?;
    let n = Assistant::handle_count(kind).ok_or_else(|| bad(cmd, format!("`kind` must be one of {}", Assistant::KINDS.join(", "))))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let points: Vec<[f64; 2]> = match p.get("points") {
        Some(Value::Array(a)) => a.iter().map(|v| point(v, cmd, "points")).collect::<Result<_>>()?,
        Some(_) => return Err(bad(cmd, "`points` must be a list of [x, y]")),
        // Default placement: centred on the canvas.
        None => {
            let (w, h) = (f64::from(st.doc.size.width), f64::from(st.doc.size.height));
            match kind {
                "vanishingPoint" => vec![[w / 2.0, h / 2.0]],
                "perspectiveGrid" | "perspectiveEllipse" => vec![[w * 0.3, h * 0.3], [w * 0.7, h * 0.3], [w * 0.8, h * 0.8], [w * 0.2, h * 0.8]],
                "spline" => vec![[w * 0.2, h * 0.7], [w * 0.35, h * 0.3], [w * 0.65, h * 0.3], [w * 0.8, h * 0.7]],
                "ellipse" | "concentricEllipse" | "fishEye" => vec![[w * 0.2, h / 2.0], [w * 0.8, h / 2.0], [w / 2.0, h * 0.3]],
                "twoPointPerspective" => vec![[w * 0.05, h * 0.4], [w * 0.95, h * 0.4], [w / 2.0, h * 0.7]],
                _ => vec![[w * 0.2, h / 2.0], [w * 0.8, h / 2.0]],
            }
        }
    };
    if points.len() != n {
        return Err(bad(cmd, format!("a {kind} assistant takes {n} point(s), got {}", points.len())));
    }
    let a = Assistant::from_points(kind, &points).ok_or_else(|| bad(cmd, "the points are degenerate (coincident handles) or out of range"))?;
    if st.assistants.len() >= MAX_ASSISTANTS {
        return Err(bad(cmd, format!("a document holds at most {MAX_ASSISTANTS} assistants")));
    }
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    align_styles(st);
    st.assistants.push(a);
    st.assistant_styles.push(AssistantStyle::default());
    Ok(json!({"index": st.assistants.len() - 1, "count": st.assistants.len()}))
}

fn move_assistant(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.assistants.move";
    let i = index(s, p, cmd)?;
    let handle = match p.get("handle") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or_else(|| bad(cmd, "`handle` must be a handle index"))? as usize),
    };
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let a = &mut st.assistants[i];
    let ok = match (handle, p.get("to"), p.get("delta")) {
        (Some(h), Some(to), _) => a.set_handle(h, point(to, cmd, "to")?),
        (None, _, Some(d)) => a.translate(point(d, cmd, "delta")?),
        (None, Some(to), None) => {
            let c = a.center();
            let to = point(to, cmd, "to")?;
            a.translate([to[0] - c[0], to[1] - c[1]])
        }
        _ => return Err(bad(cmd, "give `handle` + `to`, or `to` / `delta` for the whole assistant")),
    };
    if !ok {
        return Err(bad(cmd, "the move would make the assistant degenerate or leave the canvas range"));
    }
    Ok(json!({"index": i, "kind": a.kind(), "handles": a.handles()}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let i = index(s, p, "paint.assistants.remove")?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    align_styles(st);
    st.assistants.remove(i);
    st.assistant_styles.remove(i);
    Ok(json!({"count": st.assistants.len()}))
}

fn clear(s: &mut Session) -> Result<Value> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let n = st.assistants.len();
    st.assistants.clear();
    st.assistant_styles.clear();
    Ok(json!({"removed": n}))
}

fn color3(v: &Value, cmd: &str, what: &str) -> Result<[f32; 3]> {
    match v {
        Value::String(h) => {
            let h = h.trim_start_matches('#');
            let c = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| f32::from(v) / 255.0);
            match (c(0), c(2), c(4)) {
                (Some(r), Some(g), Some(b)) => Ok([r, g, b]),
                _ => Err(bad(cmd, format!("`{what}` must be #rrggbb or [r, g, b]"))),
            }
        }
        Value::Array(a) if a.len() >= 3 => {
            let f = |i: usize| a[i].as_f64().filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0) as f32);
            match (f(0), f(1), f(2)) {
                (Some(r), Some(g), Some(b)) => Ok([r, g, b]),
                _ => Err(bad(cmd, format!("`{what}` must be #rrggbb or [r, g, b] in 0..1"))),
            }
        }
        _ => Err(bad(cmd, format!("`{what}` must be #rrggbb or [r, g, b]"))),
    }
}

fn opacity01(v: &Value, cmd: &str) -> Result<f32> {
    let o = v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, "`opacity` must be a number 0..1"))?;
    Ok(o.clamp(0.0, 1.0) as f32)
}

/// `paint.assistants.style`: one assistant's colour, opacity, handle visibility and limit area.
fn style(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.assistants.style";
    let i = index(s, p, cmd)?;
    let mut st_style = style_of(s.active().ok_or(EngineError::NoDocument)?, i);
    match p.get("color") {
        None => {}
        Some(Value::Null) => st_style.color = None,
        Some(v) => st_style.color = Some(color3(v, cmd, "color")?),
    }
    match p.get("opacity") {
        None => {}
        Some(Value::Null) => st_style.opacity = None,
        Some(v) => st_style.opacity = Some(opacity01(v, cmd)?),
    }
    if let Some(v) = p.get("showHandles") {
        st_style.show_handles = v.as_bool().ok_or_else(|| bad(cmd, "`showHandles` must be a boolean"))?;
    }
    match p.get("limit") {
        None => {}
        Some(Value::Null) => st_style.limit = None,
        Some(Value::Array(a)) if a.len() == 2 => {
            let (c0, c1) = (point(&a[0], cmd, "limit")?, point(&a[1], cmd, "limit")?);
            if (c0[0] - c1[0]).abs() < 1.0 || (c0[1] - c1[1]).abs() < 1.0 {
                return Err(bad(cmd, "`limit` corners must span at least a pixel each way"));
            }
            st_style.limit = Some([c0, c1]);
        }
        Some(_) => return Err(bad(cmd, "`limit` must be two corners [[x0,y0],[x1,y1]] or null")),
    }
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    align_styles(st);
    st.assistant_styles[i] = st_style;
    Ok(json!({"index": i, "style": st_style}))
}

/// `paint.assistants.display`: the global colour, opacity and the View menu toggles.
fn display(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.assistants.display";
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let mut d = st.assistant_display;
    if let Some(v) = p.get("color").filter(|v| !v.is_null()) {
        d.color = color3(v, cmd, "color")?;
    }
    if let Some(v) = p.get("opacity").filter(|v| !v.is_null()) {
        d.opacity = opacity01(v, cmd)?;
    }
    for (key, slot) in [("show", &mut d.show), ("showPreviews", &mut d.show_previews)] {
        match p.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::Bool(b)) => *slot = *b,
            Some(Value::String(t)) if t == "toggle" => *slot = !*slot,
            Some(_) => return Err(bad(cmd, format!("`{key}` must be a boolean or \"toggle\""))),
        }
    }
    st.assistant_display = d;
    Ok(json!(d))
}

/// `paint.assistants.subdivisions`: a ruler's major and minor notches.
fn subdivisions(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.assistants.subdivisions";
    let i = index(s, p, cmd)?;
    let count = |k: &str, d: u32| -> Result<u32> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(d),
            Some(v) => {
                let n = v.as_u64().ok_or_else(|| bad(cmd, format!("`{k}` must be a whole number 0..={MAX_SUBDIVISIONS}")))?;
                Ok(n.min(u64::from(MAX_SUBDIVISIONS)) as u32)
            }
        }
    };
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let (major0, minor0) = st.assistants[i].subdivisions().ok_or_else(|| bad(cmd, "only rulers have subdivisions"))?;
    let (major, minor) = (count("subdivisions", major0)?, count("minor", minor0)?);
    st.assistants[i].set_subdivisions(major, minor);
    Ok(json!({"index": i, "subdivisions": major, "minor": minor}))
}

/// `paint.assistants.twoPoint`: a 2-point perspective's ray density and vertical ruler.
fn two_point(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.assistants.twoPoint";
    let i = index(s, p, cmd)?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let Assistant::TwoPointPerspective { density, vertical, .. } = &st.assistants[i] else {
        return Err(bad(cmd, "only a 2-point perspective assistant has density and a vertical ruler"));
    };
    let (mut d, mut v) = (*density, *vertical);
    if let Some(x) = p.get("density").filter(|v| !v.is_null()) {
        d = x.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, "`density` must be a number 0.25..4"))?;
    }
    if let Some(x) = p.get("vertical").filter(|v| !v.is_null()) {
        v = x.as_bool().ok_or_else(|| bad(cmd, "`vertical` must be a boolean"))?;
    }
    st.assistants[i].set_two_point(d, v);
    let Assistant::TwoPointPerspective { density, vertical, .. } = &st.assistants[i] else { unreachable!() };
    Ok(json!({"index": i, "density": density, "vertical": vertical}))
}

fn snap(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    st.assistants_snap = match p.get("enabled") {
        None => !st.assistants_snap,
        Some(v) => v.as_bool().ok_or_else(|| bad("paint.assistants.snap", "`enabled` must be a boolean"))?,
    };
    Ok(json!({"enabled": st.assistants_snap}))
}

fn path_param(p: &Value, cmd: &str) -> Result<String> {
    p.get("path").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).ok_or_else(|| bad(cmd, "missing `path`"))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.assistants.save";
    let path = path_param(p, cmd)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let mut styles = st.assistant_styles.clone();
    styles.resize(st.assistants.len(), AssistantStyle::default());
    let file = AssistantFile { format: FORMAT.into(), version: 2, assistants: st.assistants.clone(), styles, display: Some(st.assistant_display) };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|e| EngineError::Other(e.to_string()))?;
    std::fs::write(&path, bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(json!({"path": path, "count": st.assistants.len()}))
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.assistants.load";
    let replace = p.get("replace").and_then(Value::as_bool).unwrap_or(true);
    // The file's text may come straight from a file picker (`data`) instead of a path.
    let (path, bytes) = match p.get("data").and_then(Value::as_str) {
        Some(text) => {
            if text.len() as u64 > MAX_FILE_BYTES {
                return Err(bad(cmd, "`data` is too large"));
            }
            (p.get("path").and_then(Value::as_str).unwrap_or("assistants").to_string(), text.as_bytes().to_vec())
        }
        None => {
            let path = path_param(p, cmd)?;
            let len = std::fs::metadata(&path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?.len();
            if len > MAX_FILE_BYTES {
                return Err(bad(cmd, format!("{path}: file too large ({len} bytes)")));
            }
            let bytes = std::fs::read(&path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
            (path, bytes)
        }
    };
    let file: AssistantFile = serde_json::from_slice(&bytes).map_err(|e| EngineError::Other(format!("{path}: not an assistants file: {e}")))?;
    if file.format != FORMAT {
        return Err(EngineError::Other(format!("{path}: not an assistants file (format `{}`)", file.format)));
    }
    let mut styles = file.styles;
    styles.resize(file.assistants.len(), AssistantStyle::default());
    let items: Vec<(Assistant, AssistantStyle)> = file.assistants.into_iter().zip(styles).filter(|(a, _)| a.is_valid()).collect();
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    align_styles(st);
    if replace {
        st.assistants.clear();
        st.assistant_styles.clear();
        if let Some(d) = file.display {
            st.assistant_display = d;
        }
    }
    let room = MAX_ASSISTANTS.saturating_sub(st.assistants.len());
    let added = items.len().min(room);
    for (a, style) in items.into_iter().take(room) {
        st.assistants.push(a);
        st.assistant_styles.push(style);
    }
    Ok(json!({"path": path, "added": added, "count": st.assistants.len()}))
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn has_assistants(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|st| !st.assistants.is_empty()).map(|_| ()).ok_or_else(|| "no painting assistants".into())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "paint.assistants.add",
            label: "Add Painting Assistant",
            menu: &[],
            shortcut: None,
            params: r#"{"kind":"ruler|infiniteRuler|parallelRuler|vanishingPoint|perspectiveGrid|ellipse|concentricEllipse|perspectiveEllipse|spline|twoPointPerspective|fishEye|curvilinear","points":[[x,y],…]?=centred on the canvas (2 for rulers and the curvilinear grid, 1 for a vanishing point, 3 for ellipses, the fish-eye and the 2-point perspective, 4 corners clockwise from top left for a grid or perspective ellipse, 4 for a spline)} → {index, count}"#,
            enabled: has_doc,
            run: add,
            journal: true,
        },
        CommandSpec {
            id: "paint.assistants.move",
            label: "Move Painting Assistant",
            menu: &[],
            shortcut: None,
            params: r#"{"index":n,"handle":h?,"to":[x,y]? (the handle, or the whole assistant's centre),"delta":[dx,dy]? (the whole assistant)} → {index, kind, handles}"#,
            enabled: has_assistants,
            run: move_assistant,
            journal: true,
        },
        CommandSpec {
            id: "paint.assistants.remove",
            label: "Remove Painting Assistant",
            menu: &[],
            shortcut: None,
            params: r#"{"index":n} → {count}"#,
            enabled: has_assistants,
            run: remove,
            journal: true,
        },
        CommandSpec {
            id: "paint.assistants.clear",
            label: "Clear Painting Assistants",
            menu: &[],
            shortcut: None,
            params: "{} → {removed}",
            enabled: has_assistants,
            run: |s, _| clear(s),
            journal: true,
        },
        CommandSpec {
            id: "paint.assistants.snap",
            label: "Snap to Painting Assistants",
            menu: &[],
            shortcut: Some("Ctrl+Shift+L"),
            params: r#"{"enabled":bool?} — toggle: brush strokes follow the assistants"#,
            enabled: has_doc,
            run: snap,
            journal: true,
        },
        CommandSpec {
            id: "paint.assistants.save",
            label: "Save Painting Assistants",
            menu: &[],
            shortcut: None,
            params: r#"{"path":"file.rassist"} → {path, count}"#,
            enabled: has_doc,
            run: save,
            journal: false,
        },
        CommandSpec {
            id: "paint.assistants.load",
            label: "Load Painting Assistants",
            menu: &[],
            shortcut: None,
            params: r#"{"path":"file.rassist"|"data":json text,"replace":bool=true} → {path, added, count}"#,
            enabled: has_doc,
            run: load,
            journal: true,
        },
        CommandSpec {
            id: "paint.assistants.list",
            label: "List Painting Assistants",
            menu: &[],
            shortcut: None,
            params: "{} → {snap, display, items:[{kind, handles, style, subdivisions?, minor?}]}",
            enabled: has_doc,
            run: |s, _| Ok(inspect(s.active().ok_or(EngineError::NoDocument)?)),
            journal: false,
        },
        CommandSpec {
            id: "paint.assistants.style",
            label: "Painting Assistant Style",
            menu: &[],
            shortcut: None,
            params: r#"{"index":n,"color":"#rrggbb"|[r,g,b]|null (null = the global colour),"opacity":0..1|null,"showHandles":bool?,"limit":[[x0,y0],[x1,y1]]|null (the area the assistant draws and snaps in)} → {index, style}"#,
            enabled: has_assistants,
            run: style,
            journal: true,
        },
        CommandSpec {
            id: "paint.assistants.display",
            label: "Painting Assistants Display",
            menu: &[],
            shortcut: None,
            params: r#"{"color":"#rrggbb"?,"opacity":0..1?,"show":bool|"toggle"? (View › Show Painting Assistants),"showPreviews":bool|"toggle"? (View › Show Assistant Previews)} → the display settings"#,
            enabled: has_doc,
            run: display,
            journal: true,
        },
        CommandSpec {
            id: "view.show.paintingAssistants",
            label: "Painting Assistants",
            menu: &[],
            shortcut: None,
            params: r#"{"on":bool?} — toggle: draw the painting assistants with the painting tools"#,
            enabled: has_doc,
            run: |s, p| {
                let on = match p.get("on") {
                    None | Some(Value::Null) => Value::String("toggle".into()),
                    Some(v) => v.clone(),
                };
                display(s, &json!({"show": on}))
            },
            journal: true,
        },
        CommandSpec {
            id: "view.show.assistantPreviews",
            label: "Assistant Previews",
            menu: &[],
            shortcut: None,
            params: r#"{"on":bool?} — toggle: show the snapped stroke's line while painting"#,
            enabled: has_doc,
            run: |s, p| {
                let on = match p.get("on") {
                    None | Some(Value::Null) => Value::String("toggle".into()),
                    Some(v) => v.clone(),
                };
                display(s, &json!({"showPreviews": on}))
            },
            journal: true,
        },
        CommandSpec {
            id: "paint.assistants.subdivisions",
            label: "Painting Assistant Subdivisions",
            menu: &[],
            shortcut: None,
            params: r#"{"index":n,"subdivisions":0..64? (major notches along a ruler),"minor":0..64? (minor notches per major)} → {index, subdivisions, minor}"#,
            enabled: has_assistants,
            run: subdivisions,
            journal: true,
        },
        CommandSpec {
            id: "paint.assistants.twoPoint",
            label: "2-Point Perspective Settings",
            menu: &[],
            shortcut: None,
            params: r#"{"index":n,"density":0.25..4? (how dense the rays are),"vertical":bool? (show the vertical ruler)} → {index, density, vertical}"#,
            enabled: has_assistants,
            run: two_point,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_move_remove_and_round_trip_through_a_file() {
        let mut s = Session::new();
        assert!(s.execute("paint.assistants.add", json!({"kind": "ruler"})).is_err(), "needs a document");
        s.execute("file.new", json!({"width": 100, "height": 80})).unwrap();
        let r = s.execute("paint.assistants.add", json!({"kind": "ruler"})).unwrap();
        assert_eq!(r, json!({"index": 0, "count": 1}));
        assert_eq!(s.active().unwrap().assistants[0].handles(), vec![[20.0, 40.0], [80.0, 40.0]]);
        s.execute("paint.assistants.add", json!({"kind": "vanishingPoint", "points": [[10, 10]]})).unwrap();
        assert!(s.execute("paint.assistants.add", json!({"kind": "vanishingPoint", "points": [[10, 10], [2, 2]]})).is_err());
        assert!(s.execute("paint.assistants.add", json!({"kind": "ruler", "points": [[1, 1], [1, 1]]})).is_err());
        assert!(s.execute("paint.assistants.add", json!({"kind": "triangle"})).is_err());
        let r = s.execute("paint.assistants.move", json!({"index": 1, "delta": [5, 5]})).unwrap();
        assert_eq!(r["handles"], json!([[15.0, 15.0]]));
        let r = s.execute("paint.assistants.move", json!({"index": 0, "handle": 1, "to": [80, 60]})).unwrap();
        assert_eq!(r["handles"], json!([[20.0, 40.0], [80.0, 60.0]]));
        assert!(s.execute("paint.assistants.move", json!({"index": 0, "handle": 1, "to": [20, 40]})).is_err());
        assert!(s.execute("paint.assistants.move", json!({"index": 7, "delta": [1, 1]})).is_err());
        assert_eq!(s.execute("paint.assistants.snap", json!({})).unwrap()["enabled"], false);
        assert_eq!(s.execute("paint.assistants.snap", json!({"enabled": true})).unwrap()["enabled"], true);
        let dir = std::env::temp_dir().join(format!("redrob-assistants-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("set.rassist");
        let r = s.execute("paint.assistants.save", json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(r["count"], 2);
        s.execute("paint.assistants.remove", json!({"index": 0})).unwrap();
        assert_eq!(s.execute("paint.assistants.clear", json!({})).unwrap()["removed"], 1);
        assert!(s.active().unwrap().assistants.is_empty());
        let r = s.execute("paint.assistants.load", json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(r["count"], 2);
        let list = s.execute("paint.assistants.list", json!({})).unwrap();
        assert_eq!(list["items"][0]["kind"], "ruler");
        assert_eq!(list["items"][1]["handles"], json!([[15.0, 15.0]]));
        std::fs::write(&path, b"{\"format\":\"other\",\"version\":1,\"assistants\":[]}").unwrap();
        assert!(s.execute("paint.assistants.load", json!({"path": path.to_string_lossy()})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn styles_display_subdivisions_and_two_point_settings() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100, "height": 80})).unwrap();
        s.execute("paint.assistants.add", json!({"kind": "ruler"})).unwrap();
        s.execute("paint.assistants.add", json!({"kind": "twoPointPerspective"})).unwrap();
        s.execute("paint.assistants.add", json!({"kind": "ellipse"})).unwrap();
        assert_eq!(s.active().unwrap().assistant_styles.len(), 3);
        let r = s.execute("paint.assistants.style", json!({"index": 2, "color": "#ff0000", "opacity": 0.5, "limit": [[0, 0], [50, 40]]})).unwrap();
        assert_eq!(r["style"]["color"], json!([1.0, 0.0, 0.0]));
        assert_eq!(r["style"]["limit"], json!([[0.0, 0.0], [50.0, 40.0]]));
        assert!(s.execute("paint.assistants.style", json!({"index": 2, "limit": [[0, 0], [0, 40]]})).is_err(), "a flat limit is refused");
        let r = s.execute("paint.assistants.style", json!({"index": 2, "color": null, "limit": null})).unwrap();
        assert_eq!(r["style"]["color"], Value::Null);
        assert_eq!(r["style"]["opacity"], 0.5);
        let r = s.execute("paint.assistants.subdivisions", json!({"index": 0, "subdivisions": 5, "minor": 2})).unwrap();
        assert_eq!(r, json!({"index": 0, "subdivisions": 5, "minor": 2}));
        assert!(s.execute("paint.assistants.subdivisions", json!({"index": 2, "subdivisions": 5})).is_err(), "ellipses have no notches");
        let r = s.execute("paint.assistants.twoPoint", json!({"index": 1, "density": 2.0, "vertical": false})).unwrap();
        assert_eq!(r, json!({"index": 1, "density": 2.0, "vertical": false}));
        assert!(s.execute("paint.assistants.twoPoint", json!({"index": 0, "density": 2.0})).is_err());
        let d = s.execute("paint.assistants.display", json!({"color": "#00ff00", "opacity": 0.25, "show": "toggle"})).unwrap();
        assert_eq!(d["show"], false);
        assert_eq!(d["opacity"], 0.25);
        // Removing one keeps the styles aligned; a saved set carries styles and display back.
        s.execute("paint.assistants.remove", json!({"index": 0})).unwrap();
        assert_eq!(s.active().unwrap().assistant_styles.len(), 2);
        assert_eq!(s.active().unwrap().assistant_styles[1].opacity, Some(0.5));
        let dir = std::env::temp_dir().join(format!("redrob-assistants-styles-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("set.rassist");
        s.execute("paint.assistants.save", json!({"path": path.to_string_lossy()})).unwrap();
        s.execute("paint.assistants.clear", json!({})).unwrap();
        s.execute("paint.assistants.display", json!({"show": true})).unwrap();
        s.execute("paint.assistants.load", json!({"path": path.to_string_lossy()})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.assistant_styles[1].opacity, Some(0.5));
        assert!(!st.assistant_display.show, "the file's display settings replace the document's");
        let list = s.execute("paint.assistants.list", json!({})).unwrap();
        assert_eq!(list["items"][0]["kind"], "twoPointPerspective");
        assert_eq!(list["items"][1]["style"]["opacity"], 0.5);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
