//! Python plug-ins (`python.*`): Python 3 scripts that register procedures with menu entries and
//! parameter dialogs through the `redrob_canvas` helper module, and run in a separate `python3`
//! process. The process talks JSON lines over its pipes: it sends `{"call": id, "params": {}}`
//! to run an engine command and reads the result, `{"print": text}` to show text, and
//! `{"done": value}` / `{"error": text}` to finish. `python.loadFolder` asks every `.py` of a
//! folder to describe itself (`--describe`), `python.run` runs one registration, `python.eval`
//! runs console code with the helper in scope, and `python.helper` writes the helper module.

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// A procedure a plug-in registered.
#[derive(Clone, Debug, PartialEq)]
pub struct PyPlugin {
    pub path: String,
    pub name: String,
    pub menu: String,
    pub label: String,
    pub blurb: String,
    /// `[{name, label, type (number|int|bool|text|choice), default, min?, max?, options?}]`.
    pub params: Vec<Value>,
}

/// The helper module plug-ins import.
pub const HELPER_MODULE: &str = r##"
"""redrob_canvas: the Python plug-in bridge of Redrob Canvas.

Import it, decorate procedures with @register, and call main() at the end of the file:

    from redrob_canvas import register, call, message, main

    @register("my-blur", menu="Filters/Python-Fu", label="Blur Twice",
              params=[{"name": "radius", "label": "Radius", "type": "number", "default": 5, "min": 0, "max": 100}])
    def blur_twice(radius=5):
        call("filter.blur.gaussian", radius=radius)
        call("filter.blur.gaussian", radius=radius)
        message("blurred twice")

    main()

call(command, **params) runs any engine command and returns its result (a dict or value);
message(text) shows text in the host; print() is forwarded too.
"""
import json
import sys
import traceback

_REG = []
_OUT = sys.stdout


def _send(obj):
    _OUT.write(json.dumps(obj) + "\n")
    _OUT.flush()


def call(command, **params):
    """Run an engine command (e.g. call("file.new", width=640, height=480))."""
    _send({"call": command, "params": params})
    line = sys.stdin.readline()
    if not line:
        raise RuntimeError("the host closed the connection")
    reply = json.loads(line)
    if "error" in reply:
        raise RuntimeError(reply["error"])
    return reply.get("ok")


def message(text):
    _send({"print": str(text)})


class _Forward:
    def write(self, s):
        if s:
            _send({"print": s})

    def flush(self):
        pass


def register(name, menu="Filters/Python-Fu", label=None, params=(), blurb="", author=""):
    """Register fn as a procedure with a menu entry and a parameter dialog."""

    def deco(fn):
        _REG.append({"name": name, "menu": menu, "label": label or name, "params": list(params), "blurb": blurb, "author": author, "fn": fn})
        return fn

    return deco


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv[:1] == ["--describe"]:
        print(json.dumps([{k: v for k, v in r.items() if k != "fn"} for r in _REG]))
        return
    if argv[:1] == ["--run"]:
        req = json.loads(sys.stdin.readline() or "{}")
        name = req.get("name")
        params = req.get("params") or {}
        entry = next((r for r in _REG if r["name"] == name), None)
        if entry is None:
            _send({"error": "no procedure named %r" % name})
            return
        sys.stdout = _Forward()
        try:
            result = entry["fn"](**params)
            _send({"done": result if isinstance(result, (dict, list, str, int, float, bool)) or result is None else str(result)})
        except Exception:
            _send({"error": traceback.format_exc()})
        return
    print(json.dumps({"registered": [r["name"] for r in _REG]}))
"##;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// The Python 3 interpreter to run (`REDROB_CANVAS_PYTHON` overrides it).
pub fn python_program() -> String {
    std::env::var("REDROB_CANVAS_PYTHON")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| if cfg!(windows) { "python".into() } else { "python3".into() })
}

/// Writes the helper module into `dir` (plug-ins import it from there); returns its path.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_helper(dir: &std::path::Path) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir).map_err(|e| other(format!("{}: {e}", dir.display())))?;
    let path = dir.join("redrob_canvas.py");
    let current = std::fs::read_to_string(&path).ok();
    if current.as_deref() != Some(HELPER_MODULE) {
        std::fs::write(&path, HELPER_MODULE).map_err(|e| other(format!("{}: {e}", path.display())))?;
    }
    Ok(path)
}

/// Where the helper lives for runs without a plug-in folder: the temporary directory.
#[cfg(not(target_arch = "wasm32"))]
fn helper_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("redrob-canvas-python")
}

