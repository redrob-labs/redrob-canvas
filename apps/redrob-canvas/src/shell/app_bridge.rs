//! The Redrob Code app bridge: the shell's tools, served to the shared Redrob Code daemon over
//! MCP on loopback (`redrob-app-bridge`), so the agent in any Redrob surface can drive the open
//! document from a chat.
//!
//! The daemon calls a tool from one of the bridge's own threads; the editors live on the UI
//! thread. So every tool handler is a request/reply round trip: it queues a [`BridgeRequest`],
//! asks egui for a repaint and blocks on the reply, and [`ShellApp::serve_bridge`] drains the
//! queue once per frame (before anything is drawn) and runs each request against the real app
//! state with the same functions the menus use. A request nobody answers within
//! [`REPLY_TIMEOUT`] fails with a message the agent can read.
//!
//! The daemon is optional: when it cannot be reached the failure is logged and the app runs
//! without the bridge.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use redrob_app_bridge::{AppBridge, AppBridgeConfig, Tool};
use redrob_vector_engine::cmd::fileio::SaveMode;
use serde_json::{Value, json};

use super::{Mode, ShellApp};
use crate::ai::bridge::ModeBridge;
use crate::routing;

/// The name the daemon lists the tools under (`app:canvas`).
const APP_NAME: &str = "canvas";
/// How long a tool call waits for the UI thread before giving up.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// `list_layers` without `max`.
const DEFAULT_MAX_LAYERS: usize = 200;

type Reply = Result<String, String>;

/// One tool call waiting for the UI thread.
pub struct BridgeRequest {
    tool: String,
    args: Value,
    reply: Sender<Reply>,
}

/// The shell's end of the bridge: the request queue the UI thread drains and the live
/// registration, kept until exit.
pub struct Bridge {
    requests: Receiver<BridgeRequest>,
    /// Filled by the start thread once the daemon answered; `None` while connecting or when the
    /// daemon could not be reached.
    handle: Arc<Mutex<Option<AppBridge>>>,
}

impl Bridge {
    /// Queue the tools and register with the daemon on a thread of its own, so a slow or absent
    /// daemon never delays the window. The handle lands in the returned bridge.
    pub fn start(ctx: egui::Context) -> Self {
        let (tx, requests) = mpsc::channel();
        let handle: Arc<Mutex<Option<AppBridge>>> = Arc::default();
        let slot = handle.clone();
        let spawned = std::thread::Builder::new()
            .name("redrob-canvas-app-bridge".into())
            .spawn(move || {
                let config = AppBridgeConfig {
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                    ..AppBridgeConfig::new(APP_NAME, tools(&tx, &ctx))
                };
                match AppBridge::start(config) {
                    Ok(bridge) => {
                        log::info!(
                            "app bridge: bridged as {} at {}",
                            bridge.id(),
                            bridge.mcp_url()
                        );
                        if let Ok(mut slot) = slot.lock() {
                            *slot = Some(bridge);
                        }
                    }
                    Err(e) => log::warn!("app bridge: not bridged: {e}"),
                }
            });
        if let Err(e) = spawned {
            log::warn!("app bridge: could not start the bridge thread: {e}");
        }
        Self { requests, handle }
    }

    /// Unregister from the daemon and stop the MCP server.
    pub fn stop(self) {
        let bridge = self.handle.lock().ok().and_then(|mut slot| slot.take());
        if let Some(bridge) = bridge {
            bridge.stop();
        }
    }
}

/// Queue one call for the UI thread and block until it answers or the wait runs out.
fn call_ui(tx: &Sender<BridgeRequest>, ctx: &egui::Context, tool: &str, args: Value) -> Reply {
    let (reply, rx) = mpsc::channel();
    tx.send(BridgeRequest {
        tool: tool.to_string(),
        args,
        reply,
    })
    .map_err(|_| "the Redrob Canvas window is closed".to_string())?;
    ctx.request_repaint();
    match rx.recv_timeout(REPLY_TIMEOUT) {
        Ok(reply) => reply,
        Err(_) => Err(format!(
            "{tool}: the editor did not answer within {} s",
            REPLY_TIMEOUT.as_secs()
        )),
    }
}

