//! Scripting UI: the Script-Fu and Python consoles (a code box, a Run button and the output
//! log), the registered scripts and Python plug-ins as menu items under the menu paths they
//! registered (with a parameter dialog when they take parameters), and Load Scripts Folder… /
//! Load Plug-ins Folder… entries.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::menus::MenuItem;
use crate::state::DialogKind;

const CONSOLE: &str = "__console";
const SCRIPT_PREFIX: &str = "script.item.";
const PYTHON_PREFIX: &str = "python.item.";

pub fn handles(id: &str) -> bool {
    matches!(id, "script.chooseFolder" | "python.chooseFolder") || id.starts_with(SCRIPT_PREFIX) || id.starts_with(PYTHON_PREFIX)
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(CONSOLE)
}

pub fn dialog_width(fields: &Map<String, Value>) -> Option<f32> {
    owns(fields).then_some(620.0)
}

/// A registration's menu as our menu path and label. A Script-Fu `menu` (`<Image>/Filters/Blur/
/// Twice`) carries the label as its last segment; a Python plug-in's `menu` is the parent path
/// and `label` is given. `Filters` becomes `Filter`, `Colors` becomes `Image`.
fn menu_path(menu: &str, label: Option<&str>, fallback_label: &str, default_sub: &str) -> (Vec<String>, String) {
    let parts: Vec<&str> = menu.split('/').filter(|p| !p.is_empty() && !p.starts_with('<')).collect();
    let (path, label) = match label {
        Some(l) => (parts.clone(), l.to_string()),
        None => match parts.split_last() {
            Some((label, path)) if !path.is_empty() => (path.to_vec(), (*label).to_string()),
            _ => (Vec::new(), fallback_label.to_string()),
        },
    };
    let path = if path.is_empty() { vec!["Filters", default_sub] } else { path };
    let path: Vec<String> = path
        .iter()
        .map(|p| match *p {
            "Filters" => "Filter".to_string(),
            "Colors" => "Image".to_string(),
            other => other.to_string(),
        })
        .collect();
    (path, label)
}

/// Inserts the registered scripts and plug-ins into the menus.
pub fn insert_menu_items(app: &PhotocraftApp, items: &mut Vec<MenuItem>) {
    let mut extra: Vec<MenuItem> = Vec::new();
    if let Some(it) = app.session.script_fu.as_ref() {
        for r in &it.registered {
            let (path, label) = menu_path(&r.menu, None, &r.name, "Script-Fu");
            let dots = if r.params.iter().any(|p| !matches!(p.kind.as_str(), "SF-IMAGE" | "SF-DRAWABLE" | "SF-DISPLAY")) { "…" } else { "" };
            extra.push(MenuItem {
                id: format!("{SCRIPT_PREFIX}{}", r.name),
                label: format!("{label}{dots}"),
                path,
                shortcut: None,
                enabled: true,
                checked: None,
                color: None,
            });
        }
    }
    for pl in &app.session.python_plugins {
        let (path, label) = menu_path(&pl.menu, Some(&pl.label), &pl.name, "Python-Fu");
        let dots = if pl.params.is_empty() { "" } else { "…" };
        extra.push(MenuItem {
            id: format!("{PYTHON_PREFIX}{}", pl.name),
            label: format!("{label}{dots}"),
            path,
            shortcut: None,
            enabled: true,
            checked: None,
            color: None,
        });
    }
    for e in extra {
        // After the last item of the same path, else after the Script-Fu/Python-Fu console entries.
        let at = items
            .iter()
            .rposition(|i| i.path == e.path)
            .or_else(|| items.iter().rposition(|i| i.path.first() == e.path.first()))
            .map_or(items.len(), |p| p + 1);
        items.insert(at, e);
    }
}

/// The dialog spec (filter-dialog notation) for a Script-Fu registration's parameters.
fn script_spec(r: &redrob_photo_engine::scheme::Registration) -> String {
    let parts: Vec<String> = r
        .params
        .iter()
        .filter(|p| !matches!(p.kind.as_str(), "SF-IMAGE" | "SF-DRAWABLE" | "SF-DISPLAY" | "SF-LAYER" | "SF-CHANNEL" | "SF-VECTORS"))
        .map(|p| {
            let k = &p.label;
            match p.kind.as_str() {
                "SF-ADJUSTMENT" => {
                    let a = p.default.as_array().cloned().unwrap_or_default();
                    let n = |i: usize, d: f64| a.get(i).and_then(Value::as_f64).unwrap_or(d);
                    format!("\"{k}\":{}..{}={}", n(1, 0.0), n(2, 100.0), n(0, 0.0))
                }
                "SF-TOGGLE" => format!("\"{k}\":bool={}", p.default.as_bool().or_else(|| p.default.as_f64().map(|v| v != 0.0)).unwrap_or(false)),
                "SF-COLOR" => format!("\"{k}\":color={}", p.default.as_str().unwrap_or("#000000")),
                "SF-OPTION" | "SF-ENUM" => {
                    let opts: Vec<String> = p.default.as_array().map(|a| a.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()).unwrap_or_default();
                    format!("\"{k}\":\"{}\"", opts.join("|"))
                }
                "SF-VALUE" => format!("\"{k}\":int={}", p.default.as_f64().unwrap_or(0.0) as i64),
                _ => format!("\"{k}\":text"),
            }
        })
        .collect();
    format!("{{{}}}", parts.join(","))
}

