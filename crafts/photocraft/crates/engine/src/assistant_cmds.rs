//! Painting assistants per document (`paint.assistants.*`): rulers, parallel rulers, vanishing
//! points and perspective grids that the canvas snaps brush strokes to. Tool state like the
//! painting symmetry: not document pixels, not saved in the document, not a history step. A set
//! can be saved to and loaded from a JSON `.rassist` file.

use redrob_photo_paint::assistants::{Assistant, MAX_ASSISTANTS};
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
        "items": st.assistants.iter().map(|a| json!({"kind": a.kind(), "handles": a.handles()})).collect::<Vec<_>>(),
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
                "perspectiveGrid" => vec![[w * 0.3, h * 0.3], [w * 0.7, h * 0.3], [w * 0.8, h * 0.8], [w * 0.2, h * 0.8]],
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
    st.assistants.push(a);
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
    st.assistants.remove(i);
    Ok(json!({"count": st.assistants.len()}))
}

fn clear(s: &mut Session) -> Result<Value> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let n = st.assistants.len();
    st.assistants.clear();
    Ok(json!({"removed": n}))
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
    let file = AssistantFile { format: FORMAT.into(), version: 1, assistants: st.assistants.clone() };
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
    let items: Vec<Assistant> = file.assistants.into_iter().filter(Assistant::is_valid).collect();
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    if replace {
        st.assistants.clear();
    }
    let room = MAX_ASSISTANTS.saturating_sub(st.assistants.len());
    let added = items.len().min(room);
    st.assistants.extend(items.into_iter().take(room));
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
            params: r#"{"kind":"ruler|parallelRuler|vanishingPoint|perspectiveGrid","points":[[x,y],…]?=centred on the canvas (2 for rulers, 1 for a vanishing point, 4 corners clockwise from top left for a grid)} → {index, count}"#,
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
            shortcut: None,
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
            params: "{} → {snap, items:[{kind, handles}]}",
            enabled: has_doc,
            run: |s, _| Ok(inspect(s.active().ok_or(EngineError::NoDocument)?)),
            journal: false,
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
}
