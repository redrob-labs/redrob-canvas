//! Script-Fu commands: `script.eval` (the console), `script.load` (a `.scm` file or source, with
//! its `script-fu-register` entries), `script.loadFolder` (every `.scm` of a folder), `script.list`
//! (the registered scripts with their menu paths and parameters) and `script.run` (a registered
//! script with its arguments). The interpreter lives in the session ([`Session::script_fu`]) so
//! definitions persist across console lines; every engine command is reachable from Scheme as a
//! procedure (see [`crate::scheme`]).

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::scheme::{Host, Interp, Val};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// The session as the scripts' host: a command call is `Session::execute`.
struct SessionHost<'a>(&'a mut Session);

impl Host for SessionHost<'_> {
    fn call(&mut self, id: &str, params: Value) -> std::result::Result<Value, String> {
        if id.starts_with("script.") {
            return Err("scripts cannot run the script commands themselves".into());
        }
        self.0.execute(id, params).map_err(|e| e.to_string())
    }
}

/// Runs `f` with the session's interpreter taken out (so scripts can call commands) and put back.
fn with_interp<R>(s: &mut Session, f: impl FnOnce(&mut Interp, &mut SessionHost<'_>) -> R) -> R {
    let mut interp = s.script_fu.take().unwrap_or_default();
    let r = f(&mut interp, &mut SessionHost(s));
    s.script_fu = Some(interp);
    r
}

fn registration_json(r: &crate::scheme::Registration) -> Value {
    json!({
        "name": r.name,
        "menu": r.menu,
        "label": r.menu.rsplit('/').next().unwrap_or(&r.name),
        "blurb": r.blurb,
        "author": r.author,
        "params": r.params.iter().map(|p| json!({"kind": p.kind, "label": p.label, "default": p.default})).collect::<Vec<_>>(),
    })
}

/// `script.eval`: one or more forms; returns the printed output and the last value.
fn eval(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "script.eval";
    let code = p.get("code").and_then(Value::as_str).ok_or_else(|| bad(CMD, "pass `code` (Scheme source)"))?.to_string();
    if code.len() > 1 << 20 {
        return Err(bad(CMD, "the source is too long"));
    }
    let (result, output) = with_interp(s, |it, host| {
        let r = it.eval_str(&code, host);
        (r, it.take_output())
    });
    match result {
        Ok(v) => Ok(json!({ "result": v.write(), "value": crate::scheme::to_json(&v), "output": output })),
        Err(e) => Err(EngineError::Other(format!("{e}{}", if output.is_empty() { String::new() } else { format!("\n{output}") }))),
    }
}

fn source(p: &Value, cmd: &str) -> Result<(String, String)> {
    if let Some(code) = p.get("code").and_then(Value::as_str) {
        return Ok((code.to_string(), p.get("name").and_then(Value::as_str).unwrap_or("script").to_string()));
    }
    let (bytes, stem) = crate::preset_import_cmds::file_bytes(p, cmd, "a .scm script")?;
    let text = String::from_utf8(bytes).map_err(|_| bad(cmd, "the script is not UTF-8"))?;
    Ok((text, stem))
}

/// `script.load`: evaluates a script file (or `code`) so its definitions and registrations stick.
fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "script.load";
    let (code, name) = source(p, CMD)?;
    let before = s.script_fu.as_ref().map_or(0, |i| i.registered.len());
    let (result, output) = with_interp(s, |it, host| {
        let r = it.eval_str(&code, host);
        (r, it.take_output())
    });
    result.map_err(|e| EngineError::Other(format!("{name}: {e}")))?;
    let after = s.script_fu.as_ref().map_or(0, |i| i.registered.len());
    let registered: Vec<Value> = s.script_fu.as_ref().map(|i| i.registered.iter().skip(before.min(after)).map(registration_json).collect()).unwrap_or_default();
    Ok(json!({ "name": name, "registered": registered, "output": output }))
}

/// `script.loadFolder`: every `.scm` in a folder (native only).
#[cfg(not(target_arch = "wasm32"))]
fn load_folder(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "script.loadFolder";
    let folder = p.get("path").and_then(Value::as_str).filter(|f| !f.trim().is_empty()).ok_or_else(|| bad(CMD, "pass `path` (a folder of .scm files)"))?;
    let rd = std::fs::read_dir(folder).map_err(|e| EngineError::Other(format!("{folder}: {e}")))?;
    let mut files: Vec<std::path::PathBuf> =
        rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("scm"))).collect();
    files.sort();
    let mut loaded = Vec::new();
    let mut errors = Vec::new();
    for f in files.iter().take(512) {
        let path = f.to_string_lossy().into_owned();
        match load(s, &json!({"path": path})) {
            Ok(_) => loaded.push(path),
            Err(e) => errors.push(format!("{e}")),
        }
    }
    Ok(json!({ "loaded": loaded, "errors": errors, "scripts": s.script_fu.as_ref().map_or(0, |i| i.registered.len()) }))
}