#[cfg(not(target_arch = "wasm32"))]
fn param_value(p: &Value, given: Option<&Value>) -> Value {
    if let Some(v) = given {
        return v.clone();
    }
    p.get("default").cloned().unwrap_or(Value::Null)
}

/// Runs `program` with `args` and `stdin_line`, serving `call` requests from the child on the
/// session until it finishes or `timeout` passes. Returns (done value, printed text).
#[cfg(not(target_arch = "wasm32"))]
fn bridge(
    s: &mut Session,
    program: &str,
    args: &[String],
    cwd: Option<&std::path::Path>,
    helper: &std::path::Path,
    stdin_line: &str,
    timeout: std::time::Duration,
) -> Result<(Value, String)> {
    use std::io::{BufRead, Write};
    let mut cmd = std::process::Command::new(program);
    cmd.args(args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut pythonpath = helper.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    if let Ok(existing) = std::env::var("PYTHONPATH") {
        let sep = if cfg!(windows) { ";" } else { ":" };
        pythonpath = format!("{pythonpath}{sep}{existing}");
    }
    cmd.env("PYTHONPATH", pythonpath).env("REDROB_CANVAS_BRIDGE", "1").env("PYTHONUNBUFFERED", "1");
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let mut child = cmd.spawn().map_err(|e| other(format!("{program}: {e} (install Python 3 or set REDROB_CANVAS_PYTHON)")))?;
    let mut stdin = child.stdin.take().ok_or_else(|| other("no stdin pipe"))?;
    let stdout = child.stdout.take().ok_or_else(|| other("no stdout pipe"))?;
    let stderr = child.stderr.take();
    // Lines arrive on a channel so the loop can time out and kill a hung plug-in.
    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<String>>();
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines() {
            let stop = line.is_err();
            if tx.send(line).is_err() || stop {
                break;
            }
        }
    });
    let err_handle = stderr.map(|e| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = std::io::Read::read_to_string(&mut std::io::BufReader::new(e), &mut buf);
            buf
        })
    });
    if !stdin_line.is_empty() {
        let _ = writeln!(stdin, "{stdin_line}");
        let _ = stdin.flush();
    }
    let deadline = std::time::Instant::now() + timeout;
    let mut printed = String::new();
    let mut done: Option<Value> = None;
    let mut error: Option<String> = None;
    let mut calls = 0usize;
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            let _ = child.kill();
            error = Some(format!("the plug-in did not finish within {} s", timeout.as_secs()));
            break;
        }
        match rx.recv_timeout(deadline - now) {
            Ok(Ok(line)) => {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    if printed.len() < 1 << 20 {
                        printed.push_str(&line);
                        printed.push('\n');
                    }
                    continue;
                };
                if let Some(text) = msg.get("print") {
                    if printed.len() < 1 << 20 {
                        printed.push_str(text.as_str().unwrap_or(""));
                    }
                } else if let Some(id) = msg.get("call").and_then(Value::as_str) {
                    calls += 1;
                    let reply = if calls > 10_000 {
                        json!({"error": "too many command calls"})
                    } else if id.starts_with("python.") || id.starts_with("script.") {
                        json!({"error": "plug-ins cannot run the scripting commands"})
                    } else {
                        match s.execute(id, msg.get("params").cloned().unwrap_or_else(|| json!({}))) {
                            Ok(r) => json!({"ok": r}),
                            Err(e) => json!({"error": e.to_string()}),
                        }
                    };
                    if writeln!(stdin, "{reply}").and_then(|()| stdin.flush()).is_err() {
                        error = Some("the plug-in closed its input".into());
                        break;
                    }
                } else if let Some(d) = msg.get("done") {
                    done = Some(d.clone());
                    break;
                } else if let Some(e) = msg.get("error") {
                    error = Some(e.as_str().map_or_else(|| e.to_string(), str::to_string));
                    break;
                } else {
                    // The describe answer or anything else: keep it as the result.
                    done = Some(msg);
                    break;
                }
            }
            Ok(Err(e)) => {
                error = Some(format!("reading the plug-in: {e}"));
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                error = Some(format!("the plug-in did not finish within {} s", timeout.as_secs()));
                break;
            }
        }
    }
    drop(stdin);
    let status = child.wait().ok();
    let stderr_text = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
    if let Some(e) = error {
        let tail: String = stderr_text.lines().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(other(if tail.is_empty() { e } else { format!("{e}\n{tail}") }));
    }
    if done.is_none() && status.is_some_and(|st| !st.success()) {
        let tail: String = stderr_text.lines().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(other(format!("the plug-in exited with {}\n{tail}", status.map_or_else(|| "?".to_string(), |st| st.to_string()))));
    }
    Ok((done.unwrap_or(Value::Null), printed))
}