fn tool(
    tx: &Sender<BridgeRequest>,
    ctx: &egui::Context,
    name: &'static str,
    description: &str,
    schema: Value,
) -> Tool {
    let (tx, ctx) = (tx.clone(), ctx.clone());
    Tool::new(name, description, schema, move |args| {
        call_ui(&tx, &ctx, name, args)
    })
}

/// The tools the daemon sees; each one is answered by [`ShellApp::serve_request`].
fn tools(tx: &Sender<BridgeRequest>, ctx: &egui::Context) -> Vec<Tool> {
    let layer_ref = json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "description": "Layer id from list_layers" },
            "name": { "type": "string", "description": "Layer name from list_layers (used when no id is given)" }
        }
    });
    vec![
        tool(
            tx,
            ctx,
            "document_info",
            "Describe the active document: file path (or untitled), the active mode (raster, vector or design), size, layer count and whether it has unsaved changes.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            tx,
            ctx,
            "open_file",
            "Open a file in the editor its type belongs to and switch to it; answers the mode it opened in.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the file to open" }
                },
                "required": ["path"]
            }),
        ),
        tool(
            tx,
            ctx,
            "switch_mode",
            "Switch the window to the raster (photo), vector or design (layout) editor.",
            json!({
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["raster", "vector", "design"] }
                },
                "required": ["mode"]
            }),
        ),
        tool(
            tx,
            ctx,
            "list_layers",
            "List the active document's layers, one per line: id, name, visible, and whether it is selected.",
            json!({
                "type": "object",
                "properties": {
                    "max": { "type": "integer", "minimum": 1, "description": "Maximum number of lines (default 200)" }
                }
            }),
        ),
        tool(
            tx,
            ctx,
            "select_layer",
            "Make the layer with the given id (or name) the active layer.",
            layer_ref,
        ),
        tool(
            tx,
            ctx,
            "set_layer_visible",
            "Show or hide a layer.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Layer id from list_layers" },
                    "visible": { "type": "boolean" }
                },
                "required": ["id", "visible"]
            }),
        ),
        tool(
            tx,
            ctx,
            "export",
            "Export the active document (the current page or artboard) as png, svg or pdf through the editor's own export; answers the written path. svg is vector mode only.",
            json!({
                "type": "object",
                "properties": {
                    "format": { "type": "string", "enum": ["png", "svg", "pdf"] },
                    "path": { "type": "string", "description": "Absolute output path; defaults to a file next to the document, else in the Downloads folder" },
                    "scale": { "type": "number", "minimum": 0.1, "maximum": 8, "description": "Raster scale (png only, default 1)" }
                },
                "required": ["format"]
            }),
        ),
        tool(
            tx,
            ctx,
            "save",
            "Save the active document to its own file; fails naming Save As when the document is untitled.",
            json!({ "type": "object", "properties": {} }),
        ),
    ]
}

/// The bridge's name for a mode and back: `raster` / `vector` / `design`.
fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Photo => "raster",
        Mode::Vector => "vector",
        Mode::Layout => "design",
    }
}

fn parse_mode(value: &str) -> Option<Mode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "raster" | "photo" => Some(Mode::Photo),
        "vector" => Some(Mode::Vector),
        "design" | "layout" => Some(Mode::Layout),
        _ => None,
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{key} is required"))
}

fn bool_arg(args: &Value, key: &str) -> Result<bool, String> {
    args.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{key} is required"))
}