/// The dialog spec for a Python plug-in's parameters.
fn python_spec(params: &[Value]) -> String {
    let parts: Vec<String> = params
        .iter()
        .filter_map(|p| {
            let k = p.get("name").and_then(Value::as_str)?;
            let d = p.get("default").cloned().unwrap_or(Value::Null);
            Some(match p.get("type").and_then(Value::as_str).unwrap_or("number") {
                "bool" => format!("\"{k}\":bool={}", d.as_bool().unwrap_or(false)),
                "int" => format!("\"{k}\":int={}", d.as_i64().unwrap_or(0)),
                "text" | "string" => format!("\"{k}\":text"),
                "color" => format!("\"{k}\":color={}", d.as_str().unwrap_or("#000000")),
                "choice" => {
                    let opts: Vec<String> = p
                        .get("options")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect())
                        .unwrap_or_default();
                    format!("\"{k}\":\"{}\"", opts.join("|"))
                }
                _ => {
                    let n = |key: &str, dflt: f64| p.get(key).and_then(Value::as_f64).unwrap_or(dflt);
                    format!("\"{k}\":{}..{}={}", n("min", 0.0), n("max", 100.0), d.as_f64().unwrap_or(0.0))
                }
            })
        })
        .collect();
    format!("{{{}}}", parts.join(","))
}

pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let given = params.as_object().is_some_and(|o| !o.is_empty());
    match id {
        "script.eval" | "python.eval" if !given => return Some(open_console(app, id)),
        "script.chooseFolder" => {
            return Some(app.ask_folder(|app, path| {
                let r = app.run("script.loadFolder", json!({"path": path}))?;
                app.ui.status_error = false;
                app.ui.status = crate::i18n::fmt(tl!("Loaded {n} scripts"), &[("n", &r["scripts"].as_u64().unwrap_or(0).to_string())]);
                Ok(r)
            }));
        }
        "python.chooseFolder" => {
            return Some(app.ask_folder(|app, path| {
                let r = app.run("python.loadFolder", json!({"path": path}))?;
                app.ui.status_error = false;
                app.ui.status = crate::i18n::fmt(tl!("Loaded {n} Python plug-ins"), &[("n", &r["plugins"].as_u64().unwrap_or(0).to_string())]);
                Ok(r)
            }));
        }
        _ => {}
    }
    if let Some(name) = id.strip_prefix(SCRIPT_PREFIX) {
        let reg = app.session.script_fu.as_ref().and_then(|it| it.registered.iter().find(|r| r.name == name).cloned());
        let Some(reg) = reg else { return Some(Err(format!("no script registered as {name}"))) };
        let spec = script_spec(&reg);
        if given || spec == "{}" {
            let mut p = params.as_object().cloned().unwrap_or_default();
            p.insert("name".into(), json!(name));
            return Some(app.run("script.run", Value::Object(p)));
        }
        let mut fixed = Map::new();
        fixed.insert("name".into(), json!(name));
        let label = reg.menu.rsplit('/').next().unwrap_or(&reg.name).to_string();
        return Some(Ok(json!({ "dialog": crate::filter_dialog::open_with_spec(app, "script.run", &label, &spec, fixed) })));
    }
    if let Some(name) = id.strip_prefix(PYTHON_PREFIX) {
        let Some(pl) = app.session.python_plugins.iter().find(|p| p.name == name).cloned() else {
            return Some(Err(format!("no Python plug-in registered as {name}")));
        };
        let spec = python_spec(&pl.params);
        if given || spec == "{}" {
            let mut p = params.as_object().cloned().unwrap_or_default();
            p.insert("name".into(), json!(name));
            return Some(app.run("python.run", Value::Object(p)));
        }
        let mut fixed = Map::new();
        fixed.insert("name".into(), json!(name));
        return Some(Ok(json!({ "dialog": crate::filter_dialog::open_with_spec(app, "python.run", &pl.label, &spec, fixed) })));
    }
    None
}