/// `python.loadFolder`: every `.py` in the folder describes its registrations.
#[cfg(not(target_arch = "wasm32"))]
fn load_folder(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "python.loadFolder";
    let folder =
        p.get("path").and_then(Value::as_str).filter(|f| !f.trim().is_empty()).ok_or_else(|| bad(CMD, "pass `path` (a folder of .py plug-ins)"))?.to_string();
    let dir = std::path::Path::new(&folder);
    let helper = write_helper(dir)?;
    let rd = std::fs::read_dir(dir).map_err(|e| other(format!("{folder}: {e}")))?;
    let mut files: Vec<std::path::PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("py")) && f.file_name().is_none_or(|n| n != "redrob_canvas.py"))
        .collect();
    files.sort();
    let program = python_program();
    let mut loaded = Vec::new();
    let mut errors = Vec::new();
    s.python_plugins.retain(|pl| !pl.path.starts_with(&folder));
    for f in files.iter().take(256) {
        let path = f.to_string_lossy().into_owned();
        match bridge(s, &program, &[path.clone(), "--describe".into()], Some(dir), &helper, "", std::time::Duration::from_secs(20)) {
            Ok((desc, _)) => {
                let regs = desc.as_array().cloned().unwrap_or_default();
                for r in regs {
                    let name = r.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let menu = r.get("menu").and_then(Value::as_str).unwrap_or("Filters/Python-Fu").to_string();
                    let label = r.get("label").and_then(Value::as_str).unwrap_or(&name).to_string();
                    let plugin = PyPlugin {
                        path: path.clone(),
                        name: name.clone(),
                        menu,
                        label,
                        blurb: r.get("blurb").and_then(Value::as_str).unwrap_or("").to_string(),
                        params: r.get("params").and_then(Value::as_array).cloned().unwrap_or_default(),
                    };
                    s.python_plugins.retain(|pl| pl.name != name);
                    s.python_plugins.push(plugin);
                    loaded.push(name);
                }
            }
            Err(e) => errors.push(format!("{path}: {e}")),
        }
    }
    Ok(json!({ "loaded": loaded, "errors": errors, "plugins": s.python_plugins.len() }))
}

#[cfg(not(target_arch = "wasm32"))]
fn plugin_json(pl: &PyPlugin) -> Value {
    json!({ "name": pl.name, "menu": pl.menu, "label": pl.label, "blurb": pl.blurb, "path": pl.path, "params": pl.params })
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let items: Vec<Value> = s.python_plugins.iter().map(plugin_json).collect();
        Ok(json!({ "plugins": items, "python": python_program() }))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = s;
        Ok(json!({ "plugins": [], "python": Value::Null }))
    }
}

/// `python.run`: a registered procedure with `params` (by parameter name; defaults fill in).
#[cfg(not(target_arch = "wasm32"))]
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "python.run";
    let name = p.get("name").and_then(Value::as_str).ok_or_else(|| bad(CMD, "pass `name`"))?.to_string();
    let plugin = s.python_plugins.iter().find(|pl| pl.name == name).cloned().ok_or_else(|| bad(CMD, format!("no Python plug-in registered `{name}`")))?;
    let given = p.get("params").and_then(Value::as_object).cloned().unwrap_or_else(|| p.as_object().cloned().unwrap_or_default());
    let mut params = serde_json::Map::new();
    for spec in &plugin.params {
        let Some(pname) = spec.get("name").and_then(Value::as_str) else { continue };
        params.insert(pname.to_string(), param_value(spec, given.get(pname)));
    }
    let dir = std::path::Path::new(&plugin.path).parent().map(std::path::Path::to_path_buf).unwrap_or_else(helper_dir);
    let helper = write_helper(&dir)?;
    let program = python_program();
    let req = json!({ "name": name, "params": params }).to_string();
    let (done, printed) = bridge(s, &program, &[plugin.path.clone(), "--run".into()], Some(&dir), &helper, &req, std::time::Duration::from_secs(600))?;
    Ok(json!({ "name": name, "result": done, "output": printed }))
}

#[cfg(target_arch = "wasm32")]
fn run(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(other("Python plug-ins need the desktop app"))
}

#[cfg(target_arch = "wasm32")]
fn load_folder(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(other("Python plug-ins need the desktop app"))
}