/// `id` as a number, or `null` when neither `id` nor a numeric string was given.
fn id_arg(args: &Value) -> Option<u64> {
    match args.get("id") {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

/// `scale` (default 1), refused outside 0.1–8.
fn scale_arg(args: &Value) -> Result<f64, String> {
    let scale = args.get("scale").and_then(Value::as_f64).unwrap_or(1.0);
    if !(0.1..=8.0).contains(&scale) {
        return Err("scale must be between 0.1 and 8".into());
    }
    Ok(scale)
}

/// Where an export goes when the agent names no path: `<document stem>.<ext>` next to the
/// document, else in the Downloads folder, else in the temp directory.
fn default_export_path(doc_path: Option<&str>, name: &str, ext: &str) -> PathBuf {
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "untitled".to_string());
    let dir = doc_path
        .and_then(|p| Path::new(p).parent().map(Path::to_path_buf))
        .filter(|d| !d.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .filter(|h| !h.is_empty())
                .map(|h| PathBuf::from(h).join("Downloads"))
                .filter(|d| d.is_dir())
        })
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("{stem}.{ext}"))
}

/// A requested output path, or the default one; always with the format's extension.
fn export_target(args: &Value, doc_path: Option<&str>, name: &str, ext: &str) -> String {
    let path = match args
        .get("path")
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty())
    {
        Some(p) => PathBuf::from(p),
        None => default_export_path(doc_path, name, ext),
    };
    let path = match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case(ext) => path,
        _ => path.with_extension(ext),
    };
    path.to_string_lossy().into_owned()
}

/// One listed layer, as every mode reports it.
struct LayerRow {
    id: u64,
    name: String,
    visible: bool,
    selected: bool,
    depth: usize,
}

fn format_layers(rows: &[LayerRow], max: usize) -> String {
    if rows.is_empty() {
        return "no layers".to_string();
    }
    let mut out: Vec<String> = rows
        .iter()
        .take(max)
        .map(|r| {
            format!(
                "{}{} \"{}\" {}{}",
                "  ".repeat(r.depth),
                r.id,
                r.name,
                if r.visible { "visible" } else { "hidden" },
                if r.selected { " selected" } else { "" }
            )
        })
        .collect();
    if rows.len() > max {
        out.push(format!("... {} more", rows.len() - max));
    }
    out.join("\n")
}

/// The layer `args` names: by `id`, else by `name` (case-insensitive, exact).
fn find_layer(rows: &[LayerRow], args: &Value) -> Result<u64, String> {
    if let Some(id) = id_arg(args) {
        return rows
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.id)
            .ok_or_else(|| format!("no layer with id {id}"));
    }
    let name = str_arg(args, "id")
        .or_else(|_| str_arg(args, "name"))
        .map_err(|_| "id or name is required".to_string())?;
    let mut hits = rows
        .iter()
        .filter(|r| r.name.eq_ignore_ascii_case(name.trim()));
    match (hits.next(), hits.next()) {
        (Some(hit), None) => Ok(hit.id),
        (Some(_), Some(_)) => Err(format!("several layers are named \"{name}\"; use the id")),
        (None, _) => Err(format!("no layer named \"{name}\"")),
    }
}

impl ShellApp {
    /// Start the bridge once the egui context exists (from `main`, after the context is set up).
    pub fn start_bridge(&mut self, ctx: &egui::Context) {
        if self.bridge.is_none() {
            self.bridge = Some(Bridge::start(ctx.clone()));
        }
    }

    /// Stop the bridge: unregister from the daemon and stop its server.
    pub fn stop_bridge(&mut self) {
        if let Some(bridge) = self.bridge.take() {
            bridge.stop();
        }
    }

    /// Answer every tool call that arrived since the last frame. Called once per frame from
    /// `logic`, before the editors run.
    pub fn serve_bridge(&mut self) {
        loop {
            let request = match self.bridge.as_ref().map(|b| b.requests.try_recv()) {
                Some(Ok(request)) => request,
                _ => return,
            };
            let reply = self.serve_request(&request.tool, &request.args);
            if let Err(e) = &reply {
                log::debug!("app bridge: {} failed: {e}", request.tool);
            }
            // The caller may have given up (timed out); nothing to do about that.
            let _ = request.reply.send(reply);
        }
    }

