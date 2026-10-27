//! Named buffers (Edit › Buffer): Cut Named, Copy Named and Copy Visible Named put the clipboard
//! contents into a buffer with a name, Paste Named pastes one back (as a layer, in place, or as a
//! new document), and the Buffers panel lists, renames, previews and deletes them. The buffers
//! live beside the clipboard in the session; the clipboard itself is left as it was.

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::edit_cmds::Clip;
use crate::{EngineError, Result, Session};

/// One named buffer.
pub struct NamedBuffer {
    pub name: String,
    pub clip: Clip,
}

/// Most buffers kept.
pub const MAX_BUFFERS: usize = 256;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_buffers(s: &Session) -> std::result::Result<(), String> {
    if s.buffers.is_empty() { Err("no named buffers".into()) } else { Ok(()) }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// A buffer name that is not taken: `want`, else `want 2`, `want 3`...
fn unique(s: &Session, want: &str) -> String {
    let taken = |n: &str| s.buffers.iter().any(|b| b.name.eq_ignore_ascii_case(n));
    if !taken(want) {
        return want.to_string();
    }
    (2..).map(|i| format!("{want} {i}")).find(|n| !taken(n)).unwrap_or_else(|| want.to_string())
}

fn name_param(s: &Session, p: &Value, default: &str) -> String {
    let want = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).unwrap_or(default);
    unique(s, want)
}

/// Runs `copy_cmd` with the clipboard set aside, then files what it produced under `name`.
fn capture(s: &mut Session, copy_cmd: &str, p: &Value, default: &str) -> Result<Value> {
    if s.buffers.len() >= MAX_BUFFERS {
        return Err(EngineError::Other(format!("there are already {MAX_BUFFERS} named buffers; delete some first")));
    }
    let name = name_param(s, p, default);
    let saved = s.clipboard.take();
    let result = s.execute(copy_cmd, json!({}));
    let taken = s.clipboard.take();
    s.clipboard = saved;
    result?;
    let clip = taken.ok_or_else(|| EngineError::Other("nothing was copied".into()))?;
    let b = clip.bounds;
    s.buffers.push(NamedBuffer { name: name.clone(), clip });
    Ok(json!({ "name": name, "bounds": [b.x0, b.y0, b.width(), b.height()] }))
}

fn find(s: &Session, cmd: &str, p: &Value) -> Result<usize> {
    match p.get("name").and_then(Value::as_str) {
        Some(n) => s.buffers.iter().position(|b| b.name == n).ok_or_else(|| bad(cmd, format!("no buffer named \"{n}\""))),
        None => s.buffers.len().checked_sub(1).ok_or_else(|| bad(cmd, "no named buffers")),
    }
}

/// `edit.buffer.paste`: the named buffer as a layer (`as` = layer | inPlace | new | into).
fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "edit.buffer.paste";
    let i = find(s, CMD, p)?;
    let how = p.get("as").and_then(Value::as_str).unwrap_or("layer");
    let (cmd, params) = match how {
        "layer" => ("edit.paste", json!({ "center": p.get("center").cloned().unwrap_or(Value::Null) })),
        "inPlace" => ("edit.pasteSpecial.pasteInPlace", json!({})),
        "into" => ("edit.pasteSpecial.pasteInto", json!({})),
        "new" => ("file.newFromClipboard", json!({})),
        other => return Err(bad(CMD, format!("`as` must be layer, inPlace, into or new (got `{other}`)"))),
    };
    if cmd != "file.newFromClipboard" && s.active().is_none() {
        return Err(EngineError::NoDocument);
    }
    let saved = s.clipboard.take();
    s.clipboard = Some(s.buffers[i].clip.clone());
    let result = s.execute(cmd, params);
    s.clipboard = saved;
    let r = result?;
    Ok(json!({ "name": s.buffers[i].name, "as": how, "pasted": r }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let i = find(s, "edit.buffer.delete", p)?;
    let b = s.buffers.remove(i);
    Ok(json!({ "deleted": b.name, "remaining": s.buffers.len() }))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "edit.buffer.rename";
    let i = find(s, CMD, p)?;
    let to = p.get("to").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(CMD, "pass `to` (the new name)"))?;
    let to = if s.buffers.iter().enumerate().any(|(j, b)| j != i && b.name.eq_ignore_ascii_case(to)) { unique(s, to) } else { to.to_string() };
    let from = std::mem::replace(&mut s.buffers[i].name, to.clone());
    Ok(json!({ "from": from, "to": to }))
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let items: Vec<Value> = s
        .buffers
        .iter()
        .map(|b| {
            let r = b.clip.bounds;
            json!({ "name": b.name, "width": r.width(), "height": r.height(), "layers": b.clip.layers.as_ref().map_or(0, |d| d.layers.len()), "resolution": b.clip.resolution })
        })
        .collect();
    Ok(json!({ "buffers": items }))
}

