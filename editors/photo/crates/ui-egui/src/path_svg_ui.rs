//! Paths panel › SVG exchange: Import Path from SVG…, Export Path to SVG…, Copy Path as SVG
//! (clipboard text) and Paste Path from SVG (clipboard text), on top of `path.importSvg`,
//! `path.exportSvg` and `path.toSvg` ([`redrob_photo_engine::path_svg_cmds`]).

use serde_json::{Value, json};

use crate::PhotoApp;

/// Does `text` look like SVG source (what Copy Path as SVG writes, or an SVG file's text)?
pub fn looks_like_svg(text: &str) -> bool {
    let t = text.trim_start();
    if t.len() > 64 << 20 {
        return false;
    }
    let head: String = t.chars().take(512).collect();
    (head.starts_with("<?xml") || head.starts_with("<svg") || head.starts_with("<!DOCTYPE svg")) && head.contains("svg")
}

/// Copy Path as SVG: the chosen paths (`all`, `work` or a name) as SVG text on the clipboard.
pub fn copy_as_svg(app: &mut PhotoApp, ctx: &egui::Context, which: &str) {
    match app.run("path.toSvg", json!({"paths": which})) {
        Ok(r) => {
            let svg = r.get("svg").and_then(Value::as_str).unwrap_or_default().to_string();
            let n = r.get("paths").and_then(Value::as_u64).unwrap_or(0);
            ctx.copy_text(svg);
            app.ui.status_error = false;
            app.ui.status = if n == 1 { tl!("Copied 1 path as SVG").to_string() } else { tl!("Copied {n} paths as SVG").replace("{n}", &n.to_string()) };
        }
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Export Path to SVG…: a save dialog, then `path.exportSvg`.
pub fn export_dialog(app: &mut PhotoApp, which: &str) {
    let stem = app
        .session
        .active()
        .and_then(|d| d.path.as_deref())
        .and_then(|p| std::path::Path::new(p).file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "Untitled".to_string());
    let suggested = if which == "all" || which == "work" { format!("{stem}-paths.svg") } else { format!("{stem}-{}.svg", safe_name(which)) };
    let which = which.to_string();
    let result = app.pick_save(&suggested, move |app, path| {
        let r = app.run("path.exportSvg", json!({"path": path, "paths": which}))?;
        app.ui.status_error = false;
        app.ui.status = tl!("Exported paths to {path}").replace("{path}", r.get("path").and_then(Value::as_str).unwrap_or_default());
        Ok(r)
    });
    report(app, result);
}

/// Import Path from SVG…: an open dialog, then every path of the file joins the Paths panel.
pub fn import_dialog(app: &mut PhotoApp) {
    let result = app.pick_file_bytes_filtered(&["svg", "svgz"], |app, name, bytes| import_bytes(app, &name, &bytes));
    report(app, result);
}

/// `bytes` (an SVG file named `name`) as saved paths.
pub fn import_bytes(app: &mut PhotoApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    let file = std::path::Path::new(name).file_stem().map(|f| f.to_string_lossy().to_string()).filter(|s| !s.is_empty());
    let mut p = json!({"data": redrob_photo_paint::tile::b64_encode(bytes)});
    if let Some(stem) = file {
        p["name"] = json!(stem);
    }
    let r = app.run("path.importSvg", p)?;
    done(app, &r);
    Ok(r)
}

/// Paste Path from SVG: `text` (SVG source from the clipboard) as saved paths.
pub fn paste_text(app: &mut PhotoApp, text: &str) -> Result<Value, String> {
    let r = app.run("path.importSvg", json!({"svg": text}))?;
    done(app, &r);
    Ok(r)
}

fn done(app: &mut PhotoApp, r: &Value) {
    let n = r.get("count").and_then(Value::as_u64).unwrap_or(0);
    app.ui.status_error = false;
    app.ui.status = if n == 1 { tl!("Imported 1 path").to_string() } else { tl!("Imported {n} paths").replace("{n}", &n.to_string()) };
    // Select the last imported path in the panel.
    if let Some(last) = r.get("imported").and_then(Value::as_array).and_then(|a| a.last()).and_then(Value::as_str) {
        app.ui.selected_path = Some(last.to_string());
    }
}

/// Ask the OS for its text (an `Event::Paste` arrives next frame) and remember that the Paths
/// panel wants it; [`take_pending_paste`] then imports SVG text.
pub fn request_paste(app: &mut PhotoApp, ctx: &egui::Context) {
    app.ui.svg_paste_pending = true;
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
    ctx.request_repaint();
}

/// Called by the Paths panel each frame: a pending paste (or a paste while the panel is hovered)
/// whose text is SVG becomes paths; other text is left for whoever else wants it.
pub fn take_pending_paste(app: &mut PhotoApp, ui: &egui::Ui) {
    let wanted = app.ui.svg_paste_pending || ui.ui_contains_pointer();
    if !wanted {
        return;
    }
    let text = ui.input_mut(|i| {
        let mut found = None;
        i.events.retain(|e| match e {
            egui::Event::Paste(s) if found.is_none() && looks_like_svg(s) => {
                found = Some(s.clone());
                false
            }
            _ => true,
        });
        found
    });
    if let Some(text) = text {
        app.ui.svg_paste_pending = false;
        if let Err(e) = paste_text(app, &text) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    } else if app.ui.svg_paste_pending && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))) {
        // The clipboard holds text that is not SVG.
        app.ui.svg_paste_pending = false;
        app.ui.status = tl!("The clipboard holds no SVG").to_string();
        app.ui.status_error = true;
    }
}

fn report(app: &mut PhotoApp, result: Result<Value, String>) {
    if let Err(error) = result
        && error != crate::file_dialog::CANCELLED
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

fn safe_name(s: &str) -> String {
    let out: String = s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if out.is_empty() { "path".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_text_is_recognised() {
        assert!(looks_like_svg("<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"));
        assert!(looks_like_svg("  <?xml version=\"1.0\"?>\n<svg/>"));
        assert!(!looks_like_svg("#ff0000"));
        assert!(!looks_like_svg("<html><body/></html>"));
    }
}