    fn serve_request(&mut self, tool: &str, args: &Value) -> Reply {
        match tool {
            "document_info" => self.bridge_document_info(),
            "open_file" => self.bridge_open_file(args),
            "switch_mode" => self.bridge_switch_mode(args),
            "list_layers" => {
                let max = args
                    .get("max")
                    .and_then(Value::as_u64)
                    .map_or(DEFAULT_MAX_LAYERS, |m| m.max(1) as usize);
                Ok(format_layers(&self.bridge_layers(), max))
            }
            "select_layer" => self.bridge_select_layer(args),
            "set_layer_visible" => self.bridge_set_layer_visible(args),
            "export" => self.bridge_export(args),
            "save" => self.bridge_save(),
            other => Err(format!("unknown tool: {other}")),
        }
    }

    /// The active mode's document path, as the editor records it. The requests are served after
    /// a pending switch applied, so this is the mode the window shows.
    fn bridge_doc_path(&self) -> Option<String> {
        match self.mode {
            Mode::Photo => self.photo.as_ref()?.session.active()?.path.clone(),
            Mode::Vector => self.vector.as_ref()?.session.active()?.path.clone(),
            Mode::Layout => self.layout.as_ref()?.session.active()?.path.clone(),
        }
    }

    fn bridge_document_info(&mut self) -> Reply {
        let mode = self.mode;
        let Some((name, dirty)) = self.active_document() else {
            return Ok(format!("mode: {}\nno document open", mode_name(mode)));
        };
        let path = self.bridge_doc_path();
        let size = self.with_bridge(|b| ModeBridge::document_size(b));
        let layers = self.bridge_layers().len();
        let unit = match mode {
            Mode::Photo => "px",
            Mode::Vector | Mode::Layout => "pt",
        };
        Ok(format!(
            "mode: {}\nname: {name}\npath: {}\nsize: {}\nlayers: {layers}\ndirty: {dirty}",
            mode_name(mode),
            path.as_deref().unwrap_or("untitled (never saved)"),
            size.map_or_else(
                || "unknown".to_string(),
                |(w, h)| format!("{w} x {h} {unit}")
            ),
        ))
    }

    fn bridge_open_file(&mut self, args: &Value) -> Reply {
        let path = PathBuf::from(str_arg(args, "path")?);
        if !path.is_file() {
            return Err(format!("{}: no such file", path.display()));
        }
        let route = routing::route(&path)
            .ok_or_else(|| format!("{}: no editor opens this kind of file", path.display()))?;
        let name = super::file_name(&path);
        self.editor_open(route.mode, &path)
            .map_err(|e| format!("{name}: {e}"))?;
        self.set_mode(route.mode);
        self.set_status(
            crate::i18n::tf(
                "Opened {name} in {mode}",
                &[("name", &name), ("mode", route.mode.name())],
            ),
            false,
        );
        Ok(format!(
            "opened {} in {}",
            path.display(),
            mode_name(route.mode)
        ))
    }

    fn bridge_switch_mode(&mut self, args: &Value) -> Reply {
        let mode = parse_mode(str_arg(args, "mode")?)
            .ok_or_else(|| "mode must be raster, vector or design".to_string())?;
        self.set_mode(mode);
        Ok(format!("switched to {}", mode_name(mode)))
    }