/// `python.eval`: console code with the helper in scope (`call`, `message`), through the bridge.
#[cfg(not(target_arch = "wasm32"))]
fn eval(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "python.eval";
    let code = p.get("code").and_then(Value::as_str).ok_or_else(|| bad(CMD, "pass `code` (Python source)"))?.to_string();
    if code.len() > 1 << 20 {
        return Err(bad(CMD, "the source is too long"));
    }
    let dir = helper_dir();
    let helper = write_helper(&dir)?;
    // The console runs the code as one procedure so prints and calls go through the bridge.
    const WRAPPER: &str = "import sys, json, traceback\nfrom redrob_canvas import call, message, _send, _Forward\n_code = json.loads(sys.stdin.readline())\nsys.stdout = _Forward()\ntry:\n    _g = {'call': call, 'message': message}\n    exec(compile(_code, '<console>', 'exec'), _g)\n    _send({'done': None})\nexcept Exception:\n    _send({'error': traceback.format_exc()})\n";
    let wrapper = WRAPPER.to_string();
    let program = python_program();
    let (_, printed) =
        bridge(s, &program, &["-c".into(), wrapper], Some(&dir), &helper, &Value::String(code).to_string(), std::time::Duration::from_secs(120))?;
    Ok(json!({ "output": printed }))
}

#[cfg(target_arch = "wasm32")]
fn eval(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(other("the Python console needs the desktop app"))
}

/// `python.helper`: writes the helper module into a folder (for writing new plug-ins).
#[cfg(not(target_arch = "wasm32"))]
fn helper(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "python.helper";
    let folder = p
        .get("path")
        .and_then(Value::as_str)
        .filter(|f| !f.trim().is_empty())
        .ok_or_else(|| bad(CMD, "pass `path` (the folder to write redrob_canvas.py into)"))?;
    let path = write_helper(std::path::Path::new(folder))?;
    Ok(json!({ "path": path.to_string_lossy() }))
}

#[cfg(target_arch = "wasm32")]
fn helper(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(other("Python plug-ins need the desktop app"))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "python.eval",
            label: "Python Console…",
            menu: &["Filter", "Python-Fu"],
            shortcut: None,
            params: r##"{"code":"Python source (call(command, **params), message(text), print)"} → {output}"##,
            enabled: always,
            run: eval,
            journal: false,
        },
        CommandSpec {
            id: "python.loadFolder",
            label: "Load Python Plug-ins Folder",
            menu: &[],
            shortcut: None,
            params: r##"{"path":"folder of .py plug-ins"} → {loaded, errors, plugins}"##,
            enabled: always,
            run: load_folder,
            journal: false,
        },
        CommandSpec {
            id: "python.list",
            label: "List Python Plug-ins",
            menu: &[],
            shortcut: None,
            params: r##"{} → {plugins: [{name, menu, label, blurb, path, params}], python}"##,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "python.run",
            label: "Run Python Plug-in",
            menu: &[],
            shortcut: None,
            params: r##"{"name":"registered procedure","params":{name: value}?} → {name, result, output}"##,
            enabled: always,
            run,
            journal: true,
        },
        CommandSpec {
            id: "python.helper",
            label: "Write Python Helper Module",
            menu: &[],
            shortcut: None,
            params: r##"{"path":"folder"} → {path}"##,
            enabled: always,
            run: helper,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_module_is_valid_text_and_describes() {
        assert!(HELPER_MODULE.contains("def register("));
        assert!(HELPER_MODULE.contains("def call(command, **params)"));
        // Only when a Python 3 is present on the build host does the bridge get exercised.
        let have_python = std::process::Command::new(python_program()).arg("--version").output().is_ok_and(|o| o.status.success());
        if !have_python {
            return;
        }
        let dir = std::env::temp_dir().join(format!("redrob-canvas-pytest-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let script = "from redrob_canvas import register, call, message, main\n\n@register('py-new', menu='Filters/Python-Fu', label='New Doc', params=[{'name': 'w', 'label': 'Width', 'type': 'int', 'default': 40}])\ndef new_doc(w=40):\n    r = call('file.new', width=w, height=20)\n    message('made %s' % r.get('width', w))\n    return r\n\nmain()\n";
        let _ = std::fs::write(dir.join("plug.py"), script);
        let mut s = Session::new();
        let r = s.execute("python.loadFolder", json!({"path": dir.to_string_lossy()})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r["loaded"].as_array().map(Vec::len), Some(1), "{r}");
        let r = s.execute("python.run", json!({"name": "py-new", "params": {"w": 33}})).unwrap_or_else(|e| panic!("{e}"));
        assert!(r["output"].as_str().unwrap_or("").contains("made"), "{r}");
        assert_eq!(s.active().map(|d| d.doc.size.width), Some(33));
        let r = s.execute("python.eval", json!({"code": "print(1 + 1)\nmessage('ok')"})).unwrap_or_else(|e| panic!("{e}"));
        assert!(r["output"].as_str().unwrap_or("").contains('2'), "{r}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