fn open_console(app: &mut PhotocraftApp, command: &str) -> Result<Value, String> {
    let python = command == "python.eval";
    let mut f = Map::new();
    f.insert(CONSOLE.into(), json!(true));
    f.insert("__command".into(), json!(command));
    f.insert("__label".into(), json!(if python { "Python Console…" } else { "Script-Fu Console…" }));
    f.insert("code".into(), json!(if python { "print(len(call('command.list')), 'commands')" } else { "(display (length (command.list)))" }));
    f.insert("log".into(), json!(""));
    Ok(json!({ "dialog": app.ui.open_dialog(DialogKind::Command, f) }))
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let python = f.get("__command").and_then(Value::as_str) == Some("python.eval");
    let mut code = f.get("code").and_then(Value::as_str).unwrap_or("").to_string();
    let log = f.get("log").and_then(Value::as_str).unwrap_or("").to_string();
    ui.weak(if python {
        tl!("Python 3 with call(command, **params), message(text) and print(); every engine command is reachable through call.")
    } else {
        tl!(
            "Scheme: every engine command is a procedure, (file.new \"width\" 640 \"height\" 480); define procedures and register them with script-fu-register."
        )
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical().id_salt("console-log").max_height(220.0).stick_to_bottom(true).show(ui, |ui| {
        ui.add(egui::TextEdit::multiline(&mut log.clone()).desired_width(f32::INFINITY).desired_rows(10).font(egui::TextStyle::Monospace).interactive(false));
    });
    ui.add_space(4.0);
    let edit =
        ui.add(egui::TextEdit::multiline(&mut code).desired_width(f32::INFINITY).desired_rows(4).font(egui::TextStyle::Monospace).hint_text(tl!("Code")));
    if edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command) {
        f.insert("__confirm".into(), json!(true));
    }
    ui.horizontal(|ui| {
        if ui.button(tl!("Run")).on_hover_text(tl!("Cmd+Enter runs the code")).clicked() {
            f.insert("__confirm".into(), json!(true));
        }
        if ui.button(tl!("Clear Log")).clicked() {
            f.insert("log".into(), json!(""));
        }
    });
    f.insert("code".into(), json!(code));
}

/// Run: evaluates the code and reopens the console with the output appended to the log.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let command = f.get("__command").and_then(Value::as_str).unwrap_or("script.eval").to_string();
    let code = f.get("code").and_then(Value::as_str).unwrap_or("").to_string();
    let mut log = f.get("log").and_then(Value::as_str).unwrap_or("").to_string();
    let prompt = if command == "python.eval" { ">>> " } else { "> " };
    for line in code.lines() {
        log.push_str(prompt);
        log.push_str(line);
        log.push('\n');
    }
    let result = app.run(&command, json!({"code": code}));
    match &result {
        Ok(r) => {
            if let Some(out) = r.get("output").and_then(Value::as_str).filter(|o| !o.is_empty()) {
                log.push_str(out);
                if !out.ends_with('\n') {
                    log.push('\n');
                }
            }
            if let Some(v) = r.get("result").and_then(Value::as_str).filter(|v| !v.is_empty()) {
                log.push_str(v);
                log.push('\n');
            }
        }
        Err(e) => {
            log.push_str(&format!("{}: {e}\n", tl!("Error")));
        }
    }
    if log.len() > 200_000 {
        log = log[log.len() - 200_000..].to_string();
    }
    let mut again = f.clone();
    again.insert("log".into(), json!(log));
    again.remove("__confirm");
    app.ui.open_dialog(DialogKind::Command, again);
    result.map(|_| json!({ "ran": true }))
}

/// A Run click (or Cmd+Enter) confirms the console on the next frame.
pub fn tick(app: &mut PhotocraftApp) {
    let Some(d) = app.ui.dialogs.iter().find(|d| owns(&d.fields) && d.fields.contains_key("__confirm")) else { return };
    let id = d.id;
    if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) {
        d.fields.remove("__confirm");
    }
    let _ = crate::dialogs::confirm(app, id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_menus_map_to_ours() {
        assert_eq!(menu_path("<Image>/Filters/Blur/Twice", None, "x", "Script-Fu"), (vec!["Filter".to_string(), "Blur".to_string()], "Twice".to_string()));
        assert_eq!(
            menu_path("Filters/Python-Fu", Some("New Doc"), "x", "Python-Fu"),
            (vec!["Filter".to_string(), "Python-Fu".to_string()], "New Doc".to_string())
        );
        assert_eq!(menu_path("", None, "fallback", "Script-Fu"), (vec!["Filter".to_string(), "Script-Fu".to_string()], "fallback".to_string()));
    }
}