    /// The active document's layers, top to bottom as the Layers panel shows them.
    fn bridge_layers(&mut self) -> Vec<LayerRow> {
        match self.mode {
            Mode::Photo => {
                let Some(st) = self.photo.as_ref().and_then(|a| a.session.active()) else {
                    return Vec::new();
                };
                let selected = st.selected_layers();
                let mut rows: Vec<LayerRow> = st
                    .doc
                    .walk()
                    .into_iter()
                    .map(|(_, depth, l)| LayerRow {
                        id: l.id.0,
                        name: l.name.clone(),
                        visible: l.visible,
                        selected: selected.contains(&l.id) || st.active_layer == Some(l.id),
                        depth,
                    })
                    .collect();
                // `walk` is bottom-to-top; the panel lists the top layer first.
                rows.reverse();
                rows
            }
            Mode::Vector => {
                let Some(st) = self.vector.as_ref().and_then(|a| a.session.active()) else {
                    return Vec::new();
                };
                let current = st.current_layer();
                let highlighted = st.highlighted_rows();
                fn rec(
                    nodes: &[Arc<redrob_vector_doc::Node>],
                    depth: usize,
                    current: Option<redrob_vector_doc::NodeId>,
                    highlighted: &[redrob_vector_doc::NodeId],
                    out: &mut Vec<LayerRow>,
                ) {
                    // Layers are listed top first, as the panel does.
                    for n in nodes.iter().rev() {
                        if !n.is_layer() {
                            continue;
                        }
                        out.push(LayerRow {
                            id: n.id.0,
                            name: n.name.clone().unwrap_or_else(|| "Layer".to_string()),
                            visible: n.visible,
                            selected: current == Some(n.id) || highlighted.contains(&n.id),
                            depth,
                        });
                        if let Some(children) = n.children() {
                            rec(children, depth + 1, current, highlighted, out);
                        }
                    }
                }
                let mut rows = Vec::new();
                rec(&st.doc.layers, 0, current, &highlighted, &mut rows);
                rows
            }
            Mode::Layout => {
                let Some(st) = self.layout.as_ref().and_then(|a| a.session.active()) else {
                    return Vec::new();
                };
                st.doc
                    .layers
                    .iter()
                    .rev()
                    .map(|l| LayerRow {
                        id: l.id.0,
                        name: l.name.clone(),
                        visible: l.visible,
                        selected: st.active_layer == l.id,
                        depth: 0,
                    })
                    .collect()
            }
        }
    }

