//! Document scripts (`doc.scripts`, `doc.addScript`, `doc.setScript`, `doc.removeScript`): the
//! SVG `<script>` elements a document carries for interactive SVG, external (by `href`) or
//! embedded. They are kept through the SVG reader and writer and never run here.

use redrob_vector_doc::DocScript;
use serde_json::{Value, json};

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "doc.scripts",
            "Scripts",
            [],
            None,
            "{} → {scripts: [{index, href?, content, type}]} the document's scripts (SVG <script> elements)",
            has_doc,
            |s, _| Ok(json!({ "scripts": s.doc()?.doc.scripts.iter().enumerate().map(|(i, sc)| script_json(i, sc)).collect::<Vec<_>>() }))
        ),
        cmd!(
            "doc.addScript",
            "Add Script",
            [],
            None,
            "{href?: url (an external script), content?: code (an embedded one; default: empty embedded ECMAScript), type?: MIME type} add a <script> to the document → {index}",
            has_doc,
            add
        ),
        cmd!(
            "doc.setScript",
            "Edit Script",
            [],
            None,
            "{index: n, href?: url|null (null makes it embedded), content?: code, type?: MIME type} change a script → {index}",
            has_doc,
            set
        ),
        cmd!("doc.removeScript", "Remove Script", [], None, "{index: n} remove a script → {index}", has_doc, remove),
    ]
}

fn script_json(i: usize, sc: &DocScript) -> Value {
    json!({ "index": i, "href": sc.href, "content": sc.content, "type": sc.mime })
}

fn apply(sc: &mut DocScript, p: &Value) {
    match p.get("href") {
        Some(Value::Null) => sc.href = None,
        Some(Value::String(h)) => sc.href = (!h.trim().is_empty()).then(|| h.trim().to_string()),
        _ => {}
    }
    if let Some(c) = str_param(p, "content") {
        sc.content = c.chars().take(262_144).collect();
    }
    if let Some(t) = str_param(p, "type") {
        sc.mime = t.trim().to_string();
    }
}

fn index_param(s: &Session, p: &Value, cmd: &str) -> Result<usize> {
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "missing `index`"))? as usize;
    if index >= s.doc()?.doc.scripts.len() {
        return Err(bad(cmd, format!("no script {index}")));
    }
    Ok(index)
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "doc.addScript";
    if s.doc()?.doc.scripts.len() >= 64 {
        return Err(bad(C, "a document holds at most 64 scripts"));
    }
    let mut sc = DocScript::default();
    apply(&mut sc, p);
    let index = s.edit("Add Script", |d, _| {
        d.scripts.push(sc);
        Ok(d.scripts.len() - 1)
    })?;
    Ok(json!({ "index": index }))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "doc.setScript";
    let index = index_param(s, p, C)?;
    let mut sc = s.doc()?.doc.scripts[index].clone();
    apply(&mut sc, p);
    s.edit("Edit Script", |d, _| {
        if let Some(slot) = d.scripts.get_mut(index) {
            *slot = sc;
        }
        Ok(())
    })?;
    Ok(json!({ "index": index }))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "doc.removeScript";
    let index = index_param(s, p, C)?;
    s.edit("Remove Script", |d, _| {
        d.scripts.remove(index);
        Ok(())
    })?;
    Ok(json!({ "index": index }))
}