#[cfg(target_arch = "wasm32")]
fn load_folder(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(EngineError::Other("script folders are not available on the web".into()))
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let items: Vec<Value> = s.script_fu.as_ref().map(|i| i.registered.iter().map(registration_json).collect()).unwrap_or_default();
    Ok(json!({ "scripts": items }))
}

/// `script.run`: a registered script with `args` (positional) or `params` (by label); missing
/// arguments take the registered defaults.
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "script.run";
    let name = p.get("name").and_then(Value::as_str).ok_or_else(|| bad(CMD, "pass `name`"))?.to_string();
    let reg = s
        .script_fu
        .as_ref()
        .and_then(|i| i.registered.iter().find(|r| r.name == name).cloned())
        .ok_or_else(|| bad(CMD, format!("no registered script `{name}`")))?;
    let mut args: Vec<Val> = Vec::with_capacity(reg.params.len());
    let positional = p.get("args").and_then(Value::as_array);
    // Labels may come in `params` or beside `name` (a dialog's fields).
    let by_label = p.get("params").and_then(Value::as_object).or_else(|| p.as_object());
    for (i, param) in reg.params.iter().enumerate() {
        let v =
            positional.and_then(|a| a.get(i)).or_else(|| by_label.and_then(|m| m.get(&param.label))).cloned().unwrap_or_else(|| match param.kind.as_str() {
                // SF-ADJUSTMENT's default is `(value lower upper step page digits type)`.
                "SF-ADJUSTMENT" => param.default.as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null),
                _ => param.default.clone(),
            });
        args.push(crate::scheme::from_json(&v));
    }
    let (result, output) = with_interp(s, |it, host| {
        let r = it.call_named(&name, args, host);
        (r, it.take_output())
    });
    match result {
        Ok(v) => Ok(json!({ "name": name, "result": v.write(), "output": output })),
        Err(e) => Err(EngineError::Other(format!("{name}: {e}{}", if output.is_empty() { String::new() } else { format!("\n{output}") }))),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "script.eval",
            label: "Script-Fu Console…",
            menu: &["Filter", "Script-Fu"],
            shortcut: None,
            params: r##"{"code":"Scheme source"} → {result, value, output}"##,
            enabled: always,
            run: eval,
            journal: false,
        },
        CommandSpec {
            id: "script.load",
            label: "Load Script…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".scm file"?,"data":base64?,"code":source?,"name":string?} → {name, registered, output}"##,
            enabled: always,
            run: load,
            journal: false,
        },
        CommandSpec {
            id: "script.loadFolder",
            label: "Load Scripts Folder",
            menu: &[],
            shortcut: None,
            params: r##"{"path":"folder of .scm files"} → {loaded, errors, scripts}"##,
            enabled: always,
            run: load_folder,
            journal: false,
        },
        CommandSpec {
            id: "script.list",
            label: "List Scripts",
            menu: &[],
            shortcut: None,
            params: r##"{} → {scripts: [{name, menu, label, blurb, author, params}]}"##,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "script.run",
            label: "Run Script",
            menu: &[],
            shortcut: None,
            params: r##"{"name":"registered script","args":[...]? ,"params":{label: value}?} → {name, result, output}"##,
            enabled: always,
            run,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_evaluates_and_scripts_run_commands() {
        let mut s = Session::new();
        let r = s.execute("script.eval", json!({"code": "(define x 20) (display (* x 2))"})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r["output"], "40");
        // Definitions persist between console lines.
        let r = s.execute("script.eval", json!({"code": "(+ x 1)"})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r["result"], "21");
        let code = "(define (script-fu-new-doc w h) (file.new \"width\" w \"height\" h))\n(script-fu-register \"script-fu-new-doc\" \"<Image>/File/Create/Scripted Document\" \"a document\" \"me\" \"\" \"\" \"\" SF-ADJUSTMENT \"Width\" '(64 1 4096 1 10 0 0) SF-ADJUSTMENT \"Height\" '(32 1 4096 1 10 0 0))";
        let r = s.execute("script.load", json!({"code": code})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r["registered"].as_array().map(Vec::len), Some(1));
        s.execute("script.run", json!({"name": "script-fu-new-doc"})).unwrap_or_else(|e| panic!("{e}"));
        let d = s.active().unwrap_or_else(|| panic!("document"));
        assert_eq!((d.doc.size.width, d.doc.size.height), (64, 32));
        s.execute("script.run", json!({"name": "script-fu-new-doc", "params": {"Width": 10, "Height": 5}})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.active().map(|d| d.doc.size.width), Some(10));
        assert!(s.execute("script.eval", json!({"code": "(script.eval \"code\" \"1\")"})).is_err(), "no re-entry");
    }
}