    fn bridge_select_layer(&mut self, args: &Value) -> Reply {
        let rows = self.bridge_layers();
        if rows.is_empty() {
            return Err("open a document first".into());
        }
        let id = find_layer(&rows, args)?;
        let name = rows
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.name.clone())
            .unwrap_or_default();
        match self.mode {
            Mode::Photo => {
                // `layer.select` is the Layers panel's click (editors/photo/crates/engine).
                self.ensure_photo()
                    .run("layer.select", json!({ "layer": id }))?;
            }
            Mode::Vector => {
                // `layer.setCurrent` makes it the layer new art goes into; `layer.highlight`
                // is the panel row (editors/vector/crates/engine/src/cmd/layer.rs).
                let app = self.ensure_vector();
                app.run("layer.setCurrent", json!({ "id": id }))?;
                app.run("layer.highlight", json!({ "ids": [id], "mode": "set" }))?;
            }
            Mode::Layout => {
                // `layer.activate` (editors/layout/crates/engine/src/cmd/layout.rs).
                self.ensure_layout()
                    .run("layer.activate", json!({ "id": id }))?;
            }
        }
        Ok(format!("selected layer {id} \"{name}\""))
    }

    fn bridge_set_layer_visible(&mut self, args: &Value) -> Reply {
        let rows = self.bridge_layers();
        if rows.is_empty() {
            return Err("open a document first".into());
        }
        let id = find_layer(&rows, args)?;
        let visible = bool_arg(args, "visible")?;
        match self.mode {
            Mode::Photo => {
                let cmd = if visible {
                    "layer.showLayers"
                } else {
                    "layer.hideLayers"
                };
                self.ensure_photo().run(cmd, json!({ "layer": id }))?;
            }
            Mode::Vector => {
                self.ensure_vector()
                    .run("layer.setProps", json!({ "id": id, "visible": visible }))?;
            }
            Mode::Layout => {
                self.ensure_layout()
                    .run("layer.set", json!({ "id": id, "visible": visible }))?;
            }
        }
        Ok(format!(
            "layer {id} is now {}",
            if visible { "visible" } else { "hidden" }
        ))
    }

    fn bridge_export(&mut self, args: &Value) -> Reply {
        let format = str_arg(args, "format")?.trim().to_ascii_lowercase();
        if !matches!(format.as_str(), "png" | "svg" | "pdf") {
            return Err("format must be png, svg or pdf".into());
        }
        let scale = scale_arg(args)?;
        let mode = self.mode;
        let Some((name, _)) = self.active_document() else {
            return Err("open a document first".into());
        };
        let doc_path = self.bridge_doc_path();
        let target = export_target(args, doc_path.as_deref(), &name, &format);
        match mode {
            Mode::Photo => {
                if format == "svg" {
                    return Err("svg export is not available in raster mode; use png or pdf".into());
                }
                let app = self.ensure_photo();
                let st = app.session.active().ok_or("open a document first")?;
                let mut doc = st.doc.clone();
                if (scale - 1.0).abs() >= f64::EPSILON {
                    // The export dialog's own route: Image Size on a copy, bicubic
                    // (editors/photo/crates/ui-egui/src/export_dialog.rs).
                    let mut session = redrob_photo_engine::Session::new();
                    session.add_document((*doc).clone(), None);
                    let (w, h) = (
                        (f64::from(doc.size.width) * scale).round().max(1.0),
                        (f64::from(doc.size.height) * scale).round().max(1.0),
                    );
                    session
                        .execute(
                            "image.imageSize",
                            json!({ "width": w, "height": h, "resample": "bicubic" }),
                        )
                        .map_err(|e| e.to_string())?;
                    doc = session
                        .active()
                        .ok_or("the scaled copy went missing")?
                        .doc
                        .clone();
                }
                // `redrob_photo_io::export` picks the codec by the path's extension; PNG and
                // PDF flatten the layers themselves.
                let out = redrob_photo_io::export(
                    &doc,
                    &target,
                    &redrob_photo_io::ExportOptions::default(),
                )
                .map_err(|e| e.to_string())?;
                redrob_photo_format::atomic_write(Path::new(&target), &out.bytes)
                    .map_err(|e| e.to_string())?;
                app.ui.status = format!("Exported {target}");
                app.ui.status_error = false;
                Ok(target)
            }
            Mode::Vector => {
                // The editor's File > Export (editors/vector/crates/ui-egui/src/io.rs):
                // the current artboard; `scale` is pixels per point for PNG.
                let params = if format == "png" {
                    json!({ "scale": scale, "useArtboards": false })
                } else {
                    json!({ "useArtboards": false })
                };
                let r = redrob_vector_ui_egui::io::export(
                    self.ensure_vector(),
                    Some(format.as_str()),
                    Some(target.clone()),
                    &params,
                )?;
                if r.get("pending").is_some() {
                    return Err("the editor is asking about this export in a dialog; \
                                answer it in the window"
                        .into());
                }
                Ok(r["path"].as_str().unwrap_or(&target).to_string())
            }
            Mode::Layout => match format.as_str() {
                "pdf" => {
                    // `file.exportPdf` writes the file itself
                    // (editors/layout/crates/engine/src/cmd/export.rs).
                    let r = self
                        .ensure_layout()
                        .run("file.exportPdf", json!({ "path": target }))?;
                    Ok(r["path"].as_str().unwrap_or(&target).to_string())
                }
                "png" => {
                    let app = self.ensure_layout();
                    let abs = redrob_layout_ui_egui::canvas::current_page(app).unwrap_or(0);
                    let st = app.session.active().ok_or("open a document first")?;
                    if st.doc.page_count() == 0 {
                        return Err("the document has no pages".into());
                    }
                    // The page as the engine renders it for an EPUB cover
                    // (editors/layout/crates/render/src/lib.rs `render_page`).
                    let opts = redrob_layout_render::RenderOptions {
                        paper: true,
                        printing_only: true,
                        ..redrob_layout_render::RenderOptions::default()
                    };
                    let mut renderer = redrob_layout_render::Renderer::new();
                    let rendered = renderer
                        .render_page(&st.doc, &app.session.cache, abs, scale, false, &opts)
                        .ok_or("the page could not be rendered")?;
                    std::fs::write(&target, rendered.to_png()).map_err(|e| e.to_string())?;
                    Ok(target)
                }
                _ => Err("svg export is not available in design mode; use png or pdf".into()),
            },
        }
    }

    fn bridge_save(&mut self) -> Reply {
        let mode = self.mode;
        if self.active_document().is_none() {
            return Err("open a document first".into());
        }
        let Some(path) = self.bridge_doc_path() else {
            return Err(
                "the document is untitled; use File > Save As in the window to name its file"
                    .into(),
            );
        };
        match mode {
            Mode::Photo => {
                let app = self.ensure_photo();
                let st = app.session.active().ok_or("open a document first")?;
                // Only a file that holds the whole document is written in place; anything else
                // would go through Save As (editors/photo/crates/ui-egui/src/menus.rs).
                if !redrob_photo_engine::file_cmds::holds_at(&st.doc, &path) {
                    return Err(format!(
                        "{path} cannot hold this document's layers; \
                         use File > Save As in the window"
                    ));
                }
                app.run("file.save", json!({}))?;
            }
            Mode::Vector => {
                let r = redrob_vector_ui_egui::io::save(
                    self.ensure_vector(),
                    SaveMode::Save,
                    &json!({}),
                    false,
                )?;
                if r.get("pending").is_some() {
                    return Err(
                        "the editor is asking about this save in a dialog; answer it in the window"
                            .into(),
                    );
                }
            }
            Mode::Layout => {
                self.ensure_layout().run("file.save", json!({}))?;
            }
        }
        Ok(format!("saved {path}"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn mode_names_round_trip() {
        for mode in Mode::ALL {
            assert_eq!(parse_mode(mode_name(mode)), Some(mode));
        }
        assert_eq!(parse_mode("Photo"), Some(Mode::Photo));
        assert_eq!(parse_mode("3d"), None);
    }

    #[test]
    fn layers_are_found_by_id_or_unique_name() {
        let rows = vec![
            LayerRow {
                id: 1,
                name: "Sky".into(),
                visible: true,
                selected: false,
                depth: 0,
            },
            LayerRow {
                id: 2,
                name: "Sea".into(),
                visible: false,
                selected: true,
                depth: 0,
            },
            LayerRow {
                id: 3,
                name: "sea".into(),
                visible: true,
                selected: false,
                depth: 1,
            },
        ];
        assert_eq!(find_layer(&rows, &json!({ "id": 1 })).unwrap(), 1);
        assert_eq!(find_layer(&rows, &json!({ "id": "1" })).unwrap(), 1);
        assert_eq!(find_layer(&rows, &json!({ "name": "SKY" })).unwrap(), 1);
        assert!(find_layer(&rows, &json!({ "name": "sea" })).is_err());
        assert!(find_layer(&rows, &json!({ "id": 9 })).is_err());
        assert!(find_layer(&rows, &json!({})).is_err());
        let text = format_layers(&rows, 2);
        assert!(text.starts_with("1 \"Sky\" visible\n2 \"Sea\" hidden selected\n... 1 more"));
    }

    #[test]
    fn export_targets_keep_the_formats_extension() {
        assert_eq!(
            export_target(&json!({ "path": "/tmp/out.jpg" }), None, "a.psd", "png"),
            "/tmp/out.png"
        );
        assert_eq!(
            export_target(&json!({ "path": "/tmp/out.PNG" }), None, "a.psd", "png"),
            "/tmp/out.PNG"
        );
        let next_to = export_target(
            &json!({}),
            Some("/docs/poster.rphoto"),
            "poster.rphoto",
            "pdf",
        );
        assert_eq!(next_to, "/docs/poster.pdf");
        assert!(scale_arg(&json!({ "scale": 9 })).is_err());
        assert_eq!(scale_arg(&json!({})).unwrap(), 1.0);
    }
}