/// A small RGBA8 thumbnail (at most `max` on a side) of buffer `i`, for the panel.
pub fn thumbnail(s: &Session, i: usize, max: u32) -> Option<(u32, u32, Vec<u8>)> {
    let b = s.buffers.get(i)?;
    let r = b.clip.bounds;
    if r.is_empty() {
        return None;
    }
    let scale = (f64::from(max) / f64::from(r.width().max(r.height()))).min(1.0);
    let tw = ((f64::from(r.width()) * scale).round() as u32).max(1);
    let th = ((f64::from(r.height()) * scale).round() as u32).max(1);
    let mut px = vec![[0.0f32; 4]; (r.width() as usize) * (r.height() as usize)];
    b.clip.surface.read_rgba_into(r, &mut px);
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for y in 0..th {
        let sy = ((u64::from(y) * u64::from(r.height())) / u64::from(th)).min(u64::from(r.height()) - 1) as usize;
        for x in 0..tw {
            let sx = ((u64::from(x) * u64::from(r.width())) / u64::from(tw)).min(u64::from(r.width()) - 1) as usize;
            let p = px[sy * r.width() as usize + sx];
            out.extend(p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
    }
    Some((tw, th, out))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "edit.buffer.cut",
            label: "Cut Named…",
            menu: &["Edit", "Buffer"],
            shortcut: None,
            params: r##"{"name":string?="Buffer"} → {name, bounds}"##,
            enabled: has_doc,
            run: |s, p| capture(s, "edit.cut", p, "Buffer"),
            journal: true,
        },
        CommandSpec {
            id: "edit.buffer.copy",
            label: "Copy Named…",
            menu: &["Edit", "Buffer"],
            shortcut: None,
            params: r##"{"name":string?="Buffer"} → {name, bounds}"##,
            enabled: has_doc,
            run: |s, p| capture(s, "edit.copy", p, "Buffer"),
            journal: false,
        },
        CommandSpec {
            id: "edit.buffer.copyVisible",
            label: "Copy Visible Named…",
            menu: &["Edit", "Buffer"],
            shortcut: None,
            params: r##"{"name":string?="Buffer"} → {name, bounds}"##,
            enabled: has_doc,
            run: |s, p| capture(s, "edit.copyMerged", p, "Buffer"),
            journal: false,
        },
        CommandSpec {
            id: "edit.buffer.paste",
            label: "Paste Named…",
            menu: &["Edit", "Buffer"],
            shortcut: None,
            params: r##"{"name":string? (default: the newest buffer),"as":"layer|inPlace|into|new"="layer","center":[x,y]?} → {name, as, pasted}"##,
            enabled: has_buffers,
            run: paste,
            journal: true,
        },
        CommandSpec {
            id: "edit.buffer.delete",
            label: "Delete Buffer",
            menu: &[],
            shortcut: None,
            params: r##"{"name":string?} → {deleted, remaining}"##,
            enabled: has_buffers,
            run: delete,
            journal: false,
        },
        CommandSpec {
            id: "edit.buffer.rename",
            label: "Rename Buffer",
            menu: &[],
            shortcut: None,
            params: r##"{"name":string?,"to":string} → {from, to}"##,
            enabled: has_buffers,
            run: rename,
            journal: false,
        },
        CommandSpec {
            id: "edit.buffer.list",
            label: "List Buffers",
            menu: &[],
            shortcut: None,
            params: r##"{} → {buffers: [{name, width, height, layers, resolution}]}"##,
            enabled: always,
            run: list,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_buffers_round_trip_without_touching_the_clipboard() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10})).unwrap_or_else(|e| panic!("{e}"));
        s.execute("select.all", json!({})).unwrap_or_else(|e| panic!("{e}"));
        s.execute("edit.copy", json!({})).unwrap_or_else(|e| panic!("{e}"));
        let before = s.clipboard.as_ref().map(|c| c.bounds);
        let r = s.execute("edit.buffer.copy", json!({"name": "Sky"})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r["name"], "Sky");
        assert_eq!(s.clipboard.as_ref().map(|c| c.bounds), before, "the clipboard is left alone");
        let r = s.execute("edit.buffer.copy", json!({"name": "Sky"})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r["name"], "Sky 2");
        let layers = s.active().map(|d| d.doc.layers.len()).unwrap_or(0);
        s.execute("edit.buffer.paste", json!({"name": "Sky"})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.active().map(|d| d.doc.layers.len()), Some(layers + 1));
        s.execute("edit.buffer.rename", json!({"name": "Sky 2", "to": "Sea"})).unwrap_or_else(|e| panic!("{e}"));
        let l = s.execute("edit.buffer.list", json!({})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(l["buffers"].as_array().map(Vec::len), Some(2));
        s.execute("edit.buffer.delete", json!({"name": "Sea"})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.buffers.len(), 1);
        assert!(thumbnail(&s, 0, 32).is_some());
    }
}
